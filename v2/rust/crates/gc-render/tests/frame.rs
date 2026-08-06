//! Port of `spec/render/frame_spec.lua`.
//!
//! Tier-1 logic tests: no display, no rendering. What they pin is the
//! payload's SHAPE and the promises the shape makes — that the per-entity
//! data really is structure-of-arrays, that no engine type leaks through it,
//! and that building it cannot perturb the simulation.

use gc_core::vec2::Vec2;
use gc_render::frame::{self as render_frame, RenderChargeKind, RenderFrameOptions, RenderPose};
use gc_render::player_pose::{self, KeeperPoseContext, OutfieldPoseContext, PlayerPoseId};
use gc_sim::aerial::{AerialOutcome, AerialStyle};
use gc_sim::keeper::{self, KeeperBehaviorState};
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchEvent, MatchEventKind, MatchInput, MatchState, PitchSize};
use gc_sim::outfield_press::StablePressMode;
use gc_sim::tuning::Tuning;

fn fixture(seed: f64) -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(seed),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    })
}

fn step(s: &mut MatchState, tune: &Tuning) {
    sim_match::step(
        s,
        1.0 / 60.0,
        StepInput::Legacy(MatchInput::default()),
        None,
        tune,
    );
}

/// An event with every optional field absent, for tests to override with
/// struct-update syntax. `MatchEvent` has no `Default` (it is a wire-shaped
/// snapshot type owned by `gc-sim`), so this is the Rust equivalent of the
/// Lua fixture's inline `{ kind = ..., x = ..., y = ... }` table literals,
/// which likewise leave every other field `nil`.
fn bare_event(kind: MatchEventKind, x: f64, y: f64) -> MatchEvent {
    MatchEvent {
        kind,
        x,
        y,
        player: None,
        save_style: None,
        style: None,
        outcome: None,
        jumping: None,
        difficulty: None,
        shot_type: None,
        keeper_state: None,
        keeper_depth: None,
        on_target: None,
    }
}

#[test]
fn stamps_the_protocol_version_on_the_frame_and_the_roster() {
    let state = fixture(17.0);
    let frame = render_frame::build(&state, &RenderFrameOptions::default());
    assert_eq!(frame.version, render_frame::VERSION);
    assert_eq!(frame.roster.version, render_frame::VERSION);
    // `VERSION` is a `const` here rather than the Lua runtime value the
    // original spec checks, so this is a tautology clippy can prove at
    // compile time; kept (with the lint silenced) as the same documentation
    // of the contract the Lua spec states explicitly.
    #[allow(clippy::assertions_on_constants)]
    {
        assert!(
            render_frame::VERSION >= 1,
            "the payload must carry a bumpable integer version"
        );
    }
}

#[test]
fn keeps_per_entity_data_as_parallel_scalar_arrays_one_entry_per_slot() {
    let state = fixture(17.0);
    let frame = render_frame::build(&state, &RenderFrameOptions::default());
    let players = &frame.players;

    assert_eq!(players.count, state.players.len());
    assert_eq!(frame.roster.count, state.players.len());

    let n = players.count;
    for (name, len) in [
        ("x", players.x.len()),
        ("y", players.y.len()),
        ("facing_x", players.facing_x.len()),
        ("facing_y", players.facing_y.len()),
        ("speed", players.speed.len()),
        ("pose_id", players.pose_id.len()),
        ("pose_priority", players.pose_priority.len()),
        ("pose_source", players.pose_source.len()),
        ("controlled", players.controlled.len()),
        ("dashing", players.dashing.len()),
        ("holding", players.holding.len()),
        ("dive", players.dive.len()),
        ("dive_dir_x", players.dive_dir_x.len()),
        ("dive_dir_y", players.dive_dir_y.len()),
        ("grab", players.grab.len()),
        ("throw", players.throw.len()),
        ("windup", players.windup.len()),
        ("aerial", players.aerial.len()),
        ("aerial_jump", players.aerial_jump.len()),
        // The sparse arrays are allowed holes (`None`), but `Vec<Option<T>>`
        // is still one entry per roster slot, never a nested collection — a
        // buffer encoding maps an absent entry to a zero enum.
        ("aerial_style", players.aerial_style.len()),
        ("aerial_outcome", players.aerial_outcome.len()),
    ] {
        assert_eq!(len, n, "{name} must have one entry per roster slot");
    }
}

#[test]
fn carries_no_engine_types_across_the_boundary() {
    // The Lua original walks the whole payload asserting no nested table
    // carries a metatable ("Vec2 and every class in this codebase carries a
    // metatable; plain payload data never does"). Rust has no runtime
    // metatable-equivalent to walk: every field `RenderFrame` (and
    // everything it owns) can hold is a `String`, a primitive, a `Vec<_>`
    // of one of those, or a plain `#[derive(Clone, Debug, PartialEq)]`
    // struct/enum — there is no `love` handle, closure, or trait object a
    // field could smuggle through. That makes the property this spec checks
    // at runtime a static guarantee of the type here, checked once at
    // review time rather than every build. `combat` remains the documented
    // exception (see `FrameCombatModel`'s doc), matching the Lua original's
    // own `key ~= "combat"` skip.
    let state = fixture(17.0);
    let frame = render_frame::build(&state, &RenderFrameOptions::default());
    // A real, if weak, runtime witness that nothing opts out of the derived
    // traits to hide an opaque field: both walk every field to do their job.
    assert_eq!(frame, frame.clone());
    assert_eq!(format!("{frame:?}"), format!("{:?}", frame.clone()));
}

#[test]
fn does_not_perturb_the_simulation() {
    let tune = Tuning::new();
    let mut state = fixture(29.0);
    for _ in 0..30 {
        step(&mut state, &tune);
    }
    // The Lua original proves this via `match_snapshot.hash(match_snapshot
    // .capture(state))` before/after. `MatchState` derives `PartialEq` here
    // (`gc_sim::match_snapshot`), so comparing the whole state directly is
    // available and strictly stronger — no dependence on the snapshot
    // module's own (unrelated, in-flight) validation and hashing path.
    let before = state.clone();
    let roster = render_frame::roster(&state);
    for _ in 0..5 {
        let _ = render_frame::build(
            &state,
            &RenderFrameOptions {
                roster: Some(roster.clone()),
                ..Default::default()
            },
        );
        let _ = render_frame::hud(&state, Some(&roster));
    }
    assert_eq!(state, before);
}

#[test]
fn reuses_a_match_constant_roster_instead_of_rebuilding_it() {
    let state = fixture(17.0);
    let roster = render_frame::roster(&state);
    let expected = roster.clone();
    let frame = render_frame::build(
        &state,
        &RenderFrameOptions {
            roster: Some(roster),
            ..Default::default()
        },
    );
    // Rust's ownership model makes "carried, not copied" structural: `build`
    // moves the supplied roster straight into `frame.roster` rather than
    // calling `roster()` again, so there is no code path left that could
    // recompute it. Lua could only observe this via table-reference
    // identity (`frame.roster == roster`); value equality is the closest
    // Rust analogue and it still catches a producer that mutates fields.
    assert_eq!(
        frame.roster, expected,
        "a supplied roster must be carried, not copied"
    );
    assert_eq!(frame.roster.ids[0], state.players[0].id);
    assert_eq!(frame.roster.teams[0], state.players[0].team);
    assert_eq!(frame.roster.is_keeper[0], state.players[0].is_keeper);
    assert_eq!(frame.roster.species_color[0].len(), 3);
}

#[test]
fn selects_the_same_pose_the_pose_authority_does() {
    let tune = Tuning::new();
    let mut state = fixture(41.0);
    for _ in 0..45 {
        step(&mut state, &tune);
    }
    let frame = render_frame::build(&state, &RenderFrameOptions::default());
    let now = -state.time_left;
    for (zero_based, player) in state.players.iter().enumerate() {
        let index = (zero_based + 1) as i64;
        let keeper_context = player.is_keeper.then(|| KeeperPoseContext {
            near_ball: keeper::in_smother_range(player.pos.dist(state.ball)),
            shuffling: player.keeper_state == KeeperBehaviorState::Base
                && player.run_vel.y.abs() > 0.0,
            tip: false,
        });
        let outfield_context = (!player.is_keeper).then(|| {
            let press = state.outfield_press.get(player.team);
            OutfieldPoseContext {
                now: Some(now),
                containing: press.mode == StablePressMode::Contain
                    && press.presser_index == Some(index as u32),
                kick_follow: false,
            }
        });
        let expected = player_pose::select(
            player,
            None,
            keeper_context.as_ref(),
            outfield_context.as_ref(),
        );
        assert_eq!(
            frame.players.pose_id[zero_based], expected.id,
            "pose id for slot {index}"
        );
        assert_eq!(frame.players.pose_priority[zero_based], expected.priority);
        assert_eq!(frame.players.pose_source[zero_based], expected.source);
    }
}

#[test]
fn reports_displayed_positions_while_pose_inputs_stay_authoritative() {
    let mut state = fixture(17.0);
    state.players[0].pos = Vec2::new(24.0, 270.0);
    state.players[0].run_vel = Vec2::new(0.0, 0.0);
    // Authoritatively inside smother range, displayed a long way away.
    state.ball = Vec2::new(state.players[0].pos.x + 8.0, state.players[0].pos.y);
    let displaced = Vec2::new(
        state.players[0].pos.x + 300.0,
        state.players[0].pos.y + 120.0,
    );
    let goalkeeper_id = state.players[0].id.clone();
    let pose = RenderPose {
        players: vec![(goalkeeper_id, displaced)],
        ball: state.ball,
    };

    let frame = render_frame::build(
        &state,
        &RenderFrameOptions {
            render_pose: Some(pose),
            ..Default::default()
        },
    );
    assert_eq!(
        frame.players.x[0], displaced.x,
        "the payload reports where the avatar is drawn"
    );
    assert_eq!(frame.players.y[0], displaced.y);
    assert_eq!(
        frame.players.pose_id[0],
        PlayerPoseId::KeeperReadyLow,
        "smother range must be measured on the authoritative position"
    );
}

#[test]
fn takes_the_release_follow_through_window_from_the_renderer() {
    let mut state = fixture(17.0);
    state.players[3].is_keeper = false;
    let striker_id = state.players[3].id.clone();

    let without = render_frame::build(&state, &RenderFrameOptions::default());
    assert_ne!(without.players.pose_id[3], PlayerPoseId::KickFollow);

    let with = render_frame::build(
        &state,
        &RenderFrameOptions {
            kick_follow: Some(vec![striker_id]),
            ..Default::default()
        },
    );
    assert_eq!(with.players.pose_id[3], PlayerPoseId::KickFollow);
}

#[test]
fn hides_the_ground_ball_only_while_a_keeper_holds_it_in_the_hands() {
    let mut state = fixture(17.0);
    state.owner = Some(1);
    state.players[0].feet_ball = false;

    let held = render_frame::build(&state, &RenderFrameOptions::default());
    assert!(!held.ball.visible);
    assert!(held.possession.keeper_holds);
    assert!(held.players.holding[0]);
    assert_eq!(held.possession.owner, Some(1));
    assert_eq!(held.possession.owner_team, Some(state.players[0].team));

    // A back-pass at the keeper's feet is a dribbled ground ball like any other.
    state.players[0].feet_ball = true;
    let at_feet = render_frame::build(&state, &RenderFrameOptions::default());
    assert!(at_feet.ball.visible);
    assert!(!at_feet.possession.keeper_holds);
    assert!(!at_feet.players.holding[0]);
}

#[test]
fn solves_the_landing_point_only_for_a_lofted_loose_ball() {
    let mut state = fixture(17.0);
    state.owner = None;
    state.ball = Vec2::new(400.0, 270.0);
    state.ball_vel = Vec2::new(60.0, 0.0);
    state.ball_z = 90.0;
    state.ball_vz = 0.0;

    let lofted = render_frame::build(&state, &RenderFrameOptions::default());
    assert!(
        lofted.ball.landing_x.is_some(),
        "an airborne loose ball projects a landing point"
    );
    assert!(
        lofted.ball.landing_x.unwrap() > state.ball.x,
        "it lands ahead of the ball"
    );
    assert_eq!(lofted.ball.z, 90.0);

    state.ball_z = 0.0;
    assert_eq!(
        render_frame::build(&state, &RenderFrameOptions::default())
            .ball
            .landing_x,
        None,
        "a grounded pass has no reticle"
    );

    state.ball_z = 90.0;
    state.owner = Some(2);
    assert_eq!(
        render_frame::build(&state, &RenderFrameOptions::default())
            .ball
            .landing_x,
        None,
        "a carried ball has no reticle"
    );
}

#[test]
fn reports_one_charge_at_a_time_for_the_controlled_player() {
    let mut state = fixture(17.0);
    let controlled_index = (state.controlled - 1) as usize;

    assert_eq!(
        render_frame::build(&state, &RenderFrameOptions::default())
            .control
            .charge_kind,
        None
    );

    state.players[controlled_index].pass_charge = 0.5;
    let passing = render_frame::build(&state, &RenderFrameOptions::default());
    assert_eq!(passing.control.charge_kind, Some(RenderChargeKind::Pass));
    assert_eq!(passing.control.charge, 0.5);

    // A shot charge outranks a pass charge, exactly as the meter draws it.
    state.players[controlled_index].charge = 0.8;
    let shooting = render_frame::build(&state, &RenderFrameOptions::default());
    assert_eq!(shooting.control.charge_kind, Some(RenderChargeKind::Shot));
    assert_eq!(shooting.control.charge, 0.8);

    state.players[controlled_index].pass_target = Some(3);
    assert_eq!(
        render_frame::build(&state, &RenderFrameOptions::default())
            .control
            .pass_target,
        Some(3)
    );
}

#[test]
fn flattens_the_frames_event_batch_into_the_effect_trigger_channel() {
    let state = fixture(17.0);
    let striker_id = state.players[3].id.clone();
    let events = vec![
        MatchEvent {
            player: Some(striker_id.clone()),
            on_target: Some(true),
            ..bare_event(MatchEventKind::Shot, 100.0, 200.0)
        },
        MatchEvent {
            outcome: Some(AerialOutcome::Clean),
            ..bare_event(MatchEventKind::Reception, 300.0, 250.0)
        },
    ];

    let frame = render_frame::build(
        &state,
        &RenderFrameOptions {
            events: Some(events),
            ..Default::default()
        },
    );
    assert_eq!(frame.events.count, 2);
    assert_eq!(frame.events.kind[0], MatchEventKind::Shot);
    assert_eq!(frame.events.x[0], 100.0);
    assert_eq!(frame.events.y[0], 200.0);
    assert_eq!(frame.events.player[0], Some(striker_id));
    assert_eq!(
        frame.events.slot[0],
        Some(4),
        "the payload resolves an event's player to a roster slot"
    );
    assert_eq!(frame.events.on_target[0], 2, "a reported true encodes as 2");
    assert_eq!(frame.events.kind[1], MatchEventKind::Reception);
    assert_eq!(frame.events.outcome[1], Some(AerialOutcome::Clean));
    assert_eq!(frame.events.player[1], None);
    assert_eq!(frame.events.slot[1], None);
}

#[test]
fn keeps_absent_false_and_true_distinguishable_for_optional_booleans() {
    let state = fixture(17.0);
    // All three states occur for real: `sim/match.lua` sets `on_target`
    // explicitly on a released shot, and a keeper's distribution kick is
    // also `kind == "shot"` but reports nothing. Kind alone cannot tell
    // them apart, so the payload must.
    let events = vec![
        MatchEvent {
            on_target: Some(true),
            ..bare_event(MatchEventKind::Shot, 0.0, 0.0)
        },
        MatchEvent {
            on_target: Some(false),
            ..bare_event(MatchEventKind::Shot, 0.0, 0.0)
        },
        bare_event(MatchEventKind::Shot, 0.0, 0.0),
        MatchEvent {
            jumping: Some(false),
            ..bare_event(MatchEventKind::Header, 0.0, 0.0)
        },
        bare_event(MatchEventKind::Header, 0.0, 0.0),
    ];

    let flat = render_frame::build(
        &state,
        &RenderFrameOptions {
            events: Some(events),
            ..Default::default()
        },
    )
    .events;
    assert_eq!(flat.on_target[0], 2, "true");
    assert_eq!(
        flat.on_target[1], 1,
        "reported false is NOT the same as absent"
    );
    assert_eq!(flat.on_target[2], 0, "absent");
    assert_eq!(flat.jumping[3], 1);
    assert_eq!(flat.jumping[4], 0);
}

#[test]
fn resolves_a_tip_event_into_the_drawn_dive_direction() {
    let mut state = fixture(17.0);
    state.players[0].pos = Vec2::new(24.0, 270.0);
    let goalkeeper_id = state.players[0].id.clone();
    let (gx, gy) = (state.players[0].pos.x, state.players[0].pos.y);

    let up = render_frame::build(
        &state,
        &RenderFrameOptions {
            events: Some(vec![MatchEvent {
                player: Some(goalkeeper_id.clone()),
                ..bare_event(MatchEventKind::Tip, gx, gy - 40.0)
            }]),
            ..Default::default()
        },
    );
    assert_eq!(up.players.pose_id[0], PlayerPoseId::KeeperTip);
    assert_eq!(up.players.dive_dir_x[0], 0.0);
    assert_eq!(up.players.dive_dir_y[0], -1.0);

    let down = render_frame::build(
        &state,
        &RenderFrameOptions {
            events: Some(vec![MatchEvent {
                player: Some(goalkeeper_id),
                ..bare_event(MatchEventKind::Tip, gx, gy + 40.0)
            }]),
            ..Default::default()
        },
    );
    assert_eq!(down.players.dive_dir_y[0], 1.0);
}

#[test]
fn derives_the_scoreboard_section_from_the_simulation() {
    let mut state = fixture(17.0);
    state.score.home = 2;
    state.score.away = 1;
    state.time_left = 65.4;
    state.owner = Some(state.controlled);
    let controlled_index = (state.controlled - 1) as usize;
    state.players[controlled_index].sprint_meter = 1.6;

    let scoreboard = render_frame::hud(&state, None);
    assert_eq!(scoreboard.home_score, 2);
    assert_eq!(scoreboard.away_score, 1);
    assert_eq!(scoreboard.time_left, 65.4);
    assert_eq!(
        scoreboard.possession_team,
        Some(state.players[controlled_index].team)
    );
    assert!(scoreboard.controlled_owns_ball);
    assert_eq!(scoreboard.controlled_id, state.players[controlled_index].id);
    assert_eq!(
        scoreboard.controlled_stamina, 1.0,
        "stamina is clamped for the meter"
    );

    state.owner = None;
    assert_eq!(render_frame::hud(&state, None).possession_team, None);

    // `build` must publish exactly the same section it exposes on its own.
    let frame = render_frame::build(&state, &RenderFrameOptions::default());
    assert_eq!(frame.hud.home_score, 2);
    assert_eq!(frame.hud.controlled_id, scoreboard.controlled_id);
}

#[test]
fn normalises_pose_timers_so_no_renderer_re_derives_a_duration() {
    let mut state = fixture(17.0);
    state.players[2].is_keeper = false;
    state.players[2].dive_timer = 0.9; // far past the ease window
    state.players[2].grab_timer = 0.125;
    state.players[2].throw_timer = 0.25;
    state.players[2].aerial_timer = 0.11;
    state.players[2].aerial_style = Some(AerialStyle::ChestControl);

    let players = render_frame::build(&state, &RenderFrameOptions::default()).players;
    assert_eq!(players.dive[2], 1.0, "an over-long timer clamps to 1");
    assert!((players.grab[2] - 0.5).abs() < 1e-9);
    assert_eq!(players.throw[2], 1.0);
    assert!((players.aerial[2] - 0.11 / 0.18).abs() < 1e-9);

    state.players[2].dive_timer = 0.0;
    state.players[2].grab_timer = 0.0;
    assert_eq!(
        render_frame::build(&state, &RenderFrameOptions::default())
            .players
            .dive[2],
        0.0
    );
}
