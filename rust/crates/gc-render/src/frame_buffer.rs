//! The [`RenderFrame`] payload as a flat block a foreign runtime can read
//! (#332).
//!
//! [`crate::frame`] produces the payload; this module is what makes it
//! CROSSABLE. A `RenderFrame` is Rust structs and heap strings, and neither
//! exists on the far side of a wasm boundary. Handing it to JavaScript field
//! by field would be one boundary crossing per field per entity per frame —
//! exactly the shape [`crate::frame`] was designed to avoid, undone at the
//! last step. So the payload is serialised once into one contiguous array of
//! doubles, copied into linear memory in one go, and read on the far side as
//! a single typed-array view. One crossing per rendered frame, in batch.
//!
//! Doubles for EVERYTHING, including booleans, enums and counts. A
//! mixed-width layout would save bytes and cost the property that makes this
//! cheap: a single `Float64Array` over the whole block with no per-section
//! re-slicing, no alignment arithmetic, and no endian question.
//!
//! Four encoding rules, each of them forced by something in [`crate::frame`]:
//!
//! 1. ENUMS BECOME SMALL INTEGERS, AND 0 ALWAYS MEANS ABSENT. Every closed
//!    set ([`PlayerPoseId`], `MatchEventKind`, ...) has a numbering here (the
//!    `*_code`/`*_from_code` function pairs below). Slot 0 is never a real
//!    member, so a sparse field's `None` encodes as 0 and decodes back to
//!    `None` with no separate presence bit.
//!
//! 2. SPARSE *NUMBERS* GET AN EXPLICIT PRESENCE FLAG. `difficulty` and
//!    `keeper_depth` are sparse and numeric, so there is no unused value to
//!    reserve — 0 is a legitimate difficulty. [`crate::frame`] already
//!    refused to let absent and false collapse together for booleans; the
//!    same reasoning applies here, so `has_difficulty`/`has_keeper_depth`
//!    ride alongside. `ball_landing_x`/`ball_landing_y` get
//!    `ball_landing_valid` for the same reason.
//!
//! 3. ONE-BASED SLOTS MAKE 0 A FREE SENTINEL. `possession_owner`,
//!    `control_pass_target` and an event's `slot` are roster indices (the
//!    ARCHITECTURE.md §3 rule 3 wire-identity exception — they keep the 1-based value
//!    `render::frame` already carries), so 0 encodes "none" without a flag.
//!
//! 4. STRINGS DO NOT CROSS PER FRAME. The only per-frame strings in the
//!    payload are `hud.controlled_id` and an event's `player`, and both are
//!    roster lookups of an index that is already in the block
//!    (`hud_controlled`, the event's `slot`). They are dropped from the
//!    frame block and recovered from the roster, which crosses once.
//!    [`RenderFrameRoster`]'s own string columns — `ids`, `names`,
//!    `presentation_ids` and `loadout_ids` — cross once too, as a
//!    newline-joined blob beside its numeric block. See
//!    [`ROSTER_STRING_FIELD_COUNT`].
//!
//! WHAT IS NOT CARRIED: [`RenderFrame::combat`]. It is the one disclosed
//! nested exception in [`crate::frame`] — a combat telegraph model with
//! `Vec2` positions — and flattening it is that flattening's job, not this
//! one's. Rather than drop it silently, the header carries `combat_present`
//! (header word index 10, 0-based), so a reader can tell "this frame had no
//! combat telegraphs" from "this layout cannot carry them yet" and fail
//! loudly instead of drawing nothing.
//!
//! VERSIONING. Two numbers are stamped and both are checked, because they
//! can change independently:
//!
//!   - `RENDER_FRAME_VERSION` — the payload's meaning, [`crate::frame::VERSION`].
//!   - [`LAYOUT_VERSION`] — this block's shape: field order, header size.
//!
//! A field reordered here changes the second and not the first; a field
//! added to [`RenderFrame`] changes the first. [`decode`] hard-asserts both,
//! and [`encode`] hard-asserts the frame it was handed carries the version
//! it claims. The non-Rust reader (`wasm/sim-host/frame_payload.js`) is
//! meant to check the same two words against its own constants, so a Rust
//! producer and a JS reader cannot silently disagree.
//!
//! Every failure mode in this module — a version mismatch, a corrupted
//! header, an unmapped enum, a roster string with an embedded newline — is
//! `assert!` (never `return nil, err`): all of them are protocol violations
//! (the producer and reader disagreeing about the wire's shape), not
//! expected, recoverable input to handle gracefully (AGENTS.md §7).

use gc_data::species::Shape;
use gc_sim::aerial::{AerialOutcome, AerialStyle};
use gc_sim::keeper::{KeeperBehaviorState, KeeperShotType, SaveStyle};
use gc_sim::match_snapshot::{MatchEventKind, Team};

use crate::frame::{
    self, RenderChargeKind, RenderFrame, RenderFrameEvents, RenderFramePlayers, RenderFrameRoster,
};
use crate::player_pose::{PlayerPoseId, PlayerPoseSource};

/// Shape of this block. Bump on any change to field order, header size or
/// the enum numberings below — all of them are read positionally.
pub const LAYOUT_VERSION: u32 = 1;

/// Ordinary sentinel for a frame block, chosen so a misaligned or
/// wrong-buffer read fails on the first word instead of decoding garbage.
/// "GOLF" as big-endian ASCII.
pub const MAGIC: u32 = 0x474F_4C46;

/// Ordinary sentinel for a roster block. "GOLR" as big-endian ASCII.
pub const ROSTER_MAGIC: u32 = 0x474F_4C52;

/// Header word count. `total_words` (word index 3) lets a reader size its
/// view from the header alone, which is what keeps a frame read at one
/// crossing.
pub const HEADER_WORDS: usize = 12;

/// The once-per-frame scalar word count: field geometry, ball, possession,
/// control, HUD.
pub const SCALAR_FIELD_COUNT: usize = 44;

/// Per-player field count. Field-major: all of `x`, then all of `y`, and so
/// on — the structure-of-arrays promise made concrete.
pub const PLAYER_FIELD_COUNT: usize = 21;

/// Per-event field count, field-major. `player` (a roster id string) is
/// deliberately absent: `slot` indexes the roster, which already crossed.
pub const EVENT_FIELD_COUNT: usize = 15;

/// Roster block header word count.
pub const ROSTER_HEADER_WORDS: usize = 7;

/// Roster block per-slot field count. Its string fields travel as a
/// separate newline-joined blob, not in this numeric block — see
/// [`ROSTER_STRING_FIELD_COUNT`].
pub const ROSTER_FIELD_COUNT: usize = 7;

/// Per-slot string count in the roster's newline-joined blob, in the order
/// [`encode_roster`] writes them: `id`, `name`, `presentation_id`,
/// `loadout_id`.
///
/// SEPARATE FROM [`ROSTER_FIELD_COUNT`], AND SEPARATELY VERSIONED IN
/// PRACTICE. `ROSTER_FIELD_COUNT` is stamped into the numeric block (word
/// index 6) and hard-asserted by [`decode_roster`]; this count is not
/// stamped anywhere, because the blob has no header of its own — its length
/// is checked against `count * ROSTER_STRING_FIELD_COUNT` instead, which is
/// the same protection by a different route (a producer and reader that
/// disagree fail loudly on the part count, they do not mis-parse).
///
/// WHY THE NEW #447 FIELDS WENT HERE AND NOT INTO THE NUMERIC BLOCK.
/// `presentation_id` and `loadout_id` are strings; the numeric block holds
/// doubles only, so carrying them there would mean inventing a second
/// enum numbering for content ids that `gc-data` already keys by string.
/// It would also break the one differential test that pins this wire
/// against a frozen reference (`tests/frame_buffer_differential.rs`), which
/// compares the numeric block word for word against a captured fixture and
/// whose first assertion is a word-count equality. The blob is discarded by
/// that test (`let (roster_words, _digest) = ...`) by explicit, documented
/// design, so growing it changes nothing the fixture pins.
///
/// [`LAYOUT_VERSION`] IS DELIBERATELY NOT BUMPED FOR THIS. It is stamped as
/// word index 1 of both blocks and is therefore itself part of what that
/// fixture compares, so bumping it would fail the differential — and the
/// thing `LAYOUT_VERSION` guards, the numeric layout, is byte-for-byte
/// unchanged here. See this module's VERSIONING note.
pub const ROSTER_STRING_FIELD_COUNT: usize = 4;

fn bw(value: bool) -> f64 {
    if value { 1.0 } else { 0.0 }
}

fn opt_code<T: Copy>(value: Option<T>, code: impl FnOnce(T) -> f64) -> f64 {
    value.map_or(0.0, code)
}

/// Decode a possibly-zero wire word back through `from`, panicking if it is
/// a nonzero code `from` does not recognise — an unmapped code means the
/// alias grew and this numbering did not, a protocol bug, not recoverable
/// input.
fn opt_decode<T>(code: f64, from: impl FnOnce(f64) -> Option<T>, what: &str) -> Option<T> {
    if code == 0.0 {
        return None;
    }
    match from(code) {
        Some(value) => Some(value),
        None => panic!("unknown {what} code {code}"),
    }
}

/// This team's wire code: `home = 1, away = 2`. `pub` (unlike most `*_code`
/// helpers here) because the enum-density contract ("numbers every enum
/// from one, leaving zero for absent") is exercised by
/// `tests/frame_buffer.rs`'s
/// `numbers_every_enum_from_one_leaving_zero_for_absent`, external to this
/// module.
#[must_use]
pub fn team_code(team: Team) -> f64 {
    match team {
        Team::Home => 1.0,
        Team::Away => 2.0,
    }
}

/// This silhouette's wire code. Numbered as `SPECIES_SHAPE`: `round = 1,
/// broad = 2, angular = 3, cluster = 4`.
#[must_use]
pub fn species_shape_code(shape: Shape) -> f64 {
    match shape {
        Shape::Round => 1.0,
        Shape::Broad => 2.0,
        Shape::Angular => 3.0,
        Shape::Cluster => 4.0,
    }
}

/// This charge kind's wire code. Numbered as `CHARGE_KIND`: `shot = 1,
/// pass = 2`.
#[must_use]
pub fn charge_kind_code(kind: RenderChargeKind) -> f64 {
    match kind {
        RenderChargeKind::Shot => 1.0,
        RenderChargeKind::Pass => 2.0,
    }
}

/// This pose's source system wire code. Numbered as `POSE_SOURCE`:
/// `soccer = 1, combat = 2, locomotion = 3`.
#[must_use]
pub fn pose_source_code(source: PlayerPoseSource) -> f64 {
    match source {
        PlayerPoseSource::Soccer => 1.0,
        PlayerPoseSource::Combat => 2.0,
        PlayerPoseSource::Locomotion => 3.0,
    }
}

/// This aerial style's wire code. Numbered as `AERIAL_STYLE`: `leg_control
/// = 1, chest_control = 2, volley = 3, header = 4, bicycle = 5`.
#[must_use]
pub fn aerial_style_code(style: AerialStyle) -> f64 {
    match style {
        AerialStyle::LegControl => 1.0,
        AerialStyle::ChestControl => 2.0,
        AerialStyle::Volley => 3.0,
        AerialStyle::Header => 4.0,
        AerialStyle::Bicycle => 5.0,
    }
}

/// This aerial outcome's wire code. Numbered as `AERIAL_OUTCOME`: `clean =
/// 1, heavy = 2, miss = 3`.
#[must_use]
pub fn aerial_outcome_code(outcome: AerialOutcome) -> f64 {
    match outcome {
        AerialOutcome::Clean => 1.0,
        AerialOutcome::Heavy => 2.0,
        AerialOutcome::Miss => 3.0,
    }
}

/// This save style's wire code. Numbered as `SAVE_STYLE`: `spread = 1,
/// central = 2, stretch = 3`.
#[must_use]
pub fn save_style_code(style: SaveStyle) -> f64 {
    match style {
        SaveStyle::Spread => 1.0,
        SaveStyle::Central => 2.0,
        SaveStyle::Stretch => 3.0,
    }
}

/// This keeper behavior state's wire code. Numbered as `KEEPER_STATE`:
/// `base = 1, advance = 2, contain = 3, set = 4, retreat = 5, recover = 6`.
#[must_use]
pub fn keeper_state_code(state: KeeperBehaviorState) -> f64 {
    match state {
        KeeperBehaviorState::Base => 1.0,
        KeeperBehaviorState::Advance => 2.0,
        KeeperBehaviorState::Contain => 3.0,
        KeeperBehaviorState::Set => 4.0,
        KeeperBehaviorState::Retreat => 5.0,
        KeeperBehaviorState::Recover => 6.0,
    }
}

/// This shot type's wire code. Numbered as `SHOT_TYPE`: `ground = 1, chip
/// = 2`.
#[must_use]
pub fn shot_type_code(shot_type: KeeperShotType) -> f64 {
    match shot_type {
        KeeperShotType::Ground => 1.0,
        KeeperShotType::Chip => 2.0,
    }
}

/// This pose's wire code: `keeper_grab` through `locomotion`, 1..32.
#[must_use]
pub fn pose_id_code(id: PlayerPoseId) -> f64 {
    use PlayerPoseId::{
        AerialAction, AerialBicycle, CombatActive, CombatAim, CombatGuard, CombatKnockback,
        CombatRecovery, CombatStagger, CombatWindup, Contain, Fatigue, KeeperCentral, KeeperDive,
        KeeperGetUp, KeeperGrab, KeeperPunt, KeeperReadyLow, KeeperReadyTall, KeeperSet,
        KeeperShuffle, KeeperSpread, KeeperStretch, KeeperThrow, KeeperTip, KickFollow, Locomotion,
        RunTelegraph, Settle, Slide, SoccerWindup, Stumble, Tackle,
    };
    (match id {
        KeeperGrab => 1,
        KeeperThrow => 2,
        KeeperPunt => 3,
        KeeperTip => 4,
        KeeperSpread => 5,
        KeeperCentral => 6,
        KeeperStretch => 7,
        KeeperDive => 8,
        KeeperGetUp => 9,
        KeeperSet => 10,
        KeeperReadyLow => 11,
        KeeperShuffle => 12,
        KeeperReadyTall => 13,
        AerialBicycle => 14,
        AerialAction => 15,
        CombatKnockback => 16,
        CombatStagger => 17,
        CombatGuard => 18,
        CombatActive => 19,
        CombatWindup => 20,
        CombatAim => 21,
        CombatRecovery => 22,
        SoccerWindup => 23,
        Slide => 24,
        Tackle => 25,
        Stumble => 26,
        KickFollow => 27,
        Settle => 28,
        RunTelegraph => 29,
        Contain => 30,
        Fatigue => 31,
        Locomotion => 32,
    }) as f64
}

/// Decode a pose id wire code. `None` for 0 (never a real pose) and for any
/// code this numbering does not recognise; the [`pose_id`] convenience
/// wrapper is what turns an unrecognised nonzero code into a panic.
#[must_use]
pub fn pose_id_from_code(code: f64) -> Option<PlayerPoseId> {
    use PlayerPoseId::{
        AerialAction, AerialBicycle, CombatActive, CombatAim, CombatGuard, CombatKnockback,
        CombatRecovery, CombatStagger, CombatWindup, Contain, Fatigue, KeeperCentral, KeeperDive,
        KeeperGetUp, KeeperGrab, KeeperPunt, KeeperReadyLow, KeeperReadyTall, KeeperSet,
        KeeperShuffle, KeeperSpread, KeeperStretch, KeeperThrow, KeeperTip, KickFollow, Locomotion,
        RunTelegraph, Settle, Slide, SoccerWindup, Stumble, Tackle,
    };
    match code as i64 {
        1 => Some(KeeperGrab),
        2 => Some(KeeperThrow),
        3 => Some(KeeperPunt),
        4 => Some(KeeperTip),
        5 => Some(KeeperSpread),
        6 => Some(KeeperCentral),
        7 => Some(KeeperStretch),
        8 => Some(KeeperDive),
        9 => Some(KeeperGetUp),
        10 => Some(KeeperSet),
        11 => Some(KeeperReadyLow),
        12 => Some(KeeperShuffle),
        13 => Some(KeeperReadyTall),
        14 => Some(AerialBicycle),
        15 => Some(AerialAction),
        16 => Some(CombatKnockback),
        17 => Some(CombatStagger),
        18 => Some(CombatGuard),
        19 => Some(CombatActive),
        20 => Some(CombatWindup),
        21 => Some(CombatAim),
        22 => Some(CombatRecovery),
        23 => Some(SoccerWindup),
        24 => Some(Slide),
        25 => Some(Tackle),
        26 => Some(Stumble),
        27 => Some(KickFollow),
        28 => Some(Settle),
        29 => Some(RunTelegraph),
        30 => Some(Contain),
        31 => Some(Fatigue),
        32 => Some(Locomotion),
        _ => None,
    }
}

/// This event kind's wire code: `shot` through `juke`, 1..25.
#[must_use]
pub fn event_kind_code(kind: MatchEventKind) -> f64 {
    use MatchEventKind::{
        Bicycle, Block, Catch, Claim, CombatCommitCarrierContest, CombatCommitCarrierProtection,
        CombatCommitLooseBallContest, CombatCommitPassingLaneOrShotDenial,
        CombatCommitRecoveryPunish, CombatCommitUnattributedOffBall, Header, Juke, Parry, Pass,
        PressCommitBoxDesperation, PressCommitCover, PressCommitExposedBall, PressCommitHeavyTouch,
        PressCommitLowDiscipline, Reception, Shot, Tackle, Tip, Touch, Volley,
    };
    (match kind {
        Shot => 1,
        Pass => 2,
        Touch => 3,
        Tackle => 4,
        PressCommitHeavyTouch => 5,
        PressCommitExposedBall => 6,
        PressCommitCover => 7,
        PressCommitBoxDesperation => 8,
        PressCommitLowDiscipline => 9,
        CombatCommitCarrierContest => 10,
        CombatCommitCarrierProtection => 11,
        CombatCommitLooseBallContest => 12,
        CombatCommitPassingLaneOrShotDenial => 13,
        CombatCommitRecoveryPunish => 14,
        CombatCommitUnattributedOffBall => 15,
        Catch => 16,
        Parry => 17,
        Tip => 18,
        Claim => 19,
        Block => 20,
        Header => 21,
        Volley => 22,
        Bicycle => 23,
        Reception => 24,
        Juke => 25,
    }) as f64
}

/// Decode an event kind wire code. `None` for 0 (never a real event) and
/// for any code this numbering does not recognise; the [`event_kind`]
/// convenience wrapper is what turns an unrecognised nonzero code into a
/// panic.
#[must_use]
pub fn event_kind_from_code(code: f64) -> Option<MatchEventKind> {
    use MatchEventKind::{
        Bicycle, Block, Catch, Claim, CombatCommitCarrierContest, CombatCommitCarrierProtection,
        CombatCommitLooseBallContest, CombatCommitPassingLaneOrShotDenial,
        CombatCommitRecoveryPunish, CombatCommitUnattributedOffBall, Header, Juke, Parry, Pass,
        PressCommitBoxDesperation, PressCommitCover, PressCommitExposedBall, PressCommitHeavyTouch,
        PressCommitLowDiscipline, Reception, Shot, Tackle, Tip, Touch, Volley,
    };
    match code as i64 {
        1 => Some(Shot),
        2 => Some(Pass),
        3 => Some(Touch),
        4 => Some(Tackle),
        5 => Some(PressCommitHeavyTouch),
        6 => Some(PressCommitExposedBall),
        7 => Some(PressCommitCover),
        8 => Some(PressCommitBoxDesperation),
        9 => Some(PressCommitLowDiscipline),
        10 => Some(CombatCommitCarrierContest),
        11 => Some(CombatCommitCarrierProtection),
        12 => Some(CombatCommitLooseBallContest),
        13 => Some(CombatCommitPassingLaneOrShotDenial),
        14 => Some(CombatCommitRecoveryPunish),
        15 => Some(CombatCommitUnattributedOffBall),
        16 => Some(Catch),
        17 => Some(Parry),
        18 => Some(Tip),
        19 => Some(Claim),
        20 => Some(Block),
        21 => Some(Header),
        22 => Some(Volley),
        23 => Some(Bicycle),
        24 => Some(Reception),
        25 => Some(Juke),
        _ => None,
    }
}

/// Exact word count of a frame block. A reader derives its view length from
/// the header instead, but the producer needs it up front to size a reused
/// array.
#[must_use]
pub fn frame_words(count: usize, event_count: usize) -> usize {
    HEADER_WORDS + SCALAR_FIELD_COUNT + PLAYER_FIELD_COUNT * count + EVENT_FIELD_COUNT * event_count
}

fn write_scalars(frame: &RenderFrame, out: &mut [f64]) {
    let field = &frame.field;
    let ball = &frame.ball;
    let possession = &frame.possession;
    let control = &frame.control;
    let hud = &frame.hud;
    let color = hud.species_color;
    let values: [f64; SCALAR_FIELD_COUNT] = [
        field.w,
        field.h,
        field.crossbar_h,
        field.penalty_box_depth,
        field.penalty_box_h,
        field.goal_home.x,
        field.goal_home.y,
        field.goal_home.w,
        field.goal_home.h,
        field.goal_away.x,
        field.goal_away.y,
        field.goal_away.w,
        field.goal_away.h,
        ball.x,
        ball.y,
        ball.z,
        ball.vx,
        ball.vy,
        ball.vz,
        bw(ball.visible),
        bw(ball.landing_x.is_some()),
        ball.landing_x.unwrap_or(0.0),
        ball.landing_y.unwrap_or(0.0),
        possession.owner.unwrap_or(0) as f64,
        opt_code(possession.owner_team, team_code),
        bw(possession.keeper_holds),
        control.controlled as f64,
        control.pass_target.unwrap_or(0) as f64,
        opt_code(control.charge_kind, charge_kind_code),
        control.charge,
        hud.home_score as f64,
        hud.away_score as f64,
        hud.time_left,
        bw(hud.finished),
        opt_code(hud.possession_team, team_code),
        hud.controlled as f64,
        team_code(hud.controlled_team),
        bw(hud.controlled_is_keeper),
        bw(hud.controlled_owns_ball),
        hud.controlled_stamina,
        species_shape_code(hud.species_shape),
        color[0],
        color[1],
        color[2],
    ];
    out.copy_from_slice(&values);
}

// Field-major fill of one structure-of-arrays section: `out` holds
// `field_count` runs of `count` words each. `at(field_index, slot_index)`
// takes a fixed field order (0..field_count) baked into each caller instead
// of a name lookup — Rust has no generic "index a struct by field name"
// without reflection this crate does not have, so a hand-written call per
// field is what keeps the field order consistent.
fn soa_at(field_index: usize, slot_index: usize, count: usize) -> usize {
    field_index * count + slot_index
}

fn write_players(players: &RenderFramePlayers, out: &mut [f64], count: usize) {
    debug_assert_eq!(out.len(), PLAYER_FIELD_COUNT * count);
    for i in 0..count {
        out[soa_at(0, i, count)] = players.x[i];
        out[soa_at(1, i, count)] = players.y[i];
        out[soa_at(2, i, count)] = players.facing_x[i];
        out[soa_at(3, i, count)] = players.facing_y[i];
        out[soa_at(4, i, count)] = players.speed[i];
        out[soa_at(5, i, count)] = pose_id_code(players.pose_id[i]);
        out[soa_at(6, i, count)] = players.pose_priority[i] as f64;
        out[soa_at(7, i, count)] = pose_source_code(players.pose_source[i]);
        out[soa_at(8, i, count)] = bw(players.controlled[i]);
        out[soa_at(9, i, count)] = bw(players.dashing[i]);
        out[soa_at(10, i, count)] = bw(players.holding[i]);
        out[soa_at(11, i, count)] = players.dive[i];
        out[soa_at(12, i, count)] = players.dive_dir_x[i];
        out[soa_at(13, i, count)] = players.dive_dir_y[i];
        out[soa_at(14, i, count)] = players.grab[i];
        out[soa_at(15, i, count)] = players.throw[i];
        out[soa_at(16, i, count)] = players.windup[i];
        out[soa_at(17, i, count)] = players.aerial[i];
        out[soa_at(18, i, count)] = players.aerial_jump[i];
        out[soa_at(19, i, count)] = opt_code(players.aerial_style[i], aerial_style_code);
        out[soa_at(20, i, count)] = opt_code(players.aerial_outcome[i], aerial_outcome_code);
    }
}

fn write_events(events: &RenderFrameEvents, out: &mut [f64], count: usize) {
    debug_assert_eq!(out.len(), EVENT_FIELD_COUNT * count);
    for i in 0..count {
        out[soa_at(0, i, count)] = event_kind_code(events.kind[i]);
        out[soa_at(1, i, count)] = events.x[i];
        out[soa_at(2, i, count)] = events.y[i];
        out[soa_at(3, i, count)] = events.slot[i].unwrap_or(0) as f64;
        out[soa_at(4, i, count)] = opt_code(events.save_style[i], save_style_code);
        out[soa_at(5, i, count)] = opt_code(events.style[i], aerial_style_code);
        out[soa_at(6, i, count)] = opt_code(events.outcome[i], aerial_outcome_code);
        out[soa_at(7, i, count)] = bw(events.difficulty[i].is_some());
        out[soa_at(8, i, count)] = events.difficulty[i].unwrap_or(0.0);
        out[soa_at(9, i, count)] = opt_code(events.shot_type[i], shot_type_code);
        out[soa_at(10, i, count)] = opt_code(events.keeper_state[i], keeper_state_code);
        out[soa_at(11, i, count)] = bw(events.keeper_depth[i].is_some());
        out[soa_at(12, i, count)] = events.keeper_depth[i].unwrap_or(0.0);
        out[soa_at(13, i, count)] = events.jumping[i] as f64;
        out[soa_at(14, i, count)] = events.on_target[i] as f64;
    }
}

/// Serialise one whole frame into one flat array of doubles.
///
/// `words` is reused in place: resized to exactly this frame's length and
/// refilled, so a per-frame producer allocates once and never again — Rust's
/// ownership model makes this the caller's own array by construction, a
/// structural guarantee rather than a runtime promise.
///
/// # Panics
///
/// If `frame.version` or `frame.roster.version` do not match
/// [`crate::frame::VERSION`] — a frame from a foreign or future producer
/// carries fields this layout would read positionally and misinterpret.
pub fn encode(frame: &RenderFrame, words: &mut Vec<f64>) {
    assert!(
        frame.version == frame::VERSION,
        "render frame version {}; expected {}",
        frame.version,
        frame::VERSION
    );
    assert!(
        frame.roster.version == frame::VERSION,
        "render roster version {}; expected {}",
        frame.roster.version,
        frame::VERSION
    );

    let count = frame.players.count;
    let event_count = frame.events.count;
    let total = frame_words(count, event_count);

    words.resize(total, 0.0);

    words[0] = f64::from(MAGIC);
    words[1] = f64::from(LAYOUT_VERSION);
    words[2] = f64::from(frame::VERSION);
    words[3] = total as f64;
    words[4] = HEADER_WORDS as f64;
    words[5] = SCALAR_FIELD_COUNT as f64;
    words[6] = count as f64;
    words[7] = PLAYER_FIELD_COUNT as f64;
    words[8] = event_count as f64;
    words[9] = EVENT_FIELD_COUNT as f64;
    words[10] = bw(frame.combat.is_some());
    words[11] = 0.0;

    let scalars_at = HEADER_WORDS;
    write_scalars(
        frame,
        &mut words[scalars_at..scalars_at + SCALAR_FIELD_COUNT],
    );

    let players_at = scalars_at + SCALAR_FIELD_COUNT;
    write_players(
        &frame.players,
        &mut words[players_at..players_at + PLAYER_FIELD_COUNT * count],
        count,
    );

    let events_at = players_at + PLAYER_FIELD_COUNT * count;
    write_events(
        &frame.events,
        &mut words[events_at..events_at + EVENT_FIELD_COUNT * event_count],
        event_count,
    );
}

/// Serialise the match-constant roster. Two return values because it has two
/// halves with different shapes: a numeric block and the string fields.
/// Both cross once per match, which is why the strings are affordable here
/// and nowhere else.
///
/// The blob is newline-joined `id`, `name`, `presentation_id`, `loadout_id`
/// per slot, in slot order — [`ROSTER_STRING_FIELD_COUNT`] parts each.
/// Newlines in any of them would make it unparseable, so they are rejected
/// rather than escaped: no authored id, presentation name or content id
/// contains one, and a scheme with no escapes has no escaping bug.
///
/// ABSENCE IS THE EMPTY STRING, AND ONLY FOR `loadout_id`. A keeper carries
/// nothing, so `loadout_ids[i]` is `None` and the blob holds an empty part
/// there; [`decode_roster`] maps it straight back to `None`. That is a real
/// encoding of absence rather than a sentinel standing in for a value: no
/// authored loadout id is the empty string, nothing downstream may treat
/// `""` as a loadout, and `presentation_id` — which is never absent — is
/// asserted non-empty here so the two cannot be confused.
///
/// # Panics
///
/// If `roster.version` does not match [`crate::frame::VERSION`], if the
/// string columns are not all `count` long, if any string contains a
/// newline, or if a `presentation_id` is empty.
#[must_use]
pub fn encode_roster(roster: &RenderFrameRoster) -> (Vec<f64>, String) {
    assert!(
        roster.version == frame::VERSION,
        "render roster version {}; expected {}",
        roster.version,
        frame::VERSION
    );

    let count = roster.count;
    let total = ROSTER_HEADER_WORDS + ROSTER_FIELD_COUNT * count;

    let mut words = vec![0.0; total];
    words[0] = f64::from(ROSTER_MAGIC);
    words[1] = f64::from(LAYOUT_VERSION);
    words[2] = f64::from(frame::VERSION);
    words[3] = total as f64;
    words[4] = ROSTER_HEADER_WORDS as f64;
    words[5] = count as f64;
    words[6] = ROSTER_FIELD_COUNT as f64;

    let at = ROSTER_HEADER_WORDS;
    for i in 0..count {
        words[at + soa_at(0, i, count)] = team_code(roster.teams[i]);
        words[at + soa_at(1, i, count)] = bw(roster.is_keeper[i]);
        words[at + soa_at(2, i, count)] = roster.radius[i];
        words[at + soa_at(3, i, count)] = species_shape_code(roster.species_shape[i]);
        words[at + soa_at(4, i, count)] = roster.species_color[i][0];
        words[at + soa_at(5, i, count)] = roster.species_color[i][1];
        words[at + soa_at(6, i, count)] = roster.species_color[i][2];
    }

    assert!(
        roster.presentation_ids.len() == count && roster.loadout_ids.len() == count,
        "roster string columns must all be {} long; presentation_ids is {}, loadout_ids is {}",
        count,
        roster.presentation_ids.len(),
        roster.loadout_ids.len()
    );

    let mut parts = Vec::with_capacity(count * ROSTER_STRING_FIELD_COUNT);
    for index in 0..count {
        let id = &roster.ids[index];
        let name = &roster.names[index];
        let presentation_id = &roster.presentation_ids[index];
        // `None` is the keeper rule; the empty string is how it crosses.
        let loadout_id = roster.loadout_ids[index].as_deref().unwrap_or("");
        assert!(
            !id.contains('\n'),
            "roster id must not contain a newline: {id}"
        );
        assert!(
            !name.contains('\n'),
            "roster name must not contain a newline: {name}"
        );
        assert!(
            !presentation_id.is_empty(),
            "roster presentation id must not be empty (slot {index}): every authored player names one"
        );
        assert!(
            !presentation_id.contains('\n'),
            "roster presentation id must not contain a newline: {presentation_id}"
        );
        assert!(
            !loadout_id.contains('\n'),
            "roster loadout id must not contain a newline: {loadout_id}"
        );
        parts.push(id.as_str());
        parts.push(name.as_str());
        parts.push(presentation_id.as_str());
        parts.push(loadout_id);
    }
    (words, parts.join("\n"))
}

/// A decoded frame's per-player structure-of-arrays. Field values are the
/// raw wire words: use [`pose_id_from_code`] (or the [`pose_id`] convenience
/// wrapper) to turn `pose_id` back into [`PlayerPoseId`]; the rest read
/// directly as numbers/booleans-as-0-or-1/enum-codes exactly as encoded.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedPlayers {
    /// Displayed world x.
    pub x: Vec<f64>,
    /// Displayed world y.
    pub y: Vec<f64>,
    /// Facing x.
    pub facing_x: Vec<f64>,
    /// Facing y.
    pub facing_y: Vec<f64>,
    /// Locomotion speed.
    pub speed: Vec<f64>,
    /// Pose id wire code; see [`pose_id_from_code`].
    pub pose_id: Vec<f64>,
    /// Pose priority.
    pub pose_priority: Vec<f64>,
    /// Pose source wire code (1 = soccer, 2 = combat, 3 = locomotion).
    pub pose_source: Vec<f64>,
    /// 0/1: locally controlled.
    pub controlled: Vec<f64>,
    /// 0/1: mid slide-tackle.
    pub dashing: Vec<f64>,
    /// 0/1: keeper carrying the ball in the hands.
    pub holding: Vec<f64>,
    /// 0..1 dive ease.
    pub dive: Vec<f64>,
    /// Dive direction x.
    pub dive_dir_x: Vec<f64>,
    /// Dive direction y.
    pub dive_dir_y: Vec<f64>,
    /// 0..1 grab ease.
    pub grab: Vec<f64>,
    /// 0..1 throw ease.
    pub throw: Vec<f64>,
    /// Wind-up back-swing, unclamped above 1.
    pub windup: Vec<f64>,
    /// 0..1 aerial ease.
    pub aerial: Vec<f64>,
    /// 0..1 required lift.
    pub aerial_jump: Vec<f64>,
    /// Aerial style wire code, 0 = absent.
    pub aerial_style: Vec<f64>,
    /// Aerial outcome wire code, 0 = absent.
    pub aerial_outcome: Vec<f64>,
}

/// A decoded frame's per-event structure-of-arrays. Field values are the
/// raw wire words; see [`event_kind_from_code`]/[`event_kind`] for `kind`.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedEvents {
    /// Event kind wire code; see [`event_kind_from_code`].
    pub kind: Vec<f64>,
    /// World x.
    pub x: Vec<f64>,
    /// World y.
    pub y: Vec<f64>,
    /// Roster slot (one-based), 0 = unattributed.
    pub slot: Vec<f64>,
    /// Save style wire code, 0 = absent.
    pub save_style: Vec<f64>,
    /// Aerial style wire code, 0 = absent.
    pub style: Vec<f64>,
    /// Aerial outcome wire code, 0 = absent.
    pub outcome: Vec<f64>,
    /// 0/1: whether `difficulty` was reported.
    pub has_difficulty: Vec<f64>,
    /// Save difficulty; meaningless unless `has_difficulty` is 1.
    pub difficulty: Vec<f64>,
    /// Shot type wire code, 0 = absent.
    pub shot_type: Vec<f64>,
    /// Keeper behavior state wire code, 0 = absent.
    pub keeper_state: Vec<f64>,
    /// 0/1: whether `keeper_depth` was reported.
    pub has_keeper_depth: Vec<f64>,
    /// Keeper line depth; meaningless unless `has_keeper_depth` is 1.
    pub keeper_depth: Vec<f64>,
    /// Tri-state: 0 = not reported, 1 = false, 2 = true.
    pub jumping: Vec<f64>,
    /// Tri-state: 0 = not reported, 1 = false, 2 = true.
    pub on_target: Vec<f64>,
}

fn column(words: &[f64], at: usize, field_index: usize, count: usize) -> Vec<f64> {
    let start = at + soa_at(field_index, 0, count);
    words[start..start + count].to_vec()
}

fn decode_players(words: &[f64], at: usize, count: usize) -> DecodedPlayers {
    DecodedPlayers {
        x: column(words, at, 0, count),
        y: column(words, at, 1, count),
        facing_x: column(words, at, 2, count),
        facing_y: column(words, at, 3, count),
        speed: column(words, at, 4, count),
        pose_id: column(words, at, 5, count),
        pose_priority: column(words, at, 6, count),
        pose_source: column(words, at, 7, count),
        controlled: column(words, at, 8, count),
        dashing: column(words, at, 9, count),
        holding: column(words, at, 10, count),
        dive: column(words, at, 11, count),
        dive_dir_x: column(words, at, 12, count),
        dive_dir_y: column(words, at, 13, count),
        grab: column(words, at, 14, count),
        throw: column(words, at, 15, count),
        windup: column(words, at, 16, count),
        aerial: column(words, at, 17, count),
        aerial_jump: column(words, at, 18, count),
        aerial_style: column(words, at, 19, count),
        aerial_outcome: column(words, at, 20, count),
    }
}

fn decode_events(words: &[f64], at: usize, count: usize) -> DecodedEvents {
    DecodedEvents {
        kind: column(words, at, 0, count),
        x: column(words, at, 1, count),
        y: column(words, at, 2, count),
        slot: column(words, at, 3, count),
        save_style: column(words, at, 4, count),
        style: column(words, at, 5, count),
        outcome: column(words, at, 6, count),
        has_difficulty: column(words, at, 7, count),
        difficulty: column(words, at, 8, count),
        shot_type: column(words, at, 9, count),
        keeper_state: column(words, at, 10, count),
        has_keeper_depth: column(words, at, 11, count),
        keeper_depth: column(words, at, 12, count),
        jumping: column(words, at, 13, count),
        on_target: column(words, at, 14, count),
    }
}

/// Decoded shapes. Deliberately NOT [`RenderFrame`]: a decoded frame has
/// integer roster slots where the payload had id strings, and no `combat`.
/// Typing it as the thing it is keeps a reader from assuming the round trip
/// is lossless.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedRenderFrame {
    /// This block's layout version.
    pub layout_version: u32,
    /// The render frame protocol version this block was produced from.
    pub render_frame_version: u32,
    /// Whether the source frame carried a combat telegraph model (dropped,
    /// not encoded — see this module's "WHAT IS NOT CARRIED").
    pub combat_present: bool,
    /// The 44 once-per-frame scalar words, in [`SCALAR_FIELD_COUNT`] order
    /// (see [`write_scalars`]'s field list for the exact order).
    pub scalars: [f64; SCALAR_FIELD_COUNT],
    /// Per-player structure-of-arrays.
    pub players: DecodedPlayers,
    /// Per-event structure-of-arrays.
    pub events: DecodedEvents,
}

/// Read a frame block back.
///
/// # Panics
///
/// On any of: a bad magic word, a layout or render-frame version mismatch,
/// a header field count that disagrees with this module's constants, or a
/// `total_words` that disagrees with the header's own `count`/`event_count`.
/// Each is a corrupted or foreign block, not a value this reader is meant to
/// recover from.
#[must_use]
pub fn decode(words: &[f64]) -> DecodedRenderFrame {
    assert!(
        words[0] == f64::from(MAGIC),
        "not a render frame block: magic {}",
        words[0]
    );
    assert!(
        words[1] == f64::from(LAYOUT_VERSION),
        "frame layout version {}; expected {}",
        words[1],
        LAYOUT_VERSION
    );
    assert!(
        words[2] == f64::from(frame::VERSION),
        "render frame version {}; expected {}",
        words[2],
        frame::VERSION
    );
    assert!(
        words[4] == HEADER_WORDS as f64,
        "header words {}; expected {}",
        words[4],
        HEADER_WORDS
    );
    assert!(
        words[5] == SCALAR_FIELD_COUNT as f64,
        "scalar words {}; expected {}",
        words[5],
        SCALAR_FIELD_COUNT
    );
    assert!(
        words[7] == PLAYER_FIELD_COUNT as f64,
        "player field count {}; expected {}",
        words[7],
        PLAYER_FIELD_COUNT
    );
    assert!(
        words[9] == EVENT_FIELD_COUNT as f64,
        "event field count {}; expected {}",
        words[9],
        EVENT_FIELD_COUNT
    );

    let count = words[6] as usize;
    let event_count = words[8] as usize;
    let total = frame_words(count, event_count);
    assert!(
        words[3] == total as f64,
        "total words {}; expected {}",
        words[3],
        total
    );

    let scalars_at = HEADER_WORDS;
    let mut scalars = [0.0; SCALAR_FIELD_COUNT];
    scalars.copy_from_slice(&words[scalars_at..scalars_at + SCALAR_FIELD_COUNT]);

    let players_at = scalars_at + SCALAR_FIELD_COUNT;
    let events_at = players_at + PLAYER_FIELD_COUNT * count;

    DecodedRenderFrame {
        layout_version: words[1] as u32,
        render_frame_version: words[2] as u32,
        combat_present: words[10] == 1.0,
        scalars,
        players: decode_players(words, players_at, count),
        events: decode_events(words, events_at, event_count),
    }
}

/// A decoded roster block's per-slot structure-of-arrays.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedRosterFields {
    /// Team wire code (1 = home, 2 = away).
    pub team: Vec<f64>,
    /// 0/1: is the keeper.
    pub is_keeper: Vec<f64>,
    /// Collision radius.
    pub radius: Vec<f64>,
    /// Species shape wire code.
    pub species_shape: Vec<f64>,
    /// Palette red channel.
    pub species_color_r: Vec<f64>,
    /// Palette green channel.
    pub species_color_g: Vec<f64>,
    /// Palette blue channel.
    pub species_color_b: Vec<f64>,
}

fn decode_roster_fields(words: &[f64], at: usize, count: usize) -> DecodedRosterFields {
    DecodedRosterFields {
        team: column(words, at, 0, count),
        is_keeper: column(words, at, 1, count),
        radius: column(words, at, 2, count),
        species_shape: column(words, at, 3, count),
        species_color_r: column(words, at, 4, count),
        species_color_g: column(words, at, 5, count),
        species_color_b: column(words, at, 6, count),
    }
}

/// A decoded roster block.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedRenderFrameRoster {
    /// This block's layout version.
    pub layout_version: u32,
    /// The render frame protocol version this block was produced from.
    pub render_frame_version: u32,
    /// Roster slot count.
    pub count: usize,
    /// Roster ids, recovered from the string blob.
    pub ids: Vec<String>,
    /// Presentation names, recovered from the string blob.
    pub names: Vec<String>,
    /// Authored character presentation ids, recovered from the string blob.
    pub presentation_ids: Vec<String>,
    /// Authored loadout ids, recovered from the string blob; `None` where
    /// the slot carries nothing (see [`encode_roster`] on absence).
    pub loadout_ids: Vec<Option<String>>,
    /// Per-slot structure-of-arrays.
    pub fields: DecodedRosterFields,
}

/// Read a roster block back, pairing it with the string blob
/// [`encode_roster`] returned alongside it.
///
/// # Panics
///
/// On a bad magic word, a layout/version mismatch, a field count that
/// disagrees with [`ROSTER_FIELD_COUNT`], or a string blob that does not
/// hold exactly `ROSTER_STRING_FIELD_COUNT * count` newline-delimited parts.
#[must_use]
pub fn decode_roster(words: &[f64], strings: &str) -> DecodedRenderFrameRoster {
    assert!(
        words[0] == f64::from(ROSTER_MAGIC),
        "not a render roster block: magic {}",
        words[0]
    );
    assert!(
        words[1] == f64::from(LAYOUT_VERSION),
        "roster layout version {}; expected {}",
        words[1],
        LAYOUT_VERSION
    );
    assert!(
        words[2] == f64::from(frame::VERSION),
        "render frame version {}; expected {}",
        words[2],
        frame::VERSION
    );
    assert!(
        words[6] == ROSTER_FIELD_COUNT as f64,
        "roster field count {}; expected {}",
        words[6],
        ROSTER_FIELD_COUNT
    );

    let count = words[5] as usize;
    // `strings` is `id\nname\npresentation_id\nloadout_id\n...` (no trailing
    // newline); appending one and splitting on it recovers exactly the
    // `count * ROSTER_STRING_FIELD_COUNT` parts `encode_roster` joined.
    // `split` on the newline-terminated blob always yields one trailing
    // empty element past the last delimiter, and it is dropped here.
    //
    // THE BLOB FORMAT CHANGED AT #447: from two parts per slot to four
    // (`id`, `name`, `presentation_id`, `loadout_id`); this decoder reads
    // four (see the PR). Nothing still produces the old two-part blob, and
    // the one test that pins the wire against a frozen reference
    // (`tests/frame_buffer_differential.rs`) discards the blob entirely, so
    // this format change is in a lane the differential does not run in. A
    // mismatched producer/reader pair fails on the part-count assertion
    // below rather than mis-parsing.
    let mut blob = strings.to_string();
    blob.push('\n');
    let mut parts: Vec<&str> = blob.split('\n').collect();
    parts.pop();
    // THIS ASSERTION CARRIES THE BLOB'S ENTIRE SHAPE-VERSIONING BURDEN,
    // because [`LAYOUT_VERSION`] cannot be bumped for a blob change (it is
    // stamped into the numeric block, which is pinned word-for-word against a
    // frozen reference fixture — see [`ROSTER_STRING_FIELD_COUNT`]). And it
    // DEGENERATES TO A NO-OP at `count == 0`: `0 == 0` holds for any field
    // count, so a producer and a reader that disagreed would agree vacuously
    // on an empty roster. Harmless today — a match always fields two teams,
    // and an empty roster draws nothing whatever it decoded — but recorded
    // rather than left to be rediscovered, because the day something
    // legitimately decodes an empty roster is the day this stops guarding
    // anything.
    let expected_parts = count * ROSTER_STRING_FIELD_COUNT;
    assert!(
        parts.len() == expected_parts,
        "roster blob holds {} strings; expected {}",
        parts.len(),
        expected_parts
    );

    let mut ids = Vec::with_capacity(count);
    let mut names = Vec::with_capacity(count);
    let mut presentation_ids = Vec::with_capacity(count);
    let mut loadout_ids = Vec::with_capacity(count);
    for index in 0..count {
        let at = index * ROSTER_STRING_FIELD_COUNT;
        ids.push(parts[at].to_string());
        names.push(parts[at + 1].to_string());
        presentation_ids.push(parts[at + 2].to_string());
        let loadout = parts[at + 3];
        loadout_ids.push(if loadout.is_empty() {
            None
        } else {
            Some(loadout.to_string())
        });
    }

    DecodedRenderFrameRoster {
        layout_version: words[1] as u32,
        render_frame_version: words[2] as u32,
        count,
        ids,
        names,
        presentation_ids,
        loadout_ids,
        fields: decode_roster_fields(words, ROSTER_HEADER_WORDS, count),
    }
}

/// Convenience for a reader that wants the pose id back rather than the
/// code. `index` is 0-based into the roster slots.
///
/// # Panics
///
/// If the stored code is nonzero and unmapped (see [`pose_id_from_code`]).
#[must_use]
pub fn pose_id(decoded: &DecodedRenderFrame, index: usize) -> Option<PlayerPoseId> {
    opt_decode(decoded.players.pose_id[index], pose_id_from_code, "pose id")
}

/// Convenience for a reader that wants the event kind back rather than the
/// code. `index` is 0-based into the event slots.
///
/// # Panics
///
/// If the stored code is nonzero and unmapped (see [`event_kind_from_code`]).
#[must_use]
pub fn event_kind(decoded: &DecodedRenderFrame, index: usize) -> Option<MatchEventKind> {
    opt_decode(
        decoded.events.kind[index],
        event_kind_from_code,
        "event kind",
    )
}

#[cfg(test)]
mod enum_coverage {
    //! Sanity checks that the `*_code` functions above stay dense from 1.
    //! The enum-density contract itself ("numbers every enum from one,
    //! leaving zero for absent") is a real test in
    //! `tests/frame_buffer.rs`'s
    //! `numbers_every_enum_from_one_leaving_zero_for_absent`, not here;
    //! these are compile-time-ish guards that the from-code inverse tables
    //! stay total over the forward direction (every value `*_code` can
    //! produce, `*_from_code` recognises).

    use super::*;

    #[test]
    fn pose_id_round_trips_every_member() {
        for code in 1..=32 {
            let id =
                pose_id_from_code(code as f64).unwrap_or_else(|| panic!("no pose for code {code}"));
            assert_eq!(pose_id_code(id), code as f64);
        }
    }

    #[test]
    fn event_kind_round_trips_every_member() {
        for code in 1..=25 {
            let kind = event_kind_from_code(code as f64)
                .unwrap_or_else(|| panic!("no event kind for code {code}"));
            assert_eq!(event_kind_code(kind), code as f64);
        }
    }
}
