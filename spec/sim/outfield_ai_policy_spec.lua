local t = require("spec.support.runner")
local frozen = require("data.outfield_ai_baseline")
local ai = require("sim.ai")
local offball_runs = require("sim.offball_runs")
local outfield_press = require("sim.outfield_press")
local possession_transition = require("sim.possession_transition")
local outfield_ai_policy = require("sim.outfield_ai_policy")
local outfield_decision = require("sim.outfield_decision")
local tuning = require("sim.tuning")

-- Swap one live value, take the id, and always put the value back — a leaked
-- mutation would corrupt every later spec in this process.
---@param apply fun()
---@param restore fun()
---@return string
local function id_with(apply, restore)
    apply()
    local ok, id = pcall(outfield_ai_policy.id)
    restore()
    assert(ok, id)
    return id --[[@as string]]
end

t.describe("sim.outfield_ai_policy", function()
    t.it("is stable across repeated calls", function()
        local first = outfield_ai_policy.id()
        for _ = 1, 5 do
            t.eq(outfield_ai_policy.id(), first, "the id is a pure function of the surface")
        end
    end)

    t.it("names its schema, version, and combat mode in the id", function()
        local schema, version, combat, digest =
            outfield_ai_policy.id():match("^(%a[%w_]*)/v(%d+)/combat_(%a+)/(%x+)$")
        t.eq(schema, "outfield_ai_policy")
        t.eq(tonumber(version), outfield_ai_policy.SCHEMA_VERSION)
        t.eq(combat, "disabled", "the frozen policy is the combat-disabled one")
        t.eq(#(digest or ""), 16, "FNV-1a-64 rendered as 16 hex characters")
    end)

    t.it("matches the id recorded in the frozen baseline artifact", function()
        -- data/outfield_ai_baseline.lua was written by a separate process, so
        -- this is cross-process stability, not just in-process memoization.
        t.eq(
            frozen.identity.policy_id,
            outfield_ai_policy.id(),
            "the frozen baseline was recorded under a different policy than this build runs"
        )
    end)

    t.it("covers every declared surface field with a scalar", function()
        local seen = {}
        for _, row in ipairs(outfield_ai_policy.descriptor()) do
            t.is_true(not seen[row.key], "duplicate policy key " .. row.key)
            seen[row.key] = true
            local kind = type(row.value)
            t.is_true(kind == "number" or kind == "string", row.key .. " is a scalar")
        end
        for _, group in ipairs(outfield_ai_policy.SURFACE) do
            for _, field in ipairs(group.fields) do
                t.is_true(seen[group.module .. "." .. field], group.module .. "." .. field)
            end
        end
    end)

    t.it("gives every surface module a VERSION to bump", function()
        -- The docs promise a deliberate policy change always has somewhere to
        -- land, including for constants that stay file-local. That promise is
        -- only true if every module in the surface actually exposes one.
        local modules = {
            outfield_decision = outfield_decision,
            outfield_press = outfield_press,
            offball_runs = offball_runs,
            possession_transition = possession_transition,
            ai = ai,
        }
        local covered = {}
        for _, group in ipairs(outfield_ai_policy.SURFACE) do
            covered[group.module] = true
            local module_table = assert(modules[group.module], "untested module " .. group.module)
            t.is_true(
                type(module_table.VERSION) == "number",
                group.module .. " has no VERSION to bump"
            )
            t.eq(group.fields[1], "VERSION", group.module .. " leads its surface with VERSION")
        end
        for name in pairs(modules) do
            t.is_true(covered[name], name .. " is not registered in the policy surface")
        end
    end)

    t.it("covers the off-ball support weights sim.ai supplies", function()
        -- sim/ai.lua feeds sim/offball_runs.lua's pass-lane scoring, so these
        -- weights are policy rather than incidental helpers.
        local base = outfield_ai_policy.id()
        local previous = ai.LANE_WIDTH
        local moved = id_with(function()
            ai.LANE_WIDTH = previous + 1
        end, function()
            ai.LANE_WIDTH = previous
        end)
        t.is_true(moved ~= base, "a pass-lane blocking width change must move the policy id")
    end)

    t.it("detects a bumped module VERSION", function()
        local base = outfield_ai_policy.id()
        local previous = offball_runs.VERSION
        local moved = id_with(function()
            offball_runs.VERSION = previous + 1
        end, function()
            offball_runs.VERSION = previous
        end)
        t.is_true(moved ~= base, "bumping VERSION is how a file-local change is declared")
    end)

    t.it("detects a changed decision constant", function()
        local base = outfield_ai_policy.id()
        local previous = outfield_decision.BASE_TEMPERATURE
        local moved = id_with(function()
            outfield_decision.BASE_TEMPERATURE = previous + 1
        end, function()
            outfield_decision.BASE_TEMPERATURE = previous
        end)
        t.is_true(moved ~= base, "a decision-temperature change must move the policy id")
        t.eq(outfield_ai_policy.id(), base, "and the value is restored")
    end)

    t.it("detects a changed run constant", function()
        local base = outfield_ai_policy.id()
        local previous = offball_runs.MAX_ACTIVE_PER_TEAM
        local moved = id_with(function()
            offball_runs.MAX_ACTIVE_PER_TEAM = previous + 1
        end, function()
            offball_runs.MAX_ACTIVE_PER_TEAM = previous
        end)
        t.is_true(moved ~= base, "a run-cap change must move the policy id")
    end)

    t.it("detects a changed AI knob default", function()
        local base = outfield_ai_policy.id()
        local knob = assert(tuning.by_key["AI_SHOOT_RANGE"])
        local previous = knob.default
        local moved = id_with(function()
            knob.default = previous + 1
        end, function()
            knob.default = previous
        end)
        t.is_true(moved ~= base, "an AI knob default is part of the shipped policy")
    end)

    t.it("does not move when an undeclared module field is added", function()
        local base = outfield_ai_policy.id()
        local same = id_with(function()
            ---@diagnostic disable-next-line: inject-field
            outfield_decision.SPEC_ONLY_SCRATCH_FIELD = 42
        end, function()
            ---@diagnostic disable-next-line: inject-field
            outfield_decision.SPEC_ONLY_SCRATCH_FIELD = nil
        end)
        t.eq(same, base, "the surface is declared, so refactors do not churn the id")
    end)

    t.it("does not move when a live tuning value is nudged off its default", function()
        local base = outfield_ai_policy.id()
        local previous = tuning.values["AI_SHOOT_RANGE"]
        local same = id_with(function()
            tuning.set("AI_SHOOT_RANGE", previous + 10)
        end, function()
            tuning.set("AI_SHOOT_RANGE", previous)
        end)
        t.eq(same, base, "an in-session tuning-panel nudge is not a new shipped policy")
    end)

    t.it("fails loudly when a declared field disappears", function()
        local previous = outfield_decision.BASE_TEMPERATURE
        ---@diagnostic disable-next-line: assign-type-mismatch
        outfield_decision.BASE_TEMPERATURE = nil
        local ok, err = pcall(outfield_ai_policy.id)
        outfield_decision.BASE_TEMPERATURE = previous
        t.is_true(not ok, "a renamed constant must not hash as nil")
        t.is_true(
            tostring(err):find("BASE_TEMPERATURE", 1, true) ~= nil,
            "the error names the missing field"
        )
    end)

    t.it("reports the surface behind an id", function()
        local report = outfield_ai_policy.report()
        t.is_true(report:find(outfield_ai_policy.id(), 1, true) ~= nil, "the report cites the id")
        t.is_true(
            report:find("outfield_decision.VERSION", 1, true) ~= nil,
            "and lists the declared fields"
        )
    end)
end)
