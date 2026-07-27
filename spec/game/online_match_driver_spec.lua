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

---@param mode SessionMatchMode
---@param options DriverHarnessOptions?
---@return DriverHarness
local function harness(mode, options)
    options = options or {}
    local session = fixture.session(mode, nil, options.humans)
    local snapshot = fixture.initial_snapshot(options.duration, options.combat)
    local drivers = {}
    local peer_ids = { session.host_peer_id }
    drivers[1] = match_driver.new({
        role = "host",
        peer_id = session.host_peer_id,
        freeze = session.freeze,
        manifest = session.manifest,
        transport = session.host_transport,
        initial_snapshot = snapshot,
        hash_interval_ticks = options.hash_interval_ticks,
        max_rollback_ticks = options.max_rollback_ticks,
    })
    for _, peer_id in ipairs(session.guest_peer_ids) do
        peer_ids[#peer_ids + 1] = peer_id
        drivers[#drivers + 1] = match_driver.new({
            role = "guest",
            peer_id = peer_id,
            freeze = session.freeze,
            manifest = session.manifest,
            transport = assert(session.guest_transports[peer_id]),
            initial_snapshot = snapshot,
            hash_interval_ticks = options.hash_interval_ticks,
            max_rollback_ticks = options.max_rollback_ticks,
        })
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
        -- Nothing is delivered for well over the 30-tick floor, so the burst
        -- carries rows the window can no longer accept.
        run_bursty(state, 60, 50)
        local terminal = 0
        for _, driver in ipairs(state.drivers) do
            if match_driver.status(driver) == "late_input" then
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

    -- FINDING 3 / retained-floor edge. The driver keeps no floor pre-check of
    -- its own; `rollback_input_history` owns the floor. These pin the exact
    -- boundary that ownership implies.
    t.it("accepts authority exactly at the retained floor and refuses one below", function()
        ---@param offset integer
        ---@return MatchDriverStatus, integer?, integer
        local function inject_at_floor_offset(offset)
            local state = harness("2v2", { max_rollback_ticks = 6 })
            local guest = state.drivers[2]
            -- Only the guest runs. The host is never advanced, so no canonical
            -- batch is ever produced and the injected packet is the only remote
            -- authority the guest ever sees. That keeps its confirmation pinned
            -- below the retained floor, which is the only regime where the floor
            -- rule can be observed at all.
            for _ = 1, 20 do
                match_driver.advance(guest, input_frame.neutral_sample())
            end
            local before = match_driver.diagnostics(guest)
            t.eq(before.status, "active")
            local floor = before.retained_floor_tick
            t.is_true(floor > 1, "the retained floor never advanced")
            t.is_true(before.confirmed_input_tick < floor, "confirmation outran the floor")
            local packet = assert(input_protocol.new_host({
                session_id = state.session.manifest.session_id,
                manifest_id = protocol.manifest_id(state.session.manifest),
                sender_id = state.session.host_peer_id,
                sequence = 777,
                transport_tick = 20,
                first_input_tick = 0,
                rows = {
                    {
                        tick = floor + offset,
                        slot_index = 1,
                        sample = input_frame.neutral_sample(),
                    },
                },
            }))
            local envelope = assert(transport_contract.new({
                type = "input",
                seq = packet.sequence,
                tick = packet.transport_tick,
                payload = assert(input_protocol.encode(packet)),
            }))
            assert(
                state.session.host_transport:send(
                    state.session.guest_peer_ids[1],
                    "input",
                    envelope
                )
            )
            state.session.host_transport:pump()
            match_driver.advance(guest, input_frame.neutral_sample())
            local terminal = match_driver.terminal(guest)
            return match_driver.status(guest), terminal and terminal.tick or nil, floor
        end

        local at_floor = inject_at_floor_offset(0)
        t.eq(at_floor, "active")

        local below, tick, floor = inject_at_floor_offset(-1)
        t.eq(below, "late_input")
        t.eq(tick, floor - 1)
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
