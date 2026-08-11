// Ported from game/flow.lua.
//
// Wires the pre-match screen sequence: Squad -> Formation -> Tactic -> Match.
// Each menu reports an action; this router pushes the next screen and
// carries the player's choices forward into the match.
//
// The Lua original's final step is `Match.new({formation, tactic})`
// (`game.screens.match`), not yet ported to `@gc/screens` (this package's
// porting report) -- injected as a `MatchScreenFactory` instead. Every
// screen in `@gc/screens` now takes its content as an explicit `newState`
// parameter (v2/README.md rule 6.7; see `menu.ts`'s header in that
// package), so `Flow.start` takes a `FlowContent` bundle for the same
// reason `app.ts` does.

import { Menu, formation, squad, tactic, type FormationContentData, type SquadContentData, type TacticContentData, type Viewport } from "@gc/screens";
import type { Screen } from "./screen_stack.ts";
import type { ScreenStack } from "./screen_stack.ts";

export interface FlowContent {
  readonly squad: SquadContentData;
  readonly formation: FormationContentData;
  readonly tactic: TacticContentData;
}

export interface FlowChoice {
  readonly formation: string;
  readonly tactic: string;
}

/** `game.screens.match.new`, injected -- see this file's header. */
export type MatchScreenFactory = (choice: FlowChoice) => Screen;

function asScreen<State extends { readonly viewport: Viewport }, Action>(menu: Menu<State, Action>): Screen {
  // See app.ts's `asMenu` for why this cast is safe: `Menu.draw` needs a
  // `@gc/ui` backend this milestone does not wire up, and nothing in this
  // port's test coverage calls `draw`.
  return menu as unknown as Screen;
}

function start(
  stack: ScreenStack<unknown, unknown>,
  viewport: Viewport,
  content: FlowContent,
  matchFactory: MatchScreenFactory,
): void {
  const choice: { formation: string; tactic: string } = { formation: "2-1-1", tactic: "balanced" };

  const go = (action: { readonly go: string } & Record<string, unknown>): void => {
    if (action.go === "formation") {
      const menu = new Menu(formation, formation.newState(viewport, content.formation), (a) => go(a));
      stack.push(asScreen(menu));
    } else if (action.go === "tactic") {
      choice.formation = action.formationId as string;
      const menu = new Menu(tactic, tactic.newState(viewport, content.tactic), (a) => go(a));
      stack.push(asScreen(menu));
    } else if (action.go === "match") {
      choice.tactic = action.tacticId as string;
      stack.push(matchFactory({ formation: choice.formation, tactic: choice.tactic }));
    }
  };

  const menu = new Menu(squad, squad.newState(viewport, content.squad), (a) => go(a));
  stack.push(asScreen(menu));
}

export const Flow = { start };
