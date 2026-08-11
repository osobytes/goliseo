// The bridge between `gc-data`'s authored content ids and rig3d's own
// vocabulary (#447).
//
// WHY THIS FILE EXISTS. Three vocabularies describe the same six characters
// and the same six items, and until #447 nothing joined them up:
//
//   1. `crates/gc-data/src/players.rs` gives every player a
//      `presentation_id` (`medieval_rook_emberguard`, `scifi_axi`, ...) and
//      an optional `loadout_id` (`loadout_emberguard_shield`, ...).
//   2. `crates/gc-data/src/character_presentations.rs` maps each
//      presentation onto a `PrototypeThemeId`, and
//      `crates/gc-data/src/loadouts.rs` maps each loadout onto an
//      `equipment_presentation_id` (`medieval_heater_shield`, ...).
//   3. `rig3d/themes.ts` keys its themes `medieval`/`scifi`/`toybox`, and
//      `rig3d/equipment.ts` keys its builders `heater_shield`,
//      `tournament_sword`, `vector_blade`, `pulse_blaster`,
//      `spring_glove`, `foam_champion`.
//
// The renderer speaks (3). The wire carries (1). This module is (2), stated
// in TypeScript.
//
// HAND-MAINTAINED, AND THAT IS NOT AN OVERSIGHT. ARCHITECTURE.md forbids a
// TypeScript package importing a Rust crate's source (§7, and §4 rule 6's
// "TypeScript never imports content tables -- it receives them"), so these
// tables cannot be generated from `gc-data` at build time in this milestone.
// They are therefore a SECOND copy of a mapping `gc-data` already owns, which
// is exactly the drift shape #433 found in the wire enums: each side is
// compiler-checked against itself and neither can see the other. The answer
// is the same one #433 reached for -- an assertion that reads both sources.
// `scripts/check_presentation_parity.mjs` parses the three Rust tables above
// and requires them to agree with the three below, key for key and value for
// value, and `scripts/check.sh` runs it before anything is built.
//
// WHAT THIS DELIBERATELY DOES NOT DO. Two authored presentations share each
// theme (`medieval_rook_emberguard` and `medieval_bramble_quickstep` both map
// to `medieval`), so those two players still render identically to each
// other. Making the six presentations visually distinct is #115's job and
// #115 is parked; inventing a difference here would be authoring content in
// the wrong file. #447 is the WIRING, and the wiring is complete when a
// player's authored presentation and loadout decide their geometry -- which,
// with the three themes that exist, it now does.

import * as themes from "./themes.ts";
import type { Loadout, Theme } from "./themes.ts";

/**
 * `presentation_id` -> `rig3d/themes.ts` theme key.
 *
 * Mirrors `crates/gc-data/src/character_presentations.rs`'s `ALL`: its `id`
 * field is the key here, and its `theme_id` variant is the value, in the
 * `PrototypeThemeId::MedievalFantasy` -> `medieval` naming this package
 * already uses. Every authored presentation must appear.
 */
export const PRESENTATION_THEME: Readonly<Record<string, string>> = {
  medieval_rook_emberguard: "medieval",
  medieval_bramble_quickstep: "medieval",
  scifi_nova_quell: "scifi",
  scifi_axi: "scifi",
  toy_moxie_modular: "toybox",
  toy_tock: "toybox",
};

/**
 * `loadout_id` -> `equipment_presentation_id`.
 *
 * Mirrors `crates/gc-data/src/loadouts.rs`'s `ALL` exactly: `id` is the key,
 * `equipment_presentation_id` is the value. The mechanical `family_id` that
 * table also carries is deliberately absent -- it decides simulation
 * behaviour, and nothing about how the item is drawn.
 */
export const LOADOUT_EQUIPMENT: Readonly<Record<string, string>> = {
  loadout_emberguard_shield: "medieval_heater_shield",
  loadout_tournament_sword: "medieval_tournament_sword",
  loadout_vector_blade: "scifi_energy_blade",
  loadout_pulse_blaster: "scifi_pulse_blaster",
  loadout_spring_gloves: "toy_spring_gloves",
  loadout_foam_champion: "toy_foam_sword",
};

/** Which `themes.Loadout` key an item rides. */
export type LoadoutSlot = "right" | "left" | "hip";

/** One equipment presentation resolved into rig3d's own terms. */
export interface Rig3dEquipment {
  /** A key of `rig3d/equipment.ts`'s `BUILDERS`. */
  readonly id: string;
  /** Which `themes.Loadout` field it occupies. */
  readonly slot: LoadoutSlot;
}

/**
 * `equipment_presentation_id` -> the `rig3d/equipment.ts` builder id and the
 * `themes.Loadout` slot it hangs from.
 *
 * Mirrors `crates/gc-data/src/equipment_presentations.rs`'s `ALL` key set.
 * The `slot` is rig3d's own placement, taken from `body.ts`'s `SOCKETS`
 * table, and it is the RENDERER that is authoritative about it: `gc-data`
 * records `toy_spring_gloves` as `EquipmentAttachment::BothHands` while
 * `body.ts` has exactly one `spring_glove` socket (`hand_l`), and building
 * geometry the rig has no socket for is not an option this table gets to
 * take. `body.buildLoadout` looks the socket up from the item id anyway --
 * these keys only decide which `Loadout` field carries it -- so the two can
 * disagree about handedness without the mesh being wrong; they cannot
 * disagree about WHICH ITEM, and `presentation_content.spec.ts` pins that
 * every id here has a builder and a socket.
 */
export const EQUIPMENT_RIG3D: Readonly<Record<string, Rig3dEquipment>> = {
  medieval_heater_shield: { id: "heater_shield", slot: "left" },
  medieval_tournament_sword: { id: "tournament_sword", slot: "right" },
  scifi_energy_blade: { id: "vector_blade", slot: "right" },
  scifi_pulse_blaster: { id: "pulse_blaster", slot: "hip" },
  toy_spring_gloves: { id: "spring_glove", slot: "left" },
  toy_foam_sword: { id: "foam_champion", slot: "right" },
};

/**
 * The theme one authored `presentation_id` renders as.
 *
 * Throws on an id this table does not know: a presentation reaching the
 * renderer that content never authored is a protocol violation between the
 * Rust producer and this reader, not recoverable input (AGENTS.md §7). The
 * alternative -- silently falling back to `themes.LIST[0]` -- is the exact
 * behaviour #447 exists to remove, and it would remove the evidence with it.
 */
export function themeFor(presentationId: string): Theme {
  const key = PRESENTATION_THEME[presentationId];
  if (key === undefined) {
    throw new Error(
      `presentation_content.ts: no theme for presentation id '${presentationId}' -- ` +
        `add it to PRESENTATION_THEME, mirroring crates/gc-data/src/character_presentations.rs`,
    );
  }
  return themes.byKey(key);
}

/**
 * The `themes.Loadout` one authored `loadout_id` renders as, or the EMPTY
 * loadout when the player carries nothing.
 *
 * `{}` is legal and yields no equipment at all: `body.buildLoadout` reads
 * `Object.values(loadout)` and filters out the undefined ones, so an empty
 * loadout builds headgear and nothing else. That is the keeper rule, and it
 * is why the fix needs no branch on `is_keeper` anywhere -- the data already
 * says it.
 *
 * Throws on a loadout id this table does not know, for the same reason
 * `themeFor` does.
 */
export function loadoutFor(loadoutId: string | undefined): Loadout {
  if (loadoutId === undefined) {
    return {};
  }
  const equipmentId = LOADOUT_EQUIPMENT[loadoutId];
  if (equipmentId === undefined) {
    throw new Error(
      `presentation_content.ts: no equipment for loadout id '${loadoutId}' -- ` +
        `add it to LOADOUT_EQUIPMENT, mirroring crates/gc-data/src/loadouts.rs`,
    );
  }
  const item = EQUIPMENT_RIG3D[equipmentId];
  if (item === undefined) {
    throw new Error(
      `presentation_content.ts: no rig3d equipment for presentation id '${equipmentId}' -- ` +
        `add it to EQUIPMENT_RIG3D, mirroring crates/gc-data/src/equipment_presentations.rs`,
    );
  }
  return { [item.slot]: item.id };
}

/**
 * A stable, order-independent name for one resolved loadout, used as part of
 * a built character's cache key.
 *
 * Sorted so two loadouts holding the same items in different `Loadout`
 * fields cannot produce two cache entries for one geometry, and `"none"`
 * rather than `""` so an empty key is never mistaken for a missing one in a
 * composite key or an error message.
 */
export function loadoutKey(loadout: Loadout): string {
  const ids = Object.values(loadout)
    .filter((id): id is string => id !== undefined)
    .sort();
  return ids.length === 0 ? "none" : ids.join("+");
}
