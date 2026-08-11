//! `game::online::coordinator::next_live_slot` is the transition *rule*: it
//! consumes an already-total `ranked` ordering and intersects it with a
//! frozen owned set. This module is the other half — the nearest-to-ball
//! ranking and the carrier / winner derivation that *produce* that ordering.
//!
//! Two properties are load bearing, because a disagreement here moves the
//! live slot on one peer and not another:
//!
//!  1. **The ranking is total.** Slots are compared by squared distance to
//!     the ball and then by canonical slot index. Canonical indexes are
//!     distinct integers, so no two entries ever compare equal in both keys.
//!     A strict total order makes the sort's result unique regardless of
//!     pivot choice or insertion order, so exact distance ties resolve
//!     identically on every peer.
//!  2. **No hash-order dependence.** Every table walked here is walked with
//!     a numeric `1..=SLOT_COUNT` loop, never a hash-ordered structure. Two
//!     peers must agree bit for bit; nothing here may depend on iteration
//!     order that is not itself canonical.
//!
//! Squared distance is used deliberately instead of a `length()` call: it is
//! the same monotone ordering with one fewer rounded operation, so the
//! comparison never depends on how a square root rounds.
//!
//! This module is pure. It reads a [`MatchState`] — `ball`, `players[i].pos`,
//! `slot_players`, and `slot_for_player`.

use gc_sim::input_frame::{self, EdgeAction, InputSample, SlotId};
use gc_sim::match_snapshot::MatchState;

/// One canonical slot's rank key: its identity, its canonical index (the
/// total-order tiebreak), and its squared distance to the ball.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RankEntry {
    /// Slot identity.
    pub slot: SlotId,
    /// Canonical slot index, the total-order tiebreak.
    pub index: i64,
    /// Squared distance from this slot's player to the ball.
    pub distance_sq: f64,
}

/// Inputs to [`transition`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionOptions {
    /// The canonical `switch` edge of the live slot's row.
    pub switch: bool,
    /// Excluded from `ranked`, matching offline switching.
    pub live: Option<SlotId>,
    /// Carrier at the preceding boundary.
    pub previous_carrier: Option<SlotId>,
}

/// The complete transition input for
/// `game::online::coordinator::next_live_slot`, derived only from the
/// deterministic simulation and the canonical `switch` edge. Nothing local —
/// presentation, frame timing, or when a key was physically pressed — reaches
/// this struct.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveTransition {
    /// The canonical `switch` edge bit for this tick.
    pub switch: bool,
    /// The slot holding the ball, if any.
    pub carrier: Option<SlotId>,
    /// The slot that won the ball on this tick, if any.
    pub winner: Option<SlotId>,
    /// Outfield slots ordered by deterministic distance to the ball.
    pub ranked: Vec<SlotId>,
}

/// Canonical slot identity for a 1-based canonical slot index.
///
/// # Panics
///
/// Panics if `index` is outside `1..=input_frame::SLOT_COUNT`.
pub fn slot_id(index: i64) -> SlotId {
    input_frame::slot(index)
        .unwrap_or_else(|_| panic!("canonical slot index is out of range"))
        .id
}

/// The 1-based canonical slot index for a slot identity. Total: every
/// [`SlotId`] variant is a canonical slot, so this cannot fail.
pub fn slot_index(slot: SlotId) -> i64 {
    match slot {
        SlotId::Home1 => 1,
        SlotId::Home2 => 2,
        SlotId::Home3 => 3,
        SlotId::Home4 => 4,
        SlotId::Away1 => 5,
        SlotId::Away2 => 6,
        SlotId::Away3 => 7,
        SlotId::Away4 => 8,
    }
}

fn entry_cmp(left: &RankEntry, right: &RankEntry) -> std::cmp::Ordering {
    if left.distance_sq != right.distance_sq {
        // Ascending squared distance. Canonical indexes are distinct, so the
        // `else` branch always decides ties, matching the offline "scan slots
        // in order, keep the strict best" rule in `sim::match`.
        if left.distance_sq < right.distance_sq {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    } else {
        left.index.cmp(&right.index)
    }
}

/// Every canonical outfield slot's rank key, in canonical slot order (sorted
/// nearest-to-ball first by [`entry_cmp`]).
///
/// # Panics
///
/// Panics if `state` is not a slot-mode match, if a canonical slot is
/// unmapped, or if its player is missing — all programmer errors under this
/// crate's own invariants.
pub fn entries(state: &MatchState) -> Vec<RankEntry> {
    assert!(
        state.slot_mode,
        "live-slot ranking requires a slot-mode match"
    );
    let ball = state.ball;
    let mut entries = Vec::with_capacity(input_frame::SLOT_COUNT as usize);
    for index in 1..=input_frame::SLOT_COUNT {
        let player_index =
            state.slot_players[(index - 1) as usize].expect("canonical slot is unmapped");
        let player = state
            .players
            .get((player_index - 1) as usize)
            .expect("canonical slot player is missing");
        let dx = player.pos.x - ball.x;
        let dy = player.pos.y - ball.y;
        let distance_sq = dx * dx + dy * dy;
        assert!(!distance_sq.is_nan(), "live-slot distance must be a number");
        entries.push(RankEntry {
            slot: slot_id(index),
            index,
            distance_sq,
        });
    }
    entries.sort_by(entry_cmp);
    entries
}

/// Every canonical outfield slot ordered by distance to the ball, nearest
/// first, with exact ties broken by ascending canonical slot index.
///
/// `exclude` is usually the currently live slot.
pub fn rank(state: &MatchState, exclude: Option<SlotId>) -> Vec<SlotId> {
    entries(state)
        .into_iter()
        .filter(|entry| Some(entry.slot) != exclude)
        .map(|entry| entry.slot)
        .collect()
}

/// The slot holding the ball, or `None` while the ball is loose or with a
/// keeper. Keepers are slotless in every mode, so `slot_for_player` simply
/// misses them.
///
/// # Panics
///
/// Panics if `state` is not a slot-mode match.
pub fn carrier(state: &MatchState) -> Option<SlotId> {
    assert!(
        state.slot_mode,
        "live-slot carrier requires a slot-mode match"
    );
    let owner = state.owner?;
    let index = state.slot_for_player[(owner - 1) as usize]?;
    Some(slot_id(index))
}

/// Build the complete transition input for
/// `game::online::coordinator::next_live_slot`.
///
/// `state` is the geometry boundary every peer agrees on.
pub fn transition(state: &MatchState, options: &TransitionOptions) -> LiveTransition {
    let carrier = carrier(state);
    let winner = match carrier {
        Some(slot) if Some(slot) != options.previous_carrier => Some(slot),
        _ => None,
    };
    LiveTransition {
        switch: options.switch,
        carrier,
        winner,
        ranked: rank(state, options.live),
    }
}

/// Whether `sample` carries the `switch` edge for this tick.
///
/// # Panics
///
/// Panics if `sample` is not a valid [`InputSample`].
pub fn switch_edge(sample: &InputSample) -> bool {
    input_frame::has_edge(sample, EdgeAction::Switch).unwrap_or_else(|err| {
        panic!("live-slot switch edge requires a valid sample: {err}");
    })
}
