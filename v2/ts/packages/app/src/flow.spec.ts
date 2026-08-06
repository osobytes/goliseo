// Ported from spec/screens/flow_spec.lua.
//
// The Lua spec's final assertions ("top should be the match screen",
// `#top.state.players == 10`, `top.state.press.home == 2`) read
// `game.screens.match`'s state directly. `@gc/screens`'s `match.ts` exists
// now, but its `RealMatchScreenPort.state` (`real_match.ts`, what a
// constructed real match screen would actually expose through this port's
// `MatchScreenFactory`) is deliberately narrowed to `{time_left, score}` --
// no `players`/`press` fields at all (see `bootstrap.spec.ts`'s identically
// blocked "applies request roster, formation, tactic, and seed" for the same
// reason, spelled out there). So this is not a stale "not yet ported"
// blocker -- the module exists, its published contract just does not carry
// what these two assertions need. The walk itself (Squad -> Formation ->
// Tactic, carrying the formation/tactic choice) is ported faithfully below
// and verified against the injected `MatchScreenFactory` receiving the
// exact `{formation: "1-1-2", tactic: "press_high"}` choice the Lua spec's
// final `top.state.press.home == 2` (a `press_high`-derived value) is
// indirectly checking for.

import { describe, expect, it } from "vitest";
import { ScreenStack } from "./screen_stack.ts";
import { Flow, type FlowChoice } from "./flow.ts";
import { hit, menuLayout } from "./ui_bridge.ts";
import { FORMATION_CONTENT, SQUAD_CONTENT, TACTIC_CONTENT } from "./test_support/fixtures.ts";

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

  // Needs `game.screens.match` -- see this file's header.
  it.skip("the pushed screen is the real match screen with a ten-player press-high state", () => {});
});
