//! Port of `spec/render/keeper_pose_spec.lua`.
//!
//! The Lua spec has five `t.it` cases. The first three exercise
//! `render.player_pose.select` directly and belong here. The last two
//! ("draws all nine contract poses with distinct procedural geometry" and
//! "threads the smother boundary and batched tip event through pitch draw")
//! drive `game/render/player_renderer.lua` and `game/render/pitch.lua`
//! through a stubbed `love.graphics` — both TypeScript in this port
//! (`@gc/render`), covered by that package's own spec.

use gc_data::teams;
use gc_render::player_pose::{self, KeeperPoseContext, PlayerPoseId};
use gc_sim::keeper::{KeeperBehaviorState, SaveStyle};
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

const NEUTRAL: KeeperPoseContext = KeeperPoseContext {
    near_ball: false,
    shuffling: false,
    tip: false,
};

#[test]
fn selects_every_simulation_owned_positioning_and_save_state() {
    let mut state = fixture();
    let keeper = &mut state.players[0];
    let mut context = NEUTRAL;

    assert_eq!(
        player_pose::select(keeper, None, Some(&context), None).id,
        PlayerPoseId::KeeperReadyTall
    );

    context.near_ball = true;
    assert_eq!(
        player_pose::select(keeper, None, Some(&context), None).id,
        PlayerPoseId::KeeperReadyLow
    );
    context.near_ball = false;
    context.shuffling = true;
    assert_eq!(
        player_pose::select(keeper, None, Some(&context), None).id,
        PlayerPoseId::KeeperShuffle
    );
    context.shuffling = false;

    keeper.keeper_state = KeeperBehaviorState::Set;
    assert_eq!(
        player_pose::select(keeper, None, Some(&context), None).id,
        PlayerPoseId::KeeperSet
    );
    // "recover" is positioning recovery, not getting up off the floor.
    keeper.keeper_state = KeeperBehaviorState::Recover;
    assert_eq!(
        player_pose::select(keeper, None, Some(&context), None).id,
        PlayerPoseId::KeeperReadyTall
    );
    keeper.keeper_state = KeeperBehaviorState::Base;

    keeper.keeper_get_up_timer = 0.18;
    assert_eq!(
        player_pose::select(keeper, None, Some(&context), None).id,
        PlayerPoseId::KeeperGetUp
    );
    keeper.keeper_get_up_timer = 0.0;
    assert_eq!(
        player_pose::select(keeper, None, Some(&context), None).id,
        PlayerPoseId::KeeperReadyTall
    );

    keeper.dive_timer = 0.2;
    for (style, expected) in [
        (SaveStyle::Spread, PlayerPoseId::KeeperSpread),
        (SaveStyle::Central, PlayerPoseId::KeeperCentral),
        (SaveStyle::Stretch, PlayerPoseId::KeeperStretch),
    ] {
        keeper.save_style = Some(style);
        assert_eq!(
            player_pose::select(keeper, None, Some(&context), None).id,
            expected
        );
    }
    keeper.dive_timer = 0.0;
    keeper.save_style = None;
    context.tip = true;
    assert_eq!(
        player_pose::select(keeper, None, Some(&context), None).id,
        PlayerPoseId::KeeperTip
    );
}

#[test]
fn reads_getting_up_from_the_simulation_timer_never_the_recover_state() {
    let mut state = fixture();
    let keeper = &mut state.players[0];

    // The get-up window outranks the set/lean and both ready postures while
    // it runs, and hands every one of them back the moment it expires.
    for (context, after) in [
        (
            KeeperPoseContext {
                near_ball: false,
                shuffling: false,
                tip: false,
            },
            PlayerPoseId::KeeperReadyTall,
        ),
        (
            KeeperPoseContext {
                near_ball: true,
                shuffling: false,
                tip: false,
            },
            PlayerPoseId::KeeperReadyLow,
        ),
        (
            KeeperPoseContext {
                near_ball: false,
                shuffling: true,
                tip: false,
            },
            PlayerPoseId::KeeperShuffle,
        ),
    ] {
        keeper.keeper_get_up_timer = 0.18;
        assert_eq!(
            player_pose::select(keeper, None, Some(&context), None).id,
            PlayerPoseId::KeeperGetUp
        );
        keeper.keeper_get_up_timer = 0.0;
        assert_eq!(
            player_pose::select(keeper, None, Some(&context), None).id,
            after
        );
    }

    let neutral = NEUTRAL;
    keeper.keeper_get_up_timer = 0.18;
    keeper.keeper_state = KeeperBehaviorState::Set;
    assert_eq!(
        player_pose::select(keeper, None, Some(&neutral), None).id,
        PlayerPoseId::KeeperGetUp
    );
    keeper.keeper_state = KeeperBehaviorState::Base;

    // Dive, tip, and hand poses all stay ahead of it.
    keeper.dive_timer = 0.2;
    keeper.save_style = Some(SaveStyle::Stretch);
    assert_eq!(
        player_pose::select(keeper, None, Some(&neutral), None).id,
        PlayerPoseId::KeeperStretch
    );
    let tip_context = KeeperPoseContext {
        near_ball: false,
        shuffling: false,
        tip: true,
    };
    assert_eq!(
        player_pose::select(keeper, None, Some(&tip_context), None).id,
        PlayerPoseId::KeeperTip
    );
    keeper.grab_timer = 0.1;
    assert_eq!(
        player_pose::select(keeper, None, Some(&neutral), None).id,
        PlayerPoseId::KeeperGrab
    );
    keeper.grab_timer = 0.0;
    keeper.dive_timer = 0.0;
    keeper.save_style = None;

    // Every keeper_state, including "recover", selects a ready posture once
    // the get-up window is spent.
    keeper.keeper_get_up_timer = 0.0;
    for keeper_state in [
        KeeperBehaviorState::Base,
        KeeperBehaviorState::Advance,
        KeeperBehaviorState::Contain,
        KeeperBehaviorState::Retreat,
        KeeperBehaviorState::Recover,
    ] {
        keeper.keeper_state = keeper_state;
        assert_eq!(
            player_pose::select(keeper, None, Some(&neutral), None).id,
            PlayerPoseId::KeeperReadyTall,
            "{keeper_state:?}"
        );
    }
}

#[test]
fn keeps_grab_throw_and_punt_above_save_and_ready_variants() {
    let mut state = fixture();
    let keeper = &mut state.players[0];
    let context = KeeperPoseContext {
        near_ball: true,
        shuffling: true,
        tip: true,
    };
    keeper.dive_timer = 0.2;
    keeper.save_style = Some(SaveStyle::Stretch);
    keeper.windup_timer = 0.1;
    keeper.throw_timer = 0.1;
    keeper.grab_timer = 0.1;
    assert_eq!(
        player_pose::select(keeper, None, Some(&context), None).id,
        PlayerPoseId::KeeperGrab
    );

    keeper.grab_timer = 0.0;
    assert_eq!(
        player_pose::select(keeper, None, Some(&context), None).id,
        PlayerPoseId::KeeperThrow
    );
    keeper.throw_timer = 0.0;
    assert_eq!(
        player_pose::select(keeper, None, Some(&context), None).id,
        PlayerPoseId::KeeperPunt
    );
    keeper.windup_timer = 0.0;
    assert_eq!(
        player_pose::select(keeper, None, Some(&context), None).id,
        PlayerPoseId::KeeperTip
    );
}
