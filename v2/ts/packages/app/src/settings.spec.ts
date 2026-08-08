// Ported from spec/game/foundations_spec.lua's "settings" describe block —
// the only block in that file about game/settings.lua (this package's).
// The rest of foundations_spec.lua (input actions, viewport transform, menu
// focus) is about game/input/** and game/ui/**, owned by @gc/input and
// @gc/ui respectively; left to those packages' porting agents.

import { describe, expect, it } from "vitest";
import { ok, type Result } from "@gc/core";
import { settings, type SettingsStorage } from "./settings.ts";

describe("settings", () => {
  it("clamps invalid numeric input and defaults invalid types", () => {
    const value = settings.validate({
      master_volume: 3,
      sfx_volume: -1,
      muted: "no",
      fullscreen: true,
    });
    expect(value.master_volume).toBe(1);
    expect(value.sfx_volume).toBe(0);
    expect(value.muted).toBe(false);
    expect(value.fullscreen).toBe(true);
  });

  it("preserves explicit false accessibility settings", () => {
    const value = settings.validate({ screen_shake: false, bloom: false });
    expect(value.screen_shake).toBe(false);
    expect(value.bloom).toBe(false);
  });

  it("round trips through deterministic storage", () => {
    let contents: string | undefined;
    const storage: SettingsStorage = {
      read: () => contents,
      write: (value): Result<true, string> => {
        contents = value;
        return ok(true);
      },
    };
    const value = settings.defaults();
    const withVolume = { ...value, master_volume: 0.42 };
    const saved = settings.save(withVolume, storage);
    expect(saved.ok).toBe(true);
    expect(settings.load(storage).master_volume).toBe(0.42);
    expect(contents).toBeDefined();
    expect(contents?.startsWith("version=1")).toBe(true);
  });
});
