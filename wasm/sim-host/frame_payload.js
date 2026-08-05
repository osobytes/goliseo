// The JavaScript side of the render frame payload boundary (#332).
//
// `render/frame_buffer.lua` writes one whole render frame into wasm linear
// memory as a flat block of doubles. This reads it back. Together they are the
// protocol; neither is the protocol on its own, which is why both stamp the
// same two version numbers and both hard-fail on a mismatch.
//
// ZERO COPY, ONE CROSSING. `readFrame` takes a heap view and a pointer that the
// caller already has -- it calls nothing, allocates no backing store, and hands
// back `Float64Array` SUBARRAYS over the module's own memory. A renderer that
// only wants positions touches two of the twenty-one player runs and never
// materialises the rest. The single boundary crossing per frame is the
// `_goliseo_payload_frame` call that produced the pointer; everything here is
// ordinary memory reads on the JavaScript side of it.
//
// The views ALIAS live wasm memory, and that is a deliberate trade rather than
// an oversight. Two consequences a caller has to know:
//
//   * They are invalidated by the next `_goliseo_payload_frame` call, which
//     overwrites the same block. Read what you need within the frame, or copy.
//   * `ALLOW_MEMORY_GROWTH` is on, so growing the heap detaches every existing
//     view. Pass a freshly read `Module.HEAPF64` each frame; the wasm address
//     is stable across growth, the JavaScript view over it is not.
//
// VERSIONING. Two independent numbers, checked separately because they fail for
// different reasons:
//
//   RENDER_FRAME_VERSION  the payload's meaning -- `render_frame.VERSION`.
//                         Bumped when the frame gains or loses a field.
//   LAYOUT_VERSION        this block's shape -- field order, header size, enum
//                         numbering. Bumped when the same information is
//                         arranged differently.
//
// A reader that skipped these would not fail; it would silently decode a
// reordered block into plausible nonsense, which is the whole reason
// `sim/input_frame.lua` and `sim/match_snapshot.lua` hard-assert their versions
// on every read. This does the same, and additionally checks the header's own
// field counts against the field-name lists below -- so a field added on the
// Lua side without a version bump is caught anyway.

"use strict";

const MAGIC = 0x474f4c46; // "GOLF"
const ROSTER_MAGIC = 0x474f4c52; // "GOLR"

const LAYOUT_VERSION = 1;
const RENDER_FRAME_VERSION = 1;

// Header words, in order. Mirrors `frame_buffer.HEADER_FIELDS`.
const HEADER_FIELDS = [
    "magic",
    "layout_version",
    "render_frame_version",
    "total_words",
    "header_words",
    "scalar_words",
    "player_count",
    "player_field_count",
    "event_count",
    "event_field_count",
    "combat_present",
    "reserved",
];

const SCALAR_FIELDS = [
    "field_w",
    "field_h",
    "field_crossbar_h",
    "field_penalty_box_depth",
    "field_penalty_box_h",
    "goal_home_x",
    "goal_home_y",
    "goal_home_w",
    "goal_home_h",
    "goal_away_x",
    "goal_away_y",
    "goal_away_w",
    "goal_away_h",
    "ball_x",
    "ball_y",
    "ball_z",
    "ball_vx",
    "ball_vy",
    "ball_vz",
    "ball_visible",
    "ball_landing_valid",
    "ball_landing_x",
    "ball_landing_y",
    "possession_owner",
    "possession_owner_team",
    "possession_keeper_holds",
    "control_controlled",
    "control_pass_target",
    "control_charge_kind",
    "control_charge",
    "hud_home_score",
    "hud_away_score",
    "hud_time_left",
    "hud_finished",
    "hud_possession_team",
    "hud_controlled",
    "hud_controlled_team",
    "hud_controlled_is_keeper",
    "hud_controlled_owns_ball",
    "hud_controlled_stamina",
    "hud_species_shape",
    "hud_species_color_r",
    "hud_species_color_g",
    "hud_species_color_b",
];

const PLAYER_FIELDS = [
    "x",
    "y",
    "facing_x",
    "facing_y",
    "speed",
    "pose_id",
    "pose_priority",
    "pose_source",
    "controlled",
    "dashing",
    "holding",
    "dive",
    "dive_dir_x",
    "dive_dir_y",
    "grab",
    "throw",
    "windup",
    "aerial",
    "aerial_jump",
    "aerial_style",
    "aerial_outcome",
];

const EVENT_FIELDS = [
    "kind",
    "x",
    "y",
    "slot",
    "save_style",
    "style",
    "outcome",
    "has_difficulty",
    "difficulty",
    "shot_type",
    "keeper_state",
    "has_keeper_depth",
    "keeper_depth",
    "jumping",
    "on_target",
];

const ROSTER_HEADER_FIELDS = [
    "magic",
    "layout_version",
    "render_frame_version",
    "total_words",
    "header_words",
    "count",
    "field_count",
];

const ROSTER_FIELDS = [
    "team",
    "is_keeper",
    "radius",
    "species_shape",
    "species_color_r",
    "species_color_g",
    "species_color_b",
];

// Enum numberings, as decode tables. Index 0 is always absent, so a sparse
// hole reads back as `null` with no separate presence bit -- see rule 1 in
// `render/frame_buffer.lua`. Kept as arrays because the codes are dense from 1.
const ABSENT = null;
const TEAM = [ABSENT, "home", "away"];
const SPECIES_SHAPE = [ABSENT, "round", "broad", "angular", "cluster"];
const CHARGE_KIND = [ABSENT, "shot", "pass"];
const POSE_SOURCE = [ABSENT, "soccer", "combat", "locomotion"];
const AERIAL_STYLE = [ABSENT, "leg_control", "chest_control", "volley", "header", "bicycle"];
const AERIAL_OUTCOME = [ABSENT, "clean", "heavy", "miss"];
const SAVE_STYLE = [ABSENT, "spread", "central", "stretch"];
const KEEPER_STATE = [ABSENT, "base", "advance", "contain", "set", "retreat", "recover"];
const SHOT_TYPE = [ABSENT, "ground", "chip"];
const POSE_ID = [
    ABSENT,
    "keeper_grab",
    "keeper_throw",
    "keeper_punt",
    "keeper_tip",
    "keeper_spread",
    "keeper_central",
    "keeper_stretch",
    "keeper_dive",
    "keeper_get_up",
    "keeper_set",
    "keeper_ready_low",
    "keeper_shuffle",
    "keeper_ready_tall",
    "aerial_bicycle",
    "aerial_action",
    "combat_knockback",
    "combat_stagger",
    "combat_guard",
    "combat_active",
    "combat_windup",
    "combat_aim",
    "combat_recovery",
    "soccer_windup",
    "slide",
    "tackle",
    "stumble",
    "kick_follow",
    "settle",
    "run_telegraph",
    "contain",
    "fatigue",
    "locomotion",
];
const EVENT_KIND = [
    ABSENT,
    "shot",
    "pass",
    "touch",
    "tackle",
    "press_commit_heavy_touch",
    "press_commit_exposed_ball",
    "press_commit_cover",
    "press_commit_box_desperation",
    "press_commit_low_discipline",
    "combat_commit_carrier_contest",
    "combat_commit_carrier_protection",
    "combat_commit_loose_ball_contest",
    "combat_commit_passing_lane_or_shot_denial",
    "combat_commit_recovery_punish",
    "combat_commit_unattributed_off_ball",
    "catch",
    "parry",
    "tip",
    "claim",
    "block",
    "header",
    "volley",
    "bicycle",
    "reception",
    "juke",
];

// A payload mismatch is never recoverable: the bytes mean something other than
// what this file believes. Throwing is the point -- the alternative is drawing
// a frame assembled out of misread fields.
class PayloadVersionError extends Error {
    constructor(message) {
        super(message);
        this.name = "PayloadVersionError";
    }
}

function expect(actual, wanted, what) {
    if (actual !== wanted) {
        throw new PayloadVersionError(`${what}: got ${actual}, expected ${wanted}`);
    }
}

// Read one frame block. `heap` is `Module.HEAPF64`, `pointer` is the byte
// offset returned by `_goliseo_payload_frame`.
//
// Returns per-field `Float64Array` subarrays, not copies. See the aliasing note
// at the top of this file.
function readFrame(heap, pointer) {
    if (!pointer) {
        throw new Error("null frame pointer; check _goliseo_payload_error()");
    }
    if (pointer % 8 !== 0) {
        throw new Error(`frame pointer ${pointer} is not 8-byte aligned`);
    }
    const base = pointer / 8;

    expect(heap[base + 0], MAGIC, "frame magic");
    expect(heap[base + 1], LAYOUT_VERSION, "frame layout version");
    expect(heap[base + 2], RENDER_FRAME_VERSION, "render frame version");
    expect(heap[base + 4], HEADER_FIELDS.length, "header words");
    expect(heap[base + 5], SCALAR_FIELDS.length, "scalar words");
    expect(heap[base + 7], PLAYER_FIELDS.length, "player field count");
    expect(heap[base + 9], EVENT_FIELDS.length, "event field count");

    const totalWords = heap[base + 3];
    const playerCount = heap[base + 6];
    const eventCount = heap[base + 8];
    const expectedWords =
        HEADER_FIELDS.length +
        SCALAR_FIELDS.length +
        PLAYER_FIELDS.length * playerCount +
        EVENT_FIELDS.length * eventCount;
    expect(totalWords, expectedWords, "total words");

    const scalarsAt = base + HEADER_FIELDS.length;
    const scalars = {};
    for (let index = 0; index < SCALAR_FIELDS.length; index += 1) {
        scalars[SCALAR_FIELDS[index]] = heap[scalarsAt + index];
    }

    const playersAt = scalarsAt + SCALAR_FIELDS.length;
    const eventsAt = playersAt + PLAYER_FIELDS.length * playerCount;
    return {
        layoutVersion: LAYOUT_VERSION,
        renderFrameVersion: RENDER_FRAME_VERSION,
        totalWords,
        // The frame HAD combat telegraphs, which this layout does not carry --
        // see `render/frame_buffer.lua`. Surfaced rather than dropped so a
        // renderer can refuse to draw a half-frame instead of drawing one.
        combatPresent: heap[base + 10] === 1,
        playerCount,
        eventCount,
        scalars,
        players: soa(heap, playersAt, PLAYER_FIELDS, playerCount),
        events: soa(heap, eventsAt, EVENT_FIELDS, eventCount),
    };
}

// Read the match-constant roster. Crosses once per match, so the id and name
// strings are affordable here and are carried nowhere else.
function readRoster(heap, pointer, strings) {
    if (!pointer) {
        throw new Error("null roster pointer; check _goliseo_payload_error()");
    }
    const base = pointer / 8;
    expect(heap[base + 0], ROSTER_MAGIC, "roster magic");
    expect(heap[base + 1], LAYOUT_VERSION, "roster layout version");
    expect(heap[base + 2], RENDER_FRAME_VERSION, "render frame version");
    expect(heap[base + 4], ROSTER_HEADER_FIELDS.length, "roster header words");
    expect(heap[base + 6], ROSTER_FIELDS.length, "roster field count");

    const count = heap[base + 5];
    const parts = strings.length === 0 ? [] : strings.split("\n");
    expect(parts.length, count * 2, "roster string count");

    const ids = [];
    const names = [];
    for (let index = 0; index < count; index += 1) {
        ids.push(parts[index * 2]);
        names.push(parts[index * 2 + 1]);
    }
    return {
        layoutVersion: LAYOUT_VERSION,
        renderFrameVersion: RENDER_FRAME_VERSION,
        count,
        ids,
        names,
        fields: soa(heap, base + ROSTER_HEADER_FIELDS.length, ROSTER_FIELDS, count),
    };
}

// One subarray per field over the module's memory. No allocation beyond the
// view objects themselves, and no element is touched here.
function soa(heap, at, fields, count) {
    const out = {};
    let cursor = at;
    for (const name of fields) {
        out[name] = heap.subarray(cursor, cursor + count);
        cursor += count;
    }
    return out;
}

const API = {
    MAGIC,
    ROSTER_MAGIC,
    LAYOUT_VERSION,
    RENDER_FRAME_VERSION,
    HEADER_FIELDS,
    SCALAR_FIELDS,
    PLAYER_FIELDS,
    EVENT_FIELDS,
    ROSTER_HEADER_FIELDS,
    ROSTER_FIELDS,
    TEAM,
    SPECIES_SHAPE,
    CHARGE_KIND,
    POSE_SOURCE,
    POSE_ID,
    AERIAL_STYLE,
    AERIAL_OUTCOME,
    SAVE_STYLE,
    KEEPER_STATE,
    SHOT_TYPE,
    EVENT_KIND,
    PayloadVersionError,
    readFrame,
    readRoster,
};

// Loadable three ways without a bundler, because it is used from all three:
// `require` under node, `importScripts` in a worker, and a plain `<script>`.
if (typeof module !== "undefined" && module.exports) {
    module.exports = API;
}
if (typeof globalThis !== "undefined") {
    globalThis.GoliseoFramePayload = API;
}
