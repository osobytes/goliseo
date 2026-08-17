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
    baseline_version: 14,
    identity: OutfieldAiBaselineIdentity {
        schema: "outfield_ai_baseline",
        schema_version: 1,
        policy_id: "outfield_ai_policy/v1/combat_disabled/303228d776b65a19",
        fixture: "combat_disabled_control_a",
        config: "field=960x540;duration=120;max_goals=3;tick_rate=60;bot=none;combat=disabled;tactic=balanced",
        config_hash: "48c4a66267142b10",
        content_hash: "e6c01365e6311f12",
        tuning_hash: "c786c29e021f3f6a",
        snapshot_version: 14,
        input_version: 2,
        tick_rate: 60,
        seed_first: 20001,
        seed_count: 60,
        seed_hash: "accc11e953c394d0",
        fixture_hash: "f78965f8bbf14200",
    },
    stats: OutfieldAiBaselineStats {
        fun: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.26248143751638237,
            sd: 0.31897148344294196,
            min: 0.0,
            max: 0.8539350905549896,
        },
        goals_total: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.8,
            sd: 1.1168964263165337,
            min: 0.0,
            max: 5.0,
        },
        goals_home: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.6666666666666666,
            sd: 0.7287478619712604,
            min: 0.0,
            max: 3.0,
        },
        goals_away: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.1333333333333333,
            sd: 0.8918969817129672,
            min: 0.0,
            max: 3.0,
        },
        shots: OutfieldAiBaselineStat {
            n: 60,
            mean: 32.5,
            sd: 6.37925663806037,
            min: 13.0,
            max: 46.0,
        },
        shots_per_goal: OutfieldAiBaselineStat {
            n: 52,
            mean: 19.900000000000002,
            sd: 11.038107886230364,
            min: 3.5,
            max: 41.0,
        },
        save_rate: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.9083589158255946,
            sd: 0.06637581675632528,
            min: 0.7142857142857143,
            max: 1.0,
        },
        passes: OutfieldAiBaselineStat {
            n: 60,
            mean: 29.766666666666666,
            sd: 3.898572823033729,
            min: 18.0,
            max: 40.0,
        },
        pass_completion: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5049159942958708,
            sd: 0.09724266280693256,
            min: 0.2692307692307692,
            max: 0.7222222222222222,
        },
        turnovers_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 8.970492037280785,
            sd: 1.5421136824882244,
            min: 5.999166782391612,
            max: 12.498264129982523,
        },
        possession_balance: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5284807463609039,
            sd: 0.06301351603426561,
            min: 0.3717339667458505,
            max: 0.6925710373159791,
        },
        longest_drought_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 11.495555555555026,
            sd: 2.624380236524951,
            min: 7.416666666666245,
            max: 18.783333333333285,
        },
        decided_late: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.7259822716981066,
            sd: 0.30053023751382474,
            min: 0.038883488404389974,
            max: 1.0,
        },
        lead_changes: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.06666666666666667,
            sd: 0.25154887429323164,
            min: 0.0,
            max: 1.0,
        },
        margin: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.9666666666666667,
            sd: 0.8227005276478738,
            min: 0.0,
            max: 3.0,
        },
        duration: OutfieldAiBaselineStat {
            n: 60,
            mean: 117.06722222221677,
            sd: 10.46167022717859,
            min: 71.43333333333057,
            max: 120.01666666666114,
        },
        ai_dribble_carry_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 25.856111111110923,
            sd: 3.614432732796484,
            min: 15.333333333333675,
            max: 34.88333333333264,
        },
        ai_dribble_close_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.8163973234623357,
            sd: 0.03445659408136202,
            min: 0.7203972498090296,
            max: 0.8712990936555951,
        },
        ai_dribble_sprint_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.16294801674138856,
            sd: 0.03126385820875421,
            min: 0.0923611111111113,
            max: 0.2576086956521676,
        },
        ai_dribble_juke_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.0946495482208,
            sd: 0.019212280095394104,
            min: 0.05316606929510212,
            max: 0.1370056497175142,
        },
        ai_dribble_touches_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 119.45821090472523,
            sd: 14.615617002126706,
            min: 88.91566265060335,
            max: 156.76088617265057,
        },
        ai_dribble_heavy_losses_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.4075575928369676,
            sd: 0.9789879349335758,
            min: 0.0,
            max: 4.4144696505211956,
        },
        ai_jukes: OutfieldAiBaselineStat {
            n: 60,
            mean: 35.45,
            sd: 5.649133726900682,
            min: 18.0,
            max: 46.0,
        },
    },
    signature: "857c41df296746a8",
};
