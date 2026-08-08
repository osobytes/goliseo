// Ported from game/runtime_settings.lua.
//
// The Lua original reaches into four other modules and two `love.*`
// globals. `game.audio` is this package's own (`audio.ts`, imported
// directly); `game.presentation.combat_feedback` is `@gc/presentation`
// (not a declared dependency -- injected); `game.render.bloom` and
// `game.render.effects` are `@gc/render`'s (not yet ported there either --
// injected, same as `rollback_validation.ts`'s `EffectsPort`); and
// `love.audio.setVolume`/`love.window.get/setFullscreen` have no browser
// equivalent wired up this milestone (v2/README.md §1) -- injected as
// `WindowPort`/a plain volume setter.

import type { Audio, GameSettingsAudioSlice } from "./audio.ts";
import type { GameSettings } from "./settings.ts";

/** `game.presentation.combat_feedback.configure_defaults`, injected -- see this file's header. */
export interface CombatFeedbackDefaultsPort {
  configureDefaults(settings: GameSettings): void;
}

/** `game.render.bloom`'s module-level `config` table, injected -- see this file's header. */
export interface BloomPort {
  readonly config: { enabled: boolean };
}

/** `game.render.effects.configure`, injected -- see this file's header. */
export interface EffectsConfigurePort {
  configure(settings: GameSettings): void;
}

/** `love.window.getFullscreen`/`.setFullscreen`, injected -- no browser fullscreen API is wired up this milestone. */
export interface WindowPort {
  getFullscreen(): boolean;
  setFullscreen(enabled: boolean, mode: "desktop"): boolean;
}

export interface RuntimeSettingsPorts {
  readonly audio: Audio;
  readonly combatFeedback: CombatFeedbackDefaultsPort;
  readonly bloom: BloomPort;
  readonly effects: EffectsConfigurePort;
  /** `love.audio.setVolume`, injected -- no browser master-volume API is wired up this milestone. */
  readonly setMasterVolume?: (volume: number) => void;
  readonly window?: WindowPort;
}

function toAudioSlice(settings: GameSettings): GameSettingsAudioSlice {
  return { sfx_volume: settings.sfx_volume, crowd_volume: settings.crowd_volume, muted: settings.muted };
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
