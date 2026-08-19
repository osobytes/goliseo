//! `wasm-bindgen` control surface over [`gc_render::player_pose::select`],
//! standalone — over any caller-supplied `MatchPlayer`, not only a live
//! [`crate::session::Session`]'s current tick.
//!
//! ## The gap this closes
//!
//! `player_pose::select` already crosses the wasm boundary today, but only
//! baked into a live session's per-frame [`gc_render::frame::RenderFrame`]
//! (`crate::render_export`/`crate::session`): `frame::build` calls it once
//! per player, every tick, against `entry.state`. `packages/render/src/replay.spec.ts`'s
//! remaining skip names exactly what that does not reach: `@gc/render`'s
//! goal-replay buffer (`replay.ts`) records pre-render `MatchState`/
//! `MatchPlayer` snapshots — never a decoded `RenderFrame` — so a buffered,
//! replayed frame was never run through pose selection in the first place,
//! and nothing in `@gc/wasm` could select a pose for an arbitrary buffered
//! player the way `buildRenderFrame` does for a live session. This module is
//! that missing entry point: [`player_pose_select`] takes exactly the
//! `MatchPlayer`-shaped JSON `crate::match_state_bridge::match_state_to_json`
//! already emits (`packages/render/src/replay.ts`'s own `MatchPlayer`
//! interface, field-for-field — the shape a buffered frame actually is)
//! plus the same optional combat/keeper/outfield context
//! `gc_render::frame::build` itself threads through, and returns the
//! selected pose.
//!
//! ## Reconstructing a full `MatchPlayer` from a narrow JSON slice
//!
//! [`gc_sim::match_snapshot::MatchPlayer`] carries 70+ fields (stats,
//! species identity, formation anchor, ...); the narrow slice this module
//! accepts (`match_state_bridge::match_player_json`'s own shape) carries
//! only the ~25 [`gc_render::player_pose::select`] itself ever reads —
//! confirmed by reading `player_pose.rs`'s `select` end to end, not assumed.
//! [`baseline_player`] supplies every OTHER field from a real,
//! fully-constructed fixture player (the same `gc_sim::r#match::new`
//! construction `crate::online_combat_phases_bridge`/every `gc-render` pose
//! test already uses), and [`match_player_from_json`] overlays the caller's
//! fields on top of it. Because `select` never reads anything outside that
//! overlaid set, the result is exactly what `select` would compute for a
//! genuinely live player carrying those field values — not an approximation.

use gc_data::teams;
use gc_render::player_pose::{
    self, CombatPoseSample, KeeperPoseContext, OutfieldPoseContext, PlayerPoseSource,
};
use gc_sim::combat_feasibility::CombatActionPhase;
use gc_sim::combat_snapshot::CombatForcedState;
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{MatchPlayer, PitchSize, Team};
use gc_sim::outfield_decision::OutfieldDecisionState;
use wasm_bindgen::prelude::*;

use crate::json::Json;
use crate::match_snapshot_bridge::vec2_from_json;
use crate::rollback_events_bridge::{
    aerial_outcome_from_wire, aerial_style_from_wire, keeper_behavior_state_from_wire,
    save_style_from_wire,
};

fn js_err(message: impl Into<String>) -> JsValue {
    JsValue::from_str(&message.into())
}

fn team_from_wire(text: &str) -> Result<Team, String> {
    match text {
        "home" => Ok(Team::Home),
        "away" => Ok(Team::Away),
        other => Err(format!("'{other}' is not a canonical team")),
    }
}

fn combat_action_phase_from_wire(text: &str) -> Result<CombatActionPhase, String> {
    match text {
        "ready" => Ok(CombatActionPhase::Ready),
        "windup" => Ok(CombatActionPhase::Windup),
        "active" => Ok(CombatActionPhase::Active),
        "aim" => Ok(CombatActionPhase::Aim),
        "guard" => Ok(CombatActionPhase::Guard),
        "recovery" => Ok(CombatActionPhase::Recovery),
        other => Err(format!("'{other}' is not a canonical combat action phase")),
    }
}

fn combat_forced_state_from_wire(text: &str) -> Result<CombatForcedState, String> {
    match text {
        "stagger" => Ok(CombatForcedState::Stagger),
        "knockback" => Ok(CombatForcedState::Knockback),
        other => Err(format!("'{other}' is not a canonical combat forced state")),
    }
}

/// A real, fully-constructed fixture player — every field
/// [`gc_render::player_pose::select`] never reads comes from here. See the
/// module doc's "Reconstructing a full `MatchPlayer`" section.
fn baseline_player() -> MatchPlayer {
    let home = teams::get("nebula").expect("fixture team 'nebula' is authored");
    let away = teams::get("orion").expect("fixture team 'orion' is authored");
    let state = sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: None,
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    });
    state.players[0].clone()
}

fn outfield_decision_from_json(json: &Json) -> Result<OutfieldDecisionState, String> {
    let context = json
        .field_str("context")
        .map(crate::match_state_bridge::outfield_decision_context_from_wire)
        .transpose()?
        .unwrap_or(gc_sim::outfield_decision::OutfieldDecisionContext::Offball);
    let intent = json
        .field_str("intent")
        .map(crate::match_state_bridge::outfield_intent_from_wire)
        .transpose()?
        .unwrap_or(gc_sim::outfield_decision::OutfieldIntent::None);
    Ok(OutfieldDecisionState {
        version: json.field_i64("version").unwrap_or(1) as u32,
        generation: json.field_i64("generation").unwrap_or(0) as u32,
        rng_state: json.field_i64("rng_state").unwrap_or(0) as u32,
        remaining: json
            .field("remaining")
            .and_then(Json::as_f64)
            .unwrap_or(0.0),
        context,
        intent,
        target_x: json.field("target_x").and_then(Json::as_f64),
        target_y: json.field("target_y").and_then(Json::as_f64),
        target_player: json.field_i64("target_player").map(|v| v as u32),
        run_expires_at: json.field("run_expires_at").and_then(Json::as_f64),
    })
}

/// Overlays `json` (`match_state_bridge::match_player_json`'s exact shape —
/// `packages/render/src/replay.ts`'s `MatchPlayer` interface field-for-field)
/// onto [`baseline_player`]. Every key is optional; an absent key keeps the
/// baseline's value.
fn match_player_from_json(json: &Json) -> Result<MatchPlayer, String> {
    let mut player = baseline_player();
    if let Some(id) = json.field_str("id") {
        player.id = id.to_string();
    }
    if let Some(team) = json.field_str("team") {
        player.team = team_from_wire(team)?;
    }
    if let Some(pos) = json.field("pos") {
        player.pos = vec2_from_json(pos)?;
    }
    if let Some(run_vel) = json.field("run_vel") {
        player.run_vel = vec2_from_json(run_vel)?;
    }
    if let Some(facing) = json.field("facing") {
        player.facing = vec2_from_json(facing)?;
    }
    if let Some(radius) = json.field("radius").and_then(Json::as_f64) {
        player.radius = radius;
    }
    if let Some(is_keeper) = json.field_bool("is_keeper") {
        player.is_keeper = is_keeper;
    }
    if let Some(keeper_state) = json.field_str("keeper_state") {
        player.keeper_state = keeper_behavior_state_from_wire(keeper_state)?;
    }
    if let Some(keeper_set) = json.field("keeper_set").and_then(Json::as_f64) {
        player.keeper_set = keeper_set;
    }
    if let Some(slide_timer) = json.field("slide_timer").and_then(Json::as_f64) {
        player.slide_timer = slide_timer;
    }
    if let Some(tackle_timer) = json.field("tackle_timer").and_then(Json::as_f64) {
        player.tackle_timer = tackle_timer;
    }
    if let Some(stun_timer) = json.field("stun_timer").and_then(Json::as_f64) {
        player.stun_timer = stun_timer;
    }
    if let Some(settle_timer) = json.field("settle_timer").and_then(Json::as_f64) {
        player.settle_timer = settle_timer;
    }
    if let Some(sprinting) = json.field_bool("sprinting") {
        player.sprinting = sprinting;
    }
    if let Some(outfield_decision) = json.field("outfield_decision") {
        player.outfield_decision = outfield_decision_from_json(outfield_decision)?;
    }
    if let Some(dive_timer) = json.field("dive_timer").and_then(Json::as_f64) {
        player.dive_timer = dive_timer;
    }
    if let Some(dive_dir) = json.field("dive_dir") {
        player.dive_dir = vec2_from_json(dive_dir)?;
    }
    if let Some(keeper_get_up_timer) = json.field("keeper_get_up_timer").and_then(Json::as_f64) {
        player.keeper_get_up_timer = keeper_get_up_timer;
    }
    if let Some(save_style) = json.field_str("save_style") {
        player.save_style = Some(save_style_from_wire(save_style)?);
    }
    if let Some(grab_timer) = json.field("grab_timer").and_then(Json::as_f64) {
        player.grab_timer = grab_timer;
    }
    if let Some(throw_timer) = json.field("throw_timer").and_then(Json::as_f64) {
        player.throw_timer = throw_timer;
    }
    if let Some(windup_timer) = json.field("windup_timer").and_then(Json::as_f64) {
        player.windup_timer = windup_timer;
    }
    if let Some(aerial_timer) = json.field("aerial_timer").and_then(Json::as_f64) {
        player.aerial_timer = aerial_timer;
    }
    if let Some(aerial_style) = json.field_str("aerial_style") {
        player.aerial_style = Some(aerial_style_from_wire(aerial_style)?);
    }
    if let Some(aerial_outcome) = json.field_str("aerial_outcome") {
        player.aerial_outcome = Some(aerial_outcome_from_wire(aerial_outcome)?);
    }
    if let Some(aerial_jump) = json.field("aerial_jump").and_then(Json::as_f64) {
        player.aerial_jump = aerial_jump;
    }
    if let Some(sprint_meter) = json.field("sprint_meter").and_then(Json::as_f64) {
        player.sprint_meter = sprint_meter;
    }
    if let Some(jockey_timer) = json.field("jockey_timer").and_then(Json::as_f64) {
        player.jockey_timer = jockey_timer;
    }
    Ok(player)
}

fn combat_pose_sample_from_json(json: &Json) -> Result<CombatPoseSample, String> {
    let phase = combat_action_phase_from_wire(
        json.field_str("phase")
            .ok_or("combat context missing 'phase'")?,
    )?;
    let forced_state = json
        .field_str("forced_state")
        .map(combat_forced_state_from_wire)
        .transpose()?;
    Ok(CombatPoseSample {
        phase,
        forced_state,
        forced_ticks: json.field_i64("forced_ticks").unwrap_or(0),
        // Mirrors `CombatPoseSample::immunity_fraction`'s own units (an
        // already-normalised `[0, 1]`, not a raw tick count): this bridge's
        // whole point is to hand `select` the exact value it would see from
        // a live session, and that value is a fraction on this side of the
        // wall too (see `gc_render::frame::combat_model`).
        immunity_fraction: json
            .field("immunity_fraction")
            .and_then(Json::as_f64)
            .unwrap_or(0.0),
        // Same already-normalised-`[0, 1]` contract as `immunity_fraction`
        // (#576): progress through the current timed combat phase, not a raw
        // tick count. The `unwrap_or(0.0)` aliases "field omitted" with "at
        // the start of the phase", and that is safe for the same reason
        // `immunity_fraction`'s identical default is: "no combat at all" is
        // spelt by omitting the whole `combat` object (this function is then
        // never called), so a defaulted field inside a present object can
        // only understate progress within a real phase -- and `select` never
        // reads either fraction, so pose choice cannot be affected.
        phase_fraction: json
            .field("phase_fraction")
            .and_then(Json::as_f64)
            .unwrap_or(0.0),
    })
}

fn keeper_pose_context_from_json(json: &Json) -> KeeperPoseContext {
    KeeperPoseContext {
        near_ball: json.field_bool("near_ball").unwrap_or(false),
        shuffling: json.field_bool("shuffling").unwrap_or(false),
        tip: json.field_bool("tip").unwrap_or(false),
    }
}

fn outfield_pose_context_from_json(json: &Json) -> OutfieldPoseContext {
    OutfieldPoseContext {
        now: json.field("now").and_then(Json::as_f64),
        containing: json.field_bool("containing").unwrap_or(false),
        kick_follow: json.field_bool("kick_follow").unwrap_or(false),
        dispossessed: json.field_bool("dispossessed").unwrap_or(false),
    }
}

fn pose_source_wire(source: PlayerPoseSource) -> &'static str {
    match source {
        PlayerPoseSource::Soccer => "soccer",
        PlayerPoseSource::Combat => "combat",
        PlayerPoseSource::Locomotion => "locomotion",
    }
}

/// Selects the pose for an arbitrary buffered player — see the module doc.
/// `input_json` is:
///
/// ```jsonc
/// {
///   // Required: `crate::match_state_bridge::match_player_json`'s shape,
///   // i.e. `packages/render/src/replay.ts`'s `MatchPlayer` verbatim.
///   "player": { "id": "nebula_02", "is_keeper": false, ... },
///   // All three optional/omittable, mirroring `player_pose::select`'s own
///   // `Option` parameters -- omit exactly when the caller has none.
///   "combat": { "phase": "guard", "forced_state": "stagger", "forced_ticks": 4, "immunity_fraction": 0.6, "phase_fraction": 0.25 },
///   "keeper_context": { "near_ball": true, "shuffling": false, "tip": false },
///   "outfield_context": { "now": 12.5, "containing": false, "kick_follow": true, "dispossessed": false }
/// }
/// ```
///
/// Returns the selection as JSON: `{"id": "run_telegraph", "priority": 35,
/// "source": "soccer"}` — `id` is [`gc_render::player_pose::PlayerPoseId::wire_name`],
/// `source` is one of `"soccer"`/`"combat"`/`"locomotion"`.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if `input_json` fails to parse, is
/// missing `"player"`, or any field fails to decode against its closed wire
/// vocabulary (an unrecognized team/keeper-state/save-style/aerial-style/
/// aerial-outcome/outfield-intent/outfield-decision-context/combat-phase/
/// combat-forced-state).
#[wasm_bindgen(js_name = playerPoseSelect)]
pub fn player_pose_select(input_json: &str) -> Result<String, JsValue> {
    let parsed = Json::parse(input_json).map_err(js_err)?;
    let player_json = parsed
        .get("player")
        .ok_or_else(|| js_err("playerPoseSelect input is missing 'player'"))?;
    let player = match_player_from_json(player_json).map_err(js_err)?;
    let combat = parsed
        .field("combat")
        .map(combat_pose_sample_from_json)
        .transpose()
        .map_err(js_err)?;
    let keeper_context = parsed
        .field("keeper_context")
        .map(keeper_pose_context_from_json);
    let outfield_context = parsed
        .field("outfield_context")
        .map(outfield_pose_context_from_json);
    let selection = player_pose::select(
        &player,
        combat.as_ref(),
        keeper_context.as_ref(),
        outfield_context.as_ref(),
    );
    Ok(Json::obj(vec![
        ("id", Json::str(selection.id.wire_name())),
        ("priority", Json::int(selection.priority)),
        ("source", Json::str(pose_source_wire(selection.source))),
    ])
    .to_json_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player_json_for(is_keeper: bool, extra: Vec<(&str, Json)>) -> Json {
        let mut fields = vec![
            ("id", Json::str("nebula_02")),
            ("team", Json::str("home")),
            (
                "pos",
                Json::obj(vec![("x", Json::Number(100.0)), ("y", Json::Number(200.0))]),
            ),
            (
                "run_vel",
                Json::obj(vec![("x", Json::Number(0.0)), ("y", Json::Number(0.0))]),
            ),
            (
                "facing",
                Json::obj(vec![("x", Json::Number(1.0)), ("y", Json::Number(0.0))]),
            ),
            ("radius", Json::Number(10.0)),
            ("is_keeper", Json::bool(is_keeper)),
            ("keeper_state", Json::str("base")),
            ("keeper_set", Json::Number(0.0)),
            ("slide_timer", Json::Number(0.0)),
            ("tackle_timer", Json::Number(0.0)),
            ("stun_timer", Json::Number(0.0)),
            ("settle_timer", Json::Number(0.0)),
            ("sprinting", Json::bool(false)),
            (
                "outfield_decision",
                Json::obj(vec![
                    ("version", Json::int(1)),
                    ("generation", Json::int(0)),
                    ("rng_state", Json::int(1)),
                    ("remaining", Json::Number(0.0)),
                    ("context", Json::str("offball")),
                    ("intent", Json::str("none")),
                ]),
            ),
            ("dive_timer", Json::Number(0.0)),
            (
                "dive_dir",
                Json::obj(vec![("x", Json::Number(0.0)), ("y", Json::Number(0.0))]),
            ),
            ("keeper_get_up_timer", Json::Number(0.0)),
            ("grab_timer", Json::Number(0.0)),
            ("throw_timer", Json::Number(0.0)),
            ("windup_timer", Json::Number(0.0)),
            ("aerial_timer", Json::Number(0.0)),
            ("aerial_jump", Json::Number(0.0)),
            ("sprint_meter", Json::Number(1.0)),
            ("jockey_timer", Json::Number(0.0)),
        ];
        // `extra` overrides a base default of the same key rather than
        // shadowing it as a duplicate object entry -- `Json::field`/`get`
        // return the FIRST matching key, so a naive `extend` would silently
        // keep the base default instead of the override.
        fields.retain(|(key, _)| !extra.iter().any(|(extra_key, _)| extra_key == key));
        fields.extend(extra);
        Json::obj(fields)
    }

    #[test]
    fn a_neutral_player_resolves_to_locomotion() {
        let input = Json::obj(vec![("player", player_json_for(false, vec![]))]).to_json_string();
        let result = Json::parse(&player_pose_select(&input).unwrap()).unwrap();
        assert_eq!(result.field_str("id"), Some("locomotion"));
        assert_eq!(result.field_str("source"), Some("locomotion"));
        assert_eq!(result.field_i64("priority"), Some(0));
    }

    #[test]
    fn the_held_pose_input_field_from_replay_spec_ts_drives_keeper_get_up() {
        // Mirrors `replay.spec.ts`'s "preserves a held pose-input field
        // (keeper_get_up_timer)" fixture: a keeper with a nonzero
        // `keeper_get_up_timer` must select `keeper_get_up`, not the
        // locomotion fallback, purely off this narrow JSON slice.
        let input = Json::obj(vec![(
            "player",
            player_json_for(true, vec![("keeper_get_up_timer", Json::Number(0.18))]),
        )])
        .to_json_string();
        let result = Json::parse(&player_pose_select(&input).unwrap()).unwrap();
        assert_eq!(result.field_str("id"), Some("keeper_get_up"));
        assert_eq!(result.field_str("source"), Some("soccer"));
    }

    #[test]
    fn slide_timer_beats_locomotion_and_reports_soccer_source() {
        let input = Json::obj(vec![(
            "player",
            player_json_for(false, vec![("slide_timer", Json::Number(0.2))]),
        )])
        .to_json_string();
        let result = Json::parse(&player_pose_select(&input).unwrap()).unwrap();
        assert_eq!(result.field_str("id"), Some("slide"));
        assert_eq!(result.field_str("source"), Some("soccer"));
    }

    #[test]
    fn a_combat_guard_phase_outranks_locomotion_and_reports_combat_source() {
        let input = Json::obj(vec![
            ("player", player_json_for(false, vec![])),
            (
                "combat",
                Json::obj(vec![
                    ("phase", Json::str("guard")),
                    ("forced_ticks", Json::int(0)),
                ]),
            ),
        ])
        .to_json_string();
        let result = Json::parse(&player_pose_select(&input).unwrap()).unwrap();
        assert_eq!(result.field_str("id"), Some("combat_guard"));
        assert_eq!(result.field_str("source"), Some("combat"));
    }

    #[test]
    fn outfield_context_containing_selects_contain() {
        let input = Json::obj(vec![
            ("player", player_json_for(false, vec![])),
            (
                "outfield_context",
                Json::obj(vec![
                    ("now", Json::Number(12.5)),
                    ("containing", Json::bool(true)),
                    ("kick_follow", Json::bool(false)),
                ]),
            ),
        ])
        .to_json_string();
        let result = Json::parse(&player_pose_select(&input).unwrap()).unwrap();
        assert_eq!(result.field_str("id"), Some("contain"));
    }

    /// #591: a presentation-owned dispossession flinch window selects
    /// `stumble` exactly the way the simulation's own `stun_timer` does,
    /// with no sim-owned timer set at all — proves the standing-poke
    /// tackle's victim reaction is reachable purely through
    /// `outfield_context`.
    #[test]
    fn outfield_context_dispossessed_selects_stumble() {
        let input = Json::obj(vec![
            ("player", player_json_for(false, vec![])),
            (
                "outfield_context",
                Json::obj(vec![
                    ("now", Json::Number(12.5)),
                    ("containing", Json::bool(false)),
                    ("kick_follow", Json::bool(false)),
                    ("dispossessed", Json::bool(true)),
                ]),
            ),
        ])
        .to_json_string();
        let result = Json::parse(&player_pose_select(&input).unwrap()).unwrap();
        assert_eq!(result.field_str("id"), Some("stumble"));
        assert_eq!(result.field_str("source"), Some("soccer"));
    }

    // `player_pose_select`'s error paths (missing `"player"`, malformed JSON,
    // an unrecognized wire value) return a `JsValue`, which
    // `wasm_bindgen::JsValue::from_str` cannot construct off the native
    // (non-wasm32) test target this module's own tests run under -- see
    // `crate::coordinator_bridge`'s module doc for the established split
    // every bridge in this crate follows for an exported error path.
}
