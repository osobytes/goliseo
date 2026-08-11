//! Tests outfield pose projection: `player_pose::select`, driven directly
//! for the seven contract poses below.
//!
//! Other outfield pose behavior — release follow-through, pitch projection,
//! poses through the goal replay buffer, follow-through driven by the
//! simulation, contain against the counter-press window, and drawing all
//! seven contract poses with distinct procedural geometry — is TypeScript
//! (`@gc/render`), covered by that package's own spec.

use gc_data::teams;
use gc_render::player_pose::{self, OutfieldPoseContext, PlayerPoseId};
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{MatchState, PitchSize};
use gc_sim::offball_runs;
use gc_sim::outfield_decision::{self, OutfieldDecisionContext, OutfieldIntent};

fn fixture() -> MatchState {
    let home = teams::get("nebula").expect("nebula team is authored");
    let away = teams::get("orion").expect("orion team is authored");
    sim_match::new(NewMatchOptions {
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
    })
}

const NOW: f64 = 12.5;

/// The seven contract poses this test file drives one at a time.
const POSES: [PlayerPoseId; 7] = [
    PlayerPoseId::RunTelegraph,
    PlayerPoseId::KickFollow,
    PlayerPoseId::Settle,
    PlayerPoseId::Contain,
    PlayerPoseId::Tackle,
    PlayerPoseId::Stumble,
    PlayerPoseId::Fatigue,
];

/// The one authoritative overlap order, highest first. Everything below the
/// committed-combat band lives here; the combat band itself is
/// `player_pose`'s own concern (see `tests/keeper_pose.rs` and
/// `tests/combat_presentation.rs` for the combat-adjacent cases), asserted
/// separately so the two contracts stay visibly independent.
const PRIORITY_ORDER: [PlayerPoseId; 10] = [
    PlayerPoseId::SoccerWindup,
    PlayerPoseId::Slide,
    PlayerPoseId::Tackle,
    PlayerPoseId::Stumble,
    PlayerPoseId::KickFollow,
    PlayerPoseId::Settle,
    PlayerPoseId::RunTelegraph,
    PlayerPoseId::Contain,
    PlayerPoseId::Fatigue,
    PlayerPoseId::Locomotion,
];

fn clear_signals(player: &mut gc_sim::match_snapshot::MatchPlayer) {
    player.windup_timer = 0.0;
    player.slide_timer = 0.0;
    player.tackle_timer = 0.0;
    player.stun_timer = 0.0;
    player.settle_timer = 0.0;
    player.jockey_timer = 0.0;
    player.sprint_meter = 1.0;
    player.sprinting = false;
    player.outfield_decision = outfield_decision::new_state(Some(1.0));
}

/// Put a granted, still-telegraphing run on the player, exactly as
/// `match._assign_runs` would have left it.
fn grant_run(player: &mut gc_sim::match_snapshot::MatchPlayer, elapsed: f64) {
    player.outfield_decision = outfield_decision::refresh(
        &outfield_decision::new_state(Some(1.0)),
        OutfieldDecisionContext::Offball,
        OutfieldIntent::InBehind,
        0.5,
        Some(400.0),
        Some(260.0),
        None,
        Some(NOW - elapsed + offball_runs::RUN_LIFETIME_SECONDS),
    );
}

fn enable(
    player: &mut gc_sim::match_snapshot::MatchPlayer,
    id: PlayerPoseId,
    context: &mut OutfieldPoseContext,
) {
    match id {
        PlayerPoseId::SoccerWindup => player.windup_timer = 0.1,
        PlayerPoseId::Slide => player.slide_timer = 0.1,
        PlayerPoseId::Tackle => player.tackle_timer = 0.1,
        PlayerPoseId::Stumble => player.stun_timer = 0.1,
        PlayerPoseId::KickFollow => context.kick_follow = true,
        PlayerPoseId::Settle => player.settle_timer = 0.1,
        PlayerPoseId::RunTelegraph => grant_run(player, 0.05),
        PlayerPoseId::Contain => player.jockey_timer = 0.1,
        PlayerPoseId::Fatigue => {
            player.sprint_meter = 0.0;
            player.sprinting = false;
        }
        _ => {}
    }
}

fn disable(
    player: &mut gc_sim::match_snapshot::MatchPlayer,
    id: PlayerPoseId,
    context: &mut OutfieldPoseContext,
) {
    match id {
        PlayerPoseId::SoccerWindup => player.windup_timer = 0.0,
        PlayerPoseId::Slide => player.slide_timer = 0.0,
        PlayerPoseId::Tackle => player.tackle_timer = 0.0,
        PlayerPoseId::Stumble => player.stun_timer = 0.0,
        PlayerPoseId::KickFollow => context.kick_follow = false,
        PlayerPoseId::Settle => player.settle_timer = 0.0,
        PlayerPoseId::RunTelegraph => {
            player.outfield_decision = outfield_decision::new_state(Some(1.0))
        }
        PlayerPoseId::Contain => {
            player.jockey_timer = 0.0;
            context.containing = false;
        }
        PlayerPoseId::Fatigue => player.sprint_meter = 1.0,
        _ => {}
    }
}

#[test]
fn selects_every_contract_pose_from_simulation_owned_state() {
    let mut state = fixture();
    let player = &mut state.players[1];
    let mut context = OutfieldPoseContext {
        now: Some(NOW),
        containing: false,
        kick_follow: false,
    };

    clear_signals(player);
    assert_eq!(
        player_pose::select(player, None, None, Some(&context)).id,
        PlayerPoseId::Locomotion
    );

    for id in POSES {
        clear_signals(player);
        context.kick_follow = false;
        context.containing = false;
        enable(player, id, &mut context);
        assert_eq!(
            player_pose::select(player, None, None, Some(&context)).id,
            id,
            "{id:?}"
        );
    }
}

#[test]
fn reads_the_telegraph_window_from_the_simulation_run_grant() {
    let mut state = fixture();
    let player = &mut state.players[1];
    let context = OutfieldPoseContext {
        now: Some(NOW),
        containing: false,
        kick_follow: false,
    };
    clear_signals(player);

    grant_run(player, 0.0);
    assert_eq!(
        player_pose::select(player, None, None, Some(&context)).id,
        PlayerPoseId::RunTelegraph
    );
    grant_run(player, offball_runs::TELEGRAPH_SECONDS * 0.5);
    assert_eq!(
        player_pose::select(player, None, None, Some(&context)).id,
        PlayerPoseId::RunTelegraph
    );
    // The sustained run is the sprint trail's job, not a pose's.
    grant_run(player, offball_runs::TELEGRAPH_SECONDS);
    assert_eq!(
        player_pose::select(player, None, None, Some(&context)).id,
        PlayerPoseId::Locomotion
    );
    grant_run(player, offball_runs::RUN_LIFETIME_SECONDS * 0.5);
    assert_eq!(
        player_pose::select(player, None, None, Some(&context)).id,
        PlayerPoseId::Locomotion
    );

    // A non-run intent never telegraphs, and neither does a caller who
    // supplied no clock.
    clear_signals(player);
    player.outfield_decision = outfield_decision::refresh(
        &outfield_decision::new_state(Some(1.0)),
        OutfieldDecisionContext::Offball,
        OutfieldIntent::Move,
        0.5,
        Some(400.0),
        Some(260.0),
        None,
        None,
    );
    assert_eq!(
        player_pose::select(player, None, None, Some(&context)).id,
        PlayerPoseId::Locomotion
    );
    grant_run(player, 0.0);
    let no_clock = OutfieldPoseContext {
        now: None,
        containing: false,
        kick_follow: false,
    };
    assert_eq!(
        player_pose::select(player, None, None, Some(&no_clock)).id,
        PlayerPoseId::Locomotion
    );
}

#[test]
fn holds_one_explicit_overlap_order_so_stacked_signals_cannot_flicker() {
    let mut state = fixture();
    let player = &mut state.players[1];
    let mut context = OutfieldPoseContext {
        now: Some(NOW),
        containing: false,
        kick_follow: false,
    };
    clear_signals(player);

    for id in PRIORITY_ORDER {
        enable(player, id, &mut context);
    }
    for (index, id) in PRIORITY_ORDER.iter().enumerate() {
        assert_eq!(
            player_pose::select(player, None, None, Some(&context)).id,
            *id,
            "rank {index}"
        );
        disable(player, *id, &mut context);
    }

    // The published priority table must agree with the order the specs assert.
    for window in PRIORITY_ORDER.windows(2) {
        let higher = window[0].priority();
        let lower = window[1].priority();
        assert!(
            higher > lower,
            "{:?} must outrank {:?}",
            window[0],
            window[1]
        );
    }
}

#[test]
fn never_lends_an_outfield_pose_to_a_goalkeeper() {
    use gc_render::player_pose::KeeperPoseContext;

    let mut state = fixture();
    let keeper = &mut state.players[0];
    let neutral = KeeperPoseContext {
        near_ball: false,
        shuffling: false,
        tip: false,
    };
    let context = OutfieldPoseContext {
        now: Some(NOW),
        containing: true,
        kick_follow: true,
    };
    keeper.tackle_timer = 0.1;
    keeper.stun_timer = 0.1;
    keeper.settle_timer = 0.1;
    keeper.jockey_timer = 0.1;
    keeper.sprint_meter = 0.0;
    grant_run(keeper, 0.0);
    assert_eq!(
        player_pose::select(keeper, None, Some(&neutral), Some(&context)).id,
        PlayerPoseId::KeeperReadyTall
    );
}

#[test]
fn maps_both_teams_through_the_same_state() {
    let mut state = fixture();
    let mut context = OutfieldPoseContext {
        now: Some(NOW),
        containing: false,
        kick_follow: false,
    };
    for index in [1usize, 6] {
        let player = &mut state.players[index];
        for id in POSES {
            clear_signals(player);
            context.kick_follow = false;
            context.containing = false;
            enable(player, id, &mut context);
            assert_eq!(
                player_pose::select(player, None, None, Some(&context)).id,
                id,
                "{:?} {id:?}",
                player.team
            );
        }
    }
}

#[test]
fn shares_the_contain_silhouette_between_ai_contain_and_the_human_jockey() {
    let mut state = fixture();
    let player = &mut state.players[1];
    clear_signals(player);

    let ai_context = OutfieldPoseContext {
        now: Some(NOW),
        containing: true,
        kick_follow: false,
    };
    let ai_only = player_pose::select(player, None, None, Some(&ai_context));
    player.jockey_timer = 0.2;
    let jockey_context = OutfieldPoseContext {
        now: Some(NOW),
        containing: false,
        kick_follow: false,
    };
    let jockey_only = player_pose::select(player, None, None, Some(&jockey_context));
    assert_eq!(ai_only.id, PlayerPoseId::Contain);
    assert_eq!(jockey_only.id, PlayerPoseId::Contain);
    assert_eq!(ai_only.priority, jockey_only.priority);
    // Same pose, separate mechanics: the pose module owns neither timer.
    assert_eq!(player.jockey_timer, 0.2);
}
