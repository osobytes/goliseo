//! Converts a formation's normalized anchors into absolute pitch positions.
//! Home attacks right (anchors used as-is); away attacks left (x mirrored).

use gc_core::vec2::Vec2;
use gc_data::formations::FormationData;

/// Which side of the fixture a formation is being placed for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Attacks right; anchors are used as authored.
    Home,
    /// Attacks left; anchors are mirrored across the pitch's vertical centre.
    Away,
}

/// The playable pitch dimensions anchors are placed within.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    /// Pitch width, in pixels.
    pub w: f64,
    /// Pitch height, in pixels.
    pub h: f64,
}

/// A candidate used to break ties by index when sorting by distance.
///
/// `idx` is a plain in-memory position (not a wire format), so it converts to
/// 0-based per ARCHITECTURE.md §3 rule 3.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DistanceCandidate {
    /// The candidate's original index.
    pub idx: usize,
    /// The candidate's distance.
    pub d: f64,
}

fn place(side: Side, field: Field, nx: f64, ny: f64) -> Vec2 {
    let x = if side == Side::Home { nx } else { 1.0 - nx };
    Vec2::new(x * field.w, ny * field.h)
}

/// Convert a formation's normalized anchors into absolute pitch positions.
///
/// Returns the keeper first, then the outfield anchors in formation order.
#[must_use]
pub fn anchors(formation: &FormationData, side: Side, field: Field) -> Vec<Vec2> {
    let mut out = Vec::with_capacity(1 + formation.outfield.len());
    out.push(place(side, field, formation.keeper.x, formation.keeper.y));
    for a in formation.outfield {
        out.push(place(side, field, a.x, a.y));
    }
    out
}

/// Ordering used to sort distance candidates: nearest first, ties broken by
/// the higher original index first.
#[must_use]
pub fn distance_candidate_before(a: DistanceCandidate, b: DistanceCandidate) -> bool {
    if a.d != b.d {
        return a.d < b.d;
    }
    a.idx > b.idx
}
