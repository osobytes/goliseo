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
    baseline_version: 13,
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
            mean: 0.35109766488493915,
            sd: 0.3320948759174023,
            min: 0.0,
            max: 0.8539350905549896,
        },
        goals_total: OutfieldAiBaselineStat {
            n: 60,
            mean: 2.0,
            sd: 1.089238577156585,
            min: 0.0,
            max: 5.0,
        },
        goals_home: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.7833333333333333,
            sd: 0.7611691868917494,
            min: 0.0,
            max: 2.0,
        },
        goals_away: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.2166666666666666,
            sd: 0.9404590777041594,
            min: 0.0,
            max: 3.0,
        },
        shots: OutfieldAiBaselineStat {
            n: 60,
            mean: 32.25,
            sd: 6.057982547215936,
            min: 14.0,
            max: 46.0,
        },
        shots_per_goal: OutfieldAiBaselineStat {
            n: 55,
            mean: 18.860000000000003,
            sd: 10.974494185781177,
            min: 3.5,
            max: 41.0,
        },
        save_rate: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.9007947106381021,
            sd: 0.06214903379940539,
            min: 0.7142857142857143,
            max: 1.0,
        },
        passes: OutfieldAiBaselineStat {
            n: 60,
            mean: 29.566666666666666,
            sd: 4.195908687938064,
            min: 16.0,
            max: 39.0,
        },
        pass_completion: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5148263434472093,
            sd: 0.09383945312943227,
            min: 0.36363636363636365,
            max: 0.7222222222222222,
        },
        turnovers_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 8.75153268364284,
            sd: 1.477984593745427,
            min: 5.49923621719231,
            max: 12.44854273011746,
        },
        possession_balance: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5320703647229559,
            sd: 0.05586184643238152,
            min: 0.41733486063383457,
            max: 0.7248022015823796,
        },
        longest_drought_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 11.554444444443897,
            sd: 2.6092618366268825,
            min: 7.599999999999568,
            max: 18.783333333333285,
        },
        decided_late: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.7120376095415367,
            sd: 0.28562956852070137,
            min: 0.06415775586724343,
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
            mean: 1.0666666666666667,
            sd: 0.8804210600263113,
            min: 0.0,
            max: 3.0,
        },
        duration: OutfieldAiBaselineStat {
            n: 60,
            mean: 116.47888888888346,
            sd: 11.018102236724753,
            min: 71.43333333333057,
            max: 120.01666666666114,
        },
        ai_dribble_carry_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 25.484999999999832,
            sd: 3.5741021550436742,
            min: 15.333333333333675,
            max: 33.7333333333327,
        },
        ai_dribble_close_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.817908623190347,
            sd: 0.03512849811655363,
            min: 0.7203972498090296,
            max: 0.8832752613240497,
        },
        ai_dribble_sprint_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.1627474510317999,
            sd: 0.035657177246469,
            min: 0.045123726346433864,
            max: 0.2576086956521676,
        },
        ai_dribble_juke_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.09621405860755591,
            sd: 0.021278707770764228,
            min: 0.05316606929510212,
            max: 0.14296463506395743,
        },
        ai_dribble_touches_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 119.2961895654515,
            sd: 14.560751562048235,
            min: 84.66898954703746,
            max: 156.76088617265057,
        },
        ai_dribble_heavy_losses_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.4377252569975117,
            sd: 1.0498345536257534,
            min: 0.0,
            max: 4.4144696505211956,
        },
        ai_jukes: OutfieldAiBaselineStat {
            n: 60,
            mean: 35.25,
            sd: 5.984797123559787,
            min: 18.0,
            max: 46.0,
        },
    },
    signature: "264989032124a6b1",
};
