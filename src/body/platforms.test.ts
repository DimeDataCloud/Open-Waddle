import { describe, expect, it } from "vitest";
import { computeSegments, supportAt, surfaceBelow, type WinRect } from "./platforms";

const opts = { width: 1440, floorY: 900, minHeadroom: 56, minWidth: 24 };

describe("platforms", () => {
  it("always has a floor", () => {
    expect(computeSegments([], opts)).toEqual([{ x1: 0, x2: 1440, y: 900, windowId: null }]);
  });

  it("hides the parts of an edge covered by windows in front", () => {
    const front: WinRect = { id: 1, x: 300, y: 100, w: 400, h: 400 };
    const back: WinRect = { id: 2, x: 100, y: 200, w: 800, h: 300 };
    const segs = computeSegments([front, back], opts).filter((s) => s.windowId === 2);
    expect(segs).toEqual([
      { x1: 100, x2: 300, y: 200, windowId: 2 },
      { x1: 700, x2: 900, y: 200, windowId: 2 },
    ]);
  });

  it("skips edges without headroom (maximised windows) and slivers", () => {
    const maximised: WinRect = { id: 1, x: 0, y: 0, w: 1440, h: 900 };
    const sliver: WinRect = { id: 2, x: 100, y: 300, w: 10, h: 100 };
    expect(computeSegments([maximised, sliver], opts)).toHaveLength(1);
  });

  it("finds support and the surface below", () => {
    const segs = computeSegments([{ id: 7, x: 100, y: 300, w: 200, h: 200 }], opts);
    expect(supportAt(segs, 150, 301)?.windowId).toBe(7);
    expect(supportAt(segs, 50, 300)).toBeNull();
    expect(surfaceBelow(segs, 150, 100)?.windowId).toBe(7);
    expect(surfaceBelow(segs, 150, 400)?.windowId).toBeNull();
  });
});
