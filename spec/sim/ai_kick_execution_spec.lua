local t = require("spec.support.runner")
local Vec2 = require("core.vec2")
local fixed_clock = require("sim.fixed_clock")
local rng = require("core.rng")
local input_frame = require("sim.input_frame")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local stats = require("sim.stats")
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

---@param technique integer
---@return StatBlock
local function stat_block(technique)
    return {
        pace = 5,
        strength = 5,
        technique = technique,
        stamina = 5,
        mental = 5,
    }
end

---@param player MatchPlayer
---@param technique integer
---@param mental integer?
local function set_execution_technique(player, technique, mental)
    local block = stat_block(technique)
    block.mental = mental or block.mental
    player.first_touch = stats.first_touch(block)
    player.composure = stats.composure(block)
end

---@param options { seed: integer?, human_controlled: boolean?, slot_mode: boolean? }?
---@return MatchState
local function new_match(options)
    options = options or {}
    local ownership = options.slot_mode and match.ownership_for_teams(teams.nebula, teams.orion)
        or nil
    return match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        seed = options.seed or 54,
        human_controlled = options.human_controlled,
        input_ownership = ownership,
    })
end

---@param state MatchState
---@param kind MatchEventKind
---@return boolean
local function has_event(state, kind)
    for _, event in ipairs(state.events) do
        if event.kind == kind then
            return true
        end
    end
    return false
end

---@param intended Vec2
---@param actual Vec2
---@return number radians
local function signed_angle(intended, actual)
    local cross = intended.x * actual.y - intended.y * actual.x
    local dot = intended.x * actual.x + intended.y * actual.y
    return math.atan2(cross, dot)
end

---@param state MatchState
---@return integer carrier_index
---@return integer target_index
local function setup_ai_pass(state)
    local carrier ---@type integer?
    local target ---@type integer?
    for index, player in ipairs(state.players) do
        if player.team == "away" and not player.is_keeper then
            if not carrier then
                carrier = index
            elseif not target then
                target = index
            end
        end
    end
    carrier = assert(carrier)
    target = assert(target)
    state.players[carrier].pos = Vec2.new(600, 270)
    state.players[carrier].anchor = state.players[carrier].pos
    state.players[carrier].vel = Vec2.new(0, 0)
    state.players[carrier].run_vel = Vec2.new(0, 0)
    state.players[carrier].facing = Vec2.new(-1, 0)
    state.players[target].pos = Vec2.new(450, 200)
    state.players[target].anchor = state.players[target].pos
    for index, player in ipairs(state.players) do
        if
            player.team == "away"
            and not player.is_keeper
            and index ~= carrier
            and index ~= target
        then
            player.pos = Vec2.new(840, 100 + index * 30)
            player.anchor = player.pos
        elseif player.team == "home" and not player.is_keeper then
            player.pos = Vec2.new(850, 40 + index * 30)
            player.anchor = player.pos
        end
        player.vel = Vec2.new(0, 0)
        player.run_vel = Vec2.new(0, 0)
    end
    -- One defender supplies pass pressure without entering steal range.
    state.players[state.controlled].pos = Vec2.new(655, 270)
    state.players[state.controlled].anchor = state.players[state.controlled].pos
    state.owner = carrier
    state.ball = Vec2.new(582, 270)
    state.ball_vel = Vec2.new(0, 0)
    state.pickup_cd = 1
    state.kickoff_hold = 0
    return carrier, target
end

---@param state MatchState
---@return integer carrier_index
---@return integer target_index
local function setup_ai_cross(state)
    local carrier ---@type integer?
    local target ---@type integer?
    for index, player in ipairs(state.players) do
        if player.team == "away" and not player.is_keeper then
            if not carrier then
                carrier = index
            elseif not target then
                target = index
            end
        end
    end
    carrier = assert(carrier)
    target = assert(target)
    state.players[carrier].pos = Vec2.new(300, 100)
    state.players[carrier].anchor = state.players[carrier].pos
    state.players[carrier].vel = Vec2.new(0, 0)
    state.players[carrier].run_vel = Vec2.new(0, 0)
    state.players[carrier].facing = Vec2.new(-1, 0)
    state.players[target].pos = Vec2.new(150, 250)
    state.players[target].anchor = state.players[target].pos
    for index, player in ipairs(state.players) do
        if
            player.team == "away"
            and not player.is_keeper
            and index ~= carrier
            and index ~= target
        then
            player.pos = Vec2.new(700, 100 + index * 30)
            player.anchor = player.pos
        elseif player.team == "home" and not player.is_keeper then
            player.pos = Vec2.new(820, 60 + index * 30)
            player.anchor = player.pos
        end
        player.vel = Vec2.new(0, 0)
        player.run_vel = Vec2.new(0, 0)
    end
    state.owner = carrier
    state.ball = state.players[carrier].pos:add(Vec2.new(-18, 0))
    state.ball_vel = Vec2.new(0, 0)
    state.pickup_cd = 1
    state.kickoff_hold = 0
    return carrier, target
end

---@param technique integer
---@param moving_target boolean?
---@return MatchState
---@return integer
local function release_ai_pass(technique, moving_target)
    local state = new_match({ seed = 67 })
    local carrier, target = setup_ai_pass(state)
    set_execution_technique(state.players[carrier], technique)
    if moving_target then
        state.players[target].vel = Vec2.new(-45, 80)
    end
    local expected_rng = rng.roll(state.rng)
    match.step(state, 0, NO_INPUT)
    t.is_true(has_event(state, "pass"), "fixture must immediately release an AI pass")
    t.eq(state.rng, expected_rng, "an actual AI pass consumes exactly one execution draw")
    return state, target
end

---@param technique integer
---@return MatchState
---@return integer
local function release_ai_cross(technique)
    local state = new_match({ seed = 71 })
    local carrier, target = setup_ai_cross(state)
    set_execution_technique(state.players[carrier], technique)
    local expected_rng = rng.roll(state.rng)
    match.step(state, 0, NO_INPUT)
    t.is_true(has_event(state, "pass"), "fixture must immediately release an AI cross")
    t.is_true(state.ball_vz > 0, "fixture must use the lofted cross path")
    t.eq(state.rng, expected_rng, "an actual AI cross consumes exactly one execution draw")
    return state, target
end

t.describe("AI outfield kick execution", function()
    t.it("uses the same normalized draw deterministically and tightens monotonically", function()
        local intended = Vec2.new(500, -120)
        local previous = math.huge
        for technique = 0, 10 do
            local state = new_match({ seed = 83, human_controlled = false })
            local owner_index = assert(state.owner)
            local owner = state.players[owner_index]
            set_execution_technique(owner, technique)
            local execution_error =
                stats.execution_error_from_outfield(owner.first_touch, owner.composure)
            local before_rng = state.rng
            local expected_rng, sample = rng.roll(before_rng)
            local actual = match._apply_ai_outfield_execution_error(state, owner_index, intended)
            local error = math.abs(signed_angle(intended, actual))

            t.eq(state.rng, expected_rng, "every eligible release consumes one draw")
            t.near(error, math.abs((sample * 2 - 1) * execution_error), 1e-12)
            t.is_true(error <= previous + 1e-12, "higher technique cannot widen matched error")
            t.near(actual:length(), intended:length(), 1e-10, "rotation preserves kick speed")
            previous = error
            if technique == 10 then
                t.eq(execution_error, 0)
                t.near(error, 0, 1e-12, "maximum technique rotates by zero")
            end
        end

        local first = new_match({ seed = 83, human_controlled = false })
        local second = new_match({ seed = 83, human_controlled = false })
        local first_owner = assert(first.owner)
        local second_owner = assert(second.owner)
        local a = match._apply_ai_outfield_execution_error(first, first_owner, intended)
        local b = match._apply_ai_outfield_execution_error(second, second_owner, intended)
        t.eq(first.rng, second.rng)
        t.eq(a.x, b.x)
        t.eq(a.y, b.y)
    end)

    t.it("rotates ordinary and lead passes only after target and pace are fixed", function()
        local zero, zero_target = release_ai_pass(10)
        local noisy, noisy_target = release_ai_pass(0)
        t.eq(noisy_target, zero_target)
        t.is_true(noisy.players[noisy_target].receive_timer > 0)
        t.near(noisy.ball_vel:length(), zero.ball_vel:length(), 1e-10)
        t.eq(noisy.ball_vz, zero.ball_vz)
        t.is_true(math.abs(signed_angle(zero.ball_vel, noisy.ball_vel)) > 0)

        local static = release_ai_pass(10)
        local leading = release_ai_pass(10, true)
        t.is_true(
            math.abs(signed_angle(static.ball_vel, leading.ball_vel)) > 0.01,
            "moving-receiver lead must be present before execution rotation"
        )
        local leading_noisy = release_ai_pass(0, true)
        t.near(leading_noisy.ball_vel:length(), leading.ball_vel:length(), 1e-10)
        t.eq(leading_noisy.ball_vz, leading.ball_vz)
    end)

    t.it("rotates crosses only after their target, horizontal pace, and loft are fixed", function()
        local zero, zero_target = release_ai_cross(10)
        local noisy, noisy_target = release_ai_cross(0)
        t.eq(noisy_target, zero_target)
        t.is_true(noisy.players[noisy_target].receive_timer > 0)
        t.near(noisy.ball_vel:length(), zero.ball_vel:length(), 1e-10)
        t.eq(noisy.ball_vz, zero.ball_vz)
        t.is_true(math.abs(signed_angle(zero.ball_vel, noisy.ball_vel)) > 0)
    end)

    t.it("draws only when a delayed AI shot actually releases", function()
        local state = new_match({ seed = 89 })
        local owner_index ---@type integer?
        local parking = 0
        for index, player in ipairs(state.players) do
            if player.team == "away" and not player.is_keeper and not owner_index then
                owner_index = index
                player.pos = Vec2.new(200, 270)
                player.facing = Vec2.new(-1, 0)
            elseif player.team == "home" and not player.is_keeper then
                player.pos = Vec2.new(700 + parking * 40, 60)
                parking = parking + 1
            end
            player.vel = Vec2.new(0, 0)
            player.run_vel = Vec2.new(0, 0)
            player.anchor = player.pos
        end
        owner_index = assert(owner_index)
        local owner = state.players[owner_index]
        set_execution_technique(owner, 2)
        state.owner = owner_index
        state.ball = Vec2.new(182, 270)
        state.ball_vel = Vec2.new(0, 0)
        local before_rng = state.rng

        match.step(state, 0, NO_INPUT)
        t.eq(state.rng, before_rng, "candidate scoring and shot commit consume no execution draw")
        t.is_true(owner.windup_shot ~= nil, "AI shooting fixture must commit a delayed shot")
        t.is_true(not has_event(state, "shot"))

        local pending = assert(owner.windup_shot)
        pending.speed = 777
        pending.vz = 123
        pending.spin = 41
        pending.shot_type = "chip"
        owner.windup_timer = 0
        local expected_rng = rng.roll(before_rng)

        match.step(state, 0, NO_INPUT)

        t.is_true(has_event(state, "shot"))
        t.eq(state.rng, expected_rng)
        t.eq(state.owner, nil)
        t.near(state.ball_vel:length(), 777, 1e-10)
        t.eq(state.ball_vz, 123)
        t.eq(state.ball_spin, 41)
        for _, event in ipairs(state.events) do
            if event.kind == "shot" then
                t.eq(event.shot_type, "chip")
            end
        end
    end)

    t.it("reconstructs the same release after canonical snapshot restore", function()
        local original = new_match({ seed = 93, human_controlled = false })
        local owner_index = assert(original.owner)
        local owner = original.players[owner_index]
        set_execution_technique(owner, 3, 8)
        owner.windup_timer = 0
        owner.windup_shot = {
            dir = Vec2.new(1, 0.35),
            speed = 654,
            vz = 87,
            spin = -29,
            shot_type = "chip",
        }
        local snapshot = match_snapshot.capture(original)
        local restored = match_snapshot.restore(snapshot)
        local rollback_boundary = match_snapshot.capture_owned(restored)
        local rollback_restored = match_snapshot.restore_owned(rollback_boundary)
        t.is_true(
            not table.concat(match_snapshot.PLAYER_FIELDS, ","):find("execution_error", 1, true),
            "derived execution error must not add snapshot state"
        )

        match.step(original, 0, NO_INPUT)
        match.step(restored, 0, NO_INPUT)
        match.step(rollback_restored, 0, NO_INPUT)

        t.eq(restored.rng, original.rng)
        t.eq(restored.ball_vel.x, original.ball_vel.x)
        t.eq(restored.ball_vel.y, original.ball_vel.y)
        t.eq(restored.ball_vz, original.ball_vz)
        t.eq(restored.ball_spin, original.ball_spin)
        t.eq(
            match_snapshot.hash(match_snapshot.capture(restored)),
            match_snapshot.hash(match_snapshot.capture(original))
        )
        t.eq(rollback_restored.rng, original.rng)
        t.eq(rollback_restored.ball_vel.x, original.ball_vel.x)
        t.eq(rollback_restored.ball_vel.y, original.ball_vel.y)
        t.eq(
            match_snapshot.hash(match_snapshot.capture_owned(rollback_restored)),
            match_snapshot.hash(match_snapshot.capture_owned(original))
        )
    end)

    t.it("does not draw when a tackle cancels an AI wind-up", function()
        local state = new_match({ seed = 97 })
        local carrier_index ---@type integer?
        for index, player in ipairs(state.players) do
            if player.team == "away" and not player.is_keeper then
                carrier_index = index
                break
            end
        end
        carrier_index = assert(carrier_index)
        local carrier = state.players[carrier_index]
        carrier.pos = Vec2.new(300, 270)
        carrier.facing = Vec2.new(-1, 0)
        carrier.vel = Vec2.new(0, 0)
        carrier.run_vel = Vec2.new(0, 0)
        carrier.windup_timer = 0.12
        carrier.windup_shot = {
            dir = Vec2.new(-1, 0),
            speed = 500,
            vz = 0,
            spin = 0,
            shot_type = "ground",
        }
        state.owner = carrier_index
        state.ball = carrier.pos:add(carrier.facing:scale(18))
        for index, player in ipairs(state.players) do
            if player.team == "away" and not player.is_keeper and index ~= carrier_index then
                player.pos = Vec2.new(40, 380 + index * 15)
            end
        end
        local challenger = state.players[state.controlled]
        challenger.pos = Vec2.new(carrier.pos.x - 24, carrier.pos.y)
        challenger.vel = Vec2.new(0, 0)
        challenger.run_vel = Vec2.new(0, 0)
        local before_rng = state.rng
        local tackle = {}
        for key, value in pairs(NO_INPUT) do
            tackle[key] = value
        end
        tackle.dash = true
        ---@cast tackle MatchInput

        match.step(state, 0.016, tackle)

        t.eq(state.rng, before_rng)
        t.is_true(not has_event(state, "shot"))
        t.eq(carrier.windup_shot, nil)
    end)

    t.it("leaves human, fixed-slot, and keeper releases outside the draw contract", function()
        local intended = Vec2.new(400, 90)

        local human = new_match({ seed = 101 })
        local human_owner = assert(human.owner)
        local human_rng = human.rng
        local human_velocity =
            match._apply_ai_outfield_execution_error(human, human_owner, intended)
        t.eq(human.rng, human_rng)
        t.eq(human_velocity, intended)

        local fixed = new_match({ seed = 103, slot_mode = true })
        local fixed_owner = assert(fixed.owner)
        local fixed_rng = fixed.rng
        local fixed_velocity =
            match._apply_ai_outfield_execution_error(fixed, fixed_owner, intended)
        t.eq(fixed.rng, fixed_rng)
        t.eq(fixed_velocity, intended)

        local keeper_state = new_match({ seed = 107, human_controlled = false })
        local keeper_index = 1
        local keeper_rng = keeper_state.rng
        local keeper_velocity =
            match._apply_ai_outfield_execution_error(keeper_state, keeper_index, intended)
        t.eq(keeper_state.rng, keeper_rng)
        t.eq(keeper_velocity, intended)

        local human_release = new_match({ seed = 111 })
        local human_index = assert(human_release.owner)
        local human_player = human_release.players[human_index]
        human_player.windup_timer = 0
        human_player.windup_shot = {
            dir = Vec2.new(1, 0.1),
            speed = 611,
            vz = 0,
            spin = 33,
            shot_type = "ground",
        }
        local human_release_rng = human_release.rng
        match.step(human_release, 0, NO_INPUT)
        t.is_true(has_event(human_release, "shot"))
        t.eq(human_release.rng, human_release_rng)

        local human_pass = new_match({ seed = 112 })
        local human_pass_rng = human_pass.rng
        local pass_input = {}
        for key, value in pairs(NO_INPUT) do
            pass_input[key] = value
        end
        pass_input.pass = true
        ---@cast pass_input MatchInput
        match.step(human_pass, 0, pass_input)
        t.is_true(has_event(human_pass, "pass"))
        t.eq(human_pass.rng, human_pass_rng)

        local fixed_release = new_match({ seed = 113, slot_mode = true })
        local fixed_index = assert(fixed_release.owner)
        local fixed_slot = assert(fixed_release.slot_for_player[fixed_index])
        local frame = assert(input_frame.neutral(0))
        frame.slots[fixed_slot] =
            assert(input_frame.new_sample({ edges = input_frame.EDGE_BITS.pass }))
        local fixed_release_rng = fixed_release.rng
        match.step(fixed_release, fixed_clock.TICK_SECONDS, frame)
        t.is_true(has_event(fixed_release, "pass"))
        t.eq(fixed_release.rng, fixed_release_rng)

        local keeper_release = new_match({ seed = 127, human_controlled = false })
        keeper_release.owner = 1
        keeper_release.ball = keeper_release.players[1].pos
        keeper_release.players[1].windup_timer = 0
        keeper_release.players[1].windup_shot = {
            dir = Vec2.new(1, -0.1),
            speed = 680,
            vz = 0,
            spin = 0,
            shot_type = "ground",
        }
        local keeper_release_rng = keeper_release.rng
        match.step(keeper_release, 0, NO_INPUT)
        t.is_true(has_event(keeper_release, "shot"))
        t.eq(keeper_release.rng, keeper_release_rng)
    end)

    t.it("keeps non-release AI work and aerial resolution off the execution seam", function()
        local offball = new_match({ seed = 109 })
        local before_rng = offball.rng
        match.step(offball, 0, NO_INPUT)
        t.eq(offball.rng, before_rng, "selection and off-ball assignment cannot draw release noise")

        local carrier_choice = new_match({ seed = 131, human_controlled = false })
        local carrier_index = assert(carrier_choice.owner)
        local carrier = carrier_choice.players[carrier_index]
        carrier.pos = Vec2.new(480, 270)
        carrier.anchor = carrier.pos
        carrier.vel = Vec2.new(0, 0)
        carrier.run_vel = Vec2.new(0, 0)
        carrier_choice.ball = carrier.pos:add(Vec2.new(18, 0))
        for index, player in ipairs(carrier_choice.players) do
            if index ~= carrier_index then
                if player.team == carrier.team then
                    player.pos = Vec2.new(20, 20 + index * 18)
                else
                    player.pos = Vec2.new(900, 20 + index * 18)
                end
                player.anchor = player.pos
                player.vel = Vec2.new(0, 0)
                player.run_vel = Vec2.new(0, 0)
            end
        end
        local carrier_choice_rng = carrier_choice.rng
        match.step(carrier_choice, 0, NO_INPUT)
        t.eq(carrier_choice.owner, carrier_index)
        t.eq(carrier.outfield_decision.intent, "dribble")
        t.eq(
            carrier_choice.rng,
            carrier_choice_rng,
            "AI option construction and carrier choice cannot draw execution noise"
        )

        local source = assert(love.filesystem.read("sim/match.lua"))
        local calls = 0
        for _ in source:gmatch("match%._apply_ai_outfield_execution_error%(") do
            calls = calls + 1
        end
        t.eq(calls, 3, "one definition plus only the shared shot and pass release calls")
        local aerial_source = assert(love.filesystem.read("sim/aerial.lua"))
        t.is_true(
            aerial_source:find("_apply_ai_outfield_execution_error", 1, true) == nil,
            "aerial contacts retain their independent resolver"
        )
    end)
end)
