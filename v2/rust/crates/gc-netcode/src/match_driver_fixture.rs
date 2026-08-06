//! Port of `game/online/match_driver_fixture.lua`.
//!
//! The Lua original builds three things: a pinned slot-mode boundary zero
//! ([`initial_snapshot`], fully ported below), a frozen `CoordinatorFreeze`
//! (`fixture.freeze`), and a connected in-process star (`fixture.session`).
//!
//! The latter two are blocked in this crate today. `fixture.peer_ids` and
//! `fixture.freeze` need `protocol.match_mode`, `coordinator.plan_assignments`,
//! `coordinator.slot_sources`, and `protocol.owned_slots`/`protocol.assignment_id`
//! — `game/online/coordinator.lua` and `game/online/protocol.lua` are
//! `NOT YET PORTED` placeholders in this crate as of this port, owned by
//! concurrent agents (see `crate::coordinator`, `crate::protocol`).
//! `fixture.session` additionally needs `game/transport/fake_star.lua`, which
//! is permanently TypeScript-owned (`v2/README.md` §2) and has no Rust type
//! at all.
//!
//! So only [`guest_peer_id`] and [`initial_snapshot`] are ported — everything
//! [`crate::match_driver::MatchDriver::new`] (via [`crate::match_driver::new`])
//! actually needs *except* the frozen session state and transport, which a
//! caller must still supply directly (or, until the modules above land, via a
//! fake — see `crate::match_driver`'s module doc on [`crate::match_driver::MatchDriverRules`]
//! for the same pattern applied to the driver's own dependencies).

use gc_data::teams;
use gc_sim::combat;
use gc_sim::combat_snapshot::CombatMatchState;
use gc_sim::r#match::{self, NewMatchOptions};
use gc_sim::match_snapshot::{self, MatchSnapshot, PitchSize};

/// Mirrors `fixture.HOST_PEER_ID` (`transport_contract.HOST_PEER_ID` —
/// duplicated the same way `crate::match_driver` duplicates it).
pub const HOST_PEER_ID: &str = "host";
/// Mirrors `fixture.COUNTDOWN_ID`.
pub const COUNTDOWN_ID: &str = "countdown.1";
/// Mirrors `fixture.DEFAULT_DURATION`.
pub const DEFAULT_DURATION: f64 = 6.0;
/// Mirrors `fixture.DEFAULT_SEED`.
pub const DEFAULT_SEED: f64 = 74.0;

/// Mirrors `fixture.guest_peer_id`.
#[must_use]
pub fn guest_peer_id(index: i64) -> String {
    format!("guest_{index}")
}

/// Boundary zero for the pinned combat fixture: two authored fixture teams,
/// slot mode, and (when `combat_active`) the combat snapshot schema. Mirrors
/// `fixture.initial_snapshot`.
///
/// `seed` overrides the pinned match seed — every peer shares one boundary
/// zero in a real session, so a differing seed is not a configuration; it is
/// the cheapest honest way to give one peer a genuinely divergent simulation
/// while every input row still agrees, which is what a desync looks like
/// from the driver's side.
///
/// # Panics
///
/// Panics if the `nebula`/`orion` fixture teams are missing from
/// [`gc_data::teams`] — a content-table invariant, never expected to fail.
#[must_use]
pub fn initial_snapshot(
    duration: Option<f64>,
    combat_active: bool,
    seed: Option<f64>,
) -> MatchSnapshot {
    let home = teams::get("nebula").expect("fixture team nebula is always authored");
    let away = teams::get("orion").expect("fixture team orion is always authored");
    let ownership = r#match::ownership_for_teams(home, away, None);
    let mut state = r#match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(duration.unwrap_or(DEFAULT_DURATION)),
        max_goals: Some(99),
        seed: Some(seed.unwrap_or(DEFAULT_SEED)),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership),
    });
    if !combat_active {
        return match_snapshot::capture(&state, None);
    }
    let combat: CombatMatchState = combat::new_state(&mut state, None);
    match_snapshot::capture(&state, Some(&combat))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_peer_id_matches_the_lua_naming() {
        assert_eq!(guest_peer_id(1), "guest_1");
        assert_eq!(guest_peer_id(7), "guest_7");
    }

    #[test]
    fn initial_snapshot_is_slot_mode_boundary_zero() {
        let snapshot = initial_snapshot(None, false, None);
        assert_eq!(snapshot.state.input_tick, 0);
        assert!(snapshot.state.slot_mode);
        assert!(!snapshot.state.finished);
    }

    #[test]
    fn initial_snapshot_with_combat_active_carries_the_combat_companion() {
        let with_combat = initial_snapshot(None, true, None);
        assert!(with_combat.combat.is_some());
        let without_combat = initial_snapshot(None, false, None);
        assert!(without_combat.combat.is_none());
    }

    #[test]
    fn a_differing_seed_produces_a_differently_hashed_boundary_zero() {
        let a = initial_snapshot(None, false, Some(1.0));
        let b = initial_snapshot(None, false, Some(2.0));
        assert_ne!(match_snapshot::hash(&a), match_snapshot::hash(&b));
    }
}
