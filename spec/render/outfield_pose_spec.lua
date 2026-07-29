local Vec2 = require("core.vec2")
local player_pose = require("game.presentation.player_pose")
local pitch = require("game.render.pitch")
local player_renderer = require("game.render.player_renderer")
local release_follow = require("game.render.release_follow")
local outfield_decision = require("sim.outfield_decision")
local offball_runs = require("sim.offball_runs")
local outfield_press = require("sim.outfield_press")
local replay = require("game.render.replay")
local match = require("sim.match")
local teams = require("data.teams")
local t = require("spec.support.runner")

local POSES = {
    "run_telegraph",
    "kick_follow",
    "settle",
    "contain",
    "tackle",
    "stumble",
    "fatigue",
}

-- The one authoritative overlap order, highest first. Everything below the
-- committed-combat band lives here; the combat band itself is asserted
-- separately so the two contracts stay visibly independent.
local PRIORITY_ORDER = {
    "soccer_windup",
    "slide",
    "tackle",
    "stumble",
    "kick_follow",
    "settle",
    "run_telegraph",
    "contain",
    "fatigue",
    "locomotion",
}

local NOW = 12.5

---@return table
local function stub_graphics()
    local graphics = {}
    local function noop() end
    for _, name in ipairs({
        "setColor",
        "setLineWidth",
        "setBlendMode",
        "setShader",
        "rectangle",
        "polygon",
        "line",
        "circle",
        "ellipse",
        "arc",
        "push",
        "pop",
        "translate",
        "rotate",
        "print",
        "printf",
    }) do
        graphics[name] = noop
    end
    graphics.getDimensions = function()
        return 1280, 720
    end
    graphics.getWidth = function()
        return 1280
    end
    graphics.getHeight = function()
        return 720
    end
    return graphics
end

---@param id PlayerPoseId
---@return string
local function draw_signature(id)
    local calls = {}
    local graphics = stub_graphics()
    for name, fn in pairs(graphics) do
        if
            type(fn) == "function"
            and name ~= "getDimensions"
            and name ~= "getWidth"
            and name ~= "getHeight"
        then
            graphics[name] = function(...)
                local values = { name }
                for _, value in ipairs({ ... }) do
                    if type(value) == "number" then
                        values[#values + 1] = ("%.3f"):format(value)
                    else
                        values[#values + 1] = tostring(value)
                    end
                end
                calls[#calls + 1] = table.concat(values, ":")
            end
        end
    end
    local saved = love.graphics
    love.graphics = graphics
    local ok, err = pcall(function()
        player_renderer.draw(
            200,
            300,
            12,
            { 0.2, 0.8, 1 },
            { px = 0, py = 0, speed = 42, phase = 1.1, lean = 0.2 },
            {
                facing = Vec2.new(1, 0),
                is_keeper = false,
                controlled = false,
                species_shape = "round",
                species_color = { 1, 0.7, 0.2 },
                team = "home",
                pose = {
                    id = id,
                    priority = player_pose.PRIORITY[id],
                    source = "soccer",
                },
            }
        )
    end)
    love.graphics = saved
    t.is_true(ok, id .. " draw error: " .. tostring(err))
    return table.concat(calls, "|")
end

---@return MatchState
local function fixture()
    return match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
    })
end

---@param player MatchPlayer
local function clear_signals(player)
    player.windup_timer = 0
    player.slide_timer = 0
    player.tackle_timer = 0
    player.stun_timer = 0
    player.settle_timer = 0
    player.jockey_timer = 0
    player.sprint_meter = 1
    player.sprinting = false
    player.outfield_decision = outfield_decision.new_state(1)
end

-- Put a granted, still-telegraphing run on the player, exactly as
-- `match._assign_runs` would have left it.
---@param player MatchPlayer
---@param elapsed number
local function grant_run(player, elapsed)
    player.outfield_decision = outfield_decision.refresh(
        outfield_decision.new_state(1),
        "offball",
        "in_behind",
        0.5,
        400,
        260,
        nil,
        NOW - elapsed + offball_runs.RUN_LIFETIME_SECONDS
    )
end

---@param player MatchPlayer
---@param id string
---@param context OutfieldPoseContext
local function enable(player, id, context)
    if id == "soccer_windup" then
        player.windup_timer = 0.1
    elseif id == "slide" then
        player.slide_timer = 0.1
    elseif id == "tackle" then
        player.tackle_timer = 0.1
    elseif id == "stumble" then
        player.stun_timer = 0.1
    elseif id == "kick_follow" then
        context.kick_follow = true
    elseif id == "settle" then
        player.settle_timer = 0.1
    elseif id == "run_telegraph" then
        grant_run(player, 0.05)
    elseif id == "contain" then
        player.jockey_timer = 0.1
    elseif id == "fatigue" then
        player.sprint_meter = 0
        player.sprinting = false
    end
end

---@param player MatchPlayer
---@param id string
---@param context OutfieldPoseContext
local function disable(player, id, context)
    if id == "soccer_windup" then
        player.windup_timer = 0
    elseif id == "slide" then
        player.slide_timer = 0
    elseif id == "tackle" then
        player.tackle_timer = 0
    elseif id == "stumble" then
        player.stun_timer = 0
    elseif id == "kick_follow" then
        context.kick_follow = false
    elseif id == "settle" then
        player.settle_timer = 0
    elseif id == "run_telegraph" then
        player.outfield_decision = outfield_decision.new_state(1)
    elseif id == "contain" then
        player.jockey_timer = 0
        context.containing = false
    elseif id == "fatigue" then
        player.sprint_meter = 1
    end
end

t.describe("outfield pose projection", function()
    t.it("selects every contract pose from simulation-owned state", function()
        local state = fixture()
        local player = state.players[2]
        local context = { now = NOW }

        clear_signals(player)
        t.eq(player_pose.select(player, nil, nil, context).id, "locomotion")

        for _, id in ipairs(POSES) do
            clear_signals(player)
            context.kick_follow = false
            context.containing = false
            enable(player, id, context)
            t.eq(player_pose.select(player, nil, nil, context).id, id, id)
        end
    end)

    t.it("reads the telegraph window from the simulation run grant", function()
        local state = fixture()
        local player = state.players[2]
        local context = { now = NOW }
        clear_signals(player)

        grant_run(player, 0)
        t.eq(player_pose.select(player, nil, nil, context).id, "run_telegraph")
        grant_run(player, offball_runs.TELEGRAPH_SECONDS * 0.5)
        t.eq(player_pose.select(player, nil, nil, context).id, "run_telegraph")
        -- The sustained run is the sprint trail's job, not a pose's.
        grant_run(player, offball_runs.TELEGRAPH_SECONDS)
        t.eq(player_pose.select(player, nil, nil, context).id, "locomotion")
        grant_run(player, offball_runs.RUN_LIFETIME_SECONDS * 0.5)
        t.eq(player_pose.select(player, nil, nil, context).id, "locomotion")

        -- A non-run intent never telegraphs, and neither does a caller who
        -- supplied no clock.
        clear_signals(player)
        player.outfield_decision = outfield_decision.refresh(
            outfield_decision.new_state(1),
            "offball",
            "move",
            0.5,
            400,
            260
        )
        t.eq(player_pose.select(player, nil, nil, context).id, "locomotion")
        grant_run(player, 0)
        t.eq(player_pose.select(player, nil, nil, {}).id, "locomotion")
    end)

    t.it("holds one explicit overlap order so stacked signals cannot flicker", function()
        local state = fixture()
        local player = state.players[2]
        local context = { now = NOW }
        clear_signals(player)

        for _, id in ipairs(PRIORITY_ORDER) do
            enable(player, id, context)
        end
        for index, id in ipairs(PRIORITY_ORDER) do
            t.eq(player_pose.select(player, nil, nil, context).id, id, "rank " .. index)
            disable(player, id, context)
        end

        -- The published table must agree with the order the specs assert.
        for index = 2, #PRIORITY_ORDER do
            local higher = player_pose.PRIORITY[PRIORITY_ORDER[index - 1]]
            local lower = player_pose.PRIORITY[PRIORITY_ORDER[index]]
            t.is_true(
                higher > lower,
                PRIORITY_ORDER[index - 1] .. " must outrank " .. PRIORITY_ORDER[index]
            )
        end
    end)

    t.it("keeps combat forced states and committed phases in front", function()
        local state = fixture()
        local player = state.players[2]
        local context = { now = NOW, kick_follow = true }
        clear_signals(player)
        for _, id in ipairs(PRIORITY_ORDER) do
            enable(player, id, context)
        end
        player.windup_timer = 0
        player.slide_timer = 0

        ---@type CombatPlayerPresentation
        local combat = {
            player_index = 2,
            player_id = player.id,
            phase = "recovery",
            phase_progress = 0.5,
            phase_ticks = 3,
            cooldown_ticks = 0,
            cooldown_fraction = 0,
            readiness = "ready",
            forced_ticks = 0,
            immunity_ticks = 0,
            position = player.pos,
            direction = player.facing,
        }
        t.eq(player_pose.select(player, combat, nil, context).id, "combat_recovery")
        combat.phase = "active"
        t.eq(player_pose.select(player, combat, nil, context).id, "combat_active")
        combat.forced_state = "stagger"
        combat.forced_ticks = 4
        t.eq(player_pose.select(player, combat, nil, context).id, "combat_stagger")
        combat.forced_state = "knockback"
        t.eq(player_pose.select(player, combat, nil, context).id, "combat_knockback")
    end)

    t.it("never lends an outfield pose to a goalkeeper", function()
        local state = fixture()
        local keeper = state.players[1]
        local neutral = { near_ball = false, shuffling = false, tip = false }
        local context = { now = NOW, kick_follow = true, containing = true }
        keeper.tackle_timer = 0.1
        keeper.stun_timer = 0.1
        keeper.settle_timer = 0.1
        keeper.jockey_timer = 0.1
        keeper.sprint_meter = 0
        grant_run(keeper, 0)
        t.eq(player_pose.select(keeper, nil, neutral, context).id, "keeper_ready_tall")
    end)

    t.it("maps both teams through the same state", function()
        local state = fixture()
        local context = { now = NOW }
        for _, index in ipairs({ 2, 7 }) do
            local player = state.players[index]
            for _, id in ipairs(POSES) do
                clear_signals(player)
                context.kick_follow = false
                context.containing = false
                enable(player, id, context)
                t.eq(player_pose.select(player, nil, nil, context).id, id, player.team .. " " .. id)
            end
        end
    end)

    t.it("shares the contain silhouette between AI contain and the human jockey", function()
        local state = fixture()
        local player = state.players[2]
        clear_signals(player)

        local ai_only = player_pose.select(player, nil, nil, { now = NOW, containing = true })
        player.jockey_timer = 0.2
        local jockey_only = player_pose.select(player, nil, nil, { now = NOW })
        t.eq(ai_only.id, "contain")
        t.eq(jockey_only.id, "contain")
        t.eq(ai_only.priority, jockey_only.priority)
        -- Same pose, separate mechanics: the pose module owns neither timer.
        t.eq(player.jockey_timer, 0.2)
    end)

    t.it("draws all seven contract poses with distinct procedural geometry", function()
        local signatures = {}
        for _, id in ipairs(POSES) do
            local signature = draw_signature(id)
            t.is_true(signatures[signature] == nil, id .. " duplicated another pose")
            signatures[signature] = id
        end
        -- And none of them collides with the locomotion fallback they replace.
        t.is_true(signatures[draw_signature("locomotion")] == nil, "locomotion was duplicated")
    end)
end)

t.describe("outfield release follow-through", function()
    t.it("holds a confirmed release past its one-tick event and then expires", function()
        release_follow.reset()
        t.eq(release_follow.active("zyro_vex"), false)

        release_follow.update({ { kind = "pass", x = 0, y = 0, player = "zyro_vex" } }, 0)
        t.eq(release_follow.active("zyro_vex"), true)
        release_follow.update({}, release_follow.DURATION * 0.5)
        t.eq(release_follow.active("zyro_vex"), true)
        release_follow.update({}, release_follow.DURATION * 0.6)
        t.eq(release_follow.active("zyro_vex"), false)

        release_follow.update({ { kind = "shot", x = 0, y = 0, player = "zyro_vex" } }, 0.016)
        t.eq(release_follow.active("zyro_vex"), true)
        release_follow.reset()
        t.eq(release_follow.active("zyro_vex"), false)
    end)

    t.it("opens no window for a non-release event", function()
        release_follow.reset()
        release_follow.update({
            { kind = "touch", x = 0, y = 0, player = "zyro_vex" },
            { kind = "tackle", x = 0, y = 0, player = "zyro_vex" },
            { kind = "shot", x = 0, y = 0 },
        }, 0.016)
        t.eq(release_follow.active("zyro_vex"), false)
    end)
end)

t.describe("outfield pose pitch projection", function()
    ---@param state MatchState
    ---@param opts table
    ---@return table<integer, string>
    local function selected_poses(state, opts)
        local saved_graphics = love.graphics
        local saved_draw = player_renderer.draw
        local poses = {}
        local index = 0
        love.graphics = stub_graphics()
        player_renderer.draw = function(_, _, _, _, _, draw_opts)
            index = index + 1
            poses[index] = draw_opts.pose.id
        end
        local ok, err = pcall(function()
            pitch.draw(state, { w = 1280, h = 720 }, opts)
        end)
        player_renderer.draw = saved_draw
        love.graphics = saved_graphics
        t.is_true(ok, "pitch outfield-pose projection error: " .. tostring(err))
        return poses
    end

    t.it("threads press mode, jockey, and a confirmed release through the renderer", function()
        release_follow.reset()
        local state = fixture()
        local base = {
            home_color = teams.nebula.color,
            away_color = teams.orion.color,
        }
        -- Players are drawn depth-sorted, so count the poses instead of
        -- assuming the away presser lands at a particular draw slot.
        state.outfield_press.away = outfield_press.contain(7)

        local jockey = state.players[3]
        jockey.jockey_timer = 0.3

        local striker = state.players[4]
        release_follow.update({ { kind = "shot", x = 0, y = 0, player = striker.id } }, 0)

        local counts = {}
        for _, id in ipairs(selected_poses(state, base)) do
            counts[id] = (counts[id] or 0) + 1
        end
        t.eq(counts.contain, 2, "AI contain and the human jockey each get one")
        t.eq(counts.kick_follow, 1)
        release_follow.reset()
    end)

    t.it("stays safe for the headless draw harness with every pose live", function()
        release_follow.reset()
        local state = fixture()
        for index, player in ipairs(state.players) do
            if not player.is_keeper then
                player.settle_timer = 0.1
                player.tackle_timer = (index % 2 == 0) and 0.1 or 0
                player.stun_timer = (index % 3 == 0) and 0.1 or 0
                player.jockey_timer = (index % 4 == 0) and 0.1 or 0
                player.sprint_meter = 0
                release_follow.update({
                    { kind = "pass", x = 0, y = 0, player = player.id },
                }, 0)
            end
        end
        local poses = selected_poses(state, {
            home_color = teams.nebula.color,
            away_color = teams.orion.color,
        })
        t.eq(#poses, #state.players)
        release_follow.reset()
    end)
end)

t.describe("outfield poses through the goal replay buffer", function()
    ---@param state MatchState
    ---@return table<string, integer>
    local function replayed_pose_counts(state)
        replay.reset()
        for _ = 1, 40 do
            replay.record(state)
        end
        t.is_true(replay.start("home"), "enough footage to start")

        local counts = {}
        local saved_graphics = love.graphics
        local saved_draw = player_renderer.draw
        love.graphics = stub_graphics()
        player_renderer.draw = function(_, _, _, _, _, draw_opts)
            local id = draw_opts.pose.id
            counts[id] = (counts[id] or 0) + 1
        end
        local ok, err = pcall(function()
            for _ = 1, 4000 do
                local frame = replay.step(1 / 60)
                if not frame then
                    break
                end
                pitch.draw(frame, { w = 1280, h = 720 }, {
                    home_color = teams.nebula.color,
                    away_color = teams.orion.color,
                })
            end
        end)
        player_renderer.draw = saved_draw
        love.graphics = saved_graphics
        replay.reset()
        t.is_true(ok, "replay pose draw error: " .. tostring(err))
        return counts
    end

    t.it("carries every outfield pose input through capture and playback", function()
        release_follow.reset()
        local state = fixture()
        state.outfield_press.away = outfield_press.contain(7)
        state.players[2].tackle_timer = 0.2
        state.players[3].settle_timer = 0.2
        state.players[4].stun_timer = 0.2
        state.players[5].sprint_meter = 0
        state.players[5].sprinting = false

        local counts = replayed_pose_counts(state)
        for _, id in ipairs({ "tackle", "settle", "stumble", "contain", "fatigue" }) do
            t.is_true((counts[id] or 0) > 0, id .. " never reached a replay frame")
        end
    end)

    t.it("pins a run grant the live tick keeps decrementing", function()
        release_follow.reset()
        local state = fixture()
        local runner = state.players[2]
        runner.outfield_decision = outfield_decision.refresh(
            outfield_decision.new_state(1),
            "offball",
            "in_behind",
            0.5,
            400,
            260,
            nil,
            -state.time_left + offball_runs.RUN_LIFETIME_SECONDS
        )
        local captured = runner.outfield_decision.remaining

        replay.reset()
        for _ = 1, 40 do
            replay.record(state)
        end
        -- `sim.match` decrements this field in place every tick; the buffered
        -- frame must not follow it.
        runner.outfield_decision.remaining = 0

        t.is_true(replay.start("home"))
        local frame = assert(replay.step(1 / 60))
        t.eq(frame.players[2].outfield_decision.remaining, captured)
        t.eq(
            frame.players[2].outfield_decision.run_expires_at,
            runner.outfield_decision.run_expires_at
        )
        t.eq(frame.outfield_press.home.mode, "inactive")
        replay.reset()
    end)
end)
