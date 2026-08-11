//! An in-process transport: N [`coordinator::CoordinatorState`]s (one host,
//! zero or more guests) wired together by fake links, with a fake clock for
//! latency. Every coordinator is exercised exactly as production code would:
//! [`coordinator::step`]/[`coordinator::receive`] with real encoded wires,
//! never a test-only entry point.

use crate::coordinator::{
    self, CoordinatorState, ManifestExpectation, Role, Terminal, TerminalReason,
};
use crate::coordinator_fixture as fixture;
use crate::protocol::{self, Value};

/// Bound on delivery rounds one [`Driver::pump`] call may take to settle.
pub const MAX_PUMP_ROUNDS: i64 = 64;

/// One driver-side peer: its coordinator state plus bookkeeping the driver
/// itself needs (which the coordinator does not expose).
#[derive(Clone, Debug, PartialEq)]
pub struct DriverNode {
    /// This peer's id.
    pub peer_id: String,
    /// Host or guest.
    pub role: Role,
    /// This peer's own coordinator.
    pub state: CoordinatorState,
    /// The single host<->guest link this node owns (empty for the host,
    /// which owns one link per guest instead — see [`Driver::links`]).
    pub link_id: String,
    /// Whether this node has reached the running phase.
    pub started: bool,
    /// The first input tick this node's session was frozen with, once it
    /// has started.
    pub first_input_tick: Option<i64>,
    /// This node's terminal reason, once the session has ended for it.
    pub terminal: Option<Terminal>,
}

/// One fake host<->guest transport link.
#[derive(Clone, Debug, PartialEq)]
pub struct DriverLink {
    /// This link's id.
    pub id: String,
    /// The guest peer id on the other end.
    pub guest_peer_id: String,
    /// Whether the host's own end is open.
    pub host_open: bool,
    /// Whether the guest's own end is open.
    pub guest_open: bool,
}

/// One packet in flight on the fake clock.
#[derive(Clone, Debug, PartialEq)]
pub struct DriverPacket {
    /// The link this packet travels on.
    pub link_id: String,
    /// Whether this packet travels guest -> host (`false` is host -> guest).
    pub to_host: bool,
    /// The canonical encoded wire.
    pub wire: String,
    /// The fake-clock tick this packet becomes deliverable at.
    pub deliver_at: i64,
}

/// Inputs to [`Driver::new`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    /// Overrides the fixture session id.
    pub session_id: Option<String>,
    /// How many guests to seat; defaults to 0.
    pub guest_count: Option<i64>,
    /// Fake-clock ticks a packet takes to arrive; defaults to 0.
    pub latency_ticks: Option<i64>,
    /// Overrides every guest's manifest expectation.
    pub expectation: Option<ManifestExpectation>,
    /// Match mode; defaults to 4v4.
    pub mode: Option<protocol::MatchMode>,
}

/// An in-process driven coordinator session: one host, zero or more guests,
/// wired together by fake links and a fake clock.
#[derive(Clone, Debug, PartialEq)]
pub struct Driver {
    /// The fake clock, advanced by [`Driver::tick`].
    pub clock: i64,
    /// Fake-clock ticks a packet takes to arrive.
    pub latency_ticks: i64,
    /// The session identity every node shares.
    pub session_id: String,
    /// The match mode every node was constructed with.
    pub mode: protocol::MatchMode,
    /// Host first, then guests in admission order.
    pub nodes: Vec<DriverNode>,
    /// One entry per guest link.
    pub links: Vec<DriverLink>,
    /// Packets in flight.
    pub queue: Vec<DriverPacket>,
    /// Every message any node has sent, in send order.
    pub transcript: Vec<protocol::ControlMessage>,
    /// A flat `"sender>receiver kind"` trace, for golden-transcript tests.
    pub log: Vec<String>,
    /// Total packets delivered.
    pub delivered: i64,
    /// Total packets dropped (closed link, or a terminal target).
    pub dropped: i64,
}

impl Driver {
    /// Construct a new driven session.
    ///
    /// # Panics
    ///
    /// Panics if `options.guest_count` exceeds the match mode's seatable
    /// human count minus the host, or if any fixture coordinator fails to
    /// construct — both programmer errors in the test itself.
    #[must_use]
    pub fn new(options: Options) -> Driver {
        let guest_count = options.guest_count.unwrap_or(0);
        let mode = options.mode.unwrap_or(protocol::MatchMode::FourVFour);
        let shape = protocol::match_mode_shape(mode);
        assert!(
            (0..=shape.humans - 1).contains(&guest_count),
            "a direct-host {} session seats at most {} guests",
            shape.mode.wire_str(),
            shape.humans - 1
        );
        let manifest = fixture::manifest(Some(mode));
        let session_id = options.session_id.unwrap_or_else(|| {
            manifest
                .get("session_id")
                .and_then(Value::as_str)
                .expect("fixture manifest has a session id")
                .to_string()
        });
        let mut nodes = vec![DriverNode {
            peer_id: fixture::HOST_PEER_ID.to_string(),
            role: Role::Host,
            state: fixture::host(Some(&session_id)),
            link_id: String::new(),
            started: false,
            first_input_tick: None,
            terminal: None,
        }];
        let mut links = Vec::new();
        for index in 1..=guest_count {
            let state = fixture::guest(index, Some(&session_id), options.expectation.clone());
            let peer_id = state.peer_id.clone();
            let host_link_id = state
                .host_link_id
                .clone()
                .expect("guest coordinator has a host link id");
            nodes.push(DriverNode {
                peer_id: peer_id.clone(),
                role: Role::Guest,
                state,
                link_id: host_link_id.clone(),
                started: false,
                first_input_tick: None,
                terminal: None,
            });
            links.push(DriverLink {
                id: host_link_id,
                guest_peer_id: peer_id,
                host_open: true,
                guest_open: true,
            });
        }
        Driver {
            clock: 0,
            latency_ticks: options.latency_ticks.unwrap_or(0),
            session_id,
            mode,
            nodes,
            links,
            queue: Vec::new(),
            transcript: Vec::new(),
            log: Vec::new(),
            delivered: 0,
            dropped: 0,
        }
    }

    fn node_index(&self, peer_id: &str) -> Option<usize> {
        self.nodes.iter().position(|node| node.peer_id == peer_id)
    }

    fn link_index(&self, link_id: &str) -> Option<usize> {
        self.links.iter().position(|link| link.id == link_id)
    }

    /// The node with `peer_id`, if admitted.
    #[must_use]
    pub fn node(&self, peer_id: &str) -> Option<&DriverNode> {
        self.nodes.iter().find(|node| node.peer_id == peer_id)
    }

    /// The host node (always `nodes[0]`).
    #[must_use]
    pub fn host(&self) -> &DriverNode {
        &self.nodes[0]
    }

    /// The link with `link_id`, if any.
    #[must_use]
    pub fn link(&self, link_id: &str) -> Option<&DriverLink> {
        self.links.iter().find(|link| link.id == link_id)
    }

    fn enqueue(&mut self, node_index: usize, link_id: &str, message: &protocol::ControlMessage) {
        let to_host = self.nodes[node_index].role == Role::Guest;
        let Some(link_index) = self.link_index(link_id) else {
            self.dropped += 1;
            return;
        };
        let sender_open = if to_host {
            self.links[link_index].guest_open
        } else {
            self.links[link_index].host_open
        };
        if !sender_open {
            self.dropped += 1;
            return;
        }
        let wire = protocol::encode(message).expect("driver only sends already-validated messages");
        let sender = self.nodes[node_index].peer_id.clone();
        let target = if to_host {
            fixture::HOST_PEER_ID.to_string()
        } else {
            self.links[link_index].guest_peer_id.clone()
        };
        self.log
            .push(format!("{sender}>{target} {}", message.kind.wire_str()));
        self.queue.push(DriverPacket {
            link_id: link_id.to_string(),
            to_host,
            wire,
            deliver_at: self.clock + self.latency_ticks,
        });
    }

    fn apply(&mut self, node_index: usize, actions: Vec<coordinator::Action>) {
        for action in actions {
            match action {
                coordinator::Action::Send { targets, message } => {
                    // One emitted message is one transcript entry, however
                    // many links it fans out to, so per-peer sequences stay
                    // strictly monotonic.
                    self.transcript.push(message.clone());
                    for link_id in &targets {
                        self.enqueue(node_index, link_id, &message);
                    }
                }
                coordinator::Action::Close { link_id } => {
                    if let Some(link_index) = self.link_index(&link_id) {
                        if self.nodes[node_index].role == Role::Host {
                            self.links[link_index].host_open = false;
                        } else {
                            self.links[link_index].guest_open = false;
                        }
                    }
                }
                coordinator::Action::StartMatch { freeze } => {
                    self.nodes[node_index].started = true;
                    self.nodes[node_index].first_input_tick = Some(freeze.first_input_tick);
                }
                coordinator::Action::Terminate { terminal } => {
                    self.nodes[node_index].terminal = Some(terminal);
                }
            }
        }
    }

    /// Drive one event through one node's coordinator.
    ///
    /// # Panics
    ///
    /// Panics if `peer_id` names no admitted node.
    pub fn send(&mut self, peer_id: &str, event: coordinator::Event) -> coordinator::Outcome {
        let node_index = self
            .node_index(peer_id)
            .unwrap_or_else(|| panic!("unknown driver node {peer_id}"));
        let (state, outcome) = coordinator::step(&self.nodes[node_index].state, event);
        self.nodes[node_index].state = state;
        self.apply(node_index, outcome.actions.clone());
        outcome
    }

    /// Deliver every packet whose fake-clock deadline has passed, including
    /// packets produced by the deliveries themselves, so a pump reaches a
    /// stable state.
    ///
    /// # Panics
    ///
    /// Panics if the queue does not settle within [`MAX_PUMP_ROUNDS`].
    pub fn pump(&mut self) -> i64 {
        let mut delivered = 0;
        for _ in 0..MAX_PUMP_ROUNDS {
            let mut due = Vec::new();
            let mut pending = Vec::new();
            for packet in self.queue.drain(..) {
                if packet.deliver_at <= self.clock {
                    due.push(packet);
                } else {
                    pending.push(packet);
                }
            }
            self.queue = pending;
            if due.is_empty() {
                return delivered;
            }
            for packet in due {
                let link_index = self
                    .link_index(&packet.link_id)
                    .expect("packet names a known link");
                let target_peer = if packet.to_host {
                    fixture::HOST_PEER_ID.to_string()
                } else {
                    self.links[link_index].guest_peer_id.clone()
                };
                let node_index = self
                    .node_index(&target_peer)
                    .expect("packet target is a known node");
                let receiver_open = if packet.to_host {
                    self.links[link_index].host_open
                } else {
                    self.links[link_index].guest_open
                };
                if receiver_open && !coordinator::is_terminal(&self.nodes[node_index].state) {
                    delivered += 1;
                    self.delivered += 1;
                    let (state, outcome) = coordinator::receive(
                        &self.nodes[node_index].state,
                        &packet.link_id,
                        &packet.wire,
                    );
                    self.nodes[node_index].state = state;
                    self.apply(node_index, outcome.actions);
                } else {
                    self.dropped += 1;
                }
            }
        }
        panic!("coordinator driver did not settle within its pump bound");
    }

    /// Advance the fake clock by `count` ticks (default 1), pumping before
    /// and after every tick so in-flight traffic and tick-triggered traffic
    /// both settle.
    pub fn tick(&mut self, count: Option<i64>) -> &mut Self {
        for _ in 0..count.unwrap_or(1) {
            self.clock += 1;
            self.pump();
            for index in 0..self.nodes.len() {
                let (state, outcome) =
                    coordinator::step(&self.nodes[index].state, coordinator::Event::Tick);
                self.nodes[index].state = state;
                self.apply(index, outcome.actions);
            }
            self.pump();
        }
        self
    }

    /// Send every guest's manual connection, then pump.
    pub fn connect_all(&mut self) -> &mut Self {
        for index in 1..self.nodes.len() {
            let peer_id = self.nodes[index].peer_id.clone();
            self.send(&peer_id, coordinator::Event::Connect);
        }
        self.pump();
        self
    }

    /// Send a control message that the sender's own coordinator would never
    /// emit, over the sender's real link and canonical wire format. This is
    /// how a misbehaving or compromised peer is modelled: the receiving
    /// coordinator is exercised exactly as in production, with no test-only
    /// entry point. The sender's counter advances so the transcript stays
    /// monotonic for both ends.
    ///
    /// # Panics
    ///
    /// Panics if `from_peer_id` names no admitted node, or if `body` does
    /// not build a valid message of `kind`.
    pub fn inject(
        &mut self,
        from_peer_id: &str,
        to_peer_id: &str,
        kind: protocol::MessageKind,
        body: Value,
        session_id: Option<&str>,
    ) -> &mut Self {
        let node_index = self
            .node_index(from_peer_id)
            .unwrap_or_else(|| panic!("unknown driver node {from_peer_id}"));
        let guest_peer_id = if self.nodes[node_index].role == Role::Host {
            to_peer_id.to_string()
        } else {
            from_peer_id.to_string()
        };
        let sid = session_id
            .map(str::to_string)
            .unwrap_or_else(|| self.session_id.clone());
        let sequence = self.nodes[node_index].state.sequence;
        let message = protocol::new(kind, &sid, from_peer_id, sequence, body)
            .expect("inject builds a valid message");
        self.nodes[node_index].state.sequence += 1;
        let link_id = fixture::link_id(&guest_peer_id);
        self.apply(
            node_index,
            vec![coordinator::Action::Send {
                targets: vec![link_id],
                message,
            }],
        );
        self.pump();
        self
    }

    /// Replay a wire that was already delivered, exactly as a reliable
    /// transport retransmitting after loss would.
    ///
    /// # Panics
    ///
    /// Panics if `from_peer_id` names no admitted node.
    pub fn replay(&mut self, from_peer_id: &str, to_peer_id: &str, wire: &str) -> &mut Self {
        let node_index = self
            .node_index(from_peer_id)
            .unwrap_or_else(|| panic!("unknown driver node {from_peer_id}"));
        let role = self.nodes[node_index].role;
        let guest_peer_id = if role == Role::Host {
            to_peer_id.to_string()
        } else {
            from_peer_id.to_string()
        };
        self.queue.push(DriverPacket {
            link_id: fixture::link_id(&guest_peer_id),
            to_host: role == Role::Guest,
            wire: wire.to_string(),
            deliver_at: self.clock,
        });
        self.pump();
        self
    }

    /// The canonical wire of the first message of `kind` this session
    /// carried.
    ///
    /// # Panics
    ///
    /// Panics if the session never carried a message of `kind`.
    #[must_use]
    pub fn first_wire(&self, kind: protocol::MessageKind) -> String {
        self.transcript
            .iter()
            .find(|message| message.kind == kind)
            .map(|message| {
                protocol::encode(message).expect("transcript messages are already valid")
            })
            .unwrap_or_else(|| panic!("the session never carried a {} message", kind.wire_str()))
    }

    /// Simulate a transport failure: both endpoints observe the loss of the
    /// link.
    ///
    /// # Panics
    ///
    /// Panics if `guest_peer_id` names no known link.
    pub fn drop_link(&mut self, guest_peer_id: &str, code: Option<&str>) -> &mut Self {
        let link_id = fixture::link_id(guest_peer_id);
        let link_index = self.link_index(&link_id).expect("known driver link");
        self.links[link_index].host_open = false;
        self.links[link_index].guest_open = false;
        if let Some(guest_index) = self.node_index(guest_peer_id)
            && !coordinator::is_terminal(&self.nodes[guest_index].state)
        {
            self.send(
                guest_peer_id,
                coordinator::Event::LinkLost {
                    link_id: link_id.clone(),
                    code: code.map(str::to_string),
                },
            );
        }
        let host_peer_id = self.nodes[0].peer_id.clone();
        if !coordinator::is_terminal(&self.nodes[0].state) {
            self.send(
                &host_peer_id,
                coordinator::Event::LinkLost {
                    link_id,
                    code: code.map(str::to_string),
                },
            );
        }
        self.pump();
        self
    }

    /// [`protocol::transcript_id`] of every message this session carried.
    #[must_use]
    pub fn transcript_id(&self) -> String {
        protocol::transcript_id(&self.transcript)
    }

    /// A flat `"sender>receiver kind;..."` trace of every message sent.
    #[must_use]
    pub fn trace(&self) -> String {
        self.log.join(";")
    }

    /// Whether every node has reached the running phase.
    #[must_use]
    pub fn all_started(&self) -> bool {
        self.nodes.iter().all(|node| node.started)
    }

    /// Whether every node has ended, optionally requiring a shared reason.
    #[must_use]
    pub fn all_terminal(&self, reason: Option<TerminalReason>) -> bool {
        self.nodes
            .iter()
            .all(|node| match (&node.terminal, reason) {
                (Some(terminal), Some(reason)) => terminal.reason == reason,
                (Some(_), None) => true,
                (None, _) => false,
            })
    }

    /// Drive a session from manual connection to the frozen start boundary.
    ///
    /// # Panics
    ///
    /// Panics if planning the fixture assignment fails — a programmer error
    /// in the fixture itself.
    pub fn reach_start(
        &mut self,
        countdown_ticks: Option<i64>,
        first_input_tick: Option<i64>,
    ) -> &mut Self {
        let manifest = fixture::manifest(Some(self.mode));
        let guest_count = self.nodes.len() as i64 - 1;
        self.connect_all();
        self.send(
            fixture::HOST_PEER_ID,
            coordinator::Event::ProposeManifest {
                manifest: manifest.clone(),
            },
        );
        self.pump();
        let assignments = coordinator::plan_assignments(&manifest, &fixture::peer_ids(guest_count))
            .expect("driver plans a valid assignment");
        self.send(
            fixture::HOST_PEER_ID,
            coordinator::Event::AssignSlots {
                assignments,
                preserve_claims: false,
            },
        );
        self.pump();
        let peer_ids: Vec<String> = self.nodes.iter().map(|node| node.peer_id.clone()).collect();
        for peer_id in &peer_ids {
            self.send(peer_id, coordinator::Event::SetReady { ready: true });
        }
        self.pump();
        let ticks = countdown_ticks.unwrap_or(3);
        self.send(
            fixture::HOST_PEER_ID,
            coordinator::Event::BeginCountdown {
                countdown_id: fixture::COUNTDOWN_ID.to_string(),
                remaining_ticks: ticks,
                first_input_tick: first_input_tick.unwrap_or(0),
            },
        );
        self.pump();
        self.tick(Some(ticks + 1));
        self
    }

    /// Acknowledge a complete simulation: kickoff, play, full time, and
    /// result.
    pub fn play_out(&mut self, home_score: Option<i64>, away_score: Option<i64>) -> &mut Self {
        let home = home_score.unwrap_or(1);
        let away = away_score.unwrap_or(0);
        let host = fixture::HOST_PEER_ID;
        self.send(
            host,
            coordinator::Event::MatchPhase {
                phase: "kickoff".to_string(),
                tick: 0,
                home_score: 0,
                away_score: 0,
            },
        );
        self.send(
            host,
            coordinator::Event::MatchPhase {
                phase: "playing".to_string(),
                tick: 1,
                home_score: 0,
                away_score: 0,
            },
        );
        self.pump();
        self.send(
            host,
            coordinator::Event::MatchPhase {
                phase: "full_time".to_string(),
                tick: 7200,
                home_score: home,
                away_score: away,
            },
        );
        self.pump();
        self.send(
            host,
            coordinator::Event::Finish {
                final_tick: 7200,
                home_score: home,
                away_score: away,
                final_hash: "fedcba9876543210".to_string(),
            },
        );
        self.pump();
        self
    }
}
