// Gravity, landing, and riding a window that moves underneath the duck.

import { supportAt, type Segment } from "./platforms";

export const GRAVITY = 2200; // logical px / s^2
const MAX_FALL = 1600;

export interface BodyState {
  /** Feet position: x is the centre, y the bottom edge. */
  x: number;
  y: number;
  vy: number;
  grounded: boolean;
  supportId: number | null;
}

export function newBody(x: number, y: number): BodyState {
  return { x, y, vy: 0, grounded: false, supportId: null };
}

/**
 * Keeps a grounded body attached to its surface. If the window it stands on
 * moved, the body moves with it; if the surface vanished, it starts falling.
 * `moved` maps window id → (dx, dy) since the last platform update.
 */
export function ride(body: BodyState, segments: Segment[], moved: Map<number, [number, number]>): void {
  if (!body.grounded) return;
  if (body.supportId !== null) {
    const d = moved.get(body.supportId);
    if (d) {
      body.x += d[0];
      body.y += d[1];
    }
  }
  const s = supportAt(segments, body.x, body.y, 3);
  if (s) {
    body.y = s.y;
    body.supportId = s.windowId;
  } else {
    body.grounded = false;
    body.supportId = null;
    body.vy = 0;
  }
}

/** Advances a falling body; lands on the first surface crossed. Returns true on landing. */
export function fall(body: BodyState, segments: Segment[], dt: number): boolean {
  if (body.grounded) return false;
  const prevY = body.y;
  body.vy = Math.min(MAX_FALL, body.vy + GRAVITY * dt);
  const nextY = body.y + body.vy * dt;
  let landing: Segment | null = null;
  for (const s of segments) {
    if (body.x >= s.x1 && body.x <= s.x2 && s.y >= prevY - 0.5 && s.y <= nextY && (!landing || s.y < landing.y)) {
      landing = s;
    }
  }
  if (landing) {
    body.y = landing.y;
    body.vy = 0;
    body.grounded = true;
    body.supportId = landing.windowId;
    return true;
  }
  body.y = nextY;
  return false;
}
