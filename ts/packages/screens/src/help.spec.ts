// `@gc/input` (bindings.ts's TypeScript home) IS a declared dependency of
// `@gc/screens` (see package.json), so the real cross-package assertion is
// not blocked and is not skipped: `MATCH_REFERENCE` below is read straight
// from `@gc/input`'s `bindings.reference("match")`, not a transcription,
// and is driven through this file's real `help.layout`. `@gc/input`'s own
// `bindings.spec.ts` leaves this case to this file rather than duplicating
// the assertion.
//
// The second case below is additional coverage on top of the first: it
// proves the data-driven property directly by swapping in a different row
// set and checking the rendered card changes with it, which a hard-coded
// string could never do.

import { describe, expect, it } from "vitest";
import { hit } from "@gc/ui";
import { bindings } from "@gc/input";
import { help } from "./help.ts";
import type { ControlReferenceRow } from "./content.ts";

const VP = { w: 960, h: 540 };

// The REAL match reference, read straight from @gc/input's bindings — not a
// transcription. This is the whole point of the case below: "renders
// the help card from the bindings rather than a literal". A copied table proves
// the screen renders what it is handed; it does not prove the screen renders the
// bindings that actually exist, and it goes stale silently the first time
// someone rebinds a key.
const MATCH_REFERENCE: readonly ControlReferenceRow[] = bindings.reference(
  "match",
);

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
