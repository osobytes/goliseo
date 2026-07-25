local action_families = require("data.action_families")
local combat_rules = require("sim.combat_rules")
local match_snapshot = require("sim.match_snapshot")

---@class CombatIdentityModule
local combat_identity = {}

local FAMILY_IDS = { "unarmed", "guard", "light_melee", "ranged" }
local FAMILY_FIELDS = {
    "id",
    "activation",
    "contact_kind",
    "windup_ticks",
    "active_ticks",
    "held_active",
    "recovery_ticks",
    "cooldown_ticks",
    "reach_px",
    "projectile_speed_px_per_second",
    "projectile_lifetime_ticks",
    "front_arc_degrees",
    "movement_multiplier",
    "guarded_recoil_px",
}
local OUTCOME_FIELDS = { "interruption_ticks", "displacement_px", "ball_spill" }
local RULE_FIELDS = {
    "MAX_DISABLE_TICKS",
    "IMMUNITY_TICKS",
    "KNOCKBACK_THRESHOLD_PX",
}

---@param parts string[]
---@param value any
local function append(parts, value)
    local kind = type(value)
    if value == nil then
        parts[#parts + 1] = "z;"
    elseif kind == "boolean" then
        parts[#parts + 1] = value and "b1;" or "b0;"
    elseif kind == "number" then
        parts[#parts + 1] = "n" .. match_snapshot.number_bytes(value) .. ";"
    else
        assert(kind == "string", "combat identity values must be canonical scalars")
        parts[#parts + 1] = "s" .. tostring(#value) .. ":" .. value .. ";"
    end
end

---@param combat_state CombatMatchState?
---@return string
function combat_identity.for_state(combat_state)
    if not combat_state then
        return "none"
    end
    local parts = { "GCCI;1;" }
    for _, field in ipairs(RULE_FIELDS) do
        append(parts, field)
        append(parts, combat_rules[field])
    end
    for _, family_id in ipairs(FAMILY_IDS) do
        local family = assert(action_families[family_id])
        append(parts, family_id)
        for _, field in ipairs(FAMILY_FIELDS) do
            append(parts, field)
            append(parts, family[field])
        end
        append(parts, "unguarded_outcome")
        if family.unguarded_outcome then
            append(parts, "present")
            for _, field in ipairs(OUTCOME_FIELDS) do
                append(parts, field)
                append(parts, family.unguarded_outcome[field])
            end
        else
            append(parts, nil)
        end
    end
    append(parts, #combat_state.player_ids)
    for index, player_id in ipairs(combat_state.player_ids) do
        local runtime = assert(combat_state.players[index])
        append(parts, player_id)
        append(parts, runtime.loadout_id)
        append(parts, runtime.family_id)
    end
    return table.concat(parts)
end

return combat_identity
