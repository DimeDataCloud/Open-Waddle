import { describe, expect, it } from "vitest";
import { CELL, pathLength, planPath, standBeside } from "./pathfind";
import { computeSegments } from "./platforms";

const opts = { width: 1440, floorY: 900, minHeadroom: 56, minWidth: 24 };

describe("planPath", () => {
  it("walks along the floor without flying", () => {
    const segs = computeSegments([], opts);
    const path = planPath(segs, { x: 100, y: 900 }, { x: 800, y: 900 });
    expect(path).toEqual([{ x: 800, y: 900, kind: "walk" }]);
  });

  it("jumps onto a low window rather than flying", () => {
    const segs = computeSegments([{ id: 1, x: 400, y: 840, w: 400, h: 60 }], opts);
    const path = planPath(segs, { x: 100, y: 900 }, { x: 600, y: 840 });
    expect(path.some((w) => w.kind === "jump")).toBe(true);
    expect(path.some((w) => w.kind === "fly")).toBe(false);
    expect(path[path.length - 1]).toMatchObject({ y: 840 });
  });

  it("prefers walking over an expensive upward jump when both reach", () => {
    const segs = computeSegments([{ id: 1, x: 300, y: 850, w: 100, h: 50 }], opts);
    const path = planPath(segs, { x: 100, y: 900 }, { x: 700, y: 900 });
    expect(path.every((w) => w.kind === "walk")).toBe(true);
  });

  it("flies the last leg to a point with no standing room", () => {
    const segs = computeSegments([], opts);
    const path = planPath(segs, { x: 100, y: 900 }, { x: 700, y: 300 });
    expect(path[path.length - 1]).toEqual({ x: 700, y: 300, kind: "fly" });
    const walked = path.filter((w) => w.kind === "walk");
    expect(walked.length).toBeLessThanOrEqual(1);
  });

  it("drops off a window edge to reach the floor", () => {
    const segs = computeSegments([{ id: 1, x: 200, y: 600, w: 300, h: 300 }], opts);
    const path = planPath(segs, { x: 300, y: 600 }, { x: 900, y: 900 });
    expect(path.some((w) => w.kind === "drop" || w.kind === "jump")).toBe(true);
    expect(path.some((w) => w.kind === "fly")).toBe(false);
  });

  it("flies directly when the duck isn't standing on anything", () => {
    const segs = computeSegments([], opts);
    expect(planPath(segs, { x: 100, y: 500 }, { x: 300, y: 900 })).toEqual([{ x: 300, y: 900, kind: "fly" }]);
  });

  it("measures path length and grid cell size", () => {
    expect(CELL).toBe(8);
    expect(pathLength({ x: 0, y: 0 }, [{ x: 3, y: 4, kind: "walk" }])).toBe(5);
  });
});

describe("standBeside", () => {
  it("keeps the target outside the sprite, beak first", () => {
    const duck = { w: 64, h: 56 };
    const p = standBeside({ x: 500, y: 300 }, duck, 1440);
    expect(p.facing).toBe(1);
    expect(p.x + duck.w / 2).toBeLessThan(500);
    const left = standBeside({ x: 20, y: 300 }, duck, 1440);
    expect(left.facing).toBe(-1);
    expect(left.x - duck.w / 2).toBeGreaterThan(20);
  });
});
