// Ported from game/settings.lua.

import { type Result, err, ok } from "@gc/core";

export interface GameSettings {
  readonly master_volume: number;
  readonly sfx_volume: number;
  readonly crowd_volume: number;
  readonly muted: boolean;
  readonly fullscreen: boolean;
  readonly screen_shake: boolean;
  readonly bloom: boolean;
}

/**
 * The browser has no `love.filesystem` equivalent wired up in this
 * milestone (v2/README.md §1 — the asset/persistence pipeline is a later
 * milestone; see this module's header for `settings.load`/`settings.save`
 * accordingly). Callers supply a `SettingsStorage`; there is no default
 * fallback the way `love_storage()` provided one for native LÖVE.
 */
export interface SettingsStorage {
  read(): string | undefined;
  write(contents: string): Result<true, string>;
}

function defaults(): GameSettings {
  return {
    master_volume: 1,
    sfx_volume: 0.8,
    crowd_volume: 0.55,
    muted: false,
    fullscreen: false,
    screen_shake: true,
    bloom: true,
  };
}

function clamp01(value: number): number {
  return Math.max(0, Math.min(1, value));
}

function validate(input: Partial<Record<keyof GameSettings, unknown>> | undefined): GameSettings {
  const raw = input ?? {};
  const base = defaults();
  const numberOr = (key: keyof GameSettings): number => {
    const value = raw[key];
    return typeof value === "number" ? clamp01(value) : (base[key] as number);
  };
  const booleanOr = (key: keyof GameSettings): boolean => {
    const value = raw[key];
    return typeof value === "boolean" ? value : (base[key] as boolean);
  };
  return {
    master_volume: numberOr("master_volume"),
    sfx_volume: numberOr("sfx_volume"),
    crowd_volume: numberOr("crowd_volume"),
    muted: booleanOr("muted"),
    fullscreen: booleanOr("fullscreen"),
    screen_shake: booleanOr("screen_shake"),
    bloom: booleanOr("bloom"),
  };
}

function boolString(value: boolean): string {
  return value ? "true" : "false";
}

function serialize(value: GameSettings): string {
  const clean = validate(value);
  return [
    "version=1",
    `master_volume=${clean.master_volume.toFixed(2)}`,
    `sfx_volume=${clean.sfx_volume.toFixed(2)}`,
    `crowd_volume=${clean.crowd_volume.toFixed(2)}`,
    `muted=${boolString(clean.muted)}`,
    `fullscreen=${boolString(clean.fullscreen)}`,
    `screen_shake=${boolString(clean.screen_shake)}`,
    `bloom=${boolString(clean.bloom)}`,
    "",
  ].join("\n");
}

function parse(contents: string): GameSettings {
  const raw: Record<string, unknown> = {};
  for (const match of contents.matchAll(/(\w+)=([^\r\n]+)/g)) {
    const key = match[1];
    const value = match[2];
    if (key === undefined || value === undefined) {
      continue;
    }
    if (value === "true") {
      raw[key] = true;
    } else if (value === "false") {
      raw[key] = false;
    } else {
      const numeric = Number(value);
      raw[key] = Number.isNaN(numeric) ? undefined : numeric;
    }
  }
  return validate(raw);
}

function load(storage: SettingsStorage | undefined): GameSettings {
  if (!storage) {
    return defaults();
  }
  const contents = storage.read();
  return contents !== undefined ? parse(contents) : defaults();
}

function save(value: GameSettings, storage: SettingsStorage | undefined): Result<true, string> {
  if (!storage) {
    return err("settings storage is unavailable");
  }
  return storage.write(serialize(value));
}

export const settings = { defaults, validate, serialize, parse, load, save };
