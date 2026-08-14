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
    baseline_version: 10,
    identity: OutfieldAiBaselineIdentity {
        schema: "outfield_ai_baseline",
        schema_version: 1,
        policy_id: "outfield_ai_policy/v1/combat_disabled/303228d776b65a19",
        fixture: "combat_disabled_control_a",
        config: "field=960x540;duration=120;max_goals=3;tick_rate=60;bot=none;combat=disabled;tactic=balanced",
        config_hash: "48c4a66267142b10",
        content_hash: "e6c01365e6311f12",
        tuning_hash: "4a1d2ea76cd7481c",
        snapshot_version: 12,
        input_version: 2,
        tick_rate: 60,
        seed_first: 20001,
        seed_count: 60,
        seed_hash: "accc11e953c394d0",
        fixture_hash: "110e1af740715032",
    },
    stats: OutfieldAiBaselineStats {
        fun: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.26747342306180244,
            sd: 0.3459224678328692,
            min: 0.0,
            max: 0.9102273089608915,
        },
        goals_total: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.8833333333333333,
            sd: 1.0099784637184799,
            min: 0.0,
            max: 4.0,
        },
        goals_home: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.65,
            sd: 0.7773270149271754,
            min: 0.0,
            max: 2.0,
        },
        goals_away: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.2333333333333334,
            sd: 0.8707422640457282,
            min: 0.0,
            max: 3.0,
        },
        shots: OutfieldAiBaselineStat {
            n: 60,
            mean: 29.0,
            sd: 5.937527876830187,
            min: 14.0,
            max: 46.0,
        },
        shots_per_goal: OutfieldAiBaselineStat {
            n: 54,
            mean: 16.43364197530864,
            sd: 8.014864002359493,
            min: 4.666666666666667,
            max: 35.0,
        },
        save_rate: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.8890981575898227,
            sd: 0.06711490932281351,
            min: 0.7,
            max: 1.0,
        },
        passes: OutfieldAiBaselineStat {
            n: 60,
            mean: 32.68333333333333,
            sd: 4.41296881762275,
            min: 18.0,
            max: 42.0,
        },
        pass_completion: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5249821193202698,
            sd: 0.08225214143164024,
            min: 0.35135135135135137,
            max: 0.7435897435897436,
        },
        turnovers_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 9.778232070716543,
            sd: 1.535109191260365,
            min: 4.9993056519930095,
            max: 13.998055825580426,
        },
        possession_balance: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5183441543459484,
            sd: 0.04817307452374306,
            min: 0.39868324798830124,
            max: 0.6438938053097266,
        },
        longest_drought_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 13.32527777777714,
            sd: 3.4976108225168363,
            min: 8.033333333332877,
            max: 23.49999999999964,
        },
        decided_late: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.6678028475001818,
            sd: 0.3398458874878238,
            min: 0.038883488404389974,
            max: 1.0,
        },
        lead_changes: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.016666666666666666,
            sd: 0.12909944487358072,
            min: 0.0,
            max: 1.0,
        },
        margin: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.05,
            sd: 0.9641893055563064,
            min: 0.0,
            max: 3.0,
        },
        duration: OutfieldAiBaselineStat {
            n: 60,
            mean: 117.20499999999453,
            sd: 9.939009061769296,
            min: 64.78333333333094,
            max: 120.01666666666114,
        },
        ai_dribble_carry_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 28.371944444444118,
            sd: 3.971103832258356,
            min: 18.61666666666689,
            max: 37.19999999999917,
        },
        ai_dribble_close_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.8431956804257796,
            sd: 0.03389972715127779,
            min: 0.7793522267206592,
            max: 0.920818038804408,
        },
        ai_dribble_sprint_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.14041027235731912,
            sd: 0.03952977336391039,
            min: 0.04611650485436941,
            max: 0.24074074074074303,
        },
        ai_dribble_juke_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.07592354569037706,
            sd: 0.018998913215877464,
            min: 0.04456571168713146,
            max: 0.12250516173434298,
        },
        ai_dribble_touches_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 99.75885594954313,
            sd: 17.246581099584144,
            min: 60.40901940220341,
            max: 136.74418604651123,
        },
        ai_dribble_heavy_losses_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.4374614541188713,
            sd: 0.8942803205002191,
            min: 0.0,
            max: 2.9008863819500257,
        },
        ai_jukes: OutfieldAiBaselineStat {
            n: 60,
            mean: 30.933333333333334,
            sd: 5.888416283491139,
            min: 20.0,
            max: 46.0,
        },
    },
    signature: "95850e11e242ea9b",
};
