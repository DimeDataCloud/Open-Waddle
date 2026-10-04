import { describe, expect, it } from "vitest";
import { planIntent } from "./intent";
import type { Segment, WinRect } from "./platforms";

const win: WinRect = { id: 7, x: 200, y: 100, w: 600, h: 400 };
const segments: Segment[] = [
  { x1: 200, x2: 800, y: 100, windowId: 7 },
  { x1: 0, x2: 1400, y: 900, windowId: null },
];

describe("planIntent", () => {
  it("perches on the front window", () => {
    const p = planIntent("perch", win, segments, 32, null, 50, () => 0.5)!;
    expect(p.stops).toEqual([{ x: 500, y: 100 }]);
    expect(p.stay).toBe(true);
  });

  it("explores the window's edge from the nearer end", () => {
    const p = planIntent("explore", win, segments, 32, null, 900)!;
    expect(p.stops.map((s) => s.x)).toEqual([768, 500, 232]);
    expect(p.inspect).toBe(true);
  });

  it("gives space in the floor corner away from the window", () => {
    const p = planIntent("give_space", { ...win, x: 900 }, segments, 32, null, 0)!;
    expect(p.stops).toEqual([{ x: 32, y: 900 }]);
  });

  it("watches the cursor from a polite distance", () => {
    const p = planIntent("watch", win, segments, 32, { x: 400, y: 60 }, 900)!;
    expect(p.stops).toEqual([{ x: 520, y: 100 }]);
    expect(p.faceX).toBe(400);
  });

  it("naps, and falls back to wandering when there's nowhere to go", () => {
    expect(planIntent("nap", null, segments, 32, null, 0)!.sleep).toBe(true);
    expect(planIntent("perch", null, segments, 32, null, 0)).toBeNull();
    expect(planIntent("wander", win, segments, 32, null, 0)).toBeNull();
  });
});
