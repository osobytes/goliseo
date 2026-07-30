local t = require("spec.support.runner")
local combat_phases = require("spec.support.online_combat_phases")
local match_driver = require("game.online.match_driver")
local fixture = require("game.online.match_driver_fixture")
local input_protocol = require("game.online.input_protocol")
local live_slot = require("game.online.live_slot")
local protocol = require("game.online.protocol")
local transport_contract = require("game.transport.contract")
local input_frame = require("sim.input_frame")
local match_snapshot = require("sim.match_snapshot")
local rollback_session = require("sim.rollback_session")

---@class DriverHarness
---@field session MatchDriverFixtureSession
---@field drivers MatchDriver[] -- Host first, then guests in seating order.
---@field peer_ids string[]
---@field step integer

---@class DriverHarnessOptions
---@field duration number?
---@field humans integer?
---@field combat boolean?
---@field initial_snapshot MatchSnapshot? -- Boundary zero for every peer; overrides `combat`.
---@field hash_interval_ticks integer?
---@field max_rollback_ticks integer?
---@field settle_timeout_ticks integer?
---@field settle_timeout_seconds number?
---@field clock (fun(): number)?
---@field divergent_peer integer? -- Driver index whose boundary zero is seeded differently.
---@field wrap_host_transport (fun(inner: StarTransportAdapter): StarTransportAdapter)?
---@field wrap_guest_transport (fun(index: integer, inner: StarTransportAdapter): StarTransportAdapter)?

---@param mode SessionMatchMode
---@param options DriverHarnessOptions?
---@return DriverHarness
local function harness(mode, options)
    options = options or {}
    local session = fixture.session(mode, nil, options.humans)
    local snapshot = options.initial_snapshot
        or fixture.initial_snapshot(options.duration, options.combat)
    local divergent = options.divergent_peer
            and fixture.initial_snapshot(options.duration, options.combat, fixture.DEFAULT_SEED + 1)
        or nil

    ---@param index integer
    ---@return MatchDriverOptions
    local function driver_options(index)
        return {
            role = "guest",
            peer_id = "",
            freeze = session.freeze,
            manifest = session.manifest,
            transport = session.host_transport,
            initial_snapshot = index == options.divergent_peer and assert(divergent) or snapshot,
            hash_interval_ticks = options.hash_interval_ticks,
            max_rollback_ticks = options.max_rollback_ticks,
            settle_timeout_ticks = options.settle_timeout_ticks,
            settle_timeout_seconds = options.settle_timeout_seconds,
            clock = options.clock,
        }
    end

    local drivers = {}
    local peer_ids = { session.host_peer_id }
    local host_options = driver_options(1)
    host_options.role = "host"
    host_options.peer_id = session.host_peer_id
    if options.wrap_host_transport then
        host_options.transport = options.wrap_host_transport(session.host_transport)
    end
    drivers[1] = match_driver.new(host_options)
    for _, peer_id in ipairs(session.guest_peer_ids) do
        peer_ids[#peer_ids + 1] = peer_id
        local guest_options = driver_options(#drivers + 1)
        guest_options.peer_id = peer_id
        guest_options.transport = assert(session.guest_transports[peer_id])
        if options.wrap_guest_transport then
            guest_options.transport =
                options.wrap_guest_transport(#drivers + 1, guest_options.transport)
        end
        drivers[#drivers + 1] = match_driver.new(guest_options)
    end
    return { session = session, drivers = drivers, peer_ids = peer_ids, step = 0 }
end

---@param harness_state DriverHarness
---@param samples table<integer, InputSample>? -- Driver index -> sample for this step.
---@return MatchDriverBatch[]
local function advance(harness_state, samples)
    local batches = {}
    for index, driver in ipairs(harness_state.drivers) do
        local sample = samples and samples[index] or input_frame.neutral_sample()
        batches[index] = match_driver.advance(driver, sample)
    end
    harness_state.session.host_transport:pump()
    harness_state.step = harness_state.step + 1
    return batches
end

---@param harness_state DriverHarness
---@param steps integer
---@param samples table<integer, InputSample>?
---@return MatchDriverBatch[]
local function run(harness_state, steps, samples)
    local last = {}
    for _ = 1, steps do
        last = advance(harness_state, samples)
    end
    return last
end

-- Impaired delivery: the star only drains every `period` steps, so every peer
-- predicts the rows it has not received and corrects them in one burst. That is
-- the path that actually exercises rollback, prediction, and reconciliation.
---@param harness_state DriverHarness
---@param steps integer
---@param period integer
---@param samples table<integer, InputSample>?
local function run_bursty(harness_state, steps, period, samples)
    for step = 1, steps do
        for index, driver in ipairs(harness_state.drivers) do
            local sample = samples and samples[index] or input_frame.neutral_sample()
            match_driver.advance(driver, sample)
        end
        if step % period == 0 then
            harness_state.session.host_transport:pump()
        end
        harness_state.step = harness_state.step + 1
    end
end

---@param sample_options InputSampleOptions
---@return InputSample
local function sample(sample_options)
    return assert(input_frame.new_sample(sample_options))
end

---@return InputSample
local function switch_sample()
    return sample({ edges = input_frame.EDGE_BITS.switch })
end

-- Every checkpoint both peers hashed must agree, and so must the live slot each
-- peer computed for every human at that same confirmed boundary.
---@param harness_state DriverHarness
---@return integer compared
local function assert_agreement(harness_state)
    local reference = harness_state.drivers[1]
    local checkpoints = match_driver.checkpoints(reference)
    local compared = 0
    for _, checkpoint in ipairs(checkpoints) do
        for index = 2, #harness_state.drivers do
            local mine = nil
            for _, candidate in ipairs(match_driver.checkpoints(harness_state.drivers[index])) do
                if candidate.tick == checkpoint.tick then
                    mine = candidate
                end
            end
            if mine ~= nil then
                t.eq(mine.hash, checkpoint.hash, "boundary hash at " .. tostring(checkpoint.tick))
                for producer_id, slot in pairs(checkpoint.live) do
                    t.eq(
                        mine.live[producer_id],
                        slot,
                        "live slot for " .. producer_id .. " at " .. tostring(checkpoint.tick)
                    )
                end
                compared = compared + 1
            end
        end
    end
    return compared
end

-- Present state is a prediction and legitimately differs between peers. The
-- confirmed boundary is where authority is complete, so that is where identical
-- state is a contract rather than a coincidence.
---@param harness_state DriverHarness
local function assert_confirmed_state(harness_state)
    local boundary = nil
    for _, driver in ipairs(harness_state.drivers) do
        local confirmed = match_driver.diagnostics(driver).confirmed_output_tick
        if boundary == nil or confirmed < boundary then
            boundary = confirmed
        end
    end
    boundary = assert(boundary) + 1
    t.is_true(boundary > 0, "no peer confirmed a single boundary")
    local reference = nil
    for _, driver in ipairs(harness_state.drivers) do
        local lookup = match_driver.snapshot(driver, boundary)
        t.is_true(lookup.status == "present" or lookup.status == "retained")
        local hash = match_snapshot.hash(assert(lookup.snapshot))
        reference = reference or hash
        t.eq(hash, reference, "confirmed boundary " .. tostring(boundary))
    end
end

-- A short match, so a burst can straddle full time without the hold itself
-- outrunning the 30-tick retained window and turning the run into `late_input`.
local SETTLE_DURATION = 24 / 60

-- Advance every driver once per step, delivering only on the steps `deliver`
-- allows, and stopping once no driver is still active.
---@param harness_state DriverHarness
---@param steps integer
---@param deliver fun(step: integer): boolean
---@param sample_for (fun(step: integer, index: integer): InputSample)?
local function drive(harness_state, steps, deliver, sample_for)
    for _ = 1, steps do
        local step = harness_state.step
        local active = false
        for index, driver in ipairs(harness_state.drivers) do
            match_driver.advance(
                driver,
                sample_for and sample_for(step, index) or input_frame.neutral_sample()
            )
            active = active or match_driver.status(driver) == "active"
        end
        if deliver(step) then
            harness_state.session.host_transport:pump()
        end
        harness_state.step = step + 1
        if not active then
            return
        end
    end
end

-- Input that changes every step. Prediction repeats the last sample, so a
-- constant one is predicted *correctly* by definition: a burst over constant
-- input opens no divergence at all, and a test built on it would pass whether
-- or not the tail was ever confirmed.
---@param step integer
---@param index integer
---@return InputSample
local function moving_sample(step, index)
    local phase = (step * 7 + index * 13) % 8
    return sample({
        move_x = 90 - phase * 24,
        move_y = phase * 17 - 60,
        edges = phase == 3 and input_frame.EDGE_BITS.switch or 0,
    })
end

-- Which tick full time lands on is `sim.match`'s countdown to decide, not
-- arithmetic on the duration: a float remainder buys one more tick. Probe it
-- once under clean delivery rather than encode an assumption about it, because
-- every burst window below is placed relative to it.
local FULL_TIME_BOUNDARY = (function()
    local probe = harness("1v1", { duration = SETTLE_DURATION })
    drive(probe, 200, function()
        return true
    end)
    return assert(match_driver.full_time_boundary(probe.drivers[1]))
end)()
-- Driver step `T` simulates input tick `T`, so this is the step that reaches it.
local FULL_TIME_STEP = FULL_TIME_BOUNDARY - 1

-- Every peer completed *through the settle phase*, on the same final boundary,
-- with every tick of the match authoritative and the same hash captured there.
-- The final boundary hash is what the screen reports as the session's
-- `final_hash`, so this is the agreement the acknowledged result rests on.
---@param harness_state DriverHarness
---@param label string?
---@return integer boundary
local function assert_settled(harness_state, label)
    local suffix = label and (" in " .. label) or ""
    local boundary = nil
    for index, driver in ipairs(harness_state.drivers) do
        local diagnostics = match_driver.diagnostics(driver)
        t.eq(
            match_driver.status(driver),
            "completed",
            ("peer %d did not complete%s"):format(index, suffix)
        )
        t.eq(assert(match_driver.terminal(driver)).failure, nil)
        t.is_true(
            match_driver.settled(driver),
            ("peer %d completed without settling%s"):format(index, suffix)
        )
        t.is_true(not diagnostics.settling)
        local mine = assert(match_driver.full_time_boundary(driver))
        boundary = boundary or mine
        t.eq(mine, boundary, ("final boundary on peer %d%s"):format(index, suffix))
        -- The settle phase's whole contract: nothing in the match is left
        -- unconfirmed when it reports the result.
        t.eq(
            diagnostics.confirmed_output_tick,
            boundary - 1,
            ("peer %d completed with an unconfirmed tail%s"):format(index, suffix)
        )
    end
    boundary = assert(boundary)
    local reference = nil
    for index, driver in ipairs(harness_state.drivers) do
        local lookup = match_driver.snapshot(driver, boundary)
        t.is_true(lookup.status == "present" or lookup.status == "retained")
        local hash = match_snapshot.hash(assert(lookup.snapshot))
        reference = reference or hash
        t.eq(hash, reference, ("final hash on peer %d%s"):format(index, suffix))
    end
    return boundary
end

t.describe("online match driver", function()
    t.it("seats a 4v4 host on one slot and authors every bot fill", function()
        local state = harness("4v4")
        local host = match_driver.diagnostics(state.drivers[1])
        t.eq(host.role, "host")
        t.eq(#host.owned, 1)
        t.eq(host.owned[1], "home_1")
        t.eq(host.control_slot, "home_1")
        -- Eight humans in 4v4, so the host authors only its own slot.
        t.eq(#host.authored, 1)
        local guest = match_driver.diagnostics(state.drivers[2])
        t.eq(#guest.owned, 1)
        t.eq(guest.owned[1], "home_2")
    end)

    t.it("gives a 1v1 human four owned slots and one control slot", function()
        local state = harness("1v1")
        local host = match_driver.diagnostics(state.drivers[1])
        t.eq(#host.owned, 4)
        t.eq(table.concat(host.owned, ","), "home_1,home_2,home_3,home_4")
        t.eq(host.control_slot, "home_1")
        -- No bot fills at all in 1v1: every slot belongs to one of the two
        -- humans, so the host authors exactly its own four.
        t.eq(#host.authored, 4)
        local drivers = match_driver.slot_drivers(state.drivers[1])
        t.eq(drivers.home_1, "human")
        t.eq(drivers.home_2, "ai")
        t.eq(drivers.away_1, "human")
        t.eq(drivers.away_2, "ai")
    end)

    t.it("makes the host author every declared bot fill in a short lobby", function()
        -- A full lobby covers all eight slots in every mode, so a declared bot
        -- fill only exists when fewer humans are seated than the mode allows.
        local state = harness("2v2", { humans = 2 })
        local sources = state.session.freeze.sources
        local bots = 0
        for slot = 1, input_frame.SLOT_COUNT do
            local id = assert(input_frame.slot(slot)).id
            if sources[id].producer_kind == "bot" then
                bots = bots + 1
            end
        end
        t.eq(bots, 4)
        local host = match_driver.diagnostics(state.drivers[1])
        t.eq(#host.owned, 2)
        -- Its own two slots plus all four bots, every one of them carried by the
        -- host's own delayed collector path.
        t.eq(#host.authored, 6)
        local guest = match_driver.diagnostics(state.drivers[2])
        t.eq(#guest.authored, 2)

        run(state, 24)
        t.is_true(assert_agreement(state) > 0)
        assert_confirmed_state(state)
        -- Declared fills are indistinguishable from a human's non-live owned
        -- slots in the stream: every slot is authoritative at a confirmed tick.
        local drivers = match_driver.slot_drivers(state.drivers[1])
        local humans = 0
        for slot = 1, input_frame.SLOT_COUNT do
            if drivers[assert(input_frame.slot(slot)).id] == "human" then
                humans = humans + 1
            end
        end
        t.eq(humans, 2)
    end)

    t.it("converges every peer on the same confirmed boundary hashes in 4v4", function()
        local state = harness("4v4")
        run(state, 40)
        t.is_true(assert_agreement(state) > 0)
        for _, driver in ipairs(state.drivers) do
            t.eq(match_driver.status(driver), "active")
            local diagnostics = match_driver.diagnostics(driver)
            t.eq(diagnostics.present_input_tick, 40)
            t.is_true(diagnostics.confirmed_input_tick >= 30)
        end
        assert_confirmed_state(state)
    end)

    t.it("agrees on the live slot at every confirmed checkpoint in 1v1", function()
        local state = harness("1v1")
        -- The host presses switch continuously: liveness must move identically
        -- on both peers, and 1v1 is where a divergence can actually appear.
        run(state, 48, { [1] = switch_sample() })
        t.is_true(assert_agreement(state) > 0)
        for _, driver in ipairs(state.drivers) do
            t.eq(match_driver.status(driver), "active")
        end
        assert_confirmed_state(state)
    end)

    t.it("agrees on the live slot at every confirmed checkpoint in 2v2", function()
        local state = harness("2v2")
        run(state, 48, { [1] = switch_sample(), [3] = switch_sample() })
        t.is_true(assert_agreement(state) > 0)
        assert_confirmed_state(state)
    end)

    t.it("moves the control slot only through the canonical stream", function()
        local state = harness("1v1")
        local host = state.drivers[1]
        t.eq(match_driver.control_slot(host, state.session.host_peer_id, 0), "home_1")
        run(state, 24, { [1] = switch_sample() })
        local moved = false
        for tick = 1, 20 do
            if match_driver.control_slot(host, state.session.host_peer_id, tick) ~= "home_1" then
                moved = true
            end
        end
        t.is_true(moved, "a held switch never moved the control slot")
        -- The guest, which never saw the keypress, reaches the same conclusion
        -- purely from the canonical rows it received.
        local guest = state.drivers[2]
        for tick = 0, 20 do
            t.eq(
                match_driver.control_slot(guest, state.session.host_peer_id, tick),
                match_driver.control_slot(host, state.session.host_peer_id, tick)
            )
        end
    end)

    t.it("applies one transport-tick arrival batch as one reconciliation", function()
        local state = harness("2v2")
        for _ = 1, 24 do
            local batches = advance(state)
            for _, batch in ipairs(batches) do
                t.is_true(batch.reconciliations <= 1, "more than one reconciliation in one step")
            end
        end
    end)

    t.it("is insensitive to the order peers are polled and stepped", function()
        local forward = harness("2v2")
        run(forward, 32)
        local reversed = harness("2v2")
        for _ = 1, 32 do
            for index = #reversed.drivers, 1, -1 do
                match_driver.advance(reversed.drivers[index], input_frame.neutral_sample())
            end
            reversed.session.host_transport:pump()
        end
        for index = 1, #forward.drivers do
            local boundary = match_driver.diagnostics(forward.drivers[index]).confirmed_output_tick
            t.eq(match_driver.diagnostics(reversed.drivers[index]).confirmed_output_tick, boundary)
            t.eq(
                match_snapshot.hash(
                    assert(match_driver.snapshot(reversed.drivers[index], boundary).snapshot)
                ),
                match_snapshot.hash(
                    assert(match_driver.snapshot(forward.drivers[index], boundary).snapshot)
                )
            )
        end
    end)

    t.it("hands control-channel traffic back instead of eating it", function()
        local state = harness("2v2")
        run(state, 4)
        local guest_id = state.session.guest_peer_ids[1]
        local guest_transport = assert(state.session.guest_transports[guest_id])
        local control = assert(transport_contract.new({
            type = "state",
            seq = 3,
            payload = "GCOP;control",
        }))
        assert(guest_transport:send(transport_contract.HOST_PEER_ID, "control", control))
        state.session.host_transport:pump()
        local batches = advance(state)
        t.eq(#batches[1].control, 1)
        t.eq(batches[1].control[1].channel, "control")
        t.eq(batches[1].control[1].message.payload, "GCOP;control")
        t.eq(match_driver.status(state.drivers[1]), "active")
    end)

    t.it("keeps protected keepers AI-only and slotless", function()
        local state = harness("2v2")
        run(state, 8)
        local snapshot = match_driver.current_snapshot(state.drivers[1])
        for slot = 1, input_frame.SLOT_COUNT do
            local player_index = assert(snapshot.state.slot_players[slot])
            t.is_true(not snapshot.state.players[player_index].is_keeper)
        end
    end)

    t.it("refuses authority from outside a peer's frozen owned set", function()
        local state = harness("2v2")
        run(state, 4)
        local guest_id = state.session.guest_peer_ids[1]
        local guest_transport = assert(state.session.guest_transports[guest_id])
        -- The guest owns home_3/home_4; away_1 belongs to another human.
        local forged = assert(input_protocol.new_guest({
            session_id = state.session.manifest.session_id,
            manifest_id = protocol.manifest_id(state.session.manifest),
            sender_id = guest_id,
            sequence = 9000,
            transport_tick = 8,
            first_input_tick = 0,
            rows = (function()
                local rows = {}
                for tick = 0, 6 do
                    rows[#rows + 1] =
                        { tick = tick, slot_index = 5, sample = input_frame.neutral_sample() }
                end
                return rows
            end)(),
        }))
        local envelope = assert(transport_contract.new({
            type = "input",
            seq = forged.sequence,
            tick = forged.transport_tick,
            payload = assert(input_protocol.encode(forged)),
        }))
        assert(guest_transport:send(transport_contract.HOST_PEER_ID, "input", envelope))
        state.session.host_transport:pump()
        advance(state)
        t.eq(match_driver.status(state.drivers[1]), "ownership_violation")
        local terminal = assert(match_driver.terminal(state.drivers[1]))
        t.eq(terminal.failure, "input_channel")
    end)

    t.it("is idempotent for a replayed bundle and terminal for a conflicting one", function()
        local state = harness("2v2")
        run(state, 6)
        local guest_id = state.session.guest_peer_ids[1]
        local guest_transport = assert(state.session.guest_transports[guest_id])
        local owned_slot = live_slot.slot_index(assert(state.session.freeze.owned[guest_id])[1])

        ---@param edges integer
        ---@return TransportMessage
        local function bundle(edges)
            local rows = {}
            for tick = 0, 6 do
                rows[#rows + 1] = {
                    tick = tick,
                    slot_index = owned_slot,
                    sample = sample({ edges = tick == 6 and edges or 0 }),
                }
            end
            local packet = assert(input_protocol.new_guest({
                session_id = state.session.manifest.session_id,
                manifest_id = protocol.manifest_id(state.session.manifest),
                sender_id = guest_id,
                sequence = 4242,
                transport_tick = 9,
                first_input_tick = 0,
                rows = rows,
            }))
            return assert(transport_contract.new({
                type = "input",
                seq = packet.sequence,
                tick = packet.transport_tick,
                payload = assert(input_protocol.encode(packet)),
            }))
        end

        -- The same sender sequence with byte-identical authority is a no-op.
        local repeated = bundle(0)
        assert(guest_transport:send(transport_contract.HOST_PEER_ID, "input", repeated))
        assert(guest_transport:send(transport_contract.HOST_PEER_ID, "input", repeated))
        state.session.host_transport:pump()
        advance(state)
        t.eq(match_driver.status(state.drivers[1]), "active")

        -- The same identity with different bytes is not.
        assert(guest_transport:send(transport_contract.HOST_PEER_ID, "input", bundle(0)))
        assert(
            guest_transport:send(
                transport_contract.HOST_PEER_ID,
                "input",
                bundle(input_frame.EDGE_BITS.dash)
            )
        )
        state.session.host_transport:pump()
        advance(state)
        t.eq(match_driver.status(state.drivers[1]), "authority_conflict")
        t.eq(assert(match_driver.terminal(state.drivers[1])).failure, "input_channel")
    end)

    t.it("accepts every slot inside a peer's frozen owned set", function()
        local state = harness("1v1")
        run(state, 16)
        -- A 1v1 guest legitimately authors four different slots every tick.
        local diagnostics = match_driver.diagnostics(state.drivers[2])
        t.eq(#diagnostics.authored, 4)
        t.eq(match_driver.status(state.drivers[1]), "active")
        t.eq(match_driver.status(state.drivers[2]), "active")
    end)

    t.it("stops all progress after a terminal status", function()
        local state = harness("2v2")
        run(state, 6)
        local driver = state.drivers[2]
        local before = match_driver.diagnostics(driver)
        for _ = 1, 5 do
            t.is_true(not match_driver.observe_checkpoint(driver, 0, "0000000000000000"))
        end
        t.eq(match_driver.status(driver), "hash_mismatch")
        local batch = match_driver.advance(driver, input_frame.neutral_sample())
        t.eq(#batch.outputs, 0)
        t.eq(batch.sent_packets, 0)
        t.eq(batch.status, "hash_mismatch")
        local after = match_driver.diagnostics(driver)
        t.eq(after.present_input_tick, before.present_input_tick)
        t.eq(after.confirmed_input_tick, before.confirmed_input_tick)
    end)

    t.it("tolerates one boundary disagreement and clears it on the next agreement", function()
        local state = harness("4v4")
        run(state, 34)
        local driver = state.drivers[1]
        local checkpoints = match_driver.checkpoints(driver)
        t.is_true(#checkpoints >= 2)
        t.is_true(
            not match_driver.observe_checkpoint(driver, checkpoints[1].tick, "dead0000dead0000")
        )
        t.is_true(match_driver.observe_checkpoint(driver, checkpoints[1].tick, checkpoints[1].hash))
        t.eq(match_driver.status(driver), "active")
        t.eq(match_driver.diagnostics(driver).hash_mismatches, 0)
    end)

    t.it("publishes confirmed boundary hashes on the documented interval", function()
        local state = harness("4v4")
        run(state, 40)
        local checkpoints = match_driver.checkpoints(state.drivers[1])
        t.is_true(#checkpoints >= 2)
        t.eq(checkpoints[1].tick, 0)
        t.eq(checkpoints[2].tick, match_driver.DEFAULT_HASH_INTERVAL_TICKS)
        for _, checkpoint in ipairs(checkpoints) do
            t.eq(#checkpoint.hash, 16)
        end
    end)

    t.it("ends with a typed completed status at full time", function()
        local state = harness("4v4", { duration = 0.2 })
        for _ = 1, 40 do
            advance(state)
            if match_driver.status(state.drivers[1]) ~= "active" then
                break
            end
        end
        t.eq(match_driver.status(state.drivers[1]), "completed")
        local terminal = assert(match_driver.terminal(state.drivers[1]))
        t.eq(terminal.failure, nil)
    end)

    t.it("converges under impaired delivery after real corrections", function()
        local state = harness("2v2")
        run_bursty(state, 60, 6, { [1] = sample({ move_x = 90 }), [2] = sample({ move_y = -70 }) })
        for _, driver in ipairs(state.drivers) do
            t.eq(match_driver.status(driver), "active")
        end
        local rollbacks = 0
        for _, driver in ipairs(state.drivers) do
            rollbacks = rollbacks + match_driver.diagnostics(driver).rollback_count
        end
        t.is_true(rollbacks > 0, "bursty delivery never produced a correction")
        t.is_true(assert_agreement(state) > 0)
        assert_confirmed_state(state)
    end)

    t.it("converges in 1v1 under impaired delivery with live-slot switching", function()
        local state = harness("1v1")
        run_bursty(state, 60, 5, { [1] = switch_sample(), [2] = switch_sample() })
        t.is_true(match_driver.diagnostics(state.drivers[1]).rollback_count > 0)
        t.is_true(assert_agreement(state) > 0)
        assert_confirmed_state(state)
    end)

    t.it("terminates explicitly when authority falls outside the retained window", function()
        local state = harness("4v4")
        -- Nothing is delivered for well over the 30-tick floor, so confirmation
        -- stops dead and the floor slides past the ticks it was waiting on.
        run_bursty(state, 60, 50)
        local terminal = 0
        for _, driver in ipairs(state.drivers) do
            -- Since #241 this is caught on confirmation liveness rather than on
            -- the arrival that used to reveal it: the peer says so at the step
            -- the tick becomes unconfirmable, not whenever the backlog lands.
            -- Same causal family, same report to the coordinator, strictly
            -- earlier and strictly more precise.
            if match_driver.status(driver) == "confirmation_stalled" then
                terminal = terminal + 1
                local record = assert(match_driver.terminal(driver))
                t.eq(record.failure, "late_input")
                t.is_true(record.tick ~= nil)
            end
        end
        t.is_true(terminal > 0, "an over-window burst never terminated a peer")
    end)

    -- FINDING 1 regression: `confirmed_tick` runs ahead of the simulated present
    -- by up to DELAY_TICKS even with zero loss and zero jitter, because a sample
    -- is authority before it is consumed. Keying a checkpoint's snapshot lookup
    -- off it aborted healthy matches whenever the interval landed on that race.
    t.it("publishes checkpoints on the simulated ceiling, not raw confirmation", function()
        for _, interval in ipairs({ 1, 2, 3, 4 }) do
            local state = harness("1v1", { hash_interval_ticks = interval })
            run(state, 24)
            for _, driver in ipairs(state.drivers) do
                t.eq(
                    match_driver.status(driver),
                    "active",
                    "hash_interval_ticks=" .. tostring(interval)
                )
            end
            local checkpoints = match_driver.checkpoints(state.drivers[1])
            t.is_true(#checkpoints > 1, "interval " .. tostring(interval) .. " published nothing")
            for _, checkpoint in ipairs(checkpoints) do
                -- Never ahead of a boundary that was actually simulated.
                t.is_true(
                    checkpoint.tick <= match_driver.diagnostics(state.drivers[1]).present_input_tick
                )
            end
            t.is_true(assert_agreement(state) > 0)
        end
    end)

    t.it("never publishes a checkpoint boundary that was not simulated", function()
        -- The invariant the fix rests on: the output-capped confirmation is
        -- always at most one boundary behind the present, so `confirmed + 1`
        -- always names a boundary the session actually captured. The raw
        -- confirmation carries no such guarantee, which is what the regression
        -- above demonstrates behaviourally.
        local state = harness("1v1")
        for _ = 1, 24 do
            advance(state)
            for _, driver in ipairs(state.drivers) do
                local diagnostics = match_driver.diagnostics(driver)
                t.is_true(diagnostics.confirmed_output_tick <= diagnostics.confirmed_input_tick)
                t.is_true(diagnostics.confirmed_output_tick < diagnostics.present_input_tick)
                for _, checkpoint in ipairs(match_driver.checkpoints(driver)) do
                    local lookup = match_driver.snapshot(driver, checkpoint.tick)
                    t.is_true(lookup.status == "present" or lookup.status == "retained")
                end
            end
        end
    end)

    -- FINDING 3 / retained-floor edge, revisited by #241. The driver still keeps
    -- no floor pre-check of its own -- `rollback_input_history` owns the floor,
    -- and `spec/sim/rollback_input_history_spec.lua` pins its exact
    -- accept-at-the-floor / refuse-one-below boundary.
    --
    -- What changed is that the driver can no longer *reach* a below-floor
    -- arrival. A row is only offered to the history when it is above this peer's
    -- confirmation, so a row below the floor implies `confirmed + 1 < floor` --
    -- and confirmation liveness terminates on exactly that comparison at the end
    -- of the previous step, before any arrival is applied. The reactive path is
    -- unreachable because the proactive one is strictly earlier and strictly
    -- more precise, which is the whole point of #241's reporting fix.
    t.it("terminates on confirmation liveness before a below-floor row can arrive", function()
        local state = harness("2v2", { max_rollback_ticks = 6 })
        local guest = state.drivers[2]
        -- Only the guest runs. The host is never advanced, so no canonical batch
        -- is ever produced and the guest's seven remote slots never become
        -- authoritative: its confirmation is pinned while its floor keeps
        -- sliding, which is the only regime where the floor rule bites at all.
        for _ = 1, 20 do
            match_driver.advance(guest, input_frame.neutral_sample())
        end
        t.eq(match_driver.status(guest), "confirmation_stalled")
        local diagnostics = match_driver.diagnostics(guest)
        local record = assert(match_driver.terminal(guest))
        t.eq(record.failure, "late_input")
        t.eq(record.tick, diagnostics.confirmed_input_tick + 1)
        t.eq(diagnostics.late_input_tick, record.tick)
        -- The boundary itself, and the reason it is not off by one: the driver
        -- terminates and makes no further progress, so the floor it froze at is
        -- the first one that ever outran confirmation. Authority *at* the floor
        -- is still perfectly placeable, and this fires only one tick past it.
        t.eq(
            diagnostics.retained_floor_tick,
            diagnostics.confirmed_input_tick + 2,
            "confirmation liveness did not fire on the first step it could"
        )
    end)

    -- FINDING 4: the combination the 1v1 case covers, in the other mode that can
    -- exhibit live-slot divergence at all.
    t.it("agrees on the live slot in 2v2 under impaired delivery with switching", function()
        local state = harness("2v2")
        run_bursty(state, 60, 5, {
            [1] = switch_sample(),
            [2] = switch_sample(),
            [3] = sample({ move_x = 80, edges = input_frame.EDGE_BITS.switch }),
        })
        local rollbacks = 0
        for _, driver in ipairs(state.drivers) do
            rollbacks = rollbacks + match_driver.diagnostics(driver).rollback_count
        end
        t.is_true(rollbacks > 0, "bursty 2v2 switching never produced a correction")
        t.is_true(assert_agreement(state) > 0)
        assert_confirmed_state(state)
    end)

    -- FINDING 5: full time under impaired delivery, not only clean delivery.
    t.it("reaches full time under impaired delivery", function()
        local state = harness("2v2", { duration = 0.2 })
        run_bursty(state, 60, 5, { [1] = switch_sample() })
        for _, driver in ipairs(state.drivers) do
            t.eq(match_driver.status(driver), "completed")
            t.eq(assert(match_driver.terminal(driver)).failure, nil)
        end
        t.is_true(match_driver.diagnostics(state.drivers[1]).rollback_count > 0)
    end)

    -- The mechanism: the companion is restored and resimulated at all. It says
    -- nothing about *what* the companion was doing, which is what the seven
    -- named phases below pin.
    t.it("carries the combat companion through correction and resimulation", function()
        local state = harness("2v2", { combat = true })
        local initial = match_driver.current_snapshot(state.drivers[1])
        t.eq(initial.version, match_snapshot.COMBAT_VERSION)
        t.is_true(initial.combat ~= nil)
        run_bursty(state, 48, 5, { [1] = switch_sample(), [3] = sample({ move_y = 60 }) })
        t.is_true(match_driver.diagnostics(state.drivers[1]).rollback_count > 0)
        for _, driver in ipairs(state.drivers) do
            t.eq(match_driver.status(driver), "active")
            t.is_true(match_driver.current_snapshot(driver).combat ~= nil)
        end
        t.is_true(assert_agreement(state) > 0)
        assert_confirmed_state(state)
    end)

    -- #166's seven combat correction phases, each pinned as its own online
    -- rollback scenario.
    --
    -- The acceptance criterion is not "combat happened somewhere during a
    -- bursty run" -- the companion test above already shows that, and it went on
    -- passing unchanged through the whole period when the online bot fill could
    -- not press an equipment button at all. It is that
    -- a correction arriving *while a peer is in that phase* converges: the tick
    -- the rollback resimulated really was a wind-up / guard / contact / flight /
    -- forced / spill / expiry tick, the peers that resimulated it landed on one
    -- identical combat-bearing MatchSnapshot (`match_snapshot.COMBAT_VERSION`),
    -- every confirmed boundary hash still agrees, and nothing terminated on
    -- invalid or duplicate authority.
    --
    -- `batch.outputs` carries two kinds of tick and does not label them:
    -- `apply_rows` appends the reconciliation's `corrected_outputs` first, then
    -- `step_to` appends the ordinary forward tick of the same call. Only the
    -- first kind answers the question, so a phase seen on a forward tick must not
    -- count -- otherwise a break in combat restore-on-rollback specifically,
    -- leaving forward simulation intact, could still pass here.
    --
    -- The two are separated without any production change: a corrected tick is
    -- always strictly below the present boundary as it stood before the call,
    -- because reconciliation restores to the divergence and resimulates back to
    -- the *same* present, while a forward tick is the present itself. That was
    -- checked against ground truth rather than reasoned about -- instrumenting
    -- `rollback_session.reconcile` over all seven scenarios classified 4,334
    -- outputs with zero disagreements -- and `assert_corrected` below keeps it
    -- honest at runtime.
    --
    -- `spec/support/online_combat_phases.lua` owns the fixtures and the
    -- predicates; the delivery pattern and the assertions are this spec's. Five
    -- of the seven reach their phase through `gameplay_ai/combat/v1` itself, from
    -- nothing but an opening pose. `guard` is the exception and says so.
    for _, phase_id in ipairs(combat_phases.PHASES) do
        local scenario = combat_phases.scenario(phase_id)
        t.it(
            ("converges a correction taken during %s (%s)"):format(phase_id, scenario.route),
            function()
                local snapshot = combat_phases.boundary_zero(phase_id)
                t.eq(snapshot.version, match_snapshot.COMBAT_VERSION)
                -- 1v1 is where the AI carries the most of the pitch: three of
                -- every human's four owned slots are AI-driven at any instant,
                -- so six of the eight slots fight without a human behind them.
                local state = harness("1v1", { initial_snapshot = snapshot })
                local first = state.session.freeze.first_input_tick

                ---@type integer[]
                local observed = {}
                for index = 1, #state.drivers do
                    observed[index] = 0
                end
                -- Input tick -> the first peer to resimulate it in this phase and
                -- the hash it landed on, recorded only once that tick is fully
                -- authoritative. Below confirmation a tick still holds predicted
                -- rows, so peers may legitimately differ there.
                ---@type table<integer, { peer: integer, hash: string }>
                local phase_hashes = {}
                local shared = 0

                for step = 0, scenario.steps - 1 do
                    for index, driver in ipairs(state.drivers) do
                        -- Read before the call: this is the boundary a corrected
                        -- tick is strictly below and a forward tick starts at.
                        local present_before = match_driver.diagnostics(driver).present_input_tick
                        local batch = match_driver.advance(
                            driver,
                            combat_phases.live_sample(phase_id, step, index)
                        )
                        if batch.rollbacks > 0 then
                            local confirmed = match_driver.diagnostics(driver).confirmed_input_tick
                            local corrected = 0
                            for _, output in ipairs(batch.outputs) do
                                -- `RollbackTickOutput.tick` is session space.
                                local tick = output.tick + first
                                if tick >= present_before then
                                    -- `step_to`'s forward tick, not the
                                    -- correction's. It proves nothing here.
                                    goto continue
                                end
                                corrected = corrected + 1
                                local before = match_driver.snapshot(driver, tick).snapshot
                                local after = match_driver.snapshot(driver, tick + 1).snapshot
                                if
                                    before ~= nil
                                    and after ~= nil
                                    and combat_phases.observed(
                                        phase_id,
                                        before,
                                        after,
                                        output.combat_events or {}
                                    )
                                then
                                    observed[index] = observed[index] + 1
                                    if tick <= confirmed then
                                        local hash = match_snapshot.hash(before)
                                        local recorded = phase_hashes[tick]
                                        if recorded == nil then
                                            phase_hashes[tick] = { peer = index, hash = hash }
                                        else
                                            t.eq(
                                                hash,
                                                recorded.hash,
                                                ("peers disagreed on the resimulated %s at tick %d"):format(
                                                    phase_id,
                                                    tick
                                                )
                                            )
                                            -- Only a genuinely cross-peer
                                            -- comparison counts: one peer
                                            -- resimulating the same tick twice
                                            -- agrees with itself for free.
                                            if recorded.peer ~= index then
                                                shared = shared + 1
                                            end
                                        end
                                    end
                                end
                                ::continue::
                            end
                            -- The discriminator cannot quietly stop finding
                            -- anything: a reconciliation that changed state
                            -- re-derived at least one tick, by definition.
                            t.is_true(
                                corrected > 0,
                                ("peer %d reported a rollback with no corrected tick below the present"):format(
                                    index
                                )
                            )
                        end
                    end
                    if (step + 1) % scenario.deliver_period == 0 then
                        state.session.host_transport:pump()
                    end
                end

                for index, driver in ipairs(state.drivers) do
                    -- No terminal at all: a duplicate bundle that differed, or
                    -- authority from outside a frozen owned set, would have ended
                    -- this peer as `authority_conflict` / `ownership_violation`
                    -- rather than left it playing.
                    t.eq(
                        match_driver.status(driver),
                        "active",
                        ("peer %d did not survive the %s run"):format(index, phase_id)
                    )
                    t.eq(match_driver.terminal(driver), nil)
                    local diagnostics = match_driver.diagnostics(driver)
                    t.is_true(
                        diagnostics.rollback_count > 0,
                        ("the %s burst never corrected peer %d"):format(phase_id, index)
                    )
                    t.eq(diagnostics.hash_mismatches, 0)
                    t.is_true(
                        observed[index] > 0,
                        ("no correction on peer %d ever resimulated a %s tick"):format(
                            index,
                            phase_id
                        )
                    )
                    -- One boundary is published once. A checkpoint republished
                    -- under a second hash is the duplicate-authority shape the
                    -- coordinator would have to arbitrate.
                    ---@type table<integer, boolean>
                    local published = {}
                    for _, checkpoint in ipairs(match_driver.checkpoints(driver)) do
                        t.is_true(
                            not published[checkpoint.tick],
                            ("peer %d published boundary %d twice"):format(index, checkpoint.tick)
                        )
                        published[checkpoint.tick] = true
                        t.eq(#checkpoint.hash, 16)
                    end
                end
                t.is_true(
                    shared > 0,
                    ("no confirmed %s tick was resimulated by more than one peer"):format(phase_id)
                )
                t.is_true(assert_agreement(state) > 0)
                assert_confirmed_state(state)
            end
        )
    end

    -- The claim the seven scenarios above rest on: none of them starts already
    -- in the phase it is named for. A fixture that force-set `phase = "windup"`
    -- at boundary zero would pin the driver's restore path and nothing about the
    -- simulation reaching wind-up, and the difference is invisible from the
    -- scenario's own assertions.
    t.it("opens every combat phase scenario from a ready combat state", function()
        t.eq(#combat_phases.PHASES, 7)
        for _, phase_id in ipairs(combat_phases.PHASES) do
            local scenario = combat_phases.scenario(phase_id)
            t.is_true(scenario.route == "policy" or scenario.route == "canonical_input")
            local snapshot = combat_phases.boundary_zero(phase_id)
            local companion = assert(snapshot.combat)
            t.eq(#companion.projectiles, 0, phase_id)
            t.eq(#companion.events, 0, phase_id)
            local equipped = 0
            for _, runtime in ipairs(companion.players) do
                t.eq(runtime.phase, "ready", phase_id)
                t.eq(runtime.forced_state, nil, phase_id)
                t.eq(runtime.immunity_ticks, 0, phase_id)
                t.eq(runtime.intent.stage, "idle", phase_id)
                if runtime.family_id ~= nil then
                    equipped = equipped + 1
                    t.is_true(
                        runtime.family_id == scenario.home_family
                            or runtime.family_id == scenario.away_family,
                        ("%s equipped an unexpected family: %s"):format(
                            phase_id,
                            tostring(runtime.family_id)
                        )
                    )
                end
            end
            -- Both keepers are protected and slotless, so every other body is
            -- armed with the family the scenario declares.
            t.eq(equipped, input_frame.SLOT_COUNT, phase_id)
        end
    end)

    -- The evidence behind the `guard` scenario's `canonical_input` route, and the
    -- tripwire that will tell us when it can be promoted to `policy`.
    --
    -- Every geometry arms the whole home side with `guard` and the whole away
    -- side with a family that publishes a readable threat, then lets
    -- `gameplay_ai/combat/v1` play with no human input at all -- and counts the
    -- same thing a phase scenario counts: corrected ticks a peer resimulated in
    -- the `guard` phase.
    --
    -- The claim under test is about *rate*, not possibility. The policy does
    -- guard online: `vs_unarmed_scrum` reaches it, thinly, and the other three
    -- geometries never do. What none of them reaches is
    -- `GUARD_POLICY_ROUTE_MINIMUM` observations on *every* peer, which is the
    -- margin the thinnest shipped scenario runs on. Below that, a policy-route
    -- guard scenario would be one that a tuning nudge or a change of delivery
    -- cadence could silently empty out -- and the cadence really does decide it:
    -- the single commit `vs_unarmed_scrum` produces disappears when the burst
    -- period moves, because a bot fill authors from the state it predicts.
    --
    -- When a geometry does clear the bar this fails, and the fix is to move
    -- `guard` onto the `policy` route in
    -- `spec/support/online_combat_phases.lua` -- not to raise the bar.
    t.it("finds no driver-level geometry where the policy guards often enough", function()
        for _, geometry in ipairs(combat_phases.GUARD_PROBE) do
            local state = harness("1v1", {
                initial_snapshot = combat_phases.guard_probe_boundary_zero(geometry),
            })
            local first = state.session.freeze.first_input_tick
            ---@type integer[]
            local observed = {}
            for index = 1, #state.drivers do
                observed[index] = 0
            end
            local threat_ticks = 0
            for step = 1, geometry.steps do
                for index, driver in ipairs(state.drivers) do
                    local present_before = match_driver.diagnostics(driver).present_input_tick
                    local batch = match_driver.advance(driver, input_frame.neutral_sample())
                    if batch.rollbacks > 0 then
                        for _, output in ipairs(batch.outputs) do
                            local tick = output.tick + first
                            if tick < present_before then
                                local before = match_driver.snapshot(driver, tick).snapshot
                                local after = match_driver.snapshot(driver, tick + 1).snapshot
                                if
                                    before ~= nil
                                    and after ~= nil
                                    and combat_phases.observed(
                                        "guard",
                                        before,
                                        after,
                                        output.combat_events or {}
                                    )
                                then
                                    observed[index] = observed[index] + 1
                                end
                            end
                        end
                    end
                end
                if step % geometry.deliver_period == 0 then
                    state.session.host_transport:pump()
                end
                -- Away runtimes are indexes 7..10; 6 is the protected keeper.
                local companion = assert(match_driver.current_snapshot(state.drivers[1]).combat)
                for index = 7, #companion.players do
                    local runtime = companion.players[index]
                    if runtime.phase == "windup" or runtime.phase == "active" then
                        threat_ticks = threat_ticks + 1
                    end
                end
                threat_ticks = threat_ticks + #companion.projectiles
            end
            -- Without a readable threat the policy could not guard even in
            -- principle, and a low count here would mean nothing at all.
            t.is_true(
                threat_ticks > 0,
                ("%s never telegraphed a threat, so it probes nothing"):format(geometry.id)
            )
            local weakest = nil
            for _, count in ipairs(observed) do
                weakest = (weakest == nil or count < weakest) and count or weakest
            end
            t.is_true(
                assert(weakest) < combat_phases.GUARD_POLICY_ROUTE_MINIMUM,
                (
                    "%s now reaches %d corrected guard ticks on every peer -- promote the guard "
                    .. "scenario to the policy route"
                ):format(geometry.id, assert(weakest))
            )
        end
    end)

    -- A guest's own rows skip the reconciliation pass because they cannot open a
    -- divergence. The skip is guarded by a runtime `earliest_divergence` check
    -- that real traffic never trips, so the fallback is forced here: a safety
    -- net nothing ever exercises is not yet a safety net.
    t.it("still reconciles if a local insert ever reports a divergence", function()
        local state = harness("2v2")
        run(state, 12)
        local guest = state.drivers[2]

        local reconciles = 0
        local original_reconcile = rollback_session.reconcile
        local function count_reconciles()
            rollback_session.reconcile = function(...)
                reconciles = reconciles + 1
                return original_reconcile(...)
            end
        end

        -- Baseline: every reconciliation on a normal step is accounted for by an
        -- arrival batch. The guest's own authoring adds none.
        count_reconciles()
        local ok, batch = pcall(match_driver.advance, guest, input_frame.neutral_sample())
        rollback_session.reconcile = original_reconcile
        t.is_true(ok, tostring(batch))
        t.eq(reconciles, batch.reconciliations, "a clean local insert reconciled anyway")

        -- Force the fallback: report a divergence the real path never produces.
        reconciles = 0
        local arrival_reconciles = 0
        local original_add = rollback_session.add_authoritative_batch
        rollback_session.add_authoritative_batch = function(session, arrivals)
            local accepted, err, code = original_add(session, arrivals)
            if accepted ~= nil then
                accepted.earliest_divergence = accepted.confirmed_tick
            end
            return accepted, err, code
        end
        count_reconciles()
        local forced_ok, forced_err = pcall(function()
            for _ = 1, 6 do
                local batches = advance(state)
                arrival_reconciles = arrival_reconciles + batches[2].reconciliations
            end
        end)
        rollback_session.add_authoritative_batch = original_add
        rollback_session.reconcile = original_reconcile
        t.is_true(forced_ok, tostring(forced_err))
        t.is_true(
            reconciles > arrival_reconciles,
            "the divergence fallback never reconciled beyond the arrival batches"
        )

        -- And the driver is still healthy and still converging afterwards.
        t.eq(match_driver.status(guest), "active")
        run(state, 12)
        t.is_true(assert_agreement(state) > 0)
        assert_confirmed_state(state)
    end)

    t.it("costs no extra snapshot work when a peer authors only its control slot", function()
        local state = harness("4v4")
        local guest = state.drivers[2]
        t.eq(#match_driver.diagnostics(guest).authored, 1)
        local captures = 0
        local original = rollback_session.current_snapshot
        rollback_session.current_snapshot = function(...)
            captures = captures + 1
            return original(...)
        end
        local ok, err = pcall(function()
            for _ = 1, 8 do
                match_driver.advance(guest, input_frame.neutral_sample())
            end
        end)
        rollback_session.current_snapshot = original
        t.is_true(ok, tostring(err))
        t.eq(captures, 0, "a singleton owned set still paid a capture-and-restore")

        -- And the peer that does author AI rows pays it once per step, not more.
        local multi = harness("1v1")
        captures = 0
        rollback_session.current_snapshot = function(...)
            captures = captures + 1
            return original(...)
        end
        local multi_ok, multi_err = pcall(function()
            for _ = 1, 8 do
                match_driver.advance(multi.drivers[1], input_frame.neutral_sample())
            end
        end)
        rollback_session.current_snapshot = original
        t.is_true(multi_ok, tostring(multi_err))
        t.eq(captures, 8)
    end)

    -- #237. The driver used to terminate at *present* full time, leaving up to
    -- DELAY_TICKS of the match unconfirmed at the moment it reported the result.
    -- These pin the settle phase that closes it.

    t.it("settles the final boundary before completing under clean delivery", function()
        for _, mode in ipairs({ "1v1", "2v2", "4v4" }) do
            local state = harness(mode, { duration = SETTLE_DURATION })
            drive(state, FULL_TIME_BOUNDARY + 20, function()
                return true
            end, moving_sample)
            t.eq(assert_settled(state, mode), FULL_TIME_BOUNDARY)
            for index, driver in ipairs(state.drivers) do
                -- Clean delivery confirms ahead of the present, so settling is
                -- all but free for a guest: one step behind the fan-out that
                -- carries the final row to it, plus one more for the host batch
                -- that reports the host's own confirmation back.
                --
                -- That second step is #243's, and it is the price of the guest
                -- side of `tail_delivered`. A guest's authored tail exists nowhere
                -- else until the sequencer has it, so "my own boundary is
                -- confirmed" is not evidence that the rows it wrote ever landed;
                -- seed 4738 of `4v4.stress` is the row where a guest left on
                -- exactly that reasoning and took two ticks of authority with it.
                -- One extra step -- 17 ms at the whistle -- buys the guarantee,
                -- and it is pinned here so it can never quietly grow.
                --
                -- The host pays no more than a guest does. It waits on the same
                -- evidence -- every author it has heard from reporting the final
                -- boundary confirmed -- and under clean delivery those reports
                -- ride the bundles already in flight. #255 removed the four-step
                -- quiet window that used to sit above this as the host's own
                -- bound; it was never the binding constraint on a clean match,
                -- and this pins that the host is no slower without it.
                local allowed = 2
                local steps = match_driver.diagnostics(driver).settle_steps
                t.is_true(
                    steps <= allowed,
                    ("peer %d settled slowly under clean delivery in %s: %d steps"):format(
                        index,
                        mode,
                        steps
                    )
                )
            end
        end
    end)

    -- The regression test. The pre-existing full-time coverage runs bursty
    -- delivery *up to* full time, which is why this was missed: the burst has to
    -- straddle the final whistle for peers to stop at different confirmation
    -- depths.
    t.it("completes with an agreed final hash under a burst across full time", function()
        for _, mode in ipairs({ "1v1", "2v2", "4v4" }) do
            local state = harness(mode, { duration = SETTLE_DURATION })
            drive(state, FULL_TIME_BOUNDARY + 90, function(step)
                return step < FULL_TIME_STEP - 6 or step > FULL_TIME_STEP + 4
            end, moving_sample)
            assert_settled(state, mode)
            local rollbacks, settle_steps = 0, 0
            for _, driver in ipairs(state.drivers) do
                local diagnostics = match_driver.diagnostics(driver)
                rollbacks = rollbacks + diagnostics.rollback_count
                settle_steps = settle_steps + diagnostics.settle_steps
            end
            t.is_true(rollbacks > 0, ("the burst never corrected anything in %s"):format(mode))
            -- And the burst really did leave a tail to drain, so this is the
            -- path under test rather than the clean one in disguise.
            t.is_true(settle_steps > 0, ("nothing was left to settle in %s"):format(mode))
        end
    end)

    t.it("does not swallow a boundary disagreement reported while settling", function()
        local state = harness("2v2", { duration = SETTLE_DURATION, settle_timeout_ticks = 40 })
        drive(state, FULL_TIME_STEP + 1, function(step)
            return step < FULL_TIME_STEP - 5
        end, moving_sample)
        local driver = state.drivers[1]
        t.is_true(match_driver.diagnostics(driver).settling, "the peer was not settling")

        -- A peer disagreeing about a boundary this driver hashed is exactly the
        -- report the coordinator forwards. Settling must not make it wait it out.
        local checkpoint = assert(match_driver.checkpoints(driver)[1])
        for _ = 1, match_driver.MAX_HASH_MISMATCHES do
            t.is_true(
                not match_driver.observe_checkpoint(driver, checkpoint.tick, "dead0000dead0000")
            )
        end
        t.eq(match_driver.status(driver), "hash_mismatch")
        t.eq(assert(match_driver.terminal(driver)).failure, "desync")
        t.is_true(not match_driver.settled(driver))

        -- Delivery resumes and the tail would now confirm. A settle phase that
        -- completed anyway would have converted a real divergence into a result.
        drive(state, 40, function()
            return true
        end, moving_sample)
        t.eq(match_driver.status(driver), "hash_mismatch")
        t.is_true(not match_driver.settled(driver))
    end)

    t.it("settles a genuinely divergent peer without hiding the divergence", function()
        -- Peer two simulates from a differently seeded boundary zero: every
        -- input row still agrees, so every peer confirms every tick and settles,
        -- but the states never do. Settling waits for *authority*, never for
        -- agreement, so the disagreement survives into the final hash the
        -- session acknowledges -- where `coordinator.apply_result_ack` ends it as
        -- `hash_mismatch` (pinned in spec/game/online_coordinator_spec.lua).
        local state = harness("2v2", { duration = SETTLE_DURATION, divergent_peer = 2 })
        drive(state, FULL_TIME_BOUNDARY + 20, function()
            return true
        end, moving_sample)
        local boundary = nil
        for _, driver in ipairs(state.drivers) do
            t.eq(match_driver.status(driver), "completed")
            t.is_true(match_driver.settled(driver))
            boundary = assert(match_driver.full_time_boundary(driver))
        end
        boundary = assert(boundary)
        local hashes = {}
        for index, driver in ipairs(state.drivers) do
            hashes[index] =
                match_snapshot.hash(assert(match_driver.snapshot(driver, boundary).snapshot))
        end
        t.is_true(hashes[1] ~= hashes[2], "a divergent peer settled onto an agreed final hash")
        -- The driver's own comparison would fire on the same evidence: the
        -- boundaries it published during play already disagree.
        t.is_true(
            not match_driver.observe_checkpoint(
                state.drivers[3],
                assert(match_driver.checkpoints(state.drivers[2])[1]).tick,
                assert(match_driver.checkpoints(state.drivers[2])[1]).hash
            )
        )
    end)

    t.it("ends a settle nobody can finish with a bounded typed reason", function()
        local state = harness("2v2", { duration = SETTLE_DURATION, settle_timeout_ticks = 10 })
        -- Delivery stops before full time and never resumes: every peer reaches
        -- the final tick on predicted rows and none of them can ever confirm it.
        drive(state, FULL_TIME_BOUNDARY + 60, function(step)
            return step < FULL_TIME_STEP - 5
        end, moving_sample)
        for index, driver in ipairs(state.drivers) do
            local status = match_driver.status(driver)
            t.eq(status, "settle_timeout", ("peer %d did not time out"):format(index))
            -- Typed, and emphatically not the desync a healthy match used to get.
            t.is_true(status ~= "hash_mismatch")
            local terminal = assert(match_driver.terminal(driver))
            t.eq(terminal.failure, "input_channel")
            t.eq(terminal.tick, match_driver.full_time_boundary(driver))
            t.is_true(not match_driver.settled(driver))
            t.eq(match_driver.diagnostics(driver).settle_steps, 10)
        end

        -- No hidden progress after the settle phase ends, same as every other
        -- terminal status.
        local driver = state.drivers[1]
        local before = match_driver.diagnostics(driver)
        local batch = match_driver.advance(driver, input_frame.neutral_sample())
        t.eq(#batch.outputs, 0)
        t.eq(batch.sent_packets, 0)
        t.eq(batch.status, "settle_timeout")
        local after = match_driver.diagnostics(driver)
        t.eq(after.present_input_tick, before.present_input_tick)
        t.eq(after.confirmed_input_tick, before.confirmed_input_tick)
        t.eq(after.settle_steps, before.settle_steps)
    end)

    t.it("bounds the settle phase in wall clock as well as in ticks", function()
        -- One second of monotonic time per reading, so a caller whose frames
        -- have stopped arriving at 60 Hz cannot stretch a bounded number of
        -- steps into an unbounded wait.
        local now = 0
        local state = harness("2v2", {
            duration = SETTLE_DURATION,
            settle_timeout_ticks = 10000,
            settle_timeout_seconds = 2,
            clock = function()
                now = now + 1
                return now
            end,
        })
        drive(state, FULL_TIME_BOUNDARY + 60, function(step)
            return step < FULL_TIME_STEP - 5
        end, moving_sample)
        for index, driver in ipairs(state.drivers) do
            t.eq(match_driver.status(driver), "settle_timeout", ("peer %d"):format(index))
            local detail = assert(match_driver.terminal(driver)).detail
            t.is_true(
                detail:find("seconds", 1, true) ~= nil,
                "the wall-clock bound was not the one that fired: " .. detail
            )
            -- Far short of the tick bound, which is the point.
            t.is_true(match_driver.diagnostics(driver).settle_steps < 10)
        end
    end)

    t.it("re-publishes the tail while settling and simulates nothing", function()
        local state = harness("2v2", { duration = SETTLE_DURATION, settle_timeout_ticks = 40 })
        drive(state, FULL_TIME_STEP + 1, function(step)
            return step < FULL_TIME_STEP - 5
        end, moving_sample)
        for index, driver in ipairs(state.drivers) do
            local before = match_driver.diagnostics(driver)
            t.is_true(before.settling, ("peer %d was not settling"):format(index))
            t.eq(before.status, "active")
            local batch = match_driver.advance(driver, input_frame.neutral_sample())
            -- Nothing is simulated after full time, ever.
            t.eq(#batch.outputs, 0, ("peer %d simulated after full time"):format(index))
            local after = match_driver.diagnostics(driver)
            t.eq(after.present_input_tick, before.present_input_tick)
            t.eq(after.present_input_tick, assert(before.full_time_boundary))
            -- But the last authored window keeps going out, which is how a peer
            -- that lost the tail can still receive it.
            t.is_true(
                batch.sent_packets > 0 or index == 1,
                ("settling peer %d stopped re-publishing its tail"):format(index)
            )
        end
        -- The host publishes through its own collector, so its re-sends leave on
        -- the canonical batch a step later rather than immediately.
        local host_batch = match_driver.advance(state.drivers[1], input_frame.neutral_sample())
        t.is_true(host_batch.sent_packets > 0, "a settling host stopped fanning out authority")
    end)

    -- #241, the mid-match half. Confirmation could stop advancing permanently
    -- with nothing raised at the time: the retained floor slid past a tick that
    -- never got its eighth row, that tick's authority was deleted, and because
    -- confirmation only advances from `confirmed_tick + 1` it could never cross
    -- the hole again. The reactive `late_input` check cannot see it, because it
    -- fires on a row that *arrives* below the floor and this is a row that never
    -- arrives at all.
    t.it("reports a stalled confirmation at the step it becomes permanent", function()
        -- Long enough that the retained floor can outrun a hole without full
        -- time arriving first and turning this into a settle question.
        local state = harness("2v2", { duration = 2 })
        -- Delivery stops for good well before full time. Every peer keeps
        -- simulating on predicted rows, and thirty steps later the floor passes
        -- the first tick that never became authoritative.
        drive(state, 90, function(step)
            return step < 12
        end, moving_sample)
        for index, driver in ipairs(state.drivers) do
            local status = match_driver.status(driver)
            t.eq(status, "confirmation_stalled", ("peer %d did not report its stall"):format(index))
            -- Distinct from both statuses that used to absorb this, which is
            -- what #169's harness and #171's evidence gate need it to be.
            t.is_true(status ~= "settle_timeout")
            t.is_true(status ~= "hash_mismatch")
            local terminal = assert(match_driver.terminal(driver))
            t.eq(terminal.failure, "late_input")
            local diagnostics = match_driver.diagnostics(driver)
            -- At the stall, not at the whistle: full time was never reached, so
            -- the settle phase never even opened.
            t.eq(diagnostics.full_time_boundary, nil)
            t.eq(diagnostics.settle_steps, 0)
            -- And it names the tick that never became authoritative.
            t.eq(terminal.tick, diagnostics.confirmed_input_tick + 1)
            t.is_true(
                diagnostics.retained_floor_tick > diagnostics.confirmed_input_tick + 1,
                ("peer %d reported a stall it was not in"):format(index)
            )
        end
    end)

    -- The defensive fallback, exercised on purpose because nothing else can.
    --
    -- `rollback_input_history` still owns the retained floor and still rejects an
    -- over-window row with `outside_window`, and the driver still maps that onto
    -- `late_input`. Since #241 no *arrival* can reach it: a row is only offered to
    -- the history when it is above this peer's confirmation, so a row below the
    -- floor implies `confirmed + 1 < floor`, which confirmation liveness
    -- terminates on at the end of the previous step. The mapping is correct to
    -- keep — it is the fallback for a rule this driver does not own — but a safety
    -- net that nothing exercises is not yet a safety net, so this forces it
    -- directly rather than leaving it uncovered as a side effect of the fix.
    t.it("maps a rejected over-window batch onto late_input (unreachable by design)", function()
        local state = harness("2v2")
        -- A backlog, so the arriving batch genuinely carries rows above this
        -- peer's confirmation and the session call is actually reached.
        run_bursty(state, 8, 4)
        local guest = state.drivers[2]
        t.eq(match_driver.status(guest), "active")

        local original = rollback_session.apply_authoritative_batch
        local reached = 0
        ---@diagnostic disable-next-line: duplicate-set-field
        rollback_session.apply_authoritative_batch = function()
            reached = reached + 1
            return nil, "forced retained-window rejection", "outside_window"
        end
        local ok, err = pcall(match_driver.advance, guest, input_frame.neutral_sample())
        rollback_session.apply_authoritative_batch = original
        t.is_true(ok, tostring(err))

        t.is_true(reached > 0, "the arrival path never reached the session")
        t.eq(match_driver.status(guest), "late_input")
        local terminal = assert(match_driver.terminal(guest))
        t.eq(terminal.failure, "late_input")
        -- The history's own message survives, so this is the mapped rejection
        -- rather than some other terminal arriving at the same moment.
        t.eq(terminal.detail, "forced retained-window rejection")
        -- And the causal tick is attributed and published, which is the part a
        -- desync capture reads.
        t.is_true(terminal.tick ~= nil)
        t.eq(match_driver.diagnostics(guest).late_input_tick, terminal.tick)
    end)

    -- #241, the fan-out half. One host batch is meant to carry the full seven-row
    -- redundancy window for every slot. Relaying only what arrived on this
    -- transport tick spent far less of it: a slot whose author's bundle was lost
    -- or delayed contributed nothing at all, so the guest-to-host leg's losses
    -- multiplied into the host-to-guest leg, which has no retransmission.
    --
    -- The full-window figure and `MAX_HOST_ROWS` used to be the same number; #243
    -- separated them. A full window for eight slots is still 56 rows and is still
    -- what a healthy steady state emits -- what changed is that the bound is now
    -- 72, sized to the byte budget, so the 16 rows above it are headroom the
    -- targeted repair can spend. This asserts the window, and asserts it sits
    -- inside the bound rather than at it.
    t.it("fans out a full redundancy window for a slot that sent nothing", function()
        ---@type TransportMessage[]
        local sent = {}
        local state = harness("4v4", {
            wrap_host_transport = function(inner)
                local recorder = {}
                function recorder:poll_event()
                    return inner:poll_event()
                end
                function recorder:poll_batch(limit)
                    return inner:poll_batch(limit)
                end
                function recorder:send(peer_id, channel, envelope)
                    return inner:send(peer_id, channel, envelope)
                end
                function recorder:broadcast(channel, envelope)
                    if channel == "input" then
                        sent[#sent + 1] = envelope
                    end
                    return inner:broadcast(channel, envelope)
                end
                function recorder:diagnostics()
                    return inner:diagnostics()
                end
                return recorder
            end,
        })
        -- Warm every slot: each guest has published a full window and the host
        -- has accepted it.
        run(state, 12)
        t.is_true(#sent > 0, "the host never fanned anything out")

        -- Now only the host advances. The first step drains what the last pump
        -- delivered; the second has no guest traffic at all, which is exactly
        -- the shape a lost or delayed bundle produces -- here for seven slots at
        -- once rather than one.
        match_driver.advance(state.drivers[1], input_frame.neutral_sample())
        state.session.host_transport:pump()
        local before = #sent
        match_driver.advance(state.drivers[1], input_frame.neutral_sample())
        t.eq(#sent, before + 1, "the host did not fan out a batch")

        local packet = assert(input_protocol.decode(assert(sent[#sent]).payload, {
            session_id = state.session.manifest.session_id,
            manifest_id = protocol.manifest_id(state.session.manifest),
            sender_id = state.session.host_peer_id,
        }))
        ---@type table<integer, integer>
        local per_slot = {}
        for _, row in ipairs(packet.rows) do
            per_slot[row.slot_index] = (per_slot[row.slot_index] or 0) + 1
        end
        for slot_index = 1, input_frame.SLOT_COUNT do
            t.eq(
                per_slot[slot_index],
                input_protocol.RETAINED_ROWS,
                ("slot %d was not fanned out with a full window"):format(slot_index)
            )
        end
        -- A full window for every slot, with the repair headroom left unspent:
        -- no guest is behind here, so nothing has asked for a repair.
        t.eq(#packet.rows, input_frame.SLOT_COUNT * input_protocol.RETAINED_ROWS)
        t.is_true(#packet.rows < input_protocol.MAX_HOST_ROWS)
    end)

    -- #241, the tail half. The host is the star's only relay and, as sequencer,
    -- structurally the first peer to confirm the final boundary -- so
    -- "confirmed, therefore done" made it leave first every time, and a guest
    -- still missing a tail row could never obtain it afterwards.
    t.it("keeps the host relaying until its guests have stopped asking", function()
        for _, mode in ipairs({ "2v2", "4v4" }) do
            local state = harness(mode, { duration = SETTLE_DURATION })
            ---@type table<integer, integer>
            local left = {}
            for step = 1, FULL_TIME_BOUNDARY + 90 do
                for index, driver in ipairs(state.drivers) do
                    match_driver.advance(driver, moving_sample(state.step, index))
                    if left[index] == nil and match_driver.status(driver) ~= "active" then
                        left[index] = step
                    end
                end
                -- A burst straddling full time, so there is a tail to drain and
                -- the guests really are still asking when the host confirms.
                if state.step < FULL_TIME_STEP - 6 or state.step > FULL_TIME_STEP + 4 then
                    state.session.host_transport:pump()
                end
                state.step = state.step + 1
            end
            local host_left = assert(left[1], ("the host never left in %s"):format(mode))
            for index = 2, #state.drivers do
                t.eq(
                    match_driver.status(state.drivers[index]),
                    "completed",
                    ("guest %d did not complete in %s"):format(index, mode)
                )
                t.is_true(
                    host_left >= assert(left[index]),
                    ("the host left the star before guest %d in %s"):format(index, mode)
                )
            end
            t.eq(match_driver.status(state.drivers[1]), "completed")
            t.is_true(match_driver.settled(state.drivers[1]))
        end
    end)

    -- #255. The exact case the retired quiet count used to decide: a peer that
    -- reported itself behind and *then* fell silent. `tail_delivered` tested four
    -- consecutive silent settle steps before it read any report, so the host
    -- concluded "drained" while holding that peer's own evidence to the contrary
    -- -- silence overriding evidence rather than standing in for evidence that
    -- never came.
    --
    -- The host now keeps relaying for that peer until the settle deadline. That
    -- is the whole cost of the change, and it is bounded: this pins that the host
    -- waits *all* of it and then leaves with a typed terminal rather than hanging,
    -- and that a silent straggler still gets its own typed terminal too.
    t.it("keeps relaying for a peer that reported behind and then went silent", function()
        local settle_ticks = 20
        -- Blocked inbound authority a few steps before full time, so the guest
        -- cannot confirm the tail and every bundle it sends says so.
        local blackout_from = FULL_TIME_STEP - 8
        -- Silent from the first settle step onward, so the host's only evidence
        -- about this peer is the stale report it already holds.
        local silent_from = FULL_TIME_STEP + 1
        local step = 0
        ---@type DriverHarness
        local state
        state = harness("1v1", {
            duration = SETTLE_DURATION,
            settle_timeout_ticks = settle_ticks,
            wrap_guest_transport = function(index, inner)
                if index ~= 2 then
                    return inner
                end
                local filter = setmetatable({}, { __index = inner })
                function filter:poll_batch(limit)
                    local messages = inner:poll_batch(limit)
                    if step < blackout_from then
                        return messages
                    end
                    local kept = {}
                    for _, entry in ipairs(messages) do
                        if entry.message.type ~= "input" then
                            kept[#kept + 1] = entry
                        end
                    end
                    return kept
                end
                function filter:send(peer_id, channel, envelope)
                    if channel == "input" and step >= silent_from then
                        -- The wire accepted it and the network ate it: the host
                        -- hears nothing further from this peer.
                        return true
                    end
                    return inner:send(peer_id, channel, envelope)
                end
                return filter
            end,
        })

        local reported_at_silence = nil
        for _ = 1, FULL_TIME_BOUNDARY + settle_ticks + 40 do
            step = state.step
            if step == silent_from then
                reported_at_silence =
                    match_driver.diagnostics(state.drivers[2]).confirmed_input_tick
            end
            for index, driver in ipairs(state.drivers) do
                match_driver.advance(driver, moving_sample(step, index))
            end
            state.session.host_transport:pump()
            state.step = step + 1
        end

        -- The premise: the guest really was behind the final boundary at the
        -- moment it stopped speaking, so the host is holding a report that says
        -- so. Without that this test would pass for the wrong reason.
        local boundary = assert(match_driver.full_time_boundary(state.drivers[1]))
        t.is_true(
            assert(reported_at_silence) + 1 < boundary,
            ("the guest was not behind when it went silent: %d vs %d"):format(
                assert(reported_at_silence),
                boundary
            )
        )

        -- The host waited the whole phase out rather than four silent steps, and
        -- still left -- completed, because its own final boundary is confirmed.
        local host = match_driver.diagnostics(state.drivers[1])
        t.eq(host.status, "completed")
        t.is_true(match_driver.settled(state.drivers[1]))
        t.eq(host.settle_steps, settle_ticks, "the host stopped relaying before the deadline")

        -- And the straggler is bounded by the same deadline, with the typed
        -- terminal it would have had either way.
        local guest = match_driver.diagnostics(state.drivers[2])
        t.eq(guest.status, "settle_timeout")
        t.eq(assert(guest.terminal).failure, "input_channel")
        t.eq(guest.settle_steps, settle_ticks)
    end)

    -- #243, the repair half. Blind redundancy re-sends a row for seven transport
    -- ticks and then stops, so a guest that loses every one of those seven can
    -- never obtain that row from the ordinary fan-out again -- and because
    -- confirmation only advances from `confirmed_tick + 1`, that one hole freezes
    -- it for the rest of the match.
    --
    -- The bundle it already sends every tick now reports where its confirmation
    -- actually is, so the host can aim a re-send at the gap instead of guessing.
    -- This drops a burst wider than the redundancy window on one guest's inbound
    -- link and asserts three things: it really did fall out of the window, the
    -- host really did spend rows on a tick older than the window, and the guest
    -- really did recover rather than stall.
    t.it("repairs a guest whose hole has aged out of the redundancy window", function()
        local blackout_from, blackout_through = 14, 26
        local step = 0
        ---@type integer[]
        local repaired_ticks = {}
        ---@type DriverHarness
        local state
        state = harness("4v4", {
            wrap_guest_transport = function(index, inner)
                if index ~= 8 then
                    return inner
                end
                local filter = setmetatable({}, { __index = inner })
                function filter:poll_batch(limit)
                    local messages = inner:poll_batch(limit)
                    if step < blackout_from or step > blackout_through then
                        return messages
                    end
                    local kept = {}
                    for _, entry in ipairs(messages) do
                        if entry.message.type ~= "input" then
                            kept[#kept + 1] = entry
                        end
                    end
                    return kept
                end
                return filter
            end,
            wrap_host_transport = function(inner)
                local recorder = setmetatable({}, { __index = inner })
                function recorder:broadcast(channel, envelope)
                    if channel == "input" then
                        local packet = input_protocol.decode(envelope.payload, {
                            session_id = state.session.manifest.session_id,
                            manifest_id = protocol.manifest_id(state.session.manifest),
                            sender_id = state.session.host_peer_id,
                        })
                        if packet ~= nil then
                            local newest = packet.rows[#packet.rows].tick
                            local oldest = packet.rows[1].tick
                            if newest - oldest > input_protocol.HISTORY_ROWS then
                                repaired_ticks[#repaired_ticks + 1] = oldest
                            end
                        end
                    end
                    return inner:broadcast(channel, envelope)
                end
                return recorder
            end,
        })

        local stalled_during = nil
        for _ = 1, 110 do
            advance(state)
            step = state.step
            if step == blackout_through + 2 then
                stalled_during = match_driver.diagnostics(state.drivers[8]).confirmed_input_tick
            end
        end

        -- The blackout was wider than the window, so the guest is genuinely
        -- behind the peers that kept receiving. Without that this test would pass
        -- on a repair that never had anything to repair.
        local reference = match_driver.diagnostics(state.drivers[2]).confirmed_input_tick
        t.is_true(
            assert(stalled_during) < reference,
            "the blackout did not put the guest behind its peers"
        )
        -- The host spent rows on a tick older than blind redundancy would ever
        -- re-send: that only happens when a guest reported where it was stuck.
        t.is_true(#repaired_ticks > 0, "the host never fanned out a repaired tick")
        -- And the point of all of it: the guest confirms past the hole instead of
        -- freezing at it, so no peer reaches `confirmation_stalled`.
        for index, driver in ipairs(state.drivers) do
            t.is_true(
                match_driver.status(driver) ~= "confirmation_stalled",
                ("peer %d stalled its confirmation"):format(index)
            )
        end
        t.is_true(
            match_driver.diagnostics(state.drivers[8]).confirmed_input_tick > assert(stalled_during),
            "the repaired guest never confirmed past its hole"
        )
    end)

    -- #243, the settle half, and #241's tail stall in the other direction. A
    -- guest's authored tail exists nowhere else until the host has it, so a guest
    -- that leaves on "my own boundary is confirmed" can take rows with it that
    -- every other peer -- the host included -- is still missing.
    --
    -- The host reports its own confirmation in the batches it already broadcasts,
    -- so the guest can see the sequencer is behind and keep re-publishing. It
    -- re-publishes the window the host is actually missing rather than the newest
    -- one, which is the difference between seven rows aimed at the gap and seven
    -- rows aimed past it.
    t.it("keeps a guest re-publishing until the host has confirmed its tail", function()
        local state = harness("4v4", { duration = SETTLE_DURATION })
        local host_blind_until = FULL_TIME_STEP + 12
        for _ = 1, FULL_TIME_BOUNDARY + 90 do
            for index, driver in ipairs(state.drivers) do
                match_driver.advance(driver, moving_sample(state.step, index))
            end
            -- A burst on the guest-to-host leg straddling full time: the host
            -- stops hearing one author exactly when its last rows are authored.
            if state.step < FULL_TIME_STEP - 8 or state.step > host_blind_until then
                state.session.host_transport:pump()
            end
            state.step = state.step + 1
        end
        for index, driver in ipairs(state.drivers) do
            t.eq(
                match_driver.status(driver),
                "completed",
                ("peer %d did not complete"):format(index)
            )
            t.is_true(match_driver.settled(driver), ("peer %d never settled"):format(index))
        end
    end)

    t.it("reports a lost transport as a typed terminal status", function()
        local state = harness("2v2")
        run(state, 4)
        local guest_id = state.session.guest_peer_ids[1]
        assert(state.session.host_transport:close_peer(guest_id, "link failed"))
        state.session.host_transport:pump()
        advance(state)
        advance(state)
        t.is_true(match_driver.status(state.drivers[2]) ~= "active")
    end)
end)
