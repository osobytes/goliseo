import { describe, expect, it } from "vitest";
import * as themes from "./themes.ts";
import type { Theme } from "./themes.ts";

// A fixed-length tuple read with a runtime bounds check: indexing it with a
// non-literal loop variable widens to `number | undefined` under
// `noUncheckedIndexedAccess`.
function comp(v: readonly number[], i: number): number {
  const x = v[i];
  if (x === undefined) {
    throw new Error(`index ${i} out of range`);
  }
  return x;
}

// #337 slice 1: colour is a palette slot index baked per vertex, resolved
// against a team into a flat RGBA array at "palette-upload time" rather than
// per vertex. `resolvedPalette` and `SLOT_INDEX` are pure TypeScript -- no
// renderer -- so this is real coverage of the changed logic headless, not
// just the geometry around it.
describe("rig3d palette slots (#337)", () => {
  it("has exactly twelve canonical slots", () => {
    expect(themes.SLOTS.length).toBe(12);
    expect(themes.SLOT_COUNT).toBe(12);
  });

  it("SLOT_INDEX is a dense 0-based reindex of SLOTS", () => {
    const index = themes.SLOT_INDEX as unknown as Record<string, number>;
    themes.SLOTS.forEach((name, i) => {
      expect(index[name], name).toBe(i);
    });
  });

  it('resolves the "team" sentinel to the team\'s main colour', () => {
    // Medieval's `cloth` slot is wired to "team" -- the surcoat is the
    // dominant ownership surface.
    const theme = themes.byKey("medieval");
    expect(theme.color.cloth).toBe("team");
    for (const team of themes.TEAMS) {
      const palette = themes.resolvedPalette(theme, team);
      const cloth = palette[themes.SLOT_INDEX.cloth];
      expect(cloth).toBeDefined();
      if (!cloth) continue;
      for (let i = 0; i < 3; i++) {
        expect(comp(cloth, i), `${team.key} cloth[${i}]`).toBeCloseTo(comp(team.main, i), 9);
      }
    }
  });

  it('resolves the "trim" sentinel to the team\'s trim colour', () => {
    // Every theme wires `crest` to "trim" -- the readable secondary used for
    // crests, seams and edge accents.
    const theme = themes.byKey("medieval");
    expect(theme.color.crest).toBe("trim");
    for (const team of themes.TEAMS) {
      const palette = themes.resolvedPalette(theme, team);
      const crest = palette[themes.SLOT_INDEX.crest];
      expect(crest).toBeDefined();
      if (!crest) continue;
      for (let i = 0; i < 3; i++) {
        expect(comp(crest, i), `${team.key} crest[${i}]`).toBeCloseTo(comp(team.trim, i), 9);
      }
    }
  });

  it("falls `limbs` back to `skin` when a theme leaves it unset", () => {
    const theme = themes.byKey("medieval");
    expect(theme.color.limbs, "fixture assumption: medieval leaves limbs unset").toBeUndefined();
    const team = themes.TEAMS[0];
    expect(team).toBeDefined();
    if (!team) return;
    const palette = themes.resolvedPalette(theme, team);
    const limbs = palette[themes.SLOT_INDEX.limbs];
    const skin = palette[themes.SLOT_INDEX.skin];
    expect(limbs).toBeDefined();
    expect(skin).toBeDefined();
    if (!limbs || !skin) return;
    for (let i = 0; i < 4; i++) {
      expect(comp(limbs, i), `limbs[${i}]`).toBeCloseTo(comp(skin, i), 9);
    }
  });

  it("resolves a literal colour slot to exactly the authored value", () => {
    const theme = themes.byKey("scifi");
    const expected = theme.color.plate_dark;
    expect(
      Array.isArray(expected),
      "fixture assumption: plate_dark is a literal, not a sentinel",
    ).toBe(true);
    const team = themes.TEAMS[0];
    expect(team).toBeDefined();
    if (!team || !Array.isArray(expected)) return;
    const palette = themes.resolvedPalette(theme, team);
    const plateDark = palette[themes.SLOT_INDEX.plate_dark];
    expect(plateDark).toBeDefined();
    if (!plateDark) return;
    for (let i = 0; i < 3; i++) {
      expect(comp(plateDark, i), `plate_dark[${i}]`).toBeCloseTo(comp(expected, i), 9);
    }
  });

  it("never varies the constant slots (ink, sclera) by theme or team", () => {
    const teamA = themes.TEAMS[0];
    const teamB = themes.TEAMS[1];
    expect(teamA).toBeDefined();
    expect(teamB).toBeDefined();
    if (!teamA || !teamB) return;
    const a = themes.resolvedPalette(themes.byKey("medieval"), teamA);
    const b = themes.resolvedPalette(themes.byKey("toybox"), teamB);
    for (const name of ["ink", "sclera"] as const) {
      const idx = themes.SLOT_INDEX[name];
      const av = a[idx];
      const bv = b[idx];
      expect(av).toBeDefined();
      expect(bv).toBeDefined();
      if (!av || !bv) continue;
      for (let i = 0; i < 4; i++) {
        expect(comp(av, i), `${name}[${i}]`).toBeCloseTo(comp(bv, i), 9);
      }
    }
  });

  it("resolves every theme x team pair to exactly SLOT_COUNT RGBA entries", () => {
    for (const theme of themes.LIST) {
      for (const team of themes.TEAMS) {
        const palette = themes.resolvedPalette(theme, team);
        expect(palette.length, `${theme.key}/${team.key}`).toBe(themes.SLOT_COUNT);
        palette.forEach((rgba, i) => {
          expect(rgba.length, `${theme.key}/${team.key} slot ${i}`).toBe(4);
        });
      }
    }
  });

  it("fails loud instead of silently defaulting when a theme leaves a slot unauthored", () => {
    // A theme missing `crest` entirely, with no SLOT_FALLBACK entry for it,
    // must not render an inert black placeholder -- AGENTS.md #7: throw on
    // invariant violations.
    const brokenTheme = {
      key: "broken-fixture",
      color: {
        skin: [0.5, 0.5, 0.5],
        cloth: [0.5, 0.5, 0.5],
        plate: [0.5, 0.5, 0.5],
        plate_dark: [0.5, 0.5, 0.5],
        accent: [0.5, 0.5, 0.5],
        strap: [0.5, 0.5, 0.5],
        // crest deliberately omitted
        joint: [0.5, 0.5, 0.5],
        limbs: [0.5, 0.5, 0.5],
        seam: [0.5, 0.5, 0.5],
      },
      // The rest of `Theme` is irrelevant to resolvedPalette; this fixture is
      // deliberately malformed (missing `crest`), the exact case the runtime
      // check exists for -- hence the cast rather than a fully authored Theme.
    } as unknown as Theme;
    const team = themes.TEAMS[0];
    expect(team).toBeDefined();
    if (!team) return;
    expect(() => themes.resolvedPalette(brokenTheme, team)).toThrowError(/crest/);
  });
});
