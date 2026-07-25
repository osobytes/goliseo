-- Bounded start-of-tick snapshot storage for rollback sessions. Boundaries are
-- indexed by the InputFrame tick they will consume; the ring owns independent
-- canonical snapshots and has no simulation, transport, or presentation role.

local fnv1a64 = require("core.fnv1a64")
local input_frame = require("sim.input_frame")
local match_snapshot = require("sim.match_snapshot")
local rollback_input_history = require("sim.rollback_input_history")

---@alias RollbackSnapshotLookupStatus "present"|"retained"|"missing"|"outside_window"
---@alias RollbackSnapshotHistoryErrorCode "malformed"|"outside_window"|"missing"

---@class RollbackSnapshotEntry
---@field tick integer
---@field snapshot MatchSnapshot
---@field canonical_bytes integer
---@field canonical_wire string?
---@field hash string?

---@class RollbackSnapshotHistory
---@field _max_rollback_ticks integer
---@field _capacity integer
---@field _entries table<integer, RollbackSnapshotEntry>
---@field _latest_tick integer?
---@field _oldest_supported_tick integer?
---@field _count integer
---@field _canonical_bytes integer
---@field _peak_count integer
---@field _peak_canonical_bytes integer

---@class RollbackSnapshotLookup
---@field status RollbackSnapshotLookupStatus
---@field tick integer
---@field snapshot MatchSnapshot?
---@field canonical_bytes integer?

---@class RollbackSnapshotHistoryComparison
---@field matched boolean
---@field expected_status RollbackSnapshotLookupStatus
---@field actual_status RollbackSnapshotLookupStatus
---@field expected_hash string?
---@field actual_hash string?
---@field first_difference MatchSnapshotDifference?

---@class RollbackSnapshotStoreResult
---@field tick integer
---@field replaced boolean
---@field evicted integer

---@class RollbackSnapshotTruncateResult
---@field boundary_tick integer
---@field removed integer
---@field diagnostics RollbackSnapshotHistoryDiagnostics

---@class RollbackSnapshotHistoryDiagnostics
---@field max_rollback_ticks integer
---@field capacity integer
---@field retained_boundary_count integer
---@field canonical_bytes integer
---@field peak_retained_boundary_count integer
---@field peak_canonical_bytes integer
---@field oldest_supported_tick integer?
---@field oldest_boundary_tick integer?
---@field latest_tick integer?

---@class RollbackSnapshotHistoryModule
local rollback_snapshot_history = {}

rollback_snapshot_history.DEFAULT_MAX_ROLLBACK_TICKS = rollback_input_history.ROLLBACK_WINDOW_TICKS

---@param value any
---@return boolean
local function is_integer(value)
    return type(value) == "number"
        and value == value
        and value ~= math.huge
        and value ~= -math.huge
        and value == math.floor(value)
end

---@param history RollbackSnapshotHistory
local function assert_history(history)
    assert(type(history) == "table", "rollback snapshot history is required")
    assert(type(history._entries) == "table", "rollback snapshot ring is missing")
    assert(
        is_integer(history._capacity) and history._capacity >= 1,
        "rollback snapshot capacity is invalid"
    )
end

---@param snapshot MatchSnapshot
---@return MatchSnapshot
local function copy_snapshot(snapshot)
    local state, combat_state = match_snapshot.restore(snapshot)
    return match_snapshot.capture(state, combat_state)
end

---@param snapshot MatchSnapshot
---@return MatchSnapshot
local function copy_owned_snapshot(snapshot)
    return match_snapshot.capture_owned(snapshot.state, snapshot.combat)
end

---@param history RollbackSnapshotHistory
---@param tick integer
---@return integer
local function ring_index(history, tick)
    return (tick % history._capacity) + 1
end

---@param history RollbackSnapshotHistory
---@return integer?
local function oldest_supported_tick(history)
    return history._oldest_supported_tick
end

---@param history RollbackSnapshotHistory
---@param entry RollbackSnapshotEntry
local function remove_entry(history, entry)
    history._entries[ring_index(history, entry.tick)] = nil
    history._count = history._count - 1
    history._canonical_bytes = history._canonical_bytes - entry.canonical_bytes
end

---@param max_rollback_ticks integer? Maximum prior input ticks that remain restorable.
---@return RollbackSnapshotHistory
function rollback_snapshot_history.new(max_rollback_ticks)
    max_rollback_ticks = max_rollback_ticks or rollback_snapshot_history.DEFAULT_MAX_ROLLBACK_TICKS
    assert(
        is_integer(max_rollback_ticks)
            and max_rollback_ticks >= 0
            and max_rollback_ticks <= input_frame.MAX_TICK,
        "maximum rollback ticks must be a bounded non-negative integer"
    )
    return {
        _max_rollback_ticks = max_rollback_ticks,
        _capacity = max_rollback_ticks + 1,
        _entries = {},
        _latest_tick = nil,
        _oldest_supported_tick = nil,
        _count = 0,
        _canonical_bytes = 0,
        _peak_count = 0,
        _peak_canonical_bytes = 0,
    }
end

-- Store or replace one canonical start-of-tick boundary. Advancing the latest
-- boundary evicts every entry older than the supported correction window.
---@param history RollbackSnapshotHistory
---@param retained MatchSnapshot
---@return RollbackSnapshotStoreResult?, string?, RollbackSnapshotHistoryErrorCode?
local function store_retained(history, retained)
    assert_history(history)
    local tick = retained.state.input_tick
    assert(
        is_integer(tick) and tick >= 0 and tick <= input_frame.MAX_TICK,
        "snapshot input tick must be a bounded non-negative integer"
    )

    local supported = oldest_supported_tick(history)
    if supported ~= nil and tick < supported then
        return nil,
            ("snapshot boundary %d is older than retained boundary %d"):format(tick, supported),
            "outside_window"
    end

    if history._latest_tick == nil or tick > history._latest_tick then
        history._latest_tick = tick
        local next_supported = math.max(0, tick - history._max_rollback_ticks)
        if
            history._oldest_supported_tick == nil
            or next_supported > history._oldest_supported_tick
        then
            history._oldest_supported_tick = next_supported
        end
    end
    supported = assert(oldest_supported_tick(history))
    local evicted = 0
    for _, entry in pairs(history._entries) do
        if entry and entry.tick < supported then
            remove_entry(history, entry)
            evicted = evicted + 1
        end
    end

    local index = ring_index(history, tick)
    local existing = history._entries[index]
    local replaced = existing ~= nil and existing.tick == tick
    if existing then
        remove_entry(history, existing)
    end
    local canonical_bytes = match_snapshot.encoded_size_canonical(retained)
    history._entries[index] = {
        tick = tick,
        snapshot = retained,
        canonical_bytes = canonical_bytes,
        canonical_wire = nil,
        hash = nil,
    }
    history._count = history._count + 1
    history._canonical_bytes = history._canonical_bytes + canonical_bytes
    history._peak_count = math.max(history._peak_count, history._count)
    history._peak_canonical_bytes =
        math.max(history._peak_canonical_bytes, history._canonical_bytes)
    return { tick = tick, replaced = replaced, evicted = evicted }
end

---@param history RollbackSnapshotHistory
---@param snapshot MatchSnapshot
---@return RollbackSnapshotStoreResult?, string?, RollbackSnapshotHistoryErrorCode?
function rollback_snapshot_history.store(history, snapshot)
    return store_retained(history, copy_snapshot(snapshot))
end

-- Transfer a freshly captured canonical snapshot into the ring. The caller
-- must not retain and mutate the supplied table after this call. This avoids a
-- redundant restore/capture copy on the simulation hot path while `store`
-- remains the ownership-safe public boundary for arbitrary callers.
---@param history RollbackSnapshotHistory
---@param snapshot MatchSnapshot
---@return RollbackSnapshotStoreResult?, string?, RollbackSnapshotHistoryErrorCode?
function rollback_snapshot_history.store_owned(history, snapshot)
    return store_retained(history, snapshot)
end

---@param history RollbackSnapshotHistory
---@param tick integer
---@return RollbackSnapshotLookupStatus
local function lookup_status(history, tick)
    local supported = oldest_supported_tick(history)
    if supported ~= nil and tick < supported then
        return "outside_window"
    end
    local entry = history._entries[ring_index(history, tick)]
    if entry == nil or entry.tick ~= tick then
        return "missing"
    end
    return tick == history._latest_tick and "present" or "retained"
end

---@param history RollbackSnapshotHistory
---@param tick integer
---@return RollbackSnapshotLookupStatus
function rollback_snapshot_history.status(history, tick)
    assert_history(history)
    assert(
        is_integer(tick) and tick >= 0 and tick <= input_frame.MAX_TICK,
        "rollback snapshot status tick must be a bounded non-negative integer"
    )
    return lookup_status(history, tick)
end

-- Keep the named start-of-tick boundary and discard only later snapshots from
-- an obsolete predicted timeline. The historical floor never moves backward:
-- already-evicted boundaries cannot become supported merely because the
-- corrected timeline finished earlier.
---@param history RollbackSnapshotHistory
---@param boundary_tick integer
---@return RollbackSnapshotTruncateResult?, string?, RollbackSnapshotHistoryErrorCode?
function rollback_snapshot_history.truncate_after(history, boundary_tick)
    assert_history(history)
    if
        not is_integer(boundary_tick)
        or boundary_tick < 0
        or boundary_tick > input_frame.MAX_TICK
    then
        return nil,
            "rollback snapshot truncate tick must be a bounded non-negative integer",
            "malformed"
    end
    local status = lookup_status(history, boundary_tick)
    if status == "outside_window" then
        return nil,
            ("snapshot boundary %d is older than retained boundary %d"):format(
                boundary_tick,
                assert(oldest_supported_tick(history))
            ),
            "outside_window"
    end
    if status == "missing" then
        return nil,
            ("snapshot boundary %d is missing from retained history"):format(boundary_tick),
            "missing"
    end

    local removed = 0
    for _, entry in pairs(history._entries) do
        if entry.tick > boundary_tick then
            remove_entry(history, entry)
            removed = removed + 1
        end
    end
    history._latest_tick = boundary_tick
    return {
        boundary_tick = boundary_tick,
        removed = removed,
        diagnostics = rollback_snapshot_history.diagnostics(history),
    }
end

---@param history RollbackSnapshotHistory
---@param tick integer
---@return RollbackSnapshotLookup
function rollback_snapshot_history.lookup(history, tick)
    assert_history(history)
    assert(
        is_integer(tick) and tick >= 0 and tick <= input_frame.MAX_TICK,
        "rollback snapshot lookup tick must be a bounded non-negative integer"
    )
    local status = lookup_status(history, tick)
    if status == "missing" or status == "outside_window" then
        return { status = status, tick = tick }
    end
    local entry = assert(history._entries[ring_index(history, tick)])
    assert(entry.tick == tick, "rollback snapshot ring lookup is inconsistent")
    return {
        status = status,
        tick = tick,
        snapshot = copy_owned_snapshot(entry.snapshot),
        canonical_bytes = entry.canonical_bytes,
    }
end

-- Restore an independent MatchState directly from a retained entry. Callers
-- that need a state, rather than a snapshot copy, avoid a restore/capture
-- round-trip through `lookup`.
---@param history RollbackSnapshotHistory
---@param tick integer
---@return MatchState?, RollbackSnapshotLookupStatus
function rollback_snapshot_history.restore(history, tick)
    assert_history(history)
    assert(
        is_integer(tick) and tick >= 0 and tick <= input_frame.MAX_TICK,
        "rollback snapshot restore tick must be a bounded non-negative integer"
    )
    local status = lookup_status(history, tick)
    if status == "missing" or status == "outside_window" then
        return nil, status
    end
    local entry = assert(history._entries[ring_index(history, tick)])
    assert(entry.tick == tick, "rollback snapshot ring restore is inconsistent")
    local state = match_snapshot.restore_owned(entry.snapshot)
    return state, status
end

-- Restore both authoritative halves of a retained simulation boundary. The
-- legacy restore() shape keeps status as return #2 for soccer-only callers.
---@param history RollbackSnapshotHistory
---@param tick integer
---@return MatchState?
---@return CombatMatchState?
---@return RollbackSnapshotLookupStatus
function rollback_snapshot_history.restore_simulation(history, tick)
    assert_history(history)
    assert(
        is_integer(tick) and tick >= 0 and tick <= input_frame.MAX_TICK,
        "rollback simulation restore tick must be a bounded non-negative integer"
    )
    local status = lookup_status(history, tick)
    if status == "missing" or status == "outside_window" then
        return nil, nil, status
    end
    local entry = assert(history._entries[ring_index(history, tick)])
    assert(entry.tick == tick, "rollback snapshot ring restore is inconsistent")
    local state, combat_state = match_snapshot.restore_owned(entry.snapshot)
    return state, combat_state, status
end

-- Hashing and canonical wire materialization are diagnostic and deliberately
-- lazy. Retention counts exact bytes without allocating every wire string.
-- Replacement creates a fresh unhashed entry.
---@param history RollbackSnapshotHistory
---@param tick integer
---@return string?, RollbackSnapshotLookupStatus
function rollback_snapshot_history.boundary_hash(history, tick)
    assert_history(history)
    assert(
        is_integer(tick) and tick >= 0 and tick <= input_frame.MAX_TICK,
        "rollback snapshot hash tick must be a bounded non-negative integer"
    )
    local status = lookup_status(history, tick)
    if status == "missing" or status == "outside_window" then
        return nil, status
    end
    local entry = assert(history._entries[ring_index(history, tick)])
    if entry.hash == nil then
        local wire = entry.canonical_wire or match_snapshot.encode_canonical(entry.snapshot)
        entry.canonical_wire = wire
        entry.hash = fnv1a64.hash(wire)
    end
    return entry.hash, status
end

-- Compare an owned diagnostic snapshot with a retained boundary without
-- copying the retained entry. The caller's snapshot remains independent.
---@param history RollbackSnapshotHistory
---@param tick integer
---@param expected MatchSnapshot
---@return MatchSnapshotDifference?, RollbackSnapshotLookupStatus
function rollback_snapshot_history.first_difference(history, tick, expected)
    assert_history(history)
    assert(
        is_integer(tick) and tick >= 0 and tick <= input_frame.MAX_TICK,
        "rollback snapshot difference tick must be a bounded non-negative integer"
    )
    local status = lookup_status(history, tick)
    if status == "missing" or status == "outside_window" then
        return nil, status
    end
    local entry = assert(history._entries[ring_index(history, tick)])
    return match_snapshot.first_difference_canonical(expected, entry.snapshot), status
end

-- Compare retained canonical boundaries without copying them through a
-- restore/capture cycle. Hashes are materialized only for a mismatch, where
-- they become part of the diagnostic contract.
---@param expected RollbackSnapshotHistory
---@param actual RollbackSnapshotHistory
---@param tick integer
---@return RollbackSnapshotHistoryComparison
function rollback_snapshot_history.compare(expected, actual, tick)
    assert_history(expected)
    assert_history(actual)
    assert(
        is_integer(tick) and tick >= 0 and tick <= input_frame.MAX_TICK,
        "rollback snapshot comparison tick must be a bounded non-negative integer"
    )
    local expected_status = lookup_status(expected, tick)
    local actual_status = lookup_status(actual, tick)
    if
        (expected_status ~= "present" and expected_status ~= "retained")
        or (actual_status ~= "present" and actual_status ~= "retained")
    then
        return {
            matched = false,
            expected_status = expected_status,
            actual_status = actual_status,
            expected_hash = nil,
            actual_hash = nil,
            first_difference = nil,
        }
    end
    local expected_entry = assert(expected._entries[ring_index(expected, tick)])
    local actual_entry = assert(actual._entries[ring_index(actual, tick)])
    local first_difference =
        match_snapshot.first_difference_canonical(expected_entry.snapshot, actual_entry.snapshot)
    if first_difference == nil then
        return {
            matched = true,
            expected_status = expected_status,
            actual_status = actual_status,
            expected_hash = nil,
            actual_hash = nil,
            first_difference = nil,
        }
    end
    local expected_hash = assert(rollback_snapshot_history.boundary_hash(expected, tick))
    local actual_hash = assert(rollback_snapshot_history.boundary_hash(actual, tick))
    return {
        matched = false,
        expected_status = expected_status,
        actual_status = actual_status,
        expected_hash = expected_hash,
        actual_hash = actual_hash,
        first_difference = first_difference,
    }
end

---@param history RollbackSnapshotHistory
---@return RollbackSnapshotHistoryDiagnostics
function rollback_snapshot_history.diagnostics(history)
    assert_history(history)
    local oldest = nil
    for _, entry in pairs(history._entries) do
        if entry and (oldest == nil or entry.tick < oldest) then
            oldest = entry.tick
        end
    end
    return {
        max_rollback_ticks = history._max_rollback_ticks,
        capacity = history._capacity,
        retained_boundary_count = history._count,
        canonical_bytes = history._canonical_bytes,
        peak_retained_boundary_count = history._peak_count,
        peak_canonical_bytes = history._peak_canonical_bytes,
        oldest_supported_tick = oldest_supported_tick(history),
        oldest_boundary_tick = oldest,
        latest_tick = history._latest_tick,
    }
end

return rollback_snapshot_history
