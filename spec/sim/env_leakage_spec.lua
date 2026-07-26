-- Information-leakage tests for the learning environment.
--
-- These are the point of the representative profile: an observation must not
-- carry a future tick, an opponent's same-tick input, hidden RNG, a resolver
-- verdict, or a post-action event label. Every negative assertion is paired with
-- a positive control on the privileged profile, so a test that stops proving
-- anything (because the scan broke) fails instead of passing quietly.

local t = require("spec.support.runner")
local env = require("sim.env")
local env_observation = require("sim.env_observation")
local input_frame = require("sim.input_frame")

-- Names that may never appear anywhere inside a player-observable view: private
-- AI intent, resolver commitments, team scheme internals, and raw authority.
local FORBIDDEN_KEYS = {
    rng = true,
    seed = true,
    save_pending = true,
    save_vx = true,
    save_timer = true,
    save_style = true,
    windup_shot = true,
    outfield_decision = true,
    pass_target = true,
    marks = true,
    marking = true,
    press = true,
    outfield_press = true,
    keeper_release_kind = true,
    keeper_release_state = true,
    keeper_release_depth = true,
    keeper_release_motion = true,
    aerial_outcome = true,
    aerial_style = true,
    difficulty = true,
    snapshot = true,
    state = true,
    combat_state = true,
    action = true,
    actions = true,
    intent = true,
    input = true,
    frames = true,
    tape = true,
}

---@param root any
---@return table<string, boolean> keys
---@return table<string, boolean> strings
---@return table<number, boolean> numbers
local function scan(root)
    local keys, strings, numbers = {}, {}, {}
    local function walk(value)
        if type(value) == "string" then
            strings[value] = true
        elseif type(value) == "number" then
            numbers[value] = true
        elseif type(value) == "table" then
            for key, child in pairs(value) do
                if type(key) == "string" then
                    keys[key] = true
                end
                walk(child)
            end
        end
    end
    walk(root)
    return keys, strings, numbers
end

---@param profile EnvObservationProfile
---@param overrides table<string, any>?
---@return EnvInstance
local function reset(profile, overrides)
    local config = assert(env.reference_config("soccer_only", overrides))
    config.observation_profile = profile
    config.seed = config.seed or 3
    return assert(env.reset(config))
end

---@param instance EnvInstance
---@return table<integer, EnvSlotAction>
local function still(instance)
    local actions = {}
    for _, slot in ipairs(instance.controlled_slots) do
        actions[slot] = { move = { x = 0, y = 0 } }
    end
    return actions
end

---@param diverge_tick integer
---@param ticks integer
---@return InputFrame[]
local function tape_frames(diverge_tick, ticks)
    local frames = {}
    for tick = 0, ticks - 1 do
        local frame = assert(input_frame.neutral(tick))
        if tick >= diverge_tick then
            -- Only an opponent slot differs; the controlled slot is untouched.
            frame.slots[5] = assert(input_frame.new_sample({
                move_x = input_frame.MOVE_SCALE,
                held = input_frame.HELD_BITS.sprint,
            }))
        end
        frames[tick + 1] = frame
    end
    return frames
end

---@param diverge_tick integer
---@param ticks integer
---@return EnvInstance
local function reset_with_tape(diverge_tick, ticks)
    local config = assert(env.reference_config("soccer_only", { seed = 21, duration = 4 }))
    local sources = {}
    for index = 1, input_frame.SLOT_COUNT do
        if index == 1 then
            sources[index] = { kind = "policy", policy_id = "leakage-probe" }
        elseif index == 5 then
            sources[index] = { kind = "tape" }
        else
            sources[index] = { kind = "neutral" }
        end
    end
    config.slot_sources = sources
    return assert(env.reset(config, tape_frames(diverge_tick, ticks)))
end

t.describe("env representative observation leakage", function()
    t.it("hides the authoritative RNG state that the privileged profile exposes", function()
        local representative = reset("representative")
        local privileged = reset("privileged")
        local rng = representative._state.rng
        t.is_true(rng ~= 0, "the fixture must have a non-trivial RNG state to probe")

        local _, _, open_numbers = scan(env.observe(representative))
        t.is_true(not open_numbers[rng], "the representative observation must not carry the RNG")

        local _, _, privileged_numbers = scan(env.observe(privileged))
        t.is_true(privileged_numbers[rng], "positive control: the privileged profile carries it")
    end)

    t.it("hides resolver verdicts that the privileged snapshot still records", function()
        local representative = reset("representative")
        local privileged = reset("privileged")
        -- A committed save verdict is exactly the kind of authoritative outcome a
        -- player cannot read off the screen before it resolves.
        for _, instance in ipairs({ representative, privileged }) do
            for _, player in ipairs(instance._state.players) do
                if player.is_keeper then
                    player.save_pending = "catch"
                end
            end
        end

        local _, open_strings = scan(env.observe(representative))
        t.is_true(not open_strings["catch"], "the representative observation must not carry it")

        local _, privileged_strings = scan(env.observe(privileged))
        t.is_true(
            privileged_strings["catch"],
            "positive control: the privileged snapshot carries it"
        )
    end)

    t.it("names no private-state field anywhere in a player-observable view", function()
        local instance = reset("representative")
        local observation = env.observe(instance)
        t.is_true(observation.player_observable, "the representative profile is player-observable")
        local keys = scan(observation)
        for name in pairs(FORBIDDEN_KEYS) do
            t.is_true(not keys[name], "representative observation must not expose " .. name)
        end
        -- Positive control: the scan does find the fields that are meant to be there.
        t.is_true(keys.own and keys.ball and keys.opponents, "the scan reaches the view body")
    end)

    t.it("tags the privileged profile as neither observable nor a human proxy", function()
        local observation = env.observe(reset("privileged"))
        t.eq(observation.player_observable, false)
        t.eq(observation.human_proxy_valid, false)
        t.eq(env_observation.PROFILES.privileged.human_proxy_valid, false)
        local view = assert(observation.views[1])
        t.is_true(view.privileged ~= nil, "the privileged block is present")
        t.is_true(assert(view.privileged).snapshot ~= nil, "it carries the canonical snapshot")
    end)

    t.it("carries no future tick: rows the tape holds for later ticks are invisible", function()
        local baseline = reset_with_tape(99, 12)
        local diverging = reset_with_tape(4, 12)
        for _ = 1, 4 do
            assert(env.step(baseline, still(baseline)))
            assert(env.step(diverging, still(diverging)))
        end
        t.eq(baseline.tick, 4)
        t.eq(diverging.tick, 4)
        -- Both tapes agree on ticks 0..3 and differ from tick 4 onward. The
        -- boundary-4 observation is taken before tick 4 is simulated, so a view
        -- that peeked ahead would differ here.
        t.eq(
            env_observation.encode(env.observe(diverging)),
            env_observation.encode(env.observe(baseline)),
            "boundary observations must not depend on later tape rows"
        )
        -- And the tapes really do diverge afterwards, so the fixture is not vacuous.
        assert(env.step(baseline, still(baseline)))
        assert(env.step(diverging, still(diverging)))
        t.is_true(
            baseline.boundary_hashes[6] ~= diverging.boundary_hashes[6],
            "the diverging tape must actually change the simulation"
        )
    end)

    t.it("carries no opponent same-tick input", function()
        local quiet = reset_with_tape(99, 12)
        local loud = reset_with_tape(3, 12)
        for _ = 1, 3 do
            assert(env.step(quiet, still(quiet)))
            assert(env.step(loud, still(loud)))
        end
        -- The opponent slot's row for tick 3 differs between the two runs. The
        -- controlled slot's boundary-3 observation must be identical: the input
        -- for the tick about to be simulated is not observable.
        t.eq(
            env_observation.encode(env.observe(loud)),
            env_observation.encode(env.observe(quiet)),
            "the same-tick opponent row must not reach the observation"
        )
        local quiet_result = assert(env.step(quiet, still(quiet)))
        local loud_result = assert(env.step(loud, still(loud)))
        t.is_true(
            quiet_result.boundary_hash ~= loud_result.boundary_hash,
            "the differing same-tick row must actually change the simulation"
        )
    end)

    t.it("cannot leak one controlled slot's choice to another in the team profile", function()
        ---@return EnvInstance
        local function reset_two_slot()
            local config = assert(env.reference_config("soccer_only", { seed = 44 }))
            config.observation_profile = "team"
            local sources = {}
            for index = 1, input_frame.SLOT_COUNT do
                if index == 1 or index == 5 then
                    sources[index] = { kind = "policy", policy_id = "slot-" .. index }
                else
                    sources[index] = { kind = "neutral" }
                end
            end
            config.slot_sources = sources
            return assert(env.reset(config))
        end
        local left = reset_two_slot()
        local right = reset_two_slot()
        t.eq(#left.controlled_slots, 2)
        local left_view = env_observation.encode(assert(env.observe(left).views[1]))
        local right_view = env_observation.encode(assert(env.observe(right).views[1]))
        t.eq(left_view, right_view, "slot 1's view is fixed by the boundary, not by slot 5")

        -- Slot 1's view is identical whichever way slot 5 is about to act, and it
        -- names slot 5's player only through presented cues.
        local left_result = assert(env.step(left, {
            [1] = { move = { x = 1 } },
            [5] = { move = { x = -1 }, held = { sprint = true } },
        }))
        local right_result = assert(env.step(right, {
            [1] = { move = { x = 1 } },
            [5] = { move = { x = 0 } },
        }))
        t.is_true(
            left_result.boundary_hash ~= right_result.boundary_hash,
            "the two slot-5 choices must actually diverge the simulation"
        )

        -- Simultaneity is enforced by the contract: a step needs every controlled
        -- slot's action at once, so no policy can observe another's choice first.
        local partial, partial_err, partial_code = env.step(left, { [1] = { move = { x = 0 } } })
        t.eq(partial, nil)
        t.eq(partial_code, "missing_slot")
        t.is_true(
            tostring(partial_err):find("5", 1, true) ~= nil,
            "the reason names the slot with no action"
        )
    end)

    t.it("reports only confirmed past events, never post-action labels", function()
        -- Bots on the other slots make the fixture actually produce events.
        local config = assert(env.reference_config("soccer_only", { seed = 12, duration = 6 }))
        local sources = { { kind = "policy" } }
        for index = 2, input_frame.SLOT_COUNT do
            sources[index] = { kind = "bot", seed = 100 + index }
        end
        config.slot_sources = sources
        local instance = assert(env.reset(config))
        instance._state.kickoff_hold = 0
        local view = assert(env.observe(instance).views[1])
        t.eq(#view.events, 0, "the reset boundary has no confirmed events yet")
        local seen = 0
        for _ = 1, 240 do
            if instance.terminated or instance.truncated then
                break
            end
            local result = assert(env.step(instance, still(instance)))
            local stepped = assert(result.observation.views[1])
            for _, event in ipairs(stepped.events) do
                seen = seen + 1
                t.is_true(
                    event.tick < instance.tick,
                    "an observed event must belong to a completed tick"
                )
            end
        end
        t.is_true(seen > 0, "the fixture produced confirmed events to check")
    end)
end)
