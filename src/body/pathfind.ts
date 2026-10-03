// Grid path planning over the walkable surfaces. Standing positions are
// discretised into 8px columns on each surface. Moves: walk along a surface,
// step off an end and drop, or jump to another surface (jumping up costs much
// more and is capped at a maximum leap height). A shortest-path search runs
// from the duck's position; the last leg flies (ducks can) when the target
// isn't standing room on any surface.

import { supportAt, surfaceBelow, type Segment } from "./platforms";

export const CELL = 8;
const MAX_LEAP = 12; // cells (96px) of upward jump
const MAX_HOP = 6; // cells of horizontal reach per jump
const FLY_COST_PER_CELL = 3;
const ARRIVE_EPS = 12; // px: close enough to skip flying

export type MoveKind = "walk" | "jump" | "drop" | "fly";

export interface Waypoint {
  x: number;
  y: number;
  kind: MoveKind;
}

interface Node {
  seg: number;
  col: number;
}

const key = (n: Node) => n.seg * 100000 + n.col;

function cols(s: Segment): [number, number] {
  return [Math.ceil(s.x1 / CELL), Math.floor(s.x2 / CELL)];
}

class Heap {
  private items: [number, number][] = [];
  push(k: number, p: number) {
    const a = this.items;
    a.push([k, p]);
    let i = a.length - 1;
    while (i > 0) {
      const parent = (i - 1) >> 1;
      if (a[parent][1] <= a[i][1]) break;
      [a[parent], a[i]] = [a[i], a[parent]];
      i = parent;
    }
  }
  pop(): [number, number] | undefined {
    const a = this.items;
    if (!a.length) return undefined;
    const top = a[0];
    const last = a.pop()!;
    if (a.length) {
      a[0] = last;
      let i = 0;
      for (;;) {
        const l = 2 * i + 1;
        const r = l + 1;
        let m = i;
        if (l < a.length && a[l][1] < a[m][1]) m = l;
        if (r < a.length && a[r][1] < a[m][1]) m = r;
        if (m === i) break;
        [a[m], a[i]] = [a[i], a[m]];
        i = m;
      }
    }
    return top;
  }
  get size() {
    return this.items.length;
  }
}

function neighbours(segments: Segment[], n: Node): { to: Node; cost: number; kind: MoveKind }[] {
  const s = segments[n.seg];
  const [c0, c1] = cols(s);
  const out: { to: Node; cost: number; kind: MoveKind }[] = [];
  if (n.col > c0) out.push({ to: { seg: n.seg, col: n.col - 1 }, cost: 1, kind: "walk" });
  if (n.col < c1) out.push({ to: { seg: n.seg, col: n.col + 1 }, cost: 1, kind: "walk" });
  // Step off either end and drop to whatever is below.
  for (const [edge, dir] of [[c0, -1], [c1, 1]] as const) {
    if (n.col !== edge) continue;
    const col = n.col + dir;
    const below = surfaceBelow(segments, col * CELL, s.y + 1);
    if (below) {
      const t = segments.indexOf(below);
      const [t0, t1] = cols(below);
      if (col >= t0 && col <= t1) out.push({ to: { seg: t, col }, cost: 1 + ((below.y - s.y) / CELL) * 0.15, kind: "drop" });
    }
  }
  // Jumps to other surfaces within reach.
  segments.forEach((t, ti) => {
    if (ti === n.seg) return;
    const dy = (s.y - t.y) / CELL; // positive = up
    if (dy > MAX_LEAP) return;
    const [t0, t1] = cols(t);
    const lo = Math.max(t0, n.col - MAX_HOP);
    const hi = Math.min(t1, n.col + MAX_HOP);
    for (let col = lo; col <= hi; col++) {
      const dc = Math.abs(col - n.col);
      const cost = dy > 0 ? 4 + dy * 1.5 + dc * 0.5 : 2 + dc * 0.5 + -dy * 0.1;
      out.push({ to: { seg: ti, col }, cost, kind: "jump" });
    }
  });
  return out;
}

/**
 * Plans a route from a standing position to `goal` (feet position). Returns
 * waypoints after the start; the final one may be a `fly` leg.
 */
export function planPath(segments: Segment[], from: { x: number; y: number }, goal: { x: number; y: number }): Waypoint[] {
  const start = supportAt(segments, from.x, from.y, 3);
  if (!start) return [{ x: goal.x, y: goal.y, kind: "fly" }];
  const startSeg = segments.indexOf(start);
  const [s0, s1] = cols(start);
  const startNode: Node = { seg: startSeg, col: Math.max(s0, Math.min(s1, Math.round(from.x / CELL))) };

  const dist = new Map<number, number>([[key(startNode), 0]]);
  const prev = new Map<number, { from: number; kind: MoveKind }>();
  const nodes = new Map<number, Node>([[key(startNode), startNode]]);
  const heap = new Heap();
  heap.push(key(startNode), 0);
  let best = { k: key(startNode), total: Infinity };

  const flyCost = (n: Node) => {
    const x = n.col * CELL;
    const y = segments[n.seg].y;
    const d = Math.hypot(x - goal.x, y - goal.y);
    return d <= ARRIVE_EPS ? 0 : (d / CELL) * FLY_COST_PER_CELL;
  };

  let expanded = 0;
  while (heap.size && expanded < 50000) {
    const [k, d] = heap.pop()!;
    if (d > (dist.get(k) ?? Infinity)) continue;
    expanded++;
    const n = nodes.get(k)!;
    const total = d + flyCost(n);
    if (total < best.total) best = { k, total };
    if (d >= best.total) break; // nothing cheaper can follow
    for (const e of neighbours(segments, n)) {
      const nk = key(e.to);
      const nd = d + e.cost;
      if (nd < (dist.get(nk) ?? Infinity)) {
        dist.set(nk, nd);
        prev.set(nk, { from: k, kind: e.kind });
        nodes.set(nk, e.to);
        heap.push(nk, nd);
      }
    }
  }

  // Rebuild the route, merging consecutive walk steps on one surface.
  const steps: { node: Node; kind: MoveKind }[] = [];
  for (let k = best.k; prev.has(k); k = prev.get(k)!.from) steps.push({ node: nodes.get(k)!, kind: prev.get(k)!.kind });
  steps.reverse();
  const out: Waypoint[] = [];
  for (const { node, kind } of steps) {
    const wp = { x: node.col * CELL, y: segments[node.seg].y, kind };
    const last = out[out.length - 1];
    if (kind === "walk" && last?.kind === "walk" && last.y === wp.y) {
      last.x = wp.x;
    } else {
      out.push(wp);
    }
  }
  const endSeg = segments[nodes.get(best.k)!.seg];
  const end = out[out.length - 1] ?? { x: from.x, y: start.y };
  if (Math.hypot(end.x - goal.x, end.y - goal.y) > ARRIVE_EPS) {
    out.push({ x: goal.x, y: goal.y, kind: "fly" });
  } else if (Math.abs(endSeg.y - goal.y) <= 1 && goal.x >= endSeg.x1 && goal.x <= endSeg.x2 && end.x !== goal.x) {
    // Grid columns are 8px apart; finish on the exact spot.
    if (out.length && out[out.length - 1].kind === "walk") out[out.length - 1].x = goal.x;
    else out.push({ x: goal.x, y: endSeg.y, kind: "walk" });
  }
  return out;
}

/** Feet position that puts the duck just beside a target point, beak first. */
export function standBeside(target: { x: number; y: number }, duck: { w: number; h: number }, screenW: number): { x: number; y: number; facing: 1 | -1 } {
  const beakDrop = duck.h * 0.36; // beak height from the top of the sprite
  const facing: 1 | -1 = target.x - duck.w - 8 < 0 ? -1 : 1;
  const x = facing === 1 ? target.x - duck.w / 2 - 6 : target.x + duck.w / 2 + 6;
  return { x: Math.max(duck.w / 2, Math.min(screenW - duck.w / 2, x)), y: target.y + duck.h - beakDrop, facing };
}

export function pathLength(from: { x: number; y: number }, path: Waypoint[]): number {
  let len = 0;
  let p = from;
  for (const w of path) {
    len += Math.hypot(w.x - p.x, w.y - p.y);
    p = w;
  }
  return len;
}
