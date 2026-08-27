//! OMP-1 authoritative fixed-input recording.
//!
//! # One file, two halves, two different rules
//!
//! This fixture is a single JSON document but two kinds of evidence, and
//! only one of them can ever be re-recorded. Until #503 this doc said
//! flatly that the fixture "cannot be regenerated"; that was correct for the
//! recorded input and wrong for the hashes derived from it, and the
//! difference is the whole reason this section exists.
//!
//! ## The frozen half — never regenerable, never refreshed
//!
//! `frame_wires` (the recorded effective axes/action masks), `identity`,
//! `source_seeds`, `windows`, `version`, `fixture_id`, `duration_seconds`
//! and `frame_count`.
//!
//! The capture command that produced these no longer exists, and neither do
//! the source bots that authored the input, so nothing in this repository
//! can recapture them — not this crate, not `gc_sim`, not a script. Normal
//! verification decodes these effective frames and never invokes their
//! source bots. A schema migration may update headers, but the recorded
//! effective axes/action masks themselves must stay untouched.
//!
//! [`rerecorded_json`] copies every one of these fields verbatim and then
//! *verifies* it did, by parsing its own output back and comparing. That is
//! an assertion, not a convention.
//!
//! ## The derived half — regenerable, only under a deliberate reviewed change
//!
//! `boundary_hashes`, `expected_final_hash`, `expected_sequence_digest` and
//! `boundary_count`.
//!
//! These are not recorded observations. They are what the *current*
//! simulation computes when it replays the frozen frames above, so any
//! deliberate change to player state or per-tick arithmetic moves all of
//! them. `gc_sim::determinism_evidence::record` already recomputes the whole
//! vector; [`rerecorded_json`] renders that result back into this file's
//! exact byte format, and `record_omp1_derived_baseline` in
//! `gc-sim/tests/determinism_evidence.rs` is the command a human runs. That
//! recorder is `#[ignore]`d and never runs in a gate: a recorder that
//! overwrote its own fixture during `cargo test` would turn a determinism
//! regression into a no-op.
//!
//! **Re-recording is a decision, not a fix** — the rule issue #500 set for
//! behavioral vectors, applied here. A drifted boundary hash is a *finding*
//! first: something changed the state this simulation reaches from fixed
//! input, and an unintended change of that kind is a desync between a client
//! built before it and one built after. Re-record only when the change is
//! deliberate and reviewed, and record with it: **the decision, the
//! superseding change, and the last commit at which the previous baseline
//! held.**
//!
//! ## Derived, but deliberately not re-recorded: `event_counts`, `expected_score`
//!
//! Both fall out of the same replay, and [`rerecorded_json`] still copies
//! them verbatim. They are the fixture's *behavioral* claims — that this
//! recording covers a tackle, a catch, a header and a full time, ending 1-0
//! — and `windows` pins the ticks those happen on. A hash refresh does not
//! get to move them: a change that reorders the match's events changes what
//! the fixture is evidence *of*, which is a larger decision than re-deriving
//! a digest. The recorder prints a warning block instead of silently
//! folding it in.
//!
//! **Since #505 these three no longer FAIL a campaign either.** The
//! repository owner's decision, recorded on that issue, split the campaign's
//! assertions by what they prove: `expected_score`, `event_counts` and the
//! per-window `event_tick` became a reported, non-gating summary. Gating on
//! them turned "the simulation is deterministic" into "the simulation must
//! keep producing this 1-0", which foreclosed #488/#489/#490/#491.
//!
//! **#512 finished the job.** That decision kept
//! `gc_sim::determinism_evidence::DeterminismCoverage` — "a tackle, a catch, a
//! header and a full time still occurred" — as a hard gate. Measurement
//! refuted the carve-out: `frame_wires` are frozen button presses captured at
//! the recorded tuning, so a 0.45% locomotion change puts every player
//! somewhere slightly different and the recorded presses stop producing the
//! header. Coverage is now reported alongside the other three, and this
//! campaign gates on exactly one thing: the boundary-hash chain. Nothing
//! gates headline-behavior coverage until #518's live-AI fixture lands, and
//! that gap is stated rather than papered over.
//!
//! That makes these fields more important here, not less: because they are
//! never refreshed, they stay the fixed point every future build's behavior
//! is *reported against*. `gc_sim::determinism_evidence`'s module doc has the
//! full split and the three channels the report reaches a human through.
//!
//! # What a pass proves after a re-record, and what it stops proving
//!
//! Before any re-record, a green OMP-1 campaign is evidence that this
//! simulation reproduces a trajectory captured from an implementation nobody
//! can rerun. After a re-record it is evidence that the simulation
//! reproduces *itself* as of the re-record commit. That is strictly weaker:
//! it **detects change** — which is what a determinism guarantee needs — but
//! it **cannot detect "wrong but consistently wrong"**, because the baseline
//! and the code under test now come from the same build. The frozen half is
//! what keeps this fixture more than a self-portrait: the input it replays
//! is still the recorded input, and no re-record may touch it.
//!
//! # Provenance
//!
//! This is a golden determinism-evidence fixture, not authored content: it
//! was mechanically converted (14,517 source lines) to JSON and embedded
//! verbatim via `include_str!`, rather than hand-translated. The conversion
//! was verified byte-for-byte for the two large blob strings (`frame_wires`,
//! `boundary_hashes`) and value-for-value for every other field.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A named boundary window within the recorded match.
///
/// The two optional fields `skip_serializing_if`: the committed fixture
/// *omits* an absent optional key rather than writing `null` (the
/// `full_time` window has no `event_kind`), and [`rerecorded_json`] has to
/// reproduce that file byte for byte. Deserialization is unaffected — serde
/// already reads a missing key as `None`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Omp1Window {
    /// Window name.
    pub name: String,
    /// First boundary tick in the window.
    pub first_boundary: i64,
    /// Last boundary tick in the window.
    pub last_boundary: i64,
    /// Event kind the window is scoped around, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_kind: Option<String>,
    /// Tick the scoped event occurred on, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_tick: Option<i64>,
}

/// The roster each side fielded.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct InputOwnershipRosters {
    /// Home side roster, 5 player ids.
    pub home: Vec<String>,
    /// Away side roster, 5 player ids.
    pub away: Vec<String>,
}

/// One input-tape ownership slot: which recorded stream controls which player.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct InputOwnershipSlot {
    /// Slot name.
    pub slot: String,
    /// Side this slot belongs to.
    pub team: String,
    /// Player id this slot controls.
    pub player_id: String,
}

/// Which recorded input stream controls which player, on both sides.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct InputOwnership {
    /// Ownership record format version.
    pub version: i64,
    /// The roster each side fielded.
    pub rosters: InputOwnershipRosters,
    /// Per-slot stream-to-player assignment.
    pub slots: Vec<InputOwnershipSlot>,
}

/// The full identity of a recorded input tape.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct Omp1ExpectedScore {
    /// Home side goals.
    pub home: i64,
    /// Away side goals.
    pub away: i64,
}

/// The OMP-1 authoritative fixed-input recording.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
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

/// The recorded effective-frame wire lines, split on `\n`. The fixture
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

/// The derived half of the fixture: the values a replay of the frozen
/// frames recomputes. See this module's doc for which fields are derived
/// and what re-recording them costs evidentially.
///
/// `expected_final_hash` and `boundary_count` are deliberately *not* fields
/// here: they are the last element and the length of `boundary_hashes`, and
/// making them separate inputs would be a way for a re-record to produce a
/// fixture that disagrees with itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Omp1DerivedBaseline {
    /// One boundary hash per boundary, in tick order: the initial boundary
    /// first, then one per consumed frame.
    pub boundary_hashes: Vec<String>,
    /// The FNV-1a-64 digest over every boundary hash in sequence.
    pub sequence_digest: String,
}

/// A canonical 16-digit lowercase-hex FNV-1a-64 rendering, the only shape
/// either blob's hashes are allowed to take.
fn is_canonical_hash(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Render a re-recorded fixture: every frozen field copied verbatim from the
/// checked-in fixture, the derived fields replaced by `derived`. The output
/// is byte-identical in format to `omp1_determinism.json` (minified, keys in
/// declaration order, no trailing newline), so a recorder's output can
/// replace that file directly and `git diff` shows only the derived fields.
///
/// This is the write-back half of a re-record; the replay that produces
/// `derived` is `gc_sim::determinism_evidence::record`, and the command a
/// human runs is `record_omp1_derived_baseline` in
/// `gc-sim/tests/determinism_evidence.rs`. Read this module's doc before
/// running it: re-recording is a decision, not a fix.
///
/// The frozen half is *verified*, not assumed. The rendered document is
/// parsed back and every frozen field compared against the checked-in one,
/// so a future refactor that let a re-record perturb `frame_wires`,
/// `identity`, `source_seeds` or `windows` fails here instead of quietly
/// destroying the irreplaceable half of the fixture.
///
/// # Errors
///
/// Returns `Err` if `derived` is empty or carries a value that is not a
/// canonical 16-digit lowercase-hex hash, if the rendered document does not
/// serialize or parse back, or if any frozen field failed to survive the
/// round trip.
pub fn rerecorded_json(derived: &Omp1DerivedBaseline) -> Result<String, String> {
    if derived.boundary_hashes.is_empty() {
        return Err("a re-recorded baseline needs at least one boundary hash".to_string());
    }
    for (index, hash) in derived.boundary_hashes.iter().enumerate() {
        if !is_canonical_hash(hash) {
            return Err(format!("boundary hash {index} is not canonical: {hash:?}"));
        }
    }
    if !is_canonical_hash(&derived.sequence_digest) {
        return Err(format!(
            "sequence digest is not canonical: {:?}",
            derived.sequence_digest
        ));
    }

    let frozen = fixture();
    let mut rerecorded = frozen.clone();
    rerecorded.boundary_count = derived.boundary_hashes.len() as i64;
    rerecorded.expected_final_hash = derived
        .boundary_hashes
        .last()
        .expect("checked non-empty above")
        .clone();
    rerecorded.expected_sequence_digest = derived.sequence_digest.clone();
    rerecorded.boundary_hashes = derived
        .boundary_hashes
        .iter()
        .map(|hash| format!("{hash}\n"))
        .collect();

    let json = serde_json::to_string(&rerecorded)
        .map_err(|e| format!("re-recorded fixture does not serialize: {e}"))?;

    let parsed: Omp1DeterminismFixture = serde_json::from_str(&json)
        .map_err(|e| format!("re-recorded fixture does not parse back: {e}"))?;
    for (field, unchanged) in [
        ("version", parsed.version == frozen.version),
        ("fixture_id", parsed.fixture_id == frozen.fixture_id),
        (
            "duration_seconds",
            parsed.duration_seconds == frozen.duration_seconds,
        ),
        ("frame_count", parsed.frame_count == frozen.frame_count),
        ("identity", parsed.identity == frozen.identity),
        ("source_seeds", parsed.source_seeds == frozen.source_seeds),
        ("windows", parsed.windows == frozen.windows),
        ("event_counts", parsed.event_counts == frozen.event_counts),
        (
            "expected_score",
            parsed.expected_score == frozen.expected_score,
        ),
        ("frame_wires", parsed.frame_wires == frozen.frame_wires),
    ] {
        if !unchanged {
            return Err(format!(
                "re-recording perturbed the frozen field {field}; only derived fields may move"
            ));
        }
    }
    if parsed.boundary_hashes != rerecorded.boundary_hashes
        || parsed.boundary_count != rerecorded.boundary_count
        || parsed.expected_final_hash != rerecorded.expected_final_hash
        || parsed.expected_sequence_digest != rerecorded.expected_sequence_digest
    {
        return Err("re-recorded derived fields did not survive the round trip".to_string());
    }
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The checked-in fixture's own derived values, as a
    /// [`Omp1DerivedBaseline`].
    fn checked_in_derived() -> Omp1DerivedBaseline {
        Omp1DerivedBaseline {
            boundary_hashes: boundary_hash_lines()
                .iter()
                .map(|line| (*line).to_string())
                .collect(),
            sequence_digest: fixture().expected_sequence_digest.clone(),
        }
    }

    /// The JSON fixture parses and matches its recorded header fields, and
    /// the two large blob strings split into exactly
    /// `frame_count`/`boundary_count` lines — corroborating the
    /// byte-identical round trip already checked by the conversion script.
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
        // These two are the DERIVED half, and they move whenever the
        // simulation does -- re-recorded for #488 by
        // `record_omp1_derived_baseline`. Restating them here as literals is
        // what makes a silent JSON edit visible, so they are updated in the
        // same commit as the JSON and never separately. NOTE: #504's
        // documented two-step re-record command does not mention this file;
        // a re-record that stops at the JSON leaves `gc-data`'s own unit test
        // red, which is how this line came to be updated here.
        // #572 moved only the SECOND of these two, and did so on both the
        // pre- and post-#573 base. The final boundary is full time -- clock
        // expired, ball dead at the centre spot -- and completing #489's
        // possession invariant does not reach it, so `expected_final_hash`
        // holds while the chain that arrives there does not. That asymmetry
        // is exactly why a sequence digest exists alongside a final hash: a
        // final-hash-only gate would have called that change inert.
        // #578 (Recovering survives the possession change) repeated the
        // pattern: sequence digest fcdaac058c967e68 -> c165170216a8ec28,
        // final hash unmoved a second time.
        // The 2026-08-25 pitch re-dimensioning (960x540 -> 1648x927, see
        // docs/design/fun_metrics.md's drift log) moved BOTH: the replay's
        // frozen input frames are untouched, but the match the replay
        // constructs them against is a different-sized pitch from tick 0
        // onward, so every boundary -- including the first -- diverges.
        // final hash 0e41232666bc8568 -> 02085004777f30a4, sequence digest
        // c165170216a8ec28 -> bcf2dfa7e1ae7221. LOCO_PACE_REF_HI's default
        // settled at 280 (see gc_data::tunables) shortly after the resize
        // landed, moving both values a second time before either shipped;
        // the pair above is the value at 280, not an intermediate 300.
        // The pass-reception rework (`MatchPlayer::receive_target` and
        // `MatchState::stick_latch` added, `match_snapshot::VERSION` 14 ->
        // 15, `COMBAT_VERSION` 15 -> 16) moved both again: final hash
        // 02085004777f30a4 -> 61c50495d826ce10, sequence digest
        // bcf2dfa7e1ae7221 -> f7d42a56513aa355. This is also the first
        // re-record where `identity.snapshot_version` itself had to move
        // (14 -> 15, in the JSON's frozen half): `input_tape::copy_identity`
        // rejects any `snapshot_version` that is not the current
        // `match_snapshot::VERSION` or `COMBAT_VERSION`, so the frozen
        // identity is kept one schema version ahead by hand on every
        // `VERSION` bump, same as `boundary_hash_lines()[0]` below (a
        // hand-edit to the checked-in JSON, not something the recorder
        // derives -- see `rerecorded_json`'s frozen-field check, which only
        // asserts self-consistency against whatever is currently checked
        // in).
        assert_eq!(f.expected_final_hash, "61c50495d826ce10");
        assert_eq!(f.expected_sequence_digest, "f7d42a56513aa355");
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
        assert_eq!(boundary_hash_lines()[0], "2de23cb0f805ac9f");
        // Boundary 0 above is the initial state hashed by the CURRENT
        // canonical snapshot encoding -- part of the re-recordable derived
        // half, not the frozen recorded input, so it moves whenever
        // `match_snapshot::VERSION` or the simulation does (#531: bumped
        // 11 -> 12 by the new `pass_intent` field on every `MatchPlayer`,
        // including this zero-tick kickoff snapshot; #489: bumped 12 -> 13
        // by the new `action` field on every `MatchPlayer`; #490: bumped
        // 13 -> 14 by the new `keeper_fatigue` field, same reasoning again).
        // The 2026-08-25 pitch re-dimensioning moved it too, and for a
        // different reason than a schema bump: `snapshot_version` did not
        // change, but the kickoff positions this boundary hashes are laid
        // out relative to a pitch that is now a different size
        // (2226cce944213655 -> 115aedc598345a71).
        // The pass-reception rework moved it again by a schema bump:
        // `match_snapshot::VERSION` 14 -> 15 for the new
        // `MatchPlayer::receive_target` and `MatchState::stick_latch`
        // fields, present (as their default/absent encoding) on this
        // zero-tick kickoff snapshot too (115aedc598345a71 ->
        // 2de23cb0f805ac9f).
        // This last boundary is likewise derived. Same re-record, same
        // commit -- see the note beside `expected_final_hash`.
        assert_eq!(
            boundary_hash_lines()[boundary_hash_lines().len() - 1],
            "61c50495d826ce10"
        );
    }

    /// The write-back is byte-exact in the committed file's own format.
    ///
    /// Feeding [`rerecorded_json`] the fixture's *own* derived values must
    /// reproduce `omp1_determinism.json` byte for byte — same minification,
    /// same key order, same absent trailing newline. Without this, the first
    /// real re-record would rewrite all 864 KB of the file and bury the four
    /// fields that actually moved in an unreviewable diff, and nobody would
    /// find out until then.
    ///
    /// This is the assertion that makes the recorder trustworthy, so it is a
    /// CI gate here rather than a step in the recorder: the recorder is
    /// `#[ignore]`d and never runs in `check.sh`.
    #[test]
    fn rerecording_the_checked_in_derived_values_reproduces_the_fixture_byte_for_byte() {
        let json = rerecorded_json(&checked_in_derived()).expect("the frozen half survives");
        assert_eq!(json.len(), FIXTURE_JSON.len());
        assert!(
            json == FIXTURE_JSON,
            "re-recording the checked-in derived values must reproduce the fixture byte for byte"
        );
    }

    /// A changed derived value moves the four derived fields and nothing
    /// else — the property the whole split rests on. Also the demonstration
    /// that the assertion above can go red: it is the same call, with one
    /// hash different.
    #[test]
    fn rerecording_a_changed_baseline_moves_only_the_derived_fields() {
        let mut derived = checked_in_derived();
        let last = derived.boundary_hashes.len() - 1;
        derived.boundary_hashes[last] = "0123456789abcdef".to_string();
        derived.sequence_digest = "fedcba9876543210".to_string();

        let json = rerecorded_json(&derived).expect("the frozen half survives");
        assert_ne!(json, FIXTURE_JSON);

        let parsed: Omp1DeterminismFixture =
            serde_json::from_str(&json).expect("the re-recorded fixture parses");
        let frozen = fixture();
        assert_eq!(parsed.frame_wires, frozen.frame_wires);
        assert_eq!(parsed.identity, frozen.identity);
        assert_eq!(parsed.source_seeds, frozen.source_seeds);
        assert_eq!(parsed.windows, frozen.windows);
        assert_eq!(parsed.event_counts, frozen.event_counts);
        assert_eq!(parsed.expected_score, frozen.expected_score);

        assert_eq!(parsed.expected_final_hash, "0123456789abcdef");
        assert_eq!(parsed.expected_sequence_digest, "fedcba9876543210");
        assert_eq!(parsed.boundary_count, frozen.boundary_count);
        assert_eq!(
            parsed.boundary_hashes.lines().count() as i64,
            parsed.boundary_count
        );
        assert!(parsed.boundary_hashes.ends_with("0123456789abcdef\n"));
    }

    /// A shorter baseline re-derives `boundary_count` with it, so the
    /// document can never claim a count its own blob does not have.
    #[test]
    fn rerecording_derives_the_boundary_count_from_the_hashes_it_was_given() {
        let mut derived = checked_in_derived();
        derived.boundary_hashes.truncate(3);

        let json = rerecorded_json(&derived).expect("the frozen half survives");
        let parsed: Omp1DeterminismFixture =
            serde_json::from_str(&json).expect("the re-recorded fixture parses");
        assert_eq!(parsed.boundary_count, 3);
        assert_eq!(parsed.expected_final_hash, derived.boundary_hashes[2]);
    }

    /// A malformed hash is refused rather than written into the fixture: a
    /// truncated capture is the realistic way a re-record goes wrong.
    #[test]
    fn rerecording_refuses_a_non_canonical_hash() {
        let mut derived = checked_in_derived();
        derived.boundary_hashes[7] = "435F262F7968D95A".to_string();
        assert!(
            rerecorded_json(&derived)
                .expect_err("uppercase hex is not canonical")
                .contains("boundary hash 7 is not canonical")
        );

        let mut derived = checked_in_derived();
        derived.boundary_hashes[7] = "435f262f7968d95".to_string();
        assert!(rerecorded_json(&derived).is_err());

        let mut derived = checked_in_derived();
        derived.sequence_digest = String::new();
        assert!(
            rerecorded_json(&derived)
                .expect_err("an empty digest is not canonical")
                .contains("sequence digest is not canonical")
        );

        assert!(
            rerecorded_json(&Omp1DerivedBaseline {
                boundary_hashes: Vec::new(),
                sequence_digest: fixture().expected_sequence_digest.clone(),
            })
            .expect_err("an empty baseline is refused")
            .contains("at least one boundary hash")
        );
    }
}
