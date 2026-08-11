//! Information-leakage tests for the learning environment. These are the
//! point of the representative profile: an observation must not carry a
//! future tick, an opponent's same-tick input, hidden RNG, a resolver
//! verdict, or a post-action event label. Every negative assertion is
//! paired with a positive control on the privileged profile, so a test
//! that stops proving anything (because the scan broke) fails instead of
//! passing quietly.
//!
//! `gc_sim::r#match` and `gc_sim::env` both exist now, so — unlike the
//! placeholder this file replaces — every case below drives a real,
//! multi-tick episode instead of being stubbed.
//!
//! ## Leakage scanning: a `Debug`-dump search, not a generic walk
//!
//! Rust has no runtime reflection over an arbitrary struct tree, so
//! hand-writing a walker for every observation type (`EnvObservation`,
//! `EnvSlotView`, `EnvObservedSelf`, `MatchSnapshot`, ...) would be a large
//! amount of code whose only job would be to reproduce what
//! `#[derive(Debug)]` already does.
//!
//! Every type on the observation path already derives `Debug`, and derived
//! `Debug` output is a complete textual dump of every field name and value,
//! fully nested. So the tests below search `format!("{:?}", ...)` instead:
//!
//!   - **String leaves** (the resolver-verdict test's `"catch"`/`"Catch"`):
//!     plain [`str::contains`] on the `Debug` text. Collision risk is
//!     negligible for a distinctive word.
//!   - **Number leaves** (the RNG test): a *tokenizer* over the `Debug`
//!     text ([`numeric_tokens`]), splitting on any non-digit character, so
//!     the exact digit run for `rng` is checked for membership rather than
//!     a raw substring search — `str::contains` alone would let `"42"`
//!     match inside the unrelated `"1420"`, which is exactly the kind of
//!     false negative that would make this test worthless.
//!   - **Forbidden field *names*** (the "no private field anywhere" test):
//!     search for `" {name}:"` (a leading space, matching `Debug`'s
//!     `field: value` separator) rather than a bare substring — `"state:"`
//!     as a bare substring would false-positive inside the legitimate
//!     `forced_state:` field ([`EnvObservedEquipment::forced_state`]),
//!     which is presentation-appropriate telegraph data, not a private
//!     simulation detail.
//!
//! This exercises the property directly (does the specific value/name
//! appear anywhere in what this profile exposes?) against real data.

use gc_sim::env::{self, EnvErrorCode, EnvInstance, ReferenceConfigOverrides};
use gc_sim::env_action::{RawAction, RawValue};
use gc_sim::env_config;
use gc_sim::env_observation;
use gc_sim::input_frame::{self, HeldAction, InputFrame, InputSampleOptions};
use gc_sim::match_snapshot::SavePending;
use indexmap::IndexMap;
use std::collections::BTreeSet;

/// Names that may never appear anywhere inside a player-observable view:
/// private AI intent, resolver commitments, team scheme internals, and raw
/// authority.
const FORBIDDEN_KEYS: &[&str] = &[
    "rng",
    "seed",
    "save_pending",
    "save_vx",
    "save_timer",
    "save_style",
    "windup_shot",
    "outfield_decision",
    "pass_target",
    "marks",
    "marking",
    "press",
    "outfield_press",
    "keeper_release_kind",
    "keeper_release_state",
    "keeper_release_depth",
    "keeper_release_motion",
    "aerial_outcome",
    "aerial_style",
    "difficulty",
    "snapshot",
    "state",
    "combat_state",
    "action",
    "actions",
    "intent",
    "input",
    "frames",
    "tape",
];

/// Every maximal run of ASCII digits in `text`, as text. See the module
/// doc's "Leakage scanning: a `Debug`-dump search" section for why this —
/// not a bare substring search — is the right way to check for an
/// exact-value number leak.
fn numeric_tokens(text: &str) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            tokens.insert(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.insert(current);
    }
    tokens
}

fn reset(profile: &str) -> EnvInstance {
    let mut config = env::reference_config("soccer_only", None).unwrap();
    config.observation_profile = Some(profile.to_string());
    env::reset(&config, None).unwrap()
}

fn neutral_action() -> RawAction {
    RawAction::Table(IndexMap::new())
}

fn still(instance: &EnvInstance) -> IndexMap<i64, RawAction> {
    let mut actions = IndexMap::new();
    for &slot in &instance.controlled_slots {
        actions.insert(slot, neutral_action());
    }
    actions
}

fn sprint_action(slot: i64) -> IndexMap<i64, RawAction> {
    let mut table = IndexMap::new();
    let mut move_table = IndexMap::new();
    move_table.insert("x".to_string(), RawValue::Number(-1.0));
    table.insert("move".to_string(), RawValue::Table(move_table));
    let mut held_table = IndexMap::new();
    held_table.insert("sprint".to_string(), RawValue::Bool(true));
    table.insert("held".to_string(), RawValue::Table(held_table));
    let mut actions = IndexMap::new();
    actions.insert(slot, RawAction::Table(table));
    actions
}

fn move_action(slot: i64, x: f64) -> IndexMap<i64, RawAction> {
    let mut table = IndexMap::new();
    let mut move_table = IndexMap::new();
    move_table.insert("x".to_string(), RawValue::Number(x));
    table.insert("move".to_string(), RawValue::Table(move_table));
    let mut actions = IndexMap::new();
    actions.insert(slot, RawAction::Table(table));
    actions
}

fn tape_frames(diverge_tick: i64, ticks: i64) -> Vec<InputFrame> {
    (0..ticks)
        .map(|tick| {
            let mut frame = input_frame::neutral(tick).unwrap();
            if tick >= diverge_tick {
                // Only an opponent slot differs; the controlled slot is
                // untouched.
                frame.slots[4] = input_frame::new_sample(InputSampleOptions {
                    move_x: Some(input_frame::MOVE_SCALE),
                    move_y: None,
                    held: Some(HeldAction::Sprint.bit()),
                    edges: None,
                })
                .unwrap();
            }
            frame
        })
        .collect()
}

fn reset_with_tape(diverge_tick: i64, ticks: i64) -> EnvInstance {
    let mut config = env::reference_config(
        "soccer_only",
        Some(ReferenceConfigOverrides {
            seed: Some(21),
            duration: Some(4.0),
            ..Default::default()
        }),
    )
    .unwrap();
    let mut sources = Vec::with_capacity(input_frame::SLOT_COUNT as usize);
    for index in 1..=input_frame::SLOT_COUNT {
        sources.push(if index == 1 {
            env_config::RawSlotSource {
                kind: "policy".to_string(),
                seed: None,
                policy_id: Some("leakage-probe".to_string()),
            }
        } else if index == 5 {
            env_config::RawSlotSource {
                kind: "tape".to_string(),
                seed: None,
                policy_id: None,
            }
        } else {
            env_config::RawSlotSource {
                kind: "neutral".to_string(),
                seed: None,
                policy_id: None,
            }
        });
    }
    config.slot_sources = Some(sources);
    let frames = tape_frames(diverge_tick, ticks);
    env::reset(&config, Some(&frames)).unwrap()
}

#[test]
fn hides_the_authoritative_rng_state_that_the_privileged_profile_exposes() {
    let representative = reset("representative");
    let privileged = reset("privileged");
    let rng = representative.state.rng;
    assert_ne!(
        rng, 0,
        "the fixture must have a non-trivial RNG state to probe"
    );

    let rep_numbers = numeric_tokens(&format!("{:?}", env::observe(&representative)));
    assert!(
        !rep_numbers.contains(&rng.to_string()),
        "the representative observation must not carry the RNG"
    );

    let priv_numbers = numeric_tokens(&format!("{:?}", env::observe(&privileged)));
    assert!(
        priv_numbers.contains(&rng.to_string()),
        "positive control: the privileged profile carries it"
    );
}

#[test]
fn hides_resolver_verdicts_that_the_privileged_snapshot_still_records() {
    let mut representative = reset("representative");
    let mut privileged = reset("privileged");
    // A committed save verdict is exactly the kind of authoritative outcome
    // a player cannot read off the screen before it resolves.
    for instance in [&mut representative, &mut privileged] {
        for player in &mut instance.state.players {
            if player.is_keeper {
                player.save_pending = Some(SavePending::Catch);
            }
        }
    }

    let rep_debug = format!("{:?}", env::observe(&representative));
    assert!(
        !rep_debug.contains("Catch"),
        "the representative observation must not carry it"
    );

    let priv_debug = format!("{:?}", env::observe(&privileged));
    assert!(
        priv_debug.contains("Catch"),
        "positive control: the privileged snapshot carries it"
    );
}

#[test]
fn names_no_private_state_field_anywhere_in_a_player_observable_view() {
    let instance = reset("representative");
    let observation = env::observe(&instance);
    assert!(
        observation.player_observable,
        "the representative profile is player-observable"
    );
    let debug = format!("{observation:?}");
    for &name in FORBIDDEN_KEYS {
        let needle = format!(" {name}:");
        assert!(
            !debug.contains(&needle),
            "representative observation must not expose {name}"
        );
    }
    // Positive control: the scan does find the fields that are meant to be
    // there.
    assert!(
        debug.contains(" own:") && debug.contains(" ball:") && debug.contains(" opponents:"),
        "the scan reaches the view body"
    );
}

#[test]
fn tags_the_privileged_profile_as_neither_observable_nor_a_human_proxy() {
    let observation = env::observe(&reset("privileged"));
    assert!(!observation.player_observable);
    assert!(!observation.human_proxy_valid);
    assert!(
        !env_observation::profile_data(env_observation::EnvObservationProfile::Privileged)
            .human_proxy_valid
    );
    let view = observation.views[0].as_ref().unwrap();
    assert!(view.privileged.is_some(), "the privileged block is present");
}

// Divergence starts four ticks BEYOND the tick about to be simulated, which
// makes this a genuinely distinct property from the same-tick test below:
// it rules out lookahead at arbitrary depth, not just a one-tick peek.
// Boundary 4 is observed while the tapes still agree on ticks 4..7 and
// differ from tick 8.
#[test]
fn carries_no_future_tick_at_arbitrary_lookahead_depth() {
    let mut baseline = reset_with_tape(99, 16);
    let mut diverging = reset_with_tape(8, 16);
    for _ in 0..4 {
        let baseline_actions = still(&baseline);
        env::step(&mut baseline, &baseline_actions, None).unwrap();
        let diverging_actions = still(&diverging);
        env::step(&mut diverging, &diverging_actions, None).unwrap();
    }
    assert_eq!(baseline.tick, 4);
    assert_eq!(diverging.tick, 4);
    assert_eq!(
        env_observation::encode(&env::observe(&diverging)),
        env_observation::encode(&env::observe(&baseline)),
        "a boundary observation must not depend on tape rows four or more ticks ahead"
    );
    // Still identical after stepping through the ticks the tapes agree on,
    // so the equality above is not an artefact of a static fixture.
    for _ in 0..4 {
        let baseline_actions = still(&baseline);
        env::step(&mut baseline, &baseline_actions, None).unwrap();
        let diverging_actions = still(&diverging);
        env::step(&mut diverging, &diverging_actions, None).unwrap();
    }
    assert_eq!(baseline.tick, 8);
    assert_eq!(
        diverging.boundary_hashes[8], baseline.boundary_hashes[8],
        "the runs agree right up to the first differing row"
    );
    // And the divergence is real once the differing row is actually
    // consumed.
    let baseline_actions = still(&baseline);
    env::step(&mut baseline, &baseline_actions, None).unwrap();
    let diverging_actions = still(&diverging);
    env::step(&mut diverging, &diverging_actions, None).unwrap();
    assert_ne!(
        baseline.boundary_hashes[9], diverging.boundary_hashes[9],
        "the diverging tape must actually change the simulation"
    );
}

#[test]
fn carries_no_opponent_same_tick_input() {
    let mut quiet = reset_with_tape(99, 12);
    let mut loud = reset_with_tape(3, 12);
    for _ in 0..3 {
        let quiet_actions = still(&quiet);
        env::step(&mut quiet, &quiet_actions, None).unwrap();
        let loud_actions = still(&loud);
        env::step(&mut loud, &loud_actions, None).unwrap();
    }
    // The opponent slot's row for tick 3 differs between the two runs. The
    // controlled slot's boundary-3 observation must be identical: the
    // input for the tick about to be simulated is not observable.
    assert_eq!(
        env_observation::encode(&env::observe(&loud)),
        env_observation::encode(&env::observe(&quiet)),
        "the same-tick opponent row must not reach the observation"
    );
    let quiet_actions = still(&quiet);
    let quiet_result = env::step(&mut quiet, &quiet_actions, None).unwrap();
    let loud_actions = still(&loud);
    let loud_result = env::step(&mut loud, &loud_actions, None).unwrap();
    assert_ne!(
        quiet_result.boundary_hash, loud_result.boundary_hash,
        "the differing same-tick row must actually change the simulation"
    );
}

#[test]
fn cannot_leak_one_controlled_slots_choice_to_another_in_the_team_profile() {
    fn reset_two_slot() -> EnvInstance {
        let mut config = env::reference_config(
            "soccer_only",
            Some(ReferenceConfigOverrides {
                seed: Some(44),
                ..Default::default()
            }),
        )
        .unwrap();
        config.observation_profile = Some("team".to_string());
        let mut sources = Vec::with_capacity(input_frame::SLOT_COUNT as usize);
        for index in 1..=input_frame::SLOT_COUNT {
            sources.push(if index == 1 || index == 5 {
                env_config::RawSlotSource {
                    kind: "policy".to_string(),
                    seed: None,
                    policy_id: Some(format!("slot-{index}")),
                }
            } else {
                env_config::RawSlotSource {
                    kind: "neutral".to_string(),
                    seed: None,
                    policy_id: None,
                }
            });
        }
        config.slot_sources = Some(sources);
        env::reset(&config, None).unwrap()
    }
    let mut left = reset_two_slot();
    let mut right = reset_two_slot();
    assert_eq!(left.controlled_slots.len(), 2);
    let left_view = env_observation::encode_view(env::observe(&left).views[0].as_ref().unwrap());
    let right_view = env_observation::encode_view(env::observe(&right).views[0].as_ref().unwrap());
    assert_eq!(
        left_view, right_view,
        "slot 1's view is fixed by the boundary, not by slot 5"
    );

    // Slot 1's view is identical whichever way slot 5 is about to act, and
    // it names slot 5's player only through presented cues.
    let mut left_actions = move_action(1, 1.0);
    left_actions.extend(sprint_action(5));
    let left_result = env::step(&mut left, &left_actions, None).unwrap();
    let mut right_actions = move_action(1, 1.0);
    right_actions.extend(move_action(5, 0.0));
    let right_result = env::step(&mut right, &right_actions, None).unwrap();
    assert_ne!(
        left_result.boundary_hash, right_result.boundary_hash,
        "the two slot-5 choices must actually diverge the simulation"
    );

    // Simultaneity is enforced by the contract: a step needs every
    // controlled slot's action at once, so no policy can observe another's
    // choice first.
    let partial = move_action(1, 0.0);
    let partial_err = env::step(&mut left, &partial, None).unwrap_err();
    assert_eq!(partial_err.code, EnvErrorCode::MissingSlot);
    assert!(
        partial_err.message.contains('5'),
        "the reason names the slot with no action"
    );
}

#[test]
fn reports_only_confirmed_past_events_never_post_action_labels() {
    // Bots on the other slots make the fixture actually produce events.
    let mut config = env::reference_config(
        "soccer_only",
        Some(ReferenceConfigOverrides {
            seed: Some(12),
            duration: Some(6.0),
            ..Default::default()
        }),
    )
    .unwrap();
    let mut sources = vec![env_config::RawSlotSource {
        kind: "policy".to_string(),
        seed: None,
        policy_id: None,
    }];
    for index in 2..=input_frame::SLOT_COUNT {
        sources.push(env_config::RawSlotSource {
            kind: "bot".to_string(),
            seed: Some((100 + index) as f64),
            policy_id: None,
        });
    }
    config.slot_sources = Some(sources);
    let mut instance = env::reset(&config, None).unwrap();
    instance.state.kickoff_hold = 0.0;
    let view = env::observe(&instance).views[0].as_ref().unwrap().clone();
    assert_eq!(
        view.events.len(),
        0,
        "the reset boundary has no confirmed events yet"
    );
    let mut seen = 0;
    for _ in 0..240 {
        if instance.terminated || instance.truncated {
            break;
        }
        let step_actions = still(&instance);
        let result = env::step(&mut instance, &step_actions, None).unwrap();
        let stepped = result.observation.views[0].as_ref().unwrap();
        for event in &stepped.events {
            seen += 1;
            assert!(
                event.tick < instance.tick,
                "an observed event must belong to a completed tick"
            );
        }
    }
    assert!(seen > 0, "the fixture produced confirmed events to check");
}
