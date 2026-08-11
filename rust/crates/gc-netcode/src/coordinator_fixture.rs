//! Port of `game/online/coordinator_fixture.lua`.
//!
//! Shared fixtures for coordinator-focused specs: a host/guest peer id
//! scheme, a runtime and manifest borrowed from [`crate::protocol_fixture`],
//! and constructors for host/guest [`coordinator::CoordinatorState`]s.

use crate::coordinator::{self, CoordinatorState, ManifestExpectation, Options, Role};
use crate::protocol::{self, Value};
use crate::protocol_fixture;

/// The fixture host's peer id.
pub const HOST_PEER_ID: &str = "host";
/// Prefix for a fixture guest's peer id (see [`guest_peer_id`]).
pub const GUEST_PREFIX: &str = "guest.";
/// Prefix for a fixture guest's transport link id (see [`link_id`]).
pub const LINK_PREFIX: &str = "link.";
/// The fixture countdown identity `reach_start` freezes with.
pub const COUNTDOWN_ID: &str = "countdown.1";

/// A fixture runtime identity, borrowed from [`protocol_fixture::runtime`].
#[must_use]
pub fn runtime() -> Value {
    protocol_fixture::runtime()
}

/// A fixture manifest, borrowed from [`protocol_fixture::manifest`].
#[must_use]
pub fn manifest(mode: Option<protocol::MatchMode>) -> Value {
    protocol_fixture::manifest(mode)
}

/// What a guest can verify locally before accepting the fixture manifest:
/// immutable build, content, tuning, fixture, and combat-disposition
/// identity. Session-scoped values such as the seed are chosen by the host
/// and are not pre-known.
#[must_use]
pub fn expectation() -> ManifestExpectation {
    let manifest = manifest(None);
    ManifestExpectation {
        build_id: manifest
            .get("build_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        source_id: manifest
            .get("source_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        content_id: manifest
            .get("content_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        tuning_id: manifest
            .get("tuning_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        match_config_id: manifest
            .get("match_config_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        fixture_id: manifest
            .get("fixture_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        arena_id: manifest
            .get("arena_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        combat_rules_id: manifest
            .get("combat_rules_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        gameplay_ai_policy_id: manifest
            .get("gameplay_ai_policy_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        combat_status: manifest
            .get("combat_status")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// The peer id of the Nth (1-based) fixture guest.
#[must_use]
pub fn guest_peer_id(index: i64) -> String {
    format!("{GUEST_PREFIX}{index}")
}

/// The transport link id a fixture guest's host<->guest link uses.
#[must_use]
pub fn link_id(peer_id: &str) -> String {
    format!("{LINK_PREFIX}{peer_id}")
}

/// The host peer id, then `guest_count` guest peer ids in admission order.
#[must_use]
pub fn peer_ids(guest_count: i64) -> Vec<String> {
    let mut ids = vec![HOST_PEER_ID.to_string()];
    for index in 1..=guest_count {
        ids.push(guest_peer_id(index));
    }
    ids
}

/// A fixture host coordinator.
///
/// # Panics
///
/// Panics if construction fails — a programmer error in the fixture itself.
#[must_use]
pub fn host(session_id: Option<&str>) -> CoordinatorState {
    let session_id = session_id.map(str::to_string).unwrap_or_else(|| {
        manifest(None)
            .get("session_id")
            .and_then(Value::as_str)
            .expect("fixture manifest has a session id")
            .to_string()
    });
    coordinator::new(Options {
        role: Role::Host,
        session_id,
        peer_id: HOST_PEER_ID.to_string(),
        host_peer_id: None,
        host_link_id: None,
        runtime: runtime(),
        build_id: None,
        expectation: None,
    })
    .expect("fixture host coordinator constructs")
}

/// A fixture guest coordinator.
///
/// # Panics
///
/// Panics if construction fails — a programmer error in the fixture itself.
#[must_use]
pub fn guest(
    index: i64,
    session_id: Option<&str>,
    expectation_override: Option<ManifestExpectation>,
) -> CoordinatorState {
    let peer_id = guest_peer_id(index);
    let session_id = session_id.map(str::to_string).unwrap_or_else(|| {
        manifest(None)
            .get("session_id")
            .and_then(Value::as_str)
            .expect("fixture manifest has a session id")
            .to_string()
    });
    coordinator::new(Options {
        role: Role::Guest,
        session_id,
        peer_id: peer_id.clone(),
        host_peer_id: Some(HOST_PEER_ID.to_string()),
        host_link_id: Some(link_id(&peer_id)),
        runtime: runtime(),
        build_id: None,
        expectation: Some(expectation_override.unwrap_or_else(expectation)),
    })
    .expect("fixture guest coordinator constructs")
}

/// A ready-to-publish assignment plan for `guest_count` fixture guests.
///
/// # Panics
///
/// Panics if planning fails — a programmer error in the fixture itself.
#[must_use]
pub fn assignments(guest_count: i64, mode: Option<protocol::MatchMode>) -> Value {
    coordinator::plan_assignments(&manifest(mode), &peer_ids(guest_count))
        .expect("fixture assignment plan is valid")
}
