// Tier-1 tests (AGENTS.md §9) for `view_state.ts`'s per-player motion
// derivation. No display, no GL: `viewState.update` is distance/dt arithmetic
// over a plain module-level map.

import { describe, expect, it } from "vitest";
import { viewState, type ViewStatePlayer } from "./view_state.ts";

function playerAt(x: number, y = 0): readonly ViewStatePlayer[] {
  return [{ id: "p1", pos: { x, y } }];
}

describe("view_state.viewState locomotion blend", () => {
  // Regression for the bug class this retune fixed: the previous
  // WALK_SPEED/RUN_SPEED pair (150/400) sat above every speed gc-sim can
  // actually produce (a full sprint tops out around 297-351 u/s), so the
  // run stride never saturated and a sprinting player permanently read as a
  // brisk walk. 300 u/s is inside that reachable envelope.
  it("saturates the run stride at a sprint speed inside the sim's reachable envelope", () => {
    viewState.reset();
    const dt = 1 / 60;
    const speed = 300;
    expect(speed).toBeGreaterThan(viewState.RUN_SPEED);

    viewState.update(playerAt(0), dt);
    viewState.update(playerAt(speed * dt), dt);

    const gait = viewState.get("p1")?.gait ?? -1;
    // Above RUN_SPEED, runMix clamps to 1, so the stride driving this frame's
    // gait increment is exactly RUN_STRIDE -- not a value interpolated toward
    // it, and not the old, unreachable RUN_STRIDE either.
    const expected = ((speed * dt) / viewState.RUN_STRIDE) % 1;
    expect(gait).toBeCloseTo(expected, 9);
  });

  it("blends toward the walk stride at the walk threshold and holds it below there", () => {
    viewState.reset();
    const dt = 1 / 60;

    viewState.update(playerAt(0), dt);
    viewState.update(playerAt(viewState.WALK_SPEED * dt), dt);
    const gaitAtWalk = viewState.get("p1")?.gait ?? -1;
    expect(gaitAtWalk).toBeCloseTo((viewState.WALK_SPEED * dt) / viewState.WALK_STRIDE, 9);
  });
});
