// Ported from game/screens/help.lua — the "how to play" control reference.
//
// The Lua original renders its card from `bindings.reference("match")`
// (game/input/bindings.lua) rather than a hand-written string, specifically
// so a rebinding cannot leave this screen lying to the player (see the Lua
// file's header comment, and spec/game/input_bindings_spec.lua's "renders
// the help card from the bindings rather than a literal" case). That
// property is preserved here: `layout` builds the card from
// `state.matchReference` only — nothing in this file names a key or button.
//
// `game/input/bindings.lua` maps to `@gc/input` (v2/README.md's file
// table), which already exists and already exports a structurally
// identical `ControlReferenceRow` (see `@gc/input`'s `bindings.ts`) — but
// `@gc/input` is not a declared dependency of `@gc/screens`, and this task
// may not edit package.json to add it. `ControlReferenceRow` is therefore
// declared locally in content.ts (same precedent as `@gc/ui`'s
// `FocusEvent`), and the already-computed `bindings.reference("match")`
// rows are injected via `newState` rather than the bindings module itself.
// See this package's porting report for what unblocks importing the real
// thing, and help.spec.ts for how this is tested without it.

import { focus, type Layout } from "@gc/ui";
import type { FocusEvent } from "@gc/ui";
import type { ControlReferenceRow } from "./content.ts";

const CARD_LABEL_WIDTH = 18;

export interface HelpScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly matchReference: readonly ControlReferenceRow[];
  readonly focus: string;
}

export type HelpAction = { readonly go: "back" };

function controlCard(
  heading: string,
  rows: readonly ControlReferenceRow[],
  device: "keyboard" | "gamepad",
): string {
  const lines = [heading];
  for (const row of rows) {
    const label = row.label.toUpperCase() + (row.footnote ? "*" : "");
    const pad = Math.max(1, CARD_LABEL_WIDTH - label.length);
    lines.push(label + " ".repeat(pad) + row[device]);
  }
  return lines.join("\n");
}

function newState(
  viewport: { readonly w: number; readonly h: number },
  matchReference: readonly ControlReferenceRow[],
): HelpScreenState {
  return { viewport, matchReference, focus: "back" };
}

function layout(state: HelpScreenState): Layout {
  return [
    {
      id: "title",
      kind: "title",
      text: "HOW TO PLAY",
      rect: { x: 64, y: 44, w: 832, h: 36 },
      data: { align: "center" },
    },
    {
      id: "keyboard",
      kind: "card",
      text: controlCard("MATCH CONTROLS · KEYBOARD", state.matchReference, "keyboard"),
      rect: { x: 92, y: 112, w: 360, h: 292 },
      data: { focusable: false },
    },
    {
      id: "gamepad",
      kind: "card",
      text: controlCard("MATCH CONTROLS · GAMEPAD", state.matchReference, "gamepad"),
      rect: { x: 508, y: 112, w: 360, h: 292 },
      data: { focusable: false },
    },
    {
      id: "hint",
      kind: "label",
      text:
        "ACTION — shoot / tackle     PLAY — pass / switch     *COMBAT PROTOTYPE ONLY\n" +
        "HOLD ACTION OR PLAY TO CHARGE · RELEASE TO COMMIT     EQUIPMENT: HOLD / TAP" +
        "\nKEEPER: PLAY THROWS · ACTION PUNTS",
      rect: { x: 90, y: 414, w: 780, h: 54 },
      data: { align: "center", tone: "muted" },
    },
    {
      id: "back",
      kind: "button",
      text: "BACK",
      focused: state.focus === "back",
      rect: { x: 380, y: 486, w: 200, h: 42 },
    },
  ];
}

function update(state: HelpScreenState, event: FocusEvent): readonly [HelpScreenState, HelpAction | undefined] {
  if (event.kind === "action" && event.action === "back") {
    return [state, { go: "back" }];
  }
  const id = focus.activated(layout(state), state.focus, event);
  return [state, id === "back" ? { go: "back" } : undefined];
}

export const help = { newState, layout, update };
