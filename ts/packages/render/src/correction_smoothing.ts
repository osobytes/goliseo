// Ported from game/render/correction_smoothing.lua.
//
// Pure render-only reconciliation for corrected player and ball positions.
// The authoritative MatchState is read, never copied into or mutated by this
// model. Small corrections preserve the last displayed pose and linearly shed
// their offset over render time; large corrections snap immediately.
//
// One-directional by construction: every function here takes a `MatchState`
// (or a `CorrectionSmoothingState` built from one) and returns a NEW
// `CorrectionSmoothingState` -- a display-only offset table plus a displayed
// pose. Nothing returned ever has the shape of, or is fed back into, sim
// state; `advance`/`correct`/`reconcile` never mutate their `source` argument
// (see the purity assertion in correction_smoothing.spec.ts). That is what
// keeps a snapped rollback correction out of the determinism loop -- see
// v2/README.md's "Correction smoothing is TS, and strictly one-directional."
//
// Boundary note: `MatchState` is a sim shape (v2/README.md rule 6.7,
// `sim/**` -> Rust `crates/gc-sim`). Only the fields this module reads
// (`players[].id`, `players[].pos`, `ball`) are declared locally, matching
// `@gc/presentation`'s `combat.ts`.

export interface CorrectionSmoothingPoint {
  readonly x: number;
  readonly y: number;
}

export interface CorrectionSmoothingPose {
  readonly players: Readonly<Record<string, CorrectionSmoothingPoint>>;
  readonly ball: CorrectionSmoothingPoint;
}

export interface CorrectionSmoothingOffset {
  readonly x: number;
  readonly y: number;
  readonly remaining: number;
}

export interface CorrectionSmoothingOptions {
  readonly duration?: number;
  readonly hard_snap_distance?: number;
}

export interface CorrectionSmoothingState {
  readonly duration: number;
  readonly hard_snap_distance: number;
  readonly player_offsets: Readonly<Record<string, CorrectionSmoothingOffset>>;
  readonly ball_offset: CorrectionSmoothingOffset | undefined;
  readonly displayed: CorrectionSmoothingPose;
}

export interface CorrectionSmoothingDiagnostics {
  readonly maximum_magnitude: number;
  readonly active_count: number;
}

// --- sim-shaped input (the slice this module actually reads) --------------

export interface CorrectionSmoothingSourcePlayer {
  readonly id: string;
  readonly pos: CorrectionSmoothingPoint;
}

export interface CorrectionSmoothingSource {
  readonly players: readonly CorrectionSmoothingSourcePlayer[];
  readonly ball: CorrectionSmoothingPoint;
}

const DEFAULT_DURATION = 0.1;
const DEFAULT_HARD_SNAP_DISTANCE = 160;
const EPSILON = 1e-9;

function assertNonnegativeFinite(value: number, label: string): void {
  if (!(Number.isFinite(value) && value >= 0)) {
    throw new Error(`${label} must be a non-negative finite number`);
  }
}

function copyPoint(point: CorrectionSmoothingPoint): CorrectionSmoothingPoint {
  return { x: point.x, y: point.y };
}

function copyPose(pose: CorrectionSmoothingPose): CorrectionSmoothingPose {
  const players: Record<string, CorrectionSmoothingPoint> = {};
  for (const [id, point] of Object.entries(pose.players)) {
    players[id] = copyPoint(point);
  }
  return { players, ball: copyPoint(pose.ball) };
}

function authoritativePose(source: CorrectionSmoothingSource): CorrectionSmoothingPose {
  const players: Record<string, CorrectionSmoothingPoint> = {};
  for (const player of source.players) {
    players[player.id] = copyPoint(player.pos);
  }
  return { players, ball: copyPoint(source.ball) };
}

function copyOffset(offset: CorrectionSmoothingOffset): CorrectionSmoothingOffset {
  return { x: offset.x, y: offset.y, remaining: offset.remaining };
}

function correctedOffset(
  previous: CorrectionSmoothingPoint | undefined,
  authoritative: CorrectionSmoothingPoint,
  duration: number,
  hardSnapDistance: number,
): CorrectionSmoothingOffset | undefined {
  if (previous === undefined || duration <= EPSILON) {
    return undefined;
  }
  const x = previous.x - authoritative.x;
  const y = previous.y - authoritative.y;
  const magnitude = Math.sqrt(x * x + y * y);
  if (magnitude <= EPSILON || magnitude >= hardSnapDistance) {
    return undefined;
  }
  return { x, y, remaining: duration };
}

function displayedPoint(
  authoritative: CorrectionSmoothingPoint,
  offset: CorrectionSmoothingOffset | undefined,
): CorrectionSmoothingPoint {
  if (offset === undefined) {
    return copyPoint(authoritative);
  }
  return { x: authoritative.x + offset.x, y: authoritative.y + offset.y };
}

function decayedOffset(
  offset: CorrectionSmoothingOffset | undefined,
  dt: number,
): CorrectionSmoothingOffset | undefined {
  if (offset === undefined) {
    return undefined;
  }
  const remaining = Math.max(0, offset.remaining - dt);
  if (remaining <= EPSILON) {
    return undefined;
  }
  const scale = remaining / offset.remaining;
  return { x: offset.x * scale, y: offset.y * scale, remaining };
}

/** Pure render-only reconciliation for corrected player and ball positions. See file header. */
export const correctionSmoothing = {
  DEFAULT_DURATION,
  DEFAULT_HARD_SNAP_DISTANCE,

  new(source: CorrectionSmoothingSource, options?: CorrectionSmoothingOptions): CorrectionSmoothingState {
    const duration = options?.duration ?? DEFAULT_DURATION;
    const hardSnapDistance = options?.hard_snap_distance ?? DEFAULT_HARD_SNAP_DISTANCE;
    assertNonnegativeFinite(duration, "correction smoothing duration");
    assertNonnegativeFinite(hardSnapDistance, "correction hard-snap distance");
    if (!(hardSnapDistance > 0)) {
      throw new Error("correction hard-snap distance must be positive");
    }
    return {
      duration,
      hard_snap_distance: hardSnapDistance,
      player_offsets: {},
      ball_offset: undefined,
      displayed: authoritativePose(source),
    };
  },

  // Start a correction without advancing presentation time. Every smoothed
  // drawable therefore begins exactly at its pose from the preceding render.
  correct(state: CorrectionSmoothingState, source: CorrectionSmoothingSource): CorrectionSmoothingState {
    const authoritative = authoritativePose(source);
    const offsets: Record<string, CorrectionSmoothingOffset> = {};
    const displayedPlayers: Record<string, CorrectionSmoothingPoint> = {};
    for (const [id, point] of Object.entries(authoritative.players)) {
      const offset = correctedOffset(
        state.displayed.players[id],
        point,
        state.duration,
        state.hard_snap_distance,
      );
      if (offset !== undefined) {
        offsets[id] = offset;
      }
      displayedPlayers[id] = displayedPoint(point, offset);
    }
    const ballOffset = correctedOffset(
      state.displayed.ball,
      authoritative.ball,
      state.duration,
      state.hard_snap_distance,
    );
    return {
      duration: state.duration,
      hard_snap_distance: state.hard_snap_distance,
      player_offsets: offsets,
      ball_offset: ballOffset,
      displayed: {
        players: displayedPlayers,
        ball: displayedPoint(authoritative.ball, ballOffset),
      },
    };
  },

  // Advance only with render dt. Authoritative movement remains immediate
  // under the decaying correction offset; this state is never fed back into
  // the sim.
  advance(
    state: CorrectionSmoothingState,
    source: CorrectionSmoothingSource,
    dt: number,
  ): CorrectionSmoothingState {
    assertNonnegativeFinite(dt, "correction smoothing render dt");
    const authoritative = authoritativePose(source);
    const offsets: Record<string, CorrectionSmoothingOffset> = {};
    const displayedPlayers: Record<string, CorrectionSmoothingPoint> = {};
    for (const [id, point] of Object.entries(authoritative.players)) {
      const previous = state.player_offsets[id];
      const offset = previous !== undefined ? decayedOffset(copyOffset(previous), dt) : undefined;
      if (offset !== undefined) {
        offsets[id] = offset;
      }
      displayedPlayers[id] = displayedPoint(point, offset);
    }
    const ballOffset =
      state.ball_offset !== undefined ? decayedOffset(copyOffset(state.ball_offset), dt) : undefined;
    return {
      duration: state.duration,
      hard_snap_distance: state.hard_snap_distance,
      player_offsets: offsets,
      ball_offset: ballOffset,
      displayed: {
        players: displayedPlayers,
        ball: displayedPoint(authoritative.ball, ballOffset),
      },
    };
  },

  // Capture a corrected authority pose and spend the current render frame's
  // dt immediately. This preserves continuity at the start of the frame
  // without allowing consecutive correction frames to freeze the displayed
  // trajectory.
  reconcile(
    state: CorrectionSmoothingState,
    source: CorrectionSmoothingSource,
    dt: number,
  ): CorrectionSmoothingState {
    return correctionSmoothing.advance(correctionSmoothing.correct(state, source), source, dt);
  },

  // Clear offsets at a scene discontinuity while preserving configured
  // tuning.
  clear(state: CorrectionSmoothingState, source: CorrectionSmoothingSource): CorrectionSmoothingState {
    return correctionSmoothing.new(source, {
      duration: state.duration,
      hard_snap_distance: state.hard_snap_distance,
    });
  },

  pose(state: CorrectionSmoothingState): CorrectionSmoothingPose {
    return copyPose(state.displayed);
  },

  diagnostics(state: CorrectionSmoothingState): CorrectionSmoothingDiagnostics {
    let activeCount = 0;
    let maximumMagnitude = 0;
    for (const offset of Object.values(state.player_offsets)) {
      activeCount += 1;
      maximumMagnitude = Math.max(maximumMagnitude, Math.sqrt(offset.x * offset.x + offset.y * offset.y));
    }
    if (state.ball_offset !== undefined) {
      activeCount += 1;
      maximumMagnitude = Math.max(
        maximumMagnitude,
        Math.sqrt(state.ball_offset.x * state.ball_offset.x + state.ball_offset.y * state.ball_offset.y),
      );
    }
    return { maximum_magnitude: maximumMagnitude, active_count: activeCount };
  },
};
