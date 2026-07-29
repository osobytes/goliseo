-- Information-leakage tests for `combat_sim_observation/v1`.
--
-- Section 4.7 of docs/design/combat_fun_evidence_contract.md excludes pixels,
-- cue visibility, render timing, theme, presentation identity, viewport, frame
-- rate, unresolved outcomes, hidden collision results, future input, and RNG
-- state. These tests assert the absence of all of that, and each negative
-- assertion is paired with a POSITIVE CONTROL that feeds the same scanner
-- something that genuinely leaks: a scan that stopped scanning would then pass
-- the negative test and fail the control, instead of passing quietly.

local combat = require("sim.combat")
local combat_observation = require("sim.combat_observation")
local combat_policy = require("sim.combat_policy")
local fixed_clock = require("sim.fixed_clock")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local players_data = require("data.players")
local slot_input = require("sim.slot_input")
local teams = require("data.teams")
local t = require("spec.support.runner")

-- Keys that may never appear anywhere inside a combat observation.
local FORBIDDEN_KEYS = {
    loadout_id = true,
    presentation_id = true,
    cosmetic_variant_id = true,
    equipment_presentation_id = true,
    species_id = true,
    theme = true,
    viewport = true,
    pixels = true,
    frame_time = true,
    render_tick = true,
    rng = true,
    rng_state = true,
    seed = true,
    snapshot = true,
    state = true,
    combat_state = true,
    events = true,
    contacted = true,
    projectile_spawned = true,
    chain_ticks = true,
    result = true,
    outcome = true,
    future_input = true,
    intent = true,
    outfield_decision = true,
    outfield_press = true,
    marks = true,
    marking = true,
    press = true,
    windup_shot = true,
    pass_target = true,
    save_pending = true,
    composure = true,
    scan_rate = true,
}

---@param root any
---@return table<string, boolean> keys
---@return table<string, boolean> strings
local function scan(root)
    local keys, strings = {}, {}
    local function walk(value)
        if type(value) == "string" then
            strings[value] = true
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
    return keys, strings
end

---@param root any
---@return string? offender
local function first_forbidden_key(root)
    local keys = scan(root)
    for key in pairs(keys) do
        if FORBIDDEN_KEYS[key] then
            return key
        end
    end
    return nil
end

---@param overrides table<string, string>?
---@return MatchState
---@return CombatMatchState
local function new_match(overrides)
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        seed = 4242,
    })
    state.kickoff_hold = 0
    local pool = {}
    for _, player in ipairs(players_data) do
        local copied = {}
        for key, value in pairs(player) do
            copied[key] = value
        end
        if overrides and overrides[player.id] then
            copied.loadout_id = overrides[player.id]
        end
        pool[player.id] = copied
    end
    return state, combat.new_state(state, pool)
end

---@param state MatchState
---@param combat_state CombatMatchState
---@return CombatObservation
local function observe(state, combat_state)
    return combat_observation.build(state, combat_state, 2, combat_policy.POLICY_ID, nil)
end

t.describe("combat observation leakage", function()
    t.it("carries no presentation, hidden, or authority key", function()
        local state, combat_state = new_match()
        state.rng = 999983
        combat_state.players[7].contacted = true
        local observation = observe(state, combat_state)
        local offender = first_forbidden_key(observation)
        t.is_true(offender == nil, "observation leaked " .. tostring(offender))
    end)

    t.it("positive control: the same scanner rejects the authoritative snapshot", function()
        local state, combat_state = new_match()
        local snapshot = match_snapshot.capture(state, combat_state)
        local offender = first_forbidden_key(snapshot)
        t.is_true(
            offender ~= nil,
            "the leakage scanner found nothing in a full snapshot, so it proves nothing"
        )
    end)

    t.it("carries no presentation or loadout string value", function()
        local state, combat_state = new_match()
        local _, strings = scan(observe(state, combat_state))
        for _, player in ipairs(players_data) do
            t.is_true(not strings[player.presentation_id], "leaked a presentation id")
            if player.loadout_id then
                t.is_true(not strings[player.loadout_id], "leaked a loadout id")
            end
            if player.cosmetic_variant_id then
                t.is_true(not strings[player.cosmetic_variant_id], "leaked a cosmetic variant")
            end
        end
        -- Positive control: the strings are genuinely reachable from the
        -- authored content, so the assertions above are testing something.
        local authored = select(2, scan(players_data))
        t.is_true(authored[players_data[1].presentation_id] == true)
    end)

    t.it("carries no RNG state and no resolver verdict", function()
        local state, combat_state = new_match()
        state.rng = 123457
        combat_state.events[1] = {
            kind = "contact",
            tick = 0,
            family_id = "unarmed",
            source_index = 2,
            target_index = 7,
            source_sequence = 1,
            result = "hit",
            x = 1,
            y = 2,
            interruption_ticks = 10,
            displacement_px = 8,
        }
        local _, strings = scan(observe(state, combat_state))
        t.is_true(not strings.hit, "leaked a resolver verdict")
        t.is_true(not strings.superseded)
        local keys = scan(observe(state, combat_state))
        t.is_true(not keys.rng)
        t.is_true(not keys.result)
    end)

    t.it("produces an identical digest for a presentation swap inside one family", function()
        -- `loadout_tournament_sword` and `loadout_vector_blade` are both
        -- `light_melee` and differ only in equipment presentation.
        local baseline_state, baseline_combat = new_match()
        local swapped_state, swapped_combat = new_match({
            veil_nyx = "loadout_vector_blade",
        })
        local baseline = observe(baseline_state, baseline_combat)
        local swapped = observe(swapped_state, swapped_combat)
        t.eq(combat_observation.encode(swapped), combat_observation.encode(baseline))
        t.eq(swapped.digest, baseline.digest)

        -- Positive control: a swap that changes the FAMILY is a real
        -- mechanical change and must move the digest.
        local family_swap_state, family_swap_combat = new_match({
            veil_nyx = "loadout_pulse_blaster",
        })
        local family_swap = observe(family_swap_state, family_swap_combat)
        t.is_true(family_swap.digest ~= baseline.digest)
    end)

    t.it("keeps AI decisions and materialized tapes identical across that swap", function()
        ---@param overrides table<string, string>?
        ---@return string
        local function run(overrides)
            local state, combat_state = new_match(overrides)
            local parts = {}
            for _ = 1, 120 do
                match.step(
                    state,
                    fixed_clock.TICK_SECONDS,
                    slot_input.neutral_match_input(),
                    combat_state
                )
                for _, event in ipairs(state.events) do
                    if event.kind:find("^combat_") then
                        parts[#parts + 1] = event.kind .. "/" .. tostring(event.player)
                    end
                end
                for index, runtime in ipairs(combat_state.players) do
                    parts[#parts + 1] = index
                        .. runtime.intent.stage
                        .. runtime.intent.reason
                        .. tostring(runtime.intent.target_player)
                end
            end
            return table.concat(parts, ";")
        end

        local baseline = run(nil)
        t.eq(run({ veil_nyx = "loadout_vector_blade" }), baseline)
        t.is_true(#baseline > 0)
    end)
end)
