// Tier-2 UI-logic coverage (AGENTS.md §9) for `rosterPlayersView` -- the
// pure derivation extracted out of `browser_main.ts`'s `RenderPort.draw`
// "WITHOUT THIS EVERY CHARACTER STANDS STILL" block. Exercised against BOTH
// real frame shapes per #611's own test-gap report:
//
// - the OFFLINE product, built through this package's own `sim_host.ts`
//   (`createSimHost(...).frame()`) -- a real `@gc/wasm` session, decoded and
//   embedded with its roster via `@gc/render`'s `frameBuffer.toRenderFrame`,
//   exactly what `browser_sim_host.ts` hands `RenderPort.draw` in
//   production.
// - the ONLINE product's post-#611 contract, `realMatchDriverPort.frame()`'s
//   `{roster: {ids}, players: {x, y}}` shape (`online_ports.ts`) -- driving
//   a live one needs the full real coordinator/driver handshake, which is
//   heavier machinery already exercised, with real wasm on both peers, by
//   `online_ports.spec.ts`'s driver-seam frame-shape parity spec (also
//   #611). This case instead proves `rosterPlayersView` itself is agnostic
//   to which driver produced the frame, against a fixture typed to the
//   exact narrow contract `frame_view_players.ts` declares -- not a
//   fabricated shape, the SAME one both real products satisfy.

import { describe, expect, it } from "vitest";
import { rosterPlayersView, type RosterPlayersFrame } from "./frame_view_players.ts";
import { createSimHost } from "./sim_host.ts";

const HOME = "nebula";
const AWAY = "orion";

function neutralSample(): { move_x: number; move_y: number; held: number; edges: number } {
  return { move_x: 0, move_y: 0, held: 0, edges: 0 };
}

describe("frame_view_players: rosterPlayersView", () => {
  it("pairs every id in a real OFFLINE frame's roster with its live x/y, in roster order", () => {
    const host = createSimHost(HOME, AWAY, 7, 20, 3);
    try {
      // A few steps so positions are not all the authored kickoff spot --
      // not load-bearing for this derivation, just makes a stuck x/y bug
      // less likely to hide behind identical values.
      for (let i = 0; i < 3; i += 1) {
        host.step(neutralSample());
      }
      const frame = host.frame();
      const roster = host.roster();

      const players = rosterPlayersView(frame);

      expect(players.length).toBe(roster.ids.length);
      expect(players.length).toBeGreaterThan(0);
      players.forEach((player, index) => {
        expect(player.id).toBe(roster.ids[index]);
        expect(player.pos).toEqual({
          x: frame.players.x[index] ?? 0,
          y: frame.players.y[index] ?? 0,
        });
      });
    } finally {
      host.dispose();
    }
  });

  it("pairs every id in an ONLINE-shaped frame's roster with its live x/y, in roster order", () => {
    // `realMatchDriverPort.frame()`'s post-#611 contract (`online_ports.ts`):
    // a `RosterPlayersFrame`, the exact narrow type this module declares.
    const frame: RosterPlayersFrame = {
      roster: { ids: ["gax_oru", "drell", "morv"] },
      players: { x: [1.5, -2, 0], y: [3, 4.25, -1] },
    };

    expect(rosterPlayersView(frame)).toEqual([
      { id: "gax_oru", pos: { x: 1.5, y: 3 } },
      { id: "drell", pos: { x: -2, y: 4.25 } },
      { id: "morv", pos: { x: 0, y: -1 } },
    ]);
  });

  it("falls back to 0 for a roster id past the end of the live x/y arrays", () => {
    // Mirrors the `?? 0` guard the original inline block in `browser_main.ts`
    // carried -- a roster id with no matching x/y entry must not throw or
    // read `undefined` into the view.
    const frame: RosterPlayersFrame = {
      roster: { ids: ["a", "b"] },
      players: { x: [10], y: [20] },
    };

    expect(rosterPlayersView(frame)).toEqual([
      { id: "a", pos: { x: 10, y: 20 } },
      { id: "b", pos: { x: 0, y: 0 } },
    ]);
  });

  it("returns an empty view for an empty roster", () => {
    const frame: RosterPlayersFrame = { roster: { ids: [] }, players: { x: [], y: [] } };
    expect(rosterPlayersView(frame)).toEqual([]);
  });
});
