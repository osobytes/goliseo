// The per-player presentation payload `pitch.ts` derives from one
// `RenderFrame` and hands to the player renderer, plus the closed sets its
// fields draw from.
//
// WHY THIS IS ITS OWN MODULE. These types used to live in
// `player_renderer.ts`, the procedural billboard avatar, because that module
// was the first consumer written. It is gone (#415): the rigged
// `THREE.SkinnedMesh` path is the only way a player is ever drawn, and a
// second, silently-substituted renderer is exactly the diagnosability problem
// that removal set out to fix. The TYPES outlived it -- `player_renderer_3d.ts`
// and `rig3d/pose_lod.ts` both take a `PlayerRenderOptions`, and
// `frame_buffer.ts` decodes the wire codes behind `SpeciesShape`/`AerialStyle`/
// `AerialOutcome` -- so they sit here, in a module that declares shapes and
// nothing else, rather than in whichever renderer happens to be current.
//
// Boundary note (ARCHITECTURE.md §4 rule 6): `PlayerPoseSelection` mirrors the
// Rust RenderFrame producer's pose selection (`render/**` -> `crates/gc-render`)
// -- only the fields this package's renderers read are declared here.
// `CombatPlayerPresentation` is reused from `@gc/presentation` (a declared
// dependency).

import type { Vec2 } from "@gc/core";
import type { CombatPlayerPresentation } from "@gc/presentation";
import type { RGB } from "./draw2d.ts";

export type AerialStyle = "leg_control" | "chest_control" | "volley" | "header" | "bicycle";
export type AerialOutcome = "clean" | "heavy" | "miss";
export type SpeciesShape = "round" | "broad" | "angular" | "cluster";

/** The slice of the Rust RenderFrame producer's pose selection the renderers read. */
export interface PlayerPoseSelection {
  readonly id?: string;
  readonly priority?: number;
  readonly source?: string;
}

/**
 * One player's presentation inputs for one frame.
 *
 * NOT EVERY FIELD HAS A READER TODAY. `is_keeper`, `dashing`, `grab`,
 * `aerial_outcome`, `species_shape`, `species_color` and `combat` were read by
 * the procedural billboard and by nothing else; with it deleted (#415) they are
 * carried from the wire, through `pitch.ts`'s `playerAnchor`, and dropped. They
 * are deliberately kept rather than pruned here: they are produced by the Rust
 * `crates/gc-render` frame builder, decoded by `frame_buffer.ts`, and pinned by
 * `pitch.spec.ts`'s wire-fixture differential, so removing them is a
 * wire-format change spanning both languages rather than a TypeScript
 * tidy-up. See #415's report.
 */
export interface PlayerRenderOptions {
  readonly facing?: Vec2;
  readonly is_keeper: boolean;
  readonly controlled: boolean;
  readonly dashing?: boolean;
  readonly dive?: number;
  readonly dive_dir?: Vec2;
  readonly holding?: boolean;
  readonly grab?: number;
  readonly throw?: number;
  readonly windup?: number;
  readonly aerial?: number;
  readonly aerial_style?: AerialStyle;
  readonly aerial_outcome?: AerialOutcome;
  readonly aerial_jump?: number;
  readonly species_shape?: SpeciesShape;
  readonly species_color?: RGB;
  readonly team?: "home" | "away";
  /**
   * The authored `gc-data` character presentation id (#447) -- which of the
   * six authored presentations this player is, and therefore which theme's
   * geometry the renderer builds them from.
   *
   * ABSENT MEANS "NOT ON THE PRODUCT PATH". Every roster slot carries one
   * (`crates/gc-render`'s `encode_roster` asserts it non-empty), so a
   * `PlayerRenderOptions` without one is a preview, a diagnostic entry point
   * or a test fixture. `player_renderer_3d.ts` answers that with an explicit
   * named default variant rather than guessing -- see `variantFor` there.
   */
  readonly presentation_id?: string;
  /**
   * The authored `gc-data` loadout id (#447), or absent where the player
   * carries nothing.
   *
   * ABSENT MEANS TWO DIFFERENT THINGS, AND `presentation_id` DISAMBIGUATES
   * THEM. Alongside a `presentation_id` it is the keeper rule: this player is
   * authored to carry nothing, so they render with an empty loadout. Without
   * one, nothing was wired at all and the renderer's preview default applies.
   * Collapsing the two is exactly how a keeper ended up holding a shield, so
   * they are kept distinct.
   */
  readonly loadout_id?: string;
  readonly combat?: CombatPlayerPresentation;
  readonly pose?: PlayerPoseSelection;
}
