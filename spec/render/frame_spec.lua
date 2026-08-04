-- Contract tests for the sim-to-renderer boundary (#326).
--
-- These are tier-1 logic tests: no display, no love.graphics, no canvas. What
-- they pin is the payload's SHAPE and the promises the shape makes -- that the
-- per-entity data really is structure-of-arrays, that no engine type leaks
-- through it, and that building it cannot perturb the simulation.

local t = require("spec.support.runner")
local render_frame = require("render.frame")
local player_pose = require("render.player_pose")
local Vec2 = require("core.vec2")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local keeper = require("sim.keeper")
local teams = require("data.teams")

---@type MatchInput
local NO_INPUT = {
    move = Vec2.new(0, 0),
    shoot = false,
    shoot_held = false,
    pass = false,
    pass_held = false,
    switch = false,
    dash = false,
    dodge = false,
    lob = false,
    sprint = false,
    jockey = false,
    equipment_held = false,
    equipment_pressed = false,
    equipment_released = false,
}

---@return MatchState
local function fixture(seed)
    return match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        seed = seed or 17,
    })
end

-- Every per-frame player array, by name. Kept explicit rather than derived so
-- adding a field to the payload forces a conscious decision here: is the new
-- field per-entity, and did the version get bumped?
local PLAYER_ARRAYS = {
    "x",
    "y",
    "facing_x",
    "facing_y",
    "speed",
    "pose_id",
    "pose_priority",
    "pose_source",
    "controlled",
    "dashing",
    "holding",
    "dive",
    "dive_dir_x",
    "dive_dir_y",
    "grab",
    "throw",
    "windup",
    "aerial",
    "aerial_jump",
}

local SPARSE_PLAYER_ARRAYS = { "aerial_style", "aerial_outcome" }

t.describe("render frame payload", function()
    t.it("stamps the protocol version on the frame and the roster", function()
        local frame = render_frame.build(fixture())
        t.eq(frame.version, render_frame.VERSION)
        t.eq(frame.roster.version, render_frame.VERSION)
        t.is_true(render_frame.VERSION >= 1, "the payload must carry a bumpable integer version")
    end)

    t.it("keeps per-entity data as parallel scalar arrays, one entry per slot", function()
        local state = fixture()
        local frame = render_frame.build(state)
        local players = frame.players

        t.eq(players.count, #state.players)
        t.eq(frame.roster.count, #state.players)

        for _, name in ipairs(PLAYER_ARRAYS) do
            local array = players[name]
            t.is_true(type(array) == "table", name .. " must be an array")
            t.eq(#array, players.count, name .. " must have one entry per roster slot")
            for index = 1, players.count do
                local value = array[index]
                t.is_true(
                    type(value) == "number" or type(value) == "string" or type(value) == "boolean",
                    ("%s[%d] must be a scalar, got %s"):format(name, index, type(value))
                )
            end
        end

        -- The sparse arrays are allowed holes, but never a nested table: a
        -- buffer encoding maps an absent entry to a zero enum.
        for _, name in ipairs(SPARSE_PLAYER_ARRAYS) do
            for index = 1, players.count do
                local value = players[name][index]
                t.is_true(
                    value == nil or type(value) == "string",
                    name .. " must be a sparse array of scalars"
                )
            end
        end
    end)

    t.it("carries no engine types across the boundary", function()
        local frame = render_frame.build(fixture())
        ---@param value any
        ---@param path string
        local function assert_plain(value, path)
            local kind = type(value)
            if kind == "number" or kind == "string" or kind == "boolean" or kind == "nil" then
                return
            end
            t.is_true(kind == "table", path .. " is not plain old data (" .. kind .. ")")
            -- Vec2 and every class in this codebase carries a metatable; plain
            -- payload data never does.
            t.is_true(getmetatable(value) == nil, path .. " carries a metatable")
            for key, entry in pairs(value) do
                assert_plain(entry, path .. "." .. tostring(key))
            end
        end

        for key, section in pairs(frame) do
            -- `combat` is the documented exception: the combat telegraph model
            -- is carried through unflattened and still holds Vec2 positions.
            if key ~= "combat" then
                assert_plain(section, "frame." .. tostring(key))
            end
        end
    end)

    t.it("does not perturb the simulation", function()
        local state = fixture(29)
        for _ = 1, 30 do
            match.step(state, 1 / 60, NO_INPUT)
        end
        local before = match_snapshot.hash(match_snapshot.capture(state))
        local roster = render_frame.roster(state)
        for _ = 1, 5 do
            render_frame.build(state, { roster = roster })
            render_frame.hud(state, roster)
        end
        t.eq(match_snapshot.hash(match_snapshot.capture(state)), before)
    end)

    t.it("reuses a match-constant roster instead of rebuilding it", function()
        local state = fixture()
        local roster = render_frame.roster(state)
        local frame = render_frame.build(state, { roster = roster })
        t.is_true(frame.roster == roster, "a supplied roster must be carried, not copied")
        t.eq(roster.ids[1], state.players[1].id)
        t.eq(roster.teams[1], state.players[1].team)
        t.eq(roster.is_keeper[1], state.players[1].is_keeper)
        t.eq(#roster.species_color[1], 3)
    end)

    t.it("selects the same pose the pose authority does", function()
        local state = fixture(41)
        for _ = 1, 45 do
            match.step(state, 1 / 60, NO_INPUT)
        end
        local frame = render_frame.build(state)
        for index, player in ipairs(state.players) do
            local keeper_context = nil
            if player.is_keeper then
                keeper_context = {
                    near_ball = keeper.in_smother_range(player.pos:dist(state.ball)),
                    shuffling = player.keeper_state == "base"
                        and player.run_vel ~= nil
                        and math.abs(player.run_vel.y) > 0,
                    tip = false,
                }
            end
            local expected = player_pose.select(player, nil, keeper_context, {
                now = -state.time_left,
                containing = state.outfield_press[player.team].mode == "contain"
                    and state.outfield_press[player.team].presser_index == index,
                kick_follow = false,
            })
            t.eq(frame.players.pose_id[index], expected.id, "pose id for slot " .. index)
            t.eq(frame.players.pose_priority[index], expected.priority)
            t.eq(frame.players.pose_source[index], expected.source)
        end
    end)

    t.it("reports displayed positions while pose inputs stay authoritative", function()
        local state = fixture()
        local goalkeeper = state.players[1]
        goalkeeper.pos = Vec2.new(24, 270)
        goalkeeper.run_vel = Vec2.new(0, 0)
        -- Authoritatively inside smother range, displayed a long way away.
        state.ball = Vec2.new(goalkeeper.pos.x + 8, goalkeeper.pos.y)
        local displaced = { x = goalkeeper.pos.x + 300, y = goalkeeper.pos.y + 120 }
        ---@type CorrectionSmoothingPose
        local pose = {
            players = { [goalkeeper.id] = displaced },
            ball = { x = state.ball.x, y = state.ball.y },
        }

        local frame = render_frame.build(state, { render_pose = pose })
        t.eq(frame.players.x[1], displaced.x, "the payload reports where the avatar is drawn")
        t.eq(frame.players.y[1], displaced.y)
        t.eq(
            frame.players.pose_id[1],
            "keeper_ready_low",
            "smother range must be measured on the authoritative position"
        )
    end)

    t.it("takes the release follow-through window from the renderer", function()
        local state = fixture()
        local striker = state.players[4]
        striker.is_keeper = false

        local without = render_frame.build(state)
        t.is_true(without.players.pose_id[4] ~= "kick_follow")

        local with = render_frame.build(state, { kick_follow = { [striker.id] = true } })
        t.eq(with.players.pose_id[4], "kick_follow")
    end)

    t.it("hides the ground ball only while a keeper holds it in the hands", function()
        local state = fixture()
        local goalkeeper = state.players[1]
        state.owner = 1
        goalkeeper.feet_ball = false

        local held = render_frame.build(state)
        t.eq(held.ball.visible, false)
        t.eq(held.possession.keeper_holds, true)
        t.eq(held.players.holding[1], true)
        t.eq(held.possession.owner, 1)
        t.eq(held.possession.owner_team, goalkeeper.team)

        -- A back-pass at the keeper's feet is a dribbled ground ball like any other.
        goalkeeper.feet_ball = true
        local at_feet = render_frame.build(state)
        t.eq(at_feet.ball.visible, true)
        t.eq(at_feet.possession.keeper_holds, false)
        t.eq(at_feet.players.holding[1], false)
    end)

    t.it("solves the landing point only for a lofted loose ball", function()
        local state = fixture()
        state.owner = nil
        state.ball = Vec2.new(400, 270)
        state.ball_vel = Vec2.new(60, 0)
        state.ball_z = 90
        state.ball_vz = 0

        local lofted = render_frame.build(state)
        t.is_true(lofted.ball.landing_x ~= nil, "an airborne loose ball projects a landing point")
        t.is_true(assert(lofted.ball.landing_x) > state.ball.x, "it lands ahead of the ball")
        t.eq(lofted.ball.z, 90)

        state.ball_z = 0
        t.eq(render_frame.build(state).ball.landing_x, nil, "a grounded pass has no reticle")

        state.ball_z = 90
        state.owner = 2
        t.eq(render_frame.build(state).ball.landing_x, nil, "a carried ball has no reticle")
    end)

    t.it("reports one charge at a time for the controlled player", function()
        local state = fixture()
        local controlled = state.players[state.controlled]

        t.eq(render_frame.build(state).control.charge_kind, nil)

        controlled.pass_charge = 0.5
        local passing = render_frame.build(state)
        t.eq(passing.control.charge_kind, "pass")
        t.eq(passing.control.charge, 0.5)

        -- A shot charge outranks a pass charge, exactly as the meter draws it.
        controlled.charge = 0.8
        local shooting = render_frame.build(state)
        t.eq(shooting.control.charge_kind, "shot")
        t.eq(shooting.control.charge, 0.8)

        controlled.pass_target = 3
        t.eq(render_frame.build(state).control.pass_target, 3)
    end)

    t.it("flattens the frame's event batch into the effect-trigger channel", function()
        local state = fixture()
        local striker = state.players[4]
        ---@type MatchEvent[]
        local events = {
            { kind = "shot", x = 100, y = 200, player = striker.id, on_target = true },
            { kind = "reception", x = 300, y = 250, outcome = "clean" },
        }

        local frame = render_frame.build(state, { events = events })
        t.eq(frame.events.count, 2)
        t.eq(frame.events.kind[1], "shot")
        t.eq(frame.events.x[1], 100)
        t.eq(frame.events.y[1], 200)
        t.eq(frame.events.player[1], striker.id)
        t.eq(frame.events.slot[1], 4, "the payload resolves an event's player to a roster slot")
        t.eq(frame.events.on_target[1], 2, "a reported true encodes as 2")
        t.eq(frame.events.kind[2], "reception")
        t.eq(frame.events.outcome[2], "clean")
        t.eq(frame.events.player[2], nil)
        t.eq(frame.events.slot[2], nil)
    end)

    t.it("keeps absent, false and true distinguishable for optional booleans", function()
        local state = fixture()
        -- All three states occur for real: `sim/match.lua` sets `on_target`
        -- explicitly on a released shot, and a keeper's distribution kick is
        -- also `kind == "shot"` but reports nothing. Kind alone cannot tell
        -- them apart, so the payload must.
        ---@type MatchEvent[]
        local events = {
            { kind = "shot", x = 0, y = 0, on_target = true },
            { kind = "shot", x = 0, y = 0, on_target = false },
            { kind = "shot", x = 0, y = 0 },
            { kind = "header", x = 0, y = 0, jumping = false },
            { kind = "header", x = 0, y = 0 },
        }

        local flat = render_frame.build(state, { events = events }).events
        t.eq(flat.on_target[1], 2, "true")
        t.eq(flat.on_target[2], 1, "reported false is NOT the same as absent")
        t.eq(flat.on_target[3], 0, "absent")
        t.eq(flat.jumping[4], 1)
        t.eq(flat.jumping[5], 0)

        -- Tri-state arrays are dense, so a buffer write can copy the whole run.
        for index = 1, flat.count do
            t.eq(type(flat.on_target[index]), "number")
            t.eq(type(flat.jumping[index]), "number")
        end
    end)

    t.it("resolves a tip event into the drawn dive direction", function()
        local state = fixture()
        local goalkeeper = state.players[1]
        goalkeeper.pos = Vec2.new(24, 270)

        local up = render_frame.build(state, {
            events = {
                {
                    kind = "tip",
                    x = goalkeeper.pos.x,
                    y = goalkeeper.pos.y - 40,
                    player = goalkeeper.id,
                },
            },
        })
        t.eq(up.players.pose_id[1], "keeper_tip")
        t.eq(up.players.dive_dir_x[1], 0)
        t.eq(up.players.dive_dir_y[1], -1)

        local down = render_frame.build(state, {
            events = {
                {
                    kind = "tip",
                    x = goalkeeper.pos.x,
                    y = goalkeeper.pos.y + 40,
                    player = goalkeeper.id,
                },
            },
        })
        t.eq(down.players.dive_dir_y[1], 1)
    end)

    t.it("derives the scoreboard section from the simulation", function()
        local state = fixture()
        state.score.home = 2
        state.score.away = 1
        state.time_left = 65.4
        state.owner = state.controlled
        state.players[state.controlled].sprint_meter = 1.6

        local scoreboard = render_frame.hud(state)
        t.eq(scoreboard.home_score, 2)
        t.eq(scoreboard.away_score, 1)
        t.eq(scoreboard.time_left, 65.4)
        t.eq(scoreboard.possession_team, state.players[state.controlled].team)
        t.eq(scoreboard.controlled_owns_ball, true)
        t.eq(scoreboard.controlled_id, state.players[state.controlled].id)
        t.eq(scoreboard.controlled_stamina, 1, "stamina is clamped for the meter")

        state.owner = nil
        t.eq(render_frame.hud(state).possession_team, nil)

        -- `build` must publish exactly the same section it exposes on its own.
        local frame = render_frame.build(state)
        t.eq(frame.hud.home_score, 2)
        t.eq(frame.hud.controlled_id, scoreboard.controlled_id)
    end)

    t.it("normalises pose timers so no renderer re-derives a duration", function()
        local state = fixture()
        local player = state.players[3]
        player.is_keeper = false
        player.dive_timer = 0.9 -- far past the ease window
        player.grab_timer = 0.125
        player.throw_timer = 0.25
        player.aerial_timer = 0.11
        player.aerial_style = "chest_control"

        local players = render_frame.build(state).players
        t.eq(players.dive[3], 1, "an over-long timer clamps to 1")
        t.near(players.grab[3], 0.5, 1e-9)
        t.eq(players.throw[3], 1)
        t.near(players.aerial[3], 0.11 / 0.18, 1e-9)

        player.dive_timer = 0
        player.grab_timer = 0
        t.eq(render_frame.build(state).players.dive[3], 0)
    end)
end)
