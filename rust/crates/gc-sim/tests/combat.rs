//! Port of `spec/sim/combat_spec.lua`.

use gc_core::vec2::Vec2;
use gc_data::action_families::{self, ActionFamilyId};
use gc_data::players::PlayerData;
use gc_sim::combat::{self, CombatContact};
use gc_sim::combat_snapshot::{
    self, CombatContactResult, CombatEncounterTerminal, CombatEvent, CombatEventKind,
    CombatForcedState, CombatMatchState, CombatPhase, CombatRequestOutcome,
    CombatRequestRejectionReason,
};
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{MatchInput, MatchState, PitchSize};
use gc_sim::slot_input;
use indexmap::IndexMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn new_match() -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    let mut s = sim_match::new(NewMatchOptions {
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
    s.kickoff_hold = 0.0;
    s
}

fn tick(
    state: &mut MatchState,
    combat_state: &mut CombatMatchState,
    player_index: i64,
    player_input: MatchInput,
) -> Vec<CombatContact> {
    let mut inputs = IndexMap::new();
    inputs.insert(player_index, player_input);
    combat::prepare_inputs(state, combat_state, &inputs, None);
    let contacts = combat::collect_contacts(state, combat_state);
    combat::resolve_contacts(state, combat_state, &contacts);
    combat::finish_tick(state, combat_state);
    contacts
}

/// Park everyone except `keep` far away.
fn move_other_players_away(state: &mut MatchState, keep: &[i64]) {
    for (zero_index, player) in state.players.iter_mut().enumerate() {
        let index = (zero_index + 1) as i64;
        if !keep.contains(&index) {
            player.pos = Vec2::new(800.0 + index as f64, 400.0 + index as f64);
        }
    }
}

fn event_of(events: &[CombatEvent], kind: CombatEventKind) -> Option<CombatEvent> {
    events.iter().find(|e| e.kind == kind).cloned()
}

fn events_of(events: &[CombatEvent], kind: CombatEventKind) -> Vec<CombatEvent> {
    events.iter().filter(|e| e.kind == kind).cloned().collect()
}

/// Combat clears its event array every tick, so a lifecycle assertion has to
/// keep its own copy of the confirmed stream.
fn drain(sink: &mut Vec<CombatEvent>, combat_state: &CombatMatchState) {
    sink.extend(combat_state.events.iter().cloned());
}

/// An opponent's action that is injected rather than simulated still opened
/// a sequence; the reconciliation below needs to know that before it sees
/// its rows.
fn declare_external_commit(sink: &mut Vec<CombatEvent>, source_sequence: i64) {
    sink.push(CombatEvent {
        kind: CombatEventKind::Commit,
        tick: 0,
        family_id: Some(ActionFamilyId::LightMelee),
        source_index: None,
        target_index: None,
        source_sequence: Some(source_sequence),
        result: None,
        outcome: Some(CombatRequestOutcome::Accepted),
        reason: None,
        terminal: None,
        x: 0.0,
        y: 0.0,
        interruption_ticks: None,
        displacement_px: None,
    });
}

/// The closed reconciliation: every accepted sequence closes exactly once,
/// keyed by source sequence, mapping to how many terminal rows closed it.
fn terminals_by_sequence(events: &[CombatEvent]) -> IndexMap<i64, i64> {
    let mut opened: IndexMap<i64, bool> = IndexMap::new();
    let mut closed: IndexMap<i64, i64> = IndexMap::new();
    for event in events {
        if event.kind == CombatEventKind::Commit {
            let sequence = event.source_sequence.expect("a commit names its sequence");
            assert!(
                !opened.contains_key(&sequence),
                "a source sequence was opened twice"
            );
            opened.insert(sequence, true);
            closed.insert(sequence, 0);
        } else if event.terminal.is_some() {
            let sequence = event
                .source_sequence
                .expect("a terminal names its sequence");
            assert!(
                opened.contains_key(&sequence),
                "a terminal closed an unopened sequence"
            );
            *closed.get_mut(&sequence).expect("checked above") += 1;
        }
    }
    closed
}

#[test]
fn combat_request_lifecycle_types_every_blocked_equipment_press_with_its_closed_rejection_reason() {
    let press = MatchInput {
        equipment_pressed: true,
        equipment_held: true,
        ..slot_input::neutral_match_input()
    };

    fn reject(
        prepare: impl FnOnce(&mut MatchState, &mut CombatMatchState) -> Option<IndexMap<i64, bool>>,
        player_index: i64,
        values: MatchInput,
    ) -> CombatEvent {
        let mut state = new_match();
        let mut combat_state = combat::new_state(&mut state, None);
        let ineligible = prepare(&mut state, &mut combat_state);
        let mut inputs = IndexMap::new();
        inputs.insert(player_index, values);
        combat::prepare_inputs(&mut state, &mut combat_state, &inputs, ineligible.as_ref());
        let rejected = events_of(&combat_state.events, CombatEventKind::RequestRejected);
        assert_eq!(rejected.len(), 1, "exactly one typed row per refused press");
        assert_eq!(
            combat_state.next_source_sequence, 1,
            "a rejection opens no encounter"
        );
        assert_eq!(
            combat_state.players[(player_index - 1) as usize].phase,
            CombatPhase::Ready
        );
        rejected[0].clone()
    }

    // A protected keeper and a player with no equipment are the same
    // physical refusal: nothing is equipped to request with.
    let keeper = reject(|_, _| None, 1, press);
    assert_eq!(
        keeper.reason,
        Some(CombatRequestRejectionReason::ProtectedKeeperOrNoLoadout)
    );
    assert_eq!(keeper.outcome, Some(CombatRequestOutcome::Rejected));
    assert_eq!(keeper.source_sequence, None);
    assert_eq!(keeper.terminal, None);

    let bare = reject(
        |_, combat_state| {
            combat_state.players[4].family_id = None;
            combat_state.players[4].loadout_id = None;
            None
        },
        5,
        press,
    );
    assert_eq!(
        bare.reason,
        Some(CombatRequestRejectionReason::ProtectedKeeperOrNoLoadout)
    );

    let held = reject(
        |state, _| {
            state.kickoff_hold = 1.0;
            None
        },
        5,
        press,
    );
    assert_eq!(held.reason, Some(CombatRequestRejectionReason::KickoffHold));

    let same_tick = reject(
        |_, _| None,
        5,
        MatchInput {
            equipment_pressed: true,
            shoot: true,
            ..slot_input::neutral_match_input()
        },
    );
    assert_eq!(
        same_tick.reason,
        Some(CombatRequestRejectionReason::SoccerCommitment)
    );

    let sliding = reject(
        |state, _| {
            state.players[4].slide_timer = 0.4;
            None
        },
        5,
        press,
    );
    assert_eq!(
        sliding.reason,
        Some(CombatRequestRejectionReason::SoccerCommitment)
    );

    let airborne = reject(
        |state, _| {
            state.players[4].aerial_timer = 0.3;
            None
        },
        5,
        press,
    );
    assert_eq!(
        airborne.reason,
        Some(CombatRequestRejectionReason::AerialStateOrRecovery)
    );

    let recovering = reject(
        |state, _| {
            state.players[4].aerial_recovery = 0.2;
            None
        },
        5,
        press,
    );
    assert_eq!(
        recovering.reason,
        Some(CombatRequestRejectionReason::AerialStateOrRecovery)
    );

    let declared = reject(
        |_, _| {
            let mut m = IndexMap::new();
            m.insert(5, true);
            Some(m)
        },
        5,
        press,
    );
    assert_eq!(
        declared.reason,
        Some(CombatRequestRejectionReason::AerialStateOrRecovery)
    );

    let forced = reject(
        |_, combat_state| {
            combat_state.players[4].forced_ticks = 8;
            combat_state.players[4].forced_state = Some(CombatForcedState::Stagger);
            None
        },
        5,
        press,
    );
    assert_eq!(
        forced.reason,
        Some(CombatRequestRejectionReason::ForcedState)
    );

    let cooling = reject(
        |_, combat_state| {
            combat_state.players[4].cooldown_ticks = 11;
            None
        },
        5,
        press,
    );
    assert_eq!(cooling.reason, Some(CombatRequestRejectionReason::Cooldown));

    // `already_committed` cannot use the shared helper: the player is
    // mid action, so its phase is deliberately not `ready`.
    let mut state = new_match();
    let mut combat_state = combat::new_state(&mut state, None);
    combat_state.players[4].phase = CombatPhase::Windup;
    combat_state.players[4].phase_ticks = 3;
    combat_state.players[4].source_sequence = Some(1);
    combat_state.next_source_sequence = 2;
    let mut inputs = IndexMap::new();
    inputs.insert(5, press);
    combat::prepare_inputs(&mut state, &mut combat_state, &inputs, None);
    let committed = events_of(&combat_state.events, CombatEventKind::RequestRejected);
    assert_eq!(committed.len(), 1);
    assert_eq!(
        committed[0].reason,
        Some(CombatRequestRejectionReason::AlreadyCommitted)
    );
    assert_eq!(
        combat_state.next_source_sequence, 2,
        "a rejection opens no encounter"
    );
}

#[test]
fn combat_request_lifecycle_records_nothing_without_a_press_edge_and_types_an_accepted_press() {
    let mut state = new_match();
    let mut combat_state = combat::new_state(&mut state, None);

    // No press is no request: a quiet tick owes no row, and neither does
    // a refused tick that never carried an edge.
    combat_state.players[4].cooldown_ticks = 9;
    let mut inputs = IndexMap::new();
    inputs.insert(
        5,
        MatchInput {
            equipment_held: true,
            ..slot_input::neutral_match_input()
        },
    );
    combat::prepare_inputs(&mut state, &mut combat_state, &inputs, None);
    assert_eq!(combat_state.events.len(), 0);

    combat_state.players[4].cooldown_ticks = 0;
    let mut inputs = IndexMap::new();
    inputs.insert(
        5,
        MatchInput {
            equipment_pressed: true,
            equipment_held: true,
            ..slot_input::neutral_match_input()
        },
    );
    combat::prepare_inputs(&mut state, &mut combat_state, &inputs, None);
    let commits = events_of(&combat_state.events, CombatEventKind::Commit);
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].outcome, Some(CombatRequestOutcome::Accepted));
    assert_eq!(commits[0].reason, None);
    assert_eq!(commits[0].terminal, None);
    assert_eq!(commits[0].source_sequence, Some(1));
    assert_eq!(
        events_of(&combat_state.events, CombatEventKind::RequestRejected).len(),
        0
    );
}

#[test]
fn combat_request_lifecycle_closes_an_accepted_sequence_exactly_once_for_every_family() {
    let expected = [
        (ActionFamilyId::Unarmed, CombatEncounterTerminal::Miss),
        (ActionFamilyId::LightMelee, CombatEncounterTerminal::Miss),
        (ActionFamilyId::Guard, CombatEncounterTerminal::Cancelled),
        (ActionFamilyId::Ranged, CombatEncounterTerminal::Expire),
    ];
    for (family_id, expected_terminal) in expected {
        let mut state = new_match();
        let mut combat_state = combat::new_state(&mut state, None);
        move_other_players_away(&mut state, &[5]);
        state.players[4].pos = Vec2::new(100.0, 100.0);
        state.players[4].facing = Vec2::new(1.0, 0.0);
        combat_state.players[4].family_id = Some(family_id);
        let mut seen: Vec<CombatEvent> = Vec::new();

        // Press, then release on the next tick so guard and ranged
        // resolve their held windows instead of hanging in
        // `guard`/`aim` forever.
        for tick_index in 1..=200 {
            tick(
                &mut state,
                &mut combat_state,
                5,
                MatchInput {
                    equipment_pressed: tick_index == 1,
                    equipment_held: tick_index == 1,
                    equipment_released: tick_index == 2,
                    ..slot_input::neutral_match_input()
                },
            );
            drain(&mut seen, &combat_state);
        }

        let closed = terminals_by_sequence(&seen);
        assert_eq!(
            closed.get(&1).copied(),
            Some(1),
            "{family_id:?} closes its sequence exactly once"
        );
        let mut terminal = None;
        for event in &seen {
            if event.terminal.is_some() {
                terminal = event.terminal;
            }
        }
        assert_eq!(terminal, Some(expected_terminal), "{family_id:?} terminal");
    }
}

#[test]
fn combat_request_lifecycle_closes_a_landed_melee_with_its_contact_and_never_adds_a_second_terminal()
 {
    let mut state = new_match();
    let mut combat_state = combat::new_state(&mut state, None);
    move_other_players_away(&mut state, &[5, 10]);
    state.players[4].pos = Vec2::new(100.0, 100.0);
    state.players[4].facing = Vec2::new(1.0, 0.0);
    state.players[9].pos = Vec2::new(120.0, 100.0);
    state.players[9].facing = Vec2::new(-1.0, 0.0);
    combat_state.players[4].family_id = Some(ActionFamilyId::Unarmed);
    let mut seen = Vec::new();

    for tick_index in 1..=120 {
        tick(
            &mut state,
            &mut combat_state,
            5,
            MatchInput {
                equipment_pressed: tick_index == 1,
                equipment_held: tick_index == 1,
                ..slot_input::neutral_match_input()
            },
        );
        drain(&mut seen, &combat_state);
    }

    let closed = terminals_by_sequence(&seen);
    assert_eq!(closed.get(&1).copied(), Some(1));
    let contacts = events_of(&seen, CombatEventKind::Contact);
    assert_eq!(contacts.len(), 1);
    assert_eq!(contacts[0].result, Some(CombatContactResult::Hit));
    assert_eq!(contacts[0].terminal, Some(CombatEncounterTerminal::Hit));
    assert_eq!(
        events_of(&seen, CombatEventKind::Miss).len(),
        0,
        "a landed swing never also misses"
    );
    // The consequence rows share the sequence but are not terminals.
    for kind in [CombatEventKind::Forced, CombatEventKind::BallSpill] {
        for event in events_of(&seen, kind) {
            assert_eq!(
                event.terminal, None,
                "{kind:?} is a consequence, not a terminal"
            );
        }
    }
}

#[test]
fn combat_request_lifecycle_interrupts_a_committed_action_once_and_only_when_it_still_owes_one() {
    let mut state = new_match();
    let mut combat_state = combat::new_state(&mut state, None);
    move_other_players_away(&mut state, &[5, 10]);
    state.players[4].pos = Vec2::new(100.0, 100.0);
    state.players[4].facing = Vec2::new(1.0, 0.0);
    state.players[9].pos = Vec2::new(400.0, 400.0);
    combat_state.players[4].family_id = Some(ActionFamilyId::LightMelee);

    tick(
        &mut state,
        &mut combat_state,
        5,
        MatchInput {
            equipment_pressed: true,
            equipment_held: true,
            ..slot_input::neutral_match_input()
        },
    );
    assert_eq!(combat_state.players[4].phase, CombatPhase::Windup);
    let mut seen = Vec::new();
    drain(&mut seen, &combat_state);
    declare_external_commit(&mut seen, 99);

    // A hit lands on the winding-up player: its own sequence closes as
    // `interrupted` because it never reached a contact of its own.
    combat::clear_events(&mut combat_state);
    let source_pos = state.players[9].pos;
    combat::resolve_contacts(
        &mut state,
        &mut combat_state,
        &[CombatContact {
            family_id: ActionFamilyId::LightMelee,
            source_index: 10,
            target_index: 5,
            source_sequence: 99,
            projectile_sequence: None,
            guarded: false,
            immune: false,
            source_pos,
        }],
    );
    drain(&mut seen, &combat_state);

    let interrupted = events_of(&seen, CombatEventKind::Interrupted);
    assert_eq!(interrupted.len(), 1);
    assert_eq!(interrupted[0].source_sequence, Some(1));
    assert_eq!(
        interrupted[0].terminal,
        Some(CombatEncounterTerminal::Interrupted)
    );
    assert_eq!(interrupted[0].source_index, Some(5));
    assert_eq!(terminals_by_sequence(&seen).get(&1).copied(), Some(1));
    assert_eq!(combat_state.players[4].phase, CombatPhase::Ready);
}

#[test]
fn combat_request_lifecycle_hands_the_terminal_to_a_spawned_projectile_instead_of_the_cancelled_action()
 {
    let mut state = new_match();
    let mut combat_state = combat::new_state(&mut state, None);
    move_other_players_away(&mut state, &[4, 10]);
    state.players[3].pos = Vec2::new(100.0, 100.0);
    state.players[3].facing = Vec2::new(1.0, 0.0);
    state.players[9].pos = Vec2::new(600.0, 400.0);
    assert_eq!(
        combat_state.players[3].family_id,
        Some(ActionFamilyId::Ranged)
    );
    let mut seen = Vec::new();

    let windup_ticks = action_families::get(ActionFamilyId::Ranged).windup_ticks;
    for tick_index in 1..=(windup_ticks + 1) {
        tick(
            &mut state,
            &mut combat_state,
            4,
            MatchInput {
                equipment_pressed: tick_index == 1,
                equipment_released: tick_index == 1,
                ..slot_input::neutral_match_input()
            },
        );
        drain(&mut seen, &combat_state);
    }
    assert_eq!(combat_state.projectiles.len(), 1);
    declare_external_commit(&mut seen, 99);

    // The shooter is knocked down while its shot is still flying. The
    // action is cancelled, but the encounter belongs to the projectile
    // now.
    combat::clear_events(&mut combat_state);
    let source_pos = state.players[9].pos;
    combat::resolve_contacts(
        &mut state,
        &mut combat_state,
        &[CombatContact {
            family_id: ActionFamilyId::LightMelee,
            source_index: 10,
            target_index: 4,
            source_sequence: 99,
            projectile_sequence: None,
            guarded: false,
            immune: false,
            source_pos,
        }],
    );
    drain(&mut seen, &combat_state);
    assert_eq!(
        events_of(&seen, CombatEventKind::Interrupted).len(),
        0,
        "a flying projectile still owns the terminal"
    );

    let lifetime = action_families::get(ActionFamilyId::Ranged)
        .projectile_lifetime_ticks
        .expect("the ranged family defines a projectile lifetime");
    for _ in 0..(lifetime + 2) {
        tick(
            &mut state,
            &mut combat_state,
            4,
            slot_input::neutral_match_input(),
        );
        drain(&mut seen, &combat_state);
    }
    assert_eq!(combat_state.projectiles.len(), 0);
    assert_eq!(terminals_by_sequence(&seen).get(&1).copied(), Some(1));
    let expired = events_of(&seen, CombatEventKind::ProjectileExpire);
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].terminal, Some(CombatEncounterTerminal::Expire));
}

#[test]
fn combat_request_lifecycle_round_trips_the_typed_request_and_terminal_rows_through_the_snapshot() {
    let mut state = new_match();
    let mut combat_state = combat::new_state(&mut state, None);
    combat_state.players[4].cooldown_ticks = 4;
    let mut inputs = IndexMap::new();
    inputs.insert(
        5,
        MatchInput {
            equipment_pressed: true,
            equipment_held: true,
            ..slot_input::neutral_match_input()
        },
    );
    combat::prepare_inputs(&mut state, &mut combat_state, &inputs, None);
    combat_state.events.push(CombatEvent {
        kind: CombatEventKind::Miss,
        tick: combat_state.tick,
        family_id: Some(ActionFamilyId::Unarmed),
        source_index: Some(5),
        target_index: None,
        source_sequence: Some(3),
        result: None,
        outcome: None,
        reason: None,
        terminal: Some(CombatEncounterTerminal::Miss),
        x: 12.0,
        y: 34.0,
        interruption_ticks: None,
        displacement_px: None,
    });

    let owned = combat::snapshot(&combat_state);
    assert_eq!(owned.version, combat_snapshot::VERSION);
    assert_eq!(
        combat_snapshot::first_difference(Some(&owned), Some(&combat_state), "combat"),
        None
    );
    assert_eq!(
        owned.events[0].reason,
        Some(CombatRequestRejectionReason::Cooldown)
    );
    assert_eq!(
        owned.events[0].outcome,
        Some(CombatRequestOutcome::Rejected)
    );
    assert_eq!(
        owned.events[1].terminal,
        Some(CombatEncounterTerminal::Miss)
    );

    // The restore path is fail-closed on both new vocabularies. Two Lua
    // sub-cases here mutate a plain table's `reason`/`terminal` field to
    // a string outside the closed vocabulary (`"unknown"`, `"extended"`)
    // and expect `combat_snapshot.copy` to reject it.
    // `CombatRequestRejectionReason` and `CombatEncounterTerminal` are
    // closed Rust enums with no such variant, so there is no value to
    // construct for either injection — they are dropped, not merely
    // already-passing (mirrors the pattern `tests/possession_transition.rs`
    // and `tests/content_validation.rs` document for the same shape of
    // case). Only the third sub-case — a *legal* reason paired with a
    // *legal* but contradictory outcome — has a Rust equivalent, and is
    // ported below.
    combat_state.tick = 1;
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            combat_snapshot::copy(&combat_state, &state)
        }))
        .is_ok()
    );

    let mut broken = combat::snapshot(&combat_state);
    broken.events[0].outcome = Some(CombatRequestOutcome::Accepted);
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            combat_snapshot::copy(&broken, &state)
        }))
        .is_err(),
        "a reason without a rejected outcome is malformed"
    );
}

#[test]
fn deterministic_combat_resolver_uses_exact_integer_phase_windows_and_pays_recovery_and_cooldown_on_misses()
 {
    for family_id in [ActionFamilyId::Unarmed, ActionFamilyId::LightMelee] {
        let mut state = new_match();
        let mut combat_state = combat::new_state(&mut state, None);
        combat_state.players[4].family_id = Some(family_id);
        let family = action_families::get(family_id);

        for tick_index in 1..=family.windup_ticks {
            tick(
                &mut state,
                &mut combat_state,
                5,
                MatchInput {
                    equipment_pressed: tick_index == 1,
                    equipment_held: tick_index == 1,
                    ..slot_input::neutral_match_input()
                },
            );
            if tick_index < family.windup_ticks {
                assert_eq!(
                    combat_state.players[4].phase,
                    CombatPhase::Windup,
                    "{family_id:?} windup"
                );
            }
        }
        assert_eq!(
            combat_state.players[4].phase,
            CombatPhase::Active,
            "{family_id:?} active starts after windup"
        );
        assert_eq!(combat_state.tick, family.windup_ticks);

        let active_ticks = family
            .active_ticks
            .expect("melee/unarmed families define active_ticks");
        for _ in 0..active_ticks {
            tick(
                &mut state,
                &mut combat_state,
                5,
                slot_input::neutral_match_input(),
            );
        }
        assert_eq!(
            combat_state.players[4].phase,
            CombatPhase::Recovery,
            "{family_id:?} miss enters recovery"
        );

        for _ in 0..family.recovery_ticks {
            tick(
                &mut state,
                &mut combat_state,
                5,
                slot_input::neutral_match_input(),
            );
        }
        assert_eq!(
            combat_state.players[4].phase,
            CombatPhase::Ready,
            "{family_id:?} recovery completes"
        );
        let commitment_ticks = family.windup_ticks + active_ticks + family.recovery_ticks;
        assert_eq!(
            combat_state.players[4].cooldown_ticks,
            (family.cooldown_ticks - commitment_ticks).max(0),
            "{family_id:?} cooldown starts at commit"
        );
        while combat_state.players[4].cooldown_ticks > 0 {
            tick(
                &mut state,
                &mut combat_state,
                5,
                slot_input::neutral_match_input(),
            );
        }
        assert_eq!(combat_state.players[4].cooldown_ticks, 0);
    }
}

#[test]
fn deterministic_combat_resolver_raises_and_lowers_guard_without_inventing_a_held_interval_for_a_fast_tap()
 {
    let mut state = new_match();
    let mut combat_state = combat::new_state(&mut state, None);
    assert_eq!(
        combat_state.players[1].family_id,
        Some(ActionFamilyId::Guard)
    );

    let guard = action_families::get(ActionFamilyId::Guard);
    for tick_index in 1..=guard.windup_ticks {
        tick(
            &mut state,
            &mut combat_state,
            2,
            MatchInput {
                equipment_pressed: tick_index == 1,
                equipment_released: tick_index == 1,
                equipment_held: false,
                ..slot_input::neutral_match_input()
            },
        );
    }
    assert_eq!(combat_state.players[1].phase, CombatPhase::Recovery);

    combat::reset(&mut combat_state);
    for tick_index in 1..=guard.windup_ticks {
        tick(
            &mut state,
            &mut combat_state,
            2,
            MatchInput {
                equipment_pressed: tick_index == 1,
                equipment_held: true,
                ..slot_input::neutral_match_input()
            },
        );
    }
    assert_eq!(combat_state.players[1].phase, CombatPhase::Guard);
    tick(
        &mut state,
        &mut combat_state,
        2,
        MatchInput {
            equipment_released: true,
            equipment_held: false,
            ..slot_input::neutral_match_input()
        },
    );
    assert_eq!(combat_state.players[1].phase, CombatPhase::Recovery);
}

#[test]
fn deterministic_combat_resolver_latches_an_early_ranged_release_and_never_auto_fires_a_held_aim() {
    let mut state = new_match();
    let mut combat_state = combat::new_state(&mut state, None);
    assert_eq!(
        combat_state.players[3].family_id,
        Some(ActionFamilyId::Ranged)
    );

    let ranged = action_families::get(ActionFamilyId::Ranged);
    for tick_index in 1..=ranged.windup_ticks {
        tick(
            &mut state,
            &mut combat_state,
            4,
            MatchInput {
                equipment_pressed: tick_index == 1,
                equipment_released: tick_index == 1,
                equipment_held: false,
                ..slot_input::neutral_match_input()
            },
        );
    }
    assert_eq!(combat_state.players[3].phase, CombatPhase::Active);
    tick(
        &mut state,
        &mut combat_state,
        4,
        slot_input::neutral_match_input(),
    );
    assert_eq!(combat_state.players[3].phase, CombatPhase::Recovery);
    assert_eq!(combat_state.projectiles.len(), 1);

    combat::reset(&mut combat_state);
    for tick_index in 1..=ranged.windup_ticks {
        tick(
            &mut state,
            &mut combat_state,
            4,
            MatchInput {
                equipment_pressed: tick_index == 1,
                equipment_held: true,
                ..slot_input::neutral_match_input()
            },
        );
    }
    assert_eq!(combat_state.players[3].phase, CombatPhase::Aim);
    for _ in 0..20 {
        tick(
            &mut state,
            &mut combat_state,
            4,
            MatchInput {
                equipment_held: true,
                ..slot_input::neutral_match_input()
            },
        );
    }
    assert_eq!(combat_state.players[3].phase, CombatPhase::Aim);
    assert_eq!(combat_state.projectiles.len(), 0);
    tick(
        &mut state,
        &mut combat_state,
        4,
        MatchInput {
            equipment_released: true,
            equipment_held: false,
            ..slot_input::neutral_match_input()
        },
    );
    assert_eq!(combat_state.players[3].phase, CombatPhase::Recovery);
    assert_eq!(combat_state.projectiles.len(), 1);
}

#[test]
fn deterministic_combat_resolver_selects_one_legal_melee_target_by_forward_projection_and_stable_index()
 {
    let mut state = new_match();
    let mut combat_state = combat::new_state(&mut state, None);
    move_other_players_away(&mut state, &[3, 4, 6, 7, 8]);
    state.players[2].pos = Vec2::new(100.0, 100.0);
    state.players[2].facing = Vec2::new(1.0, 0.0);
    state.players[3].pos = Vec2::new(105.0, 100.0); // friendly: ignored
    state.players[5].pos = Vec2::new(110.0, 100.0); // keeper: ignored
    state.players[6].pos = Vec2::new(130.0, 106.0);
    state.players[7].pos = Vec2::new(130.0, 94.0);

    combat_state.players[2].phase = CombatPhase::Active;
    combat_state.players[2].phase_ticks = 5;
    combat_state.players[2].source_sequence = Some(1);
    let contacts = combat::collect_contacts(&state, &mut combat_state);
    assert_eq!(contacts.len(), 1);
    assert_eq!(contacts[0].target_index, 7);
}

#[test]
fn deterministic_combat_resolver_lets_a_projectile_pass_a_dodge_and_uses_swept_contact_on_a_later_tick()
 {
    let mut state = new_match();
    let mut combat_state = combat::new_state(&mut state, None);
    move_other_players_away(&mut state, &[4, 7]);
    state.players[3].pos = Vec2::new(100.0, 100.0);
    state.players[3].facing = Vec2::new(1.0, 0.0);
    state.players[6].pos = Vec2::new(104.0, 100.0);
    state.players[6].dodge_timer = 1.0;
    combat_state.players[3].phase = CombatPhase::Active;
    combat_state.players[3].phase_ticks = 1;
    combat_state.players[3].source_sequence = Some(11);

    let mut contacts = combat::collect_contacts(&state, &mut combat_state);
    assert_eq!(contacts.len(), 0);
    assert_eq!(combat_state.projectiles.len(), 1);
    assert!((combat_state.projectiles[0].pos.x - 105.0).abs() <= 1e-6);

    state.players[6].dodge_timer = 0.0;
    state.players[6].pos = Vec2::new(109.0, 100.0);
    contacts = combat::collect_contacts(&state, &mut combat_state);
    assert_eq!(contacts.len(), 1);
    assert_eq!(contacts[0].target_index, 7);
    assert_eq!(combat_state.projectiles.len(), 0);
}

#[test]
fn deterministic_combat_resolver_expires_a_missed_projectile_after_exactly_sixty_deterministic_travel_ticks()
 {
    let mut state = new_match();
    let mut combat_state = combat::new_state(&mut state, None);
    move_other_players_away(&mut state, &[4]);
    state.players[3].pos = Vec2::new(100.0, 100.0);
    state.players[3].facing = Vec2::new(1.0, 0.0);
    combat_state.players[3].phase = CombatPhase::Active;
    combat_state.players[3].phase_ticks = 1;
    combat_state.players[3].source_sequence = Some(7);

    combat::collect_contacts(&state, &mut combat_state);
    assert_eq!(combat_state.projectiles.len(), 1);
    assert!((combat_state.projectiles[0].pos.x - 105.0).abs() <= 1e-6);
    let lifetime = action_families::get(ActionFamilyId::Ranged)
        .projectile_lifetime_ticks
        .expect("the ranged family defines a projectile lifetime");
    for _ in 2..=lifetime {
        combat::collect_contacts(&state, &mut combat_state);
    }
    assert_eq!(combat_state.projectiles.len(), 0);
    assert!(event_of(&combat_state.events, CombatEventKind::ProjectileExpire).is_some());
}

#[test]
fn deterministic_combat_resolver_blocks_a_frontal_hit_with_one_bounded_recoil_and_leaves_the_ball_owned()
 {
    let mut state = new_match();
    let mut combat_state = combat::new_state(&mut state, None);
    move_other_players_away(&mut state, &[3, 7]);
    state.players[2].pos = Vec2::new(100.0, 100.0);
    state.players[2].facing = Vec2::new(1.0, 0.0);
    state.players[6].pos = Vec2::new(130.0, 100.0);
    state.players[6].facing = Vec2::new(-1.0, 0.0);
    state.owner = Some(7);
    combat_state.players[2].phase = CombatPhase::Active;
    combat_state.players[2].phase_ticks = 5;
    combat_state.players[2].source_sequence = Some(1);
    combat_state.players[6].phase = CombatPhase::Guard;

    let contacts = combat::collect_contacts(&state, &mut combat_state);
    assert!(contacts[0].guarded);
    combat::resolve_contacts(&mut state, &mut combat_state, &contacts);
    assert_eq!(combat_state.players[6].forced_ticks, 0);
    assert_eq!(state.owner, Some(7));
    assert!((state.players[6].pos.x - 136.0).abs() <= 1e-6);
    assert!(event_of(&combat_state.events, CombatEventKind::GuardRecoil).is_some());
}

#[test]
fn deterministic_combat_resolver_resolves_simultaneous_trades_and_one_dominant_outcome_per_target()
{
    let mut state = new_match();
    let mut combat_state = combat::new_state(&mut state, None);
    move_other_players_away(&mut state, &[3, 5, 10]);
    state.players[2].pos = Vec2::new(100.0, 100.0);
    state.players[2].facing = Vec2::new(1.0, 0.0);
    state.players[4].pos = Vec2::new(100.0, 100.0);
    state.players[4].facing = Vec2::new(1.0, 0.0);
    state.players[9].pos = Vec2::new(130.0, 100.0);
    state.players[9].facing = Vec2::new(-1.0, 0.0);
    combat_state.players[2].phase = CombatPhase::Active;
    combat_state.players[2].phase_ticks = 5;
    combat_state.players[2].source_sequence = Some(1);
    combat_state.players[4].phase = CombatPhase::Active;
    combat_state.players[4].phase_ticks = 4;
    combat_state.players[4].source_sequence = Some(2);
    combat_state.players[9].phase = CombatPhase::Active;
    combat_state.players[9].phase_ticks = 4;
    combat_state.players[9].source_sequence = Some(3);

    let contacts = combat::collect_contacts(&state, &mut combat_state);
    assert_eq!(contacts.len(), 3);
    combat::resolve_contacts(&mut state, &mut combat_state, &contacts);
    assert_eq!(combat_state.players[9].forced_ticks, 18);
    assert_eq!(combat_state.players[2].forced_ticks, 10);
    assert_eq!(combat_state.players[4].forced_ticks, 0);
    let mut superseded = false;
    for event in &combat_state.events {
        if event.kind == CombatEventKind::Contact && event.source_index == Some(5) {
            superseded = event.result == Some(CombatContactResult::Superseded);
        }
    }
    assert!(superseded);
}

#[test]
fn deterministic_combat_resolver_caps_a_hit_chain_and_grants_a_full_immunity_window_without_repeat_displacement()
 {
    let mut state = new_match();
    let mut combat_state = combat::new_state(&mut state, None);
    state.players[2].pos = Vec2::new(100.0, 100.0);
    state.players[9].pos = Vec2::new(130.0, 100.0);
    let mut contact = CombatContact {
        family_id: ActionFamilyId::Unarmed,
        source_index: 3,
        target_index: 10,
        source_sequence: 1,
        projectile_sequence: None,
        guarded: false,
        immune: false,
        source_pos: state.players[2].pos,
    };

    combat::resolve_contacts(&mut state, &mut combat_state, &[contact]);
    let displaced_x = state.players[9].pos.x;
    assert_eq!(combat_state.players[9].forced_ticks, 10);
    for _ in 0..5 {
        combat::finish_tick(&state, &mut combat_state);
    }
    contact.family_id = ActionFamilyId::LightMelee;
    contact.source_sequence = 2;
    combat::resolve_contacts(&mut state, &mut combat_state, &[contact]);
    assert_eq!(combat_state.players[9].forced_ticks, 18);
    assert_eq!(state.players[9].pos.x, displaced_x);

    while combat_state.players[9].forced_ticks > 0 {
        combat::finish_tick(&state, &mut combat_state);
    }
    assert_eq!(
        combat_state.players[9].immunity_ticks,
        combat::IMMUNITY_TICKS
    );
    contact.immune = true;
    combat::resolve_contacts(&mut state, &mut combat_state, &[contact]);
    assert_eq!(combat_state.players[9].forced_ticks, 0);
    assert_eq!(
        combat_state.players[9].immunity_ticks,
        combat::IMMUNITY_TICKS
    );
}

#[test]
fn deterministic_combat_resolver_does_not_invent_a_spill_when_an_unguarded_contact_finds_a_loose_ball()
 {
    let mut state = new_match();
    let mut combat_state = combat::new_state(&mut state, None);
    state.owner = None;
    state.players[2].pos = Vec2::new(100.0, 100.0);
    state.players[9].pos = Vec2::new(130.0, 100.0);
    let source_pos = state.players[2].pos;
    combat::resolve_contacts(
        &mut state,
        &mut combat_state,
        &[CombatContact {
            family_id: ActionFamilyId::LightMelee,
            source_index: 3,
            target_index: 10,
            source_sequence: 1,
            projectile_sequence: None,
            guarded: false,
            immune: false,
            source_pos,
        }],
    );
    assert_eq!(state.owner, None);
    assert!(event_of(&combat_state.events, CombatEventKind::BallSpill).is_none());
    assert_eq!(
        combat_state.players[9].forced_state,
        Some(CombatForcedState::Knockback)
    );
}

#[test]
fn deterministic_combat_resolver_keeps_combat_presentation_free_neutral_fixture_bound_and_snapshot_deferred()
 {
    let mut state = new_match();
    let mut players_by_id: Vec<PlayerData> = gc_data::players::ALL.to_vec();
    {
        let veil = players_by_id
            .iter_mut()
            .find(|p| p.id == "veil_nyx")
            .expect("veil_nyx is authored");
        veil.loadout_id = Some("loadout_vector_blade");
    }
    let combat_state = combat::new_state(&mut state, Some(&players_by_id));
    assert_eq!(
        combat_state.players[2].family_id,
        Some(ActionFamilyId::LightMelee)
    );

    {
        let zyro = players_by_id
            .iter_mut()
            .find(|p| p.id == "zyro_vex")
            .expect("zyro_vex is authored");
        zyro.loadout_id = None;
    }
    let mut neutral_state = combat::new_state(&mut state, Some(&players_by_id));
    assert_eq!(neutral_state.players[4].family_id, None);
    let mut inputs = IndexMap::new();
    inputs.insert(
        5,
        MatchInput {
            equipment_pressed: true,
            equipment_held: true,
            ..slot_input::neutral_match_input()
        },
    );
    combat::prepare_inputs(&mut state, &mut neutral_state, &inputs, None);
    assert_eq!(neutral_state.players[4].phase, CombatPhase::Ready);

    neutral_state.player_ids[4] = "wrong_player".to_string();
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            combat::prepare_inputs(&mut state, &mut neutral_state, &IndexMap::new(), None)
        }))
        .is_err()
    );

    let snapshot = combat::snapshot(&combat_state);
    assert_eq!(snapshot.version, combat_state.version);
    assert_eq!(
        snapshot.players[2].loadout_id.as_deref(),
        Some("loadout_vector_blade")
    );
    assert_eq!(
        snapshot.players[2].family_id,
        Some(ActionFamilyId::LightMelee)
    );
    let mut mutated_snapshot = snapshot;
    mutated_snapshot.players[2].phase = CombatPhase::Active;
    assert_eq!(combat_state.players[2].phase, CombatPhase::Ready);
}
