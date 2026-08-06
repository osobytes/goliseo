//! Port of `sim/fixed_clock.lua`.
//!
//! The sole simulation-time authority for render-driven matches. The clock is
//! deliberately input-shape agnostic: an offline adapter can supply one
//! legacy `MatchInput` per tick, while a multi-slot refactor can supply an
//! `InputFrame` without changing the cadence contract. Generic over the input
//! type `I` the caller threads through `input_for_tick` and `step`.

/// Simulation tick rate, in Hz.
pub const TICK_RATE: f64 = 60.0;
/// Seconds simulated per tick.
pub const TICK_SECONDS: f64 = 1.0 / TICK_RATE;
/// At most this many canonical ticks run for one render update.
pub const MAX_TICKS_PER_UPDATE: u32 = 8;

const EPSILON: f64 = TICK_SECONDS * 1e-9;

/// The clock's persistent state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedClockState {
    /// Next tick to simulate; starts at zero and only increases.
    pub tick: u64,
    /// Unsimulated render time, always smaller than one tick after advance.
    pub accumulator: f64,
    /// Whole simulation ticks discarded after overloads.
    pub dropped_ticks: u64,
    /// Number of render updates that exceeded the catch-up budget.
    pub overloads: u64,
}

/// The outcome of one `advance` call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedClockAdvance {
    /// Ticks simulated during this render update.
    pub ticks: u32,
    /// First input tick consumed, if any.
    pub first_tick: Option<u64>,
    /// Last input tick consumed, if any.
    pub last_tick: Option<u64>,
    /// Whole ticks discarded during this render update.
    pub dropped_ticks: u64,
    /// The step callback ended the simulation early.
    pub stopped: bool,
}

fn assert_finite_non_negative(value: f64, label: &str) {
    assert!(value.is_finite() && value >= 0.0, "{label}");
}

/// Construct a fresh clock at tick zero.
#[must_use]
pub fn new() -> FixedClockState {
    FixedClockState {
        tick: 0,
        accumulator: 0.0,
        dropped_ticks: 0,
        overloads: 0,
    }
}

/// Run exactly one canonical tick. Headless callers use this rather than an
/// independent `match.step(..., 1 / 60, ...)` loop, so both paths share the
/// same tick numbering and simulation interval.
///
/// `step` returns whether the simulation should continue; `false` stops it.
pub fn step<I>(
    state: &mut FixedClockState,
    input: &I,
    mut step_fn: impl FnMut(u64, &I) -> bool,
) -> (u64, bool) {
    let tick = state.tick;
    let continue_ = step_fn(tick, input);
    state.tick = tick + 1;
    (tick, continue_)
}

/// Advance a render-driven simulation. At most [`MAX_TICKS_PER_UPDATE`]
/// canonical ticks run for one render update. If a frame arrives with more
/// whole ticks of debt, the excess is intentionally dropped and only the
/// fractional remainder is retained. The match therefore slows under
/// sustained overload instead of growing an unbounded catch-up queue or
/// receiving a variable simulation dt.
pub fn advance<I>(
    state: &mut FixedClockState,
    render_dt: f64,
    mut input_for_tick: impl FnMut(u64) -> I,
    mut step_fn: impl FnMut(u64, &I) -> bool,
) -> FixedClockAdvance {
    assert_finite_non_negative(render_dt, "render dt must be a finite non-negative number");

    state.accumulator += render_dt;
    let mut result = FixedClockAdvance {
        ticks: 0,
        first_tick: None,
        last_tick: None,
        dropped_ticks: 0,
        stopped: false,
    };

    while state.accumulator + EPSILON >= TICK_SECONDS && result.ticks < MAX_TICKS_PER_UPDATE {
        let tick = state.tick;
        let input = input_for_tick(tick);
        let (_, continue_) = step(state, &input, &mut step_fn);
        state.accumulator -= TICK_SECONDS;
        if state.accumulator < 0.0 {
            state.accumulator = 0.0;
        }
        result.ticks += 1;
        if result.first_tick.is_none() {
            result.first_tick = Some(tick);
        }
        result.last_tick = Some(tick);
        if !continue_ {
            state.accumulator = 0.0;
            result.stopped = true;
            return result;
        }
    }

    if state.accumulator + EPSILON >= TICK_SECONDS {
        let dropped = ((state.accumulator + EPSILON) / TICK_SECONDS).floor() as u64;
        state.accumulator -= dropped as f64 * TICK_SECONDS;
        if state.accumulator < 0.0 {
            state.accumulator = 0.0;
        }
        state.dropped_ticks += dropped;
        state.overloads += 1;
        result.dropped_ticks = dropped;
    }

    result
}
