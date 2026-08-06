// Ported from spec/game/branding_spec.lua.
//
// "uses the corrected technical save identity" (`love.filesystem.getIdentity()
// == "goliseo"`) has no browser equivalent this milestone -- `love.filesystem`
// is a LÖVE save-directory API, and no persistence backend is wired up yet
// (v2/README.md §1; settings.ts's header covers the same gap). Ported as
// `it.skip` below.

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

  // No browser filesystem/save-identity backend is wired up this milestone
  // -- see this file's header.
  it.skip("uses the corrected technical save identity", () => {});

  it("retains no prototype product name in the player-facing shell", () => {
    const titleWidget = hit.find(title.layout(title.newState(VIEWPORT)), "title");
    const creditsWidget = hit.find(credits.layout(credits.newState(VIEWPORT, buildInfo)), "credits");
    const texts = [buildInfo.name, titleWidget?.text, creditsWidget?.text];
    for (const text of texts) {
      expect(typeof text === "string" && text.toLowerCase().includes("galactic")).toBe(false);
    }
  });
});
