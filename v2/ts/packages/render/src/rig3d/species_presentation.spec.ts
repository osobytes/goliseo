// Species -> rig casting is content, and content is exactly what a cache key
// gets wrong quietly: two looks sharing one key means one of them silently
// wears the other's colours, which no amount of staring at the pitch
// resolves.
//
// Everything here is pure table work over themes.ts, so it runs headless --
// unlike a future renderer that calls it.

import { describe, expect, it } from "vitest";
import type { RGB } from "./palette.ts";
import * as speciesPresentation from "./species_presentation.ts";
import * as themes from "./themes.ts";

const HOME = themes.TEAMS[0];
const RED: RGB = [1, 0, 0];
const BLUE: RGB = [0, 0, 1];

describe("rig3d species casting", () => {
  it("casts every authored species shape onto a real theme and figure", () => {
    for (const shape of ["round", "broad", "angular", "cluster"]) {
      const cast = speciesPresentation.casting(shape);
      expect(() => themes.byKey(cast.theme), `${shape} must cast to a known theme`).not.toThrow();
      const found = themes.FIGURES.some((figure) => figure.key === cast.figure);
      expect(found, `${shape} must cast to a known figure: ${cast.figure}`).toBe(true);
    }
  });

  it("falls back to the round build for an unknown or absent shape", () => {
    const round = speciesPresentation.casting("round");
    expect(speciesPresentation.casting(undefined).theme).toBe(round.theme);
    expect(speciesPresentation.casting(undefined).figure).toBe(round.figure);
    expect(speciesPresentation.casting("tesseract").theme).toBe(round.theme);
    expect(speciesPresentation.casting("tesseract").figure).toBe(round.figure);
  });
});

describe("rig3d presentation cache keys", () => {
  it("keys a mesh on geometry alone", () => {
    // Since #337 a vertex carries a palette slot, not a colour. If team,
    // keeper kit or species accent crept back into this key the renderer
    // would rebuild the same geometry once per team for nothing.
    expect(speciesPresentation.meshKey("round")).toBe(speciesPresentation.meshKey("round"));
    expect(speciesPresentation.meshKey("round")).not.toBe(speciesPresentation.meshKey("broad"));
    // round and angular share the scifi theme but not the figure.
    expect(speciesPresentation.meshKey("round")).not.toBe(speciesPresentation.meshKey("angular"));
  });

  it("keys a palette on everything that can change a colour", () => {
    expect(HOME).toBeDefined();
    const away = themes.TEAMS[1];
    expect(away).toBeDefined();
    if (!HOME || !away) return;
    const base = speciesPresentation.paletteKey("round", RED, HOME.key, false);
    expect(base).toBe(speciesPresentation.paletteKey("round", RED, HOME.key, false));
    expect(base, "two species that share a shape must not share a palette key").not.toBe(
      speciesPresentation.paletteKey("round", BLUE, HOME.key, false),
    );
    expect(base).not.toBe(speciesPresentation.paletteKey("round", RED, away.key, false));
    expect(base).not.toBe(speciesPresentation.paletteKey("round", RED, HOME.key, true));
    expect(base).not.toBe(speciesPresentation.paletteKey("broad", RED, HOME.key, false));
    expect(base).not.toBe(speciesPresentation.paletteKey("round", null, HOME.key, false));
  });
});

describe("rig3d species palette", () => {
  function slot(name: string, palette: readonly (readonly number[])[]): readonly number[] {
    const i = themes.SLOTS.indexOf(name);
    if (i < 0) {
      throw new Error(`unknown slot: ${name}`);
    }
    const value = palette[i];
    if (!value) {
      throw new Error(`palette missing slot: ${name}`);
    }
    return value;
  }

  it("resolves a full palette for every cast species", () => {
    expect(HOME).toBeDefined();
    if (!HOME) return;
    // themes.resolvedPalette throws on an unauthored slot, and a dressed
    // theme is a synthesized one -- exactly the case that check guards.
    for (const shape of ["round", "broad", "angular", "cluster"]) {
      const palette = speciesPresentation.palette(shape, RED, HOME, false);
      expect(palette.length, `${shape}: every slot must resolve`).toBe(themes.SLOT_COUNT);
    }
    const fallback = speciesPresentation.palette(undefined, RED, HOME, false);
    expect(fallback.length, "an uncast shape must still resolve every slot").toBe(themes.SLOT_COUNT);
  });

  it("paints the species colour onto the accent surface", () => {
    expect(HOME).toBeDefined();
    if (!HOME) return;
    const palette = speciesPresentation.palette("round", RED, HOME, false);
    const accent = slot("accent", palette);
    expect(accent[0]).toBe(RED[0]);
    expect(accent[1]).toBe(RED[1]);
    expect(accent[2]).toBe(RED[2]);
  });

  it("leaves the theme's own accent alone when a species has no colour", () => {
    expect(HOME).toBeDefined();
    if (!HOME) return;
    const themed = themes.byKey(speciesPresentation.casting("round").theme);
    const palette = speciesPresentation.palette("round", null, HOME, false);
    const accent = slot("accent", palette);
    const expected = themes.resolve(themed.color.accent, HOME);
    expect(accent[0]).toBe(expected[0]);
  });

  it("swaps main and trim for a keeper's strip", () => {
    expect(HOME).toBeDefined();
    if (!HOME) return;
    // Same club, different shirt: the keeper's ownership surface takes the
    // team's trim and the secondary takes its main.
    const outfield = speciesPresentation.palette("round", RED, HOME, false);
    const keeper = speciesPresentation.palette("round", RED, HOME, true);
    // scifi wires `plate` to "team" and `crest` to "trim".
    expect(slot("plate", keeper)[0]).toBe(slot("crest", outfield)[0]);
    expect(slot("crest", keeper)[0]).toBe(slot("plate", outfield)[0]);
  });

  it("does not mutate the shared theme it dresses", () => {
    expect(HOME).toBeDefined();
    if (!HOME) return;
    const themed = themes.byKey(speciesPresentation.casting("round").theme);
    const before = themed.color.accent;
    speciesPresentation.palette("round", RED, HOME, false);
    expect(themed.color.accent, "dressing must copy, never edit themes.LIST").toBe(before);
  });
});
