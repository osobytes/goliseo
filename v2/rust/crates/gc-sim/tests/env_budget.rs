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
#[ignore = "retired: measures a per-call allocation budget via LuaJIT collectgarbage() \
            with the JIT pinned off, which Rust has no equivalent for (see module doc); \
            the invariant the budget was a regression proxy for -- action masking never \
            reads full-observation data -- is covered structurally rather than by a \
            byte count: env_action::EnvActionView (src/env_action.rs) is a strict \
            own+ball subset with no teammates/opponents/match fields to read, \
            env::action_masks (src/env.rs) calls env_observation::action_view rather \
            than env_observation::build, and \
            env_action_mask_derives_legality_from_the_view_alone (tests/env_action.rs) \
            proves mask() derives every legality bit from that narrow view alone. No \
            test covers the numeric byte budget itself; none can without new \
            counting-allocator infrastructure this port does not own."]
fn env_allocation_budgets_masks_a_slot_without_building_an_observation() {}

#[test]
#[ignore = "retired: measures a per-call allocation budget via LuaJIT collectgarbage() \
            with the JIT pinned off, which Rust has no equivalent for (no GC, no \
            per-call allocation counter without new counting-allocator infrastructure \
            this port does not own; see module doc). This is a pure performance \
            ceiling, not a correctness invariant, so no other test covers it."]
fn env_allocation_budgets_keeps_a_single_slot_observation_within_budget() {}

#[test]
#[ignore = "retired: measures a per-call allocation budget via LuaJIT collectgarbage() \
            with the JIT pinned off, which Rust has no equivalent for (see module doc). \
            This is a pure performance ceiling, not a correctness invariant, so no \
            other test covers it."]
fn env_allocation_budgets_keeps_a_step_within_budget() {}

#[test]
#[ignore = "retired: monkey-patches env_observation.build/action_view at runtime to \
            count calls, which Rust functions cannot be (not runtime-replaceable) \
            without a new injectable counting seam gc_sim::env::step does not have and \
            this port does not own (see module doc). The call-count invariant -- \
            exactly one observation build and one action view per controlled slot, per \
            step -- has no test double: env::step's (src/env.rs) single call site for \
            each of env_observation::build and env_observation::action_view is visible \
            in source, but no test asserts the call count itself."]
fn env_allocation_budgets_builds_exactly_one_observation_and_one_action_view_per_slot_per_step() {}

#[test]
#[ignore = "retired: measures a per-call allocation budget via LuaJIT collectgarbage() \
            with the JIT pinned off, which Rust has no equivalent for (see module doc). \
            The invariant this budget was a regression proxy for -- the privileged \
            profile reuses env::step's single per-tick match_snapshot::capture call \
            (src/env.rs) rather than capturing the boundary a second time -- has no \
            test double: that call site is visibly singular in source, but no test \
            asserts capture is called exactly once per tick regardless of profile."]
fn env_allocation_budgets_does_not_re_capture_the_boundary_for_the_privileged_profile() {}

#[test]
#[ignore = "retired: measures a per-call allocation budget via LuaJIT collectgarbage() \
            with the JIT pinned off, which Rust has no equivalent for (see module doc). \
            This is a pure performance-scaling ceiling, not a correctness invariant, so \
            no other test covers it."]
fn env_allocation_budgets_scales_with_controlled_slots_rather_than_exploding() {}
