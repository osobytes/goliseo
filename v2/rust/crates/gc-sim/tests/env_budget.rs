//! Port of `spec/sim/env_budget_spec.lua`.
//!
//! Per-step allocation budgets for the learning environment, measured in
//! the Lua original via `collectgarbage("count")` deltas with the JIT
//! pinned off (`jit.off()`/`jit.flush()`) so the figure is the deterministic
//! *interpreted* allocation count rather than a bimodal JIT-trace-dependent
//! one (see that file's own extensive comment on why, referencing #223).
//!
//! None of this has a Rust equivalent, for two independent reasons, both
//! fatal on their own:
//!
//! 1. **No GC to measure.** `collectgarbage("count")` reads LuaJIT's
//!    tracked heap size. Rust has no garbage collector and no comparable
//!    "bytes allocated since I last checked" API in `std` — measuring
//!    per-call heap growth would need a custom global allocator
//!    (`#[global_allocator]`) wrapping `System` with an atomic counter, a
//!    new piece of infrastructure this porting task does not own and the
//!    top-level task rules forbid adding a new crate dependency for (a
//!    hand-rolled counting allocator needs no extra dependency, but it is
//!    still new production-facing infrastructure, not a mechanical port of
//!    `sim/env_budget_spec.lua`'s *content* — the whole measurement
//!    technique is LuaJIT-specific and does not carry over).
//! 2. **No monkey-patchable functions.** "builds exactly one observation
//!    and one action view per slot per step" replaces
//!    `env_observation.build`/`env_observation.action_view` with counting
//!    wrappers at runtime, then restores them. Rust functions are not
//!    runtime-replaceable; there is no analogous seam in
//!    [`gc_sim::env::step`] to inject a counting wrapper without changing
//!    its public signature for a test-only concern.
//!
//! Every case below is `#[ignore]`d rather than silently dropped — see
//! `v2/README.md` §4 ("if a test genuinely cannot be expressed, port it as
//! `#[ignore]`... and report it").

#[test]
#[ignore = "measures LuaJIT collectgarbage() byte deltas with the JIT pinned \
            off; Rust has no GC and no comparable per-call allocation counter \
            without new counting-allocator infrastructure this port does not \
            own (see module doc)"]
fn env_allocation_budgets_masks_a_slot_without_building_an_observation() {}

#[test]
#[ignore = "measures LuaJIT collectgarbage() byte deltas with the JIT pinned \
            off; Rust has no GC and no comparable per-call allocation counter \
            without new counting-allocator infrastructure this port does not \
            own (see module doc)"]
fn env_allocation_budgets_keeps_a_single_slot_observation_within_budget() {}

#[test]
#[ignore = "measures LuaJIT collectgarbage() byte deltas with the JIT pinned \
            off; Rust has no GC and no comparable per-call allocation counter \
            without new counting-allocator infrastructure this port does not \
            own (see module doc)"]
fn env_allocation_budgets_keeps_a_step_within_budget() {}

#[test]
#[ignore = "monkey-patches env_observation.build/action_view at runtime to \
            count calls; Rust functions are not runtime-replaceable and \
            gc_sim::env::step has no injectable counting seam for this \
            test-only concern (see module doc)"]
fn env_allocation_budgets_builds_exactly_one_observation_and_one_action_view_per_slot_per_step() {}

#[test]
#[ignore = "measures LuaJIT collectgarbage() byte deltas with the JIT pinned \
            off; Rust has no GC and no comparable per-call allocation counter \
            without new counting-allocator infrastructure this port does not \
            own (see module doc)"]
fn env_allocation_budgets_does_not_re_capture_the_boundary_for_the_privileged_profile() {}

#[test]
#[ignore = "measures LuaJIT collectgarbage() byte deltas with the JIT pinned \
            off; Rust has no GC and no comparable per-call allocation counter \
            without new counting-allocator infrastructure this port does not \
            own (see module doc)"]
fn env_allocation_budgets_scales_with_controlled_slots_rather_than_exploding() {}
