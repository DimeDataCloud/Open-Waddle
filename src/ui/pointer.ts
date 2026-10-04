// "Show me where": a pulsing ring around a spot on screen, with a short label.
// It never takes clicks, so the user can click the thing it circles.

const SHOW_MS = 8000;

export class Pointer {
  private timer = 0;
  private el: HTMLElement | null = null;

  show(x: number, y: number, label: string, screen: { w: number; h: number }): void {
    this.hide();
    const el = document.createElement("div");
    el.className = "pointer";
    el.style.left = `${x}px`;
    el.style.top = `${y}px`;
    el.innerHTML = '<div class="ring"></div><div class="ring late"></div>';
    if (label.trim()) {
      const tag = document.createElement("div");
      tag.className = "tag";
      tag.textContent = label;
      // Keep the label on screen: flip it left near the right edge, above near the bottom.
      if (x > screen.w - 220) tag.classList.add("left");
      if (y > screen.h - 80) tag.classList.add("above");
      el.appendChild(tag);
    }
    document.body.appendChild(el);
    this.el = el;
    this.timer = window.setTimeout(() => this.hide(), SHOW_MS);
  }

  hide(): void {
    window.clearTimeout(this.timer);
    this.el?.remove();
    this.el = null;
  }
}

/** Two soft notes, so a reminder is noticed without being shrill. */
export function chime(): void {
  try {
    const ac = new AudioContext();
    [880, 1318.5].forEach((freq, i) => {
      const o = ac.createOscillator();
      const g = ac.createGain();
      const t = ac.currentTime + i * 0.18;
      o.type = "sine";
      o.frequency.value = freq;
      g.gain.setValueAtTime(0, t);
      g.gain.linearRampToValueAtTime(0.18, t + 0.02);
      g.gain.exponentialRampToValueAtTime(0.001, t + 0.5);
      o.connect(g).connect(ac.destination);
      o.start(t);
      o.stop(t + 0.55);
    });
    window.setTimeout(() => void ac.close(), 1200);
  } catch {
    // No audio device: the bubble still shows.
  }
}
