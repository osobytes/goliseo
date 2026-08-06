// Ported from spec/render/camera_spec.lua.

import { describe, expect, it } from "vitest";
import { camera, type CameraField, type CameraView, type CameraViewport } from "./camera.ts";

function near(actual: number, expected: number, eps = 1e-6): void {
  expect(Math.abs(actual - expected)).toBeLessThanOrEqual(eps);
}

const field: CameraField = { w: 960, h: 540 };
const vp: CameraViewport = { w: 1280, h: 720 };

describe("camera.project", () => {
  it("places nearer points lower on screen than far points", () => {
    const [, farY] = camera.project(480, 0, field, vp);
    const [, nearY] = camera.project(480, 540, field, vp);
    expect(nearY > farY).toBe(true);
  });

  it("scales nearer points up relative to far points", () => {
    const [, , farScale] = camera.project(480, 0, field, vp);
    const [, , nearScale] = camera.project(480, 540, field, vp);
    expect(nearScale > farScale).toBe(true);
  });

  it("keeps the pitch centre line on the screen centre at any depth", () => {
    const [farX] = camera.project(480, 0, field, vp);
    const [nearX] = camera.project(480, 540, field, vp);
    near(farX, vp.w / 2, 1e-6);
    near(nearX, vp.w / 2, 1e-6);
  });

  it("spreads the near edge wider than the far edge (trapezoid)", () => {
    const [farRight] = camera.project(960, 0, field, vp);
    const [nearRight] = camera.project(960, 540, field, vp);
    expect(nearRight > farRight).toBe(true);
  });
});

describe("camera.view", () => {
  it("zoom 1 leaves the projection untouched", () => {
    const v = camera.view(120, 90, field, 1);
    const [a, b, c] = camera.project(300, 200, field, vp);
    const [x, y, z] = camera.project(300, 200, field, vp, undefined, v);
    near(x, a, 0.001);
    near(y, b, 0.001);
    near(z, c, 0.001);
  });

  it("clamps the focus so a zoomed frame stays over the pitch", () => {
    const v = camera.view(0, 0, field, 2);
    expect(v.x).toBe(240);
    expect(v.y).toBe(135);
  });

  it("puts the focus at the centre of the screen", () => {
    const v = camera.view(300, 200, field, 2);
    const [x, y] = camera.project(v.x, v.y, field, vp, undefined, v);
    near(x, vp.w / 2, 0.001);
    near(y, vp.h / 2, 0.001);
  });

  it("magnifies without adding convergence", () => {
    // The whole point of a lens zoom: the ratio between a far span and a near
    // span is a property of the perspective, so magnifying must not change
    // it. The earlier window remap did, which is what made the pitch look
    // like a funnel.
    const v = camera.view(480, 270, field, 2);
    function span(wy: number, view: CameraView | undefined): number {
      const [a] = camera.project(300, wy, field, vp, undefined, view);
      const [b] = camera.project(660, wy, field, vp, undefined, view);
      return Math.abs(b - a);
    }
    near(span(60, v) / span(480, v), span(60, undefined) / span(480, undefined), 0.0001);
  });
});

describe("camera perspective mode", () => {
  function withPerspective(fn: () => void): void {
    const saved = camera.perspective_mode;
    camera.perspective_mode = true;
    try {
      fn();
    } finally {
      camera.perspective_mode = saved;
    }
  }

  it("centres the focus on screen", () => {
    withPerspective(() => {
      const v = camera.view(300, 200, field, 1);
      const [x, y] = camera.project(v.x, v.y, field, vp, undefined, v);
      near(x, vp.w / 2, 0.5);
      near(y, vp.h / 2, 0.5);
    });
  });

  it("converges: a far span is narrower than an equal near span", () => {
    withPerspective(() => {
      const v = camera.view(480, 270, field, 1);
      function span(wy: number): number {
        const [a] = camera.project(300, wy, field, vp, undefined, v);
        const [b] = camera.project(660, wy, field, vp, undefined, v);
        return Math.abs(b - a);
      }
      // +y runs toward the viewer, so a low y is the far touchline.
      expect(span(60) < span(480)).toBe(true);
    });
  });

  it("scales a player with depth, larger when nearer", () => {
    withPerspective(() => {
      const v = camera.view(480, 270, field, 1);
      const [, , far] = camera.project(480, 60, field, vp, undefined, v);
      const [, , near_] = camera.project(480, 480, field, vp, undefined, v);
      expect(near_ > far).toBe(true);
      expect(far > 0).toBe(true);
    });
  });

  it("pushes a point behind the camera out of frame rather than mirroring it", () => {
    withPerspective(() => {
      // Clamping a negative w to a small positive would place the point at a
      // finite but wildly wrong spot on screen; it has to leave frame.
      const v = camera.view(480, 270, field, 1);
      // +y runs toward the viewer and the eye sits beyond the focus on that
      // side, so a large +y is behind the camera. (A large -y is merely very
      // far away, and correctly converges on the vanishing point rather than
      // leaving frame.)
      const [x, y] = camera.project(480, 100000, field, vp, undefined, v);
      expect(x < -1000 || x > vp.w + 1000).toBe(true);
      expect(y < -1000 || y > vp.h + 1000).toBe(true);
    });
  });
});
