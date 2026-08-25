// Tests for draw2d.ts's pure half (see that file's header).
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

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import * as THREE from "three";
import {
  appendCommands,
  disposeObject,
  materialCacheSize,
  paint,
  resetMaterialCache,
  DrawList,
  rotateAround,
  type DrawCommand,
  type PivotTransform,
  type TextCommand,
} from "./draw2d.ts";

function near(actual: number, expected: number, eps = 1e-9): void {
  expect(Math.abs(actual - expected)).toBeLessThanOrEqual(eps);
}

describe("rotateAround", () => {
  it("leaves the pivot itself fixed under any rotation", () => {
    const t: PivotTransform = {
      pivotX: 10,
      pivotY: 20,
      angle: Math.PI / 3,
      offsetX: 0,
      offsetY: 0,
    };
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
    // The composition this exercises, for a keeper_spread dive to the right:
    // translate(sx + sign*r*travel*amount, gy); rotate(angle); translate(-sx, -gy).
    const sx = 100;
    const gy = 200;
    const r = 12;
    const sign = 1;
    const travel = 0.65;
    const amount = 1;
    const angle = ((28 * Math.PI) / 180) * amount;
    const t: PivotTransform = {
      pivotX: sx,
      pivotY: gy,
      angle,
      offsetX: sign * r * travel * amount,
      offsetY: 0,
    };
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

// #403: the match rendered at exactly half the display refresh rate because
// `paint`'s per-frame clear disposed every material, which dropped three.js's
// refcount on the compiled GL program behind it to zero -- five programs
// destroyed, recompiled and SYNCHRONOUSLY re-validated every frame, ~7.8 ms of
// a 13.33 ms budget. None of that is observable without a GL context, but the
// JS-side invariant that prevents it entirely is: materials with identical
// parameters must be the SAME object across frames, and `paint` must not
// dispose them. That is what this suite pins.
describe("draw2d material cache (#403)", () => {
  beforeEach(() => {
    resetMaterialCache();
  });

  const fill = (
    color: readonly [number, number, number],
    alpha?: number,
    blend?: "add",
  ): DrawCommand => ({
    kind: "rect",
    mode: "fill",
    x: 0,
    y: 0,
    w: 4,
    h: 4,
    color,
    ...(alpha !== undefined ? { alpha } : {}),
    ...(blend !== undefined ? { blend } : {}),
  });

  const stroke = (color: readonly [number, number, number]): DrawCommand => ({
    kind: "line",
    points: [0, 0, 4, 4],
    color,
  });

  function materialOf(group: THREE.Group, index = 0): THREE.Material {
    const child = group.children[index];
    expect(child).toBeDefined();
    const owner = child as THREE.Mesh | THREE.Line;
    const material = owner.material;
    expect(Array.isArray(material)).toBe(false);
    return material as THREE.Material;
  }

  it("hands the same material instance to identical commands across repainted frames", () => {
    const group = new THREE.Group();
    paint(group, [fill([1, 0, 0])]);
    const first = materialOf(group);
    paint(group, [fill([1, 0, 0])]);
    expect(materialOf(group)).toBe(first);
    paint(group, [fill([1, 0, 0])]);
    expect(materialOf(group)).toBe(first);
  });

  it("does not dispose a shared material when paint() clears the object holding it", () => {
    const group = new THREE.Group();
    paint(group, [fill([0, 1, 0])]);
    const material = materialOf(group);
    let disposed = false;
    material.dispose = () => {
      disposed = true;
    };

    paint(group, []);

    expect(disposed).toBe(false);
  });

  it("still disposes the geometry of every cleared child", () => {
    const group = new THREE.Group();
    paint(group, [fill([0, 0, 1])]);
    const mesh = group.children[0] as THREE.Mesh;
    let disposed = false;
    mesh.geometry.dispose = () => {
      disposed = true;
    };

    paint(group, []);

    expect(disposed).toBe(true);
  });

  it("separates strokes from fills, and separates colour, alpha and blend", () => {
    const group = new THREE.Group();
    paint(group, [
      fill([1, 0, 0]),
      fill([0, 1, 0]),
      fill([1, 0, 0], 0.5),
      fill([1, 0, 0], undefined, "add"),
      stroke([1, 0, 0]),
    ]);
    const materials = group.children.map((_, i) => materialOf(group, i));
    expect(new Set(materials).size).toBe(5);
    expect(materialCacheSize()).toBe(5);
  });

  it("treats an explicit alpha of 1 as distinct from no alpha (only one sets transparent)", () => {
    const group = new THREE.Group();
    paint(group, [fill([1, 0, 0]), fill([1, 0, 0], 1)]);
    const opaque = materialOf(group, 0);
    const transparent = materialOf(group, 1);
    expect(opaque).not.toBe(transparent);
    expect(opaque.transparent).toBe(false);
    expect(transparent.transparent).toBe(true);
  });

  it("keys on PaintOptions.depthTest, so two callers never fight over one instance", () => {
    const tested = new THREE.Group();
    const untested = new THREE.Group();
    appendCommands(tested, [fill([1, 1, 0])], { depthTest: true });
    appendCommands(untested, [fill([1, 1, 0])], { depthTest: false });

    const a = materialOf(tested);
    const b = materialOf(untested);
    expect(a).not.toBe(b);
    expect(a.depthTest).toBe(true);
    expect(b.depthTest).toBe(false);

    // And building the depth-tested one again must not have flipped the other.
    appendCommands(tested, [fill([1, 1, 0])], { depthTest: true });
    expect(b.depthTest).toBe(false);
  });

  it("shares one material across a batched stroke run, since colour is per-vertex there", () => {
    const group = new THREE.Group();
    const dl = new DrawList();
    dl.line([0, 0, 4, 0], [1, 0, 0]);
    dl.line([4, 0, 8, 0], [0, 1, 0]);
    paint(group, dl.commands, { batchStrokes: true });
    expect(group.children).toHaveLength(1);
    const batched = materialOf(group);
    expect((batched as THREE.LineBasicMaterial).vertexColors).toBe(true);

    paint(group, dl.commands, { batchStrokes: true });
    expect(materialOf(group)).toBe(batched);
  });

  it("stays bounded under LRU eviction", () => {
    const group = new THREE.Group();
    paint(group, [fill([0, 0, 0])]);
    const oldest = materialOf(group);
    // Distinct colours, so distinct keys. Well past the 512-entry cap.
    for (let i = 1; i < 700; i += 1) {
      paint(group, [fill([i / 1024, 0, 0])]);
    }
    expect(materialCacheSize()).toBeLessThanOrEqual(512);
    // Least-recently-used, so the very first key is long gone, and dropping it
    // handed that material back to the ordinary per-object path.
    expect(oldest.userData["draw2dSharedMaterial"]).toBe(false);
    // The newest is still cached, and still shared.
    expect(materialOf(group).userData["draw2dSharedMaterial"]).toBe(true);
  });

  // The invariant that makes sharing safe at all, and the first thing a
  // reviewer of three.js's own `FullScreenQuad` bug would look for: module-
  // level state must never be freed while a participant still holds it.
  // Eviction happens on INSERT — i.e. midway through building a frame, since
  // `pitch.draw` issues one `paint` then several `appendCommands` — so a cache
  // that disposed on eviction would be disposing a material that an object
  // built earlier in the SAME frame is still using.
  it("never disposes on eviction — it hands ownership back instead", () => {
    const group = new THREE.Group();
    paint(group, [fill([0, 0, 0])]);
    const material = materialOf(group);
    let disposed = false;
    material.addEventListener("dispose", () => {
      disposed = true;
    });

    for (let i = 1; i < 700; i += 1) {
      paint(group, [fill([i / 1024, 0, 0])]);
    }

    expect(materialCacheSize()).toBeLessThanOrEqual(512);
    expect(disposed).toBe(false);
  });

  it("a dropped material is disposed by the ordinary path once its object is cleared", () => {
    const group = new THREE.Group();
    paint(group, [fill([0, 0, 0])]);
    const material = materialOf(group);
    let disposed = false;
    material.addEventListener("dispose", () => {
      disposed = true;
    });
    resetMaterialCache();
    expect(material.userData["draw2dSharedMaterial"]).toBe(false);
    expect(disposed).toBe(false);

    // The object still holds it, so clearing that object now owns the disposal.
    paint(group, []);

    expect(disposed).toBe(true);
  });

  it("resetMaterialCache empties the cache without disposing anything", () => {
    const group = new THREE.Group();
    paint(group, [fill([1, 0, 1])]);
    const material = materialOf(group);
    let disposed = false;
    material.addEventListener("dispose", () => {
      disposed = true;
    });
    expect(materialCacheSize()).toBe(1);

    resetMaterialCache();

    expect(materialCacheSize()).toBe(0);
    expect(disposed).toBe(false);
    expect(material.userData["draw2dSharedMaterial"]).toBe(false);
  });

  it("a frame after a reset builds a fresh material rather than reviving the old one", () => {
    const group = new THREE.Group();
    paint(group, [fill([0.25, 0.5, 0.75])]);
    const before = materialOf(group);
    resetMaterialCache();
    paint(group, [fill([0.25, 0.5, 0.75])]);
    const after = materialOf(group);
    expect(after).not.toBe(before);
    expect(after.userData["draw2dSharedMaterial"]).toBe(true);
  });
});

// Text was the LAST draw-command builder bypassing the material cache, and the
// #403 suite above could not see it: `buildTextSprite` is the one DOM-shaped
// builder (`document.createElement("canvas")`), so every test in this file
// substitutes it via `PaintOptions.buildText` and the real path went uncovered.
// That is exactly why it survived #411 -- the regression suite for the bug had
// a hole in the shape of the one remaining instance of the bug.
//
// Closed here with a minimal canvas stub rather than by adding a jsdom
// environment. `buildTextSprite` already handles a null 2d context (it renders
// nothing and still produces a texture), so the stub only has to satisfy
// `THREE.CanvasTexture`, which stores its argument without inspecting it.
describe("draw2d text material cache (#403)", () => {
  const realDocument = (globalThis as { document?: unknown }).document;

  beforeEach(() => {
    resetMaterialCache();
    (globalThis as { document?: unknown }).document = {
      createElement: () => ({ width: 0, height: 0, getContext: () => null }),
    };
  });

  afterEach(() => {
    if (realDocument === undefined) {
      delete (globalThis as { document?: unknown }).document;
    } else {
      (globalThis as { document?: unknown }).document = realDocument;
    }
  });

  const text = (
    content: string,
    over: Partial<{
      alpha: number;
      fontPx: number;
      w: number;
      color: readonly [number, number, number];
    }> = {},
  ): DrawCommand => ({
    kind: "text",
    text: content,
    x: 0,
    y: 0,
    w: over.w ?? 100,
    align: "left",
    color: over.color ?? [1, 1, 1],
    ...(over.alpha !== undefined ? { alpha: over.alpha } : {}),
    ...(over.fontPx !== undefined ? { fontPx: over.fontPx } : {}),
  });

  // `THREE.Sprite.material` is a single `SpriteMaterial`, not the
  // `Material | Material[]` union `Mesh` carries, so there is nothing to narrow
  // here -- unlike `materialOf` above, which does have to.
  function spriteMaterial(group: THREE.Group): THREE.SpriteMaterial {
    const child = group.children[0];
    expect(child, "a text command should paint exactly one sprite").toBeDefined();
    return (child as THREE.Sprite).material;
  }

  it("reuses one material and one texture across frames for unchanged text", () => {
    // The charge/windup label is emitted EVERY frame while a player holds shot
    // or pass, and its content changes only between "SHOT" and "PASS". Before
    // this cache that was a fresh canvas, a fresh GPU texture upload and a
    // fresh material sixty times a second, each disposed on the next paint --
    // landing precisely during the beat the player is timing.
    const group = new THREE.Group();
    paint(group, [text("SHOT")]);
    const first = spriteMaterial(group);
    const firstMap = first.map;

    paint(group, [text("SHOT")]);
    const second = spriteMaterial(group);

    expect(second, "same text must reuse the same material object").toBe(first);
    expect(second.map, "and the same texture").toBe(firstMap);
  });

  it("does not dispose a shared text material when the frame is cleared", () => {
    // The invariant that actually prevents the stall: `paint`'s clear must
    // leave cached materials alone, or the GL program behind them is destroyed
    // and synchronously re-linked on the next frame.
    const group = new THREE.Group();
    paint(group, [text("PASS")]);
    const material = spriteMaterial(group);
    let disposed = false;
    material.addEventListener("dispose", () => {
      disposed = true;
    });

    paint(group, []);
    expect(disposed, "paint's clear must not dispose a cached text material").toBe(false);
  });

  it("keys on every property that changes the rasterised pixels", () => {
    // A key missing a field renders STALE TEXT, which is a worse failure than
    // the churn the cache removes -- so each of these must produce a distinct
    // material rather than a stale hit.
    const base = text("SHOT");
    const variants: readonly DrawCommand[] = [
      text("PASS"),
      text("SHOT", { alpha: 0.5 }),
      text("SHOT", { fontPx: 24 }),
      text("SHOT", { w: 200 }),
      text("SHOT", { color: [1, 0, 0] }),
    ];

    const group = new THREE.Group();
    paint(group, [base]);
    const baseMaterial = spriteMaterial(group);

    for (const variant of variants) {
      const other = new THREE.Group();
      paint(other, [variant]);
      expect(
        spriteMaterial(other),
        `${JSON.stringify(variant)} must not reuse the base material`,
      ).not.toBe(baseMaterial);
    }
  });

  it("shares one material between two identical labels in the SAME frame", () => {
    const group = new THREE.Group();
    // Two SEPARATE command objects with identical content -- `text()` builds a
    // fresh one per call, which is the case that matters: the cache must key on
    // content, not on object identity.
    paint(group, [text("SHOT"), text("SHOT")]);
    const a = (group.children[0] as THREE.Sprite).material;
    const b = (group.children[1] as THREE.Sprite).material;
    expect(a).toBe(b);
  });
});
