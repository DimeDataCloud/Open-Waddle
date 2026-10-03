//! Speech-to-text through any OpenAI-compatible `/audio/transcriptions`
//! endpoint (Groq's free tier, OpenAI, or a local whisper server).

use anyhow::{anyhow, Context};
use serde_json::Value;

/// Encodes 16-bit mono PCM as a WAV file.
pub fn wav_from_pcm16(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

pub async fn transcribe(base_url: &str, api_key: Option<&str>, model: &str, language: Option<&str>, wav: Vec<u8>) -> anyhow::Result<String> {
    let url = format!("{}/audio/transcriptions", base_url.trim_end_matches('/'));
    let file = reqwest::multipart::Part::bytes(wav).file_name("speech.wav").mime_str("audio/wav")?;
    let mut form = reqwest::multipart::Form::new()
        .part("file", file)
        .text("model", model.to_string())
        .text("response_format", "json");
    if let Some(lang) = language.filter(|l| !l.is_empty()) {
        form = form.text("language", lang.to_string());
    }
    let mut req = reqwest::Client::new().post(&url).multipart(form);
    if let Some(key) = api_key.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(key);
    }
    let resp = req.send().await.with_context(|| format!("could not reach {url}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("{status} from {url}: {}", body.chars().take(300).collect::<String>()));
    }
    let v: Value = serde_json::from_str(&body).context("unexpected transcription response")?;
    Ok(v.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_well_formed() {
        let wav = wav_from_pcm16(&[0, 1000, -1000], 16_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 6);
        assert_eq!(wav.len(), 44 + 6);
    }
}
