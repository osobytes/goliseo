//! Port of `sim/offball_runs.lua`.
//!
//! Pure role-gated off-ball run geometry and bounded team arbitration. Match
//! owns authoritative player clocks/state; this module derives candidates
//! from one world boundary and delegates deterministic ranking to
//! [`crate::brain`].

use crate::ai;
use crate::brain;
use crate::outfield_decision;
use gc_core::vec2::Vec2;
use gc_data::formations::{self, FormationRole};

/// Which side a team plays, and therefore which way it attacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Team {
    /// Attacks toward +x.
    Home,
    /// Attacks toward -x.
    Away,
}

/// Playable pitch dimensions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    /// Pitch width.
    pub w: f64,
    /// Pitch height.
    pub h: f64,
}

/// One of this team's own outfield players, as offered to [`grant`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OffballRunPlayer {
    /// The player's stable identity.
    pub player_index: u32,
    /// The player's formation role.
    pub role: FormationRole,
    /// How willing this player is to make a run, in `[0, 1]`.
    pub run_drive: f64,
    /// The player's current position.
    pub pos: Vec2,
    /// Normalized authored formation width (0 = top touchline).
    pub anchor_y: f64,
}

/// One of this team's own players, for occupancy checks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OffballRunTeammate {
    /// The teammate's stable identity.
    pub player_index: u32,
    /// The teammate's current position.
    pub pos: Vec2,
}

/// One opposing player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OffballRunOpponent {
    /// The opponent's current position.
    pub pos: Vec2,
    /// Whether this opponent is the goalkeeper.
    pub is_keeper: bool,
}

/// Inputs to [`grant`].
#[derive(Clone, Debug, PartialEq)]
pub struct OffballRunContext {
    /// Which side this team plays.
    pub team: Team,
    /// Playable pitch dimensions.
    pub field: Field,
    /// The ball carrier's current position.
    pub carrier_pos: Vec2,
    /// Whether the carrier has settled on the ball (vs. an unsettled first
    /// touch).
    pub carrier_settled: bool,
    /// Distance from the carrier to the nearest pressure.
    pub carrier_pressure: f64,
    /// The pressure distance threshold used to judge `carrier_pressure`.
    pub pressure_distance: f64,
    /// Whether the team is inside a counter-attack window
    /// ([`crate::possession_transition`]). Absent and explicitly false are
    /// equivalent, matching the Lua original's `boolean?` used only in a
    /// truthy test.
    pub counterattack: bool,
    /// Every outfield player eligible to offer a run.
    pub players: Vec<OffballRunPlayer>,
    /// Every teammate, for occupancy checks.
    pub teammates: Vec<OffballRunTeammate>,
    /// Every opponent, including the keeper.
    pub opponents: Vec<OffballRunOpponent>,
}

/// How long a granted run stays active, in seconds. Mirrors
/// [`outfield_decision::RUN_LIFETIME_SECONDS`].
pub const RUN_LIFETIME_SECONDS: f64 = outfield_decision::RUN_LIFETIME_SECONDS;
/// How long a freshly granted run telegraphs before it becomes a live threat.
pub const TELEGRAPH_SECONDS: f64 = 0.2;
/// Maximum simultaneously active runs per team.
pub const MAX_ACTIVE_PER_TEAM: u32 = 2;
/// The minimum `run_drive` a forward needs to offer an in-behind run.
pub const RUN_DRIVE_THRESHOLD: f64 = 0.55;
/// The minimum net progress a run must make to be worth granting.
pub const MIN_RUN_PROGRESS: f64 = 24.0;
/// The minimum distance a support run must keep from the carrier.
pub const MIN_SUPPORT_DISTANCE: f64 = 80.0;
/// The maximum distance a support run may sit from the carrier.
pub const MAX_SUPPORT_DISTANCE: f64 = 250.0;

const FIELD_MARGIN: f64 = 24.0;
const IN_BEHIND_GAP: f64 = 56.0;
const IN_BEHIND_LANE_WIDTH: f64 = 24.0;
const COME_SHORT_DISTANCE: f64 = 115.0;
const COME_SHORT_MARKER_DISTANCE: f64 = 100.0;
const COME_SHORT_MARKER_OFFSET: f64 = 24.0;
const HOLD_WIDTH_DEPTH: f64 = 70.0;
const HOLD_WIDTH_OCCUPIED_X: f64 = 130.0;
const HOLD_WIDTH_OCCUPIED_Y: f64 = 72.0;

fn clamp(value: f64, low: f64, high: f64) -> f64 {
    value.min(high).max(low)
}

fn attack_direction(context: &OffballRunContext) -> f64 {
    if context.team == Team::Home {
        1.0
    } else {
        -1.0
    }
}

fn clamp_target(context: &OffballRunContext, point: Vec2) -> Vec2 {
    Vec2::new(
        clamp(point.x, FIELD_MARGIN, context.field.w - FIELD_MARGIN),
        clamp(point.y, FIELD_MARGIN, context.field.h - FIELD_MARGIN),
    )
}

fn bound_support_distance(
    context: &OffballRunContext,
    point: Vec2,
    minimum: f64,
    maximum: f64,
) -> Option<Vec2> {
    let offset = point.sub(context.carrier_pos);
    let distance = offset.length();
    let direction = if distance > 0.0 {
        offset.scale(1.0 / distance)
    } else {
        Vec2::new(attack_direction(context), 0.0)
    };
    let target = clamp_target(
        context,
        context
            .carrier_pos
            .add(direction.scale(clamp(distance, minimum, maximum))),
    );
    let final_distance = context.carrier_pos.dist(target);
    if final_distance < minimum || final_distance > maximum {
        return None;
    }
    Some(target)
}

fn nonkeeper_opponent_positions(context: &OffballRunContext) -> Vec<Vec2> {
    context
        .opponents
        .iter()
        .filter(|o| !o.is_keeper)
        .map(|o| o.pos)
        .collect()
}

// A counter-attack window buys ONE immediate in-behind request from an
// unsettled carrier (`grant` keeps only the best). The role gate, lane
// rules, progress floor, team cap, and slot expiry are untouched.
fn in_behind_target(
    context: &OffballRunContext,
    player: &OffballRunPlayer,
) -> Option<brain::BrainRunTarget> {
    if player.role != FormationRole::Fwd
        || player.run_drive < RUN_DRIVE_THRESHOLD
        || !(context.carrier_settled || context.counterattack)
    {
        return None;
    }

    let attack = attack_direction(context);
    let mut last_defender: Option<f64> = None;
    for opponent in &context.opponents {
        if !opponent.is_keeper
            && (last_defender.is_none() || (opponent.pos.x - last_defender.unwrap()) * attack > 0.0)
        {
            last_defender = Some(opponent.pos.x);
        }
    }
    let last_defender = last_defender?;

    let target = clamp_target(
        context,
        Vec2::new(last_defender + attack * IN_BEHIND_GAP, player.pos.y),
    );
    if (target.x - last_defender) * attack <= 0.0
        || (target.x - context.field.w / 2.0) * attack <= 0.0
        || (target.x - player.pos.x) * attack < MIN_RUN_PROGRESS
        || ai::lane_blocker(
            context.carrier_pos,
            target,
            &nonkeeper_opponent_positions(context),
            IN_BEHIND_LANE_WIDTH,
        )
        .is_some()
    {
        return None;
    }

    let progress = ((target.x - context.carrier_pos.x) * attack).max(0.0);
    Some(brain::BrainRunTarget {
        score: 100.0 + player.run_drive * 20.0 + progress / context.field.w * 10.0,
        x: target.x,
        y: target.y,
        duration: RUN_LIFETIME_SECONDS,
    })
}

fn come_short_target(
    context: &OffballRunContext,
    player: &OffballRunPlayer,
) -> Option<brain::BrainRunTarget> {
    if (player.role != FormationRole::Mid && player.role != FormationRole::Wide)
        || !context.carrier_settled
        || context.carrier_pressure > context.pressure_distance
    {
        return None;
    }

    let to_runner = player.pos.sub(context.carrier_pos);
    let runner_distance = to_runner.length();
    let direction = if runner_distance > 0.0 {
        to_runner.normalized()
    } else {
        Vec2::new(attack_direction(context), 0.0)
    };
    let mut target = context
        .carrier_pos
        .add(direction.scale(COME_SHORT_DISTANCE));
    let mut marker: Option<Vec2> = None;
    let mut marker_distance: Option<f64> = None;
    for opponent in &context.opponents {
        if !opponent.is_keeper {
            let distance = opponent.pos.dist(target);
            if marker_distance.is_none() || distance < marker_distance.unwrap() {
                marker = Some(opponent.pos);
                marker_distance = Some(distance);
            }
        }
    }
    if let (Some(marker), Some(marker_distance)) = (marker, marker_distance)
        && marker_distance < COME_SHORT_MARKER_DISTANCE
    {
        let goal_side_x = marker.x + attack_direction(context) * COME_SHORT_MARKER_OFFSET;
        if (goal_side_x - target.x) * attack_direction(context) > 0.0 {
            target = Vec2::new(goal_side_x, target.y);
        }
    }
    let bounded_target = bound_support_distance(
        context,
        target,
        MIN_SUPPORT_DISTANCE,
        COME_SHORT_DISTANCE + COME_SHORT_MARKER_OFFSET,
    )?;
    target = bounded_target;
    let target_distance = context.carrier_pos.dist(target);
    if runner_distance - target_distance < MIN_RUN_PROGRESS {
        return None;
    }
    let pressure_factor = 1.0
        - clamp(
            context.carrier_pressure / context.pressure_distance.max(1.0),
            0.0,
            1.0,
        );
    Some(brain::BrainRunTarget {
        score: 78.0 + pressure_factor * 14.0 + player.run_drive * 6.0,
        x: target.x,
        y: target.y,
        duration: RUN_LIFETIME_SECONDS,
    })
}

fn hold_width_target(
    context: &OffballRunContext,
    player: &OffballRunPlayer,
) -> Option<brain::BrainRunTarget> {
    if player.role != FormationRole::Wide || !context.carrier_settled {
        return None;
    }
    let upper_lane = player.anchor_y < 0.5;
    let lane = Vec2::new(
        context.carrier_pos.x + attack_direction(context) * HOLD_WIDTH_DEPTH,
        if upper_lane {
            context.field.h / 6.0
        } else {
            context.field.h * 5.0 / 6.0
        },
    );
    let lane = clamp_target(context, lane);
    if (context.carrier_pos.x - lane.x).abs() < HOLD_WIDTH_OCCUPIED_X
        && (context.carrier_pos.y - lane.y).abs() < HOLD_WIDTH_OCCUPIED_Y
    {
        return None;
    }
    for teammate in &context.teammates {
        if teammate.player_index != player.player_index
            && (teammate.pos.x - lane.x).abs() < HOLD_WIDTH_OCCUPIED_X
            && (teammate.pos.y - lane.y).abs() < HOLD_WIDTH_OCCUPIED_Y
        {
            return None;
        }
    }
    let target = bound_support_distance(context, lane, MIN_SUPPORT_DISTANCE, MAX_SUPPORT_DISTANCE)?;
    let remains_outer = if upper_lane {
        target.y <= context.field.h / 3.0
    } else {
        target.y >= context.field.h * 2.0 / 3.0
    };
    if !remains_outer {
        return None;
    }
    let width_gain = (target.y - context.field.h / 2.0).abs() / (context.field.h / 2.0);
    Some(brain::BrainRunTarget {
        score: 64.0 + width_gain * 12.0 + player.run_drive * 5.0,
        x: target.x,
        y: target.y,
        duration: RUN_LIFETIME_SECONDS,
    })
}

/// The authored role for a formation's outfield ordinal.
///
/// `outfield_index` is 0-based, indexing directly into
/// [`gc_data::formations::FormationData::outfield`] (README rule 5.3) —
/// unlike this module's `player_index` fields, this really is a Rust
/// collection index, not an opaque wire identity.
///
/// # Panics
///
/// Panics if `formation_id` is unknown or `outfield_index` is out of range.
#[must_use]
pub fn formation_role(formation_id: &str, outfield_index: usize) -> FormationRole {
    let formation = formations::get(formation_id)
        .unwrap_or_else(|| panic!("unknown formation: {formation_id}"));
    let anchor = formation
        .outfield
        .get(outfield_index)
        .unwrap_or_else(|| panic!("formation outfield index is out of range"));
    anchor.role
}

/// The authored normalized width for a formation's outfield ordinal. See
/// [`formation_role`] for the indexing convention.
///
/// # Panics
///
/// Panics if `formation_id` is unknown or `outfield_index` is out of range.
#[must_use]
pub fn formation_anchor_y(formation_id: &str, outfield_index: usize) -> f64 {
    let formation = formations::get(formation_id)
        .unwrap_or_else(|| panic!("unknown formation: {formation_id}"));
    let anchor = formation
        .outfield
        .get(outfield_index)
        .unwrap_or_else(|| panic!("formation outfield index is out of range"));
    anchor.y
}

/// Reconcile active run slots against this tick's role-gated offers.
#[must_use]
pub fn grant(
    context: &OffballRunContext,
    active: &[brain::RunSlot],
    now: f64,
) -> Vec<brain::RunSlot> {
    let mut players: Vec<brain::BrainRunPlayer> = context
        .players
        .iter()
        .map(|player| brain::BrainRunPlayer {
            player_index: player.player_index,
            eligible: None,
            in_behind: in_behind_target(context, player),
            come_short: come_short_target(context, player),
            hold_width: hold_width_target(context, player),
        })
        .collect();

    if context.counterattack && !context.carrier_settled {
        // Exactly one free in-behind request: the best-scoring option wins
        // and the player index breaks ties, so mirrored fixtures mirror.
        let mut best_index: Option<usize> = None;
        for (index, player) in players.iter().enumerate() {
            if let Some(target) = &player.in_behind {
                let better = match best_index {
                    None => true,
                    Some(current_best) => {
                        target.score > players[current_best].in_behind.as_ref().unwrap().score
                    }
                };
                if better {
                    best_index = Some(index);
                }
            }
        }
        for (index, player) in players.iter_mut().enumerate() {
            if Some(index) != best_index {
                player.in_behind = None;
            }
        }
    }

    let candidates = brain::run_candidates(&brain::BrainRunContext { players });
    brain::grant_runs(&candidates, active, MAX_ACTIVE_PER_TEAM, now)
}

/// Whether a run granted at `expires_at - RUN_LIFETIME_SECONDS` is still in
/// its fixed telegraph window at `now`. Derived purely from the expiry, so
/// no separate "granted at" state needs to be threaded through.
#[must_use]
pub fn telegraphing(now: f64, expires_at: f64) -> bool {
    let granted_at = expires_at - RUN_LIFETIME_SECONDS;
    let mut elapsed = now - granted_at;
    let epsilon = 1e-12;
    if elapsed.abs() <= epsilon {
        elapsed = 0.0;
    } else if (elapsed - TELEGRAPH_SECONDS).abs() <= epsilon {
        elapsed = TELEGRAPH_SECONDS;
    }
    (0.0..TELEGRAPH_SECONDS).contains(&elapsed) && now < expires_at
}
