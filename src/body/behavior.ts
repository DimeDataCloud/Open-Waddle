// The duck's body in motion: follows planned routes leg by leg, hovers while
// acting mid-air, falls under gravity, and picks the animation frame.

import { planPath, type MoveKind, type Waypoint } from "./pathfind";
import { fall, newBody, ride, type BodyState } from "./physics";
import { supportAt, type Segment } from "./platforms";
import type { FrameName } from "./sprites";

export type Mode = "idle" | "moving" | "falling" | "dragged" | "hovering";
export type ActKind = "peck" | "type" | "look";

const WALK_SPEED = 110; // logical px/s

/** Tools that work through an API, not the screen: the duck pecks at a little laptop while they run. */
export function usesLaptop(tool: string): boolean {
  return /^(mail_|calendar_|drive_|browser_read)/.test(tool) || ["contacts_find", "find_files", "read_document"].includes(tool);
}
const FLY_SPEED = 280;

interface Leg {
  fromX: number;
  fromY: number;
  to: Waypoint;
  t: number;
  dur: number;
}

export interface MoveOptions {
  /** Speed up so the whole trip takes at most this long. */
  maxSeconds?: number;
  facing?: 1 | -1;
  onArrive?: () => void;
}

function legDuration(fromX: number, fromY: number, to: Waypoint): number {
  const d = Math.hypot(to.x - fromX, to.y - fromY);
  switch (to.kind) {
    case "walk":
      return Math.max(0.05, Math.abs(to.x - fromX) / WALK_SPEED);
    case "jump":
      return Math.max(0.3, d / 320);
    case "drop":
      return Math.max(0.2, Math.sqrt((2 * Math.max(0, to.y - fromY)) / 2200) + 0.1);
    case "fly":
      return Math.max(0.2, d / FLY_SPEED);
  }
}

export class Duck {
  body: BodyState;
  facing: 1 | -1 = 1;
  mode: Mode = "falling";
  sleeping = false;
  /** Working at the laptop (API tools); ends with the next on-screen action or the task. */
  laptop = false;
  private legs: Waypoint[] = [];
  private leg: Leg | null = null;
  private speed = 1;
  private onArrive: (() => void) | null = null;
  private arriveFacing: 1 | -1 | null = null;
  private act: { kind: ActKind; until: number } | null = null;
  private blinkUntil = 0;
  private nextBlink = 0;
  lastActive = 0;

  constructor(readonly size: { w: number; h: number }, x: number, y: number) {
    this.body = newBody(x, y);
  }

  get rect() {
    return { x: this.body.x - this.size.w / 2, y: this.body.y - this.size.h, w: this.size.w, h: this.size.h };
  }

  get currentMove(): MoveKind | null {
    return this.leg?.to.kind ?? null;
  }

  moveTo(goal: { x: number; y: number }, segments: Segment[], opts: MoveOptions = {}): void {
    this.cancelMove();
    const from = { x: this.body.x, y: this.body.y };
    const grounded = this.mode === "idle" && this.body.grounded;
    const path = grounded ? planPath(segments, from, goal) : [{ x: goal.x, y: goal.y, kind: "fly" as const }];
    let total = 0;
    let p = from;
    for (const w of path) {
      total += legDuration(p.x, p.y, w);
      p = w;
    }
    this.speed = opts.maxSeconds && total > opts.maxSeconds ? total / opts.maxSeconds : 1;
    this.legs = path;
    this.onArrive = opts.onArrive ?? null;
    this.arriveFacing = opts.facing ?? null;
    this.sleeping = false;
    this.mode = "moving";
    this.body.grounded = false;
    this.nextLeg();
  }

  cancelMove(): void {
    const cb = this.onArrive;
    this.legs = [];
    this.leg = null;
    this.onArrive = null;
    if (this.mode === "moving") this.mode = "falling";
    cb?.();
  }

  private nextLeg(): void {
    const to = this.legs.shift();
    if (!to) {
      this.leg = null;
      this.arrive();
      return;
    }
    const dx = to.x - this.body.x;
    if (Math.abs(dx) > 1) this.facing = dx > 0 ? 1 : -1;
    this.leg = { fromX: this.body.x, fromY: this.body.y, to, t: 0, dur: legDuration(this.body.x, this.body.y, to) / this.speed };
  }

  private arrive(): void {
    if (this.arriveFacing) this.facing = this.arriveFacing;
    this.mode = "hovering";
    this.body.vy = 0;
    const cb = this.onArrive;
    this.onArrive = null;
    cb?.();
  }

  /** After acting mid-air, flutter down onto whatever is below. */
  land(segments: Segment[]): void {
    if (this.mode !== "hovering") return;
    const s = supportAt(segments, this.body.x, this.body.y, 3);
    if (s) {
      this.mode = "idle";
      this.body.grounded = true;
      this.body.supportId = s.windowId;
      this.body.y = s.y;
    } else {
      this.mode = "falling";
      this.body.grounded = false;
    }
  }

  doAct(kind: ActKind, now: number, ms = 600): void {
    this.act = { kind, until: now + ms };
    this.lastActive = now;
    this.sleeping = false;
  }

  grab(): void {
    this.cancelMove();
    this.mode = "dragged";
    this.body.grounded = false;
    this.sleeping = false;
  }

  dragTo(x: number, y: number): void {
    if (this.mode !== "dragged") return;
    if (Math.abs(x - this.body.x) > 1) this.facing = x > this.body.x ? 1 : -1;
    this.body.x = x;
    this.body.y = y;
  }

  /**
   * After the screen changed size (rotated, docked, new scale): back inside it,
   * falling onto whatever is below. A duck left below the new floor would fall forever.
   */
  keepOnScreen(screen: { w: number; h: number }): void {
    if (this.mode === "dragged") return;
    const b = this.body;
    const x = Math.max(this.size.w / 2, Math.min(screen.w - this.size.w / 2, b.x));
    const y = Math.max(this.size.h, Math.min(screen.h, b.y));
    if (x === b.x && y === b.y) return;
    this.cancelMove();
    b.x = x;
    b.y = y;
    b.vy = 0;
    b.grounded = false;
    b.supportId = null;
    this.mode = "falling";
  }

  /** Arrives on a new monitor: in from that side, high up, then down onto whatever is below. */
  enterFrom(side: "left" | "right", screen: { w: number; h: number }): void {
    if (this.mode === "dragged") return;
    this.cancelMove();
    const b = this.body;
    b.x = side === "left" ? this.size.w / 2 : screen.w - this.size.w / 2;
    b.y = Math.max(this.size.h, screen.h * 0.25);
    b.vy = 0;
    b.grounded = false;
    b.supportId = null;
    this.facing = side === "left" ? 1 : -1;
    this.mode = "falling";
  }

  release(): void {
    if (this.mode !== "dragged") return;
    this.mode = "falling";
    this.body.vy = 0;
  }

  update(dt: number, segments: Segment[], moved: Map<number, [number, number]>): void {
    switch (this.mode) {
      case "idle":
        ride(this.body, segments, moved);
        if (!this.body.grounded) this.mode = "falling";
        break;
      case "falling":
        if (fall(this.body, segments, dt)) this.mode = "idle";
        break;
      case "moving": {
        const leg = this.leg;
        if (!leg) break;
        leg.t += dt;
        const p = Math.min(1, leg.t / leg.dur);
        const { to } = leg;
        this.body.x = leg.fromX + (to.x - leg.fromX) * p;
        if (to.kind === "jump") {
          const height = 24 + Math.max(0, leg.fromY - to.y) * 0.6;
          this.body.y = leg.fromY + (to.y - leg.fromY) * p - height * 4 * p * (1 - p);
        } else if (to.kind === "drop") {
          this.body.y = leg.fromY + (to.y - leg.fromY) * p * p;
        } else {
          this.body.y = leg.fromY + (to.y - leg.fromY) * p;
        }
        if (p >= 1) {
          this.body.x = to.x;
          this.body.y = to.y;
          this.nextLeg();
          // Finished on a surface: stand on it.
          // nextLeg() may have arrived, which switches the mode.
          if ((this.mode as Mode) === "hovering" && to.kind !== "fly") this.land(segments);
        }
        break;
      }
      case "hovering":
      case "dragged":
        break;
    }
  }

  /** True while something is animating, so the render loop runs at full rate. */
  isAnimating(now: number): boolean {
    return this.mode !== "idle" || this.laptop || (this.act !== null && now < this.act.until) || now < this.blinkUntil;
  }

  frame(now: number): FrameName {
    const tick = (ms: number) => Math.floor(now / ms) % 2 === 0;
    if (this.act && now < this.act.until) {
      if (this.act.kind === "look") return tick(300) ? "blink" : "idle";
      return tick(this.act.kind === "type" ? 90 : 130) ? "peck" : "idle";
    }
    switch (this.mode) {
      case "moving": {
        const kind = this.leg?.to.kind;
        if (kind === "walk") return tick(Math.max(60, 130 / this.speed)) ? "walk1" : "walk2";
        if (kind === "fly") return tick(90) ? "flap_up" : "flap_down";
        return "flap_up";
      }
      case "hovering":
        return tick(100) ? "flap_up" : "flap_down";
      case "falling":
        return "flap_up";
      case "dragged":
        return tick(220) ? "flap_down" : "flap_up";
      case "idle":
        if (this.laptop) return tick(110) ? "peck" : "idle";
        if (this.sleeping) return "blink";
        if (now >= this.nextBlink) {
          this.blinkUntil = now + 140;
          this.nextBlink = now + 2500 + Math.random() * 4000;
        }
        return now < this.blinkUntil ? "blink" : "idle";
    }
  }
}

/** A random standing spot, favouring wide surfaces. */
export function pickWanderTarget(segments: Segment[], margin: number, rand = Math.random): { x: number; y: number } | null {
  const usable = segments.filter((s) => s.x2 - s.x1 > margin * 2);
  const total = usable.reduce((n, s) => n + (s.x2 - s.x1), 0);
  if (!total) return null;
  let r = rand() * total;
  for (const s of usable) {
    const w = s.x2 - s.x1;
    if (r <= w) return { x: s.x1 + margin + rand() * (w - margin * 2), y: s.y };
    r -= w;
  }
  return null;
}
