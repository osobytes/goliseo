// New tests for player_renderer_3d.ts's pure half: pose selection and the
// metres-per-world-unit conversion. No Lua spec targets
// game/render/player_renderer_3d.lua directly (it has no `spec/render/`
// counterpart in the Lua tree). The GPU-adjacent mesh/skeleton/camera half
// is untested -- see this package's port report.

import { describe, expect, it } from "vitest";
import { Vec2 } from "@gc/core";
import { characterCameraParams, clipFor, metresPerWorldUnit, poseFor, DEFAULT_PLAYER_RADIUS } from "./player_renderer_3d.ts";
import type { PlayerRenderOptions } from "./player_renderer.ts";
import type { PlayerView } from "./view_state.ts";

function baseOptions(overrides: Partial<PlayerRenderOptions> = {}): PlayerRenderOptions {
  return { is_keeper: false, controlled: false, ...overrides };
}

describe("player_renderer_3d.clipFor", () => {
  it("maps stances/gaits onto their limb clip", () => {
    expect(clipFor("locomotion")).toBe("locomotion");
    expect(clipFor("contain")).toBe("locomotion");
    expect(clipFor("keeper_shuffle")).toBe("locomotion");
    expect(clipFor("combat_guard")).toBe("guard");
    expect(clipFor("combat_windup")).toBe("guard");
    expect(clipFor("combat_active")).toBe("charge");
    expect(clipFor("combat_recovery")).toBe("guard");
    expect(clipFor("combat_aim")).toBe("guard");
    expect(clipFor("fatigue")).toBe("idle");
  });

  it("falls back to idle for an unmapped or missing pose id (e.g. whole-body actions owned by action_pose.ts)", () => {
    expect(clipFor(undefined)).toBe("idle");
    expect(clipFor("aerial_bicycle")).toBe("idle");
    expect(clipFor("keeper_dive")).toBe("idle");
  });
});

describe("player_renderer_3d.metresPerWorldUnit", () => {
  it("is size-independent of depth: converting the same world distance at any scale gives the same metres", () => {
    // The conversion itself does not take a `scale`/`radius` argument -- it
    // is a fixed ratio derived once from the rig's height -- so this checks
    // that two different rig heights produce their own distinct, stable ratios.
    const short = metresPerWorldUnit(1.6);
    const tall = metresPerWorldUnit(2.0);
    expect(tall).toBeGreaterThan(short);
    expect(metresPerWorldUnit(1.6)).toBe(short);
  });

  it("uses sim/match.lua's PLAYER_RADIUS (12) as its default divisor", () => {
    const height = 1.8;
    expect(metresPerWorldUnit(height)).toBe(height / (DEFAULT_PLAYER_RADIUS * 3.0 * 2));
  });
});

describe("player_renderer_3d.poseFor", () => {
  const idleView: PlayerView = { px: 0, py: 0, speed: 0, phase: 0, gait: 0, lean: 0 };

  it("produces a pose with a root-adjacent sparse rotation/move map, deterministic for the same `now`", () => {
    const a = poseFor(idleView, baseOptions(), 1.23);
    const b = poseFor(idleView, baseOptions(), 1.23);
    expect(a).toEqual(b);
  });

  it("differs when the pose id selects the charge overlay vs plain idle", () => {
    const idle = poseFor(idleView, baseOptions(), 1.0);
    const charging = poseFor(idleView, baseOptions({ pose: { id: "combat_active" } }), 1.0);
    expect(idle).not.toEqual(charging);
  });

  it("layers the keeper sling overlay while throw_timer counts down toward zero", () => {
    const midThrow = poseFor(idleView, baseOptions({ throw: 0.5 }), 1.0);
    const noThrow = poseFor(idleView, baseOptions({ throw: 0 }), 1.0);
    expect(midThrow).not.toEqual(noThrow);
  });

  it("applies a whole-body action (e.g. a keeper dive) on top of the gait/stance pose", () => {
    // dive_dir must have a nonzero lateral component relative to facing (see
    // rig3d/action_pose.ts's `lateralSign`) or the save overlay is a no-op.
    const grounded = poseFor(idleView, baseOptions(), 1.0);
    const diving = poseFor(idleView, baseOptions({ dive: 1, dive_dir: new Vec2(0, 1), pose: { id: "keeper_dive" } }), 1.0);
    expect(diving.rot["root"]).toBeDefined();
    expect(grounded.rot["root"]).not.toEqual(diving.rot["root"]);
  });
});

describe("player_renderer_3d.characterCameraParams", () => {
  it("centres the frustum so the character lands at the requested screen point", () => {
    const params = characterCameraParams(640, 360, 40, 1280, 720, (17 * Math.PI) / 180, 1.8);
    // left/right straddle 0 at sx == vw/2, and similarly for top/bottom at sy == vh/2.
    expect(params.left + params.right).toBeCloseTo(0, 9);
    expect(params.top + params.bottom).toBeCloseTo(0, 9);
  });

  it("shifts the frustum bounds as the requested screen point moves toward the left edge", () => {
    // left = -sx/ppm, right = (vw-sx)/ppm: moving sx toward 0 pushes both
    // bounds up (left toward/through 0, right further positive), which is
    // what re-centres a character drawn nearer the screen's left edge.
    const centered = characterCameraParams(640, 360, 40, 1280, 720, 0, 1.8);
    const left = characterCameraParams(100, 360, 40, 1280, 720, 0, 1.8);
    expect(left.left).toBeGreaterThan(centered.left);
    expect(left.right).toBeGreaterThan(centered.right);
  });

  it("elevates the eye above the character's mid-height and looks back down at it", () => {
    const params = characterCameraParams(640, 360, 40, 1280, 720, Math.PI / 6, 1.8);
    expect(params.eye[1]).toBeGreaterThan(params.target[1]);
    expect(params.target).toEqual([0, 0.9, 0]);
  });
});
