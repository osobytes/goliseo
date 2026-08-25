// The pure half of `browser_main.ts`'s "WITHOUT THIS EVERY CHARACTER STANDS
// STILL" block (AGENTS.md §9: split a renderer's own per-frame logic into a
// pure derivation and an impure caller, so the derivation is testable
// without a `<canvas>`). `browser_main.ts`'s `RenderPort.draw` is the impure
// half: it feeds this module a real rendered frame and pushes the result
// into `@gc/render`'s `viewState`/`cameraFollow` accumulators.
//
// Deliberately typed against a NARROW structural slice, not `@gc/render`'s
// full `RenderFrame` -- this derivation reads exactly `roster.ids` and
// `players.x`/`.y`, nothing else. A narrow input type is what lets
// `frame_view_players.spec.ts` exercise it against both the offline frame
// product (`sim_host.ts`'s `frameBuffer.toRenderFrame`-built `frame()`) and
// the online frame product (`online_ports.ts`'s `realMatchDriverPort.frame`,
// #611) without constructing either one's full shape.

/**
 * The slice of a rendered frame this derivation reads. Both of this
 * package's real frame products -- `sim_host.ts`'s offline `frame()` and
 * `online_ports.ts`'s online `realMatchDriverPort.frame()` -- satisfy this;
 * see #611, which made the two structurally identical (both now embed a
 * roster via `@gc/render`'s `frameBuffer.toRenderFrame`).
 */
export interface RosterPlayersFrame {
  readonly roster: { readonly ids: readonly string[] };
  readonly players: { readonly x: readonly number[]; readonly y: readonly number[] };
}

/**
 * One roster player's identity and live position -- the flat shape both
 * `@gc/render`'s `viewState.update` and `cameraFollow.update` read.
 */
export interface RosterPlayerView {
  readonly id: string;
  readonly pos: { readonly x: number; readonly y: number };
}

/**
 * Derive the flat `{id, pos}[]` roster-players view `viewState.update` and
 * `cameraFollow.update` need from one rendered frame -- pure: no I/O, no
 * renderer state, same inputs produce the same outputs.
 */
export function rosterPlayersView(frame: RosterPlayersFrame): readonly RosterPlayerView[] {
  return frame.roster.ids.map((id, index) => ({
    id,
    pos: { x: frame.players.x[index] ?? 0, y: frame.players.y[index] ?? 0 },
  }));
}
