//! OMP-1 authoritative fixed-input recording.
//!
//! Generated only by `love . --determinism-refresh`. Normal verification
//! decodes these effective frames and never invokes their source bots.
//! Refresh preserves effective axes/action masks; schema migration may update headers.
//!
//! This is a golden determinism-evidence fixture, not authored content: it is
//! mechanically converted from `data/omp1_determinism.lua` (14,517 lines) to
//! JSON and embedded verbatim via `include_str!`, rather than hand-translated.
//! The round trip from Lua to JSON was verified byte-for-byte for the two large
//! blob strings (`frame_wires`, `boundary_hashes`) and value-for-value for
//! every other field.

use std::sync::LazyLock;

use serde::Deserialize;
use std::collections::BTreeMap;

/// A named boundary window within the recorded match.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Omp1Window {
    /// Window name.
    pub name: String,
    /// First boundary tick in the window.
    pub first_boundary: i64,
    /// Last boundary tick in the window.
    pub last_boundary: i64,
    /// Event kind the window is scoped around, if any.
    pub event_kind: Option<String>,
    /// Tick the scoped event occurred on, if any.
    pub event_tick: Option<i64>,
}

/// The roster each side fielded.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct InputOwnershipRosters {
    /// Home side roster, 5 player ids.
    pub home: Vec<String>,
    /// Away side roster, 5 player ids.
    pub away: Vec<String>,
}

/// One input-tape ownership slot: which recorded stream controls which player.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct InputOwnershipSlot {
    /// Slot name.
    pub slot: String,
    /// Side this slot belongs to.
    pub team: String,
    /// Player id this slot controls.
    pub player_id: String,
}

/// Which recorded input stream controls which player, on both sides.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct InputOwnership {
    /// Ownership record format version.
    pub version: i64,
    /// The roster each side fielded.
    pub rosters: InputOwnershipRosters,
    /// Per-slot stream-to-player assignment.
    pub slots: Vec<InputOwnershipSlot>,
}

/// The full identity of a recorded input tape.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct InputTapeIdentity {
    /// Input tape format version.
    pub tape_version: i64,
    /// Input frame format version.
    pub input_version: i64,
    /// Match snapshot format version.
    pub snapshot_version: i64,
    /// Build identity string.
    pub build: String,
    /// Recording source identity string.
    pub source: String,
    /// Content identity string.
    pub content: String,
    /// Tuning identity string; empty means pure defaults.
    pub tuning: String,
    /// Everything about the run that is not the content or tuning.
    pub config: String,
    /// Fixture name.
    pub fixture: String,
    /// RNG seed.
    pub seed: i64,
    /// Simulation tick rate.
    pub tick_rate: i64,
    /// Which recorded input stream controls which player.
    pub ownership: InputOwnership,
}

/// Full-time score.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Omp1ExpectedScore {
    /// Home side goals.
    pub home: i64,
    /// Away side goals.
    pub away: i64,
}

/// The OMP-1 authoritative fixed-input recording.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Omp1DeterminismFixture {
    /// Fixture record format version.
    pub version: i64,
    /// Fixture identity.
    pub fixture_id: String,
    /// Recorded match duration, in seconds.
    pub duration_seconds: i64,
    /// Recorded frame count.
    pub frame_count: i64,
    /// Recorded boundary count.
    pub boundary_count: i64,
    /// Full identity of the recorded input tape.
    pub identity: InputTapeIdentity,
    /// The eight source seeds the recorded streams were produced from.
    pub source_seeds: Vec<i64>,
    /// Named boundary windows of interest within the recording.
    pub windows: Vec<Omp1Window>,
    /// Count of each event kind observed during the recording.
    pub event_counts: BTreeMap<String, i64>,
    /// Expected full-time score.
    pub expected_score: Omp1ExpectedScore,
    /// Expected final state hash.
    pub expected_final_hash: String,
    /// Expected digest over the whole recorded sequence.
    pub expected_sequence_digest: String,
    /// One effective-frame wire encoding per line, newline-terminated.
    pub frame_wires: String,
    /// One boundary hash per line, newline-terminated.
    pub boundary_hashes: String,
}

static FIXTURE_JSON: &str = include_str!("omp1_determinism.json");

static FIXTURE: LazyLock<Omp1DeterminismFixture> = LazyLock::new(|| {
    serde_json::from_str(FIXTURE_JSON).expect("omp1_determinism.json is well-formed")
});

/// The OMP-1 authoritative fixed-input recording.
pub fn fixture() -> &'static Omp1DeterminismFixture {
    &FIXTURE
}

/// The recorded effective-frame wire lines, split on `\n`. The Lua source
/// stores this as one blob string (`frame_wires`); this is the equivalent of
/// what a consumer splitting that string on newlines would see, with the
/// trailing empty element from the final newline removed.
pub fn frame_wire_lines() -> Vec<&'static str> {
    let fixture = fixture();
    fixture
        .frame_wires
        .strip_suffix('\n')
        .unwrap_or(&fixture.frame_wires)
        .split('\n')
        .collect()
}

/// The recorded boundary hash lines, split on `\n`. See [`frame_wire_lines`].
pub fn boundary_hash_lines() -> Vec<&'static str> {
    let fixture = fixture();
    fixture
        .boundary_hashes
        .strip_suffix('\n')
        .unwrap_or(&fixture.boundary_hashes)
        .split('\n')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The JSON fixture parses and matches the header fields transcribed by
    /// eye from `data/omp1_determinism.lua`, and the two large blob strings
    /// split into exactly `frame_count`/`boundary_count` lines — corroborating
    /// the byte-identical round trip already checked by the conversion script.
    #[test]
    fn fixture_matches_the_lua_source_header() {
        let f = fixture();
        assert_eq!(f.version, 1);
        assert_eq!(f.fixture_id, "omp1-nebula-orion-eight-streams-v2");
        assert_eq!(f.duration_seconds, 120);
        assert_eq!(f.frame_count, 7201);
        assert_eq!(f.boundary_count, 7202);
        assert_eq!(
            f.source_seeds,
            vec![1997, 2094, 2191, 2288, 2385, 2482, 2579, 2676]
        );
        assert_eq!(f.windows.len(), 4);
        assert_eq!(f.windows[0].name, "tackle");
        assert_eq!(f.windows[0].event_tick, Some(24));
        assert_eq!(f.event_counts.get("tackle"), Some(&147));
        assert_eq!(f.event_counts.get("touch"), Some(&180));
        assert_eq!(f.expected_score, Omp1ExpectedScore { home: 1, away: 0 });
        assert_eq!(f.expected_final_hash, "bfbb106aea5480f8");
        assert_eq!(f.expected_sequence_digest, "0bfd0ed355f87322");
        assert_eq!(f.identity.tape_version, 1);
        assert_eq!(f.identity.seed, 19);
        assert_eq!(
            f.identity.ownership.rosters.home,
            vec!["ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex"]
        );
        assert_eq!(f.identity.ownership.slots.len(), 8);
        assert_eq!(f.identity.ownership.slots[0].player_id, "brakka");

        assert_eq!(frame_wire_lines().len(), 7201);
        assert_eq!(boundary_hash_lines().len(), 7202);
        assert_eq!(
            frame_wire_lines()[0],
            "2|0|0,0,0,0|0,0,0,0|127,0,4,0|127,0,0,0|-127,0,4,0|-127,0,4,0|-46,118,4,0|-46,-118,4,0"
        );
        assert_eq!(boundary_hash_lines()[0], "435f262f7968d95a");
        assert_eq!(
            boundary_hash_lines()[boundary_hash_lines().len() - 1],
            "bfbb106aea5480f8"
        );
    }
}
