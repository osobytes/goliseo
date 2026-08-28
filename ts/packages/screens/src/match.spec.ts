// Tier-2 tests for `stepMatchControlLatches`'s first-touch tap buffer
// (#623). The buffer exists because play-test telemetry showed the natural
// first-touch gesture is a TAP that lifts 50-150 ms before the ball's
// arrival tick; these cases pin the buffer's length, its cancellation on
// gaining the ball, and that it arms ONLY the strike -- jockey and the
// release poke keep their live-hold/edge semantics.

import { describe, expect, it } from "vitest";

import {
  STRIKE_TAP_BUFFER_FRAMES,
  newMatchControlLatches,
  stepMatchControlLatches,
} from "./match.ts";

const UP = { actionDown: false, playDown: false, modifierDown: false };
const DOWN = { actionDown: true, playDown: false, modifierDown: false };

describe("the first-touch tap buffer", () => {
  it("keeps the strike armed for the buffer window after an off-ball release", () => {
    const latches = newMatchControlLatches();
    stepMatchControlLatches(latches, false, DOWN);
    const release = stepMatchControlLatches(latches, false, UP);
    expect(release.dashEdge, "the release poke edge still fires").toBe(true);
    expect(release.contextual.aerialStrike, "the strike stays armed on the release frame").toBe(
      true,
    );
    for (let frame = 1; frame < STRIKE_TAP_BUFFER_FRAMES; frame += 1) {
      const step = stepMatchControlLatches(latches, false, UP);
      expect(step.contextual.aerialStrike, `still armed ${frame} frames after release`).toBe(true);
      expect(step.dashEdge, "the poke edge fires once, not per buffered frame").toBe(false);
    }
    const expired = stepMatchControlLatches(latches, false, UP);
    expect(expired.contextual.aerialStrike, "the buffer expires").toBe(false);
  });

  it("cancels the buffer the moment the player carries the ball", () => {
    const latches = newMatchControlLatches();
    stepMatchControlLatches(latches, false, DOWN);
    stepMatchControlLatches(latches, false, UP);
    const carrying = stepMatchControlLatches(latches, true, UP);
    expect(carrying.contextual.aerialStrike).toBe(false);
    const after = stepMatchControlLatches(latches, false, UP);
    expect(after.contextual.aerialStrike, "gaining the ball spent the buffer for good").toBe(false);
  });

  it("arms only the strike: jockey follows the live hold", () => {
    const latches = newMatchControlLatches();
    stepMatchControlLatches(latches, false, DOWN);
    const held = stepMatchControlLatches(latches, false, DOWN);
    expect(held.contextual.jockey).toBe(true);
    const released = stepMatchControlLatches(latches, false, UP);
    expect(released.contextual.jockey, "the shadow stance releases instantly").toBe(false);
    expect(released.contextual.aerialStrike, "while the strike stays armed").toBe(true);
  });
});
