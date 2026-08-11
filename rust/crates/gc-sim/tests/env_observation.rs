//! Port of `spec/sim/env_observation_spec.lua`.
//!
//! The Lua spec's own `reset()` helper calls `env.reset(env.reference_config(...))`
//! — `sim/env.lua`, needing `sim/match.lua`'s simulation logic to build a
//! playable fixture. At the time this port was written `sim/match.lua` was not
//! yet ported (`src/match.rs` was a placeholder — see `v2/README.md`'s
//! "match-shaped view structs" note in §5.1), so every test in this file is, by
//! the letter of its `require` list, in the same boat as
//! `spec/sim/env_leakage_spec.lua`.
//!
//! In practice the two specs differ in what they actually need. Every test
//! here only calls `env_observation::view`/`build`/`view_for`/`encode` against
//! a **static** `MatchState` (and, for two tests, a static `CombatMatchState`)
//! — no tick stepping, no RNG-driven divergence, no rollback. `MatchState`'s
//! shape is complete and stable (`match_snapshot.rs`, owned by this port's
//! task as "available and complete"), so this file builds its own minimal,
//! self-contained kickoff-shaped fixture by hand (`fixture()` below) instead
//! of routing through `sim::env`/`sim::match`, and ports the specs for real.
//! `spec/sim/env_leakage_spec.lua`, by contrast, genuinely needs `env.step`'s
//! multi-tick simulation (tape-based divergence across many ticks, RNG
//! advancement) — that is not obtainable from `MatchState`'s shape alone, so
//! `tests/env_leakage.rs` stubs it. See this crate's porting report for the
//! full account.
//!
//! The fixture's field values are chosen to be *structurally* valid and
//! internally consistent (positions, team/slot wiring, one keeper per side),
//! not to reproduce the real kickoff formation's exact numbers — every
//! assertion ported here only depends on structure (side assignment, counts,
//! echoed state fields, threshold comparisons), matching what the Lua spec
//! itself checks.
//!
//! One Lua sub-case is not portable at all: `UNKNOWN_PROFILE = "oracle"`
//! passed to `env_observation.build`. This port's `profile` parameter is
//! `EnvObservationProfile`, a closed enum, so there is no way to construct an
//! "unknown" profile value to pass — see `EnvObservationErrorCode::UnknownProfile`'s
//! doc comment in `src/env_observation.rs`.

use gc_core::vec2::Vec2;
use gc_data::action_families::ActionFamilyId;
use gc_data::species::SimVerb;
use gc_data::tactics::{MarkingConfig, MarkingScheme, TransitionConfig};
use gc_sim::combat_feasibility::CombatActionPhase;
use gc_sim::combat_intent;
use gc_sim::combat_snapshot::{self, CombatMatchState, CombatPlayerState};
use gc_sim::env_observation::{
    self, EnvObservationErrorCode, EnvObservationProfile, EnvRelativeSide,
};
use gc_sim::input_frame::{self, SlotId, Team as InputTeam};
use gc_sim::keeper::KeeperBehaviorState;
use gc_sim::match_snapshot::{ByTeam, MatchPlayer, MatchState, PitchSize, Rect, Team as StateTeam};
use gc_sim::outfield_decision;
use gc_sim::outfield_press;
use gc_sim::possession_transition::{self, TransitionWindows};

// ---------------------------------------------------------------------------
// Fixture construction.
// ---------------------------------------------------------------------------

fn marking() -> MarkingConfig {
    MarkingConfig {
        scheme: MarkingScheme::Hybrid,
        man_marks: 0,
        standoff: 20.0,
        compactness: 0.5,
        support: 0.5,
    }
}

fn transition_config() -> TransitionConfig {
    TransitionConfig {
        counterpress: 0.0,
        counterattack: 0.0,
    }
}

fn player(id: &str, team: StateTeam, is_keeper: bool, pos: Vec2) -> MatchPlayer {
    MatchPlayer {
        id: id.to_string(),
        name: id.to_string(),
        team,
        pos,
        vel: Vec2::default(),
        run_vel: Vec2::default(),
        facing: Vec2::new(1.0, 0.0),
        anchor: pos,
        species_id: "fixture_species".to_string(),
        owned_verb: SimVerb::None,
        move_speed: 180.0,
        shot_speed: 320.0,
        dribble: 0.6,
        strength: 0.5,
        first_touch: 0.6,
        header_skill: 0.5,
        volley_skill: 0.5,
        bicycle_skill: 0.5,
        scan_rate: 0.5,
        composure: 0.5,
        outfield_decision: outfield_decision::new_state(None),
        is_keeper,
        radius: 10.0,
        dash_cd: 0.0,
        dodge_cd: 0.0,
        dodge_timer: 0.0,
        dodge_dir: Vec2::default(),
        reach: if is_keeper { 26.0 } else { 0.0 },
        handling: if is_keeper { 0.7 } else { 0.0 },
        keeper_aggression: if is_keeper { 40.0 } else { 0.0 },
        keeper_anticipation: if is_keeper { 0.5 } else { 0.0 },
        keeper_state: KeeperBehaviorState::Base,
        keeper_state_timer: 0.0,
        keeper_release_state: None,
        keeper_release_motion: 0.0,
        keeper_release_kind: None,
        keeper_release_depth: 0.0,
        keeper_set: 0.0,
        dive_timer: 0.0,
        dive_dir: Vec2::default(),
        dive_delay: 0.0,
        dive_target: None,
        keeper_get_up_timer: 0.0,
        hold_timer: 0.0,
        feet_ball: false,
        slide_timer: 0.0,
        slide_dir: Vec2::default(),
        slide_vel: 0.0,
        tackle_timer: 0.0,
        tackle_cd: 0.0,
        stun_timer: 0.0,
        grab_timer: 0.0,
        throw_timer: 0.0,
        receive_timer: 0.0,
        sprint_meter: 1.0,
        sprint_dur: 3.0,
        sprinting: false,
        save_pending: None,
        save_timer: 0.0,
        save_vx: 0.0,
        save_style: None,
        save_tip_emitted: false,
        settle_timer: 0.0,
        header_cd: 0.0,
        aerial_timer: 0.0,
        aerial_style: None,
        aerial_outcome: None,
        aerial_jump: 0.0,
        aerial_recovery: 0.0,
        charge: 0.0,
        pass_charge: 0.0,
        pass_target: None,
        windup_timer: 0.0,
        windup_shot: None,
        jockey_timer: 0.0,
    }
}

/// A structurally valid kickoff-shaped fixture: 4 home outfielders (slots
/// 1-4) + 1 home keeper, 4 away outfielders (slots 5-8) + 1 away keeper,
/// matching `match_snapshot::MatchState`'s documented "home indices 0..5,
/// away 5..10" convention. Loose ball, kickoff hold active, 0-0.
fn fixture() -> MatchState {
    let mut players = Vec::new();
    for i in 0..4 {
        players.push(player(
            &format!("home_out_{}", i + 1),
            StateTeam::Home,
            false,
            Vec2::new(200.0, 150.0 + 100.0 * i as f64),
        ));
    }
    players.push(player(
        "home_keeper",
        StateTeam::Home,
        true,
        Vec2::new(20.0, 300.0),
    ));
    for i in 0..4 {
        players.push(player(
            &format!("away_out_{}", i + 1),
            StateTeam::Away,
            false,
            Vec2::new(760.0, 150.0 + 100.0 * i as f64),
        ));
    }
    players.push(player(
        "away_keeper",
        StateTeam::Away,
        true,
        Vec2::new(940.0, 300.0),
    ));

    let mut slot_players: Vec<Option<i64>> = vec![None; input_frame::SLOT_COUNT as usize];
    let mut slot_for_player: Vec<Option<i64>> = vec![None; players.len()];
    for i in 0..4i64 {
        // Home outfielders: players 1..=4 <-> slots 1..=4.
        slot_players[i as usize] = Some(i + 1);
        slot_for_player[i as usize] = Some(i + 1);
    }
    for i in 0..4i64 {
        // Away outfielders: players 6..=9 <-> slots 5..=8.
        let player_index = 6 + i;
        slot_players[(4 + i) as usize] = Some(player_index);
        slot_for_player[(player_index - 1) as usize] = Some(5 + i);
    }

    MatchState {
        field: PitchSize { w: 960.0, h: 600.0 },
        goal_home: Rect {
            x: 0.0,
            y: 260.0,
            w: 10.0,
            h: 80.0,
        },
        goal_away: Rect {
            x: 950.0,
            y: 260.0,
            w: 10.0,
            h: 80.0,
        },
        players,
        ball: Vec2::new(480.0, 300.0),
        ball_vel: Vec2::default(),
        ball_z: 0.0,
        ball_vz: 0.0,
        owner: None,
        controlled: 1,
        human_controlled: true,
        score: ByTeam { home: 0, away: 0 },
        time_left: 300.0,
        max_goals: 5,
        finished: false,
        pickup_cd: 0.0,
        press: ByTeam { home: 1, away: 1 },
        marking: ByTeam {
            home: marking(),
            away: marking(),
        },
        marks: ByTeam {
            home: vec![None; 10],
            away: vec![None; 10],
        },
        outfield_press: ByTeam {
            home: outfield_press::new_state(),
            away: outfield_press::new_state(),
        },
        transition_windows: TransitionWindows {
            home: transition_config(),
            away: transition_config(),
        },
        transition: possession_transition::new_state(),
        formation: ByTeam {
            home: "1-2-1".to_string(),
            away: "1-2-1".to_string(),
        },
        ball_spin: 0.0,
        rng: 123_456,
        block_grace: 0.0,
        aerial_lock: 0.0,
        kickoff_hold: 1.5,
        events: Vec::new(),
        slot_mode: true,
        input_ownership: None,
        slot_players,
        slot_for_player,
        input_tick: 0,
        unsupported_reason: None,
    }
}

fn combat_player(loadout_id: Option<&str>, family_id: Option<ActionFamilyId>) -> CombatPlayerState {
    CombatPlayerState {
        loadout_id: loadout_id.map(str::to_string),
        family_id,
        phase: CombatActionPhase::Ready,
        phase_ticks: 0,
        cooldown_ticks: 0,
        source_sequence: None,
        contacted: false,
        release_latched: false,
        control_held: false,
        projectile_spawned: false,
        forced_state: None,
        forced_ticks: 0,
        chain_ticks: 0,
        immunity_ticks: 0,
        intent: combat_intent::new_state(),
    }
}

/// A combat companion where the controlling player (`home_out_1`, slot 1)
/// and every away outfielder carry an equipped `LightMelee` loadout; both
/// keepers and the other home outfielders stay unequipped, matching the Lua
/// fixture's "the four opposing outfielders carry visible equipment".
fn combat_fixture(state: &MatchState) -> CombatMatchState {
    let mut players = Vec::new();
    players.push(combat_player(
        Some("loadout_fixture"),
        Some(ActionFamilyId::LightMelee),
    ));
    for _ in 0..3 {
        players.push(combat_player(None, None));
    }
    players.push(combat_player(None, None)); // home keeper
    for _ in 0..4 {
        players.push(combat_player(
            Some("loadout_fixture"),
            Some(ActionFamilyId::LightMelee),
        ));
    }
    players.push(combat_player(None, None)); // away keeper

    CombatMatchState {
        version: combat_snapshot::VERSION,
        tick: state.input_tick,
        player_ids: state.players.iter().map(|p| p.id.clone()).collect(),
        players,
        projectiles: Vec::new(),
        events: Vec::new(),
        next_source_sequence: 1,
    }
}

// ---------------------------------------------------------------------------
// PROFILES.
// ---------------------------------------------------------------------------

#[test]
fn tags_observability_and_human_proxy_validity_per_profile() {
    let representative = env_observation::profile_data(EnvObservationProfile::Representative);
    assert!(representative.player_observable);
    assert!(representative.human_proxy_valid);
    assert!(!representative.multi_slot);

    let team = env_observation::profile_data(EnvObservationProfile::Team);
    assert!(team.player_observable);
    assert!(team.multi_slot);

    let privileged = env_observation::profile_data(EnvObservationProfile::Privileged);
    assert!(!privileged.player_observable);
    assert!(!privileged.human_proxy_valid);

    for data in &env_observation::PROFILES {
        assert!(!data.description.is_empty());
    }
}

#[test]
fn rejects_slot_profile_mismatches() {
    // The `UNKNOWN_PROFILE = "oracle"` sub-case from the Lua spec has no
    // Rust equivalent — see this file's module doc.
    let state = fixture();

    let mismatch = env_observation::build(
        &state,
        None,
        &[1, 5],
        EnvObservationProfile::Representative,
        None,
    );
    assert_eq!(
        mismatch.unwrap_err().code,
        EnvObservationErrorCode::ProfileMismatch
    );

    let empty = env_observation::build(&state, None, &[], EnvObservationProfile::Team, None);
    assert_eq!(empty.unwrap_err().code, EnvObservationErrorCode::Malformed);

    let unsorted = env_observation::build(&state, None, &[5, 1], EnvObservationProfile::Team, None);
    assert_eq!(
        unsorted.unwrap_err().code,
        EnvObservationErrorCode::Malformed
    );

    let off_slot = env_observation::build(
        &state,
        None,
        &[9],
        EnvObservationProfile::Representative,
        None,
    );
    assert_eq!(
        off_slot.unwrap_err().code,
        EnvObservationErrorCode::Malformed
    );
}

// ---------------------------------------------------------------------------
// view().
// ---------------------------------------------------------------------------

#[test]
fn describes_the_fixture_from_the_controlled_slots_side() {
    let state = fixture();
    let view = env_observation::view(&state, None, 1, EnvObservationProfile::Representative, None)
        .unwrap();

    assert_eq!(view.version, env_observation::VERSION);
    assert_eq!(view.slot, 1);
    assert_eq!(view.slot_id, SlotId::Home1);
    assert_eq!(view.team, InputTeam::Home);
    assert_eq!(view.own.slot, 1);
    assert_eq!(view.own.side, EnvRelativeSide::Own);
    assert!(!view.own.is_keeper, "keepers never own an input slot");
    assert_eq!(
        view.teammates.len(),
        4,
        "four teammates including the AI keeper"
    );
    assert_eq!(view.opponents.len(), 5);
    assert_eq!(view.r#match.tick, 0);
    assert_eq!(view.r#match.tick_rate, 60);
    assert_eq!(view.r#match.phase, env_observation::EnvMatchPhase::Kickoff);
    assert!(!view.r#match.finished);
    assert_eq!(view.geometry.field_w, 960.0);
    assert_eq!(view.geometry.own_goal.x, state.goal_home.x);
    assert_eq!(view.geometry.target_goal.x, state.goal_away.x);
    assert_eq!(view.events.len(), 0);
    assert!(view.privileged.is_none());

    let mut keepers = 0;
    for teammate in &view.teammates {
        assert_eq!(teammate.side, EnvRelativeSide::Own);
        if teammate.is_keeper {
            keepers += 1;
            assert_eq!(teammate.slot, None, "a keeper has no input slot to report");
        }
    }
    assert_eq!(keepers, 1);
    for opponent in &view.opponents {
        assert_eq!(opponent.side, EnvRelativeSide::Opponent);
    }
}

#[test]
fn mirrors_sides_for_an_away_slot() {
    let state = fixture();
    let view = env_observation::view(&state, None, 5, EnvObservationProfile::Representative, None)
        .unwrap();

    assert_eq!(view.team, InputTeam::Away);
    assert_eq!(view.slot_id, SlotId::Away1);
    assert_eq!(view.geometry.own_goal.x, state.goal_away.x);
    assert_eq!(view.geometry.target_goal.x, state.goal_home.x);
    assert_eq!(view.r#match.score_own, state.score.away);
}

#[test]
fn reports_the_ball_and_its_carrier_as_relative_cues() {
    let mut state = fixture();

    // Loose-ball branch.
    let view = env_observation::view(&state, None, 1, EnvObservationProfile::Representative, None)
        .unwrap();
    assert_eq!(view.ball.x, state.ball.x);
    assert_eq!(view.ball.z, state.ball_z);
    assert!(!view.ball.airborne);
    assert!(view.ball.loose);
    assert_eq!(view.ball.owner_slot, None);
    assert_eq!(view.ball.owner_side, None);

    // Owned-ball branch: an away outfielder (player 6, slot 5) carries it.
    state.owner = Some(6);
    let view = env_observation::view(&state, None, 1, EnvObservationProfile::Representative, None)
        .unwrap();
    assert!(!view.ball.loose);
    assert_eq!(view.ball.owner_side, Some(EnvRelativeSide::Opponent));
    assert_eq!(view.ball.owner_slot, Some(5));
}

#[test]
fn shows_only_the_readiness_a_player_can_see_on_themselves() {
    let mut state = fixture();
    state.players[0].dash_cd = 0.25;
    state.players[0].dodge_cd = 0.0;
    state.players[0].sprint_meter = 0.5;

    let view = env_observation::view(&state, None, 1, EnvObservationProfile::Representative, None)
        .unwrap();
    assert_eq!(view.own.dash_cooldown_s, 0.25);
    assert!(!view.own.dash_ready);
    assert!(view.own.dodge_ready);
    assert_eq!(view.own.sprint_meter, 0.5);
    // Other players expose no remaining timers and no meters at all: the
    // Lua spec checks this via `field(other, "dash_cooldown_s") == nil` at
    // runtime; here `EnvObservedPlayer` simply has no such field at all, a
    // stronger, compile-time version of the same guarantee (see
    // `player_field_set_matches_the_pinned_field_names` below).
    assert_eq!(view.opponents.len(), 5);
}

#[test]
fn player_field_set_matches_the_pinned_field_names() {
    // Exhaustive destructure (no `..`): the Lua spec's
    // "exposes no non-self field without a rendered analogue" test walks
    // `pairs()` on a live table at runtime and checks every key against
    // `env_observation.PLAYER_FIELDS`. `EnvObservedPlayer` has a fixed,
    // compile-time field set, so the equivalent guarantee here is that this
    // destructure — naming every field once — still compiles: adding a
    // field to the struct without adding it here (and to
    // `PLAYER_FIELD_NAMES`) is a compile error, not a runtime scan failure.
    let state = fixture();
    let combat = combat_fixture(&state);
    let view = env_observation::view(
        &state,
        Some(&combat),
        1,
        EnvObservationProfile::Representative,
        None,
    )
    .unwrap();
    let sample = view.opponents.first().expect("fixture has opponents");
    let gc_sim::env_observation::EnvObservedPlayer {
        slot,
        side,
        is_keeper,
        x,
        y,
        vx,
        vy,
        facing_x,
        facing_y,
        radius,
        has_ball,
        sprinting,
        sliding,
        diving,
        airborne,
        winding_up,
        equipment,
    } = *sample;
    let _ = (
        slot, side, is_keeper, x, y, vx, vy, facing_x, facing_y, radius, has_ball, sprinting,
        sliding, diving, airborne, winding_up,
    );
    assert_eq!(env_observation::PLAYER_FIELD_NAMES.len(), 17);
    // The five states the Lua spec confirms have no rendered analogue for a
    // non-local player (`charging`, `jockeying`, `tackling`, `dodging`,
    // `stunned`) are absent from `PLAYER_FIELD_NAMES`, and — because they
    // are not fields on `EnvObservedPlayer` at all — could not be named in
    // the destructure above even by mistake.
    for name in ["charging", "jockeying", "tackling", "dodging", "stunned"] {
        assert!(!env_observation::PLAYER_FIELD_NAMES.contains(&name));
    }
    if let Some(equipment) = equipment {
        let gc_sim::env_observation::EnvObservedEquipment {
            family_id,
            phase,
            forced_state,
        } = equipment;
        let _ = (family_id, phase, forced_state);
        assert_eq!(env_observation::EQUIPMENT_TELEGRAPH_FIELD_NAMES.len(), 3);
    }
    // Own-view readiness is unaffected: these remain visible about yourself.
    assert!(matches!(view.own.charging, true | false));
    assert!(matches!(view.own.stunned, true | false));
    assert!(matches!(view.own.jockeying, true | false));
}

#[test]
fn shows_equipment_as_a_telegraph_for_others_and_a_readout_for_self() {
    let state = fixture();
    let combat = combat_fixture(&state);
    let view = env_observation::view(
        &state,
        Some(&combat),
        1,
        EnvObservationProfile::Representative,
        None,
    )
    .unwrap();

    let own = view.own.equipment.expect("self is equipped in the fixture");
    assert_eq!(own.phase, CombatActionPhase::Ready);
    assert!(own.ready);
    assert!(
        !own.loadout_id.is_empty(),
        "own loadout identity is visible on the HUD"
    );

    let telegraphs = view
        .opponents
        .iter()
        .filter(|p| p.equipment.is_some())
        .count();
    assert_eq!(
        telegraphs, 4,
        "the four opposing outfielders carry visible equipment"
    );
    for opponent in view.opponents.iter().filter_map(|p| p.equipment.as_ref()) {
        assert_eq!(opponent.phase, CombatActionPhase::Ready);
        // `EnvObservedEquipment` has no `cooldown_ticks`/`loadout_id` field
        // at all — a compile-time version of the Lua spec's
        // "others expose no private tick counts" runtime check.
    }
}

#[test]
fn requires_a_fixed_slot_match() {
    let mut state = fixture();
    state.slot_mode = false;
    let result =
        env_observation::view(&state, None, 1, EnvObservationProfile::Representative, None);
    assert_eq!(result.unwrap_err().code, EnvObservationErrorCode::Malformed);
}

// ---------------------------------------------------------------------------
// encode().
// ---------------------------------------------------------------------------

#[test]
fn encode_is_stable_for_identical_observations_and_reacts_to_any_change() {
    let left = fixture();
    let mut right = fixture();

    let build = |s: &MatchState| {
        env_observation::build(s, None, &[1], EnvObservationProfile::Representative, None).unwrap()
    };
    let encoded = env_observation::encode(&build(&left));
    assert_eq!(env_observation::encode(&build(&right)), encoded);

    right.ball_z += 1.0;
    assert_ne!(
        env_observation::encode(&build(&right)),
        encoded,
        "a changed cue must change the encoding"
    );
}

#[test]
fn encode_distinguishes_profiles() {
    let state = fixture();
    let representative = env_observation::build(
        &state,
        None,
        &[1],
        EnvObservationProfile::Representative,
        None,
    )
    .unwrap();
    let privileged =
        env_observation::build(&state, None, &[1], EnvObservationProfile::Privileged, None)
            .unwrap();
    assert_ne!(
        env_observation::encode(&privileged),
        env_observation::encode(&representative),
        "the privileged profile carries strictly more"
    );
}

// ---------------------------------------------------------------------------
// view_for().
// ---------------------------------------------------------------------------

#[test]
fn view_for_returns_the_requested_slot_view_or_none() {
    let state = fixture();
    let observation =
        env_observation::build(&state, None, &[1, 5], EnvObservationProfile::Team, None).unwrap();
    assert_eq!(env_observation::view_for(&observation, 5).unwrap().slot, 5);
    assert!(env_observation::view_for(&observation, 2).is_none());
    assert_eq!(observation.slots.len(), 2);
    assert_eq!(observation.slots[1], 5);
    assert_eq!(observation.views.len(), input_frame::SLOT_COUNT as usize);
}
