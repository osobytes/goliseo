//! `wasm-bindgen` control surface over the driver-level combat-phase
//! fixture — `packages/online/src/match_presentation.spec.ts`'s nine
//! combat-phase cases (`describe.skip("online match presentation combat
//! phases ...")`, in that file's own words) and half of
//! `packages/app/src/rollback_validation.spec.ts`'s one blocked case.
//!
//! ## What this is a port of, and why it lives here rather than being reused
//!
//! `spec/support/online_combat_phases.lua` pins seven named combat-phase
//! boundary zeroes (wind-up, guard, contact, projectile flight, stagger,
//! ball spill, immunity expiry) — one rigged `gc_sim::match_snapshot::MatchSnapshot`
//! per phase, arranged so a headless run actually reaches that phase, plus
//! the predicate that recognises it. It already has a real, tested Rust
//! port: `crates/gc-netcode/tests/support/online_combat_phases.rs`. That
//! port is **not** reusable from this crate as a dependency, even though it
//! needs nothing `gc-netcode` itself provides (it only imports `gc-core`,
//! `gc-data`, `gc-sim`, and `indexmap` — all of them already dependencies of
//! this crate): it lives under `gc-netcode`'s `tests/` directory, which
//! Cargo never exposes as an importable library item to another crate, only
//! to that one crate's own integration-test binaries.
//!
//! `crates/gc-netcode/src/**` is not a file this wave owns (see this
//! crate's own file-ownership split), so the fix available to most crates
//! in this situation — promote the module from `tests/support/` to `src/`
//! and make it `pub` — is not available here either. The only place left to
//! put a `#[wasm_bindgen]` surface over this fixture is this crate, and the
//! only way to get there without touching `gc-netcode/src/**` is to bring
//! the fixture's logic along rather than import it. What follows is
//! therefore a second copy, not a shared module — the same category of
//! unavoidable, documented duplication [`crate::wasm_transport`]'s own
//! `MAX_PAYLOAD_BYTES`/`MAX_GUESTS`/`DEFAULT_QUEUE_LIMIT`/`DEFAULT_POLL_BATCH`
//! already are for `game/transport/contract.lua`'s constants, and unlike
//! `fnv1a64`/`diagnostics_schema` (v2/README.md §2.2), it sits nowhere near
//! the determinism path: nothing here runs inside a match tick or produces
//! a value two peers must agree on bit-for-bit. It only ever builds a
//! *starting* snapshot and reads one back for a test assertion. Every line
//! below was cross-checked field-for-field against the existing Rust port
//! (not re-derived from the Lua original by hand), so this is a transcription
//! of already-reviewed, already-tested logic, not a third independent
//! reading of `online_combat_phases.lua`.
//!
//! If `gc-netcode/src/**` is ever back in scope for a future wave, the
//! correct fix is to delete this module's fixture-building half and import
//! the promoted module instead — the predicates
//! ([`observed`]) and scenario table ([`SCENARIOS`]) below are the parts
//! most worth collapsing back to one copy.
//!
//! ## What is bound, and what is deliberately left out
//!
//! [`online_combat_phase_ids`], [`online_combat_phase_scenario_json`],
//! [`online_combat_phase_boundary_zero`], [`online_combat_phase_live_sample`],
//! and [`online_combat_phase_observed`] together are exactly what
//! `match_presentation.spec.ts`'s nine blocked cases need: pick a phase,
//! build its pinned boundary-zero snapshot (as an opaque
//! [`crate::rollback_events_bridge::WasmMatchSnapshot`] handle — a
//! `MatchSnapshot` never crosses to JS as JSON, see that type's doc), drive
//! a real `MatchDriverBridge` from it, script the live slot's input via
//! [`online_combat_phase_live_sample`], and ask [`online_combat_phase_observed`]
//! whether a resimulated tick actually passed through the named phase — all
//! without this crate's TypeScript side ever needing to know a single fact
//! about combat mechanics itself (README v2/§2.1: reimplementing that here
//! is exactly what the Rust/TypeScript line forbids).
//!
//! [`online_combat_phase_boundary_zero`] is also, incidentally, the first
//! general-purpose "build a `WasmMatchSnapshot` for a specific starting
//! state" entry point this crate exposes at all — before this module,
//! [`crate::session::Session`]/[`crate::match_driver_bridge::MatchDriverBridge`]
//! only ever handed back a snapshot a real, already-stepped session/driver
//! produced. It is intentionally narrow (one of seven pinned fixtures, not
//! an arbitrary hand-specified `MatchState`): the one remaining case this
//! does not reach — `rollback_validation.spec.ts`'s "derives reference
//! identities from raw campaign step inputs", which needs a *hand-mutated*
//! field set (a specific `owner`, a synthetic event at an exact ball
//! position, `time_left` nudged by one tick) with no fixed scenario behind
//! it — would need either a much more general snapshot-field-override
//! surface or scripting genuine gameplay input precisely enough to reach
//! that exact state, and is called out as still open in this crate's final
//! report rather than guessed at here.
//!
//! The guard probe (`combat_phases.GUARD_PROBE`/`guard_probe_boundary_zero`
//! in the Lua original, `GUARD_PROBE`/`guard_probe_boundary_zero` in the
//! existing Rust port) is evidentiary tooling behind *why* the `guard`
//! scenario uses the `CanonicalInput` route rather than `Policy` — nothing
//! in `match_presentation.spec.ts`'s nine cases calls it, so it is left
//! unbound here.

use gc_core::vec2::Vec2;
use gc_data::action_families::ActionFamilyId;
use gc_data::players::PlayerData;
use gc_data::teams;
use gc_sim::combat;
use gc_sim::combat_snapshot::{CombatEventKind, CombatMatchState, CombatPhase};
use gc_sim::input_frame;
use gc_sim::r#match::{self, NO_GOAL_LIMIT, NewMatchOptions};
use gc_sim::match_snapshot::{self, MatchSnapshot, PitchSize};
use indexmap::IndexMap;
use wasm_bindgen::prelude::*;

use crate::json::Json;
use crate::rollback_events_bridge::{self, WasmMatchSnapshot};

fn js_err(message: impl Into<String>) -> JsValue {
    JsValue::from_str(&message.into())
}

/// See [`HOME_OUTFIELD`]. Mirrors `AWAY_OUTFIELD`.
const AWAY_OUTFIELD: [i64; 4] = [7, 8, 9, 10];
/// Player indexes in a nebula-versus-orion `gc_sim::r#match` state
/// (1-based, matching `crates/gc-sim/src/match.rs`'s documented
/// convention). 1 and 6 are the protected keepers, which are slotless and
/// never combat-capable. Mirrors `HOME_OUTFIELD`.
const HOME_OUTFIELD: [i64; 4] = [2, 3, 4, 5];

/// Playable pitch dimensions every scenario shares. Mirrors
/// `combat_phases.FIELD`.
const FIELD: PitchSize = PitchSize { w: 960.0, h: 540.0 };
/// Long enough that no scenario can reach full time and turn a correction
/// question into a settle question. Mirrors `combat_phases.DURATION_SECONDS`.
const DURATION_SECONDS: f64 = 14.0;
/// Mirrors `combat_phases.SEED`.
const SEED: f64 = 74.0;
/// The opening lane the two facing lines are laid out along. Mirrors
/// `LINE_X`.
const LINE_X: f64 = 400.0;

/// Canonical order. Mirrors `combat_phases.PHASES`.
const PHASES: [&str; 7] = [
    "windup",
    "guard",
    "contact",
    "projectile_flight",
    "stagger",
    "ball_spill",
    "immunity_expiry",
];

/// How a scenario's named phase is reached. Mirrors `OnlineCombatPhaseRoute`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    /// The phase is produced by `gameplay_ai/combat/v1` itself, from
    /// nothing but the opening pose.
    Policy,
    /// The phase is produced by a live slot's own equipment input, carried
    /// on the canonical input stream.
    CanonicalInput,
}

impl Route {
    fn wire_str(self) -> &'static str {
        match self {
            Route::Policy => "policy",
            Route::CanonicalInput => "canonical_input",
        }
    }
}

/// Everything [`capture_boundary_zero`] needs to lay out a fixture. Mirrors
/// `OnlineCombatFixtureShape`.
#[derive(Clone, Copy, Debug)]
struct FixtureShape {
    home_family: ActionFamilyId,
    away_family: ActionFamilyId,
    separation_px: f64,
    row_spacing_px: f64,
}

/// One named combat phase's driver-level scenario. Mirrors
/// `OnlineCombatPhaseScenario`.
#[derive(Clone, Copy, Debug)]
struct Scenario {
    id: &'static str,
    route: Route,
    shape: FixtureShape,
    steps: i64,
    deliver_period: i64,
    hold_equipment: bool,
}

/// One loadout per family. Mirrors `LOADOUT_FOR`.
fn loadout_for(family: ActionFamilyId) -> &'static str {
    match family {
        ActionFamilyId::Unarmed => "loadout_spring_gloves",
        ActionFamilyId::Guard => "loadout_emberguard_shield",
        ActionFamilyId::LightMelee => "loadout_vector_blade",
        ActionFamilyId::Ranged => "loadout_pulse_blaster",
    }
}

/// Mirrors `SCENARIOS`.
const SCENARIOS: [Scenario; 7] = [
    Scenario {
        id: "windup",
        route: Route::Policy,
        shape: FixtureShape {
            home_family: ActionFamilyId::LightMelee,
            away_family: ActionFamilyId::LightMelee,
            separation_px: 36.0,
            row_spacing_px: 120.0,
        },
        steps: 240,
        deliver_period: 5,
        hold_equipment: false,
    },
    Scenario {
        id: "guard",
        route: Route::CanonicalInput,
        shape: FixtureShape {
            home_family: ActionFamilyId::Guard,
            away_family: ActionFamilyId::LightMelee,
            separation_px: 30.0,
            row_spacing_px: 120.0,
        },
        steps: 240,
        deliver_period: 5,
        hold_equipment: true,
    },
    Scenario {
        id: "contact",
        route: Route::Policy,
        shape: FixtureShape {
            home_family: ActionFamilyId::Unarmed,
            away_family: ActionFamilyId::Unarmed,
            separation_px: 24.0,
            row_spacing_px: 28.0,
        },
        steps: 480,
        deliver_period: 8,
        hold_equipment: false,
    },
    Scenario {
        id: "projectile_flight",
        route: Route::Policy,
        shape: FixtureShape {
            home_family: ActionFamilyId::Ranged,
            away_family: ActionFamilyId::Ranged,
            separation_px: 200.0,
            row_spacing_px: 120.0,
        },
        steps: 240,
        deliver_period: 5,
        hold_equipment: false,
    },
    Scenario {
        id: "stagger",
        route: Route::Policy,
        shape: FixtureShape {
            home_family: ActionFamilyId::LightMelee,
            away_family: ActionFamilyId::LightMelee,
            separation_px: 36.0,
            row_spacing_px: 120.0,
        },
        steps: 240,
        deliver_period: 5,
        hold_equipment: false,
    },
    Scenario {
        id: "ball_spill",
        route: Route::Policy,
        shape: FixtureShape {
            home_family: ActionFamilyId::Unarmed,
            away_family: ActionFamilyId::Unarmed,
            separation_px: 24.0,
            row_spacing_px: 28.0,
        },
        steps: 600,
        deliver_period: 8,
        hold_equipment: false,
    },
    Scenario {
        id: "immunity_expiry",
        route: Route::Policy,
        shape: FixtureShape {
            home_family: ActionFamilyId::Unarmed,
            away_family: ActionFamilyId::Unarmed,
            separation_px: 24.0,
            row_spacing_px: 28.0,
        },
        steps: 480,
        deliver_period: 8,
        hold_equipment: false,
    },
];

/// Looks up a scenario by id. `Err` (not a panic) for an unknown id —
/// README rule 5.5: `id` is external JS input.
fn find_scenario(id: &str) -> Result<&'static Scenario, JsValue> {
    SCENARIOS
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| js_err(format!("'{id}' is not a canonical online combat phase")))
}

/// Every outfielder on a side carries the same family, so a scenario
/// exercises exactly one pair of them. Builds both the id-keyed pool
/// `gc_sim::r#match::new` wants and the plain slice `gc_sim::combat::new_state`
/// wants, from the one copy. Mirrors `pool_for`.
fn pool_for(
    shape: &FixtureShape,
) -> Result<(IndexMap<&'static str, PlayerData>, Vec<PlayerData>), JsValue> {
    let home =
        teams::get("nebula").ok_or_else(|| js_err("fixture team 'nebula' is not authored"))?;
    let mut by_id = IndexMap::new();
    let mut list = Vec::with_capacity(gc_data::players::ALL.len());
    for player in gc_data::players::ALL {
        let mut copied = *player;
        if copied.loadout_id.is_some() {
            let family = if home.roster.contains(&player.id) {
                shape.home_family
            } else {
                shape.away_family
            };
            copied.loadout_id = Some(loadout_for(family));
        }
        by_id.insert(player.id, copied);
        list.push(copied);
    }
    Ok((by_id, list))
}

/// Two facing lines centred on the pitch, one outfielder per row, with the
/// ball on the first away outfielder. Mirrors `pose`.
fn pose(state: &mut match_snapshot::MatchState, shape: &FixtureShape) {
    let spacing = shape.row_spacing_px;
    let top = FIELD.h / 2.0 - spacing * 1.5;
    for (order, &index) in HOME_OUTFIELD.iter().enumerate() {
        let player = &mut state.players[(index - 1) as usize];
        player.pos = Vec2::new(LINE_X, top + (order as f64) * spacing);
        player.facing = Vec2::new(1.0, 0.0);
        player.vel = Vec2::new(0.0, 0.0);
        player.run_vel = Vec2::new(0.0, 0.0);
    }
    for (order, &index) in AWAY_OUTFIELD.iter().enumerate() {
        let player = &mut state.players[(index - 1) as usize];
        player.pos = Vec2::new(LINE_X + shape.separation_px, top + (order as f64) * spacing);
        player.facing = Vec2::new(-1.0, 0.0);
        player.vel = Vec2::new(0.0, 0.0);
        player.run_vel = Vec2::new(0.0, 0.0);
    }
    let carrier = AWAY_OUTFIELD[0];
    state.owner = Some(carrier);
    state.ball = state.players[(carrier - 1) as usize].pos;
    state.ball_vel = Vec2::new(0.0, 0.0);
    // The kickoff hold refuses every equipment request, so a scenario that
    // spent it would spend its whole run waiting instead of fighting.
    state.kickoff_hold = 0.0;
}

/// Mirrors `capture_boundary_zero`.
fn capture_boundary_zero(
    shape: &FixtureShape,
    duration: Option<f64>,
) -> Result<MatchSnapshot, JsValue> {
    let (by_id, list) = pool_for(shape)?;
    let home =
        teams::get("nebula").ok_or_else(|| js_err("fixture team 'nebula' is not authored"))?;
    let away = teams::get("orion").ok_or_else(|| js_err("fixture team 'orion' is not authored"))?;
    let ownership = r#match::ownership_for_teams(home, away, Some(&by_id));
    let mut state = r#match::new(NewMatchOptions {
        home,
        away,
        field: FIELD,
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(duration.unwrap_or(DURATION_SECONDS)),
        // The goal limit is not part of any scenario: a phase fixture must
        // never end early on a scoreline while a correction is still the
        // thing under test.
        max_goals: Some(NO_GOAL_LIMIT),
        seed: Some(SEED),
        players_by_id: Some(&by_id),
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership),
    });
    pose(&mut state, shape);
    let combat_state: CombatMatchState = combat::new_state(&mut state, Some(&list));
    Ok(match_snapshot::capture(&state, Some(&combat_state)))
}

fn companion(snapshot: &MatchSnapshot) -> Result<&CombatMatchState, JsValue> {
    snapshot
        .combat
        .as_ref()
        .ok_or_else(|| js_err("a combat phase scenario needs a combat-bearing snapshot"))
}

fn any_phase(snapshot: &MatchSnapshot, phase: CombatPhase) -> Result<bool, JsValue> {
    Ok(companion(snapshot)?
        .players
        .iter()
        .any(|r| r.phase == phase))
}

fn any_event(events: &[gc_sim::combat_snapshot::CombatEvent], kind: CombatEventKind) -> bool {
    events.iter().any(|e| e.kind == kind)
}

/// Did input tick `tick` (the tick that produced `after` from `before`) run
/// through the named phase? Mirrors `combat_phases.observed`.
fn observed(
    id: &str,
    before: &MatchSnapshot,
    after: &MatchSnapshot,
    events: &[gc_sim::combat_snapshot::CombatEvent],
) -> Result<bool, JsValue> {
    match id {
        "windup" => any_phase(before, CombatPhase::Windup),
        "guard" => any_phase(before, CombatPhase::Guard),
        "contact" => Ok(any_event(events, CombatEventKind::Contact)),
        // Already in flight when the tick began, so the tick advanced it. A
        // projectile that appears only in `after` was spawned by this tick
        // and has not flown a pixel yet, which is a spawn rather than
        // flight.
        "projectile_flight" => Ok(!companion(before)?.projectiles.is_empty()),
        "stagger" => Ok(companion(before)?
            .players
            .iter()
            .any(|r| r.forced_state.is_some() && r.forced_ticks > 0)),
        "ball_spill" => Ok(any_event(events, CombatEventKind::BallSpill)),
        "immunity_expiry" => {
            // The expiry itself, not merely a live immunity: the counter has
            // to reach zero inside the resimulated tick.
            let entering = &companion(before)?.players;
            let leaving = &companion(after)?.players;
            Ok(entering
                .iter()
                .zip(leaving.iter())
                .any(|(runtime, next)| runtime.immunity_ticks > 0 && next.immunity_ticks == 0))
        }
        other => Err(js_err(format!(
            "'{other}' is not a canonical online combat phase"
        ))),
    }
}

/// Every canonical online combat phase id, in [`PHASES`]'s order —
/// `"windup"`, `"guard"`, `"contact"`, `"projectile_flight"`, `"stagger"`,
/// `"ball_spill"`, `"immunity_expiry"`.
#[wasm_bindgen(js_name = onlineCombatPhaseIds)]
#[must_use]
pub fn online_combat_phase_ids() -> Vec<String> {
    PHASES.iter().map(|id| (*id).to_string()).collect()
}

/// One phase's scenario metadata, as JSON: `route` (`"policy"` or
/// `"canonical_input"`), `steps` (driver steps the scenario needs to reach
/// the phase), `deliver_period` (deliver every Nth step; the burst that
/// corrects), `hold_equipment` (whether [`online_combat_phase_live_sample`]
/// authors anything for this phase).
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if `id` is not one of
/// [`online_combat_phase_ids`]'s ids.
#[wasm_bindgen(js_name = onlineCombatPhaseScenarioJson)]
pub fn online_combat_phase_scenario_json(id: &str) -> Result<String, JsValue> {
    let scenario = find_scenario(id)?;
    Ok(Json::obj(vec![
        ("id", Json::str(scenario.id)),
        ("route", Json::str(scenario.route.wire_str())),
        ("steps", Json::int(scenario.steps)),
        ("deliver_period", Json::int(scenario.deliver_period)),
        ("hold_equipment", Json::bool(scenario.hold_equipment)),
    ])
    .to_json_string())
}

/// The shared, combat-active boundary zero for one phase, as an opaque
/// handle (never JSON — see [`WasmMatchSnapshot`]'s doc). Every peer in a
/// session must be built from the *same* snapshot: a differing one is a
/// desync fixture, not a correction fixture. `duration` is match seconds;
/// omit for the scenario's own default.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if `id` is not one of
/// [`online_combat_phase_ids`]'s ids, or the `nebula`/`orion` fixture teams
/// are missing from authored content (never expected against real content).
#[wasm_bindgen(js_name = onlineCombatPhaseBoundaryZero)]
pub fn online_combat_phase_boundary_zero(
    id: &str,
    duration: Option<f64>,
) -> Result<WasmMatchSnapshot, JsValue> {
    let scenario = find_scenario(id)?;
    let snapshot = capture_boundary_zero(&scenario.shape, duration)?;
    Ok(WasmMatchSnapshot::new(snapshot))
}

/// The live slot's input for one driver step, as a canonical
/// `gc_sim::input_frame` sample wire (`crate::input_frame_bridge`'s own
/// hex-wire convention). Driver index 1 is the host, whose opening live slot
/// is `home_1`; every other index gets a neutral sample. `step` is the
/// zero-based driver step.
///
/// Only the `guard` scenario authors anything: it presses equipment once and
/// then holds it, re-sending the press edge periodically so an interrupted
/// guard is raised again rather than leaving the rest of the run neutral.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if `id` is not one of
/// [`online_combat_phase_ids`]'s ids.
#[wasm_bindgen(js_name = onlineCombatPhaseLiveSample)]
pub fn online_combat_phase_live_sample(id: &str, step: f64, index: f64) -> Result<String, JsValue> {
    let scenario = find_scenario(id)?;
    let step = step as i64;
    let index = index as i64;
    if !scenario.hold_equipment || index != 1 {
        return input_frame::encode_sample(&input_frame::neutral_sample())
            .map_err(|err| js_err(err.to_string()));
    }
    let sample = input_frame::new_sample(input_frame::InputSampleOptions {
        held: Some(input_frame::HELD_EQUIPMENT),
        edges: Some(if step % 30 == 0 {
            input_frame::EDGE_EQUIPMENT_PRESSED
        } else {
            0
        }),
        ..Default::default()
    })
    .map_err(|err| js_err(err.to_string()))?;
    input_frame::encode_sample(&sample).map_err(|err| js_err(err.to_string()))
}

/// Did a resimulated input tick run through phase `id`? `before`/`after` are
/// this driver's own retained boundary snapshots (typically
/// `MatchDriverBridge.snapshotLookup(tick)`/`.snapshotLookup(tick + 1)`'s
/// `.snapshot`) for the tick boundary before/after the tick being asked
/// about; `combat_events_json` is that same tick's combat events, in the
/// exact shape `MatchDriverBridge.advance()`'s batch already embeds per
/// output under `combat_events` (`RollbackTickOutput.combat_events` on the
/// TypeScript side) — pass it straight through, no re-shaping needed.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if `id` is not one of
/// [`online_combat_phase_ids`]'s ids, `combat_events_json` fails to parse or
/// decode, or `before`/`after` are not combat-bearing snapshots (i.e. were
/// not built by [`online_combat_phase_boundary_zero`] or a driver seeded
/// from one).
#[wasm_bindgen(js_name = onlineCombatPhaseObserved)]
pub fn online_combat_phase_observed(
    id: &str,
    before: &WasmMatchSnapshot,
    after: &WasmMatchSnapshot,
    combat_events_json: &str,
) -> Result<bool, JsValue> {
    let parsed = Json::parse(combat_events_json).map_err(js_err)?;
    let items = parsed
        .as_array()
        .ok_or_else(|| js_err("combat_events_json must be a JSON array"))?;
    let mut events = Vec::with_capacity(items.len());
    for item in items {
        events.push(rollback_events_bridge::raw_combat_event_from_json(item).map_err(js_err)?);
    }
    observed(id, &before.snapshot, &after.snapshot, &events)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Proves [`online_combat_phase_boundary_zero`] is not merely
    /// decorative: a real
    /// [`crate::match_driver_bridge::MatchDriverBridge`] can actually be
    /// seeded from it (via that constructor's own
    /// `initial_snapshot_override` parameter) and step forward, exactly the
    /// "drive a real `MatchDriverBridge` from it" workflow this module's
    /// own doc promises `match_presentation.spec.ts`'s nine combat-phase
    /// cases. Also proves the property that workflow actually needs:
    /// `initial_snapshot_override` is consumed by value (see that
    /// parameter's own doc for why — `wasm-bindgen` has no
    /// `OptionFromWasmAbi` for a reference to a custom class), so a second
    /// peer in the same session is seeded by calling
    /// [`online_combat_phase_boundary_zero`] again with the same `(id,
    /// duration)` rather than by reusing one handle — proven here by
    /// building a `"host"` and a `"guest"` bridge from two independent
    /// calls and checking the resulting snapshots hash identically.
    #[test]
    fn boundary_zero_seeds_a_real_match_driver_bridge_end_to_end() {
        use crate::match_driver_bridge::MatchDriverBridge;
        use crate::match_driver_fixture_bridge::{
            match_driver_fixture_freeze_json, match_driver_fixture_manifest_json,
        };
        use crate::session::Session;

        let freeze_json = match_driver_fixture_freeze_json("1v1", None, None).unwrap();
        let manifest_json = match_driver_fixture_manifest_json("1v1").unwrap();
        // `session` is required by `MatchDriverBridge::new`'s signature but
        // never actually read once `initial_snapshot_override` is supplied
        // -- see that parameter's own doc.
        let session = Session::new(
            "nebula", "orion", 7.0, 20.0, 3, None, None, None, None, None,
        )
        .expect("the fixture team ids always construct a valid session");

        let host_handle = online_combat_phase_boundary_zero("contact", None)
            .expect("'contact' builds a boundary-zero snapshot");
        let guest_handle = online_combat_phase_boundary_zero("contact", None)
            .expect("'contact' builds a boundary-zero snapshot");
        assert_eq!(
            match_snapshot::hash(&host_handle.snapshot),
            match_snapshot::hash(&guest_handle.snapshot),
            "two calls with the same (id, duration) must be byte-identical -- \
             the property a caller relies on to seed more than one peer"
        );

        let mut host_bridge = MatchDriverBridge::new(
            &session,
            "host",
            gc_netcode::match_driver_fixture::HOST_PEER_ID,
            &freeze_json,
            &manifest_json,
            None,
            None,
            Some(host_handle),
        )
        .expect("a combat-phase boundary zero constructs a valid driver");

        let guest_id = gc_netcode::match_driver_fixture::guest_peer_id(1);
        let mut guest_bridge = MatchDriverBridge::new(
            &session,
            "guest",
            &guest_id,
            &freeze_json,
            &manifest_json,
            None,
            None,
            Some(guest_handle),
        )
        .expect("an independently-built, identical boundary zero constructs a second, guest-role driver");

        host_bridge.initialize_transport().expect("initializes");
        host_bridge
            .open_peer(&guest_id)
            .expect("opens the fixture guest slot");
        host_bridge.set_peer_connected(&guest_id);
        guest_bridge.initialize_transport().expect("initializes");

        let batch = host_bridge
            .advance(None)
            .expect("advances one tick from the combat-phase boundary zero");
        assert!(Json::parse(&batch).is_ok());
        assert!(guest_bridge.advance(None).is_ok());

        // The driver's own `initialSnapshotHandle` must report back the
        // exact snapshot it was seeded with, not a default one -- both
        // must be combat-bearing (nothing about seeding with a
        // combat-active override should have been silently dropped).
        assert!(
            host_bridge
                .initial_snapshot_handle()
                .snapshot
                .combat
                .is_some()
        );
    }

    #[test]
    fn every_phase_id_has_a_scenario_and_round_trips_through_scenario_json() {
        for id in PHASES {
            let scenario = find_scenario(id).unwrap();
            assert_eq!(scenario.id, id);
            let json = Json::parse(&online_combat_phase_scenario_json(id).unwrap()).unwrap();
            assert_eq!(json.field_str("id"), Some(id));
            assert!(json.field_i64("steps").unwrap() > 0);
            assert!(json.field_i64("deliver_period").unwrap() > 0);
        }
        assert_eq!(online_combat_phase_ids().len(), PHASES.len());
    }

    // `find_scenario`'s error path returns a `JsValue`, which
    // `wasm_bindgen::JsValue::from_str` cannot construct off the wasm32
    // target ("function not implemented on non-wasm32 targets", aborting
    // the whole native test process — see `crate::coordinator_bridge`'s
    // module doc for the established split this crate uses everywhere else
    // an exported function's error path returns `JsValue`).
    // `packages/wasm/src/online_combat_phases.spec.ts` covers
    // `onlineCombatPhaseScenarioJson`/`BoundaryZero`/`LiveSample`/`Observed`
    // rejecting an unrecognized phase id (README rule 5.5: `id` is external
    // JS input, and every one of those four is a recoverable `Result`, not
    // a panic).

    #[test]
    fn boundary_zero_is_a_combat_bearing_snapshot_for_every_phase() {
        for id in PHASES {
            let handle = online_combat_phase_boundary_zero(id, None)
                .unwrap_or_else(|_| panic!("phase '{id}' must build a boundary-zero snapshot"));
            assert!(
                handle.snapshot.combat.is_some(),
                "phase '{id}'s boundary zero must carry a combat companion"
            );
        }
    }

    #[test]
    fn live_sample_is_neutral_for_every_non_guard_phase_and_every_non_host_index() {
        let neutral = input_frame::encode_sample(&input_frame::neutral_sample()).unwrap();
        assert_eq!(
            online_combat_phase_live_sample("windup", 0.0, 1.0).unwrap(),
            neutral
        );
        assert_eq!(
            online_combat_phase_live_sample("guard", 0.0, 2.0).unwrap(),
            neutral,
            "only driver index 1 (the host) authors anything, even for 'guard'"
        );
    }

    #[test]
    fn guard_presses_equipment_on_the_host_and_holds_it() {
        let pressed = online_combat_phase_live_sample("guard", 0.0, 1.0).unwrap();
        let decoded = input_frame::decode_sample(&pressed).unwrap();
        assert_eq!(decoded.held, input_frame::HELD_EQUIPMENT);
        assert_eq!(decoded.edges, input_frame::EDGE_EQUIPMENT_PRESSED);

        // A step that is not a multiple of 30 still holds, but does not
        // re-press.
        let held_only = online_combat_phase_live_sample("guard", 1.0, 1.0).unwrap();
        let decoded_held = input_frame::decode_sample(&held_only).unwrap();
        assert_eq!(decoded_held.held, input_frame::HELD_EQUIPMENT);
        assert_eq!(decoded_held.edges, 0);
    }

    #[test]
    fn observed_reports_windup_from_the_before_snapshot() {
        let handle = online_combat_phase_boundary_zero("windup", None).unwrap();
        // Boundary zero for a `Policy` scenario has not committed to any
        // action yet -- nobody is in `windup` before the sim has run a
        // single tick -- so this only proves the plumbing (the call
        // succeeds and returns a bool), not a specific expected value; the
        // real phase-reaching claim is exercised end to end by
        // `match_presentation.spec.ts` through a live `MatchDriverBridge`.
        let result = online_combat_phase_observed("windup", &handle, &handle, "[]");
        assert!(result.is_ok());
    }

    // `online_combat_phase_observed`'s error paths (a malformed
    // `combat_events_json`, an unrecognized phase id) return a `JsValue`,
    // which cannot be constructed off the native (non-wasm32) test target
    // this module's own tests run under -- see this file's earlier note
    // (above the removed `an_unknown_phase_id_...` test) for the established
    // split every bridge in this crate follows for exported error paths.
    // `packages/wasm/src/online_combat_phases.spec.ts` covers
    // `onlineCombatPhaseObserved` rejecting a non-array/malformed
    // `combatEventsJson`.

    #[test]
    fn observed_reports_contact_from_a_synthetic_event() {
        let handle = online_combat_phase_boundary_zero("contact", None).unwrap();
        let contact_event = Json::obj(vec![
            ("kind", Json::str("contact")),
            ("tick", Json::int(0)),
            ("family_id", Json::Null),
            ("source_index", Json::Null),
            ("target_index", Json::Null),
            ("source_sequence", Json::Null),
            ("result", Json::Null),
            ("outcome", Json::Null),
            ("reason", Json::Null),
            ("terminal", Json::Null),
            ("x", Json::Number(0.0)),
            ("y", Json::Number(0.0)),
            ("interruption_ticks", Json::Null),
            ("displacement_px", Json::Null),
        ]);
        let events_json = Json::Array(vec![contact_event]).to_json_string();
        let observed_contact =
            online_combat_phase_observed("contact", &handle, &handle, &events_json).unwrap();
        assert!(observed_contact);

        let no_events = online_combat_phase_observed("contact", &handle, &handle, "[]").unwrap();
        assert!(!no_events);
    }
}
