//! Allocation coverage for `gc_sim::ball_prediction` (#494).
//!
//! The service is designed to be called once per query per caller per tick,
//! and every query issued during a resimulated tick is re-issued -- so its
//! real cost multiplies by rollback depth, up to `120 x 31 = 3,720` scratch
//! steps in the worst retained frame (see `STEP_BUDGET_DEFAULT`'s doc
//! comment in `src/ball_prediction.rs`). Its zero-allocation behavior was
//! previously sound only "by inspection": `Vec::clear()` retains capacity,
//! `BallFlight`/`BallArena`/`Vec2` are all `Copy`, and `sync()` rebuilds only
//! on an actual key change. Nothing asserted any of that -- this file does.
//!
//! Measured with `gc_test_alloc::CountingAllocator`, the same pattern
//! `gc-sim/tests/env_budget.rs` uses for another hot per-tick path. See that
//! file's module doc for why a counting `#[global_allocator]` needs its own
//! crate (`unsafe impl GlobalAlloc`) carved out of `unsafe_code = "forbid"`,
//! and why every test in a file that installs one takes a shared lock for
//! its entire body (the counter is one process-wide, not thread-scoped).
//!
//! ## What "steady state" means here
//!
//! `BallPredictor::begin_tick` bumps `tick_seq`, which is half of the cache
//! key -- so in the *realistic* worst case (a live tick, or a resimulated
//! tick replaying it), the very first query of every tick forces a rebuild:
//! `samples.clear()` (retains capacity) followed by regrowing the buffer
//! back out via repeated `extend_one()`. "Steady state" is therefore not
//! "never rebuilds" -- it is "the buffer's `Vec` capacity has already grown
//! to the full-horizon high-water mark, so every later rebuild-and-regrow
//! cycle reuses that capacity instead of reallocating." Each measured round
//! below calls `begin_tick()` before its query burst for exactly that
//! reason: skipping it would measure a cache hit that ties no tick boundary,
//! which is not the shape a resimulated tick actually produces.
//!
//! Every measurement excludes fixture construction (`MatchState`,
//! `BallPredictor::default()`) and the first rebuild-from-empty by running
//! one unmeasured warm-up round before `ALLOC.start()` -- mirroring
//! `env_budget.rs::measure_allocations`. The first-rebuild cost is not
//! ignored; it gets its own pinned-ceiling test below instead of leaking
//! into the steady-state figures.

use gc_core::vec2::Vec2;
use gc_sim::ball_prediction::{
    BallAxis, BallEstimateSource, BallPlane, BallPredictionConfig, BallPredictor,
};
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{MatchState, PitchSize};
use gc_test_alloc::CountingAllocator;

/// See this file's module doc and `gc-test-alloc`/`env_budget.rs` for why
/// this is safe alongside `gc-sim`'s own `unsafe_code = "forbid"`: this is a
/// `[dev-dependencies]`-only crate, and a `#[global_allocator]` declared in
/// one `tests/*.rs` file applies to that file's binary alone.
#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// `ALLOC` is one process-wide, not thread-scoped (see `gc-test-alloc`'s own
/// doc comment), and `cargo test` runs a binary's `#[test]` functions
/// concurrently by default. Every test here holds this lock for its entire
/// body so this file's tests cannot pollute each other's counts, regardless
/// of the runner's thread count -- the same fix `env_budget.rs` applies for
/// the same reason.
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Calls `body` once unmeasured (absorbing fixture-construction and
/// first-rebuild cost), then `ROUNDS` further times, returning the minimum
/// bytes reported allocated on any single round. The minimum, not the mean,
/// so that a round which happens to absorb a one-off resize does not inflate
/// the figure. Caller must hold `TEST_LOCK` for the duration.
const ROUNDS: usize = 9;

fn measure_allocations(mut body: impl FnMut()) -> usize {
    body();
    let mut best = usize::MAX;
    for _ in 0..ROUNDS {
        ALLOC.start();
        body();
        best = best.min(ALLOC.allocated());
    }
    best
}

// ---------------------------------------------------------------------
// Fixtures -- a loose, airborne ball with a trajectory that resolves every
// query kind (a distance covered, an X-plane crossed, a height reached)
// well inside the default two-second horizon. Deliberately not stepped
// through the live simulation anywhere in this file: every test here only
// ever queries a fixed `MatchState`, so nothing here needs the players
// parked out of the ball's way the way `ball_prediction.rs`'s accuracy
// fixture does.
// ---------------------------------------------------------------------

fn new_state() -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula is authored");
    let away = gc_data::teams::get("orion").expect("orion is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(120.0),
        max_goals: Some(99),
        seed: Some(733.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    })
}

/// A loose, airborne ball: no owner, moving +x and briefly upward before
/// gravity brings it back down. Never mutated by anything in this file, so
/// every round queries the exact same fingerprint -- only `tick_seq`
/// advances, which is what forces the per-tick rebuild this file measures
/// through.
fn flying_ball_state() -> MatchState {
    let mut state = new_state();
    state.owner = None;
    state.ball = Vec2::new(220.0, 60.0);
    state.ball_vel = Vec2::new(520.0, -40.0);
    state.ball_z = 90.0;
    state.ball_vz = 120.0;
    state.ball_spin = 0.0;
    state.pickup_cd = 0.0;
    state
}

/// One pass over every query kind plus the fallback flavor, per the issue's
/// coverage requirement ("all four query kinds plus the fallback flavor,
/// not only `position_at_time`"):
///
/// 1. `position_at_time` -- also the call that grows (or, once warm,
///    re-confirms) the buffer out to just under the full horizon, so every
///    other kind below resolves from the buffer alone.
/// 2. `state_after_distance`
/// 3. `time_to_cross_plane`
/// 4. `time_to_height`
/// 5. `estimate_position_at_time`, sampled flavor (inside the buffer)
/// 6. `estimate_position_at_time`, fallback flavor (beyond the horizon,
///    where an authoritative query would refuse and this one degrades)
/// 7. `reachable_before_arrival` -- `&self`, no `sync()`, included for
///    completeness though it was never a candidate for allocating.
fn full_query_burst(predictor: &mut BallPredictor, state: &MatchState) {
    let horizon = predictor.config().max_horizon;

    let at = predictor
        .position_at_time(state, horizon - 0.001)
        .expect("resolves just inside the horizon");
    assert!(at.time > 0.0);

    let after = predictor
        .state_after_distance(state, 50.0)
        .expect("the ball covers 50 px well inside the horizon");
    assert!(after.path >= 50.0);

    let crossing = predictor
        .time_to_cross_plane(
            state,
            BallPlane {
                axis: BallAxis::X,
                coord: 400.0,
            },
        )
        .expect("the ball crosses x = 400 inside the horizon");
    assert!(crossing > 0.0);

    let grounded = predictor
        .time_to_height(state, 0.0)
        .expect("a ball thrown up comes back down inside the horizon");
    assert!(grounded > 0.0);

    let sampled = predictor.estimate_position_at_time(state, horizon * 0.5);
    assert_eq!(sampled.source, BallEstimateSource::Sampled);

    let fallback = predictor.estimate_position_at_time(state, horizon + 1.0);
    assert_eq!(fallback.source, BallEstimateSource::Fallback);

    let point = Vec2::new(state.ball.x, state.ball.y);
    let reach = predictor.reachable_before_arrival(&state.players[0], point, 0.5);
    assert!(reach.arrival >= 0.0);
}

/// A predictor whose sample buffer has already grown to its full-horizon
/// high-water mark, and the fixed state it grew against. Building this is
/// itself unmeasured (fixture construction, plus the very first rebuild --
/// see `first_rebuild_from_an_empty_buffer` below for that cost measured on
/// its own).
fn warmed() -> (BallPredictor, MatchState) {
    let state = flying_ball_state();
    let mut predictor = BallPredictor::default();
    predictor.begin_tick();
    full_query_burst(&mut predictor, &state);
    (predictor, state)
}

// ---------------------------------------------------------------------
// Steady state: buffer already grown to horizon size. Each round still
// calls `begin_tick()`, because that is what a resimulated tick actually
// does -- see the module doc's "What 'steady state' means here".
// ---------------------------------------------------------------------

#[test]
fn steady_state_full_query_burst_allocates_nothing() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut predictor, state) = warmed();

    let bytes = measure_allocations(|| {
        predictor.begin_tick();
        full_query_burst(&mut predictor, &state);
    });

    assert_eq!(
        bytes, 0,
        "a steady-state burst covering all four query kinds plus the fallback flavor \
         allocated {bytes} bytes; the service is documented as zero-allocation per query \
         once the buffer has grown to the horizon"
    );
}

#[test]
fn steady_state_position_at_time_allocates_nothing() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut predictor, state) = warmed();
    let horizon = predictor.config().max_horizon;

    let bytes = measure_allocations(|| {
        predictor.begin_tick();
        let _ = predictor.position_at_time(&state, horizon - 0.001);
    });

    assert_eq!(
        bytes, 0,
        "position_at_time allocated {bytes} bytes at steady state"
    );
}

#[test]
fn steady_state_state_after_distance_allocates_nothing() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut predictor, state) = warmed();

    let bytes = measure_allocations(|| {
        predictor.begin_tick();
        let _ = predictor.state_after_distance(&state, 50.0);
    });

    assert_eq!(
        bytes, 0,
        "state_after_distance allocated {bytes} bytes at steady state"
    );
}

#[test]
fn steady_state_time_to_cross_plane_allocates_nothing() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut predictor, state) = warmed();

    let bytes = measure_allocations(|| {
        predictor.begin_tick();
        let _ = predictor.time_to_cross_plane(
            &state,
            BallPlane {
                axis: BallAxis::X,
                coord: 400.0,
            },
        );
    });

    assert_eq!(
        bytes, 0,
        "time_to_cross_plane allocated {bytes} bytes at steady state"
    );
}

#[test]
fn steady_state_time_to_height_allocates_nothing() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut predictor, state) = warmed();

    let bytes = measure_allocations(|| {
        predictor.begin_tick();
        let _ = predictor.time_to_height(&state, 0.0);
    });

    assert_eq!(
        bytes, 0,
        "time_to_height allocated {bytes} bytes at steady state"
    );
}

#[test]
fn steady_state_estimate_position_at_time_sampled_flavor_allocates_nothing() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut predictor, state) = warmed();
    let horizon = predictor.config().max_horizon;

    let bytes = measure_allocations(|| {
        predictor.begin_tick();
        let estimate = predictor.estimate_position_at_time(&state, horizon * 0.5);
        assert_eq!(estimate.source, BallEstimateSource::Sampled);
    });

    assert_eq!(
        bytes, 0,
        "estimate_position_at_time (sampled flavor) allocated {bytes} bytes at steady state"
    );
}

#[test]
fn steady_state_estimate_position_at_time_fallback_flavor_allocates_nothing() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut predictor, state) = warmed();
    let horizon = predictor.config().max_horizon;

    let bytes = measure_allocations(|| {
        predictor.begin_tick();
        let estimate = predictor.estimate_position_at_time(&state, horizon + 1.0);
        assert_eq!(estimate.source, BallEstimateSource::Fallback);
    });

    assert_eq!(
        bytes, 0,
        "estimate_position_at_time (fallback flavor) allocated {bytes} bytes at steady state -- \
         the closed-form fallback in `fallback_estimate` takes no `Vec`/`String`/heap input, so \
         this is measuring `sync()` alone"
    );
}

#[test]
fn steady_state_reachable_before_arrival_allocates_nothing() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (predictor, state) = warmed();
    let point = Vec2::new(state.ball.x, state.ball.y);

    let bytes = measure_allocations(|| {
        let _ = predictor.reachable_before_arrival(&state.players[0], point, 0.5);
    });

    assert_eq!(
        bytes, 0,
        "reachable_before_arrival allocated {bytes} bytes; it is pure kinematics over &self \
         and never calls sync()"
    );
}

// ---------------------------------------------------------------------
// First-rebuild growth: measured and pinned, not left unbounded.
// ---------------------------------------------------------------------

// Measured on this machine (System/glibc allocator, debug profile), taking
// the minimum of `ROUNDS = 9` rounds after one unmeasured warm-up call, per
// `measure_allocations`, reproduced to the exact byte across repeated
// `cargo test` process invocations (both `--test-threads=1` and the default
// concurrent runner, since `TEST_LOCK` serializes this file's tests either
// way):
//
//   first rebuild (0 -> full horizon, 121 samples @ 64 B each)   8,192 B
//   ceiling                                                     10,240 B (10 KiB, 1.25x)
//
// 8,192 B is `Vec<BallSample>` growing by doubling from empty to a capacity
// of 128 (`BallSample` is 64 bytes: `time`/`z`/`vz`/`path` at 8 B each plus
// two `Vec2` at 16 B each) -- the smallest power-of-two capacity that fits
// 121 samples (one `t = 0` plus 120 scratch ticks at the default horizon and
// stride). The margin is headroom for the kind of drift that is not a
// regression -- a `rustc`/allocator/dependency bump nudging the growth
// curve -- not for "this grew because a query path now allocates on every
// call"; that failure mode is exactly what the steady-state tests above
// already catch, at an exact zero rather than a ceiling. Re-measure and
// re-derive the ceiling (`cargo test -p gc-sim --test ball_prediction_alloc
// -- --nocapture` prints the raw figure via the `eprintln!` below) rather
// than adjusting the margin to make a new number pass -- same policy as
// `env_budget.rs`.

#[test]
fn first_rebuild_from_an_empty_buffer_grows_within_a_pinned_ceiling() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let state = flying_ball_state();
    let horizon = BallPredictionConfig::default().max_horizon;

    // Every round below is deliberately a *fresh* `BallPredictor`: this
    // measures growing `samples` from empty out to the full-horizon
    // high-water mark, which only ever happens once per predictor's
    // lifetime (every rebuild after this one reuses the capacity, per the
    // steady-state tests above). `measure_allocations`'s "one unmeasured
    // warm-up round" therefore does not defeat this test -- it just costs
    // one extra fresh predictor, still measuring the same first-rebuild
    // shape as every other round.
    let bytes = measure_allocations(|| {
        let mut predictor = BallPredictor::default();
        predictor.begin_tick();
        let _ = predictor
            .position_at_time(&state, horizon - 0.001)
            .expect("resolves just inside the horizon");
    });

    eprintln!("MEASURED first-rebuild growth (position_at_time to full horizon): {bytes} bytes");
    const CEILING: usize = 10 * 1024;
    assert!(
        bytes < CEILING,
        "growing the sample buffer from empty to the full horizon allocated {bytes} bytes \
         (ceiling {CEILING}, measured 8,192 B with a 1.25x margin -- see this test's section \
         comment)"
    );
}

// ---------------------------------------------------------------------
// Go-red demonstration (AGENTS.md §9): a standing test proving this file's
// harness would actually catch a per-query allocation, not only a
// commit-message transcript of a manual patch-and-revert (the shape PR #502
// established for this repo: "red demonstrations are now standing tests").
// ---------------------------------------------------------------------

#[test]
fn the_steady_state_guard_actually_detects_a_deliberate_allocation() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut predictor, state) = warmed();

    // Stands in for exactly the class of regression the zero-byte
    // assertions above exist to catch: some query path performing a heap
    // allocation it has no business making. `black_box` on both the
    // allocation and its use keeps the optimizer from proving the value
    // unused and eliding the allocation entirely.
    const POISON_BYTES: usize = 64;
    let bytes = measure_allocations(|| {
        predictor.begin_tick();
        full_query_burst(&mut predictor, &state);
        let poison: Vec<u8> = std::hint::black_box(Vec::with_capacity(POISON_BYTES));
        std::hint::black_box(&poison);
    });

    assert!(
        bytes >= POISON_BYTES,
        "the harness failed to observe a deliberate {POISON_BYTES}-byte allocation inside the \
         measured window (observed {bytes} bytes) -- if this fires, the zero-allocation \
         assertions in this file are not actually protecting anything"
    );
}
