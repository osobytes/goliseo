import { describe, expect, it } from "vitest";
import { camera, type CameraField } from "./camera.ts";
import { cameraFollow, type CameraFollowMatchState } from "./camera_follow.ts";
import { viewState, type ViewStatePlayer } from "./view_state.ts";

function near(actual: number, expected: number, eps = 1e-6): void {
  expect(Math.abs(actual - expected)).toBeLessThanOrEqual(eps);
}

const field: CameraField = { w: 960, h: 540 };

function fakeState(ballX: number, ballY: number): CameraFollowMatchState {
  return { field, ball: { x: ballX, y: ballY }, players: [] };
}

describe("cameraFollow", () => {
  it("moves the focus toward the ball", () => {
    // Regression: deriving the clamp margin from field/(2*zoom) pinned the
    // focus to the exact centre of the pitch at zoom 1, so the camera could
    // not follow anything at all.
    cameraFollow.reset();
    cameraFollow.update(fakeState(200, 150), 1 / 60);
    const [fx, fy] = cameraFollow.focus();
    expect(fx).toBeDefined();
    expect(fy).toBeDefined();
    near(fx as number, 200, 1);
    near(fy as number, 150, 1);
  });

  it("holds still while the ball drifts inside the deadzone", () => {
    cameraFollow.reset();
    cameraFollow.update(fakeState(480, 270), 1 / 60);
    const [beforeX] = cameraFollow.focus();
    expect(beforeX).toBeDefined();
    // A slow drift of ~30 units/s. Jumping the ball instead would derive a
    // huge velocity, and the lead would carry the target out of the box --
    // correct behaviour, but not what this test is measuring.
    let x = 480;
    for (let i = 0; i < 30; i += 1) {
      x += 0.5;
      cameraFollow.update(fakeState(x, 270), 1 / 60);
    }
    const [afterX] = cameraFollow.focus();
    expect(afterX).toBeDefined();
    near(afterX as number, beforeX as number, 2);
  });

  it("starts moving once the ball leaves the deadzone", () => {
    cameraFollow.reset();
    cameraFollow.update(fakeState(480, 270), 1 / 60);
    const [beforeX] = cameraFollow.focus();
    expect(beforeX).toBeDefined();
    for (let i = 0; i < 30; i += 1) {
      cameraFollow.update(fakeState(800, 270), 1 / 60);
    }
    const [afterX] = cameraFollow.focus();
    expect(afterX).toBeDefined();
    expect((afterX as number) > (beforeX as number) + 50).toBe(true);
  });

  it("eases rather than snapping once established", () => {
    cameraFollow.reset();
    cameraFollow.update(fakeState(480, 270), 1 / 60);
    cameraFollow.update(fakeState(900, 500), 1 / 60);
    const [fx] = cameraFollow.focus();
    expect(fx).toBeDefined();
    expect((fx as number) > 480).toBe(true);
    expect((fx as number) < 900).toBe(true);
  });

  it("keeps the focus inside the pitch but still reaches toward goal", () => {
    cameraFollow.reset();
    for (let i = 0; i < 400; i += 1) {
      cameraFollow.update(fakeState(960, 540), 1 / 30);
    }
    const view = cameraFollow.view(field);
    expect(view).toBeDefined();
    const v = view!;
    expect(v.x <= field.w).toBe(true);
    // The old clamp pinned this to 480; a usable camera gets much closer to
    // the goal than the halfway line.
    expect(v.x > 700).toBe(true);
  });

  it("looks ahead along travel once the ball is moving", () => {
    // Look-ahead is applied to the eased result, not to the tracking target,
    // so it survives the deadzone instead of being swallowed by it.
    cameraFollow.reset();
    let x = 300;
    for (let i = 0; i < 60; i += 1) {
      cameraFollow.update(fakeState(x, 270), 1 / 60);
      x += 4; // 240 units/s, a real running pace
    }
    const [fx] = cameraFollow.focus();
    expect(fx).toBeDefined();
    expect((fx as number) > x).toBe(true);
  });

  it("leads a running ball by a useful distance, not a token one", () => {
    // Regression: an exponential ease chasing a target moving at v settles
    // v/ease behind it, which at a running pace is the same order as the
    // lead itself. Both mechanisms were present and they cancelled, so the
    // carrier still could not see who was in front of them. Asserting only
    // "ahead by something" let that through -- this pins the magnitude.
    cameraFollow.reset();
    let x = 200;
    for (let i = 0; i < 180; i += 1) {
      cameraFollow.update(fakeState(x, 270), 1 / 60);
      x += 4; // 240 units/s
    }
    const [fx] = cameraFollow.focus();
    expect(fx).toBeDefined();
    expect((fx as number) - x > 60).toBe(true);
  });

  it("does not let ball-keeping clamp away the lead during fast play", () => {
    // KEEP is a backstop for when play outruns the ease. If the lead can ask
    // for more than KEEP allows, KEEP binds every frame of a fast break and
    // the look-ahead silently does nothing.
    cameraFollow.reset();
    // Kept clear of the touchlines: view() also applies the margin clamp,
    // which is a separate concern from KEEP and would mask it.
    let x = 150;
    let y = 120;
    for (let i = 0; i < 120; i += 1) {
      cameraFollow.update(fakeState(x, y), 1 / 60);
      x += 4;
      y += 2; // ~268 units/s diagonally, a genuine counter
    }
    const [fx, fy] = cameraFollow.focus();
    const view = cameraFollow.view(field);
    expect(view).toBeDefined();
    expect(fx).toBeDefined();
    expect(fy).toBeDefined();
    // view() applies KEEP to the raw focus; if KEEP did not bind, the two
    // agree exactly.
    near(view!.x, fx as number, 0.001);
    near(view!.y, fy as number, 0.001);
  });

  it("does not look ahead for a slow drift", () => {
    cameraFollow.reset();
    let x = 480;
    for (let i = 0; i < 60; i += 1) {
      cameraFollow.update(fakeState(x, 270), 1 / 60);
      x += 0.4; // 24 units/s, below the look-ahead threshold
    }
    const [fx] = cameraFollow.focus();
    expect(fx).toBeDefined();
    expect((fx as number) < x + 5).toBe(true);
  });

  it("keeps the ball near the focus however far tracking lags", () => {
    // Teleport the ball: tracking and look-ahead both lag badly, and the
    // hard cap is the only thing keeping the ball on screen.
    cameraFollow.reset();
    cameraFollow.update(fakeState(120, 120), 1 / 60);
    cameraFollow.update(fakeState(840, 420), 1 / 60);
    const view = cameraFollow.view(field);
    expect(view).toBeDefined();
    const cfg = cameraFollow.config;
    expect(Math.abs(view!.x - 840) <= field.w * cfg.ball_keep_x + 1).toBe(true);
  });

  it("view zoom 1 still produces a usable projection", () => {
    cameraFollow.reset();
    cameraFollow.update(fakeState(300, 200), 1 / 60);
    const view = cameraFollow.view(field);
    const [sx, sy] = camera.project(300, 200, field, { w: 1280, h: 720 }, undefined, view);
    expect(sx === sx && sy === sy).toBe(true); // finite (not NaN)
  });
});

describe("view_state gait phase", () => {
  function player(x: number): ViewStatePlayer {
    return { id: "p1", pos: { x, y: 100 } };
  }

  it("advances smoothly when speed changes", () => {
    // Regression: the phase used to be derived as cumulative_distance /
    // current_stride. Because the stride lengthens with speed, changing it
    // retroactively rescaled every unit already travelled, so the phase
    // jumped most of a cycle whenever speed wobbled -- the animation
    // appeared to flick between two poses.
    //
    // No view_state.reset() here: this describe block relies on "p1" being
    // untouched by any earlier-loaded spec file (alphabetically,
    // correction_smoothing.spec.ts, the only other file that touches player
    // "p1"/view_state, runs after this one and clears up after itself).
    // vitest isolates module state per spec *file* by default, which
    // reproduces the same "first touch" starting condition.
    let x = 0;
    let prev: number | undefined;
    let worst = 0;
    for (let i = 1; i <= 400; i += 1) {
      // Accelerate through the walk/run blend, where the stride changes
      // fastest and the old formulation was worst.
      const step = 1.5 + i * 0.012;
      x += step;
      viewState.update([player(x)], 1 / 60);
      const g = viewState.get("p1");
      expect(g).toBeDefined();
      const gait = g!.gait;
      if (prev !== undefined) {
        const delta = ((gait - prev) % 1 + 1) % 1;
        worst = Math.max(worst, delta);
      }
      prev = gait;
    }
    // One frame can never advance more than a small slice of a cycle: at the
    // fastest stride a 60 Hz frame is well under a tenth of a cycle.
    expect(worst < 0.1).toBe(true);
  });

  it("stays within [0, 1)", () => {
    viewState.reset();
    let x = 0;
    for (let i = 0; i < 600; i += 1) {
      x += 6;
      viewState.update([player(x)], 1 / 60);
    }
    const g = viewState.get("p1");
    expect(g).toBeDefined();
    expect(g!.gait >= 0 && g!.gait < 1).toBe(true);
  });
});
