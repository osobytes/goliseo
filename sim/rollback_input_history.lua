-- Pure per-tick input history for rollback clients. It records transport-facing
-- authoritative arrivals and materializes the complete InputFrame consumed by
-- sim.match without knowing about sockets, wall clocks, or presentation.

local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")

---@alias RollbackInputSource "local"|"remote"
---@alias RollbackInputStatus "authoritative"|"predicted"
---@alias RollbackInputHistoryErrorCode "malformed"|"conflicting_authoritative"|"outside_window"|"pending_divergence"

---@class RollbackInputSlotRecord
---@field source RollbackInputSource
---@field status RollbackInputStatus
---@field sample InputSample

---@class RollbackInputTickRecord
---@field tick integer
---@field slots RollbackInputSlotRecord[]

---@class RollbackAuthoritativeArrival
---@field duplicate boolean -- True only when this exact authoritative sample already existed.
---@field confirmed_tick integer -- Highest contiguous all-authoritative tick, or -1 before tick zero.
---@field earliest_divergence integer? -- Earliest unconsumed corrected tick used by the simulation.

---@class RollbackAuthoritativeInput
---@field tick integer
---@field slot_index integer
---@field sample InputSample

---@class RollbackAuthoritativeBatchArrival
---@field inserted integer
---@field duplicates integer
---@field inserted_rows RollbackAuthoritativeInput[]
---@field confirmed_tick integer
---@field earliest_divergence integer?

---@class RollbackInputAnchor
---@field tick integer
---@field sample InputSample

---@class RollbackInputHistoryDiagnostics
---@field oldest_retained_tick integer
---@field newest_retained_tick integer?
---@field authoritative_tick_count integer
---@field authoritative_sample_count integer
---@field effective_tick_count integer
---@field record_tick_count integer
---@field predecessor_anchor_count integer
---@field confirmed_tick integer
---@field earliest_divergence integer?

---@class RollbackInputHistoryAccounting
---@field authoritative_bytes integer
---@field predecessor_anchor_bytes integer
---@field effective_frame_bytes integer
---@field input_record_bytes integer
---@field total_bytes integer

---@class RollbackInputTruncateResult
---@field boundary_tick integer
---@field effective_removed integer
---@field records_removed integer
---@field cleared_divergence boolean
---@field diagnostics RollbackInputHistoryDiagnostics

---@class RollbackInputHistory
---@field _sources RollbackInputSource[]
---@field _authoritative table<integer, table<integer, InputSample>>
---@field _authoritative_counts table<integer, integer>
---@field _authoritative_ticks integer[][] -- Sorted authoritative ticks for each slot.
---@field _anchors table<integer, RollbackInputAnchor>
---@field _effective table<integer, InputFrame>
---@field _records table<integer, RollbackInputTickRecord>
---@field _oldest_retained_tick integer
---@field _authoritative_tick_count integer
---@field _authoritative_sample_count integer
---@field _effective_tick_count integer
---@field _record_tick_count integer
---@field _confirmed_tick integer
---@field _earliest_divergence integer?

---@class RollbackInputHistoryModule
local rollback_input_history = {}

rollback_input_history.ROLLBACK_WINDOW_TICKS = 30
rollback_input_history.ROLLBACK_WINDOW_MILLISECONDS = rollback_input_history.ROLLBACK_WINDOW_TICKS
    * 1000
    / fixed_clock.TICK_RATE

---@param value any
---@return boolean
local function is_integer(value)
    return type(value) == "number"
        and value == value
        and value ~= math.huge
        and value ~= -math.huge
        and value == math.floor(value)
end

---@param history RollbackInputHistory
local function assert_history(history)
    assert(type(history) == "table", "rollback input history is required")
    assert(type(history._sources) == "table", "rollback input history sources are missing")
    assert(
        type(history._authoritative) == "table",
        "rollback authoritative input history is missing"
    )
    assert(type(history._effective) == "table", "rollback effective input history is missing")
    assert(
        type(history._authoritative_ticks) == "table",
        "rollback authoritative input indexes are missing"
    )
    assert(type(history._anchors) == "table", "rollback predecessor anchors are missing")
end

---@param sample InputSample
---@return InputSample
local function copy_sample(sample)
    return assert(input_frame.new_sample(sample))
end

---@param left InputSample
---@param right InputSample
---@return boolean
local function samples_equal(left, right)
    return left.move_x == right.move_x
        and left.move_y == right.move_y
        and left.held == right.held
        and left.edges == right.edges
        and left.aim == right.aim
end

---@param value any
---@return boolean
local function is_array(value)
    if type(value) ~= "table" then
        return false
    end
    local count = 0
    local greatest = 0
    for index in pairs(value) do
        if not is_integer(index) or index < 1 then
            return false
        end
        count = count + 1
        greatest = math.max(greatest, index)
    end
    return count == greatest
end

---@param sample InputSample
---@return string
local function sample_wire(sample)
    return table.concat({
        tostring(sample.move_x),
        tostring(sample.move_y),
        tostring(sample.held),
        tostring(sample.edges),
        tostring(sample.aim),
    }, ",")
end

---@param frame InputFrame
---@return InputFrame
local function copy_frame(frame)
    return assert(input_frame.copy(frame))
end

---@param record RollbackInputTickRecord
---@return RollbackInputTickRecord
local function copy_record(record)
    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        local slot = record.slots[index]
        slots[index] = {
            source = slot.source,
            status = slot.status,
            sample = copy_sample(slot.sample),
        }
    end
    return { tick = record.tick, slots = slots }
end

---@param sources RollbackInputSource[]
local function assert_sources(sources)
    assert(type(sources) == "table", "rollback input sources are required")
    for index in pairs(sources) do
        assert(
            is_integer(index) and index >= 1 and index <= input_frame.SLOT_COUNT,
            "rollback input sources must use canonical numeric indexes"
        )
    end
    for index = 1, input_frame.SLOT_COUNT do
        local source = sources[index]
        assert(
            source == "local" or source == "remote",
            "rollback input source " .. index .. " must be local or remote"
        )
    end
end

---@param tick any
---@param slot_index any
---@param sample any
---@return boolean?, string?, RollbackInputHistoryErrorCode?
local function validate_arrival(tick, slot_index, sample)
    if not is_integer(tick) or tick < 0 or tick > input_frame.MAX_TICK then
        return nil, "authoritative input tick must be a bounded non-negative integer", "malformed"
    end
    if not is_integer(slot_index) or slot_index < 1 or slot_index > input_frame.SLOT_COUNT then
        return nil, "authoritative input slot must be between one and eight", "malformed"
    end
    local ok, err = input_frame.validate_sample(sample)
    if not ok then
        return nil, err or "authoritative input sample is malformed", "malformed"
    end
    return true
end

---@param history RollbackInputHistory
local function advance_confirmation(history)
    local next_tick = history._confirmed_tick + 1
    while history._authoritative_counts[next_tick] == input_frame.SLOT_COUNT do
        history._confirmed_tick = next_tick
        next_tick = next_tick + 1
    end
end

---@param ticks integer[]
---@param target integer
---@return integer -- Array index of the greatest tick <= target, or zero.
local function predecessor_index(ticks, target)
    local low, high = 1, #ticks
    local found = 0
    while low <= high do
        local middle = math.floor((low + high) / 2)
        if ticks[middle] <= target then
            found = middle
            low = middle + 1
        else
            high = middle - 1
        end
    end
    return found
end

---@param ticks integer[]
---@param tick integer
local function insert_tick(ticks, tick)
    local low, high = 1, #ticks + 1
    while low < high do
        local middle = math.floor((low + high) / 2)
        if ticks[middle] and ticks[middle] < tick then
            low = middle + 1
        else
            high = middle
        end
    end
    table.insert(ticks, low, tick)
end

---@param history RollbackInputHistory
---@param tick integer
---@param slot_index integer
---@return InputSample?
local function latest_authoritative(history, tick, slot_index)
    local ticks = history._authoritative_ticks[slot_index]
    local index = predecessor_index(ticks, tick)
    if index > 0 then
        local slots = history._authoritative[ticks[index]]
        return assert(slots[slot_index], "rollback authoritative index is inconsistent")
    end
    local anchor = history._anchors[slot_index]
    if anchor and anchor.tick <= tick then
        return anchor.sample
    end
    return nil
end

---@param sources RollbackInputSource[] Canonical eight-slot local/remote ownership metadata.
---@return RollbackInputHistory
function rollback_input_history.new(sources)
    assert_sources(sources)
    local copied_sources = {}
    for index = 1, input_frame.SLOT_COUNT do
        copied_sources[index] = sources[index]
    end
    local authoritative_ticks = {}
    for index = 1, input_frame.SLOT_COUNT do
        authoritative_ticks[index] = {}
    end
    return {
        _sources = copied_sources,
        _authoritative = {},
        _authoritative_counts = {},
        _authoritative_ticks = authoritative_ticks,
        _anchors = {},
        _effective = {},
        _records = {},
        _oldest_retained_tick = 0,
        _authoritative_tick_count = 0,
        _authoritative_sample_count = 0,
        _effective_tick_count = 0,
        _record_tick_count = 0,
        _confirmed_tick = -1,
        _earliest_divergence = nil,
    }
end

---@param history RollbackInputHistory
---@param slot_index integer
---@return RollbackInputSource
function rollback_input_history.source(history, slot_index)
    assert_history(history)
    assert(
        is_integer(slot_index) and slot_index >= 1 and slot_index <= input_frame.SLOT_COUNT,
        "rollback input slot must be between one and eight"
    )
    return history._sources[slot_index]
end

-- Store one local or remote authoritative sample. Transport-shaped validation
-- failures, including conflicting duplicates, are recoverable and leave the
-- history unchanged.
---@param history RollbackInputHistory
---@param tick integer
---@param slot_index integer
---@param sample InputSample
---@return RollbackAuthoritativeArrival?, string?, RollbackInputHistoryErrorCode?
function rollback_input_history.add_authoritative(history, tick, slot_index, sample)
    assert_history(history)
    local ok, err, code = validate_arrival(tick, slot_index, sample)
    if not ok then
        return nil, err, code
    end
    if tick < history._oldest_retained_tick then
        return nil,
            ("authoritative input tick %d is older than retained tick %d"):format(
                tick,
                history._oldest_retained_tick
            ),
            "outside_window"
    end

    local slots = history._authoritative[tick]
    local existing = slots and slots[slot_index] or nil
    if existing ~= nil then
        if not samples_equal(existing, sample) then
            return nil,
                ("authoritative input conflicts at tick %d slot %d"):format(tick, slot_index),
                "conflicting_authoritative"
        end
        return {
            duplicate = true,
            confirmed_tick = history._confirmed_tick,
            earliest_divergence = history._earliest_divergence,
        }
    end

    local used_frame = history._effective[tick]
    if used_frame ~= nil and not samples_equal(used_frame.slots[slot_index], sample) then
        if history._earliest_divergence == nil or tick < history._earliest_divergence then
            history._earliest_divergence = tick
        end
    end

    if slots == nil then
        slots = {}
        history._authoritative[tick] = slots
        history._authoritative_counts[tick] = 0
        history._authoritative_tick_count = history._authoritative_tick_count + 1
    end
    slots[slot_index] = copy_sample(sample)
    insert_tick(history._authoritative_ticks[slot_index], tick)
    history._authoritative_counts[tick] = history._authoritative_counts[tick] + 1
    history._authoritative_sample_count = history._authoritative_sample_count + 1
    advance_confirmation(history)

    return {
        duplicate = false,
        confirmed_tick = history._confirmed_tick,
        earliest_divergence = history._earliest_divergence,
    }
end

-- Validate a complete transport-tick batch before retaining any row. This
-- prevents a late conflict or malformed row from leaving a partially applied
-- authority set whose result depends on peer polling order.
---@param history RollbackInputHistory
---@param arrivals RollbackAuthoritativeInput[]
---@return RollbackAuthoritativeBatchArrival?, string?, RollbackInputHistoryErrorCode?
function rollback_input_history.add_authoritative_batch(history, arrivals)
    assert_history(history)
    if not is_array(arrivals) or #arrivals == 0 then
        return nil, "authoritative input batch must be a non-empty canonical array", "malformed"
    end

    local planned = {}
    local planned_rows = {}
    local duplicates = 0
    local previous_tick = nil
    local previous_slot = nil
    for _, arrival in ipairs(arrivals) do
        if type(arrival) ~= "table" then
            return nil, "authoritative input batch row must be a table", "malformed"
        end
        local ok, err, code = validate_arrival(arrival.tick, arrival.slot_index, arrival.sample)
        if not ok then
            return nil, err, code
        end
        if
            previous_tick ~= nil
            and (
                arrival.tick < previous_tick
                or (arrival.tick == previous_tick and arrival.slot_index < assert(previous_slot))
            )
        then
            return nil, "authoritative input batch is not in canonical tick-slot order", "malformed"
        end
        previous_tick = arrival.tick
        previous_slot = arrival.slot_index
        if arrival.tick < history._oldest_retained_tick then
            return nil,
                ("authoritative input tick %d is older than retained tick %d"):format(
                    arrival.tick,
                    history._oldest_retained_tick
                ),
                "outside_window"
        end

        local key = tostring(arrival.tick) .. ":" .. tostring(arrival.slot_index)
        local prior = planned[key]
        if prior ~= nil then
            if not samples_equal(prior.sample, arrival.sample) then
                return nil,
                    ("authoritative input conflicts at tick %d slot %d"):format(
                        arrival.tick,
                        arrival.slot_index
                    ),
                    "conflicting_authoritative"
            end
            duplicates = duplicates + 1
        else
            local slots = history._authoritative[arrival.tick]
            local existing = slots and slots[arrival.slot_index] or nil
            if existing ~= nil then
                if not samples_equal(existing, arrival.sample) then
                    return nil,
                        ("authoritative input conflicts at tick %d slot %d"):format(
                            arrival.tick,
                            arrival.slot_index
                        ),
                        "conflicting_authoritative"
                end
                duplicates = duplicates + 1
            else
                local copied = {
                    tick = arrival.tick,
                    slot_index = arrival.slot_index,
                    sample = copy_sample(arrival.sample),
                }
                planned[key] = copied
                planned_rows[#planned_rows + 1] = copied
            end
        end
    end

    local inserted_rows = {}
    for index, arrival in ipairs(planned_rows) do
        local result = assert(
            rollback_input_history.add_authoritative(
                history,
                arrival.tick,
                arrival.slot_index,
                arrival.sample
            )
        )
        assert(not result.duplicate, "rollback batch preflight produced a duplicate insertion")
        inserted_rows[index] = {
            tick = arrival.tick,
            slot_index = arrival.slot_index,
            sample = copy_sample(arrival.sample),
        }
    end
    return {
        inserted = #inserted_rows,
        duplicates = duplicates,
        inserted_rows = inserted_rows,
        confirmed_tick = history._confirmed_tick,
        earliest_divergence = history._earliest_divergence,
    }
end

-- Materialize the exact frame that the simulation is about to use. Calling
-- this tick again during resimulation replaces its former used-input record
-- with a frame derived from the now-current authoritative history.
---@param history RollbackInputHistory
---@param tick integer
---@return InputFrame frame
---@return RollbackInputTickRecord record
function rollback_input_history.materialize(history, tick)
    assert_history(history)
    assert(
        is_integer(tick) and tick >= 0 and tick <= input_frame.MAX_TICK,
        "rollback input tick must be a bounded non-negative integer"
    )
    assert(
        tick >= history._oldest_retained_tick,
        "rollback input tick is older than retained history"
    )
    assert(
        history._earliest_divergence == nil or tick < history._earliest_divergence,
        "consume the earliest divergence before materializing the corrected timeline"
    )

    local authoritative = history._authoritative[tick]
    local samples = {}
    local records = {}
    for index = 1, input_frame.SLOT_COUNT do
        local sample = authoritative and authoritative[index] or nil
        assert(
            sample ~= nil or history._sources[index] == "remote",
            "local rollback input must be authoritative before materialization"
        )
        ---@type RollbackInputStatus
        local status = "authoritative"
        if sample == nil then
            status = "predicted"
            local prior = latest_authoritative(history, tick, index)
            if prior == nil then
                sample = input_frame.neutral_sample()
            else
                -- Aim repeats like `held` and unlike `edges`: it is a continuous
                -- channel, so "still aiming where they last aimed" is the same
                -- prediction "still holding what they last held" already makes.
                sample = {
                    move_x = prior.move_x,
                    move_y = prior.move_y,
                    held = prior.held,
                    edges = 0,
                    aim = prior.aim,
                }
            end
        end
        samples[index] = copy_sample(sample)
        records[index] = {
            source = history._sources[index],
            status = status,
            sample = copy_sample(sample),
        }
    end

    local frame = assert(input_frame.new(tick, samples))
    local record = { tick = tick, slots = records }
    if history._effective[tick] == nil then
        history._effective_tick_count = history._effective_tick_count + 1
    end
    if history._records[tick] == nil then
        history._record_tick_count = history._record_tick_count + 1
    end
    history._effective[tick] = copy_frame(frame)
    history._records[tick] = copy_record(record)
    return copy_frame(frame), copy_record(record)
end

---@param history RollbackInputHistory
---@param tick integer
---@return RollbackInputTickRecord?
function rollback_input_history.record(history, tick)
    assert_history(history)
    local record = history._records[tick]
    return record and copy_record(record) or nil
end

---@param history RollbackInputHistory
---@param tick integer
---@param slot_index integer
---@return RollbackInputSlotRecord?
function rollback_input_history.authoritative_record(history, tick, slot_index)
    assert_history(history)
    local slots = history._authoritative[tick]
    local sample = slots and slots[slot_index] or nil
    if sample == nil then
        return nil
    end
    return {
        source = rollback_input_history.source(history, slot_index),
        status = "authoritative",
        sample = copy_sample(sample),
    }
end

---@param history RollbackInputHistory
---@return integer
function rollback_input_history.confirmed_tick(history)
    assert_history(history)
    return history._confirmed_tick
end

---@param history RollbackInputHistory
---@return integer?
function rollback_input_history.earliest_divergence(history)
    assert_history(history)
    return history._earliest_divergence
end

-- Consume the current correction batch boundary. Later differing arrivals can
-- then establish a new earliest divergence after the caller has resimulated.
---@param history RollbackInputHistory
---@return integer?
function rollback_input_history.consume_earliest_divergence(history)
    assert_history(history)
    local tick = history._earliest_divergence
    history._earliest_divergence = nil
    return tick
end

---@param history RollbackInputHistory
---@return RollbackInputHistoryDiagnostics
function rollback_input_history.diagnostics(history)
    assert_history(history)
    local newest = nil
    for index = 1, input_frame.SLOT_COUNT do
        local ticks = history._authoritative_ticks[index]
        local tick = ticks[#ticks]
        if tick and (newest == nil or tick > newest) then
            newest = tick
        end
    end
    for tick in pairs(history._effective) do
        if newest == nil or tick > newest then
            newest = tick
        end
    end
    local anchor_count = 0
    for index = 1, input_frame.SLOT_COUNT do
        if history._anchors[index] ~= nil then
            anchor_count = anchor_count + 1
        end
    end
    return {
        oldest_retained_tick = history._oldest_retained_tick,
        newest_retained_tick = newest,
        authoritative_tick_count = history._authoritative_tick_count,
        authoritative_sample_count = history._authoritative_sample_count,
        effective_tick_count = history._effective_tick_count,
        record_tick_count = history._record_tick_count,
        predecessor_anchor_count = anchor_count,
        confirmed_tick = history._confirmed_tick,
        earliest_divergence = history._earliest_divergence,
    }
end

-- Exact byte count for the documented canonical retained-history encoding.
-- This accounts logical rollback payload only; it is deliberately not an
-- estimate of Lua allocator overhead.
---@param history RollbackInputHistory
---@return RollbackInputHistoryAccounting
function rollback_input_history.accounting(history)
    assert_history(history)
    local authoritative_bytes = 0
    for tick, slots in pairs(history._authoritative) do
        for slot, sample in pairs(slots) do
            authoritative_bytes = authoritative_bytes
                + #("A|%d|%d|%s\n"):format(tick, slot, sample_wire(sample))
        end
    end
    local predecessor_anchor_bytes = 0
    for slot, anchor in pairs(history._anchors) do
        predecessor_anchor_bytes = predecessor_anchor_bytes
            + #("P|%d|%d|%s\n"):format(slot, anchor.tick, sample_wire(anchor.sample))
    end
    local effective_frame_bytes = 0
    for _, frame in pairs(history._effective) do
        effective_frame_bytes = effective_frame_bytes
            + #("E|" .. assert(input_frame.encode(frame)) .. "\n")
    end
    local input_record_bytes = 0
    for tick, record in pairs(history._records) do
        local parts = { "R", tostring(tick) }
        for slot = 1, input_frame.SLOT_COUNT do
            local row = record.slots[slot]
            parts[#parts + 1] = row.source
            parts[#parts + 1] = row.status
            parts[#parts + 1] = sample_wire(row.sample)
        end
        input_record_bytes = input_record_bytes + #(table.concat(parts, "|") .. "\n")
    end
    return {
        authoritative_bytes = authoritative_bytes,
        predecessor_anchor_bytes = predecessor_anchor_bytes,
        effective_frame_bytes = effective_frame_bytes,
        input_record_bytes = input_record_bytes,
        total_bytes = authoritative_bytes
            + predecessor_anchor_bytes
            + effective_frame_bytes
            + input_record_bytes,
    }
end

-- Boundary N is the state before InputFrame N. Discarding from N removes only
-- effective frames and source/status diagnostics from an obsolete simulated
-- tail; authoritative arrivals and confirmation remain valid upstream facts.
---@param history RollbackInputHistory
---@param boundary_tick integer
---@return RollbackInputTruncateResult?, string?, RollbackInputHistoryErrorCode?
function rollback_input_history.truncate_from(history, boundary_tick)
    assert_history(history)
    if
        not is_integer(boundary_tick)
        or boundary_tick < 0
        or boundary_tick > input_frame.MAX_TICK
    then
        return nil,
            "rollback input truncate tick must be a bounded non-negative integer",
            "malformed"
    end
    if boundary_tick < history._oldest_retained_tick then
        return nil,
            ("rollback input boundary %d is older than retained tick %d"):format(
                boundary_tick,
                history._oldest_retained_tick
            ),
            "outside_window"
    end

    local effective_removed = 0
    for tick in pairs(history._effective) do
        if tick >= boundary_tick then
            history._effective[tick] = nil
            history._effective_tick_count = history._effective_tick_count - 1
            effective_removed = effective_removed + 1
        end
    end
    local records_removed = 0
    for tick in pairs(history._records) do
        if tick >= boundary_tick then
            history._records[tick] = nil
            history._record_tick_count = history._record_tick_count - 1
            records_removed = records_removed + 1
        end
    end
    local cleared_divergence = history._earliest_divergence ~= nil
        and history._earliest_divergence >= boundary_tick
    if cleared_divergence then
        history._earliest_divergence = nil
    end
    return {
        boundary_tick = boundary_tick,
        effective_removed = effective_removed,
        records_removed = records_removed,
        cleared_divergence = cleared_divergence,
        diagnostics = rollback_input_history.diagnostics(history),
    }
end

-- Drop input/effective records that no retained snapshot can restore. One
-- copied authoritative predecessor per slot survives as the prediction anchor.
-- A pending correction must be handed to the rollback session before its
-- evidence can leave the retained window.
---@param history RollbackInputHistory
---@param oldest_retained_tick integer
---@return RollbackInputHistoryDiagnostics?, string?, RollbackInputHistoryErrorCode?
function rollback_input_history.prune_before(history, oldest_retained_tick)
    assert_history(history)
    assert(
        is_integer(oldest_retained_tick)
            and oldest_retained_tick >= history._oldest_retained_tick
            and oldest_retained_tick <= input_frame.MAX_TICK,
        "rollback input prune tick must advance within the bounded tick range"
    )
    if
        history._earliest_divergence ~= nil
        and history._earliest_divergence < oldest_retained_tick
    then
        return nil,
            ("cannot prune unconsumed divergence at tick %d before retained tick %d"):format(
                history._earliest_divergence,
                oldest_retained_tick
            ),
            "pending_divergence"
    end
    if oldest_retained_tick == history._oldest_retained_tick then
        return rollback_input_history.diagnostics(history)
    end

    for index = 1, input_frame.SLOT_COUNT do
        local ticks = history._authoritative_ticks[index]
        local predecessor = predecessor_index(ticks, oldest_retained_tick - 1)
        if predecessor > 0 then
            local tick = ticks[predecessor]
            local slots = history._authoritative[tick]
            history._anchors[index] = {
                tick = tick,
                sample = copy_sample(assert(slots[index], "rollback prune index is inconsistent")),
            }
        end
        local retained_ticks = {}
        for tick_index = predecessor + 1, #ticks do
            retained_ticks[#retained_ticks + 1] = ticks[tick_index]
        end
        history._authoritative_ticks[index] = retained_ticks
    end

    for tick, slots in pairs(history._authoritative) do
        if tick < oldest_retained_tick then
            local sample_count = history._authoritative_counts[tick]
            assert(sample_count ~= nil, "rollback authoritative count is missing")
            history._authoritative[tick] = nil
            history._authoritative_counts[tick] = nil
            history._authoritative_tick_count = history._authoritative_tick_count - 1
            history._authoritative_sample_count = history._authoritative_sample_count - sample_count
            assert(next(slots) ~= nil, "rollback authoritative tick is empty")
        end
    end
    for tick in pairs(history._effective) do
        if tick < oldest_retained_tick then
            history._effective[tick] = nil
            history._effective_tick_count = history._effective_tick_count - 1
        end
    end
    for tick in pairs(history._records) do
        if tick < oldest_retained_tick then
            history._records[tick] = nil
            history._record_tick_count = history._record_tick_count - 1
        end
    end
    history._oldest_retained_tick = oldest_retained_tick
    return rollback_input_history.diagnostics(history)
end

return rollback_input_history
