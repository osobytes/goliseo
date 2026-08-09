// Port of `render/frame_buffer.lua`'s DECODE half.
//
// `render/frame.lua` (Rust: `crates/gc-render/src/frame.rs`) produces a
// `RenderFrame`. `render/frame_buffer.lua` (Rust: `crates/gc-render/src/
// frame_buffer.rs`) flattens it into one contiguous array of doubles so it
// can cross a boundary in one call instead of one crossing per field per
// entity per frame. In the Lua original both directions -- `encode` and
// `decode` -- lived in the same process, because Lua was on both sides of
// the boundary. In v2 the wire is produced by Rust (`gc_render::frame_buffer
// ::encode`/`encode_roster`) and consumed by TypeScript, and only the
// consuming half existed to be written: this module is that half. See
// `v2/README.md` #1 -- this is a gap in the bridge the milestone scope
// created, not a stray port.
//
// FOUR ENCODING RULES this reader relies on (mirrored from the Rust/Lua
// module headers, which force them from `render/frame.lua`'s shape):
//
// 1. Enums are small integers, 1-based, and 0 always means absent.
// 2. Sparse *numbers* (`difficulty`, `keeper_depth`) carry an explicit
//    `has_*` presence flag, because 0 is a legitimate value for them.
// 3. One-based roster slots (`possession.owner`, `control.pass_target`, an
//    event's `slot`) are NOT enum-coded: 0 is a free "none" sentinel
//    because a real slot is always >= 1. `data/2` above bumped upward
//    for enums for the same reason.
// 4. Per-frame strings never cross: `hud.controlled_id` and an event's
//    `player` are dropped from the wire and are recoverable by joining
//    `hud.controlled` / an event's `slot` against the roster's `ids`
//    (crossed once, via `decodeRoster`).
//
// WHAT THIS DECODER DOES DIFFERENTLY FROM THE RUST/LUA `decode`. Both
// reference implementations deliberately decode to a "raw" shape --
// `DecodedRenderFrame`/`DecodedRenderFrameRoster` in Rust, a plain
// `scalars`/`players`/`events` table of numbers in Lua -- keyed by field
// name but with enum codes left UNRESOLVED, plus a `pose_id`/`event_kind`
// convenience function a caller invokes per lookup. Both docstrings say why:
// "a decoded frame has integer roster slots where the payload had id
// strings, and no `combat`. Typing it as the thing it is keeps a reader from
// assuming the round trip is lossless." That caution is aimed at Rust's own
// differential test, which is the only consumer `crates/gc-render`'s decode
// currently has.
//
// TypeScript's decode has a real consumer instead: `pitch.ts` already
// declares and consumes a `RenderFrame` shape with enums resolved to strings
// (`teams: readonly ("home" | "away")[]`, `pose_id: readonly (string |
// undefined)[]`, ...). Per this task's brief, that type is authoritative and
// is not to be shadowed by a second, code-shaped type -- so `decode` and
// `decodeRoster` resolve every enum inline instead of leaving that to a
// per-lookup convenience wrapper. The caution the Rust/Lua types encode
// still applies and is preserved the only way it can be without inventing a
// parallel shape: `decode`'s return type omits `roster` and `combat`
// (neither travels in a per-frame block -- see rule 4 and "WHAT IS NOT
// CARRIED" below) rather than declaring them and lying about their contents.
// `toRenderFrame` is what combines a decoded frame with a decoded roster
// (and an externally-sourced combat model, if any) into the real, complete
// `RenderFrame` `pitch.ts` expects.
//
// WHAT IS NOT CARRIED: `RenderFrame.combat`. Flattening a
// `CombatPresentationModel` (`Vec2`-valued) is `@gc/presentation`'s
// job, not this module's -- see `crates/gc-render/src/frame_buffer.rs`'s
// identical note. The header still stamps `combat_present` (word index 10,
// 0-based) so a reader can tell "this frame had no combat telegraphs" from
// "this layout cannot carry them yet"; `decode` surfaces it as
// `combatPresent` rather than silently dropping it.
//
// VERSIONING. Two numbers are stamped and both are checked here, because
// they can change independently: `RENDER_FRAME_VERSION` (the payload's
// meaning) and `LAYOUT_VERSION` (this block's shape). Neither can be
// imported from the Rust producer -- v2/README.md forbids a TS package
// importing a Rust crate's source -- so both are duplicated constants that
// must be bumped in lockstep with `crates/gc-render/src/frame_buffer.rs`.
//
// Every failure mode below throws, matching the Lua original's exclusive use
// of `assert` here (never `return nil, err`) and the Rust port's `assert!`/
// `panic!`: a version mismatch, a corrupted header, an unmapped enum code or
// a malformed roster string blob are protocol violations between a Rust
// producer and this reader, not expected, recoverable input (AGENTS.md §7).

import type { CombatPresentationModel } from "@gc/presentation";
import type { RGB } from "./draw2d.ts";
import type {
  RenderFrame as PitchRenderFrame,
  RenderFrameBall,
  RenderFrameControl,
  RenderFrameField,
  RenderFramePlayers,
  RenderFrameRoster,
} from "./pitch.ts";
import type { AerialOutcome, AerialStyle, SpeciesShape } from "./player_render_options.ts";

// ---------------------------------------------------------------------------
// Layout constants. Mirror `crates/gc-render/src/frame_buffer.rs` exactly;
// bump alongside `LAYOUT_VERSION` there.
// ---------------------------------------------------------------------------

/** Shape of this block: field order, header size, enum numberings. */
export const LAYOUT_VERSION = 1;

/**
 * The `RenderFrame` protocol version this decoder understands
 * (`crate::frame::VERSION` on the Rust side). See this module's header for
 * why it cannot be imported instead of duplicated.
 */
export const RENDER_FRAME_VERSION = 1;

/** "GOLF" as big-endian ASCII: a frame block's sentinel first word. */
export const MAGIC = 0x474f4c46;

/** "GOLR" as big-endian ASCII: a roster block's sentinel first word. */
export const ROSTER_MAGIC = 0x474f4c52;

/** Header word count for a frame block. */
export const HEADER_WORDS = 12;

/** The once-per-frame scalar word count: field geometry, ball, possession, control, HUD. */
export const SCALAR_FIELD_COUNT = 44;

/** Per-player field count (structure-of-arrays, field-major). */
export const PLAYER_FIELD_COUNT = 21;

/** Per-event field count (structure-of-arrays, field-major). */
export const EVENT_FIELD_COUNT = 15;

/** Header word count for a roster block. */
export const ROSTER_HEADER_WORDS = 7;

/** Per-slot field count for a roster block (structure-of-arrays, field-major). */
export const ROSTER_FIELD_COUNT = 7;

/** Exact word count of a frame block, given its player and event counts. */
export function frameWords(count: number, eventCount: number): number {
  return HEADER_WORDS + SCALAR_FIELD_COUNT + PLAYER_FIELD_COUNT * count + EVENT_FIELD_COUNT * eventCount;
}

// ---------------------------------------------------------------------------
// Types this decoder produces beyond what already exists in this package.
//
// `RenderFrame`/`RenderFrameField`/`RenderFrameBall`/`RenderFrameControl`/
// `RenderFramePlayers`/`RenderFrameRoster` come from `pitch.ts` (task rule:
// conform to the existing type, do not invent a parallel one). Everything
// below is genuinely new: the wire carries these fields and nothing in this
// package -- `pitch.ts` only declares "the slice `pitch.ts` reads" -- had a
// shape for them yet. See this file's end-of-module report for the exact
// list, reported per the task brief rather than silently added to
// `pitch.ts`, which this package does not own this wave.
// ---------------------------------------------------------------------------

/** Wire-carried closed set. Numbered exactly as `render/player_pose.lua`'s `PlayerPoseId` alias, 1..32. */
export type PlayerPoseId =
  | "keeper_grab"
  | "keeper_throw"
  | "keeper_punt"
  | "keeper_tip"
  | "keeper_spread"
  | "keeper_central"
  | "keeper_stretch"
  | "keeper_dive"
  | "keeper_get_up"
  | "keeper_set"
  | "keeper_ready_low"
  | "keeper_shuffle"
  | "keeper_ready_tall"
  | "aerial_bicycle"
  | "aerial_action"
  | "combat_knockback"
  | "combat_stagger"
  | "combat_guard"
  | "combat_active"
  | "combat_windup"
  | "combat_aim"
  | "combat_recovery"
  | "soccer_windup"
  | "slide"
  | "tackle"
  | "stumble"
  | "kick_follow"
  | "settle"
  | "run_telegraph"
  | "contain"
  | "fatigue"
  | "locomotion";

/** Which system selected the shown pose. Mirrors `render/player_pose.lua`'s `PlayerPoseSource` alias. */
export type PlayerPoseSource = "soccer" | "combat" | "locomotion";

/** A save's presentation style. Mirrors `sim/keeper.lua`'s `SaveStyle` alias. */
export type SaveStyle = "spread" | "central" | "stretch";

/** A keeper's behavior state at an event. Mirrors `sim/keeper.lua`'s `KeeperBehaviorState` alias. */
export type KeeperBehaviorState = "base" | "advance" | "contain" | "set" | "retreat" | "recover";

/** A keeper-relevant shot's presentation type. Mirrors `sim/keeper.lua`'s `KeeperShotType` alias. */
export type KeeperShotType = "ground" | "chip";

/** Wire-carried closed set. Numbered exactly as `sim/match.lua`'s `MatchEventKind` alias, 1..25. */
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
  | "juke";

/** Who holds the ball this frame. Not part of `pitch.ts`'s `RenderFrame` slice; see this module's report. */
export interface RenderFramePossession {
  /** Roster slot holding the ball, one-based; absent if loose. */
  readonly owner?: number;
  /** The owning slot's team. */
  readonly owner_team?: "home" | "away";
  /** Owner is a keeper carrying it in the hands. */
  readonly keeper_holds: boolean;
}

/** Scoreboard/identity facts a HUD needs. Not part of `pitch.ts`'s `RenderFrame` slice; see this module's report. */
export interface RenderFrameHud {
  readonly home_score: number;
  readonly away_score: number;
  readonly time_left: number;
  readonly finished: boolean;
  /** Absent while the ball is loose. */
  readonly possession_team?: "home" | "away";
  /** Roster slot, one-based. `hud.controlled_id` is dropped from the wire (rule 4) -- recover it as `roster.ids[hud.controlled - 1]`. */
  readonly controlled: number;
  readonly controlled_team: "home" | "away";
  readonly controlled_is_keeper: boolean;
  readonly controlled_owns_ball: boolean;
  /** 0..1 sprint meter, clamped for the meter widget. */
  readonly controlled_stamina: number;
  readonly species_shape: SpeciesShape;
  readonly species_color: RGB;
}

/**
 * This frame's discrete match events, flattened, structure-of-arrays. Not
 * part of `pitch.ts`'s `RenderFrame` slice; see this module's report.
 *
 * `jumping`/`on_target` are TRI-STATE, not sparse booleans: `undefined` ("this
 * event kind does not report it"), `false`, and `true` are three distinct
 * real states -- mirrors `crates/gc-render/src/frame.rs`'s `RenderFrameEvents`
 * doc comment exactly.
 */
export interface RenderFrameEvents {
  readonly count: number;
  readonly kind: readonly MatchEventKind[];
  readonly x: readonly number[];
  readonly y: readonly number[];
  /** The actor's roster slot, one-based; absent if unattributed. `player` (a roster id) is dropped from the wire -- recover it via the roster. */
  readonly slot: readonly (number | undefined)[];
  readonly save_style: readonly (SaveStyle | undefined)[];
  readonly style: readonly (AerialStyle | undefined)[];
  readonly outcome: readonly (AerialOutcome | undefined)[];
  readonly difficulty: readonly (number | undefined)[];
  readonly shot_type: readonly (KeeperShotType | undefined)[];
  readonly keeper_state: readonly (KeeperBehaviorState | undefined)[];
  readonly keeper_depth: readonly (number | undefined)[];
  readonly jumping: readonly (boolean | undefined)[];
  readonly on_target: readonly (boolean | undefined)[];
}

/** `RenderFrameBall` (`pitch.ts`) plus `vx`/`vy`/`vz`, which the wire carries but that slice does not declare. */
export interface DecodedRenderFrameBall extends RenderFrameBall {
  readonly vx: number;
  readonly vy: number;
  readonly vz: number;
}

/** `RenderFramePlayers` (`pitch.ts`) plus `speed`, which the wire carries but that slice does not declare. */
export interface DecodedRenderFramePlayers extends RenderFramePlayers {
  readonly speed: readonly number[];
}

/** `RenderFrameRoster` (`pitch.ts`) plus `names` and `count`, which the wire carries but that slice does not declare. */
export interface DecodedRenderFrameRoster extends RenderFrameRoster {
  readonly layoutVersion: number;
  readonly renderFrameVersion: number;
  readonly count: number;
  readonly names: readonly string[];
}

/**
 * One decoded frame block. Deliberately NOT `RenderFrame`: a per-frame block
 * never carries `roster` (match-constant, crosses once -- see `decodeRoster`)
 * or `combat` (never carried at all -- see this module's header). Combine
 * with a decoded roster via `toRenderFrame` to get the real thing.
 */
export interface DecodedRenderFrame {
  readonly layoutVersion: number;
  readonly renderFrameVersion: number;
  /** Whether the source frame carried a combat telegraph model (never encoded -- see "WHAT IS NOT CARRIED" above). */
  readonly combatPresent: boolean;
  readonly field: RenderFrameField;
  readonly ball: DecodedRenderFrameBall;
  readonly possession: RenderFramePossession;
  readonly control: RenderFrameControl;
  readonly hud: RenderFrameHud;
  readonly players: DecodedRenderFramePlayers;
  readonly events: RenderFrameEvents;
}

// ---------------------------------------------------------------------------
// Enum code tables (decode direction only -- this module never encodes).
// Numbered exactly as `render/frame_buffer.lua`'s tables / `crates/gc-render
// /src/frame_buffer.rs`'s `*_from_code` functions.
// ---------------------------------------------------------------------------

function teamFromCode(code: number): "home" | "away" | undefined {
  switch (code) {
    case 1:
      return "home";
    case 2:
      return "away";
    default:
      return undefined;
  }
}

function speciesShapeFromCode(code: number): SpeciesShape | undefined {
  switch (code) {
    case 1:
      return "round";
    case 2:
      return "broad";
    case 3:
      return "angular";
    case 4:
      return "cluster";
    default:
      return undefined;
  }
}

function chargeKindFromCode(code: number): "shot" | "pass" | undefined {
  switch (code) {
    case 1:
      return "shot";
    case 2:
      return "pass";
    default:
      return undefined;
  }
}

function poseSourceFromCode(code: number): PlayerPoseSource | undefined {
  switch (code) {
    case 1:
      return "soccer";
    case 2:
      return "combat";
    case 3:
      return "locomotion";
    default:
      return undefined;
  }
}

function aerialStyleFromCode(code: number): AerialStyle | undefined {
  switch (code) {
    case 1:
      return "leg_control";
    case 2:
      return "chest_control";
    case 3:
      return "volley";
    case 4:
      return "header";
    case 5:
      return "bicycle";
    default:
      return undefined;
  }
}

function aerialOutcomeFromCode(code: number): AerialOutcome | undefined {
  switch (code) {
    case 1:
      return "clean";
    case 2:
      return "heavy";
    case 3:
      return "miss";
    default:
      return undefined;
  }
}

function saveStyleFromCode(code: number): SaveStyle | undefined {
  switch (code) {
    case 1:
      return "spread";
    case 2:
      return "central";
    case 3:
      return "stretch";
    default:
      return undefined;
  }
}

function keeperStateFromCode(code: number): KeeperBehaviorState | undefined {
  switch (code) {
    case 1:
      return "base";
    case 2:
      return "advance";
    case 3:
      return "contain";
    case 4:
      return "set";
    case 5:
      return "retreat";
    case 6:
      return "recover";
    default:
      return undefined;
  }
}

function shotTypeFromCode(code: number): KeeperShotType | undefined {
  switch (code) {
    case 1:
      return "ground";
    case 2:
      return "chip";
    default:
      return undefined;
  }
}

function poseIdFromCode(code: number): PlayerPoseId | undefined {
  switch (code) {
    case 1:
      return "keeper_grab";
    case 2:
      return "keeper_throw";
    case 3:
      return "keeper_punt";
    case 4:
      return "keeper_tip";
    case 5:
      return "keeper_spread";
    case 6:
      return "keeper_central";
    case 7:
      return "keeper_stretch";
    case 8:
      return "keeper_dive";
    case 9:
      return "keeper_get_up";
    case 10:
      return "keeper_set";
    case 11:
      return "keeper_ready_low";
    case 12:
      return "keeper_shuffle";
    case 13:
      return "keeper_ready_tall";
    case 14:
      return "aerial_bicycle";
    case 15:
      return "aerial_action";
    case 16:
      return "combat_knockback";
    case 17:
      return "combat_stagger";
    case 18:
      return "combat_guard";
    case 19:
      return "combat_active";
    case 20:
      return "combat_windup";
    case 21:
      return "combat_aim";
    case 22:
      return "combat_recovery";
    case 23:
      return "soccer_windup";
    case 24:
      return "slide";
    case 25:
      return "tackle";
    case 26:
      return "stumble";
    case 27:
      return "kick_follow";
    case 28:
      return "settle";
    case 29:
      return "run_telegraph";
    case 30:
      return "contain";
    case 31:
      return "fatigue";
    case 32:
      return "locomotion";
    default:
      return undefined;
  }
}

function eventKindFromCode(code: number): MatchEventKind | undefined {
  switch (code) {
    case 1:
      return "shot";
    case 2:
      return "pass";
    case 3:
      return "touch";
    case 4:
      return "tackle";
    case 5:
      return "press_commit_heavy_touch";
    case 6:
      return "press_commit_exposed_ball";
    case 7:
      return "press_commit_cover";
    case 8:
      return "press_commit_box_desperation";
    case 9:
      return "press_commit_low_discipline";
    case 10:
      return "combat_commit_carrier_contest";
    case 11:
      return "combat_commit_carrier_protection";
    case 12:
      return "combat_commit_loose_ball_contest";
    case 13:
      return "combat_commit_passing_lane_or_shot_denial";
    case 14:
      return "combat_commit_recovery_punish";
    case 15:
      return "combat_commit_unattributed_off_ball";
    case 16:
      return "catch";
    case 17:
      return "parry";
    case 18:
      return "tip";
    case 19:
      return "claim";
    case 20:
      return "block";
    case 21:
      return "header";
    case 22:
      return "volley";
    case 23:
      return "bicycle";
    case 24:
      return "reception";
    case 25:
      return "juke";
    default:
      return undefined;
  }
}

// ---------------------------------------------------------------------------
// Low-level word access. `noUncheckedIndexedAccess` makes every array read
// `T | undefined`; `at` is the one place that gets asserted away, so an
// out-of-range read fails loudly instead of silently becoming `NaN` deeper
// in a computation.
// ---------------------------------------------------------------------------

function at(words: ArrayLike<number>, index: number): number {
  const value = words[index];
  if (value === undefined) {
    throw new Error(`frame_buffer: word ${index} out of range (block has ${words.length} words)`);
  }
  return value;
}

function decodeBool(word: number): boolean {
  if (word === 1) {
    return true;
  }
  if (word === 0) {
    return false;
  }
  throw new Error(`frame_buffer: not a boolean word: ${word}`);
}

// 0 = not reported, 1 = false, 2 = true. Mirrors `render/frame.lua`'s
// `tri_state`/its Rust port's inverse.
function triState(word: number): boolean | undefined {
  if (word === 0) {
    return undefined;
  }
  if (word === 1) {
    return false;
  }
  if (word === 2) {
    return true;
  }
  throw new Error(`frame_buffer: not a tri-state word: ${word}`);
}

// A closed-set code becomes its member; 0 becomes absent. An unrecognised
// nonzero code is a protocol violation (the alias grew and this numbering
// did not), not a value to recover from.
function optDecode<T>(code: number, fromCode: (code: number) => T | undefined, what: string): T | undefined {
  if (code === 0) {
    return undefined;
  }
  const value = fromCode(code);
  if (value === undefined) {
    throw new Error(`frame_buffer: unknown ${what} code ${code}`);
  }
  return value;
}

// Same as `optDecode`, but for a field the producer never leaves absent
// (`hud.controlled_team`, a roster slot's `team`, ...): a 0 there is still a
// protocol violation, just one `optDecode` alone would swallow as "absent".
function requireDecode<T>(code: number, fromCode: (code: number) => T | undefined, what: string): T {
  const value = optDecode(code, fromCode, what);
  if (value === undefined) {
    throw new Error(`frame_buffer: missing required ${what} (code 0)`);
  }
  return value;
}

function soaIndex(fieldIndex: number, slotIndex: number, count: number): number {
  return fieldIndex * count + slotIndex;
}

// Field-major structure-of-arrays column read: `at0` is the first word of
// the section, `fieldIndex` selects which run of `count` words.
function column(words: ArrayLike<number>, at0: number, fieldIndex: number, count: number): number[] {
  const start = at0 + soaIndex(fieldIndex, 0, count);
  const out: number[] = [];
  for (let i = 0; i < count; i += 1) {
    out.push(at(words, start + i));
  }
  return out;
}

// ---------------------------------------------------------------------------
// decode / decodeRoster
// ---------------------------------------------------------------------------

/**
 * Read one frame block back. This is the TypeScript mirror of
 * `crates/gc-render/src/frame_buffer.rs`'s `decode` / `render/frame_buffer
 * .lua`'s `frame_buffer.decode`, with every enum resolved inline (see this
 * module's header for why).
 *
 * `words` is `ArrayLike<number>` rather than `readonly number[]` so a caller
 * holding a zero-copy `Float64Array` VIEW over wasm linear memory (`@gc/app`'s
 * `sim_host.ts`, over `@gc/wasm`'s raw per-frame export) can pass it straight
 * through -- both `number[]` and `Float64Array` satisfy it, and converting a
 * `Float64Array` to a plain array would copy every frame, defeating the
 * zero-copy design that raw export exists for.
 *
 * @throws If the magic word, layout version or render-frame version do not
 *   match, if a header field count disagrees with this module's constants,
 *   if `total_words` disagrees with the header's own counts, or if any enum
 *   word carries an unrecognised nonzero code. Each is a corrupted or
 *   foreign block, not a value this reader is meant to recover from.
 */
export function decode(words: ArrayLike<number>): DecodedRenderFrame {
  if (at(words, 0) !== MAGIC) {
    throw new Error(`frame_buffer: not a render frame block: magic ${at(words, 0)}`);
  }
  if (at(words, 1) !== LAYOUT_VERSION) {
    throw new Error(`frame_buffer: frame layout version ${at(words, 1)}; expected ${LAYOUT_VERSION}`);
  }
  if (at(words, 2) !== RENDER_FRAME_VERSION) {
    throw new Error(`frame_buffer: render frame version ${at(words, 2)}; expected ${RENDER_FRAME_VERSION}`);
  }
  if (at(words, 4) !== HEADER_WORDS) {
    throw new Error(`frame_buffer: header words ${at(words, 4)}; expected ${HEADER_WORDS}`);
  }
  if (at(words, 5) !== SCALAR_FIELD_COUNT) {
    throw new Error(`frame_buffer: scalar words ${at(words, 5)}; expected ${SCALAR_FIELD_COUNT}`);
  }
  if (at(words, 7) !== PLAYER_FIELD_COUNT) {
    throw new Error(`frame_buffer: player field count ${at(words, 7)}; expected ${PLAYER_FIELD_COUNT}`);
  }
  if (at(words, 9) !== EVENT_FIELD_COUNT) {
    throw new Error(`frame_buffer: event field count ${at(words, 9)}; expected ${EVENT_FIELD_COUNT}`);
  }

  const count = at(words, 6);
  const eventCount = at(words, 8);
  const total = frameWords(count, eventCount);
  if (at(words, 3) !== total) {
    throw new Error(`frame_buffer: total words ${at(words, 3)}; expected ${total}`);
  }

  const scalarsAt = HEADER_WORDS;
  const s = (index: number): number => at(words, scalarsAt + index);

  const field: RenderFrameField = {
    w: s(0),
    h: s(1),
    crossbar_h: s(2),
    penalty_box_depth: s(3),
    penalty_box_h: s(4),
    goal_home: { x: s(5), y: s(6), w: s(7), h: s(8) },
    goal_away: { x: s(9), y: s(10), w: s(11), h: s(12) },
  };

  const landingValid = decodeBool(s(20));
  const ball: DecodedRenderFrameBall = {
    x: s(13),
    y: s(14),
    z: s(15),
    vx: s(16),
    vy: s(17),
    vz: s(18),
    visible: decodeBool(s(19)),
    ...(landingValid ? { landing_x: s(21), landing_y: s(22) } : {}),
  };

  const possessionOwner = s(23);
  const possessionOwnerTeam = optDecode(s(24), teamFromCode, "team");
  const possession: RenderFramePossession = {
    ...(possessionOwner !== 0 ? { owner: possessionOwner } : {}),
    ...(possessionOwnerTeam !== undefined ? { owner_team: possessionOwnerTeam } : {}),
    keeper_holds: decodeBool(s(25)),
  };

  const passTarget = s(27);
  const chargeKind = optDecode(s(28), chargeKindFromCode, "charge kind");
  const control: RenderFrameControl = {
    controlled: s(26),
    ...(passTarget !== 0 ? { pass_target: passTarget } : {}),
    ...(chargeKind !== undefined ? { charge_kind: chargeKind } : {}),
    charge: s(29),
  };

  const possessionTeam = optDecode(s(34), teamFromCode, "team");
  const hud: RenderFrameHud = {
    home_score: s(30),
    away_score: s(31),
    time_left: s(32),
    finished: decodeBool(s(33)),
    ...(possessionTeam !== undefined ? { possession_team: possessionTeam } : {}),
    controlled: s(35),
    controlled_team: requireDecode(s(36), teamFromCode, "team"),
    controlled_is_keeper: decodeBool(s(37)),
    controlled_owns_ball: decodeBool(s(38)),
    controlled_stamina: s(39),
    species_shape: requireDecode(s(40), speciesShapeFromCode, "species shape"),
    species_color: [s(41), s(42), s(43)],
  };

  const playersAt = scalarsAt + SCALAR_FIELD_COUNT;
  const rawControlled = column(words, playersAt, 8, count);
  const rawDashing = column(words, playersAt, 9, count);
  const rawHolding = column(words, playersAt, 10, count);
  const players: DecodedRenderFramePlayers = {
    count,
    x: column(words, playersAt, 0, count),
    y: column(words, playersAt, 1, count),
    facing_x: column(words, playersAt, 2, count),
    facing_y: column(words, playersAt, 3, count),
    speed: column(words, playersAt, 4, count),
    pose_id: column(words, playersAt, 5, count).map((code) => optDecode(code, poseIdFromCode, "pose id")),
    pose_priority: column(words, playersAt, 6, count),
    pose_source: column(words, playersAt, 7, count).map((code) => optDecode(code, poseSourceFromCode, "pose source")),
    controlled: rawControlled.map(decodeBool),
    dashing: rawDashing.map(decodeBool),
    holding: rawHolding.map(decodeBool),
    dive: column(words, playersAt, 11, count),
    dive_dir_x: column(words, playersAt, 12, count),
    dive_dir_y: column(words, playersAt, 13, count),
    grab: column(words, playersAt, 14, count),
    throw: column(words, playersAt, 15, count),
    windup: column(words, playersAt, 16, count),
    aerial: column(words, playersAt, 17, count),
    aerial_jump: column(words, playersAt, 18, count),
    aerial_style: column(words, playersAt, 19, count).map((code) => optDecode(code, aerialStyleFromCode, "aerial style")),
    aerial_outcome: column(words, playersAt, 20, count).map((code) => optDecode(code, aerialOutcomeFromCode, "aerial outcome")),
  };

  const eventsAt = playersAt + PLAYER_FIELD_COUNT * count;
  const rawSlot = column(words, eventsAt, 3, eventCount);
  const rawHasDifficulty = column(words, eventsAt, 7, eventCount);
  const rawDifficulty = column(words, eventsAt, 8, eventCount);
  const rawHasKeeperDepth = column(words, eventsAt, 11, eventCount);
  const rawKeeperDepth = column(words, eventsAt, 12, eventCount);
  const events: RenderFrameEvents = {
    count: eventCount,
    kind: column(words, eventsAt, 0, eventCount).map((code) => requireDecode(code, eventKindFromCode, "event kind")),
    x: column(words, eventsAt, 1, eventCount),
    y: column(words, eventsAt, 2, eventCount),
    slot: rawSlot.map((raw) => (raw === 0 ? undefined : raw)),
    save_style: column(words, eventsAt, 4, eventCount).map((code) => optDecode(code, saveStyleFromCode, "save style")),
    style: column(words, eventsAt, 5, eventCount).map((code) => optDecode(code, aerialStyleFromCode, "aerial style")),
    outcome: column(words, eventsAt, 6, eventCount).map((code) => optDecode(code, aerialOutcomeFromCode, "aerial outcome")),
    difficulty: rawHasDifficulty.map((hasIt, index) => (decodeBool(hasIt) ? rawDifficulty[index] : undefined)),
    shot_type: column(words, eventsAt, 9, eventCount).map((code) => optDecode(code, shotTypeFromCode, "shot type")),
    keeper_state: column(words, eventsAt, 10, eventCount).map((code) => optDecode(code, keeperStateFromCode, "keeper state")),
    keeper_depth: rawHasKeeperDepth.map((hasIt, index) => (decodeBool(hasIt) ? rawKeeperDepth[index] : undefined)),
    jumping: column(words, eventsAt, 13, eventCount).map(triState),
    on_target: column(words, eventsAt, 14, eventCount).map(triState),
  };

  return {
    layoutVersion: at(words, 1),
    renderFrameVersion: at(words, 2),
    combatPresent: decodeBool(at(words, 10)),
    field,
    ball,
    possession,
    control,
    hud,
    players,
    events,
  };
}

/**
 * Read a roster block back, pairing it with the string blob
 * `crates/gc-render`'s `encode_roster` returned alongside it. TypeScript
 * mirror of that module's `decode_roster` / `render/frame_buffer.lua`'s
 * `frame_buffer.decode_roster`.
 *
 * `words` is `ArrayLike<number>` -- see `decode`'s doc for why (the same
 * zero-copy `Float64Array`-view rationale applies to a roster block).
 *
 * @throws On a bad magic word, a layout/version mismatch, a field count that
 *   disagrees with `ROSTER_FIELD_COUNT`, an unrecognised enum code, or a
 *   string blob that does not hold exactly `2 * count` newline-delimited
 *   parts.
 */
export function decodeRoster(words: ArrayLike<number>, strings: string): DecodedRenderFrameRoster {
  if (at(words, 0) !== ROSTER_MAGIC) {
    throw new Error(`frame_buffer: not a render roster block: magic ${at(words, 0)}`);
  }
  if (at(words, 1) !== LAYOUT_VERSION) {
    throw new Error(`frame_buffer: roster layout version ${at(words, 1)}; expected ${LAYOUT_VERSION}`);
  }
  if (at(words, 2) !== RENDER_FRAME_VERSION) {
    throw new Error(`frame_buffer: render frame version ${at(words, 2)}; expected ${RENDER_FRAME_VERSION}`);
  }
  if (at(words, 6) !== ROSTER_FIELD_COUNT) {
    throw new Error(`frame_buffer: roster field count ${at(words, 6)}; expected ${ROSTER_FIELD_COUNT}`);
  }

  const count = at(words, 5);

  // `strings` is `id\nname\nid\nname\n...\nid\nname` (no trailing newline).
  // Appending one and splitting on it, then dropping the trailing empty
  // element `split` always yields past the last delimiter, recovers exactly
  // the `count * 2` parts `encode_roster` joined -- matches
  // `crates/gc-render/src/frame_buffer.rs`'s `decode_roster` exactly.
  const blob = `${strings}\n`;
  const parts = blob.split("\n");
  parts.pop();
  if (parts.length !== count * 2) {
    throw new Error(`frame_buffer: roster blob holds ${parts.length} strings; expected ${count * 2}`);
  }

  const ids: string[] = [];
  const names: string[] = [];
  for (let index = 0; index < count; index += 1) {
    ids.push(at2(parts, index * 2));
    names.push(at2(parts, index * 2 + 1));
  }

  const at0 = ROSTER_HEADER_WORDS;
  const rawIsKeeper = column(words, at0, 1, count);
  const r = column(words, at0, 4, count);
  const g = column(words, at0, 5, count);
  const b = column(words, at0, 6, count);
  const speciesColor: RGB[] = [];
  for (let index = 0; index < count; index += 1) {
    speciesColor.push([at(r, index), at(g, index), at(b, index)]);
  }

  return {
    layoutVersion: at(words, 1),
    renderFrameVersion: at(words, 2),
    count,
    ids,
    names,
    teams: column(words, at0, 0, count).map((code) => requireDecode(code, teamFromCode, "team")),
    is_keeper: rawIsKeeper.map(decodeBool),
    radius: column(words, at0, 2, count),
    species_shape: column(words, at0, 3, count).map((code) => requireDecode(code, speciesShapeFromCode, "species shape")),
    species_color: speciesColor,
  };
}

function at2(parts: readonly string[], index: number): string {
  const value = parts[index];
  if (value === undefined) {
    throw new Error(`frame_buffer: roster string ${index} out of range`);
  }
  return value;
}

/**
 * The complete `RenderFrame` the wire carries: `pitch.ts`'s drawing slice
 * (`PitchRenderFrame` -- `field`/`roster`/`players`/`ball`/`control`/
 * `combat`) plus `hud`/`possession`, which drawing does not need but other
 * consumers do -- `@gc/app`'s `sim_host.ts` decides whether a match is over
 * (`hud.finished`) and `@gc/screens`' `MatchScreen` decides whether the
 * controlled player is carrying (`hud.controlled_owns_ball`) before either
 * can compute a single contextual input field or a HUD frame; neither field
 * existed on `pitchTypes.RenderFrame`, which is sized for DRAWING only. This
 * is the one canonical frame type both packages import instead of each
 * hand-declaring their own (previously-drifting) slice -- see this
 * decoder's header and README rule 6.7. `pitch.draw` keeps accepting this
 * type via ordinary structural typing: it only reads the `PitchRenderFrame`
 * fields it already declared, and this type is a strict superset of those.
 */
export interface RenderFrame extends PitchRenderFrame {
  readonly possession: RenderFramePossession;
  readonly hud: RenderFrameHud;
  /**
   * This frame's discrete match events.
   *
   * CARRIED, after having been silently dropped. `decode` has always
   * recovered these off the wire, but `toRenderFrame` used to leave them
   * behind -- so every consumer downstream of it (both `SimHostPort`
   * implementations in `@gc/app`, and therefore `@gc/screens`'s
   * `MatchScreen`) received a frame with no `events` field at all. That is
   * what made `MatchScreen.appendObservedFrameEvents` an unconditional
   * early return in production, which in turn left BOTH consumers of the
   * per-tick event batch inert: `MatchScreenAsRealMatchScreen.frameEvents`
   * (`game.match_observer`'s attribution) and the release follow-through
   * window. Neither is reachable in a live match without this field, no
   * matter what either of them does with it.
   *
   * `pitch.draw` neither declares nor reads it -- this type stays a strict
   * superset of `PitchRenderFrame`, and drawing is unaffected.
   */
  readonly events: RenderFrameEvents;
}

/**
 * Combine a decoded frame with its match-constant decoded roster (and, when
 * the app has one, an externally-sourced combat presentation model -- see
 * this module's header on why combat never travels on the wire) into the
 * real, complete `RenderFrame` above.
 */
export function toRenderFrame(
  frame: DecodedRenderFrame,
  roster: DecodedRenderFrameRoster,
  combat?: CombatPresentationModel,
): RenderFrame {
  return {
    field: frame.field,
    roster,
    players: frame.players,
    ball: frame.ball,
    control: frame.control,
    possession: frame.possession,
    hud: frame.hud,
    events: frame.events,
    ...(combat !== undefined ? { combat } : {}),
  };
}
