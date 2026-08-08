//! Thread-local slab mapping an opaque `u32` handle to a live match session.
//!
//! Exists so [`crate::render_export`]'s raw, `wasm-bindgen`-free exports can
//! reach the same [`gc_sim::match_snapshot::MatchState`]
//! [`crate::session::Session`] (a `wasm-bindgen` class) is driving, without
//! passing a Rust reference across the FFI boundary. A raw `extern "C"`
//! export can only take FFI-safe scalars — a `u32` handle is the plain
//! scalar that stands in for "which session".
//!
//! wasm32-unknown-unknown is single-threaded, so `thread_local!` here is
//! exactly one slab per module instance, no synchronization needed.

use std::cell::RefCell;

use gc_render::frame::RenderFrameRoster;
use gc_sim::combat_snapshot::CombatMatchState;
use gc_sim::match_snapshot::{MatchSnapshot, MatchState};
use gc_sim::tuning::Tuning;

/// One live match's owned state, keyed by its registry handle.
pub struct Entry {
    /// The match's current simulation state. Since `crate::session::Session`
    /// changed to build an ORDINARY match the same way
    /// `game/screens/match.lua`'s `Match:restart` does (see that module's
    /// doc), this runs in LEGACY mode (`state.slot_mode == false`, built
    /// with `input_ownership: None`) — not slot mode, unlike before.
    pub state: MatchState,
    /// The tuning this session was constructed and must be stepped with.
    pub tune: Tuning,
    /// Match-constant roster identity, built once and reused every frame
    /// (see `gc_render::frame::RenderFrameOptions::roster`'s doc).
    pub roster: RenderFrameRoster,
    /// This session's combat companion, present only when constructed with
    /// `combat_enabled = true` (`crate::session::Session::new`). Mirrors
    /// `game/screens/match.lua`'s own `self._combat_state`: built once, at
    /// construction, via `gc_sim::combat::new_state` (`combat_sim.new_state`
    /// in the Lua), then threaded through every subsequent step unchanged
    /// in shape — `None` when the option is off, exactly like an ordinary
    /// (non-combat) Lua match never builds one either.
    pub combat: Option<CombatMatchState>,
    /// `Session`'s own local input-tick counter, incremented once per
    /// `crate::session::Session::step` call.
    ///
    /// `state.input_tick` is *not* used for this: `sim::match::step` only
    /// ever advances `input_tick` inside its slot-mode branch (see
    /// `gc_sim::r#match::step`'s doc), and `state` now runs in legacy mode,
    /// so `state.input_tick` stays `0` for the session's whole life.
    /// `packages/app/src/sim_host.ts` (and `browser_sim_host.ts`) read
    /// `Session::input_tick` once per `step` call — to build the next wire's
    /// tick field, to key its per-tick frame cache, and to report match
    /// progress — and all three assume it advances by exactly one per
    /// `step`, a contract predating this change (see
    /// `packages/wasm/src/session.spec.ts`'s
    /// `"stepping advances inputTick"` case). This field is what keeps that
    /// contract true without touching `state.input_tick` at all.
    pub tick: i64,
    /// This session's slot-mode boundary-zero snapshot, for
    /// `crate::session::Session::capture_snapshot`/`snapshot_handle` —
    /// online/rollback callers (`MatchDriverBridge`,
    /// `RollbackPlayableLab`, the coordinator bridge) need a snapshot whose
    /// `state.slot_mode` is `true`: `gc_sim::rollback_session::new` asserts
    /// exactly that. Built once, at construction, from the SAME match
    /// parameters as `state` (same teams/seed/duration/tactics/formation)
    /// but with `input_ownership: Some(...)` instead of `None` — see
    /// `crate::session::Session::new`'s doc for why this is a second,
    /// independent construction rather than a re-derivation of `state`.
    /// Every documented caller of `capture_snapshot`/`snapshot_handle`
    /// already takes it "before any `step`" (boundary zero), so a snapshot
    /// fixed at construction time, rather than recomputed from `state` on
    /// every call, is not a staleness risk — it was never meant to reflect
    /// `state`'s post-step position anyway.
    pub rollback_boundary: MatchSnapshot,
}

thread_local! {
    static SLAB: RefCell<Vec<Option<Entry>>> = const { RefCell::new(Vec::new()) };
}

/// Register `entry`, returning its handle. Reuses the first freed slot
/// before growing the slab.
pub fn insert(entry: Entry) -> u32 {
    SLAB.with(|slab| {
        let mut slab = slab.borrow_mut();
        for (index, slot) in slab.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(entry);
                return index as u32;
            }
        }
        slab.push(Some(entry));
        (slab.len() - 1) as u32
    })
}

/// Release `handle`'s slot. A no-op if `handle` is already free or
/// out of range.
pub fn free(handle: u32) {
    SLAB.with(|slab| {
        if let Some(slot) = slab.borrow_mut().get_mut(handle as usize) {
            *slot = None;
        }
    });
}

/// Run `f` against `handle`'s entry, if it names a live session.
pub fn with_entry<R>(handle: u32, f: impl FnOnce(&mut Entry) -> R) -> Option<R> {
    SLAB.with(|slab| {
        slab.borrow_mut()
            .get_mut(handle as usize)
            .and_then(|slot| slot.as_mut())
            .map(f)
    })
}
