// Ported from spec/screens/match_screen_spec.lua.
//
// Unblocker: every case here drives a live `Match.new()` through
// `sim.match`'s real physics (tackle timers, pass charge, ball flight) via
// stubbed `love.keyboard`/`love.joystick` polling and a real fixed-clock
// render loop. `sim.match`, `sim.fixed_clock`, and `game.input.bindings`
// are Rust-owned or `@gc/input`-owned (v2/README.md §2.1; not a declared
// dependency of this package), and there is no browser input-capture layer
// in this milestone regardless (v2/README.md §1: "the glue that makes a
// playable browser build ... is a separate milestone"). `match.ts` ports
// only the rollback-consumption seam these specs do not touch at all -- see
// its header. Every `t.it` below is therefore ported as `it.skip`; none can
// be faked without reimplementing sim physics, which is exactly the line
// v2/README.md §2 draws around `sim/`.

import { describe, it } from "vitest";

describe.skip("match screen rematch (tier 2) [needs wasm-compiled gc-sim + a browser input-capture layer]", () => {
  it.skip("R restarts a finished match with the same pre-match choices", () => {});
  it.skip("rematches on every key bound to CONFIRM", () => {});
  it.skip("ignores the rematch keys while the match is live", () => {});
  it.skip("ignores match inputs after full time", () => {});
  it.skip("leaves rematch ownership to the result screen in product mode", () => {});
});

describe.skip("match screen fixed simulation clock (tier 2) [needs wasm-compiled gc-sim]", () => {
  it.skip("samples render frames but only steps the match at the canonical interval", () => {});
});

describe.skip("match screen contextual controls (tier 2) [needs wasm-compiled gc-sim + a browser input-capture layer]", () => {
  it.skip("constructs and drives combat only behind the explicit option", () => {});
  it.skip("K never switches while carrying the ball (it charges a pass)", () => {});
  it.skip("K switches player when not carrying", () => {});
  it.skip("Space hold = jockey, release = poke (off the ball)", () => {});
  it.skip("Space never produces a poke while carrying (it charges the shot)", () => {});
});

describe.skip("match screen lob latch (tier 2) [needs wasm-compiled gc-sim + a browser input-capture layer]", () => {
  it.skip("MODIFIER held during a charged pass lofts it even if it lifts early", () => {});
  it.skip("holding PLAY charges the pass range for an outfielder", () => {});
});

describe.skip("match screen goal replay (tier 2) [needs wasm-compiled gc-sim + @gc/render's replay wired as a dependency]", () => {
  it.skip("a goal freezes the sim into a slow-mo replay; skipping resumes", () => {});
});
