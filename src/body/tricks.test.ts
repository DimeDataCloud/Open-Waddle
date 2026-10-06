import { describe, expect, it } from "vitest";
import { planTrick, TRICKS, type TrickStep } from "./tricks";

const screen = { w: 1440, h: 900 };
const size = { w: 48, h: 48 };
const at = { x: 700, y: 880 };

const flights = (steps: TrickStep[]) => steps.filter((s): s is Extract<TrickStep, { kind: "fly" }> => s.kind === "fly");

describe("planTrick", () => {
  it("keeps every flight on screen, with room for the duck", () => {
    for (const t of TRICKS) {
      for (const cursor of [null, { x: -50, y: 2000 }, { x: 1430, y: 5 }]) {
        for (const f of flights(planTrick(t, screen, size, at, cursor, () => 0.999))) {
          expect(f.x).toBeGreaterThanOrEqual(size.w / 2);
          expect(f.x).toBeLessThanOrEqual(screen.w - size.w / 2);
          expect(f.y).toBeGreaterThanOrEqual(size.h);
          expect(f.y).toBeLessThanOrEqual(screen.h);
        }
      }
    }
  });

  it("flies a loop round the screen and comes back", () => {
    const steps = planTrick("fly_around", screen, size, at, null);
    const f = flights(steps);
    expect(f.length).toBeGreaterThanOrEqual(6);
    expect(Math.min(...f.map((p) => p.x))).toBeLessThan(screen.w * 0.3);
    expect(Math.max(...f.map((p) => p.x))).toBeGreaterThan(screen.w * 0.7);
    expect(f[f.length - 1]).toMatchObject({ x: at.x, y: Math.min(at.y, screen.h - 8) });
  });

  it("comes over beside the pointer and faces it", () => {
    const steps = planTrick("come_here", screen, size, at, { x: 1200, y: 400 });
    const [f] = flights(steps);
    expect(f.x).toBeLessThan(1200);
    expect(Math.abs(f.x - 1200)).toBeLessThan(100);
    expect(steps).toContainEqual({ kind: "face", dir: 1 });
  });

  it("makes a harmless mess: pecks with dust in several places", () => {
    const steps = planTrick("mess", screen, size, at, null, () => 0.5);
    expect(steps.filter((s) => s.kind === "act" && s.fx === "dust")).toHaveLength(6);
  });

  it("naps and wakes without moving", () => {
    expect(planTrick("nap", screen, size, at, null)).toEqual([{ kind: "sleep" }]);
    expect(planTrick("wake_up", screen, size, at, null)[0]).toEqual({ kind: "wake" });
  });
});
