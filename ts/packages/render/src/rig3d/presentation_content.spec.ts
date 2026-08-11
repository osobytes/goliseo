// #447: the authored content ids reach the geometry, and a keeper carries
// nothing.
//
// WHAT THIS SUITE IS FOR, AND WHAT IT IS NOT. It does NOT re-assert that the
// tables in `presentation_content.ts` agree with `gc-data` -- that is a
// cross-LANGUAGE claim, TypeScript cannot see the Rust source, and asserting
// it here against hand-copied literals would only prove this file agrees with
// its neighbour. `scripts/check_presentation_parity.mjs` reads both sources
// and is what actually pins the mapping (the same division of labour #433
// settled on for the wire enums).
//
// What IS here is everything that can be checked from inside this package:
// that every mapped id resolves to real rig3d content, that the resulting
// geometry differs by theme and by loadout, and -- the assertion issue #447
// explicitly asks for -- that a player with no loadout renders a mesh with no
// equipment vertices on it at all.

import { describe, expect, it } from "vitest";
import * as body from "./body.ts";
import * as equipment from "./equipment.ts";
import {
  EQUIPMENT_RIG3D,
  LOADOUT_EQUIPMENT,
  PRESENTATION_THEME,
  loadoutFor,
  loadoutKey,
  themeFor,
} from "./presentation_content.ts";
import { RIG_MEDIUM } from "./proportions.ts";
import * as skeleton from "./skeleton.ts";
import * as themes from "./themes.ts";

const RIG = RIG_MEDIUM;
// Narrowed into its own non-optional const rather than relying on the guard:
// `noUncheckedIndexedAccess` makes `FIGURES[0]` optional, and a module-level
// narrowing does not follow the binding into the function bodies below.
const FIGURE_OR_UNDEFINED = themes.FIGURES[0];
if (FIGURE_OR_UNDEFINED === undefined) {
  throw new Error("presentation_content.spec.ts: themes.FIGURES must not be empty");
}
const FIGURE: themes.Figure = FIGURE_OR_UNDEFINED;

const BONE_ORDER = skeleton.bones(RIG).map((b) => b.name);

/** Anything hanging off a socket bone: a sword, a shield, a holstered gun. */
const isSocketBone = (bone: string): boolean => bone.startsWith("socket_");

/**
 * The bones a built character actually puts vertices on. `body.accumulate`'s
 * second return is the part list, and every part names its bone -- so this
 * reads what was BUILT rather than re-deriving what should have been.
 */
function bonesWithVertices(theme: themes.Theme, loadout: themes.Loadout): Set<string> {
  const [, parts] = body.accumulate(RIG, theme, FIGURE, loadout);
  const bones = new Set<string>();
  for (const part of parts) {
    if (part.builder.verts.length > 0) {
      bones.add(part.bone_name);
    }
  }
  return bones;
}

/** Bones the merged geometry's vertices ride, read off the vertices. */
function bonesOfMergedVertices(theme: themes.Theme, loadout: themes.Loadout): Set<string> {
  const [merged] = body.accumulate(RIG, theme, FIGURE, loadout);
  const bones = new Set<string>();
  for (const vertex of merged.verts) {
    const name = BONE_ORDER[vertex.bone];
    if (name === undefined) {
      throw new Error(
        `presentation_content.spec.ts: vertex on unknown bone index ${String(vertex.bone)}`,
      );
    }
    bones.add(name);
  }
  return bones;
}

describe("presentation_content: every mapped id resolves to real rig3d content", () => {
  it("gives every authored presentation a theme that themes.ts actually has", () => {
    const ids = Object.keys(PRESENTATION_THEME);
    expect(ids.length, "the six authored character presentations").toBe(6);
    for (const id of ids) {
      // `themeFor` throws on an unmapped id and `themes.byKey` throws on a
      // theme key that does not exist, so reaching a Theme is the assertion.
      expect(themeFor(id).key).toBe(PRESENTATION_THEME[id]);
    }
  });

  it("gives every authored loadout an equipment builder and a socket to hang it from", () => {
    const ids = Object.keys(LOADOUT_EQUIPMENT);
    expect(ids.length, "the six authored fixed loadouts").toBe(6);
    for (const loadoutId of ids) {
      const item = loadoutFor(loadoutId);
      const carried = Object.values(item).filter((id): id is string => id !== undefined);
      expect(carried, `${loadoutId} carries exactly one item`).toHaveLength(1);
      const equipmentId = carried[0] ?? "";
      // THE TWO WAYS THIS CAN BE WRONG, both of which throw deep inside
      // `body.accumulate` -- inside `build()`'s `try`, which disables rigged
      // players for the whole process and logs a warning. Checked here
      // instead, where the message names the id.
      expect(
        () => equipment.build(equipmentId, themes.SLOT_INDEX),
        `${equipmentId} has a builder`,
      ).not.toThrow();
      expect(body.SOCKETS[equipmentId], `${equipmentId} has a socket`).toBeDefined();
    }
  });

  it("maps every equipment presentation onto a distinct rig3d item", () => {
    const items = Object.values(EQUIPMENT_RIG3D).map((e) => e.id);
    expect(items).toHaveLength(6);
    expect(new Set(items).size, "six presentations, six distinct items").toBe(6);
  });

  it("names an empty loadout rather than leaving the key blank", () => {
    expect(loadoutKey({})).toBe("none");
    expect(loadoutKey(loadoutFor(undefined))).toBe("none");
    // Order-independent, so one geometry never gets two cache entries.
    expect(loadoutKey({ right: "b", left: "a" })).toBe(loadoutKey({ left: "a", right: "b" }));
  });

  it("refuses an id content never authored, rather than falling back to the first theme", () => {
    // The fallback IS the defect: `themes.LIST[0]` silently is exactly what
    // made every player medieval. An unknown id must be loud.
    expect(() => themeFor("no_such_presentation")).toThrow(/no theme for presentation id/);
    expect(() => loadoutFor("no_such_loadout")).toThrow(/no equipment for loadout id/);
  });
});

// ---------------------------------------------------------------------------
// THE KEEPER RULE, AT THE GEOMETRY (#447)
// ---------------------------------------------------------------------------
//
// `gc-data/src/players.rs` says "Fixed prototype loadout; keepers have none"
// and `gc-data/tests/players.rs` enforces it in both directions. Until #447
// the render layer implemented none of that: `player_renderer_3d.ts` built
// one geometry from `themes.LIST[0]`, whose authored loadout is a tournament
// sword and a heater shield, and every player on the pitch carried both.
//
// This is the assertion issue #447 asks for, and it is deliberately made on
// the RENDERED VERTICES rather than on the loadout table: what a player sees
// is geometry, and a mapping table that is right while the mesh builder
// ignores it would pass a table-level check.
describe("presentation_content: a player who carries nothing renders nothing on a socket", () => {
  const medieval = themes.byKey("medieval");

  it("puts no vertex on any socket bone when the loadout is empty", () => {
    const bones = bonesOfMergedVertices(medieval, loadoutFor(undefined));
    const sockets = [...bones].filter(isSocketBone);
    expect(sockets, "a keeper's mesh must carry no socket_* vertices at all").toEqual([]);
  });

  // NON-VACUOUS, AND THIS IS THE HALF THAT MATTERS. An "expect nothing"
  // assertion passes most easily when the thing it measures never happens --
  // a `bonesOfMergedVertices` that returned an empty set, a socket naming
  // convention that changed, a `PROP` predicate that stopped matching. The
  // same character WITH a loadout must therefore show the sockets the empty
  // one does not.
  it("puts vertices on exactly the socket the loadout names, for an outfielder", () => {
    const shield = bonesOfMergedVertices(medieval, loadoutFor("loadout_emberguard_shield"));
    expect([...shield].filter(isSocketBone), "the shield rides socket_shield.L").toEqual([
      "socket_shield.L",
    ]);

    const sword = bonesOfMergedVertices(medieval, loadoutFor("loadout_tournament_sword"));
    expect([...sword].filter(isSocketBone), "the sword rides socket_hand.R").toEqual([
      "socket_hand.R",
    ]);

    // The theme's OWN authored loadout still carries both -- that is what
    // every preview and diagnostic entry point renders, and it is what made
    // the defect universal when it was also what every PLAYER rendered.
    const both = bonesOfMergedVertices(medieval, medieval.loadout);
    expect([...both].filter(isSocketBone).sort()).toEqual(["socket_hand.R", "socket_shield.L"]);
  });

  // A holstered sidearm rides `hips`, an ordinary skeletal bone, not a
  // `socket_*` one -- so "no socket vertices" is NOT the same statement as
  // "no equipment". Pinned so nobody reads the test above as broader than it
  // is, and so the empty-loadout case is also checked the direct way: by part
  // count.
  it("drops a hip-mounted item too, which no socket_* check would notice", () => {
    const scifi = themes.byKey("scifi");
    const withBlaster = body.accumulate(RIG, scifi, FIGURE, loadoutFor("loadout_pulse_blaster"))[1];
    const empty = body.accumulate(RIG, scifi, FIGURE, loadoutFor(undefined))[1];
    expect(withBlaster.filter((p) => p.bone_name === "hips").length).toBeGreaterThan(
      empty.filter((p) => p.bone_name === "hips").length,
    );
    expect(withBlaster.length).toBeGreaterThan(empty.length);
  });

  it("still wears the theme's headgear with an empty loadout", () => {
    // Headgear is what a character IS, not what they carry -- see
    // `body.buildLoadout`. A keeper without a helmet would be a different
    // defect introduced by the fix for this one.
    expect(bonesWithVertices(medieval, loadoutFor(undefined)).has("head")).toBe(true);
  });
});

describe("presentation_content: geometry actually varies", () => {
  // The claim #447 makes at its widest: two players with different authored
  // content are no longer the same mesh. Vertex COUNT is a coarse proxy, but
  // it is the one that cannot be satisfied by a cache returning the same
  // object -- which is the failure mode being guarded.
  it("builds a different mesh per theme, and per loadout within a theme", () => {
    const counts = new Map<string, number>();
    for (const themeKey of ["medieval", "scifi", "toybox"]) {
      for (const loadoutId of [undefined, "loadout_emberguard_shield", "loadout_pulse_blaster"]) {
        const loadout = loadoutFor(loadoutId);
        const [merged] = body.accumulate(RIG, themes.byKey(themeKey), FIGURE, loadout);
        counts.set(`${themeKey}|${loadoutKey(loadout)}`, merged.verts.length);
      }
    }
    expect(counts.size).toBe(9);
    expect(
      new Set(counts.values()).size,
      "nine distinct (theme, loadout) pairs, nine distinct meshes",
    ).toBe(9);
  });

  // THE COST, PINNED RATHER THAN DESCRIBED (#447's performance note). Before
  // this change there was ONE geometry for the whole process; now there is
  // one per distinct `(theme, figure, loadout)` on the pitch. The two fixture
  // teams field ten players across NINE distinct variants, so this is the
  // number a reader should have in mind, and it goes red if content authoring
  // makes a match materially more expensive to draw.
  it("keeps the two fixture teams inside nine distinct variants", () => {
    // The ten players `gc-data`'s `nebula` and `orion` rosters field, as
    // (presentation_id, loadout_id) -- mirrored from
    // `crates/gc-data/src/players.rs` and pinned against it by
    // `scripts/check_presentation_parity.mjs`, same as the tables above.
    const pitch: readonly (readonly [string, string | undefined])[] = [
      ["scifi_axi", undefined], // ozzo (keeper)
      ["medieval_rook_emberguard", "loadout_emberguard_shield"], // brakka
      ["toy_moxie_modular", "loadout_tournament_sword"], // veil_nyx
      ["scifi_nova_quell", "loadout_pulse_blaster"], // rok_tann
      ["medieval_bramble_quickstep", "loadout_spring_gloves"], // zyro_vex
      ["toy_tock", undefined], // gax_oru (keeper)
      ["medieval_rook_emberguard", "loadout_emberguard_shield"], // drell
      ["scifi_axi", "loadout_vector_blade"], // morv
      ["toy_moxie_modular", "loadout_pulse_blaster"], // krag
      ["scifi_nova_quell", "loadout_spring_gloves"], // tox_vren
    ];
    const variants = new Set(
      pitch.map(
        ([presentationId, loadoutId]) =>
          `${themeFor(presentationId).key}|${FIGURE.key}|${loadoutKey(loadoutFor(loadoutId))}`,
      ),
    );
    expect(pitch).toHaveLength(10);
    expect(
      variants.size,
      "ten players collapse to nine geometries -- brakka and drell share one",
    ).toBe(9);
  });
});
