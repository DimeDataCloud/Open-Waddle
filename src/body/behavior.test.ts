import { describe, expect, it } from "vitest";
import { Duck, pickWanderTarget } from "./behavior";
import { computeSegments } from "./platforms";

const opts = { width: 1440, floorY: 900, minHeadroom: 56, minWidth: 24 };
const size = { w: 64, h: 56 };

function settle(duck: Duck, segs: ReturnType<typeof computeSegments>, seconds: number) {
  for (let t = 0; t < seconds; t += 1 / 60) duck.update(1 / 60, segs, new Map());
}

describe("Duck", () => {
  it("falls to the floor, then walks to a target and reports arrival", () => {
    const segs = computeSegments([], opts);
    const duck = new Duck(size, 200, 100);
    settle(duck, segs, 2);
    expect(duck.mode).toBe("idle");
    expect(duck.body.y).toBe(900);
    let arrived = false;
    duck.moveTo({ x: 600, y: 900 }, segs, { onArrive: () => (arrived = true) });
    expect(duck.facing).toBe(1);
    settle(duck, segs, 5);
    expect(arrived).toBe(true);
    expect(duck.mode).toBe("idle");
    expect(duck.body.x).toBe(600);
  });

  it("respects the trip time cap", () => {
    const segs = computeSegments([], opts);
    const duck = new Duck(size, 100, 900);
    settle(duck, segs, 0.1);
    let arrived = false;
    duck.moveTo({ x: 1300, y: 900 }, segs, { maxSeconds: 1, onArrive: () => (arrived = true) });
    settle(duck, segs, 1.1);
    expect(arrived).toBe(true);
  });

  it("hovers after flying to a point in mid-air, then lands", () => {
    const segs = computeSegments([], opts);
    const duck = new Duck(size, 100, 900);
    settle(duck, segs, 0.1);
    duck.moveTo({ x: 400, y: 300 }, segs, { maxSeconds: 1 });
    settle(duck, segs, 2);
    expect(duck.mode).toBe("hovering");
    expect(["flap_up", "flap_down"]).toContain(duck.frame(1000));
    duck.land(segs);
    expect(duck.mode).toBe("falling");
    settle(duck, segs, 2);
    expect(duck.mode).toBe("idle");
  });

  it("cancelling a move still releases the waiter", () => {
    const segs = computeSegments([], opts);
    const duck = new Duck(size, 100, 900);
    settle(duck, segs, 0.1);
    let released = false;
    duck.moveTo({ x: 900, y: 900 }, segs, { onArrive: () => (released = true) });
    duck.grab();
    expect(released).toBe(true);
    expect(duck.mode).toBe("dragged");
  });

  it("picks wander targets on surfaces", () => {
    const segs = computeSegments([], opts);
    const t = pickWanderTarget(segs, 40, () => 0.5)!;
    expect(t.y).toBe(900);
    expect(t.x).toBeGreaterThan(40);
  });
});
