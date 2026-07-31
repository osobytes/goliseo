local Vec2 = require("core.vec2")
local player_pose = require("game.presentation.player_pose")
local pitch = require("game.render.pitch")
local player_renderer = require("game.render.player_renderer")
local match = require("sim.match")
local teams = require("data.teams")
local t = require("spec.support.runner")

local POSES = {
    "keeper_ready_tall",
    "keeper_ready_low",
    "keeper_shuffle",
    "keeper_set",
    "keeper_spread",
    "keeper_central",
    "keeper_stretch",
    "keeper_tip",
    "keeper_get_up",
}

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
            { px = 0, py = 0, speed = 24, phase = 1.1, gait = 0.25, lean = 0 },
            {
                facing = Vec2.new(1, 0),
                is_keeper = true,
                controlled = true,
                dive = 0.5,
                dive_dir = Vec2.new(0, -1),
                holding = id == "keeper_central",
                grab = id == "keeper_central" and 0.8 or 0,
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

t.describe("goalkeeper pose projection", function()
    t.it("selects every simulation-owned positioning and save state", function()
        local state = fixture()
        local keeper = state.players[1]
        local neutral = { near_ball = false, shuffling = false, tip = false }

        t.eq(player_pose.select(keeper, nil, neutral).id, "keeper_ready_tall")

        neutral.near_ball = true
        t.eq(player_pose.select(keeper, nil, neutral).id, "keeper_ready_low")
        neutral.near_ball = false
        neutral.shuffling = true
        t.eq(player_pose.select(keeper, nil, neutral).id, "keeper_shuffle")
        neutral.shuffling = false

        keeper.keeper_state = "set"
        t.eq(player_pose.select(keeper, nil, neutral).id, "keeper_set")
        -- "recover" is positioning recovery, not getting up off the floor.
        keeper.keeper_state = "recover"
        t.eq(player_pose.select(keeper, nil, neutral).id, "keeper_ready_tall")
        keeper.keeper_state = "base"

        keeper.keeper_get_up_timer = 0.18
        t.eq(player_pose.select(keeper, nil, neutral).id, "keeper_get_up")
        keeper.keeper_get_up_timer = 0
        t.eq(player_pose.select(keeper, nil, neutral).id, "keeper_ready_tall")

        keeper.dive_timer = 0.2
        for _, style in ipairs({ "spread", "central", "stretch" }) do
            keeper.save_style = style
            t.eq(player_pose.select(keeper, nil, neutral).id, "keeper_" .. style)
        end
        keeper.dive_timer = 0
        keeper.save_style = nil
        neutral.tip = true
        t.eq(player_pose.select(keeper, nil, neutral).id, "keeper_tip")
    end)

    t.it("reads getting up from the simulation timer, never the recover state", function()
        local state = fixture()
        local keeper = state.players[1]

        -- The get-up window outranks the set/lean and both ready postures while
        -- it runs, and hands every one of them back the moment it expires.
        for _, case in ipairs({
            {
                context = { near_ball = false, shuffling = false, tip = false },
                after = "keeper_ready_tall",
            },
            {
                context = { near_ball = true, shuffling = false, tip = false },
                after = "keeper_ready_low",
            },
            {
                context = { near_ball = false, shuffling = true, tip = false },
                after = "keeper_shuffle",
            },
        }) do
            keeper.keeper_get_up_timer = 0.18
            t.eq(player_pose.select(keeper, nil, case.context).id, "keeper_get_up")
            keeper.keeper_get_up_timer = 0
            t.eq(player_pose.select(keeper, nil, case.context).id, case.after)
        end

        local neutral = { near_ball = false, shuffling = false, tip = false }
        keeper.keeper_get_up_timer = 0.18
        keeper.keeper_state = "set"
        t.eq(player_pose.select(keeper, nil, neutral).id, "keeper_get_up")
        keeper.keeper_state = "base"

        -- Dive, tip, and hand poses all stay ahead of it.
        keeper.dive_timer = 0.2
        keeper.save_style = "stretch"
        t.eq(player_pose.select(keeper, nil, neutral).id, "keeper_stretch")
        t.eq(
            player_pose.select(keeper, nil, { near_ball = false, shuffling = false, tip = true }).id,
            "keeper_tip"
        )
        keeper.grab_timer = 0.1
        t.eq(player_pose.select(keeper, nil, neutral).id, "keeper_grab")
        keeper.grab_timer = 0
        keeper.dive_timer = 0
        keeper.save_style = nil

        -- Every keeper_state, including "recover", selects a ready posture once
        -- the get-up window is spent.
        keeper.keeper_get_up_timer = 0
        for _, keeper_state in ipairs({ "base", "advance", "contain", "retreat", "recover" }) do
            keeper.keeper_state = keeper_state
            t.eq(player_pose.select(keeper, nil, neutral).id, "keeper_ready_tall", keeper_state)
        end
    end)

    t.it("keeps grab, throw, and punt above save and ready variants", function()
        local state = fixture()
        local keeper = state.players[1]
        local context = { near_ball = true, shuffling = true, tip = true }
        keeper.dive_timer = 0.2
        keeper.save_style = "stretch"
        keeper.windup_timer = 0.1
        keeper.throw_timer = 0.1
        keeper.grab_timer = 0.1
        t.eq(player_pose.select(keeper, nil, context).id, "keeper_grab")

        keeper.grab_timer = 0
        t.eq(player_pose.select(keeper, nil, context).id, "keeper_throw")
        keeper.throw_timer = 0
        t.eq(player_pose.select(keeper, nil, context).id, "keeper_punt")
        keeper.windup_timer = 0
        t.eq(player_pose.select(keeper, nil, context).id, "keeper_tip")
    end)

    t.it("draws all nine contract poses with distinct procedural geometry", function()
        local signatures = {}
        for _, id in ipairs(POSES) do
            local signature = draw_signature(id)
            t.is_true(signatures[signature] == nil, id .. " duplicated another pose")
            signatures[signature] = id
        end
    end)

    t.it("threads the smother boundary and batched tip event through pitch draw", function()
        local state = fixture()
        local keeper = state.players[1]
        keeper.pos = Vec2.new(24, 270)
        keeper.run_vel = Vec2.new(0, 0)
        state.players[6].is_keeper = false
        state.ball = Vec2.new(50, 270)
        state.events = {}

        local saved_graphics = love.graphics
        local saved_draw = player_renderer.draw
        love.graphics = stub_graphics()
        local selected = nil
        player_renderer.draw = function(_, _, _, _, _, opts)
            if opts.is_keeper and selected == nil then
                selected = opts.pose.id
            end
        end
        local ok, err = pcall(function()
            pitch.draw(state, { w = 1280, h = 720 }, {
                home_color = teams.nebula.color,
                away_color = teams.orion.color,
            })
            t.eq(selected, "keeper_ready_low")

            selected = nil
            state.ball = Vec2.new(400, 270)
            pitch.draw(state, { w = 1280, h = 720 }, {
                home_color = teams.nebula.color,
                away_color = teams.orion.color,
                events = {
                    { kind = "tip", x = keeper.pos.x, y = keeper.pos.y - 40, player = keeper.id },
                },
            })
            t.eq(selected, "keeper_tip")
        end)
        player_renderer.draw = saved_draw
        love.graphics = saved_graphics
        t.is_true(ok, "pitch keeper-pose projection error: " .. tostring(err))
    end)
end)
