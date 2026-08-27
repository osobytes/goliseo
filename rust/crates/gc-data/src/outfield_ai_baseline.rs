//! Frozen combat-disabled Outfield AI baseline (#59). DO NOT hand-edit and
//! DO NOT refresh to silence a failing baseline check: #148/#149 cite
//! this artifact as their control, so a moved baseline is evidence.
//!
//! A deliberate re-freeze is:
//!   1. confirm the change is intended and record it in the drift log of
//!      docs/design/fun_metrics.md;
//!   2. re-record with the runner, which bumps `baseline_version` itself:
//!
//!      cd rust
//!      cargo test -p gc-sim --test outfield_ai_baseline -- --ignored --nocapture record_outfield_ai_baseline
//!
//!      then splice its `pub const RECORD` block over this file's. The
//!      runner emits this doc header and that block only -- the type
//!      definitions between them live here and are not regenerated, so
//!      do not overwrite the whole file with its output.
//!
//!      Until #488 no such runner existed, and this paragraph said so;
//!      `measure` and `serialize` both existed and nothing drove them
//!      together, so every re-freeze until then was the hand edit the
//!      line above warns against.
//!
//!      Regenerating by hand defeats the purpose of a frozen control, so
//!      treat a moved baseline as a finding to investigate first, not a
//!      check to clear.
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
    baseline_version: 24,
    identity: OutfieldAiBaselineIdentity {
        schema: "outfield_ai_baseline",
        schema_version: 1,
        policy_id: "outfield_ai_policy/v1/combat_disabled/0e9a3e0c722f489e",
        fixture: "combat_disabled_control_a",
        config: "field=1648x927;duration=120;max_goals=3;tick_rate=60;bot=none;combat=disabled;tactic=balanced",
        config_hash: "7b608c384f500257",
        content_hash: "e6c01365e6311f12",
        tuning_hash: "dc38036e4d1a2f8e",
        snapshot_version: 15,
        input_version: 2,
        tick_rate: 60,
        seed_first: 20001,
        seed_count: 60,
        seed_hash: "accc11e953c394d0",
        fixture_hash: "30f2164e48b19901",
    },
    stats: OutfieldAiBaselineStats {
        fun: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.20935907855662916,
            sd: 0.3304292915111026,
            min: 0.0,
            max: 0.8597647711050576,
        },
        goals_total: OutfieldAiBaselineStat {
            n: 60,
            mean: 3.55,
            sd: 1.064448611938419,
            min: 1.0,
            max: 5.0,
        },
        goals_home: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.9833333333333334,
            sd: 0.8731720211568127,
            min: 0.0,
            max: 3.0,
        },
        goals_away: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.5666666666666667,
            sd: 0.9631633380788578,
            min: 0.0,
            max: 3.0,
        },
        shots: OutfieldAiBaselineStat {
            n: 60,
            mean: 26.583333333333332,
            sd: 8.203537200431901,
            min: 8.0,
            max: 46.0,
        },
        shots_per_goal: OutfieldAiBaselineStat {
            n: 60,
            mean: 9.089166666666667,
            sd: 6.5273759482846385,
            min: 2.0,
            max: 35.0,
        },
        save_rate: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.6942655498383056,
            sd: 0.16381272150073492,
            min: 0.2,
            max: 0.9411764705882353,
        },
        passes: OutfieldAiBaselineStat {
            n: 60,
            mean: 31.1,
            sd: 8.713325077047516,
            min: 13.0,
            max: 44.0,
        },
        pass_completion: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5969850097732342,
            sd: 0.07947359345224479,
            min: 0.4444444444444444,
            max: 0.8,
        },
        turnovers_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 10.418022472158635,
            sd: 1.696368271137375,
            min: 6.529625151148949,
            max: 15.044247787611189,
        },
        possession_balance: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.552054733394782,
            sd: 0.05948417814061363,
            min: 0.43989769820972263,
            max: 0.7495812395309928,
        },
        longest_drought_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 12.396666666666107,
            sd: 3.4580248654205503,
            min: 6.333333333333471,
            max: 21.899999999998755,
        },
        decided_late: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.7634450325568409,
            sd: 0.242380386138746,
            min: 0.11833363260909244,
            max: 1.0,
        },
        lead_changes: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.21666666666666667,
            sd: 0.41545020165658497,
            min: 0.0,
            max: 1.0,
        },
        margin: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.35,
            sd: 0.7552079071691763,
            min: 0.0,
            max: 3.0,
        },
        duration: OutfieldAiBaselineStat {
            n: 60,
            mean: 96.80027777777352,
            sd: 26.74686114108971,
            min: 38.96666666666574,
            max: 120.01666666666114,
        },
        ai_dribble_carry_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 30.082499999999577,
            sd: 6.9837364371335475,
            min: 13.850000000000268,
            max: 42.93333333333218,
        },
        ai_dribble_close_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.7983820111785749,
            sd: 0.04827215731481472,
            min: 0.6807817589576647,
            max: 0.8850267379679223,
        },
        ai_dribble_sprint_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.2378777224138056,
            sd: 0.05289777766144855,
            min: 0.14018087855297226,
            max: 0.37296416938112525,
        },
        ai_dribble_juke_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.09370493499764228,
            sd: 0.014646729811328902,
            min: 0.06257521058964989,
            max: 0.1287322564855626,
        },
        ai_dribble_touches_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 31.887353513552757,
            sd: 10.111260114806237,
            min: 12.847965738758054,
            max: 53.975465697411444,
        },
        ai_dribble_heavy_losses_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.10198932628451525,
            sd: 0.4493617347177219,
            min: 0.0,
            max: 2.2167487684729275,
        },
        ai_jukes: OutfieldAiBaselineStat {
            n: 60,
            mean: 17.883333333333333,
            sd: 5.072664083492499,
            min: 6.0,
            max: 27.0,
        },
    },
    signature: "a3cf270de2e68c50",
};
