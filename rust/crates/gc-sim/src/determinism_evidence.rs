//! OMP-1 full-match determinism recording, verification, and evidence
//! report. Verification decodes only the checked-in effective
//! [`InputFrame`]s. Bot policy is never allowed to rewrite the
//! authoritative input contract.
//!
//! ## Where the fixture data comes from
//!
//! [`gc_data::omp1_determinism`] embeds the frozen fixture as JSON (see that
//! module's doc). This module reads it through `fixture()`,
//! `frame_wire_lines()`, and `boundary_hash_lines()`.
//!
//! `gc_data::omp1_determinism::InputTapeIdentity`/`InputOwnership` are
//! JSON-shaped (plain `String` team/slot names) rather than
//! [`crate::input_tape::InputTapeIdentity`]/[`crate::input_frame::InputOwnership`]'s
//! enum-typed shape, so [`fixture_identity`] is the explicit adapter that
//! bridges them, parsing canonical slot order rather than the name strings
//! (the fixture's slots are already listed in [`input_frame::slot`]'s
//! canonical order, verified against the checked-in JSON).
//!
//! ## `Result`, matching `input_tape`'s and `replay`'s reasoning
//!
//! Every failure path here is either (a) evidence disagreeing with the
//! frozen fixture — precisely "validation of external/frozen input" per
//! AGENTS.md §7 — or (b) a call into [`crate::input_tape`]/[`crate::replay`],
//! which are already `Result`-shaped for the same reason (see
//! `input_tape.rs`'s module doc: this workspace's release profile sets
//! `panic = "abort"`, so there is no unwinding escape hatch in a release
//! build). So this module's public functions return `Result<T, String>`
//! throughout.
//!
//! ## Tuning is an explicit parameter, never a global
//!
//! Matches [`crate::input_tape`] and [`crate::replay`]: every function that
//! needs to confirm the fixture's recorded tuning identity against the
//! active configuration takes `&Tuning` explicitly instead of reading a
//! global.
//!
//! ## What this module can and cannot regenerate
//!
//! Half the fixture, and precisely which half matters. Until #503 this
//! section said the fixture could not be regenerated at all; that was true
//! of the recorded input and false of the hashes derived from it, and the
//! difference is what blocked every queued gameplay rework.
//!
//! **Never, by anything:** the recorded input. The `frame_wires`,
//! `identity`, `source_seeds` and `windows` in
//! [`gc_data::omp1_determinism`] came from source bots that no longer exist
//! in this repository. Nothing here can produce a match from a live bot
//! policy and call it that recording — bot policy is never allowed to
//! rewrite the authoritative input contract, which is the first sentence of
//! this doc.
//!
//! **Under a deliberate, reviewed change:** the derived hashes.
//! [`record`] replays those frozen frames and recomputes every
//! `boundary_hashes` entry, [`sequence_digest`] folds them into the pinned
//! digest, and `gc_data::omp1_determinism::rerecorded_json` writes the
//! result back into the fixture's byte format with the frozen half copied
//! verbatim and verified. The command is `record_omp1_derived_baseline` in
//! this crate's `tests/determinism_evidence.rs`; it is `#[ignore]`d and
//! never runs in a gate, because a recorder that refreshed its own baseline
//! during `cargo test` would turn a determinism regression into a no-op.
//!
//! A drifted boundary hash is therefore a **finding first**: an unintended
//! one is a desync between a client built before the change and one built
//! after. Re-record only for a change that is deliberate and reviewed, and
//! record the decision, the superseding change, and the last commit where
//! the previous baseline held. See [`gc_data::omp1_determinism`]'s module
//! doc for the full split, and for what a green campaign stops proving once
//! its baseline is self-recorded.
//!
//! ## What a campaign gates on, and what it only reports (#505)
//!
//! A third split, and it cuts across the two above. The repository owner's
//! decision on #505 separates the campaign's assertions by *what they
//! actually prove*:
//!
//! | | **Gated** — a failing check is an `Err` | **Reported** — surfaced, never an `Err` |
//! | --- | --- | --- |
//! | Determinism | boundary-hash chain, `expected_final_hash`, `expected_sequence_digest`, both fresh replays agreeing, every restore window replaying to its pinned hashes | |
//! | Behavior | [`DeterminismCoverage`] — a tackle, a catch, a header and a full time **occurred**; each window still contains the event it is scoped around | `expected_score`, `event_counts`, and the per-window `event_tick` — *how many* and *exactly when* |
//!
//! The line is `"these behaviors occurred"` (gated) against `"exactly 147
//! tackles occurred and the score was 1-0"` (reported). The scoreline and the
//! tackle count are incidental properties of the scenario that happened to be
//! recorded, not the guarantee this fixture exists to provide; gating on them
//! turned "the simulation is deterministic" into "the simulation must keep
//! producing this 1-0", which foreclosed every queued gameplay rework
//! (#488/#489/#490/#491) and is not a contract anyone chose.
//!
//! ### How a contributor is meant to notice a reported drift
//!
//! **A demoted assertion that prints nothing is a deleted assertion.** So the
//! reporting is the load-bearing half of that demotion, and it is deliberately
//! not routed through `cargo test`, whose stdout is swallowed for a passing
//! test. Every claim above that moved shows up as a [`BehavioralDrift`] entry
//! carrying the recorded value *and* the observed one, and reaches a human
//! through **three** independent channels, per AGENTS.md §9's "never trust one
//! signal":
//!
//! 1. `scripts/check.sh`'s determinism gate prints the `drift=` field of the
//!    `GC_DETERMINISM` terminator on **every** run, and escalates a non-empty
//!    one to a multi-line `BEHAVIORAL DRIFT` block above the gate's own
//!    summary line. Mirrored in `.github/workflows/ci.yml` by virtue of that
//!    job calling this script.
//! 2. `ts/packages/wasm/src/determinism.spec.ts` logs the same field from the
//!    compiled wasm module, which vitest prints for a passing test.
//! 3. `record_omp1_derived_baseline` — the re-record command — prints the
//!    identical comparison before it emits a document, because re-recording
//!    the derived hashes is exactly the moment a contributor is deciding
//!    whether a behavior change was intended.
//!
//! A drift entry is *not* self-evidently fine. Read it the way a drifted
//! boundary hash is read: intended, or a finding? If it was intended, say so
//! in the PR that causes it, with the recorded value and the new one — that is
//! the #500 rule (the decision, the superseding change, and the last commit at
//! which the previous claim held), and it is now the only thing standing where
//! an `Err` used to.
//!
//! The fixture's recorded claims are **never refreshed** — `rerecorded_json`
//! still copies `expected_score`, `event_counts` and `windows` verbatim and
//! verifies it did. So drift is measured against the original recording for
//! good, not against the last build that happened to run.

use crate::fixed_clock;
use crate::input_frame::{
    self, InputFixtureRosters, InputFrame, InputOwnership, InputSlotAssignment,
};
use crate::input_tape::{self, InputTape, InputTapeIdentity};
use crate::keeper::KeeperShotType;
use crate::r#match as sim_match;
use crate::match_snapshot::{self, MatchEventKind, MatchSnapshot, MatchState, PitchSize};
use crate::tuning::Tuning;
use gc_core::fnv1a64::Fnv1a64State;
use gc_data::omp1_determinism;
use gc_data::teams;
use indexmap::IndexMap;
use std::collections::BTreeMap;

const LEGACY_FIXTURE_ID: &str = "omp1-nebula-orion-eight-streams-v1";
const MIGRATED_FIXTURE_ID: &str = "omp1-nebula-orion-eight-streams-v2";
const LEGACY_MAX_WIRE_BYTES: usize = 148;
const LEGACY_MAX_HELD_MASK: i64 = 127;
const LEGACY_MAX_EDGE_MASK: i64 = 31;

/// Which of the fixture's five headline behaviors a campaign observed.
///
/// **This is the gated half of #505's split.** It answers "did a tackle, a
/// catch, a header and a full time still happen?", which is what keeps the
/// fixture from degenerating into a replay of a match where nothing occurs.
/// It deliberately says nothing about how many or when — that is
/// [`BehaviorClaims`], and it is reported rather than gated.
///
/// `goal_kickoff` is the one field the frozen fixture does not reach: its
/// predicate is written against an *away* goal and this recording's only goal
/// is the home side's, so a campaign leaves it `false` (pinned as such by
/// `tests/determinism_evidence.rs`) and `finish_campaign` does not require
/// it. The goal/kickoff behavior is carried by the bounded synthetic tape in
/// `docs/online/snapshot_replay.md` instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DeterminismCoverage {
    /// A tackle occurred.
    pub tackle: bool,
    /// A keeper catch occurred.
    pub keeper: bool,
    /// A header occurred.
    pub aerial: bool,
    /// A goal was scored and the resulting kickoff hold observed.
    pub goal_kickoff: bool,
    /// The match reached full time.
    pub full_time: bool,
}

/// The frozen fixture's behavioral claims about the match it recorded, and
/// the same shape for what a live campaign observed. One type for both sides
/// on purpose: [`behavioral_drift`] compares two of these, and a field that
/// exists on only one side is a field nothing can compare.
///
/// None of this is gated (#505). See the module doc's "What a campaign gates
/// on, and what it only reports".
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BehaviorClaims {
    /// Full-time home score.
    pub score_home: i64,
    /// Full-time away score.
    pub score_away: i64,
    /// Count of each event kind, including the synthetic `chip` bucket.
    /// Sorted, so a rendered report is stable.
    pub event_counts: BTreeMap<String, i64>,
    /// Per window name, the causal tick that window's subject was seen on —
    /// the first tick its scoped `event_kind` fired, or for a window with no
    /// scoped kind, the first tick the match was finished. `None` means the
    /// subject was never seen. In the fixture's own window order.
    pub window_event_ticks: IndexMap<String, Option<i64>>,
}

/// One behavioral claim of the frozen fixture that this build no longer
/// reproduces: what was recorded, and what actually happened.
///
/// Reported, never an `Err` — but never silent either. See the module doc's
/// "How a contributor is meant to notice a reported drift".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BehavioralDrift {
    /// Which claim moved: `score`, `event_counts.<kind>`, or
    /// `windows.<name>.event_tick`.
    pub claim: String,
    /// The value the frozen fixture records.
    pub recorded: String,
    /// The value this build produced.
    pub observed: String,
}

/// The winner of a completed OMP-1 recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Home won.
    Home,
    /// Away won.
    Away,
    /// The match was drawn.
    Draw,
}

/// A completed campaign's evidence summary.
#[derive(Clone, Debug, PartialEq)]
pub struct DeterminismEvidenceResult {
    /// The fixture's stable identity.
    pub fixture_id: String,
    /// Ticks simulated.
    pub ticks: i64,
    /// Boundaries recorded (`ticks + 1`).
    pub boundaries: i64,
    /// The final canonical snapshot hash.
    pub final_hash: String,
    /// The FNV-1a-64 digest over every boundary hash in sequence.
    pub sequence_digest: String,
    /// Final home score.
    pub score_home: i64,
    /// Final away score.
    pub score_away: i64,
    /// The match's outcome.
    pub outcome: Outcome,
    /// Encoded byte length of the final canonical snapshot.
    pub snapshot_bytes: i64,
    /// Which headline behaviors were observed. **Gated** — see
    /// [`DeterminismCoverage`].
    pub coverage: DeterminismCoverage,
    /// What this campaign actually observed, in the same shape as the
    /// fixture's recorded claims. Reported, not gated.
    pub observed: BehaviorClaims,
    /// Every recorded behavioral claim this build no longer reproduces.
    /// Empty when the campaign still matches the recording. Reported, not
    /// gated — and the reason the demotion is not a deletion.
    pub drift: Vec<BehavioralDrift>,
}

/// A fresh replay of the frozen fixture, folded into fresh recording
/// artifacts (frame wires, boundary hashes, first-occurrence event ticks,
/// and event counts).
#[derive(Clone, Debug, PartialEq)]
pub struct Omp1Recording {
    /// One canonical wire encoding per consumed frame.
    pub frame_wires: Vec<String>,
    /// One boundary hash per frame, plus the initial boundary.
    pub boundary_hashes: Vec<String>,
    /// The first tick each event kind (or synthetic `goal_kickoff` /
    /// `full_time` marker) was observed on.
    pub event_ticks: IndexMap<String, i64>,
    /// Count of each event kind observed, including the synthetic `chip`
    /// bucket.
    pub event_counts: IndexMap<String, i64>,
    /// Final home score.
    pub score_home: i64,
    /// Final away score.
    pub score_away: i64,
}

/// Incremental verification state for one OMP-1 determinism campaign.
/// Advance it with [`step_campaign`] until it yields a
/// [`DeterminismEvidenceResult`].
#[derive(Clone, Debug, PartialEq)]
pub struct DeterminismCampaign {
    /// The fixture's decoded frames, in tick order.
    pub frames: Vec<InputFrame>,
    /// The fixture's pinned boundary hashes, one more than `frames`.
    pub expected_hashes: Vec<String>,
    /// The primary, authoritative replay.
    pub reference: MatchState,
    /// An independent fresh replay, when `compare_fresh` was set; must
    /// agree with `reference` at every boundary.
    pub candidate: Option<MatchState>,
    /// Running FNV-1a-64 digest over every boundary hash in sequence.
    pub sequence: Fnv1a64State,
    /// Captured snapshots at every window's first boundary, indexed
    /// directly by boundary number.
    pub snapshots: Vec<Option<MatchSnapshot>>,
    /// Which boundary numbers a window starts at.
    pub window_starts: Vec<bool>,
    /// Headline behaviors observed so far.
    pub coverage: DeterminismCoverage,
    /// Event kind (wire string, plus the synthetic `chip` bucket) -> count
    /// observed so far.
    pub event_counts: IndexMap<String, i64>,
    /// Next 0-based frame index (== causal tick) [`step_campaign`] will
    /// consume.
    pub next_index: i64,
    /// The finished result, once every frame has been consumed.
    pub result: Option<DeterminismEvidenceResult>,
}

fn convert_ownership(o: &omp1_determinism::InputOwnership) -> Result<InputOwnership, String> {
    if o.rosters.home.len() != input_frame::FIXTURE_TEAM_SIZE as usize
        || o.rosters.away.len() != input_frame::FIXTURE_TEAM_SIZE as usize
    {
        return Err("fixture ownership roster size mismatch".to_string());
    }
    if o.slots.len() != input_frame::SLOT_COUNT as usize {
        return Err("fixture ownership slot count mismatch".to_string());
    }
    let mut slots = Vec::with_capacity(8);
    for (index, slot) in o.slots.iter().enumerate() {
        let canonical = input_frame::slot(index as i64 + 1).map_err(|e| e.to_string())?;
        slots.push(InputSlotAssignment {
            slot: canonical.id,
            team: canonical.team,
            player_id: slot.player_id.clone(),
        });
    }
    let slots: [InputSlotAssignment; 8] = slots
        .try_into()
        .map_err(|_| "fixture ownership must have exactly eight slots".to_string())?;
    Ok(InputOwnership {
        version: o.version,
        rosters: InputFixtureRosters {
            home: o.rosters.home.clone(),
            away: o.rosters.away.clone(),
        },
        slots,
    })
}

/// Convert the JSON-shaped fixture identity into this crate's
/// [`InputTapeIdentity`]. See the module doc's "Where the fixture data
/// comes from" section.
fn to_tape_identity(
    source: &omp1_determinism::InputTapeIdentity,
) -> Result<InputTapeIdentity, String> {
    Ok(InputTapeIdentity {
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
        ownership: convert_ownership(&source.ownership)?,
        combat: None,
    })
}

fn fixture_identity() -> Result<InputTapeIdentity, String> {
    to_tape_identity(&omp1_determinism::fixture().identity)
}

/// Migrate `source` to the current input/snapshot schema, preserving every
/// other identity field. Only a frozen `input_version == 1` fixture also
/// gets its `fixture` id rewritten to [`MIGRATED_FIXTURE_ID`].
///
/// # Errors
///
/// Returns `Err` if `source`'s input version is unsupported, its ownership
/// version disagrees with its input version, or (for a legacy source) its
/// fixture id is not [`LEGACY_FIXTURE_ID`].
pub fn migration_identity(source: &InputTapeIdentity) -> Result<InputTapeIdentity, String> {
    if source.input_version != 1 && source.input_version != input_frame::VERSION {
        return Err("unsupported fixture input version".to_string());
    }
    if source.ownership.version != source.input_version {
        return Err("fixture ownership version disagrees with input version".to_string());
    }
    if source.input_version == 1 && source.fixture != LEGACY_FIXTURE_ID {
        return Err("unsupported legacy fixture identity".to_string());
    }

    let mut candidate = source.clone();
    candidate.ownership.version = input_frame::VERSION;
    candidate.input_version = input_frame::VERSION;
    candidate.snapshot_version = match_snapshot::VERSION;
    let mut migrated = input_tape::copy_identity(&candidate)?;
    if source.input_version == 1 {
        migrated.fixture = MIGRATED_FIXTURE_ID.to_string();
    }
    Ok(migrated)
}

/// Migrate one legacy (`input_version == 1`) canonical fixture wire to the
/// current wire format. Deliberately narrower than a runtime v1 decoder: it
/// accepts only a canonical frozen-fixture wire whose masks and byte size
/// were legal under v1, then changes the version header so the current
/// decoder can validate it.
///
/// # Errors
///
/// Returns `Err` if `wire` exceeds the v1 wire bound, has the wrong field
/// count, is not tagged version 1, any sample's held/edge mask exceeds its
/// v1 bound, or the migrated wire does not decode canonically.
pub fn migrate_legacy_fixture_wire(wire: &str) -> Result<String, String> {
    if wire.len() > LEGACY_MAX_WIRE_BYTES {
        return Err("legacy fixture frame exceeds the v1 wire bound".to_string());
    }
    let fields: Vec<&str> = wire.split('|').collect();
    if fields.len() != input_frame::SLOT_COUNT as usize + 2 {
        return Err("legacy fixture frame has invalid fields".to_string());
    }
    if fields[0] != "1" {
        return Err("legacy fixture frame has an invalid version".to_string());
    }
    for field in &fields[2..] {
        let parts: Vec<&str> = field.split(',').collect();
        if parts.len() != 4
            || !is_signed_int(parts[0])
            || !is_signed_int(parts[1])
            || !is_unsigned_int(parts[2])
            || !is_unsigned_int(parts[3])
        {
            return Err("legacy fixture sample has invalid fields".to_string());
        }
        let held: i64 = parts[2].parse().expect("checked by is_unsigned_int");
        let edges: i64 = parts[3].parse().expect("checked by is_unsigned_int");
        if held > LEGACY_MAX_HELD_MASK {
            return Err("legacy fixture held mask exceeds v1".to_string());
        }
        if edges > LEGACY_MAX_EDGE_MASK {
            return Err("legacy fixture edge mask exceeds v1".to_string());
        }
    }

    let canonical_wire = format!("{}{}", input_frame::VERSION, &wire[1..]);
    input_frame::decode(&canonical_wire)
        .map_err(|e| format!("legacy fixture frame is not canonical: {e}"))?;
    Ok(canonical_wire)
}

fn is_signed_int(s: &str) -> bool {
    let digits = s.strip_prefix('-').unwrap_or(s);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

fn is_unsigned_int(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn new_state(identity: &InputTapeIdentity) -> Result<MatchState, String> {
    let identity = input_tape::copy_identity(identity)?;
    let state = sim_match::new(sim_match::NewMatchOptions {
        home: teams::get("nebula").expect("nebula is an authored team"),
        away: teams::get("orion").expect("orion is an authored team"),
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(omp1_determinism::fixture().duration_seconds as f64),
        max_goals: Some(3),
        seed: Some(identity.seed),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(identity.ownership),
    });
    Ok(state)
}

fn state_hash(state: &MatchState) -> String {
    match_snapshot::hash(&match_snapshot::capture(state, None))
}

/// Fold one boundary hash into a running sequence digest. The single
/// definition of how a boundary enters that digest: a campaign folds
/// boundaries in as it reaches them, [`sequence_digest`] folds a finished
/// vector, and a re-record that disagreed with the campaign here would pin a
/// digest the gate could never reproduce.
fn fold_boundary_hash(sequence: &mut Fnv1a64State, hash: &str) {
    sequence.update(format!("{hash}\n").as_bytes());
}

/// The FNV-1a-64 digest over `boundary_hashes` in sequence — the value the
/// fixture pins as `expected_sequence_digest`, computed from a complete
/// vector rather than incrementally.
///
/// Exists for the re-record path (see [`record`]): a campaign builds this
/// digest one boundary at a time, but a recorder has the whole vector at
/// once. Both go through [`fold_boundary_hash`], so there is one rule.
#[must_use]
pub fn sequence_digest(boundary_hashes: &[String]) -> String {
    let mut sequence = Fnv1a64State::new();
    for hash in boundary_hashes {
        fold_boundary_hash(&mut sequence, hash);
    }
    sequence.hex()
}

fn has_event(state: &MatchState, kind: MatchEventKind) -> bool {
    state.events.iter().any(|e| e.kind == kind)
}

fn outcome(home: i64, away: i64) -> Outcome {
    if home > away {
        Outcome::Home
    } else if away > home {
        Outcome::Away
    } else {
        Outcome::Draw
    }
}

/// The behavioral claims the frozen fixture records: its `expected_score`,
/// its `event_counts`, and each window's `event_tick`. The recorded side of
/// [`behavioral_drift`].
///
/// These are the fields `gc_data::omp1_determinism::rerecorded_json` refuses
/// to move, so this is a fixed point: drift is always measured against the
/// original recording, never against whatever the last build produced.
#[must_use]
pub fn fixture_claims() -> BehaviorClaims {
    let fixture = omp1_determinism::fixture();
    BehaviorClaims {
        score_home: fixture.expected_score.home,
        score_away: fixture.expected_score.away,
        event_counts: fixture.event_counts.clone(),
        window_event_ticks: fixture
            .windows
            .iter()
            .map(|window| (window.name.clone(), window.event_tick))
            .collect(),
    }
}

fn render_tick(tick: Option<i64>) -> String {
    match tick {
        Some(tick) => tick.to_string(),
        None => "none".to_string(),
    }
}

/// Every claim on which `recorded` and `observed` disagree, in a stable
/// order: the score, then event counts by kind, then window event ticks in
/// the fixture's own window order.
///
/// Pure over its two arguments and takes no fixture: that is what lets the
/// standing tests in `tests/determinism_evidence.rs` prove this reporter can
/// name each class of drift without replaying 7,201 ticks — the reporter is
/// what now stands where an `Err` used to, so it is the thing that has to be
/// demonstrated working (AGENTS.md §9).
#[must_use]
pub fn behavioral_drift(
    recorded: &BehaviorClaims,
    observed: &BehaviorClaims,
) -> Vec<BehavioralDrift> {
    let mut drift = Vec::new();
    if recorded.score_home != observed.score_home || recorded.score_away != observed.score_away {
        drift.push(BehavioralDrift {
            claim: "score".to_string(),
            recorded: format!("{}-{}", recorded.score_home, recorded.score_away),
            observed: format!("{}-{}", observed.score_home, observed.score_away),
        });
    }
    for (name, count) in &recorded.event_counts {
        let actual = observed.event_counts.get(name).copied();
        if actual != Some(*count) {
            drift.push(BehavioralDrift {
                claim: format!("event_counts.{name}"),
                recorded: count.to_string(),
                observed: match actual {
                    Some(actual) => actual.to_string(),
                    None => "absent".to_string(),
                },
            });
        }
    }
    for (name, count) in &observed.event_counts {
        if !recorded.event_counts.contains_key(name) {
            drift.push(BehavioralDrift {
                claim: format!("event_counts.{name}"),
                recorded: "absent".to_string(),
                observed: count.to_string(),
            });
        }
    }
    for (name, tick) in &recorded.window_event_ticks {
        let actual = observed.window_event_ticks.get(name).copied().flatten();
        if actual != *tick {
            drift.push(BehavioralDrift {
                claim: format!("windows.{name}.event_tick"),
                recorded: render_tick(*tick),
                observed: render_tick(actual),
            });
        }
    }
    drift
}

/// The headline behaviors a campaign observed, comma-joined in a fixed order.
/// The single rendering of the **gated** half of #505's split: [`report`]'s
/// `coverage=` field, `gc_wasm::determinism`'s `coverage`, and therefore
/// `scripts/check.sh`'s pinned `EXPECTED_COVERAGE` all go through here, so
/// there is one order and one spelling.
#[must_use]
pub fn coverage_list(coverage: &DeterminismCoverage) -> String {
    [
        ("goal_kickoff", coverage.goal_kickoff),
        ("tackle", coverage.tackle),
        ("aerial", coverage.aerial),
        ("keeper", coverage.keeper),
        ("full_time", coverage.full_time),
    ]
    .into_iter()
    .filter(|(_, observed)| *observed)
    .map(|(name, _)| name)
    .collect::<Vec<&str>>()
    .join(",")
}

/// Render a drift list as one field-safe line: `none`, or
/// `claim:recorded->observed` entries joined by `;`.
///
/// Contains no `|`, so it can ride inside the pipe-delimited
/// `GC_DETERMINISM` line [`report`] emits and the terminator
/// `scripts/check.sh` parses.
#[must_use]
pub fn render_drift(drift: &[BehavioralDrift]) -> String {
    if drift.is_empty() {
        return "none".to_string();
    }
    drift
        .iter()
        .map(|entry| format!("{}:{}->{}", entry.claim, entry.recorded, entry.observed))
        .collect::<Vec<String>>()
        .join(";")
}

fn fixture_frames() -> Result<(Vec<InputFrame>, Vec<String>), String> {
    let fixture = omp1_determinism::fixture();
    let wires = omp1_determinism::frame_wire_lines();
    if wires.len() as i64 != fixture.frame_count {
        return Err("fixture frame count does not match its recording".to_string());
    }
    let mut frames = Vec::with_capacity(wires.len());
    let mut canonical_wires = Vec::with_capacity(wires.len());
    for (index, wire) in wires.iter().enumerate() {
        let canonical_wire = if fixture.identity.input_version == 1 {
            migrate_legacy_fixture_wire(wire)?
        } else {
            (*wire).to_string()
        };
        let decoded = input_frame::decode(&canonical_wire)
            .map_err(|e| format!("fixture frame {index} is malformed: {e}"))?;
        if decoded.tick != index as i64 {
            return Err("fixture frames are not contiguous from tick zero".to_string());
        }
        frames.push(decoded);
        canonical_wires.push(canonical_wire);
    }
    Ok((frames, canonical_wires))
}

/// Return the checked-in OMP-1 fixture as a validated, already-materialized
/// tape. Rollback laboratories consume this seam instead of private
/// campaign state or the bots that originally produced the frozen frame
/// wires.
///
/// # Errors
///
/// Returns `Err` if the fixture fails to migrate, decode, or reconstruct
/// into a valid [`InputTape`].
pub fn fixture_tape(tune: &Tuning) -> Result<InputTape, String> {
    let identity = migration_identity(&fixture_identity()?)?;
    let (frames, _wires) = fixture_frames()?;
    let boundary_hashes: Vec<String> = omp1_determinism::boundary_hash_lines()
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let state = new_state(&identity)?;
    let initial = match_snapshot::capture(&state, None);
    input_tape::from_frozen_recording(&identity, &initial, &frames, &boundary_hashes, tune)
}

/// Replay one restore window from its captured snapshot, returning the causal
/// tick the window's subject was observed on.
///
/// **Gated:** every boundary in the window replaying to its pinned hash (the
/// determinism property), and the window still containing the event it is
/// scoped around — a window that stopped covering its behavior is a window
/// that proves nothing.
///
/// **Reported, not gated (#505):** *when* that subject fired. The returned
/// tick is compared against the fixture's recorded `event_tick` by
/// [`behavioral_drift`], not here.
fn verify_window(
    snapshots: &[Option<MatchSnapshot>],
    frames: &[InputFrame],
    expected_hashes: &[String],
    window: &omp1_determinism::Omp1Window,
    tune: &Tuning,
) -> Result<Option<i64>, String> {
    let initial = snapshots[window.first_boundary as usize]
        .as_ref()
        .ok_or_else(|| format!("missing snapshot for {} window", window.name))?;
    if window.event_kind.is_some() && window.event_tick.is_none() {
        return Err(format!("{} window needs event_tick", window.name));
    }
    let (mut state, mut combat) = match_snapshot::restore(initial);
    let mut saw_expected_event = window.event_kind.is_none();
    let mut observed_tick: Option<i64> = None;
    for boundary in (window.first_boundary + 1)..=window.last_boundary {
        let causal_tick = boundary - 1;
        let frame = &frames[causal_tick as usize];
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            sim_match::StepInput::Frame(frame),
            combat.as_mut(),
            tune,
        );
        let actual = state_hash(&state);
        let expected = &expected_hashes[boundary as usize];
        if &actual != expected {
            return Err(format!(
                "{} restore/replay diverged at causal tick {causal_tick}: expected {expected}, got {actual}",
                window.name
            ));
        }
        match &window.event_kind {
            Some(kind_str) => {
                if state.events.iter().any(|e| e.kind.wire_str() == kind_str) {
                    saw_expected_event = true;
                    observed_tick.get_or_insert(causal_tick);
                }
            }
            // A window with no scoped event kind is scoped around full time
            // instead; its subject is the tick the match finished on.
            None => {
                if state.finished {
                    observed_tick.get_or_insert(causal_tick);
                }
            }
        }
    }
    if !saw_expected_event {
        return Err(format!(
            "{} restore/replay missed its required event",
            window.name
        ));
    }
    if window.name == "goal_kickoff" {
        // "A goal was scored and its kickoff hold survived the restore" —
        // the behavior, not the scoreline. Deliberately `< 1` rather than
        // `!= 1` since #505: a gameplay change that produces a *second* away
        // goal inside this window has not stopped covering the behavior, and
        // exactly-how-many is the reported half of the split.
        if state.score.away < 1 {
            return Err("goal window did not preserve the away goal".to_string());
        }
        if state.kickoff_hold <= 0.0 {
            return Err("goal window did not preserve the home kickoff".to_string());
        }
    } else if window.name == "full_time" && !(state.finished && state.time_left == 0.0) {
        return Err("full-time window did not finish".to_string());
    }
    Ok(observed_tick)
}

/// Construct a fresh campaign against the checked-in OMP-1 fixture.
/// `compare_fresh` additionally runs a second, independent replay that
/// must agree with the primary one at every boundary.
///
/// # Errors
///
/// Returns `Err` if the fixture, its identity, or the active `tune`
/// disagree with what the campaign expects.
pub fn new_campaign(compare_fresh: bool, tune: &Tuning) -> Result<DeterminismCampaign, String> {
    let fixture = omp1_determinism::fixture();
    if fixture.version != 1 {
        return Err("unsupported OMP-1 determinism fixture".to_string());
    }
    let identity = migration_identity(&fixture_identity()?)?;
    if identity.tick_rate != fixed_clock::TICK_RATE as i64 {
        return Err("fixture tick rate drifted".to_string());
    }
    if identity.tuning != tune.serialize() {
        return Err("fixture tuning identity drifted".to_string());
    }
    if identity.fixture != MIGRATED_FIXTURE_ID {
        return Err("fixture identity disagrees with fixture id".to_string());
    }
    let (frames, wires) = fixture_frames()?;
    let expected_hashes: Vec<String> = omp1_determinism::boundary_hash_lines()
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    if expected_hashes.len() as i64 != fixture.boundary_count {
        return Err("fixture boundary count does not match its baseline".to_string());
    }
    if expected_hashes.len() != wires.len() + 1 {
        return Err("fixture needs one hash per boundary".to_string());
    }

    let reference = new_state(&identity)?;
    let candidate = if compare_fresh {
        Some(new_state(&identity)?)
    } else {
        None
    };
    let initial_hash = state_hash(&reference);
    if let Some(candidate) = &candidate
        && initial_hash != state_hash(candidate)
    {
        return Err("fresh matches disagree at boundary zero".to_string());
    }
    if initial_hash != expected_hashes[0] {
        return Err(format!(
            "pinned boundary 0 drifted: expected {}, got {initial_hash}",
            expected_hashes[0]
        ));
    }

    let mut sequence = Fnv1a64State::new();
    fold_boundary_hash(&mut sequence, &initial_hash);
    let boundary_count = expected_hashes.len();
    let mut snapshots: Vec<Option<MatchSnapshot>> = vec![None; boundary_count];
    let mut window_starts = vec![false; boundary_count];
    for window in &fixture.windows {
        window_starts[window.first_boundary as usize] = true;
    }
    if window_starts[0] {
        snapshots[0] = Some(match_snapshot::capture(&reference, None));
    }

    Ok(DeterminismCampaign {
        frames,
        expected_hashes,
        reference,
        candidate,
        sequence,
        snapshots,
        window_starts,
        coverage: DeterminismCoverage::default(),
        event_counts: IndexMap::new(),
        next_index: 0,
        result: None,
    })
}

fn finish_campaign(
    campaign: &DeterminismCampaign,
    tune: &Tuning,
) -> Result<DeterminismEvidenceResult, String> {
    let fixture = omp1_determinism::fixture();
    let reference = &campaign.reference;
    if !reference.finished {
        return Err("recording did not reach full time".to_string());
    }
    if reference.input_tick != fixture.frame_count {
        return Err("recording ended at the wrong tick".to_string());
    }
    let final_hash = state_hash(reference);
    let sequence_digest = campaign.sequence.hex();
    if final_hash != fixture.expected_final_hash {
        return Err("fixture final hash drifted".to_string());
    }
    if sequence_digest != fixture.expected_sequence_digest {
        return Err("fixture sequence digest drifted".to_string());
    }

    for (name, covered) in [
        ("tackle", campaign.coverage.tackle),
        ("keeper", campaign.coverage.keeper),
        ("aerial", campaign.coverage.aerial),
        ("full_time", campaign.coverage.full_time),
    ] {
        if !covered {
            return Err(format!("fixture did not cover {name}"));
        }
    }
    let mut window_event_ticks: IndexMap<String, Option<i64>> = IndexMap::new();
    for window in &fixture.windows {
        let observed_tick = verify_window(
            &campaign.snapshots,
            &campaign.frames,
            &campaign.expected_hashes,
            window,
            tune,
        )?;
        window_event_ticks.insert(window.name.clone(), observed_tick);
    }

    // The reported half of #505's split: how many of each event fired, what
    // the score ended, and when each window's subject fired. Compared against
    // the frozen recording and surfaced, never returned as an `Err`. See the
    // module doc for the three channels this reaches a human through.
    let observed = BehaviorClaims {
        score_home: reference.score.home,
        score_away: reference.score.away,
        event_counts: campaign
            .event_counts
            .iter()
            .map(|(name, count)| (name.clone(), *count))
            .collect(),
        window_event_ticks,
    };
    let drift = behavioral_drift(&fixture_claims(), &observed);

    Ok(DeterminismEvidenceResult {
        fixture_id: fixture.fixture_id.clone(),
        ticks: fixture.frame_count,
        boundaries: fixture.boundary_count,
        final_hash,
        sequence_digest,
        score_home: reference.score.home,
        score_away: reference.score.away,
        outcome: outcome(reference.score.home, reference.score.away),
        snapshot_bytes: match_snapshot::encode(&match_snapshot::capture(reference, None)).len()
            as i64,
        coverage: campaign.coverage,
        observed,
        drift,
    })
}

/// Advance `campaign` by up to `max_ticks` frames, returning the finished
/// result once every frame has been consumed (and on every call
/// thereafter).
///
/// # Errors
///
/// Returns `Err` if `max_ticks` is not positive, an independent candidate
/// replay diverges from the reference, a boundary hash disagrees with the
/// fixture's pinned value, or the finished campaign fails any *gated*
/// evidence check. A moved score, event count or window event tick is
/// reported on the result rather than returned here — see the module doc's
/// "What a campaign gates on, and what it only reports".
pub fn step_campaign(
    campaign: &mut DeterminismCampaign,
    max_ticks: i64,
    tune: &Tuning,
) -> Result<Option<DeterminismEvidenceResult>, String> {
    if max_ticks <= 0 {
        return Err("max_ticks must be positive".to_string());
    }
    if let Some(result) = &campaign.result {
        return Ok(Some(result.clone()));
    }
    let last_index = (campaign.frames.len() as i64 - 1).min(campaign.next_index + max_ticks - 1);
    let mut index = campaign.next_index;
    while index <= last_index {
        let frame = campaign.frames[index as usize];
        let causal_tick = index;
        sim_match::step(
            &mut campaign.reference,
            fixed_clock::TICK_SECONDS,
            sim_match::StepInput::Frame(&frame),
            None,
            tune,
        );
        if let Some(candidate) = campaign.candidate.as_mut() {
            sim_match::step(
                candidate,
                fixed_clock::TICK_SECONDS,
                sim_match::StepInput::Frame(&frame),
                None,
                tune,
            );
        }
        let reference_hash = state_hash(&campaign.reference);
        if let Some(candidate) = &campaign.candidate {
            let candidate_hash = state_hash(candidate);
            if reference_hash != candidate_hash {
                return Err(format!(
                    "independent runs diverged after causal tick {causal_tick}: reference {reference_hash}, candidate {candidate_hash}"
                ));
            }
        }
        let expected = &campaign.expected_hashes[(index + 1) as usize];
        if &reference_hash != expected {
            return Err(format!(
                "pinned boundary {} drifted after causal tick {causal_tick}: expected {expected}, got {reference_hash}",
                index + 1
            ));
        }
        fold_boundary_hash(&mut campaign.sequence, &reference_hash);
        let boundary_number = (index + 1) as usize;
        if campaign.window_starts[boundary_number] {
            campaign.snapshots[boundary_number] =
                Some(match_snapshot::capture(&campaign.reference, None));
        }
        if has_event(&campaign.reference, MatchEventKind::Tackle) {
            campaign.coverage.tackle = true;
        }
        if has_event(&campaign.reference, MatchEventKind::Catch) {
            campaign.coverage.keeper = true;
        }
        if has_event(&campaign.reference, MatchEventKind::Header) {
            campaign.coverage.aerial = true;
        }
        for event in &campaign.reference.events {
            let key = event.kind.wire_str().to_string();
            *campaign.event_counts.entry(key).or_insert(0) += 1;
            if event.kind == MatchEventKind::Shot && event.shot_type == Some(KeeperShotType::Chip) {
                *campaign.event_counts.entry("chip".to_string()).or_insert(0) += 1;
            }
        }
        if campaign.reference.score.away > 0 && campaign.reference.kickoff_hold > 0.0 {
            campaign.coverage.goal_kickoff = true;
        }
        if campaign.reference.finished && campaign.reference.time_left == 0.0 {
            campaign.coverage.full_time = true;
        }
        index += 1;
    }
    campaign.next_index = last_index + 1;
    if campaign.next_index >= campaign.frames.len() as i64 {
        campaign.result = Some(finish_campaign(campaign, tune)?);
    }
    Ok(campaign.result.clone())
}

/// Run a complete OMP-1 determinism campaign (with an independent fresh
/// comparison run) to its finished result.
///
/// A successful result is not a claim that nothing changed: read its `drift`
/// (or [`report`]'s `drift=` field), which names every recorded behavioral
/// claim this build no longer reproduces.
///
/// # Errors
///
/// Returns `Err` on the first *gated* evidence check that fails; see
/// [`new_campaign`] and [`step_campaign`].
pub fn verify(tune: &Tuning) -> Result<DeterminismEvidenceResult, String> {
    let mut campaign = new_campaign(true, tune)?;
    let frame_count = omp1_determinism::fixture().frame_count;
    loop {
        if let Some(result) = step_campaign(&mut campaign, frame_count, tune)? {
            return Ok(result);
        }
    }
}

/// Replay the frozen fixture frames from scratch, confirming each one
/// still encodes canonically, and fold the result into an
/// [`Omp1Recording`].
///
/// This is the derivation half of a re-record: the returned
/// `boundary_hashes` are what the current simulation reaches from the frozen
/// input, and it never compares them against the pinned baseline (that is
/// [`verify`]'s job — a recorder that refused to record a drifted hash could
/// not record the only case anyone needs it for). The write-back is
/// `gc_data::omp1_determinism::rerecorded_json`, and the human-run command
/// is `record_omp1_derived_baseline` in `tests/determinism_evidence.rs`.
/// Read the module doc's "What this module can and cannot regenerate"
/// before running it.
///
/// # Errors
///
/// Returns `Err` if the fixture's identity disagrees with the active
/// `tune`, a frozen frame fails canonical replay, the match finishes
/// before every frozen frame is consumed, or vice versa.
pub fn record(tune: &Tuning) -> Result<Omp1Recording, String> {
    let identity = migration_identity(&fixture_identity()?)?;
    if identity.tick_rate != fixed_clock::TICK_RATE as i64 {
        return Err("fixture tick rate drifted".to_string());
    }
    if identity.tuning != tune.serialize() {
        return Err("fixture tuning identity drifted".to_string());
    }
    if identity.fixture != MIGRATED_FIXTURE_ID {
        return Err("fixture identity disagrees with fixture id".to_string());
    }
    let mut state = new_state(&identity)?;
    let (frozen_frames, frozen_wires) = fixture_frames()?;
    let mut frame_wires = Vec::new();
    let mut boundary_hashes = vec![state_hash(&state)];
    let mut event_ticks: IndexMap<String, i64> = IndexMap::new();
    let mut event_counts: IndexMap<String, i64> = IndexMap::new();
    while !state.finished {
        let tick = state.input_tick;
        let frame = *frozen_frames
            .get(tick as usize)
            .ok_or("frozen fixture frames were exhausted before full time")?;
        let wire = input_frame::encode(&frame).map_err(|e| e.to_string())?;
        if wire != frozen_wires[tick as usize] {
            return Err("frozen fixture frame failed canonical replay".to_string());
        }
        frame_wires.push(wire);
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            sim_match::StepInput::Frame(&frame),
            None,
            tune,
        );
        boundary_hashes.push(state_hash(&state));
        for event in &state.events {
            let key = event.kind.wire_str().to_string();
            *event_counts.entry(key.clone()).or_insert(0) += 1;
            if event.kind == MatchEventKind::Shot && event.shot_type == Some(KeeperShotType::Chip) {
                *event_counts.entry("chip".to_string()).or_insert(0) += 1;
            }
            event_ticks.entry(key).or_insert(tick);
        }
        if state.score.away > 0
            && state.kickoff_hold > 0.0
            && !event_ticks.contains_key("goal_kickoff")
        {
            event_ticks.insert("goal_kickoff".to_string(), tick);
        }
        if state.finished && !event_ticks.contains_key("full_time") {
            event_ticks.insert("full_time".to_string(), tick);
        }
    }
    if frame_wires.len() != frozen_frames.len() {
        return Err("full time arrived before every frozen fixture frame was consumed".to_string());
    }
    if state.input_tick != omp1_determinism::fixture().frame_count {
        return Err("refresh did not consume the exact frozen fixture frame count".to_string());
    }
    Ok(Omp1Recording {
        frame_wires,
        boundary_hashes,
        event_ticks,
        event_counts,
        score_home: state.score.home,
        score_away: state.score.away,
    })
}

/// Render a completed campaign's result as the pipe-delimited
/// `GC_DETERMINISM` evidence line.
///
/// `events=`, `score=` and `drift=` are **observations of this run**, not the
/// fixture's pinned claims. Before #505 the `events=` field echoed the frozen
/// `event_counts` back — harmless while a count mismatch was an `Err` nobody
/// could reach this line past, and a lie the moment that check became a
/// report.
///
/// That distinction is invisible to a campaign on a green tree, where the
/// observation and the fixture are equal by construction, so it is pinned by
/// `the_report_carries_this_runs_counts_and_score_not_the_frozen_fixtures` in
/// this crate's `tests/determinism_evidence.rs` — which feeds `report` a
/// deliberately divergent observation. Change either field's source and that
/// test is what goes red; nothing else will.
#[must_use]
pub fn report(result: &DeterminismEvidenceResult) -> String {
    let fixture = omp1_determinism::fixture();
    let identity = &fixture.identity;
    let event_parts: Vec<String> = result
        .observed
        .event_counts
        .iter()
        .map(|(name, count)| format!("{name}:{count}"))
        .collect();

    let outcome_str = match result.outcome {
        Outcome::Home => "home",
        Outcome::Away => "away",
        Outcome::Draw => "draw",
    };
    let tuning_str = if identity.tuning.is_empty() {
        "defaults".to_string()
    } else {
        identity.tuning.clone()
    };

    [
        "GC_DETERMINISM".to_string(),
        "result".to_string(),
        "schema=1".to_string(),
        format!("fixture={}", result.fixture_id),
        format!("build={}", identity.build),
        format!("source={}", identity.source),
        format!("content={}", identity.content),
        format!("config={}", identity.config),
        format!("tuning={tuning_str}"),
        format!("seed={}", identity.seed),
        format!("tick_rate={}", fixed_clock::TICK_RATE),
        format!("ticks={}", result.ticks),
        format!("boundaries={}", result.boundaries),
        format!(
            "hash=fnv1a64-canonical-snapshot-v{}",
            match_snapshot::VERSION
        ),
        format!("final_hash={}", result.final_hash),
        format!("sequence_digest={}", result.sequence_digest),
        format!("score={}-{}", result.score_home, result.score_away),
        format!("outcome={outcome_str}"),
        format!("snapshot_bytes={}", result.snapshot_bytes),
        format!("coverage={}", coverage_list(&result.coverage)),
        format!("events={}", event_parts.join(",")),
        format!("drift={}", render_drift(&result.drift)),
    ]
    .join("|")
}
