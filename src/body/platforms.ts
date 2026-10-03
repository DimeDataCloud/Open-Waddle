// Turns window rectangles into the surfaces Waddle can stand on: the visible
// parts of each window's top edge, plus the floor (bottom of the work area).

/** A window in overlay-local logical pixels. Lists are ordered front to back. */
export interface WinRect {
  id: number;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** A horizontal walkable span. `windowId` is null for the floor. */
export interface Segment {
  x1: number;
  x2: number;
  y: number;
  windowId: number | null;
}

type Span = [number, number];

function subtract(spans: Span[], cut: Span): Span[] {
  const out: Span[] = [];
  for (const [a, b] of spans) {
    if (cut[1] <= a || cut[0] >= b) {
      out.push([a, b]);
      continue;
    }
    if (cut[0] > a) out.push([a, cut[0]]);
    if (cut[1] < b) out.push([cut[1], b]);
  }
  return out;
}

export interface SegmentOptions {
  width: number;
  floorY: number;
  /** Headroom needed above an edge for the duck to stand there. */
  minHeadroom: number;
  /** Ignore slivers narrower than this. */
  minWidth: number;
}

export function computeSegments(windows: WinRect[], opts: SegmentOptions): Segment[] {
  const segments: Segment[] = [{ x1: 0, x2: opts.width, y: opts.floorY, windowId: null }];
  windows.forEach((win, i) => {
    const y = win.y;
    if (y < opts.minHeadroom || y >= opts.floorY - 4) return;
    let spans: Span[] = [[Math.max(0, win.x), Math.min(opts.width, win.x + win.w)]];
    // Windows in front that cover this edge (or the space just above it) hide part of it.
    for (const front of windows.slice(0, i)) {
      if (front.y <= y && front.y + front.h >= y - opts.minHeadroom / 2) {
        spans = subtract(spans, [front.x, front.x + front.w]);
      }
    }
    for (const [x1, x2] of spans) {
      if (x2 - x1 >= opts.minWidth) segments.push({ x1, x2, y, windowId: win.id });
    }
  });
  return segments;
}

/** The surface directly under a point, if the point is standing on one. */
export function supportAt(segments: Segment[], x: number, y: number, tolerance = 2): Segment | null {
  let best: Segment | null = null;
  for (const s of segments) {
    if (x >= s.x1 && x <= s.x2 && Math.abs(s.y - y) <= tolerance) {
      if (!best || s.y < best.y) best = s;
    }
  }
  return best;
}

/** The first surface at or below `y` at column `x` (where something dropped there would land). */
export function surfaceBelow(segments: Segment[], x: number, y: number): Segment | null {
  let best: Segment | null = null;
  for (const s of segments) {
    if (x >= s.x1 && x <= s.x2 && s.y >= y - 0.5 && (!best || s.y < best.y)) best = s;
  }
  return best;
}
