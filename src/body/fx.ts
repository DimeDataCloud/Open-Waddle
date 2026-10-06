// Little effects around the duck: dust when it lands, a sparkle when a task
// is done, a sweat drop when one fails, a "?" when it asks something, a heart
// when thanked. Drawn on the duck's canvas with plain shapes (no emoji font
// needed). With reduced motion, only the still symbols show.

export type Effect = "dust" | "sparkle" | "sweat" | "question" | "heart";

export interface Particle {
  kind: Effect;
  x: number;
  y: number;
  vx: number;
  vy: number;
  /** Pixels per second squared, downwards. */
  g: number;
  born: number;
  life: number;
  size: number;
}

export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Where a particle is at `now` (ms). */
export function positionAt(p: Particle, now: number): { x: number; y: number } {
  const t = Math.max(0, now - p.born) / 1000;
  return { x: p.x + p.vx * t, y: p.y + p.vy * t + 0.5 * p.g * t * t };
}

/** 1 when new, fading to 0 at the end of its life. */
export function alphaAt(p: Particle, now: number): number {
  const age = (now - p.born) / p.life;
  if (age < 0 || age >= 1) return 0;
  // Holds, then fades over the last 40%.
  return age < 0.6 ? 1 : 1 - (age - 0.6) / 0.4;
}

const COLORS: Record<Effect, string> = {
  dust: "#b9a888",
  sparkle: "#fff1a8",
  sweat: "#6cc4ff",
  question: "#2a1e14",
  heart: "#e0453a",
};

export class Fx {
  private particles: Particle[] = [];
  private drawn: Box | null = null;

  constructor(
    private reducedMotion = false,
    private rand: () => number = Math.random,
  ) {}

  get active(): boolean {
    return this.particles.length > 0 || this.drawn !== null;
  }

  /** Adds an effect around (x, y): the duck's feet for dust, its head for the rest. */
  emit(effect: Effect, x: number, y: number, now: number): void {
    const r = this.rand;
    const still = this.reducedMotion;
    const add = (p: Omit<Particle, "kind" | "born">) => this.particles.push({ kind: effect, born: now, ...p });
    switch (effect) {
      case "dust":
        if (still) return;
        for (let i = 0; i < 6; i++) {
          const dir = i % 2 ? 1 : -1;
          add({ x: x + dir * (4 + r() * 8), y, vx: dir * (30 + r() * 50), vy: -(10 + r() * 30), g: 60, life: 450, size: 2 + r() * 2 });
        }
        return;
      case "sparkle":
        if (still) {
          add({ x, y: y - 10, vx: 0, vy: 0, g: 0, life: 900, size: 6 });
          return;
        }
        for (let i = 0; i < 6; i++) {
          const a = (i / 6) * Math.PI * 2 + r() * 0.5;
          add({ x: x + Math.cos(a) * 10, y: y + Math.sin(a) * 6 - 6, vx: Math.cos(a) * 40, vy: Math.sin(a) * 25 - 40, g: 0, life: 900, size: 3 + r() * 3 });
        }
        return;
      case "sweat":
        add({ x: x + 14, y: y - 4, vx: still ? 0 : 12, vy: 0, g: still ? 0 : 90, life: 1100, size: 4 });
        return;
      case "question":
        add({ x: x + 12, y: y - 8, vx: 0, vy: still ? 0 : -18, g: 0, life: 1500, size: 18 });
        return;
      case "heart":
        add({ x, y: y - 8, vx: 0, vy: still ? 0 : -30, g: 0, life: 1400, size: 9 });
        return;
    }
  }

  /** Forgets particles whose time is up. */
  update(now: number): void {
    this.particles = this.particles.filter((p) => now - p.born < p.life);
  }

  /** The area the last frame drew on, to clear before drawing again. */
  dirty(): Box | null {
    return this.drawn;
  }

  /** Draws everything alive and remembers where, for the next clear. */
  draw(ctx: CanvasRenderingContext2D, now: number): void {
    this.update(now);
    let box: Box | null = null;
    for (const p of this.particles) {
      const a = alphaAt(p, now);
      if (a <= 0) continue;
      const { x, y } = positionAt(p, now);
      ctx.globalAlpha = a;
      ctx.fillStyle = COLORS[p.kind];
      ctx.strokeStyle = COLORS[p.kind];
      const s = p.size;
      switch (p.kind) {
        case "dust":
          ctx.beginPath();
          ctx.arc(x, y, s, 0, Math.PI * 2);
          ctx.fill();
          break;
        case "sparkle":
          // A four-pointed star.
          ctx.beginPath();
          ctx.moveTo(x, y - s * 1.6);
          ctx.lineTo(x + s * 0.4, y - s * 0.4);
          ctx.lineTo(x + s * 1.6, y);
          ctx.lineTo(x + s * 0.4, y + s * 0.4);
          ctx.lineTo(x, y + s * 1.6);
          ctx.lineTo(x - s * 0.4, y + s * 0.4);
          ctx.lineTo(x - s * 1.6, y);
          ctx.lineTo(x - s * 0.4, y - s * 0.4);
          ctx.closePath();
          ctx.fill();
          ctx.strokeStyle = "#e6a700";
          ctx.lineWidth = 1;
          ctx.stroke();
          break;
        case "sweat":
          // A drop: a circle with a point on top.
          ctx.beginPath();
          ctx.arc(x, y, s, 0, Math.PI);
          ctx.lineTo(x, y - s * 2.2);
          ctx.closePath();
          ctx.fill();
          break;
        case "question":
          ctx.font = `bold ${s}px monospace`;
          ctx.textAlign = "center";
          ctx.lineWidth = 3;
          ctx.strokeStyle = "#fffdf6";
          ctx.strokeText("?", x, y);
          ctx.fillText("?", x, y);
          ctx.textAlign = "start";
          break;
        case "heart":
          ctx.beginPath();
          ctx.moveTo(x, y + s);
          ctx.bezierCurveTo(x - s * 1.6, y - s * 0.2, x - s * 0.8, y - s * 1.4, x, y - s * 0.5);
          ctx.bezierCurveTo(x + s * 0.8, y - s * 1.4, x + s * 1.6, y - s * 0.2, x, y + s);
          ctx.fill();
          break;
      }
      const pad = s * 2 + 4;
      box = union(box, { x: x - pad, y: y - pad - (p.kind === "question" ? s : 0), w: pad * 2, h: pad * 2 + (p.kind === "question" ? s : 0) });
    }
    ctx.globalAlpha = 1;
    this.drawn = box;
  }
}

function union(a: Box | null, b: Box): Box {
  if (!a) return b;
  const x = Math.min(a.x, b.x);
  const y = Math.min(a.y, b.y);
  return { x, y, w: Math.max(a.x + a.w, b.x + b.w) - x, h: Math.max(a.y + a.h, b.y + b.h) - y };
}

/** Whether a message thanks Waddle. */
export function isThanks(text: string): boolean {
  return /\b(thanks|thank you|thank u|thx|ty|cheers|ta|merci|danke|gracias)\b/i.test(text);
}

/** Whether a reply ends by asking the user something. */
export function asks(text: string): boolean {
  return /\?\s*["')\]]?\s*$/.test(text.trim());
}
