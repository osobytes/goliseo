// The in-menu audio/video settings screen. See squad.ts's header for the
// pure/impure seam and content-injection rationale.
//
// `@gc/app`'s `settings.ts` (`defaults`/`validate`) is not this package's
// to own, so it is injected as a `SettingsSource` (content.ts), the same
// pattern `@gc/ui`'s `tuning_panel.ts` uses for `TuningSource`.
// Re-validating on every `update` call would only be needed to get an
// independent clone before mutating; `update` here never mutates its input
// (see squad.ts et al.), so a plain object spread gives the same
// clone-before-mutate guarantee without calling back into the injected
// source on every keystroke. `validate` is therefore only needed once, at
// `newState`, keeping `update`'s arity matching the pure `(state, event)`
// seam every other screen in this package keeps.

import { focus, type Layout } from "@gc/ui";
import type { FocusEvent } from "@gc/ui";
import type { GameSettings, SettingsSource } from "./content.ts";

export interface SettingsScreenContext {
  readonly settings?: GameSettings;
}

export interface SettingsScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly settings: GameSettings;
  readonly focus: string;
}

export type SettingsAction =
  | { readonly go: "back"; readonly settings: GameSettings }
  | { readonly go: "settings_changed"; readonly settings: GameSettings };

type AdjustableKey =
  | "master_volume"
  | "sfx_volume"
  | "crowd_volume"
  | "muted"
  | "fullscreen"
  | "screen_shake";

function isAdjustableKey(key: string): key is AdjustableKey {
  return (
    key === "master_volume" ||
    key === "sfx_volume" ||
    key === "crowd_volume" ||
    key === "muted" ||
    key === "fullscreen" ||
    key === "screen_shake"
  );
}

function newState(
  viewport: { readonly w: number; readonly h: number },
  source: SettingsSource,
  context?: SettingsScreenContext,
): SettingsScreenState {
  const value = context?.settings ?? source.defaults();
  return { viewport, settings: source.validate(value), focus: "master_volume" };
}

function percent(value: number): string {
  return `${Math.floor(value * 100 + 0.5)}%`;
}

function onOff(value: boolean): string {
  return value ? "ON" : "OFF";
}

function layout(state: SettingsScreenState): Layout {
  const values: readonly [AdjustableKey, string, string][] = [
    ["master_volume", "MASTER VOLUME", percent(state.settings.master_volume)],
    ["sfx_volume", "SFX VOLUME", percent(state.settings.sfx_volume)],
    ["crowd_volume", "CROWD VOLUME", percent(state.settings.crowd_volume)],
    ["muted", "MUTE", onOff(state.settings.muted)],
    ["fullscreen", "FULLSCREEN", onOff(state.settings.fullscreen)],
    ["screen_shake", "SCREEN SHAKE", onOff(state.settings.screen_shake)],
  ];
  const widgets: Widget[] = [
    {
      id: "title",
      kind: "title",
      text: "SETTINGS",
      rect: { x: 0, y: 38, w: state.viewport.w, h: 36 },
      data: { align: "center" },
    },
    {
      id: "instructions",
      kind: "label",
      text: "LEFT / RIGHT TO ADJUST",
      rect: { x: 0, y: 82, w: state.viewport.w, h: 22 },
      data: { align: "center", tone: "muted" },
    },
  ];
  values.forEach(([id, label, value], i) => {
    widgets.push({
      id,
      kind: "button",
      text: `${label}     ${value}`,
      focused: state.focus === id,
      rect: { x: 300, y: 120 + i * 48, w: 360, h: 38 },
    });
  });
  widgets.push({
    id: "back",
    kind: "button",
    text: "SAVE & BACK",
    focused: state.focus === "back",
    rect: { x: 380, y: 474, w: 200, h: 42 },
  });
  return widgets;
}

function adjust(settings: GameSettings, key: AdjustableKey, delta: number): GameSettings {
  if (key === "master_volume" || key === "sfx_volume" || key === "crowd_volume") {
    return { ...settings, [key]: Math.max(0, Math.min(1, settings[key] + delta)) };
  }
  return { ...settings, [key]: !settings[key] };
}

function update(state: SettingsScreenState, event: FocusEvent): readonly [SettingsScreenState, SettingsAction | undefined] {
  let next: SettingsScreenState = { ...state };
  const currentLayout = layout(next);
  if (event.kind === "action" && event.action === "back") {
    return [next, { go: "back", settings: next.settings }];
  }
  if (event.kind === "action" && (event.action === "left" || event.action === "right")) {
    // The action fires even when `focus` is not an adjustable row (e.g.
    // "back") — `adjust` is simply a no-op there.
    if (isAdjustableKey(next.focus)) {
      next = { ...next, settings: adjust(next.settings, next.focus, event.action === "left" ? -0.1 : 0.1) };
    }
    return [next, { go: "settings_changed", settings: next.settings }];
  }

  next = { ...next, focus: focus.navigate(currentLayout, next.focus, event) ?? next.focus };
  const id = focus.activated(currentLayout, next.focus, event);
  if (id === "back") {
    return [next, { go: "back", settings: next.settings }];
  } else if (id !== null) {
    // `adjust` is called (and no-ops) even for an activated id that is not
    // one of the six adjustable rows — none exist in this screen's layout
    // besides "back" (handled above), but the guard stays explicit rather
    // than assumed.
    next = {
      ...next,
      focus: id,
      settings: isAdjustableKey(id) ? adjust(next.settings, id, 0.1) : next.settings,
    };
    return [next, { go: "settings_changed", settings: next.settings }];
  }
  return [next, undefined];
}

export const settings = { newState, layout, update };

type Widget = Layout[number];
