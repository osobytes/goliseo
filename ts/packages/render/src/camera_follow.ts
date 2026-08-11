// Broadcast-style following camera.
//
// The fixed whole-pitch view frames the stadium; a soccer game frames the
// play. This holds the smoothed focus, updated once per frame from the same
// place view_state is updated (where the authoritative dt lives), and read
// by the renderer.
//
// Three jobs, deliberately kept separate because an earlier version folded
// them together and they fought:
//
//   1. TRACK   an eased focus on the ball/carrier, with a deadzone so small
//              touches move nothing.
//   2. LOOK    an offset ahead along travel, so a player running with the
//              ball can see who is in front of them rather than what they
//              just left.
//   3. KEEP    a hard cap on how far the ball may sit from the focus, so it
//              is always on screen and near the middle whatever 1 and 2
//              decide.
//
// Folding LOOK into the tracking target meant the deadzone swallowed it (the
// deadzone deliberately lets the focus trail, the look-ahead deliberately
// pushes it forward). Applying LOOK to the eased result instead gives both
// their full effect, and KEEP is a backstop over the top.
//
// Impure by design: it owns frame-to-frame state. The projection maths it
// feeds stays pure in camera.ts, so the view it produces is still
// unit-testable.
//
// Boundary note: `MatchState` is a sim shape (ARCHITECTURE.md §4 rule 6, comes
// from Rust `crates/gc-sim`). Only the fields this module reads are declared
// locally below, matching `correction_smoothing.ts`. `controlled` is a
// 0-based index into `players`, the idiomatic JS/TS shape (§4 rule 3); the
// Rust/wasm source uses a 1-based index, and the JS<->wasm marshalling
// boundary that will produce this object is a later milestone, so this
// treats it as already converted.

import { camera, type CameraField, type CameraView } from "./camera.ts";
import type { CorrectionSmoothingPose } from "./correction_smoothing.ts";

export interface CameraFollowPoint {
  readonly x: number;
  readonly y: number;
}

export interface CameraFollowPlayer {
  readonly id: string;
  readonly pos: CameraFollowPoint;
}

export interface CameraFollowMatchState {
  readonly field: CameraField;
  readonly ball: CameraFollowPoint;
  readonly players: readonly CameraFollowPlayer[];
  /** 0-based index into `players`; see boundary note above. */
  readonly controlled?: number;
}

export interface CameraFollowConfig {
  /**
   * In perspective mode this scales the camera's distance and height rather
   * than magnifying the image, so 1 is the tuned framing. Above 1 moves in.
   */
  zoom: number;
  /** How fast the focus closes on its target. */
  ease: number;
  /**
   * Blend of ball position against the controlled player. Pure ball tracking
   * is twitchy when the ball is loose, and abandons whoever you are
   * steering.
   */
  ball_weight: number;

  // 1. TRACK -----------------------------------------------------------
  /**
   * How far the framing point may drift from the focus before the camera
   * moves at all, as a fraction of the pitch.
   */
  deadzone_x: number;
  deadzone_y: number;
  /**
   * The deadzone exists to absorb jitter, not to hold the camera back from
   * real play. Above this ball speed it collapses, so a moving ball is
   * tracked tightly and the look-ahead below gets its full effect instead of
   * being cancelled out by the trail the deadzone deliberately introduces.
   */
  deadzone_release_speed: number;
  /** how much of the box speed can remove */
  deadzone_release: number;

  // 2. LOOK ------------------------------------------------------------
  /**
   * Offset ahead along travel, so the carrier can see what they are running
   * into. Below `look_min_speed` there is no offset at all, which keeps a
   * jostling or slowly drifting ball from sliding the camera around.
   */
  look_min_speed: number;
  /** seconds of travel to look ahead by */
  look_gain: number;
  /** world units, before the ball-keeping cap below */
  look_max: number;
  /**
   * Fraction of the KEEP budget the look-ahead may use. Below 1 so KEEP
   * stays a backstop rather than a cap that binds during ordinary fast play.
   */
  look_headroom: number;
  /** how fast the offset itself follows, kept slow and smooth */
  look_ease: number;

  // 3. KEEP ------------------------------------------------------------
  /**
   * Hard cap on the ball's distance from the focus. Larger than the deadzone
   * so normal play is governed by TRACK, but the ball can never leave frame.
   */
  ball_keep_x: number;
  ball_keep_y: number;

  /**
   * How far the focus stays inside each touchline, as a fraction of the
   * pitch. Generous on purpose: a real camera only needs to not fly off the
   * end of the pitch, and it must still be able to reach the goals.
   */
  margin_x: number;
  margin_y: number;
}

interface CameraFollowState {
  x: number | undefined;
  y: number | undefined;
  bx: number | undefined;
  by: number | undefined;
  vx: number;
  vy: number;
  lx: number;
  ly: number;
}

const state: CameraFollowState = {
  x: undefined,
  y: undefined,
  bx: undefined,
  by: undefined,
  vx: 0,
  vy: 0,
  lx: 0,
  ly: 0,
};

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

// The camera keeps the framing point inside a box rather than chasing it, so
// only the amount by which it has escaped becomes something to move toward.
function escaped(pos: number, target: number, limit: number): number {
  const delta = target - pos;
  if (delta > limit) {
    return target - limit;
  } else if (delta < -limit) {
    return target + limit;
  }
  return pos;
}

/** Broadcast-style following camera. See file header. */
export const cameraFollow = {
  config: {
    zoom: 1.0,
    ease: 3.2,
    ball_weight: 0.62,

    deadzone_x: 0.085,
    deadzone_y: 0.07,
    deadzone_release_speed: 180,
    deadzone_release: 0.85,

    look_min_speed: 55,
    look_gain: 0.55,
    look_max: 210,
    look_headroom: 0.85,
    look_ease: 2.4,

    ball_keep_x: 0.115,
    ball_keep_y: 0.095,

    margin_x: 0.14,
    margin_y: 0.16,
  },

  // Drops the smoothed state so the next update snaps instead of sweeping
  // across the pitch. Call on kickoff and after a goal.
  reset(): void {
    state.x = undefined;
    state.y = undefined;
    state.bx = undefined;
    state.by = undefined;
    state.vx = 0;
    state.vy = 0;
    state.lx = 0;
    state.ly = 0;
  },

  update(s: CameraFollowMatchState, dt: number, pose?: CorrectionSmoothingPose): void {
    const cfg = cameraFollow.config;
    const field = s.field;
    const ball = pose?.ball ?? s.ball;

    // MatchState carries no ball velocity (the sim stays pure), so derive it
    // here the same way view_state derives player speed.
    if (state.bx !== undefined && state.by !== undefined && dt > 0) {
      const k = Math.min(dt * 9, 1);
      state.vx = state.vx + ((ball.x - state.bx) / dt - state.vx) * k;
      state.vy = state.vy + ((ball.y - state.by) / dt - state.vy) * k;
    }
    state.bx = ball.x;
    state.by = ball.y;

    // 1. TRACK
    let targetX = ball.x;
    let targetY = ball.y;
    const controlled = s.controlled !== undefined ? s.players[s.controlled] : undefined;
    if (controlled !== undefined) {
      const p = pose?.players[controlled.id] ?? controlled.pos;
      const w = cfg.ball_weight;
      targetX = ball.x * w + p.x * (1 - w);
      targetY = ball.y * w + p.y * (1 - w);
    }

    if (state.x === undefined || state.y === undefined) {
      state.x = targetX;
      state.y = targetY;
    } else {
      const moving = Math.sqrt(state.vx * state.vx + state.vy * state.vy);
      const release = 1 - Math.min(moving / cfg.deadzone_release_speed, 1) * cfg.deadzone_release;
      const wantX = escaped(state.x, targetX, field.w * cfg.deadzone_x * release);
      const wantY = escaped(state.y, targetY, field.h * cfg.deadzone_y * release);
      const k = 1 - Math.exp(-cfg.ease * Math.max(dt, 0));
      state.x = state.x + (wantX - state.x) * k;
      state.y = state.y + (wantY - state.y) * k;
    }

    // 2. LOOK: an offset ahead along travel, eased separately so it grows
    // and decays smoothly instead of snapping when the ball changes hands.
    const speed = Math.sqrt(state.vx * state.vx + state.vy * state.vy);
    let wantLx = 0;
    let wantLy = 0;
    if (speed > cfg.look_min_speed) {
      // Subtracting the threshold rather than testing against it means the
      // offset grows from zero instead of popping in at the boundary.
      const reach = Math.min((speed - cfg.look_min_speed) * cfg.look_gain, cfg.look_max);
      wantLx = (state.vx / speed) * reach;
      wantLy = (state.vy / speed) * reach;

      // Cap the lead itself, per axis, inside the ball-keeping budget. KEEP
      // is meant to be a backstop for when play outruns the ease; if LOOK
      // can ask for more offset than KEEP allows, KEEP ends up clamping
      // every frame of fast play and silently cancels the very feature it
      // sits behind.
      const capX = field.w * cfg.ball_keep_x * cfg.look_headroom;
      const capY = field.h * cfg.ball_keep_y * cfg.look_headroom;
      wantLx = Math.max(-capX, Math.min(capX, wantLx));
      wantLy = Math.max(-capY, Math.min(capY, wantLy));

      // Then add back whatever the ease is currently trailing the target by.
      //
      // Without this the two mechanisms cancel: an exponential ease chasing
      // a target moving at v settles a full v/ease behind it (75 units at a
      // running pace), which is the same order as the lead itself, so the
      // camera ends up looking at the ball rather than ahead of it. Adding
      // the trail back means `state.lx` measures the lead over the
      // *target*, which is what the cap above is expressed in and what KEEP
      // later bounds.
      //
      // Compensating against the tracking target rather than the ball keeps
      // `ball_weight` intact -- correcting toward the ball would drag the
      // framing off the player you are steering whenever the ball came
      // loose.
      //
      // Ramped in with speed so crossing `look_min_speed` sweeps rather than
      // steps: at walking pace there is barely any trail to give back
      // anyway.
      const comp = Math.min(speed / cfg.deadzone_release_speed, 1);
      wantLx = wantLx + (targetX - state.x) * comp;
      wantLy = wantLy + (targetY - state.y) * comp;
    }
    const lk = 1 - Math.exp(-cfg.look_ease * Math.max(dt, 0));
    state.lx = state.lx + (wantLx - state.lx) * lk;
    state.ly = state.ly + (wantLy - state.ly) * lk;
  },

  // The focus actually framed: tracking plus look-ahead, with the ball kept
  // close.
  focus(): readonly [number, number] | readonly [undefined, undefined] {
    if (state.x === undefined || state.y === undefined) {
      return [undefined, undefined];
    }
    return [state.x + state.lx, state.y + state.ly];
  },

  // The view to project through, or undefined before the first update (in
  // which case the caller falls back to the whole-pitch view).
  view(field: CameraField): CameraView | undefined {
    if (state.x === undefined || state.y === undefined) {
      return undefined;
    }
    const cfg = cameraFollow.config;
    let fx = state.x + state.lx;
    let fy = state.y + state.ly;

    // 3. KEEP: whatever tracking and look-ahead wanted, the ball stays on
    // screen and near the middle. This is a backstop, not the main
    // mechanism -- it only binds when play outruns the ease or the
    // look-ahead overreaches.
    if (state.bx !== undefined && state.by !== undefined) {
      fx = clamp(fx, state.bx - field.w * cfg.ball_keep_x, state.bx + field.w * cfg.ball_keep_x);
      fy = clamp(fy, state.by - field.h * cfg.ball_keep_y, state.by + field.h * cfg.ball_keep_y);
    }

    return camera.view(fx, fy, field, cfg.zoom, {
      x: field.w * cfg.margin_x,
      y: field.h * cfg.margin_y,
    });
  },
};
