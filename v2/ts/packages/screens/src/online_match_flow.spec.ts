// Ported from spec/screens/online_match_flow_spec.lua.
//
// Unblocker: every case mounts real `OnlineLobby`/`OnlineMatch` screens over
// an in-process fake star transport, completes the real manual
// offer/answer handshake, and drives a real `game.online.match_driver` +
// `sim.match` to full time and an acknowledged result -- all Rust-owned
// (`crates/gc-sim`/`crates/gc-netcode`; v2/README.md §2.1) with no wasm
// bridge this milestone, plus `game.render.pitch`/`match_hud_render`/
// `render.frame` (`@gc/render`) and `game.input.bindings` (`@gc/input`),
// neither a declared dependency of this package. `online_match.ts` and
// `lobby.ts`/`online_lobby.ts` are ported (see their headers) with every
// one of those dependencies as an injected port, but none of the fakes
// this package can build without reimplementing rollback scheduling and
// combat resimulation -- exactly the boundary v2/README.md §2.1 draws
// around this module -- so every case below is ported as `it.skip`,
// matching the precedent in `@gc/online`'s `match_presentation.spec.ts`.

import { describe, it } from "vitest";

describe.skip("online match screen flow [needs wasm-compiled gc-sim + gc-netcode + @gc/render + @gc/input]", () => {
  it.skip("carries a 1v1 session from the lobby to an agreed result", () => {});
  it.skip("keeps control inside the frozen owned set and off both keepers", () => {});
  it.skip("makes switching inert in 4v4 without branching on the mode", () => {});
  it.skip("keeps simulating through focus loss, a lost controller, and a pause request", () => {});
  it.skip("aborts deliberately and ends the session for every peer", () => {});
  it.skip("shows the controlled player, its loadout, and the network state", () => {});
});

describe.skip("online combat families [needs wasm-compiled gc-sim + gc-netcode + @gc/render + @gc/input]", () => {
  it.skip("drives and shows every accepted family from local keyboard and gamepad", () => {});
});

describe.skip("online match app routing [needs wasm-compiled gc-sim + gc-netcode + @gc/app's screen stack]", () => {
  it.skip("routes the lobby's synchronized start into the online match", () => {});
});

describe.skip("online match renderer smoke [needs wasm-compiled gc-sim + gc-netcode + @gc/render]", () => {
  it.skip("draws a live online frame with its combat model and HUD", () => {});
});
