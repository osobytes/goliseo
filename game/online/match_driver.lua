-- OMP-3 online match driver.
--
-- One instance drives one peer's match: it polls the star transport, authors the
-- rows it owns, sequences host authority, feeds real arrivals into the OMP-2
-- rollback session, advances the fixed 60 Hz clock, recomputes the live-slot
-- timeline, publishes boundary hashes, and ends with one typed terminal status.
--
-- It is the only place in the online stack that touches both a transport and a
-- rollback session. `sim/`, `data/`, and `core/` stay free of transport,
-- browser, and LOVE dependencies: everything WebRTC-shaped enters through the
-- injected `StarTransportAdapter`.
--
-- # Clocks
--
-- Three clocks are deliberately distinct:
--
--  * the **driver step** `T`, one per `advance` call, one per 60 Hz frame;
--  * the **transport tick** on the wire, `T + DELAY`, which exists only so the
--    host's own bundles can spend the versioned three-tick fairness delay in
--    the collector before the first simulated tick; and
--  * the **input tick** `first_input_tick + T`, the simulation authority tick
--    stepped during driver step `T`.
--
-- A sample taken during driver step `T` is authority for input tick
-- `first_input_tick + T + DELAY`. That is the documented input delay, and the
-- host pays it exactly like every guest: its own bundles are queued in the
-- collector and are not readable by `canonical_host_batch` until three transport
-- ticks later, so the host cannot bypass canonical sequencing.
--
-- # Authorship
--
-- Authorship is a frozen partition of the eight canonical slots, so no two peers
-- can ever author the same `(slot, input tick)`:
--
--  * a peer authors **every slot in its frozen owned set** — the human sample on
--    its control slot, deterministic AI rows on the rest;
--  * the host additionally authors every declared bot fill.
--
-- Validation is therefore set membership, not equality: a peer may author any
-- slot it owns and nothing outside it.
--
-- # The live slot
--
-- `live(N)` is the slot a human is controlling at input tick `N`. It is a pure
-- function of the frozen owned set, the canonical input stream, and the
-- deterministic simulation:
--
--     live(first)  = freeze.live
--     live(N + 1)  = coordinator.next_live_slot(owned, live(N), transition)
--
-- where the transition's `switch` edge is read from the *effective* canonical
-- row of `control_slot(N)` at tick `N` — the exact row `sim.match` consumed, and
-- the one the human's bits are actually on — and
-- `carrier`/`winner`/`ranked` come from the boundary `N + 1` simulation state
-- through `game.online.live_slot`. Nothing local reaches it. A rollback that
-- replaces ticks `[a, b]` replaces `live(a + 1 .. b + 1)` with it, so the
-- timeline is corrected by exactly the same mechanism as the simulation.
--
-- Because a sample is authority `DELAY` ticks after it is taken, the row that
-- carries a human's bits at input tick `N` is the one for `live(N - DELAY)`,
-- clamped to the opening live slot before the delay has elapsed. That
-- `control_slot` is itself a pure function of the same timeline, so every peer
-- computes it, not just the author. A correction can retroactively move
-- `live`, which can leave a human's bits on a slot the corrected timeline no
-- longer calls live for at most `DELAY` ticks. Every peer agrees on that
-- outcome, and the simulation is unaffected: it consumes the eight rows it is
-- given, whoever authored them.

local coordinator = require("game.online.coordinator")
local input_protocol = require("game.online.input_protocol")
local live_slot = require("game.online.live_slot")
local protocol = require("game.online.protocol")
local transport_contract = require("game.transport.contract")
local input_frame = require("sim.input_frame")
local match_snapshot = require("sim.match_snapshot")
local rollback_input_history = require("sim.rollback_input_history")
local rollback_session = require("sim.rollback_session")
local slot_input = require("sim.slot_input")

---@alias MatchDriverStatus
---| "active"
---| "completed"
---| "late_input"
---| "hash_mismatch"
---| "ownership_violation"
---| "authority_conflict"
---| "input_channel_failure"
---| "transport_lost"

---@class MatchDriverTerminal
---@field status MatchDriverStatus
---@field failure CoordinatorNetcodeFailure? -- What to report into the coordinator, if anything.
---@field detail string
---@field tick integer? -- Input tick the failure was attributed to.

---@class MatchDriverCheckpoint
---@field tick integer -- Confirmed boundary tick, in input-tick space.
---@field hash string
---@field live table<string, InputSlotId> -- Live slot per human at that boundary.

---@class MatchDriverOptions
---@field role SessionRole
---@field peer_id string
---@field freeze CoordinatorFreeze
---@field manifest SessionManifest
---@field transport StarTransportAdapter
---@field initial_snapshot MatchSnapshot -- Canonical slot-mode boundary zero.
---@field max_rollback_ticks integer?
---@field hash_interval_ticks integer?

---@class MatchDriverBatch
---@field step integer -- Driver step that produced this batch.
---@field input_tick integer -- Input tick simulated during this step.
---@field outputs RollbackTickOutput[]
---@field reconciliations integer -- Always at most one per driver step.
---@field applied_rows integer
---@field corrections integer
---@field rollbacks integer
---@field sent_packets integer
---@field checkpoints MatchDriverCheckpoint[]
---@field control TransportPeerMessage[] -- Control-channel traffic for the coordinator.
---@field live table<string, InputSlotId>
---@field status MatchDriverStatus

---@class MatchDriverDiagnostics
---@field role SessionRole
---@field peer_id string
---@field status MatchDriverStatus
---@field terminal MatchDriverTerminal?
---@field step integer
---@field transport_tick integer
---@field present_input_tick integer
---@field confirmed_input_tick integer
---@field owned InputSlotId[]
---@field authored InputSlotId[]
---@field live table<string, InputSlotId>
---@field control_slot InputSlotId?
---@field rollback_count integer
---@field correction_count integer
---@field predicted_slot_samples integer
---@field max_rollback_depth integer
---@field late_input_tick integer?
---@field hash_mismatches integer
---@field checkpoint_count integer
---@field dropped_outbound integer
---@field dropped_inbound integer

---@class MatchDriverPending
---@field due integer -- Driver step on which the collector may read this packet.
---@field packet InputPacket
---@field envelope TransportMessage
---@field producer_id string

---@class MatchDriver
---@field _role SessionRole
---@field _peer_id string
---@field _freeze CoordinatorFreeze
---@field _manifest SessionManifest
---@field _manifest_id string
---@field _transport StarTransportAdapter
---@field _session RollbackSession
---@field _sources RollbackInputSource[]
---@field _producer SlotInputProducerState
---@field _owned InputSlotId[]
---@field _authored integer[] -- Canonical slot indexes this peer authors, ascending.
---@field _authored_set table<integer, boolean>
---@field _owned_set table<integer, boolean>
---@field _peer_slots table<string, integer[]> -- Peer producer id -> owned slot indexes.
---@field _humans string[] -- Human producer ids, in frozen assignment order.
---@field _bot_slots integer[] -- Host only: declared bot fills, ascending.
---@field _first integer
---@field _step integer
---@field _live table<integer, table<string, InputSlotId>>
---@field _carrier table<integer, InputSlotId?>
---@field _live_tick integer -- Highest input tick with a computed live map.
---@field _history table<integer, table<integer, InputSample>> -- Slot index -> tick -> authored sample.
---@field _sequences table<string, integer>
---@field _pending MatchDriverPending[] -- Host only: the delayed local collector path.
---@field _deferred InputPacketArrival[] -- Host only: arrivals held back by the row bound.
---@field _checkpoints MatchDriverCheckpoint[]
---@field _checkpoint_by_tick table<integer, string>
---@field _hash_interval integer
---@field _next_checkpoint integer
---@field _hash_mismatches integer
---@field _status MatchDriverStatus
---@field _terminal MatchDriverTerminal?
---@field _late_input_tick integer?

---@class OnlineMatchDriverModule
local match_driver = {}

match_driver.DELAY_TICKS = input_protocol.FAIRNESS_DELAY_TICKS
match_driver.DEFAULT_HASH_INTERVAL_TICKS = 30
match_driver.MAX_HASH_MISMATCHES = 3
match_driver.POLL_BATCH_LIMIT = transport_contract.MAX_QUEUE_LIMIT

---@param value any
---@return boolean
local function is_integer(value)
    return type(value) == "number"
        and value == value
        and value ~= math.huge
        and value ~= -math.huge
        and value == math.floor(value)
end

---@param sample InputSample
---@return InputSample
local function copy_sample(sample)
    return assert(input_frame.new_sample(sample))
end

---@param seed integer
---@param slot_index integer
---@return integer
local function derived_ai_seed(seed, slot_index)
    -- A non-live owned slot needs a bot stream just like a declared fill does,
    -- and every peer must be able to name it, so it is derived from the frozen
    -- seed rather than negotiated. Park-Miller states are 1..2^31-2.
    local mixed = (seed % 2147483629) * 31 + slot_index * 7919
    return 1 + (mixed % 2147483645)
end

-- ---------------------------------------------------------------------------
-- Terminal status
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@param status MatchDriverStatus
---@param detail string
---@param failure CoordinatorNetcodeFailure?
---@param tick integer?
local function terminate(driver, status, detail, failure, tick)
    if driver._status ~= "active" then
        return
    end
    driver._status = status
    driver._terminal = {
        status = status,
        failure = failure,
        detail = detail,
        tick = tick,
    }
end

---@param driver MatchDriver
---@return boolean
local function running(driver)
    return driver._status == "active"
end

-- ---------------------------------------------------------------------------
-- Tick spaces
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@param input_tick integer
---@return integer
local function session_tick(driver, input_tick)
    return input_tick - driver._first
end

---@param driver MatchDriver
---@param tick integer
---@return integer
local function to_input_tick(driver, tick)
    return tick + driver._first
end

---@param driver MatchDriver
---@return integer -- Present boundary, in input-tick space.
local function present_input_tick(driver)
    return to_input_tick(driver, rollback_session.diagnostics(driver._session).present_boundary)
end

---@param driver MatchDriver
---@return integer
local function confirmed_input_tick(driver)
    local confirmed = rollback_session.diagnostics(driver._session).confirmed_tick
    if confirmed < 0 then
        return driver._first - 1
    end
    return to_input_tick(driver, confirmed)
end

-- ---------------------------------------------------------------------------
-- Live-slot timeline
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@param live table<string, InputSlotId>
---@return table<string, InputSlotId>
local function copy_live_map(driver, live)
    local copied = {}
    for _, producer_id in ipairs(driver._humans) do
        copied[producer_id] = live[producer_id]
    end
    return copied
end

---@param driver MatchDriver
---@param boundary integer -- Input-tick space.
---@return MatchState
local function boundary_state(driver, boundary)
    local lookup = rollback_session.snapshot(driver._session, session_tick(driver, boundary))
    assert(
        lookup.status == "present" or lookup.status == "retained",
        "the live-slot timeline needs a retained boundary snapshot"
    )
    return assert(lookup.snapshot, "retained boundary snapshot is missing").state
end

-- A real `MatchState` — Vec2 fields and all — for the boundary this peer is
-- about to simulate. The deterministic AI needs the live shape, not a
-- snapshot's plain vector tables.
---@param driver MatchDriver
---@return MatchState
local function present_state(driver)
    return (match_snapshot.restore(rollback_session.current_snapshot(driver._session)))
end

-- Extend the live timeline through the boundary that has just been simulated.
-- `record` is the effective input record `sim.match` consumed at `tick`, which
-- is where the canonical `switch` edge is read from.
---@param driver MatchDriver
---@param tick integer -- Input tick that was simulated.
---@param record RollbackInputTickRecord
local function extend_live(driver, tick, record)
    local previous = assert(driver._live[tick], "the live-slot timeline has a gap")
    local after = boundary_state(driver, tick + 1)
    local carrier = live_slot.carrier(after)
    local previous_carrier = driver._carrier[tick]
    ---@type table<string, InputSlotId>
    local next_live = {}
    for _, producer_id in ipairs(driver._humans) do
        local current = assert(previous[producer_id], "a human has no live slot")
        -- The `switch` edge is read from the row the human's bits are actually
        -- on. Because a sample is authority `DELAY` ticks after it is taken,
        -- that is the control slot, not necessarily the live slot: reading the
        -- live slot's row would read an AI row whenever the two differ, and
        -- switching would silently stall.
        local slot_index =
            live_slot.slot_index(match_driver.control_slot(driver, producer_id, tick))
        local sample = assert(record.slots[slot_index], "effective frame row is missing").sample
        local transition = live_slot.transition(after, {
            switch = live_slot.switch_edge(sample),
            live = current,
            previous_carrier = previous_carrier,
        })
        next_live[producer_id] =
            coordinator.next_live_slot(driver._freeze.owned[producer_id], current, transition)
    end
    driver._live[tick + 1] = next_live
    driver._carrier[tick + 1] = carrier
    if tick + 1 > driver._live_tick then
        driver._live_tick = tick + 1
    end
end

---@param driver MatchDriver
---@param floor integer -- Oldest input tick still retained.
local function prune_live(driver, floor)
    for tick = driver._first, floor - 1 do
        driver._live[tick] = nil
        driver._carrier[tick] = nil
    end
end

-- The slot that carries a human's authored bits at `input_tick`. The input
-- delay means it is the live slot from `DELAY` ticks earlier.
---@param driver MatchDriver
---@param producer_id string
---@param input_tick integer
---@return InputSlotId
function match_driver.control_slot(driver, producer_id, input_tick)
    local source = math.max(driver._first, input_tick - match_driver.DELAY_TICKS)
    local live = assert(driver._live[source], "the live-slot timeline has no entry for this tick")
    return assert(live[producer_id], "the producer owns no live slot")
end

---@param driver MatchDriver
---@param input_tick integer?
---@return table<string, InputSlotId>
function match_driver.live(driver, input_tick)
    local tick = input_tick or driver._live_tick
    local live = assert(driver._live[tick], "the live-slot timeline has no entry for this tick")
    return copy_live_map(driver, live)
end

-- ---------------------------------------------------------------------------
-- Authoring
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@param sender_id string
---@return integer
local function next_sequence(driver, sender_id)
    local sequence = driver._sequences[sender_id] or 0
    driver._sequences[sender_id] = sequence + 1
    return sequence
end

---@param driver MatchDriver
---@param slot_index integer
---@param tick integer
---@param sample InputSample
local function record_authored(driver, slot_index, tick, sample)
    local slot_history = driver._history[slot_index]
    slot_history[tick] = copy_sample(sample)
    local oldest = tick - input_protocol.HISTORY_ROWS - 1
    if oldest >= driver._first then
        slot_history[oldest] = nil
    end
end

-- The redundant bundle for one slot: the current row plus exactly the retained
-- prior rows, oldest first, as `input_protocol.validate` requires.
---@param driver MatchDriver
---@param slot_index integer
---@param tick integer
---@return InputAuthorityRow[]
local function redundant_rows(driver, slot_index, tick)
    local slot_history = driver._history[slot_index]
    local first = math.max(driver._first, tick - input_protocol.HISTORY_ROWS)
    local rows = {}
    for row_tick = first, tick do
        local sample = assert(slot_history[row_tick], "authored redundancy row is missing")
        rows[#rows + 1] = { tick = row_tick, slot_index = slot_index, sample = copy_sample(sample) }
    end
    return rows
end

-- Deterministic AI rows for every slot this peer authors that is not its
-- control slot. The bot stream advances once per authored tick per slot
-- regardless of which slot is live, so the stream is a function of the tick
-- count alone and never of the live-slot history.
---@param driver MatchDriver
---@param input_tick integer
---@param human_sample InputSample?
---@return table<integer, InputSample>
local function materialize_authored(driver, input_tick, human_sample)
    local geometry = present_state(driver)
    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        slots[index] = input_frame.neutral_sample()
    end
    local base = assert(input_frame.new(geometry.input_tick, slots))
    local frame = slot_input.materialize(driver._producer, geometry, base)
    local control = nil
    if #driver._owned > 0 then
        control =
            live_slot.slot_index(match_driver.control_slot(driver, driver._peer_id, input_tick))
    end
    ---@type table<integer, InputSample>
    local authored = {}
    for _, slot_index in ipairs(driver._authored) do
        if slot_index == control then
            authored[slot_index] = copy_sample(human_sample or input_frame.neutral_sample())
        else
            authored[slot_index] = copy_sample(frame.slots[slot_index])
        end
    end
    return authored
end

---@param driver MatchDriver
---@param slot_index integer
---@return string
local function producer_id_for(driver, slot_index)
    local producer = assert(driver._freeze.assignments[slot_index], "canonical slot is unassigned")
    return producer.producer_id
end

---@param driver MatchDriver
---@param slot_index integer
---@param tick integer
---@param transport_tick integer
---@return InputPacket, TransportMessage
local function build_packet(driver, slot_index, tick, transport_tick)
    local sender_id = producer_id_for(driver, slot_index)
    local sequence = next_sequence(driver, sender_id)
    local packet = assert(input_protocol.new_guest({
        session_id = driver._manifest.session_id,
        manifest_id = driver._manifest_id,
        sender_id = sender_id,
        sequence = sequence,
        transport_tick = transport_tick,
        first_input_tick = driver._first,
        rows = redundant_rows(driver, slot_index, tick),
    }))
    local wire = assert(input_protocol.encode(packet))
    local envelope = assert(transport_contract.new({
        type = "input",
        seq = packet.sequence,
        tick = packet.transport_tick,
        payload = wire,
    }))
    return packet, envelope
end

-- ---------------------------------------------------------------------------
-- Applying authority
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@param rows InputAuthorityRow[]
---@param batch MatchDriverBatch
---@param arrival boolean? -- True for authority that arrived over the transport.
local function apply_rows(driver, rows, batch, arrival)
    if #rows == 0 then
        return
    end
    local arrivals = {}
    local session = rollback_session.diagnostics(driver._session)
    local confirmed = confirmed_input_tick(driver)
    local floor = to_input_tick(driver, session.input_history.oldest_retained_tick)
    for _, row in ipairs(rows) do
        -- Authority at or below the confirmed boundary is already final. The
        -- redundant six-row history re-sends it every tick and replaying it must
        -- not re-enter the window as a late correction, so it is skipped.
        if row.tick > confirmed and row.tick < floor then
            -- Unconfirmed authority older than the retained floor can never be
            -- applied, and pretending otherwise would hide a divergence.
            driver._late_input_tick = driver._late_input_tick or row.tick
            terminate(
                driver,
                "late_input",
                "authority arrived older than the retained rollback floor",
                "late_input",
                driver._late_input_tick
            )
            return
        elseif row.tick > confirmed then
            arrivals[#arrivals + 1] = {
                tick = session_tick(driver, row.tick),
                slot_index = row.slot_index,
                sample = copy_sample(row.sample),
            }
        end
    end
    if #arrivals == 0 then
        return
    end
    local result, err, code = rollback_session.apply_authoritative_batch(driver._session, arrivals)
    if result == nil then
        if code == "outside_window" then
            driver._late_input_tick = driver._late_input_tick
                or to_input_tick(driver, arrivals[1].tick)
            terminate(
                driver,
                "late_input",
                err or "authority arrived outside the retained rollback window",
                "late_input",
                driver._late_input_tick
            )
            return
        end
        if code == "conflicting_authoritative" then
            terminate(
                driver,
                "authority_conflict",
                err or "canonical authority conflicted with retained history",
                "input_channel"
            )
            return
        end
        terminate(
            driver,
            "input_channel_failure",
            err or "canonical authority was rejected",
            "input_channel"
        )
        return
    end
    if arrival then
        batch.reconciliations = batch.reconciliations + 1
    end
    batch.applied_rows = batch.applied_rows + result.arrival.inserted
    batch.corrections = batch.corrections + result.arrival.corrections
    local reconciliation = result.reconciliation
    if reconciliation.status == "late_input_unrecoverable" then
        driver._late_input_tick = driver._late_input_tick
            or (reconciliation.causal_tick and to_input_tick(driver, reconciliation.causal_tick))
        terminate(
            driver,
            "late_input",
            "the rollback window overflowed while reconciling",
            "late_input",
            driver._late_input_tick
        )
        return
    end
    if not reconciliation.changed then
        return
    end
    batch.rollbacks = batch.rollbacks + 1
    for _, output in ipairs(reconciliation.corrected_outputs) do
        local tick = to_input_tick(driver, output.tick)
        extend_live(driver, tick, output.input)
        batch.outputs[#batch.outputs + 1] = output
    end
end

-- ---------------------------------------------------------------------------
-- Transport
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
local function drain_events(driver)
    while running(driver) do
        local event = driver._transport:poll_event()
        if event == nil then
            return
        end
        if event.kind == "star_error" or event.kind == "peer_error" then
            if event.code == "overflow" or event.code == "backpressure" then
                terminate(
                    driver,
                    "input_channel_failure",
                    event.message or "the input channel exceeded its queue budget",
                    "input_channel"
                )
            else
                terminate(
                    driver,
                    "input_channel_failure",
                    event.message or "the transport reported a terminal error",
                    "input_channel"
                )
            end
        elseif event.kind == "star_state" then
            if
                event.state == "closed"
                or event.state == "error"
                or event.state == "disconnected"
            then
                terminate(driver, "transport_lost", "the star transport is no longer connected")
            end
        elseif event.kind == "peer_state" then
            -- Every link is frozen at the countdown, so losing one after the
            -- freeze ends the match rather than shrinking the roster.
            if
                event.state == "closed"
                or event.state == "error"
                or event.state == "disconnected"
            then
                terminate(driver, "transport_lost", "a frozen peer link was lost")
            end
        end
    end
end

-- Every input envelope available on one transport tick, in the transport's own
-- deterministic drain order. Order is recorded, never used as authority: rows
-- are unioned and canonically sorted before a single reconciliation.
--
-- One transport carries both channels, and the reliable control channel belongs
-- to the session coordinator, not here. Draining it and discarding it would eat
-- the coordinator's traffic, so anything that is not an input envelope is handed
-- back on the batch for the owner to dispatch.
---@param driver MatchDriver
---@param batch MatchDriverBatch
---@return TransportPeerMessage[]
local function poll_input(driver, batch)
    local polled = driver._transport:poll_batch(match_driver.POLL_BATCH_LIMIT)
    local messages = {}
    for _, entry in ipairs(polled) do
        if entry.channel == "input" then
            messages[#messages + 1] = entry
        else
            batch.control[#batch.control + 1] = entry
        end
    end
    return messages
end

-- ---------------------------------------------------------------------------
-- Host collection
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@param messages TransportPeerMessage[]
---@param transport_tick integer
---@return InputPacketArrival[]?
local function collect_arrivals(driver, messages, transport_tick)
    ---@type InputPacketArrival[]
    local arrivals = {}
    for _, entry in ipairs(messages) do
        local packet, err, code = input_protocol.decode(entry.message.payload, {
            session_id = driver._manifest.session_id,
            manifest_id = driver._manifest_id,
            sender_id = entry.peer_id,
        })
        if packet == nil then
            terminate(
                driver,
                code == "ownership_mismatch" and "ownership_violation" or "input_channel_failure",
                err or "a guest input bundle failed to decode",
                "input_channel"
            )
            return nil
        end
        local slot_index = packet.rows[1].slot_index
        local owned = driver._peer_slots[entry.peer_id]
        local permitted = false
        for _, index in ipairs(owned or {}) do
            if index == slot_index then
                permitted = true
            end
        end
        if not permitted then
            terminate(
                driver,
                "ownership_violation",
                "a peer authored a slot outside its frozen owned set",
                "input_channel"
            )
            return nil
        end
        arrivals[#arrivals + 1] = {
            packet = packet,
            envelope = entry.message,
            arrival_tick = transport_tick,
            transport_peer_id = entry.peer_id,
        }
    end
    -- The host's own human and bot bundles enter through the same collector and
    -- only become readable once they have spent the versioned fairness delay.
    local remaining = {}
    for _, pending in ipairs(driver._pending) do
        if pending.due <= driver._step then
            arrivals[#arrivals + 1] = {
                packet = pending.packet,
                envelope = pending.envelope,
                arrival_tick = transport_tick,
                transport_peer_id = driver._peer_id,
            }
        else
            remaining[#remaining + 1] = pending
        end
    end
    driver._pending = remaining
    return arrivals
end

-- One host batch is bounded at `MAX_HOST_ROWS` distinct `(input tick, slot)`
-- rows, which is exactly eight slots times the seven-row redundancy window. A
-- delivery burst wider than that window cannot fit in one batch, so the excess
-- is carried to the next transport tick rather than dropped: dropping it would
-- strand authority the peers still need to confirm, and splitting it into two
-- batches on one tick would break the one-batch-one-reconciliation contract.
--
-- Selection is deterministic and independent of poll order: arrivals are sorted
-- by `(transport tick, sender, sequence)` first, then taken greedily.
---@param driver MatchDriver
---@return fun(left: InputPacketArrival, right: InputPacketArrival): boolean
local function arrival_order(driver)
    return function(left, right)
        -- The host's own collector path is never deferred. Its slots are the
        -- ones its own `materialize` demands be authoritative, so starving them
        -- would stall the host on its own input rather than on the network.
        local left_local = left.transport_peer_id == driver._peer_id
        local right_local = right.transport_peer_id == driver._peer_id
        if left_local ~= right_local then
            return left_local
        end
        if left.packet.transport_tick ~= right.packet.transport_tick then
            return left.packet.transport_tick < right.packet.transport_tick
        end
        if left.packet.sender_id ~= right.packet.sender_id then
            return left.packet.sender_id < right.packet.sender_id
        end
        return left.packet.sequence < right.packet.sequence
    end
end

---@param driver MatchDriver
---@param arrivals InputPacketArrival[]
---@return InputPacketArrival[] selected
local function select_within_bound(driver, arrivals)
    table.sort(arrivals, arrival_order(driver))
    ---@type InputPacketArrival[]
    local selected = {}
    ---@type InputPacketArrival[]
    local deferred = {}
    ---@type table<string, boolean>
    local keys = {}
    local count = 0
    for _, arrival in ipairs(arrivals) do
        local added = {}
        local additional = 0
        for _, row in ipairs(arrival.packet.rows) do
            local key = tostring(row.tick) .. ":" .. tostring(row.slot_index)
            if not keys[key] and not added[key] then
                added[key] = true
                additional = additional + 1
            end
        end
        if #deferred == 0 and count + additional <= input_protocol.MAX_HOST_ROWS then
            for _, row in ipairs(arrival.packet.rows) do
                keys[tostring(row.tick) .. ":" .. tostring(row.slot_index)] = true
            end
            count = count + additional
            selected[#selected + 1] = arrival
        else
            -- Once one arrival is held back, everything after it is held back
            -- too, so the carried-over stream keeps its canonical order.
            deferred[#deferred + 1] = arrival
        end
    end
    driver._deferred = deferred
    if #deferred > transport_contract.MAX_QUEUE_LIMIT then
        terminate(
            driver,
            "input_channel_failure",
            "the host authority backlog exceeded its bounded queue",
            "input_channel"
        )
    end
    return selected
end

---@param driver MatchDriver
---@param batch MatchDriverBatch
---@param transport_tick integer
---@param messages TransportPeerMessage[]
local function host_sequence_authority(driver, batch, transport_tick, messages)
    local arrivals = collect_arrivals(driver, messages, transport_tick)
    if arrivals == nil then
        return
    end
    -- Anything held back by the row bound last tick is re-stamped onto this
    -- transport tick and sequenced ahead of the new arrivals.
    local carried = driver._deferred
    driver._deferred = {}
    for _, arrival in ipairs(carried) do
        arrival.arrival_tick = transport_tick
        arrivals[#arrivals + 1] = arrival
    end
    -- Selection re-sorts, so appending here cannot change which arrivals win.
    arrivals = select_within_bound(driver, arrivals)
    if #arrivals == 0 then
        return
    end
    local packet, err, code = input_protocol.canonical_host_batch({
        manifest = driver._manifest,
        assignments = driver._freeze.assignments,
        host_peer_id = driver._peer_id,
        sequence = next_sequence(driver, driver._peer_id .. ".batch"),
        transport_tick = transport_tick,
        first_input_tick = driver._first,
    }, arrivals)
    if packet == nil then
        local status = "input_channel_failure"
        if code == "ownership_mismatch" then
            status = "ownership_violation"
        elseif code == "authority_conflict" or code == "packet_conflict" then
            status = "authority_conflict"
        end
        terminate(
            driver,
            status,
            err or "the host batch could not be canonicalized",
            "input_channel"
        )
        return
    end
    apply_rows(driver, packet.rows, batch, true)
    if not running(driver) then
        return
    end
    local wire = assert(input_protocol.encode(packet))
    local envelope = assert(transport_contract.new({
        type = "input",
        seq = packet.sequence,
        tick = packet.transport_tick,
        payload = wire,
    }))
    local delivered, broadcast_err, broadcast_code = driver._transport:broadcast("input", envelope)
    if delivered == nil then
        terminate(
            driver,
            broadcast_code == "backpressure" and "input_channel_failure" or "transport_lost",
            broadcast_err or "the host could not fan out canonical authority",
            "input_channel"
        )
        return
    end
    batch.sent_packets = batch.sent_packets + 1
end

---@param driver MatchDriver
---@param batch MatchDriverBatch
---@param messages TransportPeerMessage[]
local function guest_apply_authority(driver, batch, messages)
    ---@type InputAuthorityRow[]
    local rows = {}
    ---@type table<string, InputSample>
    local seen = {}
    for _, entry in ipairs(messages) do
        local packet, err, code = input_protocol.decode(entry.message.payload, {
            session_id = driver._manifest.session_id,
            manifest_id = driver._manifest_id,
            sender_id = entry.peer_id,
        })
        if packet == nil then
            terminate(
                driver,
                "input_channel_failure",
                err or "a host authority batch failed to decode",
                "input_channel"
            )
            return
        end
        if packet.kind ~= "host" then
            terminate(
                driver,
                "ownership_violation",
                "a guest received authority that was not a host batch",
                "input_channel"
            )
            return
        end
        for _, row in ipairs(packet.rows) do
            local key = tostring(row.tick) .. ":" .. tostring(row.slot_index)
            local prior = seen[key]
            if prior == nil then
                seen[key] = row.sample
                rows[#rows + 1] = row
            elseif
                prior.move_x ~= row.sample.move_x
                or prior.move_y ~= row.sample.move_y
                or prior.held ~= row.sample.held
                or prior.edges ~= row.sample.edges
            then
                terminate(
                    driver,
                    "authority_conflict",
                    "two host batches on one transport tick disagreed on authority",
                    "input_channel"
                )
                return
            end
        end
    end
    -- Callback order cannot reach the simulation: the union is sorted into the
    -- canonical (tick, slot) order before one atomic application.
    table.sort(rows, function(left, right)
        if left.tick ~= right.tick then
            return left.tick < right.tick
        end
        return left.slot_index < right.slot_index
    end)
    apply_rows(driver, rows, batch, true)
end

-- ---------------------------------------------------------------------------
-- Checkpoints
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@param batch MatchDriverBatch
local function publish_checkpoints(driver, batch)
    local confirmed = confirmed_input_tick(driver)
    while running(driver) and driver._next_checkpoint <= confirmed + 1 do
        local boundary = driver._next_checkpoint
        local lookup = rollback_session.snapshot(driver._session, session_tick(driver, boundary))
        if lookup.status ~= "present" and lookup.status ~= "retained" then
            terminate(
                driver,
                "late_input",
                "a confirmed hash checkpoint fell out of the retained window",
                "late_input",
                boundary
            )
            return
        end
        local hash = match_snapshot.hash(assert(lookup.snapshot))
        -- The live slot is captured with the hash rather than looked up later:
        -- the timeline is pruned with the rollback window, and a checkpoint is
        -- exactly the boundary at which peers must agree on both.
        local live = copy_live_map(driver, assert(driver._live[boundary]))
        driver._checkpoints[#driver._checkpoints + 1] =
            { tick = boundary, hash = hash, live = live }
        driver._checkpoint_by_tick[boundary] = hash
        batch.checkpoints[#batch.checkpoints + 1] =
            { tick = boundary, hash = hash, live = copy_live_map(driver, live) }
        driver._next_checkpoint = boundary + driver._hash_interval
    end
end

-- Compare a peer's published checkpoint. A single disagreement is tolerated and
-- cleared by the next agreement, matching the coordinator's hash policy;
-- `MAX_HASH_MISMATCHES` consecutive disagreements end the match.
---@param driver MatchDriver
---@param tick integer
---@param hash string
---@return boolean matched -- False when the boundary was hashed and disagreed.
function match_driver.observe_checkpoint(driver, tick, hash)
    local mine = driver._checkpoint_by_tick[tick]
    if mine == nil then
        return true
    end
    if mine == hash then
        driver._hash_mismatches = 0
        return true
    end
    driver._hash_mismatches = driver._hash_mismatches + 1
    if driver._hash_mismatches >= match_driver.MAX_HASH_MISMATCHES then
        terminate(
            driver,
            "hash_mismatch",
            "boundary hashes disagreed at "
                .. tostring(match_driver.MAX_HASH_MISMATCHES)
                .. " consecutive checkpoints",
            "desync",
            tick
        )
    end
    return false
end

---@param driver MatchDriver
---@return MatchDriverCheckpoint[]
function match_driver.checkpoints(driver)
    local copied = {}
    for index, checkpoint in ipairs(driver._checkpoints) do
        copied[index] = {
            tick = checkpoint.tick,
            hash = checkpoint.hash,
            live = copy_live_map(driver, checkpoint.live),
        }
    end
    return copied
end

-- ---------------------------------------------------------------------------
-- Construction
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
local function prime(driver)
    -- Nobody could sample before the start boundary, so the first `DELAY` input
    -- ticks carry neutral human rows. AI rows are materialized from boundary
    -- zero, which every peer shares.
    for tick = driver._first, driver._first + match_driver.DELAY_TICKS - 1 do
        local authored = materialize_authored(driver, tick, nil)
        for _, slot_index in ipairs(driver._authored) do
            record_authored(driver, slot_index, tick, authored[slot_index])
        end
    end
    if driver._role ~= "host" then
        -- A guest's step-zero bundle already carries these rows as redundancy,
        -- and a guest applies its own rows without a collector.
        return
    end
    -- The host's collector is where its fairness delay is spent, so its
    -- pre-start rows must be placed in it on transport tick zero to be readable
    -- on the first step. They cannot bypass anything: nobody had sampled yet.
    local last = driver._first + match_driver.DELAY_TICKS - 1
    for _, slot_index in ipairs(driver._authored) do
        local packet, envelope = build_packet(driver, slot_index, last, 0)
        driver._pending[#driver._pending + 1] = {
            due = 0,
            packet = packet,
            envelope = envelope,
            producer_id = packet.sender_id,
        }
    end
end

---@param options MatchDriverOptions
---@return MatchDriver
function match_driver.new(options)
    assert(type(options) == "table", "match driver options are required")
    local freeze = assert(options.freeze, "match driver requires a coordinator freeze")
    local manifest = assert(options.manifest, "match driver requires the frozen manifest")
    local transport = assert(options.transport, "match driver requires a star transport")
    local role = options.role
    assert(role == "host" or role == "guest", "match driver role must be host or guest")
    local peer_id = options.peer_id
    assert(type(peer_id) == "string" and peer_id ~= "", "match driver requires a peer id")
    local manifest_id = protocol.manifest_id(manifest)
    assert(
        manifest_id == freeze.manifest_id,
        "match driver manifest does not match the frozen session"
    )
    local hash_interval = options.hash_interval_ticks or match_driver.DEFAULT_HASH_INTERVAL_TICKS
    assert(
        is_integer(hash_interval) and hash_interval >= 1,
        "match driver hash interval must be a positive integer"
    )
    local maximum = options.max_rollback_ticks or rollback_input_history.ROLLBACK_WINDOW_TICKS
    assert(
        is_integer(maximum)
            and maximum >= 1
            and maximum <= rollback_input_history.ROLLBACK_WINDOW_TICKS,
        "match driver rollback window must be a bounded positive integer"
    )

    local owned = protocol.owned_slots(freeze.assignments, peer_id)
    ---@type table<integer, boolean>
    local owned_set = {}
    for _, slot in ipairs(owned) do
        owned_set[live_slot.slot_index(slot)] = true
    end

    ---@type table<string, integer[]>
    local peer_slots = {}
    ---@type string[]
    local humans = {}
    ---@type integer[]
    local bot_slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        local producer = assert(freeze.assignments[index], "the freeze has an unassigned slot")
        if producer.producer_kind == "peer" then
            if peer_slots[producer.producer_id] == nil then
                peer_slots[producer.producer_id] = {}
                humans[#humans + 1] = producer.producer_id
            end
            local slots = peer_slots[producer.producer_id]
            slots[#slots + 1] = index
        else
            bot_slots[#bot_slots + 1] = index
        end
    end

    ---@type integer[]
    local authored = {}
    ---@type table<integer, boolean>
    local authored_set = {}
    ---@type MatchSlotSource[]
    local sources = {}
    ---@type RollbackInputSource[]
    local rollback_sources = {}
    for index = 1, input_frame.SLOT_COUNT do
        local producer = freeze.assignments[index]
        local mine = owned_set[index] or (role == "host" and producer.producer_kind == "bot")
        if mine then
            authored[#authored + 1] = index
            authored_set[index] = true
            local seed = producer.bot_seed or derived_ai_seed(freeze.seed, index)
            sources[index] = { kind = "bot", seed = seed }
            rollback_sources[index] = "local"
        else
            sources[index] = { kind = "neutral" }
            rollback_sources[index] = "remote"
        end
    end

    local session = rollback_session.new(options.initial_snapshot, rollback_sources, maximum)
    ---@type table<integer, table<integer, InputSample>>
    local history = {}
    for index = 1, input_frame.SLOT_COUNT do
        history[index] = {}
    end

    ---@type MatchDriver
    local driver = {
        _role = role,
        _peer_id = peer_id,
        _freeze = freeze,
        _manifest = manifest,
        _manifest_id = manifest_id,
        _transport = transport,
        _session = session,
        _sources = rollback_sources,
        _producer = slot_input.new_producer(sources),
        _owned = owned,
        _authored = authored,
        _authored_set = authored_set,
        _owned_set = owned_set,
        _peer_slots = peer_slots,
        _humans = humans,
        _bot_slots = bot_slots,
        _first = freeze.first_input_tick,
        _step = 0,
        _live = {},
        _carrier = {},
        _live_tick = freeze.first_input_tick,
        _history = history,
        _sequences = {},
        _pending = {},
        _deferred = {},
        _checkpoints = {},
        _checkpoint_by_tick = {},
        _hash_interval = hash_interval,
        _next_checkpoint = freeze.first_input_tick,
        _hash_mismatches = 0,
        _status = "active",
        _terminal = nil,
        _late_input_tick = nil,
    }
    ---@type table<string, InputSlotId>
    local opening = {}
    for _, producer_id in ipairs(humans) do
        opening[producer_id] = assert(freeze.live[producer_id], "a human has no opening live slot")
    end
    driver._live[driver._first] = opening
    driver._carrier[driver._first] = live_slot.carrier(boundary_state(driver, driver._first))
    prime(driver)
    return driver
end

-- ---------------------------------------------------------------------------
-- The fixed-tick step
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@param batch MatchDriverBatch
---@param input_tick integer
---@param transport_tick integer
---@param sample InputSample?
local function author_and_send(driver, batch, input_tick, transport_tick, sample)
    local authored = materialize_authored(driver, input_tick, sample)
    for _, slot_index in ipairs(driver._authored) do
        record_authored(driver, slot_index, input_tick, authored[slot_index])
    end
    ---@type InputAuthorityRow[]
    local local_rows = {}
    for _, slot_index in ipairs(driver._authored) do
        local packet, envelope = build_packet(driver, slot_index, input_tick, transport_tick)
        if driver._role == "host" then
            driver._pending[#driver._pending + 1] = {
                due = driver._step + match_driver.DELAY_TICKS,
                packet = packet,
                envelope = envelope,
                producer_id = packet.sender_id,
            }
        else
            for _, row in ipairs(packet.rows) do
                local_rows[#local_rows + 1] = row
            end
            local ok, err, code =
                driver._transport:send(transport_contract.HOST_PEER_ID, "input", envelope)
            if ok == nil then
                terminate(
                    driver,
                    code == "backpressure" and "input_channel_failure" or "transport_lost",
                    err or "a guest could not publish its input bundle",
                    "input_channel"
                )
                return
            end
            batch.sent_packets = batch.sent_packets + 1
        end
    end
    -- A guest's own rows are local authority immediately, applied as one batch
    -- so its own authoring never costs a second reconciliation. The host's
    -- canonical echo is byte-identical and therefore idempotent.
    table.sort(local_rows, function(left, right)
        if left.tick ~= right.tick then
            return left.tick < right.tick
        end
        return left.slot_index < right.slot_index
    end)
    apply_rows(driver, local_rows, batch, false)
end

---@param driver MatchDriver
---@param batch MatchDriverBatch
---@param input_tick integer
local function step_to(driver, batch, input_tick)
    while running(driver) do
        local diagnostics = rollback_session.diagnostics(driver._session)
        if to_input_tick(driver, diagnostics.present_boundary) > input_tick then
            return
        end
        if diagnostics.status == "finished" then
            terminate(driver, "completed", "the match reached full time")
            return
        end
        local output, err, code = rollback_session.step(driver._session)
        if output == nil then
            if code == "match_finished" then
                terminate(driver, "completed", "the match reached full time")
                return
            end
            terminate(
                driver,
                "late_input",
                err or "the rollback session cannot progress",
                "late_input",
                driver._late_input_tick
            )
            return
        end
        local tick = to_input_tick(driver, output.tick)
        extend_live(driver, tick, output.input)
        batch.outputs[#batch.outputs + 1] = output
        local floor =
            rollback_session.diagnostics(driver._session).input_history.oldest_retained_tick
        prune_live(driver, to_input_tick(driver, floor))
        if output.finished then
            terminate(driver, "completed", "the match reached full time")
            return
        end
    end
end

-- One fixed 60 Hz driver step: poll, apply one canonical arrival batch through
-- one reconciliation, author this step's rows, publish them, simulate one input
-- tick, and hash any confirmed checkpoint that came due.
---@param driver MatchDriver
---@param sample InputSample? -- The local human's sample; nil for a peer that owns nothing.
---@return MatchDriverBatch
function match_driver.advance(driver, sample)
    local input_tick = driver._first + driver._step
    local transport_tick = driver._step + match_driver.DELAY_TICKS
    ---@type MatchDriverBatch
    local batch = {
        step = driver._step,
        input_tick = input_tick,
        outputs = {},
        reconciliations = 0,
        applied_rows = 0,
        corrections = 0,
        rollbacks = 0,
        sent_packets = 0,
        checkpoints = {},
        control = {},
        live = {},
        status = driver._status,
    }
    if not running(driver) then
        -- No hidden progress after a terminal status: nothing is polled, sent,
        -- applied, or simulated.
        batch.live = copy_live_map(driver, driver._live[driver._live_tick] or {})
        return batch
    end

    drain_events(driver)
    local messages = running(driver) and poll_input(driver, batch) or {}
    if running(driver) then
        if driver._role == "host" then
            host_sequence_authority(driver, batch, transport_tick, messages)
        else
            guest_apply_authority(driver, batch, messages)
        end
    end
    if running(driver) then
        author_and_send(
            driver,
            batch,
            input_tick + match_driver.DELAY_TICKS,
            transport_tick,
            sample
        )
    end
    if running(driver) then
        step_to(driver, batch, input_tick)
    end
    publish_checkpoints(driver, batch)

    driver._step = driver._step + 1
    batch.status = driver._status
    batch.live = copy_live_map(driver, driver._live[driver._live_tick] or {})
    return batch
end

-- ---------------------------------------------------------------------------
-- Pure diagnostics
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@return MatchDriverStatus
function match_driver.status(driver)
    return driver._status
end

---@param driver MatchDriver
---@return MatchDriverTerminal?
function match_driver.terminal(driver)
    local terminal = driver._terminal
    if terminal == nil then
        return nil
    end
    return {
        status = terminal.status,
        failure = terminal.failure,
        detail = terminal.detail,
        tick = terminal.tick,
    }
end

---@param driver MatchDriver
---@return MatchSnapshot
function match_driver.current_snapshot(driver)
    return rollback_session.current_snapshot(driver._session)
end

---@param driver MatchDriver
---@param boundary integer -- Input-tick space.
---@return RollbackSnapshotLookup
function match_driver.snapshot(driver, boundary)
    return rollback_session.snapshot(driver._session, session_tick(driver, boundary))
end

---@param driver MatchDriver
---@return table<InputSlotId, CoordinatorSlotDriver>
function match_driver.slot_drivers(driver)
    return coordinator.slot_drivers(driver._freeze, driver._live[driver._live_tick])
end

---@param driver MatchDriver
---@return MatchDriverDiagnostics
function match_driver.diagnostics(driver)
    local session = rollback_session.diagnostics(driver._session)
    local transport = driver._transport:diagnostics()
    ---@type InputSlotId[]
    local authored = {}
    for index, slot_index in ipairs(driver._authored) do
        authored[index] = live_slot.slot_id(slot_index)
    end
    ---@type InputSlotId[]
    local owned = {}
    for index, slot in ipairs(driver._owned) do
        owned[index] = slot
    end
    local control = nil
    if #driver._owned > 0 then
        control = match_driver.control_slot(driver, driver._peer_id, driver._live_tick)
    end
    return {
        role = driver._role,
        peer_id = driver._peer_id,
        status = driver._status,
        terminal = match_driver.terminal(driver),
        step = driver._step,
        transport_tick = driver._step + match_driver.DELAY_TICKS,
        present_input_tick = present_input_tick(driver),
        confirmed_input_tick = confirmed_input_tick(driver),
        owned = owned,
        authored = authored,
        live = copy_live_map(driver, driver._live[driver._live_tick] or {}),
        control_slot = control,
        rollback_count = session.rollback_count,
        correction_count = session.correction_count,
        predicted_slot_samples = session.predicted_slot_samples,
        max_rollback_depth = session.max_rollback_depth,
        late_input_tick = driver._late_input_tick,
        hash_mismatches = driver._hash_mismatches,
        checkpoint_count = #driver._checkpoints,
        dropped_outbound = transport.dropped_outbound,
        dropped_inbound = transport.dropped_inbound,
    }
end

return match_driver
