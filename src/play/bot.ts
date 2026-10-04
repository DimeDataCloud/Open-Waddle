// Autoplay: the duck wrecks the screen by itself. It picks something solid
// (inside the target window when the AI named one), gets within range by
// walking, jumping and flapping, and fires, switching weapons now and then.

import type { Controls, Player } from "./player";
import type { Rect, Terrain } from "./terrain";
import { WEAPONS } from "./weapons";

export interface BotMove {
  controls: Controls;
  aimX: number;
  aimY: number;
  fire: boolean;
  grenade: boolean;
  /** Switch to this weapon index, if set. */
  weapon: number | null;
}

const RANGE = 520;

export class Bot {
  private target: { x: number; y: number } | null = null;
  private retarget = 0;
  private switchIn = 1.5;
  private stuck = 0;

  constructor(
    private rand: () => number,
    private area: Rect | null,
  ) {}

  think(dt: number, p: Player, t: Terrain): BotMove {
    this.retarget -= dt;
    this.switchIn -= dt;
    if (!this.target || this.retarget <= 0 || !t.solidAt(this.target.x, this.target.y)) this.pick(p, t);
    const controls: Controls = { left: false, right: false, jump: false, jumpPressed: false };
    const move: BotMove = { controls, aimX: p.cx + p.facing * 100, aimY: p.cy, fire: false, grenade: false, weapon: null };
    if (this.switchIn <= 0) {
      this.switchIn = 3 + this.rand() * 4;
      move.weapon = Math.floor(this.rand() * WEAPONS.length);
    }
    const tgt = this.target;
    if (!tgt) return move;

    const dx = tgt.x - p.cx;
    const dy = tgt.y - p.cy;
    const dist = Math.hypot(dx, dy);
    if (dist > RANGE * 0.8 || Math.abs(dx) > RANGE * 0.7) {
      controls.right = dx > 0;
      controls.left = dx < 0;
    }
    // Blocked by a wall while walking: jump it, then flap over.
    const blocked = (controls.left || controls.right) && Math.abs(p.vx) < 5;
    this.stuck = blocked ? this.stuck + dt : 0;
    if (this.stuck > 0.15 && p.onGround) controls.jumpPressed = true;
    if ((this.stuck > 0.15 || dy < -160) && p.fuel > 0.25) controls.jump = true;
    if (dy < -160 && p.onGround) controls.jumpPressed = true;

    move.aimX = tgt.x + (this.rand() - 0.5) * 30;
    move.aimY = tgt.y + (this.rand() - 0.5) * 30;
    move.fire = dist < RANGE * 1.4;
    move.grenade = this.rand() < dt * 0.25;
    return move;
  }

  private pick(p: Player, t: Terrain): void {
    this.retarget = 2.5 + this.rand() * 2.5;
    // Of a few random solid spots, prefer one that's close: less travelling, more blasting.
    let best: { x: number; y: number } | null = null;
    let bestD = Infinity;
    for (let i = 0; i < 6; i++) {
      const c = t.randomSolid(this.rand, this.area ?? undefined) ?? (this.area ? t.randomSolid(this.rand) : null);
      if (!c) continue;
      const d = Math.hypot(c.x - p.cx, c.y - p.cy) * (0.6 + this.rand() * 0.8);
      if (d < bestD) {
        bestD = d;
        best = c;
      }
    }
    this.target = best;
  }
}
