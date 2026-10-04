// The duck as a platformer hero: run, jump, and flap (a duck's jetpack) on
// destructible terrain. Moves one pixel at a time so it never tunnels through.

import type { Terrain } from "./terrain";

export interface Controls {
  left: boolean;
  right: boolean;
  /** Held: flap while airborne. */
  jump: boolean;
  /** Pressed this frame: jump if standing. */
  jumpPressed: boolean;
}

export const GRAVITY = 1700;
const MAX_FALL = 900;
const RUN = 270;
const ACCEL_GROUND = 2600;
const ACCEL_AIR = 1400;
const JUMP = 560;
const FLAP = 2700;
const FLAP_MAX_UP = 380;
const FUEL_BURN = 0.55;
const FUEL_REGEN = 0.5;
const STEP_UP = 8;

export class Player {
  vx = 0;
  vy = 0;
  onGround = false;
  /** Flapping stamina, 0..1. Refills on the ground. */
  fuel = 1;
  flapping = false;
  facing: 1 | -1 = 1;
  /** Distance walked, for the walk animation. */
  walked = 0;

  constructor(
    public x: number,
    public y: number,
    readonly w: number,
    readonly h: number,
  ) {}

  get cx(): number {
    return this.x + this.w / 2;
  }
  get cy(): number {
    return this.y + this.h / 2;
  }

  update(dt: number, c: Controls, t: Terrain): void {
    const want = (c.right ? 1 : 0) - (c.left ? 1 : 0);
    const accel = (this.onGround ? ACCEL_GROUND : ACCEL_AIR) * dt;
    const target = want * RUN;
    this.vx = this.vx < target ? Math.min(target, this.vx + accel) : Math.max(target, this.vx - accel);

    if (c.jumpPressed && this.onGround) {
      this.vy = -JUMP;
      this.onGround = false;
    }
    this.flapping = !this.onGround && c.jump && this.fuel > 0 && this.vy > -FLAP_MAX_UP;
    if (this.flapping) {
      this.vy = Math.max(-FLAP_MAX_UP, this.vy - FLAP * dt);
      this.fuel = Math.max(0, this.fuel - FUEL_BURN * dt);
    }
    this.vy = Math.min(MAX_FALL, this.vy + GRAVITY * dt);

    const before = this.x;
    this.moveX(this.vx * dt, t);
    this.moveY(this.vy * dt, t);
    this.walked += Math.abs(this.x - before);
    if (this.y < 0) {
      this.y = 0;
      this.vy = Math.max(0, this.vy);
    }
    this.onGround = t.hitsBox(this.x, this.y + 1, this.w, this.h);
    if (this.onGround) this.fuel = Math.min(1, this.fuel + FUEL_REGEN * dt);
  }

  /** Pushes the duck away from a blast. */
  knock(fromX: number, fromY: number, strength: number): void {
    const dx = this.cx - fromX;
    const dy = this.cy - fromY;
    const d = Math.max(20, Math.hypot(dx, dy));
    this.vx += (dx / d) * strength;
    this.vy += (dy / d) * strength - strength * 0.3;
  }

  private moveX(dx: number, t: Terrain): void {
    const s = Math.sign(dx);
    for (let rem = Math.abs(dx); rem > 0; rem -= 1) {
      const d = Math.min(1, rem) * s;
      if (!t.hitsBox(this.x + d, this.y, this.w, this.h)) {
        this.x += d;
        continue;
      }
      // Walk up small ledges (debris edges, uneven holes) instead of stopping dead.
      let up = 1;
      while (up <= STEP_UP && t.hitsBox(this.x + d, this.y - up, this.w, this.h)) up++;
      if (this.onGround && up <= STEP_UP) {
        this.y -= up;
        this.x += d;
      } else {
        this.vx = 0;
        return;
      }
    }
  }

  private moveY(dy: number, t: Terrain): void {
    const s = Math.sign(dy);
    for (let rem = Math.abs(dy); rem > 0; rem -= 1) {
      const d = Math.min(1, rem) * s;
      if (t.hitsBox(this.x, this.y + d, this.w, this.h)) {
        this.vy = 0;
        return;
      }
      this.y += d;
    }
  }
}
