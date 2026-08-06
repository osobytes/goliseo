// Ported from spec/screens/match_gamepad_spec.lua.
//
// Unblocker: same as match_screen.spec.ts -- these drive a live
// `Match.new()` through `sim.match`'s real physics via a stubbed
// `love.joystick`, which needs both a wasm-compiled `gc-sim` and a browser
// gamepad-capture layer neither of which exist this milestone
// (v2/README.md §1, §2.1). `match.ts` ports only the rollback-consumption
// seam; see its header.

import { describe, it } from "vitest";

describe.skip("match screen gamepad input [needs wasm-compiled gc-sim + a browser gamepad-capture layer]", () => {
  it.skip("polls the left stick and contextual action button", () => {});
  it.skip("accepts abstract gamepad actions for off-ball switching", () => {});
  it.skip("requests an acrobatic strike from MODIFIER + ACTION off the ball", () => {});
});
