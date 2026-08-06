//! Port of `game/online/fault_harness.lua`.
//!
//! The Lua original owns one `FaultHarness`: N fully isolated clients (one
//! host plus up to seven guests) driven through the complete OMP-3 session
//! lifecycle over a real in-process star. AGENTS.md §9's closing
//! subsection — *"A harness self-test is not a harness run"* — is about the
//! exact code this file ports: a defect that broke every online match once
//! passed nine green checks because a harness printed failures and exited
//! 0 (`docs/online/fault_harness.md`, issue #279). Everything below is
//! written with that incident in mind (see "Making a failure impossible to
//! miss").
//!
//! # What this crate already had, and what this pass added
//!
//! `fault_harness.lua` wires together nine other modules
//! (`game/online/fault_harness.lua:50-67`):
//!
//! | Module | Status in this crate |
//! | --- | --- |
//! | `coordinator`, `protocol`, `protocol_fixture`, `match_manifest`, `match_session` | **ported** (`crate::coordinator`, `crate::protocol`, `crate::protocol_fixture`, `crate::match_manifest`, `crate::match_session`) |
//! | `FakeStarTransport` | **ported** (`crate::fake_star`) — a real in-process Rust star |
//! | `FakeRelayTransport` | **ported** (`crate::fake_relay`) — a real in-process Rust relay, no sequencer |
//! | `DiagnosticTransport`, `lobby_link`, `match_presentation`, `net_diagnostics`, `online_match_model` | TypeScript-owned (`v2/README.md` §2); no Rust type exists or is planned |
//! | `fault_transport`, `match_driver`, `match_driver_fixture`, `input_frame`, `match_snapshot`, `rollback_input_history` | **available**, used below |
//!
//! Two prior passes over this file left the same note: the hard blockers —
//! no coordinator, no protocol, no star — were gone, but building
//! [`FaultHarness`] itself (construction, the pre-match lifecycle, the match,
//! teardown) was real, bounded work neither pass reached, and each said so
//! explicitly rather than claiming a blocker that no longer existed. This
//! pass is the one that reached it — see "The live harness, and exactly what
//! it proves" below for what got built and how it was verified, and "What
//! still is not built, and why" for what remains out of reach and stays
//! that way (TypeScript-owned dependencies this crate does not port).
//!
//! [`scripted_sample`], every constant, the resource gates ([`Gates`]), and
//! the comparison logic ([`compare_checkpoints`]/[`compare_status`]) needed
//! no live harness and were real before this pass; [`FaultHarness::report`]
//! now calls them over the live clients it drives instead of the minimal
//! per-client views ([`ClientCheckpoints`]/[`ClientStatus`]) those functions
//! were originally exercised through in isolation — both are still real,
//! tested surfaces of this module, not superseded.
//!
//! # Making a failure impossible to miss
//!
//! The Lua `fault_harness.report(harness, expect_completed)` returns a
//! plain table with an `ok: boolean` field. Nothing in the Lua *forces* a
//! caller to read it — `report.ok` is exactly the kind of signal AGENTS.md
//! §9 warns about: a harness that computes a correct verdict but never
//! makes a caller act on it is one dropped `if` away from #279 again.
//!
//! [`FaultHarnessReport`] is `#[must_use]`, so a caller that discards the
//! return value of [`report`] gets a compiler warning — but a struct field
//! can still be read and ignored, so that alone is not enough.
//! [`FaultHarnessReport::into_outcome`] goes further: it converts the report
//! into `Result<FaultHarnessReport, FaultHarnessReport>`, `Ok` exactly when
//! every finding passed. `Result` is `#[must_use]` in `std` itself, and
//! unlike a boolean field it cannot be pattern-matched away by accident —
//! propagating it with `?` is the path of least resistance, and a caller
//! who wants the report *and* the pass/fail outcome gets both from the one
//! call. A caller that wants the gate to fail the process can
//! `std::process::exit` on `Err`; nothing here does that automatically,
//! because a library must not call `exit` on its caller's behalf, but the
//! `Result` shape means the caller has to decide, not silently continue.
//!
//! # The live harness, and exactly what it proves
//!
//! [`FaultHarness::new`]/[`FaultHarness::reach_start`]/
//! [`FaultHarness::start_match`]/[`FaultHarness::advance`]/
//! [`FaultHarness::report`]/[`FaultHarness::teardown`] are now real: N
//! independent [`coordinator::CoordinatorState`]s are driven through
//! handshake/manifest/assignment/ready/countdown over real
//! [`crate::fake_star::FakeStarTransport`] links, each wrapped in its own
//! [`crate::fault_transport::FaultTransport`] for impairment, exactly the
//! design this doc previously described as future work. The event sequence
//! in [`FaultHarness::reach_start`] is the same order
//! `crate::coordinator_driver::Driver::reach_start` uses (that type predates
//! this one and is the proof the sequence itself is correct against real
//! Lua-equivalent coordinator behaviour — see
//! `crates/gc-netcode/tests/coordinator_driver.rs`); what changed here is
//! *how* a message crosses from one client to another: not an internal fake
//! packet queue, but `coordinator::Action::Send`'s `targets`/`message`
//! encoded via [`crate::protocol::encode`] into a real
//! [`crate::fault_transport::TransportMessage`] on the transport's
//! `Control` channel, polled back out and fed to [`coordinator::receive`] on
//! the other end. A `Rc<RefCell<FaultTransport>>` is shared per client
//! between this module's own control-channel pump and the
//! `Box<dyn StarTransportAdapter>` [`FaultHarness::start_match`] hands to
//! [`crate::match_driver::new`] once that client's freeze lands (via
//! [`SharedFaultTransport`], a thin delegating wrapper) — the design this
//! doc's git history recorded before the harness itself existed.
//!
//! Concretely, [`FaultHarness::reach_start`] returning `true` means: every
//! client's coordinator independently processed a real encoded
//! `ManifestProposal`/`PeerAssignment`/countdown sequence arriving over a
//! real (impairment-capable, though unimpaired on the control channel by
//! design — see `crate::fault_transport`) transport link and reached
//! `Action::StartMatch` on its own, which is what `client.started` reports.
//! This is verified in this crate's own tests by asserting `reach_start`
//! returns `true` for 1v1/2v2/4v4 lobbies and by asserting the resulting
//! per-client [`coordinator::Freeze`]s agree (manifest id, assignment id,
//! seed) — the same cross-client agreement [`compare_checkpoints`] later
//! demands of the match itself, applied to the handshake's own output.
//!
//! # What still is not built, and why
//!
//! Everything downstream of `game/online/lobby_link.lua`,
//! `game/online/net_diagnostics.lua`, `game/online/match_presentation.lua`,
//! and `game/screens/online_match_model.lua` stays out of reach —
//! TypeScript-owned per `v2/README.md` §2, same as before. Concretely:
//!
//! - **No message chunking.** The real `lobby_link.lua` exists because a
//!   WebRTC data channel has a message-size ceiling a canonical control
//!   message can exceed, so it frames/reassembles. This crate's
//!   [`crate::fake_star::FakeStarTransport`] has no such ceiling on one
//!   [`crate::fault_transport::TransportMessage`], so [`FaultHarness`] sends
//!   one whole encoded control message as one transport envelope. This is a
//!   real simplification, not a hidden one: it means this harness cannot
//!   exercise `lobby_link`'s reassembly logic at all, only the coordinator
//!   underneath it.
//! - **No presentation timeline.** `match_presentation.lua`'s add/revoke/
//!   confirm bookkeeping has no Rust port, so [`FaultHarness::report`]
//!   declares `presentation.published_once`/`presentation.no_revoked_survivor`
//!   *skipped*, the same pattern [`declare_contingent`] already uses for the
//!   combat-phase and browser-multi-context rows, rather than either
//!   omitting them or reimplementing the timeline to force a green check.
//! - **No acknowledged-result exchange.** `online_match_model.lua`'s
//!   `command`/`result` handling has no Rust port, so
//!   [`FaultHarness::report`] compares only the boundary hash
//!   (`converge.final_hash`, computed for real from
//!   [`crate::match_driver::current_snapshot`]) and declares
//!   `converge.result` skipped rather than inventing an "acknowledged
//!   result" concept this port cannot produce.
//! - **No resource recorder.** `net_diagnostics.lua`'s folded running peaks
//!   have no Rust port. Where the underlying data is real and directly
//!   readable — [`crate::match_driver::diagnostics`]'s rollback/correction
//!   counters, and [`crate::fault_transport::StarTransportAdapter::diagnostics`]'s
//!   cumulative overflow/backpressure counters and per-channel depths —
//!   [`FaultHarness::report`] measures it directly rather than through a
//!   recorder that does not exist, sampling channel depth every
//!   [`FaultHarness::advance`] step so the peak gate reads a real running
//!   maximum rather than a quiescent final snapshot (the exact bug
//!   `docs/online/fault_harness.md` names: "queue depth is read from
//!   `runtime.pressure`, never from `runtime.peers`"). `teardown.clean` is
//!   declared *skipped* rather than measured, and this needed an actual
//!   investigation to get right, not just an absent dependency: an early
//!   draft gated it on residual queue depth read before
//!   [`FaultHarness::teardown`] closed every link, and that gate failed on
//!   an ordinary healthy 2v2 run — 9 residual input envelopes, not zero.
//!   [`match_driver::advance`]'s own doc is explicit that this is by
//!   design ("no hidden progress after a terminal status: nothing is
//!   polled"), and `docs/online/fault_harness.md`'s #241 section documents
//!   the same shape from the other direction (a peer that finished first
//!   stops reading mail a still-active peer keeps sending). Reading
//!   diagnostics *after* `shutdown` instead does not fix this either, for a
//!   different reason: `game/transport/fake_star.lua`'s `shutdown` drains
//!   each peer's queues to zero in place and marks it `"closed"`, matching
//!   what `game/online/diagnostic_transport.lua:180-201`'s `shutdown` reads
//!   back — `crate::fake_star::FakeStarTransport::shutdown` instead clears
//!   the whole peer table, so a post-shutdown reading would report zero
//!   unconditionally, the exact "gate that structurally cannot fail" this
//!   document elsewhere calls worse than no gate. Neither timing gives this
//!   port a residual-queue reading that means what `net_diagnostics`'s
//!   `runtime.teardown.residual_*` means, so it is named contingent instead
//!   of quietly measuring the wrong thing.
//! - **Boundary zero comes from the fixture, not from `match_session.lua`
//!   end to end.** [`crate::match_driver_fixture::initial_snapshot`] (pinned
//!   `nebula`/`orion` fixture teams, already differential-tested against the
//!   real Lua via `crates/gc-netcode/tests/match_driver.rs`) builds each
//!   client's boundary-zero snapshot, not
//!   [`crate::match_session::request`]/[`crate::match_manifest::resolve`]'s
//!   full content-resolution path. Both would produce a valid slot-mode
//!   boundary zero the freeze agrees with; the fixture path is the one this
//!   crate has already proven byte-exact, so it is the lower-risk choice for
//!   a module whose entire purpose is catching subtle divergence, not
//!   introducing an unverified one. `crate::match_session::request` still
//!   has real Rust unit-test coverage of its own (`tests/match_session.rs`)
//!   — it is unused *here*, not unported.
//!
//! [`FaultHarness::finished`] mirrors `fault_harness.all_drivers_terminal`
//! only, not `all_drivers_terminal and all_ended` — `all_ended` reads
//! `online_match_model.ended`, which does not exist here. A driver reaching
//! a terminal status is the strictly-necessary half of that conjunction this
//! port can observe, and [`FaultHarness::report`]'s
//! `lifecycle.ended` finding says exactly that in its own detail string
//! rather than silently narrowing what "ended" means.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gc_data::network_profiles::NetworkProfileName;
use gc_sim::input_frame::{self, InputSample, SlotId};
use gc_sim::match_snapshot;
use gc_sim::rollback_input_history;
use indexmap::IndexMap;

use crate::coordinator;
use crate::fake_relay::{self, FakeRelayTransport};
use crate::fake_star::{self, FakeStarTransport};
use crate::fault_transport::{
    FaultTransport, FaultTransportOptions, FaultTransportPollOrder, StarTransportAdapter,
    TransportChannel, TransportMessage, TransportMessageType, TransportPeerEvent,
    TransportPeerEventKind, TransportPeerState, TransportResult, TransportRole,
    TransportStarDiagnostics, TransportState,
};
use crate::match_driver::{self, MatchDriverCheckpoint, MatchDriverStatus};
use crate::match_driver_fixture;
use crate::protocol::{self, Value};
use crate::protocol_fixture;

/// Which shape the wire has beneath an unchanged session stack. Mirrors
/// `FaultHarnessTopology`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultHarnessTopology {
    /// The shipped OMP-3 direct-host star.
    Star,
    /// Every client holds one link to an in-process relay room.
    Relay,
}

impl FaultHarnessTopology {
    /// The exact Lua wire string (`harness.topology`'s own value, used
    /// verbatim in `fault_harness.lua`'s marker lines).
    #[must_use]
    fn wire_str(self) -> &'static str {
        match self {
            FaultHarnessTopology::Star => "star",
            FaultHarnessTopology::Relay => "relay",
        }
    }
}

/// One seated client's own transport endpoint: either shape
/// [`StarTransportAdapter`] admits. Mirrors the Lua `---@alias
/// FaultHarnessEndpoint FakeStarTransport|FakeRelayTransport` — Lua's duck
/// typing has no equivalent in Rust, so this crate needs an explicit sum type
/// where the Lua original needed none. Every method below delegates to
/// whichever concrete endpoint this client actually holds; the topology
/// itself is decided once, in [`FaultHarness::new`].
#[derive(Clone)]
pub enum FaultHarnessEndpoint {
    /// The shipped OMP-3 direct-host star.
    Star(FakeStarTransport),
    /// An in-process relay room member.
    Relay(FakeRelayTransport),
}

impl FaultHarnessEndpoint {
    /// Test seam: delivers everything currently buffered on this endpoint's
    /// wire (and, for a star, on every endpoint linked to it too — see
    /// `crate::fake_star::FakeStarTransport::pump`; a relay's single shared
    /// room makes the same true of `crate::fake_relay::FakeRelayTransport::pump`).
    pub fn pump(&self) {
        match self {
            FaultHarnessEndpoint::Star(star) => star.pump(),
            FaultHarnessEndpoint::Relay(relay) => relay.pump(),
        }
    }

    /// This endpoint's uplink/downlink envelope byte counters.
    #[must_use]
    pub fn wire_bytes(&self) -> (i64, i64) {
        match self {
            FaultHarnessEndpoint::Star(star) => star.wire_bytes(),
            FaultHarnessEndpoint::Relay(relay) => relay.wire_bytes(),
        }
    }
}

impl StarTransportAdapter for FaultHarnessEndpoint {
    fn initialize(&mut self) -> TransportResult<bool> {
        match self {
            FaultHarnessEndpoint::Star(star) => star.initialize(),
            FaultHarnessEndpoint::Relay(relay) => relay.initialize(),
        }
    }
    fn shutdown(&mut self) -> TransportResult<bool> {
        match self {
            FaultHarnessEndpoint::Star(star) => star.shutdown(),
            FaultHarnessEndpoint::Relay(relay) => relay.shutdown(),
        }
    }
    fn role(&self) -> TransportRole {
        match self {
            FaultHarnessEndpoint::Star(star) => star.role(),
            FaultHarnessEndpoint::Relay(relay) => relay.role(),
        }
    }
    fn capacity(&self) -> i64 {
        match self {
            FaultHarnessEndpoint::Star(star) => star.capacity(),
            FaultHarnessEndpoint::Relay(relay) => relay.capacity(),
        }
    }
    fn open_peer(&mut self, peer_id: &str) -> TransportResult<i64> {
        match self {
            FaultHarnessEndpoint::Star(star) => star.open_peer(peer_id),
            FaultHarnessEndpoint::Relay(relay) => relay.open_peer(peer_id),
        }
    }
    fn close_peer(&mut self, peer_id: &str, reason: Option<&str>) -> TransportResult<bool> {
        match self {
            FaultHarnessEndpoint::Star(star) => star.close_peer(peer_id, reason),
            FaultHarnessEndpoint::Relay(relay) => relay.close_peer(peer_id, reason),
        }
    }
    fn peer_ids(&self) -> Vec<String> {
        match self {
            FaultHarnessEndpoint::Star(star) => star.peer_ids(),
            FaultHarnessEndpoint::Relay(relay) => relay.peer_ids(),
        }
    }
    fn peer_state(&self, peer_id: &str) -> Option<TransportPeerState> {
        match self {
            FaultHarnessEndpoint::Star(star) => star.peer_state(peer_id),
            FaultHarnessEndpoint::Relay(relay) => relay.peer_state(peer_id),
        }
    }
    fn request_offer(&mut self, peer_id: &str) -> TransportResult<bool> {
        match self {
            FaultHarnessEndpoint::Star(star) => star.request_offer(peer_id),
            FaultHarnessEndpoint::Relay(relay) => relay.request_offer(peer_id),
        }
    }
    fn accept_offer(&mut self, signal: &str) -> TransportResult<bool> {
        match self {
            FaultHarnessEndpoint::Star(star) => star.accept_offer(signal),
            FaultHarnessEndpoint::Relay(relay) => relay.accept_offer(signal),
        }
    }
    fn accept_answer(&mut self, peer_id: &str, signal: &str) -> TransportResult<bool> {
        match self {
            FaultHarnessEndpoint::Star(star) => star.accept_answer(peer_id, signal),
            FaultHarnessEndpoint::Relay(relay) => relay.accept_answer(peer_id, signal),
        }
    }
    fn take_signal(&mut self, peer_id: &str) -> TransportResult<Option<String>> {
        match self {
            FaultHarnessEndpoint::Star(star) => star.take_signal(peer_id),
            FaultHarnessEndpoint::Relay(relay) => relay.take_signal(peer_id),
        }
    }
    fn send(
        &mut self,
        peer_id: &str,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<bool> {
        match self {
            FaultHarnessEndpoint::Star(star) => star.send(peer_id, channel, message),
            FaultHarnessEndpoint::Relay(relay) => relay.send(peer_id, channel, message),
        }
    }
    fn broadcast(
        &mut self,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<i64> {
        match self {
            FaultHarnessEndpoint::Star(star) => star.broadcast(channel, message),
            FaultHarnessEndpoint::Relay(relay) => relay.broadcast(channel, message),
        }
    }
    fn poll(&mut self) -> Option<crate::fault_transport::TransportPeerMessage> {
        match self {
            FaultHarnessEndpoint::Star(star) => star.poll(),
            FaultHarnessEndpoint::Relay(relay) => relay.poll(),
        }
    }
    fn poll_batch(
        &mut self,
        limit: Option<i64>,
    ) -> Vec<crate::fault_transport::TransportPeerMessage> {
        match self {
            FaultHarnessEndpoint::Star(star) => star.poll_batch(limit),
            FaultHarnessEndpoint::Relay(relay) => relay.poll_batch(limit),
        }
    }
    fn poll_event(&mut self) -> Option<TransportPeerEvent> {
        match self {
            FaultHarnessEndpoint::Star(star) => star.poll_event(),
            FaultHarnessEndpoint::Relay(relay) => relay.poll_event(),
        }
    }
    fn state(&self) -> TransportState {
        match self {
            FaultHarnessEndpoint::Star(star) => star.state(),
            FaultHarnessEndpoint::Relay(relay) => relay.state(),
        }
    }
    fn diagnostics(&self) -> TransportStarDiagnostics {
        match self {
            FaultHarnessEndpoint::Star(star) => star.diagnostics(),
            FaultHarnessEndpoint::Relay(relay) => relay.diagnostics(),
        }
    }
}

/// Mirrors `fault_harness.HOST_PEER_ID` (`transport_contract.HOST_PEER_ID`,
/// duplicated the same way `crate::match_driver` duplicates it).
pub const HOST_PEER_ID: &str = "host";
/// Mirrors `fault_harness.COUNTDOWN_ID`.
pub const COUNTDOWN_ID: &str = "countdown.1";
/// Mirrors `fault_harness.DEFAULT_DURATION_TICKS`.
pub const DEFAULT_DURATION_TICKS: i64 = 150;
/// Mirrors `fault_harness.DEFAULT_NETWORK_SEED`.
pub const DEFAULT_NETWORK_SEED: f64 = 4703.0;
/// Mirrors `fault_harness.DEFAULT_COUNTDOWN_TICKS`.
pub const DEFAULT_COUNTDOWN_TICKS: i64 = 2;
/// One 60 Hz frame, in milliseconds. Mirrors `fault_harness.CLOCK_STEP_MS`.
pub const CLOCK_STEP_MS: f64 = 1000.0 / 60.0;
/// Control rounds the pre-match lifecycle is allowed for one exchange.
/// Mirrors `fault_harness.MAX_CONTROL_ROUNDS`.
pub const MAX_CONTROL_ROUNDS: i64 = 24;

/// Mirrors `fault_harness.guest_peer_id`.
#[must_use]
pub fn guest_peer_id(index: i64) -> String {
    format!("guest_{index}")
}

// ---------------------------------------------------------------------------
// Deterministic scripted input
// ---------------------------------------------------------------------------

/// A pure function of `(client index, driver step)` only. Mirrors
/// `fault_harness.scripted_sample`. No clock, no RNG, no simulation read:
/// two runs of the same matrix produce byte-identical authority, which is
/// the precondition for comparing deterministic markers at all.
///
/// Rule 5.2 (README): both `step` (a driver step counter) and `index` (a
/// 1-based client index) are always non-negative in every caller this crate
/// has, so `(step + index * 11) % 47`, `phase % 13`, and `phase % 7` never
/// see a negative operand — Rust's truncating `%` already agrees with
/// Lua's floored `%` here and no `rem_euclid` is needed.
///
/// # Panics
///
/// Panics if the computed axes/edges somehow fall outside the sample's
/// valid range (a producer invariant: the arithmetic below is bounded by
/// construction and never does).
#[must_use]
pub fn scripted_sample(index: i64, step: i64) -> InputSample {
    let phase = (step + index * 11) % 47;
    let move_x = ((phase % 13) - 6) * 15;
    let move_y = ((phase % 7) - 3) * 21;
    let edges = if phase == 0 {
        input_frame::EDGE_SWITCH
    } else {
        0
    };
    input_frame::new_sample(input_frame::InputSampleOptions {
        move_x: Some(move_x),
        move_y: Some(move_y),
        edges: Some(edges),
        ..Default::default()
    })
    .expect("a scripted sample's axes/edges are always in range")
}

// ---------------------------------------------------------------------------
// Comparison and the deterministic marker set
// ---------------------------------------------------------------------------

/// One comparison's pass/fail/skip outcome. Mirrors `FaultHarnessFinding`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaultHarnessFinding {
    /// Stable finding id (`"converge.checkpoint_hash"`, ...).
    pub id: String,
    /// Whether this finding passed.
    pub ok: bool,
    /// Whether this finding is declared inert/blocked rather than checked.
    pub skipped: bool,
    /// Human-readable detail.
    pub detail: String,
}

fn finding(findings: &mut Vec<FaultHarnessFinding>, id: &str, ok: bool, detail: String) {
    findings.push(FaultHarnessFinding {
        id: id.to_string(),
        ok,
        skipped: false,
        detail,
    });
}

fn skipped(findings: &mut Vec<FaultHarnessFinding>, id: &str, detail: String) {
    findings.push(FaultHarnessFinding {
        id: id.to_string(),
        ok: true,
        skipped: true,
        detail,
    });
}

/// A full campaign run's verdict. Mirrors `FaultHarnessReport`.
///
/// `#[must_use]`: see the module doc's "Making a failure impossible to
/// miss". Prefer [`FaultHarnessReport::into_outcome`] over reading `ok`
/// directly — a `Result` cannot be silently matched away the way a boolean
/// field can.
#[must_use]
#[derive(Clone, Debug, PartialEq)]
pub struct FaultHarnessReport {
    /// Which match mode this run seated.
    pub mode: protocol::MatchMode,
    /// Which network profile this run drove the input channel through.
    pub profile: NetworkProfileName,
    /// Seated clients (host plus every guest).
    pub clients: i64,
    /// Driver steps the run took.
    pub steps: i64,
    /// Every comparison this run made.
    pub findings: Vec<FaultHarnessFinding>,
    /// Ordered, deterministic, comparable-across-processes markers.
    pub markers: Vec<String>,
    /// Explicitly logged coverage bounds; never silent truncation.
    pub notes: Vec<String>,
    /// Whether every non-skipped finding passed.
    pub ok: bool,
}

impl FaultHarnessReport {
    /// Converts this report into a `Result`, `Ok` exactly when [`Self::ok`]
    /// is `true`. See the module doc: prefer this over reading `ok`
    /// directly, since a `Result` cannot be silently dropped the way a
    /// struct field can (`std`'s own `#[must_use]` on `Result` — this is
    /// deliberately not reimplemented here, it is inherited for free by
    /// returning the real type — clippy's `double_must_use` refuses a
    /// redundant `#[must_use]` on a function that already returns a
    /// `#[must_use]`-carrying `Result`).
    pub fn into_outcome(self) -> Result<FaultHarnessReport, FaultHarnessReport> {
        if self.ok { Ok(self) } else { Err(self) }
    }
}

/// Declared resource gates. A run that exceeds one produces a blocking
/// finding rather than a warning. Mirrors `fault_harness.GATES`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gates {
    /// Worst observed rollback depth must not exceed this.
    pub max_rollback_depth: i64,
    /// Peak observed transport channel depth must not exceed this.
    pub max_channel_depth: i64,
    /// Residual queued messages after teardown must not exceed this.
    pub max_residual_queue: i64,
    /// Orphaned peers after teardown must not exceed this.
    pub max_orphan_peers: i64,
    /// Refused-send (overflow) events must not exceed this.
    pub max_overflow: i64,
}

/// Mirrors `fault_harness.GATES`'s values exactly
/// (`rollback_input_history.ROLLBACK_WINDOW_TICKS` and
/// `transport_contract.MAX_QUEUE_LIMIT`, the latter duplicated the same way
/// `crate::match_driver` duplicates it).
pub const GATES: Gates = Gates {
    max_rollback_depth: rollback_input_history::ROLLBACK_WINDOW_TICKS,
    max_channel_depth: 256,
    max_residual_queue: 0,
    max_orphan_peers: 0,
    max_overflow: 0,
};

fn live_marker(live: &IndexMap<String, SlotId>) -> String {
    let mut keys: Vec<&String> = live.keys().collect();
    keys.sort();
    keys.iter()
        .map(|producer_id| format!("{producer_id}={:?}", live[*producer_id]))
        .collect::<Vec<_>>()
        .join(",")
}

/// One client's published checkpoints, the minimal view
/// [`compare_checkpoints`] needs. A narrow stand-in for reading
/// `client.checkpoints`/`client.peer_id` off a live `FaultHarnessClient`
/// (blocked — see module doc).
#[derive(Clone, Debug, PartialEq)]
pub struct ClientCheckpoints {
    /// This client's peer id.
    pub peer_id: String,
    /// Every checkpoint this client published.
    pub checkpoints: Vec<MatchDriverCheckpoint>,
}

/// Pairwise checkpoint comparison. Mirrors `compare_checkpoints`. Two
/// clients are compared only at boundaries *both* hashed: a peer that has
/// not reached a checkpoint has not disagreed about it.
///
/// `is_4v4` mirrors the Lua's `harness.mode == "4v4"` special case: 4v4
/// cannot exhibit a live-slot divergence at all (singleton owned sets make
/// every branch of `next_live_slot` return the slot already live), so the
/// live-slot check is declared inert there rather than counted as coverage
/// it does not provide.
pub fn compare_checkpoints(
    clients: &[ClientCheckpoints],
    is_4v4: bool,
    findings: &mut Vec<FaultHarnessFinding>,
) {
    let mut compared = 0i64;
    let mut hash_bad = 0i64;
    let mut live_bad = 0i64;
    let mut detail: Vec<String> = Vec::new();
    for left in 0..clients.len() {
        for right in (left + 1)..clients.len() {
            let a = &clients[left];
            let b = &clients[right];
            let mut by_tick: IndexMap<i64, &MatchDriverCheckpoint> = IndexMap::new();
            for checkpoint in &b.checkpoints {
                by_tick.insert(checkpoint.tick, checkpoint);
            }
            for checkpoint in &a.checkpoints {
                let Some(&other) = by_tick.get(&checkpoint.tick) else {
                    continue;
                };
                compared += 1;
                if other.hash != checkpoint.hash {
                    hash_bad += 1;
                    if detail.len() < 4 {
                        detail.push(format!(
                            "hash {}/{}@{}",
                            a.peer_id, b.peer_id, checkpoint.tick
                        ));
                    }
                }
                if live_marker(&other.live) != live_marker(&checkpoint.live) {
                    live_bad += 1;
                    if detail.len() < 8 {
                        detail.push(format!(
                            "live {}/{}@{}",
                            a.peer_id, b.peer_id, checkpoint.tick
                        ));
                    }
                }
            }
        }
    }
    finding(
        findings,
        "converge.checkpoint_hash",
        hash_bad == 0 && compared > 0,
        format!(
            "compared {compared} shared checkpoints, {hash_bad} disagreed {}",
            detail.join(" ")
        ),
    );
    if is_4v4 {
        skipped(
            findings,
            "converge.live_slot",
            format!(
                "{compared} comparisons ran, but 4v4 owned sets are singletons so switching is inert"
            ),
        );
    } else {
        finding(
            findings,
            "converge.live_slot",
            live_bad == 0 && compared > 0,
            format!("compared {compared} shared checkpoints, {live_bad} live-slot maps disagreed"),
        );
    }
}

/// One client's driver status, the minimal view [`compare_status`] needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientStatus {
    /// This client's peer id.
    pub peer_id: String,
    /// This client's driver status, `None` if it never started a driver.
    pub status: Option<MatchDriverStatus>,
}

/// The exact Lua wire string for a driver status (`"unstarted"` for `None`,
/// mirroring `client.driver and match_driver.status(client.driver) or
/// "unstarted"`). `pub` so `crate::fault_scenarios::run` and this crate's
/// integration tests can name a declared terminal's finding id the same way.
#[must_use]
pub fn status_label(status: Option<MatchDriverStatus>) -> &'static str {
    match status {
        None => "unstarted",
        Some(MatchDriverStatus::Active) => "active",
        Some(MatchDriverStatus::Completed) => "completed",
        Some(MatchDriverStatus::SettleTimeout) => "settle_timeout",
        Some(MatchDriverStatus::ConfirmationStalled) => "confirmation_stalled",
        Some(MatchDriverStatus::LateInput) => "late_input",
        Some(MatchDriverStatus::HashMismatch) => "hash_mismatch",
        Some(MatchDriverStatus::OwnershipViolation) => "ownership_violation",
        Some(MatchDriverStatus::AuthorityConflict) => "authority_conflict",
        Some(MatchDriverStatus::InputChannelFailure) => "input_channel_failure",
        Some(MatchDriverStatus::TransportLost) => "transport_lost",
    }
}

/// Mirrors `compare_status`. A false `settle_timeout` on a healthy match is
/// the #237 defect inverted, so it is asserted separately from "everyone
/// completed" and only on scenarios that expect a clean completion.
pub fn compare_status(
    clients: &[ClientStatus],
    expect_completed: bool,
    findings: &mut Vec<FaultHarnessFinding>,
) {
    let mut statuses: Vec<String> = Vec::new();
    let mut completed = 0i64;
    let mut settle_timeouts = 0i64;
    for client in clients {
        statuses.push(format!(
            "{}={}",
            client.peer_id,
            status_label(client.status)
        ));
        match client.status {
            Some(MatchDriverStatus::Completed) => completed += 1,
            Some(MatchDriverStatus::SettleTimeout) => settle_timeouts += 1,
            _ => {}
        }
    }
    let joined = statuses.join(" ");
    if expect_completed {
        finding(
            findings,
            "lifecycle.no_settle_timeout",
            settle_timeouts == 0,
            format!("{settle_timeouts} peers timed out settling: {joined}"),
        );
    } else {
        skipped(
            findings,
            "lifecycle.no_settle_timeout",
            format!("an injected terminal fault strands the tail by construction: {joined}"),
        );
    }
    if expect_completed {
        finding(
            findings,
            "lifecycle.completed",
            completed == clients.len() as i64,
            joined,
        );
    } else {
        skipped(
            findings,
            "lifecycle.completed",
            format!("this scenario injects a terminal fault: {joined}"),
        );
    }
}

/// The contingent rows: declared and skipped with a reason rather than
/// omitted, so an absent row reads as "not applicable" and a silent pass
/// never reads as "covered". Mirrors `declare_contingent`.
pub fn declare_contingent(findings: &mut Vec<FaultHarnessFinding>) {
    for phase in [
        "wind_up",
        "guard",
        "contact",
        "projectile_flight",
        "stagger",
        "ball_spill",
        "immunity_expiry",
    ] {
        skipped(
            findings,
            &format!("combat.correction.{phase}"),
            "blocked on #112: the pre-#112 bot never drives the companion into this phase, \
             so a passing row here would pin an absence"
                .to_string(),
        );
    }
    skipped(
        findings,
        "combat.default_disposition",
        "blocked on #114: the manifest carries a placeholder combat disposition".to_string(),
    );
    skipped(
        findings,
        "browser.multi_context",
        "not covered by this tier: the browser multi-context bridge is exercised by \
         scripts/browser_matrix.py and by #170, and no claim about it is made here"
            .to_string(),
    );
}

// ---------------------------------------------------------------------------
// The live harness: construction, the pre-match lifecycle over the real
// star, the match itself, and teardown. See the module doc's "The live
// harness, and exactly what it proves" / "What still is not built, and why".
// ---------------------------------------------------------------------------

/// Inputs to [`FaultHarness::new`]. Mirrors `FaultHarnessOptions`.
#[derive(Clone, Debug, Default)]
pub struct FaultHarnessOptions {
    /// Wire shape; defaults to the shipped [`FaultHarnessTopology::Star`].
    pub topology: Option<FaultHarnessTopology>,
    /// Which match mode to seat.
    pub mode: Option<protocol::MatchMode>,
    /// Seated humans; fewer than the mode allows declares bot fills.
    pub humans: Option<i64>,
    /// Which network profile to run the input channel through; defaults to
    /// [`gc_data::network_profiles::NetworkProfileName::Clean`].
    pub profile: Option<NetworkProfileName>,
    /// Match seed override; defaults to the manifest fixture's own.
    pub seed: Option<i64>,
    /// Impairment seed, never the match seed; defaults to
    /// [`DEFAULT_NETWORK_SEED`].
    pub network_seed: Option<f64>,
    /// Simulated match duration, in ticks; defaults to
    /// [`DEFAULT_DURATION_TICKS`].
    pub duration_ticks: Option<i64>,
    /// Boundary-hash interval override, in ticks.
    pub hash_interval_ticks: Option<i64>,
    /// Release order every client's input channel hands to its driver.
    pub poll_order: Option<FaultTransportPollOrder>,
    /// Re-deliver every Nth control envelope.
    pub duplicate_control_every: Option<i64>,
    /// Clamps every endpoint's simulated send buffer. Clamping is only a
    /// fault if it actually latches — see [`FaultHarness::expect_backpressure`].
    pub buffered_amount_limit: Option<i64>,
}

/// A cheap `Clone` handle onto one client's [`FaultTransport`], substitutable
/// for the endpoint it wraps. Mirrors the module doc's "a `Rc<RefCell<...>>`
/// per client, shared between the harness's own control-channel pump and the
/// `Box<dyn StarTransportAdapter>` handed to `match_driver::new`". Delegating
/// through the shared cell is the whole point: [`FaultHarness`] keeps its own
/// handle (for the pre-match control pump and for resource sampling) while
/// [`FaultHarness::start_match`] boxes another handle into the driver, and
/// both observe the identical impairment state.
struct SharedFaultTransport(Rc<RefCell<FaultTransport>>);

impl StarTransportAdapter for SharedFaultTransport {
    fn initialize(&mut self) -> TransportResult<bool> {
        self.0.borrow_mut().initialize()
    }
    fn shutdown(&mut self) -> TransportResult<bool> {
        self.0.borrow_mut().shutdown()
    }
    fn role(&self) -> TransportRole {
        self.0.borrow().role()
    }
    fn capacity(&self) -> i64 {
        self.0.borrow().capacity()
    }
    fn open_peer(&mut self, peer_id: &str) -> TransportResult<i64> {
        self.0.borrow_mut().open_peer(peer_id)
    }
    fn close_peer(&mut self, peer_id: &str, reason: Option<&str>) -> TransportResult<bool> {
        self.0.borrow_mut().close_peer(peer_id, reason)
    }
    fn peer_ids(&self) -> Vec<String> {
        self.0.borrow().peer_ids()
    }
    fn peer_state(&self, peer_id: &str) -> Option<TransportPeerState> {
        self.0.borrow().peer_state(peer_id)
    }
    fn request_offer(&mut self, peer_id: &str) -> TransportResult<bool> {
        self.0.borrow_mut().request_offer(peer_id)
    }
    fn accept_offer(&mut self, signal: &str) -> TransportResult<bool> {
        self.0.borrow_mut().accept_offer(signal)
    }
    fn accept_answer(&mut self, peer_id: &str, signal: &str) -> TransportResult<bool> {
        self.0.borrow_mut().accept_answer(peer_id, signal)
    }
    fn take_signal(&mut self, peer_id: &str) -> TransportResult<Option<String>> {
        self.0.borrow_mut().take_signal(peer_id)
    }
    fn send(
        &mut self,
        peer_id: &str,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<bool> {
        self.0.borrow_mut().send(peer_id, channel, message)
    }
    fn broadcast(
        &mut self,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<i64> {
        self.0.borrow_mut().broadcast(channel, message)
    }
    fn poll(&mut self) -> Option<crate::fault_transport::TransportPeerMessage> {
        self.0.borrow_mut().poll()
    }
    fn poll_batch(
        &mut self,
        limit: Option<i64>,
    ) -> Vec<crate::fault_transport::TransportPeerMessage> {
        self.0.borrow_mut().poll_batch(limit)
    }
    fn poll_event(&mut self) -> Option<TransportPeerEvent> {
        self.0.borrow_mut().poll_event()
    }
    fn state(&self) -> TransportState {
        self.0.borrow().state()
    }
    fn diagnostics(&self) -> TransportStarDiagnostics {
        self.0.borrow().diagnostics()
    }
}

fn expectation_for(manifest: &Value) -> coordinator::ManifestExpectation {
    let field = |name: &str| {
        manifest
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    coordinator::ManifestExpectation {
        build_id: field("build_id"),
        source_id: field("source_id"),
        content_id: field("content_id"),
        tuning_id: field("tuning_id"),
        match_config_id: field("match_config_id"),
        fixture_id: field("fixture_id"),
        arena_id: field("arena_id"),
        combat_rules_id: field("combat_rules_id"),
        gameplay_ai_policy_id: field("gameplay_ai_policy_id"),
        combat_status: field("combat_status"),
    }
}

// ---------------------------------------------------------------------------
// Minimal control-channel chunking.
//
// `game/online/lobby_link.lua` exists to frame/reassemble one canonical
// control message across a real WebRTC channel's payload ceiling
// (`crate::fake_star::FakeStarTransport` restates the identical
// `transport_contract.MAX_PAYLOAD_BYTES` = 1024 bound). A whole encoded
// `ManifestProposal` regularly exceeds it, so sending one control message as
// one transport envelope (as the module doc's "no message chunking"
// deviation originally proposed) does not merely lose a feature — it cannot
// deliver the handshake at all. This is the smallest framing that fixes
// that without porting `lobby_link.lua`'s own reassembly buffers: a 2-byte
// header (`chunk index`, `chunk count`) per envelope, safe here because
// every client sends its chunks for one message synchronously and in full
// before starting the next (see `FaultHarnessClient::apply_outcome`), and
// the control channel is reliable, ordered, and never impaired
// (`crate::fault_transport`'s module doc) — the three preconditions this
// scheme needs and a real chunked reassembly buffer would have to detect
// for itself.
// ---------------------------------------------------------------------------

/// Leaves room for the 2-byte chunk header inside
/// [`crate::fake_star`]'s 1024-byte transport payload ceiling.
const CONTROL_CHUNK_PAYLOAD_BYTES: usize = 1022;

/// Splits `wire` into at most 255 header-prefixed chunks, each within the
/// transport's payload ceiling. Mirrors the sending half of
/// `lobby_link.lua`'s framing, narrowed to what this harness's synchronous
/// send pattern needs (see the section doc above).
///
/// # Panics
///
/// Panics if `wire` needs more than 255 chunks — no control message this
/// harness sends (a manifest proposal is the largest) comes close.
fn chunk_control_wire(wire: &[u8]) -> Vec<Vec<u8>> {
    let chunks: Vec<&[u8]> = if wire.is_empty() {
        vec![&[]]
    } else {
        wire.chunks(CONTROL_CHUNK_PAYLOAD_BYTES).collect()
    };
    assert!(
        chunks.len() <= u8::MAX as usize,
        "a control message needs more than 255 chunks to frame"
    );
    let total = chunks.len() as u8;
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            let mut framed = Vec::with_capacity(2 + chunk.len());
            framed.push(index as u8);
            framed.push(total);
            framed.extend_from_slice(chunk);
            framed
        })
        .collect()
}

/// The exact Lua wire string [`gc_data::network_profiles`] declares.
#[must_use]
fn profile_wire_str(profile: NetworkProfileName) -> &'static str {
    match profile {
        NetworkProfileName::Clean => "clean",
        NetworkProfileName::Omp0Parity => "omp0_parity",
        NetworkProfileName::Playable => "playable",
        NetworkProfileName::Stress => "stress",
    }
}

/// One isolated client: its own coordinator, its own transport, and — once
/// [`FaultHarness::start_match`] has run — its own [`match_driver::MatchDriver`].
/// Mirrors `FaultHarnessClient`, narrowed to the fields this port can
/// populate (see the module doc's "What still is not built, and why").
pub struct FaultHarnessClient {
    /// 1-based seating order; the host is always `1`.
    pub index: i64,
    /// This client's peer id.
    pub peer_id: String,
    /// Host or guest.
    pub role: coordinator::Role,
    star: FaultHarnessEndpoint,
    transport: Rc<RefCell<FaultTransport>>,
    /// This client's own coordinator session state.
    pub coordinator: coordinator::CoordinatorState,
    /// Whether this client's coordinator reached [`coordinator::Action::StartMatch`].
    pub started: bool,
    /// This client's match driver, once [`FaultHarness::start_match`] has run.
    pub driver: Option<match_driver::MatchDriver>,
    /// Every checkpoint this client's driver has published.
    pub checkpoints: Vec<MatchDriverCheckpoint>,
    /// This client's own final boundary hash, once its driver went terminal.
    pub final_hash: Option<String>,
    /// Non-fatal coordinator/transport errors observed, in order.
    pub errors: Vec<String>,
    peak_channel_depth: i64,
    outbound_seq: i64,
    /// Sender link id -> partially reassembled control wire bytes. See
    /// [`chunk_control_wire`].
    control_partial: IndexMap<String, Vec<u8>>,
}

impl FaultHarnessClient {
    fn apply_outcome(&mut self, outcome: coordinator::Outcome) {
        if !outcome.accepted
            && let Some(reason) = outcome.reason
        {
            self.errors.push(reason);
        }
        for action in outcome.actions {
            match action {
                coordinator::Action::Send { targets, message } => {
                    match protocol::encode(&message) {
                        Ok(wire) => {
                            for target in &targets {
                                for chunk in chunk_control_wire(wire.as_bytes()) {
                                    self.outbound_seq += 1;
                                    let result = self.transport.borrow_mut().send(
                                        target,
                                        TransportChannel::Control,
                                        TransportMessage {
                                            version: protocol::VERSION,
                                            kind: TransportMessageType::Event,
                                            seq: self.outbound_seq,
                                            tick: None,
                                            payload: chunk,
                                        },
                                    );
                                    if let Err(err) = result {
                                        self.errors.push(err.message);
                                    }
                                }
                            }
                        }
                        Err(_) => self
                            .errors
                            .push("a control message could not be encoded".to_string()),
                    }
                }
                coordinator::Action::Close { link_id } => {
                    let _ = self.transport.borrow_mut().close_peer(&link_id, None);
                }
                coordinator::Action::StartMatch { .. } => {
                    self.started = true;
                }
                coordinator::Action::Terminate { .. } => {}
            }
        }
    }

    /// Feeds one received control-channel envelope's payload through the
    /// chunk reassembly buffer for `link_id`, dispatching the decoded
    /// [`coordinator::Event::Control`] once every chunk of one message has
    /// arrived. See [`chunk_control_wire`].
    fn absorb_control(&mut self, link_id: String, payload: Vec<u8>) {
        if payload.len() < 2 {
            self.errors
                .push("a control envelope is too short to carry a chunk header".to_string());
            return;
        }
        let index = payload[0];
        let total = payload[1];
        let body = &payload[2..];
        if index == 0 {
            self.control_partial.insert(link_id.clone(), Vec::new());
        }
        let Some(buffer) = self.control_partial.get_mut(&link_id) else {
            self.errors.push(
                "a control chunk arrived out of order (no chunk 0 seen for this link)".to_string(),
            );
            return;
        };
        buffer.extend_from_slice(body);
        if index + 1 < total {
            return;
        }
        let bytes = self
            .control_partial
            .swap_remove(&link_id)
            .unwrap_or_default();
        match String::from_utf8(bytes) {
            Ok(wire) => self.dispatch(coordinator::Event::Control {
                link_id,
                message: None,
                wire: Some(wire),
            }),
            Err(_) => self
                .errors
                .push("a reassembled control envelope was not valid utf-8".to_string()),
        }
    }

    fn dispatch(&mut self, event: coordinator::Event) {
        let (state, outcome) = coordinator::step(&self.coordinator, event);
        self.coordinator = state;
        self.apply_outcome(outcome);
    }

    fn report_driver_terminal(&mut self) {
        let Some(driver) = &self.driver else {
            return;
        };
        if self.final_hash.is_some() || match_driver::terminal(driver).is_none() {
            return;
        }
        let snapshot = match_driver::current_snapshot(driver);
        self.final_hash = Some(match_snapshot::hash(&snapshot));
    }

    /// Sends one envelope directly on this client's own transport, as if
    /// this client's own driver had sent it. `fault_scenarios`'s forged-input
    /// injections use this to reach the wire without going through
    /// [`match_driver`] at all — see `crate::fault_scenarios::forged_bundle`'s
    /// doc: "a harness that pokes private state proves the poke works".
    ///
    /// # Errors
    ///
    /// Returns the transport's own error, unchanged, for the caller to
    /// handle.
    pub fn send_raw(
        &self,
        peer_id: &str,
        channel: TransportChannel,
        message: TransportMessage,
    ) -> TransportResult<bool> {
        self.transport.borrow_mut().send(peer_id, channel, message)
    }

    /// Buffers every input arrival in `[from, through]` on this client's own
    /// transport and releases the backlog afterwards. See
    /// [`FaultTransport::hold`].
    pub fn hold(&self, from: i64, through: i64) {
        self.transport.borrow_mut().hold(from, through);
    }
}

/// N fully isolated clients driven over a real in-process direct-host star:
/// handshake through the frozen start boundary, the match itself, and
/// teardown. Mirrors `FaultHarness`. See the module doc for exactly what
/// this proves and what it does not.
pub struct FaultHarness {
    /// Which wire shape this run seated over — see [`FaultHarnessOptions::topology`].
    pub topology: FaultHarnessTopology,
    /// Which match mode this run seated.
    pub mode: protocol::MatchMode,
    /// Which network profile this run drives the input channel through.
    pub profile: NetworkProfileName,
    /// The frozen manifest offered at handshake.
    pub manifest: Value,
    /// Every seated peer id, host first.
    pub peer_ids: Vec<String>,
    /// Every client, host first then guests in seating order.
    pub clients: Vec<FaultHarnessClient>,
    host_star: FaultHarnessEndpoint,
    /// The shared impairment clock every client's [`FaultTransport::tick`]
    /// advances with.
    pub transport_tick: i64,
    /// Driver steps taken so far.
    pub step: i64,
    clock_ms: Rc<Cell<f64>>,
    /// Boundary-hash interval every driver was constructed with.
    pub hash_interval_ticks: i64,
    /// Whether this run clamps the send buffer — a clamp is only a fault if
    /// it actually latches, so a row that asks for one has to observe it.
    pub expect_backpressure: bool,
    duration_ticks: i64,
    script: fn(i64, i64) -> InputSample,
    /// Explicitly logged coverage bounds; never silent truncation.
    pub notes: Vec<String>,
}

impl FaultHarness {
    /// Builds a new harness: one host plus `humans - 1` guests, every
    /// endpoint initialized and joined over a real in-process wire — a
    /// direct-host star or a relay room, per `options.topology`.
    ///
    /// # Panics
    ///
    /// Panics if `options.humans` seats more humans than the mode allows, or
    /// if constructing any fixture endpoint or coordinator fails — every
    /// step here is a fixture invariant, never expected to fail.
    #[must_use]
    pub fn new(options: FaultHarnessOptions) -> FaultHarness {
        let mode = options.mode.unwrap_or(protocol::MatchMode::FourVFour);
        let shape = protocol::match_mode_shape(mode);
        let humans = options.humans.unwrap_or(shape.humans);
        assert!(
            (1..=shape.humans).contains(&humans),
            "the mode does not seat that many humans"
        );

        let duration_ticks = options.duration_ticks.unwrap_or(DEFAULT_DURATION_TICKS);
        let mut manifest = protocol_fixture::manifest(Some(mode));
        manifest.set("duration_ticks", Value::int(duration_ticks));
        if let Some(seed) = options.seed {
            manifest.set("seed", Value::int(seed));
        }
        let expectation = expectation_for(&manifest);
        let session_id = coordinator::value_str(&manifest, "session_id");

        let mut peer_ids = vec![HOST_PEER_ID.to_string()];
        for index in 1..humans {
            peer_ids.push(guest_peer_id(index));
        }

        let profile = options.profile.unwrap_or(NetworkProfileName::Clean);
        let network_seed = options.network_seed.unwrap_or(DEFAULT_NETWORK_SEED);

        let topology = options.topology.unwrap_or(FaultHarnessTopology::Star);
        // A star needs a shared signaling rendezvous; a relay needs a shared
        // room. Only one is ever used, but both are cheap to mint (mirrors
        // `fault_harness.lua`'s `rendezvous`/`room` locals, which the Lua
        // original also builds unconditionally).
        let star_rendezvous = FakeStarTransport::new_rendezvous();
        let relay_room = FakeRelayTransport::new_room();

        // The star topology needs a concrete, mutable `FakeStarTransport` to
        // `open_peer`/`link` guests onto — operations `FaultHarnessEndpoint`
        // does not expose generically because a relay has no equivalent (see
        // the module doc: joining a relay room is the whole handshake, done
        // entirely inside `initialize`). Kept alongside the boxed
        // `host_endpoint` used everywhere else.
        let mut host_star_concrete: Option<FakeStarTransport> = None;
        let host_endpoint = match topology {
            FaultHarnessTopology::Star => {
                let mut star = FakeStarTransport::new(fake_star::FakeStarTransportOptions {
                    role: TransportRole::Host,
                    rendezvous: Some(star_rendezvous.clone()),
                    buffered_amount_limit: options.buffered_amount_limit,
                    ..Default::default()
                });
                star.initialize().expect("harness host star initializes");
                host_star_concrete = Some(star.clone());
                FaultHarnessEndpoint::Star(star)
            }
            FaultHarnessTopology::Relay => {
                let mut relay = FakeRelayTransport::new(fake_relay::FakeRelayTransportOptions {
                    role: TransportRole::Host,
                    peer_id: Some(HOST_PEER_ID.to_string()),
                    room: Some(relay_room.clone()),
                    buffered_amount_limit: options.buffered_amount_limit,
                    ..Default::default()
                });
                relay.initialize().expect("harness host relay initializes");
                FaultHarnessEndpoint::Relay(relay)
            }
        };

        let mut clients = Vec::with_capacity(peer_ids.len());
        let host_fault = FaultTransport::new(FaultTransportOptions {
            transport: Box::new(host_endpoint.clone()),
            profile,
            seed: network_seed + 101.0,
            legs: peer_ids[1..].to_vec(),
            poll_order: options.poll_order,
            duplicate_control_every: options.duplicate_control_every,
        });
        let host_state = coordinator::new(coordinator::Options {
            role: coordinator::Role::Host,
            session_id: session_id.clone(),
            peer_id: HOST_PEER_ID.to_string(),
            host_peer_id: None,
            host_link_id: None,
            runtime: protocol_fixture::runtime(),
            build_id: None,
            expectation: None,
        })
        .expect("harness host coordinator constructs");
        clients.push(FaultHarnessClient {
            index: 1,
            peer_id: HOST_PEER_ID.to_string(),
            role: coordinator::Role::Host,
            star: host_endpoint.clone(),
            transport: Rc::new(RefCell::new(host_fault)),
            coordinator: host_state,
            started: false,
            driver: None,
            checkpoints: Vec::new(),
            final_hash: None,
            errors: Vec::new(),
            peak_channel_depth: 0,
            outbound_seq: 0,
            control_partial: IndexMap::new(),
        });

        for (offset, peer_id) in peer_ids.iter().enumerate().skip(1) {
            let index = offset as i64 + 1;
            let guest_endpoint = match topology {
                FaultHarnessTopology::Star => {
                    let mut guest_star =
                        FakeStarTransport::new(fake_star::FakeStarTransportOptions {
                            role: TransportRole::Guest,
                            peer_id: Some(peer_id.clone()),
                            rendezvous: Some(star_rendezvous.clone()),
                            buffered_amount_limit: options.buffered_amount_limit,
                            ..Default::default()
                        });
                    guest_star
                        .initialize()
                        .expect("harness guest star initializes");
                    // The star needs the host to allocate a slot and join the
                    // link. A relay's `initialize` already joins it to every
                    // already-connected member symmetrically, with no member
                    // acting as the opener — see the module doc.
                    let host_star_mut = host_star_concrete
                        .as_mut()
                        .expect("star topology always has a concrete host star");
                    host_star_mut
                        .open_peer(peer_id)
                        .expect("harness host opens a guest slot");
                    host_star_mut
                        .link(&guest_star)
                        .expect("harness host links a guest");
                    FaultHarnessEndpoint::Star(guest_star)
                }
                FaultHarnessTopology::Relay => {
                    let mut guest_relay =
                        FakeRelayTransport::new(fake_relay::FakeRelayTransportOptions {
                            role: TransportRole::Guest,
                            peer_id: Some(peer_id.clone()),
                            room: Some(relay_room.clone()),
                            buffered_amount_limit: options.buffered_amount_limit,
                            ..Default::default()
                        });
                    guest_relay
                        .initialize()
                        .expect("harness guest relay initializes");
                    FaultHarnessEndpoint::Relay(guest_relay)
                }
            };

            let guest_fault = FaultTransport::new(FaultTransportOptions {
                transport: Box::new(guest_endpoint.clone()),
                profile,
                seed: network_seed + index as f64 * 101.0,
                legs: vec![HOST_PEER_ID.to_string()],
                poll_order: options.poll_order,
                duplicate_control_every: options.duplicate_control_every,
            });
            let guest_state = coordinator::new(coordinator::Options {
                role: coordinator::Role::Guest,
                session_id: session_id.clone(),
                peer_id: peer_id.clone(),
                host_peer_id: Some(HOST_PEER_ID.to_string()),
                host_link_id: Some(HOST_PEER_ID.to_string()),
                runtime: protocol_fixture::runtime(),
                build_id: None,
                expectation: Some(expectation.clone()),
            })
            .expect("harness guest coordinator constructs");
            clients.push(FaultHarnessClient {
                index,
                peer_id: peer_id.clone(),
                role: coordinator::Role::Guest,
                star: guest_endpoint,
                transport: Rc::new(RefCell::new(guest_fault)),
                coordinator: guest_state,
                started: false,
                driver: None,
                checkpoints: Vec::new(),
                final_hash: None,
                errors: Vec::new(),
                peak_channel_depth: 0,
                outbound_seq: 0,
                control_partial: IndexMap::new(),
            });
        }

        let mut notes = Vec::new();
        if humans < shape.humans {
            notes.push(format!(
                "bot fill: {humans} of {} seats are humans; the rest are declared fills",
                shape.humans
            ));
        }

        FaultHarness {
            topology,
            mode,
            profile,
            manifest,
            peer_ids,
            clients,
            host_star: host_endpoint,
            transport_tick: 0,
            step: 0,
            clock_ms: Rc::new(Cell::new(0.0)),
            hash_interval_ticks: options
                .hash_interval_ticks
                .unwrap_or(match_driver::DEFAULT_HASH_INTERVAL_TICKS),
            expect_backpressure: options.buffered_amount_limit.is_some(),
            duration_ticks,
            script: scripted_sample,
            notes,
        }
    }

    /// The guest at 1-based seating `index` (`2` is the first guest).
    ///
    /// # Panics
    ///
    /// Panics if `index` names no seated client.
    #[must_use]
    pub fn client(&self, index: i64) -> &FaultHarnessClient {
        &self.clients[(index - 1) as usize]
    }

    /// A mutable handle to the client at 1-based seating `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` names no seated client.
    #[must_use]
    pub fn client_mut(&mut self, index: i64) -> &mut FaultHarnessClient {
        &mut self.clients[(index - 1) as usize]
    }

    /// A cheap `Clone` handle onto the host's own transport, for injecting
    /// host-level transport faults ([`StarTransportAdapter::close_peer`]/
    /// [`StarTransportAdapter::shutdown`], both `&mut self`) the way
    /// `crate::fault_scenarios::inject` does. A handle rather than `&mut
    /// FakeStarTransport` because both endpoint shapes
    /// [`FaultHarnessEndpoint`] wraps are themselves just cheap
    /// `Rc<RefCell<_>>` wrappers (see `crate::fake_star`'s and
    /// `crate::fake_relay`'s module docs); mutating through a clone reaches
    /// the identical shared state whichever topology this run seated.
    #[must_use]
    pub fn host_star(&self) -> FaultHarnessEndpoint {
        self.host_star.clone()
    }

    fn pending_control(&self) -> i64 {
        let mut pending = 0;
        for client in &self.clients {
            let diagnostics = client.transport.borrow().diagnostics();
            for peer in &diagnostics.peers {
                pending += peer.control.outbound_depth + peer.control.inbound_depth;
            }
        }
        pending
    }

    /// One control round: advance every impairment clock, let every client
    /// read its mail, then flush the star. Keeps going while anything is
    /// still queued (a backpressure scenario clamps the send buffer on
    /// purpose), up to [`MAX_CONTROL_ROUNDS`]. Mirrors `fault_harness.pump_control`.
    pub fn pump_control(&mut self, rounds: Option<i64>) {
        let minimum = rounds.unwrap_or(2);
        for round in 1..=MAX_CONTROL_ROUNDS {
            if round > minimum && self.pending_control() == 0 {
                return;
            }
            self.transport_tick += 1;
            for client in &mut self.clients {
                client.transport.borrow_mut().tick(self.transport_tick);
            }
            for client in &mut self.clients {
                let messages = client.transport.borrow_mut().poll_batch(Some(64));
                for entry in messages {
                    if entry.channel != TransportChannel::Control {
                        continue;
                    }
                    client.absorb_control(entry.peer_id, entry.message.payload);
                }
                loop {
                    let event = client.transport.borrow_mut().poll_event();
                    let Some(event) = event else {
                        break;
                    };
                    if event.kind == TransportPeerEventKind::PeerState
                        && matches!(
                            event.state,
                            Some(TransportPeerState::Closed)
                                | Some(TransportPeerState::Disconnected)
                                | Some(TransportPeerState::Error)
                        )
                        && let Some(link_id) = event.peer_id
                    {
                        client.dispatch(coordinator::Event::LinkLost {
                            link_id,
                            code: Some("transport_lost".to_string()),
                        });
                    }
                }
            }
            self.host_star.pump();
        }
    }

    /// Handshake through the frozen start boundary, using the same event
    /// vocabulary and canonical wires production code uses. Mirrors
    /// `fault_harness.reach_start`.
    ///
    /// # Panics
    ///
    /// Panics if planning the seating assignment fails — a fixture
    /// invariant, never expected to fail for a manifest this module builds
    /// itself.
    #[must_use]
    pub fn reach_start(
        &mut self,
        countdown_ticks: Option<i64>,
        first_input_tick: Option<i64>,
    ) -> bool {
        for index in 1..self.clients.len() {
            self.clients[index].dispatch(coordinator::Event::Connect);
        }
        self.pump_control(Some(3));

        let manifest = self.manifest.clone();
        self.clients[0].dispatch(coordinator::Event::ProposeManifest { manifest });
        self.pump_control(Some(3));

        let assignments = coordinator::plan_assignments(&self.manifest, &self.peer_ids)
            .expect("harness seating failed");
        self.clients[0].dispatch(coordinator::Event::AssignSlots {
            assignments,
            preserve_claims: false,
        });
        self.pump_control(Some(3));

        for client in &mut self.clients {
            client.dispatch(coordinator::Event::SetReady { ready: true });
        }
        self.pump_control(Some(3));

        let ticks = countdown_ticks.unwrap_or(DEFAULT_COUNTDOWN_TICKS);
        self.clients[0].dispatch(coordinator::Event::BeginCountdown {
            countdown_id: COUNTDOWN_ID.to_string(),
            remaining_ticks: ticks,
            first_input_tick: first_input_tick.unwrap_or(0),
        });
        self.pump_control(Some(2));

        for _ in 0..(ticks + MAX_CONTROL_ROUNDS) {
            for client in &mut self.clients {
                client.dispatch(coordinator::Event::Tick);
            }
            self.pump_control(Some(2));
            if self.clients.iter().all(|client| client.started) {
                return true;
            }
        }
        false
    }

    /// Turn every frozen session into a running one: one
    /// [`match_driver::MatchDriver`] per client, each built from that
    /// client's own independently-derived freeze (see the module doc's
    /// "Isolation" note in `docs/online/fault_harness.md`). Mirrors
    /// `fault_harness.start_match`.
    ///
    /// # Panics
    ///
    /// Panics if any client never froze a session or never accepted a
    /// manifest — a caller error: [`FaultHarness::reach_start`] must have
    /// returned `true` first.
    pub fn start_match(&mut self) {
        let duration_seconds = self.duration_ticks as f64 / 60.0;
        let hash_interval_ticks = self.hash_interval_ticks;
        for client in &mut self.clients {
            let freeze = client
                .coordinator
                .freeze
                .clone()
                .expect("a client never froze a session");
            let manifest = client
                .coordinator
                .manifest
                .clone()
                .expect("a client never accepted a manifest");
            let driver_role = match client.role {
                coordinator::Role::Host => match_driver::DriverRole::Host,
                coordinator::Role::Guest => match_driver::DriverRole::Guest,
            };
            let initial_snapshot = match_driver_fixture::initial_snapshot(
                Some(duration_seconds),
                false,
                Some(freeze.seed as f64),
            );
            let driver_freeze = match_driver_fixture::to_driver_freeze(&freeze);
            let driver_manifest = match_driver_fixture::to_driver_manifest(&manifest);
            let rules = Box::new(match_driver_fixture::DriverRules::new(
                manifest.clone(),
                freeze.clone(),
            ));
            let clock_handle = self.clock_ms.clone();
            let driver = match_driver::new(match_driver::MatchDriverOptions {
                role: driver_role,
                peer_id: client.peer_id.clone(),
                freeze: driver_freeze,
                manifest: driver_manifest,
                transport: Box::new(SharedFaultTransport(client.transport.clone())),
                initial_snapshot,
                max_rollback_ticks: None,
                hash_interval_ticks: Some(hash_interval_ticks),
                settle_timeout_ticks: None,
                settle_timeout_seconds: None,
                clock: Some(Box::new(move || clock_handle.get() / 1000.0)),
                rules,
            });
            client.driver = Some(driver);
        }
    }

    /// One fixed 60 Hz step for every client: impair, advance, fold the
    /// terminal boundary hash once reached, flush the star. Mirrors
    /// `fault_harness.advance`.
    ///
    /// # Panics
    ///
    /// Panics if any client has no driver — a caller error:
    /// [`FaultHarness::start_match`] must run first.
    pub fn advance(&mut self) -> Vec<match_driver::MatchDriverBatch> {
        self.transport_tick += 1;
        for client in &mut self.clients {
            client.transport.borrow_mut().tick(self.transport_tick);
        }
        let step = self.step;
        let script = self.script;
        let mut batches = Vec::with_capacity(self.clients.len());
        for client in &mut self.clients {
            let driver = client
                .driver
                .as_mut()
                .expect("start_match must run before advance");
            let sample = script(client.index, step);
            let batch = match_driver::advance(driver, Some(sample));
            for checkpoint in &batch.checkpoints {
                client.checkpoints.push(checkpoint.clone());
            }
            let diagnostics = client.transport.borrow().diagnostics();
            let mut worst = 0;
            for peer in &diagnostics.peers {
                worst = worst
                    .max(peer.control.outbound_depth)
                    .max(peer.control.inbound_depth)
                    .max(peer.input.outbound_depth)
                    .max(peer.input.inbound_depth);
            }
            client.peak_channel_depth = client.peak_channel_depth.max(worst);
            batches.push(batch);
        }
        for client in &mut self.clients {
            client.report_driver_terminal();
        }
        self.host_star.pump();
        self.step += 1;
        self.clock_ms.set(self.clock_ms.get() + CLOCK_STEP_MS);
        batches
    }

    /// Every client's driver has left [`MatchDriverStatus::Active`]. Mirrors
    /// `fault_harness.all_drivers_terminal`.
    #[must_use]
    pub fn all_drivers_terminal(&self) -> bool {
        self.clients.iter().all(|client| match &client.driver {
            None => false,
            Some(driver) => match_driver::status(driver) != MatchDriverStatus::Active,
        })
    }

    /// See the module doc: mirrors `fault_harness.all_drivers_terminal` only,
    /// not the Lua's `all_drivers_terminal and all_ended` — `all_ended` reads
    /// `online_match_model.ended`, which has no Rust port.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.all_drivers_terminal()
    }

    /// Shuts every client's transport down. Mirrors `fault_harness.teardown`.
    /// [`FaultHarness::report`] reads the cumulative overflow/backpressure
    /// counters `shutdown` does not reset; see the module doc for why a
    /// residual-queue reading, before or after this call, is not measured
    /// here at all.
    pub fn teardown(&mut self) {
        for client in &mut self.clients {
            let _ = client.transport.borrow_mut().shutdown();
        }
    }

    /// Build the full report. `expect_completed` is `false` for scenarios
    /// that inject a deliberately terminal fault, where "nobody completed"
    /// is the expected outcome rather than a failure. Mirrors
    /// `fault_harness.report`; see the module doc for exactly which findings
    /// are real versus declared skipped in this port.
    pub fn report(&self, expect_completed: bool) -> FaultHarnessReport {
        let mut findings = Vec::new();
        let mut markers = Vec::new();

        markers.push(format!(
            "scenario topology={} mode={} profile={} clients={} steps={}",
            self.topology.wire_str(),
            self.mode.wire_str(),
            profile_wire_str(self.profile),
            self.clients.len(),
            self.step
        ));

        for client in &self.clients {
            let status = client.driver.as_ref().map(match_driver::status);
            let terminal = client.driver.as_ref().and_then(match_driver::terminal);
            let diagnostics = client.driver.as_ref().map(match_driver::diagnostics);
            markers.push(format!(
                "client {} status={} terminal={} final={} confirmed_output={} full_time={}",
                client.peer_id,
                status_label(status),
                terminal
                    .as_ref()
                    .map(|terminal| status_label(Some(terminal.status)))
                    .unwrap_or("-"),
                client.final_hash.as_deref().unwrap_or("-"),
                diagnostics
                    .as_ref()
                    .map(|diagnostics| diagnostics.confirmed_output_tick.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                diagnostics
                    .as_ref()
                    .and_then(|diagnostics| diagnostics.full_time_boundary)
                    .map(|tick| tick.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ));
            for checkpoint in &client.checkpoints {
                markers.push(format!(
                    "checkpoint {} {} {} {}",
                    client.peer_id,
                    checkpoint.tick,
                    checkpoint.hash,
                    live_marker(&checkpoint.live)
                ));
            }
            let counters = client.transport.borrow().counters();
            let model = client.transport.borrow().model_diagnostics();
            markers.push(format!(
                "impairment {} profile={} scheduled={} delivered={} dropped={} independent={} \
                 burst={} duplicated={} reordered={} withheld={} held={} control={} \
                 control_dup={} pending={} peak_pending={} peak_retained={}",
                client.peer_id,
                profile_wire_str(counters.profile),
                counters.scheduled,
                counters.delivered,
                counters.dropped,
                counters.independent_lost,
                counters.burst_lost,
                counters.duplicated,
                counters.reordered,
                counters.withheld,
                counters.held,
                counters.control_delivered,
                counters.control_duplicated,
                counters.pending,
                model.peak_pending_envelopes,
                model.peak_retained_authoritative_records,
            ));
        }

        let started = self.clients.iter().all(|client| client.started);
        finding(
            &mut findings,
            "lifecycle.started",
            started,
            "every client reached the frozen start".to_string(),
        );
        finding(
            &mut findings,
            "lifecycle.ended",
            self.all_drivers_terminal(),
            "every client's driver reached a terminal status (no session-model check: \
             online_match_model is TypeScript-owned in this port)"
                .to_string(),
        );

        let statuses: Vec<ClientStatus> = self
            .clients
            .iter()
            .map(|client| ClientStatus {
                peer_id: client.peer_id.clone(),
                status: client.driver.as_ref().map(match_driver::status),
            })
            .collect();
        compare_status(&statuses, expect_completed, &mut findings);

        let checkpoints: Vec<ClientCheckpoints> = self
            .clients
            .iter()
            .map(|client| ClientCheckpoints {
                peer_id: client.peer_id.clone(),
                checkpoints: client.checkpoints.clone(),
            })
            .collect();
        compare_checkpoints(
            &checkpoints,
            self.mode == protocol::MatchMode::FourVFour,
            &mut findings,
        );

        if expect_completed {
            let reference_hash = self
                .clients
                .first()
                .and_then(|client| client.final_hash.clone());
            let mut ok = reference_hash.is_some();
            let mut detail = Vec::new();
            for client in &self.clients {
                if client.final_hash != reference_hash {
                    ok = false;
                    detail.push(format!("{} final hash differs", client.peer_id));
                }
            }
            finding(&mut findings, "converge.final_hash", ok, detail.join("; "));
            skipped(
                &mut findings,
                "converge.result",
                "game.screens.online_match_model is TypeScript-owned in this port; only the \
                 boundary hash is compared, not an acknowledged result"
                    .to_string(),
            );
        } else {
            skipped(
                &mut findings,
                "converge.final_hash",
                "a terminal fault scenario has no agreed result".to_string(),
            );
            skipped(
                &mut findings,
                "converge.result",
                "a terminal fault scenario has no agreed result".to_string(),
            );
        }

        skipped(
            &mut findings,
            "presentation.published_once",
            "game.online.match_presentation is TypeScript-owned in this port; no presentation \
             timeline is folded here"
                .to_string(),
        );
        skipped(
            &mut findings,
            "presentation.no_revoked_survivor",
            "game.online.match_presentation is TypeScript-owned in this port; no presentation \
             timeline is folded here"
                .to_string(),
        );

        for client in &self.clients {
            let (uplink_bytes, downlink_bytes) = client.star.wire_bytes();
            markers.push(format!(
                "wire {} uplink_bytes={uplink_bytes} downlink_bytes={downlink_bytes}",
                client.peer_id,
            ));
        }

        let mut worst_rollback_depth = 0;
        let mut worst_channel_depth = 0;
        let mut overflow_total = 0;
        let mut backpressure_total = 0;
        for client in &self.clients {
            if let Some(driver) = &client.driver {
                worst_rollback_depth =
                    worst_rollback_depth.max(match_driver::diagnostics(driver).max_rollback_depth);
            }
            worst_channel_depth = worst_channel_depth.max(client.peak_channel_depth);
            // Cumulative since construction; `shutdown` does not reset them
            // (see `crate::fake_star`), so reading live here versus a
            // snapshot taken before `FaultHarness::teardown` reports the
            // same totals either way.
            let diagnostics = client.transport.borrow().diagnostics();
            overflow_total += diagnostics.overflow;
            backpressure_total += diagnostics.backpressure;
        }
        finding(
            &mut findings,
            "resources.rollback_depth",
            worst_rollback_depth <= GATES.max_rollback_depth,
            format!(
                "worst rollback depth {worst_rollback_depth} against a gate of {}",
                GATES.max_rollback_depth
            ),
        );
        finding(
            &mut findings,
            "resources.channel_depth",
            worst_channel_depth <= GATES.max_channel_depth,
            format!(
                "peak channel depth {worst_channel_depth} against a gate of {}",
                GATES.max_channel_depth
            ),
        );
        finding(
            &mut findings,
            "resources.channel_depth_observed",
            worst_channel_depth > 0,
            format!(
                "the depth gate observed a peak of {worst_channel_depth}; a zero peak would \
                 mean the gate cannot fail"
            ),
        );
        finding(
            &mut findings,
            "resources.no_overflow",
            overflow_total <= GATES.max_overflow,
            format!(
                "{overflow_total} refused sends (star overflow) against a gate of {}",
                GATES.max_overflow
            ),
        );
        if self.expect_backpressure {
            finding(
                &mut findings,
                "faults.backpressure_observed",
                backpressure_total > 0,
                format!(
                    "the clamped send buffer latched {backpressure_total} times across {} clients",
                    self.clients.len()
                ),
            );
        } else {
            skipped(
                &mut findings,
                "faults.backpressure_observed",
                format!(
                    "this row does not clamp the send buffer; {backpressure_total} latches \
                     observed anyway"
                ),
            );
        }
        skipped(
            &mut findings,
            "teardown.clean",
            "neither timing this port could read a residual-queue snapshot at (before or after \
             crate::fake_star::FakeStarTransport::shutdown) means what \
             net_diagnostics.runtime.teardown.residual_* means here; see the module doc's \
             'No resource recorder' section for the investigation"
                .to_string(),
        );

        declare_contingent(&mut findings);

        let ok = findings.iter().all(|entry| entry.ok);
        FaultHarnessReport {
            mode: self.mode,
            profile: self.profile,
            clients: self.clients.len() as i64,
            steps: self.step,
            findings,
            markers,
            notes: self.notes.clone(),
            ok,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_driver::MatchDriverStatus;

    /// Foundational proof, at this module's own layer rather than through
    /// `crate::fault_scenarios`: a real 8-client 4v4 lobby reaches the
    /// frozen start over the real star, plays out under the `stress`
    /// impairment profile (real loss/duplication/reordering — see the
    /// `impairment` markers this asserts against), and every client
    /// converges on the same final boundary hash despite it.
    #[test]
    fn reach_start_and_advance_converge_under_real_impairment() {
        let mut harness = FaultHarness::new(FaultHarnessOptions {
            mode: Some(protocol::MatchMode::FourVFour),
            duration_ticks: Some(60),
            profile: Some(NetworkProfileName::Stress),
            ..Default::default()
        });
        assert!(
            harness.reach_start(None, None),
            "harness never reached start: {:?}",
            harness
                .clients
                .iter()
                .map(|client| (client.peer_id.clone(), client.errors.clone()))
                .collect::<Vec<_>>()
        );
        let reference = harness.clients[0]
            .coordinator
            .freeze
            .clone()
            .expect("the host froze a session");
        for client in &harness.clients {
            let freeze = client
                .coordinator
                .freeze
                .clone()
                .expect("every client must have frozen a session");
            assert_eq!(
                freeze.manifest_id, reference.manifest_id,
                "{} disagreed on the frozen manifest id",
                client.peer_id
            );
            assert_eq!(
                freeze.assignment_id, reference.assignment_id,
                "{} disagreed on the frozen assignment id",
                client.peer_id
            );
            assert_eq!(
                freeze.seed, reference.seed,
                "{} disagreed on the frozen match seed",
                client.peer_id
            );
        }
        harness.start_match();
        for _ in 0..200 {
            harness.advance();
            if harness.finished() {
                break;
            }
        }
        harness.teardown();
        let report = harness.report(true);
        assert!(
            report
                .markers
                .iter()
                .any(|marker| marker.starts_with("impairment")
                    && marker.contains("dropped=")
                    && !marker.contains("dropped=0")),
            "the stress profile must actually drop packets: {:?}",
            report.markers
        );
        for finding in &report.findings {
            assert!(finding.ok, "{} failed: {}", finding.id, finding.detail);
        }
        assert!(report.into_outcome().is_ok());
    }

    #[test]
    fn scripted_sample_is_a_pure_function_of_index_and_step() {
        let a = scripted_sample(3, 17);
        let b = scripted_sample(3, 17);
        assert_eq!(a, b);
    }

    #[test]
    fn scripted_sample_fires_the_switch_edge_only_at_phase_zero() {
        // phase = (step + index * 11) % 47; index=1 step=36 -> phase 47%47=0.
        let sample = scripted_sample(1, 36);
        assert_eq!(sample.edges, input_frame::EDGE_SWITCH);
    }

    #[test]
    fn a_report_with_a_failing_finding_converts_to_err() {
        let report = FaultHarnessReport {
            mode: protocol::MatchMode::OneVOne,
            profile: NetworkProfileName::Clean,
            clients: 2,
            steps: 10,
            findings: vec![FaultHarnessFinding {
                id: "converge.final_hash".to_string(),
                ok: false,
                skipped: false,
                detail: "peer disagreed".to_string(),
            }],
            markers: Vec::new(),
            notes: Vec::new(),
            ok: false,
        };
        assert!(report.into_outcome().is_err());
    }

    #[test]
    fn a_fully_passing_report_converts_to_ok() {
        let report = FaultHarnessReport {
            mode: protocol::MatchMode::OneVOne,
            profile: NetworkProfileName::Clean,
            clients: 2,
            steps: 10,
            findings: Vec::new(),
            markers: Vec::new(),
            notes: Vec::new(),
            ok: true,
        };
        assert!(report.into_outcome().is_ok());
    }

    #[test]
    fn compare_checkpoints_flags_a_disagreeing_hash() {
        let mut findings = Vec::new();
        let a = ClientCheckpoints {
            peer_id: "host".to_string(),
            checkpoints: vec![MatchDriverCheckpoint {
                tick: 30,
                hash: "aaaa".to_string(),
                live: IndexMap::new(),
            }],
        };
        let b = ClientCheckpoints {
            peer_id: "guest_1".to_string(),
            checkpoints: vec![MatchDriverCheckpoint {
                tick: 30,
                hash: "bbbb".to_string(),
                live: IndexMap::new(),
            }],
        };
        compare_checkpoints(&[a, b], false, &mut findings);
        let hash_finding = findings
            .iter()
            .find(|f| f.id == "converge.checkpoint_hash")
            .expect("finding recorded");
        assert!(!hash_finding.ok);
    }

    #[test]
    fn compare_checkpoints_passes_when_every_shared_boundary_agrees() {
        let mut findings = Vec::new();
        let a = ClientCheckpoints {
            peer_id: "host".to_string(),
            checkpoints: vec![MatchDriverCheckpoint {
                tick: 30,
                hash: "aaaa".to_string(),
                live: IndexMap::new(),
            }],
        };
        let b = ClientCheckpoints {
            peer_id: "guest_1".to_string(),
            checkpoints: vec![MatchDriverCheckpoint {
                tick: 30,
                hash: "aaaa".to_string(),
                live: IndexMap::new(),
            }],
        };
        compare_checkpoints(&[a, b], false, &mut findings);
        assert!(findings.iter().all(|f| f.ok));
    }

    #[test]
    fn compare_checkpoints_declares_live_slot_inert_on_4v4() {
        let mut findings = Vec::new();
        compare_checkpoints(&[], true, &mut findings);
        let live_finding = findings
            .iter()
            .find(|f| f.id == "converge.live_slot")
            .expect("finding recorded");
        assert!(live_finding.skipped);
    }

    #[test]
    fn compare_status_reports_settle_timeouts_only_when_expecting_completion() {
        let mut findings = Vec::new();
        let clients = [ClientStatus {
            peer_id: "host".to_string(),
            status: Some(MatchDriverStatus::SettleTimeout),
        }];
        compare_status(&clients, true, &mut findings);
        let no_timeout = findings
            .iter()
            .find(|f| f.id == "lifecycle.no_settle_timeout")
            .expect("finding recorded");
        assert!(!no_timeout.ok);

        let mut findings = Vec::new();
        compare_status(&clients, false, &mut findings);
        let no_timeout = findings
            .iter()
            .find(|f| f.id == "lifecycle.no_settle_timeout")
            .expect("finding recorded");
        assert!(no_timeout.skipped);
    }

    #[test]
    fn declare_contingent_names_every_blocked_combat_phase() {
        let mut findings = Vec::new();
        declare_contingent(&mut findings);
        assert_eq!(findings.len(), 9);
        assert!(findings.iter().all(|f| f.skipped));
    }
}
