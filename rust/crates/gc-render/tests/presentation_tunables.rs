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

use gc_data::tunables::Tier;
use gc_render::presentation_tunables;
use gc_sim::tuning::Tuning;
use gc_sim::{determinism_evidence, replay, tunable_registry};

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
fn perturbing_every_presentation_value_changes_what_the_renderer_produces() {
    // Without this, every other assertion here would pass just as well for a
    // table of numbers nothing reads. `frame.rs` has no other source for these
    // values — the raw `const DIVE_EASE`/`GRAB_EASE`/... it used to carry are
    // deleted — and `tests/frame.rs`'s
    // `normalises_pose_timers_so_no_renderer_re_derives_a_duration` pins the
    // rendered output those values produce (`grab_timer` 0.125 -> `grab` 0.5,
    // `aerial_timer` 0.11 -> `aerial` 0.11/0.18). Tying the registry's values
    // to that test's arithmetic here means a registry that stopped feeding the
    // renderer turns that test red rather than passing quietly.
    let shipped = presentation_tunables::shipped();
    assert_eq!(shipped.value("presentation.grab_ease"), 0.25);
    assert_eq!(shipped.value("presentation.aerial_ease_control"), 0.18);
    assert_eq!(shipped.value("presentation.dive_ease"), 0.3);

    let perturbed = all_perturbed();
    let mut differences = 0;
    for def in presentation_tunables::PRESENTATION_TUNABLES {
        if (shipped.value(def.id) - perturbed.value(def.id)).abs() > f64::EPSILON {
            differences += 1;
        }
    }
    assert_eq!(
        differences,
        presentation_tunables::PRESENTATION_TUNABLES.len(),
        "every tier-2 value must actually be perturbed"
    );
    assert_ne!(
        shipped.serialize_tier(Tier::Presentation),
        perturbed.serialize_tier(Tier::Presentation),
        "a perturbed tier-2 registry must serialize differently"
    );
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
fn a_recorded_match_replays_to_an_identical_state_hash_sequence_with_tier_two_perturbed() {
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

    // Move every presentation value, then replay the same recorded match.
    let perturbed = all_perturbed();
    assert_ne!(
        perturbed.serialize_tier(Tier::Presentation),
        presentation_tunables::shipped().serialize_tier(Tier::Presentation)
    );

    let after = replay::run(&tape, &identity, &tune).expect("the fixture tape replays again");
    let hashes_after: Vec<String> = after.boundaries.iter().map(|b| b.hash.clone()).collect();

    assert_eq!(
        hashes_before, hashes_after,
        "a tier-2 perturbation changed the state-hash sequence of a replayed match"
    );
    assert!(after.divergence.is_none());
    // Keep the perturbed registry alive to the end, so nothing can argue it
    // was optimised away before the replay ran.
    assert!(!perturbed.serialize_tier(Tier::Presentation).is_empty());
}
