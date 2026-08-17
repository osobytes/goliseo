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
// shapes (ARCHITECTURE.md §4 rule 6). Only the fields this module reads are
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
  //
  // Retuned for the sim's actual reachable envelope (gc-sim stats.rs, SPRINT_MULT
  // 1.35, roster pace 4-8): base move speed is 140-220 u/s and a full sprint
  // tops out around 297-351 u/s. The previous WALK_SPEED/RUN_SPEED pair (150/400)
  // meant runMix never exceeded ~0.59 at the sim's fastest sprint, so the run
  // clip was permanently under-weighted and every player read as a brisk walk.
  //
  // Cadence (cycles/sec = speed / stride) at these thresholds: 90/100 = 0.9
  // at WALK_SPEED, 260/185 ≈ 1.41 at RUN_SPEED -- in the same 0.8-1.1 / 1.3-1.5
  // bands the old 150/130 ≈ 1.15 and 400/285 ≈ 1.40 pair read at.
  //
  // A STRIDE IS A CONTRACT WITH THE CLIP, and #574 established that the clip
  // cannot currently honour it: a stride says how much ground one authored
  // cycle covers, so the planted foot has to sweep backward by
  // `duty x stride` in body frame. At 185 with a run's ~0.27 duty that is
  // 50 wu, and the run clip's authored foot sweep is 43 wu -- which is already
  // the GEOMETRIC MAXIMUM for this rig's 0.66 m leg at a +/-45 degree split.
  // The gap is why the stance foot skates, and it cannot be closed by widening
  // the pose (there is no room) or by shortening the stride (that buys the
  // ground back by raising cadence, which is the fast-forward read the retune
  // exists to avoid). It is closed by concentrating the sweep into a real
  // stance window, which needs keyframes the cycles do not have yet -- see the
  // gait-duty issue. These values are deliberately UNCHANGED until then, and
  // `rig3d/foot_contact.spec.ts` measures the gap rather than leaving it
  // invisible.
  WALK_STRIDE: 100,
  RUN_STRIDE: 185,
  WALK_SPEED: 90,
  RUN_SPEED: 260,

  update(players: readonly ViewStatePlayer[], dt: number, pose?: ViewStatePose): void {
    for (const p of players) {
      const pos = pose !== undefined ? pose.players[p.id] : undefined;
      const effectivePos = pos ?? p.pos;
      const v = state.get(p.id);
      if (v === undefined) {
        state.set(p.id, {
          px: effectivePos.x,
          py: effectivePos.y,
          speed: 0,
          phase: 0,
          gait: 0,
          lean: 0,
        });
      } else if (dt > 0) {
        const vx = (effectivePos.x - v.px) / dt;
        const vy = (effectivePos.y - v.py) / dt;
        const sp = Math.min(viewState.MAX_DISPLAY_SPEED, Math.sqrt(vx * vx + vy * vy));
        // Exponential smoothing so the gait doesn't strobe on jittery steps.
        //
        // TIGHTENED IN #574 from `dt * 8` (time constant 125 ms) to `dt * 16`
        // (62 ms). What players call "slow" is usually RESPONSIVENESS -- the
        // delay between the simulation doing something and the animation
        // showing it -- rather than playback rate, and this filter was the
        // largest single contributor on the render side: at 125 ms the
        // locomotion blend took ~290 ms to reach 90% of a speed change the sim
        // had already applied within one tick, so a player who set off sprinting
        // kept a jog's pose while their body was already moving.
        //
        // The strobing this exists to prevent is still prevented: 62 ms is ~4
        // frames at 60 Hz, and the gait PHASE below does not read this value at
        // all (it accumulates from raw `sp`), so what is being smoothed is only
        // the idle/walk/run blend weight.
        const k = clamp(dt * 16, 0, 1);
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
        const stride =
          viewState.WALK_STRIDE + (viewState.RUN_STRIDE - viewState.WALK_STRIDE) * runMix;
        v.gait = (v.gait + (sp * dt) / stride) % 1;
        // Lean tightened alongside the speed filter above, and for the same
        // reason -- a torso that banks into a turn 100 ms after the body has
        // already turned reads as the body dragging the pose behind it.
        const targetLean = clamp(vx / 120, -1, 1);
        v.lean = v.lean + (targetLean - v.lean) * clamp(dt * 14, 0, 1);
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
