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
use gc_sim::match_snapshot::MatchState;
use gc_sim::slot_input::SlotInputProducerState;
use gc_sim::tuning::Tuning;

/// One live match's owned state, keyed by its registry handle.
pub struct Entry {
    /// The match's current simulation state.
    pub state: MatchState,
    /// The tuning this session was constructed and must be stepped with.
    pub tune: Tuning,
    /// Match-constant roster identity, built once and reused every frame
    /// (see `gc_render::frame::RenderFrameOptions::roster`'s doc).
    pub roster: RenderFrameRoster,
    /// This session's fixed-slot input producer (`gc_sim::slot_input`) —
    /// retains the declared bot fills' RNG/combat-intent state across
    /// ticks so `crate::session::Session::step` can materialize a complete
    /// effective `InputFrame` from the single-slot wire it receives. See
    /// `crate::session`'s module doc for why every session runs in slot
    /// mode and therefore always needs one.
    pub producer: SlotInputProducerState,
    /// This session's combat companion, present only when constructed with
    /// `combat_enabled = true` (`crate::session::Session::new`). Mirrors
    /// `game/screens/match.lua`'s own `self._combat_state`: built once, at
    /// construction, via `gc_sim::combat::new_state` (`combat_sim.new_state`
    /// in the Lua), then threaded through every subsequent step unchanged
    /// in shape — `None` when the option is off, exactly like an ordinary
    /// (non-combat) Lua match never builds one either.
    pub combat: Option<CombatMatchState>,
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
