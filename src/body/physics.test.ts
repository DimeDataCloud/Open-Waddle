import { describe, expect, it } from "vitest";
import { computeSegments } from "./platforms";
import { fall, newBody, ride } from "./physics";

const opts = { width: 1440, floorY: 900, minHeadroom: 56, minWidth: 24 };

describe("physics", () => {
  it("falls and lands on the first window edge below", () => {
    const segs = computeSegments([{ id: 3, x: 0, y: 400, w: 500, h: 300 }], opts);
    const b = newBody(100, 100);
    let landed = false;
    for (let i = 0; i < 200 && !landed; i++) landed = fall(b, segs, 1 / 60);
    expect(landed).toBe(true);
    expect(b.y).toBe(400);
    expect(b.supportId).toBe(3);
  });

  it("rides a window that moves and falls when it disappears", () => {
    let segs = computeSegments([{ id: 3, x: 0, y: 400, w: 500, h: 300 }], opts);
    const b = { ...newBody(100, 400), grounded: true, supportId: 3 };
    segs = computeSegments([{ id: 3, x: 50, y: 380, w: 500, h: 300 }], opts);
    ride(b, segs, new Map([[3, [50, -20]]]));
    expect([b.x, b.y, b.grounded]).toEqual([150, 380, true]);
    segs = computeSegments([], opts);
    ride(b, segs, new Map());
    expect(b.grounded).toBe(false);
  });
});
