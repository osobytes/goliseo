import { mat4 } from "@gc/core";
import { describe, expect, it } from "vitest";
import * as geometry from "./geometry.ts";
import { PartBuilder } from "./geometry.ts";
import * as themes from "./themes.ts";

// meshbuilder.lua no longer bakes a literal colour: every vertex carries a
// slot index instead, and rejects anything that isn't one -- coverage for
// the invariant the #337 review specifically asked to see exercised. This
// file is geometry.ts's spec, the v2 replacement for meshbuilder.lua's and
// shapes.lua's tests -- see geometry.ts's header for what changed and why.
describe("rig3d part builder palette-slot vertices (#337)", () => {
  it("bakes a numeric slot index per vertex instead of a literal colour", () => {
    const mb = new PartBuilder();
    mb.triangle(null, [0, 0, 0], [1, 0, 0], [0, 1, 0], themes.SLOT_INDEX.skin);
    expect(mb.verts.length).toBe(3);
    for (const v of mb.verts) {
      expect(v.paletteSlot).toBe(themes.SLOT_INDEX.skin);
    }
  });

  it("quad() propagates the same slot to both triangles", () => {
    const mb = new PartBuilder();
    mb.quad(null, [0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], themes.SLOT_INDEX.plate);
    expect(mb.verts.length).toBe(6);
    for (const v of mb.verts) {
      expect(v.paletteSlot).toBe(themes.SLOT_INDEX.plate);
    }
  });

  it("rejects a literal colour table now that vertices carry a slot index", () => {
    const mb = new PartBuilder();
    const colorTable = [1, 0, 0, 1] as unknown as number;
    expect(() => mb.triangle(null, [0, 0, 0], [1, 0, 0], [0, 1, 0], colorTable)).toThrowError();
  });

  it("rejects a non-finite slot", () => {
    const mb = new PartBuilder();
    const missing = undefined as unknown as number;
    expect(() => mb.triangle(null, [0, 0, 0], [1, 0, 0], [0, 1, 0], missing)).toThrowError();
  });
});

// #337 slice 2's vertex format (position/texcoord/normal/slot/bone/material
// as 11 LÖVE-attribute floats) is NOT ported: it existed to satisfy
// `love.graphics.newMesh`'s vertex-format API and GLSL ES 1.00's lack of
// integer vertex attributes, neither of which apply once geometry becomes a
// `THREE.BufferGeometry` with typed attributes. What survives, because it is
// content-relevant rather than LÖVE-specific, is the MATERIAL ordering.
describe("rig3d material ids (#337 slice 2)", () => {
  it.skip(
    "carries a bone index and a material alongside the palette slot, as named " +
      "float vertex attributes -- dropped: `VERTEX_FORMAT` was LÖVE's " +
      "`love.graphics.newMesh` attribute declaration (attribute names, and " +
      "every index as a float because GLSL ES 1.00 has no integer vertex " +
      "attributes). A `THREE.BufferGeometry` attribute (skinIndex/skinWeight, " +
      "a custom palette-slot attribute) has no equivalent named-format API to " +
      "test, and building it is deferred pending @types/three (see this " +
      "package's port report).",
    () => {
      // Intentionally not ported; see skip reason above.
    },
  );

  it.skip(
    "emits one value per format component, in format order -- dropped for the " +
      "same reason as the attribute-declaration test above.",
    () => {
      // Intentionally not ported; see skip reason above.
    },
  );

  it("maps material names to the ids a shader (or a THREE.Material[] group index) would branch on", () => {
    expect(geometry.materialIndex(undefined)).toBe(geometry.MATERIAL.plain);
    expect(geometry.materialIndex("metal")).toBe(geometry.MATERIAL.metal);
    expect(geometry.materialIndex("emissive")).toBe(geometry.MATERIAL.emissive);
    // Ordering is load-bearing wherever a consumer thresholds on it (the old
    // shader tested `v_material > 1.5` for emissive, `> 0.5` for metal); keep
    // it, in case a future three.js integration inherits the same pattern.
    expect(geometry.MATERIAL.plain).toBeLessThan(geometry.MATERIAL.metal);
    expect(geometry.MATERIAL.metal).toBeLessThan(geometry.MATERIAL.emissive);
  });

  it("rejects an unknown material rather than silently shading it plain", () => {
    expect(() => geometry.materialIndex("chrome")).toThrowError();
  });
});

describe("rig3d part merge (#337 slice 2)", () => {
  function tri(mb: PartBuilder, slot: number, y: number): void {
    mb.triangle(null, [0, y, 0], [1, y, 0], [0, y + 1, 0], slot);
  }

  it("stamps each part's bone index and material onto every one of its vertices", () => {
    const a = new PartBuilder();
    const b = new PartBuilder();
    tri(a, themes.SLOT_INDEX.skin, 0);
    tri(b, themes.SLOT_INDEX.plate, 0);
    const merged = geometry.merge([
      { builder: a, bone: 4 },
      { builder: b, bone: 17, material: "emissive" },
    ]);
    expect(merged.verts.length).toBe(6);
    for (let i = 0; i < 3; i++) {
      const v = merged.verts[i];
      expect(v).toBeDefined();
      if (!v) continue;
      expect(v.bone, "bone index").toBe(4);
      expect(v.material, "material").toBe(geometry.MATERIAL.plain);
      expect(v.paletteSlot, "palette slot survives the merge").toBe(themes.SLOT_INDEX.skin);
    }
    for (let i = 3; i < 6; i++) {
      const v = merged.verts[i];
      expect(v).toBeDefined();
      if (!v) continue;
      expect(v.bone, "bone index").toBe(17);
      expect(v.material, "material").toBe(geometry.MATERIAL.emissive);
      expect(v.paletteSlot, "palette slot survives the merge").toBe(themes.SLOT_INDEX.plate);
    }
  });

  it("preserves part order, so triangles rasterise in submission order", () => {
    const a = new PartBuilder();
    const b = new PartBuilder();
    tri(a, themes.SLOT_INDEX.skin, 10);
    tri(b, themes.SLOT_INDEX.skin, 20);
    const merged = geometry.merge([
      { builder: a, bone: 0 },
      { builder: b, bone: 1 },
    ]);
    const first = merged.verts[0];
    const fourth = merged.verts[3];
    expect(first).toBeDefined();
    expect(fourth).toBeDefined();
    if (!first || !fourth) return;
    expect(first.position[1]).toBeCloseTo(10, 12);
    expect(fourth.position[1]).toBeCloseTo(20, 12);
  });

  it("bakes a part's attach transform into its vertices", () => {
    // The static per-part offset is what used to force a per-part model
    // matrix. Once it lives in the vertex the bone matrix alone drives it.
    const plain = new PartBuilder();
    const attached = new PartBuilder();
    tri(plain, themes.SLOT_INDEX.skin, 0);
    tri(attached, themes.SLOT_INDEX.skin, 0);
    const shift = mat4.translation(5, -2, 3);
    const merged = geometry.merge([
      { builder: plain, bone: 0 },
      { builder: attached, bone: 0, attach: shift },
    ]);
    for (let i = 0; i < 3; i++) {
      const before = merged.verts[i];
      const after = merged.verts[i + 3];
      expect(before).toBeDefined();
      expect(after).toBeDefined();
      if (!before || !after) continue;
      expect(after.position[0]).toBeCloseTo(before.position[0] + 5, 12);
      expect(after.position[1]).toBeCloseTo(before.position[1] - 2, 12);
      expect(after.position[2]).toBeCloseTo(before.position[2] + 3, 12);
      // A pure translation must leave the normal alone.
      expect(after.normal[0]).toBeCloseTo(before.normal[0], 12);
      expect(after.normal[1]).toBeCloseTo(before.normal[1], 12);
      expect(after.normal[2]).toBeCloseTo(before.normal[2], 12);
    }
  });

  it("rotates normals with an attach and keeps them unit length", () => {
    const part = new PartBuilder();
    tri(part, themes.SLOT_INDEX.skin, 0);
    const merged = geometry.merge([{ builder: part, bone: 0, attach: mat4.rotationX((37 * Math.PI) / 180) }]);
    for (const v of merged.verts) {
      const [nx, ny, nz] = v.normal;
      const len = Math.sqrt(nx * nx + ny * ny + nz * nz);
      expect(len, "normal must stay unit length through an attach").toBeCloseTo(1, 9);
    }
  });

  it("rejects a part with no valid bone index", () => {
    // `Part.bone` is a required `number` at compile time, so this exercises
    // the runtime guard the same way malformed external/content data would:
    // via a cast, not a type the compiler would otherwise accept.
    const part = new PartBuilder();
    tri(part, themes.SLOT_INDEX.skin, 0);
    const malformed = { builder: part } as unknown as geometry.Part;
    expect(() => geometry.merge([malformed])).toThrowError();
  });

  it.skip(
    "refuses to build a mesh from an unmerged part builder -- dropped: there " +
      "is no `.build()` on `PartBuilder` in this milestone. The Lua original " +
      "gated `love.graphics.newMesh`; constructing a `THREE.BufferGeometry` / " +
      "`THREE.SkinnedMesh` is a live-renderer-adjacent step out of scope here " +
      "(v2/README.md #1) and blocked on `@types/three` (see the port report).",
    () => {
      // Intentionally not ported; see skip reason above.
    },
  );
});
