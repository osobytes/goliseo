// The shared product look. Pure data, no logic.
//
// Every value here is sampled from the shipped coliseum renderer rather than
// invented, so the menu layer stops guessing at the game's look and simply
// adopts it. The source for each is named in a comment beside it; if one of
// those renderer constants moves, this file is stale and should follow.
//
// The palette this replaced was authored before the coliseum existed: its
// three backdrop tokens were literally named `void`, `space` and `nebula`.
// Amber leads now, because fire is what lights the arena. Cyan survives,
// demoted to what it was always best at — focus and navigation. Nothing else
// in the UI may use it.

import type { RgbColor } from "./types.ts";

export interface ThemeColors {
  /** Sky zenith. Letterbox bars, transition wipe, the top of the backdrop. */
  readonly zenith: RgbColor;
  /** Sky at the horizon. The bottom of the backdrop gradient. */
  readonly sky: RgbColor;
  /** The sky's mid band, between horizon and zenith. */
  readonly skyMid: RgbColor;
  /** The bowl's stone. Arcade arches, resting borders. */
  readonly sand: RgbColor;
  /** Brazier bodies. Disabled and recessed surfaces. */
  readonly stoneDeep: RgbColor;
  /** Fascia trim and brazier fire. The primary accent. */
  readonly amber: RgbColor;
  /** The hottest part of the brazier flame. Highlights on an amber surface. */
  readonly flame: RgbColor;
  /** Focus and navigation ONLY. Never decoration. */
  readonly cyan: RgbColor;
  /** Warm off-white. Titles and primary copy. */
  readonly marble: RgbColor;
  readonly panel: RgbColor;
  readonly panelRaised: RgbColor;
  readonly panelSelected: RgbColor;
  readonly border: RgbColor;
  readonly borderSoft: RgbColor;
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
    // `@gc/render`'s stadium_sky.ts fragment shader: `zenith`, `mid`,
    // `horizon`. The coliseum still drifts in space, so the sky stays — it
    // just stops being the whole backdrop.
    zenith: [0.01, 0.008, 0.03],
    skyMid: [0.1, 0.05, 0.2],
    sky: [0.07, 0.05, 0.16],
    // stadium_bowl.ts's bowl material (0x8a7a63).
    sand: [0.541, 0.478, 0.388],
    // stadium_props.ts's brazier body (0x6a5c4c).
    stoneDeep: [0.416, 0.361, 0.298],
    // stadium_bowl.ts's FASCIA_TRIM_COLOR.
    amber: [1.0, 0.66, 0.24],
    // stadium_props.ts's brazier flame (0xfff2d0).
    flame: [1.0, 0.949, 0.816],
    cyan: [0.25, 0.88, 1.0],
    marble: [0.949, 0.925, 0.878],
    panel: [0.047, 0.039, 0.063],
    panelRaised: [0.078, 0.068, 0.058],
    panelSelected: [0.16, 0.113, 0.055],
    border: [0.32, 0.28, 0.23],
    borderSoft: [0.2, 0.175, 0.145],
    text: [0.949, 0.925, 0.878],
    textMuted: [0.604, 0.561, 0.492],
    textDark: [0.114, 0.071, 0.02],
    disabled: [0.24, 0.215, 0.18],
    // `gc-data`'s only arena entry (`DEFAULT_ARENA.floor_color` in
    // `@gc/render`'s pitch.ts) — the formation preview draws the real floor.
    pitch: [0.025, 0.16, 0.17],
    keeper: [0.85, 0.95, 1.0],
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
