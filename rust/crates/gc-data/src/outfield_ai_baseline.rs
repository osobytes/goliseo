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
    baseline_version: 23,
    identity: OutfieldAiBaselineIdentity {
        schema: "outfield_ai_baseline",
        schema_version: 1,
        policy_id: "outfield_ai_policy/v1/combat_disabled/0e9a3e0c722f489e",
        fixture: "combat_disabled_control_a",
        config: "field=1648x927;duration=120;max_goals=3;tick_rate=60;bot=none;combat=disabled;tactic=balanced",
        config_hash: "7b608c384f500257",
        content_hash: "e6c01365e6311f12",
        tuning_hash: "edd104c4828fca99",
        snapshot_version: 14,
        input_version: 2,
        tick_rate: 60,
        seed_first: 20001,
        seed_count: 60,
        seed_hash: "accc11e953c394d0",
        fixture_hash: "b7658ececade1fe7",
    },
    stats: OutfieldAiBaselineStats {
        fun: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.3253586079417081,
            sd: 0.3708220190267524,
            min: 0.0,
            max: 0.9129007625670492,
        },
        goals_total: OutfieldAiBaselineStat {
            n: 60,
            mean: 2.9166666666666665,
            sd: 1.0781601822229725,
            min: 1.0,
            max: 5.0,
        },
        goals_home: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.2833333333333334,
            sd: 0.9036961141150639,
            min: 0.0,
            max: 3.0,
        },
        goals_away: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.6333333333333333,
            sd: 0.9382035966145971,
            min: 0.0,
            max: 3.0,
        },
        shots: OutfieldAiBaselineStat {
            n: 60,
            mean: 28.416666666666668,
            sd: 7.39970605615616,
            min: 11.0,
            max: 41.0,
        },
        shots_per_goal: OutfieldAiBaselineStat {
            n: 60,
            mean: 12.383333333333331,
            sd: 8.696331780738241,
            min: 2.75,
            max: 41.0,
        },
        save_rate: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.7247515811246629,
            sd: 0.14735860900838219,
            min: 0.2,
            max: 0.9545454545454546,
        },
        passes: OutfieldAiBaselineStat {
            n: 60,
            mean: 34.65,
            sd: 7.338833325282105,
            min: 10.0,
            max: 48.0,
        },
        pass_completion: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.6102244476382371,
            sd: 0.09188132048784564,
            min: 0.391304347826087,
            max: 0.9,
        },
        turnovers_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 9.601919553717575,
            sd: 1.7835407564039127,
            min: 4.731653888280583,
            max: 13.331358317286933,
        },
        possession_balance: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.51351114070453,
            sd: 0.06632722516359676,
            min: 0.3650793650793727,
            max: 0.7064297800338456,
        },
        longest_drought_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 13.439722222221512,
            sd: 3.0897956277628107,
            min: 7.616666666666234,
            max: 21.39999999999879,
        },
        decided_late: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.7299918904508356,
            sd: 0.28462363372823596,
            min: 0.09595889459797818,
            max: 1.0,
        },
        lead_changes: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.26666666666666666,
            sd: 0.44594849085648375,
            min: 0.0,
            max: 1.0,
        },
        margin: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.2166666666666666,
            sd: 0.9222607937841727,
            min: 0.0,
            max: 3.0,
        },
        duration: OutfieldAiBaselineStat {
            n: 60,
            mean: 109.1177777777728,
            sd: 20.75937944406457,
            min: 42.43333333333221,
            max: 120.01666666666114,
        },
        ai_dribble_carry_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 33.78666666666604,
            sd: 6.498681453979456,
            min: 16.116666666667033,
            max: 46.449999999998646,
        },
        ai_dribble_close_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.8084591500924739,
            sd: 0.06039010972842806,
            min: 0.6411582213029902,
            max: 0.8993243243243295,
        },
        ai_dribble_sprint_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.26231197388017996,
            sd: 0.061886485812680574,
            min: 0.16959855660803214,
            max: 0.48810754912098026,
        },
        ai_dribble_juke_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.0833331650446236,
            sd: 0.017325532371361665,
            min: 0.04680038204393432,
            max: 0.11917808219178312,
        },
        ai_dribble_touches_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 34.58904856107774,
            sd: 12.744123837552355,
            min: 11.496350364963753,
            max: 70.76923076923013,
        },
        ai_dribble_heavy_losses_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.1958814917448713,
            sd: 0.6240278516306561,
            min: 0.0,
            max: 3.2447048219919594,
        },
        ai_jukes: OutfieldAiBaselineStat {
            n: 60,
            mean: 18.333333333333332,
            sd: 5.491647484170436,
            min: 5.0,
            max: 27.0,
        },
    },
    signature: "5f1ea5758de255eb",
};
