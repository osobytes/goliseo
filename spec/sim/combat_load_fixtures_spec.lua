local action_families = require("data.action_families")
local config = require("data.omp2_rollback_validation")
local loadouts = require("data.loadouts")
local combat = require("sim.combat")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local input_tape = require("sim.input_tape")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local rollback_lab = require("sim.rollback_lab")
local rollback_validation = require("sim.rollback_validation")
local t = require("spec.support.runner")

---@param scenario string
---@return Omp2RollbackCombatLoadFixture
local function fixture_for(scenario)
    for _, fixture in ipairs(config.combat_load_fixtures) do
        if fixture.scenario == scenario then
            return fixture
        end
    end
    error("unknown combat load scenario " .. scenario)
end

--- Replay a tape through the plain simulation and tally its combat events by
--- `kind` (and `result`, where a contact has one).
---@param tape InputTape
---@return table<string, integer>
---@return CombatMatchState?
local function replay_events(tape)
    local state, combat_state = match_snapshot.restore(tape.initial)
    local tally = {}
    for index = 1, #tape.frames do
        match.step(state, fixed_clock.TICK_SECONDS, tape.frames[index], combat_state)
        if combat_state then
            for _, event in ipairs(combat_state.events) do
                local key = event.kind .. (event.result and ("/" .. event.result) or "")
                tally[key] = (tally[key] or 0) + 1
            end
        end
    end
    return tally, combat_state
end

---@param tape InputTape
---@param network_seed integer
---@return RollbackLabResult
local function run_lab(tape, network_seed)
    local sources = { "local" }
    for slot = 2, input_frame.SLOT_COUNT do
        sources[slot] = "remote"
    end
    local campaign = rollback_lab.new_campaign(tape, {
        profile_name = config.stress_profile,
        network_seed = network_seed,
        sources = sources,
        prevalidated_tape = true,
    })
    local result = nil
    while result == nil do
        result = rollback_lab.step_campaign(campaign, 8)
    end
    return result
end

---@type table<string, RollbackLabResult>
local lab_cache = {}

--- One lab campaign per scenario, shared across the tests below. Each is a real
--- 160-tick rollback run; caching keeps this spec to four of them.
---@param scenario string
---@return RollbackLabResult
local function lab_result(scenario)
    if lab_cache[scenario] == nil then
        lab_cache[scenario] =
            run_lab(rollback_validation.combat_load_tape(scenario), config.network_seeds[1])
    end
    return lab_cache[scenario]
end

t.describe("OMP-2 crowded combat load fixtures", function()
    t.it("builds every pinned fixture at its recorded identity", function()
        t.eq(#config.combat_load_fixtures, 4)
        for _, fixture in ipairs(config.combat_load_fixtures) do
            -- combat_load_tape asserts the pinned triple internally, so reaching
            -- the assertions below already proves the artifact is unchanged.
            local tape = rollback_validation.combat_load_tape(fixture.scenario)
            t.eq(#tape.frames, fixture.frame_count, fixture.id .. " frame count")
            t.eq(tape.identity.fixture, fixture.id)
            t.eq(tape.identity.seed, fixture.seed)
            t.eq(tape.boundary_hashes[1], fixture.initial_hash, fixture.id .. " initial")
            t.eq(
                tape.boundary_hashes[#tape.boundary_hashes],
                fixture.final_hash,
                fixture.id .. " final"
            )
            t.eq(rollback_lab.tape_digest(tape), fixture.tape_digest, fixture.id .. " digest")
        end
    end)

    t.it("declares the artifact versions its combat companion implies", function()
        for _, fixture in ipairs(config.combat_load_fixtures) do
            local identity = rollback_validation.combat_load_tape(fixture.scenario).identity
            if fixture.combat then
                t.eq(identity.tape_version, input_tape.COMBAT_VERSION, fixture.id)
                t.eq(identity.snapshot_version, match_snapshot.COMBAT_VERSION, fixture.id)
                t.is_true(
                    type(identity.combat) == "string" and identity.combat ~= "",
                    fixture.id .. " needs a combat identity"
                )
            else
                t.eq(identity.tape_version, input_tape.VERSION, fixture.id)
                t.eq(identity.snapshot_version, match_snapshot.VERSION, fixture.id)
                t.eq(identity.combat, nil, fixture.id .. " must not carry a combat identity")
            end
        end
    end)

    -- The twin only attributes cost to combat if it is the *same* workload. Comparing
    -- seeds is not enough: a diverged input plan would still share a seed and would
    -- quietly turn the paired measurement into a comparison of two different matches.
    t.it("pairs each fixture with a byte-identical combat-disabled twin", function()
        for _, scenario in ipairs({ "combat_crowded", "combat_repeated_family" }) do
            local active = fixture_for(scenario)
            local twin = fixture_for(scenario .. "_disabled")
            t.eq(twin.seed, active.seed, scenario .. " twin seed")
            t.eq(twin.frame_count, active.frame_count, scenario .. " twin length")
            t.eq(twin.layout, active.layout, scenario .. " twin layout")
            t.eq(twin.combat, false, scenario .. " twin must disable combat")
            t.eq(active.combat, true, scenario .. " must enable combat")

            local active_tape = rollback_validation.combat_load_tape(active.scenario)
            local twin_tape = rollback_validation.combat_load_tape(twin.scenario)
            t.eq(#twin_tape.frames, #active_tape.frames)
            for index = 1, #active_tape.frames do
                t.eq(
                    input_frame.encode(twin_tape.frames[index]),
                    input_frame.encode(active_tape.frames[index]),
                    scenario .. " twin frame " .. index
                )
            end
        end
    end)

    t.it("drives all four action families in the crowded fixture", function()
        local tape = rollback_validation.combat_load_tape("combat_crowded")
        local tally, combat_state = replay_events(tape)
        local families = {}
        for _, runtime in ipairs(assert(combat_state).players) do
            if runtime.family_id then
                families[runtime.family_id] = true
            end
        end
        for family_id in pairs(action_families) do
            t.is_true(families[family_id], "crowded fixture is missing family " .. family_id)
        end
        -- A crowd that only commits proves nothing about crowded combat. Require the
        -- resolution paths a lone attacker never reaches: a blocked hit, the recoil it
        -- produces, an unguarded hit, the forced state it inflicts, and a projectile.
        for _, key in ipairs({
            "commit",
            "contact/guarded",
            "guard_recoil/guarded",
            "contact/hit",
            "forced/hit",
            "projectile_spawn",
        }) do
            t.is_true((tally[key] or 0) > 0, "crowded fixture never produced " .. key)
        end
    end)

    t.it("puts every outfielder on one family in the repeated-family fixture", function()
        local fixture = fixture_for("combat_repeated_family")
        local expected = assert(loadouts[assert(fixture.repeated_loadout_id)]).family_id
        local tape = rollback_validation.combat_load_tape(fixture.scenario)
        local tally, combat_state = replay_events(tape)
        local outfielders = 0
        for _, runtime in ipairs(assert(combat_state).players) do
            if runtime.family_id then
                outfielders = outfielders + 1
                t.eq(runtime.family_id, expected, "repeated-family fixture mixed a family in")
            end
        end
        t.eq(outfielders, input_frame.SLOT_COUNT)
        t.is_true((tally["contact/hit"] or 0) > 0, "repeated-family fixture landed no contact")
    end)

    t.it("keeps the combat-disabled twins free of combat entirely", function()
        for _, scenario in ipairs({ "combat_crowded_disabled", "combat_repeated_family_disabled" }) do
            local tape = rollback_validation.combat_load_tape(scenario)
            local state, combat_state = match_snapshot.restore(tape.initial)
            t.eq(combat_state, nil, scenario .. " restored a combat companion")
            t.eq(tape.initial.version, match_snapshot.VERSION, scenario)
            t.eq(combat.blocks_actions(nil, 1), false)
            t.eq(#state.players, 10)
        end
    end)

    -- The regression guard that matters most for these fixtures. `budgets.snapshot_bytes`
    -- has roughly nine kilobytes left across the whole 31-boundary window once a
    -- ten-player combat match is in it, so a few hundred extra bytes per snapshot
    -- anywhere in CombatMatchState pushes the real campaign over the gate. Failing here
    -- says so in seconds instead of in CI.
    t.it("converges inside the pinned snapshot and history budgets", function()
        for _, fixture in ipairs(config.combat_load_fixtures) do
            local result = lab_result(fixture.scenario)
            local peaks = result.metrics.peaks
            t.is_true(result.success, fixture.id .. " did not converge")
            t.eq(result.status, "converged", fixture.id)
            t.eq(result.input_ticks, fixture.frame_count, fixture.id)
            t.eq(result.reference_final_hash, fixture.final_hash, fixture.id)
            t.eq(result.client_final_hash, result.reference_final_hash, fixture.id)
            t.is_true(
                peaks.snapshot_count <= config.budgets.snapshot_count,
                fixture.id .. " exceeded the snapshot-count budget"
            )
            t.is_true(
                peaks.snapshot_bytes < config.budgets.snapshot_bytes,
                ("%s peaked at %d snapshot bytes, budget is %d"):format(
                    fixture.id,
                    peaks.snapshot_bytes,
                    config.budgets.snapshot_bytes
                )
            )
            t.is_true(
                peaks.history_bytes < config.budgets.history_bytes,
                ("%s peaked at %d history bytes, budget is %d"):format(
                    fixture.id,
                    peaks.history_bytes,
                    config.budgets.history_bytes
                )
            )
            local confirmed = result.event_metrics.confirmed_combat_events
            if fixture.combat then
                t.is_true(confirmed > 0, fixture.id .. " confirmed no combat event")
            else
                t.eq(confirmed, 0, fixture.id .. " confirmed a combat event with combat off")
            end
        end
    end)

    -- A healthy campaign only ever reaches the two passing corners of this truth table,
    -- so the rejecting corners are unreachable from a normal run. They still decide
    -- `scenario_pass`, which becomes the `success=` marker CI enforces, so assert all
    -- four directly. The active/zero corner is the one that matters most: an operator
    -- precedence slip there let a fixture that measured no combat at all pass as
    -- covered, which is precisely the regression this fixture pair exists to catch.
    t.it("rejects a case whose combat presence contradicts its fixture", function()
        for _, scenario in ipairs({ "combat_crowded", "combat_crowded_disabled" }) do
            local result = lab_result(scenario)
            local metrics = result.event_metrics
            local observed = metrics.confirmed_combat_events
            local active = fixture_for(scenario).combat

            t.is_true(
                rollback_validation.combat_load_covered(result, scenario),
                scenario .. " should cover itself as measured"
            )
            t.eq(observed > 0, active, scenario .. " measured the wrong combat presence")

            metrics.confirmed_combat_events = 0
            t.eq(
                rollback_validation.combat_load_covered(result, scenario),
                not active,
                scenario .. " mishandled zero confirmed combat events"
            )

            metrics.confirmed_combat_events = 1
            t.eq(
                rollback_validation.combat_load_covered(result, scenario),
                active,
                scenario .. " mishandled a non-zero confirmed combat event count"
            )

            metrics.confirmed_combat_events = observed
        end
    end)
end)
