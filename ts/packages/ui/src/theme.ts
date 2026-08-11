// game/ui/theme.lua — the shared product look. Pure data, no logic.

import type { RgbColor } from "./types.ts";

export interface ThemeColors {
  readonly void: RgbColor;
  readonly space: RgbColor;
  readonly nebula: RgbColor;
  readonly panel: RgbColor;
  readonly panelRaised: RgbColor;
  readonly panelSelected: RgbColor;
  readonly border: RgbColor;
  readonly borderSoft: RgbColor;
  readonly cyan: RgbColor;
  readonly amber: RgbColor;
  readonly text: RgbColor;
  readonly textMuted: RgbColor;
  readonly textDark: RgbColor;
  readonly disabled: RgbColor;
  readonly pitch: RgbColor;
  readonly keeper: RgbColor;
}

export type FontKind = "body" | "eyebrow" | "title" | "hero";

export interface UiTheme {
  readonly colors: ThemeColors;
  readonly radius: number;
  readonly borderWidth: number;
  readonly focusWidth: number;
  readonly fonts: Readonly<Record<FontKind, number>>;
}

export const theme: UiTheme = {
  colors: {
    void: [0.015, 0.022, 0.055],
    space: [0.035, 0.052, 0.11],
    nebula: [0.12, 0.09, 0.24],
    panel: [0.065, 0.095, 0.17],
    panelRaised: [0.09, 0.135, 0.23],
    panelSelected: [0.105, 0.255, 0.39],
    border: [0.18, 0.43, 0.62],
    borderSoft: [0.12, 0.27, 0.4],
    cyan: [0.25, 0.88, 1.0],
    amber: [1.0, 0.66, 0.24],
    text: [0.91, 0.96, 1.0],
    textMuted: [0.55, 0.67, 0.78],
    textDark: [0.025, 0.055, 0.08],
    disabled: [0.22, 0.28, 0.34],
    pitch: [0.025, 0.16, 0.17],
    keeper: [0.78, 0.9, 1.0],
  },
  radius: 6,
  borderWidth: 1,
  focusWidth: 2,
  fonts: {
    body: 13,
    eyebrow: 11,
    title: 24,
    hero: 38,
  },
};
