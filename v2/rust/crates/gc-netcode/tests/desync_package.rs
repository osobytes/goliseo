//! Port of `spec/game/online_desync_package_spec.lua`.
//!
//! ## What could not be ported as-is
//!
//! The Lua spec's `capture()` helper spins up a real 2v2 fixture match
//! through `game.online.match_driver`, `game.online.net_diagnostics_fixture`,
//! and `sim.rollback_session`. `match_driver` and `sim.match_snapshot`/
//! `sim.rollback_session` have since landed in this workspace (`gc-netcode`'s
//! `match_driver`/`match_driver_fixture`, `gc-sim`'s `match_snapshot`/
//! `rollback_session`) and are used directly below — see
//! `hash_at_boundary`/`rebuilds_the_agreed_boundary_hash_from_the_package_alone`.
//! `game.online.net_diagnostics`/`net_diagnostics_fixture` remain
//! TypeScript-owned per `v2/README.md` §2 with no Rust port planned, so
//! `capture()`'s live-harness half — and the one test that needs
//! `net_diagnostics.export` specifically — stay out of reach.
//!
//! What *is* ported below exercises exactly the same `desync_package` logic
//! the Lua spec does — reproducibility classification, wire truncation,
//! digest determinism, the opt-in requirement, boundary ordering, `rows`
//! de-duplication and ordering, redaction-free summaries, and (now) offline
//! boundary-hash reproduction through a real `rollback_session` — using
//! hand-built
//! [`gc_netcode::desync_package::Diagnostics`]/[`gc_netcode::desync_package::BuildOptions`]
//! fixtures in place of a live match harness. The one assertion that only a
//! live `net_diagnostics` export could produce is marked `#[ignore]`, naming
//! that module.

use gc_netcode::desync_package::{
    self, BuildOptions, CheckpointRecord, ControlRecord, Diagnostics, DifferenceValue,
    FirstDifference, RuntimeEventRecord, SessionIdentity,
};
use gc_netcode::input_protocol::{self, AuthorityRow, PacketOptions};
use gc_sim::input_frame;

const SESSION_ID: &str = "session_alpha";
const MANIFEST_ID: &str = "eb59f113614c35b2";

fn session_identity(peer_id: &str) -> SessionIdentity {
    SessionIdentity {
        session_id: SESSION_ID.to_string(),
        peer_id: peer_id.to_string(),
        role: "host".to_string(),
        match_mode: "2v2".to_string(),
        combat_status: "provisional_114".to_string(),
        manifest_id: MANIFEST_ID.to_string(),
        assignment_id: "0011223344556677".to_string(),
        countdown_id: "countdown_1".to_string(),
        build_id: "build.97b60ea".to_string(),
        source_id: "source.97b60ea".to_string(),
        content_id: "content.omp3.v1".to_string(),
        tuning_id: "tuning.omp3.v1".to_string(),
        match_config_id: "match_config.direct_host.v1".to_string(),
        fixture_id: "fixture.default_mixed.v1".to_string(),
        arena_id: "arena.goliseo".to_string(),
        combat_rules_id: "combat_interaction.accepted_2026_07_23".to_string(),
        gameplay_ai_policy_id: "gameplay_ai.combat.v1".to_string(),
        network_profile_digest: None,
        protocol_version: 1,
        input_version: input_frame::VERSION,
        snapshot_version: 11,
        tape_version: 1,
        combat_schema_version: 13,
        seed: 20001,
        tick_rate: 60,
        duration_ticks: 7200,
        max_goals: 99,
    }
}

fn diagnostics(peer_id: &str) -> Diagnostics {
    Diagnostics {
        session: session_identity(peer_id),
        checkpoints: vec![
            CheckpointRecord {
                tick: 0,
                hash: "0000000000000001".to_string(),
                live: vec![("host".to_string(), "host".to_string())],
            },
            CheckpointRecord {
                tick: 5,
                hash: "0000000000000002".to_string(),
                live: vec![("host".to_string(), "host".to_string())],
            },
        ],
        control: vec![ControlRecord {
            ordinal: 0,
            kind: "handshake".to_string(),
            peer_id: peer_id.to_string(),
            sequence: 0,
            message_id: "msg_0".to_string(),
        }],
        runtime_events: vec![RuntimeEventRecord {
            ordinal: 0,
            kind: "peer_state".to_string(),
            monotonic_ms: 1234.5,
            peer_id: Some(peer_id.to_string()),
            channel: Some("control".to_string()),
            state: Some("connected".to_string()),
            code: None,
            detail: None,
        }],
    }
}

/// A single-row host packet naming exactly `tick`, encoded to its wire bytes
/// — a minimal, always-decodable stand-in for a real match's canonical
/// input wire at that tick.
fn wire_for_tick(tick: i64) -> Vec<u8> {
    let packet = input_protocol::new_host(PacketOptions {
        session_id: SESSION_ID.to_string(),
        manifest_id: MANIFEST_ID.to_string(),
        sender_id: "host".to_string(),
        sequence: tick,
        transport_tick: tick,
        first_input_tick: 0,
        confirmed_span: None,
        rows: vec![AuthorityRow {
            tick,
            slot_index: 1,
            sample: input_frame::neutral_sample(),
        }],
    })
    .expect("wire_for_tick packet is always valid");
    input_protocol::encode(&packet).expect("wire_for_tick packet always encodes")
}

fn base_options(wires: Vec<Vec<u8>>) -> BuildOptions {
    BuildOptions {
        diagnostics: Some(diagnostics("host")),
        session_id: SESSION_ID.to_string(),
        manifest_id: MANIFEST_ID.to_string(),
        first_input_tick: 0,
        peer_id: "host".to_string(),
        remote_peer_id: "guest_2".to_string(),
        agreed_boundary_tick: 0,
        agreed_boundary_hash: "0000000000000001".to_string(),
        divergence_tick: 5,
        local_hash: "0000000000000002".to_string(),
        remote_hash: "deadbeefdeadbeef".to_string(),
        input_wires: wires,
        first_difference: Some(FirstDifference {
            path: "state.ball.pos.x".to_string(),
            expected: DifferenceValue::Number(480.5),
            actual: DifferenceValue::Number(480.75),
        }),
        tape: None,
    }
}

fn wires_reaching_boundary_zero() -> Vec<Vec<u8>> {
    (0..=5).map(wire_for_tick).collect()
}

#[test]
fn carries_the_identity_needed_to_rebuild_the_exact_match() {
    let options = base_options(wires_reaching_boundary_zero());
    let package = desync_package::build(options).unwrap();
    assert_eq!(package.package_version, desync_package::VERSION);
    assert_eq!(package.session.manifest_id, MANIFEST_ID);
    assert_eq!(package.session.fixture_id, "fixture.default_mixed.v1");
    assert_eq!(package.session.build_id, "build.97b60ea");
    assert_eq!(package.session.input_version, input_frame::VERSION);
    assert_eq!(package.session.snapshot_version, 11);
    assert_eq!(package.session.tape_version, 1);
    assert_eq!(package.reproduction.local_peer_id, "host");
    assert_eq!(package.reproduction.remote_peer_id, "guest_2");
}

#[test]
fn names_the_first_differing_path_and_both_boundary_hashes() {
    let options = base_options(wires_reaching_boundary_zero());
    let package = desync_package::build(options).unwrap();
    let divergence = &package.divergence;
    assert_eq!(divergence.agreed_boundary_tick, 0);
    assert_eq!(divergence.agreed_boundary_hash, "0000000000000001");
    assert_eq!(divergence.divergence_tick, 5);
    assert_eq!(divergence.local_hash, "0000000000000002");
    assert_eq!(divergence.remote_hash, "deadbeefdeadbeef");
    let difference = divergence.first_difference.as_ref().unwrap();
    assert_eq!(difference.path, "state.ball.pos.x");
    assert_ne!(difference.expected, difference.actual);
}

#[test]
fn claims_fixture_boundary_zero_reproducibility_only_when_the_wires_reach_it() {
    let full = base_options(wires_reaching_boundary_zero());
    let package = desync_package::build(full).unwrap();
    assert_eq!(
        package.reproduction.reproducible_from,
        desync_package::ReproducibleFrom::FixtureBoundaryZero
    );
    assert_eq!(package.inputs.from_input_tick, 0);
    assert_eq!(
        package.inputs.retention,
        desync_package::Retention::Complete
    );

    // Drop the opening wire and the claim must weaken rather than persist.
    let mut trimmed = base_options(wires_reaching_boundary_zero());
    trimmed.input_wires.remove(0);
    let weaker = desync_package::build(trimmed).unwrap();
    assert_eq!(
        weaker.reproduction.reproducible_from,
        desync_package::ReproducibleFrom::RetainedWindow
    );
    assert!(weaker.inputs.from_input_tick > package.inputs.from_input_tick);
}

#[test]
fn stays_bounded_and_marks_truncation_of_the_wire_window() {
    let mut many = Vec::new();
    for tick in 0..(desync_package::MAX_WIRES as i64 + 20) {
        many.push(wire_for_tick(tick));
    }
    let options = base_options(many);
    let built = desync_package::build(options).unwrap();
    assert_eq!(built.inputs.wires.len(), desync_package::MAX_WIRES);
    assert_eq!(built.inputs.retention, desync_package::Retention::Truncated);
    assert_ne!(
        built.reproduction.reproducible_from,
        desync_package::ReproducibleFrom::FixtureBoundaryZero
    );
    let bytes = desync_package::encode(&built).unwrap();
    assert!(
        bytes.len() < 400 * 1024,
        "a bounded capture grew past a size anyone would attach"
    );
}

#[test]
fn refuses_a_divergence_that_is_not_after_the_agreed_boundary() {
    let mut options = base_options(wires_reaching_boundary_zero());
    options.agreed_boundary_tick = 5;
    options.divergence_tick = 0;
    let err = desync_package::build(options).unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn requires_the_diagnostic_export_opt_in() {
    let mut options = base_options(wires_reaching_boundary_zero());
    options.diagnostics = None;
    let err = desync_package::build(options).unwrap_err();
    assert!(err.contains("opted-in"), "unexpected error: {err}");
}

#[test]
fn encodes_deterministically_and_carries_no_signalling_material() {
    let first = desync_package::build(base_options(wires_reaching_boundary_zero())).unwrap();
    let second = desync_package::build(base_options(wires_reaching_boundary_zero())).unwrap();
    assert_eq!(
        desync_package::digest(&second).unwrap(),
        desync_package::digest(&first).unwrap(),
        "two identical captures produced different package digests"
    );

    // Package -> Value isn't exposed publicly (it is an internal encode/
    // digest implementation detail), so this walks the encoded wire bytes
    // and every plain-text identity field directly instead.
    let mut haystack = desync_package::encode(&first).unwrap();
    haystack.extend_from_slice(first.session.session_id.as_bytes());
    for forbidden in ["ice-", "candidate", "v=0", "sdp", "192.168."] {
        assert!(
            !contains_subslice(&haystack, forbidden.as_bytes()),
            "{forbidden} reached a desync package"
        );
    }
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn summarises_without_exposing_anything_sensitive() {
    let package = desync_package::build(base_options(wires_reaching_boundary_zero())).unwrap();
    let lines = desync_package::summary(&package);
    assert!(lines.len() >= 5);
    let joined = lines.join("\n");
    assert!(joined.contains("reproducible from"));
    assert!(joined.contains(&package.session.manifest_id));
}

#[test]
fn rows_deduplicates_and_orders_canonically() {
    // Two overlapping wires: tick 0..=2 twice (idempotent resend) plus a
    // fresh tick 3, in reverse arrival order — `rows` must still return
    // ascending (tick, slot) with no duplicate.
    let mut package_wires = Vec::new();
    for tick in 0..=2 {
        package_wires.push(wire_for_tick(tick));
    }
    package_wires.push(wire_for_tick(3));
    package_wires.reverse();
    let options = base_options(package_wires);
    let package = desync_package::build(options).unwrap();

    let rows = desync_package::rows(&package, SESSION_ID, "host").unwrap();
    assert_eq!(rows.len(), 4);
    for window in rows.windows(2) {
        let (previous, current) = (&window[0], &window[1]);
        assert!(
            current.tick > previous.tick
                || (current.tick == previous.tick && current.slot_index > previous.slot_index),
            "package rows are not in canonical (tick, slot) order"
        );
    }
}

/// Feeds `rows` into a fresh [`gc_sim::rollback_session`], steps it to
/// `boundary`, and returns [`gc_sim::match_snapshot::hash`] of the boundary
/// it reaches. Mirrors the Lua original's offline-reproduction steps: a
/// reproducer holds no local slots, so every row arrives as remote
/// authority, exactly as it would on a machine that never played.
fn hash_at_boundary(rows: &[AuthorityRow], boundary: i64) -> String {
    let initial = gc_netcode::match_driver_fixture::initial_snapshot(None, false, None);
    let sources = [gc_sim::rollback_input_history::RollbackInputSource::Remote; 8];
    let mut session = gc_sim::rollback_session::new(&initial, sources, None, None);
    let arrivals: Vec<gc_sim::rollback_input_history::RollbackAuthoritativeInput> = rows
        .iter()
        .map(
            |row| gc_sim::rollback_input_history::RollbackAuthoritativeInput {
                tick: row.tick,
                slot_index: row.slot_index,
                sample: row.sample,
            },
        )
        .collect();
    let accepted = gc_sim::rollback_session::add_authoritative_batch(&mut session, &arrivals)
        .expect("a well-formed authoritative batch is accepted");
    assert!(accepted.inserted > 0);
    loop {
        let diagnostics = gc_sim::rollback_session::diagnostics(&session);
        if diagnostics.present_boundary > boundary {
            break;
        }
        if gc_sim::rollback_session::step(&mut session).is_err() {
            break;
        }
    }
    let lookup = gc_sim::rollback_session::snapshot(&session, boundary);
    assert!(
        matches!(
            lookup.status,
            gc_sim::rollback_snapshot_history::RollbackSnapshotLookupStatus::Present
                | gc_sim::rollback_snapshot_history::RollbackSnapshotLookupStatus::Retained
        ),
        "the reproduction could not reach the boundary the package named"
    );
    gc_sim::match_snapshot::hash(
        lookup
            .snapshot
            .as_ref()
            .expect("Present/Retained always carries a snapshot"),
    )
}

/// `gc_sim::match_snapshot`/`gc_sim::rollback_session` — this test's
/// original blocker — have since landed, so the offline-reproduction proof
/// itself is ported for real below. What is still substituted, same as
/// every other test in this file: the Lua original's `capture()` runs a real
/// 2v2 match through `net_diagnostics_fixture` (`fixture.harness`/
/// `fixture.run`) to get a *captured* boundary hash to reproduce; that
/// fixture is TypeScript-owned with no Rust port planned (see the module doc
/// comment), so this builds the "captured" side the same way the rest of
/// this file substitutes a live harness — hand-built wires, real
/// `rollback_session` math — and computes the hash it claims via the exact
/// same reproduction recipe rather than importing one from a live capture.
/// The property under test is unchanged: a package's own rows, replayed
/// through a *fresh* session that never saw the original, reach the
/// boundary hash the package names.
#[test]
fn rebuilds_the_agreed_boundary_hash_from_the_package_alone() {
    let boundary = 8i64;
    let wires: Vec<Vec<u8>> = (0..=boundary).map(wire_for_tick).collect();

    // What a live capture's boundary hash would have been — computed by the
    // same reproduction recipe the package's own claim is checked against
    // below, since there is no `net_diagnostics_fixture` session to capture
    // it from directly (see this test's doc comment).
    let captured_rows: Vec<AuthorityRow> = (0..=boundary)
        .map(|tick| AuthorityRow {
            tick,
            slot_index: 1,
            sample: input_frame::neutral_sample(),
        })
        .collect();
    let captured_hash = hash_at_boundary(&captured_rows, boundary);

    let mut options = base_options(wires);
    options.agreed_boundary_tick = boundary;
    options.agreed_boundary_hash = captured_hash.clone();
    options.divergence_tick = boundary + 1;
    options.local_hash = "0000000000000003".to_string();
    let package = desync_package::build(options).unwrap();
    assert_eq!(
        package.reproduction.reproducible_from,
        desync_package::ReproducibleFrom::FixtureBoundaryZero
    );

    let rows = desync_package::rows(&package, SESSION_ID, "host").unwrap();
    assert!(!rows.is_empty());
    // Canonical order is a contract of `rows`, not an accident of arrival.
    for window in rows.windows(2) {
        let (previous, current) = (&window[0], &window[1]);
        assert!(
            current.tick > previous.tick
                || (current.tick == previous.tick && current.slot_index > previous.slot_index),
            "package rows are not in canonical (tick, slot) order"
        );
    }

    let reproduced_hash = hash_at_boundary(&rows, boundary);
    assert_eq!(
        reproduced_hash, package.divergence.agreed_boundary_hash,
        "an offline reproduction disagreed with the captured boundary hash"
    );
}

#[test]
#[ignore = "blocked on game.online.net_diagnostics (TypeScript-owned per \
v2/README.md \u{a7}2; no Rust port exists or is planned): the Lua test asserts \
net_diagnostics.export(...)'s output agrees field-for-field with the built \
package's session, and net_diagnostics has no Rust type to call export() on \
at all. (game.online.match_driver_fixture, this reason's other blocker as \
originally written, has since landed and was never really load-bearing here \
in the first place -- this port's desync_package::build takes an \
already-exported Diagnostics directly, see the module doc comment, so there \
is no separate export call to cross-check against regardless of \
match_driver_fixture's status.)"]
fn keeps_the_export_and_the_package_agreeing_on_identity() {}
