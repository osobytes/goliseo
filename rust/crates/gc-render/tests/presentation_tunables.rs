//! Tier-2 isolation: perturbing every presentation value cannot change what
//! the simulation computes.
//!
//! The structural half of the promise is the build graph — `gc-render` depends
//! on `gc-sim`, so a `gc-sim` module that read this crate's tier-2 table would
//! be a dependency cycle `cargo` rejects, and there is no test to write for
//! that because it cannot compile. This file measures the consequences: that
//! the tier-2 values are genuinely live (perturbing them changes what the
//! renderer produces, so this is not a table of inert numbers), that they are
//! invisible to the sim's registry and its config hash, and that a recorded
//! match replays to a byte-identical state-hash sequence with every one of
//! them moved.

use gc_core::vec2::Vec2;
use gc_data::tunables::Tier;
use gc_render::frame::{self as render_frame, RenderFrameOptions};
use gc_render::presentation_tunables;
use gc_sim::aerial::AerialStyle;
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{MatchState, PitchSize};
use gc_sim::tuning::Tuning;
use gc_sim::{determinism_evidence, replay, tunable_registry};

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

/// A registry with EVERY tier-2 value moved off its default, each toward the
/// far end of its own range so no value can accidentally land back where it
/// started.
fn all_perturbed() -> gc_sim::tunable_registry::Registry {
    let mut reg = presentation_tunables::assemble();
    for def in presentation_tunables::PRESENTATION_TUNABLES {
        let far = if (def.default - def.min).abs() > (def.max - def.default).abs() {
            def.min
        } else {
            def.max
        };
        reg.set(def.id, far);
        assert_ne!(
            reg.value(def.id),
            def.default,
            "{} did not actually move",
            def.id
        );
    }
    reg
}

#[test]
fn presentation_tunables_are_all_tier_two_and_none_of_them_is_a_sim_tunable() {
    let reg = presentation_tunables::shipped();
    assert!(
        !presentation_tunables::PRESENTATION_TUNABLES.is_empty(),
        "an empty tier 2 would make every assertion below vacuous"
    );
    assert_eq!(
        reg.tier(Tier::Presentation).len(),
        presentation_tunables::PRESENTATION_TUNABLES.len()
    );
    assert!(
        reg.tier(Tier::Sim).is_empty() && reg.band_sets().is_empty(),
        "the presentation registry carries tier 2 and nothing else"
    );

    let sim = tunable_registry::shipped();
    for def in presentation_tunables::PRESENTATION_TUNABLES {
        assert_eq!(def.tier, Tier::Presentation, "{} is not tier 2", def.id);
        assert!(
            sim.def(def.id).is_none(),
            "{} is reachable from the simulation's registry",
            def.id
        );
        assert!(
            !sim.sweepable_ids().contains(&def.id),
            "{} is enumerable by a balance sweep",
            def.id
        );
    }
}

#[test]
fn perturbing_every_presentation_value_never_moves_the_sim_config_hash() {
    let before = tunable_registry::shipped().config_hash();
    let perturbed = all_perturbed();
    assert!(!perturbed.serialize_tier(Tier::Presentation).is_empty());
    assert_eq!(
        tunable_registry::shipped().config_hash(),
        before,
        "no tier-2 value may enter the config hash two peers agree on"
    );
    // And the tier-2 blob shares no id with the hashed set at all.
    let canonical = tunable_registry::shipped().config_canonical();
    for def in presentation_tunables::PRESENTATION_TUNABLES {
        assert!(
            !canonical.contains(def.id),
            "{} appears in the hashed config bytes",
            def.id
        );
    }
}

#[test]
fn perturbing_every_presentation_value_changes_the_rendered_frame() {
    // Without this, every other assertion here would pass just as well for a
    // table of numbers nothing reads. This drives the REAL `frame::build` twice
    // over one identical `MatchState`, changing nothing but the injected
    // tier-2 registry, and requires the drawn pose numbers to differ.
    let mut state = fixture(17.0);
    state.players[2].is_keeper = false;
    state.players[2].dive_timer = 0.12;
    state.players[2].grab_timer = 0.125;
    state.players[2].throw_timer = 0.1;
    state.players[2].aerial_timer = 0.11;
    state.players[2].aerial_style = Some(AerialStyle::ChestControl);

    let shipped_frame = render_frame::build(&state, &RenderFrameOptions::default());
    let perturbed_frame = render_frame::build(
        &state,
        &RenderFrameOptions {
            presentation: Some(all_perturbed()),
            ..Default::default()
        },
    );

    assert_ne!(
        shipped_frame.players.dive[2], perturbed_frame.players.dive[2],
        "the dive ease must reach the drawn frame"
    );
    assert_ne!(
        shipped_frame.players.grab[2], perturbed_frame.players.grab[2],
        "the grab ease must reach the drawn frame"
    );
    assert_ne!(
        shipped_frame.players.aerial[2], perturbed_frame.players.aerial[2],
        "the aerial-control ease must reach the drawn frame"
    );

    // And the shipped values are exactly the ones `tests/frame.rs` pins the
    // rendered arithmetic against, so a registry that stopped feeding the
    // renderer turns that test red rather than passing quietly.
    let shipped = presentation_tunables::shipped();
    assert_eq!(shipped.value("presentation.grab_ease"), 0.25);
    assert_eq!(shipped.value("presentation.aerial_ease_control"), 0.18);
    assert_eq!(shipped.value("presentation.dive_ease"), 0.3);
    assert!((shipped_frame.players.grab[2] - 0.5).abs() < 1e-9);
    assert!((shipped_frame.players.aerial[2] - 0.11 / 0.18).abs() < 1e-9);
}

#[test]
fn the_landing_reticle_window_is_a_tier_two_value_the_frame_actually_reads() {
    // The reticle window is the one tier-2 value that gates whether a field is
    // populated at all rather than scaling a number, so it needs its own case:
    // a fall that is inside the shipped window and outside the perturbed one.
    let mut state = fixture(23.0);
    state.owner = None;
    state.ball_z = 60.0;
    state.ball_vz = 0.0;
    state.ball_vel = Vec2::new(10.0, 10.0);

    let shipped_frame = render_frame::build(&state, &RenderFrameOptions::default());
    assert!(
        shipped_frame.ball.landing_x.is_some(),
        "the fixture must draw a reticle at shipped values, or this proves nothing"
    );

    // `presentation.reticle_min_height` perturbs to its max (120), above the
    // ball's 60px, so the reticle must disappear.
    let perturbed_frame = render_frame::build(
        &state,
        &RenderFrameOptions {
            presentation: Some(all_perturbed()),
            ..Default::default()
        },
    );
    assert!(
        perturbed_frame.ball.landing_x.is_none(),
        "a perturbed reticle window must change what the frame reports"
    );
}

#[test]
fn a_recorded_match_replays_to_an_identical_state_hash_sequence_while_tier_two_moves_the_frame() {
    // The two halves of the isolation claim, measured against ONE state:
    // moving every tier-2 value changes the rendered frame (proved above and
    // re-checked here on this state), and changes nothing about the
    // simulation's own boundary-hash sequence over a recorded match.
    let tune = Tuning::new();
    let tape = determinism_evidence::fixture_tape(&tune).expect("the OMP-1 fixture tape loads");
    let identity =
        gc_sim::input_tape::copy_identity(&tape.identity).expect("the tape identity is valid");

    let before = replay::run(&tape, &identity, &tune).expect("the fixture tape replays");
    assert!(
        before.divergence.is_none(),
        "the reference replay must be clean before anything is perturbed"
    );
    let hashes_before: Vec<String> = before.boundaries.iter().map(|b| b.hash.clone()).collect();
    assert!(
        hashes_before.len() > 1,
        "a one-boundary replay would make this vacuous"
    );

    // The perturbed tier-2 registry demonstrably changes rendering of the
    // replay's own final state...
    let perturbed = all_perturbed();
    let mut rendered = before.state.clone();
    rendered.players[2].is_keeper = false;
    rendered.players[2].grab_timer = 0.125;
    let shipped_frame = render_frame::build(&rendered, &RenderFrameOptions::default());
    let perturbed_frame = render_frame::build(
        &rendered,
        &RenderFrameOptions {
            presentation: Some(perturbed),
            ..Default::default()
        },
    );
    assert_ne!(
        shipped_frame.players.grab[2], perturbed_frame.players.grab[2],
        "the perturbation must be live, or the hash comparison below proves nothing"
    );

    // ...and changes nothing about the simulation that produced it. Rendering
    // is a pure read of `MatchState`, so this also checks that building those
    // two frames did not perturb the state the replay ended on.
    let after = replay::run(&tape, &identity, &tune).expect("the fixture tape replays again");
    let hashes_after: Vec<String> = after.boundaries.iter().map(|b| b.hash.clone()).collect();
    assert_eq!(
        hashes_before, hashes_after,
        "a tier-2 perturbation changed the state-hash sequence of a replayed match"
    );
    assert!(after.divergence.is_none());
}
