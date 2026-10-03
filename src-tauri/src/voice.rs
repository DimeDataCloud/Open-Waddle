//! Push-to-talk recording for the Whisper-compatible backend. The cpal stream
//! lives on its own thread (it is not `Send` on every platform); samples are
//! downmixed and resampled to 16 kHz mono when recording stops.

use anyhow::{anyhow, Context};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TARGET_RATE: u32 = 16_000;
const MAX_SECONDS: usize = 60;

pub struct Recording {
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    stop: mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub fn start() -> anyhow::Result<Recording> {
    let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<anyhow::Result<u32>>();
    let buf = samples.clone();
    let thread = std::thread::Builder::new().name("waddle-mic".into()).spawn(move || {
        let setup = || -> anyhow::Result<(cpal::Stream, u32)> {
            let device = cpal::default_host().default_input_device().ok_or_else(|| anyhow!("no microphone found"))?;
            let config = device.default_input_config().context("microphone config")?;
            let rate = config.sample_rate();
            let stream_config: cpal::StreamConfig = config.into();
            let channels = config.channels() as usize;
            let cap = rate as usize * MAX_SECONDS;
            let err_fn = |e| log::warn!("microphone stream error: {e}");
            let push = move |data: &mut dyn Iterator<Item = f32>, buf: &Arc<Mutex<Vec<f32>>>| {
                let mut b = buf.lock().unwrap();
                let mut frame = Vec::with_capacity(channels);
                for s in data {
                    frame.push(s);
                    if frame.len() == channels {
                        if b.len() < cap {
                            b.push(frame.iter().sum::<f32>() / channels as f32);
                        }
                        frame.clear();
                    }
                }
            };
            let stream = match config.sample_format() {
                cpal::SampleFormat::F32 => {
                    let buf = buf.clone();
                    device.build_input_stream(stream_config, move |d: &[f32], _| push(&mut d.iter().copied(), &buf), err_fn, None)?
                }
                cpal::SampleFormat::I16 => {
                    let buf = buf.clone();
                    device.build_input_stream(
                        stream_config,
                        move |d: &[i16], _| push(&mut d.iter().map(|s| *s as f32 / i16::MAX as f32), &buf),
                        err_fn,
                        None,
                    )?
                }
                cpal::SampleFormat::U16 => {
                    let buf = buf.clone();
                    device.build_input_stream(
                        stream_config,
                        move |d: &[u16], _| push(&mut d.iter().map(|s| (*s as f32 - 32768.0) / 32768.0), &buf),
                        err_fn,
                        None,
                    )?
                }
                other => return Err(anyhow!("unsupported microphone format {other:?}")),
            };
            stream.play()?;
            Ok((stream, rate))
        };
        match setup() {
            Ok((stream, rate)) => {
                let _ = ready_tx.send(Ok(rate));
                let _ = stop_rx.recv_timeout(Duration::from_secs(MAX_SECONDS as u64 + 5));
                drop(stream);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
            }
        }
    })?;
    let rate = ready_rx.recv_timeout(Duration::from_secs(5)).map_err(|_| anyhow!("microphone did not start"))??;
    Ok(Recording { samples, rate, stop: stop_tx, thread: Some(thread) })
}

impl Recording {
    /// Stops recording and returns 16 kHz mono PCM.
    pub fn finish(mut self) -> Vec<i16> {
        let _ = self.stop.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let samples = std::mem::take(&mut *self.samples.lock().unwrap());
        resample_to_pcm16(&samples, self.rate)
    }
}

/// Linear-interpolation resample to 16 kHz, clipped to i16.
pub fn resample_to_pcm16(samples: &[f32], rate: u32) -> Vec<i16> {
    if samples.is_empty() || rate == 0 {
        return vec![];
    }
    let ratio = rate as f64 / TARGET_RATE as f64;
    let out_len = (samples.len() as f64 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = samples[idx.min(samples.len() - 1)];
            let b = samples[(idx + 1).min(samples.len() - 1)];
            ((a + (b - a) * frac).clamp(-1.0, 1.0) * i16::MAX as f32) as i16
        })
        .collect()
}

pub fn target_rate() -> u32 {
    TARGET_RATE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resamples_48k_to_16k() {
        let input: Vec<f32> = (0..48_000).map(|i| (i as f32 / 48_000.0 * std::f32::consts::TAU * 440.0).sin()).collect();
        let out = resample_to_pcm16(&input, 48_000);
        assert_eq!(out.len(), 16_000);
        assert!(out.iter().any(|s| *s > 20_000));
    }
}
