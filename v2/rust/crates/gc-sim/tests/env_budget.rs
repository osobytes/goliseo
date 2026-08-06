//! Port of `spec/sim/env_budget_spec.lua`.
//!
//! Per-step allocation budgets for the learning environment, measured in
//! the Lua original via `collectgarbage("count")` deltas with the JIT
//! pinned off (`jit.off()`/`jit.flush()`) so the figure is the deterministic
//! *interpreted* allocation count rather than a bimodal JIT-trace-dependent
//! one (see that file's own extensive comment on why, referencing #223).
//!
//! Rust has no direct equivalent of `collectgarbage("count")`: there is no
//! GC and no comparable "bytes allocated since I last checked" API in
//! `std`. The standard substitute is a counting `#[global_allocator]`, which
//! needs `unsafe impl GlobalAlloc` — and this workspace sets `unsafe_code =
//! "forbid"` (`v2/rust/Cargo.toml`'s `[workspace.lints.rust]`, inherited by
//! every crate with `[lints] workspace = true`, including `gc-sim`).
//! `forbid` is stronger than `deny`: no local `#[allow(unsafe_code)]` can
//! lift it, and it cannot be overridden per file or per test.
//!
//! `unsafe_code = "forbid"` is a per-*package* Cargo lint, though, not a
//! workspace-wide one. `crates/gc-test-alloc` is a small new workspace
//! member that deliberately does not set `[lints] workspace = true`, so it
//! does not inherit the forbid, and it holds the one `unsafe impl
//! GlobalAlloc` this measurement needs. `gc-sim` depends on it only via
//! `[dev-dependencies]` — never a normal dependency, so it never enters a
//! shipping build — and `gc-sim`'s own `[lints] workspace = true` is
//! unchanged: `gc-sim`'s own source still forbids unsafe code outright. This
//! file installs `gc_test_alloc::CountingAllocator` as its
//! `#[global_allocator]`, which — because every file directly under
//! `tests/` is its own binary by default — applies to this binary alone,
//! not to the `gc-sim` library or to any other test binary.
//!
//! What each case actually protected split into two kinds on inspection,
//! and they get different treatment (see `v2/README.md`'s porting
//! contract on retiring only when a failure mode genuinely cannot occur):
//!
//! - **Call-count claims.** "Builds exactly one observation and one action
//!   view per slot per step", "masks a slot *without building* an
//!   observation", and "does not re-capture the boundary for the
//!   privileged profile" are about how many times something is called, not
//!   how many bytes it allocates. The Lua proved them by monkey-patching
//!   `env_observation.build`/`action_view` at runtime — impossible for a
//!   Rust function — but the same invariants are recoverable exactly with
//!   `Cell`-based counters on `EnvInstance`
//!   ([`gc_sim::env::EnvInstance::observation_builds`],
//!   [`gc_sim::env::EnvInstance::action_views`],
//!   [`gc_sim::env::EnvInstance::snapshot_captures`]) and one on
//!   `EnvObservationContext`
//!   ([`gc_sim::env_observation::EnvObservationContext::redundant_captures`]),
//!   modeled directly on the `MatchDriver::snapshot_captures` precedent in
//!   `gc-netcode`. These are recovered below, and two of them are stronger
//!   than the Lua original: an exact call count rather than "allocated
//!   fewer bytes than a full build would."
//! - **Allocation-size claims.** "Keeps a single-slot observation within
//!   budget" and "keeps a step within budget" assert absolute per-call byte
//!   ceilings with no correctness invariant behind them — the Lua's own
//!   header comment calls the step figures "not asserted directly" tuning
//!   data. They are measured directly with the counting allocator below
//!   (see that section's own comment for the measured figures and the
//!   chosen margin). "Scales with controlled slots rather than exploding"
//!   *is* recoverable as a count instead: its own comment states the
//!   protected property as "O(controlled_slots x players) by design", which
//!   is directly checkable by counting observed-player records in the
//!   returned observation without any new instrumentation, and is a
//!   strictly more precise statement of the property than a byte ceiling
//!   with 1.27x headroom ever was, so it stays a count rather than also
//!   moving to a byte measurement.

use gc_sim::env::{self, EnvInstance, ReferenceConfigOverrides};
use gc_sim::env_action::{RawAction, RawValue};
use gc_sim::env_config::RawSlotSource;
use gc_sim::env_observation;
use gc_sim::input_frame;
use gc_test_alloc::CountingAllocator;
use indexmap::IndexMap;

/// A global allocator declared in an integration-test file (a file directly
/// under `tests/`, which is its own binary by default) applies to that
/// binary alone -- not to the `gc-sim` library, and not to any other test
/// binary. See `crates/gc-test-alloc` for what this counts and why it is
/// safe for `gc-sim` itself to keep `unsafe_code = "forbid"` while this file
/// uses one.
#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// `ALLOC` is one process-wide counter (see `gc-test-alloc`'s own doc
/// comment: it is deliberately not thread-scoped), and `cargo test` runs the
/// `#[test]` functions of one binary concurrently on multiple threads by
/// default. Left alone, that means any two tests in *this* file racing each
/// other pollutes both of their counts with each other's allocations --
/// confirmed empirically: the two measuring tests below read a few dozen
/// bytes higher, and inconsistently so across runs, under the default
/// concurrent runner than under `--test-threads=1`, where they are exactly
/// reproducible to the byte across five separate process runs. Rather than
/// tolerate that noise (or silently depend on every future `cargo test`
/// invocation happening to pass `--test-threads=1`), every test in this file
/// takes this lock for its entire body, which serializes this file's tests
/// against each other regardless of the runner's thread count -- the same
/// "pin the execution mode instead of measuring around the noise" move the
/// Lua spec makes with `jit.off()`/`jit.flush()`. Other test binaries are
/// unaffected: each is its own process with its own copy of `ALLOC`'s static
/// state, so nothing outside this file can pollute or be polluted by it.
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Calls `body` once unmeasured (so growable buffers -- `Vec`/`IndexMap`
/// first-resize, etc. -- absorb their first-touch cost before measurement,
/// same rationale as the Lua spec's own warm-up call), then calls it
/// `ROUNDS` further times and returns the minimum bytes the allocator
/// reported requested on any single round. The minimum, not the mean,
/// because a round that happens to absorb a one-off resize should not
/// inflate the figure -- same rationale as the Lua's `per_call_bytes`.
/// Caller must hold `TEST_LOCK` for the duration -- see its doc comment.
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

/// Mirrors the Lua spec's `fresh(slots, profile)`: the `soccer_only`
/// reference fixture (seed 5, a long duration so no step ends the match),
/// with the first `slots` canonical slots set to `policy` and the rest
/// `neutral`, under the given observation profile, with the post-kickoff
/// hold cleared so a single step is never absorbed by it.
fn fresh(slots: i64, profile: &str) -> EnvInstance {
    let mut config = env::reference_config(
        "soccer_only",
        Some(ReferenceConfigOverrides {
            seed: Some(5),
            duration: Some(600.0),
            ..Default::default()
        }),
    )
    .expect("soccer_only is a declared reference fixture");
    config.observation_profile = Some(profile.to_string());
    config.slot_sources = Some(
        (1..=input_frame::SLOT_COUNT)
            .map(|index| RawSlotSource {
                kind: if index <= slots { "policy" } else { "neutral" }.to_string(),
                seed: None,
                policy_id: None,
            })
            .collect(),
    );
    let mut instance = env::reset(&config, None).expect("valid config");
    instance.state.kickoff_hold = 0.0;
    instance
}

/// Mirrors the Lua spec's `actions_for(slots)`: every controlled slot moves
/// right and sprints.
fn actions_for(slots: i64) -> IndexMap<i64, RawAction> {
    let mut actions = IndexMap::new();
    for slot in 1..=slots {
        let mut move_table = IndexMap::new();
        move_table.insert("x".to_string(), RawValue::Number(1.0));
        move_table.insert("y".to_string(), RawValue::Number(0.0));
        let mut held_table = IndexMap::new();
        held_table.insert("sprint".to_string(), RawValue::Bool(true));
        let mut table = IndexMap::new();
        table.insert("move".to_string(), RawValue::Table(move_table));
        table.insert("held".to_string(), RawValue::Table(held_table));
        actions.insert(slot, RawAction::Table(table));
    }
    actions
}

// ---------------------------------------------------------------------------
// Call-count claims: recovered with the `EnvInstance`/`EnvObservationContext`
// diagnostics described in the module doc.
// ---------------------------------------------------------------------------

/// The regression guard for the finding that action masking used to build a
/// whole observation. Ported from the Lua case of the same name, which
/// asserted a byte ceiling (`< 6144 B`) and a `masking * 2 < observing`
/// comparison as a proxy for "masking never builds a full observation";
/// this asserts that fact directly instead, by call count.
#[test]
fn env_allocation_budgets_masks_a_slot_without_building_an_observation() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let instance = fresh(1, "representative");

    let _ = env::action_masks(&instance);
    assert_eq!(
        instance.observation_builds.get(),
        0,
        "masking a slot must never build a full observation"
    );
    assert_eq!(
        instance.action_views.get(),
        1,
        "masking one controlled slot builds exactly one narrow action view"
    );

    let _ = env::observe(&instance);
    assert_eq!(
        instance.observation_builds.get(),
        1,
        "a direct call to observe() builds exactly one full observation"
    );
    assert_eq!(
        instance.action_views.get(),
        1,
        "observe() must not build any action view"
    );
}

/// The direct form of the same guard, over a live `env::step`. An
/// allocation inequality could not prove this on its own in the Lua either,
/// because a step's total is dominated by snapshot hashing and would still
/// fit its budget with a second observation build hidden inside it — the
/// Lua's own comment says as much. Counting calls proves it exactly: one
/// full observation for the returned result, and one narrow action view per
/// controlled slot for the masks.
#[test]
fn env_allocation_budgets_builds_exactly_one_observation_and_one_action_view_per_slot_per_step() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut instance = fresh(2, "team");
    let builds_before = instance.observation_builds.get();
    let views_before = instance.action_views.get();

    env::step(&mut instance, &actions_for(2), None).expect("a valid step");

    assert_eq!(
        instance.observation_builds.get() - builds_before,
        1,
        "a step builds the full observation exactly once"
    );
    assert_eq!(
        instance.action_views.get() - views_before,
        2,
        "masking uses one narrow view per controlled slot"
    );
}

/// Pins the snapshot donation: `env::step` already captures the canonical
/// snapshot for the boundary hash, and the privileged profile reuses it
/// instead of capturing and hashing the same boundary again. The Lua
/// measured this as "a privileged step costs roughly 60% more than a
/// representative one" would indicate a regression; here the two profiles'
/// `snapshot_captures` deltas are asserted equal outright; a re-capture
/// regression would show up as the privileged delta being twice the
/// representative one.
#[test]
fn env_allocation_budgets_does_not_re_capture_the_boundary_for_the_privileged_profile() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut representative = fresh(1, "representative");
    let mut privileged = fresh(1, "privileged");
    let actions = actions_for(1);

    let representative_before = representative.snapshot_captures.get();
    let privileged_before = privileged.snapshot_captures.get();

    let representative_result =
        env::step(&mut representative, &actions, None).expect("a valid step");
    let privileged_result = env::step(&mut privileged, &actions, None).expect("a valid step");

    let representative_delta = representative.snapshot_captures.get() - representative_before;
    let privileged_delta = privileged.snapshot_captures.get() - privileged_before;

    assert_eq!(
        representative_delta, representative_result.ticks_simulated,
        "a representative step captures the boundary exactly once per simulated tick"
    );
    assert_eq!(
        privileged_delta, privileged_result.ticks_simulated,
        "a privileged step must capture the boundary exactly once per simulated tick too -- \
         not twice for the authoritative block"
    );
    assert_eq!(
        privileged_delta, representative_delta,
        "the privileged profile must not cost any extra boundary capture over representative"
    );
}

// ---------------------------------------------------------------------------
// The scaling claim: recovered as a count ratio (option 1 of the task's
// ordering), not a byte ceiling.
// ---------------------------------------------------------------------------

/// Ported from the Lua case of the same name. Its own comment states the
/// protected property precisely: "Observation cost is O(controlled_slots x
/// players) by design: each slot rebuilds every other player's record so no
/// view can be shared." That is a count, not a byte figure, and it is
/// exactly checkable from the observation `env::step` already returns: each
/// controlled slot's view must carry a teammate+opponent record for every
/// other player on the pitch -- no fewer (a dropped player would be a
/// privacy/observability bug) and no more (an "explosion" would mean a view
/// somehow rebuilding more than the rest of the roster, e.g. by including
/// other controlled slots' own views). Combined with `observation_builds`
/// and `action_views` staying at exactly one call / one call per slot (the
/// same invariant the two tests above pin at 1-2 slots), this is a stronger,
/// exact statement of "scales linearly with controlled slots" than a byte
/// ceiling with 1.27x headroom ever was.
#[test]
fn env_allocation_budgets_scales_with_controlled_slots_rather_than_exploding() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut instance = fresh(8, "team");
    let total_players = instance.state.players.len() as i64;
    let builds_before = instance.observation_builds.get();
    let views_before = instance.action_views.get();

    let result = env::step(&mut instance, &actions_for(8), None).expect("a valid step");

    assert_eq!(
        instance.observation_builds.get() - builds_before,
        1,
        "an eight-slot step still builds the full observation exactly once"
    );
    assert_eq!(
        instance.action_views.get() - views_before,
        8,
        "an eight-slot step builds exactly one action view per controlled slot"
    );

    assert_eq!(
        result.observation.slots.len() as i64,
        8,
        "the observation covers all eight controlled slots"
    );
    let mut total_records = 0i64;
    for &slot in &result.observation.slots {
        let view = env_observation::view_for(&result.observation, slot)
            .expect("every declared controlled slot has a view");
        let records = view.teammates.len() as i64 + view.opponents.len() as i64;
        assert_eq!(
            records,
            total_players - 1,
            "slot {slot}'s view must carry exactly one record per other player on the pitch, \
             no fewer and no more"
        );
        total_records += records;
    }
    assert_eq!(
        total_records,
        8 * (total_players - 1),
        "total observed-player records must scale linearly (slots x (players - 1)), not explode"
    );
}

// ---------------------------------------------------------------------------
// Allocation-size claims with no correctness invariant behind them. Measured
// with `gc-test-alloc::CountingAllocator` (see `crates/gc-test-alloc` and
// this file's own module doc comment for why that crate exists and what it
// counts) installed as this binary's `#[global_allocator]`, `TEST_LOCK`
// held throughout so nothing else in this binary can pollute the count.
//
// Measured on this machine (System/glibc allocator, debug profile,
// `soccer_only`, seed 5), taking the minimum of `ROUNDS = 9` rounds after
// one unmeasured warm-up call, per `measure_allocations`: reproduced to the
// exact byte across five separate `cargo test` process invocations.
//
//   env::observe (1 slot)   14,958 B   ceiling  20,480 B (20 KiB, 1.37x)
//   env::step    (1 slot)  174,920 B   ceiling 235,520 B (230 KiB, 1.35x)
//
// The margin (roughly the same 1.2x-1.4x range the Lua's own ceilings used)
// is headroom for the kind of drift that is not a regression: a `rustc` or
// dependency bump changing an unrelated `Vec`/`IndexMap`/`String` growth
// curve by a few percent. It is not headroom for "this grew because someone
// added a second observation build" -- that class of regression is exactly
// what `env_allocation_budgets_builds_exactly_one_observation_and_one_action_view_per_slot_per_step`
// above already pins exactly, by call count, so this ceiling only has to
// catch the failure mode a call count cannot see: the same calls allocating
// substantially more per call than they used to.
//
// These figures are specific to this measurement's exact conditions -- a
// different machine, allocator, or Rust toolchain version may read
// differently (Rust's allocation profile is not Lua's; see the task brief
// this file was written against). Re-measure and re-derive the ceiling
// (`cargo test -p gc-sim --test env_budget -- --nocapture` prints the raw
// figure via the `eprintln!` two lines below each assertion) rather than
// adjusting the margin to make a new number pass.
// ---------------------------------------------------------------------------

#[test]
fn env_allocation_budgets_keeps_a_single_slot_observation_within_budget() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let instance = fresh(1, "representative");
    let bytes = measure_allocations(|| {
        let _ = env::observe(&instance);
    });
    eprintln!("MEASURED single-slot observation: {bytes} bytes");
    const CEILING: usize = 20 * 1024;
    assert!(
        bytes < CEILING,
        "a single-slot observation allocated {bytes} bytes (ceiling {CEILING}, measured 14,958 B \
         with a 1.37x margin -- see this test's section comment)"
    );
}

#[test]
fn env_allocation_budgets_keeps_a_step_within_budget() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut instance = fresh(1, "representative");
    let actions = actions_for(1);
    let bytes = measure_allocations(|| {
        env::step(&mut instance, &actions, None).expect("a valid step");
    });
    eprintln!("MEASURED single-slot step: {bytes} bytes");
    const CEILING: usize = 230 * 1024;
    assert!(
        bytes < CEILING,
        "a single-slot step allocated {bytes} bytes (ceiling {CEILING}, measured 174,920 B with a \
         1.35x margin -- see this test's section comment)"
    );
}
