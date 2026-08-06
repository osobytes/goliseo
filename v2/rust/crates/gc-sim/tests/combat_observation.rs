//! Port of `spec/sim/combat_observation_spec.lua`.
//!
//! `combat_sim_observation/v1`: schema validity, field ordering, rejection
//! of malformed/duplicated rows, projectile ids, threat-path publication,
//! and sentinel discipline.
//!
//! Two Lua cases have no direct Rust analogue and are ported as adapted
//! assertions rather than dropped:
//!
//! - "rejects a missing declared field" mutates `observation.self.facing_x
//!   = nil` on an untyped Lua table. [`CombatObservation`]'s rows are typed
//!   Rust structs (`SelfRow` et al.) where every field is a plain `f64`/
//!   `bool`/enum — there is no way to construct a `SelfRow` missing a
//!   field, so the exact case cannot be expressed. The closest surviving
//!   invariant `validate` still checks against a value that isn't
//!   type-constrained to be correct is the `schema` tag itself (a `String`,
//!   not an enum) going missing/blank, so this is ported as that check
//!   instead.
//! - "rejects an undeclared field anywhere in the schema" injects extra
//!   keys (`loadout_id`, `presentation_id`, `theme`, `rng`, `viewport`)
//!   onto an untyped Lua table. A typed [`CombatObservation`] has no
//!   undeclared-field slot to inject into at all (see
//!   `combat_observation::validate`'s doc comment), so the runtime
//!   rejection this case exercised no longer exists to run. What survives
//!   is the documentation of exactly those leakage vectors:
//!   [`combat_observation::FORBIDDEN_FIELDS`] is the module's own
//!   still-live denylist, so this is ported as an assertion that it
//!   actually names every field the Lua sub-cases tried to smuggle in.

use gc_core::vec2::Vec2;
use gc_data::action_families::ActionFamilyId;
use gc_sim::combat;
use gc_sim::combat_feasibility::CombatActionPhase;
use gc_sim::combat_observation::{self, CombatObservation};
use gc_sim::combat_policy;
use gc_sim::combat_snapshot::CombatMatchState;
use gc_sim::combat_snapshot::CombatProjectile;
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{MatchState, PitchSize};

fn new_match(seed: Option<f64>) -> (MatchState, CombatMatchState) {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    let mut state = sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(seed.unwrap_or(19.0)),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    });
    state.kickoff_hold = 0.0;
    let combat_state = combat::new_state(&mut state, None);
    (state, combat_state)
}

fn build(state: &MatchState, combat_state: &CombatMatchState, index: i64) -> CombatObservation {
    combat_observation::build(
        state,
        Some(combat_state),
        index,
        combat_policy::POLICY_ID,
        None,
    )
}

#[test]
fn combat_sim_observation_v1_builds_a_valid_observation_whose_digest_covers_its_body() {
    let (state, combat_state) = new_match(None);
    let observation = build(&state, &combat_state, 2);
    assert_eq!(observation.schema, combat_observation::SCHEMA);
    assert_eq!(observation.version, combat_observation::VERSION);
    assert_eq!(observation.policy_id, combat_policy::POLICY_ID);
    assert!(combat_observation::validate(&observation).is_ok());
    assert_eq!(observation.digest, combat_observation::digest(&observation));
    assert_eq!(observation.digest.len(), 16);
}

#[test]
fn combat_sim_observation_v1_orders_teammate_opponent_and_anchor_rows_by_canonical_player_index() {
    let (state, combat_state) = new_match(None);
    let observation = build(&state, &combat_state, 3);
    let mut previous = 0;
    for row in &observation.teammates {
        assert!(row.player_index > previous);
        assert_eq!(row.team, observation.own.team);
        previous = row.player_index;
    }
    previous = 0;
    for row in &observation.opponents {
        assert!(row.player_index > previous);
        assert_ne!(row.team, observation.own.team);
        previous = row.player_index;
    }
    for (index, row) in observation.anchors.iter().enumerate() {
        assert_eq!(row.player_index, index as i64 + 1);
    }
    assert_eq!(observation.player_order.len(), state.players.len());
    assert_eq!(
        observation.teammates.len() + observation.opponents.len() + 1,
        state.players.len()
    );
}

#[test]
fn combat_sim_observation_v1_rejects_a_reordered_peer_row_array() {
    let (state, combat_state) = new_match(None);
    let mut observation = build(&state, &combat_state, 2);
    observation.opponents.swap(0, 1);
    let err = combat_observation::validate(&observation).expect_err("reordered rows must fail");
    assert!(err.contains("canonical player order"), "{err}");
}

#[test]
fn combat_sim_observation_v1_rejects_a_row_filed_under_the_wrong_team_array() {
    let (state, combat_state) = new_match(None);
    let mut observation = build(&state, &combat_state, 2);
    // Home is 1..5 and away is 6..10 and the observer is player 2, so moving
    // opponent 6 onto the end of the teammate array keeps the canonical
    // ascending player order intact. Only the team disagrees.
    let moved = observation.opponents.remove(0);
    assert_eq!(moved.player_index, 6);
    observation.teammates.push(moved);
    // Recompute the digest so a stale hash cannot mask the real rejection:
    // this has to fail on the team, not on the digest.
    observation.digest = combat_observation::digest(&observation);
    let err =
        combat_observation::validate(&observation).expect_err("a team-swapped row was accepted");
    assert!(err.contains("wrong team array"), "{err}");

    // Every consumer reads array membership as team ground truth, so the
    // mirror case has to be rejected too.
    let mut mirrored = build(&state, &combat_state, 2);
    let mate = mirrored.teammates.remove(0);
    mirrored.opponents.insert(0, mate);
    mirrored.digest = combat_observation::digest(&mirrored);
    assert!(combat_observation::validate(&mirrored).is_err());
}

#[test]
fn combat_sim_observation_v1_rejects_a_missing_declared_field() {
    let (state, combat_state) = new_match(None);
    let mut observation = build(&state, &combat_state, 2);
    observation.schema = String::new();
    let err = combat_observation::validate(&observation)
        .expect_err("a blanked schema tag must be rejected");
    assert!(err.contains("observation schema tag is not"), "{err}");
}

#[test]
fn combat_sim_observation_v1_rejects_an_undeclared_field_anywhere_in_the_schema() {
    // The Lua sub-cases smuggle in exactly these five keys; assert the
    // module's own denylist still names every one of them.
    for field in ["loadout_id", "presentation_id", "theme", "rng", "viewport"] {
        assert!(
            combat_observation::FORBIDDEN_FIELDS.contains(&field),
            "FORBIDDEN_FIELDS should still name {field}"
        );
    }
}

#[test]
fn combat_sim_observation_v1_rejects_a_duplicated_peer_row_and_a_duplicated_projectile_row() {
    let (state, mut combat_state) = new_match(None);
    // A peer row cannot be duplicated without ALSO breaking something else:
    // within one array it breaks the strict player-index ordering, and
    // across arrays it breaks team membership. Both rejections are asserted
    // here; `seen_index` stays as defence in depth behind them.
    let mut across = build(&state, &combat_state, 2);
    let dup = across.opponents[0].clone();
    across.teammates.push(dup);
    assert!(combat_observation::validate(&across).is_err());
    let mut within = build(&state, &combat_state, 2);
    let dup = within.teammates[0].clone();
    within.teammates.push(dup);
    assert!(combat_observation::validate(&within).is_err());

    combat_state.projectiles = vec![CombatProjectile {
        family_id: ActionFamilyId::Ranged,
        source_index: 7,
        source_sequence: 1,
        pos: Vec2::new(400.0, 270.0),
        dir: Vec2::new(-1.0, 0.0),
        remaining_ticks: 40,
    }];
    let mut with_projectile = build(&state, &combat_state, 2);
    assert_eq!(with_projectile.projectiles.len(), 1);
    assert!(combat_observation::validate(&with_projectile).is_ok());
    let dup_projectile = with_projectile.projectiles[0].clone();
    with_projectile.projectiles.push(dup_projectile);
    with_projectile
        .projectile_order
        .push(with_projectile.projectiles[0].projectile_id.clone());
    let err = combat_observation::validate(&with_projectile)
        .expect_err("a duplicated projectile row must be rejected");
    assert!(err.contains("duplicate projectile"), "{err}");
}

#[test]
fn combat_sim_observation_v1_orders_projectiles_by_source_sequence_source_player_then_id() {
    let (state, mut combat_state) = new_match(None);
    combat_state.projectiles = vec![
        CombatProjectile {
            family_id: ActionFamilyId::Ranged,
            source_index: 8,
            source_sequence: 5,
            pos: Vec2::new(300.0, 200.0),
            dir: Vec2::new(-1.0, 0.0),
            remaining_ticks: 20,
        },
        CombatProjectile {
            family_id: ActionFamilyId::Ranged,
            source_index: 7,
            source_sequence: 2,
            pos: Vec2::new(320.0, 210.0),
            dir: Vec2::new(-1.0, 0.0),
            remaining_ticks: 30,
        },
    ];
    let mut observation = build(&state, &combat_state, 2);
    assert_eq!(observation.projectiles.len(), 2);
    assert_eq!(observation.projectiles[0].source_sequence, 2);
    assert_eq!(observation.projectiles[1].source_sequence, 5);
    for (index, id) in observation.projectile_order.iter().enumerate() {
        assert_eq!(*id, observation.projectiles[index].projectile_id);
    }
    assert!(combat_observation::validate(&observation).is_ok());

    observation.projectiles.swap(0, 1);
    observation.projectile_order[0] = observation.projectiles[0].projectile_id.clone();
    observation.projectile_order[1] = observation.projectiles[1].projectile_id.clone();
    let err = combat_observation::validate(&observation)
        .expect_err("reordered projectiles must be rejected");
    assert!(err.contains("canonical order"), "{err}");
}

#[test]
fn combat_sim_observation_v1_gives_every_projectile_a_collision_free_id_from_its_source_and_sequence()
 {
    assert_eq!(
        combat_observation::projectile_id("zyro_vex", 3),
        "8:zyro_vex/3"
    );
    // Without the length prefix, `("a", 11)` and `("a/1", 1)` would collide.
    assert_ne!(
        combat_observation::projectile_id("a", 11),
        combat_observation::projectile_id("a/1", 1)
    );
}

#[test]
fn combat_sim_observation_v1_publishes_the_bounded_public_threat_path_of_a_committed_melee_row() {
    let (state, mut combat_state) = new_match(None);
    {
        let runtime = &mut combat_state.players[6]; // canonical player index 7
        runtime.family_id = Some(ActionFamilyId::LightMelee);
        runtime.loadout_id = Some("loadout_vector_blade".to_string());
        runtime.phase = CombatActionPhase::Windup;
        runtime.phase_ticks = 5;
        runtime.source_sequence = Some(4);
    }
    combat_state.tick = 40;
    let observation = build(&state, &combat_state, 2);
    let (_, phase, _) =
        combat_observation::telegraph(Some(&combat_state), 7).expect("a public telegraph");
    assert_eq!(phase, CombatActionPhase::Windup);
    for peer in &observation.opponents {
        if peer.player_index == 7 {
            assert_eq!(peer.family_id, Some(ActionFamilyId::LightMelee));
            // Twelve windup ticks, five of which are gone; the remaining
            // five plus five active ticks end the public path at tick 50.
            assert_eq!(peer.telegraph_start_tick, 33);
            assert_eq!(peer.telegraph_end_tick, 50);
            assert_eq!(peer.projected_reach_px, 42.0);
            assert!(!peer.release_latched);
            assert_eq!(peer.projected_spawn_tick, 0);
        }
    }
}

#[test]
fn combat_sim_observation_v1_holds_the_same_sentinel_discipline_on_the_self_row() {
    let (state, mut combat_state) = new_match(None);
    // A player with no accepted action: every optional value is present as
    // its sentinel rather than absent, which is what keeps the row total.
    let idle = build(&state, &combat_state, 2);
    assert_eq!(idle.own.source_sequence, 0);
    assert_eq!(idle.own.forced_state, None);
    assert!(!idle.own.release_latched);
    assert_eq!(idle.own.projected_spawn_tick, 0);
    assert!(combat_observation::validate(&idle).is_ok());

    // A ranged self row publishes its spawn tick only once its own release
    // latch is set, exactly as a peer row does.
    {
        let runtime = &mut combat_state.players[1]; // canonical player index 2
        runtime.family_id = Some(ActionFamilyId::Ranged);
        runtime.loadout_id = Some("loadout_pulse_blaster".to_string());
        runtime.phase = CombatActionPhase::Windup;
        runtime.phase_ticks = 6;
        runtime.source_sequence = Some(3);
    }
    combat_state.tick = 100;
    let unlatched = build(&state, &combat_state, 2);
    assert!(!unlatched.own.release_latched);
    assert_eq!(unlatched.own.projected_spawn_tick, 0);

    combat_state.players[1].release_latched = true;
    let latched = build(&state, &combat_state, 2);
    assert!(latched.own.release_latched);
    assert_eq!(latched.own.projected_spawn_tick, 106);

    // A non-ranged family never claims a latch or a spawn tick.
    combat_state.players[1].family_id = Some(ActionFamilyId::LightMelee);
    combat_state.players[1].loadout_id = Some("loadout_vector_blade".to_string());
    let melee = build(&state, &combat_state, 2);
    assert!(!melee.own.release_latched);
    assert_eq!(melee.own.projected_spawn_tick, 0);
}

#[test]
fn combat_sim_observation_v1_publishes_a_ranged_spawn_tick_only_once_the_release_latch_is_public() {
    let (state, mut combat_state) = new_match(None);
    {
        let runtime = &mut combat_state.players[6]; // canonical player index 7
        runtime.family_id = Some(ActionFamilyId::Ranged);
        runtime.loadout_id = Some("loadout_pulse_blaster".to_string());
        runtime.phase = CombatActionPhase::Windup;
        runtime.phase_ticks = 6;
        runtime.source_sequence = Some(2);
    }
    combat_state.tick = 100;

    let unlatched = build(&state, &combat_state, 2);
    for peer in &unlatched.opponents {
        if peer.player_index == 7 {
            assert!(!peer.release_latched);
            assert_eq!(peer.projected_spawn_tick, 0);
            assert_eq!(peer.telegraph_end_tick, 0);
        }
    }

    combat_state.players[6].release_latched = true;
    let latched = build(&state, &combat_state, 2);
    for peer in &latched.opponents {
        if peer.player_index == 7 {
            assert!(peer.release_latched);
            assert_eq!(peer.projected_spawn_tick, 106);
            assert_eq!(peer.telegraph_end_tick, 107);
        }
    }
}
