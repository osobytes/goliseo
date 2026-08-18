//! Combat presentation tests.
//!
//! "shared player pose priority" exercises `player_pose::select` (Rust) and
//! lives here rather than in `@gc/presentation`'s own spec: combat
//! presentation projection itself is TypeScript (`@gc/presentation`, covered
//! by that package's own spec), but pose priority resolution crosses the
//! language boundary into this crate.
//!
//! #441 added the two cases at the bottom, which are this crate's own: they
//! cover `frame::combat_model` (the adapter that finally feeds `select`'s
//! `combat` argument from live simulation state) and the reachability of all
//! seven combat poses through `frame::build`.

use gc_core::vec2::Vec2;
use gc_data::teams;
use gc_render::frame::{self, RenderFrameOptions};
use gc_render::player_pose::{
    self, CombatPoseSample, PlayerPoseId, PlayerPoseSelection, PlayerPoseSource,
};
use gc_sim::aerial::AerialStyle;
use gc_sim::combat;
use gc_sim::combat::IMMUNITY_TICKS;
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
        immunity_fraction: 0.0,
        phase_fraction: 0.0,
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

    // `player_pose::resolve` is `select`'s reduction step, factored out
    // precisely so the priority/tie-break contract can be driven with
    // explicit `PlayerPoseSelection` priorities below — raising `slide`'s
    // priority above `soccer_windup`'s to observe the winner flip, then
    // setting them equal to observe the lexical tie-break — without a
    // mutable global. See that function's own doc comment for why.
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

/// The adapter #441 added: one live `CombatMatchState` in, one
/// `FrameCombatModel` out, in canonical player order. `phase` needs no
/// mapping (`CombatPhase` IS `CombatActionPhase`, see
/// `gc_render::frame::combat_model`'s doc), so this checks the thing that
/// could actually go wrong: that slot N's model entry describes slot N's
/// player.
#[test]
fn the_combat_model_carries_each_slot_s_own_phase_and_forced_state() {
    let mut state = fixture();
    let mut combat = combat::new_state(&mut state, None);
    combat.players[1].phase = CombatActionPhase::Aim;
    combat.players[2].forced_state = Some(CombatForcedState::Knockback);
    combat.players[2].forced_ticks = 4;
    combat.players[2].immunity_ticks = IMMUNITY_TICKS;

    let model = frame::combat_model(&state, &combat);
    assert!(model.enabled, "a model is only built for a combat match");
    assert_eq!(model.players.len(), state.players.len());
    assert_eq!(model.players[0].phase, CombatActionPhase::Ready);
    assert_eq!(model.players[1].phase, CombatActionPhase::Aim);
    assert_eq!(model.players[1].forced_state, None);
    assert_eq!(
        model.players[2].forced_state,
        Some(CombatForcedState::Knockback)
    );
    assert_eq!(model.players[2].forced_ticks, 4);
    // Slot 0 has no immunity running (0 ticks): the cue is off.
    assert_eq!(model.players[0].immunity_fraction, 0.0);
    // Slot 2 was just hit -- immunity is at its full window -- so the cue
    // starts at exactly 1.0, not merely "greater than zero".
    assert_eq!(model.players[2].immunity_fraction, 1.0);
}

/// The immunity fraction's own contract, pinned as an exact ratio rather
/// than only bounds: it is `runtime.immunity_ticks / IMMUNITY_TICKS`, so a
/// renderer can read it as "how much of the post-hit window is left"
/// without knowing the window's length itself.
#[test]
fn the_combat_model_normalises_immunity_ticks_into_an_exact_fraction() {
    let mut state = fixture();
    let mut combat = combat::new_state(&mut state, None);
    combat.players[1].immunity_ticks = IMMUNITY_TICKS / 3;

    let model = frame::combat_model(&state, &combat);
    assert_eq!(
        model.players[1].immunity_fraction,
        (IMMUNITY_TICKS / 3) as f64 / IMMUNITY_TICKS as f64
    );
    assert!(model.players[1].immunity_fraction > 0.0 && model.players[1].immunity_fraction < 1.0);
}

/// `phase_fraction`'s own contract (#576), pinned as exact ratios against the
/// family's authored phase lengths, the same way `immunity_fraction` is
/// pinned above: elapsed progress through the current TIMED phase, so the
/// animator can sweep the swing through its strike key without owning a copy
/// of any family's timing table.
#[test]
fn the_combat_model_normalises_phase_ticks_into_elapsed_phase_progress() {
    use gc_data::action_families::{self, ActionFamilyId};

    let family = action_families::get(ActionFamilyId::Unarmed);

    // Windup, first tick: nothing elapsed yet.
    let mut state = fixture();
    let mut combat = combat::new_state(&mut state, None);
    combat.players[1].family_id = Some(ActionFamilyId::Unarmed);
    combat.players[1].phase = CombatActionPhase::Windup;
    combat.players[1].phase_ticks = family.windup_ticks;
    let model = frame::combat_model(&state, &combat);
    assert_eq!(model.players[1].phase_fraction, 0.0);

    // Windup, two ticks in.
    combat.players[1].phase_ticks = family.windup_ticks - 2;
    let model = frame::combat_model(&state, &combat);
    assert_eq!(
        model.players[1].phase_fraction,
        2.0 / family.windup_ticks as f64
    );

    // Active, last tick before recovery: (total - 1) / total, never 1.0 —
    // the phase transitions before a full fraction is ever reported.
    let active = family.active_ticks.expect("unarmed authors active_ticks");
    combat.players[1].phase = CombatActionPhase::Active;
    combat.players[1].phase_ticks = 1;
    let model = frame::combat_model(&state, &combat);
    assert_eq!(
        model.players[1].phase_fraction,
        (active - 1) as f64 / active as f64
    );
    assert!(model.players[1].phase_fraction < 1.0);

    // Recovery normalises against ITS phase's length, not the windup's.
    combat.players[1].phase = CombatActionPhase::Recovery;
    combat.players[1].phase_ticks = family.recovery_ticks - 3;
    let model = frame::combat_model(&state, &combat);
    assert_eq!(
        model.players[1].phase_fraction,
        3.0 / family.recovery_ticks as f64
    );

    // The held phases have no length to be a fraction of.
    for held in [
        CombatActionPhase::Ready,
        CombatActionPhase::Guard,
        CombatActionPhase::Aim,
    ] {
        combat.players[1].phase = held;
        combat.players[1].phase_ticks = 0;
        let model = frame::combat_model(&state, &combat);
        assert_eq!(model.players[1].phase_fraction, 0.0, "{held:?}");
    }

    // No family (a keeper's slot, or any player authored without a loadout)
    // reports 0.0 whatever the phase machine claims: there is no timing
    // table to normalise against.
    combat.players[2].family_id = None;
    combat.players[2].phase = CombatActionPhase::Windup;
    combat.players[2].phase_ticks = 3;
    let model = frame::combat_model(&state, &combat);
    assert_eq!(model.players[2].phase_fraction, 0.0);
}

/// The per-player column at the `frame::build` seam (#576): the fraction the
/// combat model reports for a slot is the fraction the built frame's
/// structure-of-arrays carries for it, and a frame without combat carries a
/// dense column of zeros rather than no column at all.
#[test]
fn the_built_frame_carries_each_slot_s_phase_fraction() {
    use gc_data::action_families::{self, ActionFamilyId};

    let family = action_families::get(ActionFamilyId::Unarmed);
    let mut state = fixture();
    let mut combat = combat::new_state(&mut state, None);
    combat.players[1].family_id = Some(ActionFamilyId::Unarmed);
    combat.players[1].phase = CombatActionPhase::Active;
    combat.players[1].phase_ticks = 1;
    let expected = (family.active_ticks.expect("unarmed authors active_ticks") - 1) as f64
        / family.active_ticks.expect("unarmed authors active_ticks") as f64;

    let with_combat = frame::build(
        &state,
        &RenderFrameOptions {
            combat: Some(frame::combat_model(&state, &combat)),
            ..RenderFrameOptions::default()
        },
    );
    assert_eq!(with_combat.players.phase_fraction[1], expected);
    assert_eq!(with_combat.players.phase_fraction[0], 0.0);

    let without = frame::build(&state, &RenderFrameOptions::default());
    assert_eq!(without.players.phase_fraction.len(), state.players.len());
    assert!(without.players.phase_fraction.iter().all(|&f| f == 0.0));
}

/// The alignment guard, shown FIRING rather than merely present.
///
/// The case above pins positional correspondence on a well-formed pair, but
/// a passing assertion and an absent one look identical from outside. This
/// hands `combat_model` a genuinely reordered `player_ids` and requires the
/// panic. Misalignment is the one failure in this module that would look
/// like working software — a bystander staggering while the struck player
/// runs on — so it is the guard most worth having tested.
#[test]
#[should_panic(expected = "combat presentation player identity mismatch")]
fn a_reordered_combat_state_panics_instead_of_posing_the_wrong_player() {
    let mut state = fixture();
    let mut combat = combat::new_state(&mut state, None);
    assert_ne!(combat.player_ids[1], combat.player_ids[2]);
    combat.player_ids.swap(1, 2);
    let _ = frame::combat_model(&state, &combat);
}

/// The other half of the same guard: a combat companion describing a
/// different-sized fixture is refused before any slot is read, rather than
/// silently posing whichever prefix happened to line up.
#[test]
#[should_panic(expected = "combat presentation player count mismatch")]
fn a_short_combat_state_panics_instead_of_posing_a_prefix() {
    let mut state = fixture();
    let mut combat = combat::new_state(&mut state, None);
    combat.players.pop();
    let _ = frame::combat_model(&state, &combat);
}

/// WHAT THIS GUARD DOES NOT COVER, recorded so nobody reads the two cases
/// above as broader than they are: `player_ids` is the only identity the
/// combat state carries, so a `players` vector reordered while `player_ids`
/// stays put is undetectable here — and by `gc_sim::combat_snapshot::copy`'s
/// identical assertion, which is where this one is modelled on. That pairing
/// cannot be produced upstream (`gc_sim::combat::new_state` builds both
/// vectors in one pass over `state.players`, and every step and rollback
/// moves the whole `CombatMatchState` as a unit), which is why neither layer
/// tries to detect it. This case pins the honest scope: ids aligned, so no
/// panic, whatever the runtimes say.
#[test]
fn an_id_aligned_pair_is_accepted_and_that_is_the_whole_check() {
    let mut state = fixture();
    let mut combat = combat::new_state(&mut state, None);
    combat.players.swap(1, 2);
    let model = frame::combat_model(&state, &combat);
    assert_eq!(model.players.len(), state.players.len());
}

/// THE REACHABILITY CONTRACT #441 exists for, at the `frame::build` seam:
/// every one of the seven combat poses must come out of a built frame's
/// `pose_id` column when the match's own combat state says so. Before the
/// fix `RenderFrameOptions.combat` was never populated, so every case below
/// produced `Locomotion` instead.
///
/// The list is written out rather than derived so a new phase or forced
/// state has to be added here deliberately; `player_pose::select`'s own
/// `match` over `CombatActionPhase` is already exhaustive with no wildcard,
/// so a new variant cannot silently fall through to no pose at all.
#[test]
fn every_combat_pose_reaches_a_built_frame_s_pose_column() {
    let expectations: [(CombatActionPhase, Option<CombatForcedState>, PlayerPoseId); 7] = [
        (
            CombatActionPhase::Ready,
            Some(CombatForcedState::Knockback),
            PlayerPoseId::CombatKnockback,
        ),
        (
            CombatActionPhase::Ready,
            Some(CombatForcedState::Stagger),
            PlayerPoseId::CombatStagger,
        ),
        (CombatActionPhase::Guard, None, PlayerPoseId::CombatGuard),
        (CombatActionPhase::Active, None, PlayerPoseId::CombatActive),
        (CombatActionPhase::Windup, None, PlayerPoseId::CombatWindup),
        (CombatActionPhase::Aim, None, PlayerPoseId::CombatAim),
        (
            CombatActionPhase::Recovery,
            None,
            PlayerPoseId::CombatRecovery,
        ),
    ];

    for (phase, forced_state, expected) in expectations {
        let mut state = fixture();
        let mut combat = combat::new_state(&mut state, None);
        combat.players[1].phase = phase;
        combat.players[1].forced_state = forced_state;
        combat.players[1].forced_ticks = if forced_state.is_some() { 3 } else { 0 };

        let without = frame::build(&state, &RenderFrameOptions::default());
        assert_eq!(
            without.players.pose_id[1],
            PlayerPoseId::Locomotion,
            "no combat model means no combat pose, whatever the simulation says"
        );

        let options = RenderFrameOptions {
            combat: Some(frame::combat_model(&state, &combat)),
            ..Default::default()
        };
        let built = frame::build(&state, &options);
        assert_eq!(built.players.pose_id[1], expected);
        assert_eq!(built.players.pose_source[1], PlayerPoseSource::Combat);
        // ...and only the slot the combat state named.
        assert_eq!(built.players.pose_id[2], PlayerPoseId::Locomotion);
    }
}
