//! Port of `spec/game/online_coordinator_driver_spec.lua`.
//!
//! Covers all four `describe` blocks: `"online coordinator driver"`,
//! `"online coordinator retransmission and stale readiness"`,
//! `"online coordinator adversarial peers"` (constructing malformed peer
//! messages via `driver:inject`), and `"online coordinator conformance"`.

use gc_netcode::coordinator::{self, Event, Origin, Terminal, TerminalReason};
use gc_netcode::coordinator_driver::{self as driver, Driver};
use gc_netcode::coordinator_fixture as fixture;
use gc_netcode::protocol::{self, Value};
use gc_sim::input_frame;

const HOST: &str = fixture::HOST_PEER_ID;

fn count_sources(session: &Driver) -> (i64, i64) {
    let freeze = session.host().state.freeze.as_ref().expect("frozen");
    let mut peers = 0;
    let mut bots = 0;
    for index in 1..=input_frame::SLOT_COUNT {
        let producer = freeze
            .assignments
            .get_index(index)
            .expect("canonical producer");
        match producer.get("producer_kind").and_then(Value::as_str) {
            Some("peer") => peers += 1,
            _ => bots += 1,
        }
    }
    (peers, bots)
}

/// Clone `assignments`, overwriting the `producer_id` of the producer at
/// 1-based canonical `index`.
fn with_producer_id(assignments: &Value, index: i64, producer_id: &str) -> Value {
    let mut result = assignments.clone();
    if let Value::Table(entries) = &mut result {
        for (key, value) in entries.iter_mut() {
            if key.as_int() == Some(index) {
                value.set("producer_id", Value::str(producer_id));
            }
        }
    }
    result
}

#[test]
fn runs_a_host_plus_seven_humans_from_connect_to_acknowledged_result() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(7),
        ..Default::default()
    });
    session.reach_start(Some(3), Some(0));

    assert_eq!(
        session.host().state.phase,
        protocol::LifecyclePhase::Running
    );
    assert!(session.all_started());
    for node in &session.nodes {
        assert_eq!(node.state.phase, protocol::LifecyclePhase::Running);
        assert_eq!(node.first_input_tick, Some(0));
        assert_eq!(
            node.state.freeze.as_ref().unwrap().countdown_id,
            fixture::COUNTDOWN_ID
        );
    }
    let (peers, bots) = count_sources(&session);
    assert_eq!(peers, 8);
    assert_eq!(bots, 0);

    session.play_out(Some(2), Some(1));
    assert!(session.all_terminal(Some(TerminalReason::Completed)));
    assert_eq!(session.host().state.result.as_ref().unwrap().home_score, 2);
    assert_eq!(session.host().state.terminal.as_ref().unwrap().code, None);
}

#[test]
fn fills_unoccupied_development_slots_with_deterministic_bots() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(2),
        ..Default::default()
    });
    session.reach_start(Some(2), Some(0));
    let (peers, bots) = count_sources(&session);
    assert_eq!(peers, 3);
    assert_eq!(bots, 5);

    let freeze = session.host().state.freeze.clone().unwrap();
    let mut seen: Vec<String> = Vec::new();
    for index in 1..=input_frame::SLOT_COUNT {
        let producer = freeze.assignments.get_index(index).unwrap();
        let producer_id = producer
            .get("producer_id")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        assert!(!seen.contains(&producer_id), "producer ids must be unique");
        seen.push(producer_id);
        if producer.get("producer_kind").and_then(Value::as_str) == Some("bot") {
            assert!(
                producer.get("bot_seed").is_some_and(|v| !v.is_nil()),
                "bot producers declare a seed"
            );
        }
    }
    let twin =
        coordinator::plan_assignments(&fixture::manifest(None), &fixture::peer_ids(2)).unwrap();
    assert_eq!(
        twin.get_index(8).unwrap().get("bot_seed"),
        freeze.assignments.get_index(8).unwrap().get("bot_seed")
    );

    session.play_out(Some(0), Some(3));
    assert!(session.all_terminal(Some(TerminalReason::Completed)));
}

#[test]
fn runs_a_solo_host_session_with_no_control_traffic_at_all() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(0),
        ..Default::default()
    });
    session.reach_start(Some(1), Some(0));
    assert_eq!(
        session.host().state.phase,
        protocol::LifecyclePhase::Running
    );
    assert_eq!(session.transcript.len(), 0);
    session.play_out(Some(1), Some(1));
    assert_eq!(
        session.host().state.terminal.as_ref().unwrap().reason,
        TerminalReason::Completed
    );
}

#[test]
fn is_deterministic_across_identical_runs() {
    let mut first = Driver::new(driver::Options {
        guest_count: Some(3),
        ..Default::default()
    });
    first.reach_start(Some(2), Some(0));
    first.play_out(Some(1), Some(0));
    let mut second = Driver::new(driver::Options {
        guest_count: Some(3),
        ..Default::default()
    });
    second.reach_start(Some(2), Some(0));
    second.play_out(Some(1), Some(0));
    assert_eq!(first.transcript_id(), second.transcript_id());
    assert_eq!(first.trace(), second.trace());
    assert_eq!(first.transcript_id().len(), 16);
}

#[test]
fn keeps_the_session_alive_when_a_guest_leaves_before_the_countdown() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(2),
        ..Default::default()
    });
    session.connect_all();
    session.send(
        HOST,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    );
    session.pump();
    session.send(
        HOST,
        Event::AssignSlots {
            assignments: fixture::assignments(2, None),
            preserve_claims: false,
        },
    );
    session.pump();
    let peer_ids: Vec<String> = session.nodes.iter().map(|n| n.peer_id.clone()).collect();
    for peer_id in &peer_ids {
        session.send(peer_id, Event::SetReady { ready: true });
    }
    session.pump();
    assert_eq!(session.host().state.phase, protocol::LifecyclePhase::Ready);

    session.send(&fixture::guest_peer_id(2), Event::Leave);
    session.pump();
    {
        let host = session.host();
        assert_eq!(host.terminal, None);
        assert_eq!(host.state.phase, protocol::LifecyclePhase::Assigned);
        assert_eq!(host.state.peers.len(), 2);
        assert_eq!(host.state.assignments, None);
        assert!(!host.state.peers[0].ready);
    }
    assert_eq!(
        session
            .node(&fixture::guest_peer_id(2))
            .unwrap()
            .terminal
            .as_ref()
            .unwrap()
            .reason,
        TerminalReason::GuestLeft
    );
    {
        let survivor = session.node(&fixture::guest_peer_id(1)).unwrap();
        assert_eq!(survivor.state.phase, protocol::LifecyclePhase::Assigned);
        assert!(!survivor.state.peers[0].ready);
    }

    // The remaining humans can reconfigure and start a bot-filled session.
    let reseated = coordinator::plan_assignments(
        &fixture::manifest(None),
        &[HOST.to_string(), fixture::guest_peer_id(1)],
    )
    .unwrap();
    session.send(
        HOST,
        Event::AssignSlots {
            assignments: reseated,
            preserve_claims: false,
        },
    );
    session.pump();
    session.send(HOST, Event::SetReady { ready: true });
    session.send(&fixture::guest_peer_id(1), Event::SetReady { ready: true });
    session.pump();
    assert_eq!(session.host().state.phase, protocol::LifecyclePhase::Ready);
}

#[test]
fn ends_the_session_when_a_frozen_guest_departs() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(2),
        ..Default::default()
    });
    session.reach_start(Some(1), Some(0));
    session.send(&fixture::guest_peer_id(1), Event::Leave);
    session.pump();
    let host_terminal = session.host().terminal.clone().unwrap();
    assert_eq!(host_terminal.reason, TerminalReason::GuestLeft);
    assert_eq!(host_terminal.code.as_deref(), Some("peer_disconnect"));
    assert_eq!(
        session
            .node(&fixture::guest_peer_id(2))
            .unwrap()
            .terminal
            .as_ref()
            .unwrap()
            .reason,
        TerminalReason::PeerAbort
    );
}

#[test]
fn ends_the_session_when_a_frozen_guest_link_drops() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(2),
        ..Default::default()
    });
    session.reach_start(Some(1), Some(0));
    session.drop_link(&fixture::guest_peer_id(2), Some("transport_lost"));
    assert_eq!(
        session.host().terminal.as_ref().unwrap().reason,
        TerminalReason::TransportLost
    );
    assert_eq!(
        session
            .node(&fixture::guest_peer_id(2))
            .unwrap()
            .terminal
            .as_ref()
            .unwrap()
            .reason,
        TerminalReason::TransportLost
    );
}

#[test]
fn gives_a_guest_a_stable_reason_when_the_host_disappears() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(1),
        ..Default::default()
    });
    session.reach_start(Some(1), Some(0));
    let guest = fixture::guest_peer_id(1);
    session.send(
        &guest,
        Event::LinkLost {
            link_id: fixture::link_id(&guest),
            code: Some("host_left".to_string()),
        },
    );
    let terminal = session.node(&guest).unwrap().terminal.clone().unwrap();
    assert_eq!(terminal.reason, TerminalReason::HostLeft);
    assert_eq!(terminal.origin, Origin::Remote);
}

#[test]
fn aborts_when_a_peer_never_acknowledges_the_start_boundary() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(2),
        ..Default::default()
    });
    session.connect_all();
    session.send(
        HOST,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    );
    session.pump();
    session.send(
        HOST,
        Event::AssignSlots {
            assignments: fixture::assignments(2, None),
            preserve_claims: false,
        },
    );
    session.pump();
    let peer_ids: Vec<String> = session.nodes.iter().map(|n| n.peer_id.clone()).collect();
    for peer_id in &peer_ids {
        session.send(peer_id, Event::SetReady { ready: true });
    }
    session.pump();
    session.send(
        HOST,
        Event::BeginCountdown {
            countdown_id: fixture::COUNTDOWN_ID.to_string(),
            remaining_ticks: 1,
            first_input_tick: 0,
        },
    );
    session.pump();
    // Silence one guest's uplink before the start boundary is published.
    let silenced_link = fixture::link_id(&fixture::guest_peer_id(2));
    let link_index = session
        .links
        .iter()
        .position(|l| l.id == silenced_link)
        .unwrap();
    session.links[link_index].guest_open = false;
    session.tick(Some(coordinator::START_ACK_TIMEOUT_TICKS + 3));

    let terminal = session.host().terminal.clone().unwrap();
    assert_eq!(terminal.reason, TerminalReason::StartAckTimeout);
    assert_eq!(terminal.origin, Origin::Timeout);
    assert_eq!(
        terminal.peer_id.as_deref(),
        Some(fixture::guest_peer_id(2).as_str())
    );
    assert!(!session.host().started);
}

#[test]
fn ends_the_session_on_a_persistent_boundary_hash_mismatch() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(1),
        ..Default::default()
    });
    session.reach_start(Some(1), Some(0));
    let guest = fixture::guest_peer_id(1);
    for index in 1..=coordinator::MAX_HASH_MISMATCHES {
        let tick = index * 60;
        session.send(
            HOST,
            Event::HashReport {
                tick,
                boundary_hash: "0123456789abcdef".to_string(),
            },
        );
        session.send(
            &guest,
            Event::HashReport {
                tick,
                boundary_hash: "ffffffffffffffff".to_string(),
            },
        );
        session.pump();
    }
    let terminal = session.host().terminal.clone().unwrap();
    assert_eq!(terminal.reason, TerminalReason::HashMismatch);
    assert_eq!(terminal.code.as_deref(), Some("desync"));
    assert_eq!(terminal.peer_id.as_deref(), Some(guest.as_str()));
    // Detection is symmetric: the guest reaches the same verdict locally.
    assert_eq!(
        session
            .node(&guest)
            .unwrap()
            .terminal
            .as_ref()
            .unwrap()
            .reason,
        TerminalReason::HashMismatch
    );
}

#[test]
fn tolerates_a_single_boundary_hash_disagreement() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(1),
        ..Default::default()
    });
    session.reach_start(Some(1), Some(0));
    let guest = fixture::guest_peer_id(1);
    session.send(
        HOST,
        Event::HashReport {
            tick: 60,
            boundary_hash: "0123456789abcdef".to_string(),
        },
    );
    session.send(
        &guest,
        Event::HashReport {
            tick: 60,
            boundary_hash: "ffffffffffffffff".to_string(),
        },
    );
    session.pump();
    assert_eq!(session.host().terminal, None);
    assert_eq!(session.host().state.peers[1].hash_mismatches, 1);
    session.send(
        HOST,
        Event::HashReport {
            tick: 120,
            boundary_hash: "0123456789abcdef".to_string(),
        },
    );
    session.send(
        &guest,
        Event::HashReport {
            tick: 120,
            boundary_hash: "0123456789abcdef".to_string(),
        },
    );
    session.pump();
    assert_eq!(session.host().state.peers[1].hash_mismatches, 0);
}

#[test]
fn turns_netcode_terminal_failures_into_stable_session_reasons() {
    let cases = [
        (
            "input_channel",
            TerminalReason::InputChannelFailure,
            "peer_disconnect",
        ),
        ("late_input", TerminalReason::LateInput, "desync"),
        ("desync", TerminalReason::HashMismatch, "desync"),
    ];
    for (failure, reason, code) in cases {
        let mut session = Driver::new(driver::Options {
            guest_count: Some(1),
            ..Default::default()
        });
        session.reach_start(Some(1), Some(0));
        session.send(
            HOST,
            Event::NetcodeFailure {
                failure: failure.to_string(),
                peer_id: None,
                detail: None,
            },
        );
        session.pump();
        let terminal = session.host().terminal.clone().unwrap();
        assert_eq!(terminal.reason, reason, "failure {failure}");
        assert_eq!(terminal.code.as_deref(), Some(code), "failure {failure}");
        assert_eq!(
            session
                .node(&fixture::guest_peer_id(1))
                .unwrap()
                .terminal
                .as_ref()
                .unwrap()
                .reason,
            TerminalReason::PeerAbort
        );
    }
}

#[test]
fn aborts_every_peer_when_the_host_quits() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(3),
        ..Default::default()
    });
    session.reach_start(Some(1), Some(0));
    session.send(
        HOST,
        Event::Abort {
            code: Some("host_abort".to_string()),
            detail: None,
        },
    );
    session.pump();
    assert_eq!(
        session.host().terminal.as_ref().unwrap().reason,
        TerminalReason::LocalAbort
    );
    for index in 1..=3 {
        let guest = session.node(&fixture::guest_peer_id(index)).unwrap();
        let terminal = guest.terminal.as_ref().unwrap();
        assert_eq!(terminal.reason, TerminalReason::PeerAbort);
        assert_eq!(terminal.code.as_deref(), Some("host_abort"));
    }
}

#[test]
fn survives_duplicated_and_delayed_control_delivery() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(2),
        latency_ticks: Some(2),
        ..Default::default()
    });
    session.connect_all();
    session.tick(Some(3));
    session.send(
        HOST,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    );
    session.tick(Some(3));
    // Replay every queued wire once more; a reliable channel may retransmit.
    let replay = session.queue.clone();
    session.queue.extend(replay);
    session.tick(Some(4));
    assert_eq!(session.host().terminal, None);
    assert_eq!(session.host().state.peers.len(), 3);
    for index in 1..=2 {
        assert_eq!(
            session.host().state.peers[index].accepted_manifest_id,
            session.host().state.manifest_id
        );
    }
}

#[test]
fn survives_a_retransmission_that_aged_out_of_the_duplicate_window() {
    // A reliable transport may retransmit arbitrarily late. Ending a synced
    // match because the wire no longer fits the retention window would be a
    // self-inflicted failure, so an unprovable duplicate is dropped.
    let mut session = Driver::new(driver::Options {
        guest_count: Some(1),
        ..Default::default()
    });
    session.reach_start(Some(1), Some(0));
    let guest = fixture::guest_peer_id(1);
    let aged_wire = session.first_wire(protocol::MessageKind::ManifestProposal);

    for index in 1..=6 {
        let tick = index * 60;
        session.send(
            HOST,
            Event::HashReport {
                tick,
                boundary_hash: "0123456789abcdef".to_string(),
            },
        );
        session.send(
            &guest,
            Event::HashReport {
                tick,
                boundary_hash: "0123456789abcdef".to_string(),
            },
        );
        session.pump();
    }
    {
        let node = session.node(&guest).unwrap();
        assert_eq!(node.terminal, None);
        assert!(
            node.state.peers[1].window.len() as i64 >= coordinator::DUPLICATE_WINDOW as i64,
            "the window must be saturated for this to be a real eviction"
        );
    }

    session.replay(HOST, &guest, &aged_wire);
    assert_eq!(session.node(&guest).unwrap().terminal, None);
    assert_eq!(
        session.node(&guest).unwrap().state.phase,
        protocol::LifecyclePhase::Running
    );
    assert_eq!(session.host().terminal, None);

    session.play_out(Some(1), Some(0));
    assert!(session.all_terminal(Some(TerminalReason::Completed)));
}

#[test]
fn refuses_readiness_that_predates_the_current_slot_assignment() {
    // Latency holds one readiness in flight across a republished
    // assignment. Owning *a* slot under the new generation is not evidence
    // that the peer ever saw that generation.
    let mut session = Driver::new(driver::Options {
        guest_count: Some(1),
        latency_ticks: Some(4),
        ..Default::default()
    });
    let guest = fixture::guest_peer_id(1);
    session.connect_all();
    session.tick(Some(6));
    session.send(
        HOST,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    );
    session.tick(Some(12));
    session.send(
        HOST,
        Event::AssignSlots {
            assignments: fixture::assignments(1, None),
            preserve_claims: false,
        },
    );
    session.tick(Some(6));
    assert_eq!(session.host().state.assignment_epoch, 1);

    session.send(&guest, Event::SetReady { ready: true });
    assert!(
        !session.host().state.peers[1].ready,
        "the readiness is still in flight"
    );

    let assignments = fixture::assignments(1, None);
    let swapped = with_producer_id(&with_producer_id(&assignments, 1, &guest), 2, HOST);
    session.send(
        HOST,
        Event::AssignSlots {
            assignments: swapped,
            preserve_claims: false,
        },
    );
    session.send(HOST, Event::SetReady { ready: true });
    session.tick(Some(6));

    {
        let host = session.host();
        assert_eq!(host.terminal, None);
        assert!(
            !host.state.peers[1].ready,
            "stale readiness must not satisfy the barrier"
        );
        assert_eq!(host.state.phase, protocol::LifecyclePhase::Assigned);
        assert_eq!(host.state.freeze, None);
    }

    let (_, outcome) = coordinator::step(
        &session.host().state,
        Event::BeginCountdown {
            countdown_id: fixture::COUNTDOWN_ID.to_string(),
            remaining_ticks: 1,
            first_input_tick: 0,
        },
    );
    assert_eq!(outcome.code, Some(coordinator::RejectCode::InvalidPhase));

    session.send(&guest, Event::SetReady { ready: true });
    session.tick(Some(6));
    assert!(session.host().state.peers[1].ready);
    assert_eq!(session.host().state.phase, protocol::LifecyclePhase::Ready);
}

#[test]
fn refuses_stale_readiness_across_two_republishes_in_flight() {
    // The interleaving that defeats every ordering-based scheme: a guest's
    // honest readiness for S0 is still in flight while the host advances
    // S0 -> S1 -> S2. No S0-era message can be credited to a generation the
    // guest never saw, because the generation is named on the wire.
    let mut session = Driver::new(driver::Options {
        guest_count: Some(1),
        latency_ticks: Some(6),
        ..Default::default()
    });
    let guest = fixture::guest_peer_id(1);
    session.connect_all();
    session.tick(Some(8));
    session.send(
        HOST,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    );
    session.tick(Some(16));
    session.send(
        HOST,
        Event::AssignSlots {
            assignments: fixture::assignments(1, None),
            preserve_claims: false,
        },
    );
    session.tick(Some(8));
    assert_eq!(session.host().state.assignment_epoch, 1);
    assert_eq!(
        session.node(&guest).unwrap().state.assignment_id,
        session.host().state.assignment_id
    );

    session.send(&guest, Event::SetReady { ready: true });
    assert!(
        !session.host().state.peers[1].ready,
        "the S0 readiness is still in flight"
    );

    let assignments = fixture::assignments(1, None);
    let swapped = with_producer_id(&with_producer_id(&assignments, 1, &guest), 2, HOST);
    session.send(
        HOST,
        Event::AssignSlots {
            assignments: swapped,
            preserve_claims: false,
        },
    );
    session.send(
        HOST,
        Event::AssignSlots {
            assignments: fixture::assignments(1, None),
            preserve_claims: false,
        },
    );
    session.send(HOST, Event::SetReady { ready: true });
    session.tick(Some(10));

    {
        let host = session.host();
        assert_eq!(host.terminal, None);
        assert!(
            !host.state.peers[1].ready,
            "no S0-era message may satisfy the S2 barrier"
        );
        assert_eq!(host.state.phase, protocol::LifecyclePhase::Assigned);
        assert_eq!(host.state.freeze, None);
        assert_eq!(host.state.assignment_epoch, 3);
    }

    let (frozen, outcome) = coordinator::step(
        &session.host().state,
        Event::BeginCountdown {
            countdown_id: fixture::COUNTDOWN_ID.to_string(),
            remaining_ticks: 1,
            first_input_tick: 0,
        },
    );
    assert_eq!(outcome.code, Some(coordinator::RejectCode::InvalidPhase));
    assert_eq!(frozen.freeze, None);

    session.send(&guest, Event::SetReady { ready: true });
    session.tick(Some(10));
    assert!(session.host().state.peers[1].ready);
    assert_eq!(session.host().state.phase, protocol::LifecyclePhase::Ready);
}

#[test]
fn mints_a_distinct_generation_even_for_byte_identical_ownership() {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(1),
        ..Default::default()
    });
    session.connect_all();
    session.send(
        HOST,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    );
    session.pump();
    session.send(
        HOST,
        Event::AssignSlots {
            assignments: fixture::assignments(1, None),
            preserve_claims: false,
        },
    );
    session.pump();
    let first = session.host().state.assignment_id.clone().unwrap();

    // A different generation, then the original ownership restored: the
    // identity must not repeat, or readiness for the first could satisfy
    // the third.
    let guest = fixture::guest_peer_id(1);
    let assignments = fixture::assignments(1, None);
    let swapped = with_producer_id(&with_producer_id(&assignments, 1, &guest), 2, HOST);
    session.send(
        HOST,
        Event::AssignSlots {
            assignments: swapped,
            preserve_claims: false,
        },
    );
    session.pump();
    session.send(
        HOST,
        Event::AssignSlots {
            assignments: fixture::assignments(1, None),
            preserve_claims: false,
        },
    );
    session.pump();
    let third = session.host().state.assignment_id.clone().unwrap();

    assert_ne!(third, first, "restored ownership is still a new generation");
    assert_eq!(session.host().state.assignment_epoch, 3);
    assert_eq!(
        session.node(&guest).unwrap().state.assignment_id,
        Some(third)
    );
}

// ---------------------------------------------------------------------------
// "online coordinator adversarial peers"
// ---------------------------------------------------------------------------

/// Build a driven session paused at `stage`: `"handshake"`, `"manifest"`,
/// `"assigned"`, `"ready"`, `"countdown"`, or anything else for the running
/// phase — mirrors the Lua spec's local `staged` helper.
fn staged(stage: &str) -> Driver {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(1),
        ..Default::default()
    });
    session.connect_all();
    if stage == "handshake" {
        return session;
    }
    session.send(
        HOST,
        Event::ProposeManifest {
            manifest: fixture::manifest(None),
        },
    );
    session.pump();
    if stage == "manifest" {
        return session;
    }
    session.send(
        HOST,
        Event::AssignSlots {
            assignments: fixture::assignments(1, None),
            preserve_claims: false,
        },
    );
    session.pump();
    if stage == "assigned" {
        return session;
    }
    let peer_ids: Vec<String> = session.nodes.iter().map(|n| n.peer_id.clone()).collect();
    for peer_id in &peer_ids {
        session.send(peer_id, Event::SetReady { ready: true });
    }
    session.pump();
    if stage == "ready" {
        return session;
    }
    session.send(
        HOST,
        Event::BeginCountdown {
            countdown_id: fixture::COUNTDOWN_ID.to_string(),
            remaining_ticks: 2,
            first_input_tick: 0,
        },
    );
    session.pump();
    if stage == "countdown" {
        return session;
    }
    session.tick(Some(3));
    session
}

/// The terminal of `peer_id`, panicking (with `peer_id` in the message) if
/// the node is unknown or never ended — mirrors the Lua spec's
/// `terminal_of` helper's assertion messages.
fn terminal_of(session: &Driver, peer_id: &str) -> Terminal {
    session
        .node(peer_id)
        .unwrap_or_else(|| panic!("{peer_id} is not an admitted driver node"))
        .terminal
        .clone()
        .unwrap_or_else(|| panic!("{peer_id} never ended"))
}

#[test]
fn drops_a_guest_that_aborts_before_the_countdown() {
    let mut session = staged("ready");
    let guest = fixture::guest_peer_id(1);
    session.inject(
        &guest,
        HOST,
        protocol::MessageKind::Abort,
        Value::record(vec![("code", Value::str("host_abort"))]),
        None,
    );
    let host = session.host();
    assert_eq!(
        host.terminal, None,
        "a pre-freeze guest abort is a departure, not a failure"
    );
    assert_eq!(host.state.peers.len(), 1);
    assert_eq!(host.state.assignments, None);
    assert!(!host.state.peers[0].ready);
    assert_eq!(
        terminal_of(&session, &guest).reason,
        TerminalReason::Removed
    );
}

#[test]
fn refuses_a_second_handshake_from_an_admitted_link() {
    let mut session = staged("handshake");
    let guest = fixture::guest_peer_id(1);
    session.inject(
        &guest,
        HOST,
        protocol::MessageKind::Handshake,
        Value::record(vec![
            ("role", Value::str("guest")),
            ("runtime", fixture::runtime()),
        ]),
        None,
    );
    assert_eq!(
        terminal_of(&session, HOST).detail.as_deref(),
        Some("an admitted link cannot handshake again")
    );
    assert_eq!(
        terminal_of(&session, HOST).reason,
        TerminalReason::ProtocolViolation
    );
}

#[test]
fn refuses_control_traffic_that_names_another_session() {
    let mut session = staged("assigned");
    let guest = fixture::guest_peer_id(1);
    let manifest_id = session.host().state.manifest_id.clone().unwrap();
    session.inject(
        &guest,
        HOST,
        protocol::MessageKind::ManifestAccept,
        Value::record(vec![("manifest_id", Value::str(manifest_id))]),
        Some("other_session"),
    );
    assert_eq!(
        terminal_of(&session, HOST).detail.as_deref(),
        Some("control message names a different session")
    );
}

#[test]
fn refuses_a_guest_that_disconnects_another_peer() {
    let mut session = staged("assigned");
    let guest = fixture::guest_peer_id(1);
    session.inject(
        &guest,
        HOST,
        protocol::MessageKind::Disconnect,
        Value::record(vec![
            ("target_peer_id", Value::str("guest.9")),
            ("code", Value::str("peer_left")),
        ]),
        None,
    );
    assert_eq!(
        terminal_of(&session, HOST).detail.as_deref(),
        Some("a guest cannot disconnect another peer")
    );
}

#[test]
fn refuses_readiness_for_a_manifest_the_session_never_proposed() {
    let mut session = staged("assigned");
    let guest = fixture::guest_peer_id(1);
    let assignment_id = session.host().state.assignment_id.clone().unwrap();
    session.inject(
        &guest,
        HOST,
        protocol::MessageKind::Ready,
        Value::record(vec![
            ("manifest_id", Value::str("0123456789abcdef")),
            ("assignment_id", Value::str(assignment_id)),
            ("ready", Value::bool(true)),
        ]),
        None,
    );
    assert_eq!(
        terminal_of(&session, HOST).reason,
        TerminalReason::ManifestMismatch
    );
    assert_eq!(
        terminal_of(&session, HOST).detail.as_deref(),
        Some("readiness names a different manifest")
    );
}

#[test]
fn refuses_a_guest_sent_message_that_only_the_host_may_send() {
    let mut session = staged("assigned");
    let guest = fixture::guest_peer_id(1);
    let manifest_id = session.host().state.manifest_id.clone().unwrap();
    session.inject(
        &guest,
        HOST,
        protocol::MessageKind::Countdown,
        Value::record(vec![
            ("manifest_id", Value::str(manifest_id)),
            ("countdown_id", Value::str(fixture::COUNTDOWN_ID)),
            ("remaining_ticks", Value::int(1)),
            ("first_input_tick", Value::int(0)),
        ]),
        None,
    );
    assert_eq!(
        terminal_of(&session, HOST).detail.as_deref(),
        Some("countdown may only be sent by the host")
    );
}

#[test]
fn gives_a_guest_a_stable_reason_for_every_misbehaving_host_message() {
    let guest = fixture::guest_peer_id(1);
    let manifest_id = protocol::manifest_id(&fixture::manifest(None));
    let mut other = fixture::manifest(None);
    let seed = other.get("seed").and_then(Value::as_int).unwrap();
    other.set("seed", Value::int(seed + 1));

    struct Case {
        stage: &'static str,
        kind: protocol::MessageKind,
        body: Value,
        detail: &'static str,
    }

    let cases = vec![
        Case {
            stage: "manifest",
            kind: protocol::MessageKind::ManifestProposal,
            body: Value::record(vec![
                ("manifest_id", Value::str(protocol::manifest_id(&other))),
                ("manifest", other),
            ]),
            detail: "the manifest is immutable after proposal",
        },
        Case {
            stage: "assigned",
            kind: protocol::MessageKind::PeerAssignment,
            body: Value::record(vec![
                ("assigned_peer_id", Value::str("guest.9")),
                ("role", Value::str("guest")),
            ]),
            detail: "the host named a different peer identity",
        },
        Case {
            stage: "assigned",
            kind: protocol::MessageKind::SlotAssignment,
            body: Value::record(vec![
                ("manifest_id", Value::str("0123456789abcdef")),
                (
                    "assignment_id",
                    Value::str(protocol::assignment_id(&fixture::assignments(1, None), 9)),
                ),
                ("assignments", fixture::assignments(1, None)),
            ]),
            detail: "slot assignment names a different manifest",
        },
        Case {
            stage: "ready",
            kind: protocol::MessageKind::Countdown,
            body: Value::record(vec![
                ("manifest_id", Value::str("0123456789abcdef")),
                ("countdown_id", Value::str("countdown.2")),
                ("remaining_ticks", Value::int(1)),
                ("first_input_tick", Value::int(0)),
            ]),
            detail: "countdown names a different manifest",
        },
        Case {
            stage: "countdown",
            kind: protocol::MessageKind::Countdown,
            body: Value::record(vec![
                ("manifest_id", Value::str(manifest_id.clone())),
                ("countdown_id", Value::str("countdown.2")),
                ("remaining_ticks", Value::int(1)),
                ("first_input_tick", Value::int(0)),
            ]),
            detail: "a frozen countdown cannot be restarted",
        },
        Case {
            stage: "countdown",
            kind: protocol::MessageKind::Start,
            body: Value::record(vec![
                ("manifest_id", Value::str(manifest_id)),
                ("countdown_id", Value::str(fixture::COUNTDOWN_ID)),
                ("first_input_tick", Value::int(99)),
            ]),
            detail: "start does not name the frozen countdown boundary",
        },
        Case {
            stage: "running",
            kind: protocol::MessageKind::MatchPhase,
            body: Value::record(vec![
                ("phase", Value::str("goal_stoppage")),
                ("tick", Value::int(5)),
                ("home_score", Value::int(1)),
                ("away_score", Value::int(0)),
            ]),
            detail: "a running match opens with kickoff",
        },
    ];

    for case in cases {
        let mut session = staged(case.stage);
        session.inject(HOST, &guest, case.kind, case.body, None);
        let terminal = terminal_of(&session, &guest);
        assert_eq!(
            terminal.detail.as_deref(),
            Some(case.detail),
            "{}/{:?}",
            case.stage,
            case.kind
        );
        assert_eq!(terminal.origin, Origin::Remote);
    }
}

#[test]
fn refuses_out_of_order_running_phases_from_the_host() {
    let mut session = staged("running");
    let guest = fixture::guest_peer_id(1);
    session.send(
        HOST,
        Event::MatchPhase {
            phase: "kickoff".to_string(),
            tick: 0,
            home_score: 0,
            away_score: 0,
        },
    );
    session.pump();
    assert_eq!(session.node(&guest).unwrap().terminal, None);
    session.inject(
        HOST,
        &guest,
        protocol::MessageKind::MatchPhase,
        Value::record(vec![
            ("phase", Value::str("goal_stoppage")),
            ("tick", Value::int(30)),
            ("home_score", Value::int(1)),
            ("away_score", Value::int(0)),
        ]),
        None,
    );
    assert_eq!(
        terminal_of(&session, &guest).detail.as_deref(),
        Some("match phase ordering regressed")
    );
}

#[test]
fn accepts_a_repeated_peer_assignment_and_proposal_as_no_ops() {
    let mut session = staged("manifest");
    let guest = fixture::guest_peer_id(1);
    session.inject(
        HOST,
        &guest,
        protocol::MessageKind::PeerAssignment,
        Value::record(vec![
            ("assigned_peer_id", Value::str(guest.clone())),
            ("role", Value::str("guest")),
        ]),
        None,
    );
    assert_eq!(session.node(&guest).unwrap().terminal, None);
    assert_eq!(
        session.node(&guest).unwrap().state.phase,
        protocol::LifecyclePhase::Manifest
    );
    session.inject(
        HOST,
        &guest,
        protocol::MessageKind::ManifestProposal,
        Value::record(vec![
            (
                "manifest_id",
                Value::str(protocol::manifest_id(&fixture::manifest(None))),
            ),
            ("manifest", fixture::manifest(None)),
        ]),
        None,
    );
    assert_eq!(
        session.node(&guest).unwrap().terminal,
        None,
        "a repeated proposal is idempotent"
    );
    assert_eq!(
        session.node(&guest).unwrap().state.phase,
        protocol::LifecyclePhase::Manifest
    );
}

// ---------------------------------------------------------------------------
// "online coordinator conformance"
// ---------------------------------------------------------------------------

#[test]
fn matches_the_pinned_canonical_session_goldens() {
    let report = gc_netcode::coordinator_conformance::verify();
    assert_eq!(report.full_transcript_id.len(), 16);
    assert!(report.message_count > 0);
    assert!(
        gc_netcode::coordinator_conformance::marker(&report).starts_with("GC_COORDINATOR|golden|")
    );
}
