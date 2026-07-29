-- Fixed-slot input production. Producers materialize complete effective
-- InputFrames before sim.match consumes them; the simulation never knows
-- whether a row began as a recording, a neutral fill, or a deterministic bot.

local Vec2 = require("core.vec2")
local combat_intent = require("sim.combat_intent")
local combat_observation = require("sim.combat_observation")
local combat_policy = require("sim.combat_policy")
local input_frame = require("sim.input_frame")
local rng = require("core.rng")

---@alias MatchSlotSourceKind "frame"|"bot"|"neutral"

---@class MatchSlotSource
---@field kind MatchSlotSourceKind
---@field seed integer? -- Canonical Park-Miller seed; required only for `bot`.

---@class SlotBotState
---@field rng integer
---@field intent CombatIntentState -- Producer-side combat intent; never enters the tape.

---@class SlotInputProducerState
---@field sources MatchSlotSource[] -- Canonical InputFrame slot order.
---@field bots table<integer, SlotBotState>

---@class MatchSlotInputModule
local slot_input = {}

---@return MatchInput
function slot_input.neutral_match_input()
    return {
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
        aerial_strike = false,
        aerial_acrobatic = false,
        equipment_held = false,
        equipment_pressed = false,
        equipment_released = false,
    }
end

---@param sample InputSample
---@return MatchInput
function slot_input.to_match_input(sample)
    local move_x = assert(input_frame.dequantize_axis(sample.move_x))
    local move_y = assert(input_frame.dequantize_axis(sample.move_y))
    return {
        move = Vec2.new(move_x, move_y),
        shoot = input_frame.has_edge(sample, "shoot") == true,
        shoot_held = input_frame.is_held(sample, "shoot") == true,
        pass = input_frame.has_edge(sample, "pass") == true,
        pass_held = input_frame.is_held(sample, "pass") == true,
        switch = input_frame.has_edge(sample, "switch") == true,
        dash = input_frame.has_edge(sample, "dash") == true,
        dodge = input_frame.has_edge(sample, "dodge") == true,
        lob = input_frame.is_held(sample, "lob") == true,
        sprint = input_frame.is_held(sample, "sprint") == true,
        jockey = input_frame.is_held(sample, "jockey") == true,
        aerial_strike = input_frame.is_held(sample, "aerial_strike") == true,
        aerial_acrobatic = input_frame.is_held(sample, "aerial_acrobatic") == true,
        equipment_held = input_frame.is_held(sample, "equipment") == true,
        equipment_pressed = input_frame.has_edge(sample, "equipment_pressed") == true,
        equipment_released = input_frame.has_edge(sample, "equipment_released") == true,
    }
end

---@param input MatchInput
---@return InputSample
function slot_input.to_sample(input)
    local move_x, move_y = assert(input_frame.quantize_move(input.move.x, input.move.y))
    local held = 0
    local edges = 0
    local function held_bit(enabled, name)
        if enabled then
            held = held + input_frame.HELD_BITS[name]
        end
    end
    local function edge_bit(enabled, name)
        if enabled then
            edges = edges + input_frame.EDGE_BITS[name]
        end
    end
    held_bit(input.shoot_held, "shoot")
    held_bit(input.pass_held, "pass")
    held_bit(input.sprint, "sprint")
    held_bit(input.jockey, "jockey")
    held_bit(input.lob, "lob")
    held_bit(input.aerial_strike == true, "aerial_strike")
    held_bit(input.aerial_acrobatic == true, "aerial_acrobatic")
    held_bit(input.equipment_held, "equipment")
    edge_bit(input.shoot, "shoot")
    edge_bit(input.pass, "pass")
    edge_bit(input.switch, "switch")
    edge_bit(input.dash, "dash")
    edge_bit(input.dodge, "dodge")
    edge_bit(input.equipment_pressed, "equipment_pressed")
    edge_bit(input.equipment_released, "equipment_released")
    return assert(input_frame.new_sample({
        move_x = move_x,
        move_y = move_y,
        held = held,
        edges = edges,
    }))
end

---@param source MatchSlotSource
---@param index integer
---@return MatchSlotSource
local function copy_source(source, index)
    assert(type(source) == "table", "slot source " .. index .. " is required")
    for key in pairs(source) do
        assert(key == "kind" or key == "seed", "slot source " .. index .. " has an unknown field")
    end
    assert(
        source.kind == "frame" or source.kind == "bot" or source.kind == "neutral",
        "slot source " .. index .. " has an unknown kind"
    )
    if source.kind == "bot" then
        assert(
            type(source.seed) == "number"
                and source.seed == source.seed
                and source.seed ~= math.huge
                and source.seed ~= -math.huge
                and source.seed == math.floor(source.seed),
            "bot slot source " .. index .. " needs an integer seed"
        )
    else
        assert(source.seed == nil, "only bot slot sources may have a seed")
    end
    if source.kind == "bot" then
        return { kind = "bot", seed = rng.seed(assert(source.seed)) }
    end
    return { kind = source.kind }
end

---@param sources MatchSlotSource[]
---@return SlotInputProducerState
function slot_input.new_producer(sources)
    assert(#sources == input_frame.SLOT_COUNT, "slot sources must have eight entries")
    for index in pairs(sources) do
        assert(
            type(index) == "number"
                and index == math.floor(index)
                and index >= 1
                and index <= input_frame.SLOT_COUNT,
            "slot sources must use canonical numeric indexes"
        )
    end
    local copied = {}
    local bots = {}
    for index = 1, input_frame.SLOT_COUNT do
        local source = assert(sources[index], "slot source " .. index .. " is required")
        copied[index] = copy_source(source, index)
        if copied[index].kind == "bot" then
            bots[index] = {
                rng = rng.seed(assert(copied[index].seed)),
                intent = combat_intent.new_state(),
            }
        end
    end
    return { sources = copied, bots = bots }
end

-- The declared fixed-slot bot fill runs the SAME `gameplay_ai/combat/v1` policy,
-- the same observation schema, the same option ordering, and the same reason
-- vocabulary as the gameplay match AI. Its retained intent lives here, on the
-- producer, not in MatchState: a materialized tape is the replay authority, and
-- inserting producer policy state into the simulation would make a replay
-- without this producer diverge.
--
-- `sim.bot` is a different population and deliberately does not get this. It is
-- the human-proxy evidence driver, and it may only gain combat capability behind
-- its own explicit policy id.
---@param state MatchState
---@param combat_state CombatMatchState?
---@param player_idx integer
---@param bot_state SlotBotState
---@return CombatIntentSignals?
local function bot_combat_signals(state, combat_state, player_idx, bot_state)
    if not combat_state then
        return nil
    end
    local runtime = combat_state.players[player_idx]
    local player = state.players[player_idx]
    if not runtime or player.is_keeper then
        return nil
    end
    if runtime.forced_ticks > 0 and bot_state.intent.stage ~= "idle" then
        bot_state.intent = combat_intent.reset(bot_state.intent)
    end
    local signals, next_intent = combat_intent.materialize(bot_state.intent)
    bot_state.intent = next_intent
    if signals then
        return signals
    end
    if
        runtime.family_id == nil
        or runtime.phase ~= "ready"
        or runtime.forced_ticks > 0
        or runtime.cooldown_ticks > 0
        or state.kickoff_hold > 0
        or state.finished
        or not combat_intent.should_decide(combat_state.tick, player_idx, player.scan_rate)
    then
        return nil
    end
    local observation =
        combat_observation.build(state, combat_state, player_idx, combat_policy.POLICY_ID, nil)
    local temperature = player.composure >= combat_policy.SHARP_COMPOSURE and 0
        or combat_policy.BASE_TEMPERATURE * (1 - player.composure)
    local decision = combat_policy.decide(
        observation,
        combat_intent.decision_seed(combat_state.tick, player_idx),
        temperature
    )
    if decision.action ~= "commit" then
        bot_state.intent = combat_intent.decline(bot_state.intent)
        return nil
    end
    assert(
        not decision.context_violation,
        "representative_policy_context_violation: gameplay_ai/combat/v1 committed without a "
            .. "purpose context"
    )
    bot_state.intent = combat_intent.commit(
        bot_state.intent,
        decision.reason,
        assert(decision.target_player),
        combat_policy.hold_ticks(observation, decision)
    )
    return combat_intent.commit_signals()
end

---@param state MatchState
---@param combat_state CombatMatchState?
---@param player_idx integer
---@param bot_state SlotBotState
---@return MatchInput
local function bot_input(state, combat_state, player_idx, bot_state)
    local player = state.players[player_idx]
    local target
    local dash = false
    local shoot = false
    local sprint = false
    local roll
    bot_state.rng, roll = rng.roll(bot_state.rng)

    if state.owner == player_idx then
        target = Vec2.new(player.team == "home" and state.field.w or 0, state.field.h / 2)
        local goal_distance = player.pos:dist(target)
        shoot = goal_distance < 180 and roll < 0.12
        sprint = not shoot
    elseif state.owner == nil then
        target = state.ball
        sprint = player.pos:dist(target) > 100
    elseif state.players[state.owner].team ~= player.team then
        target = state.ball
        local distance = player.pos:dist(target)
        dash = distance < 34 and roll < 0.20
        sprint = distance > 100
    else
        target = player.anchor
    end

    local delta = target:sub(player.pos)
    local move = delta:length() > 1 and delta:normalized() or Vec2.new(0, 0)
    local equipment = bot_combat_signals(state, combat_state, player_idx, bot_state)
    return {
        move = move,
        shoot = shoot,
        shoot_held = false,
        pass = false,
        pass_held = false,
        switch = false,
        dash = dash,
        dodge = false,
        lob = false,
        sprint = sprint,
        jockey = false,
        aerial_strike = state.ball_z > 18 and player.pos:dist(state.ball) < 72,
        aerial_acrobatic = false,
        equipment_held = equipment ~= nil and equipment.equipment_held,
        equipment_pressed = equipment ~= nil and equipment.equipment_pressed,
        equipment_released = equipment ~= nil and equipment.equipment_released,
    }
end

-- Produce the complete effective frame consumed by sim.match. Bot decisions
-- are quantized through to_sample, so the result can be saved and replayed
-- later without this producer or its RNG state.
---@param producer SlotInputProducerState
---@param state MatchState
---@param frame InputFrame
---@param combat_state CombatMatchState? -- Required for bot fills to use equipment.
---@return InputFrame frame
function slot_input.materialize(producer, state, frame, combat_state)
    assert(state.slot_mode, "slot producer requires a slot-mode match")
    assert(input_frame.validate(frame), "slot producer requires a valid InputFrame")
    assert(frame.tick == state.input_tick, "input frame tick does not match match state")
    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        local player_idx = assert(state.slot_players[index], "slot mapping is incomplete")
        local source = producer.sources[index]
        if source.kind == "frame" then
            slots[index] = frame.slots[index]
        elseif source.kind == "bot" then
            slots[index] = slot_input.to_sample(
                bot_input(state, combat_state, player_idx, assert(producer.bots[index]))
            )
        else
            slots[index] = input_frame.neutral_sample()
        end
    end
    return assert(input_frame.new(frame.tick, slots))
end

return slot_input
