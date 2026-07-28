-- The headless entry point for the OMP-3 fault harness.
--
-- It prints one line per fact, in a fixed order, so two runs -- or two *separate
-- operating-system processes* -- can be compared with a plain text diff and a
-- digest. That is the whole reason this is a marker stream rather than a
-- human-readable log: `scripts/fault_harness.py` compares processes, and a
-- comparison is only as good as the determinism of what it compares.
--
-- # The hash-order probe
--
-- This LuaJIT/LOVE build randomizes `pairs()` iteration order per process, which
-- is exactly why a same-process multi-client harness cannot catch a
-- hash-order-induced divergence: two clients in one process share a hash seed
-- and agree by accident. The probe emits this process's observed iteration order
-- over a fixed table, so the campaign controller can *demonstrate* that the
-- processes it compared really did have different seeds rather than assume it.
-- If every process reports the same order, the cross-process claim is not
-- established for that run and the controller says so instead of passing quietly.

local fault_scenarios = require("game.online.fault_scenarios")

---@class FaultCampaignModule
local fault_campaign = {}

fault_campaign.MARKER_VERSION = 1

-- A fixed table with enough string keys that a per-process hash seed reorders it
-- with overwhelming probability. It is never used for anything else.
fault_campaign.PROBE_KEYS = {
    "alpha",
    "bravo",
    "charlie",
    "delta",
    "echo",
    "foxtrot",
    "golf",
    "hotel",
    "india",
    "juliet",
    "kilo",
    "lima",
}

-- This process's `pairs()` order over `PROBE_KEYS`. Deliberately the one place
-- in this repository where hash order is *observed on purpose*.
---@return string
function fault_campaign.hash_order_probe()
    ---@type table<string, boolean>
    local probe = {}
    for _, key in ipairs(fault_campaign.PROBE_KEYS) do
        probe[key] = true
    end
    ---@type string[]
    local order = {}
    for key in pairs(probe) do
        order[#order + 1] = key
    end
    return table.concat(order, ",")
end

---@param selector string?
---@return FaultScenario[], boolean smoke_only, string label
local function select_rows(selector)
    if selector == nil or selector == "smoke" then
        return fault_scenarios.select(true), true, "smoke"
    end
    if selector == "full" then
        return fault_scenarios.select(false), false, "full"
    end
    local scenario = fault_scenarios.find(selector)
    assert(scenario ~= nil, "unknown fault harness scenario: " .. tostring(selector))
    return { scenario }, false, selector
end

-- Run the selected rows and print the marker stream. Returns false when any
-- non-skipped finding failed.
---@param selector string?
---@param network_seed number?
---@param duration_ticks integer?
---@param topology string?
---@return boolean ok
function fault_campaign.main(selector, network_seed, duration_ticks, topology)
    -- A raised error inside `love.load` reaches LOVE's error handler, which in a
    -- headless run has no window to draw on and no key to dismiss it with: the
    -- process hangs instead of failing. Every row is therefore protected, and a
    -- crash is reported as a failing row and a non-zero exit.
    local ok, result =
        pcall(fault_campaign.run_selection, selector, network_seed, duration_ticks, topology)
    if not ok then
        print("RESULT fail rows=0 failures=1")
        print("error " .. tostring(result))
        return false
    end
    return result == true
end

---@param selector string?
---@param network_seed number?
---@param duration_ticks integer?
---@param topology string?
---@return boolean ok
function fault_campaign.run_selection(selector, network_seed, duration_ticks, topology)
    local rows, smoke_only, label = select_rows(selector)
    local wire = topology or "star"
    assert(wire == "star" or wire == "relay", "unknown fault harness topology: " .. tostring(wire))
    ---@cast wire FaultHarnessTopology
    print("FAULT-HARNESS v" .. tostring(fault_campaign.MARKER_VERSION))
    print("selection " .. label .. " rows=" .. tostring(#rows) .. " topology=" .. wire)
    print("hash-order-probe " .. fault_campaign.hash_order_probe())
    if smoke_only then
        -- Logged, never implied: the CI subset is a subset.
        print(
            ("note bounded coverage: the smoke selection runs %d of %d declared rows; "):format(
                #rows,
                #fault_scenarios.SCENARIOS
            ) .. "run `--fault-harness full` for the complete matrix"
        )
    end
    -- Named in every run, selected or not: a gap the campaign already knows about
    -- is evidence, and hiding it behind an unselected row would be the silent
    -- truncation this harness exists to refuse.
    for _, scenario in ipairs(fault_scenarios.SCENARIOS) do
        if scenario.known_gap then
            print(("known-gap %s %s"):format(scenario.id, scenario.known_gap))
        end
    end
    local failures = 0
    for _, scenario in ipairs(rows) do
        print("scenario " .. scenario.id)
        local ran, report = pcall(fault_scenarios.run, scenario, {
            network_seed = network_seed,
            duration_ticks = duration_ticks,
            topology = wire,
        })
        if not ran then
            print(("finding %s scenario.crashed FAIL %s"):format(scenario.id, tostring(report)))
            print(("scenario-result %s FAIL"):format(scenario.id))
            failures = failures + 1
            goto continue
        end
        for _, marker in ipairs(report.markers) do
            print("marker " .. marker)
        end
        for _, note in ipairs(report.notes) do
            print("note " .. note)
        end
        for _, entry in ipairs(report.findings) do
            local verdict = entry.skipped and "SKIP" or (entry.ok and "ok" or "FAIL")
            print(("finding %s %s %s %s"):format(scenario.id, entry.id, verdict, entry.detail))
            if not entry.ok then
                failures = failures + 1
            end
        end
        print(("scenario-result %s %s"):format(scenario.id, report.ok and "ok" or "FAIL"))
        ::continue::
    end
    print(
        ("RESULT %s rows=%d failures=%d"):format(failures == 0 and "ok" or "fail", #rows, failures)
    )
    return failures == 0
end

return fault_campaign
