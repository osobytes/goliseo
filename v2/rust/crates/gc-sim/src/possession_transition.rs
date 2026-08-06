//! Port of `sim/possession_transition.lua`.
//!
//! Serializable possession-transition state and the pure helpers that turn
//! it into per-team phases. Match owns the fixed clock, world eligibility,
//! and the resulting steering; this module owns the compact versioned value
//! contract, the one-turnover-per-flip rule, and the deterministic
//! transition rankings.
//!
//! A transition starts exactly once when authoritative possession moves
//! from one team to the other. The intervening loose-ball interval keeps
//! `last_team`, so a spill neither declares a new possession team nor
//! restarts the window.
//!
//! "Authoritative" means ESTABLISHED, not merely touched. Raw ownership
//! flickers roughly thirty times a minute in a poke-and-scramble, which
//! would refresh a multi-second window forever and erase the ordinary
//! attack/defend phases entirely. A team therefore has to HOLD the ball for
//! [`ESTABLISH_SECONDS`] before it counts: loose frames pause that streak
//! (pass flights bridge) and only the other team's touch resets it.
//! `sim::metrics` measures settled possession the same way; this module is
//! the simulation-side owner of the rule.
//!
//! [`PossessionTransitionState::clear`], [`PossessionTransitionState::advance`],
//! and [`PossessionTransitionState::observe`] are the Lua original's
//! mutate-and-return functions (README rule 7), ported as `&mut self`
//! methods.

use crate::brain;
use gc_data::formations::FormationRole;
use gc_data::tactics::TransitionConfig;

/// This state's format version.
pub const VERSION: u32 = 1;

/// Seconds a team must hold the ball before its possession is
/// authoritative. Deliberately the same threshold `sim::metrics` uses for a
/// settled possession change, so one counted turnover is exactly one
/// transition window (a spec locks the two together). Raw ownership flips
/// are ~30/min here; settled ones are ~3.
pub const ESTABLISH_SECONDS: f64 = 0.7;

/// Counter-press commits two hunters instead of possession defense's single
/// primary presser. Everyone else holds their turnover position.
pub const MAX_PRESSERS: u32 = 2;

/// Which side of the ball a team is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionTeam {
    /// The home side.
    Home,
    /// The away side.
    Away,
}

/// The team's serializable possession-transition state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PossessionTransitionState {
    /// This state's format version.
    pub version: u32,
    /// Last team to ESTABLISH possession.
    pub last_team: Option<TransitionTeam>,
    /// Team currently accruing a possession hold.
    pub holding_team: Option<TransitionTeam>,
    /// Seconds `holding_team` has held, capped at [`ESTABLISH_SECONDS`].
    pub hold: f64,
    /// Team that won the live turnover (`None` = none).
    pub turnover_team: Option<TransitionTeam>,
    /// Seconds since that turnover.
    pub elapsed: f64,
}

/// One candidate for [`select_pressers`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionPresserCandidate {
    /// The candidate's stable player identity.
    pub player_index: u32,
    /// Non-negative cost; lower wins.
    pub distance_cost: f64,
    /// `Some(false)` excludes the candidate; `None`/`Some(true)` include it.
    pub eligible: Option<bool>,
}

fn validate_state(state: &PossessionTransitionState) {
    assert_eq!(
        state.version, VERSION,
        "unsupported possession transition version"
    );
    assert!(
        state.hold.is_finite() && state.hold >= 0.0,
        "possession transition hold must be a non-negative finite number"
    );
    assert!(
        state.holding_team.is_some() || state.hold == 0.0,
        "an unheld possession transition cannot retain hold time"
    );
    assert!(
        state.hold <= ESTABLISH_SECONDS,
        "possession hold must saturate at the establish threshold"
    );
    assert!(
        state.elapsed.is_finite() && state.elapsed >= 0.0,
        "possession transition elapsed must be a non-negative finite number"
    );
    assert!(
        state.last_team.is_none() || state.holding_team.is_some(),
        "an established possession transition must retain its holder"
    );
    if state.turnover_team.is_none() {
        assert_eq!(
            state.elapsed, 0.0,
            "an idle possession transition cannot retain elapsed time"
        );
    } else {
        assert_eq!(
            state.last_team, state.turnover_team,
            "a live possession transition must be owned by the last possession team"
        );
    }
}

/// A fresh, idle possession-transition state.
#[must_use]
pub fn new_state() -> PossessionTransitionState {
    PossessionTransitionState {
        version: VERSION,
        last_team: None,
        holding_team: None,
        hold: 0.0,
        turnover_team: None,
        elapsed: 0.0,
    }
}

/// Validate and defensively copy a possession-transition state.
#[must_use]
pub fn copy_state(state: &PossessionTransitionState) -> PossessionTransitionState {
    validate_state(state);
    *state
}

/// Whether a live transition is in progress.
#[must_use]
pub fn active(state: &PossessionTransitionState) -> bool {
    validate_state(state);
    state.turnover_team.is_some()
}

/// Seconds after which no team can still be inside a window, so the live
/// transition may be dropped. The loser's counter-press and the winner's
/// counter-attack are the only two windows a single turnover opens.
#[must_use]
pub fn window_limit(state: &PossessionTransitionState, windows: TransitionWindows) -> f64 {
    validate_state(state);
    let winner = match state.turnover_team {
        Some(winner) => winner,
        None => return 0.0,
    };
    let loser = match winner {
        TransitionTeam::Home => TransitionTeam::Away,
        TransitionTeam::Away => TransitionTeam::Home,
    };
    windows
        .get(winner)
        .counterattack
        .max(windows.get(loser).counterpress)
}

/// Decay this team's transition memory into a phase. Ordinary attack,
/// defend, and loose phases are unchanged outside a live window.
#[must_use]
pub fn phase(
    state: &PossessionTransitionState,
    team: TransitionTeam,
    owner_team: Option<TransitionTeam>,
    windows: TransitionConfig,
) -> brain::TeamPhase {
    validate_state(state);
    let possession = if owner_team == Some(team) {
        brain::BrainPossession::Team
    } else if owner_team.is_some() {
        brain::BrainPossession::Opponent
    } else {
        brain::BrainPossession::Loose
    };
    let transition = state.turnover_team.map(|winner| {
        if winner == team {
            brain::BrainTransition::Won
        } else {
            brain::BrainTransition::Lost
        }
    });
    brain::phase(&brain::BrainPhaseContext {
        possession,
        transition,
        transition_elapsed: state.elapsed,
        counterpress_window: windows.counterpress,
        counterattack_window: windows.counterattack,
    })
}

/// Validate and defensively copy a tactic's transition windows.
#[must_use]
pub fn copy_windows(source: TransitionConfig) -> TransitionConfig {
    assert!(
        source.counterpress.is_finite() && source.counterpress >= 0.0,
        "counterpress window must be a non-negative finite number"
    );
    assert!(
        source.counterattack.is_finite() && source.counterattack >= 0.0,
        "counterattack window must be a non-negative finite number"
    );
    source
}

/// Per-team possession-transition windows, keyed by team. The Lua original
/// takes a `table<TransitionTeam, TransitionConfig>`; the key set here is
/// exactly the two teams, so a small struct replaces the map (README rule
/// 6) rather than reaching for a banned hash table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionWindows {
    /// The home side's windows.
    pub home: TransitionConfig,
    /// The away side's windows.
    pub away: TransitionConfig,
}

impl TransitionWindows {
    /// This team's windows.
    #[must_use]
    pub fn get(self, team: TransitionTeam) -> TransitionConfig {
        match team {
            TransitionTeam::Home => self.home,
            TransitionTeam::Away => self.away,
        }
    }
}

/// The nearest eligible counter-pressers, capped at `limit`. Distance leads
/// and the player index breaks ties, so mirrored fixtures produce mirrored
/// hunters.
#[must_use]
pub fn select_pressers(candidates: &[TransitionPresserCandidate], limit: u32) -> Vec<u32> {
    let mut eligible: Vec<TransitionPresserCandidate> = Vec::new();
    let mut seen: Vec<u32> = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let label = format!("counter-press candidate {}", index + 1);
        assert!(
            candidate.player_index >= 1,
            "{label} player must be a positive integer"
        );
        assert!(
            !seen.contains(&candidate.player_index),
            "counter-press candidates must be unique"
        );
        seen.push(candidate.player_index);
        assert!(
            candidate.distance_cost.is_finite() && candidate.distance_cost >= 0.0,
            "{label} distance cost must be non-negative"
        );
        if candidate.eligible != Some(false) {
            eligible.push(*candidate);
        }
    }
    eligible.sort_by(|a, b| {
        if a.distance_cost != b.distance_cost {
            a.distance_cost
                .partial_cmp(&b.distance_cost)
                .expect("distance costs must be comparable")
        } else {
            a.player_index.cmp(&b.player_index)
        }
    });
    eligible
        .iter()
        .take(limit as usize)
        .map(|c| c.player_index)
        .collect()
}

/// Role-shaped counter-attack support depth. Every [`FormationRole`] is
/// authored explicitly; the Lua original's fallback for an unknown role
/// string has no analogue once the role is a closed Rust enum.
#[must_use]
pub fn support_push(role: FormationRole) -> f64 {
    match role {
        FormationRole::Fwd => 1.5,
        FormationRole::Wide => 1.3,
        FormationRole::Mid => 1.2,
        FormationRole::Def => 1.0,
    }
}

impl PossessionTransitionState {
    /// Forget every possession memory. Lifecycle boundaries (construction,
    /// kickoff placement, full time) use this so a restart can never
    /// manufacture a turnover out of the possession that preceded it.
    pub fn clear(&mut self) {
        validate_state(self);
        self.last_team = None;
        self.holding_team = None;
        self.hold = 0.0;
        self.turnover_team = None;
        self.elapsed = 0.0;
    }

    /// Advance the live transition by one fixed tick and retire it once both
    /// windows have decayed, so `elapsed` stays bounded and canonical.
    pub fn advance(&mut self, dt: f64, windows: TransitionWindows) {
        validate_state(self);
        assert!(dt.is_finite() && dt >= 0.0, "transition delta must be >= 0");
        if self.turnover_team.is_none() {
            return;
        }
        let limit = window_limit(self, windows);
        let elapsed = self.elapsed + dt;
        if elapsed >= limit {
            self.turnover_team = None;
            self.elapsed = 0.0;
            return;
        }
        self.elapsed = elapsed;
    }

    /// Fold one tick of ownership into the possession memory. A loose ball
    /// (`owner_team == None`) is not a possession change: it pauses the hold
    /// streak and keeps the prior team, so a spill can neither declare a new
    /// possession team nor restart a live window. A turnover opens only when
    /// the ball's new holder has held it for [`ESTABLISH_SECONDS`], so
    /// scramble flicker cannot refresh the window.
    pub fn observe(&mut self, owner_team: Option<TransitionTeam>, dt: f64) {
        validate_state(self);
        assert!(dt.is_finite() && dt >= 0.0, "observed delta must be >= 0");
        let owner_team = match owner_team {
            Some(owner_team) => owner_team,
            None => return,
        };
        if Some(owner_team) != self.holding_team {
            self.holding_team = Some(owner_team);
            self.hold = 0.0;
        }
        // Saturate at the threshold: past it the streak carries no
        // information, and a pinned value keeps the serialized state
        // bounded instead of drifting upward for the rest of the match.
        self.hold = (self.hold + dt).min(ESTABLISH_SECONDS);
        if self.hold < ESTABLISH_SECONDS || Some(owner_team) == self.last_team {
            return;
        }
        if self.last_team.is_some() {
            // Possession has authoritatively changed hands: exactly one
            // transition.
            self.turnover_team = Some(owner_team);
            self.elapsed = 0.0;
        }
        // First established possession after a lifecycle reset (a kickoff, a
        // restart, a fresh match) is remembered, never a turnover.
        self.last_team = Some(owner_team);
    }
}
