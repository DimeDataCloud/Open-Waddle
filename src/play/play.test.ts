import { describe, expect, it } from "vitest";
import { Bot } from "./bot";
import { Player, type Controls } from "./player";
import { CELL, Terrain } from "./terrain";
import { fire, steer, WEAPONS, weaponIndex } from "./weapons";

const none: Controls = { left: false, right: false, jump: false, jumpPressed: false };

function seeded(seed = 1): () => number {
  let s = seed;
  return () => ((s = (s * 16807) % 2147483647) / 2147483647);
}

describe("terrain", () => {
  it("makes windows solid and the wallpaper open air", () => {
    const t = new Terrain(400, 300, [{ x: 100, y: 100, w: 200, h: 100 }]);
    expect(t.solidAt(150, 150)).toBe(true);
    expect(t.solidAt(50, 50)).toBe(false);
    expect(t.total).toBe((200 / CELL) * (100 / CELL));
    // The bottom of the screen is a floor, the sides are walls, the top is open.
    expect(t.solidAt(10, 300)).toBe(true);
    expect(t.solidAt(-1, 10)).toBe(true);
    expect(t.solidAt(10, -5)).toBe(false);
  });

  it("blasts holes and counts the damage", () => {
    const t = new Terrain(400, 300, [{ x: 0, y: 0, w: 400, h: 300 }]);
    const gone: number[][] = [];
    const n = t.carveCircle(200, 150, 20, (x, y) => gone.push([x, y]));
    expect(n).toBeGreaterThan(70);
    expect(gone.length).toBe(n);
    expect(t.solidAt(200, 150)).toBe(false);
    expect(t.solidAt(260, 150)).toBe(true);
    expect(t.destroyed).toBeCloseTo(n / t.total);
    // Blasting the same spot again finds nothing left.
    expect(t.carveCircle(200, 150, 20)).toBe(0);
  });

  it("finds where a shot hits", () => {
    const t = new Terrain(400, 300, [{ x: 200, y: 0, w: 50, h: 300 }]);
    const hit = t.firstHit(10, 100, 390, 100)!;
    expect(hit.x).toBeGreaterThanOrEqual(200);
    expect(hit.x).toBeLessThan(204);
    expect(t.firstHit(10, 100, 150, 100)).toBeNull();
  });

  it("lasers cut a line straight through", () => {
    const t = new Terrain(400, 300, [{ x: 0, y: 100, w: 400, h: 100 }]);
    t.carveLine(0, 150, 400, 150, 6);
    for (let x = 4; x < 400; x += 20) expect(t.solidAt(x, 150)).toBe(false);
    expect(t.solidAt(200, 120)).toBe(true);
  });
});

describe("player", () => {
  it("lands on a window and walks along it", () => {
    const t = new Terrain(600, 400, [{ x: 0, y: 200, w: 600, h: 40 }]);
    const p = new Player(100, 50, 40, 36);
    for (let i = 0; i < 120; i++) p.update(1 / 60, none, t);
    expect(p.onGround).toBe(true);
    expect(p.y + p.h).toBeCloseTo(200, 0);
    for (let i = 0; i < 60; i++) p.update(1 / 60, { ...none, right: true }, t);
    expect(p.x).toBeGreaterThan(300);
    expect(p.y + p.h).toBeCloseTo(200, 0);
  });

  it("falls through a hole blasted under its feet", () => {
    const t = new Terrain(600, 400, [{ x: 0, y: 200, w: 600, h: 40 }]);
    const p = new Player(100, 164, 40, 36);
    p.update(1 / 60, none, t);
    expect(p.onGround).toBe(true);
    t.carveCircle(120, 220, 40);
    for (let i = 0; i < 90; i++) p.update(1 / 60, none, t);
    expect(p.y + p.h).toBeCloseTo(400, 0);
  });

  it("flaps to climb, but runs out of puff", () => {
    const t = new Terrain(600, 800, []);
    const p = new Player(100, 764, 40, 36);
    p.update(1 / 60, none, t);
    p.update(1 / 60, { ...none, jump: true, jumpPressed: true }, t);
    let top = p.y;
    let lowest = 1;
    for (let i = 0; i < 240; i++) {
      p.update(1 / 60, { ...none, jump: true }, t);
      top = Math.min(top, p.y);
      lowest = Math.min(lowest, p.fuel);
    }
    expect(top).toBeLessThan(500);
    expect(lowest).toBe(0);
    expect(p.y + p.h).toBeCloseTo(800, 0);
  });

  it("can't walk through a wall but steps up small ledges", () => {
    const t = new Terrain(600, 400, [
      { x: 300, y: 0, w: 40, h: 400 },
      { x: 150, y: 394, w: 60, h: 6 },
    ]);
    const p = new Player(50, 364, 40, 36);
    for (let i = 0; i < 240; i++) p.update(1 / 60, { ...none, right: true }, t);
    expect(p.x + p.w).toBeLessThanOrEqual(300);
    expect(p.x + p.w).toBeGreaterThan(290);
  });
});

describe("weapons", () => {
  it("shotguns spray several pellets, pistols one", () => {
    const rand = seeded();
    expect(fire(WEAPONS[weaponIndex("feathers")], 0, 0, 0, rand)).toHaveLength(8);
    const [pea] = fire(WEAPONS[weaponIndex("pea")], 0, 0, Math.PI / 2, rand);
    expect(Math.abs(pea.vx)).toBeLessThan(30);
    expect(pea.vy).toBeGreaterThan(1200);
    expect(weaponIndex("nope")).toBe(0);
  });

  it("missiles turn toward their target", () => {
    const [m] = fire(WEAPONS[weaponIndex("quack")], 0, 0, 0, () => 0.5);
    for (let i = 0; i < 60; i++) steer(m, 0, 500, 4, 1 / 60);
    expect(m.vy).toBeGreaterThan(400);
  });
});

describe("autoplay bot", () => {
  it("heads for something solid and opens fire", () => {
    const t = new Terrain(1200, 600, [{ x: 800, y: 200, w: 300, h: 300 }]);
    const p = new Player(50, 564, 40, 36);
    const bot = new Bot(seeded(3), null);
    let fired = false;
    for (let i = 0; i < 300; i++) {
      const m = bot.think(1 / 60, p, t);
      p.update(1 / 60, m.controls, t);
      fired ||= m.fire;
    }
    expect(p.x).toBeGreaterThan(200);
    expect(fired).toBe(true);
  });

  it("aims inside the window the AI picked", () => {
    const t = new Terrain(1200, 600, [
      { x: 0, y: 100, w: 400, h: 300 },
      { x: 700, y: 100, w: 400, h: 300 },
    ]);
    const p = new Player(500, 564, 40, 36);
    const bot = new Bot(seeded(5), { x: 700, y: 100, w: 400, h: 300 });
    for (let i = 0; i < 50; i++) {
      const m = bot.think(1 / 60, p, t);
      expect(m.aimX).toBeGreaterThan(680);
    }
  });
});
