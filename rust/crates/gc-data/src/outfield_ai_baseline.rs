//! Frozen combat-disabled Outfield AI baseline (#59). DO NOT hand-edit and
//! DO NOT refresh to silence a failing baseline check: #148/#149 cite
//! this artifact as their control, so a moved baseline is evidence.
//!
//! A deliberate re-freeze is:
//!   1. confirm the change is intended and record it in the drift log of
//!      docs/design/fun_metrics.md;
//!   2. re-run the frozen fixture and bump `baseline_version` -- this
//!      repository does not currently provide a runner that drives that
//!      re-run outside `cargo test`'s own self-reproducibility checks (see
//!      `gc_sim::outfield_ai_baseline` for the record shape and
//!      `serialize`). Regenerating by hand defeats the purpose of a frozen
//!      control, so treat a moved baseline as a finding to investigate
//!      first, not a check to clear.
//!
//! See `sim::outfield_ai_baseline` and docs/design/fun_metrics.md.
//!
//! The recorded means/standard-deviations below are kept at full precision,
//! matching the frozen evidence contract's `%.17g` round-trip requirement
//! (see `gc_sim::outfield_ai_baseline::serialize`). Clippy's
//! `excessive_precision` lint would otherwise ask for a shorter —
//! bit-identical — decimal form; this file keeps the full literal digit
//! sequence instead, so a reviewer diffing a re-frozen version against this
//! one sees every digit.
#![allow(clippy::excessive_precision)]

/// One tracked metric's summary statistics across the baseline seed set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutfieldAiBaselineStat {
    /// Matches contributing; 0 when no match had a denominator.
    pub n: i64,
    /// Mean.
    pub mean: f64,
    /// Standard deviation.
    pub sd: f64,
    /// Minimum.
    pub min: f64,
    /// Maximum.
    pub max: f64,
}

/// Everything about the recorded run that is not the AI policy or the content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutfieldAiBaselineIdentity {
    /// Baseline record schema name.
    pub schema: &'static str,
    /// Baseline record schema version.
    pub schema_version: i64,
    /// Frozen AI policy id.
    pub policy_id: &'static str,
    /// Fixture name.
    pub fixture: &'static str,
    /// Hash of the fixture's declared shape.
    pub fixture_hash: &'static str,
    /// Everything about the run that is not the AI policy or the content.
    pub config: &'static str,
    /// Hash of `config`.
    pub config_hash: &'static str,
    /// Hash of the authored content the fixture instantiates.
    pub content_hash: &'static str,
    /// Hash of the tuning knobs in effect.
    pub tuning_hash: &'static str,
    /// Match snapshot format version.
    pub snapshot_version: i64,
    /// Input frame format version.
    pub input_version: i64,
    /// Simulation tick rate.
    pub tick_rate: i64,
    /// First seed in the declared seed set.
    pub seed_first: i64,
    /// Count of seeds in the declared seed set.
    pub seed_count: i64,
    /// Hash over the exact seed list, not just first/count.
    pub seed_hash: &'static str,
}

/// Per-metric summary statistics, in hash and comparison order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutfieldAiBaselineStats {
    /// Composite fun score.
    pub fun: OutfieldAiBaselineStat,
    /// Total goals per match.
    pub goals_total: OutfieldAiBaselineStat,
    /// Home-side goals per match.
    pub goals_home: OutfieldAiBaselineStat,
    /// Away-side goals per match.
    pub goals_away: OutfieldAiBaselineStat,
    /// Shots per match.
    pub shots: OutfieldAiBaselineStat,
    /// Shots per goal.
    pub shots_per_goal: OutfieldAiBaselineStat,
    /// Keeper save rate.
    pub save_rate: OutfieldAiBaselineStat,
    /// Passes per match.
    pub passes: OutfieldAiBaselineStat,
    /// Pass completion rate.
    pub pass_completion: OutfieldAiBaselineStat,
    /// Turnovers per minute.
    pub turnovers_per_min: OutfieldAiBaselineStat,
    /// Possession balance between sides.
    pub possession_balance: OutfieldAiBaselineStat,
    /// Longest scoreless drought, in seconds.
    pub longest_drought_s: OutfieldAiBaselineStat,
    /// Share of matches decided late.
    pub decided_late: OutfieldAiBaselineStat,
    /// Lead changes per match.
    pub lead_changes: OutfieldAiBaselineStat,
    /// Final goal margin.
    pub margin: OutfieldAiBaselineStat,
    /// Match duration, in seconds.
    pub duration: OutfieldAiBaselineStat,
    /// AI dribble carry time, in seconds.
    pub ai_dribble_carry_s: OutfieldAiBaselineStat,
    /// Share of AI dribble touches that are close control.
    pub ai_dribble_close_share: OutfieldAiBaselineStat,
    /// Share of AI dribble touches that are a sprint.
    pub ai_dribble_sprint_share: OutfieldAiBaselineStat,
    /// Share of AI dribble touches that are a juke.
    pub ai_dribble_juke_share: OutfieldAiBaselineStat,
    /// AI dribble touches per minute.
    pub ai_dribble_touches_per_min: OutfieldAiBaselineStat,
    /// AI heavy dribble losses per minute.
    pub ai_dribble_heavy_losses_per_min: OutfieldAiBaselineStat,
    /// AI jukes per match.
    pub ai_jukes: OutfieldAiBaselineStat,
}

/// A frozen combat-disabled Outfield AI baseline recording.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutfieldAiBaselineRecord {
    /// Bumped by every deliberate re-freeze.
    pub baseline_version: i64,
    /// Everything about the recorded run that is not the AI policy or the content.
    pub identity: OutfieldAiBaselineIdentity,
    /// Per-metric summary statistics.
    pub stats: OutfieldAiBaselineStats,
    /// Hash over identity + stats; excludes `baseline_version`.
    pub signature: &'static str,
}

/// The frozen baseline recording.
pub const RECORD: OutfieldAiBaselineRecord = OutfieldAiBaselineRecord {
    baseline_version: 3,
    identity: OutfieldAiBaselineIdentity {
        schema: "outfield_ai_baseline",
        schema_version: 1,
        policy_id: "outfield_ai_policy/v1/combat_disabled/303228d776b65a19",
        fixture: "combat_disabled_control_a",
        config: "field=960x540;duration=120;max_goals=3;tick_rate=60;bot=none;combat=disabled;tactic=balanced",
        config_hash: "48c4a66267142b10",
        content_hash: "e6c01365e6311f12",
        tuning_hash: "84908592d5981f4a",
        snapshot_version: 11,
        input_version: 2,
        tick_rate: 60,
        seed_first: 20001,
        seed_count: 60,
        seed_hash: "accc11e953c394d0",
        fixture_hash: "d6463f56f154f710",
    },
    stats: OutfieldAiBaselineStats {
        fun: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.26989021733508156,
            sd: 0.32380522272471113,
            min: 0.0,
            max: 0.9055281673577299,
        },
        goals_total: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.5,
            sd: 0.9655068046526178,
            min: 0.0,
            max: 4.0,
        },
        goals_home: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.7166666666666667,
            sd: 0.8252717408997374,
            min: 0.0,
            max: 3.0,
        },
        goals_away: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.7833333333333333,
            sd: 0.8044719642367522,
            min: 0.0,
            max: 3.0,
        },
        shots: OutfieldAiBaselineStat {
            n: 60,
            mean: 34.083333333333336,
            sd: 5.126506943946111,
            min: 20.0,
            max: 45.0,
        },
        shots_per_goal: OutfieldAiBaselineStat {
            n: 50,
            mean: 22.988333333333333,
            sd: 10.58391596767871,
            min: 6.666666666666667,
            max: 45.0,
        },
        save_rate: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.9275106268370243,
            sd: 0.04784793623006504,
            min: 0.8181818181818182,
            max: 1.0,
        },
        passes: OutfieldAiBaselineStat {
            n: 60,
            mean: 34.36666666666667,
            sd: 3.686745230617412,
            min: 21.0,
            max: 43.0,
        },
        pass_completion: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5584667686999681,
            sd: 0.07181280640842758,
            min: 0.35294117647058826,
            max: 0.7241379310344828,
        },
        turnovers_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 8.740769093635683,
            sd: 1.4989902938657804,
            min: 5.999166782391612,
            max: 12.998194695181825,
        },
        possession_balance: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.54750513150193,
            sd: 0.04448073325181838,
            min: 0.42659826361484265,
            max: 0.6275303643724648,
        },
        longest_drought_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 11.449444444443847,
            sd: 2.261778058838767,
            min: 6.699999999999619,
            max: 17.0166666666657,
        },
        decided_late: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.6941214910291988,
            sd: 0.3126763624251476,
            min: 0.0680461047076825,
            max: 1.0,
        },
        lead_changes: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.03333333333333333,
            sd: 0.1810203347193924,
            min: 0.0,
            max: 1.0,
        },
        margin: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.0,
            sd: 0.8437205738748232,
            min: 0.0,
            max: 3.0,
        },
        duration: OutfieldAiBaselineStat {
            n: 60,
            mean: 118.8841666666611,
            sd: 5.806466316345819,
            min: 80.63333333333004,
            max: 120.01666666666114,
        },
        ai_dribble_carry_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 25.846944444444258,
            sd: 3.0652792422581006,
            min: 16.0833333333337,
            max: 34.23333333333267,
        },
        ai_dribble_close_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.833516708472886,
            sd: 0.03420689184835349,
            min: 0.7582619339045401,
            max: 0.916262135922334,
        },
        ai_dribble_sprint_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.14769205512699066,
            sd: 0.035262895584314224,
            min: 0.07943625880845662,
            max: 0.24092582851131308,
        },
        ai_dribble_juke_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.08590664922137813,
            sd: 0.015566446525576256,
            min: 0.05693950177936004,
            max: 0.12881355932203425,
        },
        ai_dribble_touches_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 107.62066424255293,
            sd: 17.61238889201344,
            min: 54.37382001258699,
            max: 153.7627118644075,
        },
        ai_dribble_heavy_losses_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5105573335481142,
            sd: 1.1704696641672463,
            min: 0.0,
            max: 5.020920502092067,
        },
        ai_jukes: OutfieldAiBaselineStat {
            n: 60,
            mean: 32.71666666666667,
            sd: 4.888907079233406,
            min: 23.0,
            max: 45.0,
        },
    },
    signature: "614ed81d38e82116",
};
