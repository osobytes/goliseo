//! Support fixture for driver-level combat-correction tests.
//!
//! Driver-level boundary zeroes for the seven combat correction phases: wind-up,
//! guard, contact, projectile flight, stagger/knockback, ball spill, and
//! immunity expiry.
//!
//! Why this exists. `match_driver_fixture::initial_snapshot` pins one
//! combat-active boundary zero, and from it the AI-driven slots reach *some*
//! combat, eventually, somewhere on the pitch. That is enough to prove the
//! companion survives a correction; it is not enough to pin a named phase,
//! because nothing in that fixture decides which phase happens or when. Each
//! scenario here rigs boundary zero -- equipped families, opening pose, ball
//! ownership -- so the phase it is named for actually occurs inside a headless
//! run, and supplies the predicate that recognises it.
//!
//! Two routes reach a phase, and each scenario declares which one it uses:
//!
//!   * `Policy` -- the phase is produced by `gameplay_ai/combat/v1` itself,
//!     through `gc_sim::slot_input`'s bot combat signals. Nothing but the
//!     opening pose is arranged; the decision, the commit and the resolution
//!     are the shipped policy's.
//!   * `CanonicalInput` -- the phase is produced by a live slot's own
//!     equipment input, carried on the canonical input stream like any other
//!     authority. Used only where the policy cannot be steered into the phase
//!     deterministically from a fixed boundary zero (see `guard`).
//!
//! Neither route force-sets a combat runtime field. Every scenario's boundary
//! zero is a `Ready` combat state, so what a correction resimulates through is
//! a phase the simulation entered on its own, from authority every peer holds.
//!
//! Nothing here iterates a hash-keyed table in a way that reaches an output:
//! this module never reaches for `HashMap`/`HashSet` (denied workspace-wide
//! anyway), and `pool_for` walks `gc_data::players::ALL` in its authored
//! order.
//!
//! Usage. A caller owns its own driver harness and delivery pattern; this
//! module owns the fixture and the predicates:
//!
//! ```ignore
//! let snapshot = online_combat_phases::boundary_zero("projectile_flight", None); // every peer
//! // ... drive drivers built on `snapshot`, bursting delivery ...
//! // for a resimulated input tick T on some peer:
//! online_combat_phases::observed("projectile_flight", &snapshot_at_t, &snapshot_at_t_plus_1, &events_at_t);
//! ```
//!
//! `live_sample(id, step, index)` is the live slot's input program for the
//! scenario; it is `input_frame::neutral_sample()` for every `Policy` scenario.
//!
//! ## A note on runtime field-growth guards
//!
//! Copying a struct by an explicit field list and asserting against it is
//! one way to make a new field silently dropped from the players content
//! table fail loudly rather than vanishing. Rust does not need a runtime
//! check for that: `PlayerData` is a plain `Copy` struct, so
//! `let mut copied = *player;` carries every field by construction, and the
//! compiler -- not a runtime assertion -- refuses to compile if the struct
//! shape ever changes in a way this file does not account for.

use gc_core::vec2::Vec2;
use gc_data::action_families::ActionFamilyId;
use gc_data::players::PlayerData;
use gc_data::teams;
use gc_sim::combat;
use gc_sim::combat_snapshot::{CombatEvent, CombatEventKind, CombatMatchState, CombatPhase};
use gc_sim::input_frame::{self, InputSample};
use gc_sim::r#match::{self, NO_GOAL_LIMIT, NewMatchOptions};
use gc_sim::match_snapshot::{self, MatchSnapshot, PitchSize};
use indexmap::IndexMap;

/// Player indexes in a nebula-versus-orion `gc_sim::r#match` state (1-based,
/// matching `crates/gc-sim/src/match.rs`'s documented convention). 1 and 6
/// are the protected keepers, which are slotless and never combat-capable.
const HOME_OUTFIELD: [i64; 4] = [2, 3, 4, 5];
/// See [`HOME_OUTFIELD`].
const AWAY_OUTFIELD: [i64; 4] = [7, 8, 9, 10];

/// Playable pitch dimensions every scenario shares.
const FIELD: PitchSize = PitchSize { w: 960.0, h: 540.0 };
/// Long enough that no scenario can reach full time and turn a correction
/// question into a settle question.
const DURATION_SECONDS: f64 = 14.0;
/// The fixed seed every scenario's boundary zero is captured with.
const SEED: f64 = 74.0;
/// The opening lane the two facing lines are laid out along.
const LINE_X: f64 = 400.0;

/// The bar a driver-level guard-probe geometry must clear before `guard`
/// should move to the [`OnlineCombatPhaseRoute::Policy`] route: every peer
/// must see at least this many corrected-tick guard observations — see the
/// guard-probe section below for the full rationale.
pub const GUARD_POLICY_ROUTE_MINIMUM: i64 = 4;

/// Canonical order, and the order a caller should report results in.
pub const PHASES: [&str; 7] = [
    "windup",
    "guard",
    "contact",
    "projectile_flight",
    "stagger",
    "ball_spill",
    "immunity_expiry",
];

/// How a scenario's named phase is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnlineCombatPhaseRoute {
    /// The phase is produced by `gameplay_ai/combat/v1` itself, from nothing
    /// but the opening pose.
    Policy,
    /// The phase is produced by a live slot's own equipment input, carried on
    /// the canonical input stream.
    CanonicalInput,
}

/// Everything [`boundary_zero`] needs to lay out a fixture: who is armed with
/// what, and how the two lines stand.
#[derive(Clone, Copy, Debug)]
pub struct OnlineCombatFixtureShape {
    /// Loadout family every home outfielder carries.
    pub home_family: ActionFamilyId,
    /// Loadout family every away outfielder carries.
    pub away_family: ActionFamilyId,
    /// Opening gap between the two facing lines, in pixels.
    pub separation_px: f64,
    /// Vertical gap between rows, in pixels; a small one is a scrum.
    pub row_spacing_px: f64,
}

/// One named combat phase's driver-level scenario.
#[derive(Clone, Copy, Debug)]
pub struct OnlineCombatPhaseScenario {
    /// The phase this scenario reaches (one of [`PHASES`]).
    pub id: &'static str,
    /// How the phase is reached.
    pub route: OnlineCombatPhaseRoute,
    /// The opening fixture shape.
    pub shape: OnlineCombatFixtureShape,
    /// Driver steps the scenario needs to reach the phase.
    pub steps: i64,
    /// Transport drains every Nth step; the burst that corrects.
    pub deliver_period: i64,
    /// The host's live slot holds equipment on the canonical stream.
    pub hold_equipment: bool,
}

/// One loadout per family, matching the loadout combat-AI match tests use
/// elsewhere so a scenario here and a scenario there mean the same thing by
/// `light_melee`.
fn loadout_for(family: ActionFamilyId) -> &'static str {
    match family {
        ActionFamilyId::Unarmed => "loadout_spring_gloves",
        ActionFamilyId::Guard => "loadout_emberguard_shield",
        ActionFamilyId::LightMelee => "loadout_vector_blade",
        ActionFamilyId::Ranged => "loadout_pulse_blaster",
    }
}

/// The seven declared phase scenarios, one per entry in [`PHASES`].
const SCENARIOS: [OnlineCombatPhaseScenario; 7] = [
    // Light melee telegraphs the longest melee wind-up (12 ticks), so the
    // policy's own commit leaves a wide window for a burst to land inside.
    OnlineCombatPhaseScenario {
        id: "windup",
        route: OnlineCombatPhaseRoute::Policy,
        shape: OnlineCombatFixtureShape {
            home_family: ActionFamilyId::LightMelee,
            away_family: ActionFamilyId::LightMelee,
            separation_px: 36.0,
            row_spacing_px: 120.0,
        },
        steps: 240,
        deliver_period: 5,
        hold_equipment: false,
    },
    // The policy raises a guard only while it can attribute a telegraphed
    // hostile path to a purpose target (`combat_feasibility`'s guard
    // witness), which needs the threat readable inside the deciding player's
    // scan cadence. A driver-level scenario cannot re-pin the hostile every
    // tick the way a live per-tick harness could, because boundary
    // zero is the only thing it controls -- see [`GUARD_PROBE`]'s doc for the
    // measured evidence. So the guard here is raised by a live slot's own
    // held equipment, which reaches the resolver through the canonical input
    // stream rather than by mutating a runtime field.
    OnlineCombatPhaseScenario {
        id: "guard",
        route: OnlineCombatPhaseRoute::CanonicalInput,
        shape: OnlineCombatFixtureShape {
            home_family: ActionFamilyId::Guard,
            away_family: ActionFamilyId::LightMelee,
            separation_px: 30.0,
            row_spacing_px: 120.0,
        },
        steps: 240,
        deliver_period: 5,
        hold_equipment: true,
    },
    // A scrum: eight unarmed outfielders inside one 30px reach of each other
    // and of the ball. Unarmed has the shortest wind-up and the tightest
    // reach, so the policy's commits land contacts instead of missing.
    OnlineCombatPhaseScenario {
        id: "contact",
        route: OnlineCombatPhaseRoute::Policy,
        shape: OnlineCombatFixtureShape {
            home_family: ActionFamilyId::Unarmed,
            away_family: ActionFamilyId::Unarmed,
            separation_px: 24.0,
            row_spacing_px: 28.0,
        },
        steps: 480,
        deliver_period: 8,
        hold_equipment: false,
    },
    // Two ranged lines at shooting distance. A projectile lives 60 ticks, so
    // flight covers most of the run once the first shot is released.
    OnlineCombatPhaseScenario {
        id: "projectile_flight",
        route: OnlineCombatPhaseRoute::Policy,
        shape: OnlineCombatFixtureShape {
            home_family: ActionFamilyId::Ranged,
            away_family: ActionFamilyId::Ranged,
            separation_px: 200.0,
            row_spacing_px: 120.0,
        },
        steps: 240,
        deliver_period: 5,
        hold_equipment: false,
    },
    // Light melee displaces 18px on an unguarded hit, over the knockback
    // threshold, and interrupts for 18 ticks -- the widest forced window any
    // family produces.
    OnlineCombatPhaseScenario {
        id: "stagger",
        route: OnlineCombatPhaseRoute::Policy,
        shape: OnlineCombatFixtureShape {
            home_family: ActionFamilyId::LightMelee,
            away_family: ActionFamilyId::LightMelee,
            separation_px: 36.0,
            row_spacing_px: 120.0,
        },
        // 240 -> 480 under #488: a body with momentum closes and re-closes a
        // duel more slowly than one that could reverse on a frame, so a
        // budget calibrated against instant reversal stopped reaching a
        // knockback inside the window. The scenario's INTENT is unchanged --
        // it still measures corrections that resimulate a stagger tick, on
        // the same geometry and the same family -- only the number of steps
        // it needs to get there moved.
        steps: 480,
        deliver_period: 5,
        hold_equipment: false,
    },
    // A spill is the narrowest event of the seven: it needs an unguarded
    // contact on the carrier specifically, not on whoever is nearest. The
    // contact scrum produces those, and this runs it longer so a burst covers
    // several of them rather than the one.
    OnlineCombatPhaseScenario {
        id: "ball_spill",
        route: OnlineCombatPhaseRoute::Policy,
        shape: OnlineCombatFixtureShape {
            home_family: ActionFamilyId::Unarmed,
            away_family: ActionFamilyId::Unarmed,
            separation_px: 24.0,
            row_spacing_px: 28.0,
        },
        // 600 -> 820 steps and a 8 -> 12 delivery period, under #488's
        // carry-composition fix. The reason is the mechanic, not the harness:
        // a carrier who shields now keeps carry's handling penalty AND its
        // ball control while backing off, where before it silently got the
        // empty-handed profile. Spills are correspondingly rarer.
        //
        // The step budget alone could not fix it, and that is worth recording
        // so nobody retries it: measured, 780 and 820 steps produce too few
        // spills, and by 860 the match has reached full time and the peers
        // report `Completed` rather than `Active`. The window is closed. The
        // delivery period is the lever that actually applies -- pumping the
        // transport less often leaves more predicted ticks, so each
        // correction resimulates more of them, which is exactly what this
        // scenario asserts is covered. Every assertion is unchanged.
        steps: 820,
        deliver_period: 12,
        hold_equipment: false,
    },
    // The same scrum as `contact`, read one tick later: immunity is granted
    // by a landed contact and counts down to zero over `combat::IMMUNITY_TICKS`,
    // so the family that lands the most contacts also expires the most
    // immunities.
    OnlineCombatPhaseScenario {
        id: "immunity_expiry",
        route: OnlineCombatPhaseRoute::Policy,
        shape: OnlineCombatFixtureShape {
            home_family: ActionFamilyId::Unarmed,
            away_family: ActionFamilyId::Unarmed,
            separation_px: 24.0,
            row_spacing_px: 28.0,
        },
        steps: 480,
        deliver_period: 8,
        hold_equipment: false,
    },
];

/// # Panics
///
/// Panics if `id` is not one of [`PHASES`].
#[must_use]
pub fn scenario(id: &str) -> &'static OnlineCombatPhaseScenario {
    SCENARIOS
        .iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("unknown online combat phase: {id}"))
}

/// Every outfielder on a side carries the same family, so a scenario
/// exercises exactly one pair of them. Builds both the id-keyed pool
/// `gc_sim::r#match::new` wants and the plain slice `gc_sim::combat::new_state`
/// wants, from the one copy.
///
/// # Panics
///
/// Panics if the `nebula` fixture team is missing from [`gc_data::teams`] --
/// a content-table invariant, never expected to fail.
fn pool_for(
    shape: &OnlineCombatFixtureShape,
) -> (IndexMap<&'static str, PlayerData>, Vec<PlayerData>) {
    let home = teams::get("nebula").expect("fixture team nebula is always authored");
    let mut by_id = IndexMap::new();
    let mut list = Vec::with_capacity(gc_data::players::ALL.len());
    for player in gc_data::players::ALL {
        let mut copied = *player;
        if copied.loadout_id.is_some() {
            let family = if home.roster.contains(&player.id) {
                shape.home_family
            } else {
                shape.away_family
            };
            copied.loadout_id = Some(loadout_for(family));
        }
        by_id.insert(player.id, copied);
        list.push(copied);
    }
    (by_id, list)
}

/// Two facing lines centred on the pitch, one outfielder per row, with the
/// ball on the first away outfielder. Every scenario opens from the same
/// shape and differs only in the families carrying it, how far apart the
/// lines stand, and how tightly the rows are stacked -- a wide
/// `row_spacing_px` is four separate duels, a narrow one is a scrum where
/// every body is inside every reach.
fn pose(state: &mut match_snapshot::MatchState, shape: &OnlineCombatFixtureShape) {
    let spacing = shape.row_spacing_px;
    let top = FIELD.h / 2.0 - spacing * 1.5;
    for (order, &index) in HOME_OUTFIELD.iter().enumerate() {
        let player = &mut state.players[(index - 1) as usize];
        player.pos = Vec2::new(LINE_X, top + (order as f64) * spacing);
        player.facing = Vec2::new(1.0, 0.0);
        player.vel = Vec2::new(0.0, 0.0);
        player.run_vel = Vec2::new(0.0, 0.0);
    }
    for (order, &index) in AWAY_OUTFIELD.iter().enumerate() {
        let player = &mut state.players[(index - 1) as usize];
        player.pos = Vec2::new(LINE_X + shape.separation_px, top + (order as f64) * spacing);
        player.facing = Vec2::new(-1.0, 0.0);
        player.vel = Vec2::new(0.0, 0.0);
        player.run_vel = Vec2::new(0.0, 0.0);
    }
    let carrier = AWAY_OUTFIELD[0];
    state.owner = Some(carrier);
    state.ball = state.players[(carrier - 1) as usize].pos;
    state.ball_vel = Vec2::new(0.0, 0.0);
    // The kickoff hold refuses every equipment request, so a scenario that
    // spent it would spend its whole run waiting instead of fighting.
    state.kickoff_hold = 0.0;
}

/// Builds and captures the boundary-zero snapshot for one fixture shape.
///
/// # Panics
///
/// Panics if the `nebula`/`orion` fixture teams are missing from
/// [`gc_data::teams`] -- a content-table invariant, never expected to fail.
fn capture_boundary_zero(shape: &OnlineCombatFixtureShape, duration: Option<f64>) -> MatchSnapshot {
    let (by_id, list) = pool_for(shape);
    let home = teams::get("nebula").expect("fixture team nebula is always authored");
    let away = teams::get("orion").expect("fixture team orion is always authored");
    let ownership = r#match::ownership_for_teams(home, away, Some(&by_id));
    let mut state = r#match::new(NewMatchOptions {
        home,
        away,
        field: FIELD,
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(duration.unwrap_or(DURATION_SECONDS)),
        // The goal limit is not part of any scenario: #302 made
        // `r#match::NO_GOAL_LIMIT` the default, and a phase fixture must never
        // end early on a scoreline while a correction is still the thing
        // under test.
        max_goals: Some(NO_GOAL_LIMIT),
        seed: Some(SEED),
        players_by_id: Some(&by_id),
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership),
    });
    pose(&mut state, shape);
    let combat_state: CombatMatchState = combat::new_state(&mut state, Some(&list));
    match_snapshot::capture(&state, Some(&combat_state))
}

/// The shared, combat-active boundary zero for one phase. Every peer in a
/// session must be given the *same* snapshot: a differing one is a desync
/// fixture, not a correction fixture.
///
/// `duration` is match seconds; defaults to [`DURATION_SECONDS`].
///
/// # Panics
///
/// Panics if `id` is not one of [`PHASES`], or if the fixture teams are
/// missing (see [`capture_boundary_zero`]).
#[must_use]
pub fn boundary_zero(id: &str, duration: Option<f64>) -> MatchSnapshot {
    capture_boundary_zero(&scenario(id).shape, duration)
}

/// The live slot's input for one driver step. Driver index 1 is the host,
/// whose opening live slot is `home_1`; every other index gets a neutral
/// sample.
///
/// Only the `guard` scenario authors anything: it presses equipment once and
/// then holds it, which is a human raising a shield and keeping it up. The
/// press edge is re-sent periodically so an interrupted guard is raised again
/// rather than leaving the rest of the run neutral.
///
/// `step` is the zero-based driver step; `index` is the 1-based driver index.
///
/// # Panics
///
/// Panics if `id` is not one of [`PHASES`].
#[must_use]
pub fn live_sample(id: &str, step: i64, index: i64) -> InputSample {
    let scenario = scenario(id);
    if !scenario.hold_equipment || index != 1 {
        return input_frame::neutral_sample();
    }
    input_frame::new_sample(input_frame::InputSampleOptions {
        held: Some(input_frame::HELD_EQUIPMENT),
        edges: Some(if step % 30 == 0 {
            input_frame::EDGE_EQUIPMENT_PRESSED
        } else {
            0
        }),
        ..Default::default()
    })
    .expect("a held-equipment sample is always valid")
}

fn companion(snapshot: &MatchSnapshot) -> &CombatMatchState {
    snapshot
        .combat
        .as_ref()
        .expect("a combat phase scenario needs a combat-bearing snapshot")
}

fn any_phase(snapshot: &MatchSnapshot, phase: CombatPhase) -> bool {
    companion(snapshot).players.iter().any(|r| r.phase == phase)
}

fn any_event(events: &[CombatEvent], kind: CombatEventKind) -> bool {
    events.iter().any(|e| e.kind == kind)
}

/// Did input tick `tick` run through the named phase?
///
/// `before` is the snapshot at boundary `tick` -- the state the tick was
/// simulated *from* -- and `after` is the snapshot at boundary `tick + 1`.
/// `events` are the combat events that tick emitted. A caller asks this about
/// a tick a correction resimulated, so a true answer means the correction
/// carried the companion through that phase and landed on the state every
/// peer agrees on.
///
/// # Panics
///
/// Panics if `id` is not one of [`PHASES`], or if `before`/`after` are not
/// combat-bearing snapshots.
#[must_use]
pub fn observed(
    id: &str,
    before: &MatchSnapshot,
    after: &MatchSnapshot,
    events: &[CombatEvent],
) -> bool {
    match id {
        "windup" => any_phase(before, CombatPhase::Windup),
        "guard" => any_phase(before, CombatPhase::Guard),
        "contact" => any_event(events, CombatEventKind::Contact),
        // Already in flight when the tick began, so the tick advanced it. A
        // projectile that appears only in `after` was spawned by this tick
        // and has not flown a pixel yet, which is a spawn rather than flight.
        "projectile_flight" => !companion(before).projectiles.is_empty(),
        "stagger" => companion(before)
            .players
            .iter()
            .any(|r| r.forced_state.is_some() && r.forced_ticks > 0),
        "ball_spill" => any_event(events, CombatEventKind::BallSpill),
        "immunity_expiry" => {
            // The expiry itself, not merely a live immunity: the counter has
            // to reach zero inside the resimulated tick.
            let entering = &companion(before).players;
            let leaving = &companion(after).players;
            entering
                .iter()
                .zip(leaving.iter())
                .any(|(runtime, next)| runtime.immunity_ticks > 0 && next.immunity_ticks == 0)
        }
        other => panic!("unknown online combat phase: {other}"),
    }
}

// ---------------------------------------------------------------------------
// The guard probe
// ---------------------------------------------------------------------------
//
// The evidence behind the `guard` scenario's `CanonicalInput` route, kept as
// runnable code rather than as a claim in a pull request. Each geometry arms
// every home outfielder with `guard` and every away outfielder with a family
// that produces a *public* hostile path, then lets the policy play with no
// human input at all. A caller drives them exactly like a phase scenario and
// counts how often a correction resimulates a genuine `guard` tick.
//
// What it found, and the honest version of the claim: the policy does raise a
// guard online, but barely. Three of the four geometries produce none at all,
// and `vs_unarmed_scrum` produces exactly one commit in 240 steps -- which
// disappears entirely when the delivery cadence changes, because a bot fill
// authors from the state it currently predicts. One commit is not a scenario.
// The route decision is therefore about *rate*, not about possibility, and
// [`GUARD_POLICY_ROUTE_MINIMUM`] is where that judgement is written down.

/// One driver-level geometry the guard probe measures.
#[derive(Clone, Copy, Debug)]
pub struct OnlineGuardProbeGeometry {
    /// This geometry's stable id.
    pub id: &'static str,
    /// The opening fixture shape (`home_family` is always
    /// [`ActionFamilyId::Guard`]).
    pub shape: OnlineCombatFixtureShape,
    /// Driver steps the probe runs.
    pub steps: i64,
    /// Transport drains every Nth step.
    pub deliver_period: i64,
}

/// The four driver-level geometries the guard probe measures.
pub const GUARD_PROBE: [OnlineGuardProbeGeometry; 4] = [
    // The widest melee telegraph, one duel per row -- the geometry a live
    // per-tick harness guards in when it re-pins the hostile every tick.
    OnlineGuardProbeGeometry {
        id: "vs_light_melee_duels",
        shape: OnlineCombatFixtureShape {
            home_family: ActionFamilyId::Guard,
            away_family: ActionFamilyId::LightMelee,
            separation_px: 30.0,
            row_spacing_px: 120.0,
        },
        steps: 240,
        deliver_period: 5,
    },
    // The narrowest telegraph, but eight bodies inside one reach, so a
    // wind-up is nearly always in progress somewhere.
    OnlineGuardProbeGeometry {
        id: "vs_unarmed_scrum",
        shape: OnlineCombatFixtureShape {
            home_family: ActionFamilyId::Guard,
            away_family: ActionFamilyId::Unarmed,
            separation_px: 26.0,
            row_spacing_px: 28.0,
        },
        steps: 240,
        deliver_period: 5,
    },
    // The long public window: a latched release plus a 60-tick projectile
    // horizon is the threat most likely to still be readable on a slow
    // scanner's next decision tick.
    OnlineGuardProbeGeometry {
        id: "vs_ranged_lines",
        shape: OnlineCombatFixtureShape {
            home_family: ActionFamilyId::Guard,
            away_family: ActionFamilyId::Ranged,
            separation_px: 200.0,
            row_spacing_px: 120.0,
        },
        steps: 240,
        deliver_period: 5,
    },
    // The same long window aimed down one lane, so a projectile's path
    // crosses several guard bodies rather than one.
    OnlineGuardProbeGeometry {
        id: "vs_ranged_scrum",
        shape: OnlineCombatFixtureShape {
            home_family: ActionFamilyId::Guard,
            away_family: ActionFamilyId::Ranged,
            separation_px: 200.0,
            row_spacing_px: 28.0,
        },
        // 240 -> 480 for the same reason as the `stagger` scenario above:
        // with momentum, 240 steps was no longer enough for this geometry to
        // telegraph a single threat, and a probe that observes nothing proves
        // nothing. The assertion it feeds is unchanged.
        steps: 480,
        deliver_period: 5,
    },
];

/// Boundary zero for one guard probe geometry, built exactly like a phase
/// scenario's so the probe measures the policy rather than a different
/// fixture.
///
/// # Panics
///
/// Panics if `geometry` does not arm the home side with
/// [`ActionFamilyId::Guard`], or if the fixture teams are missing (see
/// [`capture_boundary_zero`]).
#[must_use]
pub fn guard_probe_boundary_zero(
    geometry: &OnlineGuardProbeGeometry,
    duration: Option<f64>,
) -> MatchSnapshot {
    assert!(
        geometry.shape.home_family == ActionFamilyId::Guard,
        "a guard probe geometry must arm the home side"
    );
    capture_boundary_zero(&geometry.shape, duration)
}
