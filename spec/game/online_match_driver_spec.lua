local t = require("spec.support.runner")
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
---@field hash_interval_ticks integer?
---@field max_rollback_ticks integer?
---@field settle_timeout_ticks integer?
---@field settle_timeout_seconds number?
---@field clock (fun(): number)?
---@field divergent_peer integer? -- Driver index whose boundary zero is seeded differently.
---@field wrap_host_transport (fun(inner: StarTransportAdapter): StarTransportAdapter)?

---@param mode SessionMatchMode
---@param options DriverHarnessOptions?
---@return DriverHarness
local function harness(mode, options)
    options = options or {}
    local session = fixture.session(mode, nil, options.humans)
    local snapshot = fixture.initial_snapshot(options.duration, options.combat)
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
                -- all but free for a guest: it is exactly one step behind the
                -- fan-out that carries the final row to it. The healthy guest
                -- path stays as prompt as it was.
                --
                -- The host pays the relay quiet window on top, because it is the
                -- star's only relay and "my own boundary is confirmed" is not
                -- evidence that nobody still needs it -- it is, in fact, the
                -- first peer to confirm. That is a bounded 67 ms at the whistle,
                -- and it is pinned here so it can never quietly grow.
                --
                -- The host's bound is the quiet window plus two: the step it
                -- reaches full time (its guests are still mid-play and sending),
                -- and the step their last in-flight bundle lands on.
                local allowed = index == 1 and match_driver.SETTLE_RELAY_QUIET_STEPS + 2 or 1
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

    -- #241, the fan-out half. `MAX_HOST_ROWS` is eight slots times the seven-row
    -- redundancy window because one host batch is meant to carry a full window
    -- for every slot. Relaying only what arrived on this transport tick spent
    -- far less of it: a slot whose author's bundle was lost or delayed
    -- contributed nothing at all, so the guest-to-host leg's losses multiplied
    -- into the host-to-guest leg, which has no retransmission.
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
        -- Exactly the bound the batch was always sized for, and never past it.
        t.eq(#packet.rows, input_protocol.MAX_HOST_ROWS)
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
