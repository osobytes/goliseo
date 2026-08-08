// Ported from spec/game/branding_spec.lua.
//
// "uses the corrected technical save identity" (`love.filesystem.getIdentity()
// == "goliseo"`) originally had no browser equivalent to assert on -- no
// persistence backend existed yet (v2/README.md §1). This batch's
// `browser_main.ts` adds one (`localStorage`, namespaced by
// `buildInfo.identity` -- see build_info.ts's header for why that field
// exists), so the analogous check is now expressible: was the persisted-data
// namespace actually corrected from a prototype-era name.

import { describe, expect, it } from "vitest";
import { credits, title } from "@gc/screens";
import { hit } from "./ui_bridge.ts";
import { buildInfo } from "./build_info.ts";

const VIEWPORT = { w: 960, h: 540 };

describe("GOLISEO branding", () => {
  it("uses the canonical name in metadata and player-facing shell screens", () => {
    expect(buildInfo.name).toBe("GOLISEO");

    const titleWidget = hit.find(title.layout(title.newState(VIEWPORT)), "title");
    expect(titleWidget?.text).toBe("GOLISEO");

    const creditsWidget = hit.find(credits.layout(credits.newState(VIEWPORT, buildInfo)), "credits");
    expect(typeof creditsWidget?.text === "string" && creditsWidget.text.includes("GOLISEO")).toBe(true);
    expect(
      typeof creditsWidget?.text === "string" && creditsWidget.text.includes("GOLISEO contributors"),
    ).toBe(true);
  });

  it("uses the corrected technical save identity", () => {
    expect(buildInfo.identity).toBe("goliseo");
  });

  it("retains no prototype product name in the player-facing shell", () => {
    const titleWidget = hit.find(title.layout(title.newState(VIEWPORT)), "title");
    const creditsWidget = hit.find(credits.layout(credits.newState(VIEWPORT, buildInfo)), "credits");
    const texts = [buildInfo.name, titleWidget?.text, creditsWidget?.text];
    for (const text of texts) {
      expect(typeof text === "string" && text.toLowerCase().includes("galactic")).toBe(false);
    }
  });
});
