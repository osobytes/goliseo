// `audio.ts` is this package's own, so it is imported directly.
// `@gc/presentation`'s `combatFeedback` and `@gc/render`'s `bloom`/`effects`
// are not declared dependencies of this package (same reasoning as
// `rollback_validation.ts`'s `EffectsPort`) -- injected. Volume control and
// window fullscreen have no browser equivalent wired up this milestone --
// injected as `WindowPort`/a plain volume setter.

import type { Audio, GameSettingsAudioSlice } from "./audio.ts";
import type { GameSettings } from "./settings.ts";

/** `@gc/presentation`'s `combatFeedback.configureDefaults`, injected -- see this file's header. */
export interface CombatFeedbackDefaultsPort {
  configureDefaults(settings: GameSettings): void;
}

/** `@gc/render`'s `bloom` module's `config` table, injected -- see this file's header. */
export interface BloomPort {
  readonly config: { enabled: boolean };
}

/** `@gc/render`'s `effects.configure`, injected -- see this file's header. */
export interface EffectsConfigurePort {
  configure(settings: GameSettings): void;
}

/** Fullscreen control, injected -- no browser fullscreen API is wired up this milestone. */
export interface WindowPort {
  getFullscreen(): boolean;
  setFullscreen(enabled: boolean, mode: "desktop"): boolean;
}

export interface RuntimeSettingsPorts {
  readonly audio: Audio;
  readonly combatFeedback: CombatFeedbackDefaultsPort;
  readonly bloom: BloomPort;
  readonly effects: EffectsConfigurePort;
  /** Master volume control, injected -- no browser master-volume API is wired up this milestone. */
  readonly setMasterVolume?: (volume: number) => void;
  readonly window?: WindowPort;
}

function toAudioSlice(settings: GameSettings): GameSettingsAudioSlice {
  return {
    sfx_volume: settings.sfx_volume,
    crowd_volume: settings.crowd_volume,
    muted: settings.muted,
  };
}

function apply(ports: RuntimeSettingsPorts, settings: GameSettings): void {
  ports.bloom.config.enabled = settings.bloom;
  ports.audio.configure(toAudioSlice(settings));
  ports.combatFeedback.configureDefaults(settings);
  ports.effects.configure(settings);

  if (ports.setMasterVolume) {
    ports.setMasterVolume(settings.master_volume);
  }
  if (ports.window) {
    const fullscreen = ports.window.getFullscreen();
    if (fullscreen !== settings.fullscreen) {
      ports.window.setFullscreen(settings.fullscreen, "desktop");
    }
  }
}

export const runtimeSettings = { apply };
