//! Port of `spec/sim/determinism_evidence_spec.lua`.
//!
//! Two structural notes:
//!
//! - `gc_data::omp1_determinism::InputTapeIdentity`/`InputOwnership` are
//!   JSON-shaped, not `gc_sim::input_tape::InputTapeIdentity`-shaped (see
//!   `determinism_evidence.rs`'s module doc's "Where the fixture data comes
//!   from" section). `input_tape::copy_identity(fixture.identity)` in the
//!   Lua spec becomes `input_tape::copy_identity(&fixture_tape_identity())`
//!   here, where `fixture_tape_identity` reproduces the module's own
//!   private adapter for test purposes.
//! - Two `pcall`-guarded negative assertions in "validates a current-input
//!   migration while changing only snapshot version" — adding an
//!   `unexpected` field to the identity table, and setting a roster id to
//!   `false` — check that Lua's duck-typed `assert_fields`/type checks
//!   reject a malformed table. `InputTapeIdentity` is a proper Rust struct;
//!   neither malformation is expressible (there is no field to add, and
//!   `ownership.rosters.home` is `Vec<String>`, so a boolean cannot occupy
//!   a slot). The compiler enforces both invariants instead of a runtime
//!   check, which is strictly stronger, not a coverage gap — this mirrors
//!   `input_frame.rs`'s documented precedent for dropping `assert_fields`.

use gc_data::omp1_determinism;
use gc_sim::input_frame;
use gc_sim::input_tape::{self, InputTapeIdentity};
use gc_sim::match_snapshot;
use gc_sim::placement::{self, DistanceCandidate};
use gc_sim::tuning::Tuning;
use gc_sim::{determinism_evidence, replay};

/// Reproduces `determinism_evidence.rs`'s own private `fixture_identity`
/// adapter (JSON-shaped -> `input_tape::InputTapeIdentity`), since that
/// function is private and the spec needs the same starting value the
/// module itself uses.
fn fixture_tape_identity() -> InputTapeIdentity {
    let source = &omp1_determinism::fixture().identity;
    let mut slots = Vec::with_capacity(8);
    for (index, slot) in source.ownership.slots.iter().enumerate() {
        let canonical = input_frame::slot(index as i64 + 1).expect("canonical slot index");
        slots.push(input_frame::InputSlotAssignment {
            slot: canonical.id,
            team: canonical.team,
            player_id: slot.player_id.clone(),
        });
    }
    let slots: [input_frame::InputSlotAssignment; 8] =
        slots.try_into().unwrap_or_else(|_| panic!("eight slots"));
    InputTapeIdentity {
        tape_version: source.tape_version,
        input_version: source.input_version,
        snapshot_version: source.snapshot_version,
        build: source.build.clone(),
        source: source.source.clone(),
        content: source.content.clone(),
        tuning: source.tuning.clone(),
        config: source.config.clone(),
        fixture: source.fixture.clone(),
        seed: source.seed as f64,
        tick_rate: source.tick_rate,
        ownership: input_frame::InputOwnership {
            version: source.ownership.version,
            rosters: input_frame::InputFixtureRosters {
                home: source.ownership.rosters.home.clone(),
                away: source.ownership.rosters.away.clone(),
            },
            slots,
        },
        combat: None,
    }
}

#[test]
fn determinism_evidence_pins_the_authoritative_fixture_to_the_current_snapshot_schema() {
    let fixture = omp1_determinism::fixture();
    assert_eq!(fixture.identity.snapshot_version, match_snapshot::VERSION);
    assert_eq!(match_snapshot::VERSION, 11);
}

#[test]
fn determinism_evidence_uses_a_total_order_for_equal_distance_match_candidates() {
    let before = placement::distance_candidate_before;
    assert!(before(
        DistanceCandidate { idx: 9, d: 10.0 },
        DistanceCandidate { idx: 2, d: 10.0 }
    ));
    assert!(!before(
        DistanceCandidate { idx: 2, d: 10.0 },
        DistanceCandidate { idx: 9, d: 10.0 }
    ));
    assert!(before(
        DistanceCandidate { idx: 9, d: 9.0 },
        DistanceCandidate { idx: 2, d: 10.0 }
    ));
    assert!(!before(
        DistanceCandidate { idx: 2, d: 10.0 },
        DistanceCandidate { idx: 9, d: 9.0 }
    ));
}

#[test]
fn determinism_evidence_validates_a_current_input_migration_while_changing_only_snapshot_version() {
    let mut prior =
        input_tape::copy_identity(&fixture_tape_identity()).expect("fixture identity is valid");
    prior.snapshot_version = match_snapshot::VERSION - 1;
    let migrated = determinism_evidence::migration_identity(&prior).expect("migration succeeds");

    assert_eq!(migrated.tape_version, prior.tape_version);
    assert_eq!(migrated.input_version, prior.input_version);
    assert_eq!(migrated.build, prior.build);
    assert_eq!(migrated.source, prior.source);
    assert_eq!(migrated.content, prior.content);
    assert_eq!(migrated.tuning, prior.tuning);
    assert_eq!(migrated.config, prior.config);
    assert_eq!(migrated.fixture, prior.fixture);
    assert_eq!(migrated.seed, prior.seed);
    assert_eq!(migrated.tick_rate, prior.tick_rate);
    assert_eq!(migrated.combat, prior.combat);
    assert_eq!(migrated.snapshot_version, match_snapshot::VERSION);

    // Rust value semantics make the "different table, independent copy"
    // assertion trivially true by construction (no aliasing exists to
    // begin with) — ported anyway for spec fidelity, demonstrating the
    // same observable contract `input_tape::copy_identity` documents.
    let frozen_keeper = migrated.ownership.rosters.home[0].clone();
    prior.ownership.rosters.home[0] = "mutated-after-copy".to_string();
    assert_eq!(migrated.ownership.rosters.home[0], frozen_keeper);
}

#[test]
fn determinism_evidence_explicitly_migrates_only_the_frozen_v1_fixture_identity_to_input_v2() {
    let mut legacy =
        input_tape::copy_identity(&fixture_tape_identity()).expect("fixture identity is valid");
    legacy.fixture = "omp1-nebula-orion-eight-streams-v1".to_string();
    legacy.input_version = 1;
    legacy.ownership.version = 1;

    let migrated = determinism_evidence::migration_identity(&legacy).expect("migration succeeds");
    assert_eq!(migrated.fixture, "omp1-nebula-orion-eight-streams-v2");
    assert_eq!(migrated.input_version, 2);
    assert_eq!(migrated.ownership.version, 2);
    assert_eq!(migrated.build, legacy.build);
    assert_eq!(migrated.source, legacy.source);
    assert_eq!(migrated.content, legacy.content);
    assert_eq!(migrated.config, legacy.config);
    assert_eq!(migrated.tuning, legacy.tuning);
    assert_eq!(migrated.seed, legacy.seed);
    assert_eq!(migrated.tick_rate, legacy.tick_rate);
    assert_eq!(
        migrated.ownership.rosters.home,
        legacy.ownership.rosters.home
    );
    assert_eq!(
        migrated.ownership.rosters.away,
        legacy.ownership.rosters.away
    );
    for (migrated_slot, legacy_slot) in migrated
        .ownership
        .slots
        .iter()
        .zip(legacy.ownership.slots.iter())
    {
        assert_eq!(migrated_slot.slot, legacy_slot.slot);
        assert_eq!(migrated_slot.team, legacy_slot.team);
        assert_eq!(migrated_slot.player_id, legacy_slot.player_id);
    }
}

#[test]
fn determinism_evidence_migrates_only_wires_that_were_canonical_within_the_v1_bounds() {
    let current = input_frame::encode(&input_frame::neutral(0).expect("canonical frame"))
        .expect("canonical frame encodes");
    let legacy = format!("1{}", &current[1..]);
    assert_eq!(
        determinism_evidence::migrate_legacy_fixture_wire(&legacy).expect("legacy wire migrates"),
        current
    );

    let v2_only_held = legacy.replacen("0,0,0,0", "0,0,128,0", 1);
    assert!(determinism_evidence::migrate_legacy_fixture_wire(&v2_only_held).is_err());

    let v2_only_edge = legacy.replacen("0,0,0,0", "0,0,0,64", 1);
    assert!(determinism_evidence::migrate_legacy_fixture_wire(&v2_only_edge).is_err());

    let oversized = format!("{legacy}{}", "0".repeat(149));
    assert!(determinism_evidence::migrate_legacy_fixture_wire(&oversized).is_err());
}

#[test]
fn determinism_evidence_pins_the_full_fixed_input_match_on_the_explicit_evidence_command() {
    let tune = Tuning::new();
    let result = determinism_evidence::verify(&tune).expect("the frozen OMP-1 fixture reproduces");
    assert_eq!(result.ticks, 7201);
    assert_eq!(result.boundaries, 7202);
    assert_eq!(result.score_home, 1);
    assert_eq!(result.score_away, 0);
    assert_eq!(result.outcome, determinism_evidence::Outcome::Home);
    assert!(!result.coverage.goal_kickoff);
    assert!(result.coverage.tackle);
    assert!(result.coverage.aerial);
    assert!(result.coverage.keeper);
    assert!(result.coverage.full_time);

    let report = determinism_evidence::report(&result);
    assert!(report.contains("coverage=tackle,aerial,keeper,full_time"));
    assert!(!report.contains("coverage=goal_kickoff"));
}

#[test]
fn determinism_evidence_publishes_the_frozen_fixture_as_a_validated_materialized_tape() {
    let tune = Tuning::new();
    let fixture = omp1_determinism::fixture();
    let tape = determinism_evidence::fixture_tape(&tune).expect("the fixture materializes");
    assert!(input_tape::validate(&tape, &tune).is_ok());
    assert_eq!(tape.identity.fixture, fixture.fixture_id);
    assert_eq!(tape.frames.len() as i64, fixture.frame_count);
    assert_eq!(tape.boundary_hashes.len() as i64, fixture.boundary_count);
    assert_eq!(
        tape.boundary_hashes.last().expect("at least one boundary"),
        &fixture.expected_final_hash
    );
}

/// Not in the Lua spec, but a natural extension `sim/replay.lua` exists
/// for: the frozen fixture tape replayed through `replay::run` must
/// reproduce every boundary hash with zero divergence, the same guarantee
/// `determinism_evidence::verify` proves through its own path.
#[test]
fn the_frozen_fixture_tape_replays_through_sim_replay_with_no_divergence() {
    let tune = Tuning::new();
    let tape = determinism_evidence::fixture_tape(&tune).expect("the fixture materializes");
    let result = replay::run(&tape, &tape.identity, &tune).expect("replay context is valid");
    assert!(result.divergence.is_none());
    assert_eq!(result.boundaries.len(), tape.boundary_hashes.len());
}
