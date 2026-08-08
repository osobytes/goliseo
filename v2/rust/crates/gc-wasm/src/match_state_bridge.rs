//! JSON conversion for the RAW `gc_sim::match_snapshot::MatchState` slice
//! `@gc/render`'s `replay.ts` needs to capture — not `crates/gc-render`'s
//! presentation-derived `RenderFrame`.
//!
//! ## The gap this closes
//!
//! `crate::session::Session`/`crate::render_export` only ever hand JS a
//! `RenderFrame` (`gc_render::frame::build`'s structure-of-arrays, eased
//! timers, no `outfield_press`/`transition`/`outfield_decision` at all —
//! `frame.rs`'s own doc explains why: it is built for the renderer, not for
//! reconstructing raw sim state). `packages/render/src/replay.ts`'s
//! `captureFrame`/`recordBoundary` need the OTHER thing:
//! `gc_sim::match_snapshot::MatchState` itself, field-for-field, so a goal
//! replay can play back the actual poses (dive/grab/throw/windup timers,
//! `outfield_decision`, team-owned `outfield_press`/`transition`) a
//! presentation frame already eased away. `match_screen.spec.ts`'s "goal
//! replay" skip and `match_rollback_lab.spec.ts`'s two ScreenStack-tier
//! skips (`packages/app/src/match_rollback_lab.spec.ts`) both name this
//! exact gap.
//!
//! [`match_state_to_json`] is deliberately narrower than the full
//! `MatchState`/`MatchPlayer` structs (70+ fields on `MatchPlayer` alone —
//! see that struct's doc in `crates/gc-sim/src/match_snapshot.rs`): it
//! encodes exactly the slice `replay.ts`'s own `MatchState`/`MatchPlayer`
//! TypeScript interfaces declare (the fields `captureFrame` actually reads),
//! confirmed against that file directly rather than guessed. Widening this
//! encoder to the full struct is a straightforward follow-up if a future
//! consumer needs more of it; nothing here forecloses that, but nothing
//! this milestone's skipped tests need asks for it either.
//!
//! ## Why JSON, not the raw-block ABI `render_export` uses
//!
//! `crate::render_export`'s module doc explains why the *presentation*
//! `RenderFrame` crosses as a raw `f64` block: it is homogeneous (every
//! field is numeric) and built every rendered frame, including up to eight
//! resimulated ticks folded into one. This slice is neither: it carries
//! enums, optional fields, and nested per-team/per-player records, and (in
//! the base, non-rollback case) is only needed once per fixed tick a
//! `replay`-consuming caller actually wants recorded — the same call
//! frequency `crate::rollback_events_bridge`/`crate::match_driver_bridge`
//! already serve as hand-rolled JSON rather than raw exports. A raw
//! flat-block encoding for this shape is a plausible future optimization if
//! per-tick replay capture turns out to be a measured hot path; it is not
//! attempted here, matching this crate's own precedent of flagging such
//! follow-up rather than pre-optimizing a shape no profiling has asked for.
//!
//! ## Enum wire strings: reusing, and one more local mirror
//!
//! `Team::wire_str`/`MatchEventKind::wire_str` are `pub` on `gc_sim` and
//! reused directly. `SaveStyle`/`AerialStyle`/`AerialOutcome`/
//! `KeeperBehaviorState` are not (see `crate::rollback_events_bridge`'s
//! module doc for why) — that module already carries the local mirror this
//! one needs, now `pub(crate)` so this module reuses the exact same mapping
//! rather than a third copy. `OutfieldDecisionContext`/`OutfieldIntent`/
//! `outfield_press::StablePressMode`/`brain::PressReason`/
//! `possession_transition::TransitionTeam` have no `pub wire_str` anywhere
//! reachable from this crate either — `gc_sim::match_snapshot`'s own
//! canonical wire encoder keeps a *private* local mirror of exactly these
//! five for the identical reason (confirmed reading that file: `fn
//! outfield_decision_context_wire`/`outfield_intent_wire`/
//! `stable_press_mode_wire`/`press_reason_wire`/`transition_team_wire`, all
//! private to that module). This module keeps its own small local mirror of
//! the same five, string-for-string identical to `match_snapshot.rs`'s
//! (and therefore to `replay.ts`'s own TypeScript union types), rather than
//! widen `gc-sim`'s public surface for glue this crate does not own.

use gc_data::tactics::TransitionConfig;
use gc_sim::brain::PressReason;
use gc_sim::match_snapshot::{MatchPlayer, MatchState, PitchSize, Rect, Team};
use gc_sim::outfield_decision::{OutfieldDecisionContext, OutfieldDecisionState, OutfieldIntent};
use gc_sim::outfield_press::{OutfieldPressState, StablePressMode};
use gc_sim::possession_transition::{PossessionTransitionState, TransitionTeam, TransitionWindows};

use crate::json::Json;
use crate::rollback_events_bridge::{
    aerial_outcome_wire, aerial_style_wire, keeper_behavior_state_wire, raw_match_event_to_json,
    save_style_wire,
};

fn outfield_decision_context_wire(v: OutfieldDecisionContext) -> &'static str {
    match v {
        OutfieldDecisionContext::Ineligible => "ineligible",
        OutfieldDecisionContext::Offball => "offball",
        OutfieldDecisionContext::Carrier => "carrier",
    }
}

fn outfield_intent_wire(v: OutfieldIntent) -> &'static str {
    match v {
        OutfieldIntent::None => "none",
        OutfieldIntent::Move => "move",
        OutfieldIntent::InBehind => "in_behind",
        OutfieldIntent::ComeShort => "come_short",
        OutfieldIntent::HoldWidth => "hold_width",
        OutfieldIntent::Shoot => "shoot",
        OutfieldIntent::Cross => "cross",
        OutfieldIntent::Pass => "pass",
        OutfieldIntent::Dribble => "dribble",
    }
}

/// `pub(crate)`, not private: [`crate::player_pose_bridge`] needs this exact
/// wire mapping to decode a caller-supplied `MatchPlayer.outfield_decision`
/// for standalone pose selection (`player_pose::select`'s `run_telegraphing`
/// helper reads `outfield_decision.intent`) — reusing it here keeps exactly
/// one `OutfieldIntent` wire mapping in this crate rather than a second copy.
pub(crate) fn outfield_intent_from_wire(text: &str) -> Result<OutfieldIntent, String> {
    match text {
        "none" => Ok(OutfieldIntent::None),
        "move" => Ok(OutfieldIntent::Move),
        "in_behind" => Ok(OutfieldIntent::InBehind),
        "come_short" => Ok(OutfieldIntent::ComeShort),
        "hold_width" => Ok(OutfieldIntent::HoldWidth),
        "shoot" => Ok(OutfieldIntent::Shoot),
        "cross" => Ok(OutfieldIntent::Cross),
        "pass" => Ok(OutfieldIntent::Pass),
        "dribble" => Ok(OutfieldIntent::Dribble),
        other => Err(format!("'{other}' is not a canonical outfield intent")),
    }
}

/// `pub(crate)`: see [`outfield_intent_from_wire`]'s doc —
/// [`crate::player_pose_bridge`] reuses this one too (for round-trip
/// fidelity; `player_pose::select` itself never reads
/// `outfield_decision.context`).
pub(crate) fn outfield_decision_context_from_wire(
    text: &str,
) -> Result<OutfieldDecisionContext, String> {
    match text {
        "ineligible" => Ok(OutfieldDecisionContext::Ineligible),
        "offball" => Ok(OutfieldDecisionContext::Offball),
        "carrier" => Ok(OutfieldDecisionContext::Carrier),
        other => Err(format!(
            "'{other}' is not a canonical outfield decision context"
        )),
    }
}

fn stable_press_mode_wire(v: StablePressMode) -> &'static str {
    match v {
        StablePressMode::Inactive => "inactive",
        StablePressMode::Contain => "contain",
        StablePressMode::Commit => "commit",
    }
}

fn press_reason_wire(v: PressReason) -> &'static str {
    match v {
        PressReason::HeavyTouch => "heavy_touch",
        PressReason::ExposedBall => "exposed_ball",
        PressReason::Cover => "cover",
        PressReason::BoxDesperation => "box_desperation",
        PressReason::LowDiscipline => "low_discipline",
        PressReason::NoTrigger => "no_trigger",
    }
}

fn transition_team_wire(v: TransitionTeam) -> &'static str {
    match v {
        TransitionTeam::Home => "home",
        TransitionTeam::Away => "away",
    }
}

fn vec2_json(x: f64, y: f64) -> Json {
    Json::obj(vec![("x", Json::Number(x)), ("y", Json::Number(y))])
}

fn pitch_size_json(field: PitchSize) -> Json {
    Json::obj(vec![
        ("w", Json::Number(field.w)),
        ("h", Json::Number(field.h)),
    ])
}

fn rect_json(rect: Rect) -> Json {
    Json::obj(vec![
        ("x", Json::Number(rect.x)),
        ("y", Json::Number(rect.y)),
        ("w", Json::Number(rect.w)),
        ("h", Json::Number(rect.h)),
    ])
}

fn outfield_decision_json(d: &OutfieldDecisionState) -> Json {
    Json::obj_omit_null(vec![
        ("version", Json::int(i64::from(d.version))),
        ("generation", Json::int(i64::from(d.generation))),
        ("rng_state", Json::int(i64::from(d.rng_state))),
        ("remaining", Json::Number(d.remaining)),
        (
            "context",
            Json::str(outfield_decision_context_wire(d.context)),
        ),
        ("intent", Json::str(outfield_intent_wire(d.intent))),
        ("target_x", d.target_x.map_or(Json::Null, Json::Number)),
        ("target_y", d.target_y.map_or(Json::Null, Json::Number)),
        (
            "target_player",
            d.target_player
                .map_or(Json::Null, |v| Json::int(i64::from(v))),
        ),
        (
            "run_expires_at",
            d.run_expires_at.map_or(Json::Null, Json::Number),
        ),
    ])
}

fn outfield_press_json(p: &OutfieldPressState) -> Json {
    Json::obj_omit_null(vec![
        ("version", Json::int(i64::from(p.version))),
        (
            "presser_index",
            p.presser_index
                .map_or(Json::Null, |v| Json::int(i64::from(v))),
        ),
        ("mode", Json::str(stable_press_mode_wire(p.mode))),
        ("reason", Json::str(press_reason_wire(p.reason))),
    ])
}

fn possession_transition_json(t: &PossessionTransitionState) -> Json {
    Json::obj_omit_null(vec![
        ("version", Json::int(i64::from(t.version))),
        (
            "last_team",
            t.last_team
                .map_or(Json::Null, |v| Json::str(transition_team_wire(v))),
        ),
        (
            "holding_team",
            t.holding_team
                .map_or(Json::Null, |v| Json::str(transition_team_wire(v))),
        ),
        ("hold", Json::Number(t.hold)),
        (
            "turnover_team",
            t.turnover_team
                .map_or(Json::Null, |v| Json::str(transition_team_wire(v))),
        ),
        ("elapsed", Json::Number(t.elapsed)),
    ])
}

fn transition_config_json(c: TransitionConfig) -> Json {
    Json::obj(vec![
        ("counterpress", Json::Number(c.counterpress)),
        ("counterattack", Json::Number(c.counterattack)),
    ])
}

fn transition_windows_json(w: TransitionWindows) -> Json {
    Json::obj(vec![
        ("home", transition_config_json(w.home)),
        ("away", transition_config_json(w.away)),
    ])
}

fn team_wire(team: Team) -> &'static str {
    team.wire_str()
}

/// One [`MatchPlayer`], narrowed to exactly `replay.ts`'s `MatchPlayer`
/// interface — see the module doc.
fn match_player_json(p: &MatchPlayer) -> Json {
    Json::obj_omit_null(vec![
        ("id", Json::str(p.id.clone())),
        ("team", Json::str(team_wire(p.team))),
        ("pos", vec2_json(p.pos.x, p.pos.y)),
        ("run_vel", vec2_json(p.run_vel.x, p.run_vel.y)),
        ("facing", vec2_json(p.facing.x, p.facing.y)),
        ("radius", Json::Number(p.radius)),
        ("is_keeper", Json::bool(p.is_keeper)),
        (
            "keeper_state",
            Json::str(keeper_behavior_state_wire(p.keeper_state)),
        ),
        ("keeper_set", Json::Number(p.keeper_set)),
        ("slide_timer", Json::Number(p.slide_timer)),
        ("tackle_timer", Json::Number(p.tackle_timer)),
        ("stun_timer", Json::Number(p.stun_timer)),
        ("settle_timer", Json::Number(p.settle_timer)),
        ("sprinting", Json::bool(p.sprinting)),
        (
            "outfield_decision",
            outfield_decision_json(&p.outfield_decision),
        ),
        ("dive_timer", Json::Number(p.dive_timer)),
        ("dive_dir", vec2_json(p.dive_dir.x, p.dive_dir.y)),
        ("keeper_get_up_timer", Json::Number(p.keeper_get_up_timer)),
        (
            "save_style",
            p.save_style
                .map_or(Json::Null, |v| Json::str(save_style_wire(v))),
        ),
        ("grab_timer", Json::Number(p.grab_timer)),
        ("throw_timer", Json::Number(p.throw_timer)),
        ("windup_timer", Json::Number(p.windup_timer)),
        ("aerial_timer", Json::Number(p.aerial_timer)),
        (
            "aerial_style",
            p.aerial_style
                .map_or(Json::Null, |v| Json::str(aerial_style_wire(v))),
        ),
        (
            "aerial_outcome",
            p.aerial_outcome
                .map_or(Json::Null, |v| Json::str(aerial_outcome_wire(v))),
        ),
        ("aerial_jump", Json::Number(p.aerial_jump)),
        ("sprint_meter", Json::Number(p.sprint_meter)),
        ("jockey_timer", Json::Number(p.jockey_timer)),
    ])
}

/// Encodes the raw-`MatchState`-for-replay slice, matching
/// `packages/render/src/replay.ts`'s `MatchState` interface field-for-field
/// (`field`, `goal_home`, `goal_away`, `score`, `time_left`, `press`,
/// `outfield_press`, `transition`, `transition_windows`, `controlled?`,
/// `owner?`, `ball`, `ball_vel`, `ball_z`, `ball_vz`, `players`, `events`).
/// See the module doc for why this is a narrow, hand-picked slice rather
/// than the full `MatchState`/`MatchPlayer` structs.
///
/// `press` (`ByTeam<i64>` — chasers per team, copied from each side's
/// authored `TacticData.press` at kickoff, `crates/gc-sim/src/match.rs`) was
/// added for `packages/app/src/bootstrap.spec.ts`'s/`flow.spec.ts`'s
/// `state.press.home` cases: before this addition it was exposed nowhere
/// reachable from `@gc/app` — not `SimSession` (confirmed reading
/// `session.rs`: score/timeLeft/finished/inputTick/snapshotHash/roster/
/// matchStateJson only, and this narrow encoder did not carry it either),
/// not `gc_render::frame::RenderFrame` (`gc-render` is a reference crate this
/// task does not own, and it has no `press` field either), and not
/// `@gc/app`'s own `content.ts`. `Session.matchStateJson()` was the only file
/// in this task's ownership that could close the gap.
#[must_use]
pub(crate) fn match_state_to_json(state: &MatchState) -> Json {
    Json::obj_omit_null(vec![
        ("field", pitch_size_json(state.field)),
        ("goal_home", rect_json(state.goal_home)),
        ("goal_away", rect_json(state.goal_away)),
        (
            "score",
            Json::obj(vec![
                ("home", Json::int(state.score.home)),
                ("away", Json::int(state.score.away)),
            ]),
        ),
        ("time_left", Json::Number(state.time_left)),
        (
            "press",
            Json::obj(vec![
                ("home", Json::int(state.press.home)),
                ("away", Json::int(state.press.away)),
            ]),
        ),
        (
            "outfield_press",
            Json::obj(vec![
                ("home", outfield_press_json(&state.outfield_press.home)),
                ("away", outfield_press_json(&state.outfield_press.away)),
            ]),
        ),
        ("transition", possession_transition_json(&state.transition)),
        (
            "transition_windows",
            transition_windows_json(state.transition_windows),
        ),
        // `controlled` is always present on `MatchState` (a plain `i64`, not
        // `Option`) -- `replay.ts`'s `controlled?: number` accepts a present
        // value just as well as an absent one, so this is included
        // unconditionally rather than treated as optional.
        ("controlled", Json::int(state.controlled)),
        ("owner", Json::opt_int(state.owner)),
        ("ball", vec2_json(state.ball.x, state.ball.y)),
        ("ball_vel", vec2_json(state.ball_vel.x, state.ball_vel.y)),
        ("ball_z", Json::Number(state.ball_z)),
        ("ball_vz", Json::Number(state.ball_vz)),
        (
            "players",
            Json::Array(state.players.iter().map(match_player_json).collect()),
        ),
        (
            "events",
            Json::Array(state.events.iter().map(raw_match_event_to_json).collect()),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use gc_data::teams;
    use gc_sim::r#match as sim_match;
    use gc_sim::match_snapshot::PitchSize as SimPitchSize;

    fn fixture_state() -> MatchState {
        let home = teams::get("nebula").expect("fixture team");
        let away = teams::get("orion").expect("fixture team");
        sim_match::new(sim_match::NewMatchOptions {
            home,
            away,
            field: SimPitchSize { w: 960.0, h: 540.0 },
            home_formation: None,
            tactic: None,
            away_tactic: None,
            duration: Some(300.0),
            max_goals: Some(3),
            seed: Some(11.0),
            players_by_id: None,
            species_by_id: None,
            showcase_players_by_id: None,
            human_controlled: None,
            input_ownership: None,
        })
    }

    #[test]
    fn encodes_every_field_replay_ts_needs() {
        let state = fixture_state();
        let json = match_state_to_json(&state);
        let field = json.get("field").expect("field present");
        assert_eq!(field.field_i64("w"), Some(960));
        assert_eq!(field.field_i64("h"), Some(540));
        assert!(json.get("goal_home").is_some());
        assert!(json.get("goal_away").is_some());
        let score = json.get("score").expect("score present");
        assert_eq!(score.field_i64("home"), Some(0));
        assert_eq!(score.field_i64("away"), Some(0));
        assert!(json.get("time_left").is_some());
        let press = json.get("press").expect("press present");
        assert!(press.field_i64("home").is_some());
        assert!(press.field_i64("away").is_some());
        let outfield_press = json.get("outfield_press").expect("outfield_press present");
        assert!(outfield_press.get("home").is_some());
        assert!(outfield_press.get("away").is_some());
        assert!(json.get("transition").is_some());
        let windows = json
            .get("transition_windows")
            .expect("transition_windows present");
        assert!(
            windows
                .get("home")
                .and_then(|w| w.get("counterpress"))
                .is_some()
        );
        assert!(json.get("controlled").is_some());
        assert!(json.get("ball").is_some());
        let players = json
            .get("players")
            .and_then(Json::as_array)
            .expect("players array present");
        assert_eq!(players.len(), state.players.len());
        let first = &players[0];
        assert!(first.get("id").is_some());
        assert!(first.get("outfield_decision").is_some());
        assert!(first.get("keeper_get_up_timer").is_some());
        assert!(json.get("events").and_then(Json::as_array).is_some());
    }

    #[test]
    fn press_carries_the_authored_tactic_chaser_count() {
        let home = teams::get("nebula").expect("fixture team");
        let away = teams::get("orion").expect("fixture team");
        let press_high = gc_data::tactics::get("press_high").expect("authored tactic");
        let state = sim_match::new(sim_match::NewMatchOptions {
            home,
            away,
            field: SimPitchSize { w: 960.0, h: 540.0 },
            home_formation: None,
            tactic: Some(press_high),
            away_tactic: None,
            duration: Some(300.0),
            max_goals: Some(3),
            seed: Some(11.0),
            players_by_id: None,
            species_by_id: None,
            showcase_players_by_id: None,
            human_controlled: None,
            input_ownership: None,
        });
        let json = match_state_to_json(&state);
        let press = json.get("press").expect("press present");
        assert_eq!(press.field_i64("home"), Some(press_high.press));
    }

    #[test]
    fn omits_absent_optional_player_style_fields_rather_than_nulling_them() {
        let state = fixture_state();
        let json = match_state_to_json(&state);
        let players = json.get("players").and_then(Json::as_array).unwrap();
        let first = &players[0];
        // A freshly constructed match has no in-flight save/aerial style yet.
        assert!(first.get("save_style").is_none());
        assert!(first.get("aerial_style").is_none());
        assert!(first.get("aerial_outcome").is_none());
    }
}
