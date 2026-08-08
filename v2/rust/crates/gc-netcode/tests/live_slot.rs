//! Port of `spec/game/online_live_slot_spec.lua`.

use gc_core::vec2::Vec2;
use gc_data::teams;
use gc_netcode::coordinator;
use gc_netcode::live_slot::{self, TransitionOptions};
use gc_netcode::protocol;
use gc_sim::input_frame::{self, SlotId};
use gc_sim::r#match as sim_match;
use gc_sim::match_snapshot::{self, MatchState};

fn slot_match() -> MatchState {
    let home = teams::get("nebula").expect("nebula team is authored");
    let away = teams::get("orion").expect("orion team is authored");
    let ownership = sim_match::ownership_for_teams(home, away, None);
    sim_match::new(sim_match::NewMatchOptions {
        home,
        away,
        field: match_snapshot::PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(4.0),
        max_goals: Some(99),
        seed: Some(74.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership),
    })
}

/// Park every outfielder on a distinct ring radius so the ranking is fully
/// determined, then let a caller collapse chosen slots onto the same radius.
fn place(state: &mut MatchState, radii: &[(i64, f64)]) {
    state.ball = Vec2::new(480.0, 270.0);
    state.owner = None;
    for slot in 1..=input_frame::SLOT_COUNT {
        let player_index =
            state.slot_players[(slot - 1) as usize].expect("canonical slot is mapped");
        let radius = radii
            .iter()
            .find(|(candidate, _)| *candidate == slot)
            .map_or(100.0 + slot as f64 * 10.0, |(_, radius)| *radius);
        let player = &mut state.players[(player_index - 1) as usize];
        player.pos = Vec2::new(480.0 + radius, 270.0);
        player.vel = Vec2::new(0.0, 0.0);
        player.run_vel = Vec2::new(0.0, 0.0);
    }
}

fn joined(ranked: &[SlotId]) -> String {
    ranked
        .iter()
        .map(|slot| protocol::slot_wire_id(*slot))
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
fn orders_every_canonical_slot_by_distance_to_the_ball() {
    let mut state = slot_match();
    place(&mut state, &[(3, 5.0), (7, 15.0), (1, 25.0)]);
    let ranked = live_slot::rank(&state, None);
    assert_eq!(ranked.len(), input_frame::SLOT_COUNT as usize);
    assert_eq!(ranked[0], SlotId::Home3);
    assert_eq!(ranked[1], SlotId::Away3);
    assert_eq!(ranked[2], SlotId::Home1);
}

#[test]
fn resolves_an_exact_distance_tie_by_ascending_canonical_slot_index() {
    let mut state = slot_match();
    // Two owned slots at exactly the same distance: the classic desync.
    place(&mut state, &[(2, 40.0), (4, 40.0)]);
    let ranked = live_slot::rank(&state, None);
    assert_eq!(ranked[0], SlotId::Home2);
    assert_eq!(ranked[1], SlotId::Home4);

    // The tie is on opposite sides of the ball, so the vectors differ while
    // the squared distances are bit-identical.
    let mut mirrored = slot_match();
    place(&mut mirrored, &[(2, 40.0), (4, 40.0)]);
    let slot4_player = mirrored.slot_players[3].expect("home_4 is mapped");
    mirrored.players[(slot4_player - 1) as usize].pos = Vec2::new(480.0 - 40.0, 270.0);
    assert_eq!(joined(&live_slot::rank(&mirrored, None)), joined(&ranked));
}

#[test]
fn is_independent_of_the_order_slots_are_inserted_or_sorted() {
    let mut state = slot_match();
    // Every outfielder equidistant: a comparator that fell through to
    // insertion order would return an arbitrary permutation here.
    place(&mut state, &[]);
    for slot in 1..=input_frame::SLOT_COUNT {
        let player_index =
            state.slot_players[(slot - 1) as usize].expect("canonical slot is mapped");
        state.players[(player_index - 1) as usize].pos = Vec2::new(480.0 + 60.0, 270.0);
    }
    let expected = joined(&live_slot::rank(&state, None));
    assert_eq!(
        expected,
        "home_1,home_2,home_3,home_4,away_1,away_2,away_3,away_4"
    );
    for _ in 0..16 {
        assert_eq!(joined(&live_slot::rank(&state, None)), expected);
    }
}

/// `pairs()` over a Lua hash part is the classic cross-peer divergence; the
/// Lua spec asserts the module's own source never calls it. This Rust port
/// has no `pairs` to forbid, but the same invariant has a direct Rust
/// analogue — README rule 5.4 forbids `HashMap`/`HashSet` for exactly the
/// same reason — so this asserts that instead, scanning the actual shipped
/// source rather than trusting a comment.
#[test]
fn never_uses_a_hash_map_or_hash_set_in_the_ranking_path() {
    let source = include_str!("../src/live_slot.rs");
    assert!(
        !source.contains("HashMap"),
        "live_slot.rs must never use HashMap (README rule 5.4)"
    );
    assert!(
        !source.contains("HashSet"),
        "live_slot.rs must never use HashSet (README rule 5.4)"
    );
}

#[test]
fn excludes_the_live_slot_so_switching_actually_switches() {
    let mut state = slot_match();
    place(&mut state, &[(1, 5.0), (2, 15.0)]);
    let ranked = live_slot::rank(&state, Some(SlotId::Home1));
    assert_eq!(ranked[0], SlotId::Home2);
    for slot in &ranked {
        assert_ne!(*slot, SlotId::Home1);
    }
}

#[test]
fn reports_the_carrier_and_derives_the_winner_from_a_possession_change() {
    let mut state = slot_match();
    place(&mut state, &[]);
    assert_eq!(live_slot::carrier(&state), None);
    state.owner = state.slot_players[5]; // away_2's player.
    assert_eq!(live_slot::carrier(&state), Some(SlotId::Away2));

    let held = live_slot::transition(
        &state,
        &TransitionOptions {
            switch: false,
            live: None,
            previous_carrier: Some(SlotId::Away2),
        },
    );
    assert_eq!(held.winner, None);
    let won = live_slot::transition(
        &state,
        &TransitionOptions {
            switch: false,
            live: None,
            previous_carrier: None,
        },
    );
    assert_eq!(won.winner, Some(SlotId::Away2));
}

#[test]
fn keeps_keepers_slotless() {
    let mut state = slot_match();
    place(&mut state, &[]);
    state.owner = Some(1);
    assert_eq!(live_slot::carrier(&state), None);
}

#[test]
fn feeds_a_total_ordering_into_the_frozen_transition_rule() {
    let mut state = slot_match();
    place(&mut state, &[(2, 40.0), (4, 40.0)]);
    let owned = [SlotId::Home2, SlotId::Home4];
    let transition = live_slot::transition(
        &state,
        &TransitionOptions {
            switch: true,
            live: Some(SlotId::Home4),
            previous_carrier: None,
        },
    );
    assert_eq!(
        coordinator::next_live_slot(&owned, SlotId::Home4, &transition),
        SlotId::Home2
    );

    // Same geometry, opposite starting slot: the rule still lands on the
    // one member of the owned set the ranking put first.
    let other = live_slot::transition(
        &state,
        &TransitionOptions {
            switch: true,
            live: Some(SlotId::Home2),
            previous_carrier: None,
        },
    );
    assert_eq!(
        coordinator::next_live_slot(&owned, SlotId::Home2, &other),
        SlotId::Home4
    );
}
