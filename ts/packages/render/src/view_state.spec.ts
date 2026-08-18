// Tier-1 tests (AGENTS.md §9) for `view_state.ts`'s per-player motion
// derivation. No display, no GL: `viewState.update` is distance/dt arithmetic
// over a plain module-level map.

import { describe, expect, it } from "vitest";
import { viewState, type ViewStatePlayer } from "./view_state.ts";

function playerAt(x: number, y = 0): readonly ViewStatePlayer[] {
  return [{ id: "p1", pos: { x, y } }];
}

describe("view_state.viewState responsiveness", () => {
  // #574. What players report as "slow animation" is usually RESPONSIVENESS:
  // the lag between the simulation doing something and the animation showing
  // it. The sim acts on an input within one tick (16.7 ms); everything after
  // that is render-side filtering, and this smoothing filter was the biggest
  // single contributor. Pinned as a measured settling time rather than as the
  // gain constant, so the claim survives someone rewriting the filter.
  //
  // Budget: a committed action's onset should be visible inside ~100 ms, and
  // the blend should be substantially there well before the 200 ms this
  // asserts. The old `dt * 8` took ~290 ms to reach 90%.
  it("reaches 90% of a step change in speed inside 200 ms", () => {
    viewState.reset();
    const dt = 1 / 60;
    const speed = viewState.RUN_SPEED;

    let x = 0;
    viewState.update(playerAt(x), dt);
    let elapsed = 0;
    for (let frame = 0; frame < 60; frame += 1) {
      x += speed * dt;
      viewState.update(playerAt(x), dt);
      elapsed += dt;
      if ((viewState.get("p1")?.speed ?? 0) >= 0.9 * speed) {
        break;
      }
    }
    expect(viewState.get("p1")?.speed ?? 0).toBeGreaterThanOrEqual(0.9 * speed);
    expect(elapsed, "seconds for the locomotion blend to catch up").toBeLessThan(0.2);
  });
});

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
