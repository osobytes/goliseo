// New tests for draw2d.ts's pure half (see that file's header). Not a port
// of a Lua spec -- this is new shared infrastructure this package's port
// needed, so there is no Lua original to mirror.
//
// The "PAINT / APPENDCOMMANDS" suite below additionally exercises part of
// the impure half: every non-text `DrawCommand` kind builds through real
// three.js constructors that need no DOM (`PlaneGeometry`, `CircleGeometry`,
// `ShapeGeometry`, `BufferGeometry`, plain `Line`/`Mesh`), so those are not
// actually GPU- or DOM-shaped and were previously untested only because they
// sat in the same file as the one thing that IS DOM-shaped (`buildTextSprite`,
// via `document.createElement("canvas")`). `PaintOptions.buildText` lets the
// text command be substituted so this suite can cover the rest without a DOM
// polyfill -- see match_hud.spec.ts's population suite for the same seam
// used against the HUD specifically (draw2d.ts's `PaintOptions` doc comment
// explains the choice).

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { appendCommands, clearMaterialCache, disposeObject, materialCacheSize, paint, DrawList, rotateAround, type DrawCommand, type PivotTransform, type TextCommand } from "./draw2d.ts";

function near(actual: number, expected: number, eps = 1e-9): void {
  expect(Math.abs(actual - expected)).toBeLessThanOrEqual(eps);
}

describe("rotateAround", () => {
  it("leaves the pivot itself fixed under any rotation", () => {
    const t: PivotTransform = { pivotX: 10, pivotY: 20, angle: Math.PI / 3, offsetX: 0, offsetY: 0 };
    const [x, y] = rotateAround(t, 10, 20);
    near(x, 10);
    near(y, 20);
  });

  it("rotates a point 90 degrees around the pivot", () => {
    // (pivot + (1, 0)) rotated 90deg CCW-in-math-convention becomes (pivot + (0, 1)).
    const t: PivotTransform = { pivotX: 0, pivotY: 0, angle: Math.PI / 2, offsetX: 0, offsetY: 0 };
    const [x, y] = rotateAround(t, 1, 0);
    near(x, 0);
    near(y, 1);
  });

  it("applies the offset after rotation, matching push/translate/rotate/translate composition order", () => {
    const t: PivotTransform = { pivotX: 5, pivotY: 5, angle: 0, offsetX: 3, offsetY: -2 };
    const [x, y] = rotateAround(t, 5, 5);
    near(x, 8);
    near(y, 3);
  });

  it("matches the dive lunge transform's shape: rotate about the feet, then travel along the dive axis", () => {
    // Mirrors player_renderer.lua's push(); translate(sx + sign*r*travel*amount, gy);
    // rotate(angle); translate(-sx, -gy) for a keeper_spread dive to the right.
    const sx = 100;
    const gy = 200;
    const r = 12;
    const sign = 1;
    const travel = 0.65;
    const amount = 1;
    const angle = ((28 * Math.PI) / 180) * amount;
    const t: PivotTransform = { pivotX: sx, pivotY: gy, angle, offsetX: sign * r * travel * amount, offsetY: 0 };
    // A point directly "above" the feet (head, at -r on y) should end up
    // rotated toward the dive direction and translated along it.
    const [x, y] = rotateAround(t, sx, gy - r);
    const dy = -r; // (gy - r) - gy
    const c = Math.cos(angle);
    const s = Math.sin(angle);
    near(x, sx + sign * r * travel * amount + -dy * s);
    near(y, gy + dy * c);
  });
});

describe("DrawList", () => {
  it("records commands in call order, matching a sequence of love.graphics calls", () => {
    const dl = new DrawList();
    dl.rect("fill", 0, 0, 10, 10, [1, 0, 0]);
    dl.circle("fill", 5, 5, 3, [0, 1, 0]);
    dl.line([0, 0, 10, 10], [0, 0, 1]);
    expect(dl.commands.map((c) => c.kind)).toEqual(["rect", "circle", "line"]);
  });

  it("omits optional fields entirely rather than writing them as undefined (exactOptionalPropertyTypes)", () => {
    const dl = new DrawList();
    dl.circle("fill", 0, 0, 1, [1, 1, 1]);
    const command = dl.commands[0];
    expect(command).toBeDefined();
    expect(command && "alpha" in command).toBe(false);
  });

  it("transforms a circle's centre under a PivotTransform but keeps its radius", () => {
    const dl = new DrawList();
    const t: PivotTransform = { pivotX: 0, pivotY: 0, angle: Math.PI / 2, offsetX: 0, offsetY: 0 };
    dl.circle("fill", 1, 0, 7, [1, 1, 1], undefined, t);
    const command = dl.commands[0];
    if (command === undefined || command.kind !== "circle") {
      throw new Error("expected a circle command");
    }
    near(command.x, 0);
    near(command.y, 1);
    expect(command.r).toBe(7);
  });

  it("degrades a rotated rect to its four transformed corners as a polygon", () => {
    const dl = new DrawList();
    const t: PivotTransform = { pivotX: 0, pivotY: 0, angle: Math.PI / 2, offsetX: 0, offsetY: 0 };
    dl.rect("fill", 0, 0, 2, 4, [1, 1, 1], undefined, t);
    const command = dl.commands[0];
    if (command === undefined || command.kind !== "polygon") {
      throw new Error("expected a rotated rect to degrade to a polygon");
    }
    expect(command.points.length).toBe(8);
  });

  it("rotates an arc's angle range along with its centre", () => {
    const dl = new DrawList();
    const t: PivotTransform = { pivotX: 0, pivotY: 0, angle: Math.PI / 4, offsetX: 0, offsetY: 0 };
    dl.arc("fill", 0, 0, 5, 0, Math.PI, [1, 1, 1], undefined, t);
    const command = dl.commands[0];
    if (command === undefined || command.kind !== "arc") {
      throw new Error("expected an arc command");
    }
    near(command.angle1, Math.PI / 4);
    near(command.angle2, Math.PI / 4 + Math.PI);
  });

  it("extend() concatenates another command list in order", () => {
    const a = new DrawList();
    a.circle("fill", 0, 0, 1, [1, 0, 0]);
    const b = new DrawList();
    b.circle("fill", 1, 1, 1, [0, 1, 0]);
    b.extend(a.commands);
    expect(b.commands.map((c) => (c.kind === "circle" ? c.x : undefined))).toEqual([1, 0]);
  });
});

const stubText = (c: TextCommand): THREE.Object3D => {
  const o = new THREE.Object3D();
  o.name = `text:${c.text}`;
  return o;
};

function nonTextCommands(): readonly DrawCommand[] {
  const dl = new DrawList();
  dl.rect("fill", 0, 0, 10, 10, [1, 0, 0]);
  dl.rect("line", 0, 0, 10, 10, [1, 0, 0]);
  dl.circle("fill", 5, 5, 3, [0, 1, 0]);
  dl.circle("line", 5, 5, 3, [0, 1, 0]);
  dl.ellipse("fill", 5, 5, 3, 2, [0, 0, 1]);
  dl.polygon("fill", [0, 0, 1, 0, 1, 1], [1, 1, 0]);
  dl.line([0, 0, 10, 10], [0, 0, 1]);
  dl.arc("fill", 0, 0, 5, 0, Math.PI, [1, 0, 1]);
  return dl.commands;
}

describe("appendCommands", () => {
  it("adds one three.js object per command, in order, without clearing existing children", () => {
    const group = new THREE.Group();
    group.add(new THREE.Object3D());
    const commands = nonTextCommands();
    appendCommands(group, commands);
    expect(group.children).toHaveLength(1 + commands.length);
  });

  it("routes a text command through PaintOptions.buildText instead of document.createElement", () => {
    const group = new THREE.Group();
    const dl = new DrawList();
    dl.text("SCORE", 0, 0, 100, "left", [1, 1, 1]);
    expect(() => appendCommands(group, dl.commands, { buildText: stubText })).not.toThrow();
    expect(group.children.map((c) => c.name)).toEqual(["text:SCORE"]);
  });
});

describe("appendCommands depth placement (PaintOptions.z/renderOrder/depthTest)", () => {
  it("leaves every built object at z=0 with depthTest untouched when no depth options are given, matching every existing caller", () => {
    const group = new THREE.Group();
    appendCommands(group, nonTextCommands());
    for (const child of group.children) {
      expect(child.position.z).toBe(0);
      expect(child.renderOrder).toBe(0);
    }
  });

  it("offsets every built object's position.z by PaintOptions.z", () => {
    const group = new THREE.Group();
    appendCommands(group, nonTextCommands(), { z: 0.35 });
    expect(group.children.length).toBeGreaterThan(0);
    for (const child of group.children) {
      expect(child.position.z).toBeCloseTo(0.35);
    }
  });

  it("forwards PaintOptions.renderOrder to every built object", () => {
    const group = new THREE.Group();
    appendCommands(group, nonTextCommands(), { renderOrder: 10 });
    for (const child of group.children) {
      expect(child.renderOrder).toBe(10);
    }
  });

  it("forwards PaintOptions.depthTest to every built object's material(s), including line-mode commands", () => {
    const group = new THREE.Group();
    appendCommands(group, nonTextCommands(), { depthTest: false });
    for (const child of group.children) {
      if (child instanceof THREE.Mesh || child instanceof THREE.Line) {
        const materials = Array.isArray(child.material) ? child.material : [child.material];
        for (const m of materials) {
          expect(m.depthTest).toBe(false);
        }
      }
    }
  });

  it("paint() forwards the same depth options as appendCommands (paint is appendCommands preceded by a clear)", () => {
    const group = new THREE.Group();
    paint(group, nonTextCommands(), { z: 1.5, renderOrder: 3, depthTest: false });
    expect(group.children.length).toBeGreaterThan(0);
    for (const child of group.children) {
      expect(child.position.z).toBeCloseTo(1.5);
      expect(child.renderOrder).toBe(3);
    }
  });

  it("composes PaintOptions.z with a non-zero base z rather than overwriting it (e.g. a circle-line command's translateX/Y chain)", () => {
    const group = new THREE.Group();
    const dl = new DrawList();
    dl.circle("line", 5, 5, 3, [0, 1, 0]);
    appendCommands(group, dl.commands, { z: 2 });
    const child = group.children[0];
    expect(child).toBeDefined();
    expect(child?.position.z).toBeCloseTo(2);
  });
});

describe("paint", () => {
  it("clears previously painted children before adding the new ones (does not accumulate)", () => {
    const group = new THREE.Group();
    paint(group, nonTextCommands());
    const first = group.children.length;
    expect(first).toBeGreaterThan(0);
    paint(group, nonTextCommands());
    expect(group.children).toHaveLength(first);
  });

  it("disposes a cleared child's geometry and material", () => {
    const group = new THREE.Group();
    const geometry = new THREE.PlaneGeometry(1, 1);
    const material = new THREE.MeshBasicMaterial();
    let geometryDisposed = false;
    let materialDisposed = false;
    geometry.dispose = () => {
      geometryDisposed = true;
    };
    material.dispose = () => {
      materialDisposed = true;
    };
    group.add(new THREE.Mesh(geometry, material));

    paint(group, []);

    expect(geometryDisposed).toBe(true);
    expect(materialDisposed).toBe(true);
  });

  it("accepts a text command via PaintOptions.buildText without touching document", () => {
    const group = new THREE.Group();
    const dl = new DrawList();
    dl.rect("fill", 0, 0, 10, 10, [1, 0, 0]);
    dl.text("VENUE", 0, 0, 100, "center", [1, 1, 1]);
    paint(group, dl.commands, { buildText: stubText });
    expect(group.children).toHaveLength(2);
    expect(group.children.some((c) => c.name === "text:VENUE")).toBe(true);
  });
});

describe("disposeObject", () => {
  it("disposes a Sprite's material and its map texture", () => {
    const texture = new THREE.Texture();
    const material = new THREE.SpriteMaterial({ map: texture });
    let textureDisposed = false;
    let materialDisposed = false;
    texture.dispose = () => {
      textureDisposed = true;
    };
    material.dispose = () => {
      materialDisposed = true;
    };
    const sprite = new THREE.Sprite(material);

    disposeObject(sprite);

    expect(textureDisposed).toBe(true);
    expect(materialDisposed).toBe(true);
  });

  it("releases a userData.ownedRenderTarget when present, for objects paint()/appendCommands() never built directly", () => {
    const target = new THREE.WebGLRenderTarget(4, 4);
    let targetDisposed = false;
    target.dispose = () => {
      targetDisposed = true;
    };
    const holder = new THREE.Object3D();
    holder.userData["ownedRenderTarget"] = target;

    disposeObject(holder);

    expect(targetDisposed).toBe(true);
  });

  it("is a no-op for a plain Object3D carrying no owned resources", () => {
    expect(() => disposeObject(new THREE.Object3D())).not.toThrow();
  });
});

// The material cache is a PERFORMANCE fix with a correctness hazard on either
// side, so both sides get pinned here. Profiling the real app
// (scripts/v2_frame_profile.py) measured 3905 `gl.linkProgram` calls in 25
// seconds -- ~2.6 shader compiles every frame, with `getProgramInfoLog`
// blocking 3572ms of that window -- because `paint` disposed every material
// each frame, dropping each compiled program's refcount to zero. See draw2d.ts's
// MATERIAL CACHE note for the mechanism.
//
// Hazard on one side: if `paint` still disposes cached materials, the compiles
// come straight back (and silently -- nothing about the picture changes).
// Hazard on the other: if the cache key is too coarse, two commands with
// DIFFERENT colours share one material and the second silently repaints the
// first. Both are invisible to every other test in this file.
describe("draw2d material cache", () => {
  const cmd = (color: readonly [number, number, number], alpha?: number): DrawCommand =>
    ({ kind: "rect", mode: "fill", x: 0, y: 0, w: 10, h: 10, color, ...(alpha === undefined ? {} : { alpha }) }) as DrawCommand;

  it("reuses one material across frames instead of disposing it, so the program is never released", () => {
    clearMaterialCache();
    const group = new THREE.Group();
    paint(group, [cmd([1, 0, 0])]);
    const first = (group.children[0] as THREE.Mesh).material as THREE.Material;

    // The frame boundary: `paint` clears and rebuilds, which is exactly where
    // the old code disposed the material and forced the recompile.
    paint(group, [cmd([1, 0, 0])]);
    const second = (group.children[0] as THREE.Mesh).material as THREE.Material;

    expect(second, "the same command must reuse the same material instance").toBe(first);
    // `Material.dispose()` is what calls three.js's `releaseProgram`; a
    // disposed material would have had its `version` bumped past use. The
    // observable proxy is that the instance survived the repaint at all, which
    // the identity check above already establishes -- this asserts it is still
    // attached and usable rather than a zombie.
    expect(first.userData["gcCachedMaterial"]).toBe(true);
  });

  it("does not share a material between different colours or alphas", () => {
    clearMaterialCache();
    const group = new THREE.Group();
    paint(group, [cmd([1, 0, 0]), cmd([0, 1, 0]), cmd([1, 0, 0], 0.5)]);
    const materials = group.children.map((child) => (child as THREE.Mesh).material);
    expect(new Set(materials).size, "three distinct visual configs, three materials").toBe(3);
  });

  it("keeps the cache BOUNDED under continuously varying alpha", () => {
    // Callers pass continuous alphas (pulsing rings use `0.3 * hk`), so an
    // unquantised key would grow the cache without limit -- trading a shader
    // recompile for a memory leak is not a fix. Quantisation caps it at the
    // number of distinct 1/255 steps.
    clearMaterialCache();
    const group = new THREE.Group();
    for (let i = 0; i < 4000; i += 1) {
      paint(group, [cmd([1, 0, 0], (i % 1000) / 1000)]);
    }
    expect(materialCacheSize()).toBeLessThanOrEqual(256);
  });

  it("does not let an overlay's depthTest:false leak onto depth-tested draws with the same colour", () => {
    // The hazard a shared material introduces that a per-frame material never
    // could. `applyDepthPlacement` WRITES `depthTest` onto the material after
    // construction, and pitch.ts's overlay pass paints with `depthTest: false`
    // (`drawPitchAfterItems`). With `depthTest` missing from the cache key, the
    // overlay and the ordinary depth-tested pass shared one instance, so the
    // overlay's `false` came back and flattened depth ordering for every draw
    // of that colour -- characters and pitch content losing their occlusion
    // against each other, with nothing thrown and no test failing.
    clearMaterialCache();
    const normal = new THREE.Group();
    const overlay = new THREE.Group();
    paint(normal, [cmd([1, 0, 0])]);
    paint(overlay, [cmd([1, 0, 0])], { depthTest: false });

    const normalMaterial = (normal.children[0] as THREE.Mesh).material as THREE.Material;
    const overlayMaterial = (overlay.children[0] as THREE.Mesh).material as THREE.Material;

    expect(overlayMaterial, "different depthTest must not share an instance").not.toBe(normalMaterial);
    expect(overlayMaterial.depthTest).toBe(false);
    expect(normalMaterial.depthTest, "the ordinary pass must keep depth testing").toBe(true);
  });

  it("still disposes NON-cached materials, so per-sprite textures are not leaked", () => {
    clearMaterialCache();
    const texture = new THREE.Texture();
    const sprite = new THREE.Sprite(new THREE.SpriteMaterial({ map: texture }));
    let disposed = false;
    texture.addEventListener("dispose", () => {
      disposed = true;
    });
    disposeObject(sprite);
    expect(disposed, "a sprite's own texture is not cache-owned and must still be released").toBe(true);
  });
});
