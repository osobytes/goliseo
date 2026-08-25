// The `buildInfo.identity` field (see build_info.ts's header for why it
// exists) is this app's persisted-data namespace -- `browser_main.ts`
// namespaces its `localStorage` usage by it. "uses the corrected technical
// save identity" verifies that namespace was actually corrected from a
// prototype-era name, not merely renamed at the display-name level.

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

    const creditsWidget = hit.find(
      credits.layout(credits.newState(VIEWPORT, buildInfo)),
      "credits",
    );
    expect(typeof creditsWidget?.text === "string" && creditsWidget.text.includes("GOLISEO")).toBe(
      true,
    );
    expect(
      typeof creditsWidget?.text === "string" &&
        creditsWidget.text.includes("GOLISEO contributors"),
    ).toBe(true);
  });

  it("uses the corrected technical save identity", () => {
    expect(buildInfo.identity).toBe("goliseo");
  });

  // #612: `vitest` never sets `VITE_BUILD_SHA` (only `deploy.yml` does), so
  // this pins the local/CI fallback that keeps two peers in the SAME test
  // run -- or two local dev tabs -- agreeing and able to pair.
  it("falls back to a stable build identity when no deploy SHA is injected", () => {
    expect(buildInfo.build_sha).toBe("dev");
  });

  it("retains no prototype product name in the player-facing shell", () => {
    const titleWidget = hit.find(title.layout(title.newState(VIEWPORT)), "title");
    const creditsWidget = hit.find(
      credits.layout(credits.newState(VIEWPORT, buildInfo)),
      "credits",
    );
    const texts = [buildInfo.name, titleWidget?.text, creditsWidget?.text];
    for (const text of texts) {
      expect(typeof text === "string" && text.toLowerCase().includes("galactic")).toBe(false);
    }
  });
});
