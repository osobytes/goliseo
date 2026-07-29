-- Versioned identity for the combat-disabled gameplay-AI policy.
--
-- #59's orchestrator refresh requires a policy id that #112/#148/#149 can cite
-- instead of copying constants or silently re-freezing them. `id()` is that
-- citation: a canonical FNV-1a-64 hash over an explicitly DECLARED surface of
-- outfield gameplay-AI configuration, prefixed with the schema version and the
-- combat mode it was taken under.
--
-- The surface is declared, never reflected, and that is the whole point:
--
--   * adding an unrelated field to an AI module does NOT move the id, so the id
--     does not churn on refactors;
--   * changing a declared constant DOES move it, so a policy change cannot be
--     absorbed by a stale baseline;
--   * renaming or deleting a declared field fails loudly instead of hashing
--     `nil` into a plausible-looking id.
--
-- A behavioural change that lives outside this surface (a file-local constant,
-- a rewritten heuristic) is caught by the recorded metric signature in
-- `sim/outfield_ai_baseline.lua`, which compares exactly. Between the two,
-- "the policy changed" is always observable; neither check may be quieted by
-- refreshing the other. When a module changes behaviour without changing a
-- declared constant, bump that module's own `VERSION` — it is in the surface
-- precisely so a deliberate policy change always has somewhere to land.
--
-- Pure module: no love, no I/O.

local fnv1a64 = require("core.fnv1a64")
local ai = require("sim.ai")
local match_snapshot = require("sim.match_snapshot")
local offball_runs = require("sim.offball_runs")
local outfield_decision = require("sim.outfield_decision")
local outfield_press = require("sim.outfield_press")
local possession_transition = require("sim.possession_transition")
local tuning = require("sim.tuning")

---@class OutfieldAiPolicyModule
local outfield_ai_policy = {}

outfield_ai_policy.SCHEMA = "outfield_ai_policy"
outfield_ai_policy.SCHEMA_VERSION = 1

-- The policy is frozen with combat off. `sim.headless` builds soccer-only
-- matches and never constructs a `CombatMatchState`, so this is a statement of
-- fact about the fixture, not a switch this module owns.
outfield_ai_policy.COMBAT_MODE = "disabled"

-- Knob category whose defaults belong to the AI policy rather than to the
-- world rules. The remaining categories are hashed separately as the tuning
-- identity of a fixture (see `sim.outfield_ai_baseline`), so a movement tweak
-- invalidates the baseline without pretending the decision policy changed.
outfield_ai_policy.KNOB_CATEGORY = "AI"

---@class OutfieldAiPolicyGroup
---@field module string
---@field fields string[]

-- Ordered; the order is part of the hashed form. Append to a group's field
-- list rather than reordering it when the surface grows.
---@type OutfieldAiPolicyGroup[]
outfield_ai_policy.SURFACE = {
    {
        module = "outfield_decision",
        fields = {
            "VERSION",
            "SLOW_REFRESH_SECONDS",
            "FAST_REFRESH_SECONDS",
            "BASE_TEMPERATURE",
            "RUN_LIFETIME_SECONDS",
        },
    },
    {
        module = "outfield_press",
        fields = { "VERSION" },
    },
    {
        module = "offball_runs",
        fields = {
            "VERSION",
            "RUN_LIFETIME_SECONDS",
            "TELEGRAPH_SECONDS",
            "MAX_ACTIVE_PER_TEAM",
            "RUN_DRIVE_THRESHOLD",
            "MIN_RUN_PROGRESS",
            "MIN_SUPPORT_DISTANCE",
            "MAX_SUPPORT_DISTANCE",
        },
    },
    {
        module = "possession_transition",
        fields = { "VERSION", "ESTABLISH_SECONDS", "MAX_PRESSERS" },
    },
    {
        -- `sim.ai` supplies the off-ball support scoring `sim.offball_runs`
        -- consumes, so its weights are policy. Its intercept-sampling
        -- constants stay file-local for the hot loop and are covered by
        -- `VERSION`.
        module = "ai",
        fields = { "VERSION", "IMPORTANCE_K", "CENTER_SIGMA", "LANE_WIDTH", "LANE_BLOCK" },
    },
}

---@type table<string, table<string, any>>
local MODULES = {
    ai = ai,
    offball_runs = offball_runs,
    outfield_decision = outfield_decision,
    outfield_press = outfield_press,
    possession_transition = possession_transition,
}

-- Every module in the surface must offer a `VERSION` to bump: it is the
-- documented landing spot for a behaviour change that moves no declared
-- constant, and the docs promise it exists.
for _, group in ipairs(outfield_ai_policy.SURFACE) do
    local module_table = assert(MODULES[group.module], "unknown policy module: " .. group.module)
    assert(
        type(module_table.VERSION) == "number",
        "policy surface module has no VERSION to bump: " .. group.module
    )
    assert(group.fields[1] == "VERSION", "VERSION must lead the surface of " .. group.module)
end

---@class OutfieldAiPolicyRow
---@field key string
---@field value number|string

-- Length-prefixed canonical scalars, byte-identical to the encoding
-- `sim/combat_identity.lua` uses, so an id is unambiguous under concatenation.
---@param parts string[]
---@param value number|string
local function append(parts, value)
    local kind = type(value)
    if kind == "number" then
        ---@cast value number
        parts[#parts + 1] = "n" .. match_snapshot.number_bytes(value) .. ";"
    else
        assert(kind == "string", "policy surface values must be numbers or strings")
        ---@cast value string
        parts[#parts + 1] = "s" .. tostring(#value) .. ":" .. value .. ";"
    end
end

-- The declared surface, resolved against the live modules, in hash order.
---@return OutfieldAiPolicyRow[]
function outfield_ai_policy.descriptor()
    ---@type OutfieldAiPolicyRow[]
    local rows = {
        { key = "schema", value = outfield_ai_policy.SCHEMA },
        { key = "schema_version", value = outfield_ai_policy.SCHEMA_VERSION },
        { key = "combat", value = outfield_ai_policy.COMBAT_MODE },
    }
    for _, group in ipairs(outfield_ai_policy.SURFACE) do
        local module_table = MODULES[group.module]
        assert(module_table, "policy surface names an unknown module: " .. group.module)
        for _, field in ipairs(group.fields) do
            local value = module_table[field]
            assert(
                type(value) == "number" or type(value) == "string",
                ("declared policy field is missing or non-scalar: %s.%s"):format(
                    group.module,
                    field
                )
            )
            rows[#rows + 1] = { key = group.module .. "." .. field, value = value }
        end
    end
    for _, knob in ipairs(tuning.knobs) do
        if knob.cat == outfield_ai_policy.KNOB_CATEGORY then
            -- The DEFAULT, not the live value: the policy is the shipped
            -- balance, so an in-session tuning-panel nudge is not a new policy.
            rows[#rows + 1] = { key = "tuning." .. knob.key, value = knob.default }
        end
    end
    return rows
end

-- Canonical bytes behind the id. Exposed so a mismatch can be diffed.
---@return string
function outfield_ai_policy.canonical()
    local parts = { "GCOAP;", tostring(outfield_ai_policy.SCHEMA_VERSION), ";" }
    for _, row in ipairs(outfield_ai_policy.descriptor()) do
        append(parts, row.key)
        append(parts, row.value)
    end
    return table.concat(parts)
end

-- The citable identity, e.g.
-- `outfield_ai_policy/v1/combat_disabled/0123456789abcdef`.
---@return string
function outfield_ai_policy.id()
    return ("%s/v%d/combat_%s/%s"):format(
        outfield_ai_policy.SCHEMA,
        outfield_ai_policy.SCHEMA_VERSION,
        outfield_ai_policy.COMBAT_MODE,
        fnv1a64.hash(outfield_ai_policy.canonical())
    )
end

-- Human-readable dump of the surface behind an id.
---@return string
function outfield_ai_policy.report()
    local lines = { "policy " .. outfield_ai_policy.id() }
    for _, row in ipairs(outfield_ai_policy.descriptor()) do
        lines[#lines + 1] = ("  %-44s %s"):format(row.key, tostring(row.value))
    end
    return table.concat(lines, "\n")
end

return outfield_ai_policy
