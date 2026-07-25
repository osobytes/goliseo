local Vec2 = require("core.vec2")
local combat_rules = require("sim.combat_rules")
local fixed_clock = require("sim.fixed_clock")
local match_snapshot = require("sim.match_snapshot")
local combat_snapshot = require("sim.combat_snapshot")
local action_families = require("data.action_families")
local loadouts = require("data.loadouts")
local player_pool = require("data.players")

---@alias CombatActionPhase "ready"|"windup"|"active"|"aim"|"guard"|"recovery"
---@alias CombatForcedState "stagger"|"knockback"
---@alias CombatContactResult "hit"|"extended"|"guarded"|"immune"|"superseded"
---@alias CombatEventKind
---| "commit"
---| "projectile_spawn"
---| "projectile_expire"
---| "contact"
---| "ball_spill"
---| "forced"
---| "guard_recoil"

---@class CombatPlayerState
---@field loadout_id string?
---@field family_id ActionFamilyId?
---@field phase CombatActionPhase
---@field phase_ticks integer
---@field cooldown_ticks integer
---@field source_sequence integer?
---@field contacted boolean
---@field release_latched boolean
---@field control_held boolean
---@field projectile_spawned boolean
---@field forced_state CombatForcedState?
---@field forced_ticks integer
---@field chain_ticks integer
---@field immunity_ticks integer

---@class CombatProjectile
---@field family_id ActionFamilyId
---@field source_index integer
---@field source_sequence integer
---@field pos Vec2
---@field dir Vec2
---@field remaining_ticks integer

---@class CombatEvent
---@field kind CombatEventKind
---@field tick integer
---@field family_id ActionFamilyId?
---@field source_index integer?
---@field target_index integer?
---@field source_sequence integer?
---@field result CombatContactResult?
---@field x number
---@field y number
---@field interruption_ticks integer?
---@field displacement_px number?

---@class CombatMatchState
---@field version integer
---@field tick integer
---@field player_ids string[]
---@field players CombatPlayerState[]
---@field projectiles CombatProjectile[]
---@field events CombatEvent[]
---@field next_source_sequence integer

---@class CombatContact
---@field family_id ActionFamilyId
---@field source_index integer
---@field target_index integer
---@field source_sequence integer
---@field projectile_sequence integer?
---@field guarded boolean
---@field immune boolean
---@field source_pos Vec2

local MAX_DISABLE_TICKS = combat_rules.MAX_DISABLE_TICKS
local IMMUNITY_TICKS = combat_rules.IMMUNITY_TICKS
local KNOCKBACK_THRESHOLD_PX = combat_rules.KNOCKBACK_THRESHOLD_PX
local PROJECTILE_STEP_PX = assert(action_families.ranged.projectile_speed_px_per_second)
    * fixed_clock.TICK_SECONDS
local EPSILON = 1e-9

---@class CombatModule
local combat = {}

combat.MAX_DISABLE_TICKS = MAX_DISABLE_TICKS
combat.IMMUNITY_TICKS = IMMUNITY_TICKS
combat.PROJECTILE_STEP_PX = PROJECTILE_STEP_PX

---@return table<string, PlayerData>
local function default_players_by_id()
    local by_id = {}
    for _, player in ipairs(player_pool) do
        by_id[player.id] = player
    end
    return by_id
end

---@param players_by_id table<string, PlayerData>|PlayerData[]?
---@return table<string, PlayerData>
local function normalize_players_by_id(players_by_id)
    if not players_by_id then
        return default_players_by_id()
    end
    local by_id = {}
    for key, player in pairs(players_by_id) do
        if type(key) == "string" then
            by_id[key] = player
        else
            by_id[player.id] = player
        end
    end
    return by_id
end

---@param loadout_id string?
---@param family_id ActionFamilyId?
---@return CombatPlayerState
local function new_player_state(loadout_id, family_id)
    return {
        loadout_id = loadout_id,
        family_id = family_id,
        phase = "ready",
        phase_ticks = 0,
        cooldown_ticks = 0,
        source_sequence = nil,
        contacted = false,
        release_latched = false,
        control_held = false,
        projectile_spawned = false,
        forced_state = nil,
        forced_ticks = 0,
        chain_ticks = 0,
        immunity_ticks = 0,
    }
end

---@param state MatchState
---@param players_by_id table<string, PlayerData>|PlayerData[]?
---@return CombatMatchState
function combat.new_state(state, players_by_id)
    match_snapshot.mark_unsupported(
        state,
        "combat-active match snapshots require their CombatMatchState companion"
    )
    local by_id = normalize_players_by_id(players_by_id)
    local runtimes = {}
    local player_ids = {}
    for index, match_player in ipairs(state.players) do
        player_ids[index] = match_player.id
        local loadout_id ---@type string?
        local family_id ---@type ActionFamilyId?
        if not match_player.is_keeper then
            local player_data = by_id[match_player.id]
            if player_data and player_data.loadout_id then
                loadout_id = player_data.loadout_id
                local loadout =
                    assert(loadouts[loadout_id], "unknown combat loadout: " .. tostring(loadout_id))
                family_id = loadout.family_id
                assert(action_families[family_id], "unknown action family: " .. tostring(family_id))
            end
        end
        runtimes[index] = new_player_state(loadout_id, family_id)
    end
    return {
        version = combat_snapshot.VERSION,
        tick = 0,
        player_ids = player_ids,
        players = runtimes,
        projectiles = {},
        events = {},
        next_source_sequence = 1,
    }
end

---@param input MatchInput
---@return MatchInput
local function copy_input(input)
    return {
        move = input.move,
        shoot = input.shoot,
        shoot_held = input.shoot_held,
        pass = input.pass,
        pass_held = input.pass_held,
        switch = input.switch,
        dash = input.dash,
        dodge = input.dodge,
        lob = input.lob,
        sprint = input.sprint,
        jockey = input.jockey,
        aerial_strike = input.aerial_strike,
        aerial_acrobatic = input.aerial_acrobatic,
        equipment_held = input.equipment_held,
        equipment_pressed = input.equipment_pressed,
        equipment_released = input.equipment_released,
    }
end

---@param input MatchInput
local function suppress_soccer_actions(input)
    input.shoot = false
    input.shoot_held = false
    input.pass = false
    input.pass_held = false
    input.switch = false
    input.dash = false
    input.dodge = false
    input.lob = false
    input.sprint = false
    input.jockey = false
    input.aerial_strike = false
    input.aerial_acrobatic = false
end

---@param input MatchInput
---@return boolean
local function soccer_has_priority(input)
    return input.shoot
        or input.shoot_held
        or input.pass
        or input.pass_held
        or input.switch
        or input.dash
        or input.dodge
        or input.jockey
end

---@param player MatchPlayer
---@return boolean
local function player_has_soccer_commitment(player)
    return player.slide_timer > 0
        or player.tackle_timer > 0
        or player.jockey_timer > 0
        or player.dodge_timer > 0
        or player.aerial_timer > 0
        or player.aerial_recovery > 0
        or player.windup_timer > 0
        or player.windup_shot ~= nil
end

---@param state CombatMatchState
---@param family_id ActionFamilyId
---@param player_index integer
---@param runtime CombatPlayerState
---@param input MatchInput
---@param position Vec2
local function commit_action(state, family_id, player_index, runtime, input, position)
    local family = action_families[family_id]
    runtime.phase = "windup"
    runtime.phase_ticks = family.windup_ticks
    runtime.cooldown_ticks = family.cooldown_ticks
    runtime.source_sequence = state.next_source_sequence
    runtime.contacted = false
    runtime.release_latched = input.equipment_released
    runtime.control_held = input.equipment_held
    runtime.projectile_spawned = false
    state.next_source_sequence = state.next_source_sequence + 1
    state.events[#state.events + 1] = {
        kind = "commit",
        tick = state.tick,
        family_id = family_id,
        source_index = player_index,
        target_index = nil,
        source_sequence = runtime.source_sequence,
        result = nil,
        x = position.x,
        y = position.y,
        interruption_ticks = nil,
        displacement_px = nil,
    }
end

---@param runtime CombatPlayerState
---@return boolean
local function is_committed(runtime)
    return runtime.phase ~= "ready"
end

---@param state MatchState
---@param combat_state CombatMatchState
---@param inputs table<integer, MatchInput>
---@param equipment_ineligible table<integer, boolean>?
---@return table<integer, MatchInput>
function combat.prepare_inputs(state, combat_state, inputs, equipment_ineligible)
    assert(#combat_state.players == #state.players, "combat player state does not match fixture")
    assert(
        #combat_state.player_ids == #state.players,
        "combat player identity does not match fixture"
    )
    for index, player in ipairs(state.players) do
        assert(
            combat_state.player_ids[index] == player.id,
            "combat player identity does not match fixture"
        )
    end
    combat.sanitize_forced_players(state, combat_state)
    combat.clear_events(combat_state)

    local prepared = {}
    for player_index, input in pairs(inputs) do
        prepared[player_index] = copy_input(input)
    end

    for player_index, runtime in ipairs(combat_state.players) do
        local input = prepared[player_index]
        if input then
            local match_player = state.players[player_index]
            runtime.control_held = input.equipment_held

            if runtime.phase == "windup" then
                if
                    (runtime.family_id == "guard" or runtime.family_id == "ranged")
                    and input.equipment_released
                then
                    runtime.release_latched = true
                end
            elseif runtime.phase == "aim" and runtime.family_id == "ranged" then
                if input.equipment_released then
                    runtime.release_latched = true
                    runtime.phase = "active"
                    runtime.phase_ticks = 1
                end
            elseif runtime.phase == "guard" and runtime.family_id == "guard" then
                if input.equipment_released or not input.equipment_held then
                    runtime.phase = "recovery"
                    runtime.phase_ticks = action_families.guard.recovery_ticks
                end
            end

            if is_committed(runtime) or runtime.forced_ticks > 0 then
                suppress_soccer_actions(input)
                input.equipment_pressed = false
            elseif
                runtime.family_id
                and runtime.cooldown_ticks == 0
                and input.equipment_pressed
                and state.kickoff_hold == 0
                and not match_player.is_keeper
                and not soccer_has_priority(input)
                and not player_has_soccer_commitment(match_player)
                and not (equipment_ineligible and equipment_ineligible[player_index])
            then
                commit_action(
                    combat_state,
                    runtime.family_id,
                    player_index,
                    runtime,
                    input,
                    match_player.pos
                )
                match_player.charge = 0
                match_player.pass_charge = 0
                match_player.pass_target = nil
                suppress_soccer_actions(input)
                input.equipment_pressed = false
            end
        end
    end

    return prepared
end

---@param state CombatMatchState
function combat.clear_events(state)
    for index = #state.events, 1, -1 do
        state.events[index] = nil
    end
end

---@param state CombatMatchState?
---@param player_index integer
---@return boolean
function combat.blocks_actions(state, player_index)
    if not state then
        return false
    end
    local runtime = state.players[player_index]
    return runtime ~= nil and (runtime.forced_ticks > 0 or is_committed(runtime))
end

---@param state CombatMatchState?
---@param player_index integer
---@return number
function combat.movement_multiplier(state, player_index)
    if not state then
        return 1
    end
    local runtime = state.players[player_index]
    if not runtime then
        return 1
    end
    if runtime.forced_ticks > 0 then
        return 0
    end
    if is_committed(runtime) and runtime.family_id then
        return action_families[runtime.family_id].movement_multiplier
    end
    return 1
end

---@param a Vec2
---@param b Vec2
---@return number
local function dot(a, b)
    return a.x * b.x + a.y * b.y
end

---@param direction Vec2
---@param fallback Vec2
---@return Vec2
local function unit_or(direction, fallback)
    if direction:length() > EPSILON then
        return direction:normalized()
    end
    return fallback
end

---@param source MatchPlayer
---@param target MatchPlayer
---@param family ActionFamilyData
---@return boolean
---@return number projection
local function melee_geometry(source, target, family)
    local offset = target.pos:sub(source.pos)
    local distance = offset:length()
    if distance > assert(family.reach_px) + target.radius then
        return false, 0
    end
    local facing = unit_or(source.facing, Vec2.new(source.team == "home" and 1 or -1, 0))
    local projection = dot(offset, facing)
    if projection < 0 then
        return false, projection
    end
    if distance <= EPSILON then
        return true, projection
    end
    local half_arc = math.rad(family.front_arc_degrees / 2)
    return dot(offset:scale(1 / distance), facing) + EPSILON >= math.cos(half_arc), projection
end

---@param target MatchPlayer
---@param source_pos Vec2
---@return boolean
local function target_guarding(target, source_pos)
    local offset = source_pos:sub(target.pos)
    local distance = offset:length()
    if distance <= EPSILON then
        return true
    end
    local facing = unit_or(target.facing, Vec2.new(target.team == "home" and 1 or -1, 0))
    return dot(offset:scale(1 / distance), facing) + EPSILON
        >= math.cos(math.rad(action_families.guard.front_arc_degrees / 2))
end

---@param state MatchState
---@param combat_state CombatMatchState
---@param source_index integer
---@param family ActionFamilyData
---@return integer?
local function select_melee_target(state, combat_state, source_index, family)
    local source = state.players[source_index]
    local best_index ---@type integer?
    local best_projection ---@type number?
    for target_index, target in ipairs(state.players) do
        local target_runtime = combat_state.players[target_index]
        if
            target.team ~= source.team
            and not target.is_keeper
            and target.dodge_timer <= 0
            and target_runtime ~= nil
        then
            local legal, projection = melee_geometry(source, target, family)
            if
                legal
                and (
                    best_projection == nil
                    or projection < best_projection - EPSILON
                    or (
                        math.abs(projection - best_projection) <= EPSILON
                        and target_index < assert(best_index)
                    )
                )
            then
                best_index = target_index
                best_projection = projection
            end
        end
    end
    return best_index
end

---@param start_pos Vec2
---@param end_pos Vec2
---@param center Vec2
---@param radius number
---@return number?
local function segment_circle_time(start_pos, end_pos, center, radius)
    local segment = end_pos:sub(start_pos)
    local to_center = center:sub(start_pos)
    local length_sq = dot(segment, segment)
    if length_sq <= EPSILON then
        return start_pos:dist(center) <= radius and 0 or nil
    end
    local projection = math.max(0, math.min(1, dot(to_center, segment) / length_sq))
    local closest = start_pos:add(segment:scale(projection))
    if closest:dist(center) <= radius + EPSILON then
        return projection
    end
    return nil
end

---@param state MatchState
---@param combat_state CombatMatchState
---@param projectile CombatProjectile
---@param end_pos Vec2
---@return integer?
local function select_projectile_target(state, combat_state, projectile, end_pos)
    local source = state.players[projectile.source_index]
    local best_index ---@type integer?
    local best_time ---@type number?
    for target_index, target in ipairs(state.players) do
        local runtime = combat_state.players[target_index]
        if
            target.team ~= source.team
            and not target.is_keeper
            and target.dodge_timer <= 0
            and runtime ~= nil
        then
            local contact_time =
                segment_circle_time(projectile.pos, end_pos, target.pos, target.radius)
            if
                contact_time
                and (
                    best_time == nil
                    or contact_time < best_time - EPSILON
                    or (
                        math.abs(contact_time - best_time) <= EPSILON
                        and target_index < assert(best_index)
                    )
                )
            then
                best_index = target_index
                best_time = contact_time
            end
        end
    end
    return best_index
end

---@param state MatchState
---@param combat_state CombatMatchState
---@param source_index integer
---@param source_sequence integer
---@return CombatProjectile
local function spawn_projectile(state, combat_state, source_index, source_sequence)
    local source = state.players[source_index]
    local family = action_families.ranged
    local projectile = {
        family_id = "ranged",
        source_index = source_index,
        source_sequence = source_sequence,
        pos = source.pos,
        dir = unit_or(source.facing, Vec2.new(source.team == "home" and 1 or -1, 0)),
        remaining_ticks = assert(family.projectile_lifetime_ticks),
    }
    combat_state.events[#combat_state.events + 1] = {
        kind = "projectile_spawn",
        tick = combat_state.tick,
        family_id = "ranged",
        source_index = source_index,
        target_index = nil,
        source_sequence = source_sequence,
        result = nil,
        x = projectile.pos.x,
        y = projectile.pos.y,
        interruption_ticks = nil,
        displacement_px = nil,
    }
    return projectile
end

---@param state MatchState
---@param combat_state CombatMatchState
---@param family_id ActionFamilyId
---@param source_index integer
---@param target_index integer
---@param source_sequence integer
---@param projectile_sequence integer?
---@param source_pos Vec2
---@return CombatContact
local function make_contact(
    state,
    combat_state,
    family_id,
    source_index,
    target_index,
    source_sequence,
    projectile_sequence,
    source_pos
)
    local target_runtime = combat_state.players[target_index]
    local guarded = target_runtime.phase == "guard"
        and target_guarding(state.players[target_index], source_pos)
    return {
        family_id = family_id,
        source_index = source_index,
        target_index = target_index,
        source_sequence = source_sequence,
        projectile_sequence = projectile_sequence,
        guarded = guarded,
        immune = target_runtime.immunity_ticks > 0,
        source_pos = source_pos,
    }
end

---@param state MatchState
---@param combat_state CombatMatchState
---@return CombatContact[]
function combat.collect_contacts(state, combat_state)
    local contacts = {}

    for source_index, runtime in ipairs(combat_state.players) do
        if
            runtime.phase == "active"
            and runtime.family_id == "ranged"
            and not runtime.projectile_spawned
        then
            local sequence =
                assert(runtime.source_sequence, "active ranged action needs a sequence")
            combat_state.projectiles[#combat_state.projectiles + 1] =
                spawn_projectile(state, combat_state, source_index, sequence)
            runtime.projectile_spawned = true
        elseif
            runtime.phase == "active"
            and runtime.family_id
            and runtime.family_id ~= "guard"
            and runtime.family_id ~= "ranged"
            and not runtime.contacted
        then
            local family = action_families[runtime.family_id]
            local target_index = select_melee_target(state, combat_state, source_index, family)
            if target_index then
                runtime.contacted = true
                contacts[#contacts + 1] = make_contact(
                    state,
                    combat_state,
                    runtime.family_id,
                    source_index,
                    target_index,
                    assert(runtime.source_sequence, "active melee action needs a sequence"),
                    nil,
                    state.players[source_index].pos
                )
            end
        end
    end

    local retained = {}
    for _, projectile in ipairs(combat_state.projectiles) do
        local start_pos = projectile.pos
        local end_pos = start_pos:add(projectile.dir:scale(PROJECTILE_STEP_PX))
        local target_index = select_projectile_target(state, combat_state, projectile, end_pos)
        projectile.remaining_ticks = projectile.remaining_ticks - 1
        if target_index then
            contacts[#contacts + 1] = make_contact(
                state,
                combat_state,
                projectile.family_id,
                projectile.source_index,
                target_index,
                projectile.source_sequence,
                projectile.source_sequence,
                start_pos
            )
        else
            projectile.pos = end_pos
            local in_field = end_pos.x >= 0
                and end_pos.x <= state.field.w
                and end_pos.y >= 0
                and end_pos.y <= state.field.h
            if projectile.remaining_ticks > 0 and in_field then
                retained[#retained + 1] = projectile
            else
                combat_state.events[#combat_state.events + 1] = {
                    kind = "projectile_expire",
                    tick = combat_state.tick,
                    family_id = projectile.family_id,
                    source_index = projectile.source_index,
                    target_index = nil,
                    source_sequence = projectile.source_sequence,
                    result = nil,
                    x = end_pos.x,
                    y = end_pos.y,
                    interruption_ticks = nil,
                    displacement_px = nil,
                }
            end
        end
    end
    combat_state.projectiles = retained

    table.sort(contacts, function(left, right)
        if left.source_sequence ~= right.source_sequence then
            return left.source_sequence < right.source_sequence
        end
        if left.source_index ~= right.source_index then
            return left.source_index < right.source_index
        end
        return left.target_index < right.target_index
    end)
    return contacts
end

---@param left CombatContact
---@param right CombatContact
---@return boolean
local function contact_dominates(left, right)
    local left_outcome = assert(action_families[left.family_id].unguarded_outcome)
    local right_outcome = assert(action_families[right.family_id].unguarded_outcome)
    if left_outcome.interruption_ticks ~= right_outcome.interruption_ticks then
        return left_outcome.interruption_ticks > right_outcome.interruption_ticks
    end
    if left_outcome.displacement_px ~= right_outcome.displacement_px then
        return left_outcome.displacement_px > right_outcome.displacement_px
    end
    if left.source_index ~= right.source_index then
        return left.source_index < right.source_index
    end
    return left.source_sequence < right.source_sequence
end

---@param state MatchState
---@param player MatchPlayer
---@param position Vec2
---@return Vec2
local function clamp_player(state, player, position)
    return Vec2.new(
        math.max(player.radius, math.min(state.field.w - player.radius, position.x)),
        math.max(player.radius, math.min(state.field.h - player.radius, position.y))
    )
end

---@param runtime CombatPlayerState
local function cancel_action(runtime)
    runtime.phase = "ready"
    runtime.phase_ticks = 0
    runtime.source_sequence = nil
    runtime.contacted = false
    runtime.release_latched = false
    runtime.projectile_spawned = false
end

---@param player MatchPlayer
local function cancel_soccer_commitments(player)
    player.slide_timer = 0
    player.slide_vel = 0
    player.tackle_timer = 0
    player.jockey_timer = 0
    player.dodge_timer = 0
    player.aerial_timer = 0
    player.aerial_recovery = 0
    player.aerial_style = nil
    player.aerial_outcome = nil
    player.aerial_jump = 0
    player.receive_timer = 0
    player.windup_timer = 0
    player.windup_shot = nil
    player.charge = 0
    player.pass_charge = 0
    player.pass_target = nil
    player.sprinting = false
    player.run_vel = Vec2.new(0, 0)
end

---@param state MatchState
---@param combat_state CombatMatchState
function combat.sanitize_forced_players(state, combat_state)
    for player_index, runtime in ipairs(combat_state.players) do
        if runtime.forced_ticks > 0 then
            cancel_soccer_commitments(state.players[player_index])
        end
    end
end

---@param state MatchState
---@param combat_state CombatMatchState
---@param contact CombatContact
---@param result CombatContactResult
local function emit_contact(state, combat_state, contact, result)
    local target = state.players[contact.target_index]
    local outcome = assert(action_families[contact.family_id].unguarded_outcome)
    combat_state.events[#combat_state.events + 1] = {
        kind = "contact",
        tick = combat_state.tick,
        family_id = contact.family_id,
        source_index = contact.source_index,
        target_index = contact.target_index,
        source_sequence = contact.source_sequence,
        result = result,
        x = target.pos.x,
        y = target.pos.y,
        interruption_ticks = outcome.interruption_ticks,
        displacement_px = outcome.displacement_px,
    }
end

---@param state MatchState
---@param combat_state CombatMatchState
---@param contact CombatContact
local function apply_guard_recoil(state, combat_state, contact)
    local family = action_families[contact.family_id]
    local recoil = math.min(6, family.guarded_recoil_px)
    if recoil <= 0 then
        return
    end
    local target = state.players[contact.target_index]
    local away = target.pos:sub(contact.source_pos)
    local direction = unit_or(away, target.facing)
    target.pos = clamp_player(state, target, target.pos:add(direction:scale(recoil)))
    combat_state.events[#combat_state.events + 1] = {
        kind = "guard_recoil",
        tick = combat_state.tick,
        family_id = contact.family_id,
        source_index = contact.source_index,
        target_index = contact.target_index,
        source_sequence = contact.source_sequence,
        result = "guarded",
        x = target.pos.x,
        y = target.pos.y,
        interruption_ticks = 0,
        displacement_px = recoil,
    }
end

---@param state MatchState
---@param combat_state CombatMatchState
---@param contact CombatContact
local function apply_unguarded_outcome(state, combat_state, contact)
    local target_index = contact.target_index
    local target = state.players[target_index]
    local runtime = combat_state.players[target_index]
    local outcome = assert(action_families[contact.family_id].unguarded_outcome)

    if runtime.forced_ticks > 0 then
        runtime.forced_ticks = math.min(
            runtime.chain_ticks,
            math.max(runtime.forced_ticks, outcome.interruption_ticks)
        )
        emit_contact(state, combat_state, contact, "extended")
        return
    end

    runtime.forced_ticks = math.min(MAX_DISABLE_TICKS, outcome.interruption_ticks)
    runtime.chain_ticks = MAX_DISABLE_TICKS
    runtime.forced_state = outcome.displacement_px >= KNOCKBACK_THRESHOLD_PX and "knockback"
        or "stagger"
    cancel_action(runtime)
    cancel_soccer_commitments(target)

    local away = target.pos:sub(contact.source_pos)
    local direction = unit_or(away, target.facing)
    target.pos =
        clamp_player(state, target, target.pos:add(direction:scale(outcome.displacement_px)))

    emit_contact(state, combat_state, contact, "hit")
    combat_state.events[#combat_state.events + 1] = {
        kind = "forced",
        tick = combat_state.tick,
        family_id = contact.family_id,
        source_index = contact.source_index,
        target_index = target_index,
        source_sequence = contact.source_sequence,
        result = "hit",
        x = target.pos.x,
        y = target.pos.y,
        interruption_ticks = runtime.forced_ticks,
        displacement_px = outcome.displacement_px,
    }

    if outcome.ball_spill and state.owner == target_index then
        state.owner = nil
        state.pickup_cd = math.max(state.pickup_cd, fixed_clock.TICK_SECONDS)
        combat_state.events[#combat_state.events + 1] = {
            kind = "ball_spill",
            tick = combat_state.tick,
            family_id = contact.family_id,
            source_index = contact.source_index,
            target_index = target_index,
            source_sequence = contact.source_sequence,
            result = "hit",
            x = state.ball.x,
            y = state.ball.y,
            interruption_ticks = nil,
            displacement_px = nil,
        }
    end
end

---@param state MatchState
---@param combat_state CombatMatchState
---@param contacts CombatContact[]
function combat.resolve_contacts(state, combat_state, contacts)
    local dominant_by_target = {}
    for _, contact in ipairs(contacts) do
        local current = dominant_by_target[contact.target_index]
        if not current or contact_dominates(contact, current) then
            dominant_by_target[contact.target_index] = contact
        end
    end

    for _, contact in ipairs(contacts) do
        if contact.guarded then
            emit_contact(state, combat_state, contact, "guarded")
        elseif contact.immune then
            emit_contact(state, combat_state, contact, "immune")
        elseif dominant_by_target[contact.target_index] ~= contact then
            emit_contact(state, combat_state, contact, "superseded")
        end
    end

    for target_index = 1, #state.players do
        local dominant = dominant_by_target[target_index]
        if dominant then
            if dominant.guarded then
                apply_guard_recoil(state, combat_state, dominant)
            elseif not dominant.immune then
                apply_unguarded_outcome(state, combat_state, dominant)
            end
        end
    end
end

---@param runtime CombatPlayerState
local function finish_action_tick(runtime)
    if runtime.phase == "windup" then
        runtime.phase_ticks = runtime.phase_ticks - 1
        if runtime.phase_ticks == 0 then
            if runtime.family_id == "guard" then
                if runtime.release_latched or not runtime.control_held then
                    runtime.phase = "recovery"
                    runtime.phase_ticks = action_families.guard.recovery_ticks
                else
                    runtime.phase = "guard"
                end
            elseif runtime.family_id == "ranged" then
                if runtime.release_latched then
                    runtime.phase = "active"
                    runtime.phase_ticks = 1
                else
                    runtime.phase = "aim"
                end
            else
                local family = action_families[assert(runtime.family_id)]
                runtime.phase = "active"
                runtime.phase_ticks = assert(family.active_ticks)
            end
        end
    elseif runtime.phase == "active" then
        runtime.phase_ticks = runtime.phase_ticks - 1
        if runtime.phase_ticks == 0 then
            runtime.phase = "recovery"
            runtime.phase_ticks = action_families[assert(runtime.family_id)].recovery_ticks
        end
    elseif runtime.phase == "recovery" then
        runtime.phase_ticks = runtime.phase_ticks - 1
        if runtime.phase_ticks == 0 then
            cancel_action(runtime)
        end
    end
end

---@param combat_state CombatMatchState
function combat.finish_tick(combat_state)
    for _, runtime in ipairs(combat_state.players) do
        if runtime.cooldown_ticks > 0 then
            runtime.cooldown_ticks = runtime.cooldown_ticks - 1
        end

        if runtime.forced_ticks > 0 then
            runtime.forced_ticks = runtime.forced_ticks - 1
            runtime.chain_ticks = math.max(0, runtime.chain_ticks - 1)
            if runtime.forced_ticks == 0 then
                runtime.forced_state = nil
                runtime.chain_ticks = 0
                runtime.immunity_ticks = IMMUNITY_TICKS
            end
        elseif runtime.immunity_ticks > 0 then
            runtime.immunity_ticks = runtime.immunity_ticks - 1
        end

        finish_action_tick(runtime)
    end
    combat_state.tick = combat_state.tick + 1
end

-- Full time still consumes one slot-mode InputFrame boundary, but it must not
-- advance action mechanics after the match becomes terminal.
---@param combat_state CombatMatchState
function combat.advance_boundary(combat_state)
    combat_state.tick = combat_state.tick + 1
end

---@param combat_state CombatMatchState
function combat.reset(combat_state)
    for index, runtime in ipairs(combat_state.players) do
        combat_state.players[index] = new_player_state(runtime.loadout_id, runtime.family_id)
    end
    combat_state.projectiles = {}
    combat_state.events = {}
    combat_state.next_source_sequence = 1
end

---@param combat_state CombatMatchState
function combat.reset_for_kickoff(combat_state)
    for index, runtime in ipairs(combat_state.players) do
        combat_state.players[index] = new_player_state(runtime.loadout_id, runtime.family_id)
    end
    combat_state.projectiles = {}
end

---@param combat_state CombatMatchState
---@return CombatMatchState
function combat.snapshot(combat_state)
    assert(type(combat_state) == "table", "combat state is required")
    return combat_snapshot.copy_owned(combat_state, false)
end

return combat
