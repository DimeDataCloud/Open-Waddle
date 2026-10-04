// Tiny synthesized sound effects (no audio files): pews, booms, zaps and quacks.

export class Sfx {
  private ctx: AudioContext | null = null;
  private noise: AudioBuffer | null = null;
  private voices = 0;
  muted = false;

  private audio(): AudioContext | null {
    if (this.muted) return null;
    try {
      this.ctx ??= new AudioContext();
      if (this.ctx.state === "suspended") void this.ctx.resume();
      if (!this.noise) {
        const len = this.ctx.sampleRate;
        this.noise = this.ctx.createBuffer(1, len, this.ctx.sampleRate);
        const d = this.noise.getChannelData(0);
        for (let i = 0; i < len; i++) d[i] = Math.random() * 2 - 1;
      }
      return this.ctx;
    } catch {
      return null;
    }
  }

  /** Keeps rapid-fire weapons from stacking dozens of sounds. */
  private voice(seconds: number): boolean {
    if (this.voices > 10) return false;
    this.voices++;
    window.setTimeout(() => this.voices--, seconds * 1000);
    return true;
  }

  private tone(type: OscillatorType, from: number, to: number, seconds: number, gain: number): void {
    const a = this.audio();
    if (!a || !this.voice(seconds)) return;
    const o = a.createOscillator();
    const g = a.createGain();
    o.type = type;
    o.frequency.setValueAtTime(from, a.currentTime);
    o.frequency.exponentialRampToValueAtTime(Math.max(1, to), a.currentTime + seconds);
    g.gain.setValueAtTime(gain, a.currentTime);
    g.gain.exponentialRampToValueAtTime(0.0001, a.currentTime + seconds);
    o.connect(g).connect(a.destination);
    o.start();
    o.stop(a.currentTime + seconds);
  }

  private burst(seconds: number, gain: number, cutoff: number): void {
    const a = this.audio();
    if (!a || !this.noise || !this.voice(seconds)) return;
    const src = a.createBufferSource();
    src.buffer = this.noise;
    const f = a.createBiquadFilter();
    f.type = "lowpass";
    f.frequency.setValueAtTime(cutoff, a.currentTime);
    f.frequency.exponentialRampToValueAtTime(60, a.currentTime + seconds);
    const g = a.createGain();
    g.gain.setValueAtTime(gain, a.currentTime);
    g.gain.exponentialRampToValueAtTime(0.0001, a.currentTime + seconds);
    src.connect(f).connect(g).connect(a.destination);
    src.start();
    src.stop(a.currentTime + seconds);
  }

  pew(): void {
    this.tone("square", 900 + Math.random() * 200, 220, 0.09, 0.05);
  }
  boom(size: number): void {
    this.burst(0.25 + size / 120, 0.5, 1800);
    this.tone("sine", 120, 40, 0.3, 0.3);
  }
  zap(): void {
    this.tone("sawtooth", 2400, 300, 0.35, 0.06);
  }
  fizz(): void {
    this.burst(0.08, 0.06, 3000);
  }
  quack(): void {
    this.tone("square", 520, 380, 0.08, 0.08);
    window.setTimeout(() => this.tone("square", 480, 330, 0.1, 0.08), 90);
  }
  thunk(): void {
    this.tone("triangle", 200, 90, 0.08, 0.15);
  }
}
