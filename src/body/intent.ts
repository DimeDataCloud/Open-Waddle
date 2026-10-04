// Turns the ambient decision (from the backend's quick decision model) into
// where the duck goes next. Pure, so it can be tested without a screen.

import type { Segment, WinRect } from "./platforms";

export type Intent = "perch" | "explore" | "watch" | "nap" | "give_space" | "wander";

export interface Plan {
  /** Standing spots to visit in order. */
  stops: { x: number; y: number }[];
  /** Pause at each stop before the next one (and after the last, if `stay`). */
  pauseMs: number;
  /** Peck/look at each stop, as if inspecting it. */
  inspect: boolean;
  /** Stay put after the last stop instead of going back to wandering. */
  stay: boolean;
  sleep: boolean;
  /** Face this x on arrival (e.g. the cursor). */
  faceX?: number;
}

const near = (a: number, b: number, d = 4) => Math.abs(a - b) <= d;

/** Walkable spans along a window's top edge (the parts no other window covers). */
function topOf(win: WinRect, segments: Segment[], margin: number): Segment[] {
  return segments.filter((s) => s.windowId === win.id && near(s.y, win.y) && s.x2 - s.x1 > margin * 2);
}

export function planIntent(
  intent: Intent,
  win: WinRect | null,
  segments: Segment[],
  margin: number,
  cursor: { x: number; y: number } | null,
  duckX: number,
  rand = Math.random,
): Plan | null {
  const base: Plan = { stops: [], pauseMs: 0, inspect: false, stay: false, sleep: false };
  switch (intent) {
    case "nap":
      return { ...base, sleep: true, stay: true };
    case "perch": {
      const tops = win ? topOf(win, segments, margin) : [];
      if (!tops.length) return null;
      const s = tops.reduce((a, b) => (b.x2 - b.x1 > a.x2 - a.x1 ? b : a));
      const x = s.x1 + margin + rand() * (s.x2 - s.x1 - margin * 2);
      return { ...base, stops: [{ x, y: s.y }], pauseMs: 30_000, stay: true };
    }
    case "explore": {
      const tops = win ? topOf(win, segments, margin) : [];
      if (!tops.length) return null;
      const pts = tops.flatMap((s) => [s.x1 + margin, (s.x1 + s.x2) / 2, s.x2 - margin].map((x) => ({ x, y: s.y })));
      // Start from the end nearest the duck so it doesn't cross the window twice.
      pts.sort((a, b) => a.x - b.x);
      if (Math.abs(pts[pts.length - 1].x - duckX) < Math.abs(pts[0].x - duckX)) pts.reverse();
      return { ...base, stops: pts, pauseMs: 2500, inspect: true };
    }
    case "watch": {
      if (!cursor) return null;
      // The surface closest under the cursor, standing a polite distance to one side.
      const below = segments.filter((s) => s.y >= cursor.y - 40 && s.x2 - s.x1 > margin * 2);
      if (!below.length) return null;
      const s = below.reduce((a, b) => (b.y - cursor.y < a.y - cursor.y ? b : a));
      const side = duckX < cursor.x ? -1 : 1;
      const x = Math.min(s.x2 - margin, Math.max(s.x1 + margin, cursor.x + side * 120));
      return { ...base, stops: [{ x, y: s.y }], pauseMs: 15_000, stay: true, faceX: cursor.x };
    }
    case "give_space": {
      const floor = segments.filter((s) => s.windowId === null).sort((a, b) => b.y - a.y)[0];
      if (!floor) return null;
      const centre = win ? win.x + win.w / 2 : (floor.x1 + floor.x2) / 2;
      const x = centre > (floor.x1 + floor.x2) / 2 ? floor.x1 + margin : floor.x2 - margin;
      return { ...base, stops: [{ x, y: floor.y }], pauseMs: 60_000, stay: true };
    }
    case "wander":
      return null;
  }
}
