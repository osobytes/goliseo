local t = require("spec.support.runner")
local env = require("sim.env")
local env_action = require("sim.env_action")
local input_frame = require("sim.input_frame")

---@param overrides table<string, any>?
---@return EnvInstance
local function reset(overrides)
    return assert(env.reset(assert(env.reference_config("soccer_only", overrides))))
end

---@param instance EnvInstance
---@return EnvSlotView
local function view(instance)
    return assert(env.observe(instance).views[1])
end

t.describe("env_action.validate", function()
    t.it("normalizes a sparse action into the full contract", function()
        local action = assert(env_action.validate({ move = { x = 0.5 }, held = { sprint = true } }))
        t.eq(action.version, env_action.VERSION)
        t.eq(assert(action.move).x, 0.5)
        t.eq(assert(action.move).y, 0)
        t.eq(assert(action.held).sprint, true)
        t.eq(assert(action.held).lob, nil)
        t.eq(next(assert(action.edges)), nil)
    end)

    t.it("treats a neutral action as a real no-op", function()
        local sample = assert(env_action.to_sample(env_action.neutral()))
        local neutral = input_frame.neutral_sample()
        t.eq(sample.move_x, neutral.move_x)
        t.eq(sample.move_y, neutral.move_y)
        t.eq(sample.held, neutral.held)
        t.eq(sample.edges, neutral.edges)
        t.eq(sample.aim, neutral.aim)
        t.eq(sample.aim, input_frame.AIM_NONE, "a no-op action aims nowhere, not at code zero")
    end)

    -- #316: `to_sample`/`from_sample` used to drop aim on the floor, so a policy
    -- that expressed one lost it on the round trip without any error.
    t.it("carries an independent aim direction through the sample round trip", function()
        local aimless = assert(env_action.validate({ move = { x = 1, y = 0 } }))
        t.eq(aimless.aim, nil)
        t.eq(assert(env_action.to_sample(aimless)).aim, input_frame.AIM_NONE)

        local aimed =
            assert(env_action.validate({ move = { x = 1, y = 0 }, aim = { x = 0, y = 3 } }))
        local sample = assert(env_action.to_sample(aimed))
        t.eq(sample.aim, 64, "aim is quantized by angle alone; the magnitude is discarded")
        t.eq(sample.move_x, input_frame.MOVE_SCALE, "movement is untouched by aim")

        local decoded = assert(env_action.from_sample(sample))
        t.near(assert(decoded.aim).y, 1, 0.01)
        t.eq(assert(env_action.to_sample(decoded)).aim, sample.aim)

        local still = assert(env_action.validate({ aim = { x = 0, y = 0 } }))
        t.eq(still.aim, nil, "a zero-length aim is the absence of one")
        t.eq(assert(env_action.to_sample(still)).aim, input_frame.AIM_NONE)

        local kept = env_action.without_edges({ aim = { x = -1, y = 0 }, edges = { dodge = true } })
        t.eq(assert(kept.aim).x, -1, "aim is held across ticks like a held bit, not an edge")

        local bad, _, code = env_action.validate({ aim = { x = 0 / 0, y = 0 } })
        t.eq(bad, nil)
        t.eq(code, "malformed")
    end)

    t.it("rejects malformed actions with machine-readable reasons", function()
        for _, case in ipairs({
            { action = "left", code = "malformed" },
            { action = { throttle = 1 }, code = "malformed" },
            { action = { version = 99 }, code = "malformed" },
            { action = { move = { x = 0, y = 0, z = 1 } }, code = "malformed" },
            { action = { move = { x = 1 / 0 } }, code = "malformed" },
            { action = { move = { x = 1, y = 1 } }, code = "move_out_of_range" },
            { action = { held = { teleport = true } }, code = "unknown_held_action" },
            { action = { held = { sprint = 1 } }, code = "malformed" },
            { action = { edges = { set_owner = true } }, code = "unknown_edge_action" },
            { action = { edges = { dash = "yes" } }, code = "malformed" },
        }) do
            local action, err, code = env_action.validate(case.action)
            t.eq(action, nil, "the action must be rejected")
            t.eq(code, case.code)
            t.is_true(type(err) == "string" and err ~= "", "a reason is always supplied")
        end
    end)

    t.it("accepts the unit disc boundary", function()
        t.is_true(env_action.validate({ move = { x = 1, y = 0 } }) ~= nil)
        t.is_true(env_action.validate({ move = { x = 0.6, y = 0.8 } }) ~= nil)
    end)
end)

t.describe("env_action sample conversion", function()
    t.it("round-trips through the canonical quantization", function()
        local action = {
            move = { x = 1, y = 0 },
            held = { sprint = true, lob = true, equipment = true },
            edges = { dash = true, equipment_pressed = true },
        }
        local sample = assert(env_action.to_sample(action))
        t.eq(input_frame.is_held(sample, "sprint"), true)
        t.eq(input_frame.is_held(sample, "lob"), true)
        t.eq(input_frame.is_held(sample, "equipment"), true)
        t.eq(input_frame.is_held(sample, "jockey"), false)
        t.eq(input_frame.has_edge(sample, "dash"), true)
        t.eq(input_frame.has_edge(sample, "equipment_pressed"), true)
        t.eq(input_frame.has_edge(sample, "switch"), false)

        local decoded = assert(env_action.from_sample(sample))
        t.eq(assert(decoded.held).sprint, true)
        t.eq(assert(decoded.held).jockey, false)
        t.eq(assert(decoded.edges).dash, true)
        t.eq(assert(env_action.to_sample(decoded)).held, sample.held)
        t.eq(assert(env_action.to_sample(decoded)).move_x, sample.move_x)
    end)

    t.it("drops one-shot edges when an action is held across ticks", function()
        local held = env_action.without_edges({
            move = { x = -1, y = 0 },
            held = { jockey = true },
            edges = { dodge = true },
        })
        t.eq(assert(held.held).jockey, true)
        t.eq(next(assert(held.edges)), nil)
        t.eq(assert(held.move).x, -1)
    end)
end)

t.describe("env_action.mask", function()
    t.it("derives legality from the view alone", function()
        local mask = assert(env_action.mask(view(reset({ seed = 6, duration = 4 }))))
        t.eq(mask.version, env_action.VERSION)
        t.eq(mask.slot, 1)
        t.eq(mask.move, true)
        t.eq(mask.aim, true)
        t.eq(mask.privileged, false)
        t.eq(mask.held.shoot, true)
        t.eq(mask.held.pass, true)
        t.eq(mask.held.sprint, true)
        t.eq(mask.edges.dash, true, "a fresh player may challenge")
        t.eq(mask.edges.dodge, true)
        t.eq(mask.edges.switch, false, "fixed-slot routing has no player switch")
        t.eq(mask.held.equipment, false, "no loadout, no equipment intent")
        t.eq(mask.edges.equipment_pressed, false)
    end)

    t.it("closes gated intents while the observable cooldown runs", function()
        local instance = reset({ seed = 6, duration = 4 })
        local player = instance._state.players[assert(instance._state.slot_players[1])]
        player.tackle_cd = 0.4
        player.dodge_cd = 0.2
        local gated = assert(env_action.mask(view(instance)))
        t.eq(gated.edges.dash, false)
        t.eq(gated.edges.dodge, false)

        player.tackle_cd = 0
        player.dodge_cd = 0
        player.stun_timer = 0.3
        local stunned = assert(env_action.mask(view(instance)))
        t.eq(stunned.edges.dash, false)
        t.eq(stunned.edges.dodge, false)
    end)

    t.it("opens aerial intents only while the ball is airborne", function()
        local instance = reset({ seed = 6, duration = 4 })
        t.eq(assert(env_action.mask(view(instance))).held.aerial_strike, false)
        instance._state.ball_z = 40
        local airborne = assert(env_action.mask(view(instance)))
        t.eq(airborne.held.aerial_strike, true)
        t.eq(airborne.held.aerial_acrobatic, true)
    end)

    t.it("gates equipment on the observable combat readiness", function()
        local config =
            assert(env.reference_config("combat_all_families", { seed = 6, duration = 4 }))
        local instance = assert(env.reset(config))
        local ready = assert(env_action.mask(view(instance)))
        t.eq(ready.held.equipment, true)
        t.eq(ready.edges.equipment_pressed, true)

        local runtime =
            assert(assert(instance._combat).players[assert(instance._state.slot_players[1])])
        runtime.cooldown_ticks = 12
        local cooling = assert(env_action.mask(view(instance)))
        t.eq(cooling.edges.equipment_pressed, false)
        t.eq(cooling.held.equipment, true, "the equipment still exists, it is just not ready")
    end)

    t.it("rejects a masked-out intent with a reason naming the slot", function()
        local instance = reset({ seed = 6, duration = 4 })
        local mask = assert(env_action.mask(view(instance)))
        local ok, err, code = env_action.check_mask({ edges = { switch = true } }, mask)
        t.eq(ok, nil)
        t.eq(code, "unavailable_action")
        t.is_true(tostring(err):find("fixed-slot routing", 1, true) ~= nil)

        local allowed = env_action.check_mask({ move = { x = 1 }, edges = { dash = true } }, mask)
        t.eq(allowed, true)

        local aerial, _, aerial_code =
            env_action.check_mask({ held = { aerial_strike = true } }, mask)
        t.eq(aerial, nil)
        t.eq(aerial_code, "unavailable_action")
    end)
end)
