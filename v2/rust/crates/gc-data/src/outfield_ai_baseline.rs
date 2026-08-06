//! Frozen combat-disabled Outfield AI baseline (#59). DO NOT hand-edit and
//! DO NOT refresh to silence a failing `love . --ai-baseline`: #148/#149 cite
//! this artifact as their control, so a moved baseline is evidence.
//!
//! A deliberate re-freeze is:
//!   1. confirm the change is intended and record it in the drift log of
//!      docs/design/fun_metrics.md;
//!   2. `love . --ai-baseline write --refreeze-ack` (bumps `baseline_version`).
//!
//! See `sim::outfield_ai_baseline` and docs/design/fun_metrics.md.
//!
//! The recorded means/standard-deviations below are transcribed verbatim from
//! `data/outfield_ai_baseline.lua` at full precision, matching the frozen
//! evidence contract's `%.17g` round-trip requirement (see that file). Clippy's
//! `excessive_precision` lint would otherwise ask for a shorter — bit-identical
//! — decimal form; this file keeps the literal digit sequence the Lua source
//! authored instead, since a reviewer diffing the two files line by line
//! should see the same digits.
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
    baseline_version: 1,
    identity: OutfieldAiBaselineIdentity {
        schema: "outfield_ai_baseline",
        schema_version: 1,
        policy_id: "outfield_ai_policy/v1/combat_disabled/303228d776b65a19",
        fixture: "combat_disabled_control_a",
        config: "field=960x540;duration=120;max_goals=3;tick_rate=60;bot=none;combat=disabled;tactic=balanced",
        config_hash: "48c4a66267142b10",
        content_hash: "e6c01365e6311f12",
        tuning_hash: "4e69ddad3a53984f",
        snapshot_version: 11,
        input_version: 2,
        tick_rate: 60,
        seed_first: 20001,
        seed_count: 60,
        seed_hash: "accc11e953c394d0",
        fixture_hash: "766e9087d00023c3",
    },
    stats: OutfieldAiBaselineStats {
        fun: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.29177376372857222,
            sd: 0.35984730984093694,
            min: 0.0,
            max: 0.99341540259250938,
        },
        goals_total: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.9166666666666667,
            sd: 1.2794552159874584,
            min: 0.0,
            max: 5.0,
        },
        goals_home: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.6333333333333333,
            sd: 0.78040985899884585,
            min: 0.0,
            max: 3.0,
        },
        goals_away: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.2833333333333334,
            sd: 1.0430019690032386,
            min: 0.0,
            max: 3.0,
        },
        shots: OutfieldAiBaselineStat {
            n: 60,
            mean: 31.766666666666666,
            sd: 6.7707413075015168,
            min: 3.0,
            max: 44.0,
        },
        shots_per_goal: OutfieldAiBaselineStat {
            n: 53,
            mean: 19.656918238993711,
            sd: 11.397621881378541,
            min: 1.0,
            max: 43.0,
        },
        save_rate: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.89254963169066315,
            sd: 0.099621080712890483,
            min: 0.40000000000000002,
            max: 1.0,
        },
        passes: OutfieldAiBaselineStat {
            n: 60,
            mean: 32.633333333333333,
            sd: 6.1063086762558534,
            min: 5.0,
            max: 43.0,
        },
        pass_completion: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.58335014944457309,
            sd: 0.09043831822054714,
            min: 0.40000000000000002,
            max: 0.80000000000000004,
        },
        turnovers_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 8.2229480527160153,
            sd: 1.7309861646028935,
            min: 4.9953746530992067,
            max: 13.473053892216075,
        },
        possession_balance: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5441707506477037,
            sd: 0.054594832802888987,
            min: 0.4319638145495705,
            max: 0.65120428189116364,
        },
        longest_drought_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 11.48333333333283,
            sd: 2.7869314339116369,
            min: 5.3833333333330273,
            max: 21.599999999998772,
        },
        decided_late: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.647151564310537,
            sd: 0.3522101392174764,
            min: 0.031801138730733237,
            max: 1.0,
        },
        lead_changes: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.10000000000000001,
            sd: 0.30253169045376627,
            min: 0.0,
            max: 1.0,
        },
        margin: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.1166666666666667,
            sd: 0.95831183960175703,
            min: 0.0,
            max: 3.0,
        },
        duration: OutfieldAiBaselineStat {
            n: 60,
            mean: 113.61277777777255,
            sd: 17.917113347910476,
            min: 16.816666666666993,
            max: 120.01666666666114,
        },
        ai_dribble_carry_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 25.171944444444293,
            sd: 4.4128935932144229,
            min: 6.5999999999999819,
            max: 33.366666666666056,
        },
        ai_dribble_close_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.82223864107734634,
            sd: 0.027592916082100037,
            min: 0.76274509803921464,
            max: 0.90476190476190965,
        },
        ai_dribble_sprint_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.18167493674386589,
            sd: 0.040585472299935643,
            min: 0.078703703703704081,
            max: 0.2607561929595838,
        },
        ai_dribble_juke_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.066779322944788028,
            sd: 0.015173917388109161,
            min: 0.035570854847963775,
            max: 0.10997643362136629,
        },
        ai_dribble_touches_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 117.54667942183522,
            sd: 17.752660038193358,
            min: 81.159420289856328,
            max: 175.33385703063567,
        },
        ai_dribble_heavy_losses_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.56110367040555054,
            sd: 1.1133152269028339,
            min: 0.0,
            max: 4.2203985932005184,
        },
        ai_jukes: OutfieldAiBaselineStat {
            n: 60,
            mean: 30.949999999999999,
            sd: 7.1578450502027398,
            min: 6.0,
            max: 44.0,
        },
    },
    signature: "8fa6a781d26002fe",
};
