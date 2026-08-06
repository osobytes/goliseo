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
//! # Why most of the lifecycle is not ported here
//!
//! `fault_harness.lua` wires together nine other modules
//! (`game/online/fault_harness.lua:50-67`):
//!
//! | Module | Status in this crate |
//! | --- | --- |
//! | `coordinator` | `NOT YET PORTED` placeholder, owned by a concurrent agent |
//! | `DiagnosticTransport`, `lobby_link`, `match_presentation`, `net_diagnostics`, `online_match_model` | TypeScript-owned (`v2/README.md` §2); no Rust type exists or is planned |
//! | `match_manifest`, `match_session`, `protocol`, `protocol_fixture` | `NOT YET PORTED` placeholders, owned by concurrent agents |
//! | `FakeRelayTransport`, `FakeStarTransport`, `transport_contract` | TypeScript-owned; see `crate::fault_transport`'s module doc |
//! | `fault_transport`, `match_driver`, `input_frame`, `match_snapshot`, `rollback_input_history` | **available**, used below |
//!
//! Every function that builds or drives a live `FaultHarness` —
//! construction, the pre-match control lifecycle, `start_match`, `advance`,
//! `teardown`, and the parts of `report` that read a client's presentation
//! timeline, session model, or exported diagnostics artifact — needs at
//! least one of the blocked modules and is **not ported**. What *is* ported
//! is everything that does not: the deterministic scripted-input generator
//! ([`scripted_sample`]), every constant, the resource gates ([`Gates`]),
//! and the comparison logic that only needs data this crate can already
//! produce — [`compare_checkpoints`] and [`compare_status`] are ported as
//! pure functions over a minimal per-client view
//! ([`ClientCheckpoints`]/[`ClientStatus`]) instead of a live
//! `FaultHarnessClient`, so they are real, tested code today and only need
//! rewiring — not rewriting — once the harness itself exists.
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

use gc_sim::input_frame::{self, InputSample, SlotId};
use gc_sim::rollback_input_history;
use indexmap::IndexMap;

use crate::match_driver::{MatchDriverCheckpoint, MatchDriverStatus};

/// Which shape the wire has beneath an unchanged session stack. Mirrors
/// `FaultHarnessTopology`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultHarnessTopology {
    /// The shipped OMP-3 direct-host star.
    Star,
    /// Every client holds one link to an in-process relay room.
    Relay,
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

fn status_label(status: Option<MatchDriverStatus>) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_driver::MatchDriverStatus;

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
