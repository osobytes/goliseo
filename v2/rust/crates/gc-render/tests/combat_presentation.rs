//! Carry-forward from `spec/game/combat_presentation_spec.lua`.
//!
//! That spec's first `t.describe` ("combat presentation projection") drives
//! `game/presentation/combat.lua`, TypeScript in this port (`@gc/presentation`,
//! covered by that package's own spec). Its second `t.describe`, "shared
//! player pose priority", exercises `render.player_pose.select` — Rust here
//! — and was deferred to this crate by the TypeScript presentation agent for
//! exactly that reason. Both of that block's `t.it` cases are ported below.

use gc_core::vec2::Vec2;
use gc_data::teams;
use gc_render::player_pose::{
    self, CombatPoseSample, PlayerPoseId, PlayerPoseSelection, PlayerPoseSource,
};
use gc_sim::aerial::AerialStyle;
use gc_sim::combat_feasibility::CombatActionPhase;
use gc_sim::combat_snapshot::CombatForcedState;
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{MatchState, PitchSize};

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

#[test]
fn keeps_keeper_aerial_forced_and_committed_combat_poses_in_one_order() {
    let mut state = fixture();
    let player = &mut state.players[1];

    let mut sample = CombatPoseSample {
        phase: CombatActionPhase::Windup,
        forced_state: None,
        forced_ticks: 2,
    };
    assert_eq!(
        player_pose::select(player, Some(&sample), None, None).id,
        PlayerPoseId::CombatWindup
    );

    sample.forced_state = Some(CombatForcedState::Stagger);
    sample.forced_ticks = 5;
    assert_eq!(
        player_pose::select(player, Some(&sample), None, None).id,
        PlayerPoseId::CombatStagger
    );

    player.aerial_timer = 0.2;
    player.aerial_style = Some(AerialStyle::Header);
    assert_eq!(
        player_pose::select(player, Some(&sample), None, None).id,
        PlayerPoseId::AerialAction
    );

    player.aerial_timer = 0.0;
    player.aerial_style = None;
    player.is_keeper = true;
    player.dive_timer = 0.2;
    player.dive_dir = Vec2::new(1.0, 0.0);
    assert_eq!(
        player_pose::select(player, Some(&sample), None, None).id,
        PlayerPoseId::KeeperDive
    );
    assert!(
        PlayerPoseId::KeeperDive.priority() > PlayerPoseId::AerialAction.priority()
            && PlayerPoseId::AerialAction.priority() > PlayerPoseId::CombatKnockback.priority()
            && PlayerPoseId::CombatKnockback.priority() > PlayerPoseId::CombatActive.priority()
            && PlayerPoseId::CombatActive.priority() > PlayerPoseId::SoccerWindup.priority(),
        "the extensible priority contract is explicit"
    );
}

#[test]
fn chooses_overlapping_poses_from_declared_priority_with_a_stable_tie_rule() {
    let mut state = fixture();
    let player = &mut state.players[1];
    player.windup_timer = 0.2;
    player.slide_timer = 0.2;

    let default = player_pose::select(player, None, None, None).id;
    assert_eq!(default, PlayerPoseId::SoccerWindup);

    // The Lua original raises `slide`'s declared priority above
    // `soccer_windup`'s by mutating the module-global `PRIORITY` table in
    // place, observes the winner flip, then sets them equal and observes
    // the lexical tie-break. `player_pose::resolve` is `select`'s reduction
    // step, factored out precisely so this contract is testable without a
    // mutable global — see that function's own doc comment for why.
    let soccer_windup_priority = PlayerPoseId::SoccerWindup.priority();
    let raised = [
        PlayerPoseSelection {
            id: PlayerPoseId::SoccerWindup,
            priority: soccer_windup_priority,
            source: PlayerPoseSource::Soccer,
        },
        PlayerPoseSelection {
            id: PlayerPoseId::Slide,
            priority: soccer_windup_priority + 1,
            source: PlayerPoseSource::Soccer,
        },
        PlayerPoseSelection {
            id: PlayerPoseId::Locomotion,
            priority: PlayerPoseId::Locomotion.priority(),
            source: PlayerPoseSource::Locomotion,
        },
    ];
    assert_eq!(player_pose::resolve(&raised).id, PlayerPoseId::Slide);

    let tied = [
        PlayerPoseSelection {
            id: PlayerPoseId::SoccerWindup,
            priority: soccer_windup_priority,
            source: PlayerPoseSource::Soccer,
        },
        PlayerPoseSelection {
            id: PlayerPoseId::Slide,
            priority: soccer_windup_priority,
            source: PlayerPoseSource::Soccer,
        },
        PlayerPoseSelection {
            id: PlayerPoseId::Locomotion,
            priority: PlayerPoseId::Locomotion.priority(),
            source: PlayerPoseSource::Locomotion,
        },
    ];
    assert_eq!(
        player_pose::resolve(&tied).id,
        PlayerPoseId::Slide,
        "equal priorities choose the lexically smaller pose id"
    );
}
