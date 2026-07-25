local action_families = require("data.action_families")
local equipment_presentations = require("data.equipment_presentations")
local loadouts = require("data.loadouts")

---@alias CombatPresentationReadiness
---| "unavailable"
---| "ready"
---| "cooldown"
---| "committed"
---| "forced"

---@alias CombatTelegraphKind "arc"|"guard_arc"|"line"

---@class CombatPlayerPresentation
---@field player_index integer
---@field player_id string
---@field family_id ActionFamilyId?
---@field family_name string?
---@field equipment_presentation_id string?
---@field equipment_name string?
---@field equipment_attachment EquipmentAttachment?
---@field phase CombatActionPhase
---@field phase_progress number
---@field phase_ticks integer
---@field cooldown_ticks integer
---@field cooldown_fraction number
---@field readiness CombatPresentationReadiness
---@field forced_state CombatForcedState?
---@field forced_ticks integer
---@field immunity_ticks integer
---@field position Vec2
---@field direction Vec2
---@field reach_px number?
---@field projectile_range_px number?
---@field front_arc_degrees number?
---@field telegraph_kind CombatTelegraphKind?
---@field source_sequence integer?

---@class CombatProjectilePresentation
---@field family_id ActionFamilyId
---@field source_index integer
---@field source_sequence integer
---@field position Vec2
---@field direction Vec2
---@field remaining_ticks integer

---@class CombatPresentationModel
---@field enabled boolean
---@field tick integer?
---@field players CombatPlayerPresentation[]
---@field projectiles CombatProjectilePresentation[]

---@alias CombatPresentationEventId
---| "combat.commit"
---| "combat.projectile.spawn"
---| "combat.projectile.expire"
---| "combat.contact.hit"
---| "combat.contact.extended"
---| "combat.contact.guarded"
---| "combat.contact.immune"
---| "combat.contact.superseded"
---| "combat.ball_spill"
---| "combat.forced"
---| "combat.guard_recoil"

---@class CombatEventPresentation
---@field stable_id string?
---@field semantic_id CombatPresentationEventId
---@field tick integer
---@field family_id ActionFamilyId?
---@field source_index integer?
---@field target_index integer?
---@field source_sequence integer?
---@field result CombatContactResult?
---@field x number
---@field y number

---@class CombatPresentationModule
local presentation = {}

---@param value number
---@return number
local function clamp01(value)
    return math.max(0, math.min(1, value))
end

---@param runtime CombatPlayerState
---@param family ActionFamilyData?
---@return number
local function phase_progress(runtime, family)
    if not family then
        return 0
    end
    local duration = 1
    if runtime.phase == "windup" then
        duration = family.windup_ticks
    elseif runtime.phase == "active" then
        duration = family.active_ticks or 1
    elseif runtime.phase == "recovery" then
        duration = family.recovery_ticks
    else
        return runtime.phase == "ready" and 0 or 1
    end
    return clamp01(1 - runtime.phase_ticks / math.max(1, duration))
end

---@param runtime CombatPlayerState
---@return CombatPresentationReadiness
local function readiness(runtime)
    if not runtime.family_id then
        return "unavailable"
    elseif runtime.forced_ticks > 0 then
        return "forced"
    elseif runtime.phase ~= "ready" then
        return "committed"
    elseif runtime.cooldown_ticks > 0 then
        return "cooldown"
    end
    return "ready"
end

---@param runtime CombatPlayerState
---@param family ActionFamilyData?
---@return CombatTelegraphKind?
local function telegraph_kind(runtime, family)
    if not family or runtime.forced_ticks > 0 then
        return nil
    end
    if family.id == "guard" and (runtime.phase == "windup" or runtime.phase == "guard") then
        return "guard_arc"
    elseif
        family.id == "ranged"
        and (runtime.phase == "windup" or runtime.phase == "aim" or runtime.phase == "active")
    then
        return "line"
    elseif
        (family.id == "unarmed" or family.id == "light_melee")
        and (runtime.phase == "windup" or runtime.phase == "active")
    then
        return "arc"
    end
    return nil
end

---@param state MatchState
---@param combat_state CombatMatchState?
---@return CombatPresentationModel
function presentation.model(state, combat_state)
    if not combat_state then
        return { enabled = false, tick = nil, players = {}, projectiles = {} }
    end
    assert(#combat_state.players == #state.players, "combat presentation player count mismatch")
    assert(#combat_state.player_ids == #state.players, "combat presentation identity mismatch")

    local players = {}
    for index, player in ipairs(state.players) do
        assert(
            combat_state.player_ids[index] == player.id,
            "combat presentation player identity mismatch"
        )
        local runtime = combat_state.players[index]
        local family = runtime.family_id and action_families[runtime.family_id] or nil
        local loadout = runtime.loadout_id and loadouts[runtime.loadout_id] or nil
        local equipment = loadout and equipment_presentations[loadout.equipment_presentation_id]
            or nil
        local projectile_range = nil
        if
            family
            and family.projectile_speed_px_per_second
            and family.projectile_lifetime_ticks
        then
            projectile_range = family.projectile_speed_px_per_second
                * family.projectile_lifetime_ticks
                / 60
        end
        players[index] = {
            player_index = index,
            player_id = player.id,
            family_id = runtime.family_id,
            family_name = family and family.name or nil,
            equipment_presentation_id = equipment and equipment.id or nil,
            equipment_name = equipment and equipment.name or nil,
            equipment_attachment = equipment and equipment.attachment or nil,
            phase = runtime.phase,
            phase_progress = phase_progress(runtime, family),
            phase_ticks = runtime.phase_ticks,
            cooldown_ticks = runtime.cooldown_ticks,
            cooldown_fraction = family and family.cooldown_ticks > 0 and clamp01(
                runtime.cooldown_ticks / family.cooldown_ticks
            ) or 0,
            readiness = readiness(runtime),
            forced_state = runtime.forced_state,
            forced_ticks = runtime.forced_ticks,
            immunity_ticks = runtime.immunity_ticks,
            position = player.pos,
            direction = player.facing,
            reach_px = family and family.reach_px or nil,
            projectile_range_px = projectile_range,
            front_arc_degrees = family and family.front_arc_degrees or nil,
            telegraph_kind = telegraph_kind(runtime, family),
            source_sequence = runtime.source_sequence,
        }
    end

    local projectiles = {}
    for index, projectile in ipairs(combat_state.projectiles) do
        projectiles[index] = {
            family_id = projectile.family_id,
            source_index = projectile.source_index,
            source_sequence = projectile.source_sequence,
            position = projectile.pos,
            direction = projectile.dir,
            remaining_ticks = projectile.remaining_ticks,
        }
    end
    return {
        enabled = true,
        tick = combat_state.tick,
        players = players,
        projectiles = projectiles,
    }
end

---@param event CombatEvent
---@param stable_id string?
---@return CombatEventPresentation
function presentation.event(event, stable_id)
    local semantic_id ---@type CombatPresentationEventId
    if event.kind == "projectile_spawn" then
        semantic_id = "combat.projectile.spawn"
    elseif event.kind == "projectile_expire" then
        semantic_id = "combat.projectile.expire"
    elseif event.kind == "contact" then
        semantic_id = "combat.contact." .. assert(event.result, "combat contact result is required")
    else
        semantic_id = "combat." .. event.kind
    end
    return {
        stable_id = stable_id,
        semantic_id = semantic_id,
        tick = event.tick,
        family_id = event.family_id,
        source_index = event.source_index,
        target_index = event.target_index,
        source_sequence = event.source_sequence,
        result = event.result,
        x = event.x,
        y = event.y,
    }
end

return presentation
