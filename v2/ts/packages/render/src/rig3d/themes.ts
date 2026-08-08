// One proof-of-concept presentation per proof theme, per
// docs/design/prototype_theme_roster.md.
//
// All three ride the SAME Rig_Medium skeleton and the SAME animation clips.
// Everything that differs between them lives in this file: shape language,
// material, palette and loadout. That is the roster's central claim, and having
// it be literally one table per theme is the cheapest way to test it.
//
// Colour slots may be a literal RGB triple or one of the strings "team" /
// "trim". Those resolve against the active team at build time, which is how
// the roster rule "Theme color is secondary to team ownership" is enforced
// structurally rather than by hoping an artist remembers it -- the largest
// readable surfaces (torso, shield face, helmet crest) are wired to the team,
// not the theme.

import type { RGB, RGBA } from "./palette.ts";

/** Both sides of a match. */
export interface Team {
  readonly key: string;
  readonly label: string;
  readonly main: RGB;
  readonly trim: RGB;
}

export const TEAMS: readonly Team[] = [
  { key: "crimson", label: "Crimson", main: [0.7, 0.15, 0.16], trim: [0.94, 0.78, 0.32] },
  { key: "azure", label: "Azure", main: [0.14, 0.34, 0.74], trim: [0.72, 0.88, 1.0] },
];

/** Which face/head shape the head builder produces for this figure. */
export type HeadShape = "round" | "cylinder" | "boxround";
/** How the hand is built for this figure. */
export type HandStyle = "mitten" | "clamp";
/** How limbs are tapered for this figure. */
export type LimbStyle = "tapered" | "tube";
/** How the torso profile is built for this figure. */
export type TorsoStyle = "bean" | "trapezoid";
/** Which face treatment face.ts applies. */
export type FaceStyle = "expressive" | "minifig" | "vinyl";

// FIGURE STYLE is a second, independent axis from THEME. Theme decides what the
// character is (knight / ranger / action figure); figure decides how stylised
// the body language is. Any theme can wear any figure style.
//
// IMPORTANT scope note: `head_scale` changes the head MESH only. The skeleton
// is byte-identical across all three -- same bone names, same bone lengths, and
// the same clips drive all of them.
export interface Figure {
  readonly key: string;
  readonly label: string;
  readonly blurb: string;
  readonly head_scale: number;
  readonly head_h: number;
  readonly head_shape: HeadShape;
  readonly head_drop: number;
  readonly neck: boolean;
  readonly stud: boolean;
  readonly hands: HandStyle;
  readonly limbs: LimbStyle;
  readonly torso: TorsoStyle;
  readonly face: FaceStyle;
  readonly helm_scale: number;
}

export const FIGURES: readonly Figure[] = [
  {
    key: "natural",
    label: "Natural",
    blurb: "Stylised humanoid. Head sized to the rig.",
    head_scale: 1.0,
    head_h: 1.95,
    head_shape: "round",
    head_drop: 0.0,
    neck: true,
    stud: false,
    hands: "mitten",
    limbs: "tapered",
    torso: "bean",
    face: "expressive",
    helm_scale: 1.0,
  },
  {
    key: "minifig",
    label: "Minifig",
    blurb: "Lego-like: cylinder head, printed face, C-clamp hands, tube limbs.",
    head_scale: 1.3,
    head_h: 1.3,
    head_shape: "cylinder",
    head_drop: -0.045,
    neck: false,
    stud: true,
    hands: "clamp",
    limbs: "tube",
    torso: "trapezoid",
    face: "minifig",
    helm_scale: 1.34,
  },
  {
    key: "vinyl",
    label: "Vinyl",
    blurb: "Funko-like: oversized rounded head, big blank eyes, small body.",
    head_scale: 1.85,
    head_h: 1.62,
    head_shape: "boxround",
    head_drop: -0.055,
    neck: false,
    stud: false,
    hands: "mitten",
    limbs: "tapered",
    torso: "bean",
    face: "vinyl",
    helm_scale: 1.62,
  },
];

/** Shape-language flags a theme sets for the body/kit builders. */
export interface ThemeShape {
  readonly bevel: number;
  readonly plate_layers: number;
  readonly seams: boolean;
  readonly joint_balls: boolean;
  readonly heraldic_block: boolean;
  readonly pteruges: number;
}

/** A colour slot: a literal RGB, or a sentinel resolved against the team. */
export type ColorSlot = RGB | "team" | "trim";

/** A theme's authored colour slots. `limbs` and `seam` may be left unset. */
export interface ThemeColor {
  readonly skin: ColorSlot;
  readonly cloth: ColorSlot;
  readonly plate: ColorSlot;
  readonly plate_dark: ColorSlot;
  readonly accent: ColorSlot;
  readonly strap: ColorSlot;
  readonly crest: ColorSlot;
  readonly joint: ColorSlot;
  readonly limbs?: ColorSlot;
  readonly seam?: ColorSlot;
}

/** Which equipment id rides which hand/hip socket. */
export interface Loadout {
  readonly right?: string;
  readonly left?: string;
  readonly hip?: string;
}

/** One presentation theme: shape language, palette, headgear and loadout. */
export interface Theme {
  readonly key: string;
  readonly label: string;
  readonly presentation: string;
  readonly blurb: string;
  readonly shape: ThemeShape;
  readonly color: ThemeColor;
  readonly head: string;
  readonly loadout: Loadout;
}

export const LIST: readonly Theme[] = [
  // ---------------------------------------------------------------------
  {
    key: "medieval",
    label: "Medieval Fantasy",
    presentation: "Rook Emberguard",
    blurb: "Broad armoured humanoid. Plates, leather, heraldic blocks, forged edges.",
    shape: {
      bevel: 0.32,
      plate_layers: 3,
      seams: false,
      joint_balls: false,
      heraldic_block: true,
      pteruges: 0,
    },
    color: {
      skin: [0.74, 0.55, 0.4],
      cloth: "team", // surcoat: the dominant ownership surface
      plate: [0.66, 0.69, 0.74],
      plate_dark: [0.44, 0.47, 0.53],
      accent: [0.78, 0.56, 0.22], // warm bronze trim
      strap: [0.3, 0.2, 0.13],
      crest: "trim",
      joint: [0.34, 0.3, 0.28],
      // unused: shape.seams = false, see themes.resolvedPalette's note on why
      // this is authored explicitly rather than left absent.
      seam: [0, 0, 0],
    },
    head: "great_helm",
    loadout: { right: "tournament_sword", left: "heater_shield" },
  },

  // ---------------------------------------------------------------------
  {
    key: "scifi",
    label: "Galactic Sci-Fi",
    presentation: "Nova Quell",
    blurb: "Clean athletic space-ranger. Panels, luminous seams, compact tech.",
    shape: {
      bevel: 0.62,
      plate_layers: 1,
      seams: true,
      joint_balls: false,
      heraldic_block: false,
      pteruges: 0,
    },
    color: {
      skin: [0.72, 0.54, 0.42],
      limbs: [0.22, 0.24, 0.3], // sealed undersuit, not bare arms
      cloth: [0.17, 0.18, 0.22],
      plate: "team", // chest and shoulder panels: the ownership surface
      plate_dark: [0.3, 0.33, 0.39],
      accent: [0.86, 0.88, 0.92], // clean white secondary
      strap: [0.2, 0.22, 0.26],
      crest: "trim",
      seam: "trim", // emissive
      joint: [0.26, 0.28, 0.33],
    },
    head: "visor_helm",
    loadout: { right: "vector_blade", hip: "pulse_blaster" },
  },

  // ---------------------------------------------------------------------
  {
    key: "toybox",
    label: "Toybox",
    presentation: "Moxie Modular",
    blurb: "Heroic action figure. Chunky pieces, visible joints, molded edges.",
    shape: {
      bevel: 0.92,
      plate_layers: 2,
      seams: false,
      joint_balls: true,
      heraldic_block: false,
      pteruges: 0,
    },
    color: {
      skin: [0.96, 0.78, 0.6], // moulded plastic flesh tone
      limbs: [0.93, 0.9, 0.84], // cream plastic body
      cloth: [0.88, 0.85, 0.78],
      plate: "team", // the armour carries ownership
      plate_dark: [0.86, 0.82, 0.75],
      accent: "trim",
      strap: [0.28, 0.28, 0.32],
      crest: "trim",
      joint: [0.24, 0.24, 0.28], // dark exposed ball joints
      // unused: shape.seams = false, see the medieval theme's `seam` for why
      // this is authored rather than left absent.
      seam: [0, 0, 0],
    },
    head: "figure_helm",
    loadout: { right: "foam_champion", left: "spring_glove" },
  },
];

// Resolves a theme colour slot against the active team.
export function resolve(slot: ColorSlot, team: Team): RGB {
  if (slot === "team") {
    return team.main;
  } else if (slot === "trim") {
    return team.trim;
  }
  return slot;
}

// ---------------------------------------------------------------------------
// Palette slots (#337)
//
// A vertex used to carry its literal RGBA baked in at mesh-build time, which
// meant every colour variant -- every team, every future cosmetic skin --
// needed its own copy of every mesh. Instead each vertex now carries a SLOT
// INDEX (a small integer, one of `SLOTS` below) and the actual colours are
// resolved once per (theme, team) into a small array. Meshes become
// colour-agnostic: the same geometry is reused for any palette.
//
// This is the same eight names `theme.color` already speaks (skin, cloth,
// plate, plate_dark, accent, strap, crest, joint), plus the two theme-color
// keys that are only meshed by some themes (limbs, seam) and the two literal
// constants face.ts reaches for regardless of theme (ink, sclera). Twelve
// slots total.
// ---------------------------------------------------------------------------

/** Canonical slot order; index 0 is the shader/uniform palette index. */
export const SLOTS: readonly string[] = [
  "skin",
  "cloth",
  "plate",
  "plate_dark",
  "accent",
  "strap",
  "crest",
  "joint",
  "limbs",
  "seam",
  "ink",
  "sclera",
];

export const SLOT_COUNT: number = SLOTS.length;

/**
 * Every palette slot name, as non-optional `number` fields. Content builders
 * (face.ts, body.ts, equipment.ts, headgear.ts) index `c.skin`, `c.ink`, etc.
 * directly; a plain `Record<string, number>` would make every one of those
 * reads `number | undefined` under `noUncheckedIndexedAccess` even though
 * every slot is always present.
 */
export interface SlotIndex {
  readonly skin: number;
  readonly cloth: number;
  readonly plate: number;
  readonly plate_dark: number;
  readonly accent: number;
  readonly strap: number;
  readonly crest: number;
  readonly joint: number;
  readonly limbs: number;
  readonly seam: number;
  readonly ink: number;
  readonly sclera: number;
}

// Builders reference `SLOT_INDEX.skin`, `SLOT_INDEX.cloth`, etc. exactly the
// way they used to reference a resolved colour table -- the value baked per
// vertex is now this fixed index rather than a literal colour, and it does
// NOT depend on theme or team, so it is computed once at load time. Built
// off `SLOTS` (the single source of truth for slot order) and cast once to
// the fully-named `SlotIndex` shape.
const slotIndexBuilt: Record<string, number> = {};
SLOTS.forEach((name, i) => {
  slotIndexBuilt[name] = i;
});
/** 0-based: matches the palette array index directly. */
export const SLOT_INDEX: SlotIndex = slotIndexBuilt as unknown as SlotIndex;

// Colours that never vary by theme or team, so they are not part of any
// theme's `color` table. face.ts reaches for these for pupils/brows/mouths
// (`ink`) and eye whites/catchlights (`sclera`) on every figure style.
const CONSTANT_COLOR: Readonly<Record<string, RGB>> = {
  ink: [0.12, 0.11, 0.13],
  sclera: [0.97, 0.97, 0.98],
};

// A theme may leave a slot unset when its own default already reads right --
// e.g. Medieval Fantasy shows bare skin on the forearms, so `limbs` is simply
// not authored.
const SLOT_FALLBACK: Readonly<Record<string, string>> = { limbs: "skin" };

// Resolves every palette slot for one (theme, team) pair into the flat array
// a shader would want: `out[i]` is the RGBA for `SLOTS[i]`.
//
// Every non-constant slot MUST be authored by the theme (directly, or via
// SLOT_FALLBACK) -- this throws rather than substituting a placeholder,
// because a theme whose shape never meshes a slot (e.g. `seam` when
// `shape.seams = false`) still authors an explicit, documented "unused"
// colour for it (see `LIST`). AGENTS.md #7 reserves throwing for invariants
// precisely so this fails loud instead of rendering an opaque black patch.
export function resolvedPalette(theme: Theme, team: Team): readonly RGBA[] {
  const out: RGBA[] = [];
  for (const name of SLOTS) {
    const constant = CONSTANT_COLOR[name];
    if (constant) {
      out.push([constant[0], constant[1], constant[2], 1]);
      continue;
    }
    const color = theme.color as unknown as Readonly<Record<string, ColorSlot | undefined>>;
    let slot = color[name];
    if (slot === undefined) {
      const fallbackName = SLOT_FALLBACK[name];
      if (fallbackName !== undefined) {
        slot = color[fallbackName];
      }
    }
    if (slot === undefined) {
      throw new Error(
        `theme '${theme.key}' has no colour authored for palette slot '${name}' -- add ` +
          `theme.color.${name} (even an explicit, documented placeholder if the theme's shape ` +
          `never meshes it), or add a SLOT_FALLBACK entry for it`,
      );
    }
    const raw = resolve(slot, team);
    out.push([raw[0], raw[1], raw[2], 1]);
  }
  return out;
}

export function byKey(key: string): Theme {
  const found = LIST.find((theme) => theme.key === key);
  if (!found) {
    throw new Error(`unknown theme: ${key}`);
  }
  return found;
}
