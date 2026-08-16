//! JSON conversion for `gc_sim::rollback_events` — the presentation-facing
//! stable-event timeline a match driver's tick outputs feed — plus, since the
//! W3-G wave, this module's own `#[wasm_bindgen]` surface: [`WasmMatchSnapshot`]
//! (an opaque handle) and [`RollbackEventsTimeline`] (`create`/`apply`/
//! `confirm`/`diagnosticsJson` as separate callables).
//!
//! ## Why a `MatchSnapshot` crosses as an opaque handle, never JSON
//!
//! [`gc_sim::rollback_events::apply`] needs a real
//! [`gc_sim::match_snapshot::MatchSnapshot`] per step
//! (`RollbackEventStepInput::snapshot`). This crate deliberately never
//! serializes a `MatchSnapshot` TO JSON anywhere: it is a large,
//! binary-shaped structure with an existing canonical wire encoding
//! (`gc_sim::match_snapshot::encode`) built for hashing and replication, not
//! for a JSON round trip a JS caller would hand-edit. [`WasmMatchSnapshot`]
//! keeps that rule intact while still letting a `MatchSnapshot` cross the
//! boundary: it is a `#[wasm_bindgen]` class with no exposed fields or
//! methods, so JS can only ever hold a reference to one (from
//! [`crate::session::Session::snapshot_handle`] or
//! [`crate::match_driver_bridge::MatchDriverBridge::snapshot_lookup`]) and
//! pass it back into [`RollbackEventsTimeline::apply`] — never inspect or
//! mutate it. That is exactly the `TSnapshot` opaque type parameter
//! `packages/online/src/match_presentation.ts`'s `RollbackEventsPort`/
//! `MatchDriverPort` declare.
//!
//! ## Why `apply`'s `outputs_json` needs real events, not counts
//!
//! [`crate::match_driver_bridge::MatchDriverBridge::advance`]'s own JSON
//! (`output_summary_to_json`) used to report only `event_count`/
//! `combat_event_count` per output — enough for a diagnostics readout, but
//! not enough to reconstruct a [`gc_sim::rollback_events::RollbackEventTickOutput`]
//! (`rollback_events::apply` needs the actual `events`/`combat_events`, not a
//! count). [`tick_output_to_json`]/[`tick_output_from_json`] are the
//! round-trippable pair that closes that gap: `advance`'s batch now embeds
//! the full shape per output (see that module's doc), and a caller building
//! `RollbackEventStepInput.output` for [`RollbackEventsTimeline::apply`]
//! passes that same JSON straight through — this module never needs to see
//! it "mean" anything, only decode it back into the same struct it was
//! encoded from.
//!
//! `MatchEventKind`/`CombatEventKind`/`CombatContactResult`/
//! `CombatRequestOutcome`/`CombatRequestRejectionReason`/
//! `CombatEncounterTerminal` have a `pub wire_str()` this module reuses
//! directly. `SaveStyle`/`AerialStyle`/`AerialOutcome`/`KeeperShotType`/
//! `KeeperBehaviorState`/`ActionFamilyId` do not (`gc_sim::rollback_events`'s
//! own `accounting` keeps a private local mirror of the first five for the
//! exact same reason, by that function's own comment: the canonical mapping
//! lives in `match_snapshot`'s private helpers, not `pub`) — so this module
//! keeps its own small local mirror too, rather than reaching across crates
//! for a private item or widening `gc-sim`'s public surface for glue this
//! wave does not own.

use gc_data::action_families::ActionFamilyId;
use gc_sim::aerial::{AerialOutcome, AerialStyle};
use gc_sim::combat_snapshot::{CombatEvent, CombatEventKind};
use gc_sim::keeper::{KeeperBehaviorState, KeeperShotType, SaveStyle};
use gc_sim::match_snapshot::{ByTeam, MatchEvent, MatchEventKind, MatchSnapshot, Team};
use gc_sim::rollback_events::{
    self, RollbackConfirmedStateView, RollbackEventDiff, RollbackEventPayload, RollbackEventStep,
    RollbackEventStepInput, RollbackEventTickOutput, RollbackEventTimeline,
    RollbackEventsAccounting, RollbackEventsDiagnostics, RollbackEventsErrorCode,
    RollbackLifecycleKind, RollbackLifecyclePayload, RollbackOutputStateView, RollbackWrappedEvent,
};
use wasm_bindgen::prelude::*;

use crate::json::{Json, debug_tag};

fn js_err(message: impl Into<String>) -> JsValue {
    JsValue::from_str(&message.into())
}

fn team_str(team: Team) -> &'static str {
    match team {
        Team::Home => "home",
        Team::Away => "away",
    }
}

/// [`debug_tag`], applied inside an `Option::map` for a `Copy` enum field —
/// [`debug_tag`] takes `&T`, so this is the by-value adapter every optional
/// enum field below needs.
fn opt_tag<T: std::fmt::Debug>(value: Option<T>) -> Json {
    Json::opt_str(value.map(|value| debug_tag(&value)).as_deref())
}

fn match_event_to_json(event: &MatchEvent) -> Json {
    Json::obj(vec![
        ("origin", Json::str("match")),
        ("kind", Json::str(match_event_kind_str(event.kind))),
        ("x", Json::Number(event.x)),
        ("y", Json::Number(event.y)),
        ("player", Json::opt_str(event.player.as_deref())),
        ("save_style", opt_tag(event.save_style)),
        ("style", opt_tag(event.style)),
        ("outcome", opt_tag(event.outcome)),
        ("jumping", event.jumping.map_or(Json::Null, Json::Bool)),
        (
            "difficulty",
            event.difficulty.map_or(Json::Null, Json::Number),
        ),
        ("shot_type", opt_tag(event.shot_type)),
        ("keeper_state", opt_tag(event.keeper_state)),
        (
            "keeper_depth",
            event.keeper_depth.map_or(Json::Null, Json::Number),
        ),
        ("on_target", event.on_target.map_or(Json::Null, Json::Bool)),
    ])
}

fn match_event_kind_str(kind: MatchEventKind) -> String {
    debug_tag(&kind)
}

fn combat_event_to_json(event: &CombatEvent) -> Json {
    Json::obj(vec![
        ("origin", Json::str("combat")),
        ("kind", Json::str(debug_tag::<CombatEventKind>(&event.kind))),
        ("tick", Json::int(event.tick)),
        (
            "family_id",
            Json::opt_str(
                event
                    .family_id
                    .map(gc_sim::combat_snapshot::action_family_wire_id),
            ),
        ),
        ("source_index", Json::opt_int(event.source_index)),
        ("target_index", Json::opt_int(event.target_index)),
        ("source_sequence", Json::opt_int(event.source_sequence)),
        ("result", opt_tag(event.result)),
        ("outcome", opt_tag(event.outcome)),
        ("reason", opt_tag(event.reason)),
        ("terminal", opt_tag(event.terminal)),
        ("x", Json::Number(event.x)),
        ("y", Json::Number(event.y)),
        (
            "interruption_ticks",
            Json::opt_int(event.interruption_ticks),
        ),
        (
            "displacement_px",
            event.displacement_px.map_or(Json::Null, Json::Number),
        ),
    ])
}

fn lifecycle_payload_to_json(payload: &RollbackLifecyclePayload) -> Json {
    Json::obj(vec![
        ("origin", Json::str("lifecycle")),
        ("kind", Json::str(lifecycle_kind_str(payload.kind))),
        (
            "team",
            payload
                .team
                .map_or(Json::Null, |team| Json::str(team_str(team))),
        ),
        (
            "score",
            Json::obj(vec![
                ("home", Json::int(payload.score.home)),
                ("away", Json::int(payload.score.away)),
            ]),
        ),
    ])
}

fn lifecycle_kind_str(kind: RollbackLifecycleKind) -> &'static str {
    kind.wire_str()
}

fn payload_to_json(payload: &RollbackEventPayload) -> Json {
    match payload {
        RollbackEventPayload::Match(event) => match_event_to_json(event),
        RollbackEventPayload::Combat(event) => combat_event_to_json(event),
        RollbackEventPayload::Lifecycle(payload) => lifecycle_payload_to_json(payload),
    }
}

fn wrapped_event_to_json(event: &RollbackWrappedEvent) -> Json {
    Json::obj(vec![
        ("id", Json::str(event.id.clone())),
        ("tick", Json::int(event.tick)),
        ("domain", Json::str(event.domain.clone())),
        ("ordinal", Json::int(event.ordinal)),
        ("payload", payload_to_json(&event.payload)),
    ])
}

/// Converts one [`RollbackEventDiff`] (an [`gc_sim::rollback_events::apply`]
/// result) to JSON: `{"added": [...], "revoked": [...], "replaced":
/// [{"before": ..., "after": ...}, ...]}`.
#[must_use]
pub(crate) fn diff_to_json(diff: &RollbackEventDiff) -> Json {
    Json::obj(vec![
        (
            "added",
            Json::Array(diff.added.iter().map(wrapped_event_to_json).collect()),
        ),
        (
            "revoked",
            Json::Array(diff.revoked.iter().map(wrapped_event_to_json).collect()),
        ),
        (
            "replaced",
            Json::Array(
                diff.replaced
                    .iter()
                    .map(|replacement| {
                        Json::obj(vec![
                            ("before", wrapped_event_to_json(&replacement.before)),
                            ("after", wrapped_event_to_json(&replacement.after)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn confirmed_state_to_json(state: &RollbackConfirmedStateView) -> Json {
    Json::obj(vec![
        (
            "score",
            Json::obj(vec![
                ("home", Json::int(state.score.home)),
                ("away", Json::int(state.score.away)),
            ]),
        ),
        ("time_left", Json::Number(state.time_left)),
        ("finished", Json::bool(state.finished)),
        ("owner_id", Json::opt_str(state.owner_id.as_deref())),
        (
            "owner_team",
            state
                .owner_team
                .map_or(Json::Null, |team| Json::str(team_str(team))),
        ),
    ])
}

/// One retained speculative step, as JSON — used by
/// [`crate::match_driver_bridge`]'s diagnostics surface only; the timeline
/// itself is never handed to JS wholesale (see the module doc).
#[must_use]
pub(crate) fn step_to_json(step: &RollbackEventStep) -> Json {
    Json::obj(vec![
        ("tick", Json::int(step.tick)),
        ("start_boundary", Json::int(step.start_boundary)),
        ("end_boundary", Json::int(step.end_boundary)),
        ("state", confirmed_state_to_json(&step.state)),
        (
            "match_events",
            Json::Array(
                step.match_events
                    .iter()
                    .map(wrapped_event_to_json)
                    .collect(),
            ),
        ),
        (
            "combat_events",
            step.combat_events.as_ref().map_or(Json::Null, |events| {
                Json::Array(events.iter().map(wrapped_event_to_json).collect())
            }),
        ),
        (
            "lifecycle_events",
            Json::Array(
                step.lifecycle_events
                    .iter()
                    .map(wrapped_event_to_json)
                    .collect(),
            ),
        ),
    ])
}

/// [`gc_sim::rollback_events::diagnostics`], as JSON.
#[must_use]
pub(crate) fn diagnostics_to_json(diagnostics: &RollbackEventsDiagnostics) -> Json {
    Json::obj(vec![
        ("status", Json::str(debug_tag(&diagnostics.status))),
        ("confirmed_tick", Json::int(diagnostics.confirmed_tick)),
        (
            "confirmed_boundary",
            Json::int(diagnostics.confirmed_boundary),
        ),
        (
            "max_unconfirmed_ticks",
            Json::int(diagnostics.max_unconfirmed_ticks),
        ),
        (
            "retained_step_count",
            Json::int(diagnostics.retained_step_count),
        ),
        (
            "retained_event_count",
            Json::int(diagnostics.retained_event_count),
        ),
        ("oldest_tick", Json::opt_int(diagnostics.oldest_tick)),
        ("latest_tick", Json::opt_int(diagnostics.latest_tick)),
    ])
}

/// [`gc_sim::rollback_events::accounting`], as JSON.
#[must_use]
pub(crate) fn accounting_to_json(accounting: &RollbackEventsAccounting) -> Json {
    Json::obj(vec![
        (
            "retained_step_bytes",
            Json::int(accounting.retained_step_bytes),
        ),
        ("total_bytes", Json::int(accounting.total_bytes)),
    ])
}

// ---------------------------------------------------------------------
// Raw, round-trippable `MatchEvent`/`CombatEvent` <-> JSON. Unlike
// `match_event_to_json`/`combat_event_to_json` above (which wrap a payload
// for presentation, tagged with `"origin"`), these encode/decode the plain
// struct fields one to one, so `tick_output_to_json`/`tick_output_from_json`
// can round-trip a whole `RollbackEventTickOutput` losslessly across the
// wasm boundary and back. See the module doc's "why `apply` needs real
// events" section.
// ---------------------------------------------------------------------

/// `pub(crate)`, not private: [`crate::match_state_bridge`] needs this exact
/// wire mapping for a [`crate::match_snapshot::MatchPlayer::save_style`]
/// field it also serializes (the raw-`MatchState`-for-replay surface) —
/// reusing it here keeps exactly one `SaveStyle` wire mapping in this crate
/// rather than a third copy (the module doc already explains why this is a
/// local mirror rather than a `gc_sim`-side `pub wire_str()`).
pub(crate) fn save_style_wire(v: SaveStyle) -> &'static str {
    match v {
        SaveStyle::Spread => "spread",
        SaveStyle::Central => "central",
        SaveStyle::Stretch => "stretch",
    }
}

/// `pub(crate)`: [`crate::player_pose_bridge`] reuses this one too, decoding
/// a caller-supplied `MatchPlayer.save_style` for standalone pose selection.
pub(crate) fn save_style_from_wire(text: &str) -> Result<SaveStyle, String> {
    match text {
        "spread" => Ok(SaveStyle::Spread),
        "central" => Ok(SaveStyle::Central),
        "stretch" => Ok(SaveStyle::Stretch),
        other => Err(format!("'{other}' is not a canonical save style")),
    }
}

/// `pub(crate)`: see [`save_style_wire`]'s doc — [`crate::match_state_bridge`]
/// reuses this one too.
pub(crate) fn aerial_style_wire(v: AerialStyle) -> &'static str {
    match v {
        AerialStyle::LegControl => "leg_control",
        AerialStyle::ChestControl => "chest_control",
        AerialStyle::Volley => "volley",
        AerialStyle::Header => "header",
        AerialStyle::Bicycle => "bicycle",
    }
}

/// `pub(crate)`: see [`save_style_from_wire`]'s doc — [`crate::player_pose_bridge`]
/// reuses this one too.
pub(crate) fn aerial_style_from_wire(text: &str) -> Result<AerialStyle, String> {
    match text {
        "leg_control" => Ok(AerialStyle::LegControl),
        "chest_control" => Ok(AerialStyle::ChestControl),
        "volley" => Ok(AerialStyle::Volley),
        "header" => Ok(AerialStyle::Header),
        "bicycle" => Ok(AerialStyle::Bicycle),
        other => Err(format!("'{other}' is not a canonical aerial style")),
    }
}

/// `pub(crate)`: see [`save_style_wire`]'s doc — [`crate::match_state_bridge`]
/// reuses this one too.
pub(crate) fn aerial_outcome_wire(v: AerialOutcome) -> &'static str {
    match v {
        AerialOutcome::Clean => "clean",
        AerialOutcome::Heavy => "heavy",
        AerialOutcome::Miss => "miss",
    }
}

/// `pub(crate)`: see [`save_style_from_wire`]'s doc — [`crate::player_pose_bridge`]
/// reuses this one too (for round-trip fidelity; `player_pose::select` itself
/// never reads `aerial_outcome`).
pub(crate) fn aerial_outcome_from_wire(text: &str) -> Result<AerialOutcome, String> {
    match text {
        "clean" => Ok(AerialOutcome::Clean),
        "heavy" => Ok(AerialOutcome::Heavy),
        "miss" => Ok(AerialOutcome::Miss),
        other => Err(format!("'{other}' is not a canonical aerial outcome")),
    }
}

fn keeper_shot_type_wire(v: KeeperShotType) -> &'static str {
    match v {
        KeeperShotType::Ground => "ground",
        KeeperShotType::Chip => "chip",
    }
}

fn keeper_shot_type_from_wire(text: &str) -> Result<KeeperShotType, String> {
    match text {
        "ground" => Ok(KeeperShotType::Ground),
        "chip" => Ok(KeeperShotType::Chip),
        other => Err(format!("'{other}' is not a canonical keeper shot type")),
    }
}

/// `pub(crate)`: see [`save_style_wire`]'s doc — [`crate::match_state_bridge`]
/// reuses this one too.
pub(crate) fn keeper_behavior_state_wire(v: KeeperBehaviorState) -> &'static str {
    match v {
        KeeperBehaviorState::Base => "base",
        KeeperBehaviorState::Advance => "advance",
        KeeperBehaviorState::Contain => "contain",
        KeeperBehaviorState::Set => "set",
        KeeperBehaviorState::Retreat => "retreat",
        KeeperBehaviorState::Recover => "recover",
    }
}

/// `pub(crate)`: see [`save_style_from_wire`]'s doc — [`crate::player_pose_bridge`]
/// reuses this one too.
pub(crate) fn keeper_behavior_state_from_wire(text: &str) -> Result<KeeperBehaviorState, String> {
    match text {
        "base" => Ok(KeeperBehaviorState::Base),
        "advance" => Ok(KeeperBehaviorState::Advance),
        "contain" => Ok(KeeperBehaviorState::Contain),
        "set" => Ok(KeeperBehaviorState::Set),
        "retreat" => Ok(KeeperBehaviorState::Retreat),
        "recover" => Ok(KeeperBehaviorState::Recover),
        other => Err(format!(
            "'{other}' is not a canonical keeper behavior state"
        )),
    }
}

fn match_event_kind_from_wire(text: &str) -> Result<MatchEventKind, String> {
    use MatchEventKind::{
        Bicycle, Block, Catch, Claim, CombatCommitCarrierContest, CombatCommitCarrierProtection,
        CombatCommitLooseBallContest, CombatCommitPassingLaneOrShotDenial,
        CombatCommitRecoveryPunish, CombatCommitUnattributedOffBall, Header, Juke, Parry, Pass,
        PressCommitBoxDesperation, PressCommitCover, PressCommitExposedBall, PressCommitHeavyTouch,
        PressCommitLowDiscipline, Reception, Shot, Tackle, TackleMiss, Tip, Touch, Volley,
    };
    Ok(match text {
        "shot" => Shot,
        "pass" => Pass,
        "touch" => Touch,
        "tackle" => Tackle,
        "tackle_miss" => TackleMiss,
        "press_commit_heavy_touch" => PressCommitHeavyTouch,
        "press_commit_exposed_ball" => PressCommitExposedBall,
        "press_commit_cover" => PressCommitCover,
        "press_commit_box_desperation" => PressCommitBoxDesperation,
        "press_commit_low_discipline" => PressCommitLowDiscipline,
        "combat_commit_carrier_contest" => CombatCommitCarrierContest,
        "combat_commit_carrier_protection" => CombatCommitCarrierProtection,
        "combat_commit_loose_ball_contest" => CombatCommitLooseBallContest,
        "combat_commit_passing_lane_or_shot_denial" => CombatCommitPassingLaneOrShotDenial,
        "combat_commit_recovery_punish" => CombatCommitRecoveryPunish,
        "combat_commit_unattributed_off_ball" => CombatCommitUnattributedOffBall,
        "catch" => Catch,
        "parry" => Parry,
        "tip" => Tip,
        "claim" => Claim,
        "block" => Block,
        "header" => Header,
        "volley" => Volley,
        "bicycle" => Bicycle,
        "reception" => Reception,
        "juke" => Juke,
        other => return Err(format!("'{other}' is not a canonical match event kind")),
    })
}

fn combat_event_kind_from_wire(text: &str) -> Result<CombatEventKind, String> {
    use CombatEventKind::{
        BallSpill, Cancelled, Commit, Contact, Forced, GuardRecoil, Interrupted, MatchTerminated,
        Miss, ProjectileExpire, ProjectileSpawn, RequestRejected,
    };
    Ok(match text {
        "request_rejected" => RequestRejected,
        "commit" => Commit,
        "projectile_spawn" => ProjectileSpawn,
        "projectile_expire" => ProjectileExpire,
        "contact" => Contact,
        "ball_spill" => BallSpill,
        "forced" => Forced,
        "guard_recoil" => GuardRecoil,
        "miss" => Miss,
        "interrupted" => Interrupted,
        "cancelled" => Cancelled,
        "match_terminated" => MatchTerminated,
        other => return Err(format!("'{other}' is not a canonical combat event kind")),
    })
}

fn combat_contact_result_from_wire(
    text: &str,
) -> Result<gc_sim::combat_snapshot::CombatContactResult, String> {
    use gc_sim::combat_snapshot::CombatContactResult::{
        Extended, Guarded, Hit, Immune, Superseded,
    };
    Ok(match text {
        "hit" => Hit,
        "extended" => Extended,
        "guarded" => Guarded,
        "immune" => Immune,
        "superseded" => Superseded,
        other => {
            return Err(format!(
                "'{other}' is not a canonical combat contact result"
            ));
        }
    })
}

fn combat_request_outcome_from_wire(
    text: &str,
) -> Result<gc_sim::combat_snapshot::CombatRequestOutcome, String> {
    use gc_sim::combat_snapshot::CombatRequestOutcome::{Accepted, Rejected};
    Ok(match text {
        "accepted" => Accepted,
        "rejected" => Rejected,
        other => {
            return Err(format!(
                "'{other}' is not a canonical combat request outcome"
            ));
        }
    })
}

fn combat_request_rejection_reason_from_wire(
    text: &str,
) -> Result<gc_sim::combat_snapshot::CombatRequestRejectionReason, String> {
    use gc_sim::combat_snapshot::CombatRequestRejectionReason::{
        AerialStateOrRecovery, AlreadyCommitted, Cooldown, ForcedState, KickoffHold,
        MalformedInput, MissingPressEdge, ProtectedKeeperOrNoLoadout, SoccerCommitment,
    };
    Ok(match text {
        "protected_keeper_or_no_loadout" => ProtectedKeeperOrNoLoadout,
        "kickoff_hold" => KickoffHold,
        "soccer_commitment" => SoccerCommitment,
        "aerial_state_or_recovery" => AerialStateOrRecovery,
        "forced_state" => ForcedState,
        "already_committed" => AlreadyCommitted,
        "cooldown" => Cooldown,
        "missing_press_edge" => MissingPressEdge,
        "malformed_input" => MalformedInput,
        other => {
            return Err(format!(
                "'{other}' is not a canonical combat request rejection reason"
            ));
        }
    })
}

fn combat_encounter_terminal_from_wire(
    text: &str,
) -> Result<gc_sim::combat_snapshot::CombatEncounterTerminal, String> {
    use gc_sim::combat_snapshot::CombatEncounterTerminal::{
        Cancelled, Expire, Guarded, Hit, Immune, Interrupted, MatchTerminated, Miss, Superseded,
    };
    Ok(match text {
        "miss" => Miss,
        "expire" => Expire,
        "guarded" => Guarded,
        "immune" => Immune,
        "superseded" => Superseded,
        "hit" => Hit,
        "interrupted" => Interrupted,
        "cancelled" => Cancelled,
        "match_terminated" => MatchTerminated,
        other => {
            return Err(format!(
                "'{other}' is not a canonical combat encounter terminal"
            ));
        }
    })
}

fn action_family_from_wire(text: &str) -> Result<ActionFamilyId, String> {
    match text {
        "unarmed" => Ok(ActionFamilyId::Unarmed),
        "guard" => Ok(ActionFamilyId::Guard),
        "light_melee" => Ok(ActionFamilyId::LightMelee),
        "ranged" => Ok(ActionFamilyId::Ranged),
        other => Err(format!("'{other}' is not a canonical action family id")),
    }
}

/// [`MatchEvent`] as plain, round-trippable JSON (field-for-field, no
/// `"origin"` tag — contrast [`match_event_to_json`]). `pub(crate)`, not
/// private: [`crate::match_state_bridge`]'s raw-`MatchState`-for-replay
/// surface embeds a `MatchState.events` array of exactly this shape (it is
/// also `replay.ts`'s own `MatchEvent` interface field-for-field), so this
/// is the one place that encoding happens rather than a second copy.
pub(crate) fn raw_match_event_to_json(event: &MatchEvent) -> Json {
    Json::obj(vec![
        ("kind", Json::str(event.kind.wire_str())),
        ("x", Json::Number(event.x)),
        ("y", Json::Number(event.y)),
        ("player", Json::opt_str(event.player.as_deref())),
        (
            "save_style",
            event
                .save_style
                .map_or(Json::Null, |v| Json::str(save_style_wire(v))),
        ),
        (
            "style",
            event
                .style
                .map_or(Json::Null, |v| Json::str(aerial_style_wire(v))),
        ),
        (
            "outcome",
            event
                .outcome
                .map_or(Json::Null, |v| Json::str(aerial_outcome_wire(v))),
        ),
        ("jumping", event.jumping.map_or(Json::Null, Json::Bool)),
        (
            "difficulty",
            event.difficulty.map_or(Json::Null, Json::Number),
        ),
        (
            "shot_type",
            event
                .shot_type
                .map_or(Json::Null, |v| Json::str(keeper_shot_type_wire(v))),
        ),
        (
            "keeper_state",
            event
                .keeper_state
                .map_or(Json::Null, |v| Json::str(keeper_behavior_state_wire(v))),
        ),
        (
            "keeper_depth",
            event.keeper_depth.map_or(Json::Null, Json::Number),
        ),
        ("on_target", event.on_target.map_or(Json::Null, Json::Bool)),
    ])
}

/// `pub(crate)`, not private: [`crate::match_snapshot_bridge`]'s `events`
/// override needs this exact decode too — the round-trippable counterpart of
/// [`raw_match_event_to_json`], which [`crate::match_state_bridge`] already
/// reuses on the encode side.
pub(crate) fn raw_match_event_from_json(json: &Json) -> Result<MatchEvent, String> {
    let kind =
        match_event_kind_from_wire(json.field_str("kind").ok_or("match event missing 'kind'")?)?;
    let save_style = json
        .field_str("save_style")
        .map(save_style_from_wire)
        .transpose()?;
    let style = json
        .field_str("style")
        .map(aerial_style_from_wire)
        .transpose()?;
    let outcome = json
        .field_str("outcome")
        .map(aerial_outcome_from_wire)
        .transpose()?;
    let shot_type = json
        .field_str("shot_type")
        .map(keeper_shot_type_from_wire)
        .transpose()?;
    let keeper_state = json
        .field_str("keeper_state")
        .map(keeper_behavior_state_from_wire)
        .transpose()?;
    Ok(MatchEvent {
        kind,
        x: json
            .field("x")
            .and_then(Json::as_f64)
            .ok_or("match event missing 'x'")?,
        y: json
            .field("y")
            .and_then(Json::as_f64)
            .ok_or("match event missing 'y'")?,
        player: json.field_str("player").map(str::to_string),
        save_style,
        style,
        outcome,
        jumping: json.field_bool("jumping"),
        difficulty: json.field("difficulty").and_then(Json::as_f64),
        shot_type,
        keeper_state,
        keeper_depth: json.field("keeper_depth").and_then(Json::as_f64),
        on_target: json.field_bool("on_target"),
    })
}

/// [`CombatEvent`] as plain, round-trippable JSON (field-for-field, no
/// `"origin"` tag — contrast [`combat_event_to_json`]). `pub(crate)`, not
/// private: [`crate::session::Session::combat_events_json`] reuses this
/// directly for the BASE (non-rollback) per-tick combat event surface,
/// rather than inventing a second combat-event wire shape (the same reason
/// [`raw_combat_event_from_json`] below is already `pub(crate)`).
pub(crate) fn raw_combat_event_to_json(event: &CombatEvent) -> Json {
    Json::obj(vec![
        ("kind", Json::str(event.kind.wire_str())),
        ("tick", Json::int(event.tick)),
        (
            "family_id",
            event.family_id.map_or(Json::Null, |id| {
                Json::str(gc_sim::combat_snapshot::action_family_wire_id(id))
            }),
        ),
        ("source_index", Json::opt_int(event.source_index)),
        ("target_index", Json::opt_int(event.target_index)),
        ("source_sequence", Json::opt_int(event.source_sequence)),
        (
            "result",
            event.result.map_or(Json::Null, |v| Json::str(v.wire_str())),
        ),
        (
            "outcome",
            event
                .outcome
                .map_or(Json::Null, |v| Json::str(v.wire_str())),
        ),
        (
            "reason",
            event.reason.map_or(Json::Null, |v| Json::str(v.wire_str())),
        ),
        (
            "terminal",
            event
                .terminal
                .map_or(Json::Null, |v| Json::str(v.wire_str())),
        ),
        ("x", Json::Number(event.x)),
        ("y", Json::Number(event.y)),
        (
            "interruption_ticks",
            Json::opt_int(event.interruption_ticks),
        ),
        (
            "displacement_px",
            event.displacement_px.map_or(Json::Null, Json::Number),
        ),
    ])
}

/// `pub(crate)`, not private: [`crate::online_combat_phases_bridge`]'s
/// `onlineCombatPhaseObserved` decodes the same `combat_events` shape
/// [`crate::match_driver_bridge`]'s `advance` batch already embeds per
/// output (see [`tick_output_to_json`]'s doc), so a caller can hand that
/// exact array straight back in rather than this crate inventing a second
/// combat-event wire shape.
pub(crate) fn raw_combat_event_from_json(json: &Json) -> Result<CombatEvent, String> {
    let kind = combat_event_kind_from_wire(
        json.field_str("kind")
            .ok_or("combat event missing 'kind'")?,
    )?;
    let family_id = json
        .field_str("family_id")
        .map(action_family_from_wire)
        .transpose()?;
    let result = json
        .field_str("result")
        .map(combat_contact_result_from_wire)
        .transpose()?;
    let outcome = json
        .field_str("outcome")
        .map(combat_request_outcome_from_wire)
        .transpose()?;
    let reason = json
        .field_str("reason")
        .map(combat_request_rejection_reason_from_wire)
        .transpose()?;
    let terminal = json
        .field_str("terminal")
        .map(combat_encounter_terminal_from_wire)
        .transpose()?;
    Ok(CombatEvent {
        kind,
        tick: json
            .field_i64("tick")
            .ok_or("combat event missing 'tick'")?,
        family_id,
        source_index: json.field_i64("source_index"),
        target_index: json.field_i64("target_index"),
        source_sequence: json.field_i64("source_sequence"),
        result,
        outcome,
        reason,
        terminal,
        x: json
            .field("x")
            .and_then(Json::as_f64)
            .ok_or("combat event missing 'x'")?,
        y: json
            .field("y")
            .and_then(Json::as_f64)
            .ok_or("combat event missing 'y'")?,
        interruption_ticks: json.field_i64("interruption_ticks"),
        displacement_px: json.field("displacement_px").and_then(Json::as_f64),
    })
}

fn state_view_plain_from_json(json: &Json) -> Result<RollbackOutputStateView, String> {
    Ok(RollbackOutputStateView {
        score: ByTeam {
            home: json.field_i64("home").ok_or("score missing 'home'")?,
            away: json.field_i64("away").ok_or("score missing 'away'")?,
        },
        // `time_left`/`finished` are read from the enclosing tick output's
        // own top-level fields by `tick_output_from_json` — see that
        // function: they are carried once, not duplicated under `score`.
        time_left: 0.0,
        finished: false,
    })
}

/// A [`RollbackEventTickOutput`], as plain, round-trippable JSON. This is
/// the SAME shape [`crate::match_driver_bridge`]'s `advance` embeds per
/// output (see that module's `output_summary_to_json`) — one encoding
/// serves both "read this in JS" and "hand it back to
/// [`RollbackEventsTimeline::apply`]".
#[must_use]
pub(crate) fn tick_output_to_json(output: &RollbackEventTickOutput) -> Json {
    Json::obj(vec![
        ("tick", Json::int(output.tick)),
        ("start_boundary", Json::int(output.start_boundary)),
        ("end_boundary", Json::int(output.end_boundary)),
        ("finished", Json::bool(output.finished)),
        (
            "score",
            Json::obj(vec![
                ("home", Json::int(output.state.score.home)),
                ("away", Json::int(output.state.score.away)),
            ]),
        ),
        ("time_left", Json::Number(output.state.time_left)),
        (
            "events",
            Json::Array(output.events.iter().map(raw_match_event_to_json).collect()),
        ),
        (
            "combat_events",
            output.combat_events.as_ref().map_or(Json::Null, |events| {
                Json::Array(events.iter().map(raw_combat_event_to_json).collect())
            }),
        ),
    ])
}

pub(crate) fn tick_output_from_json(json: &Json) -> Result<RollbackEventTickOutput, String> {
    let events_field = json.get("events").and_then(Json::as_array).unwrap_or(&[]);
    let mut events = Vec::with_capacity(events_field.len());
    for item in events_field {
        events.push(raw_match_event_from_json(item)?);
    }
    let combat_events = match json.field("combat_events").and_then(Json::as_array) {
        Some(items) => {
            let mut combat_events = Vec::with_capacity(items.len());
            for item in items {
                combat_events.push(raw_combat_event_from_json(item)?);
            }
            Some(combat_events)
        }
        None => None,
    };
    let mut state =
        state_view_plain_from_json(json.get("score").ok_or("tick output missing 'score'")?)?;
    state.time_left = json
        .field("time_left")
        .and_then(Json::as_f64)
        .ok_or("tick output missing 'time_left'")?;
    let finished = json
        .field_bool("finished")
        .ok_or("tick output missing 'finished'")?;
    state.finished = finished;
    Ok(RollbackEventTickOutput {
        tick: json.field_i64("tick").ok_or("tick output missing 'tick'")?,
        start_boundary: json
            .field_i64("start_boundary")
            .ok_or("tick output missing 'start_boundary'")?,
        end_boundary: json
            .field_i64("end_boundary")
            .ok_or("tick output missing 'end_boundary'")?,
        events,
        combat_events,
        state,
        finished,
    })
}

// ---------------------------------------------------------------------
// The `#[wasm_bindgen]` surface: an opaque `MatchSnapshot` handle, and
// `RollbackEventsTimeline` (create/apply/confirm/diagnostics as separate
// callables). See the module doc.
// ---------------------------------------------------------------------

/// An opaque handle over a [`MatchSnapshot`]. Never inspected on the JS
/// side — see the module doc. Obtained from
/// [`crate::session::Session::snapshot_handle`] or
/// [`crate::match_driver_bridge::MatchDriverBridge::snapshot_lookup`]; only
/// ever passed back into [`RollbackEventsTimeline::create`]/
/// [`RollbackEventsTimeline::apply`].
#[wasm_bindgen]
#[derive(Clone)]
pub struct WasmMatchSnapshot {
    pub(crate) snapshot: MatchSnapshot,
}

impl WasmMatchSnapshot {
    /// Wraps a real snapshot. Not exposed to JS — every JS-visible source of
    /// a [`WasmMatchSnapshot`] is a dedicated method elsewhere in this crate.
    pub(crate) fn new(snapshot: MatchSnapshot) -> WasmMatchSnapshot {
        WasmMatchSnapshot { snapshot }
    }
}

fn apply_result_json(result: rollback_events::Result<RollbackEventDiff>) -> String {
    match result {
        Ok(diff) => Json::obj(vec![
            ("ok", Json::bool(true)),
            ("value", diff_to_json(&diff)),
        ])
        .to_json_string(),
        Err(err) => {
            let code = match err.code {
                RollbackEventsErrorCode::UnconfirmedWindowExceeded => "unconfirmed_window_exceeded",
            };
            Json::obj(vec![
                ("ok", Json::bool(false)),
                (
                    "error",
                    Json::obj(vec![
                        ("message", Json::str(err.message)),
                        ("code", Json::str(code)),
                    ]),
                ),
            ])
            .to_json_string()
        }
    }
}

/// `wasm-bindgen` control surface over [`gc_sim::rollback_events`]:
/// `create`/`apply`/`confirm`/`diagnosticsJson` as separate callables,
/// satisfying `packages/online/src/match_presentation.ts`'s
/// `RollbackEventsPort<TTimeline, TSnapshot>` (`TTimeline` =
/// `RollbackEventsTimeline`, `TSnapshot` = [`WasmMatchSnapshot`]).
///
/// `rollback_events::apply`'s [`RollbackEventsErrorCode::UnconfirmedWindowExceeded`]
/// is an expected, recoverable failure (ARCHITECTURE.md §3 rule 5) — [`Self::apply`]
/// reports it as JSON data (`{"ok": false, "error": {...}}`, mirroring
/// `RollbackApplyResult`'s discriminated union), never a thrown `JsValue`.
/// Everything else `rollback_events::apply`/`confirm` guard with `assert!`
/// is a programmer error (a malformed replaced interval, a gap in the
/// retained window, confirming past what was applied) and panics, exactly
/// as it does natively — a caller driving this class is expected to satisfy
/// those preconditions the same way `match_presentation.ts`'s `consume`
/// already does.
#[wasm_bindgen]
pub struct RollbackEventsTimeline {
    inner: RollbackEventTimeline,
}

#[wasm_bindgen]
impl RollbackEventsTimeline {
    /// Mirrors [`gc_sim::rollback_events::new`]; named `create` because
    /// `new` is a reserved word in every `wasm-bindgen`-generated class
    /// (matches `RollbackEventsPort.create`'s own doc comment).
    /// `max_unconfirmed_ticks` defaults the same way the Rust function does
    /// when omitted.
    #[wasm_bindgen(js_name = create)]
    #[must_use]
    pub fn create(
        initial_snapshot: &WasmMatchSnapshot,
        max_unconfirmed_ticks: Option<f64>,
    ) -> RollbackEventsTimeline {
        RollbackEventsTimeline {
            inner: rollback_events::new(
                &initial_snapshot.snapshot,
                max_unconfirmed_ticks.map(|value| value as i64),
            ),
        }
    }

    /// Mirrors [`gc_sim::rollback_events::apply`]. `outputs_json` is a JSON
    /// array of [`tick_output_to_json`]'s shape (exactly what
    /// [`crate::match_driver_bridge::MatchDriverBridge::advance`] embeds per
    /// output), `snapshots` the parallel array of boundary snapshots (same
    /// length, same order — one per `outputs_json` entry, each the
    /// canonical post-step snapshot at that output's `end_boundary`).
    /// Returns `RollbackApplyResult` as JSON: `{"ok": true, "value":
    /// <RollbackEventDiff>}` or `{"ok": false, "error": {"message", "code"}}`.
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if `outputs_json` fails to parse, is
    /// not an array, does not have the same length as `snapshots`, or an
    /// entry fails to decode as a [`RollbackEventTickOutput`]. A structural
    /// failure of `rollback_events::apply` itself (the unconfirmed window)
    /// is not an error here — see this method's doc.
    #[wasm_bindgen(js_name = apply)]
    pub fn apply(
        &mut self,
        replaced_from_tick: f64,
        replaced_through_tick: f64,
        outputs_json: &str,
        snapshots: Vec<WasmMatchSnapshot>,
    ) -> Result<String, JsValue> {
        let parsed = Json::parse(outputs_json).map_err(js_err)?;
        let items = parsed
            .as_array()
            .ok_or_else(|| js_err("apply outputs_json must be a JSON array"))?;
        if items.len() != snapshots.len() {
            return Err(js_err(format!(
                "apply outputs_json has {} entries but {} snapshots were given",
                items.len(),
                snapshots.len()
            )));
        }
        let mut steps = Vec::with_capacity(items.len());
        for (item, snapshot) in items.iter().zip(snapshots) {
            let output = tick_output_from_json(item).map_err(js_err)?;
            steps.push(RollbackEventStepInput {
                output,
                snapshot: snapshot.snapshot,
            });
        }
        let result = rollback_events::apply(
            &mut self.inner,
            replaced_from_tick as i64,
            replaced_through_tick as i64,
            &steps,
        );
        Ok(apply_result_json(result))
    }

    /// Mirrors [`gc_sim::rollback_events::confirm`]. Returns the confirmed
    /// steps ([`RollbackEventStep`]) as a JSON array, in causal order —
    /// [`step_to_json`]'s shape.
    #[wasm_bindgen(js_name = confirm)]
    #[must_use]
    pub fn confirm(&mut self, confirmed_output_tick: f64) -> String {
        let steps = rollback_events::confirm(&mut self.inner, confirmed_output_tick as i64);
        Json::Array(steps.iter().map(step_to_json).collect()).to_json_string()
    }

    /// Mirrors [`gc_sim::rollback_events::diagnostics`], as JSON
    /// ([`diagnostics_to_json`]'s shape).
    #[wasm_bindgen(js_name = diagnosticsJson)]
    #[must_use]
    pub fn diagnostics_json(&self) -> String {
        diagnostics_to_json(&rollback_events::diagnostics(&self.inner)).to_json_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gc_sim::fixed_clock;
    use gc_sim::input_frame;
    use gc_sim::r#match as sim_match;
    use gc_sim::match_snapshot::{self, PitchSize};
    use gc_sim::tuning::Tuning;

    // Slot mode, not the legacy path: `sim_match::step` only advances
    // `input_tick` in slot mode (`gc_sim::r#match::step`'s own doc/source),
    // and `rollback_events::apply`'s `assert_step_coherent` requires the
    // restored snapshot's `input_tick` to equal the output's
    // `end_boundary`. `crate::session::ownership` is the same slot
    // assignment `Session::new` itself builds for these two fixture teams.
    fn boundary_zero_snapshot() -> MatchSnapshot {
        let home = gc_data::teams::get("nebula").expect("fixture team");
        let away = gc_data::teams::get("orion").expect("fixture team");
        let home_roster: [&str; 5] = home.roster.try_into().expect("fixture roster is five");
        let away_roster: [&str; 5] = away.roster.try_into().expect("fixture roster is five");
        let state = sim_match::new(sim_match::NewMatchOptions {
            home,
            away,
            field: PitchSize { w: 960.0, h: 540.0 },
            home_formation: None,
            tactic: None,
            away_tactic: None,
            duration: Some(20.0),
            max_goals: Some(3),
            seed: Some(7.0),
            players_by_id: None,
            species_by_id: None,
            showcase_players_by_id: None,
            human_controlled: None,
            input_ownership: Some(crate::session::ownership(&home_roster, &away_roster)),
        });
        match_snapshot::capture(&state, None)
    }

    /// Steps `state` (currently at `state.input_tick == tick`) forward one
    /// neutral slot-mode tick.
    fn step_once(
        mut state: gc_sim::match_snapshot::MatchState,
        tick: i64,
    ) -> gc_sim::match_snapshot::MatchState {
        let tune = Tuning::new();
        let frame = input_frame::neutral(tick).expect("a neutral frame is always valid");
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            sim_match::StepInput::Frame(&frame),
            None,
            &tune,
        );
        state
    }

    #[test]
    fn tick_output_round_trips_through_json() {
        let snapshot = boundary_zero_snapshot();
        let (state, _combat) = match_snapshot::restore(&snapshot);
        let stepped = step_once(state, 0);
        let output = RollbackEventTickOutput {
            tick: 0,
            start_boundary: 0,
            end_boundary: 1,
            events: stepped.events.clone(),
            combat_events: None,
            state: RollbackOutputStateView {
                score: stepped.score,
                time_left: stepped.time_left,
                finished: stepped.finished,
            },
            finished: stepped.finished,
        };
        let json = tick_output_to_json(&output).to_json_string();
        let parsed = Json::parse(&json).expect("valid JSON");
        let decoded = tick_output_from_json(&parsed).expect("decodes back");
        assert_eq!(decoded, output);
    }

    #[test]
    fn create_then_apply_then_confirm_round_trips_one_normal_step() {
        let snapshot = boundary_zero_snapshot();
        let mut timeline =
            RollbackEventsTimeline::create(&WasmMatchSnapshot::new(snapshot.clone()), None);

        let (state, _combat) = match_snapshot::restore(&snapshot);
        let stepped = step_once(state, 0);
        let after = match_snapshot::capture(&stepped, None);
        let output = RollbackEventTickOutput {
            tick: 0,
            start_boundary: 0,
            end_boundary: 1,
            events: stepped.events.clone(),
            combat_events: None,
            state: RollbackOutputStateView {
                score: stepped.score,
                time_left: stepped.time_left,
                finished: stepped.finished,
            },
            finished: stepped.finished,
        };
        let outputs_json = Json::Array(vec![tick_output_to_json(&output)]).to_json_string();
        let result_json = timeline
            .apply(0.0, 0.0, &outputs_json, vec![WasmMatchSnapshot::new(after)])
            .expect("well-formed JSON/snapshot pairing applies");
        let parsed = Json::parse(&result_json).expect("valid JSON");
        assert_eq!(parsed.field_bool("ok"), Some(true));

        let diagnostics = Json::parse(&timeline.diagnostics_json()).expect("valid JSON");
        assert_eq!(diagnostics.field_i64("retained_step_count"), Some(1));

        let confirmed_json = timeline.confirm(0.0);
        let confirmed = Json::parse(&confirmed_json).expect("valid JSON");
        assert_eq!(confirmed.as_array().map(<[Json]>::len), Some(1));

        let diagnostics = Json::parse(&timeline.diagnostics_json()).expect("valid JSON");
        assert_eq!(diagnostics.field_i64("confirmed_tick"), Some(0));
        assert_eq!(diagnostics.field_i64("retained_step_count"), Some(0));
    }

    /// The correcting-call path — `replaced_from_tick == replaced_through_tick`
    /// naming an already-retained tick, per `rollback_events::apply`'s own
    /// contract for that case — is what
    /// `MatchDriverBridge::feed_rollback_timeline`'s correction-batch
    /// splitting (this wave's item 2) actually calls when a driver batch
    /// carries a correction. This proves the wasm binding's plumbing for
    /// that path (not just the ordinary append path above): retained tick 0
    /// is replaced in place with a different post-step outcome, and the
    /// returned diff reflects exactly that difference.
    #[test]
    fn apply_replaces_an_already_retained_tick_on_a_correction() {
        let snapshot = boundary_zero_snapshot();
        let mut timeline =
            RollbackEventsTimeline::create(&WasmMatchSnapshot::new(snapshot.clone()), None);

        let (state, _combat) = match_snapshot::restore(&snapshot);
        let stepped = step_once(state, 0);
        let after = match_snapshot::capture(&stepped, None);
        let output = RollbackEventTickOutput {
            tick: 0,
            start_boundary: 0,
            end_boundary: 1,
            events: stepped.events.clone(),
            combat_events: None,
            state: RollbackOutputStateView {
                score: stepped.score,
                time_left: stepped.time_left,
                finished: stepped.finished,
            },
            finished: stepped.finished,
        };
        let outputs_json = Json::Array(vec![tick_output_to_json(&output)]).to_json_string();
        timeline
            .apply(0.0, 0.0, &outputs_json, vec![WasmMatchSnapshot::new(after)])
            .expect("the first, ordinary append applies");

        // A resimulated tick 0 that scored a home goal -- a genuinely
        // different outcome, not a repeat of the same one, so the
        // correction's diff has something real to report.
        let mut corrected_state = stepped.clone();
        corrected_state.score.home += 1;
        let corrected_snapshot = match_snapshot::capture(&corrected_state, None);
        let corrected_output = RollbackEventTickOutput {
            tick: 0,
            start_boundary: 0,
            end_boundary: 1,
            events: corrected_state.events.clone(),
            combat_events: None,
            state: RollbackOutputStateView {
                score: corrected_state.score,
                time_left: corrected_state.time_left,
                finished: corrected_state.finished,
            },
            finished: corrected_state.finished,
        };
        let corrected_outputs_json =
            Json::Array(vec![tick_output_to_json(&corrected_output)]).to_json_string();
        let result_json = timeline
            .apply(
                0.0,
                0.0,
                &corrected_outputs_json,
                vec![WasmMatchSnapshot::new(corrected_snapshot)],
            )
            .expect("a correction over an already-retained tick applies");
        let parsed = Json::parse(&result_json).expect("valid JSON");
        assert_eq!(parsed.field_bool("ok"), Some(true));

        // The lifecycle payload is derived from the score delta
        // (`gc_sim::rollback_events::derive_lifecycle_events`), so the
        // corrected step has a `lifecycle/goal` event the original step
        // never had -- an addition, not a same-identity replacement.
        let diff = parsed.get("value").expect("an ok result carries a diff");
        let added = diff
            .get("added")
            .and_then(Json::as_array)
            .expect("diff has an 'added' array");
        assert!(
            added
                .iter()
                .any(|event| event.get("domain").and_then(Json::as_str) == Some("lifecycle/goal")),
            "the corrected home goal should surface as an added lifecycle event"
        );

        // A correction replaces in place -- it does not grow the retained
        // window.
        let diagnostics = Json::parse(&timeline.diagnostics_json()).expect("valid JSON");
        assert_eq!(diagnostics.field_i64("retained_step_count"), Some(1));
    }

    #[test]
    fn apply_reports_the_unconfirmed_window_as_data_not_a_thrown_error() {
        let snapshot = boundary_zero_snapshot();
        let mut timeline =
            RollbackEventsTimeline::create(&WasmMatchSnapshot::new(snapshot.clone()), Some(1.0));

        let mut state = match_snapshot::restore(&snapshot).0;
        for tick in 0..3i64 {
            let stepped = step_once(state.clone(), tick);
            let after = match_snapshot::capture(&stepped, None);
            let output = RollbackEventTickOutput {
                tick,
                start_boundary: tick,
                end_boundary: tick + 1,
                events: stepped.events.clone(),
                combat_events: None,
                state: RollbackOutputStateView {
                    score: stepped.score,
                    time_left: stepped.time_left,
                    finished: stepped.finished,
                },
                finished: stepped.finished,
            };
            let outputs_json = Json::Array(vec![tick_output_to_json(&output)]).to_json_string();
            let result_json = timeline
                .apply(
                    tick as f64,
                    tick as f64,
                    &outputs_json,
                    vec![WasmMatchSnapshot::new(after)],
                )
                .expect("well-formed JSON/snapshot pairing applies");
            let parsed = Json::parse(&result_json).expect("valid JSON");
            if tick == 0 {
                // The first append: one retained step against a
                // one-tick unconfirmed window — still inside it.
                assert_eq!(
                    parsed.field_bool("ok"),
                    Some(true),
                    "tick {tick} should apply cleanly"
                );
            } else {
                // A second unconfirmed append would retain two steps
                // against a one-tick window: `apply` reports this as data
                // (never a thrown error), and the timeline stalls for good.
                assert_eq!(parsed.field_bool("ok"), Some(false));
                assert_eq!(
                    parsed
                        .get("error")
                        .and_then(|error| error.field_str("code")),
                    Some("unconfirmed_window_exceeded")
                );
            }
            state = stepped;
        }
    }
}
