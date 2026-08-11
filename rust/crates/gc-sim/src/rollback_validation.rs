//! Pure OMP-2 validation campaign and deterministic scenario registry
//! adapter. Runtime clocks, process memory, browser identity, and
//! game-layer consumers remain outside this module.
//!
//! ## Scope note
//!
//! This module's own campaign machinery (`new_campaign`/`step_campaign`) is
//! complete — every suite ("native", "browser-full", "browser-stress",
//! "late-window", "soak") builds its case list from
//! `crate::determinism_evidence::fixture_tape`, `crate::rollback_lab`, and
//! `gc_data::omp2_rollback_validation`, all available. What is not exercised
//! by the test suite is the expensive suites themselves
//! (`plan_cases("native" | "soak")` alone builds dozens of multi-hundred-tick
//! cases): `tests/rollback_validation.rs` only runs the cheap, fully
//! self-contained assertions (`config()`, `profile_digest()`) plus the
//! two-case `"late-window"` suite that `new_campaign`/`step_campaign` use
//! for their own case. Report this precisely — the module is complete, the
//! expensive suites are unverified by that test run, not "blocked".
//!
//! ## No manual deep-copy helpers, as elsewhere
//!
//! Every payload type here is an owned Rust value with a real [`Clone`]
//! impl, matching `rollback_session`/`rollback_lab`/`rollback_playable_lab`.

use crate::combat;
use crate::combat_identity;
use crate::determinism_evidence;
use crate::fixed_clock;
use crate::input_frame::{self, InputSample, InputSampleOptions};
use crate::input_tape::{self, InputTape, InputTapeIdentity};
use crate::r#match::{self as sim_match, NewMatchOptions};
use crate::match_snapshot::{self, MatchSnapshot};
use crate::rollback_input_history::RollbackInputSource;
use crate::rollback_lab::{self, RollbackLabOptions, RollbackLabResult};
use crate::rollback_session::RollbackSessionMeasure;
use crate::tuning::Tuning;
use gc_core::fnv1a64::Fnv1a64State;
use gc_core::vec2::Vec2;
use gc_data::network_profiles;
use gc_data::omp2_rollback_validation::{
    self, Omp2RollbackCombatLoadFixture, Omp2RollbackLayout, Omp2RollbackScenarioKind,
    Omp2RollbackValidationData,
};
use gc_data::players::PlayerData;
use gc_data::teams;

/// Schema version for [`RollbackValidationResult`].
pub const SCHEMA: i64 = 1;

/// Which validation suite to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackValidationSuite {
    /// The full local/CI matrix.
    Native,
    /// One profile/seed pair, sharded for a browser run.
    BrowserFull,
    /// The stress-profile scenario matrix, sharded for a browser run.
    BrowserStress,
    /// The exact-window and one-tick-over-window boundary case.
    LateWindow,
    /// The extended soak matrix.
    Soak,
}

/// Construction options for [`new_campaign`].
#[derive(Default)]
pub struct RollbackValidationOptions {
    /// A profile name required by `BrowserFull`/`BrowserStress`.
    pub profile_name: Option<String>,
    /// A network seed required by `BrowserFull`/`BrowserStress`.
    pub network_seed: Option<i64>,
    /// Optional runner-owned wall-time observer; see
    /// `crate::rollback_lab`'s module doc comment on the same option.
    pub measure: Option<RollbackSessionMeasure>,
}

struct RollbackValidationCaseSpec {
    id: String,
    scenario: String,
    tape: InputTape,
    options: RollbackLabOptions,
    expected_failure: bool,
    sample: Option<String>,
}

/// One completed validation case.
pub struct RollbackValidationCompletedCase {
    /// Persistent case identity.
    pub id: String,
    /// The scenario this case covers.
    pub scenario: String,
    /// The case's initial boundary snapshot.
    pub initial_snapshot: MatchSnapshot,
    /// The underlying lab result.
    pub result: RollbackLabResult,
    /// Whether this case is expected to fail (an explicit terminal case).
    pub expected_failure: bool,
    /// Whether this case's outcome was accepted.
    pub accepted: bool,
    /// Whether a terminal probe found hidden progress.
    pub hidden_progress: bool,
    /// Whether the case covered the scenario it claims to.
    pub scenario_pass: bool,
    /// An optional named sample point.
    pub sample: Option<String>,
}

/// An incremental validation campaign.
pub struct RollbackValidationCampaign {
    /// Which suite this campaign runs.
    pub suite: RollbackValidationSuite,
    cases: Vec<RollbackValidationCaseSpec>,
    next_case: usize,
    active: Option<crate::rollback_lab::RollbackLabCampaign>,
    active_spec_index: Option<usize>,
    /// Completed case count.
    pub completed: i64,
    /// Whether any case so far was rejected.
    pub failed: bool,
    logical: Fnv1a64State,
    /// The completed result, present once every case has run.
    pub result: Option<RollbackValidationResult>,
}

/// The complete, immutable outcome of one validation campaign.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackValidationResult {
    /// Always [`SCHEMA`].
    pub schema: i64,
    /// Which suite produced this result.
    pub suite: RollbackValidationSuite,
    /// Whether every case was accepted.
    pub success: bool,
    /// Completed case count.
    pub case_count: i64,
    /// A digest over every completed case's logical marker, in order.
    pub logical_digest: String,
}

fn sources() -> [RollbackInputSource; 8] {
    let mut sources = [RollbackInputSource::Remote; 8];
    sources[0] = RollbackInputSource::Local;
    sources
}

fn state_at(source: &InputTape, first_boundary: i64, tune: &Tuning) -> match_snapshot::MatchState {
    let (mut state, mut combat_state) = match_snapshot::restore(&source.initial);
    for index in 0..first_boundary {
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            sim_match::StepInput::Frame(&source.frames[index as usize]),
            combat_state.as_mut(),
            tune,
        );
    }
    state
}

fn normalized_window(
    source: &InputTape,
    first_boundary: i64,
    last_boundary: i64,
    scenario: &str,
    tune: &Tuning,
) -> InputTape {
    assert!(
        first_boundary >= 0
            && last_boundary > first_boundary
            && last_boundary <= source.frames.len() as i64,
        "rollback validation window is outside the frozen tape"
    );
    let mut state = state_at(source, first_boundary, tune);
    state.input_tick = 0;
    state.events.clear();
    let initial = match_snapshot::capture(&state, None);
    let mut frames = Vec::new();
    for boundary in first_boundary..last_boundary {
        let mut frame = input_frame::copy(&source.frames[boundary as usize])
            .expect("tape frame is always valid");
        frame.tick = frames.len() as i64;
        input_frame::validate(&frame).expect("normalized window frame is always valid");
        frames.push(frame);
    }
    let mut identity =
        input_tape::copy_identity(&source.identity).expect("source identity is always valid");
    identity.build = "omp2-rollback-validation-v1".to_string();
    identity.source = format!("omp2-normalized-{scenario}");
    identity.fixture = format!("omp2-{scenario}");
    identity.config = format!("{};normalized_boundary={first_boundary}", identity.config);
    input_tape::new(&identity, &initial, &frames, tune)
        .expect("normalized window tape is always well formed")
}

fn synthetic_goal_tape(tune: &Tuning) -> InputTape {
    let home = teams::get("nebula").expect("nebula is authored");
    let away = teams::get("orion").expect("orion is authored");
    let ownership = sim_match::ownership_for_teams(home, away, None);
    let mut state = sim_match::new(NewMatchOptions {
        home,
        away,
        field: match_snapshot::PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(2.0),
        max_goals: Some(3),
        seed: Some(83.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership.clone()),
    });
    {
        let away_keeper = &mut state.players[5];
        away_keeper.keeper_state = crate::keeper::KeeperBehaviorState::Retreat;
        away_keeper.keeper_state_timer = 0.1;
        away_keeper.keeper_release_state = Some(crate::keeper::KeeperBehaviorState::Advance);
        away_keeper.keeper_release_motion = 0.5;
        away_keeper.keeper_release_kind = Some(crate::keeper::KeeperShotType::Chip);
        away_keeper.keeper_release_depth = 42.0;
        away_keeper.receive_timer = 1.0;
    }
    state.owner = None;
    state.ball = Vec2::new(965.0, 270.0);
    state.ball_vel = Vec2::new(600.0, 0.0);
    state.ball_z = 0.0;
    state.ball_vz = 0.0;
    state.pickup_cd = 1.0;
    state.block_grace = 1.0;
    let frames = vec![
        input_frame::neutral(0).expect("neutral frame is always valid"),
        input_frame::neutral(1).expect("neutral frame is always valid"),
        input_frame::neutral(2).expect("neutral frame is always valid"),
    ];
    let identity = InputTapeIdentity {
        tape_version: input_tape::VERSION,
        input_version: input_frame::VERSION,
        snapshot_version: match_snapshot::VERSION,
        build: "omp2-rollback-validation-v1".to_string(),
        source: "omp2-synthetic-goal-kickoff-v1".to_string(),
        content: "nebula-orion-showcase-content-v1".to_string(),
        tuning: tune.serialize(),
        config: "field=960x540;duration=2;max_goals=3;tick_rate=60".to_string(),
        fixture: "omp2-goal-kickoff".to_string(),
        seed: 83.0,
        tick_rate: fixed_clock::TICK_RATE as i64,
        ownership,
        combat: None,
    };
    let initial = match_snapshot::capture_owned(&state, None);
    input_tape::new(&identity, &initial, &frames, tune)
        .expect("synthetic goal tape is always well formed")
}

fn combat_validation_tape(config: &Omp2RollbackValidationData, tune: &Tuning) -> InputTape {
    let fixture = config.combat_fixture;
    let home = teams::get("nebula").expect("nebula is authored");
    let away = teams::get("orion").expect("orion is authored");
    let ownership = sim_match::ownership_for_teams(home, away, None);
    let mut state = sim_match::new(NewMatchOptions {
        home,
        away,
        field: match_snapshot::PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(20.0),
        max_goals: Some(99),
        seed: Some(fixture.seed as f64),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership.clone()),
    });
    state.kickoff_hold = 0.0;
    let combat_state = combat::new_state(&mut state, None);
    let initial = match_snapshot::capture(&state, Some(&combat_state));
    let mut frames = Vec::with_capacity(fixture.frame_count as usize);
    for tick in 0..fixture.frame_count {
        let mut frame = input_frame::neutral(tick).expect("neutral frame is always valid");
        let sample = if tick == 0 {
            Some(InputSampleOptions {
                held: Some(input_frame::HELD_EQUIPMENT),
                edges: Some(input_frame::EDGE_EQUIPMENT_PRESSED),
                ..Default::default()
            })
        } else if tick < 20 {
            Some(InputSampleOptions {
                held: Some(input_frame::HELD_EQUIPMENT),
                ..Default::default()
            })
        } else if tick == 20 {
            Some(InputSampleOptions {
                edges: Some(input_frame::EDGE_EQUIPMENT_RELEASED),
                ..Default::default()
            })
        } else {
            None
        };
        if let Some(options) = sample {
            let value = input_frame::new_sample(options).expect("valid sample");
            for slot in &mut frame.slots {
                *slot = value;
            }
        }
        frames.push(frame);
    }
    let identity = InputTapeIdentity {
        tape_version: input_tape::COMBAT_VERSION,
        input_version: input_frame::VERSION,
        snapshot_version: match_snapshot::COMBAT_VERSION,
        build: "omp2-combat-rollback-v1".to_string(),
        source: "issue-111-bounded-combat-v1".to_string(),
        content: "nebula-orion-showcase-content-v1".to_string(),
        tuning: tune.serialize(),
        config: format!(
            "field=960x540;duration=20;max_goals=99;tick_rate=60;ticks={}",
            fixture.frame_count
        ),
        fixture: fixture.id.to_string(),
        seed: fixture.seed as f64,
        tick_rate: fixed_clock::TICK_RATE as i64,
        ownership,
        combat: Some(combat_identity::for_state(Some(&combat_state))),
    };
    let tape = input_tape::new(&identity, &initial, &frames, tune)
        .expect("combat validation tape is always well formed");
    let actual_initial = &tape.boundary_hashes[0];
    let actual_final = tape
        .boundary_hashes
        .last()
        .expect("tapes always have at least one boundary hash");
    let actual_digest = rollback_lab::tape_digest(&tape);
    assert!(
        actual_initial == fixture.initial_hash
            && actual_final == fixture.final_hash
            && actual_digest == fixture.tape_digest,
        "combat validation identity changed: initial={actual_initial} final={actual_final} digest={actual_digest}"
    );
    tape
}

fn repeated_family_roster(loadout_id: &str) -> Vec<PlayerData> {
    gc_data::players::ALL
        .iter()
        .map(|player| PlayerData {
            id: player.id,
            name: player.name,
            number: player.number,
            position: player.position,
            stats: player.stats,
            presentation_id: player.presentation_id,
            cosmetic_variant_id: player.cosmetic_variant_id,
            loadout_id: if player.loadout_id.is_some() {
                Some(Box::leak(loadout_id.to_string().into_boxed_str()))
            } else {
                None
            },
        })
        .collect()
}

fn layout_positions(layout: Omp2RollbackLayout) -> [(f64, f64); 10] {
    match layout {
        Omp2RollbackLayout::Crowded => [
            (95.0, 270.0),
            (452.0, 240.0),
            (452.0, 290.0),
            (452.0, 200.0),
            (452.0, 340.0),
            (865.0, 270.0),
            (490.0, 288.0),
            (490.0, 242.0),
            (490.0, 202.0),
            (490.0, 338.0),
        ],
        Omp2RollbackLayout::Pocket => [
            (95.0, 270.0),
            (460.0, 210.0),
            (460.0, 250.0),
            (460.0, 290.0),
            (460.0, 330.0),
            (865.0, 270.0),
            (500.0, 212.0),
            (500.0, 252.0),
            (500.0, 292.0),
            (500.0, 332.0),
        ],
    }
}

struct CombatLoadSlotPlan {
    offset: i64,
    period: i64,
    hold: i64,
    move_x: i64,
    approach: i64,
}

fn combat_load_slot_plans(layout: Omp2RollbackLayout) -> [CombatLoadSlotPlan; 8] {
    match layout {
        Omp2RollbackLayout::Crowded => [
            CombatLoadSlotPlan {
                offset: 0,
                period: 40,
                hold: 30,
                move_x: 38,
                approach: 5,
            },
            CombatLoadSlotPlan {
                offset: 4,
                period: 46,
                hold: 1,
                move_x: 38,
                approach: 5,
            },
            CombatLoadSlotPlan {
                offset: 8,
                period: 60,
                hold: 20,
                move_x: 38,
                approach: 4,
            },
            CombatLoadSlotPlan {
                offset: 2,
                period: 30,
                hold: 1,
                move_x: 38,
                approach: 6,
            },
            CombatLoadSlotPlan {
                offset: 0,
                period: 40,
                hold: 30,
                move_x: -38,
                approach: 5,
            },
            CombatLoadSlotPlan {
                offset: 4,
                period: 46,
                hold: 1,
                move_x: -38,
                approach: 5,
            },
            CombatLoadSlotPlan {
                offset: 12,
                period: 60,
                hold: 20,
                move_x: -38,
                approach: 4,
            },
            CombatLoadSlotPlan {
                offset: 6,
                period: 30,
                hold: 1,
                move_x: -38,
                approach: 6,
            },
        ],
        Omp2RollbackLayout::Pocket => [
            CombatLoadSlotPlan {
                offset: 0,
                period: 30,
                hold: 1,
                move_x: 38,
                approach: 6,
            },
            CombatLoadSlotPlan {
                offset: 5,
                period: 30,
                hold: 1,
                move_x: 38,
                approach: 6,
            },
            CombatLoadSlotPlan {
                offset: 10,
                period: 30,
                hold: 1,
                move_x: 38,
                approach: 6,
            },
            CombatLoadSlotPlan {
                offset: 15,
                period: 30,
                hold: 1,
                move_x: 38,
                approach: 6,
            },
            CombatLoadSlotPlan {
                offset: 20,
                period: 30,
                hold: 1,
                move_x: -38,
                approach: 6,
            },
            CombatLoadSlotPlan {
                offset: 25,
                period: 30,
                hold: 1,
                move_x: -38,
                approach: 6,
            },
            CombatLoadSlotPlan {
                offset: 0,
                period: 30,
                hold: 1,
                move_x: -38,
                approach: 6,
            },
            CombatLoadSlotPlan {
                offset: 5,
                period: 30,
                hold: 1,
                move_x: -38,
                approach: 6,
            },
        ],
    }
}

fn combat_load_frames(fixture: &Omp2RollbackCombatLoadFixture) -> Vec<input_frame::InputFrame> {
    let plans = combat_load_slot_plans(fixture.layout);
    let mut frames = Vec::with_capacity(fixture.frame_count as usize);
    for tick in 0..fixture.frame_count {
        let mut frame = input_frame::neutral(tick).expect("neutral frame is always valid");
        for (index, plan) in plans.iter().enumerate() {
            assert!(
                plan.hold < plan.period,
                "combat load hold must end inside its period"
            );
            let mut held = 0;
            let mut edges = 0;
            let mut move_x = 0;
            let elapsed = tick - plan.offset;
            if elapsed >= 0 {
                let cycle = elapsed.rem_euclid(plan.period);
                if cycle == 0 {
                    held = input_frame::HELD_EQUIPMENT;
                    edges = input_frame::EDGE_EQUIPMENT_PRESSED;
                } else if cycle < plan.hold {
                    held = input_frame::HELD_EQUIPMENT;
                } else if cycle == plan.hold {
                    edges = input_frame::EDGE_EQUIPMENT_RELEASED;
                }
                if cycle < plan.approach {
                    move_x = plan.move_x;
                }
            }
            frame.slots[index] = input_frame::new_sample(InputSampleOptions {
                move_x: Some(move_x),
                held: Some(held),
                edges: Some(edges),
                ..Default::default()
            })
            .expect("valid sample");
        }
        frames.push(frame);
    }
    frames
}

/// Build the pinned combat-load input tape for a frozen OMP-2 fixture.
///
/// `pub` per ARCHITECTURE.md §3 rule 6: `tests/combat_load_fixtures.rs` drives this
/// directly, and the alternative was reconstructing ~150 lines of fixture
/// setup inside the test. These fixtures exist to guard bit-exact hashes, so a
/// second independent construction path could silently diverge and pin the
/// wrong thing — which is worse than widening visibility inside an internal
/// crate.
pub fn combat_load_tape(fixture: &Omp2RollbackCombatLoadFixture, tune: &Tuning) -> InputTape {
    let layout = layout_positions(fixture.layout);
    let repeated_roster = fixture.repeated_loadout_id.map(repeated_family_roster);
    let home = teams::get("nebula").expect("nebula is authored");
    let away = teams::get("orion").expect("orion is authored");
    let ownership = sim_match::ownership_for_teams(home, away, None);
    let mut state = sim_match::new(NewMatchOptions {
        home,
        away,
        field: match_snapshot::PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(fixture.duration as f64),
        max_goals: Some(99),
        seed: Some(fixture.seed as f64),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership.clone()),
    });
    state.kickoff_hold = 0.0;
    state.owner = None;
    state.ball = Vec2::new(480.0, 470.0);
    state.ball_vel = Vec2::new(0.0, 0.0);
    for (index, player) in state.players.iter_mut().enumerate() {
        let (x, y) = layout[index];
        player.pos = Vec2::new(x, y);
        player.facing = Vec2::new(
            if player.team == match_snapshot::Team::Home {
                1.0
            } else {
                -1.0
            },
            0.0,
        );
    }

    let combat_state = if fixture.combat {
        Some(combat::new_state(&mut state, repeated_roster.as_deref()))
    } else {
        None
    };
    let initial = match_snapshot::capture(&state, combat_state.as_ref());
    let identity = InputTapeIdentity {
        tape_version: if combat_state.is_some() {
            input_tape::COMBAT_VERSION
        } else {
            input_tape::VERSION
        },
        input_version: input_frame::VERSION,
        snapshot_version: if combat_state.is_some() {
            match_snapshot::COMBAT_VERSION
        } else {
            match_snapshot::VERSION
        },
        build: fixture.id.to_string(),
        source: "issue-150-combat-load-v1".to_string(),
        content: "nebula-orion-showcase-content-v1".to_string(),
        tuning: tune.serialize(),
        config: format!(
            "field=960x540;duration={};max_goals=99;tick_rate=60;ticks={};layout={};loadout={}",
            fixture.duration,
            fixture.frame_count,
            match fixture.layout {
                Omp2RollbackLayout::Crowded => "crowded",
                Omp2RollbackLayout::Pocket => "pocket",
            },
            fixture.repeated_loadout_id.unwrap_or("roster"),
        ),
        fixture: fixture.id.to_string(),
        seed: fixture.seed as f64,
        tick_rate: fixed_clock::TICK_RATE as i64,
        ownership,
        combat: combat_state
            .as_ref()
            .map(|c| combat_identity::for_state(Some(c))),
    };
    let tape = input_tape::new(&identity, &initial, &combat_load_frames(fixture), tune)
        .expect("combat load tape is always well formed");
    let actual_initial = &tape.boundary_hashes[0];
    let actual_final = tape
        .boundary_hashes
        .last()
        .expect("tapes always have at least one boundary hash");
    let actual_digest = rollback_lab::tape_digest(&tape);
    assert!(
        actual_initial == fixture.initial_hash
            && actual_final == fixture.final_hash
            && actual_digest == fixture.tape_digest,
        "{} identity changed: initial={actual_initial} final={actual_final} digest={actual_digest}",
        fixture.id
    );
    tape
}

fn late_window_tape(tune: &Tuning) -> InputTape {
    let source = determinism_evidence::fixture_tape(tune)
        .expect("determinism evidence fixture tape is valid");
    let (state, combat) = match_snapshot::restore(&source.initial);
    let initial = match_snapshot::capture(&state, combat.as_ref());
    let mut frames = Vec::with_capacity(40);
    for tick in 0..40i64 {
        let mut slots = [InputSample::default(); 8];
        for slot in slots.iter_mut() {
            *slot = input_frame::neutral_sample();
        }
        slots[1] = input_frame::new_sample(InputSampleOptions {
            move_x: Some(if tick % 2 == 0 {
                input_frame::MOVE_SCALE
            } else {
                -input_frame::MOVE_SCALE
            }),
            ..Default::default()
        })
        .expect("valid sample");
        frames.push(input_frame::new(tick, Some(slots)).expect("canonical slots always validate"));
    }
    let mut identity =
        input_tape::copy_identity(&source.identity).expect("source identity is always valid");
    identity.build = "omp2-rollback-validation-v1".to_string();
    identity.source = "omp2-late-window-v1".to_string();
    identity.fixture = "omp2-late-window".to_string();
    identity.config = "field=960x540;duration=120;max_goals=3;tick_rate=60;ticks=40".to_string();
    input_tape::new(&identity, &initial, &frames, tune)
        .expect("late window tape is always well formed")
}

fn lab_options(
    profile_name: &str,
    network_seed: i64,
    measure: Option<RollbackSessionMeasure>,
) -> RollbackLabOptions {
    RollbackLabOptions {
        profile_name: Some(profile_name.to_string()),
        network_seed: Some(network_seed),
        sources: Some(sources()),
        measure,
        prevalidated_tape: true,
        ..Default::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn case_spec(
    id: String,
    scenario: &str,
    tape: InputTape,
    options: RollbackLabOptions,
    expected_failure: bool,
    sample: Option<String>,
) -> RollbackValidationCaseSpec {
    RollbackValidationCaseSpec {
        id,
        scenario: scenario.to_string(),
        tape,
        options,
        expected_failure,
        sample,
    }
}

/// This crate never carries a `measure` clone across cases (see
/// `rollback_lab`'s module doc comment on the same limitation); every case
/// in a plan built here runs unmeasured.
fn combat_load_case_prefix(scenario: &str) -> String {
    scenario.replace('_', "-")
}

fn scenario_cases(
    profile_name: &str,
    network_seed: i64,
    tune: &Tuning,
) -> Vec<RollbackValidationCaseSpec> {
    let source = determinism_evidence::fixture_tape(tune)
        .expect("determinism evidence fixture tape is valid");
    let goal = synthetic_goal_tape(tune);
    let mut cases = Vec::new();
    for scenario in omp2_rollback_validation::DATA.scenarios {
        let tape = if scenario.kind == Omp2RollbackScenarioKind::SyntheticGoal {
            clone_tape(&goal)
        } else {
            normalized_window(
                &source,
                scenario
                    .first_boundary
                    .expect("window scenarios always carry a first boundary"),
                scenario
                    .last_boundary
                    .expect("window scenarios always carry a last boundary"),
                scenario.id,
                tune,
            )
        };
        cases.push(case_spec(
            format!("scenario-{}-{profile_name}-{network_seed}", scenario.id),
            scenario.id,
            tape,
            lab_options(profile_name, network_seed, None),
            false,
            None,
        ));
    }
    cases
}

fn combat_load_cases(network_seed: i64, tune: &Tuning) -> Vec<RollbackValidationCaseSpec> {
    let stress_profile = omp2_rollback_validation::DATA.stress_profile;
    omp2_rollback_validation::DATA
        .combat_load_fixtures
        .iter()
        .map(|fixture| {
            case_spec(
                format!(
                    "{}-{stress_profile}-{network_seed}",
                    combat_load_case_prefix(fixture.scenario)
                ),
                fixture.scenario,
                combat_load_tape(fixture, tune),
                lab_options(stress_profile, network_seed, None),
                false,
                None,
            )
        })
        .collect()
}

/// Build the complete case plan for `suite`. Every suite's case list is
/// complete; see the module doc comment for which ones
/// `tests/rollback_validation.rs` actually exercises.
///
/// # Panics
///
/// Panics if `suite` requires `options.profile_name`/`.network_seed` and
/// either is missing (producer invariant, ARCHITECTURE.md §3 rule 5).
fn plan_cases(
    suite: RollbackValidationSuite,
    options: &RollbackValidationOptions,
    tune: &Tuning,
) -> Vec<RollbackValidationCaseSpec> {
    let mut cases = Vec::new();
    match suite {
        RollbackValidationSuite::Native => {
            let full = determinism_evidence::fixture_tape(tune)
                .expect("determinism evidence fixture tape is valid");
            let combat_tape = combat_validation_tape(&omp2_rollback_validation::DATA, tune);
            for profile_name in omp2_rollback_validation::DATA.full_profiles {
                for &network_seed in omp2_rollback_validation::DATA.network_seeds {
                    cases.push(case_spec(
                        format!("full-{profile_name}-{network_seed}"),
                        "complete_fixture",
                        clone_tape(&full),
                        lab_options(profile_name, network_seed, None),
                        false,
                        None,
                    ));
                    cases.push(case_spec(
                        format!("combat-{profile_name}-{network_seed}"),
                        "combat",
                        clone_tape(&combat_tape),
                        lab_options(profile_name, network_seed, None),
                        false,
                        None,
                    ));
                }
            }
            for &network_seed in omp2_rollback_validation::DATA.network_seeds {
                cases.extend(scenario_cases(
                    omp2_rollback_validation::DATA.stress_profile,
                    network_seed,
                    tune,
                ));
                cases.push(case_spec(
                    format!("combat-stress-evidence-{network_seed}"),
                    "combat",
                    clone_tape(&combat_tape),
                    lab_options(
                        omp2_rollback_validation::DATA.stress_profile,
                        network_seed,
                        None,
                    ),
                    false,
                    None,
                ));
                cases.extend(combat_load_cases(network_seed, tune));
            }
        }
        RollbackValidationSuite::BrowserFull => {
            let profile_name = options
                .profile_name
                .as_deref()
                .expect("browser-full requires a profile");
            let network_seed = options
                .network_seed
                .expect("browser-full requires a network seed");
            let full = determinism_evidence::fixture_tape(tune)
                .expect("determinism evidence fixture tape is valid");
            let combat_tape = combat_validation_tape(&omp2_rollback_validation::DATA, tune);
            cases.push(case_spec(
                format!("full-{profile_name}-{network_seed}"),
                "complete_fixture",
                full,
                lab_options(profile_name, network_seed, None),
                false,
                None,
            ));
            cases.push(case_spec(
                format!("combat-{profile_name}-{network_seed}"),
                "combat",
                combat_tape,
                lab_options(profile_name, network_seed, None),
                false,
                None,
            ));
        }
        RollbackValidationSuite::BrowserStress => {
            let profile_name = options
                .profile_name
                .clone()
                .unwrap_or_else(|| omp2_rollback_validation::DATA.stress_profile.to_string());
            let network_seed = options
                .network_seed
                .expect("browser-stress requires a network seed");
            let combat_tape = combat_validation_tape(&omp2_rollback_validation::DATA, tune);
            cases.extend(scenario_cases(&profile_name, network_seed, tune));
            cases.push(case_spec(
                format!("combat-stress-evidence-{network_seed}"),
                "combat",
                combat_tape,
                lab_options(&profile_name, network_seed, None),
                false,
                None,
            ));
            cases.extend(combat_load_cases(network_seed, tune));
        }
        RollbackValidationSuite::LateWindow => cases.extend(plan_late_window(tune)),
        RollbackValidationSuite::Soak => {
            let full = determinism_evidence::fixture_tape(tune)
                .expect("determinism evidence fixture tape is valid");
            let combat_tape = combat_validation_tape(&omp2_rollback_validation::DATA, tune);
            for (index, &network_seed) in omp2_rollback_validation::DATA
                .soak_network_seeds
                .iter()
                .enumerate()
            {
                cases.push(case_spec(
                    format!("combat-soak-{}-{network_seed}", index + 1),
                    "combat",
                    clone_tape(&combat_tape),
                    lab_options("playable", network_seed, None),
                    false,
                    None,
                ));
                cases.push(case_spec(
                    format!("soak-{}-{network_seed}", index + 1),
                    "complete_fixture",
                    clone_tape(&full),
                    lab_options("playable", network_seed, None),
                    false,
                    omp2_rollback_validation::DATA
                        .soak_samples
                        .get(index)
                        .map(|s| (*s).to_string()),
                ));
            }
        }
    }
    assert!(!cases.is_empty(), "rollback validation suite has no cases");
    cases
}

fn plan_late_window(tune: &Tuning) -> Vec<RollbackValidationCaseSpec> {
    let tape = late_window_tape(tune);
    let mut cases = Vec::new();
    for delay in [30i64, 31i64] {
        let profile = network_profiles::NetworkProfile {
            name: network_profiles::NetworkProfileName::Clean,
            base_delay_ticks: delay,
            jitter_min_ticks: 0,
            jitter_max_ticks: 0,
            independent_loss_rate: 0.0,
            duplication_rate: 0.0,
            burst_start_rate: 0.0,
            burst_length_ticks: 0,
        };
        let mut options = lab_options(&format!("delay_{delay}"), delay, None);
        options.profile = Some((&profile).into());
        cases.push(case_spec(
            format!("delay-{delay}"),
            "late_window",
            clone_tape(&tape),
            options,
            delay == 31,
            None,
        ));
    }
    cases
}

fn clone_tape(tape: &InputTape) -> InputTape {
    InputTape {
        version: tape.version,
        identity: tape.identity.clone(),
        initial: tape.initial.clone(),
        frames: tape.frames.clone(),
        boundary_hashes: tape.boundary_hashes.clone(),
    }
}

fn combat_load_by_scenario(scenario: &str) -> Option<&'static Omp2RollbackCombatLoadFixture> {
    omp2_rollback_validation::DATA
        .combat_load_fixtures
        .iter()
        .find(|fixture| fixture.scenario == scenario)
}

fn combat_load_covered(
    result: &RollbackLabResult,
    fixture: &Omp2RollbackCombatLoadFixture,
) -> bool {
    let expected_version = if fixture.combat {
        match_snapshot::COMBAT_VERSION
    } else {
        match_snapshot::VERSION
    };
    let combat_events = result.event_metrics.confirmed_combat_events;
    let combat_present =
        (fixture.combat && combat_events > 0) || (!fixture.combat && combat_events == 0);
    result.input_ticks == fixture.frame_count
        && result.initial_hash == fixture.initial_hash
        && result.reference_final_hash == fixture.final_hash
        && result.tape_digest == fixture.tape_digest
        && combat_present
        && result.reference_final_snapshot.version == expected_version
        && result.client_final_snapshot.version == expected_version
}

/// Whether a lab result covers the combat load scenario it claims to.
/// Exposed so a test can drive both fixture kinds against both zero and
/// non-zero confirmed combat events directly.
///
/// # Panics
///
/// Panics if `scenario` names no authored combat load fixture (producer
/// invariant, ARCHITECTURE.md §3 rule 5).
#[must_use]
pub fn combat_load_covered_for(result: &RollbackLabResult, scenario: &str) -> bool {
    let fixture = combat_load_by_scenario(scenario)
        .unwrap_or_else(|| panic!("unknown combat load scenario: {scenario}"));
    combat_load_covered(result, fixture)
}

fn scenario_covered(result: &RollbackLabResult, scenario: &str) -> bool {
    if scenario == "complete_fixture" || scenario == "late_window" {
        return true;
    }
    if let Some(fixture) = combat_load_by_scenario(scenario) {
        return combat_load_covered(result, fixture);
    }
    if scenario == "combat" {
        let fixture = omp2_rollback_validation::DATA.combat_fixture;
        return result.input_ticks == fixture.frame_count
            && result.initial_hash == fixture.initial_hash
            && result.reference_final_hash == fixture.final_hash
            && result.tape_digest == fixture.tape_digest
            && result.event_metrics.confirmed_combat_events > 0
            && result.reference_final_snapshot.version == match_snapshot::COMBAT_VERSION
            && result.client_final_snapshot.version == match_snapshot::COMBAT_VERSION;
    }
    if scenario == "repeated_rollback" {
        return result.metrics.rollback_count >= 2;
    }
    let mut previous_owner: Option<String> = None;
    for row in &result.event_trace {
        if let rollback_lab::RollbackLabEventTraceRow::ReferenceConfirmed { step } = row {
            if scenario == "possession_change" {
                let current = step.state.owner_id.clone();
                if let Some(previous) = &previous_owner
                    && current.as_ref() != Some(previous)
                {
                    return true;
                }
                previous_owner = current;
            }
            for event in &step.match_events {
                if let crate::rollback_events::RollbackEventPayload::Match(match_event) =
                    &event.payload
                {
                    let kind = match_event.kind;
                    if (scenario == "tackle" && kind == match_snapshot::MatchEventKind::Tackle)
                        || (scenario == "shot" && kind == match_snapshot::MatchEventKind::Shot)
                        || (scenario == "aerial" && kind == match_snapshot::MatchEventKind::Header)
                        || (scenario == "keeper_action"
                            && kind == match_snapshot::MatchEventKind::Catch)
                    {
                        return true;
                    }
                }
            }
            for event in &step.lifecycle_events {
                if let crate::rollback_events::RollbackEventPayload::Lifecycle(payload) =
                    &event.payload
                {
                    let kind = payload.kind;
                    if (scenario == "goal"
                        && kind == crate::rollback_events::RollbackLifecycleKind::Goal)
                        || (scenario == "kickoff"
                            && kind == crate::rollback_events::RollbackLifecycleKind::Kickoff)
                        || (scenario == "full_time"
                            && kind == crate::rollback_events::RollbackLifecycleKind::FullTime)
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn complete_case(
    spec: &RollbackValidationCaseSpec,
    active: &mut crate::rollback_lab::RollbackLabCampaign,
    result: RollbackLabResult,
) -> RollbackValidationCompletedCase {
    let expected_terminal = spec.expected_failure
        && !result.success
        && result.status == rollback_lab::RollbackLabStatus::LateInputUnrecoverable
        && result.late_input_tick == Some(0);
    let scenario_pass = scenario_covered(&result, &spec.scenario);
    let hidden_progress = expected_terminal && rollback_lab::probe_terminal_stability(active);
    let accepted = (result.success || expected_terminal) && scenario_pass && !hidden_progress;
    RollbackValidationCompletedCase {
        id: spec.id.clone(),
        scenario: spec.scenario.clone(),
        initial_snapshot: spec.tape.initial.clone(),
        result,
        expected_failure: spec.expected_failure,
        accepted,
        hidden_progress,
        scenario_pass,
        sample: spec.sample.clone(),
    }
}

/// Construct a fresh validation campaign for `suite`.
///
/// # Panics
///
/// Panics if `suite` requires `options.profile_name`/`.network_seed` and
/// they are missing, or if `suite`'s case plan is empty (producer
/// invariants, ARCHITECTURE.md §3 rule 5). Every suite builds its full case plan; see
/// the module doc comment for which ones `tests/rollback_validation.rs`
/// actually exercises within this pass's time budget.
#[must_use]
pub fn new_campaign(
    suite: RollbackValidationSuite,
    options: RollbackValidationOptions,
) -> RollbackValidationCampaign {
    let tune = Tuning::new();
    let cases = plan_cases(suite, &options, &tune);
    RollbackValidationCampaign {
        suite,
        cases,
        next_case: 0,
        active: None,
        active_spec_index: None,
        completed: 0,
        failed: false,
        logical: Fnv1a64State::new(),
        result: None,
    }
}

/// Advance at most `max_ticks` authoritative input rows of the active case.
///
/// # Panics
///
/// Panics if `max_ticks` is not positive (producer invariant,
/// ARCHITECTURE.md §3 rule 5).
pub fn step_campaign(
    campaign: &mut RollbackValidationCampaign,
    max_ticks: i64,
) -> (
    Option<RollbackValidationResult>,
    Option<RollbackValidationCompletedCase>,
) {
    assert!(max_ticks > 0, "validation step must be positive");
    if let Some(result) = &campaign.result {
        return (Some(result.clone()), None);
    }
    if campaign.active.is_none() {
        let spec = &campaign.cases[campaign.next_case];
        campaign.active_spec_index = Some(campaign.next_case);
        campaign.active = Some(rollback_lab::new_campaign(
            clone_tape(&spec.tape),
            clone_options(&spec.options),
        ));
    }
    let active = campaign.active.as_mut().expect("just constructed above");
    let Some(result) = rollback_lab::step_campaign(active, max_ticks, None).cloned() else {
        return (None, None);
    };
    let spec_index = campaign
        .active_spec_index
        .expect("active campaign has a spec index");
    let completed = complete_case(&campaign.cases[spec_index], active, result);
    campaign.completed += 1;
    campaign.failed = campaign.failed || !completed.accepted;
    campaign.logical.update(completed.id.as_bytes());
    campaign.logical.update(b"\n");
    campaign.next_case += 1;
    campaign.active = None;
    campaign.active_spec_index = None;
    if campaign.next_case >= campaign.cases.len() {
        campaign.result = Some(RollbackValidationResult {
            schema: SCHEMA,
            suite: campaign.suite,
            success: !campaign.failed,
            case_count: campaign.completed,
            logical_digest: campaign.logical.hex(),
        });
    }
    (campaign.result.clone(), Some(completed))
}

fn clone_options(options: &RollbackLabOptions) -> RollbackLabOptions {
    RollbackLabOptions {
        profile_name: options.profile_name.clone(),
        profile: options.profile,
        network_seed: options.network_seed,
        sources: options.sources,
        max_rollback_ticks: options.max_rollback_ticks,
        drain_ticks: options.drain_ticks,
        corruption: options.corruption,
        measure: None,
        prevalidated_tape: options.prevalidated_tape,
    }
}

/// A stable marker string for one completed case.
#[must_use]
pub fn case_marker(completed: &RollbackValidationCompletedCase) -> String {
    let result = &completed.result;
    format!(
        "GC_ROLLBACK_VALIDATION|case|schema=1|case={}|scenario={}|fixture={}|profile={}|network_seed={}|success={}|lab_success={}|expected_failure={}|status={:?}|late_tick={}|hidden_progress={}|scenario_pass={}",
        completed.id,
        completed.scenario,
        result.fixture,
        result.profile,
        result.network_seed,
        i64::from(completed.accepted),
        i64::from(result.success),
        i64::from(completed.expected_failure),
        result.status,
        completed
            .result
            .late_input_tick
            .map_or("none".to_string(), |t| t.to_string()),
        i64::from(completed.hidden_progress),
        i64::from(completed.scenario_pass),
    )
}

/// A stable marker string for a completed campaign result.
#[must_use]
pub fn result_marker(result: &RollbackValidationResult) -> String {
    format!(
        "GC_ROLLBACK_VALIDATION|result|schema={}|suite={:?}|success={}|logical_digest={}|case_count={}",
        result.schema,
        result.suite,
        i64::from(result.success),
        result.logical_digest,
        result.case_count,
    )
}

/// The OMP-2 rollback validation fixture and budget register.
#[must_use]
pub fn config() -> &'static Omp2RollbackValidationData {
    &omp2_rollback_validation::DATA
}

fn network_profile_by_name(name: &str) -> Option<&'static network_profiles::NetworkProfile> {
    use network_profiles::NetworkProfileName::{Clean, Omp0Parity, Playable, Stress};
    let authored = match name {
        "clean" => Clean,
        "omp0_parity" => Omp0Parity,
        "playable" => Playable,
        "stress" => Stress,
        _ => return None,
    };
    Some(network_profiles::get(authored))
}

/// A digest over every authored full-suite network profile's parameters.
///
/// # Panics
///
/// Panics if `gc_data::omp2_rollback_validation::DATA.full_profiles` names
/// an unauthored profile (a data-consistency invariant, ARCHITECTURE.md §3 rule 5).
#[must_use]
pub fn profile_digest() -> String {
    let mut state = Fnv1a64State::new();
    for name in omp2_rollback_validation::DATA.full_profiles {
        let profile = network_profile_by_name(name)
            .unwrap_or_else(|| panic!("unknown network profile: {name}"));
        let line = format!(
            "{name}|{}|{}|{}|{}|{}|{}|{}\n",
            profile.base_delay_ticks,
            profile.jitter_min_ticks,
            profile.jitter_max_ticks,
            profile.independent_loss_rate,
            profile.duplication_rate,
            profile.burst_start_rate,
            profile.burst_length_ticks,
        );
        state.update(line.as_bytes());
    }
    state.hex()
}
