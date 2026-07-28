-- The declared OMP-3 fault matrix.
--
-- `SCENARIOS` is the data: one row per (mode x profile x fault) combination the
-- campaign claims to cover. `run` is the interpreter that turns a row into a
-- driven `FaultHarness` and a report. Keeping them apart is what lets the CI
-- subset, the full local campaign, and the separate-process campaign all quote
-- the *same* row list instead of each maintaining its own.
--
-- Two rules the rows obey:
--
--  * Every row that is bounded says so. `smoke` marks the bounded CI subset, and
--    a run that executes only the subset logs that it did. Silent truncation
--    reads as "covered everything" when it is not.
--  * A row whose fault is deliberately terminal declares `expect_status`. The
--    report then compares against that status instead of demanding a completed
--    match, so "nobody finished" is evidence rather than a failure.
--
-- The impairment rows drive `sim.network_conditions` against
-- `data/network_profiles.lua` directly. The driver's own specs use hand-picked
-- withhold-then-resume windows, which are deterministic; the profiles are
-- probabilistic, and the failure modes that matter here -- a settle that times
-- out on a healthy match, or a burst that lands exactly across full time -- are
-- precisely the ones a fixed window is least likely to hit.

local fault_harness = require("game.online.fault_harness")
local input_protocol = require("game.online.input_protocol")
local live_slot = require("game.online.live_slot")
local match_driver = require("game.online.match_driver")
local transport_contract = require("game.transport.contract")
local input_frame = require("sim.input_frame")

---@alias FaultInjectionKind
---| "none"
---| "poll_reversed"
---| "control_duplication"
---| "backpressure"
---| "burst_across_full_time"
---| "peer_disconnect"
---| "host_departure"
---| "ownership_violation"
---| "authority_conflict"
---| "malformed_input"
---| "manifest_mismatch"
---| "over_window_input"
---| "hash_divergence"

---@class FaultScenario
---@field id string
---@field mode SessionMatchMode
---@field profile NetworkProfileName
---@field injection FaultInjectionKind
---@field humans integer?
---@field duration_ticks integer?
---@field hash_interval_ticks integer?
---@field at_step integer? -- Driver step the injection fires on.
---@field expect_status MatchDriverStatus? -- Declared terminal for a deliberately fatal row.
---@field smoke boolean -- Part of the bounded CI subset.
---@field known_gap string? -- A defect this row currently exposes, named rather than excused.
---@field note string?

---@class FaultScenariosModule
local fault_scenarios = {}

fault_scenarios.MAX_STEPS = 900
fault_scenarios.SMOKE_DURATION_TICKS = 120

-- The forged sequence sits far above anything a real peer emits in a scenario of
-- this length, so a forged bundle can never collide with honest traffic.
fault_scenarios.FORGED_SEQUENCE = 9000

-- Comfortably past the 30-tick retained floor, so the released backlog is
-- provably outside the window rather than marginally so.
fault_scenarios.OVER_WINDOW_HOLD_TICKS = 45

-- Twice the largest legal payload, so a single message always fits and several
-- queued in one tick do not. A clamp below `MAX_PAYLOAD_BYTES` would deadlock
-- the handshake rather than model backpressure.
fault_scenarios.BACKPRESSURE_LIMIT_BYTES = 2 * transport_contract.MAX_PAYLOAD_BYTES

---@type FaultScenario[]
fault_scenarios.SCENARIOS = {
    -- Convergence across every mode and every documented profile. 1v1 and 2v2
    -- are the rows that can exhibit a live-slot divergence at all.
    {
        id = "1v1.clean",
        mode = "1v1",
        profile = "clean",
        injection = "none",
        smoke = true,
    },
    {
        id = "1v1.omp0_parity",
        mode = "1v1",
        profile = "omp0_parity",
        injection = "none",
        smoke = false,
    },
    {
        id = "1v1.playable",
        mode = "1v1",
        profile = "playable",
        injection = "none",
        smoke = true,
    },
    {
        id = "1v1.stress",
        mode = "1v1",
        profile = "stress",
        injection = "none",
        smoke = false,
    },
    {
        id = "2v2.clean",
        mode = "2v2",
        profile = "clean",
        injection = "none",
        smoke = false,
    },
    {
        id = "2v2.omp0_parity",
        mode = "2v2",
        profile = "omp0_parity",
        injection = "none",
        smoke = false,
    },
    {
        id = "2v2.playable",
        mode = "2v2",
        profile = "playable",
        injection = "none",
        smoke = true,
    },
    {
        id = "2v2.stress",
        mode = "2v2",
        profile = "stress",
        injection = "none",
        smoke = false,
    },
    {
        id = "4v4.clean",
        mode = "4v4",
        profile = "clean",
        injection = "none",
        smoke = true,
        note = "the eight-client row: one host plus seven guests",
    },
    {
        id = "4v4.omp0_parity",
        mode = "4v4",
        profile = "omp0_parity",
        injection = "none",
        smoke = true,
        note = "the eight-client row under a lossy but jitter-free profile",
    },
    {
        id = "4v4.playable",
        mode = "4v4",
        profile = "playable",
        injection = "none",
        smoke = false,
        note = "the eight-client row under the documented playable profile; this is the row "
            .. "that found #241, and it is the row that proves the fix -- eight clients under "
            .. "a reordering profile is the only shape in this matrix where the guest-to-host "
            .. "and host-to-guest legs compound",
    },
    {
        id = "4v4.stress",
        mode = "4v4",
        profile = "stress",
        injection = "none",
        smoke = false,
        note = "the same eight-client shape at a higher loss and burst rate; this is the row "
            .. "that found #243, and the row that proves it closed -- it stranded a guest on "
            .. "18 of 48 impairment seeds until the host's canonical batch was sized to the "
            .. "byte budget and aimed by the confirmation each guest now reports, and it "
            .. "converges on all 48 since",
    },
    -- Short lobbies: fewer humans than the mode seats, so declared bot fills
    -- exist at all.
    {
        id = "4v4.bot_fill",
        mode = "4v4",
        profile = "playable",
        injection = "none",
        humans = 3,
        smoke = false,
        note = "five declared bot fills authored by the host",
    },
    -- Recovery paths that must not depend on transport polling order.
    {
        id = "2v2.poll_reversed",
        mode = "2v2",
        profile = "stress",
        injection = "poll_reversed",
        smoke = true,
    },
    {
        id = "4v4.control_duplication",
        mode = "4v4",
        -- Deliberately the jitter-free profile: this row is about a duplicated
        -- control envelope, and pairing it with `playable` would only re-run
        -- what the `4v4.playable` row already covers and say nothing about
        -- duplication.
        profile = "omp0_parity",
        injection = "control_duplication",
        smoke = false,
    },
    {
        id = "2v2.backpressure",
        mode = "2v2",
        profile = "playable",
        injection = "backpressure",
        smoke = false,
        note = "a clamped send buffer, so the star latches backpressure and drains it again",
    },
    {
        id = "2v2.burst_across_full_time",
        mode = "2v2",
        profile = "playable",
        injection = "burst_across_full_time",
        smoke = true,
        note = "a declared window straddling full time, which a profile roll cannot be made to hit",
    },
    -- The explicit terminal taxonomy. Every row declares the status it expects.
    {
        id = "2v2.peer_disconnect",
        mode = "2v2",
        profile = "clean",
        injection = "peer_disconnect",
        at_step = 40,
        expect_status = "transport_lost",
        smoke = true,
    },
    {
        id = "2v2.host_departure",
        mode = "2v2",
        profile = "clean",
        injection = "host_departure",
        at_step = 40,
        expect_status = "transport_lost",
        smoke = false,
    },
    {
        id = "2v2.ownership_violation",
        mode = "2v2",
        profile = "clean",
        injection = "ownership_violation",
        at_step = 30,
        expect_status = "ownership_violation",
        smoke = true,
    },
    {
        id = "2v2.authority_conflict",
        mode = "2v2",
        profile = "clean",
        injection = "authority_conflict",
        at_step = 30,
        expect_status = "authority_conflict",
        smoke = false,
    },
    {
        id = "2v2.malformed_input",
        mode = "2v2",
        profile = "clean",
        injection = "malformed_input",
        at_step = 30,
        expect_status = "input_channel_failure",
        smoke = false,
    },
    {
        id = "2v2.manifest_mismatch",
        mode = "2v2",
        profile = "clean",
        injection = "manifest_mismatch",
        at_step = 30,
        expect_status = "input_channel_failure",
        smoke = false,
    },
    {
        id = "2v2.over_window_input",
        mode = "2v2",
        profile = "clean",
        injection = "over_window_input",
        at_step = 30,
        expect_status = "confirmation_stalled",
        smoke = true,
    },
    {
        id = "2v2.hash_divergence",
        mode = "2v2",
        profile = "clean",
        injection = "hash_divergence",
        at_step = 45,
        hash_interval_ticks = 6,
        expect_status = "hash_mismatch",
        smoke = true,
    },
}

---@param id string
---@return FaultScenario?
function fault_scenarios.find(id)
    for _, scenario in ipairs(fault_scenarios.SCENARIOS) do
        if scenario.id == id then
            return scenario
        end
    end
    return nil
end

---@param smoke_only boolean
---@return FaultScenario[]
function fault_scenarios.select(smoke_only)
    if not smoke_only then
        return fault_scenarios.SCENARIOS
    end
    ---@type FaultScenario[]
    local rows = {}
    for _, scenario in ipairs(fault_scenarios.SCENARIOS) do
        if scenario.smoke then
            rows[#rows + 1] = scenario
        end
    end
    return rows
end

-- ---------------------------------------------------------------------------
-- Injections
-- ---------------------------------------------------------------------------

-- A real, encodable guest bundle that the sender's own driver would never
-- author. Every fault below is reached this way rather than by reaching into a
-- driver's internals: a harness that pokes private state proves the poke works.
---@param harness FaultHarness
---@param client FaultHarnessClient
---@param slot_index integer
---@param first_input_tick integer
---@param sequence integer
---@param transport_tick integer
---@param edges integer
---@param manifest_id string
---@return TransportMessage
local function forged_bundle(
    harness,
    client,
    slot_index,
    first_input_tick,
    sequence,
    transport_tick,
    edges,
    manifest_id
)
    ---@type InputAuthorityRow[]
    local rows = {}
    for tick = 0, input_protocol.HISTORY_ROWS do
        rows[#rows + 1] = {
            tick = tick,
            slot_index = slot_index,
            sample = assert(input_frame.new_sample({
                edges = tick == input_protocol.HISTORY_ROWS and edges or 0,
            })),
        }
    end
    local packet = assert(input_protocol.new_guest({
        session_id = harness.manifest.session_id,
        manifest_id = manifest_id,
        sender_id = client.peer_id,
        sequence = sequence,
        transport_tick = transport_tick,
        first_input_tick = first_input_tick,
        rows = rows,
    }))
    return assert(transport_contract.new({
        type = "input",
        seq = packet.sequence,
        tick = packet.transport_tick,
        payload = assert(input_protocol.encode(packet)),
    }))
end

---@param harness FaultHarness
---@return FaultHarnessClient
local function first_guest(harness)
    assert(#harness.clients > 1, "a guest-side fault needs at least one guest")
    return harness.clients[2]
end

-- A slot the guest provably does not own: the frozen partition covers all eight,
-- so the host's own first owned slot is outside every guest's set.
---@param harness FaultHarness
---@param guest FaultHarnessClient
---@return integer
local function foreign_slot_index(harness, guest)
    local freeze = assert(guest.coordinator.freeze)
    local owned = {}
    for _, slot in ipairs(freeze.owned[guest.peer_id] or {}) do
        owned[slot] = true
    end
    for index = 1, input_frame.SLOT_COUNT do
        if not owned[live_slot.slot_id(index)] then
            return index
        end
    end
    error("every slot belongs to one guest, which the frozen partition forbids")
end

---@param harness FaultHarness
---@param scenario FaultScenario
local function inject(harness, scenario)
    local kind = scenario.injection
    if kind == "peer_disconnect" then
        harness.host_star:close_peer(first_guest(harness).peer_id, "peer_left")
        return
    end
    if kind == "host_departure" then
        harness.host_star:shutdown()
        return
    end
    if kind == "hash_divergence" then
        -- The driver's own detection path: report a foreign hash for a boundary
        -- this peer did hash, repeatedly, until the documented streak is reached.
        local client = first_guest(harness)
        local driver = assert(client.driver)
        local checkpoints = match_driver.checkpoints(driver)
        assert(
            #checkpoints >= match_driver.MAX_HASH_MISMATCHES,
            "the divergence row fires before enough boundaries were hashed"
        )
        for index = #checkpoints - match_driver.MAX_HASH_MISMATCHES + 1, #checkpoints do
            match_driver.observe_checkpoint(driver, checkpoints[index].tick, "deadbeefdeadbeef")
        end
        return
    end
    local guest = first_guest(harness)
    local freeze = assert(guest.coordinator.freeze)
    local transport_tick = harness.step + match_driver.DELAY_TICKS
    if kind == "ownership_violation" then
        assert(
            guest.fault:send(
                transport_contract.HOST_PEER_ID,
                "input",
                forged_bundle(
                    harness,
                    guest,
                    foreign_slot_index(harness, guest),
                    freeze.first_input_tick,
                    fault_scenarios.FORGED_SEQUENCE,
                    transport_tick,
                    0,
                    freeze.manifest_id
                )
            )
        )
        return
    end
    if kind == "manifest_mismatch" then
        assert(
            guest.fault:send(
                transport_contract.HOST_PEER_ID,
                "input",
                forged_bundle(
                    harness,
                    guest,
                    1,
                    freeze.first_input_tick,
                    fault_scenarios.FORGED_SEQUENCE,
                    transport_tick,
                    0,
                    string.rep("0", #freeze.manifest_id)
                )
            )
        )
        return
    end
    if kind == "over_window_input" then
        -- Nothing is delivered for well over the 30-tick retained floor. Since
        -- #241 the driver catches this on confirmation liveness -- at the step
        -- the floor passes a tick that never became authoritative -- rather than
        -- waiting for the backlog to land and be refused. The backlog is still
        -- released, because the row must prove the peer says so *before* it
        -- arrives rather than only that something eventually terminates.
        local from = harness.transport_tick + 1
        for _, client in ipairs(harness.clients) do
            client.fault:hold(from, from + fault_scenarios.OVER_WINDOW_HOLD_TICKS)
        end
        return
    end
    if kind == "authority_conflict" then
        local slot_index = live_slot.slot_index(assert(freeze.owned[guest.peer_id])[1])
        for _, edges in ipairs({ 0, input_frame.EDGE_BITS.dash }) do
            assert(
                guest.fault:send(
                    transport_contract.HOST_PEER_ID,
                    "input",
                    forged_bundle(
                        harness,
                        guest,
                        slot_index,
                        freeze.first_input_tick,
                        fault_scenarios.FORGED_SEQUENCE,
                        transport_tick,
                        edges,
                        freeze.manifest_id
                    )
                )
            )
        end
        return
    end
    if kind == "malformed_input" then
        assert(
            guest.fault:send(
                transport_contract.HOST_PEER_ID,
                "input",
                assert(transport_contract.new({
                    type = "input",
                    seq = fault_scenarios.FORGED_SEQUENCE,
                    tick = transport_tick,
                    payload = "not-an-input-packet",
                }))
            )
        )
        return
    end
end

-- Run one declared row end to end and report on it.
---@param scenario FaultScenario
---@param options { duration_ticks: integer?, network_seed: number?, topology: FaultHarnessTopology? }?
---@return FaultHarnessReport
function fault_scenarios.run(scenario, options)
    options = options or {}
    local duration = scenario.duration_ticks
        or options.duration_ticks
        or fault_scenarios.SMOKE_DURATION_TICKS
    local harness = fault_harness.new({
        topology = options.topology,
        mode = scenario.mode,
        humans = scenario.humans,
        profile = scenario.profile,
        duration_ticks = duration,
        hash_interval_ticks = scenario.hash_interval_ticks,
        network_seed = options.network_seed,
        poll_order = scenario.injection == "poll_reversed" and "reverse" or nil,
        duplicate_control_every = scenario.injection == "control_duplication" and 3 or nil,
        buffered_amount_limit = scenario.injection == "backpressure"
                and fault_scenarios.BACKPRESSURE_LIMIT_BYTES
            or nil,
    })
    assert(fault_harness.reach_start(harness), "the session never reached its start boundary")
    fault_harness.start_match(harness)

    if scenario.injection == "burst_across_full_time" then
        -- Full time lands at driver step `duration`; the transport clock already
        -- advanced through the pre-match control rounds, so the window is
        -- expressed against the harness clock. The hold is the profile's own
        -- burst length (three ticks), placed on purpose across the boundary --
        -- a probabilistic roll cannot be aimed at a named tick.
        local full_time = harness.transport_tick + duration
        for _, client in ipairs(harness.clients) do
            client.fault:hold(full_time - 1, full_time + 1)
        end
    end

    local at = scenario.at_step
    for _ = 1, fault_scenarios.MAX_STEPS do
        if at ~= nil and harness.step == at then
            inject(harness, scenario)
        end
        fault_harness.advance(harness)
        if fault_harness.finished(harness) then
            break
        end
    end
    fault_harness.teardown(harness)
    local report = fault_harness.report(harness, scenario.expect_status == nil)
    report.notes[#report.notes + 1] = ("scenario %s injection=%s duration_ticks=%d"):format(
        scenario.id,
        scenario.injection,
        duration
    )
    if scenario.expect_status ~= nil then
        local reached = false
        for _, client in ipairs(harness.clients) do
            local status = client.driver and match_driver.status(client.driver) or "unstarted"
            reached = reached or status == scenario.expect_status
        end
        report.findings[#report.findings + 1] = {
            id = "terminal." .. scenario.expect_status,
            ok = reached,
            skipped = false,
            detail = ("at least one peer reached the declared terminal %s"):format(
                scenario.expect_status
            ),
        }
        report.ok = report.ok and reached
    end
    return report
end

return fault_scenarios
