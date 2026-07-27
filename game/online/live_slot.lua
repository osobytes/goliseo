-- Deterministic live-slot geometry for online owned sets.
--
-- `game.online.coordinator.next_live_slot` is the transition *rule*: it consumes
-- an already-total `ranked` ordering and intersects it with a frozen owned set.
-- This module is the other half — the nearest-to-ball ranking and the carrier /
-- winner derivation that *produce* that ordering.
--
-- Two properties are load bearing, because a disagreement here moves the live
-- slot on one peer and not another:
--
--  1. **The ranking is total.** Slots are compared by squared distance to the
--     ball and then by canonical slot index. Canonical indexes are distinct
--     integers, so no two entries ever compare equal in both keys. A strict
--     total order makes `table.sort`'s result unique regardless of the sort's
--     internal pivot choices or of the order elements were inserted, so exact
--     distance ties resolve identically on every peer.
--  2. **No hash-order dependence.** Every table walked here is walked with a
--     numeric `for index = 1, input_frame.SLOT_COUNT` loop or `ipairs`. There is
--     no `pairs()` anywhere in the ranking path. Two peers inside one Lua
--     process share their hash seed and would agree by accident; two browsers
--     would not.
--
-- Squared distance is used deliberately instead of `Vec2:dist`: it is the same
-- monotone ordering with one fewer rounded operation, so the comparison never
-- depends on how a square root rounds.
--
-- This module is pure. It reads a `MatchState` or the `state` half of a
-- `MatchSnapshot` — both expose `ball`, `players[i].pos`, `slot_players`, and
-- `slot_for_player`, with vectors as plain `{ x, y }` fields.

local input_frame = require("sim.input_frame")

---@class LiveSlotRankEntry
---@field slot InputSlotId
---@field index integer -- Canonical slot index, the total-order tiebreak.
---@field distance_sq number

---@class LiveSlotTransitionOptions
---@field switch boolean -- The canonical `switch` edge of the live slot's row.
---@field live InputSlotId? -- Excluded from `ranked`, matching offline switching.
---@field previous_carrier InputSlotId? -- Carrier at the preceding boundary.

---@class OnlineLiveSlotModule
local live_slot = {}

-- Precomputed once with a numeric loop so the per-tick path never rebuilds slot
-- identity and never iterates a hash table.
---@type InputSlotId[]
local SLOT_IDS = {}
---@type table<InputSlotId, integer>
local SLOT_INDEXES = {}
for index = 1, input_frame.SLOT_COUNT do
    local slot = assert(input_frame.slot(index))
    SLOT_IDS[index] = slot.id
    SLOT_INDEXES[slot.id] = index
end

---@param left LiveSlotRankEntry
---@param right LiveSlotRankEntry
---@return boolean
local function entry_before(left, right)
    if left.distance_sq ~= right.distance_sq then
        return left.distance_sq < right.distance_sq
    end
    -- Canonical index ascending. Distinct by construction, so this branch always
    -- decides, and it matches the offline "scan slots in order, keep the strict
    -- best" rule in sim.match.
    return left.index < right.index
end

---@param index integer
---@return InputSlotId
function live_slot.slot_id(index)
    return assert(SLOT_IDS[index], "canonical slot index is out of range")
end

---@param slot InputSlotId
---@return integer
function live_slot.slot_index(slot)
    return assert(SLOT_INDEXES[slot], "canonical slot id is unknown")
end

---@param state MatchState
---@return LiveSlotRankEntry[]
function live_slot.entries(state)
    assert(state.slot_mode, "live-slot ranking requires a slot-mode match")
    local ball = state.ball
    ---@type LiveSlotRankEntry[]
    local entries = {}
    for index = 1, input_frame.SLOT_COUNT do
        local player_index = assert(state.slot_players[index], "canonical slot is unmapped")
        local player = assert(state.players[player_index], "canonical slot player is missing")
        local dx = player.pos.x - ball.x
        local dy = player.pos.y - ball.y
        local distance_sq = dx * dx + dy * dy
        assert(distance_sq == distance_sq, "live-slot distance must be a number")
        entries[index] = {
            slot = SLOT_IDS[index],
            index = index,
            distance_sq = distance_sq,
        }
    end
    table.sort(entries, entry_before)
    return entries
end

-- Every canonical outfield slot ordered by distance to the ball, nearest first,
-- with exact ties broken by ascending canonical slot index.
---@param state MatchState
---@param exclude InputSlotId? -- Usually the currently live slot.
---@return InputSlotId[]
function live_slot.rank(state, exclude)
    local entries = live_slot.entries(state)
    ---@type InputSlotId[]
    local ranked = {}
    for position = 1, #entries do
        local slot = entries[position].slot
        if slot ~= exclude then
            ranked[#ranked + 1] = slot
        end
    end
    return ranked
end

-- The slot holding the ball, or nil while the ball is loose or with a keeper.
-- Keepers are slotless in every mode, so `slot_for_player` simply misses them.
---@param state MatchState
---@return InputSlotId?
function live_slot.carrier(state)
    assert(state.slot_mode, "live-slot carrier requires a slot-mode match")
    local owner = state.owner
    if owner == nil then
        return nil
    end
    local index = state.slot_for_player[owner]
    if index == nil then
        return nil
    end
    return SLOT_IDS[index]
end

-- The complete transition input for `coordinator.next_live_slot`, derived only
-- from the deterministic simulation and the canonical `switch` edge. Nothing
-- local — presentation, frame timing, or when a key was physically pressed —
-- reaches this table.
---@param state MatchState -- The geometry boundary every peer agrees on.
---@param options LiveSlotTransitionOptions
---@return CoordinatorLiveTransition
function live_slot.transition(state, options)
    local carrier = live_slot.carrier(state)
    local winner = nil
    if carrier ~= nil and carrier ~= options.previous_carrier then
        winner = carrier
    end
    return {
        switch = options.switch and true or false,
        carrier = carrier,
        winner = winner,
        ranked = live_slot.rank(state, options.live),
    }
end

---@param sample InputSample
---@return boolean
function live_slot.switch_edge(sample)
    local edge, err = input_frame.has_edge(sample, "switch")
    assert(edge ~= nil, err or "live-slot switch edge requires a valid sample")
    return edge
end

return live_slot
