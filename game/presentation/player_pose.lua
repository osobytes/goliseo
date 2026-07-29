local offball_runs = require("sim.offball_runs")
local outfield_decision = require("sim.outfield_decision")
local sim_match = require("sim.match")

---@alias PlayerPoseId
---| "keeper_grab"
---| "keeper_throw"
---| "keeper_punt"
---| "keeper_tip"
---| "keeper_spread"
---| "keeper_central"
---| "keeper_stretch"
---| "keeper_dive"
---| "keeper_get_up"
---| "keeper_set"
---| "keeper_ready_low"
---| "keeper_shuffle"
---| "keeper_ready_tall"
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
---| "tackle"
---| "stumble"
---| "kick_follow"
---| "settle"
---| "run_telegraph"
---| "contain"
---| "fatigue"
---| "locomotion"

---@alias PlayerPoseSource "soccer"|"combat"|"locomotion"

---@class PlayerPoseSelection
---@field id PlayerPoseId
---@field priority integer
---@field source PlayerPoseSource

---@class KeeperPoseContext
---@field near_ball boolean
---@field shuffling boolean
---@field tip boolean

-- The outfield signals the renderer cannot read off `MatchPlayer` alone: the
-- match clock the run telegraph window is measured against, the team-owned
-- press mode, and the render-owned release follow-through. Every field is
-- supplied by the caller; this module never samples a clock or a global.
---@class OutfieldPoseContext
---@field now number?  -- seconds since kickoff (`-MatchState.time_left`)
---@field containing boolean?  -- team press state holds this player in contain
---@field kick_follow boolean?  -- a confirmed release is still following through

---@class PlayerPoseModule
local player_pose = {}

-- One authority owns overlap precedence. Outfield presentation work extends
-- this ordered contract instead of adding renderer-local condition chains.
player_pose.PRIORITY = {
    keeper_grab = 123,
    keeper_throw = 122,
    keeper_punt = 121,
    keeper_tip = 110,
    keeper_spread = 102,
    keeper_central = 101,
    keeper_stretch = 100,
    keeper_dive = 100,
    keeper_get_up = 79,
    keeper_set = 78,
    keeper_ready_low = 15,
    keeper_shuffle = 14,
    keeper_ready_tall = 13,
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
    -- Outfield ground poses, all below the combat band so a forced state or a
    -- committed combat phase keeps presentation precedence. The order reads
    -- down from most committed to least: challenges (slide, tackle) outrank the
    -- recovery they can cause (stumble), which outranks what the player is
    -- doing with the ball (kick_follow, settle), which outranks intent
    -- off the ball (run_telegraph, contain) and finally condition (fatigue).
    slide = 60,
    tackle = 55,
    stumble = 50,
    kick_follow = 45,
    settle = 40,
    run_telegraph = 35,
    contain = 30,
    fatigue = 20,
    locomotion = 0,
}

---@param id PlayerPoseId
---@param source PlayerPoseSource
---@return PlayerPoseSelection
local function selection(id, source)
    return { id = id, priority = assert(player_pose.PRIORITY[id]), source = source }
end

-- The opening beat of a run the simulation has already granted. The window and
-- its edge handling belong to `sim.offball_runs`; presentation only asks.
---@param player MatchPlayer
---@param now number?
---@return boolean
local function run_telegraphing(player, now)
    if now == nil then
        return false
    end
    local decision = player.outfield_decision
    if not outfield_decision.is_run_intent(decision.intent) then
        return false
    end
    local expires_at = decision.run_expires_at
    if expires_at == nil then
        return false
    end
    return offball_runs.telegraphing(now, expires_at)
end

-- The sprint tank is spent when the player is not sprinting and cannot engage
-- one: the same hysteresis boundary `sim.match` uses, so the pose never claims
-- a burst is available when it is not.
---@param player MatchPlayer
---@return boolean
local function sprint_spent(player)
    return not player.sprinting and player.sprint_meter <= sim_match.SPRINT_ENGAGE
end

---@param player MatchPlayer
---@param combat CombatPlayerPresentation?
---@param keeper_context KeeperPoseContext?
---@param outfield_context OutfieldPoseContext?
---@return PlayerPoseSelection
function player_pose.select(player, combat, keeper_context, outfield_context)
    ---@type PlayerPoseSelection[]
    local candidates = {}
    local function add(id, source)
        candidates[#candidates + 1] = selection(id, source)
    end

    if player.is_keeper then
        if player.grab_timer > 0 then
            add("keeper_grab", "soccer")
        elseif player.throw_timer > 0 then
            add("keeper_throw", "soccer")
        elseif player.windup_timer > 0 then
            add("keeper_punt", "soccer")
        end
        if keeper_context and keeper_context.tip then
            add("keeper_tip", "soccer")
        end
        if player.dive_timer > 0 then
            if player.save_style == "spread" then
                add("keeper_spread", "soccer")
            elseif player.save_style == "central" then
                add("keeper_central", "soccer")
            elseif player.save_style == "stretch" then
                add("keeper_stretch", "soccer")
            else
                add("keeper_dive", "soccer")
            end
        end
        -- Getting up is the post-dive recovery window the simulation owns, not
        -- the keeper's "recover" positioning state.
        if player.keeper_get_up_timer > 0 then
            add("keeper_get_up", "soccer")
        elseif player.keeper_state == "set" or player.keeper_set > 0 then
            add("keeper_set", "soccer")
        elseif keeper_context and keeper_context.near_ball then
            add("keeper_ready_low", "soccer")
        elseif keeper_context and keeper_context.shuffling then
            add("keeper_shuffle", "soccer")
        else
            add("keeper_ready_tall", "soccer")
        end
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
    -- Outfield poses. Keepers keep their own vocabulary end to end, so a keeper
    -- punt or a keeper stumble never borrows an outfielder's silhouette.
    if not player.is_keeper then
        if player.tackle_timer > 0 then
            add("tackle", "soccer")
        end
        if player.stun_timer > 0 then
            add("stumble", "soccer")
        end
        if outfield_context and outfield_context.kick_follow then
            add("kick_follow", "soccer")
        end
        if player.settle_timer > 0 then
            add("settle", "soccer")
        end
        if run_telegraphing(player, outfield_context and outfield_context.now) then
            add("run_telegraph", "soccer")
        end
        -- One shared silhouette, two independent mechanics: the human jockey
        -- stance and the AI presser's contain mode read the same on screen and
        -- keep their own reach, speed, and trigger rules in `sim.match`.
        if player.jockey_timer > 0 or (outfield_context and outfield_context.containing) then
            add("contain", "soccer")
        end
        if sprint_spent(player) then
            add("fatigue", "soccer")
        end
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
