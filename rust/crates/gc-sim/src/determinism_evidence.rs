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
//! ## What this module does not do: regenerate the fixture
//!
//! [`record`] replays the frozen frames and folds the result into an
//! [`Omp1Recording`], but this module has no function that writes a new
//! fixture back out — [`gc_data::omp1_determinism`]'s frozen JSON is
//! sourced by a one-time conversion script (see that module's doc), not by
//! anything here. Regenerating the fixture from a live match run is not
//! something this crate can do at all: the implementation this fixture was
//! originally captured from no longer exists in this repository, so the
//! checked-in fixture is frozen, permanent evidence, not a value to refresh.

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

const LEGACY_FIXTURE_ID: &str = "omp1-nebula-orion-eight-streams-v1";
const MIGRATED_FIXTURE_ID: &str = "omp1-nebula-orion-eight-streams-v2";
const LEGACY_MAX_WIRE_BYTES: usize = 148;
const LEGACY_MAX_HELD_MASK: i64 = 127;
const LEGACY_MAX_EDGE_MASK: i64 = 31;

/// Which of the fixture's five headline behaviors a campaign observed.
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
    /// Which headline behaviors were observed.
    pub coverage: DeterminismCoverage,
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

fn verify_window(
    snapshots: &[Option<MatchSnapshot>],
    frames: &[InputFrame],
    expected_hashes: &[String],
    window: &omp1_determinism::Omp1Window,
    tune: &Tuning,
) -> Result<(), String> {
    let initial = snapshots[window.first_boundary as usize]
        .as_ref()
        .ok_or_else(|| format!("missing snapshot for {} window", window.name))?;
    let (mut state, mut combat) = match_snapshot::restore(initial);
    let mut saw_expected_event = window.event_kind.is_none();
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
        if let Some(kind_str) = &window.event_kind
            && state.events.iter().any(|e| e.kind.wire_str() == kind_str)
        {
            let expected_tick = window
                .event_tick
                .ok_or_else(|| format!("{} window needs event_tick", window.name))?;
            if causal_tick != expected_tick {
                return Err(format!(
                    "{} event moved from tick {expected_tick} to {causal_tick}",
                    window.name
                ));
            }
            saw_expected_event = true;
        }
    }
    if !saw_expected_event {
        return Err(format!(
            "{} restore/replay missed its required event",
            window.name
        ));
    }
    if window.name == "goal_kickoff" {
        if state.score.away != 1 {
            return Err("goal window did not preserve the away goal".to_string());
        }
        if state.kickoff_hold <= 0.0 {
            return Err("goal window did not preserve the home kickoff".to_string());
        }
    } else if window.name == "full_time" && !(state.finished && state.time_left == 0.0) {
        return Err("full-time window did not finish".to_string());
    }
    Ok(())
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
    sequence.update(format!("{initial_hash}\n").as_bytes());
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
    if reference.score.home != fixture.expected_score.home {
        return Err("home score drifted".to_string());
    }
    if reference.score.away != fixture.expected_score.away {
        return Err("away score drifted".to_string());
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
    for (name, expected) in &fixture.event_counts {
        let actual = campaign
            .event_counts
            .get(name.as_str())
            .copied()
            .unwrap_or(0);
        if actual != *expected {
            return Err(format!(
                "fixture event count {name} drifted: expected {expected}, got {actual}"
            ));
        }
    }
    for name in campaign.event_counts.keys() {
        if !fixture.event_counts.contains_key(name.as_str()) {
            return Err(format!("fixture gained unexpected event {name}"));
        }
    }
    for window in &fixture.windows {
        verify_window(
            &campaign.snapshots,
            &campaign.frames,
            &campaign.expected_hashes,
            window,
            tune,
        )?;
    }

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
/// fixture's pinned value, or the finished campaign fails any evidence
/// check.
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
        campaign
            .sequence
            .update(format!("{reference_hash}\n").as_bytes());
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
/// # Errors
///
/// Returns `Err` on the first evidence check that fails; see
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
/// [`Omp1Recording`]. See the module doc for why this module has no
/// function that writes a new fixture back out.
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
#[must_use]
pub fn report(result: &DeterminismEvidenceResult) -> String {
    let fixture = omp1_determinism::fixture();
    let identity = &fixture.identity;
    let mut event_names: Vec<&String> = fixture.event_counts.keys().collect();
    event_names.sort();
    let event_parts: Vec<String> = event_names
        .iter()
        .map(|name| format!("{name}:{}", fixture.event_counts[name.as_str()]))
        .collect();

    let coverage_parts: Vec<&str> = ["goal_kickoff", "tackle", "aerial", "keeper", "full_time"]
        .into_iter()
        .filter(|&name| match name {
            "goal_kickoff" => result.coverage.goal_kickoff,
            "tackle" => result.coverage.tackle,
            "aerial" => result.coverage.aerial,
            "keeper" => result.coverage.keeper,
            "full_time" => result.coverage.full_time,
            _ => false,
        })
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
        format!("coverage={}", coverage_parts.join(",")),
        format!("events={}", event_parts.join(",")),
    ]
    .join("|")
}
