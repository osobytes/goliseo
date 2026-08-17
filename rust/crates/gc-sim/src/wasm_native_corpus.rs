//! A seeded corpus of AI-driven scenarios for the native-vs-wasm
//! differential (#517).
//!
//! ## Why a corpus, and why not [`crate::ai_driven_evidence`]'s one fixture
//!
//! `gc-sim` calls `sin`/`cos`/`exp`/`ln` on thirteen per-tick or
//! per-decision paths. Rust links a different libm for
//! `wasm32-unknown-unknown` than for native, and none of those functions is
//! required to be correctly rounded, so the compiled wasm module can compute
//! different simulation state than a native build from the identical
//! source. #517's own scoping comment measured this as **trajectory
//! dependent**: which transcendental calls fire, with which arguments,
//! moves with the exact match trajectory — so one recorded scenario can
//! reproduce a pinned reference for months while the sites it never happens
//! to exercise stay silently unconverted. [`crate::ai_driven_evidence`]'s
//! `session_ai_driven` fixture is exactly that: one seed, one bot policy,
//! combat disabled. It is real evidence for the paths it reaches — but
//! `session_ai_driven`'s own `combat_state: None` (see that module's `run_to`)
//! means it structurally cannot reach `combat.rs`/`combat_feasibility.rs`'s
//! arc-cosine sites no matter how many ticks it runs.
//!
//! This module is a small corpus instead: several independently seeded
//! matches, most with combat disabled (the product default) and a few with
//! it enabled (`crate::combat::new_state`, the same opt-in path
//! `gc-wasm/src/session.rs`'s `Session::new` offers a real caller), chosen so
//! that across the whole corpus every one of #517's thirteen sites has a
//! reachable, empirically-confirmed-where-possible path. See
//! [`CORPUS`]'s own doc for the scenario-by-scenario mapping against the
//! site inventory, and `ts/packages/wasm/src/wasm_native_corpus.spec.ts` for
//! the wasm side of the comparison this exists to feed.
//!
//! ## What this module does and does not decide
//!
//! [`run_scenario`] runs ONE scenario and returns [`CorpusRun`] — every
//! field a caller (native test or `gc-wasm` binding) needs to compare
//! against another target's run of the same scenario. It makes no claim
//! about what the digests *should* be; exactly like
//! [`crate::ai_driven_evidence::run`], the comparison lives in the caller.
//!
//! ## Per-tick hashes, not just a final/sequence digest
//!
//! [`crate::ai_driven_evidence`] only ever needed a single frozen reference,
//! so a `final_hash` plus a folded `sequence_digest` was enough: either the
//! whole replay matches or it does not. A corpus differential exists
//! specifically to be read on a red run, and "scenario 4 disagreed
//! somewhere" is not an actionable report — #517's own history is a
//! sequence of by-hand bisections (`runAiDrivenEvidenceTo`, tried at
//! several tick counts) to find the first divergent tick. [`CorpusRun`]
//! records one independent hash per tick ([`CorpusRun::tick_hashes`], the
//! same per-row [`crate::ai_driven_evidence::Digest`] construction used
//! elsewhere) so a caller comparing two runs of the same scenario can walk
//! the two vectors in lockstep and name the first index where they differ —
//! no bisection needed, at the cost of one short hex string per tick.
//!
//! ## Reused rather than re-implemented
//!
//! [`run_scenario`] is deliberately built from
//! [`crate::ai_driven_evidence`]'s own building blocks —
//! [`crate::ai_driven_evidence::tick_input`],
//! [`crate::ai_driven_evidence::row_of`], [`crate::ai_driven_evidence::Row`],
//! [`crate::ai_driven_evidence::Digest`] and
//! [`crate::ai_driven_evidence::new_match_with_seed`] — rather than a second
//! copy of the bot-input-quantization-and-digest plumbing. Two
//! implementations of "digest a `MatchState`" are two things that can
//! disagree for reasons that have nothing to do with wasm vs. native float
//! behavior.

use gc_core::fnv1a64::Fnv1a64State;

use crate::ai_driven_evidence::{self, Digest, Row};
use crate::bot;
use crate::combat;
use crate::combat_snapshot::CombatMatchState;
use crate::fixed_clock;
use crate::r#match::{self as sim_match, StepInput};
use crate::match_snapshot::MatchEventKind;
use crate::tuning::Tuning;

/// One seeded corpus scenario.
#[derive(Clone, Copy, Debug)]
pub struct CorpusScenario {
    /// Stable identity, so a caller cannot compare two different scenarios'
    /// digests and believe they agree.
    pub id: &'static str,
    /// The match's own RNG seed (`gc_sim::r#match::NewMatchOptions.seed`).
    pub match_seed: f64,
    /// The bot's seed, driving the local (`home_1`) slot — see
    /// [`crate::ai_driven_evidence::tick_input`]. Deliberately independent of
    /// `match_seed`, matching [`crate::ai_driven_evidence::BOT_SEED`]'s own
    /// reasoning: a divergence in one stream stays attributable.
    pub bot_seed: f64,
    /// Ticks to simulate.
    pub ticks: i64,
    /// Whether to build a [`crate::combat::new_state`] companion and thread
    /// it through every step — the same opt-in `combat_enabled` path
    /// `gc-wasm/src/session.rs`'s `Session::new` offers a real caller.
    /// `combat.rs`/`combat_feasibility.rs`'s arc-cosine sites
    /// (`ai_combat_inputs`) are only ever reached when this is `true`.
    pub combat_enabled: bool,
}

/// The corpus: eight independently seeded scenarios, chosen against #517's
/// scoping comment's thirteen-site inventory (`match.rs`, `ai.rs`,
/// `brain.rs`, `combat.rs`, `combat_feasibility.rs`, `aerial.rs`, `bot.rs`).
///
/// | site(s) | file | reached by |
/// | --- | --- | --- |
/// | 1 dribble touch | `match.rs` `update_ball` | every scenario: fires on any sustained sprint-dribble, common within a few hundred ticks of live play |
/// | 2 AI outfield aim error | `match.rs` `apply_ai_outfield_execution_error` | every scenario: any of the nine non-local AI outfielders shooting or passing |
/// | 3 combat arc cosines | `combat.rs`, `combat_feasibility.rs` | only `combat-a`/`combat-b` (`combat_enabled: true`) — structurally unreachable otherwise, since `ai_combat_inputs` only runs when a `CombatMatchState` is threaded through `sim::match::step` |
/// | 4 aerial contact rotation | `aerial.rs` | empirically confirmed (via `SiteCoverage::aerial`, not merely asserted) on every scenario below |
/// | 5 bot aim noise | `bot.rs` | every scenario: the local slot's `tick_input` always runs `bot::input` |
/// | 6 `exp`/`ln` AI scoring | `ai.rs`, `brain.rs` | `ai.rs`'s `sigmoid`/gaussian/`pass_intercept` sites: every scenario, structurally (off-ball support-spot scoring runs continuously for AI outfielders). `brain.rs`'s softmax `select_scored_option` short-circuits to exact argmax (no `exp` call) only at exactly zero temperature; its `match.rs:5613` -> `outfield_decision::decide_carrier` call site computes `temperature = base * (1.0 - composure) * (1.0 + pressure)`, non-zero whenever the ball carrier's composure is not perfect or pressure is nonzero — true almost every tick a carrier decides an action in live AI play, so structurally reached, but not independently confirmed by a dedicated `SiteCoverage` flag the way sites 1/4/7b are |
/// | 7a support-run triangle | `match.rs` `support_target` | every scenario with a completed pass or reception; confirmed via `SiteCoverage::pass`/`reception`, true on every scenario below |
/// | 7b keeper catch curve | `match.rs` `attempt_save` | empirically confirmed (via `SiteCoverage::keeper_save`) on every scenario below — see the module doc on trajectory dependence for why this one needs real confirmation rather than a structural argument |
///
/// Seeds and tick counts are arbitrary but fixed — this is what makes the
/// corpus reproducible. `mid-a`/`mid-b` run somewhat longer (25 game
/// seconds) than `short-*`/`combat-*` (12/15 game seconds) for a little more
/// margin on the rarer events (aerial duels, keeper saves), chosen from
/// measurement (this crate's `tests/wasm_native_corpus.rs` prints each
/// scenario's observed coverage) rather than assumed — a scenario long
/// enough to plausibly cover every site is not automatically one confirmed
/// to. The whole corpus stays well under `session_ai_driven`'s own 7,200-tick
/// length to keep total wall-clock cost bounded; see that test file and
/// `ts/packages/wasm/src/wasm_native_corpus.spec.ts` for the measured cost.
pub const CORPUS: &[CorpusScenario] = &[
    CorpusScenario {
        id: "corpus/short-a",
        match_seed: 2.0,
        bot_seed: 23.0,
        ticks: 700,
        combat_enabled: false,
    },
    CorpusScenario {
        id: "corpus/short-b",
        match_seed: 9.0,
        bot_seed: 41.0,
        ticks: 700,
        combat_enabled: false,
    },
    CorpusScenario {
        id: "corpus/short-c",
        match_seed: 17.0,
        bot_seed: 5.0,
        ticks: 700,
        combat_enabled: false,
    },
    CorpusScenario {
        id: "corpus/short-d",
        match_seed: 62.0,
        bot_seed: 71.0,
        ticks: 700,
        combat_enabled: false,
    },
    CorpusScenario {
        id: "corpus/mid-a",
        match_seed: 6.0,
        bot_seed: 12.0,
        ticks: 1_500,
        combat_enabled: false,
    },
    CorpusScenario {
        id: "corpus/mid-b",
        match_seed: 44.0,
        bot_seed: 8.0,
        ticks: 1_500,
        combat_enabled: false,
    },
    CorpusScenario {
        id: "corpus/combat-a",
        match_seed: 4.0,
        bot_seed: 19.0,
        ticks: 900,
        combat_enabled: true,
    },
    CorpusScenario {
        id: "corpus/combat-b",
        match_seed: 27.0,
        bot_seed: 53.0,
        ticks: 900,
        combat_enabled: true,
    },
];

/// Which observable events a [`run_scenario`] call witnessed, as an
/// empirical (not merely structural) signal for which of #517's sites a run
/// actually reached — see [`CORPUS`]'s doc table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SiteCoverage {
    /// A dribble touch occurred (site 1, `match.rs` `update_ball`).
    pub touch: bool,
    /// A shot occurred (site 2/5's outlet).
    pub shot: bool,
    /// A pass occurred (site 2/5/7a's outlet).
    pub pass: bool,
    /// A pass reception occurred (corroborates site 7a).
    pub reception: bool,
    /// A tackle occurred.
    pub tackle: bool,
    /// A gameplay-AI combat commit occurred (corroborates site 3 actually
    /// selected a target, not merely that the geometry check ran).
    pub combat_commit: bool,
    /// A header, volley or bicycle contact occurred (site 4, `aerial.rs`).
    pub aerial: bool,
    /// A keeper catch, parry, tip or claim occurred (site 7b, `match.rs`
    /// `attempt_save`).
    pub keeper_save: bool,
}

impl SiteCoverage {
    fn observe(&mut self, kind: MatchEventKind) {
        use MatchEventKind::{
            Bicycle, Catch, Claim, CombatCommitCarrierContest, CombatCommitCarrierProtection,
            CombatCommitLooseBallContest, CombatCommitPassingLaneOrShotDenial,
            CombatCommitRecoveryPunish, CombatCommitUnattributedOffBall, Header, Parry, Pass,
            Reception, Shot, Tackle, Tip, Touch, Volley,
        };
        match kind {
            Touch => self.touch = true,
            Shot => self.shot = true,
            Pass => self.pass = true,
            Reception => self.reception = true,
            Tackle => self.tackle = true,
            CombatCommitCarrierContest
            | CombatCommitCarrierProtection
            | CombatCommitLooseBallContest
            | CombatCommitPassingLaneOrShotDenial
            | CombatCommitRecoveryPunish
            | CombatCommitUnattributedOffBall => self.combat_commit = true,
            Header | Volley | Bicycle => self.aerial = true,
            Catch | Parry | Tip | Claim => self.keeper_save = true,
            _ => {}
        }
    }
}

/// The digested result of one [`run_scenario`] call.
#[derive(Clone, Debug, PartialEq)]
pub struct CorpusRun {
    /// The scenario's own stable identity ([`CorpusScenario::id`]).
    pub scenario_id: &'static str,
    /// Ticks simulated.
    pub ticks: i64,
    /// One FNV-1a-64 hash per row, tick 0 through `ticks` inclusive
    /// (`ticks + 1` entries) — see the module doc on why this is a vector
    /// and not only a final/sequence digest.
    pub tick_hashes: Vec<String>,
    /// The final row's hash — `tick_hashes`'s last entry, named separately
    /// so a caller does not need to know the vector is never empty.
    pub final_hash: String,
    /// FNV-1a-64 folded over `tick_hashes` in order — a single value a
    /// caller can compare first, before walking the full vector to find
    /// where two runs actually diverged.
    pub sequence_digest: String,
    /// Final home score.
    pub score_home: i64,
    /// Final away score.
    pub score_away: i64,
    /// What this run observed — see [`SiteCoverage`].
    pub coverage: SiteCoverage,
    /// The final tick's raw row — ball position/velocity, owner, score, RNG
    /// state, and every player's position. Exists for ATTRIBUTION: comparing
    /// only `tick_hashes` tells a caller a scenario diverged at a given tick,
    /// but not WHICH scalar changed. Running the same scenario to `ticks`
    /// and `ticks - 1` on both targets and diffing this field pinpoints the
    /// changed value (a rotated ball velocity implicates a dribble-touch or
    /// aerial angle; a single shifted player position implicates an aim
    /// error) — see `gc_wasm::wasm_native_corpus_bridge::run_corpus_scenario`,
    /// whose caller uses exactly this to attribute a divergence to a
    /// candidate site rather than only naming the tick.
    pub last_row: Row,
}

fn push_row(row: &Row, hashes: &mut Vec<String>, sequence: &mut Fnv1a64State) {
    let mut digest = Digest::new();
    row.absorb(&mut digest);
    let hex = digest.hex();
    sequence.update(format!("{hex}\n").as_bytes());
    hashes.push(hex);
}

/// Run one seeded corpus scenario and digest every tick.
///
/// Deterministic and self-contained — no clock, no I/O, no fixture — so this
/// runs unchanged inside the compiled wasm module, exactly like
/// [`crate::ai_driven_evidence::run`].
#[must_use]
pub fn run_scenario(tune: &Tuning, scenario: &CorpusScenario) -> CorpusRun {
    let mut state = ai_driven_evidence::new_match_with_seed(scenario.match_seed);
    let mut combat_state: Option<CombatMatchState> = if scenario.combat_enabled {
        Some(combat::new_state(&mut state, None))
    } else {
        None
    };
    let mut bot_state = bot::new(bot::BotOptions {
        seed: Some(scenario.bot_seed),
        reaction: None,
    });

    let mut coverage = SiteCoverage::default();
    let mut tick_hashes = Vec::with_capacity((scenario.ticks + 1) as usize);
    let mut sequence = Fnv1a64State::new();

    let mut row = ai_driven_evidence::row_of(0, &state);
    push_row(&row, &mut tick_hashes, &mut sequence);

    for tick in 1..=scenario.ticks {
        let input = ai_driven_evidence::tick_input(&mut bot_state, &state, tick, tune);
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            StepInput::Legacy(input),
            combat_state.as_mut(),
            tune,
        );
        for event in &state.events {
            coverage.observe(event.kind);
        }
        row = ai_driven_evidence::row_of(tick, &state);
        push_row(&row, &mut tick_hashes, &mut sequence);
    }

    let final_hash = tick_hashes
        .last()
        .expect("tick_hashes always has at least the tick-0 row")
        .clone();

    CorpusRun {
        scenario_id: scenario.id,
        ticks: scenario.ticks,
        tick_hashes,
        final_hash,
        sequence_digest: sequence.hex(),
        score_home: row.score_home,
        score_away: row.score_away,
        coverage,
        last_row: row,
    }
}

/// The first index at which `a` and `b` disagree, or `None` if one is a
/// prefix of the other or they are equal.
///
/// Pure so it is unit-testable without running a scenario — see this
/// module's tests below and `tests/wasm_native_corpus.rs`, which uses this
/// to turn a wasm-vs-native `tick_hashes` mismatch into a specific divergent
/// tick rather than a bare "the scenario disagreed" verdict.
#[must_use]
pub fn first_divergence(a: &[String], b: &[String]) -> Option<usize> {
    a.iter().zip(b.iter()).position(|(x, y)| x != y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scenario_id_in_the_corpus_is_unique() {
        let mut ids: Vec<&str> = CORPUS.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(
            ids.len(),
            CORPUS.len(),
            "corpus scenario ids must be unique"
        );
    }

    #[test]
    fn at_least_one_corpus_scenario_enables_combat() {
        assert!(
            CORPUS.iter().any(|s| s.combat_enabled),
            "site 3 (combat.rs / combat_feasibility.rs) is only reachable \
             when a scenario enables combat"
        );
    }

    #[test]
    fn first_divergence_finds_the_first_differing_index() {
        let a = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut b = a.clone();
        b[2] = "different".to_string();
        assert_eq!(first_divergence(&a, &b), Some(2));
        assert_eq!(first_divergence(&a, &a), None);
    }

    #[test]
    fn first_divergence_treats_a_shorter_prefix_as_agreeing() {
        let a = vec!["a".to_string(), "b".to_string()];
        let b = vec!["a".to_string()];
        assert_eq!(first_divergence(&a, &b), None);
    }

    /// A cheap smoke test (short ticks, not the real corpus lengths) that
    /// combat-enabled scenarios actually run without panicking and that
    /// `run_scenario` is deterministic on repeated native calls — the
    /// property the whole wasm-vs-native comparison depends on holding on
    /// EACH side individually before it can mean anything between them.
    #[test]
    fn run_scenario_is_deterministic_across_repeated_native_calls() {
        let tune = Tuning::new();
        for scenario in CORPUS {
            let mut short = *scenario;
            short.ticks = short.ticks.min(50);
            let first = run_scenario(&tune, &short);
            let second = run_scenario(&tune, &short);
            assert_eq!(
                first.tick_hashes, second.tick_hashes,
                "scenario {} was not deterministic across two native runs",
                scenario.id
            );
        }
    }
}
