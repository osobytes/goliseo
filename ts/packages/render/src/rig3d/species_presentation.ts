// Which rig presentation a species wears.
//
// Two content vocabularies meet here, and this file is the whole of the
// translation between them:
//
//   * The simulation describes a player's look as a species SHAPE
//     (round / broad / angular / cluster) plus a palette. That is what
//     data/species.lua carries and what game/presentation/identity.lua hands
//     the renderer.
//   * The rig describes it as a THEME (shape language, materials, loadout) and
//     a FIGURE (how stylised the body language is), in rig3d/themes.ts.
//
// Neither vocabulary is derivable from the other, so the casting below is a
// content decision, not a computation. Keeping it as one table means changing
// who wears what is a one-line edit rather than a hunt through the renderer.
//
// Proportions deliberately do NOT vary: docs/design/prototype_theme_roster.md
// puts a second production rig out of scope and requires one validated
// skeleton contract. Species read apart through shape language, figure style
// and accent colour -- which is exactly the roster's claim, and the thing
// worth testing.
//
// IDENTITY IS SPLIT IN TWO, and that split is the whole reason this file has
// two key functions rather than one. Before #337 a vertex carried its literal
// colour, so species accent, team and keeper-ness all changed the MESH and had
// to appear in one cache key. Now a vertex carries a palette SLOT INDEX and
// colour is a uniform, so:
//
//   * MESH identity is (theme, figure) and nothing else. Species colour, team
//     and keeper kit no longer touch geometry at all.
//   * COLOUR identity is (dressed theme, dressed team), resolved through
//     themes.resolvedPalette into the small array the shader takes as a
//     uniform. Twenty players share a handful of meshes and differ only here.
//
// INTERIM: #115 owns the six conformed character presentations this will be
// replaced by. Until then this keeps four species distinguishable on the pitch
// instead of twenty identical bodies in two colours.

import type { RGB, RGBA } from "./palette.ts";
import * as themes from "./themes.ts";
import type { Figure, Team, Theme } from "./themes.ts";

/** Which theme and figure one species shape casts onto. */
export interface Casting {
  readonly theme: string;
  readonly figure: string;
}

// Casting, one row per species shape. Each pairs a theme with a figure whose
// silhouette carries that shape's read at match scale.
const CASTING: Readonly<Record<string, Casting>> = {
  // The baseline athletic build: nothing exaggerated, so it anchors the set.
  round: { theme: "scifi", figure: "natural" },
  // Plate over a natural frame is the heaviest silhouette the rig can make.
  broad: { theme: "medieval", figure: "natural" },
  // Hard bevels plus the minifig's cylinder head and tube limbs read angular.
  angular: { theme: "scifi", figure: "minifig" },
  // Molded blobby mass with an oversized head: the least humanoid outline.
  cluster: { theme: "toybox", figure: "vinyl" },
};

const DEFAULT_SHAPE = "round";

// The theme/figure pair one species shape wears. An unknown or absent shape
// casts as `round` rather than erroring: species content lands ahead of rig
// content, and a new shape should read as the baseline build until it is cast.
export function casting(shape?: string): Casting {
  const found = CASTING[shape ?? ""];
  if (found) {
    return found;
  }
  const fallback = CASTING[DEFAULT_SHAPE];
  if (!fallback) {
    throw new Error("unreachable: DEFAULT_SHAPE must be a real casting entry");
  }
  return fallback;
}

// Cache key for one built MESH. Only what changes geometry belongs here, and
// since #337 that is the theme's shape flags and the figure's proportions --
// not team, not keeper kit, not the species accent, all of which are now
// palette entries resolved at draw time.
export function meshKey(shape?: string): string {
  const cast = casting(shape);
  return `${cast.theme}/${cast.figure}`;
}

// Cache key for one resolved PALETTE. Everything that changes a colour
// belongs here -- the species accent included, because two species can share
// a shape and differ only by colour (neutral and terran are both `round`).
export function paletteKey(shape: string | undefined, color: RGB | null, teamKey: string, isKeeper: boolean): string {
  const cast = casting(shape);
  const colorPart = color ? `${color[0].toFixed(3)},${color[1].toFixed(3)},${color[2].toFixed(3)}` : "-";
  return [cast.theme, teamKey, isKeeper ? "gk" : "of", colorPart].join("/");
}

function figureByKey(key: string): Figure {
  const found = themes.FIGURES.find((figure) => figure.key === key);
  if (!found) {
    throw new Error(`unknown figure: ${key}`);
  }
  return found;
}

// The theme and figure tables one species shape is cast onto. This is the
// pair a mesh is built from, and it is exactly what `meshKey` keys.
export function dressing(shape?: string): readonly [Theme, Figure] {
  const cast = casting(shape);
  return [themes.byKey(cast.theme), figureByKey(cast.figure)];
}

// A shallow copy of `theme` with its colour slots adjusted for one player.
//
// Two overrides, both deliberate:
//   * `accent` takes the species palette, so the secondary surface is the
//     same colour the 2.5D renderer used for that species and the HUD still
//     agrees.
//   * A keeper swaps the team's main and trim, which is how a keeper strip
//     actually works -- same club, different shirt. Team ownership stays
//     readable because trim is already a team-owned colour.
//
// The copy stays fully authored: every slot the source theme declared
// survives it, which matters because themes.resolvedPalette now THROWS on a
// missing slot rather than substituting a placeholder. A synthesized theme is
// exactly the case that check exists for, so this must not drop anything.
function dress(theme: Theme, team: Team, color: RGB | null, isKeeper: boolean): readonly [Theme, Team] {
  const dressedColor = color ? { ...theme.color, accent: color } : { ...theme.color };
  const dressedTheme: Theme = { ...theme, color: dressedColor };
  const dressedTeam: Team = isKeeper
    ? { key: team.key, label: team.label, main: team.trim, trim: team.main }
    : team;
  return [dressedTheme, dressedTeam];
}

// The shader palette for one player. Callers cache on `paletteKey`.
//
// `themes.resolve` only ever reads `team.main` / `team.trim`, so the keeper's
// swapped strip needs nothing more than the swapped table `dress` returns.
export function palette(shape: string | undefined, color: RGB | null, team: Team, isKeeper: boolean): readonly RGBA[] {
  const theme = themes.byKey(casting(shape).theme);
  const [dressedTheme, dressedTeam] = dress(theme, team, color, isKeeper);
  return themes.resolvedPalette(dressedTheme, dressedTeam);
}
