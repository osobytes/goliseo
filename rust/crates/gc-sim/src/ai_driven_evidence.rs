//! Replays the AI-driven reference match and digests it, so the SAME run can
//! be checked from inside the compiled wasm module as well as natively.
//!
//! WHY THIS IS SEPARATE FROM THE DIFFERENTIAL.
//!
//! `tests/session_ai_driven_differential.rs` proves that native Rust
//! reproduces real Lua bit for bit over this match. That is a statement about
//! the SOURCE. It says nothing about the artifact a browser executes: the same
//! source compiled to `wasm32-unknown-unknown` is a different code generator,
//! a different float-op ordering opportunity, and — as of 2026-08-07 — was
//! demonstrably able to be an entirely different BUILD. The whole reason the
//! browser looked wrong all day while every native probe looked right is that
//! nothing tied the shipped artifact to the verified source.
//!
//! [`run`] closes that. It is deliberately shaped like
//! [`crate::determinism_evidence`]'s OMP-1 campaign — the acceptance surface
//! that already proves the wasm build does not perturb float behaviour for an
//! IDLE match — but drives the AI-driven scenario instead, so the same claim
//! covers shooting, charging, passing, dashing and the rest of the human-input
//! branch.
//!
//! THE DIGEST IS OVER BIT PATTERNS, NOT TEXT. The Lua capture prints
//! `%.17g`, and reproducing C's `%g` formatting in Rust to hash the same
//! STRING would put a formatting implementation between the two languages and
//! the thing being compared — a place for a difference to hide that has
//! nothing to do with the simulation. Seventeen significant digits round-trip
//! an `f64` exactly, so parsing the fixture recovers the identical bits, and
//! hashing `f64::to_bits()` compares the numbers themselves. See
//! [`Digest::push_f64`].
//!
//! The field order and the field set are exactly the Lua capture's row layout
//! (`tools/lua_reference/capture_session_ai_driven_match.lua`), which is
//! what lets `tests/ai_driven_evidence.rs` derive the expected digest FROM the
//! Lua fixture rather than from another Rust run. Agreement here is therefore
//! agreement with Lua directly, not transitively.

use gc_core::fnv1a64::Fnv1a64State;

use crate::bot;
use crate::headless;
use crate::input_frame::{self, InputSample};
use crate::r#match::{self as sim_match, NewMatchOptions, StepInput};
use crate::match_snapshot::{MatchInput, MatchState, PitchSize};
use crate::slot_input;
use crate::tuning::Tuning;

/// Ticks simulated — a full 120-second match at 60 Hz.
pub const TICKS: i64 = 7_200;
/// Seconds per tick.
const DT: f64 = 1.0 / 60.0;
/// The reference match's seed. Matches the capture script.
pub const MATCH_SEED: f64 = 5.0;
/// The bot's seed. Deliberately different from [`MATCH_SEED`] so a divergence
/// in one is attributable rather than ambiguous.
pub const BOT_SEED: f64 = 11.0;
/// Players per match.
const PLAYER_COUNT: usize = 10;

/// The digested result of one [`run`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiDrivenEvidenceResult {
    /// Stable identity for this scenario, so a caller cannot compare digests
    /// from two different campaigns and believe they agree.
    pub fixture_id: String,
    /// Ticks simulated ([`TICKS`]).
    pub ticks: i64,
    /// Rows digested — `ticks + 1`, because tick 0 is recorded before the
    /// first step.
    pub rows: i64,
    /// FNV-1a-64 over the FINAL row's fields alone, lowercase hex.
    pub final_hash: String,
    /// FNV-1a-64 over every row in sequence, lowercase hex. A divergence that
    /// self-corrects a tick later still changes this; `final_hash` alone
    /// would not catch it.
    pub sequence_digest: String,
    /// Final home score.
    pub score_home: i64,
    /// Shot events observed across the whole run — the canary
    /// `the_reference_match_is_actually_played` asserts on this rather than
    /// on goals, because a well-played 0-0 exercises the shooting path just
    /// as fully (this fixed seed produces exactly that on the
    /// pass-reception-rework base: 12 shots, 5 claims, 0-0).
    pub shots: i64,
    /// Final away score.
    pub score_away: i64,
}

/// A running FNV-1a-64 over canonical field values.
///
/// Public so the test that derives the expected digest from the Lua fixture
/// uses THIS code rather than a second implementation of it — two copies of a
/// hash are two things that can disagree for reasons unrelated to what they
/// are hashing.
#[derive(Debug, Default)]
pub struct Digest(Fnv1a64State);

impl Digest {
    /// A fresh digest.
    #[must_use]
    pub fn new() -> Self {
        Self(Fnv1a64State::new())
    }

    /// Absorb an `f64` by its BIT PATTERN, big-endian.
    ///
    /// Not by its printed form: see this module's header. `-0.0` and `0.0`
    /// hash differently under this, and correctly so — they are different
    /// bit patterns, and a simulation that produced one where the other was
    /// expected has diverged even though `==` would say otherwise.
    pub fn push_f64(&mut self, value: f64) {
        self.0.update(&value.to_bits().to_be_bytes());
    }

    /// Absorb an `i64`, big-endian.
    pub fn push_i64(&mut self, value: i64) {
        self.0.update(&value.to_be_bytes());
    }

    /// Absorb a `u32`, big-endian.
    pub fn push_u32(&mut self, value: u32) {
        self.0.update(&value.to_be_bytes());
    }

    /// The digest so far, lowercase hex.
    #[must_use]
    pub fn hex(&self) -> String {
        self.0.hex()
    }
}

/// The per-tick row this scenario digests, in the Lua capture's field order.
///
/// Constructed from a live [`MatchState`] by [`row_of`], and from a fixture
/// line by the test — one shape, so the two cannot drift on which fields are
/// covered or in what order.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// Tick index.
    pub tick: i64,
    /// Ball position and motion: x, y, vel.x, vel.y, z, vz.
    pub ball: [f64; 6],
    /// Owning player index, or `-1`.
    pub owner: i64,
    /// Home score.
    pub score_home: i64,
    /// Away score.
    pub score_away: i64,
    /// The match RNG state.
    pub rng: u32,
    /// Each player's `(x, y)`.
    pub players: [(f64, f64); PLAYER_COUNT],
}

impl Row {
    /// Absorb this row into a digest, in the canonical field order.
    pub fn absorb(&self, digest: &mut Digest) {
        digest.push_i64(self.tick);
        for value in self.ball {
            digest.push_f64(value);
        }
        digest.push_i64(self.owner);
        digest.push_i64(self.score_home);
        digest.push_i64(self.score_away);
        digest.push_u32(self.rng);
        for (x, y) in self.players {
            digest.push_f64(x);
            digest.push_f64(y);
        }
    }
}

/// The row a given state produces at a given tick.
#[must_use]
pub fn row_of(tick: i64, s: &MatchState) -> Row {
    let mut players = [(0.0_f64, 0.0_f64); PLAYER_COUNT];
    for (index, slot) in players.iter_mut().enumerate() {
        let p = &s.players[index];
        *slot = (p.pos.x, p.pos.y);
    }
    Row {
        tick,
        ball: [
            s.ball.x,
            s.ball.y,
            s.ball_vel.x,
            s.ball_vel.y,
            s.ball_z,
            s.ball_vz,
        ],
        owner: s.owner.unwrap_or(-1),
        score_home: s.score.home,
        score_away: s.score.away,
        rng: s.rng,
        players,
    }
}

/// One tick of the browser's own input path, driven by the bot.
///
/// Identical to the capture script's `tick_input` and to the differential's:
/// the bot's `MatchInput` is quantized into a slot sample, encoded to the
/// canonical wire, decoded, validated, and dequantized back — the lossy trip
/// `crates/gc-wasm/src/session.rs` performs on every real tick.
///
/// `pub(crate)`, not private: [`crate::wasm_native_corpus`] drives the same
/// bot-authored, wire-quantized input path for its own seeded scenarios
/// rather than re-deriving it — see that module's doc for why reusing this
/// exact function (not a second copy) matters.
pub(crate) fn tick_input(
    b: &mut bot::BotState,
    s: &MatchState,
    tick: i64,
    tune: &Tuning,
) -> MatchInput {
    let raw = bot::input(b, &headless::to_bot_view(s), DT, tune);
    let mut slots: [InputSample; 8] = [input_frame::neutral_sample(); 8];
    // Slot 0 is the canonical `home_1` slot the session reads back out.
    slots[0] = slot_input::to_sample(&raw);
    let frame = input_frame::new(tick, Some(slots)).expect("a bot-authored frame is always valid");
    let wire = input_frame::encode(&frame).expect("a valid frame always encodes");
    let decoded = input_frame::decode(&wire).expect("a wire just encoded always decodes");
    input_frame::validate(&decoded).expect("a freshly encoded frame is always valid");
    slot_input::to_match_input(&decoded.slots[0])
}

/// A match built exactly as `game/screens/match.lua`'s `Match:restart` built
/// an ordinary one and as `crates/gc-wasm/src/session.rs` builds its live
/// state: no `input_ownership`, no `human_controlled` override, default
/// duration and goal limit — parameterized on `seed` so a caller can build
/// the one frozen reference match ([`new_reference_match`]) or any other
/// seeded scenario from the identical construction path.
///
/// `pub(crate)`: [`crate::wasm_native_corpus`] reuses this rather than
/// re-deriving the same `NewMatchOptions` a second time for its own seeded
/// corpus.
#[must_use]
pub(crate) fn new_match_with_seed(seed: f64) -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize {
            w: 1648.0,
            h: 927.0,
        },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(seed),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    })
}

/// The frozen reference match: [`new_match_with_seed`] at [`MATCH_SEED`].
#[must_use]
pub fn new_reference_match() -> MatchState {
    new_match_with_seed(MATCH_SEED)
}

/// Replay the AI-driven reference match and digest every tick.
///
/// Deterministic and self-contained: no clock, no I/O, no fixture. The point
/// is that this can run unchanged inside the wasm module, where none of those
/// are available anyway.
#[must_use]
pub fn run(tune: &Tuning) -> AiDrivenEvidenceResult {
    run_to(tune, TICKS)
}

/// [`run`], stopped after `ticks` ticks.
///
/// Exists for BISECTION. When two builds of this same source disagree on
/// `sequence_digest` but agree on `final_hash` -- a divergence that
/// self-corrects, which is still a desync -- the only way to find it is to ask
/// where the prefixes stop matching. Native and wasm can both answer this, so
/// the search runs across the boundary rather than only on one side of it.
#[must_use]
pub fn run_to(tune: &Tuning, ticks: i64) -> AiDrivenEvidenceResult {
    let mut s = new_reference_match();
    let mut b = bot::new(bot::BotOptions {
        seed: Some(BOT_SEED),
        reaction: None,
    });

    let mut sequence = Digest::new();
    let mut last = row_of(0, &s);
    last.absorb(&mut sequence);

    let mut shots = 0;
    for tick in 1..=ticks {
        let input = tick_input(&mut b, &s, tick, tune);
        sim_match::step(&mut s, DT, StepInput::Legacy(input), None, tune);
        shots += s
            .events
            .iter()
            .filter(|e| e.kind == crate::match_snapshot::MatchEventKind::Shot)
            .count() as i64;
        last = row_of(tick, &s);
        last.absorb(&mut sequence);
    }

    let mut final_digest = Digest::new();
    last.absorb(&mut final_digest);

    AiDrivenEvidenceResult {
        fixture_id: "session_ai_driven/v1".to_string(),
        ticks,
        rows: ticks + 1,
        final_hash: final_digest.hex(),
        sequence_digest: sequence.hex(),
        score_home: last.score_home,
        score_away: last.score_away,
        shots,
    }
}
