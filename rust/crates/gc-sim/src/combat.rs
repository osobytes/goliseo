//! The combat tick pipeline: request gating, contact collection and
//! resolution, forced-state application, and encounter lifecycle
//! (commit/contact/terminal). Operates on [`crate::match_snapshot::MatchState`]
//! and [`crate::combat_snapshot::CombatMatchState`] directly — see
//! `match_snapshot`'s module doc for why this layer shares one canonical
//! pair of types instead of a narrow view.

use crate::combat_feasibility::CombatActionPhase as CombatPhase;
use crate::combat_intent;
use crate::combat_snapshot::{
    self, CombatContactResult, CombatEncounterTerminal, CombatEvent, CombatEventKind,
    CombatForcedState, CombatMatchState, CombatPlayerState, CombatProjectile, CombatRequestOutcome,
    CombatRequestRejectionReason,
};
use crate::fixed_clock;
use crate::match_snapshot::{self, MatchInput, MatchPlayer, MatchState};
use gc_core::vec2::Vec2;
use gc_data::action_families::{self, ActionFamilyData, ActionFamilyId};
use gc_data::loadouts;
use gc_data::players::PlayerData;
use indexmap::IndexMap;

/// Re-exported combat-rule constant (`combat_rules::MAX_DISABLE_TICKS`).
pub const MAX_DISABLE_TICKS: i64 = crate::combat_rules::MAX_DISABLE_TICKS;
/// Re-exported combat-rule constant (`combat_rules::IMMUNITY_TICKS`).
pub const IMMUNITY_TICKS: i64 = crate::combat_rules::IMMUNITY_TICKS;

const EPSILON: f64 = 1e-9;

/// The ranged family's projectile travel per tick, in px.
#[must_use]
pub fn projectile_step_px() -> f64 {
    action_families::get(ActionFamilyId::Ranged)
        .projectile_speed_px_per_second
        .expect("the ranged family defines a projectile speed")
        * fixed_clock::TICK_SECONDS
}

/// The closed terminal vocabulary has no `extended` member: an extended
/// contact is a landed contact that lengthened an existing forced state, so
/// `hit` is the only lawful terminal it can carry.
fn contact_terminal(result: CombatContactResult) -> CombatEncounterTerminal {
    result.terminal()
}

/// One collected contact this tick, awaiting resolution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatContact {
    /// The contacting action family.
    pub family_id: ActionFamilyId,
    /// The acting player's canonical index.
    pub source_index: i64,
    /// The targeted player's canonical index.
    pub target_index: i64,
    /// The acting encounter's source sequence.
    pub source_sequence: i64,
    /// The originating projectile's source sequence, for a ranged contact.
    pub projectile_sequence: Option<i64>,
    /// Whether the target was guarding when contact was made.
    pub guarded: bool,
    /// Whether the target was immune when contact was made.
    pub immune: bool,
    /// The source's position at the moment of contact.
    pub source_pos: Vec2,
}

fn find_player_data<'a>(by_id: &'a [PlayerData], id: &str) -> Option<&'a PlayerData> {
    by_id.iter().find(|p| p.id == id)
}

fn new_player_state(
    loadout_id: Option<String>,
    family_id: Option<ActionFamilyId>,
    intent: combat_intent::CombatIntentState,
) -> CombatPlayerState {
    CombatPlayerState {
        loadout_id,
        family_id,
        phase: CombatPhase::Ready,
        phase_ticks: 0,
        cooldown_ticks: 0,
        source_sequence: None,
        contacted: false,
        release_latched: false,
        control_held: false,
        projectile_spawned: false,
        forced_state: None,
        forced_ticks: 0,
        chain_ticks: 0,
        immunity_ticks: 0,
        intent,
    }
}

/// Build a fresh [`CombatMatchState`] for `state`'s fixture. `players_by_id`
/// overrides the default full player pool (used by fixtures/specs that
/// author custom loadout pairings); `None` uses `gc_data::players::ALL`.
///
/// # Panics
///
/// Panics if an authored player names an unknown loadout or family id.
pub fn new_state(state: &mut MatchState, players_by_id: Option<&[PlayerData]>) -> CombatMatchState {
    match_snapshot::mark_unsupported(
        state,
        "combat-active match snapshots require their CombatMatchState companion",
    );
    let by_id: &[PlayerData] = players_by_id.unwrap_or(gc_data::players::ALL);
    let mut runtimes = Vec::with_capacity(state.players.len());
    let mut player_ids = Vec::with_capacity(state.players.len());
    for match_player in &state.players {
        player_ids.push(match_player.id.clone());
        let mut loadout_id: Option<String> = None;
        let mut family_id: Option<ActionFamilyId> = None;
        if !match_player.is_keeper
            && let Some(player_data) = find_player_data(by_id, &match_player.id)
            && let Some(id) = player_data.loadout_id
        {
            let loadout =
                loadouts::get(id).unwrap_or_else(|| panic!("unknown combat loadout: {id}"));
            loadout_id = Some(id.to_string());
            family_id = Some(loadout.family_id);
        }
        runtimes.push(new_player_state(
            loadout_id,
            family_id,
            combat_intent::new_state(),
        ));
    }
    CombatMatchState {
        version: combat_snapshot::VERSION,
        tick: 0,
        player_ids,
        players: runtimes,
        projectiles: Vec::new(),
        events: Vec::new(),
        next_source_sequence: 1,
    }
}

fn suppress_soccer_actions(input: &mut MatchInput) {
    input.shoot = false;
    input.shoot_held = false;
    input.pass = false;
    input.pass_held = false;
    input.switch = false;
    input.dash = false;
    input.dodge = false;
    input.lob = false;
    input.sprint = false;
    input.jockey = false;
    // Deliberately `Some(false)`, not `None`: this is an explicit "no
    // strike" while soccer actions are suppressed, not an absence of
    // intent, so `aerial::strike_requested`'s jockey/dash fallback must not
    // fire.
    input.aerial_strike = Some(false);
    input.aerial_acrobatic = Some(false);
}

fn soccer_has_priority(input: &MatchInput) -> bool {
    input.shoot
        || input.shoot_held
        || input.pass
        || input.pass_held
        || input.switch
        || input.dash
        || input.dodge
        || input.jockey
}

fn player_has_aerial_commitment(player: &MatchPlayer) -> bool {
    player.aerial_timer > 0.0 || player.aerial_recovery > 0.0
}

fn player_has_ground_commitment(player: &MatchPlayer) -> bool {
    player.slide_timer > 0.0
        || player.tackle_timer > 0.0
        || player.jockey_timer > 0.0
        || player.dodge_timer > 0.0
        || player.windup_timer > 0.0
        || player.windup_shot.is_some()
}

fn commit_action(
    combat_state: &mut CombatMatchState,
    family_id: ActionFamilyId,
    player_index: usize,
    input: &MatchInput,
    position: Vec2,
) {
    let family = action_families::get(family_id);
    let sequence = combat_state.next_source_sequence;
    let runtime = &mut combat_state.players[player_index - 1];
    runtime.phase = CombatPhase::Windup;
    runtime.phase_ticks = family.windup_ticks;
    runtime.cooldown_ticks = family.cooldown_ticks;
    runtime.source_sequence = Some(sequence);
    runtime.contacted = false;
    runtime.release_latched = input.equipment_released;
    runtime.control_held = input.equipment_held;
    runtime.projectile_spawned = false;
    combat_state.next_source_sequence += 1;
    combat_state.events.push(CombatEvent {
        kind: CombatEventKind::Commit,
        tick: combat_state.tick,
        family_id: Some(family_id),
        source_index: Some(player_index as i64),
        target_index: None,
        source_sequence: Some(sequence),
        result: None,
        outcome: Some(CombatRequestOutcome::Accepted),
        reason: None,
        terminal: None,
        x: position.x,
        y: position.y,
        interruption_ticks: None,
        displacement_px: None,
    });
}

fn reject_request(
    combat_state: &mut CombatMatchState,
    player_index: usize,
    runtime_family_id: Option<ActionFamilyId>,
    position: Vec2,
    reason: CombatRequestRejectionReason,
) {
    combat_state.events.push(CombatEvent {
        kind: CombatEventKind::RequestRejected,
        tick: combat_state.tick,
        family_id: runtime_family_id,
        source_index: Some(player_index as i64),
        target_index: None,
        source_sequence: None,
        result: None,
        outcome: Some(CombatRequestOutcome::Rejected),
        reason: Some(reason),
        terminal: None,
        x: position.x,
        y: position.y,
        interruption_ticks: None,
        displacement_px: None,
    });
}

#[allow(clippy::too_many_arguments)]
fn emit_terminal(
    combat_state: &mut CombatMatchState,
    terminal: CombatEncounterTerminal,
    tick: i64,
    family_id: Option<ActionFamilyId>,
    source_index: i64,
    source_sequence: i64,
    position: Vec2,
) {
    combat_state.events.push(CombatEvent {
        kind: terminal_event_kind(terminal),
        tick,
        family_id,
        source_index: Some(source_index),
        target_index: None,
        source_sequence: Some(source_sequence),
        result: None,
        outcome: None,
        reason: None,
        terminal: Some(terminal),
        x: position.x,
        y: position.y,
        interruption_ticks: None,
        displacement_px: None,
    });
}

fn terminal_event_kind(terminal: CombatEncounterTerminal) -> CombatEventKind {
    match terminal {
        CombatEncounterTerminal::Miss => CombatEventKind::Miss,
        CombatEncounterTerminal::Interrupted => CombatEventKind::Interrupted,
        CombatEncounterTerminal::Cancelled => CombatEventKind::Cancelled,
        CombatEncounterTerminal::MatchTerminated => CombatEventKind::MatchTerminated,
        CombatEncounterTerminal::Expire
        | CombatEncounterTerminal::Guarded
        | CombatEncounterTerminal::Immune
        | CombatEncounterTerminal::Superseded
        | CombatEncounterTerminal::Hit => {
            panic!("emit_terminal is only used for the four terminal-only event kinds")
        }
    }
}

fn is_committed(runtime: &CombatPlayerState) -> bool {
    runtime.phase != CombatPhase::Ready
}

/// True while the action itself still owes its sequence a terminal. A melee
/// contact and a spawned projectile both hand that debt away: the contact
/// row and the projectile's own contact/expiry row are the terminals, so
/// cancelling the action afterwards must not write a second one.
fn owes_terminal(runtime: &CombatPlayerState) -> bool {
    if runtime.source_sequence.is_none() {
        return false;
    }
    match runtime.family_id {
        Some(ActionFamilyId::Ranged) => !runtime.projectile_spawned,
        Some(ActionFamilyId::Guard) => true,
        _ => !runtime.contacted,
    }
}

/// Precedence is the gate's own shape, then the closed vocabulary's order.
/// The committed/forced branch also suppresses soccer input, so it owns the
/// refusal instead of losing it to a same-tick soccer press.
fn request_rejection(
    state: &MatchState,
    match_player: &MatchPlayer,
    runtime: &CombatPlayerState,
    input: &MatchInput,
    ineligible: bool,
) -> Option<CombatRequestRejectionReason> {
    use CombatRequestRejectionReason as R;
    if match_player.is_keeper || runtime.family_id.is_none() {
        return Some(R::ProtectedKeeperOrNoLoadout);
    }
    if runtime.forced_ticks > 0 {
        return Some(R::ForcedState);
    }
    if is_committed(runtime) {
        return Some(R::AlreadyCommitted);
    }
    if state.kickoff_hold != 0.0 {
        return Some(R::KickoffHold);
    }
    if soccer_has_priority(input) || player_has_ground_commitment(match_player) {
        return Some(R::SoccerCommitment);
    }
    if ineligible || player_has_aerial_commitment(match_player) {
        return Some(R::AerialStateOrRecovery);
    }
    if runtime.cooldown_ticks != 0 {
        return Some(R::Cooldown);
    }
    None
}

/// Gate this tick's inputs against combat eligibility, and mutate
/// `combat_state`: a legal equipment press either commits a fresh action or
/// is rejected with a typed reason; suppressed soccer input is written back
/// into the returned copy so the soccer step never sees it.
///
/// # Panics
///
/// Panics if `combat_state`'s player identity/count does not match
/// `state`'s fixture.
pub fn prepare_inputs(
    state: &mut MatchState,
    combat_state: &mut CombatMatchState,
    inputs: &IndexMap<i64, MatchInput>,
    equipment_ineligible: Option<&IndexMap<i64, bool>>,
) -> IndexMap<i64, MatchInput> {
    assert_eq!(
        combat_state.players.len(),
        state.players.len(),
        "combat player state does not match fixture"
    );
    assert_eq!(
        combat_state.player_ids.len(),
        state.players.len(),
        "combat player identity does not match fixture"
    );
    for (index, player) in state.players.iter().enumerate() {
        assert_eq!(
            combat_state.player_ids[index], player.id,
            "combat player identity does not match fixture"
        );
    }
    sanitize_forced_players(state, combat_state);
    clear_events(combat_state);

    let mut prepared: IndexMap<i64, MatchInput> = IndexMap::new();
    for (&player_index, input) in inputs {
        prepared.insert(player_index, *input);
    }

    for player_index in 1..=state.players.len() {
        let Some(input) = prepared.get_mut(&(player_index as i64)) else {
            continue;
        };
        combat_state.players[player_index - 1].control_held = input.equipment_held;

        let phase = combat_state.players[player_index - 1].phase;
        let family_id = combat_state.players[player_index - 1].family_id;
        if phase == CombatPhase::Windup {
            if matches!(
                family_id,
                Some(ActionFamilyId::Guard) | Some(ActionFamilyId::Ranged)
            ) && input.equipment_released
            {
                combat_state.players[player_index - 1].release_latched = true;
            }
        } else if phase == CombatPhase::Aim && family_id == Some(ActionFamilyId::Ranged) {
            if input.equipment_released {
                let runtime = &mut combat_state.players[player_index - 1];
                runtime.release_latched = true;
                runtime.phase = CombatPhase::Active;
                runtime.phase_ticks = 1;
            }
        } else if phase == CombatPhase::Guard
            && family_id == Some(ActionFamilyId::Guard)
            && (input.equipment_released || !input.equipment_held)
        {
            let recovery_ticks = action_families::get(ActionFamilyId::Guard).recovery_ticks;
            let runtime = &mut combat_state.players[player_index - 1];
            runtime.phase = CombatPhase::Recovery;
            runtime.phase_ticks = recovery_ticks;
        }

        let ineligible = equipment_ineligible
            .and_then(|m| m.get(&(player_index as i64)))
            .copied()
            .unwrap_or(false);
        let runtime_now = combat_state.players[player_index - 1].clone();
        if is_committed(&runtime_now) || runtime_now.forced_ticks > 0 {
            // The press edge dies here, so it is recorded here. Without a
            // press there is no request at all, and no row is owed.
            if input.equipment_pressed {
                let reason = request_rejection(
                    state,
                    &state.players[player_index - 1],
                    &runtime_now,
                    input,
                    ineligible,
                )
                .expect("a committed/forced player always has a rejection reason");
                let pos = state.players[player_index - 1].pos;
                reject_request(
                    combat_state,
                    player_index,
                    runtime_now.family_id,
                    pos,
                    reason,
                );
            }
            suppress_soccer_actions(input);
            input.equipment_pressed = false;
        } else if input.equipment_pressed {
            if let Some(reason) = request_rejection(
                state,
                &state.players[player_index - 1],
                &runtime_now,
                input,
                ineligible,
            ) {
                let pos = state.players[player_index - 1].pos;
                reject_request(
                    combat_state,
                    player_index,
                    runtime_now.family_id,
                    pos,
                    reason,
                );
            } else {
                let family_id = runtime_now
                    .family_id
                    .expect("request_rejection admits only a loadout-bearing player");
                let pos = state.players[player_index - 1].pos;
                commit_action(combat_state, family_id, player_index, input, pos);
                let match_player = &mut state.players[player_index - 1];
                match_player.charge = 0.0;
                match_player.pass_charge = 0.0;
                match_player.pass_target = None;
                suppress_soccer_actions(input);
                input.equipment_pressed = false;
            }
        }
    }

    prepared
}

/// Discard this tick's combat events.
pub fn clear_events(state: &mut CombatMatchState) {
    state.events.clear();
}

/// Whether `player_index` is currently blocked from soccer actions by
/// combat (forced or mid-action).
#[must_use]
pub fn blocks_actions(state: Option<&CombatMatchState>, player_index: i64) -> bool {
    let Some(state) = state else {
        return false;
    };
    let Some(runtime) = state.players.get(player_index as usize - 1) else {
        return false;
    };
    runtime.forced_ticks > 0 || is_committed(runtime)
}

/// This player's combat-imposed locomotion multiplier: 0 forced, the
/// family's catalogued multiplier mid-action, 1 otherwise.
#[must_use]
pub fn movement_multiplier(state: Option<&CombatMatchState>, player_index: i64) -> f64 {
    let Some(state) = state else {
        return 1.0;
    };
    let Some(runtime) = state.players.get(player_index as usize - 1) else {
        return 1.0;
    };
    if runtime.forced_ticks > 0 {
        return 0.0;
    }
    if is_committed(runtime)
        && let Some(family_id) = runtime.family_id
    {
        return action_families::get(family_id).movement_multiplier;
    }
    1.0
}

fn dot(a: Vec2, b: Vec2) -> f64 {
    a.x * b.x + a.y * b.y
}

fn unit_or(direction: Vec2, fallback: Vec2) -> Vec2 {
    if direction.length() > EPSILON {
        direction.normalized()
    } else {
        fallback
    }
}

fn team_facing_fallback(team: match_snapshot::Team) -> Vec2 {
    Vec2::new(
        if team == match_snapshot::Team::Home {
            1.0
        } else {
            -1.0
        },
        0.0,
    )
}

fn melee_geometry(
    source: &MatchPlayer,
    target: &MatchPlayer,
    family: &ActionFamilyData,
) -> (bool, f64) {
    let offset = target.pos.sub(source.pos);
    let distance = offset.length();
    if distance
        > family
            .reach_px
            .expect("melee_geometry is only called with a melee family")
            + target.radius
    {
        return (false, 0.0);
    }
    let facing = unit_or(source.facing, team_facing_fallback(source.team));
    let projection = dot(offset, facing);
    if projection < 0.0 {
        return (false, projection);
    }
    if distance <= EPSILON {
        return (true, projection);
    }
    let half_arc = (family.front_arc_degrees / 2.0).to_radians();
    (
        dot(offset.scale(1.0 / distance), facing) + EPSILON >= half_arc.cos(),
        projection,
    )
}

fn target_guarding(target: &MatchPlayer, source_pos: Vec2) -> bool {
    let offset = source_pos.sub(target.pos);
    let distance = offset.length();
    if distance <= EPSILON {
        return true;
    }
    let facing = unit_or(target.facing, team_facing_fallback(target.team));
    let guard_arc =
        (action_families::get(ActionFamilyId::Guard).front_arc_degrees / 2.0).to_radians();
    dot(offset.scale(1.0 / distance), facing) + EPSILON >= guard_arc.cos()
}

fn select_melee_target(
    state: &MatchState,
    combat_state: &CombatMatchState,
    source_index: usize,
    family: &ActionFamilyData,
) -> Option<usize> {
    let source = &state.players[source_index - 1];
    let mut best_index: Option<usize> = None;
    let mut best_projection: Option<f64> = None;
    for (zero_index, target) in state.players.iter().enumerate() {
        let target_index = zero_index + 1;
        let target_runtime = &combat_state.players[zero_index];
        if target.team == source.team || target.is_keeper || target.dodge_timer > 0.0 {
            continue;
        }
        let _ = target_runtime;
        let (legal, projection) = melee_geometry(source, target, family);
        if legal
            && (best_projection.is_none()
                || projection < best_projection.unwrap() - EPSILON
                || ((projection - best_projection.unwrap()).abs() <= EPSILON
                    && target_index < best_index.unwrap()))
        {
            best_index = Some(target_index);
            best_projection = Some(projection);
        }
    }
    best_index
}

fn segment_circle_time(start_pos: Vec2, end_pos: Vec2, center: Vec2, radius: f64) -> Option<f64> {
    let segment = end_pos.sub(start_pos);
    let to_center = center.sub(start_pos);
    let length_sq = dot(segment, segment);
    if length_sq <= EPSILON {
        return if start_pos.dist(center) <= radius {
            Some(0.0)
        } else {
            None
        };
    }
    let projection = (dot(to_center, segment) / length_sq).clamp(0.0, 1.0);
    let closest = start_pos.add(segment.scale(projection));
    if closest.dist(center) <= radius + EPSILON {
        Some(projection)
    } else {
        None
    }
}

fn select_projectile_target(
    state: &MatchState,
    combat_state: &CombatMatchState,
    projectile: &CombatProjectile,
    end_pos: Vec2,
) -> Option<usize> {
    let source = &state.players[projectile.source_index as usize - 1];
    let mut best_index: Option<usize> = None;
    let mut best_time: Option<f64> = None;
    for (zero_index, target) in state.players.iter().enumerate() {
        let target_index = zero_index + 1;
        let _runtime = &combat_state.players[zero_index];
        if target.team == source.team || target.is_keeper || target.dodge_timer > 0.0 {
            continue;
        }
        let Some(contact_time) =
            segment_circle_time(projectile.pos, end_pos, target.pos, target.radius)
        else {
            continue;
        };
        if best_time.is_none()
            || contact_time < best_time.unwrap() - EPSILON
            || ((contact_time - best_time.unwrap()).abs() <= EPSILON
                && target_index < best_index.unwrap())
        {
            best_index = Some(target_index);
            best_time = Some(contact_time);
        }
    }
    best_index
}

fn spawn_projectile(
    state: &MatchState,
    combat_state: &mut CombatMatchState,
    source_index: usize,
    source_sequence: i64,
) -> CombatProjectile {
    let source = &state.players[source_index - 1];
    let family = action_families::get(ActionFamilyId::Ranged);
    let projectile = CombatProjectile {
        family_id: ActionFamilyId::Ranged,
        source_index: source_index as i64,
        source_sequence,
        pos: source.pos,
        dir: unit_or(source.facing, team_facing_fallback(source.team)),
        remaining_ticks: family
            .projectile_lifetime_ticks
            .expect("the ranged family defines a projectile lifetime"),
    };
    combat_state.events.push(CombatEvent {
        kind: CombatEventKind::ProjectileSpawn,
        tick: combat_state.tick,
        family_id: Some(ActionFamilyId::Ranged),
        source_index: Some(source_index as i64),
        target_index: None,
        source_sequence: Some(source_sequence),
        result: None,
        outcome: None,
        reason: None,
        terminal: None,
        x: projectile.pos.x,
        y: projectile.pos.y,
        interruption_ticks: None,
        displacement_px: None,
    });
    projectile
}

#[allow(clippy::too_many_arguments)]
fn make_contact(
    combat_state: &CombatMatchState,
    family_id: ActionFamilyId,
    source_index: usize,
    target_index: usize,
    source_sequence: i64,
    projectile_sequence: Option<i64>,
    source_pos: Vec2,
    state: &MatchState,
) -> CombatContact {
    let target_runtime = &combat_state.players[target_index - 1];
    let guarded = target_runtime.phase == CombatPhase::Guard
        && target_guarding(&state.players[target_index - 1], source_pos);
    CombatContact {
        family_id,
        source_index: source_index as i64,
        target_index: target_index as i64,
        source_sequence,
        projectile_sequence,
        guarded,
        immune: target_runtime.immunity_ticks > 0,
        source_pos,
    }
}

/// Collect this tick's contacts: melee reach checks against active
/// windows, in-flight projectile advancement and target checks, and
/// projectile spawn/expiry. Mutates `combat_state.projectiles` and appends
/// spawn/expiry events; returns contacts sorted into the deterministic
/// `(source_sequence, source_index, target_index)` resolution order.
pub fn collect_contacts(
    state: &MatchState,
    combat_state: &mut CombatMatchState,
) -> Vec<CombatContact> {
    let mut contacts = Vec::new();

    for source_index in 1..=combat_state.players.len() {
        let runtime = &combat_state.players[source_index - 1];
        if runtime.phase == CombatPhase::Active
            && runtime.family_id == Some(ActionFamilyId::Ranged)
            && !runtime.projectile_spawned
        {
            let sequence = runtime
                .source_sequence
                .expect("active ranged action needs a sequence");
            let projectile = spawn_projectile(state, combat_state, source_index, sequence);
            combat_state.projectiles.push(projectile);
            combat_state.players[source_index - 1].projectile_spawned = true;
        } else if runtime.phase == CombatPhase::Active
            && matches!(runtime.family_id, Some(id) if id != ActionFamilyId::Guard && id != ActionFamilyId::Ranged)
            && !runtime.contacted
        {
            let family_id = runtime.family_id.expect("matched Some above");
            let family = action_families::get(family_id);
            if let Some(target_index) =
                select_melee_target(state, combat_state, source_index, family)
            {
                let sequence = combat_state.players[source_index - 1]
                    .source_sequence
                    .expect("active melee action needs a sequence");
                combat_state.players[source_index - 1].contacted = true;
                contacts.push(make_contact(
                    combat_state,
                    family_id,
                    source_index,
                    target_index,
                    sequence,
                    None,
                    state.players[source_index - 1].pos,
                    state,
                ));
            }
        }
    }

    let mut retained = Vec::new();
    let step = projectile_step_px();
    for projectile in combat_state.projectiles.clone() {
        let start_pos = projectile.pos;
        let end_pos = start_pos.add(projectile.dir.scale(step));
        let target_index = select_projectile_target(state, combat_state, &projectile, end_pos);
        let mut projectile = projectile;
        projectile.remaining_ticks -= 1;
        if let Some(target_index) = target_index {
            contacts.push(make_contact(
                combat_state,
                projectile.family_id,
                projectile.source_index as usize,
                target_index,
                projectile.source_sequence,
                Some(projectile.source_sequence),
                start_pos,
                state,
            ));
        } else {
            projectile.pos = end_pos;
            let in_field = end_pos.x >= 0.0
                && end_pos.x <= state.field.w
                && end_pos.y >= 0.0
                && end_pos.y <= state.field.h;
            if projectile.remaining_ticks > 0 && in_field {
                retained.push(projectile);
            } else {
                combat_state.events.push(CombatEvent {
                    kind: CombatEventKind::ProjectileExpire,
                    tick: combat_state.tick,
                    family_id: Some(projectile.family_id),
                    source_index: Some(projectile.source_index),
                    target_index: None,
                    source_sequence: Some(projectile.source_sequence),
                    result: None,
                    outcome: None,
                    reason: None,
                    terminal: Some(CombatEncounterTerminal::Expire),
                    x: end_pos.x,
                    y: end_pos.y,
                    interruption_ticks: None,
                    displacement_px: None,
                });
            }
        }
    }
    combat_state.projectiles = retained;

    contacts.sort_by(|left, right| {
        left.source_sequence
            .cmp(&right.source_sequence)
            .then_with(|| left.source_index.cmp(&right.source_index))
            .then_with(|| left.target_index.cmp(&right.target_index))
    });
    contacts
}

fn contact_dominates(left: &CombatContact, right: &CombatContact) -> bool {
    let left_outcome = action_families::get(left.family_id)
        .unguarded_outcome
        .expect("contact families always define an unguarded outcome");
    let right_outcome = action_families::get(right.family_id)
        .unguarded_outcome
        .expect("contact families always define an unguarded outcome");
    if left_outcome.interruption_ticks != right_outcome.interruption_ticks {
        return left_outcome.interruption_ticks > right_outcome.interruption_ticks;
    }
    if left_outcome.displacement_px != right_outcome.displacement_px {
        return left_outcome.displacement_px > right_outcome.displacement_px;
    }
    if left.source_index != right.source_index {
        return left.source_index < right.source_index;
    }
    left.source_sequence < right.source_sequence
}

fn clamp_player(state: &MatchState, player: &MatchPlayer, position: Vec2) -> Vec2 {
    Vec2::new(
        position
            .x
            .clamp(player.radius, state.field.w - player.radius),
        position
            .y
            .clamp(player.radius, state.field.h - player.radius),
    )
}

fn cancel_action(runtime: &mut CombatPlayerState) {
    runtime.phase = CombatPhase::Ready;
    runtime.phase_ticks = 0;
    runtime.source_sequence = None;
    runtime.contacted = false;
    runtime.release_latched = false;
    runtime.projectile_spawned = false;
}

fn cancel_soccer_commitments(player: &mut MatchPlayer) {
    player.slide_timer = 0.0;
    player.slide_vel = 0.0;
    player.tackle_timer = 0.0;
    player.jockey_timer = 0.0;
    player.dodge_timer = 0.0;
    player.aerial_timer = 0.0;
    player.aerial_recovery = 0.0;
    player.aerial_style = None;
    player.aerial_outcome = None;
    player.aerial_jump = 0.0;
    player.receive_timer = 0.0;
    player.windup_timer = 0.0;
    player.windup_shot = None;
    player.charge = 0.0;
    player.pass_charge = 0.0;
    player.pass_target = None;
    player.sprinting = false;
    player.run_vel = Vec2::new(0.0, 0.0);
}

/// Cancel every soccer commitment for a player currently in a forced
/// (stagger/knockback) state. Match calls this before assembling inputs so
/// a forced player can never carry a stale soccer action into the tick.
pub fn sanitize_forced_players(state: &mut MatchState, combat_state: &CombatMatchState) {
    for (index, runtime) in combat_state.players.iter().enumerate() {
        if runtime.forced_ticks > 0 {
            cancel_soccer_commitments(&mut state.players[index]);
        }
    }
}

/// Every collected contact emits exactly one row, and a melee action
/// contacts at most once while a projectile is consumed by its contact.
/// The contact row is therefore its sequence's terminal.
fn emit_contact(
    state: &MatchState,
    combat_state: &mut CombatMatchState,
    contact: &CombatContact,
    result: CombatContactResult,
) {
    let target = &state.players[contact.target_index as usize - 1];
    let outcome = action_families::get(contact.family_id)
        .unguarded_outcome
        .expect("contact families always define an unguarded outcome");
    combat_state.events.push(CombatEvent {
        kind: CombatEventKind::Contact,
        tick: combat_state.tick,
        family_id: Some(contact.family_id),
        source_index: Some(contact.source_index),
        target_index: Some(contact.target_index),
        source_sequence: Some(contact.source_sequence),
        result: Some(result),
        outcome: None,
        reason: None,
        terminal: Some(contact_terminal(result)),
        x: target.pos.x,
        y: target.pos.y,
        interruption_ticks: Some(outcome.interruption_ticks),
        displacement_px: Some(outcome.displacement_px),
    });
}

fn apply_guard_recoil(
    state: &mut MatchState,
    combat_state: &mut CombatMatchState,
    contact: &CombatContact,
) {
    let family = action_families::get(contact.family_id);
    let recoil = family.guarded_recoil_px.min(6.0);
    if recoil <= 0.0 {
        return;
    }
    let target_index = contact.target_index as usize;
    let target = &state.players[target_index - 1];
    let away = target.pos.sub(contact.source_pos);
    let direction = unit_or(away, target.facing);
    let new_pos = clamp_player(state, target, target.pos.add(direction.scale(recoil)));
    state.players[target_index - 1].pos = new_pos;
    combat_state.events.push(CombatEvent {
        kind: CombatEventKind::GuardRecoil,
        tick: combat_state.tick,
        family_id: Some(contact.family_id),
        source_index: Some(contact.source_index),
        target_index: Some(contact.target_index),
        source_sequence: Some(contact.source_sequence),
        result: Some(CombatContactResult::Guarded),
        outcome: None,
        reason: None,
        terminal: None, // The guarded contact row already closed the sequence.
        x: new_pos.x,
        y: new_pos.y,
        interruption_ticks: Some(0),
        displacement_px: Some(recoil),
    });
}

fn apply_unguarded_outcome(
    state: &mut MatchState,
    combat_state: &mut CombatMatchState,
    contact: &CombatContact,
) {
    let target_index = contact.target_index as usize;
    let outcome = action_families::get(contact.family_id)
        .unguarded_outcome
        .expect("contact families always define an unguarded outcome");

    let runtime = &mut combat_state.players[target_index - 1];
    if runtime.forced_ticks > 0 {
        runtime.forced_ticks = runtime
            .chain_ticks
            .min(runtime.forced_ticks.max(outcome.interruption_ticks));
        emit_contact(state, combat_state, contact, CombatContactResult::Extended);
        return;
    }

    let runtime = &mut combat_state.players[target_index - 1];
    runtime.forced_ticks = MAX_DISABLE_TICKS.min(outcome.interruption_ticks);
    runtime.chain_ticks = MAX_DISABLE_TICKS;
    runtime.forced_state = Some(
        if outcome.displacement_px >= crate::combat_rules::KNOCKBACK_THRESHOLD_PX {
            CombatForcedState::Knockback
        } else {
            CombatForcedState::Stagger
        },
    );
    let owed = owes_terminal(runtime);
    let forced_family_id = runtime.family_id;
    let forced_source_sequence = runtime.source_sequence;
    if owed {
        emit_terminal(
            combat_state,
            CombatEncounterTerminal::Interrupted,
            combat_state.tick,
            forced_family_id,
            contact.target_index,
            forced_source_sequence.expect("owes_terminal implies a source sequence"),
            state.players[target_index - 1].pos,
        );
    }
    cancel_action(&mut combat_state.players[target_index - 1]);
    cancel_soccer_commitments(&mut state.players[target_index - 1]);

    let target = &state.players[target_index - 1];
    let away = target.pos.sub(contact.source_pos);
    let direction = unit_or(away, target.facing);
    let new_pos = clamp_player(
        state,
        target,
        target.pos.add(direction.scale(outcome.displacement_px)),
    );
    state.players[target_index - 1].pos = new_pos;

    emit_contact(state, combat_state, contact, CombatContactResult::Hit);
    let forced_ticks_now = combat_state.players[target_index - 1].forced_ticks;
    combat_state.events.push(CombatEvent {
        kind: CombatEventKind::Forced,
        tick: combat_state.tick,
        family_id: Some(contact.family_id),
        source_index: Some(contact.source_index),
        target_index: Some(contact.target_index),
        source_sequence: Some(contact.source_sequence),
        result: Some(CombatContactResult::Hit),
        outcome: None,
        reason: None,
        terminal: None, // Consequence of the hit row, not a second terminal.
        x: new_pos.x,
        y: new_pos.y,
        interruption_ticks: Some(forced_ticks_now),
        displacement_px: Some(outcome.displacement_px),
    });

    if outcome.ball_spill && state.owner == Some(contact.target_index) {
        state.owner = None;
        state.pickup_cd = state.pickup_cd.max(fixed_clock::TICK_SECONDS);
        combat_state.events.push(CombatEvent {
            kind: CombatEventKind::BallSpill,
            tick: combat_state.tick,
            family_id: Some(contact.family_id),
            source_index: Some(contact.source_index),
            target_index: Some(contact.target_index),
            source_sequence: Some(contact.source_sequence),
            result: Some(CombatContactResult::Hit),
            outcome: None,
            reason: None,
            terminal: None, // Consequence of the hit row, not a second terminal.
            x: state.ball.x,
            y: state.ball.y,
            interruption_ticks: None,
            displacement_px: None,
        });
    }
}

/// Resolve this tick's collected contacts: guarded/immune/superseded rows
/// first, then apply exactly one dominant outcome per target.
pub fn resolve_contacts(
    state: &mut MatchState,
    combat_state: &mut CombatMatchState,
    contacts: &[CombatContact],
) {
    let mut dominant_by_target: IndexMap<i64, CombatContact> = IndexMap::new();
    for contact in contacts {
        match dominant_by_target.get(&contact.target_index) {
            Some(current) if !contact_dominates(contact, current) => {}
            _ => {
                dominant_by_target.insert(contact.target_index, *contact);
            }
        }
    }

    for contact in contacts {
        if contact.guarded {
            emit_contact(state, combat_state, contact, CombatContactResult::Guarded);
        } else if contact.immune {
            emit_contact(state, combat_state, contact, CombatContactResult::Immune);
        } else if dominant_by_target.get(&contact.target_index) != Some(contact) {
            emit_contact(
                state,
                combat_state,
                contact,
                CombatContactResult::Superseded,
            );
        }
    }

    for target_index in 1..=state.players.len() as i64 {
        let Some(dominant) = dominant_by_target.get(&target_index).copied() else {
            continue;
        };
        if dominant.guarded {
            apply_guard_recoil(state, combat_state, &dominant);
        } else if !dominant.immune {
            apply_unguarded_outcome(state, combat_state, &dominant);
        }
    }
}

fn finish_action_tick(
    state: &MatchState,
    combat_state: &mut CombatMatchState,
    player_index: usize,
) {
    let runtime = &mut combat_state.players[player_index - 1];
    match runtime.phase {
        CombatPhase::Windup => {
            runtime.phase_ticks -= 1;
            if runtime.phase_ticks == 0 {
                if runtime.family_id == Some(ActionFamilyId::Guard) {
                    if runtime.release_latched || !runtime.control_held {
                        runtime.phase = CombatPhase::Recovery;
                        runtime.phase_ticks =
                            action_families::get(ActionFamilyId::Guard).recovery_ticks;
                    } else {
                        runtime.phase = CombatPhase::Guard;
                    }
                } else if runtime.family_id == Some(ActionFamilyId::Ranged) {
                    if runtime.release_latched {
                        runtime.phase = CombatPhase::Active;
                        runtime.phase_ticks = 1;
                    } else {
                        runtime.phase = CombatPhase::Aim;
                    }
                } else {
                    let family =
                        action_families::get(runtime.family_id.expect("windup implies a family"));
                    runtime.phase = CombatPhase::Active;
                    runtime.phase_ticks = family
                        .active_ticks
                        .expect("melee/unarmed families define active_ticks");
                }
            }
        }
        CombatPhase::Active => {
            runtime.phase_ticks -= 1;
            if runtime.phase_ticks == 0 {
                let family =
                    action_families::get(runtime.family_id.expect("active implies a family"));
                runtime.phase = CombatPhase::Recovery;
                runtime.phase_ticks = family.recovery_ticks;
            }
        }
        CombatPhase::Recovery => {
            runtime.phase_ticks -= 1;
            if runtime.phase_ticks == 0 {
                if owes_terminal(runtime) {
                    let family_id = runtime.family_id.expect("recovery implies a family");
                    let missed = action_families::get(family_id).contact_kind
                        == gc_data::action_families::ActionContactKind::Melee;
                    let source_sequence = runtime
                        .source_sequence
                        .expect("owes_terminal implies a sequence");
                    emit_terminal(
                        combat_state,
                        if missed {
                            CombatEncounterTerminal::Miss
                        } else {
                            CombatEncounterTerminal::Cancelled
                        },
                        combat_state.tick,
                        Some(family_id),
                        player_index as i64,
                        source_sequence,
                        state.players[player_index - 1].pos,
                    );
                }
                cancel_action(&mut combat_state.players[player_index - 1]);
            }
        }
        _ => {}
    }
}

/// Advance every combat runtime by one tick: cooldowns, forced-state decay
/// (immunity engages the instant a forced state clears), and action-phase
/// advancement. Advances `combat_state.tick`.
pub fn finish_tick(state: &MatchState, combat_state: &mut CombatMatchState) {
    for player_index in 1..=combat_state.players.len() {
        let runtime = &mut combat_state.players[player_index - 1];
        if runtime.cooldown_ticks > 0 {
            runtime.cooldown_ticks -= 1;
        }
        if runtime.forced_ticks > 0 {
            runtime.forced_ticks -= 1;
            runtime.chain_ticks = 0.max(runtime.chain_ticks - 1);
            if runtime.forced_ticks == 0 {
                runtime.forced_state = None;
                runtime.chain_ticks = 0;
                runtime.immunity_ticks = IMMUNITY_TICKS;
            }
        } else if runtime.immunity_ticks > 0 {
            runtime.immunity_ticks -= 1;
        }
        finish_action_tick(state, combat_state, player_index);
    }
    combat_state.tick += 1;
}

fn close_open_sequences(
    state: &MatchState,
    combat_state: &mut CombatMatchState,
    terminal: CombatEncounterTerminal,
    tick: i64,
) {
    for player_index in 1..=combat_state.players.len() {
        let runtime = &combat_state.players[player_index - 1];
        if owes_terminal(runtime) {
            emit_terminal(
                combat_state,
                terminal,
                tick,
                runtime.family_id,
                player_index as i64,
                runtime
                    .source_sequence
                    .expect("owes_terminal implies a sequence"),
                state.players[player_index - 1].pos,
            );
        }
    }
    for projectile in combat_state.projectiles.clone() {
        emit_terminal(
            combat_state,
            terminal,
            tick,
            Some(projectile.family_id),
            projectile.source_index,
            projectile.source_sequence,
            projectile.pos,
        );
    }
}

/// Full time discards every open action and in-flight projectile. Each one
/// still owes its sequence a terminal, and `match_terminated` is the only
/// one with no forward attribution window.
pub fn terminate_open_sequences(state: &MatchState, combat_state: &mut CombatMatchState) {
    close_open_sequences(
        state,
        combat_state,
        CombatEncounterTerminal::MatchTerminated,
        combat_state.tick,
    );
}

/// Full time still consumes one slot-mode `InputFrame` boundary, but it
/// must not advance action mechanics after the match becomes terminal.
pub fn advance_boundary(combat_state: &mut CombatMatchState) {
    combat_state.tick += 1;
}

/// A restart clears the retained intent itself — no half-materialized press
/// survives a kickoff. What survives is the decision CADENCE and SEED,
/// because both are derived from `combat_state.tick`, which keeps counting:
/// the AI does not replay the same choices after every restart the way a
/// per-record stream reset here would make it.
pub fn reset(combat_state: &mut CombatMatchState) {
    for runtime in &mut combat_state.players {
        *runtime = new_player_state(
            runtime.loadout_id.clone(),
            runtime.family_id,
            combat_intent::reset(&runtime.intent),
        );
    }
    combat_state.projectiles.clear();
    combat_state.events.clear();
    combat_state.next_source_sequence = 1;
}

/// A goal restarts play under an allowed lifecycle rule, so anything still
/// open terminates as `cancelled`. The caller has already run
/// [`finish_tick`], so this tick's rows still name the causal input tick
/// one behind the boundary.
///
/// # Panics
///
/// Panics if `combat_state.tick == 0` (a kickoff reset cannot precede the
/// first boundary).
pub fn reset_for_kickoff(state: &MatchState, combat_state: &mut CombatMatchState) {
    assert!(
        combat_state.tick >= 1,
        "a kickoff reset cannot precede the first boundary"
    );
    close_open_sequences(
        state,
        combat_state,
        CombatEncounterTerminal::Cancelled,
        combat_state.tick - 1,
    );
    for runtime in &mut combat_state.players {
        *runtime = new_player_state(
            runtime.loadout_id.clone(),
            runtime.family_id,
            combat_intent::reset(&runtime.intent),
        );
    }
    combat_state.projectiles.clear();
}

/// A defensive, re-validated copy of `combat_state`, suitable for retaining
/// across ticks (e.g. rollback history).
#[must_use]
pub fn snapshot(combat_state: &CombatMatchState) -> CombatMatchState {
    combat_snapshot::copy_owned(combat_state)
}
