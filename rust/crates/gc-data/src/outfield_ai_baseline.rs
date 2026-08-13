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
    baseline_version: 2,
    identity: OutfieldAiBaselineIdentity {
        schema: "outfield_ai_baseline",
        schema_version: 1,
        policy_id: "outfield_ai_policy/v1/combat_disabled/303228d776b65a19",
        fixture: "combat_disabled_control_a",
        config: "field=960x540;duration=120;max_goals=3;tick_rate=60;bot=none;combat=disabled;tactic=balanced",
        config_hash: "48c4a66267142b10",
        content_hash: "e6c01365e6311f12",
        tuning_hash: "815f8929cfce068e",
        snapshot_version: 11,
        input_version: 2,
        tick_rate: 60,
        seed_first: 20001,
        seed_count: 60,
        seed_hash: "accc11e953c394d0",
        fixture_hash: "483fe1d1297befc1",
    },
    stats: OutfieldAiBaselineStats {
        fun: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.2834357565364863,
            sd: 0.3569848068883793,
            min: 0.0,
            max: 0.9934154025925094,
        },
        goals_total: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.75,
            sd: 1.2020462779100012,
            min: 0.0,
            max: 5.0,
        },
        goals_home: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.6333333333333333,
            sd: 0.8018340558415714,
            min: 0.0,
            max: 3.0,
        },
        goals_away: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.1166666666666667,
            sd: 0.9404590777041594,
            min: 0.0,
            max: 3.0,
        },
        shots: OutfieldAiBaselineStat {
            n: 60,
            mean: 32.2,
            sd: 5.044883294747969,
            min: 18.0,
            max: 44.0,
        },
        shots_per_goal: OutfieldAiBaselineStat {
            n: 53,
            mean: 21.732389937106916,
            sd: 11.046956118203044,
            min: 4.4,
            max: 40.0,
        },
        save_rate: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.9105688393971558,
            sd: 0.07010650027316158,
            min: 0.6666666666666666,
            max: 1.0,
        },
        passes: OutfieldAiBaselineStat {
            n: 60,
            mean: 33.96666666666667,
            sd: 4.521011836148947,
            min: 18.0,
            max: 43.0,
        },
        pass_completion: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5700843893704797,
            sd: 0.07461557025904812,
            min: 0.4,
            max: 0.7307692307692307,
        },
        turnovers_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 8.258094492084686,
            sd: 1.6119200250756567,
            min: 4.995374653099207,
            max: 13.473053892216075,
        },
        possession_balance: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5424676357916095,
            sd: 0.05180703952659427,
            min: 0.4453883495145665,
            max: 0.6512042818911636,
        },
        longest_drought_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 11.718055555555043,
            sd: 2.239277214877933,
            min: 7.833333333332888,
            max: 17.649999999999373,
        },
        decided_late: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5960033636860714,
            sd: 0.34092267482662586,
            min: 0.030967921122067737,
            max: 1.0,
        },
        lead_changes: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.08333333333333333,
            sd: 0.2787178067853024,
            min: 0.0,
            max: 1.0,
        },
        margin: OutfieldAiBaselineStat {
            n: 60,
            mean: 1.0833333333333333,
            sd: 0.8086747196864057,
            min: 0.0,
            max: 3.0,
        },
        duration: OutfieldAiBaselineStat {
            n: 60,
            mean: 116.69666666666124,
            sd: 10.557320359952206,
            min: 66.7999999999975,
            max: 120.01666666666114,
        },
        ai_dribble_carry_s: OutfieldAiBaselineStat {
            n: 60,
            mean: 25.51166666666651,
            sd: 3.149646947601823,
            min: 17.000000000000316,
            max: 33.333333333332725,
        },
        ai_dribble_close_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.8211653338682867,
            sd: 0.029050879842203323,
            min: 0.7393364928910071,
            max: 0.9047619047619097,
        },
        ai_dribble_sprint_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.1795378417357361,
            sd: 0.0414930464480764,
            min: 0.07870370370370408,
            max: 0.2607561929595838,
        },
        ai_dribble_juke_share: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.06813482896783493,
            sd: 0.013957842637362319,
            min: 0.035570854847963775,
            max: 0.10388639760837048,
        },
        ai_dribble_touches_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 118.82950548243316,
            sd: 15.508359246298664,
            min: 94.180459156435,
            max: 158.74439461883398,
        },
        ai_dribble_heavy_losses_per_min: OutfieldAiBaselineStat {
            n: 60,
            mean: 0.5303821371087173,
            sd: 1.1906940190868802,
            min: 0.0,
            max: 4.91467576791811,
        },
        ai_jukes: OutfieldAiBaselineStat {
            n: 60,
            mean: 31.966666666666665,
            sd: 5.716365642450813,
            min: 16.0,
            max: 43.0,
        },
    },
    signature: "9bf9c999d7b077f8",
};
