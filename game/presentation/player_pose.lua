---@alias PlayerPoseId
---| "keeper_dive"
---| "aerial_bicycle"
---| "aerial_action"
---| "combat_knockback"
---| "combat_stagger"
---| "combat_guard"
---| "combat_active"
---| "combat_windup"
---| "combat_aim"
---| "combat_recovery"
---| "soccer_windup"
---| "slide"
---| "locomotion"

---@alias PlayerPoseSource "soccer"|"combat"|"locomotion"

---@class PlayerPoseSelection
---@field id PlayerPoseId
---@field priority integer
---@field source PlayerPoseSource

---@class PlayerPoseModule
local player_pose = {}

-- One authority owns overlap precedence. Outfield presentation work extends
-- this ordered contract instead of adding renderer-local condition chains.
player_pose.PRIORITY = {
    keeper_dive = 100,
    aerial_bicycle = 95,
    aerial_action = 94,
    combat_knockback = 90,
    combat_stagger = 89,
    combat_guard = 84,
    combat_active = 83,
    combat_windup = 82,
    combat_aim = 81,
    combat_recovery = 80,
    soccer_windup = 70,
    slide = 60,
    locomotion = 0,
}

---@param id PlayerPoseId
---@param source PlayerPoseSource
---@return PlayerPoseSelection
local function selection(id, source)
    return { id = id, priority = assert(player_pose.PRIORITY[id]), source = source }
end

---@param player MatchPlayer
---@param combat CombatPlayerPresentation?
---@return PlayerPoseSelection
function player_pose.select(player, combat)
    ---@type PlayerPoseSelection[]
    local candidates = {}
    local function add(id, source)
        candidates[#candidates + 1] = selection(id, source)
    end

    if player.is_keeper and player.dive_timer > 0 then
        add("keeper_dive", "soccer")
    end
    if player.aerial_timer > 0 and player.aerial_style == "bicycle" then
        add("aerial_bicycle", "soccer")
    elseif player.aerial_timer > 0 and player.aerial_style ~= nil then
        add("aerial_action", "soccer")
    end
    if combat and combat.forced_state == "knockback" and combat.forced_ticks > 0 then
        add("combat_knockback", "combat")
    elseif combat and combat.forced_state == "stagger" and combat.forced_ticks > 0 then
        add("combat_stagger", "combat")
    end
    if combat and combat.phase == "guard" then
        add("combat_guard", "combat")
    elseif combat and combat.phase == "active" then
        add("combat_active", "combat")
    elseif combat and combat.phase == "windup" then
        add("combat_windup", "combat")
    elseif combat and combat.phase == "aim" then
        add("combat_aim", "combat")
    elseif combat and combat.phase == "recovery" then
        add("combat_recovery", "combat")
    end
    if player.windup_timer > 0 then
        add("soccer_windup", "soccer")
    end
    if player.slide_timer > 0 then
        add("slide", "soccer")
    end
    add("locomotion", "locomotion")

    local best = candidates[1]
    for index = 2, #candidates do
        local candidate = candidates[index]
        -- Equal priorities use pose id order so selection never depends on
        -- candidate discovery or condition ordering.
        if
            candidate.priority > best.priority
            or (candidate.priority == best.priority and candidate.id < best.id)
        then
            best = candidate
        end
    end
    return best
end

return player_pose
