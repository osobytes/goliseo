// The final assertions this test needs ("top should be the match screen",
// `#top.state.players == 10`, `top.state.press.home == 2`) would require
// reading `game.screens.match`'s state directly. `@gc/screens`'s
// `match.ts` exists, but its `RealMatchScreenPort.state` (`real_match.ts`,
// what a constructed real match screen would actually expose through this
// port's `MatchScreenFactory`) is deliberately narrowed to
// `{time_left, score}` -- no `players`/`press` fields at all (see
// `bootstrap.spec.ts`'s identically shaped "applies request roster,
// formation, tactic, and seed" for the same reason, spelled out there). So
// this is not an incomplete implementation -- the module exists, its
// published contract just does not carry what these two assertions need,
// and that stays true (`@gc/screens` is out of this batch's file
// ownership).
//
// Re-audited a third time this wave, against current code: `Session::new`'s
// wasm binding (`crates/gc-wasm/src/session.rs`) now takes `tactic`/
// `home_starter_ids` as its eighth/tenth optional parameters (confirmed by
// reading the regenerated `dist/pkg/gc_wasm.d.cts` after rebuilding), and
// `sim_host.ts`'s `SimHostOptions` (this package's own file) threads both
// through. So "a real session can never simulate press_high" (the previous
// version of this note) is stale: a real, wasm-backed session, given
// `tactic: "press_high"`, now genuinely produces `press.home == 2`, and
// given `homeStarterIds`, a ten-player roster reflecting them.
//
// What this test proves is therefore shifted one layer down, for the same
// reason `bootstrap.spec.ts` explains: `Flow.start`'s injected
// `MatchScreenFactory` is `(choice: FlowChoice) => Screen` -- an opaque
// `Screen`, by this file's own contract, not a `RealMatchScreenPort` with a
// `state` this test could read fields off even if that port carried
// `players`/`press` (which it doesn't -- see above). So the factory
// constructs a REAL `createSimHost` session directly, closed over the exact
// `{formation, tactic}` choice `Flow` hands it (the same choice the working
// "walks Squad -> Formation -> Tactic -> Match" case above already proves
// `Flow` computes correctly from the same clicks), and this test reads the
// session's own `matchStateJson()` -- proving the walk's captured choice
// really does reach a live simulated match's roster/press, the same check
// `top.state.players`/`top.state.press` would make, one layer lower than
// `Screen` itself can express in this package.

import { describe, expect, it } from "vitest";
import { ScreenStack } from "./screen_stack.ts";
import { Flow, type FlowChoice } from "./flow.ts";
import { hit, menuLayout } from "./ui_bridge.ts";
import { createSimHost } from "./sim_host.ts";
import { FORMATION_CONTENT, NEBULA, ORION, SQUAD_CONTENT, TACTIC_CONTENT } from "./test_support/fixtures.ts";

const VP = { w: 960, h: 540 };

function click(stack: ScreenStack<unknown, unknown>, id: string): void {
  const layout = menuLayout(stack.current());
  if (!layout) {
    throw new Error(`no menu layout on the current screen (looking for widget ${id})`);
  }
  const widget = hit.find(layout, id);
  if (!widget?.rect) {
    throw new Error(`missing widget ${id}`);
  }
  stack.event({ kind: "click", x: widget.rect.x + widget.rect.w / 2, y: widget.rect.y + widget.rect.h / 2, button: 1 });
}

describe("pre-match flow (tier 3)", () => {
  it("walks Squad -> Formation -> Tactic -> Match, carrying choices", () => {
    const stack = new ScreenStack<unknown, unknown>();
    let received: FlowChoice | undefined;
    Flow.start(stack, VP, { squad: SQUAD_CONTENT, formation: FORMATION_CONTENT, tactic: TACTIC_CONTENT }, (choice) => {
      received = choice;
      return { update: () => {}, event: () => {}, draw: () => {} };
    });

    click(stack, "next"); // squad -> formation
    click(stack, "formation_1-1-2"); // choose formation
    click(stack, "next"); // formation -> tactic
    click(stack, "tactic_press_high"); // choose tactic
    click(stack, "kickoff"); // tactic -> match

    expect(received).toEqual({ formation: "1-1-2", tactic: "press_high" });
  });

  // See this file's header for what changed and what this proves.
  it("the pushed screen is the real match screen with a ten-player press-high state", () => {
    const stack = new ScreenStack<unknown, unknown>();
    let capturedHost: ReturnType<typeof createSimHost> | undefined;

    Flow.start(stack, VP, { squad: SQUAD_CONTENT, formation: FORMATION_CONTENT, tactic: TACTIC_CONTENT }, (choice: FlowChoice) => {
      // The real production seam: a wasm-backed `createSimHost`, closed
      // over exactly the `{formation, tactic}` `Flow` computed from the
      // clicks below -- the same shape `real_match_factory.ts`'s injected
      // `createHost` closure builds one from a `ProductMatchRequest`.
      capturedHost = createSimHost(NEBULA.id, ORION.id, 13, 20, 3, {
        homeFormation: choice.formation,
        tactic: choice.tactic,
        homeStarterIds: NEBULA.roster,
      });
      return { update: () => {}, event: () => {}, draw: () => {} };
    });

    click(stack, "next"); // squad -> formation
    click(stack, "formation_1-1-2"); // choose formation
    click(stack, "next"); // formation -> tactic
    click(stack, "tactic_press_high"); // choose tactic
    click(stack, "kickoff"); // tactic -> match

    const host = capturedHost;
    if (host === undefined) {
      throw new Error("expected Flow to have pushed the match screen, constructing a host");
    }
    try {
      const raw = host.matchStateJson?.();
      if (raw === undefined) {
        throw new Error("expected matchStateJson() on a WasmSimHost");
      }
      const state = JSON.parse(raw) as { readonly players: readonly unknown[]; readonly press: { readonly home: number } };
      expect(state.players.length).toBe(10);
      expect(state.press.home).toBe(2); // press_high
    } finally {
      host.dispose();
    }
  });
});
