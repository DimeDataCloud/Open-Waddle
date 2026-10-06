import { describe, expect, it } from "vitest";
import { alphaAt, asks, Fx, isThanks, positionAt, type Particle } from "./fx";

const p: Particle = { kind: "dust", x: 100, y: 200, vx: 50, vy: -20, g: 40, born: 1000, life: 500, size: 2 };

describe("particles", () => {
  it("move with their speed and gravity", () => {
    expect(positionAt(p, 1000)).toEqual({ x: 100, y: 200 });
    const half = positionAt(p, 1500);
    expect(half.x).toBeCloseTo(125);
    expect(half.y).toBeCloseTo(200 - 10 + 0.5 * 40 * 0.25);
  });

  it("hold, fade, and are gone at the end of their life", () => {
    expect(alphaAt(p, 1000)).toBe(1);
    expect(alphaAt(p, 1250)).toBe(1);
    expect(alphaAt(p, 1400)).toBeCloseTo(0.5);
    expect(alphaAt(p, 1500)).toBe(0);
    expect(alphaAt(p, 900)).toBe(0);
  });
});

describe("Fx", () => {
  it("runs until its effects are over", () => {
    const fx = new Fx(false, () => 0.5);
    expect(fx.active).toBe(false);
    fx.emit("dust", 50, 50, 0);
    fx.emit("heart", 50, 30, 0);
    expect(fx.active).toBe(true);
    fx.update(600);
    // Dust (450 ms) is gone; the heart (1.4 s) isn't.
    fx.update(1500);
    expect(fx.active).toBe(false);
  });

  it("with reduced motion, shows only still symbols", () => {
    const fx = new Fx(true);
    fx.emit("dust", 0, 0, 0);
    expect(fx.active).toBe(false);
    fx.emit("question", 0, 0, 0);
    expect(fx.active).toBe(true);
  });
});

describe("words", () => {
  it("knows thanks and questions", () => {
    expect(isThanks("Thanks!")).toBe(true);
    expect(isThanks("thank you so much")).toBe(true);
    expect(isThanks("that's thankless work")).toBe(false);
    expect(asks("Want me to send it?")).toBe(true);
    expect(asks('Should I say "yes?"')).toBe(true);
    expect(asks("Done. Any questions? No? Great.")).toBe(false);
  });
});
