//! Tests for `gc_sim::env`.

use gc_data::{tactics, teams};
use gc_sim::env::{self, EnvErrorCode, EnvInstance, ReferenceConfigOverrides};
use gc_sim::env_action::{self, RawAction, RawValue};
use gc_sim::env_config;
use gc_sim::fixed_clock;
use gc_sim::input_frame::{self, EdgeAction, HeldAction};
use gc_sim::r#match as sim_match;
use gc_sim::match_snapshot::{self, PitchSize};
use gc_sim::replay;
use gc_sim::slot_input::{self, MatchSlotSource, MatchSlotSourceKind};
use gc_sim::tuning::Tuning;
use indexmap::IndexMap;

fn players_by_id() -> IndexMap<&'static str, gc_data::players::PlayerData> {
    gc_data::players::ALL.iter().map(|p| (p.id, *p)).collect()
}

fn reset(overrides: ReferenceConfigOverrides) -> EnvInstance {
    let config = env::reference_config("soccer_only", Some(overrides)).expect("valid fixture");
    env::reset(&config, None).expect("valid config")
}

/// A small builder mirroring the Lua spec's inline `EnvSlotAction` table
/// literals (`{ move = { x = 1, y = 0 }, held = { sprint = true }, edges =
/// { dash = true } }`).
#[derive(Default)]
struct ActionSpec {
    move_: Option<(f64, f64)>,
    held: &'static [&'static str],
    edges: &'static [&'static str],
}

fn raw_action(spec: ActionSpec) -> RawAction {
    let mut table = IndexMap::new();
    if let Some((x, y)) = spec.move_ {
        let mut move_table = IndexMap::new();
        move_table.insert("x".to_string(), RawValue::Number(x));
        move_table.insert("y".to_string(), RawValue::Number(y));
        table.insert("move".to_string(), RawValue::Table(move_table));
    }
    if !spec.held.is_empty() {
        let mut held_table = IndexMap::new();
        for &key in spec.held {
            held_table.insert(key.to_string(), RawValue::Bool(true));
        }
        table.insert("held".to_string(), RawValue::Table(held_table));
    }
    if !spec.edges.is_empty() {
        let mut edges_table = IndexMap::new();
        for &key in spec.edges {
            edges_table.insert(key.to_string(), RawValue::Bool(true));
        }
        table.insert("edges".to_string(), RawValue::Table(edges_table));
    }
    RawAction::Table(table)
}

fn neutral_action() -> RawAction {
    RawAction::Table(IndexMap::new())
}

fn actions_for(instance: &EnvInstance, action: Option<RawAction>) -> IndexMap<i64, RawAction> {
    let mut out = IndexMap::new();
    for &slot in &instance.controlled_slots {
        out.insert(slot, action.clone().unwrap_or_else(neutral_action));
    }
    out
}

/// The fixture `sim::env` builds from a config, spelled out independently
/// so a silent change to that construction breaks the equivalence test.
fn direct_match(seed: f64, duration: f64) -> match_snapshot::MatchState {
    let by_id = players_by_id();
    let home = teams::get("nebula").unwrap();
    let away = teams::get("orion").unwrap();
    let balanced = tactics::get("balanced").unwrap();
    sim_match::new(sim_match::NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: Some(balanced),
        away_tactic: Some(balanced),
        duration: Some(duration),
        // env_config::DEFAULT_MAX_GOALS since #268: no goal limit. Spelled
        // as a literal, like every other field here, so a silent change to
        // what the environment builds still breaks this equivalence.
        max_goals: Some(99),
        seed: Some(seed),
        players_by_id: Some(&by_id),
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: Some(false),
        input_ownership: Some(sim_match::ownership_for_teams(home, away, Some(&by_id))),
    })
}

// ---------------------------------------------------------------------------
// env::reset
// ---------------------------------------------------------------------------

#[test]
fn env_reset_builds_the_configured_fixture_and_pins_the_reset_boundary() {
    let instance = reset(ReferenceConfigOverrides {
        seed: Some(7),
        duration: Some(4.0),
        ..Default::default()
    });
    assert_eq!(instance.tick, 0);
    assert_eq!(instance.episode_ticks, 0);
    assert!(!instance.terminated);
    assert!(!instance.truncated);
    assert_eq!(instance.boundary_hashes.len(), 1);
    assert_eq!(instance.controlled_slots.len(), 1);
    assert_eq!(instance.controlled_slots[0], 1);
    assert_eq!(instance.config.version, 1);
    assert_eq!(instance.identity.tick_rate, fixed_clock::TICK_RATE as i64);
    assert_eq!(
        instance.boundary_hashes[0],
        match_snapshot::hash(&match_snapshot::capture(&direct_match(7.0, 4.0), None)),
        "the environment fixture is the plain sim fixture"
    );
}

#[test]
fn env_reset_rejects_a_config_that_cannot_be_honored() {
    let config = env_config::RawEnvConfig {
        build: Some("spec".to_string()),
        seed: Some(1.0),
        home_team_id: Some("nope".to_string()),
        ..Default::default()
    };
    let err = env::reset(&config, None).unwrap_err();
    assert_eq!(err.code, EnvErrorCode::UnknownContent);
    assert!(err.message.contains("nope"));
}

#[test]
fn env_reset_requires_recorded_rows_when_a_slot_is_tape_driven_and_rejects_stray_rows() {
    let mut config = env::reference_config("soccer_only", None).unwrap();
    config.slot_sources.as_mut().unwrap()[4] = env_config::RawSlotSource {
        kind: "tape".to_string(),
        seed: None,
        policy_id: None,
    };
    let missing = env::reset(&config, None).unwrap_err();
    assert_eq!(missing.code, EnvErrorCode::MissingTape);

    let stray = env::reference_config("soccer_only", None).unwrap();
    let frame = input_frame::neutral(0).unwrap();
    let unused = env::reset(&stray, Some(&[frame])).unwrap_err();
    assert_eq!(unused.code, EnvErrorCode::Malformed);
}

// ---------------------------------------------------------------------------
// env::step
// ---------------------------------------------------------------------------

#[test]
fn env_step_advances_exactly_one_canonical_tick_by_default() {
    let mut instance = reset(ReferenceConfigOverrides {
        seed: Some(7),
        duration: Some(4.0),
        ..Default::default()
    });
    let actions = actions_for(&instance, None);
    let result = env::step(&mut instance, &actions, None).unwrap();
    assert_eq!(result.ticks_simulated, 1);
    assert_eq!(result.tick, 1);
    assert_eq!(instance.tick, 1);
    assert_eq!(result.boundary_hashes.len(), 1);
    assert_eq!(result.action_wires.len(), 1);
    assert!(!result.terminated);
    assert!(!result.truncated);
    assert_eq!(result.termination, None);
    assert_eq!(result.truncation, None);
    assert_eq!(result.observation.tick, 1);
    assert_eq!(result.diagnostics.role, "evaluation");
    assert_eq!(
        result.diagnostics.channel,
        gc_sim::env_reward::EnvRewardChannelId::ExperienceProxyMetrics
    );
    assert_eq!(
        result.diagnostics.metrics, None,
        "metrics are produced once the episode ends"
    );
}

#[test]
fn env_step_advances_a_declared_number_of_ticks_firing_edges_only_on_the_first() {
    let mut instance = reset(ReferenceConfigOverrides {
        seed: Some(7),
        duration: Some(4.0),
        ..Default::default()
    });
    let actions = actions_for(
        &instance,
        Some(raw_action(ActionSpec {
            move_: Some((1.0, 0.0)),
            held: &["sprint"],
            edges: &["dash"],
        })),
    );
    let result = env::step(&mut instance, &actions, Some(3)).unwrap();
    assert_eq!(result.ticks_simulated, 3);
    assert_eq!(result.tick, 3);
    assert_eq!(result.boundary_hashes.len(), 3);
    for (index, wire) in result.action_wires.iter().enumerate() {
        let frame = input_frame::decode(wire).unwrap();
        let sample = frame.slots[0];
        assert_eq!(frame.tick, index as i64);
        assert!(
            input_frame::is_held(&sample, HeldAction::Sprint).unwrap(),
            "held intents persist"
        );
        assert_eq!(
            input_frame::has_edge(&sample, EdgeAction::Dash).unwrap(),
            index == 0,
            "a one-shot edge fires on the first tick only"
        );
    }
}

#[test]
fn env_step_materializes_complete_rows_for_every_slot_including_unowned_ones() {
    let mut config = env::reference_config(
        "soccer_only",
        Some(ReferenceConfigOverrides {
            seed: Some(5),
            duration: Some(2.0),
            ..Default::default()
        }),
    )
    .unwrap();
    config.slot_sources.as_mut().unwrap()[2] = env_config::RawSlotSource {
        kind: "bot".to_string(),
        seed: Some(99.0),
        policy_id: None,
    };
    let mut instance = env::reset(&config, None).unwrap();
    let actions = actions_for(&instance, None);
    let result = env::step(&mut instance, &actions, None).unwrap();
    let frame = input_frame::decode(&result.action_wires[0]).unwrap();
    assert_eq!(frame.slots.len(), input_frame::SLOT_COUNT as usize);
    assert_eq!(instance.frames.len(), 1);
}

#[test]
fn env_step_refuses_an_illegal_action_with_a_reason_and_does_not_advance() {
    let mut instance = reset(ReferenceConfigOverrides {
        seed: Some(7),
        duration: Some(4.0),
        ..Default::default()
    });
    let actions = actions_for(
        &instance,
        Some(raw_action(ActionSpec {
            edges: &["switch"],
            ..Default::default()
        })),
    );
    let refused = env::step(&mut instance, &actions, None).unwrap_err();
    assert_eq!(refused.code, EnvErrorCode::IllegalAction);
    assert!(
        refused.message.contains("fixed-slot routing"),
        "the reason explains why player switching does not exist here"
    );
    assert_eq!(
        instance.tick, 0,
        "a refused action leaves the boundary untouched"
    );

    let mut unknown = IndexMap::new();
    unknown.insert(1, neutral_action());
    unknown.insert(4, neutral_action());
    let unknown_err = env::step(&mut instance, &unknown, None).unwrap_err();
    assert_eq!(unknown_err.code, EnvErrorCode::UnknownSlot);

    let malformed_actions = actions_for(
        &instance,
        Some(raw_action(ActionSpec {
            move_: Some((4.0, 4.0)),
            ..Default::default()
        })),
    );
    let malformed = env::step(&mut instance, &malformed_actions, None).unwrap_err();
    assert_eq!(malformed.code, EnvErrorCode::IllegalAction);
}

#[test]
fn env_step_separates_termination_from_truncation() {
    let mut instance = reset(ReferenceConfigOverrides {
        seed: Some(7),
        duration: Some(fixed_clock::TICK_SECONDS * 2.5),
        ..Default::default()
    });
    let mut final_result = None;
    for _ in 0..5 {
        let actions = actions_for(&instance, None);
        let result = env::step(&mut instance, &actions, None).unwrap();
        let done = result.terminated || result.truncated;
        final_result = Some(result);
        if done {
            break;
        }
    }
    let final_result = final_result.unwrap();
    assert!(final_result.terminated);
    assert!(!final_result.truncated);
    assert_eq!(
        final_result.termination,
        Some(gc_sim::env::EnvTerminationReason::TimeExpired)
    );
    assert_eq!(final_result.truncation, None);
    assert!(
        final_result.diagnostics.metrics.is_some(),
        "the episode end carries #128 metrics"
    );
    let view = final_result.observation.views[0].as_ref().unwrap();
    assert_eq!(
        final_result
            .diagnostics
            .metrics
            .as_ref()
            .unwrap()
            .goals_total,
        view.r#match.score_own + view.r#match.score_opponent
    );

    let actions = actions_for(&instance, None);
    let after = env::step(&mut instance, &actions, None).unwrap_err();
    assert_eq!(after.code, EnvErrorCode::EpisodeOver);
}

#[test]
fn env_step_truncates_on_the_episode_tick_budget_without_claiming_the_match_ended() {
    let mut instance = reset(ReferenceConfigOverrides {
        seed: Some(7),
        duration: Some(30.0),
        max_episode_ticks: Some(4),
        ..Default::default()
    });
    let mut final_result = None;
    for _ in 0..6 {
        let actions = actions_for(&instance, None);
        let result = env::step(&mut instance, &actions, None).unwrap();
        let done = result.terminated || result.truncated;
        final_result = Some(result);
        if done {
            break;
        }
    }
    let final_result = final_result.unwrap();
    assert!(final_result.truncated);
    assert!(!final_result.terminated);
    assert_eq!(
        final_result.truncation,
        Some(gc_sim::env::EnvTruncationReason::StepLimit)
    );
    assert_eq!(final_result.termination, None);
    assert_eq!(instance.episode_ticks, 4);
    assert!(
        final_result.diagnostics.metrics.is_some(),
        "truncation still reports diagnostics"
    );
}

#[test]
fn env_step_truncates_on_an_incomplete_tape_instead_of_faulting() {
    let config = env::reference_config(
        "soccer_only",
        Some(ReferenceConfigOverrides {
            seed: Some(8),
            duration: Some(30.0),
            ..Default::default()
        }),
    )
    .unwrap();
    let mut config = config;
    config.slot_sources.as_mut().unwrap()[4] = env_config::RawSlotSource {
        kind: "tape".to_string(),
        seed: None,
        policy_id: None,
    };
    let frames = [
        input_frame::neutral(0).unwrap(),
        input_frame::neutral(1).unwrap(),
    ];
    let mut instance = env::reset(&config, Some(&frames)).unwrap();
    let actions = actions_for(&instance, None);
    assert!(!env::step(&mut instance, &actions, None).unwrap().truncated);
    let actions = actions_for(&instance, None);
    assert!(!env::step(&mut instance, &actions, None).unwrap().truncated);
    let actions = actions_for(&instance, None);
    let exhausted = env::step(&mut instance, &actions, None).unwrap();
    assert!(exhausted.truncated);
    assert_eq!(
        exhausted.truncation,
        Some(gc_sim::env::EnvTruncationReason::TapeExhausted)
    );
    assert!(!exhausted.terminated);
    assert_eq!(exhausted.ticks_simulated, 0);
    assert_eq!(instance.episode_ticks, 2);
}

#[test]
fn env_step_reports_a_stoppage_distinctly_from_termination() {
    let mut instance = reset(ReferenceConfigOverrides {
        seed: Some(7),
        duration: Some(4.0),
        ..Default::default()
    });
    let actions = actions_for(&instance, None);
    let result = env::step(&mut instance, &actions, None).unwrap();
    assert!(
        result.stoppage,
        "the fixture opens in the post-kickoff hold"
    );
    assert_eq!(
        result.stoppage_reason,
        Some(gc_sim::env::EnvStoppageReason::KickoffHold)
    );
    assert!(!result.terminated);
    let view = result.observation.views[0].as_ref().unwrap();
    assert_eq!(
        view.r#match.phase,
        gc_sim::env_observation::EnvMatchPhase::Kickoff
    );
}

#[test]
fn env_step_surfaces_a_simulation_fault_as_a_reproducible_diagnostic() {
    let mut instance = reset(ReferenceConfigOverrides {
        seed: Some(7),
        duration: Some(4.0),
        ..Default::default()
    });
    // Corrupt the routing the simulation asserts on. A policy must never be
    // able to crash the trainer silently: the fault is returned with the
    // boundary hash and the exact input row that triggered it.
    instance.state.slot_players[2] = None;
    let actions = actions_for(&instance, None);
    let faulted = env::step(&mut instance, &actions, None).unwrap_err();
    assert_eq!(faulted.code, EnvErrorCode::SimFault);
    assert!(faulted.message.contains("simulation fault at tick 0"));
    assert!(faulted.message.contains(&instance.boundary_hashes[0]));
    let actions = actions_for(&instance, None);
    let again = env::step(&mut instance, &actions, None).unwrap_err();
    assert_eq!(again.code, EnvErrorCode::Faulted);
}

// ---------------------------------------------------------------------------
// env determinism and hash equivalence
// ---------------------------------------------------------------------------

#[test]
fn env_determinism_reproduces_boundary_hashes_for_the_same_config_and_action_tape() {
    let script: Vec<ActionSpec> = vec![
        ActionSpec {
            move_: Some((1.0, 0.0)),
            held: &["sprint"],
            ..Default::default()
        },
        ActionSpec {
            move_: Some((0.0, 1.0)),
            edges: &["dash"],
            ..Default::default()
        },
        ActionSpec {
            move_: Some((-1.0, 0.0)),
            ..Default::default()
        },
        ActionSpec {
            move_: Some((0.0, 0.0)),
            held: &["jockey"],
            ..Default::default()
        },
        ActionSpec {
            move_: Some((0.5, 0.5)),
            edges: &["pass"],
            ..Default::default()
        },
    ];
    fn run(seed: i64, script: &[ActionSpec]) -> EnvInstance {
        let mut instance = reset(ReferenceConfigOverrides {
            seed: Some(seed),
            duration: Some(4.0),
            ..Default::default()
        });
        for spec in script {
            let action = raw_action(ActionSpec {
                move_: spec.move_,
                held: spec.held,
                edges: spec.edges,
            });
            let actions = actions_for(&instance, Some(action));
            env::step(&mut instance, &actions, None).unwrap();
        }
        instance
    }
    let first = run(31, &script);
    let second = run(31, &script);
    assert_eq!(first.boundary_hashes.len(), script.len() + 1);
    for (index, hash) in first.boundary_hashes.iter().enumerate() {
        assert_eq!(
            &second.boundary_hashes[index], hash,
            "boundary {index} must reproduce"
        );
    }

    let other = run(32, &script);
    assert_ne!(
        other.boundary_hashes.last().unwrap(),
        first.boundary_hashes.last().unwrap(),
        "a different seed must diverge"
    );
}

#[test]
fn env_determinism_agrees_with_the_direct_sim_and_with_replay_on_every_boundary_hash() {
    fn scripted(index: i64) -> ActionSpec {
        ActionSpec {
            move_: Some((if index % 2 == 0 { 1.0 } else { -1.0 }, 0.0)),
            held: if index % 3 == 0 { &["sprint"] } else { &[] },
            edges: if index == 2 { &["dodge"] } else { &[] },
        }
    }

    let mut instance = reset(ReferenceConfigOverrides {
        seed: Some(17),
        duration: Some(4.0),
        ..Default::default()
    });
    for index in 1..=6 {
        let spec = scripted(index);
        let action = raw_action(ActionSpec {
            move_: spec.move_,
            held: spec.held,
            edges: spec.edges,
        });
        let actions = actions_for(&instance, Some(action));
        env::step(&mut instance, &actions, None).unwrap();
    }

    // Independent reconstruction: build a second fixture and a second
    // producer from the config alone and rematerialize every row from the
    // action script, so this leg never reads instance.frames. This is the
    // leg that would catch the environment materializing something other
    // than what its config and actions describe.
    let mut rebuilt_state = direct_match(17.0, 4.0);
    let mut rebuilt_producer = slot_input::new_producer([
        MatchSlotSource {
            kind: MatchSlotSourceKind::Frame,
            seed: None,
        },
        MatchSlotSource {
            kind: MatchSlotSourceKind::Neutral,
            seed: None,
        },
        MatchSlotSource {
            kind: MatchSlotSourceKind::Neutral,
            seed: None,
        },
        MatchSlotSource {
            kind: MatchSlotSourceKind::Neutral,
            seed: None,
        },
        MatchSlotSource {
            kind: MatchSlotSourceKind::Neutral,
            seed: None,
        },
        MatchSlotSource {
            kind: MatchSlotSourceKind::Neutral,
            seed: None,
        },
        MatchSlotSource {
            kind: MatchSlotSourceKind::Neutral,
            seed: None,
        },
        MatchSlotSource {
            kind: MatchSlotSourceKind::Neutral,
            seed: None,
        },
    ]);
    let tune = Tuning::new();
    let mut rebuilt = vec![match_snapshot::hash(&match_snapshot::capture(
        &rebuilt_state,
        None,
    ))];
    for index in 1..=6 {
        let mut slots = [input_frame::neutral_sample(); 8];
        for slot in slots.iter_mut() {
            *slot = input_frame::neutral_sample();
        }
        let spec = scripted(index);
        let action = env_action::validate(&raw_action(ActionSpec {
            move_: spec.move_,
            held: spec.held,
            edges: spec.edges,
        }))
        .unwrap();
        slots[0] = env_action::to_sample(&action).unwrap();
        let row = input_frame::new(rebuilt_state.input_tick, Some(slots)).unwrap();
        let (effective, _decisions) =
            slot_input::materialize(&mut rebuilt_producer, &rebuilt_state, &row, None);
        sim_match::step(
            &mut rebuilt_state,
            fixed_clock::TICK_SECONDS,
            sim_match::StepInput::Frame(&effective),
            None,
            &tune,
        );
        rebuilt.push(match_snapshot::hash(&match_snapshot::capture(
            &rebuilt_state,
            None,
        )));
    }
    assert_eq!(rebuilt.len(), instance.boundary_hashes.len());
    for (index, hash) in instance.boundary_hashes.iter().enumerate() {
        assert_eq!(
            &rebuilt[index], hash,
            "independently rematerialized boundary {index} must match"
        );
    }

    // Direct sim: the same fixture stepped with the effective rows the
    // environment materialized, through sim::r#match alone.
    let mut state = direct_match(17.0, 4.0);
    let mut direct = vec![match_snapshot::hash(&match_snapshot::capture(&state, None))];
    for frame in &instance.frames {
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            sim_match::StepInput::Frame(frame),
            None,
            &tune,
        );
        direct.push(match_snapshot::hash(&match_snapshot::capture(&state, None)));
    }
    assert_eq!(direct.len(), instance.boundary_hashes.len());
    for (index, hash) in instance.boundary_hashes.iter().enumerate() {
        assert_eq!(
            &direct[index], hash,
            "direct sim boundary {index} must match"
        );
    }

    // Replay: the exported tape carries the environment's own hashes, and
    // sim::replay re-derives them from the snapshot and the rows.
    let tape = env::tape(&instance).unwrap();
    let replayed = replay::run(&tape, &instance.identity, &instance.tune).unwrap();
    assert_eq!(replayed.divergence, None);
    assert_eq!(replayed.boundaries.len(), instance.boundary_hashes.len());
    for (index, row) in replayed.boundaries.iter().enumerate() {
        assert_eq!(
            row.hash, instance.boundary_hashes[index],
            "replay boundary {index}"
        );
    }
}

// ---------------------------------------------------------------------------
// env observation profiles and masks
// ---------------------------------------------------------------------------

#[test]
fn env_observation_serves_per_slot_views_for_a_multi_slot_team_fixture() {
    let mut config = env::reference_config(
        "soccer_only",
        Some(ReferenceConfigOverrides {
            seed: Some(12),
            duration: Some(4.0),
            ..Default::default()
        }),
    )
    .unwrap();
    config.observation_profile = Some("team".to_string());
    config.slot_sources.as_mut().unwrap()[4] = env_config::RawSlotSource {
        kind: "policy".to_string(),
        seed: None,
        policy_id: Some("away-probe".to_string()),
    };
    let instance = env::reset(&config, None).unwrap();
    assert_eq!(instance.controlled_slots.len(), 2);
    let observation = env::observe(&instance);
    assert_eq!(
        observation.profile,
        gc_sim::env_observation::EnvObservationProfile::Team
    );
    assert_eq!(observation.slots.len(), 2);
    let home_view = observation.views[0].as_ref().unwrap();
    let away_view = observation.views[4].as_ref().unwrap();
    assert_eq!(home_view.team, input_frame::Team::Home);
    assert_eq!(away_view.team, input_frame::Team::Away);
    assert_eq!(
        home_view.own.side,
        gc_sim::env_observation::EnvRelativeSide::Own
    );
    assert_eq!(home_view.teammates.len(), 4);
    assert_eq!(home_view.opponents.len(), 5);
    assert_eq!(
        home_view.r#match.score_own, away_view.r#match.score_opponent,
        "the two sides see mirrored scores"
    );
    assert_ne!(
        home_view.geometry.target_goal.x, away_view.geometry.target_goal.x,
        "each slot attacks its own target goal"
    );
}

#[test]
fn env_masks_publishes_a_client_knowable_mask_per_controlled_slot() {
    let instance = reset(ReferenceConfigOverrides {
        seed: Some(12),
        duration: Some(4.0),
        ..Default::default()
    });
    let masks = env::action_masks(&instance);
    let mask = masks.get(&1).unwrap();
    assert!(!mask.privileged);
    assert_eq!(
        mask.profile,
        env_action::EnvObservationProfile::Representative
    );
    assert!(!mask.edges.switch);
    assert!(mask.edges.shoot);
    assert!(
        !mask.held.equipment,
        "the soccer-only fixture has no combat loadout"
    );
    assert!(
        !mask.held.aerial_strike,
        "no airborne ball, no aerial intent"
    );
}

#[test]
fn env_masks_tags_a_privileged_mask_as_privileged() {
    let mut config = env::reference_config(
        "soccer_only",
        Some(ReferenceConfigOverrides {
            seed: Some(12),
            ..Default::default()
        }),
    )
    .unwrap();
    config.observation_profile = Some("privileged".to_string());
    let instance = env::reset(&config, None).unwrap();
    assert!(env::action_masks(&instance).get(&1).unwrap().privileged);
}

// ---------------------------------------------------------------------------
// env reference fixtures
// ---------------------------------------------------------------------------

#[test]
fn env_reference_fixtures_covers_a_soccer_only_environment_with_no_combat_state() {
    let instance = reset(ReferenceConfigOverrides {
        seed: Some(4),
        duration: Some(2.0),
        ..Default::default()
    });
    assert!(!instance.config.combat);
    assert!(instance.combat.is_none());
    assert_eq!(instance.identity.combat, None);
    assert_eq!(instance.identity.tape_version, gc_sim::input_tape::VERSION);
    let observation = env::observe(&instance);
    assert!(
        observation.views[0]
            .as_ref()
            .unwrap()
            .own
            .equipment
            .is_none()
    );
}

#[test]
fn env_reference_fixtures_covers_all_four_combat_families_in_one_environment() {
    let mut config = env::reference_config(
        "combat_all_families",
        Some(ReferenceConfigOverrides {
            seed: Some(91),
            duration: Some(4.0),
            ..Default::default()
        }),
    )
    .unwrap();
    config.observation_profile = Some("team".to_string());
    let mut sources = Vec::with_capacity(input_frame::SLOT_COUNT as usize);
    for index in 1..=input_frame::SLOT_COUNT {
        sources.push(env_config::RawSlotSource {
            kind: "policy".to_string(),
            seed: None,
            policy_id: Some(format!("family-probe-{index}")),
        });
    }
    config.slot_sources = Some(sources);
    let mut instance = env::reset(&config, None).unwrap();
    // Equipment cannot be committed during the post-restart hold, which is
    // a rule, not an environment limitation: clear it the way replay
    // fixtures do.
    instance.state.kickoff_hold = 0.0;
    assert!(
        instance.combat.is_some(),
        "the combat companion state exists"
    );
    assert!(
        instance.identity.combat.is_some(),
        "the tape identity pins combat mechanics"
    );
    assert_eq!(
        instance.identity.tape_version,
        gc_sim::input_tape::COMBAT_VERSION
    );

    let observation = env::observe(&instance);
    let mut families: Vec<gc_data::action_families::ActionFamilyId> = Vec::new();
    for &slot in &instance.controlled_slots {
        let view = observation.views[(slot - 1) as usize].as_ref().unwrap();
        let equipment = view.own.equipment.as_ref().unwrap();
        families.push(equipment.family_id);
        assert_eq!(
            equipment.phase,
            gc_sim::combat_feasibility::CombatActionPhase::Ready
        );
    }
    for family in [
        gc_data::action_families::ActionFamilyId::Unarmed,
        gc_data::action_families::ActionFamilyId::Guard,
        gc_data::action_families::ActionFamilyId::LightMelee,
        gc_data::action_families::ActionFamilyId::Ranged,
    ] {
        assert!(
            families.contains(&family),
            "the reference fixture exposes the {family:?} family"
        );
    }

    let actions = actions_for(
        &instance,
        Some(raw_action(ActionSpec {
            held: &["equipment"],
            edges: &["equipment_pressed"],
            ..Default::default()
        })),
    );
    let result = env::step(&mut instance, &actions, None).unwrap();
    for &slot in &instance.controlled_slots {
        let view = result.observation.views[(slot - 1) as usize]
            .as_ref()
            .unwrap();
        let equipment = view.own.equipment.as_ref().unwrap();
        assert_ne!(
            equipment.phase,
            gc_sim::combat_feasibility::CombatActionPhase::Ready,
            "the {:?} family left the ready phase",
            equipment.family_id
        );
    }
    assert_eq!(
        instance.controlled_slots.len(),
        input_frame::SLOT_COUNT as usize
    );
}

#[test]
fn env_reference_fixtures_reports_goal_cap_termination_separately_from_time_expiry() {
    let mut instance = reset(ReferenceConfigOverrides {
        seed: Some(3),
        duration: Some(30.0),
        max_goals: Some(1),
        ..Default::default()
    });
    // Roll a loose ball across the away goal line: a real goal through the
    // rules, reached without touching the resolver.
    instance.state.kickoff_hold = 0.0;
    instance.state.owner = None;
    instance.state.pickup_cd = 1.0;
    instance.state.ball =
        gc_core::vec2::Vec2::new(instance.state.field.w - 2.0, instance.state.field.h / 2.0);
    instance.state.ball_vel = gc_core::vec2::Vec2::new(1200.0, 0.0);
    instance.state.ball_z = 0.0;
    instance.state.ball_vz = 0.0;
    let actions = actions_for(&instance, None);
    let result = env::step(&mut instance, &actions, None).unwrap();
    assert!(result.terminated);
    assert_eq!(
        result.termination,
        Some(gc_sim::env::EnvTerminationReason::GoalCap)
    );
    assert!(!result.truncated);
    assert!(
        result.observation.views[0]
            .as_ref()
            .unwrap()
            .r#match
            .time_left
            > 0.0,
        "regulation time remained"
    );
}

// ---------------------------------------------------------------------------
// env::manifest
// ---------------------------------------------------------------------------

#[test]
fn env_manifest_names_every_input_a_reproduction_needs() {
    let mut config = env::reference_config(
        "soccer_only",
        Some(ReferenceConfigOverrides {
            seed: Some(66),
            duration: Some(4.0),
            ..Default::default()
        }),
    )
    .unwrap();
    {
        let sources = config.slot_sources.as_mut().unwrap();
        sources[0] = env_config::RawSlotSource {
            kind: "policy".to_string(),
            seed: None,
            policy_id: Some("scripted-v3".to_string()),
        };
        sources[5] = env_config::RawSlotSource {
            kind: "bot".to_string(),
            seed: Some(4242.0),
            policy_id: None,
        };
    }
    config.shaping_channels = Some(vec!["possession_gain".to_string()]);
    let mut instance = env::reset(&config, None).unwrap();
    let actions = actions_for(&instance, None);
    env::step(&mut instance, &actions, None).unwrap();
    let manifest = env::manifest(&instance);
    assert_eq!(manifest.env_version, env::VERSION);
    assert_eq!(manifest.seed, 66);
    assert_eq!(manifest.tick_rate, fixed_clock::TICK_RATE as i64);
    assert_eq!(
        manifest.observation_profile,
        env_config::EnvObservationProfile::Representative
    );
    assert_eq!(manifest.policy_ids.get(&1).unwrap(), "scripted-v3");
    assert_eq!(manifest.slot_sources[5].seed, Some(4242));
    assert_eq!(
        manifest.diagnostic_channel,
        gc_sim::env_reward::EnvRewardChannelId::ExperienceProxyMetrics
    );
    assert_eq!(
        manifest.objective_channels[0],
        gc_sim::env_reward::EnvRewardChannelId::MatchOutcome
    );
    assert_eq!(
        manifest.shaping_channels[0],
        gc_sim::env_reward::EnvRewardChannelId::PossessionGain
    );
    assert_eq!(manifest.initial_boundary_hash, instance.boundary_hashes[0]);
    assert_eq!(manifest.boundary_hash, instance.boundary_hashes[1]);
    assert_eq!(manifest.episode_ticks, 1);
    assert!(
        !manifest.config.contains("seed"),
        "the seed has its own manifest field"
    );
    assert!(manifest.config.contains("slots=1:policy#scripted-v3"));
    assert_eq!(
        manifest.tuning,
        Tuning::new().serialize(),
        "active tuning is pinned"
    );
}
