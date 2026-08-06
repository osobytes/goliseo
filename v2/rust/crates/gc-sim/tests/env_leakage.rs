//! Port of `spec/sim/env_leakage_spec.lua`.
//!
//! This spec's real subject is `sim/env.lua` (`require("sim.env")` is the
//! first line after the test runner), not `sim/env_observation.lua` alone.
//! Every one of its eight cases drives a full episode through
//! `env.reset`/`env.step`/`env.observe`, and several are load-bearing on
//! genuine multi-tick simulation:
//!
//!   - "carries no future tick, at arbitrary lookahead depth" and "carries no
//!     opponent same-tick input" replay two `InputTape`s that diverge at a
//!     chosen tick, step both instances tick by tick, and compare
//!     `boundary_hashes` across the run — this needs `env.step`'s real
//!     physics/AI/RNG advancement, not just `env_observation`'s view-building.
//!   - "cannot leak one controlled slot's choice to another" steps a two-slot
//!     `env` instance with different slot-5 actions and compares the
//!     resulting `boundary_hash`.
//!   - "reports only confirmed past events, never post-action labels" runs a
//!     bot-driven fixture for up to 240 ticks waiting for real gameplay
//!     events (shots, tackles, ...) to occur.
//!
//! `sim/env.lua` requires `sim/match.lua` for exactly this — the per-tick
//! physics/AI/rollback update loop — and at the time this port was written
//! `src/match.rs` (`sim/match.lua`'s port) was a placeholder
//! (`// NOT YET PORTED`), same as `src/env.rs`. Per this crate's task scope,
//! this file is not the owner of either module and must not port them to
//! unblock these tests.
//!
//! `tests/env_observation.rs` ported `spec/sim/env_observation_spec.lua` for
//! real despite that spec *also* nominally requiring `sim.env`, because every
//! one of ITS assertions turned out to depend only on a static `MatchState`
//! snapshot (view-building, not stepping) — see that file's module doc. Three
//! of this spec's eight cases (RNG-state hiding, resolver-verdict hiding, the
//! privileged-profile tagging check) have that same shape at first glance,
//! but two of them collide with a second, independent scope decision this
//! port made: `EnvPrivilegedView.snapshot`'s canonical encoding is
//! `match_snapshot::hash(&self.snapshot)` (a hash digest), not a full
//! field-by-field walk of `MatchSnapshot`'s own nested state (see
//! `src/env_observation.rs`'s module doc for why). "Hides the authoritative
//! RNG state..." and "hides resolver verdicts..." both require a *positive
//! control* — the privileged encoding must actually contain the raw value
//! being checked for — which a hash digest structurally cannot provide. So
//! rather than port three of eight and leave five behind (fracturing the
//! spec across two different unblocking stories), every case here is kept
//! together and stubbed, so the eventual `sim::match`/`sim::env` port lands
//! the whole spec at once against real stepping.
//!
//! Every case below is `#[ignore]`d with the exact reason string this
//! crate's task greps for.

#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn hides_the_authoritative_rng_state_that_the_privileged_profile_exposes() {}

#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn hides_resolver_verdicts_that_the_privileged_snapshot_still_records() {}

#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn names_no_private_state_field_anywhere_in_a_player_observable_view() {}

#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn tags_the_privileged_profile_as_neither_observable_nor_a_human_proxy() {}

#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn carries_no_future_tick_at_arbitrary_lookahead_depth() {}

#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn carries_no_opponent_same_tick_input() {}

#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn cannot_leak_one_controlled_slots_choice_to_another_in_the_team_profile() {}

#[test]
#[ignore = "needs sim::match (sim/match.lua), not yet ported"]
fn reports_only_confirmed_past_events_never_post_action_labels() {}
