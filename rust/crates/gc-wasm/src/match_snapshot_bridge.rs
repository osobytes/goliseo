//! `wasm-bindgen` control surface for constructing a
//! [`gc_sim::match_snapshot::MatchSnapshot`] from caller-specified fields —
//! the gap `packages/app/src/rollback_validation.spec.ts`'s "derives
//! reference identities from raw campaign step inputs" and
//! `packages/app/src/match_rollback_lab.spec.ts`'s "reconciles a rollback
//! goal…" both name directly: `@gc/wasm` had no entry point that built a
//! [`crate::rollback_events_bridge::WasmMatchSnapshot`] from anything but a
//! real, already-stepped session/driver's own current state.
//! `crate::online_combat_phases_bridge::online_combat_phase_boundary_zero`
//! was the first step in this direction (that module's own doc says so
//! explicitly), but it is narrow on purpose: one of seven pinned fixtures,
//! not an arbitrary hand-specified state.
//!
//! ## The shape this exposes, and why
//!
//! A `MatchSnapshot` never crosses to JS as JSON anywhere in this crate (see
//! [`crate::rollback_events_bridge::WasmMatchSnapshot`]'s doc for why) — so
//! this module cannot simply decode one field-for-field from a JSON blob
//! either. It also cannot decode a full, freestanding
//! [`gc_sim::match_snapshot::MatchState`] from JSON: that struct carries 30+
//! fields (`gc_sim::match_snapshot::MatchState`'s own doc), most of them
//! authored-content-shaped (per-player stats, formation anchors, tactic
//! knobs) that no caller of this surface has ever needed to hand-specify —
//! `crate::match_state_bridge::match_state_to_json`'s own doc names the same
//! reason for staying a narrow, hand-picked slice rather than the whole
//! struct.
//!
//! So [`match_snapshot_build`] follows the pattern every other fixture
//! builder in this crate already uses
//! ([`crate::online_combat_phases_bridge::capture_boundary_zero`],
//! `crate::session::Session::new`): build a real, fully-valid `MatchState`
//! the ordinary way (`gc_sim::r#match::new`, from a team-id/seed/duration
//! construction identical to `Session::new`'s own), then apply a small,
//! explicit set of JSON-encoded OVERRIDES on top of it, then capture. The
//! override set is exactly what both blocked tests need (cross-checked
//! against `packages/app/src/rollback_validation.spec.ts`'s
//! "derives reference identities from raw campaign step inputs" and
//! `packages/app/src/match_rollback_lab.spec.ts`'s `rollback_goal_fixture`):
//! `owner` (nullable), `time_left`, `input_tick`, `ball`/`ball_vel`/
//! `ball_z`/`ball_vz`, `pickup_cd`, `block_grace`, a wholesale `events`
//! replacement, and per-player `pos`/`run_vel`/`facing`/`vel` overrides
//! (one-based `index`, matching `MatchState.owner`'s own one-based
//! convention). See [`apply_overrides`]'s doc for the exact JSON shape.
//!
//! Two calls with identical `(home_team_id, away_team_id, seed,
//! duration_seconds, max_goals, home_formation, overrides_json)` produce a
//! byte-identical snapshot — the same determinism property
//! `online_combat_phase_boundary_zero` already documents and tests, and the
//! one a caller seeding more than one peer from a hand-built snapshot
//! depends on.
//!
//! [`match_snapshot_state_json`] is the read-back half: reuses
//! [`crate::match_state_bridge::match_state_to_json`] unchanged (no widening
//! of that encoder's own documented narrow scope) to let a caller inspect a
//! snapshot built this way — e.g. to read back a freshly-built player's `id`
//! or a computed `time_left` before nudging it, exactly the
//! `match_snapshot::restore(match_snapshot::capture(initial))` round trip.
//!
//! ## What is deliberately out of scope
//!
//! No combat companion: [`match_snapshot_build`] always captures with
//! `combat_state: None`. Neither blocked test needs one (confirmed reading
//! both specs), and threading one through would mean either
//! generalizing this module's override set to `CombatMatchState` fields too
//! (nothing today asks for that) or silently guessing at defaults for it —
//! flagged here as follow-up rather than built speculatively.

use gc_core::vec2::Vec2;
use gc_data::teams;
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{self, MatchState, PitchSize};
use wasm_bindgen::prelude::*;

use crate::json::Json;
use crate::rollback_events_bridge::{self, WasmMatchSnapshot};

fn js_err(message: impl Into<String>) -> JsValue {
    JsValue::from_str(&message.into())
}

/// Playable pitch dimensions. Mirrors `crate::session`'s own
/// `FIELD_W`/`FIELD_H` (private to that module) — the same deliberate small
/// constant duplication `crate::online_combat_phases_bridge`'s own `FIELD`
/// already is, for the same reason: nothing here depends on `crate::session`
/// beyond its `ownership` helper, and this module's field size has no
/// occasion to differ from it.
const FIELD_W: f64 = 1648.0;
const FIELD_H: f64 = 927.0;

/// `pub(crate)`, not private: [`crate::player_pose_bridge`] needs the exact
/// same `{"x": ..., "y": ...}` decode for its own `pos`/`run_vel`/`facing`
/// overlay fields — reusing it here keeps exactly one `Vec2` JSON mapping in
/// this crate rather than a second copy.
pub(crate) fn vec2_from_json(json: &Json) -> Result<Vec2, String> {
    let x = json
        .field("x")
        .and_then(Json::as_f64)
        .ok_or("expected a vec2 object with a numeric 'x'")?;
    let y = json
        .field("y")
        .and_then(Json::as_f64)
        .ok_or("expected a vec2 object with a numeric 'y'")?;
    Ok(Vec2::new(x, y))
}

/// Applies `overrides` on top of `state`, in place. Every key is optional; an
/// absent key leaves that field exactly as `gc_sim::r#match::new` built it.
///
/// ```jsonc
/// {
///   "owner": 2,            // integer (one-based player index) or `null` to clear
///   "time_left": 119.9833,
///   "input_tick": 1,
///   "ball": {"x": 350, "y": 270},
///   "ball_vel": {"x": 1200, "y": 0},
///   "ball_z": 10,
///   "ball_vz": 200,
///   "pickup_cd": 999,
///   "block_grace": 999,
///   // Replaces `state.events` wholesale (never merged) -- the same
///   // round-trippable shape `crate::match_state_bridge::match_state_to_json`'s
///   // "events" already emits / `crate::rollback_events_bridge`'s
///   // `raw_match_event_from_json` already decodes.
///   "events": [ {"kind": "shot", "x": 350, "y": 270, "player": "nebula_02"} ],
///   // One-based `index` into `state.players` (matches `MatchState.owner`'s
///   // own one-based convention). Only the listed fields on that player are
///   // overridden; an unlisted player is untouched.
///   "players": [
///     {"index": 1, "pos": {"x": 180, "y": 40}, "run_vel": {"x": 0, "y": 0}}
///   ]
/// }
/// ```
fn apply_overrides(state: &mut MatchState, overrides: &Json) -> Result<(), String> {
    if let Some(owner) = overrides.get("owner") {
        // Construction-time, so outside #489's possession invariant and its
        // `gc_sim::r#match::set_owner` choke point (which clears the
        // outgoing owner's committed action slot): the only caller is
        // `match_snapshot_build`, between `sim_match::new` and the first
        // `match_snapshot::capture`, so no tick has run and every action
        // slot is still idle. There is no outgoing owner whose commitment
        // could need clearing. A mid-match owner change must not be added
        // here.
        state.owner = if owner.is_null() {
            None
        } else {
            Some(
                owner
                    .as_i64()
                    .ok_or("'owner' override must be an integer or null")?,
            )
        };
    }
    if let Some(time_left) = overrides.field("time_left").and_then(Json::as_f64) {
        state.time_left = time_left;
    }
    if let Some(input_tick) = overrides.field_i64("input_tick") {
        state.input_tick = input_tick;
    }
    if let Some(ball) = overrides.field("ball") {
        state.ball = vec2_from_json(ball)?;
    }
    if let Some(ball_vel) = overrides.field("ball_vel") {
        state.ball_vel = vec2_from_json(ball_vel)?;
    }
    if let Some(ball_z) = overrides.field("ball_z").and_then(Json::as_f64) {
        state.ball_z = ball_z;
    }
    if let Some(ball_vz) = overrides.field("ball_vz").and_then(Json::as_f64) {
        state.ball_vz = ball_vz;
    }
    if let Some(pickup_cd) = overrides.field("pickup_cd").and_then(Json::as_f64) {
        state.pickup_cd = pickup_cd;
    }
    if let Some(block_grace) = overrides.field("block_grace").and_then(Json::as_f64) {
        state.block_grace = block_grace;
    }
    if let Some(events) = overrides.field("events").and_then(Json::as_array) {
        let mut decoded = Vec::with_capacity(events.len());
        for item in events {
            decoded.push(rollback_events_bridge::raw_match_event_from_json(item)?);
        }
        state.events = decoded;
    }
    if let Some(players) = overrides.field("players").and_then(Json::as_array) {
        for entry in players {
            let index = entry
                .field_i64("index")
                .ok_or("player override missing integer 'index'")?;
            if index < 1 || index as usize > state.players.len() {
                return Err(format!(
                    "player override 'index' {index} is out of range (1..={})",
                    state.players.len()
                ));
            }
            let player = &mut state.players[(index - 1) as usize];
            if let Some(pos) = entry.field("pos") {
                player.pos = vec2_from_json(pos)?;
            }
            if let Some(run_vel) = entry.field("run_vel") {
                player.run_vel = vec2_from_json(run_vel)?;
            }
            if let Some(facing) = entry.field("facing") {
                player.facing = vec2_from_json(facing)?;
            }
            if let Some(vel) = entry.field("vel") {
                player.vel = vec2_from_json(vel)?;
            }
        }
    }
    Ok(())
}

/// Builds a fresh, slot-mode `MatchState` exactly like `Session::new` does
/// (`home_team_id`/`away_team_id` from `gc_data::teams::ALL`, a five-player
/// roster required), applies `overrides_json` on top of it (see
/// [`apply_overrides`]'s doc for the shape), and captures the result as an
/// opaque [`WasmMatchSnapshot`] handle. `overrides_json` omitted/`None`
/// captures the freshly-built state unchanged — equivalent to
/// `crate::session::Session::new(...).snapshot_handle()` before any `step`.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if either team id is not authored
/// content, either roster is not five players, `overrides_json` fails to
/// parse, or an override is malformed (wrong type, an out-of-range player
/// `index`, or an `events` entry that fails to decode).
#[wasm_bindgen(js_name = matchSnapshotBuild)]
#[allow(clippy::too_many_arguments)]
pub fn match_snapshot_build(
    home_team_id: &str,
    away_team_id: &str,
    seed: f64,
    duration_seconds: f64,
    max_goals: i32,
    home_formation: Option<String>,
    overrides_json: Option<String>,
) -> Result<WasmMatchSnapshot, JsValue> {
    let home = teams::get(home_team_id).ok_or_else(|| js_err("unknown home team id"))?;
    let away = teams::get(away_team_id).ok_or_else(|| js_err("unknown away team id"))?;
    if home.roster.len() != 5 || away.roster.len() != 5 {
        return Err(js_err(
            "match snapshot construction requires a five-player roster (one keeper, four outfield)",
        ));
    }
    let home_roster: [&str; 5] = home.roster.try_into().expect("checked len == 5");
    let away_roster: [&str; 5] = away.roster.try_into().expect("checked len == 5");
    let mut state = sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize {
            w: FIELD_W,
            h: FIELD_H,
        },
        home_formation: home_formation.as_deref(),
        tactic: None,
        away_tactic: None,
        duration: Some(duration_seconds),
        max_goals: Some(i64::from(max_goals)),
        seed: Some(seed),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(crate::session::ownership(&home_roster, &away_roster)),
    });
    if let Some(overrides_json) = overrides_json {
        let parsed = Json::parse(&overrides_json).map_err(js_err)?;
        apply_overrides(&mut state, &parsed).map_err(js_err)?;
    }
    Ok(WasmMatchSnapshot::new(match_snapshot::capture(
        &state, None,
    )))
}

/// `snapshot`'s underlying `MatchState`, as JSON —
/// [`crate::match_state_bridge::match_state_to_json`]'s exact shape
/// (`packages/render/src/replay.ts`'s `MatchState` interface field-for-field:
/// `field`, `goal_home`, `goal_away`, `score`, `time_left`, `outfield_press`,
/// `transition`, `transition_windows`, `controlled`, `owner?`, `ball`,
/// `ball_vel`, `ball_z`, `ball_vz`, `players`, `events`). The read-back half
/// of this module — see the module doc — for a caller that needs to inspect
/// a snapshot's fields (e.g. a freshly built player's `id`, or `time_left`
/// before nudging it) rather than only ever writing to one.
#[wasm_bindgen(js_name = matchSnapshotStateJson)]
#[must_use]
pub fn match_snapshot_state_json(snapshot: &WasmMatchSnapshot) -> String {
    let (state, _combat) = match_snapshot::restore(&snapshot.snapshot);
    crate::match_state_bridge::match_state_to_json(&state).to_json_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_an_unmodified_snapshot_when_overrides_are_omitted() {
        let handle = match_snapshot_build("nebula", "orion", 7.0, 20.0, 3, None, None)
            .expect("fixture teams with five-player rosters always construct");
        let json = match_snapshot_state_json(&handle);
        let parsed = Json::parse(&json).expect("valid JSON");
        assert!(parsed.field_i64("controlled").is_some_and(|v| v >= 1));
        // `gc_sim::r#match::new` assigns kickoff possession to the most
        // advanced home outfielder -- a fresh, unmodified state has an
        // owner, not none.
        assert!(parsed.field_i64("owner").is_some_and(|v| v >= 1));
    }

    #[test]
    fn two_calls_with_identical_arguments_are_byte_identical() {
        let a = match_snapshot_build("nebula", "orion", 7.0, 20.0, 3, None, None).unwrap();
        let b = match_snapshot_build("nebula", "orion", 7.0, 20.0, 3, None, None).unwrap();
        assert_eq!(
            match_snapshot::hash(&a.snapshot),
            match_snapshot::hash(&b.snapshot),
            "two calls with the same arguments must be byte-identical -- the \
             property a caller relies on to seed more than one peer"
        );
    }

    #[test]
    fn overrides_apply_owner_events_and_time_left_reproducing_the_lua_fixture() {
        let initial = match_snapshot_build("nebula", "orion", 77.0, 300.0, 3, None, None).unwrap();
        let initial_json = Json::parse(&match_snapshot_state_json(&initial)).unwrap();
        let player_id = initial_json
            .get("players")
            .and_then(Json::as_array)
            .and_then(|players| players.get(1))
            .and_then(|p| p.field_str("id"))
            .expect("second player has an id")
            .to_string();
        let time_left = initial_json
            .field("time_left")
            .and_then(Json::as_f64)
            .unwrap();
        let ball = initial_json.get("ball").unwrap();
        let ball_x = ball.field("x").and_then(Json::as_f64).unwrap();
        let ball_y = ball.field("y").and_then(Json::as_f64).unwrap();

        let overrides = Json::obj(vec![
            ("owner", Json::int(2)),
            ("input_tick", Json::int(1)),
            ("time_left", Json::Number(time_left - 1.0 / 60.0)),
            (
                "events",
                Json::Array(vec![Json::obj(vec![
                    ("kind", Json::str("shot")),
                    ("x", Json::Number(ball_x)),
                    ("y", Json::Number(ball_y)),
                    ("player", Json::str(player_id.clone())),
                ])]),
            ),
        ])
        .to_json_string();

        let final_handle =
            match_snapshot_build("nebula", "orion", 77.0, 300.0, 3, None, Some(overrides))
                .expect("well-formed overrides apply");
        let final_json = Json::parse(&match_snapshot_state_json(&final_handle)).unwrap();
        assert_eq!(final_json.field_i64("owner"), Some(2));
        assert_eq!(
            final_json.field("time_left").and_then(Json::as_f64),
            Some(time_left - 1.0 / 60.0)
        );
        let events = final_json.get("events").and_then(Json::as_array).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].field_str("kind"), Some("shot"));
        assert_eq!(events[0].field_str("player"), Some(player_id.as_str()));
    }

    #[test]
    fn overrides_reposition_every_player_and_prime_a_loose_ball_reproducing_the_rollback_goal_fixture()
     {
        let mut players = Vec::new();
        for index in 1..=10i64 {
            players.push(Json::obj(vec![
                ("index", Json::int(index)),
                (
                    "pos",
                    Json::obj(vec![("x", Json::Number(180.0)), ("y", Json::Number(40.0))]),
                ),
                (
                    "run_vel",
                    Json::obj(vec![("x", Json::Number(0.0)), ("y", Json::Number(0.0))]),
                ),
            ]));
        }
        let overrides = Json::obj(vec![
            ("owner", Json::Null),
            (
                "ball",
                Json::obj(vec![("x", Json::Number(350.0)), ("y", Json::Number(270.0))]),
            ),
            (
                "ball_vel",
                Json::obj(vec![("x", Json::Number(1200.0)), ("y", Json::Number(0.0))]),
            ),
            ("ball_z", Json::Number(10.0)),
            ("ball_vz", Json::Number(200.0)),
            ("pickup_cd", Json::Number(999.0)),
            ("block_grace", Json::Number(999.0)),
            ("players", Json::Array(players)),
        ])
        .to_json_string();

        let handle = match_snapshot_build("nebula", "orion", 1.0, 2.0, 2, None, Some(overrides))
            .expect("well-formed overrides apply");
        let json = Json::parse(&match_snapshot_state_json(&handle)).unwrap();
        assert!(
            json.get("owner").is_none(),
            "owner override of null must clear it"
        );
        let ball = json.get("ball").unwrap();
        assert_eq!(ball.field("x").and_then(Json::as_f64), Some(350.0));
        assert_eq!(ball.field("y").and_then(Json::as_f64), Some(270.0));
        let players = json.get("players").and_then(Json::as_array).unwrap();
        assert_eq!(players.len(), 10);
        for player in players {
            let pos = player.get("pos").unwrap();
            assert_eq!(pos.field("x").and_then(Json::as_f64), Some(180.0));
            assert_eq!(pos.field("y").and_then(Json::as_f64), Some(40.0));
        }
    }

    // `match_snapshot_build`'s error paths (an unknown team id, a malformed
    // `overrides_json`, an out-of-range player `index`) return a `JsValue`,
    // which `wasm_bindgen::JsValue::from_str` cannot construct off the
    // native (non-wasm32) test target this module's own tests run under --
    // see `crate::coordinator_bridge`'s module doc for the established split
    // every bridge in this crate follows for an exported error path.
    // `packages/wasm/src/match_snapshot.spec.ts` (or wherever the
    // TypeScript-side owning agent lands this surface's coverage) is
    // expected to cover those paths against the real compiled artifact.
}
