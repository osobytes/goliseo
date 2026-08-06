//! Match session lifecycle: the wasm-bindgen control surface a JS lobby/app
//! layer uses to create a match and advance it one tick at a time.
//!
//! ## Why input crosses as wire text, not fields
//!
//! [`Session::step`] takes a canonical [`gc_sim::input_frame`] wire string,
//! the exact same encoding the rollback netcode already puts on the network
//! and in the OMP-1 fixture. That format is already `String`-in/`String`-out
//! (`input_frame::decode`/`encode`), which `wasm-bindgen` binds natively
//! with no bespoke marshalling code — and it means a session driven from JS
//! consumes bit-identical input to a session driven from a recorded fixture,
//! which is the whole property this wasm build exists to prove.
//!
//! ## Scope
//!
//! This binds slot-mode play only ([`gc_sim::input_frame::InputOwnership`]),
//! not the legacy single-player `MatchInput` path — slot mode is what the
//! rollback netcode and the determinism fixture both use, and is the only
//! path this wave's acceptance test exercises. A session always runs
//! without combat (`combat_state: None` at every [`Session::step`]); wiring
//! `gc-sim`'s combat system through this surface is follow-up work, not
//! this wave's job.
//!
//! ## Where the `MatchState` actually lives
//!
//! `Session` itself is just a `u32` handle into [`crate::registry`]'s slab.
//! [`crate::render_export`]'s raw, non-`wasm-bindgen` per-frame exports look
//! the same entry up by that handle — see the registry module's doc for why
//! the state cannot simply live inside this struct.

use gc_data::teams;
use gc_render::frame::{self as render_frame, RenderFrameOptions};
use gc_render::frame_buffer;
use gc_sim::fixed_clock;
use gc_sim::input_frame::{self, InputFixtureRosters, InputOwnership, InputSlotAssignment};
use gc_sim::r#match as sim_match;
use gc_sim::match_snapshot::{self, PitchSize};
use gc_sim::tuning::Tuning;
use wasm_bindgen::prelude::*;

use crate::registry::{self, Entry};

const FIELD_W: f64 = 960.0;
const FIELD_H: f64 = 540.0;

/// Build the eight canonical slot assignments from two five-player rosters
/// (`[keeper, outfield_1, outfield_2, outfield_3, outfield_4]`), matching
/// the convention `gc_data::teams::TeamData::roster` and the OMP-1 fixture
/// both use: the roster's first id is the keeper and sits outside the
/// four controlled slots per side.
/// `pub(crate)`, not private: [`crate::rollback_events_bridge`]'s own tests
/// need a slot-mode boundary-zero `MatchState` (input_tick advances only in
/// slot mode — see `gc_sim::r#match::step`'s doc) with direct access to
/// `MatchState.events`, which [`Session`]'s own narrow getter surface does
/// not expose. Reusing this rather than a second copy keeps exactly one
/// canonical slot assignment for the fixture teams.
pub(crate) fn ownership(home_roster: &[&str; 5], away_roster: &[&str; 5]) -> InputOwnership {
    let mut slots = Vec::with_capacity(8);
    for index in 0..input_frame::SLOT_COUNT {
        let canonical = input_frame::slot(index + 1).expect("canonical slot index in range");
        let source = match canonical.team {
            input_frame::Team::Home => home_roster,
            input_frame::Team::Away => away_roster,
        };
        // Slots 0..4 are home outfielders, 4..8 are away outfielders (see
        // input_frame::SLOT_ORDER); within one side the outfield roster
        // positions (1..5) map onto the four slots in order.
        let side_index = if matches!(canonical.team, input_frame::Team::Home) {
            index
        } else {
            index - input_frame::HOME_SLOT_COUNT
        };
        slots.push(InputSlotAssignment {
            slot: canonical.id,
            team: canonical.team,
            player_id: source[(side_index + 1) as usize].to_string(),
        });
    }
    InputOwnership {
        version: input_frame::VERSION,
        rosters: InputFixtureRosters {
            home: home_roster.iter().map(|s| (*s).to_string()).collect(),
            away: away_roster.iter().map(|s| (*s).to_string()).collect(),
        },
        slots: slots.try_into().expect("exactly eight slots built"),
    }
}

/// One live match. Construct with [`Session::new`], advance with
/// [`Session::step`], read state with the getters below. Dropping the JS
/// wrapper (or calling the generated `free()`) releases the underlying
/// [`crate::registry`] slot.
#[wasm_bindgen]
pub struct Session {
    handle: u32,
}

impl Drop for Session {
    fn drop(&mut self) {
        registry::free(self.handle);
    }
}

#[wasm_bindgen]
impl Session {
    /// Start a new match between two authored teams
    /// (`gc_data::teams::ALL`'s ids, e.g. `"nebula"` / `"orion"`).
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if either team id is not authored
    /// content, or does not carry a five-player roster.
    #[wasm_bindgen(constructor)]
    pub fn new(
        home_team_id: &str,
        away_team_id: &str,
        seed: f64,
        duration_seconds: f64,
        max_goals: i32,
    ) -> Result<Session, JsValue> {
        let home =
            teams::get(home_team_id).ok_or_else(|| JsValue::from_str("unknown home team id"))?;
        let away =
            teams::get(away_team_id).ok_or_else(|| JsValue::from_str("unknown away team id"))?;
        if home.roster.len() != 5 || away.roster.len() != 5 {
            return Err(JsValue::from_str(
                "session play requires a five-player roster (one keeper, four outfield)",
            ));
        }
        let home_roster: [&str; 5] = home.roster.try_into().expect("checked len == 5");
        let away_roster: [&str; 5] = away.roster.try_into().expect("checked len == 5");
        let tune = Tuning::new();
        let state = sim_match::new(sim_match::NewMatchOptions {
            home,
            away,
            field: PitchSize {
                w: FIELD_W,
                h: FIELD_H,
            },
            home_formation: None,
            tactic: None,
            away_tactic: None,
            duration: Some(duration_seconds),
            max_goals: Some(i64::from(max_goals)),
            seed: Some(seed),
            players_by_id: None,
            species_by_id: None,
            showcase_players_by_id: None,
            // `None` matches the OMP-1 fixture's own construction
            // (`determinism_evidence::new_state`) rather than guessing at an
            // untested combination.
            human_controlled: None,
            input_ownership: Some(ownership(&home_roster, &away_roster)),
        });
        let roster = render_frame::roster(&state);
        let handle = registry::insert(Entry {
            state,
            tune,
            roster,
        });
        Ok(Session { handle })
    }

    fn with_entry<R>(&self, f: impl FnOnce(&mut Entry) -> R) -> R {
        registry::with_entry(self.handle, f)
            .expect("a live Session's registry entry is never missing")
    }

    /// Advance the match by exactly one fixed tick, consuming one canonical
    /// `input_frame` wire (`gc_sim::input_frame::encode`'s format). `wire`'s
    /// tick must equal [`Session::input_tick`].
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if `wire` fails to decode as a
    /// canonical input frame.
    pub fn step(&mut self, wire: &str) -> Result<(), JsValue> {
        let frame = input_frame::decode(wire).map_err(|err| JsValue::from_str(&err.to_string()))?;
        self.with_entry(|entry| {
            sim_match::step(
                &mut entry.state,
                gc_sim::fixed_clock::TICK_SECONDS,
                sim_match::StepInput::Frame(&frame),
                None,
                &entry.tune,
            );
        });
        Ok(())
    }

    /// Home score.
    #[wasm_bindgen(getter, js_name = scoreHome)]
    pub fn score_home(&self) -> f64 {
        self.with_entry(|entry| entry.state.score.home as f64)
    }

    /// Away score.
    #[wasm_bindgen(getter, js_name = scoreAway)]
    pub fn score_away(&self) -> f64 {
        self.with_entry(|entry| entry.state.score.away as f64)
    }

    /// Seconds remaining.
    #[wasm_bindgen(getter, js_name = timeLeft)]
    pub fn time_left(&self) -> f64 {
        self.with_entry(|entry| entry.state.time_left)
    }

    /// Whether the match has finished.
    #[wasm_bindgen(getter)]
    pub fn finished(&self) -> bool {
        self.with_entry(|entry| entry.state.finished)
    }

    /// Next tick [`Session::step`] expects.
    #[wasm_bindgen(getter, js_name = inputTick)]
    pub fn input_tick(&self) -> f64 {
        self.with_entry(|entry| entry.state.input_tick as f64)
    }

    /// The current canonical snapshot hash (`gc_sim::match_snapshot::hash`),
    /// for a JS caller that wants to compare against a peer without
    /// decoding the render block.
    #[wasm_bindgen(js_name = snapshotHash)]
    pub fn snapshot_hash(&self) -> String {
        self.with_entry(|entry| match_snapshot::hash(&match_snapshot::capture(&entry.state, None)))
    }

    /// This match's registry handle, for [`crate::render_export`]'s raw
    /// per-frame exports (`renderFrameBuild(handle)`,
    /// `renderFramePtr()`/`renderFrameLen()`).
    #[wasm_bindgen(getter)]
    pub fn handle(&self) -> u32 {
        self.handle
    }

    /// Match-constant per-player roster fields, as a flat `Float64Array`
    /// (`gc_render::frame_buffer::encode_roster`'s numeric half). Crosses
    /// once per match, not per frame, so it uses the ordinary
    /// `wasm-bindgen` path rather than [`crate::render_export`]'s raw one.
    #[wasm_bindgen(js_name = rosterNumeric)]
    pub fn roster_numeric(&self) -> Vec<f64> {
        self.with_entry(|entry| frame_buffer::encode_roster(&entry.roster).0)
    }

    /// Match-constant roster ids and display names
    /// (`gc_render::frame_buffer::encode_roster`'s string half, a
    /// newline-joined blob per that function's documented format).
    #[wasm_bindgen(js_name = rosterIdsAndNames)]
    pub fn roster_ids_and_names(&self) -> String {
        self.with_entry(|entry| frame_buffer::encode_roster(&entry.roster).1)
    }

    /// This session's current boundary snapshot
    /// (`gc_sim::match_snapshot::capture`), as an opaque handle — for
    /// building a `crate::rollback_events_bridge::RollbackEventsTimeline`
    /// via its `create` (satisfying
    /// `packages/online/src/match_presentation.ts`'s
    /// `newOnlineMatchPresentation`) without a `MatchDriverBridge`. See
    /// [`crate::rollback_events_bridge::WasmMatchSnapshot`]'s doc for why
    /// this crosses as an opaque handle rather than JSON.
    #[wasm_bindgen(js_name = snapshotHandle)]
    pub fn snapshot_handle(&self) -> crate::rollback_events_bridge::WasmMatchSnapshot {
        crate::rollback_events_bridge::WasmMatchSnapshot::new(self.capture_snapshot())
    }
}

impl Session {
    /// Captures this session's current boundary snapshot
    /// (`gc_sim::match_snapshot::capture`), for
    /// `crate::match_driver_bridge`'s online match construction: a
    /// [`gc_netcode::match_driver::MatchDriver`] is built from a boundary-zero
    /// snapshot the same way `sim_match::new` builds one here, and reusing
    /// this crate's own already-tested team/ownership resolution (rather
    /// than re-deriving it from a manifest's content id — see that module's
    /// doc for why online content resolution beyond team roster selection is
    /// out of this wave's scope) is cheaper and no less correct than
    /// building a second path to the same state.
    pub(crate) fn capture_snapshot(&self) -> match_snapshot::MatchSnapshot {
        self.with_entry(|entry| match_snapshot::capture(&entry.state, None))
    }
}

/// Build a [`RenderFrameOptions`] that reuses `entry`'s cached roster
/// instead of rebuilding it every frame. Shared with
/// [`crate::render_export`].
pub(crate) fn frame_options(entry: &Entry) -> RenderFrameOptions {
    RenderFrameOptions {
        roster: Some(entry.roster.clone()),
        ..Default::default()
    }
}

/// A wasm-bound `gc_sim::fixed_clock::FixedClockState`: the render-driven
/// tick-count decision for one match, planned once per render update from a
/// `dt`. See v2/README.md §2.1 -- tick COUNT changes what the simulation
/// computes (it decides how many [`Session::step`] calls run), so the policy
/// that turns a render `dt` into a tick count must be computed here, not
/// re-derived in TypeScript. That is exactly what happened before this type
/// existed: `packages/screens/src/match.ts` hand-copied
/// `gc_sim::fixed_clock::advance`'s accumulator/catch-up/drop algorithm,
/// undetectably drifting from this crate the moment either side's constants
/// changed. `FixedClock` below removes the second implementation instead of
/// pinning it: TypeScript calls [`FixedClock::advance`] and gets back a tick
/// count, nothing more.
///
/// `gc_sim::fixed_clock::advance` is generic over a per-tick input type and
/// step callback so a native, single-process caller can supply both inline;
/// across the wasm boundary that shape does not fit (a JS caller cannot hand
/// wasm a closure to invoke once per simulated tick without a much larger
/// marshalling layer), and it does not need to -- `MatchScreen`
/// (`packages/screens/src/match.ts`) already advances one input sample per
/// [`Session::step`] call itself, outside this clock. So [`FixedClock::advance`]
/// below calls `fixed_clock::advance` with a trivial `()` input and a
/// `step_fn` that always continues, and reports only the resulting tick
/// COUNT -- an upper bound the caller must run via [`Session::step`],
/// breaking out of that loop and calling [`FixedClock::stop_early`] instead
/// of running the rest if it stops before running all of them (e.g. the
/// match finishes mid-batch) -- exactly mirroring what a `false` return from
/// `advance`'s own `step_fn` would have done to the accumulator.
#[wasm_bindgen]
pub struct FixedClock {
    state: fixed_clock::FixedClockState,
}

#[wasm_bindgen]
impl FixedClock {
    /// A fresh clock at tick zero.
    #[wasm_bindgen(constructor)]
    pub fn new() -> FixedClock {
        FixedClock {
            state: fixed_clock::new(),
        }
    }

    /// Ticks this clock has authorized so far -- its own bookkeeping,
    /// independent of (and expected to track) [`Session::input_tick`].
    #[wasm_bindgen(getter)]
    pub fn tick(&self) -> f64 {
        self.state.tick as f64
    }

    /// Accumulate `render_dt` seconds and report how many ticks the caller
    /// should run this render update -- `gc_sim::fixed_clock::advance`'s
    /// exact catch-up/drop policy (`TICK_SECONDS`, `MAX_TICKS_PER_UPDATE`,
    /// its epsilon), minus the `step_fn` early-stop hook (see this struct's
    /// doc). The caller must run at most this many [`Session::step`] calls,
    /// in order; if it runs fewer, it must call [`FixedClock::stop_early`]
    /// afterward.
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if `render_dt` is not finite and
    /// non-negative -- mirrors `advance`'s own assertion.
    pub fn advance(&mut self, render_dt: f64) -> Result<u32, JsValue> {
        if !(render_dt.is_finite() && render_dt >= 0.0) {
            return Err(JsValue::from_str(
                "render dt must be a finite non-negative number",
            ));
        }
        let outcome =
            fixed_clock::advance(&mut self.state, render_dt, |_tick| (), |_tick, ()| true);
        Ok(outcome.ticks)
    }

    /// The caller ran fewer ticks than the last [`FixedClock::advance`] call
    /// authorized (its `step_fn` would have returned `false`) -- zeroes the
    /// carried-over accumulator, exactly as `advance`'s own early-stop
    /// branch does.
    #[wasm_bindgen(js_name = stopEarly)]
    pub fn stop_early(&mut self) {
        self.state.accumulator = 0.0;
    }
}

impl Default for FixedClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod fixed_clock_tests {
    use super::*;

    #[test]
    fn a_fresh_clock_plans_zero_ticks_for_a_sub_tick_dt() {
        let mut clock = FixedClock::new();
        assert_eq!(
            clock.advance(1.0 / 120.0).expect("finite non-negative dt"),
            0
        );
    }

    #[test]
    fn ticks_accumulate_across_calls_exactly_like_gc_sim_fixed_clock() {
        let mut clock = FixedClock::new();
        assert_eq!(clock.advance(1.0 / 120.0).unwrap(), 0);
        assert_eq!(clock.advance(1.0 / 120.0).unwrap(), 1);
        assert_eq!(clock.advance(1.0 / 30.0).unwrap(), 2);
    }

    #[test]
    fn a_long_frame_is_capped_at_max_ticks_per_update() {
        let mut clock = FixedClock::new();
        // Ten seconds of debt in one render update -- far more than
        // MAX_TICKS_PER_UPDATE (8) whole ticks.
        let ticks = clock.advance(10.0).unwrap();
        assert_eq!(ticks, fixed_clock::MAX_TICKS_PER_UPDATE);
    }

    // `advance`'s error path returns a `JsValue`, which
    // `wasm_bindgen::JsValue::from_str` cannot construct off the wasm32
    // target ("function not implemented on non-wasm32 targets", aborting the
    // whole native test process) -- see `coordinator_bridge.rs`'s identical
    // note. `sim_host.spec.ts` (`@gc/app`, which exercises the real compiled
    // artifact) covers `planTicks` rejecting a non-finite/negative `dt`.

    #[test]
    fn stop_early_zeroes_the_accumulator_so_the_next_advance_starts_fresh() {
        let mut clock = FixedClock::new();
        // Bank a fraction of a tick, then discard it via stop_early --
        // mirrors what MatchScreen does when the match finishes mid-batch.
        clock.advance(1.0 / 120.0).unwrap();
        clock.stop_early();
        // A dt just under one tick after a reset plans zero ticks -- if
        // stop_early had not zeroed the accumulator, the banked 1/120s
        // would combine with this call and plan one.
        assert_eq!(clock.advance(1.0 / 120.0).unwrap(), 0);
    }
}
