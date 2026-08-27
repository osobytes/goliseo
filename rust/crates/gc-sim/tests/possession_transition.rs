//! Tests for `gc_sim::possession_transition`.
//!
//! Two sub-cases of "rejects malformed, unsupported, and leaked state" are
//! not expressible here and are dropped, not merely already-passing:
//!
//! - An out-of-alias team string (`"neutral"`/`"middle"`) passed to
//!   `observe`/`phase`. [`transitions::TransitionTeam`] is a Rust `enum`
//!   with exactly the `Home`/`Away` variants, so there is no value that
//!   could be constructed to exercise the rejection — the invariant is
//!   enforced by the type system instead of a runtime check.
//! - A `TransitionConfig` missing `counterattack`, and a negative
//!   `select_pressers` limit. [`transitions::TransitionConfig`] requires
//!   both fields at construction (a compile error, not a runtime one) and
//!   `select_pressers`'s `limit` is `u32`, so neither malformed value can
//!   exist to hand the function.
//!
//! Every other case, including the still-constructible malformed states
//! (wrong version, leaked elapsed, unheld/overheld hold, an orphaned or
//! detached turnover, negative deltas, negative window seconds, and
//! duplicate presser indices), uses `catch_unwind` the way
//! `tests/content_validation.rs` documents: `validate_state`'s `assert!`s
//! panic on an invariant violation, and `catch_unwind` observes that panic
//! the way a caller expecting a `Result` would observe an error.
//!
//! `gc_sim::possession_transition`'s `support_push` falls back to the
//! ordinary push (`1`) for a role the tactic system never authors.
//! [`gc_data::formations::FormationRole`] is a closed enum with no
//! "unauthored" variant to construct, so that one assertion inside "pushes
//! forwards hardest and defenders not at all on a counter-attack" has no
//! Rust analogue and is omitted; the rest of that test (the authored-role
//! ordering) is exercised in full.

use gc_data::formations::FormationRole;
use gc_data::tactics;
use gc_sim::brain;
use gc_sim::metrics;
use gc_sim::possession_transition::{self as transitions, TransitionTeam};
use std::panic::{AssertUnwindSafe, catch_unwind};

const TICK: f64 = 1.0 / 60.0;

fn near_eps(actual: f64, expected: f64, eps: f64) {
    assert!(
        (actual - expected).abs() <= eps,
        "expected ~{expected} (+/-{eps}), got {actual}"
    );
}

fn fails<F: FnOnce()>(f: F) -> bool {
    catch_unwind(AssertUnwindSafe(f)).is_err()
}

fn balanced_windows() -> transitions::TransitionWindows {
    let balanced = transitions::copy_windows(
        tactics::get("balanced")
            .expect("balanced tactic")
            .transition,
    );
    transitions::TransitionWindows {
        home: balanced,
        away: balanced,
    }
}

/// Hold `owner_team` until its possession is authoritative.
fn establish(state: &mut transitions::PossessionTransitionState, owner_team: TransitionTeam) {
    let ticks = (transitions::ESTABLISH_SECONDS / TICK).ceil() as u32;
    for _ in 0..ticks {
        state.observe(Some(owner_team), TICK);
    }
}

#[test]
// `PossessionTransitionState` is `Copy`, so `copy_state` is structurally
// independent of `state` already. The mutations below (proving the copy is
// not an alias of an original that later changes) are kept anyway as a
// regression guard on that invariant, which is why these assignments go
// unread.
#[allow(unused_assignments)]
fn possession_transition_state_starts_idle_and_copies_defensively() {
    let mut state = transitions::new_state();
    assert_eq!(state.version, transitions::VERSION);
    assert_eq!(state.last_team, None);
    assert_eq!(state.holding_team, None);
    assert_eq!(state.hold, 0.0);
    assert_eq!(state.turnover_team, None);
    assert_eq!(state.elapsed, 0.0);
    assert!(!transitions::active(&state));

    establish(&mut state, TransitionTeam::Home);
    let copy = transitions::copy_state(&state);
    state.hold = 0.0;
    state.holding_team = None;
    state.last_team = None;
    assert_eq!(copy.last_team, Some(TransitionTeam::Home));
    assert_eq!(copy.holding_team, Some(TransitionTeam::Home));
    assert_eq!(copy.hold, transitions::ESTABLISH_SECONDS);
}

#[test]
fn possession_transition_state_uses_the_same_settled_possession_threshold_the_metrics_collector_counts()
 {
    // One counted turnover is exactly one transition window. If these ever
    // diverge, `metrics`' `turnovers_per_min` stops describing the phases —
    // including in the frozen Outfield AI baseline, which records it.
    assert_eq!(transitions::ESTABLISH_SECONDS, metrics::SETTLE_HOLD);
}

#[test]
fn possession_transition_state_remembers_a_first_established_possession_without_inventing_a_turnover()
 {
    let mut state = transitions::new_state();
    establish(&mut state, TransitionTeam::Home);
    assert_eq!(state.last_team, Some(TransitionTeam::Home));
    assert_eq!(state.turnover_team, None);
    assert_eq!(state.elapsed, 0.0);
    assert!(!transitions::active(&state));
}

#[test]
fn possession_transition_state_waits_for_authoritative_possession_before_opening_a_transition() {
    let mut state = transitions::new_state();
    establish(&mut state, TransitionTeam::Home);

    // A touch is not possession: ownership flicker cannot open a window.
    state.observe(Some(TransitionTeam::Away), TICK);
    state.observe(None, TICK);
    state.observe(Some(TransitionTeam::Home), TICK);
    assert_eq!(state.turnover_team, None);
    assert_eq!(state.last_team, Some(TransitionTeam::Home));

    establish(&mut state, TransitionTeam::Away);
    assert_eq!(state.turnover_team, Some(TransitionTeam::Away));
    assert_eq!(state.last_team, Some(TransitionTeam::Away));
    assert_eq!(state.elapsed, 0.0);
    assert!(transitions::active(&state));
}

#[test]
fn possession_transition_state_saturates_the_hold_streak_at_the_establish_threshold() {
    let mut state = transitions::new_state();
    for _ in 0..600 {
        state.observe(Some(TransitionTeam::Home), TICK);
    }
    assert_eq!(state.hold, transitions::ESTABLISH_SECONDS);
}

#[test]
fn possession_transition_state_lets_a_loose_ball_pause_the_streak_without_erasing_the_prior_team() {
    let mut state = transitions::new_state();
    establish(&mut state, TransitionTeam::Home);
    state.observe(Some(TransitionTeam::Away), TICK);
    let partial = state.hold;
    assert!(partial > 0.0 && partial < transitions::ESTABLISH_SECONDS);

    for _ in 0..300 {
        state.observe(None, TICK);
    }
    assert_eq!(state.hold, partial, "loose frames must bridge, not accrue");
    assert_eq!(state.holding_team, Some(TransitionTeam::Away));
    assert_eq!(state.last_team, Some(TransitionTeam::Home));
    assert_eq!(state.turnover_team, None);
}

#[test]
fn possession_transition_state_resets_the_streak_when_the_other_team_touches_it() {
    let mut state = transitions::new_state();
    establish(&mut state, TransitionTeam::Home);
    for _ in 0..20 {
        state.observe(Some(TransitionTeam::Away), TICK);
    }
    state.observe(Some(TransitionTeam::Home), TICK);
    assert_eq!(state.holding_team, Some(TransitionTeam::Home));
    near_eps(state.hold, TICK, 1e-12);
    establish(&mut state, TransitionTeam::Away);
    assert_eq!(
        state.turnover_team,
        Some(TransitionTeam::Away),
        "a reset streak still reaches a real turnover"
    );
}

#[test]
fn possession_transition_state_does_not_restart_the_same_transition_while_the_ball_keeps_changing_hands()
 {
    let mut state = transitions::new_state();
    establish(&mut state, TransitionTeam::Home);
    establish(&mut state, TransitionTeam::Away);
    let windows = balanced_windows();
    state.advance(1.0, windows);
    near_eps(state.elapsed, 1.0, 1e-12);

    // Flicker back and forth: neither touch is authoritative, so the live
    // counter-press keeps decaying instead of resetting.
    for _ in 0..10 {
        state.observe(Some(TransitionTeam::Home), TICK);
        state.observe(Some(TransitionTeam::Away), TICK);
    }
    assert_eq!(state.turnover_team, Some(TransitionTeam::Away));
    near_eps(state.elapsed, 1.0, 1e-12);
}

#[test]
fn possession_transition_state_decays_and_retires_a_transition_at_the_widest_live_window() {
    let mut state = transitions::new_state();
    establish(&mut state, TransitionTeam::Home);
    establish(&mut state, TransitionTeam::Away);
    let windows = transitions::TransitionWindows {
        home: transitions::copy_windows(
            tactics::get("press_high")
                .expect("press_high tactic")
                .transition,
        ),
        away: transitions::copy_windows(
            tactics::get("balanced")
                .expect("balanced tactic")
                .transition,
        ),
    };
    // Away won it, so the limit is max(away counterattack, home counterpress).
    assert_eq!(transitions::window_limit(&state, windows), 3.0);

    state.advance(2.9, windows);
    assert!(transitions::active(&state));
    state.advance(0.1, windows);
    assert!(!transitions::active(&state));
    assert_eq!(state.elapsed, 0.0);
    assert_eq!(
        state.last_team,
        Some(TransitionTeam::Away),
        "retiring a window keeps the possession memory"
    );
    assert_eq!(transitions::window_limit(&state, windows), 0.0);
}

#[test]
fn possession_transition_state_clears_every_possession_memory_for_a_lifecycle_boundary() {
    let mut state = transitions::new_state();
    establish(&mut state, TransitionTeam::Home);
    establish(&mut state, TransitionTeam::Away);
    state.clear();
    assert_eq!(state.last_team, None);
    assert_eq!(state.holding_team, None);
    assert_eq!(state.hold, 0.0);
    assert_eq!(state.turnover_team, None);
    assert_eq!(state.elapsed, 0.0);

    // A cleared state cannot manufacture a turnover from the next possession.
    establish(&mut state, TransitionTeam::Home);
    assert_eq!(state.turnover_team, None);
}

#[test]
fn possession_transition_state_decays_each_team_through_brain_phase_into_ordinary_attack_and_defend()
 {
    let mut state = transitions::new_state();
    establish(&mut state, TransitionTeam::Home);
    establish(&mut state, TransitionTeam::Away);
    let balanced = transitions::copy_windows(
        tactics::get("balanced")
            .expect("balanced tactic")
            .transition,
    );
    assert_eq!(
        transitions::phase(
            &state,
            TransitionTeam::Away,
            Some(TransitionTeam::Away),
            balanced
        ),
        brain::TeamPhase::Counterattack
    );
    assert_eq!(
        transitions::phase(
            &state,
            TransitionTeam::Home,
            Some(TransitionTeam::Away),
            balanced
        ),
        brain::TeamPhase::Counterpress
    );
    // A spill inside the window keeps both phases: possession is loose, the
    // transition is not.
    assert_eq!(
        transitions::phase(&state, TransitionTeam::Away, None, balanced),
        brain::TeamPhase::Counterattack
    );
    assert_eq!(
        transitions::phase(&state, TransitionTeam::Home, None, balanced),
        brain::TeamPhase::Counterpress
    );

    state.advance(
        2.5,
        transitions::TransitionWindows {
            home: balanced,
            away: balanced,
        },
    );
    assert_eq!(
        transitions::phase(
            &state,
            TransitionTeam::Away,
            Some(TransitionTeam::Away),
            balanced
        ),
        brain::TeamPhase::Attack
    );
    assert_eq!(
        transitions::phase(
            &state,
            TransitionTeam::Home,
            Some(TransitionTeam::Away),
            balanced
        ),
        brain::TeamPhase::Defend
    );
    assert_eq!(
        transitions::phase(&state, TransitionTeam::Home, None, balanced),
        brain::TeamPhase::Loose
    );
}

#[test]
fn possession_transition_state_honours_the_authored_tactic_ordering() {
    let mut state = transitions::new_state();
    establish(&mut state, TransitionTeam::Home);
    establish(&mut state, TransitionTeam::Away);
    let balanced = transitions::copy_windows(
        tactics::get("balanced")
            .expect("balanced tactic")
            .transition,
    );
    let press = transitions::copy_windows(
        tactics::get("press_high")
            .expect("press_high tactic")
            .transition,
    );
    let counter =
        transitions::copy_windows(tactics::get("counter").expect("counter tactic").transition);

    assert!(
        press.counterpress > balanced.counterpress,
        "Press High presses longer"
    );
    assert_eq!(
        counter.counterpress, 0.0,
        "Counter Attack does not counter-press"
    );
    assert!(
        counter.counterattack > balanced.counterattack,
        "Counter Attack breaks longer"
    );

    state.elapsed = 2.7;
    assert_eq!(
        transitions::phase(
            &state,
            TransitionTeam::Home,
            Some(TransitionTeam::Away),
            press
        ),
        brain::TeamPhase::Counterpress
    );
    assert_eq!(
        transitions::phase(
            &state,
            TransitionTeam::Home,
            Some(TransitionTeam::Away),
            balanced
        ),
        brain::TeamPhase::Defend
    );
    state.elapsed = 0.0;
    assert_eq!(
        transitions::phase(
            &state,
            TransitionTeam::Home,
            Some(TransitionTeam::Away),
            counter
        ),
        brain::TeamPhase::Defend
    );
    assert_eq!(
        transitions::phase(
            &state,
            TransitionTeam::Away,
            Some(TransitionTeam::Away),
            counter
        ),
        brain::TeamPhase::Counterattack
    );
    state.elapsed = 2.7;
    assert_eq!(
        transitions::phase(
            &state,
            TransitionTeam::Away,
            Some(TransitionTeam::Away),
            counter
        ),
        brain::TeamPhase::Counterattack
    );
    assert_eq!(
        transitions::phase(
            &state,
            TransitionTeam::Away,
            Some(TransitionTeam::Away),
            balanced
        ),
        brain::TeamPhase::Attack
    );
}

#[test]
fn possession_transition_state_selects_at_most_the_nearest_two_eligible_counter_pressers() {
    let candidates = [
        transitions::TransitionPresserCandidate {
            player_index: 5,
            distance_cost: 30.0,
            eligible: None,
        },
        transitions::TransitionPresserCandidate {
            player_index: 2,
            distance_cost: 10.0,
            eligible: None,
        },
        transitions::TransitionPresserCandidate {
            player_index: 3,
            distance_cost: 20.0,
            eligible: None,
        },
        transitions::TransitionPresserCandidate {
            player_index: 4,
            distance_cost: 5.0,
            eligible: Some(false),
        },
    ];
    let chosen = transitions::select_pressers(&candidates, transitions::MAX_PRESSERS);
    assert_eq!(transitions::MAX_PRESSERS, 2);
    assert_eq!(chosen.len(), 2);
    assert_eq!(chosen[0], 2);
    assert_eq!(chosen[1], 3);

    // Fewer eligible players than the cap uses only what exists.
    assert_eq!(
        transitions::select_pressers(
            &[transitions::TransitionPresserCandidate {
                player_index: 9,
                distance_cost: 1.0,
                eligible: None
            }],
            2
        )
        .len(),
        1
    );
    assert_eq!(transitions::select_pressers(&[], 2).len(), 0);
    assert_eq!(
        transitions::select_pressers(
            &[transitions::TransitionPresserCandidate {
                player_index: 9,
                distance_cost: 1.0,
                eligible: Some(false)
            }],
            2
        )
        .len(),
        0
    );
}

#[test]
fn possession_transition_state_breaks_equal_counter_press_distances_by_player_index() {
    let chosen = transitions::select_pressers(
        &[
            transitions::TransitionPresserCandidate {
                player_index: 8,
                distance_cost: 12.0,
                eligible: None,
            },
            transitions::TransitionPresserCandidate {
                player_index: 3,
                distance_cost: 12.0,
                eligible: None,
            },
            transitions::TransitionPresserCandidate {
                player_index: 5,
                distance_cost: 12.0,
                eligible: None,
            },
        ],
        2,
    );
    assert_eq!(chosen[0], 3);
    assert_eq!(chosen[1], 5);
}

#[test]
fn possession_transition_state_pushes_forwards_hardest_and_defenders_not_at_all_on_a_counter_attack()
 {
    assert!(
        transitions::support_push(FormationRole::Fwd)
            > transitions::support_push(FormationRole::Wide)
    );
    assert!(
        transitions::support_push(FormationRole::Wide)
            > transitions::support_push(FormationRole::Mid)
    );
    assert!(
        transitions::support_push(FormationRole::Mid)
            > transitions::support_push(FormationRole::Def)
    );
    assert_eq!(transitions::support_push(FormationRole::Def), 1.0);
    // An unauthored role string ("keeper") also keeps the ordinary push of
    // 1, but that sub-case has no Rust analogue with a closed
    // `FormationRole` — see the module doc comment.
}

#[test]
fn possession_transition_state_rejects_malformed_unsupported_and_leaked_state() {
    let state = transitions::new_state();

    let mut wrong_version = transitions::copy_state(&state);
    wrong_version.version = transitions::VERSION + 1;
    assert!(fails(|| {
        let _ = transitions::copy_state(&wrong_version);
    }));

    let mut leaked = transitions::copy_state(&state);
    leaked.elapsed = 1.0;
    assert!(fails(|| {
        let _ = transitions::copy_state(&leaked);
    }));

    let mut unheld = transitions::copy_state(&state);
    unheld.hold = 1.0;
    assert!(fails(|| {
        let _ = transitions::copy_state(&unheld);
    }));

    let mut overheld = transitions::copy_state(&state);
    overheld.holding_team = Some(TransitionTeam::Home);
    overheld.hold = transitions::ESTABLISH_SECONDS + 1.0;
    assert!(fails(|| {
        let _ = transitions::copy_state(&overheld);
    }));

    let mut orphaned = transitions::copy_state(&state);
    orphaned.holding_team = Some(TransitionTeam::Home);
    orphaned.hold = transitions::ESTABLISH_SECONDS;
    orphaned.last_team = Some(TransitionTeam::Home);
    orphaned.turnover_team = Some(TransitionTeam::Away);
    assert!(fails(|| {
        let _ = transitions::copy_state(&orphaned);
    }));

    let mut detached = transitions::copy_state(&state);
    detached.last_team = Some(TransitionTeam::Home);
    assert!(fails(|| {
        let _ = transitions::copy_state(&detached);
    }));

    let mut negative_observe = transitions::new_state();
    assert!(fails(AssertUnwindSafe(|| {
        negative_observe.observe(Some(TransitionTeam::Home), -1.0);
    })));

    let mut negative_advance = transitions::new_state();
    assert!(fails(AssertUnwindSafe(|| {
        negative_advance.advance(-1.0, balanced_windows());
    })));

    assert!(fails(|| {
        let _ = transitions::copy_windows(gc_data::tactics::TransitionConfig {
            counterpress: -1.0,
            counterattack: 1.0,
        });
    }));

    assert!(fails(|| {
        let _ = transitions::select_pressers(
            &[
                transitions::TransitionPresserCandidate {
                    player_index: 2,
                    distance_cost: 1.0,
                    eligible: None,
                },
                transitions::TransitionPresserCandidate {
                    player_index: 2,
                    distance_cost: 2.0,
                    eligible: None,
                },
            ],
            2,
        );
    }));
}
