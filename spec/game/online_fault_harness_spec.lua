-- The bounded CI subset of the OMP-3 fault harness (#169).
--
-- This is deliberately *not* the campaign. It runs short, deterministic rows so
-- `love . --test` stays fast, and it asserts the harness's own mechanisms --
-- that the profiles really are `sim.network_conditions`, that release order is
-- not authority, that the declared terminals are reached, and that bounded
-- coverage is logged rather than implied. The full matrix, the impaired 4v4
-- rows, and the separate-process comparison live in
-- `scripts/fault_harness.py`; see `docs/online/fault_harness.md` for what each
-- tier does and does not prove.

local t = require("spec.support.runner")
local fault_campaign = require("game.online.fault_campaign")
local fault_harness = require("game.online.fault_harness")
local fault_scenarios = require("game.online.fault_scenarios")
local FaultTransport = require("game.online.fault_transport")
local match_driver = require("game.online.match_driver")
local FakeStarTransport = require("game.transport.fake_star")
local transport_contract = require("game.transport.contract")

-- Short enough to keep the suite quick, long enough to cross several confirmed
-- checkpoints and at least one correction on every peer.
local SPEC_DURATION_TICKS = 60

---@param id string
---@param overrides table<string, any>?
---@return FaultHarnessReport
local function run(id, overrides)
    local declared = assert(fault_scenarios.find(id), "unknown scenario " .. id)
    ---@type FaultScenario
    local scenario = {
        id = declared.id,
        mode = declared.mode,
        profile = declared.profile,
        injection = declared.injection,
        humans = declared.humans,
        duration_ticks = declared.duration_ticks,
        hash_interval_ticks = declared.hash_interval_ticks,
        at_step = declared.at_step,
        expect_status = declared.expect_status,
        smoke = declared.smoke,
    }
    for key, value in pairs(overrides or {}) do
        scenario[key] = value
    end
    return fault_scenarios.run(scenario, { duration_ticks = SPEC_DURATION_TICKS })
end

---@param report FaultHarnessReport
---@param id string
---@return FaultHarnessFinding
local function finding(report, id)
    for _, entry in ipairs(report.findings) do
        if entry.id == id then
            return entry
        end
    end
    error("no finding named " .. id .. " in the report")
end

---@param report FaultHarnessReport
local function assert_all_ok(report)
    for _, entry in ipairs(report.findings) do
        t.is_true(entry.ok, ("%s %s: %s"):format(report.mode, entry.id, entry.detail))
    end
end

---@param report FaultHarnessReport
---@return string[]
local function checkpoint_markers(report)
    ---@type string[]
    local markers = {}
    for _, marker in ipairs(report.markers) do
        if marker:sub(1, #"checkpoint ") == "checkpoint " then
            markers[#markers + 1] = marker
        end
    end
    return markers
end

t.describe("online fault harness", function()
    t.it("runs a host plus seven guests from handshake to an agreed result", function()
        local report = run("4v4.clean")
        t.eq(report.clients, 8)
        assert_all_ok(report)
        t.is_true(
            finding(report, "converge.checkpoint_hash").detail:find("0 disagreed") ~= nil,
            "every shared checkpoint must agree"
        )
        t.is_true(finding(report, "teardown.clean").ok, "teardown must leave no orphan peers")
    end)

    t.it("agrees on the live slot at every confirmed checkpoint in 1v1 and 2v2", function()
        for _, id in ipairs({ "1v1.clean", "2v2.clean" }) do
            local report = run(id)
            local live = finding(report, "converge.live_slot")
            t.is_true(not live.skipped, id .. " must actually compare live slots")
            t.is_true(live.ok, id .. " live slot: " .. live.detail)
            assert_all_ok(report)
        end
    end)

    -- 4v4 owned sets are singletons, so `next_live_slot` returns the slot
    -- already live on every path. The row still runs; it is declared inert
    -- rather than counted as coverage it does not provide.
    t.it("declares the 4v4 live-slot comparison inert instead of claiming it", function()
        local live = finding(run("4v4.clean"), "converge.live_slot")
        t.is_true(live.skipped, "4v4 cannot exhibit a live-slot divergence")
        t.is_true(live.detail:find("singleton") ~= nil, live.detail)
    end)

    t.it("drives the documented profiles through sim.network_conditions", function()
        local report = run("1v1.stress")
        local impaired = false
        for _, marker in ipairs(report.markers) do
            if marker:find("impairment ") == 1 then
                t.is_true(marker:find("profile=stress") ~= nil, marker)
                local dropped = tonumber(marker:match("dropped=(%d+)")) or 0
                impaired = impaired or dropped > 0
            end
        end
        t.is_true(impaired, "the stress profile must actually drop packets")
        -- Impairment is not a licence to disagree.
        t.is_true(finding(report, "converge.checkpoint_hash").ok, "stress must still converge")
        t.is_true(finding(report, "converge.live_slot").ok, "stress must agree on the live slot")
    end)

    t.it("keeps confirmed boundaries independent of arrival release order", function()
        local forward = run("2v2.clean")
        local reversed = run("2v2.clean", { injection = "poll_reversed" })
        t.eq(
            table.concat(checkpoint_markers(reversed), "\n"),
            table.concat(checkpoint_markers(forward), "\n"),
            "reversing the release order must not move a confirmed boundary"
        )
    end)

    t.it("publishes each confirmed event once and never resurrects a revoked one", function()
        local report = run("2v2.clean")
        t.is_true(finding(report, "presentation.published_once").ok, "duplicate publication")
        t.is_true(finding(report, "presentation.no_revoked_survivor").ok, "revoked survivor")
    end)

    t.it("reaches the declared terminal for each injected fault", function()
        for _, id in ipairs({
            "2v2.ownership_violation",
            "2v2.authority_conflict",
            "2v2.malformed_input",
            "2v2.peer_disconnect",
            "2v2.hash_divergence",
        }) do
            local declared = assert(fault_scenarios.find(id))
            local report = run(id)
            local entry = finding(report, "terminal." .. assert(declared.expect_status))
            t.is_true(entry.ok, id .. ": " .. entry.detail)
        end
    end)

    t.it("names the contingent rows instead of omitting them", function()
        local report = run("1v1.clean")
        for _, id in ipairs({
            "combat.correction.wind_up",
            "combat.correction.contact",
            "combat.correction.immunity_expiry",
            "combat.default_disposition",
            "browser.multi_context",
        }) do
            local entry = finding(report, id)
            t.is_true(entry.skipped, id .. " must be skipped, not silently passed")
            t.is_true(#entry.detail > 0, id .. " must carry a reason")
        end
    end)

    t.it("logs the smoke subset as a subset", function()
        local smoke = fault_scenarios.select(true)
        local full = fault_scenarios.select(false)
        t.is_true(#smoke > 0, "the CI subset must not be empty")
        t.is_true(#smoke < #full, "the CI subset must be a strict subset")
        local declared = {}
        for _, scenario in ipairs(full) do
            t.is_true(declared[scenario.id] == nil, "scenario ids must be unique")
            declared[scenario.id] = true
        end
        for _, scenario in ipairs(smoke) do
            t.is_true(scenario.smoke, "the subset must only contain rows marked smoke")
        end
    end)

    t.it("observes this process's own pairs() order for the campaign controller", function()
        local probe = fault_campaign.hash_order_probe()
        local seen = {}
        for key in probe:gmatch("[^,]+") do
            seen[key] = true
        end
        for _, key in ipairs(fault_campaign.PROBE_KEYS) do
            t.is_true(seen[key] == true, "the probe must enumerate every key: " .. key)
        end
    end)
end)

t.describe("fault transport", function()
    ---@param profile NetworkProfileName
    ---@return FaultTransport, FakeStarTransport, FakeStarTransport
    local function pair(profile)
        local rendezvous = FakeStarTransport.new_rendezvous()
        local host = FakeStarTransport.new({ role = "host", rendezvous = rendezvous })
        assert(host:initialize())
        local guest =
            FakeStarTransport.new({ role = "guest", peer_id = "guest_1", rendezvous = rendezvous })
        assert(guest:initialize())
        assert(host:open_peer("guest_1"))
        assert(host:link(guest))
        local fault = FaultTransport.new({
            transport = guest,
            profile = profile,
            seed = 17,
            legs = { transport_contract.HOST_PEER_ID },
        })
        return fault, host, guest
    end

    ---@param host FakeStarTransport
    ---@param tick integer
    local function send_input(host, tick)
        assert(host:send(
            "guest_1",
            "input",
            assert(transport_contract.new({
                type = "input",
                seq = tick,
                tick = tick,
                payload = "payload-" .. tostring(tick),
            }))
        ))
        host:pump()
    end

    t.it("delivers everything under the clean profile", function()
        local fault, host = pair("clean")
        local delivered = 0
        for tick = 1, 40 do
            send_input(host, tick)
            fault:tick(tick)
            delivered = delivered + #fault:poll_batch(64)
        end
        t.eq(delivered, 40)
        local counters = fault:counters()
        t.eq(counters.dropped, 0)
        t.eq(counters.pending, 0)
        t.eq(counters.profile, "clean")
    end)

    t.it("drops what the profile says and nothing else", function()
        local fault, host = pair("stress")
        for tick = 1, 120 do
            send_input(host, tick)
            fault:tick(tick)
            fault:poll_batch(64)
        end
        local counters = fault:counters()
        t.eq(counters.scheduled, 120)
        t.is_true(counters.dropped > 0, "the stress profile must lose packets")
        t.eq(counters.dropped, counters.independent_lost + counters.burst_lost)
        t.eq(counters.withheld, 0)
    end)

    t.it("withholds a declared window and only that window", function()
        local fault, host = pair("clean")
        fault:withhold(10, 14)
        local delivered = 0
        for tick = 1, 30 do
            send_input(host, tick)
            fault:tick(tick)
            delivered = delivered + #fault:poll_batch(64)
        end
        t.eq(fault:counters().withheld, 5)
        t.eq(delivered, 25)
    end)

    t.it("never impairs the reliable control channel", function()
        local fault, host = pair("stress")
        for tick = 1, 60 do
            assert(host:send(
                "guest_1",
                "control",
                assert(transport_contract.new({
                    type = "event",
                    seq = tick,
                    payload = "control-" .. tostring(tick),
                }))
            ))
            host:pump()
            fault:tick(tick)
            fault:poll_batch(64)
        end
        local counters = fault:counters()
        t.eq(counters.control_delivered, 60)
        t.eq(counters.scheduled, 0)
    end)

    t.it("duplicates control traffic only when a scenario declares it", function()
        local rendezvous = FakeStarTransport.new_rendezvous()
        local host = FakeStarTransport.new({ role = "host", rendezvous = rendezvous })
        assert(host:initialize())
        local guest =
            FakeStarTransport.new({ role = "guest", peer_id = "guest_1", rendezvous = rendezvous })
        assert(guest:initialize())
        assert(host:open_peer("guest_1"))
        assert(host:link(guest))
        local fault = FaultTransport.new({
            transport = guest,
            profile = "clean",
            seed = 3,
            legs = { transport_contract.HOST_PEER_ID },
            duplicate_control_every = 2,
        })
        local delivered = 0
        for tick = 1, 10 do
            assert(host:send(
                "guest_1",
                "control",
                assert(transport_contract.new({
                    type = "event",
                    seq = tick,
                    payload = "control-" .. tostring(tick),
                }))
            ))
            host:pump()
            fault:tick(tick)
            delivered = delivered + #fault:poll_batch(64)
        end
        t.eq(fault:counters().control_duplicated, 5)
        t.eq(delivered, 15)
    end)

    t.it("substitutes for the endpoint it wraps", function()
        local fault, _, guest = pair("clean")
        t.eq(fault:role(), guest:role())
        t.eq(fault:state(), guest:state())
        t.eq(#fault:peer_ids(), #guest:peer_ids())
        t.eq(fault:capacity(), guest:capacity())
        t.is_true(fault:diagnostics().role == "guest")
    end)
end)

t.describe("fault harness input script", function()
    t.it("is a pure function of the client index and the step", function()
        for index = 1, 8 do
            for step = 0, 90 do
                local left = fault_harness.scripted_sample(index, step)
                local right = fault_harness.scripted_sample(index, step)
                t.eq(left.move_x, right.move_x)
                t.eq(left.move_y, right.move_y)
                t.eq(left.edges, right.edges)
                t.eq(left.held, right.held)
            end
        end
    end)

    t.it("moves the live slot by putting a real switch edge on the stream", function()
        local switches = 0
        for step = 0, 200 do
            if fault_harness.scripted_sample(1, step).edges ~= 0 then
                switches = switches + 1
            end
        end
        t.is_true(switches > 0, "the script must exercise switching")
        t.eq(match_driver.DELAY_TICKS, 3)
    end)
end)
