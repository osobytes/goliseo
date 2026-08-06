//! JSON conversion for `gc_sim::rollback_events` — the presentation-facing
//! stable-event timeline a match driver's tick outputs feed.
//!
//! ## Why this crosses as a diff, never a snapshot
//!
//! [`gc_sim::rollback_events::apply`] needs a real
//! [`gc_sim::match_snapshot::MatchSnapshot`] per step
//! (`RollbackEventStepInput::snapshot`). This crate deliberately never
//! serializes a `MatchSnapshot` to JSON anywhere (see
//! [`crate::match_driver_bridge`]'s doc): it is a large, binary-shaped
//! structure with an existing canonical wire encoding
//! (`gc_sim::match_snapshot::encode`) built for hashing and replication, not
//! for a JSON round trip a JS caller would hand-edit. So the timeline this
//! module bridges is driven entirely on the Rust side, inside
//! [`crate::match_driver_bridge::MatchDriverBridge::advance`] — same
//! process, same call, a real `MatchSnapshot` never leaving Rust — and only
//! this module's output, [`diff_to_json`] (added/revoked/replaced
//! presentation events), crosses to JS. That is exactly the data a
//! presentation layer needs (what changed this step) and nothing a
//! presentation layer has any business mutating.
//!
//! This module therefore has no `#[wasm_bindgen]` surface of its own; it is
//! conversion helpers `match_driver_bridge` uses to build its own JSON.

use gc_sim::combat_snapshot::{CombatEvent, CombatEventKind};
use gc_sim::match_snapshot::{MatchEvent, MatchEventKind, Team};
use gc_sim::rollback_events::{
    RollbackConfirmedStateView, RollbackEventDiff, RollbackEventPayload, RollbackEventStep,
    RollbackEventsAccounting, RollbackEventsDiagnostics, RollbackLifecycleKind,
    RollbackLifecyclePayload, RollbackWrappedEvent,
};

use crate::json::{Json, debug_tag};

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
