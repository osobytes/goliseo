local Vec2 = require("core.vec2")
local combat = require("sim.combat")
local match = require("sim.match")
local teams = require("data.teams")

---@class CrowdedCombatFeedbackFixture
---@field state MatchState
---@field combat CombatMatchState
---@field events RollbackWrappedCombatEvent[]
---@field ball { x: number, y: number }
---@field hud_rects Rect[]
---@field selection_occluders Rect[]
---@field threat_occluders Rect[]
---@field occluders Rect[]

---@class CombatFeedbackFixtureModule
local fixture = {}

---@param id string
---@param ordinal integer
---@param event CombatEvent
---@return RollbackWrappedCombatEvent
local function wrapped(id, ordinal, event)
    return {
        id = id,
        tick = event.tick,
        domain = ("combat/%s/%d"):format(event.kind, event.source_sequence or 0),
        ordinal = ordinal,
        payload = event,
    }
end

---@return CrowdedCombatFeedbackFixture
function fixture.new()
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        seed = 147,
    })
    local combat_state = combat.new_state(state)
    state.owner = nil
    state.ball = Vec2.new(480, 270)
    state.ball_vel = Vec2.new(0, 0)
    state.controlled = 4

    local positions = {
        Vec2.new(95, 270),
        Vec2.new(220, 190),
        Vec2.new(300, 230),
        Vec2.new(360, 350),
        Vec2.new(420, 155),
        Vec2.new(865, 270),
        Vec2.new(650, 220),
        Vec2.new(710, 360),
        Vec2.new(600, 150),
        Vec2.new(540, 385),
    }
    for index, player in ipairs(state.players) do
        player.pos = positions[index]
        player.facing = Vec2.new(player.team == "home" and 1 or -1, 0)
        local runtime = combat_state.players[index]
        if runtime.family_id == "guard" then
            runtime.phase = "guard"
        elseif runtime.family_id == "ranged" then
            runtime.phase = "aim"
        elseif runtime.family_id then
            runtime.phase = "windup"
            runtime.phase_ticks = 2
        end
    end
    combat_state.projectiles[1] = {
        family_id = "ranged",
        source_index = 4,
        source_sequence = 7,
        pos = Vec2.new(525, 315),
        dir = Vec2.new(1, 0),
        remaining_ticks = 40,
    }

    local events = {
        wrapped("crowded/commit/1", 1, {
            kind = "commit",
            tick = 20,
            family_id = "unarmed",
            source_index = 2,
            source_sequence = 1,
            x = 220,
            y = 190,
        }),
        wrapped("crowded/guarded/2", 2, {
            kind = "contact",
            tick = 20,
            family_id = "light_melee",
            source_index = 3,
            target_index = 7,
            source_sequence = 2,
            result = "guarded",
            x = 300,
            y = 230,
        }),
        wrapped("crowded/hit/3", 3, {
            kind = "contact",
            tick = 20,
            family_id = "ranged",
            source_index = 4,
            target_index = 8,
            source_sequence = 3,
            result = "hit",
            x = 306,
            y = 234,
        }),
        wrapped("crowded/spill/3", 4, {
            kind = "ball_spill",
            tick = 20,
            family_id = "ranged",
            source_index = 4,
            target_index = 8,
            source_sequence = 3,
            x = state.ball.x,
            y = state.ball.y,
        }),
        wrapped("crowded/forced/3", 5, {
            kind = "forced",
            tick = 20,
            family_id = "ranged",
            source_index = 4,
            target_index = 8,
            source_sequence = 3,
            x = 710,
            y = 360,
        }),
    }
    -- Screen-space reservations at the fixture's native 960x540 projection.
    -- They represent the controlled-player selection ring and the ranged aim
    -- lane, ensuring feedback glyph composition leaves both readable.
    local selection_occluders = {
        { x = 378, y = 342, w = 32, h = 24 },
    }
    local threat_occluders = {
        { x = 414, y = 348, w = 206, h = 14 },
    }
    local occluders = {
        selection_occluders[1],
        threat_occluders[1],
    }
    return {
        state = state,
        combat = combat_state,
        events = events,
        ball = { x = state.ball.x, y = state.ball.y },
        hud_rects = {
            { x = 696, y = 436, w = 240, h = 28 },
            { x = 696, y = 468, w = 240, h = 44 },
        },
        selection_occluders = selection_occluders,
        threat_occluders = threat_occluders,
        occluders = occluders,
    }
end

return fixture
