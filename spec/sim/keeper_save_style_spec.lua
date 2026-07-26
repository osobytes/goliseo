local t = require("spec.support.runner")
local fixed_clock = require("sim.fixed_clock")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local teams = require("data.teams")
local Vec2 = require("core.vec2")

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
}

---@return MatchState
---@return MatchPlayer
local function new_save_state()
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        seed = 73,
    })
    local keeper
    local parking_x = 80
    for _, player in ipairs(state.players) do
        if player.team == "away" and player.is_keeper then
            keeper = player
        elseif not player.is_keeper then
            player.pos = Vec2.new(parking_x, 60)
            parking_x = parking_x + 40
        end
    end
    keeper = assert(keeper)
    keeper.pos = Vec2.new(938, 220)
    keeper.run_vel = Vec2.new(0, 0)
    keeper.vel = Vec2.new(0, 0)
    state.owner = nil
    state.pickup_cd = 1
    state.block_grace = 1
    state.ball_z = 0
    state.ball_vz = 0
    return state, keeper
end

---@param distance number
---@param dive_distance number
---@param speed number?
---@return MatchState
---@return MatchPlayer
local function setup_attempt(distance, dive_distance, speed)
    local state, keeper = new_save_state()
    local horizontal_speed = speed or 500
    state.ball = Vec2.new(keeper.pos.x - distance, keeper.pos.y)
    state.ball_vel = Vec2.new(horizontal_speed, dive_distance * horizontal_speed / distance)
    return state, keeper
end

---@param state MatchState
---@param kind string
---@return MatchEvent?
local function event_of(state, kind)
    for _, event in ipairs(state.events) do
        if event.kind == kind then
            return event
        end
    end
    return nil
end

---@param state MatchState
---@return MatchEvent?
local function save_event(state)
    return event_of(state, "catch") or event_of(state, "parry")
end

t.describe("keeper save-style integration", function()
    t.it("propagates the exact 26 and 78 pixel spread boundaries", function()
        local smother_edge, edge_keeper = setup_attempt(26, 0)
        match.step(smother_edge, 0, NO_INPUT)
        local close_event = assert(save_event(smother_edge))
        t.eq(close_event.save_style, nil, "26px remains outside save-style classification")
        t.eq(edge_keeper.save_style, nil)

        local above_smother = setup_attempt(26.000001, 0)
        match.step(above_smother, 0, NO_INPUT)
        t.eq(assert(save_event(above_smother)).save_style, "spread")

        local spread_edge, spread_keeper = setup_attempt(78, 50)
        match.step(spread_edge, 0, NO_INPUT)
        t.eq(spread_keeper.save_style, "spread")

        local beyond_spread, beyond_keeper = setup_attempt(78.000001, 20)
        match.step(beyond_spread, 0, NO_INPUT)
        t.eq(beyond_keeper.save_style, "central")
    end)

    t.it("uses effective physical reach for the exact central/stretch boundary", function()
        local state, keeper = setup_attempt(100, 0)
        keeper.reach = 100
        state.ball_vel.y = 40 * state.ball_vel.x / 100
        match.step(state, 0, NO_INPUT)
        t.eq(keeper.save_style, "central")

        local beyond, beyond_keeper = setup_attempt(100, 0)
        beyond_keeper.reach = 100
        beyond.ball_vel.y = 40.000001 * beyond.ball_vel.x / 100
        match.step(beyond, 0, NO_INPUT)
        t.eq(beyond_keeper.save_style, "stretch")
    end)

    t.it("keeps identical geometry, outcome, RNG, and style deterministic", function()
        local first = setup_attempt(100, 35, 420)
        local second = setup_attempt(100, 35, 420)

        match.step(first, 0, NO_INPUT)
        match.step(second, 0, NO_INPUT)

        local first_event = save_event(first)
        local second_event = save_event(second)
        t.eq(first.rng, second.rng)
        t.eq(first.players[6].save_pending, second.players[6].save_pending)
        t.eq(first.players[6].save_style, second.players[6].save_style)
        if first_event or second_event then
            local first_resolved = assert(first_event)
            local second_resolved = assert(second_event)
            t.eq(first_resolved.kind, second_resolved.kind)
            t.eq(first_resolved.save_style, second_resolved.save_style)
        end
    end)

    t.it("keeps ground save RNG invariant for equivalent set contact", function()
        local legacy, legacy_keeper = setup_attempt(100, 35, 420)
        local explicit, explicit_keeper = setup_attempt(100, 35, 420)
        explicit_keeper.keeper_release_state = "set"
        explicit_keeper.keeper_release_kind = "ground"
        explicit_keeper.keeper_release_depth = 42
        explicit_keeper.keeper_release_motion = 0

        match.step(legacy, 0, NO_INPUT)
        match.step(explicit, 0, NO_INPUT)

        t.eq(explicit.rng, legacy.rng)
        t.eq(explicit_keeper.save_pending, legacy_keeper.save_pending)
        t.eq(explicit_keeper.dive_delay, legacy_keeper.dive_delay)
        t.eq(explicit_keeper.save_style, legacy_keeper.save_style)
    end)

    t.it("makes release-time movement no better than set at the same position", function()
        local set, set_keeper = setup_attempt(120, 70, 250)
        local moving, moving_keeper = setup_attempt(120, 70, 250)
        set_keeper.reach = 100
        moving_keeper.reach = 100
        set_keeper.handling = 1
        moving_keeper.handling = 1
        moving_keeper.keeper_release_motion = 1
        local moving_rng = moving.rng

        match.step(set, 0, NO_INPUT)
        match.step(moving, 0, NO_INPUT)

        t.is_true(set_keeper.save_pending ~= nil, "the set keeper reaches the ground shot")
        t.eq(moving_keeper.save_pending, nil, "planting consumes the moving keeper's dive budget")
        t.eq(moving.rng, moving_rng, "an unreachable shot never consumes catch/parry RNG")
    end)

    t.it("makes a bounded advance improve ground-angle coverage", function()
        ---@param keeper_x number
        ---@return MatchState
        ---@return MatchPlayer
        local function ground_attempt(keeper_x)
            local state, keeper = new_save_state()
            keeper.pos = Vec2.new(keeper_x, 270)
            keeper.reach = 45
            keeper.handling = 1
            local point_x = keeper_x - 120
            state.ball = Vec2.new(point_x, 270 + 55 * (point_x - 700) / 260)
            state.ball_vel = Vec2.new(260, 55):normalized():scale(250)
            return state, keeper
        end

        local deep, deep_keeper = ground_attempt(948)
        local advanced, advanced_keeper = ground_attempt(880)
        match.step(deep, 0, NO_INPUT)
        match.step(advanced, 0, NO_INPUT)

        t.eq(deep_keeper.save_pending, nil, "the deep keeper cannot reach the same corner ray")
        t.is_true(
            advanced_keeper.save_pending ~= nil,
            "the advanced plane narrows that ground angle"
        )
    end)

    t.it("pins save timing for the cross-runtime travel ratio", function()
        local ratio = 0.618234602637309
        local speed = 200
        local distance = ratio * speed / 1.2
        local state, keeper = setup_attempt(distance, 0, speed)

        match.step(state, 0, NO_INPUT)

        t.is_true(keeper.save_pending ~= nil)
        t.eq(match_snapshot.number_bytes(keeper.save_timer), "p:1:35314613:90526140")
    end)

    t.it("carries style on catch/parry then clears it at completed resolution", function()
        local state, keeper = setup_attempt(29, 0, 300)
        match.step(state, 0, NO_INPUT)

        local event = assert(save_event(state))
        t.eq(event.save_style, "spread")
        t.eq(keeper.save_style, nil)
    end)

    t.it("does not change the away-from-goal parry invariant", function()
        local state, keeper = setup_attempt(29, 20, 500)
        keeper.save_pending = "parry"
        keeper.save_timer = 1
        keeper.save_vx = state.ball_vel.x
        keeper.save_style = "spread"
        local before_score = state.score.home
        local before_rng = state.rng
        match.step(state, 0, NO_INPUT)

        local event = assert(save_event(state))
        t.eq(event.kind, "parry")
        t.eq(event.save_style, "spread")
        t.eq(state.score.home, before_score)
        t.eq(state.rng, before_rng)
        t.is_true(state.ball_vel.x < 0, "away keeper parries away from its goal")
        t.is_true(state.ball.x < state.goal_away.x, "parry stays outside the goal line")
        t.eq(keeper.save_style, nil)
    end)
end)

t.describe("keeper near-miss tip events", function()
    t.it(
        "emits once through exactly 110 percent of effective reach without side effects",
        function()
            local state, keeper = setup_attempt(120, 0)
            local effective_reach = keeper.reach -- includes the currently neutral species seam
            local dive_distance = effective_reach * 1.1
            state.ball_vel.y = dive_distance * state.ball_vel.x / 120
            local velocity = state.ball_vel
            local score = state.score.home
            local rng_state = state.rng
            local tip_count = 0

            for _ = 1, 3 do
                match.step(state, 0, NO_INPUT)
                if event_of(state, "tip") then
                    tip_count = tip_count + 1
                end
            end

            t.eq(tip_count, 1)
            t.is_true(keeper.save_tip_emitted)
            t.eq(state.owner, nil)
            t.eq(state.score.home, score)
            t.eq(state.rng, rng_state)
            t.eq(state.ball_vel.x, velocity.x)
            t.eq(state.ball_vel.y, velocity.y)
            t.eq(keeper.save_style, nil)
        end
    )

    t.it("emits nothing beyond the 110 percent boundary", function()
        local state, keeper = setup_attempt(120, 0)
        local dive_distance = keeper.reach * 1.1 + 0.000001
        state.ball_vel.y = dive_distance * state.ball_vel.x / 120

        match.step(state, 0, NO_INPUT)

        t.eq(event_of(state, "tip"), nil)
        t.is_true(not keeper.save_tip_emitted)
        t.eq(keeper.save_style, nil)
    end)

    t.it("does not turn movement debt inside physical reach into a tip", function()
        local state, keeper = setup_attempt(120, 65, 250)
        keeper.reach = 100
        keeper.keeper_release_motion = 1
        local rng_state = state.rng

        match.step(state, 0, NO_INPUT)

        t.eq(keeper.save_pending, nil, "the moving keeper cannot finish the plant in time")
        t.eq(event_of(state, "tip"), nil, "a late plant is a whiff, not physical glove contact")
        t.is_true(not keeper.save_tip_emitted)
        t.eq(state.rng, rng_state)
    end)
end)

t.describe("keeper save-style lifecycle", function()
    t.it("clears stale presentation state after a same-direction aerial strike", function()
        local state, keeper = new_save_state()
        local striker = state.players[state.controlled]
        striker.pos = Vec2.new(500, 220)
        striker.run_vel = Vec2.new(0, 0)
        striker.vel = Vec2.new(0, 0)
        striker.facing = Vec2.new(1, 0)
        striker.header_cd = 0
        striker.aerial_recovery = 0
        state.ball = striker.pos:add(Vec2.new(6, 0))
        state.ball_vel = Vec2.new(120, 0)
        state.ball_z = 56
        state.ball_vz = -50
        state.pickup_cd = 0
        state.aerial_lock = 0
        keeper.save_pending = "parry"
        keeper.save_timer = 2
        keeper.save_vx = state.ball_vel.x
        keeper.dive_delay = 0.5
        keeper.save_style = "stretch"
        keeper.save_tip_emitted = true
        local strike_input = {
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
            aerial_strike = true,
        }

        match.step(state, 1 / 60, strike_input)

        local strike = event_of(state, "header") or event_of(state, "volley")
        t.is_true(strike ~= nil and strike.outcome ~= "miss", "the aerial strike redirects play")
        t.is_true(state.ball_vel.x > 0, "the redirected ball keeps the original x direction")
        t.eq(keeper.save_pending, "parry", "the existing save verdict remains outcome-invariant")
        t.is_true(keeper.dive_delay > 0, "the existing dive timing remains outcome-invariant")
        t.eq(keeper.save_style, nil)
        t.is_true(not keeper.save_tip_emitted)

        local resolved
        for _ = 1, 150 do
            match.step(state, 1 / 60, NO_INPUT)
            resolved = save_event(state)
            if resolved then
                break
            end
        end
        local parry = assert(resolved)
        t.eq(parry.kind, "parry")
        t.eq(parry.save_style, nil, "the stale incoming style never reaches the redirected event")
    end)

    t.it("clears style when a pending shot reverses or dies before contact", function()
        local reversed, reversed_keeper = new_save_state()
        reversed.ball = Vec2.new(800, 220)
        reversed.ball_vel = Vec2.new(-300, 0)
        reversed_keeper.save_pending = "catch"
        reversed_keeper.save_timer = 1
        reversed_keeper.save_vx = 300
        reversed_keeper.save_style = "central"
        reversed_keeper.save_tip_emitted = true
        match.step(reversed, 0, NO_INPUT)
        t.eq(reversed_keeper.save_style, nil)
        t.is_true(not reversed_keeper.save_tip_emitted)

        local dead, dead_keeper = new_save_state()
        dead.ball = Vec2.new(800, 220)
        dead.ball_vel = Vec2.new(20, 0)
        dead_keeper.save_pending = "catch"
        dead_keeper.save_timer = 1
        dead_keeper.save_vx = 300
        dead_keeper.save_style = "central"
        dead_keeper.save_tip_emitted = true
        match.step(dead, 0, NO_INPUT)
        t.eq(dead_keeper.save_style, nil)
        t.is_true(not dead_keeper.save_tip_emitted)
    end)

    t.it("clears style at recovery completion and when possession cancels the attempt", function()
        local recovery, recovery_keeper = new_save_state()
        recovery.owner = 2
        recovery_keeper.dive_timer = 0.01
        recovery_keeper.save_style = "stretch"
        recovery_keeper.save_tip_emitted = true
        match.step(recovery, 0.02, NO_INPUT)
        t.eq(recovery_keeper.save_style, nil)
        t.is_true(not recovery_keeper.save_tip_emitted)

        local owned, owned_keeper = new_save_state()
        owned.owner = 2
        owned_keeper.save_style = "central"
        owned_keeper.save_tip_emitted = true
        match.step(owned, 0, NO_INPUT)
        t.eq(owned_keeper.save_style, nil)
        t.is_true(not owned_keeper.save_tip_emitted)
    end)

    t.it("clears style and the one-shot guard on kickoff reset", function()
        local state, keeper = new_save_state()
        keeper.save_style = "stretch"
        keeper.save_tip_emitted = true
        keeper.receive_timer = 1
        state.ball = Vec2.new(965, 270)
        state.ball_vel = Vec2.new(600, 0)
        state.ball_z = 0
        state.ball_vz = 0

        match.step(state, 0.01, NO_INPUT)

        t.eq(state.score.home, 1)
        t.eq(keeper.save_style, nil)
        t.is_true(not keeper.save_tip_emitted)
    end)
end)

-- The get-up window is the ONLY signal the presentation layer may read for the
-- post-dive recovery pose, so the simulation owns arming, decay, and reset.
t.describe("keeper post-dive get-up window", function()
    t.it("arms the window on the tick the dive lunge ends", function()
        local state, keeper = new_save_state()
        keeper.dive_timer = 2 * fixed_clock.TICK_SECONDS
        keeper.dive_dir = Vec2.new(0, 1)
        t.eq(keeper.keeper_get_up_timer, 0)

        match.step(state, fixed_clock.TICK_SECONDS, NO_INPUT)
        t.eq(keeper.dive_timer, fixed_clock.TICK_SECONDS)
        t.eq(keeper.keeper_get_up_timer, 0, "an unfinished dive never arms the get-up window")

        match.step(state, fixed_clock.TICK_SECONDS, NO_INPUT)
        t.eq(keeper.dive_timer, 0)
        t.eq(keeper.keeper_get_up_timer, 0.18)
    end)

    t.it("decays the armed window to exactly zero over 0.18 simulated seconds", function()
        local state, keeper = new_save_state()
        keeper.keeper_get_up_timer = 0.18

        local ticks = 0
        while keeper.keeper_get_up_timer > 0 do
            match.step(state, fixed_clock.TICK_SECONDS, NO_INPUT)
            ticks = ticks + 1
            t.is_true(ticks <= 12, "the get-up window outlived its budget")
        end
        t.eq(keeper.keeper_get_up_timer, 0)
        t.eq(ticks, 11) -- ceil(0.18 * 60)

        local exact, exact_keeper = new_save_state()
        exact_keeper.keeper_get_up_timer = 0.18
        match.step(exact, 0.18, NO_INPUT)
        t.eq(exact_keeper.keeper_get_up_timer, 0)
    end)

    t.it("survives a canonical snapshot capture and restore", function()
        local state, keeper = new_save_state()
        keeper.keeper_get_up_timer = 0.18

        local boundary = match_snapshot.capture(state)
        local restored = match_snapshot.restore(boundary)
        local restored_keeper
        for _, player in ipairs(restored.players) do
            if player.team == "away" and player.is_keeper then
                restored_keeper = player
            end
        end
        t.eq(assert(restored_keeper).keeper_get_up_timer, 0.18)
        t.is_true(
            match_snapshot.encode(boundary):find("k19:keeper_get_up_timer;", 1, true) ~= nil,
            "the get-up window is missing from the canonical wire"
        )
        t.eq(match_snapshot.hash(match_snapshot.capture(restored)), match_snapshot.hash(boundary))
    end)

    t.it("clears the window on kickoff reset", function()
        local state, keeper = new_save_state()
        keeper.keeper_get_up_timer = 0.18
        state.ball = Vec2.new(965, 270)
        state.ball_vel = Vec2.new(600, 0)

        match.step(state, fixed_clock.TICK_SECONDS, NO_INPUT)

        t.eq(state.score.home, 1)
        t.eq(keeper.keeper_get_up_timer, 0)
    end)
end)
