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
    baseline_version: 20,
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
            mean: 0.2633172305167056,
            sd: 0.35283936729702586,
            min: 0.0,
            max: 0.9021579215939761,
        },
        goals_total: OutfieldAiBaselineStat {
            n: 60,
            mean: 2.6166666666666667,
            sd: 1.2900094161614906,
            min: 0.0,
            max: 5.0,
        },
        goals_home: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.3,
            sd: 1.1244584383875718,
            min: 0.0,
            max: 3.0,
        },
        goals_away: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.3166666666666667,
            sd: 1.0166689821220272,
            min: 0.0,
            max: 3.0,
        },
        shots: OutfieldAiBaselineStat {
            n: 60,
            mean: 25.5,
            sd: 7.427616813313927,
            min: 7.0,
            max: 39.0,
        },
        shots_per_goal: OutfieldAiBaselineStat {
            n: 57,
            mean: 12.345614035087719,
            sd: 8.836836977169451,
            min: 2.3333333333333335,
            max: 38.0,
        },
        save_rate: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.780047887197655,
            sd: 0.15782878535932648,
            min: 0.25,
            max: 1.0,
        },
        passes: OutfieldAiBaselineStat {
            n: 60,
            mean: 25.766666666666666,
            sd: 6.544156836488944,
            min: 6.0,
            max: 36.0,
        },
        pass_completion: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5339548219512996,
            sd: 0.11476779064320636,
            min: 0.16666666666666666,
            max: 0.7333333333333333,
        },
        turnovers_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 8.50589348622559,
            sd: 1.8401843976956471,
            min: 2.4844720496894506,
            max: 12.198221092757798,
        },
        possession_balance: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5136280642099911,
            sd: 0.05147463769727451,
            min: 0.35502958579881233,
            max: 0.6592592592592619,
        },
        longest_drought_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 14.241388888888201,
            sd: 4.684244027170906,
            min: 5.716666666666788,
            max: 26.066666666665192,
        },
        decided_late: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.6781709157373139,
            sd: 0.30158861943659937,
            min: 0.03916122760727848,
            max: 1.0,
        },
        lead_changes: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.16666666666666666,
            sd: 0.4184987968176888,
            min: 0.0,
            max: 2.0,
        },
        margin: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.3833333333333333,
            sd: 0.9930549793895578,
            min: 0.0,
            max: 3.0,
        },
        duration: OutfieldAiBaselineStat {
            n: 60,
            mean: 105.74666666666187,
            sd: 25.154706964816828,
            min: 24.14999999999991,
            max: 120.01666666666114,
        },
        ai_dribble_carry_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 23.35416666666658,
            sd: 5.667157360775024,
            min: 5.466666666666653,
            max: 34.79999999999931,
        },
        ai_dribble_close_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.828252224035077,
            sd: 0.06677826094343443,
            min: 0.6666666666666788,
            max: 0.9335820895522426,
        },
        ai_dribble_sprint_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.3285128582270215,
            sd: 0.06270505995095924,
            min: 0.16358325219084388,
            max: 0.4709141274238161,
        },
        ai_dribble_juke_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.03727807105009398,
            sd: 0.011944018774351686,
            min: 0.012195121951219544,
            max: 0.059332509270705096,
        },
        ai_dribble_touches_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 77.18405258962576,
            sd: 17.425857376073765,
            min: 42.755344418051415,
            max: 115.51925320886953,
        },
        ai_dribble_heavy_losses_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5498231157398642,
            sd: 1.22994633820671,
            min: 0.0,
            max: 4.819277108433761,
        },
        ai_jukes: OutfieldAiBaselineStat {
            n: 60,
            mean: 15.883333333333333,
            sd: 5.198082001774803,
            min: 2.0,
            max: 28.0,
        },
    },
    signature: "82cb3b3319de78b4",
};
