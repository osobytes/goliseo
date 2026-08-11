//! Fun-signature tripwire baseline. REGENERATE, don't hand-edit.
//!
//! Regenerating this baseline needs a sweep runner -- something that plays
//! a fresh batch of seeded matches and calls `gc_sim::tripwire::measure`/
//! `serialize` -- which this repository does not currently provide.
//! Refreshing it by hand defeats the purpose of the tripwire: confirm the
//! drift is intended, log the shift in the drift log of
//! docs/design/fun_metrics.md, and only then replace this header and const
//! with `gc_sim::tripwire::serialize`'s output.

/// The frozen fun-signature tripwire baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FunBaseline {
    /// Sample count.
    pub n: i64,
    /// Composite fun score.
    pub fun: f64,
    /// Total goals per match.
    pub goals_total: f64,
    /// Shots per goal.
    pub shots_per_goal: f64,
    /// Keeper save rate.
    pub save_rate: f64,
    /// Pass completion rate.
    pub pass_completion: f64,
    /// Turnovers per minute.
    pub turnovers_per_min: f64,
    /// Possession balance between sides.
    pub possession_balance: f64,
    /// Longest scoreless drought, in seconds.
    pub longest_drought_s: f64,
    /// Share of matches decided late.
    pub decided_late: f64,
    /// Share of controlled dribble touches that are close control.
    pub controlled_dribble_close_share: f64,
    /// Share of controlled dribble touches that are a sprint.
    pub controlled_dribble_sprint_share: f64,
    /// Share of controlled dribble touches that are a juke.
    pub controlled_dribble_juke_share: f64,
    /// Controlled dribble touches per minute.
    pub controlled_dribble_touches_per_min: f64,
    /// Controlled heavy dribble losses per minute.
    pub controlled_dribble_heavy_losses_per_min: f64,
    /// Share of AI dribble touches that are close control.
    pub ai_dribble_close_share: f64,
    /// Share of AI dribble touches that are a sprint.
    pub ai_dribble_sprint_share: f64,
    /// Share of AI dribble touches that are a juke.
    pub ai_dribble_juke_share: f64,
    /// AI dribble touches per minute.
    pub ai_dribble_touches_per_min: f64,
    /// AI heavy dribble losses per minute.
    pub ai_dribble_heavy_losses_per_min: f64,
}

/// The frozen fun-signature tripwire baseline.
pub const BASELINE: FunBaseline = FunBaseline {
    n: 30,
    fun: 0.442418,
    goals_total: 2.100000,
    shots_per_goal: 23.725309,
    save_rate: 0.857460,
    pass_completion: 0.578243,
    turnovers_per_min: 3.782808,
    possession_balance: 0.398516,
    longest_drought_s: 11.093889,
    decided_late: 0.433093,
    controlled_dribble_close_share: 0.862801,
    controlled_dribble_sprint_share: 0.273743,
    controlled_dribble_juke_share: 0.026119,
    controlled_dribble_touches_per_min: 85.189413,
    controlled_dribble_heavy_losses_per_min: 0.450153,
    ai_dribble_close_share: 0.886487,
    ai_dribble_sprint_share: 0.088690,
    ai_dribble_juke_share: 0.033158,
    ai_dribble_touches_per_min: 75.957928,
    ai_dribble_heavy_losses_per_min: 0.838670,
};
