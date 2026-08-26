//! Tests for `gc_sim::tunable_registry` — the three tiers, their separate
//! serialization, band-set atomicity, and the config hash a handshake carries.

use gc_data::tunables::{BandEdge, BandSet, Tier, TunableDef};
use gc_sim::tunable_registry::{self, RegistryBuilder};
use gc_sim::tuning::{self, Tuning};

static FIXTURE_A: &[TunableDef] = &[TunableDef {
    id: "fixture.alpha",
    tier: Tier::Sim,
    label: "Alpha",
    cat: "Fixture",
    default: 1.0,
    unit: "px",
    min: 0.0,
    max: 10.0,
    step: 1.0,
    desc: "fixture",
}];

static FIXTURE_A_AGAIN: &[TunableDef] = &[TunableDef {
    id: "fixture.alpha",
    tier: Tier::Sim,
    label: "Alpha (again)",
    cat: "Fixture",
    default: 2.0,
    unit: "px",
    min: 0.0,
    max: 10.0,
    step: 1.0,
    desc: "fixture",
}];

// A tier-2 entry registered from a test, so the isolation assertions below
// have something to fail on. The SHIPPED tier-2 set lives in `gc-render`,
// which this crate cannot name — that unreachability is the point, and
// `crates/gc-render/tests/presentation_tunables.rs` is where the shipped set
// is exercised.
static FIXTURE_PRESENTATION: &[TunableDef] = &[TunableDef {
    id: "fixture.smoothing",
    tier: Tier::Presentation,
    label: "Smoothing",
    cat: "Fixture",
    default: 0.5,
    unit: "s",
    min: 0.0,
    max: 1.0,
    step: 0.1,
    desc: "fixture",
}];

#[test]
fn registry_registers_declaratively_and_keeps_registration_order() {
    let mut b = RegistryBuilder::new();
    b.register_tunables("fixture", FIXTURE_A);
    b.register_tunables("fixture_presentation", FIXTURE_PRESENTATION);
    let reg = b.build();
    let ids: Vec<&str> = reg.defs().iter().map(|d| d.id).collect();
    assert_eq!(ids, vec!["fixture.alpha", "fixture.smoothing"]);
    assert_eq!(reg.value("fixture.alpha"), 1.0);
}

#[test]
fn registry_panics_on_a_duplicate_id() {
    let result = std::panic::catch_unwind(|| {
        let mut b = RegistryBuilder::new();
        b.register_tunables("first", FIXTURE_A);
        b.register_tunables("second", FIXTURE_A_AGAIN);
        b.build()
    });
    let err = result.expect_err("a duplicate id must be a startup assertion failure");
    let message = err
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_else(|| (*err.downcast_ref::<&str>().unwrap_or(&"")).to_string());
    assert!(
        message.contains("duplicate tunable id") && message.contains("fixture.alpha"),
        "unhelpful duplicate-id panic: {message}"
    );
}

#[test]
fn registry_sweepable_ids_are_tier_one_only_and_sorted() {
    let mut b = RegistryBuilder::new();
    b.register_tunables("fixture_presentation", FIXTURE_PRESENTATION);
    b.register_tunables("fixture", FIXTURE_A);
    let reg = b.build();
    assert_eq!(reg.sweepable_ids(), vec!["fixture.alpha"]);

    let shipped = tunable_registry::shipped();
    let ids = shipped.sweepable_ids();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "sweep enumeration must be id-sorted");
    assert!(ids.contains(&"AI_SHOOT_RANGE"));
    assert_eq!(
        ids.len(),
        shipped.tier(Tier::Sim).len(),
        "every tier-1 entry is sweepable and nothing else is"
    );
    assert!(
        shipped.tier(Tier::Presentation).is_empty(),
        "gc-sim must not be able to register or reach a tier-2 value"
    );
}

#[test]
fn registry_serializes_each_tier_separately_and_refuses_a_foreign_tier_blob() {
    let mut b = RegistryBuilder::new();
    b.register_tunables("fixture", FIXTURE_A);
    b.register_tunables("fixture_presentation", FIXTURE_PRESENTATION);
    let mut reg = b.build();
    reg.set("fixture.alpha", 4.0);
    reg.set("fixture.smoothing", 0.9);

    let sim_blob = reg.serialize_tier(Tier::Sim);
    let presentation_blob = reg.serialize_tier(Tier::Presentation);
    assert_eq!(sim_blob, "#tier=sim\nfixture.alpha=4");
    assert_eq!(
        presentation_blob,
        "#tier=presentation\nfixture.smoothing=0.9"
    );
    assert!(
        !sim_blob.contains("smoothing") && !presentation_blob.contains("alpha"),
        "a tier's blob must carry only that tier"
    );

    let mut fresh = {
        let mut b = RegistryBuilder::new();
        b.register_tunables("fixture", FIXTURE_A);
        b.register_tunables("fixture_presentation", FIXTURE_PRESENTATION);
        b.build()
    };
    fresh.deserialize_tier(Tier::Sim, &sim_blob).unwrap();
    assert_eq!(fresh.value("fixture.alpha"), 4.0);
    assert_eq!(
        fresh.value("fixture.smoothing"),
        0.5,
        "a sim blob must not touch the presentation tier"
    );

    let err = fresh
        .deserialize_tier(Tier::Sim, &presentation_blob)
        .expect_err("a presentation blob applied to the sim tier must fail");
    assert!(err.contains("tier"), "unhelpful cross-tier error: {err}");
    let err = fresh
        .deserialize_tier(Tier::Sim, "fixture.alpha=9")
        .expect_err("an untagged blob must fail");
    assert!(err.contains("#tier="), "unhelpful untagged error: {err}");
}

#[test]
fn registry_refuses_a_single_band_edge_override_and_accepts_a_whole_versioned_set() {
    let mut reg = tunable_registry::assemble();
    let before = reg.band_edge("keeper_save_style", "spread_distance");

    let err = reg
        .override_band_edge("keeper_save_style", "spread_distance", 40.0)
        .expect_err("a single tier-3 edge must not be overridable");
    assert!(
        err.contains("one unit") && err.contains("substitute_band_set"),
        "the refusal must name the supported route: {err}"
    );
    assert_eq!(
        reg.band_edge("keeper_save_style", "spread_distance"),
        before,
        "a refused override must not have written anything"
    );

    static TIGHT_EDGES: &[BandEdge] = &[
        BandEdge {
            id: "smother_distance",
            value: 18.0,
            desc: "fixture",
        },
        BandEdge {
            id: "spread_distance",
            value: 55.0,
            desc: "fixture",
        },
        BandEdge {
            id: "central_reach_fraction",
            value: 0.3,
            desc: "fixture",
        },
    ];
    reg.substitute_band_set(BandSet {
        id: "keeper_save_style",
        version: "test_tight",
        desc: "fixture",
        edges: TIGHT_EDGES,
    })
    .expect("a whole versioned set substitutes");
    assert_eq!(reg.band_edge("keeper_save_style", "spread_distance"), 55.0);
    assert_eq!(reg.band_edge("keeper_save_style", "smother_distance"), 18.0);
}

#[test]
fn registry_rejects_a_band_set_substitution_that_changes_shape_or_keeps_its_version() {
    let mut reg = tunable_registry::assemble();
    static PARTIAL: &[BandEdge] = &[BandEdge {
        id: "spread_distance",
        value: 55.0,
        desc: "fixture",
    }];
    let err = reg
        .substitute_band_set(BandSet {
            id: "keeper_save_style",
            version: "test_partial",
            desc: "fixture",
            edges: PARTIAL,
        })
        .expect_err("a partial set is not a version of the whole set");
    assert!(err.contains("same edges"), "unhelpful shape error: {err}");

    let current = *reg.band_set("keeper_save_style").unwrap();
    let err = reg
        .substitute_band_set(current)
        .expect_err("a substitution must change the version");
    assert!(err.contains("version"), "unhelpful version error: {err}");

    static UNKNOWN: &[BandEdge] = &[];
    let err = reg
        .substitute_band_set(BandSet {
            id: "not_a_band_set",
            version: "v1",
            desc: "fixture",
            edges: UNKNOWN,
        })
        .expect_err("an unregistered set id must fail");
    assert!(err.contains("unknown band set"), "unhelpful error: {err}");
}

#[test]
fn registry_config_hash_moves_with_a_tier_one_value_and_holds_when_nothing_differs() {
    let a = tunable_registry::assemble();
    let b = tunable_registry::assemble();
    assert_eq!(
        a.config_hash(),
        b.config_hash(),
        "identical tier-1 and tier-3 values must produce identical hashes"
    );

    let mut c = tunable_registry::assemble();
    c.set("AI_SHOOT_RANGE", a.value("AI_SHOOT_RANGE") + 10.0);
    assert_ne!(
        a.config_hash(),
        c.config_hash(),
        "one differing tier-1 value must produce a different hash"
    );

    // The smallest representable difference, not a comfortable one: the hash
    // must be injective over f64, not merely over values a designer would
    // type.
    let mut d = tunable_registry::assemble();
    let nudged = f64::from_bits(a.value("AI_SHOOT_RANGE").to_bits() + 1);
    d.set("AI_SHOOT_RANGE", nudged);
    assert_ne!(
        a.config_hash(),
        d.config_hash(),
        "one ULP must move the hash"
    );
}

#[test]
fn registry_config_hash_moves_with_a_band_set_and_never_with_a_presentation_value() {
    let base = tunable_registry::assemble();

    let mut swapped = tunable_registry::assemble();
    static TIGHT_EDGES: &[BandEdge] = &[
        BandEdge {
            id: "defender_handoff_distance",
            value: 90.0,
            desc: "fixture",
        },
        BandEdge {
            id: "advance_threat_distance",
            value: 170.0,
            desc: "fixture",
        },
    ];
    swapped
        .substitute_band_set(BandSet {
            id: "keeper_engagement",
            version: "test_tight",
            desc: "fixture",
            edges: TIGHT_EDGES,
        })
        .unwrap();
    assert_ne!(
        base.config_hash(),
        swapped.config_hash(),
        "a substituted tier-3 band set must move the config hash"
    );

    let mut with_presentation = RegistryBuilder::new();
    with_presentation.register_tunables("gc-sim", gc_data::tunables::SIM_TUNABLES);
    for set in gc_data::tunables::BAND_SETS {
        with_presentation.register_band_set("gc-sim", *set);
    }
    with_presentation.register_tunables("fixture_presentation", FIXTURE_PRESENTATION);
    let mut reg = with_presentation.build();
    let before = reg.config_hash();
    assert_eq!(
        before,
        base.config_hash(),
        "adding a tier-2 entry must not move the config hash"
    );
    reg.set("fixture.smoothing", 0.9);
    assert_eq!(
        reg.config_hash(),
        before,
        "changing a tier-2 value must not move the config hash"
    );
}

#[test]
fn tuning_knobs_are_the_registrys_tier_one_set_and_the_panel_contract_is_unchanged() {
    let shipped = tunable_registry::shipped();
    let knob_keys: Vec<&str> = tuning::KNOBS.iter().map(|k| k.key).collect();
    let tier_ids: Vec<&str> = shipped.tier(Tier::Sim).iter().map(|d| d.id).collect();
    assert_eq!(
        knob_keys, tier_ids,
        "the panel's knob list IS the registry's tier-1 set, in registration order"
    );

    // The `TuningSource` methods `packages/ui/src/tuning_panel.ts` calls, and
    // the categories its tabs are built from.
    let t = Tuning::new();
    assert_eq!(
        t.categories(),
        vec![
            "Movement",
            // Locomotion is the per-context kinematics tab (#488): the
            // Movement tab keeps the shared bases every context multiplies.
            "Locomotion",
            "Dribble",
            "Aerial",
            "Attacking",
            // Passing is its own tab (#491): eleven knobs describing the
            // soft cone, the distance-to-speed curve and the lead solver.
            "Passing",
            "Defending",
            "Keeper",
            "AI",
            "Replay"
        ]
    );
    assert!(t.is_default("AI_SHOOT_RANGE"));
    assert_eq!(t.serialize(), "", "a fresh registry serializes to nothing");
    assert!(
        t.in_category("Attacking")
            .iter()
            .any(|k| k.key == "PASS_RANGE_MIN"),
        "the spot-check knob shows up in the panel like any other"
    );
}

#[test]
fn tuning_still_round_trips_the_untagged_panel_blob_byte_for_byte() {
    let mut t = Tuning::new();
    t.set("AI_SHOOT_RANGE", 300.0);
    t.set("JOCKEY_SLOW", 0.6);
    let blob = t.serialize();
    assert_eq!(blob, "JOCKEY_SLOW=0.6\nAI_SHOOT_RANGE=300");
    let mut back = Tuning::new();
    back.deserialize(&blob);
    assert_eq!(back, t);
    assert_eq!(back.serialize(), blob);
}

#[test]
fn pass_range_min_is_registry_backed_and_reaches_the_simulation() {
    // The spot-check (#487): `PASS_RANGE_MIN` was `const PASS_RANGE_MIN: f64 =
    // 110.0` in `match.rs`, invisible to the sweep and to the config hash.
    // That value was itself rescaled 110 -> 190 for the 960x540 -> 1648x927
    // futsal pitch re-dimensioning (k = 1.7166667; see
    // `gc-data/src/tunables.rs`), so the golden value asserted here tracks
    // gc-data's current shipped default, not the pre-rescale literal above.
    let shipped = tunable_registry::shipped();
    let def = shipped
        .def("PASS_RANGE_MIN")
        .expect("PASS_RANGE_MIN is a registered tunable");
    assert_eq!(
        def.default, 190.0,
        "the registered default IS the shipped gc-data constant"
    );
    assert_eq!(def.tier, Tier::Sim);
    assert!(shipped.sweepable_ids().contains(&"PASS_RANGE_MIN"));

    // And a sweep can actually move it: two headless batches on the same seeds
    // whose only difference is this value do not produce the same match.
    let short = Some(20.0);
    let seeds = [11.0, 12.0, 13.0, 14.0];
    let base = gc_sim::headless::run_batch(&gc_sim::headless::BatchOpts {
        seeds: Some(&seeds),
        duration: short,
        ..Default::default()
    });
    let moved = gc_sim::headless::run_batch(&gc_sim::headless::BatchOpts {
        seeds: Some(&seeds),
        tuning_blob: Some("PASS_RANGE_MIN=260"),
        duration: short,
        ..Default::default()
    });
    let base_passes: Vec<i64> = base.matches.iter().map(|m| m.metrics.passes).collect();
    let moved_passes: Vec<i64> = moved.matches.iter().map(|m| m.metrics.passes).collect();
    assert_ne!(
        base_passes, moved_passes,
        "a registry-backed PASS_RANGE_MIN must change what the simulation does"
    );
}
