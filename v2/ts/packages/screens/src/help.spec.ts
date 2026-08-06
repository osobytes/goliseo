// Ported from the "renders the help card from the bindings rather than a
// literal" case in spec/game/input_bindings_spec.lua.
//
// That Lua case drives the real `game/input/bindings.lua` end to end
// through `help.lua`. This package cannot do the same: `@gc/input`
// (bindings.ts's TypeScript home) is not a declared dependency of
// `@gc/screens` (help.ts's header explains why), so `@gc/input`'s own port
// of this same case is `it.skip` in `packages/input/src/bindings.spec.ts`,
// with a comment pointing here.
//
// What *is* fully testable headless, without that dependency, is the
// property the assertion is actually protecting: that the card is built
// from whatever `ControlReferenceRow[]` it is handed, never a hand-written
// string. The first case below transcribes `@gc/input`'s real "match"
// section rows (bindings.ts's `REFERENCE`, `modifier`/`juke` bindings)
// verbatim and reproduces the exact Lua assertions (MODIFIER heading, the
// bound modifier/juke key labels). The second case proves the data-driven
// property directly: swapping in a different row set changes the rendered
// card, which a hard-coded string could never do.
//
// Reviving the real cross-package assertion (driving the actual
// `bindings.reference("match")` through this file's `help.layout`) needs
// `@gc/screens` added as a dependency of `@gc/input` (or vice versa) in
// package.json — out of scope for this task (see this package's porting
// report).

import { describe, expect, it } from "vitest";
import { hit } from "@gc/ui";
import { help } from "./help.ts";
import type { ControlReferenceRow } from "./content.ts";

const VP = { w: 960, h: 540 };

// Transcribed verbatim from @gc/input's bindings.ts (`REFERENCE`, section
// "match"), which is itself ported 1:1 from game/input/bindings.lua.
const MATCH_REFERENCE: readonly ControlReferenceRow[] = [
  { label: "Move", keyboard: "WASD or Arrows", gamepad: "Left Stick or D-pad", footnote: false },
  { label: "ACTION", note: "shoot / tackle", keyboard: "Space", gamepad: "A", footnote: false },
  { label: "PLAY", note: "pass / switch", keyboard: "K", gamepad: "X", footnote: false },
  { label: "Sprint", note: "hold", keyboard: "Shift", gamepad: "LB", footnote: false },
  { label: "Modifier", note: "loft / chip, hold", keyboard: "J", gamepad: "RT", footnote: false },
  { label: "Juke", keyboard: "L", gamepad: "Y", footnote: false },
  { label: "Equipment", note: "hold / tap", keyboard: "U", gamepad: "RB", footnote: true },
  { label: "Pause", keyboard: "P / Esc", gamepad: "Start / B", footnote: false },
];

function keyboardCardText(reference: readonly ControlReferenceRow[]): string {
  const layout = help.layout(help.newState(VP, reference));
  const card = hit.find(layout, "keyboard");
  expect(card, "the help screen lost its keyboard card").not.toBeNull();
  expect(card?.text).toBeDefined();
  return card?.text ?? "";
}

describe("help screen", () => {
  it("renders the help card from the bindings rather than a literal", () => {
    const card = keyboardCardText(MATCH_REFERENCE);
    expect(card).toContain("MODIFIER");
    expect(card, "the card does not show the bound modifier key").toContain("J");
    expect(card, "the card does not show the bound juke key").toContain("L");
  });

  it("is driven entirely by the injected reference, not a hard-coded string", () => {
    const alternate: readonly ControlReferenceRow[] = [
      { label: "Zoop", keyboard: "Q", gamepad: "Z", footnote: false },
    ];
    const card = keyboardCardText(alternate);
    expect(card).toContain("ZOOP");
    expect(card).not.toContain("MODIFIER");
    expect(card).not.toContain("JUKE");
  });
});
