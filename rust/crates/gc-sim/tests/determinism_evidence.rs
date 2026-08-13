//! Tests for `gc_sim::determinism_evidence`.
//!
//! Two structural notes:
//!
//! - `gc_data::omp1_determinism::InputTapeIdentity`/`InputOwnership` are
//!   JSON-shaped, not `gc_sim::input_tape::InputTapeIdentity`-shaped (see
//!   `determinism_evidence.rs`'s module doc's "Where the fixture data comes
//!   from" section), so `input_tape::copy_identity(&fixture_tape_identity())`
//!   is used here, where `fixture_tape_identity` reproduces the module's own
//!   private adapter for test purposes.
//! - A negative-assertion case would want to check that "validates a
//!   current-input migration while changing only snapshot version" rejects
//!   a malformed identity table — adding an unexpected field, or setting a
//!   roster id to a non-string value. `InputTapeIdentity` is a proper Rust
//!   struct; neither malformation is expressible (there is no field to add,
//!   and `ownership.rosters.home` is `Vec<String>`, so a boolean cannot
//!   occupy a slot). The compiler enforces both invariants instead of a
//!   runtime check, which is strictly stronger, not a coverage gap — this
//!   mirrors `input_frame.rs`'s documented precedent for dropping runtime
//!   field-shape checks that the type system already makes impossible to
//!   violate.

use gc_data::omp1_determinism;
use gc_sim::input_frame;
use gc_sim::input_tape::{self, InputTapeIdentity};
use gc_sim::match_snapshot;
use gc_sim::placement::{self, DistanceCandidate};
use gc_sim::tuning::Tuning;
use gc_sim::{determinism_evidence, replay};

/// Reproduces `determinism_evidence.rs`'s own private `fixture_identity`
/// adapter (JSON-shaped -> `input_tape::InputTapeIdentity`), since that
/// function is private and this file needs the same starting value the
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
    // begin with) — kept anyway, demonstrating the same observable contract
    // `input_tape::copy_identity` documents.
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

/// The campaign's **gated** half: the fixture still replays deterministically
/// to its pinned hashes, and still covers the four headline behaviors it is
/// evidence of.
///
/// What this test deliberately no longer asserts (#505, decision recorded on
/// the issue): the score, the event counts and the per-window event ticks.
/// Those are read off `result` and printed below rather than compared, so a
/// gameplay rework can move them without failing a determinism test that has
/// nothing to do with determinism. The reporting that replaces them is
/// exercised by the three `the_behavioral_drift_report_*` tests, and surfaced
/// to humans by `scripts/check.sh`'s determinism gate and
/// `ts/packages/wasm/src/determinism.spec.ts`.
#[test]
fn determinism_evidence_pins_the_full_fixed_input_match_on_the_explicit_evidence_command() {
    let tune = Tuning::new();
    let result = determinism_evidence::verify(&tune).expect("the frozen OMP-1 fixture reproduces");
    assert_eq!(result.ticks, 7201);
    assert_eq!(result.boundaries, 7202);
    assert!(!result.coverage.goal_kickoff);
    assert!(result.coverage.tackle);
    assert!(result.coverage.aerial);
    assert!(result.coverage.keeper);
    assert!(result.coverage.full_time);

    // As recorded, and as reproduced by this build: 1-0 home; 147 tackles,
    // 180 touches, 2 headers, 1 catch; tackle at tick 24, catch at 1692,
    // header at 1788, full time at 7200. Printed rather than asserted — if a
    // gameplay change moved these, that is the demotion working. Restate the
    // new values here and say why in the PR that moves them.
    let report = determinism_evidence::report(&result);
    println!("{report}");
    println!(
        "drift vs the frozen recording: {}",
        determinism_evidence::render_drift(&result.drift)
    );
    assert!(report.contains("coverage=tackle,aerial,keeper,full_time"));
    assert!(!report.contains("coverage=goal_kickoff"));

    // The reported half must be *present*. It cannot be shown *observed* here:
    // on an unperturbed tree `result.observed` equals the fixture's own frozen
    // claims by construction, so a reporter echoing the fixture back is
    // indistinguishable from one reading the run. That distinction is what
    // `the_report_carries_this_runs_counts_and_score_not_the_frozen_fixtures`
    // below exists for — do not weaken it into this test.
    assert!(report.contains("|drift="));
    assert!(report.contains(&format!(
        "|score={}-{}|",
        result.observed.score_home, result.observed.score_away
    )));
    for (name, count) in &result.observed.event_counts {
        assert!(
            report.contains(&format!("{name}:{count}")),
            "the report must carry the observed count for {name}"
        );
    }
    assert_eq!(
        result
            .observed
            .window_event_ticks
            .keys()
            .collect::<Vec<_>>(),
        vec!["tackle", "keeper", "aerial", "full_time"],
        "every window reports the tick its subject fired on"
    );
}

/// The reported half's own go-red demonstration, standing rather than
/// transcribed into a commit message (AGENTS.md §9, and the shape #502
/// settled on). `behavioral_drift` is pure over two `BehaviorClaims`, so each
/// class of drift is provable in microseconds instead of 7,201 ticks.
///
/// Without these three tests, a refactor that quietly stopped comparing one
/// of the three demoted claims would look exactly like a build with no drift.
#[test]
fn the_behavioral_drift_report_names_a_moved_score_and_event_count() {
    let recorded = determinism_evidence::BehaviorClaims {
        score_home: 1,
        score_away: 0,
        event_counts: [("tackle".to_string(), 147), ("touch".to_string(), 180)]
            .into_iter()
            .collect(),
        window_event_ticks: [("tackle".to_string(), Some(24))].into_iter().collect(),
    };
    assert!(determinism_evidence::behavioral_drift(&recorded, &recorded).is_empty());
    assert_eq!(
        determinism_evidence::render_drift(&determinism_evidence::behavioral_drift(
            &recorded, &recorded
        )),
        "none"
    );

    let mut observed = recorded.clone();
    observed.score_away = 2;
    observed.event_counts.insert("tackle".to_string(), 151);
    let drift = determinism_evidence::behavioral_drift(&recorded, &observed);
    assert_eq!(
        determinism_evidence::render_drift(&drift),
        "score:1-0->1-2;event_counts.tackle:147->151"
    );
    assert_eq!(drift[0].claim, "score");
    assert_eq!(drift[0].recorded, "1-0");
    assert_eq!(drift[0].observed, "1-2");
}

/// An event kind the recording never saw, and one it saw that this build does
/// not, are both named — the two directions `finish_campaign` used to return
/// `fixture gained unexpected event` / `fixture event count drifted` for.
#[test]
fn the_behavioral_drift_report_names_a_gained_and_a_lost_event_kind() {
    let recorded = determinism_evidence::BehaviorClaims {
        score_home: 1,
        score_away: 0,
        event_counts: [("catch".to_string(), 1)].into_iter().collect(),
        window_event_ticks: Default::default(),
    };
    let observed = determinism_evidence::BehaviorClaims {
        event_counts: [("save".to_string(), 3)].into_iter().collect(),
        ..recorded.clone()
    };
    assert_eq!(
        determinism_evidence::render_drift(&determinism_evidence::behavioral_drift(
            &recorded, &observed
        )),
        "event_counts.catch:1->absent;event_counts.save:absent->3"
    );
}

/// A window whose event moved, and one whose event never fired at all. The
/// second case is the one that matters: `verify_window` still fails a window
/// that lost its event entirely (that is coverage, and it stays gated), so a
/// `none` here can only come from the reporter being handed an incomplete
/// observation — which is exactly the silent-report failure this test exists
/// to catch.
#[test]
fn the_behavioral_drift_report_names_a_moved_window_event_tick() {
    let recorded = determinism_evidence::BehaviorClaims {
        score_home: 1,
        score_away: 0,
        event_counts: std::collections::BTreeMap::new(),
        window_event_ticks: [
            ("tackle".to_string(), Some(24)),
            ("keeper".to_string(), Some(1692)),
        ]
        .into_iter()
        .collect(),
    };
    let observed = determinism_evidence::BehaviorClaims {
        window_event_ticks: [
            ("tackle".to_string(), Some(25)),
            ("keeper".to_string(), None),
        ]
        .into_iter()
        .collect(),
        ..recorded.clone()
    };
    assert_eq!(
        determinism_evidence::render_drift(&determinism_evidence::behavioral_drift(
            &recorded, &observed
        )),
        "windows.tackle.event_tick:24->25;windows.keeper.event_tick:1692->none"
    );
}

/// Extract one `name=value` field from a rendered `GC_DETERMINISM` line, so
/// the assertions below compare whole field values rather than hunting for
/// substrings that a neighbouring field could satisfy.
fn report_field(report: &str, name: &str) -> String {
    let prefix = format!("{name}=");
    report
        .split('|')
        .find_map(|part| part.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("the report must carry a {name}= field"))
        .to_string()
}

/// `report()`'s own go-red demonstration, and the only place in this file
/// where the reporter's central claim is provable at all.
///
/// `events=` and `score=` are observations *of this run*. Before #505
/// `events=` iterated the fixture's frozen `event_counts` — harmless while a
/// count mismatch was an `Err` no caller could reach this line past, and a lie
/// the moment that check became a report. Since the demotion, `events=` is the
/// only place a reader learns what actually happened, so an unasserted
/// reporter here is precisely the failure shape AGENTS.md §9 warns about.
///
/// Why it cannot be proved by the campaign test: on a green tree the
/// campaign's observation *equals* the frozen fixture by construction, so the
/// echoing implementation and the correct one render byte-identical lines.
/// Reverting `event_parts` to iterate `fixture.event_counts` leaves every
/// other test in this file green. Handing `report` a result whose observation
/// deliberately diverges is the only thing that separates them — and since
/// `report` is pure over its argument and every field of
/// `DeterminismEvidenceResult` is `pub`, it costs no campaign to do.
#[test]
fn the_report_carries_this_runs_counts_and_score_not_the_frozen_fixtures() {
    let frozen = determinism_evidence::fixture_claims();
    assert!(
        !frozen.event_counts.is_empty(),
        "the frozen fixture must record counts for this test to have anything to diverge from"
    );

    // A divergence of the shape #488 and #489 actually produce: the same event
    // kinds still fire, at different counts, with a different scoreline.
    let observed = determinism_evidence::BehaviorClaims {
        score_home: frozen.score_home + 3,
        score_away: frozen.score_away + 4,
        event_counts: frozen
            .event_counts
            .iter()
            .map(|(name, count)| (name.clone(), count + 1000))
            .collect(),
        window_event_ticks: frozen.window_event_ticks.clone(),
    };
    let result = determinism_evidence::DeterminismEvidenceResult {
        fixture_id: "test-only".to_string(),
        ticks: 7201,
        boundaries: 7202,
        final_hash: "0".to_string(),
        sequence_digest: "0".to_string(),
        score_home: observed.score_home,
        score_away: observed.score_away,
        outcome: determinism_evidence::Outcome::Away,
        snapshot_bytes: 0,
        coverage: determinism_evidence::DeterminismCoverage::default(),
        drift: determinism_evidence::behavioral_drift(&frozen, &observed),
        observed: observed.clone(),
    };
    let report = determinism_evidence::report(&result);

    let render_counts = |claims: &determinism_evidence::BehaviorClaims| {
        claims
            .event_counts
            .iter()
            .map(|(name, count)| format!("{name}:{count}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    assert_ne!(
        render_counts(&observed),
        render_counts(&frozen),
        "the perturbation must actually diverge from the recording, or this test proves nothing"
    );

    assert_eq!(
        report_field(&report, "events"),
        render_counts(&observed),
        "events= must render this run's counts"
    );
    assert_ne!(
        report_field(&report, "events"),
        render_counts(&frozen),
        "events= must not echo the frozen fixture's counts back — that is the pre-#505 bug"
    );
    assert_eq!(
        report_field(&report, "score"),
        format!("{}-{}", observed.score_home, observed.score_away),
        "score= must render this run's score"
    );
    assert_ne!(
        report_field(&report, "score"),
        format!("{}-{}", frozen.score_home, frozen.score_away),
        "score= must not echo the fixture's expected_score — the same bug one field over"
    );
    // And the drift that same divergence produces still reaches the line, so a
    // reader of a drifted run sees both what happened and what moved.
    assert_ne!(report_field(&report, "drift"), "none");
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

/// A natural extension `gc_sim::replay` exists for: the frozen fixture tape
/// replayed through `replay::run` must reproduce every boundary hash with
/// zero divergence, the same guarantee `determinism_evidence::verify`
/// proves through its own path.
#[test]
fn the_frozen_fixture_tape_replays_through_sim_replay_with_no_divergence() {
    let tune = Tuning::new();
    let tape = determinism_evidence::fixture_tape(&tune).expect("the fixture materializes");
    let result = replay::run(&tape, &tape.identity, &tune).expect("replay context is valid");
    assert!(result.divergence.is_none());
    assert_eq!(result.boundaries.len(), tape.boundary_hashes.len());
}

/// `determinism_evidence::sequence_digest` is the same fold a campaign
/// performs incrementally: run over the pinned boundary hashes it must
/// reproduce the pinned digest. Without this, a re-record could write a
/// digest no campaign can ever agree with.
#[test]
fn the_sequence_digest_helper_reproduces_the_pinned_digest_from_the_pinned_hashes() {
    let fixture = omp1_determinism::fixture();
    let hashes: Vec<String> = omp1_determinism::boundary_hash_lines()
        .iter()
        .map(|line| (*line).to_string())
        .collect();
    assert_eq!(
        determinism_evidence::sequence_digest(&hashes),
        fixture.expected_sequence_digest
    );
    assert_ne!(
        determinism_evidence::sequence_digest(&hashes[..hashes.len() - 1]),
        fixture.expected_sequence_digest
    );
}

/// Re-records the **derived half** of the OMP-1 determinism fixture
/// (`gc-data/src/omp1_determinism.json`) from this build: `boundary_hashes`,
/// `boundary_count`, `expected_final_hash`, `expected_sequence_digest`.
///
/// NOT a CI-asserted test, and `#[ignore]`d so it never runs in the gate: it
/// PRINTS the regenerated fixture document to stdout for a human to capture,
/// the same workflow as `input_sample_vector_generator.rs` and
/// `session_legacy_differential.rs`'s recorder. A recorder that overwrote
/// the fixture on every `cargo test` would turn a determinism regression
/// into a no-op.
///
/// **The frozen half is never re-recorded, and this asserts that rather than
/// trusting it.** The recorded input — `frame_wires`, `identity`,
/// `source_seeds`, `windows` — came from source bots deleted from this
/// repository and can never be recaptured; only values *derived* from it by
/// replaying it may be re-derived. Two checks, before anything is printed:
/// every wire this replay consumed re-encodes to the frozen blob byte for
/// byte, and `omp1_determinism::rerecorded_json` parses its own output back
/// and refuses it if any frozen field moved.
///
/// **Re-recording is a decision, not a fix.** A red boundary hash in
/// `determinism_evidence_pins_the_full_fixed_input_match_on_the_explicit_evidence_command`
/// is a FINDING first: the state this simulation reaches from fixed input
/// changed, and an unintended change of that kind is a desync between
/// clients built on either side of it. Re-record only for a deliberate,
/// reviewed change, and record with it the decision, the superseding change,
/// and the last commit at which the previous baseline held — the rule #500
/// set for behavioral vectors. And read what a pass proves afterwards: a
/// self-recorded baseline detects change, it cannot detect "wrong but
/// consistently wrong". `gc_data::omp1_determinism`'s module doc states the
/// full trade.
///
/// Run (two steps on purpose — a `>` redirect straight onto the fixture
/// truncates it to nothing if the recorder panics):
///
/// ```text
/// cd rust
/// cargo test -p gc-sim --test determinism_evidence -- \
///     --ignored --nocapture record_omp1_derived_baseline \
///   | sed -n 's/^GC_OMP1_FIXTURE_JSON //p' | tr -d '\n' > /tmp/omp1_determinism.json
/// test -s /tmp/omp1_determinism.json \
///   && mv /tmp/omp1_determinism.json crates/gc-data/src/omp1_determinism.json
/// git -C .. diff --stat rust/crates/gc-data/src/omp1_determinism.json
/// ```
///
/// The `sed` selects the one marked line (the document is minified onto a
/// single line, so nothing of it can be filtered) and drops libtest's own
/// status lines and the `#` summary; `tr -d '\n'` removes the newline `sed`
/// adds, because the committed file has none.
#[test]
#[ignore]
fn record_omp1_derived_baseline() {
    let tune = Tuning::new();
    let fixture = omp1_determinism::fixture();
    let recording = determinism_evidence::record(&tune).expect("the frozen OMP-1 frames replay");

    // The frozen half, checked before a single byte is printed.
    let replayed_wires: String = recording
        .frame_wires
        .iter()
        .map(|wire| format!("{wire}\n"))
        .collect();
    assert_eq!(
        recording.frame_wires.len() as i64,
        fixture.frame_count,
        "the replay must consume every frozen frame and no more"
    );
    assert!(
        replayed_wires == fixture.frame_wires,
        "the replayed frames must re-encode to the frozen frame_wires blob byte for byte -- the \
         recorded input is not re-recordable"
    );
    assert_eq!(
        recording.boundary_hashes.len(),
        recording.frame_wires.len() + 1,
        "one boundary per frame, plus the initial boundary"
    );

    let digest = determinism_evidence::sequence_digest(&recording.boundary_hashes);
    let final_hash = recording
        .boundary_hashes
        .last()
        .expect("at least one boundary")
        .clone();
    let json = omp1_determinism::rerecorded_json(&omp1_determinism::Omp1DerivedBaseline {
        boundary_hashes: recording.boundary_hashes.clone(),
        sequence_digest: digest.clone(),
    })
    .expect("the frozen half survives the write-back");

    let pinned = omp1_determinism::boundary_hash_lines();
    let moved: Vec<usize> = recording
        .boundary_hashes
        .iter()
        .enumerate()
        .filter(|(index, hash)| pinned.get(*index) != Some(&hash.as_str()))
        .map(|(index, _)| index)
        .collect();

    println!(
        "# GC_OMP1_RERECORD -- derived half of the OMP-1 determinism fixture, from this build."
    );
    println!(
        "# Frozen half verified untouched before printing: frame_wires (all {} wires re-encoded), identity, source_seeds, windows.",
        recording.frame_wires.len()
    );
    println!(
        "# boundary_count           {} (pinned {})",
        recording.boundary_hashes.len(),
        fixture.boundary_count
    );
    println!(
        "# expected_final_hash      {final_hash} (pinned {})",
        fixture.expected_final_hash
    );
    println!(
        "# expected_sequence_digest {digest} (pinned {})",
        fixture.expected_sequence_digest
    );
    println!(
        "# boundary hashes moved    {} of {}{}",
        moved.len(),
        recording.boundary_hashes.len(),
        match moved.first() {
            Some(first) => format!(" (first at boundary {first})"),
            None => " -- this build reproduces the pinned baseline exactly".to_string(),
        }
    );

    // event_counts, expected_score and windows are derived from the same
    // replay but are the fixture's BEHAVIORAL claims, so they are copied
    // verbatim rather than refreshed (see gc_data::omp1_determinism's module
    // doc). Since #505 they no longer FAIL the campaign either -- they are
    // reported. This block is one of the three channels that reporting
    // reaches a human through, and it is the earliest: re-recording the
    // derived hashes is exactly when a contributor is deciding whether a
    // behavior change was intended.
    //
    // It shares `behavioral_drift` with the campaign rather than
    // re-implementing the comparison, so the recorder can never disagree with
    // the gate about what moved.
    let observed = determinism_evidence::BehaviorClaims {
        score_home: recording.score_home,
        score_away: recording.score_away,
        event_counts: recording
            .event_counts
            .iter()
            .map(|(name, count)| (name.clone(), *count))
            .collect(),
        window_event_ticks: fixture
            .windows
            .iter()
            .map(|window| {
                let key = window.event_kind.as_deref().unwrap_or(&window.name);
                (window.name.clone(), recording.event_ticks.get(key).copied())
            })
            .collect(),
    };
    let behavioral =
        determinism_evidence::behavioral_drift(&determinism_evidence::fixture_claims(), &observed);
    if behavioral.is_empty() {
        println!(
            "# behavioral claims        unchanged (event_counts, expected_score, window event ticks)"
        );
    } else {
        println!("#");
        println!("# WARNING: this build also moved the fixture's BEHAVIORAL claims, which this");
        println!(
            "# recorder deliberately does NOT refresh -- they are what the fixture is evidence"
        );
        println!("# OF (that it covers a tackle, a catch, a header, a full time, ending 1-0), and");
        println!(
            "# moving them is a larger decision than re-deriving a digest. The document below"
        );
        println!("# keeps the pinned values. Since #505 the campaign REPORTS this difference as");
        println!("# drift instead of failing on it, so nothing downstream will stop you: decide");
        println!("# here whether it was intended, and record it in the PR that ships the change.");
        for entry in &behavioral {
            println!(
                "#   {} {} -> {}",
                entry.claim, entry.recorded, entry.observed
            );
        }
    }
    println!("#");
    println!("# Capture the marked line below; see this test's doc comment for the command.");
    println!("GC_OMP1_FIXTURE_JSON {json}");
}
