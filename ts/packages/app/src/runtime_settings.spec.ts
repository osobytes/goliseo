// `runtime_settings.apply` forwards settings to a
// `CombatFeedbackDefaultsPort`. `@gc/presentation`'s `combat_feedback`
// (whose `diagnostics` would confirm the downstream effect on motion/flash)
// is not a declared dependency of this package, so the second case below
// asserts against a recording `CombatFeedbackDefaultsPort` double instead:
// it proves `runtime_settings` forwards the exact settings object to
// `configureDefaults`, which is this module's own contribution -- the
// downstream claim that `@gc/presentation` actually reduces motion/flash
// from it belongs to that package's own tests, once this package can
// depend on it.

import { describe, expect, it } from "vitest";
import { Audio, type CombatFeedbackPort } from "./audio.ts";
import { runtimeSettings, type RuntimeSettingsPorts } from "./runtime_settings.ts";
import { settings } from "./settings.ts";

const NOOP_COMBAT_FEEDBACK: CombatFeedbackPort = {
  link: () => ({ disposition: {} }),
  disposition: () => ({}),
};

function newPorts(overrides: Partial<RuntimeSettingsPorts> = {}): RuntimeSettingsPorts & {
  readonly bloom: { config: { enabled: boolean } };
  readonly configureDefaultsCalls: unknown[];
} {
  const configureDefaultsCalls: unknown[] = [];
  const effectsConfigureCalls: unknown[] = [];
  return {
    audio: new Audio(NOOP_COMBAT_FEEDBACK),
    combatFeedback: {
      configureDefaults: (value) => {
        configureDefaultsCalls.push(value);
      },
    },
    bloom: { config: { enabled: true } },
    effects: {
      configure: (value) => {
        effectsConfigureCalls.push(value);
      },
    },
    configureDefaultsCalls,
    ...overrides,
  };
}

describe("runtime settings", () => {
  it("applies master volume and fullscreen through thin browser-port adapters", () => {
    let volume: number | undefined;
    let fullscreen = false;
    const ports = newPorts({
      setMasterVolume: (value) => {
        volume = value;
      },
      window: {
        getFullscreen: () => false,
        setFullscreen: (value) => {
          fullscreen = value;
          return true;
        },
      },
    });

    const value = { ...settings.defaults(), master_volume: 0.35, fullscreen: true };
    runtimeSettings.apply(ports, value);

    expect(volume).toBe(0.35);
    expect(fullscreen).toBe(true);
  });

  it("routes existing shake and bloom settings to the combat feedback defaults port", () => {
    const ports = newPorts();
    const value = { ...settings.defaults(), screen_shake: false, bloom: false };
    runtimeSettings.apply(ports, value);

    expect(ports.configureDefaultsCalls).toEqual([value]);
    expect(ports.bloom.config.enabled).toBe(false);
  });
});
