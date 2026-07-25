local t = require("spec.support.runner")
local outfield_decision = require("sim.outfield_decision")
local rng = require("core.rng")

---@param overrides table?
---@return OutfieldCarrierContext
local function carrier_context(overrides)
    local context = {
        goal_distance = 280,
        shoot_range = 260,
        angle_quality = 0.8,
        keeper_coverage = 0.5,
        space = 0.5,
        flank_depth = 0.8,
        cross_target = 4,
        box_targets = 2,
        cross_space = 0.7,
        goal_progress = 0.65,
        dribble_space = 0.55,
        passes = {
            {
                player_index = 2,
                openness = 100,
                forward_progress = 80,
                distance = 150,
                lane_blocked = false,
                interception_risk = false,
            },
            {
                player_index = 3,
                openness = 75,
                forward_progress = 150,
                distance = 230,
                lane_blocked = true,
                interception_risk = false,
                lane_fraction = 0.45,
            },
        },
    }
    for key, value in pairs(overrides or {}) do
        context[key] = value
    end
    return context
end

t.describe("outfield decision cadence state", function()
    t.it("refreshes high scan rate no slower and advances only stored countdown state", function()
        local initial = outfield_decision.new_state(53)
        local slow = outfield_decision.refresh(initial, "offball", "move", 0, 100, 200, nil)
        local fast = outfield_decision.refresh(initial, "offball", "move", 1, 100, 200, nil)
        t.near(slow.remaining, 0.45)
        t.near(fast.remaining, 0.15)
        t.is_true(fast.remaining <= slow.remaining)
        t.eq(slow.generation, 1)
        t.eq(initial.generation, 0, "refresh does not mutate caller state")
        t.eq(slow.rng_state, 53)

        local retained = outfield_decision.advance(slow, 0.1)
        t.near(retained.remaining, 0.35)
        t.eq(retained.target_x, 100)
        t.is_true(not outfield_decision.should_refresh(retained, "offball"))
        t.is_true(outfield_decision.should_refresh(retained, "offball", true))
    end)

    t.it("keeps a monotonic refresh generation across deliberate resets", function()
        local state =
            outfield_decision.refresh(outfield_decision.new_state(), "carrier", "dribble", 0.5)
        state = outfield_decision.refresh(state, "carrier", "shoot", 0.5)
        t.eq(state.generation, 2)
        local reset = outfield_decision.reset(state)
        t.eq(reset.generation, 2)
        t.eq(reset.context, "ineligible")
        t.eq(reset.intent, "none")
        t.eq(reset.remaining, 0)
        t.eq(reset.rng_state, state.rng_state)
        t.eq(outfield_decision.refresh(reset, "offball", "move", 0.5, 10, 20).generation, 3)
    end)

    t.it("advances only the dedicated decision stream when selection samples", function()
        local state = outfield_decision.new_state(53)
        local options = {
            { id = "best", kind = "dribble", score = 10 },
            { id = "second", kind = "pass", score = 9, reference = 2 },
        }
        local _, next_rng = outfield_decision.decide_carrier(options, 0.79, 1, state.rng_state)
        local advanced = outfield_decision.with_rng_state(state, next_rng)
        t.is_true(advanced.rng_state ~= state.rng_state)
        t.eq(advanced.generation, state.generation)
        t.eq(state.rng_state, 53)
    end)

    t.it("requires a refresh when eligibility context changes or cadence expires", function()
        local state =
            outfield_decision.refresh(outfield_decision.new_state(), "offball", "move", 0, 1, 2)
        t.is_true(not outfield_decision.should_refresh(state, "offball"))
        t.is_true(outfield_decision.should_refresh(state, "carrier"))
        state = outfield_decision.advance(state, state.remaining)
        t.is_true(outfield_decision.should_refresh(state, "offball"))
    end)
end)

t.describe("outfield carrier choices", function()
    t.it("constructs the complete legitimate option set without consuming RNG", function()
        local seed = rng.seed(91)
        local options = outfield_decision.carrier_options(carrier_context())
        t.eq(#options, 5)
        t.eq(options[1].kind, "shoot")
        t.eq(options[2].kind, "dribble")
        t.eq(options[3].kind, "cross")
        t.eq(options[4].id, "pass_2")
        t.eq(options[5].id, "pass_3")
        t.eq(seed, rng.seed(91), "candidate construction has no RNG state")
        t.eq(options[5].payload.lane_fraction, 0.45)
    end)

    t.it("keeps shooting as a scored falloff beyond the old range", function()
        local options = outfield_decision.carrier_options(carrier_context({
            goal_distance = 600,
            shoot_range = 260,
            cross_target = nil,
            box_targets = 0,
            passes = {},
        }))
        t.eq(#options, 2)
        t.eq(options[1].kind, "shoot")
        t.is_true(options[1].score < options[2].score)
    end)

    t.it("favors carrying in open space and a legitimate outlet under pressure", function()
        local pass = {
            player_index = 2,
            openness = 100,
            forward_progress = 150,
            distance = 170,
            lane_blocked = false,
            interception_risk = false,
        }
        local open_options = outfield_decision.carrier_options(carrier_context({
            goal_distance = 600,
            cross_target = nil,
            box_targets = 0,
            goal_progress = 0.3,
            dribble_space = 1,
            space = 1,
            passes = { pass },
        }))
        local pressured_options = outfield_decision.carrier_options(carrier_context({
            goal_distance = 600,
            cross_target = nil,
            box_targets = 0,
            goal_progress = 0.3,
            dribble_space = 0.65,
            space = 0.45,
            passes = { pass },
        }))
        t.is_true(open_options[2].score > open_options[3].score)
        t.is_true(pressured_options[3].score > pressured_options[2].score)
    end)

    t.it("uses exact argmax at the high-composure boundary without advancing RNG", function()
        local options = outfield_decision.carrier_options(carrier_context())
        local seed = rng.seed(17)
        local selected, next_seed = outfield_decision.decide_carrier(options, 0.8, 1, seed)
        local best = options[1]
        for _, option in ipairs(options) do
            if option.score > best.score then
                best = option
            end
        end
        t.eq(selected.id, best.id)
        t.eq(next_seed, seed)
    end)

    t.it("reproduces a sampled action and next RNG state from the same ordered options", function()
        local options = outfield_decision.carrier_options(carrier_context())
        local seed = rng.seed(71)
        local first, first_next = outfield_decision.decide_carrier(options, 0.4, 0.9, seed)
        local second, second_next = outfield_decision.decide_carrier(options, 0.4, 0.9, seed)
        t.eq(first.id, second.id)
        t.eq(first_next, second_next)
        t.is_true(first_next ~= seed)
    end)

    t.it(
        "can choose only a legitimate lower-ranked option just below the sharp boundary",
        function()
            ---@type CarrierOption[]
            local options = {
                { id = "best", kind = "dribble", score = 10 },
                { id = "second", kind = "pass", score = 9, reference = 2 },
            }
            local best = options[1]
            for _, option in ipairs(options) do
                if option.score > best.score then
                    best = option
                end
            end
            local saw_lower = false
            for seed = 1000, 200000, 997 do
                local selected = outfield_decision.decide_carrier(options, 0.79, 1, rng.seed(seed))
                local legitimate = false
                for _, option in ipairs(options) do
                    legitimate = legitimate or selected.id == option.id
                end
                t.is_true(legitimate)
                if selected.id ~= best.id then
                    saw_lower = true
                    break
                end
            end
            t.is_true(saw_lower, "pressure never produced a legitimate lower-ranked choice")
        end
    )
end)
