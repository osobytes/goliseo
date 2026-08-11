//! Pure input-tape replay and first-divergence diagnostics.
//!
//! ## Failures are reported via `?`, not a panic/unwind boundary
//!
//! `validate_context` needs to convert [`crate::input_tape`]'s validation
//! failures into a reported [`ReplayFailure`] rather than a crash — exactly
//! the boundary AGENTS.md §7 calls "validation of external input".
//! [`crate::input_tape`] makes that boundary explicit: its public functions
//! return `Result<T, String>` instead of panicking (see that module's doc
//! for why — in short, this workspace's release profile sets
//! `panic = "abort"`, so `catch_unwind` would not survive a release build
//! even if it were attempted). This module's `validate_context` is
//! therefore just `?` propagation over [`crate::input_tape`]'s own
//! `Result`s, with no unwinding involved anywhere.
//!
//! ## Tuning is an explicit parameter, not a singleton
//!
//! To confirm a tape was recorded under the active knob configuration,
//! [`run`] and [`compare`] need the tuning that was active at record time.
//! [`crate::tuning::Tuning`] is an owned value, not a singleton (see that
//! module's doc), so both functions take the caller's active `&Tuning` as
//! an explicit parameter — the same shape [`crate::input_tape`] already
//! uses.

use crate::combat_snapshot::CombatMatchState;
use crate::fixed_clock;
use crate::input_frame;
use crate::input_tape::{self, InputTape, InputTapeIdentity};
use crate::r#match as sim_match;
use crate::match_snapshot::{self, MatchState};
use crate::tuning::Tuning;

/// One recorded or replayed start/end-of-frame boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayBoundary {
    /// The `InputFrame` tick this boundary sits at.
    pub tick: i64,
    /// The boundary's canonical snapshot hash.
    pub hash: String,
}

/// The first point two states (or a tape and its own replay) disagreed.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayDivergence {
    /// The input tick whose step produced the divergence, if replay had
    /// advanced at least one frame.
    pub causal_input_tick: Option<i64>,
    /// The `input_tick` boundary the divergence was observed at.
    pub boundary_tick: i64,
    /// The expected boundary hash.
    pub expected_hash: String,
    /// The actual boundary hash.
    pub actual_hash: String,
    /// Dotted path to the first differing field, or a sentinel when no
    /// reference state was available to localize it.
    pub state_path: String,
    /// The expected value at `state_path`, rendered for display.
    pub expected_state: Option<String>,
    /// The actual value at `state_path`, rendered for display.
    pub actual_state: Option<String>,
    /// The expected frame's wire encoding, if any.
    pub expected_input: Option<String>,
    /// The actual frame's wire encoding, if any.
    pub actual_input: Option<String>,
}

/// One tape's complete replay: every boundary, and the first divergence
/// found, if any.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayResult {
    /// The final match state.
    pub state: MatchState,
    /// The final combat companion, if any.
    pub combat_state: Option<CombatMatchState>,
    /// Every boundary observed, in order.
    pub boundaries: Vec<ReplayBoundary>,
    /// The first divergence found, if any.
    pub divergence: Option<ReplayDivergence>,
}

/// The result of replaying two tapes side by side.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayComparison {
    /// Whether both tapes replayed to bit-identical boundaries throughout.
    pub equal: bool,
    /// The reference tape's replay.
    pub expected: ReplayResult,
    /// The candidate tape's replay.
    pub actual: ReplayResult,
    /// The first divergence found, if any (mirrored onto both results too).
    pub divergence: Option<ReplayDivergence>,
}

/// Why [`run`] or [`compare`] could not proceed at all (distinct from a
/// [`ReplayDivergence`], which is a successful replay that disagreed with
/// its tape).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayFailureCode {
    /// The tape or identity itself is structurally invalid.
    Malformed,
    /// The tape's identity does not match the expected identity.
    IdentityMismatch,
}

/// An expected, recoverable replay failure (ARCHITECTURE.md §3 rule 5): the caller is
/// meant to handle it, not a programmer error.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayFailure {
    /// Machine-readable failure reason.
    pub code: ReplayFailureCode,
    /// Human-readable detail.
    pub message: String,
    /// Dotted path to the mismatched identity field, for
    /// [`ReplayFailureCode::IdentityMismatch`].
    pub path: Option<String>,
    /// The expected value, rendered for display.
    pub expected: Option<String>,
    /// The actual value, rendered for display.
    pub actual: Option<String>,
}

fn malformed(message: impl Into<String>) -> ReplayFailure {
    ReplayFailure {
        code: ReplayFailureCode::Malformed,
        message: message.into(),
        path: None,
        expected: None,
        actual: None,
    }
}

fn identity_failure(
    path: impl Into<String>,
    expected: impl Into<String>,
    actual: impl Into<String>,
) -> ReplayFailure {
    let path = path.into();
    ReplayFailure {
        code: ReplayFailureCode::IdentityMismatch,
        message: format!("replay identity mismatch at {path}"),
        path: Some(path),
        expected: Some(expected.into()),
        actual: Some(actual.into()),
    }
}

/// Confirm `tape`'s identity matches `expected_identity` and the active
/// `tune`, then fully validate the tape (structure and a complete
/// consumability replay).
fn validate_context(
    tape: &InputTape,
    expected_identity: &InputTapeIdentity,
    tune: &Tuning,
) -> Result<(), ReplayFailure> {
    let copied_expected = input_tape::copy_identity(expected_identity).map_err(malformed)?;
    if tape.identity.input_version != copied_expected.input_version {
        return Err(identity_failure(
            "identity.input_version",
            copied_expected.input_version.to_string(),
            tape.identity.input_version.to_string(),
        ));
    }
    let tape_identity = input_tape::copy_identity(&tape.identity).map_err(malformed)?;
    if let Some(diff) =
        input_tape::identity_difference(&copied_expected, &tape_identity).map_err(malformed)?
    {
        return Err(identity_failure(diff.path, diff.expected, diff.actual));
    }
    let active_tuning = tune.serialize();
    if active_tuning != tape_identity.tuning {
        return Err(identity_failure(
            "identity.tuning",
            tape_identity.tuning,
            active_tuning,
        ));
    }
    input_tape::validate(tape, tune).map_err(malformed)?;
    Ok(())
}

fn boundary(state: &MatchState, combat_state: Option<&CombatMatchState>) -> ReplayBoundary {
    let snapshot = match_snapshot::capture(state, combat_state);
    ReplayBoundary {
        tick: state.input_tick,
        hash: match_snapshot::hash(&snapshot),
    }
}

fn self_divergence(
    state: &MatchState,
    expected_hash: String,
    actual_hash: String,
    causal_input_tick: Option<i64>,
    actual_input: Option<String>,
) -> ReplayDivergence {
    ReplayDivergence {
        causal_input_tick,
        boundary_tick: state.input_tick,
        expected_hash,
        actual_hash,
        state_path: "unavailable_without_reference_tape".to_string(),
        expected_state: None,
        actual_state: None,
        expected_input: None,
        actual_input,
    }
}

/// Replay `tape` against a freshly restored copy of its own initial
/// snapshot, checking every recorded boundary hash.
///
/// # Errors
///
/// Returns `Err` if `tape`'s identity does not match `expected_identity` or
/// the active `tune`, or if `tape` itself is structurally invalid.
pub fn run(
    tape: &InputTape,
    expected_identity: &InputTapeIdentity,
    tune: &Tuning,
) -> Result<ReplayResult, ReplayFailure> {
    validate_context(tape, expected_identity, tune)?;
    let (mut state, mut combat_state) = match_snapshot::restore(&tape.initial);
    let first_boundary = boundary(&state, combat_state.as_ref());
    let mut boundaries = vec![first_boundary.clone()];
    if first_boundary.hash != tape.boundary_hashes[0] {
        let divergence = self_divergence(
            &state,
            tape.boundary_hashes[0].clone(),
            first_boundary.hash,
            None,
            None,
        );
        return Ok(ReplayResult {
            state,
            combat_state,
            boundaries,
            divergence: Some(divergence),
        });
    }

    let mut divergence = None;
    for (index, frame) in tape.frames.iter().enumerate() {
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            sim_match::StepInput::Frame(frame),
            combat_state.as_mut(),
            tune,
        );
        let this_boundary = boundary(&state, combat_state.as_ref());
        let expected_hash = tape.boundary_hashes[index + 1].clone();
        let matched = this_boundary.hash == expected_hash;
        boundaries.push(this_boundary.clone());
        if !matched {
            divergence = Some(self_divergence(
                &state,
                expected_hash,
                this_boundary.hash,
                Some(frame.tick),
                Some(input_frame::encode(frame).expect("tape frame already validated")),
            ));
            break;
        }
    }
    Ok(ReplayResult {
        state,
        combat_state,
        boundaries,
        divergence,
    })
}

#[allow(clippy::too_many_arguments)]
fn compare_states(
    expected_state: &MatchState,
    actual_state: &MatchState,
    expected_combat: Option<&CombatMatchState>,
    actual_combat: Option<&CombatMatchState>,
    causal_input_tick: Option<i64>,
    expected_input: Option<String>,
    actual_input: Option<String>,
) -> ReplayDivergence {
    let expected_snapshot = match_snapshot::capture(expected_state, expected_combat);
    let actual_snapshot = match_snapshot::capture(actual_state, actual_combat);
    let expected_hash = match_snapshot::hash(&expected_snapshot);
    let actual_hash = match_snapshot::hash(&actual_snapshot);
    let found = match_snapshot::first_difference(&expected_snapshot, &actual_snapshot);
    ReplayDivergence {
        causal_input_tick,
        boundary_tick: actual_state.input_tick,
        expected_hash,
        actual_hash,
        state_path: found
            .as_ref()
            .map_or_else(|| "<canonical_hash>".to_string(), |f| f.path.clone()),
        expected_state: found.as_ref().map(|f| f.expected.clone()),
        actual_state: found.as_ref().map(|f| f.actual.clone()),
        expected_input,
        actual_input,
    }
}

/// Replay `reference` and `candidate` side by side from their own initial
/// snapshots, checking that both agree at every boundary.
///
/// # Errors
///
/// Returns `Err` if either tape's identity does not match
/// `expected_identity` or the active `tune`, or if either tape is
/// structurally invalid.
pub fn compare(
    reference: &InputTape,
    candidate: &InputTape,
    expected_identity: &InputTapeIdentity,
    tune: &Tuning,
) -> Result<ReplayComparison, ReplayFailure> {
    validate_context(reference, expected_identity, tune)?;
    validate_context(candidate, expected_identity, tune)?;
    if let Some(diff) = input_tape::identity_difference(&reference.identity, &candidate.identity)
        .map_err(malformed)?
    {
        return Err(identity_failure(diff.path, diff.expected, diff.actual));
    }

    let (mut expected_state, mut expected_combat) = match_snapshot::restore(&reference.initial);
    let (mut actual_state, mut actual_combat) = match_snapshot::restore(&candidate.initial);
    let first_expected = boundary(&expected_state, expected_combat.as_ref());
    let first_actual = boundary(&actual_state, actual_combat.as_ref());
    let mut expected_boundaries = vec![first_expected.clone()];
    let mut actual_boundaries = vec![first_actual.clone()];

    if first_expected.hash != first_actual.hash {
        let divergence = compare_states(
            &expected_state,
            &actual_state,
            expected_combat.as_ref(),
            actual_combat.as_ref(),
            None,
            None,
            None,
        );
        return Ok(ReplayComparison {
            equal: false,
            expected: ReplayResult {
                state: expected_state,
                combat_state: expected_combat,
                boundaries: expected_boundaries,
                divergence: Some(divergence.clone()),
            },
            actual: ReplayResult {
                state: actual_state,
                combat_state: actual_combat,
                boundaries: actual_boundaries,
                divergence: Some(divergence.clone()),
            },
            divergence: Some(divergence),
        });
    }

    let count = reference.frames.len().min(candidate.frames.len());
    for index in 0..count {
        let expected_frame = &reference.frames[index];
        let actual_frame = &candidate.frames[index];
        let expected_wire =
            input_frame::encode(expected_frame).expect("tape frame already validated");
        let actual_wire = input_frame::encode(actual_frame).expect("tape frame already validated");
        sim_match::step(
            &mut expected_state,
            fixed_clock::TICK_SECONDS,
            sim_match::StepInput::Frame(expected_frame),
            expected_combat.as_mut(),
            tune,
        );
        sim_match::step(
            &mut actual_state,
            fixed_clock::TICK_SECONDS,
            sim_match::StepInput::Frame(actual_frame),
            actual_combat.as_mut(),
            tune,
        );
        let expected_b = boundary(&expected_state, expected_combat.as_ref());
        let actual_b = boundary(&actual_state, actual_combat.as_ref());
        let mismatch = expected_b.hash != actual_b.hash;
        expected_boundaries.push(expected_b);
        actual_boundaries.push(actual_b);
        if mismatch {
            let divergence = compare_states(
                &expected_state,
                &actual_state,
                expected_combat.as_ref(),
                actual_combat.as_ref(),
                Some(expected_frame.tick),
                Some(expected_wire),
                Some(actual_wire),
            );
            return Ok(ReplayComparison {
                equal: false,
                expected: ReplayResult {
                    state: expected_state,
                    combat_state: expected_combat,
                    boundaries: expected_boundaries,
                    divergence: Some(divergence.clone()),
                },
                actual: ReplayResult {
                    state: actual_state,
                    combat_state: actual_combat,
                    boundaries: actual_boundaries,
                    divergence: Some(divergence.clone()),
                },
                divergence: Some(divergence),
            });
        }
    }

    if reference.frames.len() != candidate.frames.len() {
        let expected_frame = reference.frames.get(count);
        let actual_frame = candidate.frames.get(count);
        let causal_tick = expected_frame
            .map(|f| f.tick)
            .or_else(|| actual_frame.map(|f| f.tick))
            .unwrap_or(expected_state.input_tick);
        let divergence = ReplayDivergence {
            causal_input_tick: Some(causal_tick),
            boundary_tick: actual_state.input_tick,
            expected_hash: expected_boundaries
                .last()
                .expect("at least one boundary")
                .hash
                .clone(),
            actual_hash: actual_boundaries
                .last()
                .expect("at least one boundary")
                .hash
                .clone(),
            state_path: "frames.length".to_string(),
            expected_state: Some(reference.frames.len().to_string()),
            actual_state: Some(candidate.frames.len().to_string()),
            expected_input: expected_frame
                .map(|f| input_frame::encode(f).expect("tape frame already validated")),
            actual_input: actual_frame
                .map(|f| input_frame::encode(f).expect("tape frame already validated")),
        };
        return Ok(ReplayComparison {
            equal: false,
            expected: ReplayResult {
                state: expected_state,
                combat_state: expected_combat,
                boundaries: expected_boundaries,
                divergence: Some(divergence.clone()),
            },
            actual: ReplayResult {
                state: actual_state,
                combat_state: actual_combat,
                boundaries: actual_boundaries,
                divergence: Some(divergence.clone()),
            },
            divergence: Some(divergence),
        });
    }

    Ok(ReplayComparison {
        equal: true,
        expected: ReplayResult {
            state: expected_state,
            combat_state: expected_combat,
            boundaries: expected_boundaries,
            divergence: None,
        },
        actual: ReplayResult {
            state: actual_state,
            combat_state: actual_combat,
            boundaries: actual_boundaries,
            divergence: None,
        },
        divergence: None,
    })
}
