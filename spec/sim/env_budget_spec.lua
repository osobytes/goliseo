-- Per-step allocation budgets for the learning environment.
--
-- #139 has to decide training feasibility from this environment's measured
-- throughput, so a silent allocation regression here would corrupt that decision.
-- These budgets are set from measured behaviour with modest headroom, not chosen
-- to pass; each failure message prints the observed figure so a regression is
-- diagnosable rather than just red.
--
-- Measured on this fixture (LuaJIT, soccer_only, seed 5), for the record:
--
--   env.action_masks (1 slot)          3.0 KB
--   env.observe      (1 slot)         11.8 KB
--   env.action_masks (8 slots)        22.7 KB
--   env.observe      (8 slots)       104.8 KB
--   env.step         (1 slot)        202.2 KB
--   env.step         (8 slots, team) 333.7 KB
--
-- The step figure is dominated by `match_snapshot.capture` + `hash` (~120 KB of
-- it) and the `match.step` engine baseline (~32 KB), neither of which this module
-- introduces. The observation is ~6% of a single-slot step. See
-- docs/design/learning_environment.md for the full breakdown and the identified
-- lever if #139 needs the boundary hash to become optional.

local t = require("spec.support.runner")
local env = require("sim.env")
local env_observation = require("sim.env_observation")
local input_frame = require("sim.input_frame")

local ITERATIONS = 500
local WARMUP = 200

---@param slots integer
---@param profile EnvObservationProfile
---@return EnvInstance
local function fresh(slots, profile)
    local config = assert(env.reference_config("soccer_only", { seed = 5, duration = 600 }))
    config.observation_profile = profile
    local sources = {}
    for index = 1, input_frame.SLOT_COUNT do
        sources[index] = index <= slots and { kind = "policy" } or { kind = "neutral" }
    end
    config.slot_sources = sources
    local instance = assert(env.reset(config))
    instance._state.kickoff_hold = 0
    return instance
end

-- Allocation per call, measured with the collector stopped so nothing is reclaimed
-- mid-measurement. Warm up first: the JIT and the instance's growable arrays both
-- allocate more on their first passes.
---@param body fun()
---@return number bytes_per_call
local function per_call_bytes(body)
    for _ = 1, WARMUP do
        body()
    end
    collectgarbage("collect")
    collectgarbage("stop")
    local before_kib = collectgarbage("count")
    local ok, failure = pcall(function()
        for _ = 1, ITERATIONS do
            body()
        end
    end)
    local bytes = (collectgarbage("count") - before_kib) * 1024
    collectgarbage("restart")
    collectgarbage("collect")
    t.is_true(ok, tostring(failure))
    return bytes / ITERATIONS
end

---@param slots integer
---@return table<integer, EnvSlotAction>
local function actions_for(slots)
    local actions = {}
    for slot = 1, slots do
        actions[slot] = { move = { x = 1, y = 0 }, held = { sprint = true } }
    end
    return actions
end

t.describe("env allocation budgets", function()
    -- The regression guard for the finding that action masking used to build a
    -- whole observation. A mask reads only own+ball; if masking ever goes back
    -- through env.observe, this budget is exceeded roughly fourfold.
    t.it("masks a slot without building an observation", function()
        local instance = fresh(1, "representative")
        local masking = per_call_bytes(function()
            env.action_masks(instance)
        end)
        local observing = per_call_bytes(function()
            env.observe(instance)
        end)
        t.is_true(masking < 6144, ("action masking allocated %.0f bytes per call"):format(masking))
        t.is_true(
            masking * 2 < observing,
            ("masking (%.0f B) must stay far cheaper than a full observation (%.0f B)"):format(
                masking,
                observing
            )
        )
    end)

    t.it("keeps a single-slot observation within budget", function()
        local instance = fresh(1, "representative")
        local bytes = per_call_bytes(function()
            env.observe(instance)
        end)
        t.is_true(bytes < 16384, ("a single-slot observation allocated %.0f bytes"):format(bytes))
    end)

    t.it("keeps a step within budget", function()
        local instance = fresh(1, "representative")
        local actions = actions_for(1)
        local bytes = per_call_bytes(function()
            assert(env.step(instance, actions))
        end)
        t.is_true(bytes < 245760, ("a single-slot step allocated %.0f bytes"):format(bytes))
    end)

    -- The direct form of the same guard. An allocation inequality cannot prove this
    -- on its own, because a step's total is dominated by snapshot hashing and would
    -- still fit its budget with a second observation build hidden inside it. So
    -- count the calls: one full observation for the returned result, and one narrow
    -- action view per controlled slot for the masks.
    t.it("builds exactly one observation and one action view per slot per step", function()
        local instance = fresh(2, "team")
        local original_build = env_observation.build
        local original_action_view = env_observation.action_view
        local builds, action_views = 0, 0
        env_observation.build = function(...)
            builds = builds + 1
            return original_build(...)
        end
        env_observation.action_view = function(...)
            action_views = action_views + 1
            return original_action_view(...)
        end
        local ok, failure = pcall(function()
            assert(env.step(instance, actions_for(2)))
        end)
        env_observation.build = original_build
        env_observation.action_view = original_action_view

        t.is_true(ok, tostring(failure))
        t.eq(builds, 1, "a step builds the full observation exactly once")
        t.eq(action_views, 2, "masking uses one narrow view per controlled slot")
    end)

    -- Pins the snapshot donation: env.step already captures the canonical snapshot
    -- for the boundary hash, and the privileged profile reuses it instead of
    -- capturing and hashing the same boundary again. Without the donation a
    -- privileged step costs roughly 60% more than a representative one.
    t.it("does not re-capture the boundary for the privileged profile", function()
        local representative = fresh(1, "representative")
        local privileged = fresh(1, "privileged")
        local actions = actions_for(1)
        local plain = per_call_bytes(function()
            assert(env.step(representative, actions))
        end)
        local oracle = per_call_bytes(function()
            assert(env.step(privileged, actions))
        end)
        t.is_true(
            oracle < plain * 1.25,
            ("a privileged step (%.0f B) must reuse the boundary snapshot a plain step"):format(
                oracle
            ) .. (" already captured (%.0f B)"):format(plain)
        )
    end)

    t.it("scales with controlled slots rather than exploding", function()
        local eight = fresh(8, "team")
        local bytes = per_call_bytes(function()
            assert(env.step(eight, actions_for(8)))
        end)
        -- Observation cost is O(controlled_slots x players) by design: each slot
        -- rebuilds every other player's record so no view can be shared, which is
        -- what keeps the per-slot privacy guarantee. Eight slots is the worst case.
        t.is_true(bytes < 425984, ("an eight-slot team step allocated %.0f bytes"):format(bytes))
    end)
end)
