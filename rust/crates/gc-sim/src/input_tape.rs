//! Immutable-by-construction recorded input tapes.
//!
//! A tape owns deep copies of its initial start-of-tick snapshot, materialized
//! effective [`InputFrame`]s, identity, and boundary hashes. It never records
//! bot policy or producer RNG: those have already been materialized into
//! frames.
//!
//! ## Why validation returns `Result`, not `assert!`
//!
//! This module's entire job is validating a tape that arrived from outside
//! the current process — a recording loaded from disk, replayed from a
//! fixture, or received from a peer — and AGENTS.md §7 is explicit that
//! "validation of external input" is exactly the `return Err` case, not the
//! programmer-invariant case that would call for `assert!`.
//!
//! In Rust that reading has a sharper, additional reason: this workspace's
//! release profile sets `panic = "abort"` (`rust/Cargo.toml`), so
//! `catch_unwind` does not survive a release build. There is no escape
//! hatch here; `Result` is not a stylistic choice, it is the only mechanism
//! that still works. Every public function below that validates tape-shaped
//! data returns `Result<T, String>` accordingly, matching
//! [`crate::input_frame`]'s own precedent.
//!
//! Structural shape checks that a typed language's compiler already
//! guarantees (field presence, "is this a canonical array") are dropped
//! here as redundant, exactly as [`crate::input_frame`]'s module doc
//! describes; the bound/range/uniqueness/routing checks they wrapped around
//! are kept.

use crate::combat_identity;
use crate::combat_snapshot::CombatMatchState;
use crate::fixed_clock;
use crate::input_frame::{self, InputFrame, InputOwnership, Team as SlotTeam};
use crate::r#match as sim_match;
use crate::match_snapshot::{self, MatchSnapshot, MatchState, Team};
use crate::tuning::Tuning;
use indexmap::IndexMap;

/// Pre-check for a second, unrelated upstream boundary: `match_snapshot::
/// restore` (`crates/gc-sim/src/match_snapshot.rs`) validates a
/// snapshot's version with `assert!`, not `Result` — reasonable there,
/// except that every caller here is validating a tape that may have
/// arrived from outside the process (a loaded recording, a network peer),
/// exactly the "expected recoverable failure" case AGENTS.md §7 describes.
/// This crate's release profile sets `panic = "abort"`, so there is no
/// unwinding left to catch a panic with (see this module's own doc for the
/// fuller version of this same reasoning about `assert` vs `Result` in this
/// file). Duplicating `match_snapshot::restore`'s own version check here,
/// rather than changing that function's own validation, turns that panic
/// into a reported `Err` before `restore` is ever called.
fn checked_snapshot_version(snapshot: &MatchSnapshot) -> Result<(), String> {
    if snapshot.version != match_snapshot::VERSION
        && snapshot.version != match_snapshot::COMBAT_VERSION
    {
        return Err("unsupported match snapshot version".to_string());
    }
    Ok(())
}

/// Soccer-only input tape format version.
pub const VERSION: i64 = 1;
/// Combat-companion input tape format version.
pub const COMBAT_VERSION: i64 = 2;

/// Field names an [`InputTapeIdentity`] carries, in canonical/display order.
pub const IDENTITY_FIELDS: &[&str] = &[
    "tape_version",
    "input_version",
    "snapshot_version",
    "build",
    "source",
    "content",
    "tuning",
    "config",
    "fixture",
    "seed",
    "tick_rate",
    "ownership",
    "combat",
];

/// A stable identity for one input tape's build/content/fixture: everything
/// that must match before its recorded frames can be replayed against a
/// live simulation.
#[derive(Clone, Debug, PartialEq)]
pub struct InputTapeIdentity {
    /// [`VERSION`] or [`COMBAT_VERSION`].
    pub tape_version: i64,
    /// Exactly `input_frame::VERSION`.
    pub input_version: i64,
    /// `match_snapshot::VERSION` or `match_snapshot::COMBAT_VERSION`.
    pub snapshot_version: i64,
    /// Build/content identity string.
    pub build: String,
    /// Recording source identity string.
    pub source: String,
    /// Content identity string.
    pub content: String,
    /// Serialized [`Tuning`] blob active when the tape was recorded; may be
    /// empty (a fresh registry's defaults serialize to `""`).
    pub tuning: String,
    /// Config identity string.
    pub config: String,
    /// Fixture identity string.
    pub fixture: String,
    /// Recording RNG seed. Stored as `f64` (ARCHITECTURE.md §3 rule 1:
    /// numeric fields default to `f64`) but must hold a finite integral value.
    pub seed: f64,
    /// Recording tick rate, in Hz. Must equal `fixed_clock::TICK_RATE`.
    pub tick_rate: i64,
    /// Stable fixture slot-to-player identity.
    pub ownership: InputOwnership,
    /// Combat build/content identity string (`combat_identity::for_state`),
    /// present exactly when `tape_version == COMBAT_VERSION`.
    pub combat: Option<String>,
}

/// A recorded, replayable input tape.
#[derive(Clone, Debug, PartialEq)]
pub struct InputTape {
    /// [`VERSION`] or [`COMBAT_VERSION`]; always equal to `identity.tape_version`.
    pub version: i64,
    /// The tape's identity.
    pub identity: InputTapeIdentity,
    /// The start-of-tick snapshot every frame replays from.
    pub initial: MatchSnapshot,
    /// Recorded effective frames, contiguous from `initial`'s input tick.
    pub frames: Vec<InputFrame>,
    /// One more hash than `frames`: the boundary before frame 1, then the
    /// boundary after each frame.
    pub boundary_hashes: Vec<String>,
}

/// One leaf disagreement between two [`InputTapeIdentity`] values.
#[derive(Clone, Debug, PartialEq)]
pub struct InputTapeIdentityDifference {
    /// Dotted path to the differing field.
    pub path: String,
    /// The left/expected value, rendered for display.
    pub expected: String,
    /// The right/actual value, rendered for display.
    pub actual: String,
}

fn make_difference(
    path: impl Into<String>,
    expected: impl std::fmt::Debug,
    actual: impl std::fmt::Debug,
) -> InputTapeIdentityDifference {
    InputTapeIdentityDifference {
        path: path.into(),
        expected: format!("{expected:?}"),
        actual: format!("{actual:?}"),
    }
}

/// A validated, defensively copied [`InputOwnership`]. Structural
/// shape/type checks are dropped (the Rust type already guarantees them);
/// the length bounds are still checked at runtime.
fn copy_ownership(ownership: &InputOwnership, path: &str) -> Result<InputOwnership, String> {
    if ownership.version != input_frame::VERSION {
        return Err(format!("{path} version is unsupported"));
    }
    if ownership.rosters.home.len() != input_frame::FIXTURE_TEAM_SIZE as usize {
        return Err(format!("{path}.rosters.home has the wrong length"));
    }
    if ownership.rosters.away.len() != input_frame::FIXTURE_TEAM_SIZE as usize {
        return Err(format!("{path}.rosters.away has the wrong length"));
    }
    Ok(ownership.clone())
}

/// Validate a routing-complete [`InputOwnership`] against a constructed
/// slot-mode [`MatchState`]: every roster player exists and belongs to the
/// right side with exactly one keeper, every canonical slot names a
/// non-keeper outfielder from the right side exactly once, and the
/// assignment agrees with `state.slot_players`/`state.slot_for_player` in
/// both directions.
fn assert_ownership_routes(
    ownership: &InputOwnership,
    state: &MatchState,
    path: &str,
) -> Result<(), String> {
    let mut player_index_by_id: IndexMap<&str, usize> = IndexMap::new();
    for (index, player) in state.players.iter().enumerate() {
        if player_index_by_id.contains_key(player.id.as_str()) {
            return Err(format!("{path} match player ids are not unique"));
        }
        player_index_by_id.insert(player.id.as_str(), index);
    }

    let mut fixture_players: Vec<&str> = Vec::new();
    let mut roster_members_home: Vec<&str> = Vec::new();
    let mut roster_members_away: Vec<&str> = Vec::new();
    let mut roster_count = 0usize;
    for team in [SlotTeam::Home, SlotTeam::Away] {
        let roster: &[String] = match team {
            SlotTeam::Home => &ownership.rosters.home,
            SlotTeam::Away => &ownership.rosters.away,
        };
        let expected_team = match team {
            SlotTeam::Home => Team::Home,
            SlotTeam::Away => Team::Away,
        };
        let mut keeper_count = 0;
        for player_id in roster {
            let &player_index = player_index_by_id
                .get(player_id.as_str())
                .ok_or_else(|| format!("{path} roster player is not in the match"))?;
            let player = &state.players[player_index];
            if player.team != expected_team {
                return Err(format!(
                    "{path} roster player belongs to the other match side"
                ));
            }
            if fixture_players.contains(&player_id.as_str()) {
                return Err(format!("{path} fixture roster player is duplicated"));
            }
            fixture_players.push(player_id.as_str());
            match team {
                SlotTeam::Home => roster_members_home.push(player_id.as_str()),
                SlotTeam::Away => roster_members_away.push(player_id.as_str()),
            }
            roster_count += 1;
            if player.is_keeper {
                keeper_count += 1;
            }
        }
        if keeper_count != 1 {
            return Err(format!(
                "{path} fixture side must contain exactly one keeper"
            ));
        }
    }
    if roster_count != state.players.len() {
        return Err(format!("{path} fixture rosters do not cover the match"));
    }

    let mut assigned_players = vec![false; state.players.len()];
    for index in 1..=input_frame::SLOT_COUNT {
        let expected_slot = input_frame::slot(index).map_err(|e| e.to_string())?;
        let assignment = &ownership.slots[(index - 1) as usize];
        if assignment.slot != expected_slot.id || assignment.team != expected_slot.team {
            return Err(format!("{path} assignments violate canonical slot order"));
        }
        let roster_members: &[&str] = match expected_slot.team {
            SlotTeam::Home => &roster_members_home,
            SlotTeam::Away => &roster_members_away,
        };
        if !roster_members.contains(&assignment.player_id.as_str()) {
            return Err(format!(
                "{path} assignment is not a member of its fixture side"
            ));
        }
        let &player_index = player_index_by_id
            .get(assignment.player_id.as_str())
            .ok_or_else(|| format!("{path} assignment player is not in the match"))?;
        let player = &state.players[player_index];
        if player.is_keeper {
            return Err(format!("{path} keeper cannot own an input slot"));
        }
        if assigned_players[player_index] {
            return Err(format!("{path} player owns multiple input slots"));
        }
        assigned_players[player_index] = true;
        if state.slot_players[(index - 1) as usize] != Some(player_index as i64 + 1) {
            return Err(format!(
                "{path} assignment disagrees with slot_players routing"
            ));
        }
        if state.slot_for_player[player_index] != Some(index) {
            return Err(format!(
                "{path} assignment disagrees with slot_for_player routing"
            ));
        }
    }
    for (index, player) in state.players.iter().enumerate() {
        if player.is_keeper {
            if state.slot_for_player[index].is_some() {
                return Err(format!("{path} keeper appears in inverse slot routing"));
            }
        } else if !assigned_players[index] {
            return Err(format!("{path} outfielder has no input assignment"));
        }
    }
    Ok(())
}

/// Validate and defensively copy an [`InputTapeIdentity`].
///
/// # Errors
///
/// Returns `Err` if any field violates its version, non-emptiness,
/// finite-integer, or tape/combat-consistency invariant. See the module doc
/// for why this is `Result` rather than `assert!`.
pub fn copy_identity(identity: &InputTapeIdentity) -> Result<InputTapeIdentity, String> {
    if identity.tape_version != VERSION && identity.tape_version != COMBAT_VERSION {
        return Err("unsupported input tape identity version".to_string());
    }
    if identity.input_version != input_frame::VERSION {
        return Err("unsupported identity input version".to_string());
    }
    if identity.snapshot_version != match_snapshot::VERSION
        && identity.snapshot_version != match_snapshot::COMBAT_VERSION
    {
        return Err("unsupported identity snapshot version".to_string());
    }
    for (name, value) in [
        ("build", &identity.build),
        ("source", &identity.source),
        ("content", &identity.content),
        ("config", &identity.config),
        ("fixture", &identity.fixture),
    ] {
        if value.is_empty() {
            return Err(format!("identity.{name} must not be empty"));
        }
    }
    if !identity.seed.is_finite() || identity.seed.fract() != 0.0 {
        return Err("identity.seed must be a finite integer".to_string());
    }
    if identity.tick_rate != fixed_clock::TICK_RATE as i64 {
        return Err("identity tick rate is unsupported".to_string());
    }
    if identity.tape_version == VERSION {
        if identity.snapshot_version != match_snapshot::VERSION {
            return Err("soccer tape identity requires the soccer snapshot version".to_string());
        }
        if identity.combat.is_some() {
            return Err("soccer tape identity cannot contain combat identity".to_string());
        }
    } else {
        if identity.snapshot_version != match_snapshot::COMBAT_VERSION {
            return Err("combat tape identity requires the combat snapshot version".to_string());
        }
        if identity.combat.as_deref().unwrap_or("").is_empty() {
            return Err("combat tape identity is required".to_string());
        }
    }
    let ownership = copy_ownership(&identity.ownership, "identity.ownership")?;
    Ok(InputTapeIdentity {
        ownership,
        ..identity.clone()
    })
}

/// The first field two validated identities disagree on, if any.
///
/// # Errors
///
/// Returns `Err` if either identity is itself invalid.
pub fn identity_difference(
    expected: &InputTapeIdentity,
    actual: &InputTapeIdentity,
) -> Result<Option<InputTapeIdentityDifference>, String> {
    let left = copy_identity(expected)?;
    let right = copy_identity(actual)?;

    if left.tape_version != right.tape_version {
        return Ok(Some(make_difference(
            "identity.tape_version",
            left.tape_version,
            right.tape_version,
        )));
    }
    if left.input_version != right.input_version {
        return Ok(Some(make_difference(
            "identity.input_version",
            left.input_version,
            right.input_version,
        )));
    }
    if left.snapshot_version != right.snapshot_version {
        return Ok(Some(make_difference(
            "identity.snapshot_version",
            left.snapshot_version,
            right.snapshot_version,
        )));
    }
    if left.build != right.build {
        return Ok(Some(make_difference(
            "identity.build",
            &left.build,
            &right.build,
        )));
    }
    if left.source != right.source {
        return Ok(Some(make_difference(
            "identity.source",
            &left.source,
            &right.source,
        )));
    }
    if left.content != right.content {
        return Ok(Some(make_difference(
            "identity.content",
            &left.content,
            &right.content,
        )));
    }
    if left.tuning != right.tuning {
        return Ok(Some(make_difference(
            "identity.tuning",
            &left.tuning,
            &right.tuning,
        )));
    }
    if left.config != right.config {
        return Ok(Some(make_difference(
            "identity.config",
            &left.config,
            &right.config,
        )));
    }
    if left.fixture != right.fixture {
        return Ok(Some(make_difference(
            "identity.fixture",
            &left.fixture,
            &right.fixture,
        )));
    }
    if left.seed != right.seed {
        return Ok(Some(make_difference(
            "identity.seed",
            left.seed,
            right.seed,
        )));
    }
    if left.tick_rate != right.tick_rate {
        return Ok(Some(make_difference(
            "identity.tick_rate",
            left.tick_rate,
            right.tick_rate,
        )));
    }
    if left.combat != right.combat {
        return Ok(Some(make_difference(
            "identity.combat",
            &left.combat,
            &right.combat,
        )));
    }

    let (a, b) = (&left.ownership, &right.ownership);
    if a.version != b.version {
        return Ok(Some(make_difference(
            "identity.ownership.version",
            a.version,
            b.version,
        )));
    }
    for (team_name, a_roster, b_roster) in [
        ("home", &a.rosters.home, &b.rosters.home),
        ("away", &a.rosters.away, &b.rosters.away),
    ] {
        for (index, (a_id, b_id)) in a_roster.iter().zip(b_roster.iter()).enumerate() {
            if a_id != b_id {
                return Ok(Some(make_difference(
                    format!("identity.ownership.rosters.{team_name}.{}", index + 1),
                    a_id,
                    b_id,
                )));
            }
        }
    }
    for (index, (a_slot, b_slot)) in a.slots.iter().zip(b.slots.iter()).enumerate() {
        if a_slot.slot != b_slot.slot {
            return Ok(Some(make_difference(
                format!("identity.ownership.slots.{}.slot", index + 1),
                a_slot.slot,
                b_slot.slot,
            )));
        }
        if a_slot.team != b_slot.team {
            return Ok(Some(make_difference(
                format!("identity.ownership.slots.{}.team", index + 1),
                a_slot.team,
                b_slot.team,
            )));
        }
        if a_slot.player_id != b_slot.player_id {
            return Ok(Some(make_difference(
                format!("identity.ownership.slots.{}.player_id", index + 1),
                &a_slot.player_id,
                &b_slot.player_id,
            )));
        }
    }
    Ok(None)
}

#[allow(clippy::type_complexity)]
fn copy_components(
    identity: &InputTapeIdentity,
    initial: &MatchSnapshot,
    frames: &[InputFrame],
    tune: &Tuning,
) -> Result<
    (
        InputTapeIdentity,
        MatchState,
        Option<CombatMatchState>,
        MatchSnapshot,
        Vec<InputFrame>,
    ),
    String,
> {
    let copied_identity = copy_identity(identity)?;
    if copied_identity.tuning != tune.serialize() {
        return Err("identity tuning does not match active simulation tuning".to_string());
    }
    checked_snapshot_version(initial)?;
    let (initial_state, initial_combat) = match_snapshot::restore(initial);
    if !initial_state.slot_mode {
        return Err("input tapes require a fixed-slot match".to_string());
    }
    let Some(ownership) = initial_state.input_ownership.clone() else {
        return Err("input tape snapshot has no ownership".to_string());
    };
    assert_ownership_routes(
        &copied_identity.ownership,
        &initial_state,
        "identity.ownership",
    )?;
    let snapshot_identity = InputTapeIdentity {
        tape_version: if initial_combat.is_some() {
            COMBAT_VERSION
        } else {
            VERSION
        },
        input_version: input_frame::VERSION,
        snapshot_version: initial.version,
        build: copied_identity.build.clone(),
        source: copied_identity.source.clone(),
        content: copied_identity.content.clone(),
        tuning: copied_identity.tuning.clone(),
        config: copied_identity.config.clone(),
        fixture: copied_identity.fixture.clone(),
        seed: copied_identity.seed,
        tick_rate: copied_identity.tick_rate,
        ownership,
        combat: initial_combat
            .as_ref()
            .map(|c| combat_identity::for_state(Some(c))),
    };
    if let Some(diff) = identity_difference(&copied_identity, &snapshot_identity)? {
        return Err(format!("tape {} differs from snapshot", diff.path));
    }
    let mut copied_frames = Vec::with_capacity(frames.len());
    for (index, frame) in frames.iter().enumerate() {
        let frame = input_frame::copy(frame).map_err(|e| e.to_string())?;
        if frame.tick != initial_state.input_tick + index as i64 {
            return Err(
                "input tape frames must be contiguous from the snapshot boundary".to_string(),
            );
        }
        copied_frames.push(frame);
    }
    let normalized_initial = match_snapshot::capture(&initial_state, initial_combat.as_ref());
    Ok((
        copied_identity,
        initial_state,
        initial_combat,
        normalized_initial,
        copied_frames,
    ))
}

/// Construct a tape by replaying `frames` from `initial` and recording every
/// boundary hash, including intermediate ones.
///
/// # Errors
///
/// Returns `Err` if `identity`/`initial`/`frames` are inconsistent with each
/// other, or a frame arrives after the match finishes.
pub fn new(
    identity: &InputTapeIdentity,
    initial: &MatchSnapshot,
    frames: &[InputFrame],
    tune: &Tuning,
) -> Result<InputTape, String> {
    let (copied_identity, _, _, normalized_initial, copied_frames) =
        copy_components(identity, initial, frames, tune)?;
    let mut boundary_hashes = Vec::with_capacity(copied_frames.len() + 1);
    boundary_hashes.push(match_snapshot::hash(&normalized_initial));
    let (mut replay_state, mut replay_combat) = match_snapshot::restore(&normalized_initial);
    for frame in &copied_frames {
        if replay_state.finished {
            return Err("input tape contains a frame after the match finished".to_string());
        }
        sim_match::step(
            &mut replay_state,
            fixed_clock::TICK_SECONDS,
            sim_match::StepInput::Frame(frame),
            replay_combat.as_mut(),
            tune,
        );
        boundary_hashes.push(match_snapshot::hash(&match_snapshot::capture(
            &replay_state,
            replay_combat.as_ref(),
        )));
    }
    Ok(InputTape {
        version: copied_identity.tape_version,
        identity: copied_identity,
        initial: normalized_initial,
        frames: copied_frames,
        boundary_hashes,
    })
}

/// Construct a tape from a checked-in recording whose complete boundary-hash
/// sequence is verified independently. This validates/copies every
/// structural input but deliberately avoids replaying the full match during
/// startup.
///
/// # Errors
///
/// Returns `Err` if `identity`/`initial`/`frames`/`boundary_hashes` are
/// inconsistent with each other, or a hash is malformed.
pub fn from_frozen_recording(
    identity: &InputTapeIdentity,
    initial: &MatchSnapshot,
    frames: &[InputFrame],
    boundary_hashes: &[String],
    tune: &Tuning,
) -> Result<InputTape, String> {
    let (copied_identity, _, _, normalized_initial, copied_frames) =
        copy_components(identity, initial, frames, tune)?;
    if boundary_hashes.len() != copied_frames.len() + 1 {
        return Err("boundary_hashes has the wrong length".to_string());
    }
    for hash in boundary_hashes {
        if !is_canonical_hash(hash) {
            return Err("frozen input tape boundary hash is malformed".to_string());
        }
    }
    if boundary_hashes[0] != match_snapshot::hash(&normalized_initial) {
        return Err(
            "frozen input tape initial boundary hash disagrees with its snapshot".to_string(),
        );
    }
    Ok(InputTape {
        version: copied_identity.tape_version,
        identity: copied_identity,
        initial: normalized_initial,
        frames: copied_frames,
        boundary_hashes: boundary_hashes.to_vec(),
    })
}

fn is_canonical_hash(hash: &str) -> bool {
    hash.len() == 16
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Validate a tape's structure, identity/snapshot consistency, and frame
/// contiguity, WITHOUT replaying it.
///
/// # Errors
///
/// Returns `Err` describing the first structural problem found.
pub fn validate_structure(tape: &InputTape) -> Result<(), String> {
    if tape.version != VERSION && tape.version != COMBAT_VERSION {
        return Err("unsupported input tape version".to_string());
    }
    let identity = copy_identity(&tape.identity)?;
    if tape.version != identity.tape_version {
        return Err("tape and identity versions differ".to_string());
    }
    checked_snapshot_version(&tape.initial)?;
    let (state, combat_state) = match_snapshot::restore(&tape.initial);
    if !state.slot_mode {
        return Err("input tape snapshot is not a fixed-slot match".to_string());
    }
    let Some(ref state_ownership) = state.input_ownership else {
        return Err("input tape snapshot has no ownership".to_string());
    };
    let mut snapshot_identity = copy_identity(&tape.identity)?;
    snapshot_identity.ownership =
        copy_ownership(state_ownership, "tape.initial.state.input_ownership")?;
    snapshot_identity.snapshot_version = tape.initial.version;
    snapshot_identity.tape_version = if combat_state.is_some() {
        COMBAT_VERSION
    } else {
        VERSION
    };
    snapshot_identity.combat = combat_state
        .as_ref()
        .map(|c| combat_identity::for_state(Some(c)));
    if let Some(diff) = identity_difference(&tape.identity, &snapshot_identity)? {
        return Err(format!("tape {} differs from snapshot", diff.path));
    }
    assert_ownership_routes(&tape.identity.ownership, &state, "tape.identity.ownership")?;
    if tape.boundary_hashes.len() != tape.frames.len() + 1 {
        return Err("tape.boundary_hashes has the wrong length".to_string());
    }
    for (index, frame) in tape.frames.iter().enumerate() {
        input_frame::validate(frame).map_err(|e| e.to_string())?;
        if frame.tick != state.input_tick + index as i64 {
            return Err("input tape frames are not contiguous".to_string());
        }
    }
    for hash in &tape.boundary_hashes {
        if !is_canonical_hash(hash) {
            return Err("input tape boundary hash is malformed".to_string());
        }
    }
    if tape.boundary_hashes[0] != match_snapshot::hash(&tape.initial) {
        return Err("input tape initial boundary hash disagrees with its snapshot".to_string());
    }
    Ok(())
}

/// Validate a tape's structure AND replay it end to end, proving every
/// frame is consumable by the simulation.
///
/// # Errors
///
/// Returns `Err` describing the first structural problem found, or if a
/// frame arrives after the match finishes.
pub fn validate(tape: &InputTape, tune: &Tuning) -> Result<(), String> {
    validate_structure(tape)?;
    let (mut state, mut combat_state) = match_snapshot::restore(&tape.initial);
    for frame in &tape.frames {
        if state.finished {
            return Err("input tape contains a frame after the match finished".to_string());
        }
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            sim_match::StepInput::Frame(frame),
            combat_state.as_mut(),
            tune,
        );
    }
    Ok(())
}
