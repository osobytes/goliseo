//! Fun-proxy metrics: fold a running match into the per-match numbers that
//! `docs/design/fun_metrics.md` bands and scores. Pure observer — reads
//! match state after each step, never mutates it.
//!
//! - This module never depends on [`crate::r#match`] directly, and
//!   `tests/metrics.rs` never builds a real `MatchState` either — it
//!   hand-builds a minimal MatchState-shaped value: enough surface for the
//!   collector. [`MetricsMatchView`] and [`MetricsPlayerView`] are that same
//!   minimal surface, typed. `crate::headless::to_metrics_view` adapts a
//!   real [`crate::match_snapshot::MatchState`] into one.
//!
//!   This module's view stays genuinely narrow — a fun-proxy observer has
//!   no business seeing keeper release timers or tactic state — distinct
//!   from `crate::bot`'s equally narrow but differently shaped
//!   `BotMatchView`/`BotPlayerView`.
//! - AGENTS.md §3 forbids stray global mutable state, so [`crate::tuning::Tuning`]
//!   is an owned value, not a singleton (see that module's doc). [`observe`]
//!   therefore takes an explicit `&Tuning` parameter rather than reading a
//!   global.

use crate::metric_registry;
use crate::tuning::Tuning;
use gc_core::vec2::Vec2;
use indexmap::IndexMap;

/// Seconds a team must hold the ball before its possession is "settled" —
/// the unit turnovers are counted in (see [`observe`]).
pub const SETTLE_HOLD: f64 = 0.7;

/// A fixture side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchTeam {
    /// Home side.
    Home,
    /// Away side.
    Away,
}

/// The six live keeper behavior states this collector buckets time and
/// outcomes by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeeperState {
    /// Default positioning.
    Base,
    /// Advancing off the line.
    Advance,
    /// Containing an attacker.
    Contain,
    /// Set for a shot.
    Set,
    /// Retreating.
    Retreat,
    /// Recovering position.
    Recover,
}

impl KeeperState {
    fn index(self) -> usize {
        match self {
            KeeperState::Base => 0,
            KeeperState::Advance => 1,
            KeeperState::Contain => 2,
            KeeperState::Set => 3,
            KeeperState::Retreat => 4,
            KeeperState::Recover => 5,
        }
    }
}

/// A `KeeperState` outcome bucket, plus "unclassified" for a save/goal with
/// no attributable keeper state.
const UNCLASSIFIED_INDEX: usize = 6;

fn keeper_metric_index(state: Option<KeeperState>) -> usize {
    state.map_or(UNCLASSIFIED_INDEX, KeeperState::index)
}

/// One player's collector-relevant state for one observed frame.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricsPlayerView {
    /// Stable player identity.
    pub id: String,
    /// Fixture side.
    pub team: MatchTeam,
    /// Whether this player is the keeper.
    pub is_keeper: bool,
    /// Current velocity.
    pub vel: Vec2,
    /// Locomotion velocity (px/s) — the part of motion `gc_sim::locomotion`
    /// owns, before knockback and other external impulses fold into [`vel`].
    ///
    /// `time_to_reverse` reads this rather than `vel` on purpose: a body
    /// shoved backwards by a tackle is not a body that chose to turn around,
    /// and measuring the primitive means measuring what the primitive drives.
    ///
    /// [`vel`]: MetricsPlayerView::vel
    pub run_vel: Vec2,
    /// Current move speed, px/s.
    pub move_speed: f64,
    /// Whether the player is currently sprinting.
    pub sprinting: bool,
    /// Seconds remaining on an active dodge/juke.
    pub dodge_timer: f64,
    /// Live keeper behavior state; only meaningful when `is_keeper`. `None`
    /// falls back to [`KeeperState::Base`] for the time bucket.
    pub keeper_state: Option<KeeperState>,
}

/// One frame's match event, in the shape [`observe`] inspects. `kind` and
/// `shot_type` stay open strings — [`crate::r#match`] owns the full event
/// vocabulary; this collector only branches on a subset of it and ignores
/// everything else.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricsEventView {
    /// Event kind, e.g. `"pass"`, `"shot"`, `"catch"`, `"touch"`, `"juke"`.
    pub kind: String,
    /// The player this event is attributed to, if any.
    pub player: Option<String>,
    /// Shot subtype, e.g. `"chip"`. Only meaningful for strike events.
    pub shot_type: Option<String>,
    /// Whether a strike was on target. Only meaningful for strike events.
    pub on_target: Option<bool>,
    /// The keeper's behavior state at the moment of this event, if
    /// attributable.
    pub keeper_state: Option<KeeperState>,
    /// The keeper's depth (px) the strike was released from, if recorded.
    pub keeper_depth: Option<f64>,
}

/// Match score.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Score {
    /// Home goals.
    pub home: i64,
    /// Away goals.
    pub away: i64,
}

/// The minimal match-state surface [`observe`] needs.
#[derive(Clone, Debug, PartialEq)]
pub struct MetricsMatchView {
    /// Every player in the fixture.
    pub players: Vec<MetricsPlayerView>,
    /// Whether a human is controlling the `controlled` slot.
    pub human_controlled: bool,
    /// Index (0-based; not a wire value, see ARCHITECTURE.md §3 rule 3) of the
    /// human-controllable player.
    pub controlled: usize,
    /// Current score.
    pub score: Score,
    /// Index (0-based) of the ball owner, if the ball is owned.
    pub owner: Option<usize>,
    /// This frame's events, populated by the same `dt` step [`observe`] is
    /// called with.
    pub events: Vec<MetricsEventView>,
}

/// Which population a dribble observation belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DribbleRole {
    /// The human-controlled slot.
    Controlled,
    /// A team-AI slot.
    Ai,
}

/// Accumulated dribble-usage time and counts for one population
/// ([`DribbleRole::Controlled`] or [`DribbleRole::Ai`]).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct DribbleMetricCollector {
    /// Seconds spent carrying the ball.
    pub carry_s: f64,
    /// Seconds spent carrying under close control.
    pub close_s: f64,
    /// Seconds spent carrying while sprinting.
    pub sprint_s: f64,
    /// Seconds spent carrying mid-juke.
    pub juke_s: f64,
    /// Touches while carrying.
    pub touches: i64,
    /// Heavy touches that lost possession.
    pub heavy_losses: i64,
    /// Jukes attempted.
    pub jukes: i64,
}

/// Both dribble-usage populations.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct DribbleBuckets {
    /// Human-controlled population.
    pub controlled: DribbleMetricCollector,
    /// Team-AI population.
    pub ai: DribbleMetricCollector,
}

/// Home/away split of owned-ball time.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct OwnTime {
    /// Seconds home has owned the ball.
    pub home: f64,
    /// Seconds away has owned the ball.
    pub away: f64,
}

/// One recorded goal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GoalEvent {
    /// Match seconds elapsed when the goal was recorded.
    pub t: f64,
    /// Scoring side.
    pub team: MatchTeam,
}

/// Fraction of base move speed a body must be carrying for its heading to
/// count as a cruise that a later reversal is measured against.
///
/// #488 words this "at top speed". The reference here is
/// [`MetricsPlayerView::move_speed`], the player's BASE speed, not the
/// resolved locomotion context's top speed — contexts multiply that base by
/// 0.6 (backpedal) to 1.35 (sprint). Using the base is a deliberate deviation
/// and it is the honest one available at this layer: the context is a pure
/// function of state AND the tick's inputs, and the metrics collector sees
/// only state, so computing it here would mean duplicating
/// `locomotion::resolve` against inputs it does not have. What the metric
/// therefore measures is "a reversal from a genuine run", which is the case
/// the primitive is about; a reversal out of a backpedal never arms it.
const REVERSAL_ARM_FRACTION: f64 = 0.8;

/// Fraction of base move speed the body must have rebuilt, in the new
/// direction, for the reversal to count as completed. #488's figure.
const REVERSAL_COMPLETE_FRACTION: f64 = 0.5;

/// Cosine of the half-arc the new heading must fall inside to count as "the
/// opposite direction". #488 says 15 degrees; `cos(15 deg)` is authored
/// directly for the reason `gc_data::locomotion`'s arc knobs are — this is
/// compared against a dot product every tick per player, and a `cos()` there
/// would put a non-correctly-rounded libm call on a path two targets must
/// agree on.
const REVERSAL_ARC_COS: f64 = 0.965_925_826_289_068_3;

/// Fraction of base move speed the body must drop BELOW at some point during
/// the event for it to count as a reversal rather than an arc.
///
/// This is the observable signature of `locomotion`'s reversing branch, and
/// it is what makes the metric measure what #488 specified rather than
/// something broader. A commanded 180-degree reversal always trips that
/// branch — the arc thresholds put anything past 120 degrees there — so the
/// body brakes to the snap speed and pivots at exactly zero. A body that
/// merely turned a long way round keeps its pace through the arc and never
/// comes near zero.
///
/// The first draft of this metric had no such gate, and the difference is not
/// academic: it counted every "ended up going the other way", arcs included,
/// and `LOCO_RUN_DECEL_MULT` then reported DECORATION at a 77% perturbation
/// (delta -0.007 against a 0.017 threshold). Braking is most of a true
/// reversal and almost none of an arc, so folding the two populations
/// together diluted exactly the term the metric exists to measure.
const REVERSAL_THROUGH_ZERO_FRACTION: f64 = 0.1;

/// Seconds after leaving cruise beyond which a body is no longer considered
/// to be reversing. Without this, a player who slowed for unrelated reasons
/// and happened to run the other way a minute later would record an enormous
/// "reversal".
const REVERSAL_TIMEOUT_S: f64 = 2.0;

/// One player's in-flight reversal measurement.
///
/// The reversal is detected from the TRAJECTORY, not from a stored command,
/// and that is a design constraint rather than a shortcut: persisting the
/// commanded direction would add a `MatchPlayer` field, hence a snapshot
/// version bump, hence state two peers can disagree about — the same argument
/// that kept the locomotion context out of the snapshot (see
/// `docs/design/locomotion.md`). A reversal is fully observable without it:
/// the body was running one way at speed, and it is now running the other way
/// at speed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReversalTracker {
    /// Heading the body last cruised on, if it has cruised recently.
    pub cruise_dir: Option<Vec2>,
    /// Seconds elapsed since the body left that cruise.
    pub since_cruise_s: f64,
    /// Whether the body has passed near enough to a standstill during this
    /// event to have gone through `locomotion`'s reversing branch rather than
    /// around an arc.
    pub through_zero: bool,
}

/// Running per-match accumulator, updated one frame at a time via
/// [`observe`].
#[derive(Clone, Debug, PartialEq)]
pub struct MetricsCollector {
    /// Seconds observed so far.
    pub t: f64,
    /// Every recorded goal, in order.
    pub goals: Vec<GoalEvent>,
    /// Home score as of the last [`observe`] call.
    pub prev_home: i64,
    /// Away score as of the last [`observe`] call.
    pub prev_away: i64,
    /// Outfield strikes at goal (shot/header/volley/bicycle).
    pub shots: i64,
    /// Keeper catches + parries.
    pub saves: i64,
    /// Passes attempted.
    pub passes: i64,
    /// Passes completed.
    pub passes_completed: i64,
    /// A pass in flight, resolved at the next ownership change.
    pub pending_pass: Option<MatchTeam>,
    /// Settled-possession changes.
    pub turnovers: i64,
    /// Last team with SETTLED possession.
    pub settled_team: Option<MatchTeam>,
    /// Team of the current ownership streak.
    pub hold_team: Option<MatchTeam>,
    /// Seconds the streak team has held the ball.
    pub hold_t: f64,
    /// Home/away split of owned-ball time.
    pub own_time: OwnTime,
    /// Time of the last shot or goal, for drought tracking.
    pub last_chance_t: f64,
    /// Longest observed scoring drought, in seconds.
    pub longest_drought: f64,
    /// Player id -> team.
    pub team_of: IndexMap<String, MatchTeam>,
    /// Player id -> is keeper.
    pub keeper: IndexMap<String, bool>,
    /// Player id -> `MetricsMatchView.players` index.
    pub index_of: IndexMap<String, usize>,
    /// Ball owner id as of the last frame.
    pub prev_owner_id: Option<String>,
    /// Ball owner's dribble role as of the last frame.
    pub prev_owner_role: Option<DribbleRole>,
    /// Both dribble-usage populations.
    pub dribble: DribbleBuckets,
    /// Seconds spent in each keeper behavior state.
    pub keeper_state_s: [f64; 6],
    /// Saves by attributed keeper state, `[6]` is "unclassified".
    pub keeper_saves_by_state: [i64; 7],
    /// Goals conceded by attributed keeper state, `[6]` is "unclassified".
    pub keeper_goals_by_state: [i64; 7],
    /// Running sum of keeper release depth at the moment of a strike.
    pub keeper_shot_depth_sum: f64,
    /// Count of strikes with a recorded keeper release depth.
    pub keeper_shot_depth_count: i64,
    /// Chip strikes attempted.
    pub chip_shots: i64,
    /// Chip strikes on target.
    pub chip_on_target: i64,
    /// Chip strikes scored.
    pub chip_goals: i64,
    /// Per-player reversal tracking, indexed like `MetricsMatchView.players`.
    pub reversal: Vec<ReversalTracker>,
    /// Completed reversals: total seconds, for the mean.
    pub reversal_s: f64,
    /// Completed reversals: how many.
    pub reversals: i64,
    /// Scoring side of the most recent unresolved strike.
    pub pending_shot_team: Option<MatchTeam>,
    /// Shot subtype of the most recent unresolved strike.
    pub pending_shot_type: Option<String>,
    /// Keeper state at the most recent unresolved strike.
    pub pending_shot_keeper_state: Option<KeeperState>,
    /// Standing-poke tackle attempts: a charge that reached its
    /// executing-phase resolution (#489). Hit + miss.
    pub tackle_attempts: i64,
    /// Standing-poke tackle attempts that resolved without winning the ball.
    pub tackle_misses: i64,
}

fn dribble_bucket_for_mut(
    buckets: &mut DribbleBuckets,
    role: DribbleRole,
) -> &mut DribbleMetricCollector {
    match role {
        DribbleRole::Controlled => &mut buckets.controlled,
        DribbleRole::Ai => &mut buckets.ai,
    }
}

/// A fresh collector, seeded from the match's starting state.
#[must_use]
pub fn new(s: &MetricsMatchView) -> MetricsCollector {
    let mut team_of = IndexMap::new();
    let mut keeper = IndexMap::new();
    let mut index_of = IndexMap::new();
    for (i, p) in s.players.iter().enumerate() {
        team_of.insert(p.id.clone(), p.team);
        keeper.insert(p.id.clone(), p.is_keeper);
        index_of.insert(p.id.clone(), i);
    }
    let reversal = vec![ReversalTracker::default(); s.players.len()];
    let prev_owner = s.owner.map(|i| &s.players[i]);
    let prev_role = prev_owner.and_then(|owner| {
        if owner.is_keeper {
            None
        } else {
            Some(if s.human_controlled && s.owner == Some(s.controlled) {
                DribbleRole::Controlled
            } else {
                DribbleRole::Ai
            })
        }
    });

    MetricsCollector {
        reversal,
        reversal_s: 0.0,
        reversals: 0,
        t: 0.0,
        goals: Vec::new(),
        prev_home: s.score.home,
        prev_away: s.score.away,
        shots: 0,
        saves: 0,
        passes: 0,
        passes_completed: 0,
        pending_pass: None,
        turnovers: 0,
        settled_team: None,
        hold_team: None,
        hold_t: 0.0,
        own_time: OwnTime::default(),
        last_chance_t: 0.0,
        longest_drought: 0.0,
        team_of,
        keeper,
        index_of,
        prev_owner_id: prev_owner.map(|owner| owner.id.clone()),
        prev_owner_role: prev_role,
        dribble: DribbleBuckets::default(),
        keeper_state_s: [0.0; 6],
        keeper_saves_by_state: [0; 7],
        keeper_goals_by_state: [0; 7],
        keeper_shot_depth_sum: 0.0,
        keeper_shot_depth_count: 0,
        chip_shots: 0,
        chip_on_target: 0,
        chip_goals: 0,
        pending_shot_team: None,
        pending_shot_type: None,
        pending_shot_keeper_state: None,
        tackle_attempts: 0,
        tackle_misses: 0,
    }
}

fn dribble_role(s: &MetricsMatchView, player_id: &str) -> DribbleRole {
    let index = s.players.iter().position(|p| p.id == player_id);
    if s.human_controlled && index == Some(s.controlled) {
        DribbleRole::Controlled
    } else {
        DribbleRole::Ai
    }
}

/// One tick of `time_to_reverse` accumulation.
///
/// The state machine is deliberately small. A body at or above
/// [`REVERSAL_ARM_FRACTION`] of its base speed is *cruising*, and its heading
/// is remembered along with a clock that resets every tick it keeps cruising.
/// A reversal is complete when the body is back above
/// [`REVERSAL_COMPLETE_FRACTION`] while heading within
/// [`REVERSAL_ARC_COS`] of the OPPOSITE of that remembered heading; the
/// elapsed clock is the measurement.
///
/// Completion is checked before the cruise is refreshed, so the two
/// thresholds may be set equal without the arming update swallowing the
/// event. A body that slows and then leaves in some direction that is not the
/// opposite simply re-arms on its new heading, which is the correct reading:
/// it turned, it did not reverse. [`REVERSAL_TIMEOUT_S`] discards a cruise
/// nothing followed, so a player who stops for unrelated reasons and runs
/// back a minute later does not record a minute-long reversal.
fn observe_reversals(c: &mut MetricsCollector, s: &MetricsMatchView, dt: f64) {
    for (index, player) in s.players.iter().enumerate() {
        let Some(tracker) = c.reversal.get_mut(index) else {
            continue;
        };
        if player.move_speed <= 0.0 {
            *tracker = ReversalTracker::default();
            continue;
        }
        let speed = player.run_vel.length();
        let fraction = speed / player.move_speed;
        let heading = if speed > 0.0 {
            Some(player.run_vel.normalized())
        } else {
            None
        };

        if let Some(cruise) = tracker.cruise_dir {
            tracker.since_cruise_s += dt;
            if fraction < REVERSAL_THROUGH_ZERO_FRACTION {
                tracker.through_zero = true;
            }
            if let Some(heading) = heading
                && tracker.through_zero
                && fraction >= REVERSAL_COMPLETE_FRACTION
            {
                let opposed = -(heading.x * cruise.x + heading.y * cruise.y);
                if opposed >= REVERSAL_ARC_COS {
                    c.reversal_s += tracker.since_cruise_s;
                    c.reversals += 1;
                    *tracker = ReversalTracker::default();
                    continue;
                }
            }
            if tracker.since_cruise_s > REVERSAL_TIMEOUT_S {
                *tracker = ReversalTracker::default();
            }
        }

        if let Some(heading) = heading
            && fraction >= REVERSAL_ARM_FRACTION
        {
            tracker.cruise_dir = Some(heading);
            tracker.since_cruise_s = 0.0;
            tracker.through_zero = false;
        }
    }
}

/// Observe one frame, AFTER stepping the match for the same `dt` (so
/// `s.events` holds exactly this frame's actions).
pub fn observe(c: &mut MetricsCollector, s: &MetricsMatchView, dt: f64, tuning: &Tuning) {
    c.t += dt;

    for player in &s.players {
        if player.is_keeper {
            let state = player.keeper_state.unwrap_or(KeeperState::Base);
            c.keeper_state_s[state.index()] += dt;
        }
    }

    observe_reversals(c, s, dt);

    for e in &s.events {
        let team = e.player.as_deref().and_then(|p| c.team_of.get(p).copied());
        let is_keeper = e
            .player
            .as_deref()
            .is_some_and(|p| c.keeper.get(p).copied().unwrap_or(false));
        match e.kind.as_str() {
            "pass" => {
                c.passes += 1;
                c.pending_pass = team;
            }
            "catch" | "parry" => {
                c.saves += 1;
                c.keeper_saves_by_state[keeper_metric_index(e.keeper_state)] += 1;
                c.pending_shot_team = None;
                c.pending_shot_type = None;
                c.pending_shot_keeper_state = None;
            }
            "shot" | "header" | "volley" | "bicycle" if !is_keeper => {
                // Keeper "shot" events are punts/clearances, not strikes at
                // goal.
                c.shots += 1;
                c.longest_drought = c.longest_drought.max(c.t - c.last_chance_t);
                c.last_chance_t = c.t;
                c.pending_shot_team = team;
                c.pending_shot_type.clone_from(&e.shot_type);
                c.pending_shot_keeper_state = e.keeper_state;
                if let Some(depth) = e.keeper_depth {
                    c.keeper_shot_depth_sum += depth;
                    c.keeper_shot_depth_count += 1;
                }
                if e.shot_type.as_deref() == Some("chip") {
                    c.chip_shots += 1;
                    if e.on_target == Some(true) {
                        c.chip_on_target += 1;
                    }
                }
            }
            "touch" if e.player.is_some() && c.prev_owner_id == e.player => {
                let role = c.prev_owner_role.unwrap_or(DribbleRole::Ai);
                let owner = s.owner.map(|i| &s.players[i]);
                let bucket = dribble_bucket_for_mut(&mut c.dribble, role);
                match owner {
                    Some(owner) if Some(&owner.id) == e.player.as_ref() => bucket.touches += 1,
                    None => bucket.heavy_losses += 1,
                    Some(_) => {}
                }
            }
            "juke" => {
                if let Some(player_id) = &e.player {
                    let role = dribble_role(s, player_id);
                    dribble_bucket_for_mut(&mut c.dribble, role).jukes += 1;
                }
            }
            // #489: a committed standing-poke tackle's two resolutions.
            // `"tackle"` already fired only on a successful steal, before
            // this metric existed; `"tackle_miss"` is new, fired once per
            // whiffed executing-phase resolution.
            "tackle" => {
                c.tackle_attempts += 1;
            }
            "tackle_miss" => {
                c.tackle_attempts += 1;
                c.tackle_misses += 1;
            }
            _ => {}
        }
    }

    if s.score.home > c.prev_home || s.score.away > c.prev_away {
        let team = if s.score.home > c.prev_home {
            MatchTeam::Home
        } else {
            MatchTeam::Away
        };
        c.goals.push(GoalEvent { t: c.t, team });
        c.prev_home = s.score.home;
        c.prev_away = s.score.away;
        c.longest_drought = c.longest_drought.max(c.t - c.last_chance_t);
        c.last_chance_t = c.t;
        c.pending_pass = None; // a goal ends any pass in flight
        if c.pending_shot_team == Some(team) {
            c.keeper_goals_by_state[keeper_metric_index(c.pending_shot_keeper_state)] += 1;
            if c.pending_shot_type.as_deref() == Some("chip") {
                c.chip_goals += 1;
            }
        }
        c.pending_shot_team = None;
        c.pending_shot_type = None;
        c.pending_shot_keeper_state = None;
    }

    let owner_team = s.owner.map(|i| s.players[i].team);
    if let Some(owner_team) = owner_team {
        match owner_team {
            MatchTeam::Home => c.own_time.home += dt,
            MatchTeam::Away => c.own_time.away += dt,
        }
        if let Some(pending) = c.pending_pass {
            if owner_team == pending {
                c.passes_completed += 1;
            }
            c.pending_pass = None;
        }
        // A turnover is settled possession changing team, not ownership
        // flicker: in a poke-and-scramble the ball changes hands every few
        // frames, and counting each touch reads as ping-pong chaos. The new
        // team must hold the ball SETTLE_HOLD seconds (pass flights bridge:
        // loose frames pause the streak, only the other team's touch resets).
        if Some(owner_team) != c.hold_team {
            c.hold_team = Some(owner_team);
            c.hold_t = 0.0;
        }
        c.hold_t += dt;
        if c.hold_t >= SETTLE_HOLD && c.settled_team != Some(owner_team) {
            if c.settled_team.is_some() {
                c.turnovers += 1;
            }
            c.settled_team = Some(owner_team);
        }
    }

    if let Some(owner_index) = s.owner {
        let owner = &s.players[owner_index];
        if !owner.is_keeper {
            let role = if s.human_controlled && owner_index == s.controlled {
                DribbleRole::Controlled
            } else {
                DribbleRole::Ai
            };
            let bucket = dribble_bucket_for_mut(&mut c.dribble, role);
            bucket.carry_s += dt;
            if owner.vel.length() < owner.move_speed * tuning.value("DRIBBLE_CLOSE") {
                bucket.close_s += dt;
            }
            if owner.sprinting {
                bucket.sprint_s += dt;
            }
            if owner.dodge_timer > 0.0 {
                bucket.juke_s += dt;
            }
            c.prev_owner_role = Some(role);
        } else {
            c.prev_owner_role = None;
        }
        c.prev_owner_id = Some(owner.id.clone());
    } else {
        c.prev_owner_id = None;
        c.prev_owner_role = None;
    }
}

// The moment the final winner took a lead they never lost (draw: the full
// match — tension never resolved).
fn decided_at(goals: &[GoalEvent], duration: f64) -> f64 {
    let mut diff = 0_i64;
    for g in goals {
        diff += if g.team == MatchTeam::Home { 1 } else { -1 };
    }
    if diff == 0 || duration <= 0.0 {
        return 1.0;
    }
    let winner = if diff > 0 {
        MatchTeam::Home
    } else {
        MatchTeam::Away
    };
    // Walk backwards: the deciding goal is the one that last put the winner
    // ahead for good (margin from the loser's view never recovers after it).
    let (mut h, mut a) = (0_i64, 0_i64);
    let mut decided = 0.0_f64;
    for g in goals {
        if g.team == MatchTeam::Home {
            h += 1;
        } else {
            a += 1;
        }
        let lead = if winner == MatchTeam::Home {
            h - a
        } else {
            a - h
        };
        if lead == 1 {
            decided = g.t; // candidate; overwritten if the lead is later lost
        }
    }
    (decided / duration).min(1.0)
}

fn lead_changes(goals: &[GoalEvent]) -> i64 {
    let (mut h, mut a, mut leader, mut changes) = (0_i64, 0_i64, 0_i64, 0_i64);
    for g in goals {
        if g.team == MatchTeam::Home {
            h += 1;
        } else {
            a += 1;
        }
        let sign = if h > a {
            1
        } else if a > h {
            -1
        } else {
            0
        };
        if sign != 0 && leader != 0 && sign != leader {
            changes += 1;
        }
        if sign != 0 {
            leader = sign;
        }
    }
    changes
}

/// Per-match summary metrics. Rate metrics are `None` when their denominator
/// never happened (e.g. `save_rate` with zero on-target shots) and are
/// skipped by [`fun_score`] rather than defaulted.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct MatchMetrics {
    /// Seconds of match time observed.
    pub duration: f64,
    /// Home goals.
    pub goals_home: i64,
    /// Away goals.
    pub goals_away: i64,
    /// Total goals.
    pub goals_total: i64,
    /// Absolute goal margin.
    pub margin: i64,
    /// Number of times the leader changed.
    pub lead_changes: i64,
    /// Fraction of the match elapsed when the winner took the lead for good
    /// (a draw is `1`).
    pub decided_late: f64,
    /// Outfield strikes at goal.
    pub shots: i64,
    /// Strikes per goal scored.
    pub shots_per_goal: Option<f64>,
    /// Save rate among on-target attempts.
    pub save_rate: Option<f64>,
    /// Passes attempted.
    pub passes: i64,
    /// Pass completion rate.
    pub pass_completion: Option<f64>,
    /// Settled-possession turnovers per minute.
    pub turnovers_per_min: f64,
    /// Home's share of owned time.
    pub possession_balance: Option<f64>,
    /// Longest scoring drought, in seconds.
    pub longest_drought_s: f64,
    /// Mean seconds a body takes to complete a 180-degree reversal, or `None`
    /// when the match contained no completed reversal to measure.
    ///
    /// `None` rather than `0.0` deliberately: zero is a *fast* reversal and
    /// would read as the best possible score, so a match that never armed the
    /// measurement would silently certify the primitive as perfect. The band
    /// treats absence as absence.
    pub time_to_reverse: Option<f64>,
    /// How many completed reversals the mean above is over. Reported so a
    /// mean resting on three samples is visibly different from one resting on
    /// three hundred.
    pub reversals: i64,
    /// Controlled-slot carry time, in seconds.
    pub controlled_dribble_carry_s: f64,
    /// Controlled-slot close-control share of carry time.
    pub controlled_dribble_close_share: Option<f64>,
    /// Controlled-slot sprint share of carry time.
    pub controlled_dribble_sprint_share: Option<f64>,
    /// Controlled-slot juke share of carry time.
    pub controlled_dribble_juke_share: Option<f64>,
    /// Controlled-slot touches per minute of carry time.
    pub controlled_dribble_touches_per_min: f64,
    /// Controlled-slot heavy losses per minute of carry time.
    pub controlled_dribble_heavy_losses_per_min: f64,
    /// Controlled-slot jukes attempted.
    pub controlled_jukes: i64,
    /// Team-AI carry time, in seconds.
    pub ai_dribble_carry_s: f64,
    /// Team-AI close-control share of carry time.
    pub ai_dribble_close_share: Option<f64>,
    /// Team-AI sprint share of carry time.
    pub ai_dribble_sprint_share: Option<f64>,
    /// Team-AI juke share of carry time.
    pub ai_dribble_juke_share: Option<f64>,
    /// Team-AI touches per minute of carry time.
    pub ai_dribble_touches_per_min: f64,
    /// Team-AI heavy losses per minute of carry time.
    pub ai_dribble_heavy_losses_per_min: f64,
    /// Team-AI jukes attempted.
    pub ai_jukes: i64,
    /// Seconds the keeper spent in `base`.
    pub keeper_base_s: f64,
    /// Seconds the keeper spent in `advance`.
    pub keeper_advance_s: f64,
    /// Seconds the keeper spent in `contain`.
    pub keeper_contain_s: f64,
    /// Seconds the keeper spent in `set`.
    pub keeper_set_s: f64,
    /// Seconds the keeper spent in `retreat`.
    pub keeper_retreat_s: f64,
    /// Seconds the keeper spent in `recover`.
    pub keeper_recover_s: f64,
    /// Mean keeper release depth at the moment of a strike.
    pub keeper_shot_depth_mean: Option<f64>,
    /// Chip strikes attempted.
    pub chip_shots: i64,
    /// Chip strikes on target.
    pub chip_on_target: i64,
    /// Chip strikes scored.
    pub chip_goals: i64,
    /// Chip conversion rate.
    pub chip_conversion: Option<f64>,
    /// Saves while the keeper was in `base`.
    pub keeper_saves_base: i64,
    /// Saves while the keeper was in `advance`.
    pub keeper_saves_advance: i64,
    /// Saves while the keeper was in `contain`.
    pub keeper_saves_contain: i64,
    /// Saves while the keeper was in `set`.
    pub keeper_saves_set: i64,
    /// Saves while the keeper was in `retreat`.
    pub keeper_saves_retreat: i64,
    /// Saves while the keeper was in `recover`.
    pub keeper_saves_recover: i64,
    /// Saves with no attributable keeper state.
    pub keeper_saves_unclassified: i64,
    /// Goals conceded while the keeper was in `base`.
    pub keeper_goals_base: i64,
    /// Goals conceded while the keeper was in `advance`.
    pub keeper_goals_advance: i64,
    /// Goals conceded while the keeper was in `contain`.
    pub keeper_goals_contain: i64,
    /// Goals conceded while the keeper was in `set`.
    pub keeper_goals_set: i64,
    /// Goals conceded while the keeper was in `retreat`.
    pub keeper_goals_retreat: i64,
    /// Goals conceded while the keeper was in `recover`.
    pub keeper_goals_recover: i64,
    /// Goals conceded with no attributable keeper state.
    pub keeper_goals_unclassified: i64,
    /// Mean aim error of aimed pass releases, in chord (`0..=2`) — the angle
    /// between where the passer pointed and the receiver soft-cone selection
    /// chose. Stamped on by the headless runner from
    /// `r#match::pass_shadow_take`, never set by [`finish`]: the tally lives
    /// on a diagnostic thread-local rather than in simulation state, so it
    /// cannot enter a snapshot or a state hash. `None` when the match
    /// contained no aimed release.
    pub pass_aim_error: Option<f64>,
    /// Mean lead time in seconds of driven ground passes, counting an unled
    /// pass as zero — how far into the receiver's run passes are played.
    /// Stamped on by the headless runner, same seam and same reason as
    /// [`Self::pass_aim_error`]. `None` when the match contained no driven
    /// ground pass.
    pub pass_lead_time: Option<f64>,
    /// Composite fun score, stamped on by the headless runner (never set by
    /// [`finish`]).
    pub fun: Option<f64>,
    /// Missed standing-poke tackles / attempted standing-poke tackles
    /// (#489). `None` when the match attempted none.
    pub whiff_rate: Option<f64>,
}

/// Fold the collector and final match state into a [`MatchMetrics`] summary.
#[must_use]
pub fn finish(c: &mut MetricsCollector, s: &MetricsMatchView) -> MatchMetrics {
    c.longest_drought = c.longest_drought.max(c.t - c.last_chance_t);
    let longest_drought = c.longest_drought;
    let gh = s.score.home;
    let ga = s.score.away;
    let owned = c.own_time.home + c.own_time.away;
    let on_target = c.saves + gh + ga;
    let controlled = c.dribble.controlled;
    let ai = c.dribble.ai;

    MatchMetrics {
        duration: c.t,
        goals_home: gh,
        goals_away: ga,
        goals_total: gh + ga,
        margin: (gh - ga).abs(),
        lead_changes: lead_changes(&c.goals),
        decided_late: decided_at(&c.goals, c.t),
        shots: c.shots,
        shots_per_goal: (gh + ga > 0).then(|| c.shots as f64 / (gh + ga) as f64),
        save_rate: (on_target > 0).then(|| c.saves as f64 / on_target as f64),
        passes: c.passes,
        pass_completion: (c.passes > 0).then(|| c.passes_completed as f64 / c.passes as f64),
        turnovers_per_min: if c.t > 0.0 {
            c.turnovers as f64 / (c.t / 60.0)
        } else {
            0.0
        },
        possession_balance: (owned > 0.0).then_some(c.own_time.home / owned),
        longest_drought_s: longest_drought,
        time_to_reverse: (c.reversals > 0).then(|| c.reversal_s / c.reversals as f64),
        reversals: c.reversals,
        controlled_dribble_carry_s: controlled.carry_s,
        controlled_dribble_close_share: (controlled.carry_s > 0.0)
            .then_some(controlled.close_s / controlled.carry_s),
        controlled_dribble_sprint_share: (controlled.carry_s > 0.0)
            .then_some(controlled.sprint_s / controlled.carry_s),
        controlled_dribble_juke_share: (controlled.carry_s > 0.0)
            .then_some(controlled.juke_s / controlled.carry_s),
        controlled_dribble_touches_per_min: if controlled.carry_s > 0.0 {
            controlled.touches as f64 / (controlled.carry_s / 60.0)
        } else {
            0.0
        },
        controlled_dribble_heavy_losses_per_min: if controlled.carry_s > 0.0 {
            controlled.heavy_losses as f64 / (controlled.carry_s / 60.0)
        } else {
            0.0
        },
        controlled_jukes: controlled.jukes,
        ai_dribble_carry_s: ai.carry_s,
        ai_dribble_close_share: (ai.carry_s > 0.0).then_some(ai.close_s / ai.carry_s),
        ai_dribble_sprint_share: (ai.carry_s > 0.0).then_some(ai.sprint_s / ai.carry_s),
        ai_dribble_juke_share: (ai.carry_s > 0.0).then_some(ai.juke_s / ai.carry_s),
        ai_dribble_touches_per_min: if ai.carry_s > 0.0 {
            ai.touches as f64 / (ai.carry_s / 60.0)
        } else {
            0.0
        },
        ai_dribble_heavy_losses_per_min: if ai.carry_s > 0.0 {
            ai.heavy_losses as f64 / (ai.carry_s / 60.0)
        } else {
            0.0
        },
        ai_jukes: ai.jukes,
        keeper_base_s: c.keeper_state_s[KeeperState::Base.index()],
        keeper_advance_s: c.keeper_state_s[KeeperState::Advance.index()],
        keeper_contain_s: c.keeper_state_s[KeeperState::Contain.index()],
        keeper_set_s: c.keeper_state_s[KeeperState::Set.index()],
        keeper_retreat_s: c.keeper_state_s[KeeperState::Retreat.index()],
        keeper_recover_s: c.keeper_state_s[KeeperState::Recover.index()],
        keeper_shot_depth_mean: (c.keeper_shot_depth_count > 0)
            .then_some(c.keeper_shot_depth_sum / c.keeper_shot_depth_count as f64),
        chip_shots: c.chip_shots,
        chip_on_target: c.chip_on_target,
        chip_goals: c.chip_goals,
        chip_conversion: (c.chip_shots > 0).then(|| c.chip_goals as f64 / c.chip_shots as f64),
        keeper_saves_base: c.keeper_saves_by_state[KeeperState::Base.index()],
        keeper_saves_advance: c.keeper_saves_by_state[KeeperState::Advance.index()],
        keeper_saves_contain: c.keeper_saves_by_state[KeeperState::Contain.index()],
        keeper_saves_set: c.keeper_saves_by_state[KeeperState::Set.index()],
        keeper_saves_retreat: c.keeper_saves_by_state[KeeperState::Retreat.index()],
        keeper_saves_recover: c.keeper_saves_by_state[KeeperState::Recover.index()],
        keeper_saves_unclassified: c.keeper_saves_by_state[UNCLASSIFIED_INDEX],
        keeper_goals_base: c.keeper_goals_by_state[KeeperState::Base.index()],
        keeper_goals_advance: c.keeper_goals_by_state[KeeperState::Advance.index()],
        keeper_goals_contain: c.keeper_goals_by_state[KeeperState::Contain.index()],
        keeper_goals_set: c.keeper_goals_by_state[KeeperState::Set.index()],
        keeper_goals_retreat: c.keeper_goals_by_state[KeeperState::Retreat.index()],
        keeper_goals_recover: c.keeper_goals_by_state[KeeperState::Recover.index()],
        keeper_goals_unclassified: c.keeper_goals_by_state[UNCLASSIFIED_INDEX],
        // Both stamped on by the headless runner from the diagnostic tally;
        // `finish` has no access to it and must not pretend otherwise.
        pass_aim_error: None,
        pass_lead_time: None,
        fun: None,
        whiff_rate: (c.tackle_attempts > 0)
            .then(|| c.tackle_misses as f64 / c.tackle_attempts as f64),
    }
}

/// Desirability of `v` under a trapezoid band `{zero_lo, good_lo, good_hi,
/// zero_hi}`. Returns `0..1`.
///
/// The bands themselves moved to [`crate::metric_registry`]; this stays as the
/// name every caller already imports.
#[must_use]
pub fn desirability(v: f64, band: [f64; 4]) -> f64 {
    metric_registry::desirability(v, band)
}

/// Geometric mean of the registered metrics present in `m`. A collapsed
/// dimension (desirability 0) zeroes the whole score by design; missing
/// metrics (`None` denominators) are skipped, not defaulted. Returns the
/// score (`0..1`) and each contributing band's desirability, by key.
///
/// The bands and the fold order are [`crate::metric_registry`]'s now. This
/// module used to carry a private `BAND_*` table that two other modules
/// duplicated by hand (`crate::headless`'s `band_for`, `crate::lever_metrics`'s
/// `band_width`); all three read one registry. Scores are unchanged — the
/// registry folds the same eight metrics in the same order, which
/// `tests/metric_registry.rs` pins against the pre-migration values.
#[must_use]
pub fn fun_score(m: &MatchMetrics) -> (f64, IndexMap<&'static str, f64>) {
    metric_registry::shipped().fun_score(m)
}

/// Distribution stats for one metric across a batch of matches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricStats {
    /// Number of matches this metric was present for.
    pub n: i64,
    /// Mean.
    pub mean: f64,
    /// Sample standard deviation (`0` when `n <= 1`).
    pub sd: f64,
    /// Minimum.
    pub min: f64,
    /// Maximum.
    pub max: f64,
}

/// Per-key distribution stats over a batch of per-match metric maps — any
/// maps with numeric fields, not tied to [`MatchMetrics`]'s shape. Keys
/// missing from a given entry are excluded from that key's stats.
#[must_use]
pub fn aggregate<'a>(list: &[IndexMap<&'a str, f64>]) -> IndexMap<&'a str, MetricStats> {
    let mut keys: Vec<&'a str> = Vec::new();
    for m in list {
        for &k in m.keys() {
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
    }

    let mut out = IndexMap::new();
    for key in keys {
        let mut n = 0_i64;
        let mut sum = 0.0_f64;
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for m in list {
            if let Some(&v) = m.get(key) {
                n += 1;
                sum += v;
                min = min.min(v);
                max = max.max(v);
            }
        }
        let mean = sum / n as f64;
        let mut var = 0.0_f64;
        for m in list {
            if let Some(&v) = m.get(key) {
                var += (v - mean).powi(2);
            }
        }
        let sd = if n > 1 {
            (var / (n - 1) as f64).sqrt()
        } else {
            0.0
        };
        out.insert(
            key,
            MetricStats {
                n,
                mean,
                sd,
                min,
                max,
            },
        );
    }
    out
}
