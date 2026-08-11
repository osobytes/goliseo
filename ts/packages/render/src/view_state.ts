// Ported from game/render/view_state.lua.
//
// Per-player view state derived from frame-to-frame motion. The sim stays
// pure (a MatchPlayer has no velocity); the renderer needs cadence, lean and
// speed to animate, so we derive them here from position deltas. Keyed by
// player id.
//
// `update` is called from the match screen's update (where the authoritative
// dt lives); the renderer only reads via `get`. Drawing without an update
// first (e.g. a smoke test) just yields `undefined` -- the renderer falls
// back to idle.
//
// Boundary note: `MatchPlayer`/`CorrectionSmoothingPose` are sim/render
// shapes (v2/README.md rule 6.7). Only the fields this module reads are
// declared locally, matching `correction_smoothing.ts`.

export interface ViewStatePoint {
  readonly x: number;
  readonly y: number;
}

export interface ViewStatePose {
  readonly players: Readonly<Record<string, ViewStatePoint>>;
}

export interface ViewStatePlayer {
  readonly id: string;
  readonly pos: ViewStatePoint;
}

export interface PlayerView {
  /** last world x */
  px: number;
  /** last world y */
  py: number;
  /** smoothed world-units/sec */
  speed: number;
  /** gait accumulator (radians), advances with distance */
  phase: number;
  /** normalised gait cycle position in [0, 1) */
  gait: number;
  /** smoothed screen-x lean, -1..1 */
  lean: number;
}

function clamp(x: number, a: number, b: number): number {
  return Math.max(a, Math.min(b, x));
}

// Gait cadence: radians of limb swing per world-unit travelled. Tuned so a
// full-speed runner pumps a few times a second.
//
// Exported because `phase` is distance-derived, and a consumer that wants the
// distance back (a clip whose stride is measured in world units) has to
// divide it out. Hardcoding the constant in two places is how they drift
// apart.
const CADENCE = 0.066;

const state = new Map<string, PlayerView>();

/** Per-player view state derived from frame-to-frame motion. See file header. */
export const viewState = {
  MAX_DISPLAY_SPEED: 480,

  CADENCE,

  // Gait cycle parameters, in world units. A stride is the distance covered
  // by one full two-step cycle, and it lengthens as a player speeds up.
  WALK_STRIDE: 130,
  RUN_STRIDE: 285,
  WALK_SPEED: 150,
  RUN_SPEED: 400,

  update(players: readonly ViewStatePlayer[], dt: number, pose?: ViewStatePose): void {
    for (const p of players) {
      const pos = pose !== undefined ? pose.players[p.id] : undefined;
      const effectivePos = pos ?? p.pos;
      const v = state.get(p.id);
      if (v === undefined) {
        state.set(p.id, { px: effectivePos.x, py: effectivePos.y, speed: 0, phase: 0, gait: 0, lean: 0 });
      } else if (dt > 0) {
        const vx = (effectivePos.x - v.px) / dt;
        const vy = (effectivePos.y - v.py) / dt;
        const sp = Math.min(viewState.MAX_DISPLAY_SPEED, Math.sqrt(vx * vx + vy * vy));
        // Exponential smoothing so the gait doesn't strobe on jittery steps.
        const k = clamp(dt * 8, 0, 1);
        v.speed = v.speed + (sp - v.speed) * k;
        v.phase = v.phase + sp * dt * CADENCE;

        // Normalised gait cycle, accumulated INCREMENTALLY.
        //
        // The stride lengthens with speed, so the obvious formulation --
        // cumulative_distance / current_stride -- is wrong: changing the
        // stride retroactively rescales every metre already travelled. A two
        // percent stride change after 4000 units jumps the phase by most of
        // a cycle, every frame that speed wobbles, which reads as the
        // animation flicking between a couple of poses.
        //
        // Advancing by the increment alone means a stride change only ever
        // affects the step being taken.
        const runMix = clamp(
          (sp - viewState.WALK_SPEED) / (viewState.RUN_SPEED - viewState.WALK_SPEED),
          0,
          1,
        );
        const stride = viewState.WALK_STRIDE + (viewState.RUN_STRIDE - viewState.WALK_STRIDE) * runMix;
        v.gait = (v.gait + (sp * dt) / stride) % 1;
        const targetLean = clamp(vx / 120, -1, 1);
        v.lean = v.lean + (targetLean - v.lean) * clamp(dt * 10, 0, 1);
        v.px = effectivePos.x;
        v.py = effectivePos.y;
      }
    }
  },

  get(id: string): PlayerView | undefined {
    return state.get(id);
  },

  // Drop all tracking (call when starting a fresh match so ids don't carry
  // over).
  reset(): void {
    state.clear();
  },
};
