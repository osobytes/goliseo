// Goal replays: a bounded, boundary-keyed buffer of lightweight render
// snapshots. When a goal goes in the sequence runs in two phases through the
// normal renderer (same camera): first a brief CELEBRATION -- the scorer
// wheels away and teammates converge to mob them -- then a slow-motion
// PLAYBACK of the last few seconds of footage. Game-layer only -- the sim
// knows nothing of it.
//
// Snapshots copy exactly what the pitch renderer touches. `Vec2`s are
// immutable in this codebase so aliasing player fields is safe; the ball is
// defensively copied because the (Rust) sim mutates ball position/velocity
// in place in its bounce code.
//
// Boundary note (ARCHITECTURE.md §4 rule 6): `MatchState`/`MatchPlayer`/
// `MatchEvent`/`OutfieldDecisionState`/`OutfieldPressState`/
// `PossessionTransitionState` are sim shapes owned by Rust `crates/gc-sim`.
// Only the fields this module reads/copies are declared locally below.
// `CombatMatchState` is reused from `@gc/presentation` (already a declared
// dependency of this package) rather than redeclared.
// `tuning.values.REPLAY_SECONDS`/`REPLAY_SLOWMO` (also Rust-owned even
// though the knobs themselves are presentation-only replay timing) are
// exposed here as a mutable `replay.tuning` property instead of importing
// the sim tuning module directly -- the same pattern `camera.ts` uses for
// `perspective_mode`.

import { Vec2 } from "@gc/core";
import type { CombatMatchState } from "@gc/presentation";

// --- sim-shaped inputs (the slice this module reads) -----------------------

export type OutfieldDecisionContext = "ineligible" | "offball" | "carrier";
export type OutfieldIntent =
  | "none"
  | "move"
  | "in_behind"
  | "come_short"
  | "hold_width"
  | "shoot"
  | "cross"
  | "pass"
  | "dribble";

export interface OutfieldDecisionState {
  readonly version: number;
  readonly generation: number;
  readonly rng_state: number;
  readonly remaining: number;
  readonly context: OutfieldDecisionContext;
  readonly intent: OutfieldIntent;
  readonly target_x?: number;
  readonly target_y?: number;
  readonly target_player?: number;
  readonly run_expires_at?: number;
}

export type StablePressMode = "inactive" | "contain" | "commit";
export type PressReason =
  "heavy_touch" | "exposed_ball" | "cover" | "box_desperation" | "low_discipline" | "no_trigger";

export interface OutfieldPressState {
  readonly version: number;
  readonly presser_index?: number;
  readonly mode: StablePressMode;
  readonly reason: PressReason;
}

export type TransitionTeam = "home" | "away";

export interface PossessionTransitionState {
  readonly version: number;
  readonly last_team?: TransitionTeam;
  readonly holding_team?: TransitionTeam;
  readonly hold: number;
  readonly turnover_team?: TransitionTeam;
  readonly elapsed: number;
}

export interface TransitionWindows {
  readonly counterpress: number;
  readonly counterattack: number;
}

export type SaveStyle = "spread" | "central" | "stretch";
export type KeeperBehaviorState = "base" | "advance" | "contain" | "set" | "retreat" | "recover";
export type AerialStyle = "leg_control" | "chest_control" | "volley" | "header" | "bicycle";
export type AerialOutcome = "clean" | "heavy" | "miss";

export type MatchEventKind =
  | "shot"
  | "pass"
  | "touch"
  | "tackle"
  | "press_commit_heavy_touch"
  | "press_commit_exposed_ball"
  | "press_commit_cover"
  | "press_commit_box_desperation"
  | "press_commit_low_discipline"
  | "combat_commit_carrier_contest"
  | "combat_commit_carrier_protection"
  | "combat_commit_loose_ball_contest"
  | "combat_commit_passing_lane_or_shot_denial"
  | "combat_commit_recovery_punish"
  | "combat_commit_unattributed_off_ball"
  | "catch"
  | "parry"
  | "tip"
  | "claim"
  | "block"
  | "header"
  | "volley"
  | "bicycle"
  | "reception"
  | "juke"
  | "first_touch_shot";

export interface MatchEvent {
  readonly kind: MatchEventKind;
  readonly x: number;
  readonly y: number;
  readonly player?: string;
  readonly save_style?: SaveStyle;
  readonly style?: AerialStyle;
  readonly outcome?: AerialOutcome;
  readonly jumping?: boolean;
  readonly difficulty?: number;
  readonly shot_type?: "ground" | "chip";
  readonly keeper_state?: KeeperBehaviorState;
  readonly keeper_depth?: number;
  readonly on_target?: boolean;
}

export interface Rect {
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

/** The slice of `MatchPlayer` (Rust `crates/gc-sim`) the replay buffer captures. */
export interface MatchPlayer {
  readonly id: string;
  readonly team: "home" | "away";
  readonly pos: Vec2;
  readonly run_vel: Vec2;
  readonly facing: Vec2;
  readonly radius: number;
  readonly is_keeper: boolean;
  readonly keeper_state: KeeperBehaviorState;
  readonly keeper_set: number;
  readonly slide_timer: number;
  readonly tackle_timer: number;
  readonly stun_timer: number;
  readonly settle_timer: number;
  readonly sprinting: boolean;
  readonly outfield_decision: OutfieldDecisionState;
  readonly dive_timer: number;
  readonly dive_dir: Vec2;
  readonly keeper_get_up_timer: number;
  readonly save_style?: SaveStyle;
  readonly grab_timer: number;
  readonly throw_timer: number;
  readonly windup_timer: number;
  readonly aerial_timer: number;
  readonly aerial_style?: AerialStyle;
  readonly aerial_outcome?: AerialOutcome;
  readonly aerial_jump: number;
  readonly sprint_meter: number;
  readonly jockey_timer: number;
}

/** A buffered player: `MatchPlayer` plus the three fields replay always zeroes on capture. */
export interface ReplayPlayer extends MatchPlayer {
  readonly charge: number;
  readonly pass_charge: number;
  readonly pass_target?: number;
}

/** The slice of `MatchState` (Rust `crates/gc-sim`) the replay buffer captures. */
export interface MatchState {
  readonly field: { readonly w: number; readonly h: number };
  readonly goal_home: Rect;
  readonly goal_away: Rect;
  readonly score: { readonly home: number; readonly away: number };
  readonly time_left: number;
  readonly outfield_press: { readonly home: OutfieldPressState; readonly away: OutfieldPressState };
  readonly transition: PossessionTransitionState;
  readonly transition_windows: {
    readonly home: TransitionWindows;
    readonly away: TransitionWindows;
  };
  readonly controlled?: number;
  readonly owner?: number;
  readonly ball: Vec2;
  readonly ball_vel: Vec2;
  readonly ball_z: number;
  readonly ball_vz: number;
  readonly players: readonly MatchPlayer[];
  readonly events: readonly MatchEvent[];
}

export interface ReplayFrame {
  readonly _replay_boundary: number;
  readonly _combat_state?: CombatMatchState;
  readonly field: { readonly w: number; readonly h: number };
  readonly goal_home: Rect;
  readonly goal_away: Rect;
  readonly score: { readonly home: number; readonly away: number };
  readonly time_left: number;
  readonly outfield_press: { readonly home: OutfieldPressState; readonly away: OutfieldPressState };
  readonly transition: PossessionTransitionState;
  readonly transition_windows: {
    readonly home: TransitionWindows;
    readonly away: TransitionWindows;
  };
  readonly controlled?: number;
  readonly owner?: number;
  readonly ball: Vec2;
  readonly ball_vel: Vec2;
  readonly ball_z: number;
  readonly ball_vz: number;
  readonly finished: boolean;
  readonly players: readonly ReplayPlayer[];
  readonly events: readonly MatchEvent[];
}

export interface ReplayTuning {
  /** seconds of footage the slow-motion playback window covers */
  replay_seconds: number;
  /** slow-motion playback speed multiplier */
  replay_slowmo: number;
}

const REPLAY_TUNING_DEFAULTS: ReplayTuning = { replay_seconds: 4, replay_slowmo: 0.35 };

export interface ReplayDiagnostics {
  readonly count: number;
  readonly oldest_boundary?: number;
  readonly newest_boundary?: number;
  readonly boundaries: readonly number[];
  readonly phase?: ReplayPhase;
  readonly celebration_elapsed: number;
}

export interface ReplayBoundarySample {
  readonly ball_x: number;
  readonly ball_y: number;
  readonly score_home: number;
  readonly score_away: number;
}

type ReplayPhase = "celebrate" | "playback";

function assertDefined<T>(value: T | undefined, message: string): T {
  if (value === undefined) {
    throw new Error(message);
  }
  return value;
}

const MAX_SECONDS = 8; // ring capacity; playback window length is tunable
// synthetic tail frames: the sim resets for kickoff on the goal frame, so we
// extend the final ball flight into the net ourselves (long enough to show
// the netting catch the ball and rebound it back out).
const EXTRAPOLATE = 20;

// Net-catch physics for the synthetic tail (mirrors the sim's loose-ball net
// handling, which the replay never runs). Hardcoded like the gravity
// constant below: this is presentation-only ballistics.
const TAIL_BALL_R = 6; // ball radius (sim BALL_RADIUS)
const TAIL_GRAVITY = 900; // downward accel on ball_vz (sim GRAVITY)
const TAIL_NET_DAMP = 0.3; // pace kept when the netting stops the ball (sim NET_DAMP)
const TAIL_CROSSBAR = 70; // roof net height; a scored ball stays under the bar (sim CROSSBAR)

// Celebration (presentation-only, plays before the replay cuts in).
const CELEBRATE_SECONDS = 1.6; // the "brief delay" before playback begins
const MATE_DELAY = 0.25; // teammates set off this fraction into the run (scorer leads)
const MOB_RADIUS = 30; // how tightly teammates ring the scorer when they arrive
const STRIKE: Readonly<Partial<Record<MatchEventKind, true>>> = {
  shot: true,
  header: true,
  volley: true,
  bicycle: true,
}; // events that score

let buf: ReplayFrame[] = [];
let legacyBoundary = 0;
let phase: ReplayPhase | undefined;
let playhead = 1;
let emitted = 0;
let replayWindow: ReplayFrame[] = [];
let activeEndBoundary: number | undefined;
// Celebration state (procedural: start poses + targets, animated by elapsed time).
let celElapsed = 0;
/** the goal-moment snapshot (start poses + goals + field) */
let celBase: ReplayFrame | undefined;
/** id of the player that scored */
let celScorer: string | undefined;
/** id -> target the player runs to */
let celTargets: Record<string, Vec2> | undefined;
/** ball resting in the net */
let celBall: Vec2 | undefined;

function cap(): number {
  return MAX_SECONDS * 60;
}

function clamp(x: number, a: number, b: number): number {
  return Math.max(a, Math.min(b, x));
}

// Smootherstep-lite: ease-in-out on 0..1, flat at both ends.
function smooth(raw: number): number {
  const u = clamp(raw, 0, 1);
  return u * u * (3 - 2 * u);
}

// --- shallow-copy helpers (mirror the sim's outfield-decision,
// outfield-press, and possession-transition `copy_state` functions, and
// combat's `snapshot`) -------------------------------------------------

function copyOutfieldDecisionState(s: OutfieldDecisionState): OutfieldDecisionState {
  return {
    version: s.version,
    generation: s.generation,
    rng_state: s.rng_state,
    remaining: s.remaining,
    context: s.context,
    intent: s.intent,
    ...(s.target_x !== undefined ? { target_x: s.target_x } : {}),
    ...(s.target_y !== undefined ? { target_y: s.target_y } : {}),
    ...(s.target_player !== undefined ? { target_player: s.target_player } : {}),
    ...(s.run_expires_at !== undefined ? { run_expires_at: s.run_expires_at } : {}),
  };
}

function copyOutfieldPressState(s: OutfieldPressState): OutfieldPressState {
  return {
    version: s.version,
    ...(s.presser_index !== undefined ? { presser_index: s.presser_index } : {}),
    mode: s.mode,
    reason: s.reason,
  };
}

function copyPossessionTransitionState(s: PossessionTransitionState): PossessionTransitionState {
  return {
    version: s.version,
    ...(s.last_team !== undefined ? { last_team: s.last_team } : {}),
    ...(s.holding_team !== undefined ? { holding_team: s.holding_team } : {}),
    hold: s.hold,
    ...(s.turnover_team !== undefined ? { turnover_team: s.turnover_team } : {}),
    elapsed: s.elapsed,
  };
}

function copyTransitionWindows(w: TransitionWindows): TransitionWindows {
  return { counterpress: w.counterpress, counterattack: w.counterattack };
}

function cloneCombatMatchState(s: CombatMatchState): CombatMatchState {
  return {
    ...(s.tick !== undefined ? { tick: s.tick } : {}),
    player_ids: [...s.player_ids],
    players: s.players.map((p) => ({ ...p })),
    projectiles: s.projectiles.map((p) => ({ ...p })),
  };
}

/** @param s the source MatchState */
function captureFrame(
  s: MatchState,
  boundary: number,
  combatState: CombatMatchState | undefined,
): ReplayFrame {
  const players: ReplayPlayer[] = s.players.map((p) => ({
    id: p.id,
    team: p.team,
    pos: p.pos,
    run_vel: p.run_vel,
    facing: p.facing,
    radius: p.radius,
    is_keeper: p.is_keeper,
    keeper_state: p.keeper_state,
    keeper_set: p.keeper_set,
    slide_timer: p.slide_timer,
    tackle_timer: p.tackle_timer,
    stun_timer: p.stun_timer,
    settle_timer: p.settle_timer,
    sprinting: p.sprinting,
    // `remaining` is decremented in place each tick, so the run grant the
    // telegraph pose reads has to be copied, not aliased.
    outfield_decision: copyOutfieldDecisionState(p.outfield_decision),
    dive_timer: p.dive_timer,
    dive_dir: p.dive_dir,
    keeper_get_up_timer: p.keeper_get_up_timer,
    ...(p.save_style !== undefined ? { save_style: p.save_style } : {}),
    grab_timer: p.grab_timer,
    throw_timer: p.throw_timer,
    windup_timer: p.windup_timer,
    aerial_timer: p.aerial_timer,
    ...(p.aerial_style !== undefined ? { aerial_style: p.aerial_style } : {}),
    ...(p.aerial_outcome !== undefined ? { aerial_outcome: p.aerial_outcome } : {}),
    aerial_jump: p.aerial_jump,
    // The HUD always models the live state, never a replay frame, so the
    // meter and the jockey stance can carry their real values and let a
    // replay show the condition the player was actually in.
    sprint_meter: p.sprint_meter,
    jockey_timer: p.jockey_timer,
    charge: 0,
    pass_charge: 0,
  }));
  const events = [...s.events];
  return {
    _replay_boundary: boundary,
    ...(combatState !== undefined ? { _combat_state: cloneCombatMatchState(combatState) } : {}),
    field: s.field,
    goal_home: s.goal_home,
    goal_away: s.goal_away,
    score: { home: s.score.home, away: s.score.away },
    time_left: s.time_left,
    // Team-owned press state is a pose input, and `outfield_press.contain`
    // returns a fresh table per transition, so a copy pins this frame's.
    outfield_press: {
      home: copyOutfieldPressState(s.outfield_press.home),
      away: copyOutfieldPressState(s.outfield_press.away),
    },
    // The contain pose is narrowed by the counter-press window, so the
    // transition memory it decays from is a pose input too. `observe`
    // mutates this state in place, hence the copy.
    transition: copyPossessionTransitionState(s.transition),
    transition_windows: {
      home: copyTransitionWindows(s.transition_windows.home),
      away: copyTransitionWindows(s.transition_windows.away),
    },
    ...(s.controlled !== undefined ? { controlled: s.controlled } : {}),
    ...(s.owner !== undefined ? { owner: s.owner } : {}),
    ball: new Vec2(s.ball.x, s.ball.y),
    ball_vel: new Vec2(s.ball_vel.x, s.ball_vel.y),
    ball_z: s.ball_z,
    ball_vz: s.ball_vz,
    finished: false,
    players,
    events,
  };
}

/** Index of the frame with this boundary (0-based), and whether it was an exact match. */
function boundaryIndex(boundary: number): readonly [number | undefined, boolean] {
  for (let index = 0; index < buf.length; index += 1) {
    const frame = assertDefined(buf[index], "replay buffer index out of range");
    if (frame._replay_boundary === boundary) {
      return [index, true];
    } else if (frame._replay_boundary > boundary) {
      return [index, false];
    }
  }
  return [undefined, false];
}

// Who scored: the most recent player on the scoring team to strike the ball
// (shot/header/volley), scanning the buffer back from the goal frame. Falls
// back to the scoring-team outfielder nearest the ball (deflections/own
// goals).
/** @param endIndex 0-based index into `buf` of the goal-moment snapshot */
function findScorer(
  scoringTeam: TransitionTeam,
  last: ReplayFrame,
  endIndex: number,
): string | undefined {
  const teamOf = new Map<string, TransitionTeam>();
  for (const p of last.players) {
    teamOf.set(p.id, p.team);
  }
  for (let cursor = endIndex; cursor >= 0; cursor -= 1) {
    const fr = buf[cursor];
    if (fr !== undefined) {
      for (let i = fr.events.length - 1; i >= 0; i -= 1) {
        const e = assertDefined(fr.events[i], "replay event index out of range");
        if (
          STRIKE[e.kind] === true &&
          e.player !== undefined &&
          teamOf.get(e.player) === scoringTeam
        ) {
          return e.player;
        }
      }
    }
  }
  let best: string | undefined;
  let bestD: number | undefined;
  for (const p of last.players) {
    if (p.team === scoringTeam && !p.is_keeper) {
      const d = p.pos.dist(last.ball);
      if (bestD === undefined || d < bestD) {
        bestD = d;
        best = p.id;
      }
    }
  }
  return best;
}

// Set up the celebration: scorer wheels away toward the corner they scored
// in, outfield teammates converge to mob them, everyone else holds
// (dejected).
function buildCelebration(scoringTeam: TransitionTeam, endIndex: number): boolean {
  const base = assertDefined(buf[endIndex], "celebration requires a captured frame");
  celBase = base;
  celScorer = findScorer(scoringTeam, base, endIndex);
  const scorer = celScorer;
  if (scorer === undefined) {
    return false;
  }
  const field = base.field;
  let scorerPos: Vec2 | undefined;
  const mates: string[] = [];
  for (const p of base.players) {
    if (p.id === scorer) {
      scorerPos = p.pos;
    } else if (p.team === scoringTeam && !p.is_keeper) {
      mates.push(p.id);
    }
  }
  const pos = assertDefined(scorerPos, "scorer position missing from captured frame");
  // Wheel toward the corner of the goal just scored in, nearest touchline.
  const cornerX = scoringTeam === "home" ? field.w - 70 : 70;
  const cornerY = pos.y < field.h / 2 ? 80 : field.h - 80;
  const corner = new Vec2(cornerX, cornerY);
  const targets: Record<string, Vec2> = { [scorer]: corner };
  // Teammates ring the corner so they arrive AROUND the scorer, not on top.
  mates.forEach((id, zeroBasedIndex) => {
    const i = zeroBasedIndex + 1;
    const ang = (i / mates.length) * 2 * Math.PI;
    targets[id] = new Vec2(
      clamp(cornerX + MOB_RADIUS * Math.cos(ang), 20, field.w - 20),
      clamp(cornerY + MOB_RADIUS * Math.sin(ang), 20, field.h - 20),
    );
  });
  celTargets = targets;
  // Ball rests in the net behind the line it crossed.
  const g = scoringTeam === "home" ? base.goal_away : base.goal_home;
  celBall = new Vec2(
    scoringTeam === "home" ? field.w + 12 : -12,
    clamp(base.ball.y, g.y + 10, g.y + g.h - 10),
  );
  celElapsed = 0;
  return true;
}

// Build the slow-motion window: the last REPLAY_SECONDS of footage plus a
// synthetic net-bulge tail (the sim reset to kickoff on the goal frame).
/** @param endIndex 0-based index into `buf` */
function buildWindow(n: number, endIndex: number): void {
  replayWindow = [];
  const first = endIndex - n + 1;
  for (let index = first; index <= endIndex; index += 1) {
    replayWindow.push(assertDefined(buf[index], "replay window frame out of range"));
  }
  // Synthetic tail: fly the ball on while everyone holds their final pose,
  // and let the goal net catch it. The ball drives into the netting, sheds
  // most of its pace, and rebounds gently back toward the line -- so a goal
  // reads clearly instead of the ball sailing straight out the back.
  const last = assertDefined(replayWindow[replayWindow.length - 1], "replay window is empty");
  // The ball scored, so it's crossing one goal line; pick which by heading.
  const intoAway = last.ball_vel.x >= 0;
  const gg = intoAway ? last.goal_away : last.goal_home;
  // Back netting plane (ball centre clamp) and the y-extent of the mouth.
  const backX = intoAway ? gg.x + gg.w - TAIL_BALL_R : gg.x + TAIL_BALL_R;
  const yLo = gg.y + TAIL_BALL_R;
  const yHi = gg.y + gg.h - TAIL_BALL_R;
  let bx = last.ball.x;
  let by = last.ball.y;
  let bz = last.ball_z;
  let bvz = last.ball_vz;
  let vx = last.ball_vel.x;
  let vy = last.ball_vel.y;
  const dt = 1 / 60;
  for (let step = 0; step < EXTRAPOLATE; step += 1) {
    bx = bx + vx * dt;
    by = by + vy * dt;
    bvz = bvz - TAIL_GRAVITY * dt;
    bz = bz + bvz * dt;
    if (bz <= 0) {
      bz = 0;
      bvz = 0;
    } else if (bz > TAIL_CROSSBAR) {
      bz = TAIL_CROSSBAR; // roof net holds it under the bar
      if (bvz > 0) {
        bvz = 0;
      }
    }
    // Back net: clamp at the netting and rebound with much less force.
    if ((intoAway && bx > backX) || (!intoAway && bx < backX)) {
      bx = backX;
      vx = -vx * TAIL_NET_DAMP;
    }
    // Side nets: keep the ball inside the mouth as it settles.
    if (by < yLo) {
      by = yLo;
      vy = -vy * TAIL_NET_DAMP;
    } else if (by > yHi) {
      by = yHi;
      vy = -vy * TAIL_NET_DAMP;
    }
    replayWindow.push({
      ...last,
      ball: new Vec2(bx, by),
      ball_z: bz,
      ball_vel: new Vec2(vx, vy),
      ball_vz: bvz,
      events: [],
    });
  }
}

// Begin the goal sequence: a celebration beat, then slow-motion playback of
// the last REPLAY_SECONDS (+ net-bulge tail).
/** @param endIndex 0-based index into `buf` */
function startAtIndex(scoringTeam: TransitionTeam, endIndex: number): boolean {
  // `endIndex` is 0-based; the number of frames available up to and
  // including it is one more than the index.
  const availableCount = endIndex + 1;
  const n = Math.min(availableCount, Math.floor(replay.tuning.replay_seconds * 60));
  if (n < 30) {
    return false; // not enough footage to be worth showing
  }
  buildWindow(n, endIndex);
  const frame = assertDefined(buf[endIndex], "replay start index out of range");
  activeEndBoundary = frame._replay_boundary;
  playhead = 1;
  emitted = 0;
  // Celebrate first if we can name a scorer; otherwise cut straight to
  // replay.
  if (buildCelebration(scoringTeam, endIndex)) {
    phase = "celebrate";
  } else {
    phase = "playback";
  }
  return true;
}

// One celebration frame: players ease from their goal-moment poses to their
// targets (scorer leads, teammates set off a beat later), ball sat in the
// net.
function celebrationFrame(): ReplayFrame {
  const T = CELEBRATE_SECONDS;
  const base = assertDefined(celBase, "celebration frame requires a captured base");
  const targets = assertDefined(celTargets, "celebration frame requires targets");
  const ball = assertDefined(celBall, "celebration frame requires a resting ball position");
  const players: ReplayPlayer[] = base.players.map((p) => {
    const tgt = targets[p.id];
    if (tgt === undefined) {
      return { ...p };
    }
    // Teammates lead-in later so the scorer visibly sets off first.
    const u =
      p.id === celScorer
        ? smooth(celElapsed / T)
        : smooth((celElapsed - MATE_DELAY * T) / ((1 - MATE_DELAY) * T));
    const pos = p.pos.add(tgt.sub(p.pos).scale(u));
    const d = tgt.sub(p.pos);
    if (u > 0.02 && u < 0.98 && d.length() > 1) {
      return { ...p, pos, facing: d.normalized() };
    }
    return { ...p, pos };
  });
  return {
    ...base,
    players,
    ball,
    ball_z: 0,
    ball_vel: new Vec2(0, 0),
    ball_vz: 0,
    events: [],
  };
}

/** Goal replays: a bounded, boundary-keyed buffer of lightweight render snapshots. See file header. */
export const replay = {
  /** Presentation-only replay timing, mirroring the sim's `REPLAY_SECONDS`/`REPLAY_SLOWMO` knobs. */
  tuning: { ...REPLAY_TUNING_DEFAULTS },

  resetTuning(): void {
    replay.tuning.replay_seconds = REPLAY_TUNING_DEFAULTS.replay_seconds;
    replay.tuning.replay_slowmo = REPLAY_TUNING_DEFAULTS.replay_slowmo;
  },

  capacity(): number {
    return cap();
  },

  reset(): void {
    buf = [];
    legacyBoundary = 0;
    phase = undefined;
    replayWindow = [];
    activeEndBoundary = undefined;
  },

  active(): boolean {
    return phase !== undefined;
  },

  // True only during the pre-replay celebration beat (the HUD shows GOAL,
  // not REPLAY).
  celebrating(): boolean {
    return phase === "celebrate";
  },

  stop(): void {
    phase = undefined;
    replayWindow = [];
    activeEndBoundary = undefined;
  },

  // Record or replace one canonical simulation boundary. Corrected rollback
  // footage uses this API so obsolete interval frames cannot survive by
  // index.
  recordBoundary(boundary: number, s: MatchState, combatState?: CombatMatchState): void {
    if (!(Number.isInteger(boundary) && boundary >= 0)) {
      throw new Error("replay boundary must be a non-negative integer");
    }
    const frame = captureFrame(s, boundary, combatState);
    const [index, found] = boundaryIndex(boundary);
    if (found) {
      buf[assertDefined(index, "replay boundary index missing on an exact match")] = frame;
    } else if (index !== undefined) {
      buf.splice(index, 0, frame);
    } else {
      buf.push(frame);
    }
    while (buf.length > cap()) {
      buf.shift();
    }
  },

  // Delete a corrected speculative tail before inserting replacement frames.
  truncateFrom(boundary: number): void {
    const [index] = boundaryIndex(boundary);
    if (index !== undefined) {
      buf.length = index;
    }
    if (phase !== undefined && activeEndBoundary !== undefined && boundary <= activeEndBoundary) {
      replay.stop();
    }
  },

  // Record one legacy live frame (called before its simulation step). The
  // compatibility adapter supplies a private contiguous boundary sequence.
  record(s: MatchState, combatState?: CombatMatchState): void {
    replay.recordBoundary(legacyBoundary, s, combatState);
    legacyBoundary += 1;
  },

  // Begin the legacy goal sequence at the newest recorded frame.
  start(scoringTeam: TransitionTeam): boolean {
    return startAtIndex(scoringTeam, buf.length - 1);
  },

  // Begin a confirmed rollback goal sequence at its corrected pre-goal
  // boundary, even when newer speculative/live simulation boundaries already
  // exist.
  startAt(scoringTeam: TransitionTeam, boundary: number): boolean {
    const [index, found] = boundaryIndex(boundary);
    if (!found || index === undefined) {
      return false;
    }
    return startAtIndex(scoringTeam, index);
  },

  diagnostics(): ReplayDiagnostics {
    const boundaries = buf.map((frame) => frame._replay_boundary);
    const oldest = buf[0];
    const newest = buf[buf.length - 1];
    return {
      count: buf.length,
      ...(oldest !== undefined ? { oldest_boundary: oldest._replay_boundary } : {}),
      ...(newest !== undefined ? { newest_boundary: newest._replay_boundary } : {}),
      boundaries,
      ...(phase !== undefined ? { phase } : {}),
      celebration_elapsed: celElapsed,
    };
  },

  boundarySample(boundary: number): ReplayBoundarySample | undefined {
    const [index, found] = boundaryIndex(boundary);
    if (!found || index === undefined) {
      return undefined;
    }
    const frame = assertDefined(buf[index], "replay boundary sample index out of range");
    return {
      ball_x: frame.ball.x,
      ball_y: frame.ball.y,
      score_home: frame.score.home,
      score_away: frame.score.away,
    };
  },

  // Advance the sequence and return an interpolated, drawable
  // ReplayFrame -- or undefined when it has finished. During playback
  // `state.events` carries each recorded frame's events exactly once (for
  // the effects layer), regardless of how slowly the playhead crawls.
  step(dt: number): ReplayFrame | undefined {
    if (phase === "celebrate") {
      celElapsed += dt;
      const st = celebrationFrame();
      if (celElapsed >= CELEBRATE_SECONDS) {
        phase = "playback"; // next step cuts to the slow-motion replay
      }
      return st;
    }
    if (phase !== "playback") {
      return undefined;
    }
    playhead = playhead + dt * 60 * replay.tuning.replay_slowmo;
    const i = Math.floor(playhead);
    if (i >= replayWindow.length) {
      phase = undefined;
      replayWindow = [];
      activeEndBoundary = undefined;
      return undefined;
    }
    // `i` is 1-based (matching `playhead`'s starting value); only the array
    // reads below are shifted by one for `replayWindow`'s 0-based storage,
    // so `t`/`emitted`'s arithmetic stays untouched.
    const a = assertDefined(replayWindow[i - 1], "replay window pair missing frame a");
    const b = assertDefined(replayWindow[i], "replay window pair missing frame b");
    const t = playhead - i;
    const ball = a.ball.add(b.ball.sub(a.ball).scale(t));
    const ballZ = a.ball_z + (b.ball_z - a.ball_z) * t;
    const players: ReplayPlayer[] = a.players.map((pa, index) => {
      const pb = assertDefined(b.players[index], "replay window pair missing a player");
      return { ...pa, pos: pa.pos.add(pb.pos.sub(pa.pos).scale(t)) };
    });
    const events = i > emitted ? a.events : [];
    emitted = Math.max(emitted, i);
    return { ...a, ball, ball_z: ballZ, players, events };
  },
};
