import { describe, expect, it } from "vitest";
import * as body from "./body.ts";
import * as geometry from "./geometry.ts";
import { RIG_MEDIUM } from "./proportions.ts";
import * as skeleton from "./skeleton.ts";
import * as themes from "./themes.ts";

const RIG = RIG_MEDIUM;

// #337 slice 2: rigid GPU skinning, expressed here as "rigid CPU-side
// skinning data": every part of a character is folded into ONE geometry; the
// bone that drives a vertex and the material that shades it travel IN the
// vertex instead of in a per-part uniform. All of this is pure TypeScript up
// to the point a `THREE.BufferGeometry`/`SkinnedMesh` would be constructed
// (deferred -- see geometry.ts's header), so the accumulation -- bone
// assignment, material assignment, attach baking, the merge itself -- is
// fully covered headless here, up to but not including that one deferred
// construction call.
describe("rig3d character accumulation (#337 slice 2)", () => {
  const THEME = themes.LIST[0];
  const FIGURE = themes.FIGURES[0];

  it("merges every part into one builder without losing a triangle", () => {
    expect(THEME).toBeDefined();
    expect(FIGURE).toBeDefined();
    if (!THEME || !FIGURE) return;
    const [merged, parts] = body.accumulate(RIG, THEME, FIGURE);
    expect(
      parts.length,
      `the character really is built from many parts: ${parts.length}`,
    ).toBeGreaterThan(1);
    let sum = 0;
    for (const part of parts) {
      sum += part.builder.triangleCount();
    }
    expect(merged.triangleCount(), "the merge must lose nothing and invent nothing").toBe(sum);
    expect(sum, "the character must have geometry").toBeGreaterThan(0);
  });

  it("gives every emitted vertex a bone index inside the skeleton", () => {
    expect(THEME).toBeDefined();
    expect(FIGURE).toBeDefined();
    if (!THEME || !FIGURE) return;
    const [merged] = body.accumulate(RIG, THEME, FIGURE);
    const count = skeleton.boneCount(RIG);
    let lowest = Infinity;
    let highest = -Infinity;
    for (const v of merged.verts) {
      const bone = v.bone;
      expect(
        Number.isInteger(bone) && bone >= 0 && bone < count,
        `bone index out of [0, ${count - 1}]: ${String(bone)}`,
      ).toBe(true);
      lowest = Math.min(lowest, bone);
      highest = Math.max(highest, bone);
    }
    expect(highest, "the character must actually use more than one bone").toBeGreaterThan(lowest);
  });

  it("keeps every part on a bone the skeleton declares", () => {
    expect(THEME).toBeDefined();
    expect(FIGURE).toBeDefined();
    if (!THEME || !FIGURE) return;
    const [, parts] = body.accumulate(RIG, THEME, FIGURE);
    const index = skeleton.boneIndex(RIG);
    for (const part of parts) {
      expect(index[part.bone_name], part.bone_name).toBe(part.bone);
    }
  });

  it("resolves each part's material onto its vertices", () => {
    expect(THEME).toBeDefined();
    expect(FIGURE).toBeDefined();
    if (!THEME || !FIGURE) return;
    const [merged, parts] = body.accumulate(RIG, THEME, FIGURE);
    let at = 0;
    const seen = new Map<number, number>();
    for (const part of parts) {
      const expected = geometry.materialIndex(part.material);
      seen.set(expected, (seen.get(expected) ?? 0) + 1);
      for (let i = 0; i < part.builder.verts.length; i++) {
        const v = merged.verts[at];
        expect(v, `${part.bone_name} material`).toBeDefined();
        if (v) {
          expect(v.material, `${part.bone_name} material`).toBe(expected);
        }
        at += 1;
      }
    }
    expect(at, "every merged vertex must belong to a part").toBe(merged.verts.length);
    // Medieval Fantasy is plate armour over bare skin: it must produce both
    // families, or "material is per vertex" would be untested in practice.
    expect(seen.get(geometry.MATERIAL.plain) ?? 0, "expected plain parts").toBeGreaterThan(0);
    expect(seen.get(geometry.MATERIAL.metal) ?? 0, "expected metal parts").toBeGreaterThan(0);
  });

  it("mixes materials inside one geometry for a theme that glows", () => {
    expect(FIGURE).toBeDefined();
    if (!FIGURE) return;
    // Galactic Sci-Fi has emissive seams and an energy blade. Before slice 2
    // those forced their own draw calls; now all three families share one
    // geometry.
    const scifi = themes.byKey("scifi");
    const [merged] = body.accumulate(RIG, scifi, FIGURE);
    const seen = new Set<number>();
    for (const v of merged.verts) {
      seen.add(v.material);
    }
    for (const name of ["plain", "metal", "emissive"] as const) {
      expect(seen.has(geometry.MATERIAL[name]), `expected ${name} vertices`).toBe(true);
    }
  });

  it("builds every theme x figure pair without a bad bone or material", () => {
    for (const theme of themes.LIST) {
      for (const figure of themes.FIGURES) {
        const [merged, parts] = body.accumulate(RIG, theme, figure);
        expect(
          merged.triangleCount(),
          `${theme.key}/${figure.key} produced no geometry`,
        ).toBeGreaterThan(0);
        expect(parts.length, `${theme.key}/${figure.key} produced no parts`).toBeGreaterThan(0);
      }
    }
  });
});
