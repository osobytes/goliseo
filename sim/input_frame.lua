-- Pure, versioned input records for the multiplayer-shaped simulation.
-- This module deliberately knows neither render input nor transport delivery.

---@alias InputTeam "home"|"away"
---@alias InputSlotId
---| "home_1"
---| "home_2"
---| "home_3"
---| "home_4"
---| "away_1"
---| "away_2"
---| "away_3"
---| "away_4"
---@alias InputHeldAction
---| "shoot"
---| "pass"
---| "sprint"
---| "jockey"
---| "lob"
---| "aerial_strike"
---| "aerial_acrobatic"
---| "equipment"
---@alias InputEdgeAction
---| "shoot"
---| "pass"
---| "switch"
---| "dash"
---| "dodge"
---| "equipment_pressed"
---| "equipment_released"
---@alias InputFrameErrorCode "malformed"|"unsupported_version"|"wire_too_large"

---@class InputSlot
---@field index integer -- Canonical position in every InputFrame (one through eight).
---@field id InputSlotId
---@field team InputTeam
---@field outfield_index integer -- Stable one-based outfield position within the team.

---@class InputSample
---@field move_x integer -- Signed quantized horizontal axis in [-127, 127].
---@field move_y integer -- Signed quantized vertical axis in [-127, 127].
---@field held integer -- Bitmask of InputHeldAction values currently held for this tick.
---@field edges integer -- Bitmask of InputEdgeAction values that occurred during this tick only.

---@class InputSampleOptions
---@field move_x integer?
---@field move_y integer?
---@field held integer?
---@field edges integer?

---@class InputFrame
---@field version integer -- Exactly InputFrame.VERSION.
---@field tick integer -- Non-negative fixed-simulation tick.
---@field slots InputSample[] -- Exactly eight samples in canonical InputSlot order.

---@class InputSlotAssignment
---@field slot InputSlotId
---@field team InputTeam
---@field player_id string -- Authored outfield PlayerData id, never a keeper.

---@class InputFixtureRosters
---@field home string[] -- Exactly five ordered home-fixture PlayerData ids, including its AI keeper.
---@field away string[] -- Exactly five ordered away-fixture PlayerData ids, including its AI keeper.

---@class InputRosterMembership
---@field home table<string, boolean>
---@field away table<string, boolean>

---@class InputOwnership
---@field version integer -- Exactly InputFrame.VERSION.
---@field rosters InputFixtureRosters -- Explicit fixture-side roster membership.
---@field slots InputSlotAssignment[] -- Exactly eight assignments in canonical InputSlot order.

---@class InputFrameModule
local input_frame = {}

input_frame.VERSION = 2
input_frame.HOME_SLOT_COUNT = 4
input_frame.AWAY_SLOT_COUNT = 4
input_frame.SLOT_COUNT = input_frame.HOME_SLOT_COUNT + input_frame.AWAY_SLOT_COUNT
input_frame.FIXTURE_TEAM_SIZE = input_frame.HOME_SLOT_COUNT + 1
input_frame.MOVE_SCALE = 127
input_frame.MAX_TICK = 2147483647
input_frame.MAX_PLAYER_ID_BYTES = 64
input_frame.MAX_SAMPLE_WIRE_BYTES = 8 -- ASCII bytes: two hex characters per sample byte.
input_frame.MAX_WIRE_BYTES = 156

---@type table<InputHeldAction, integer>
input_frame.HELD_BITS = {
    shoot = 1,
    pass = 2,
    sprint = 4,
    jockey = 8,
    lob = 16,
    aerial_strike = 32,
    aerial_acrobatic = 64,
    equipment = 128,
}

---@type table<InputEdgeAction, integer>
input_frame.EDGE_BITS = {
    shoot = 1,
    pass = 2,
    switch = 4,
    dash = 8,
    dodge = 16,
    equipment_pressed = 32,
    equipment_released = 64,
}

local MAX_HELD_MASK = 255
local MAX_EDGE_MASK = 127

---@type InputSlot[]
local SLOT_ORDER = {
    { index = 1, id = "home_1", team = "home", outfield_index = 1 },
    { index = 2, id = "home_2", team = "home", outfield_index = 2 },
    { index = 3, id = "home_3", team = "home", outfield_index = 3 },
    { index = 4, id = "home_4", team = "home", outfield_index = 4 },
    { index = 5, id = "away_1", team = "away", outfield_index = 1 },
    { index = 6, id = "away_2", team = "away", outfield_index = 2 },
    { index = 7, id = "away_3", team = "away", outfield_index = 3 },
    { index = 8, id = "away_4", team = "away", outfield_index = 4 },
}

local FRAME_FIELDS = {
    version = true,
    tick = true,
    slots = true,
}

local SAMPLE_FIELDS = {
    move_x = true,
    move_y = true,
    held = true,
    edges = true,
}

local OWNERSHIP_FIELDS = {
    version = true,
    rosters = true,
    slots = true,
}

local ROSTER_FIELDS = {
    home = true,
    away = true,
}

local ASSIGNMENT_FIELDS = {
    slot = true,
    team = true,
    player_id = true,
}

---@param value any
---@return boolean
local function is_integer(value)
    return type(value) == "number"
        and value == value
        and value ~= math.huge
        and value ~= -math.huge
        and value == math.floor(value)
end

---@param value any
---@param allowed table<string, boolean>
---@return boolean
local function has_only_fields(value, allowed)
    if type(value) ~= "table" then
        return false
    end
    for key in pairs(value) do
        if type(key) ~= "string" or not allowed[key] then
            return false
        end
    end
    return true
end

---@param code InputFrameErrorCode
---@param message string
---@return nil, string, InputFrameErrorCode
local function failure(code, message)
    return nil, message, code
end

---@param value any
---@return boolean
local function is_axis(value)
    return is_integer(value)
        and value >= -input_frame.MOVE_SCALE
        and value <= input_frame.MOVE_SCALE
end

---@param value any
---@param maximum integer
---@return boolean
local function is_mask(value, maximum)
    return is_integer(value) and value >= 0 and value <= maximum
end

---@param mask integer
---@param bit integer
---@return boolean
local function has_bit(mask, bit)
    return math.floor(mask / bit) % 2 == 1
end

---@param value any
---@return boolean
local function is_canonical_array(value)
    if type(value) ~= "table" then
        return false
    end
    for index in pairs(value) do
        if
            type(index) ~= "number"
            or not is_integer(index)
            or index < 1
            or index > input_frame.SLOT_COUNT
        then
            return false
        end
    end
    for index = 1, input_frame.SLOT_COUNT do
        if value[index] == nil then
            return false
        end
    end
    return true
end

---@param value any
---@return boolean
local function is_roster_array(value)
    if type(value) ~= "table" then
        return false
    end
    local count = 0
    for index in pairs(value) do
        if type(index) ~= "number" or not is_integer(index) or index < 1 then
            return false
        end
        count = math.max(count, index)
    end
    if count ~= input_frame.FIXTURE_TEAM_SIZE then
        return false
    end
    for index = 1, count do
        if value[index] == nil then
            return false
        end
    end
    return true
end

---@param sample any
---@return boolean?, string?, InputFrameErrorCode?
function input_frame.validate_sample(sample)
    if type(sample) ~= "table" or not has_only_fields(sample, SAMPLE_FIELDS) then
        return failure("malformed", "input sample must contain only canonical fields")
    end
    if not is_axis(sample.move_x) or not is_axis(sample.move_y) then
        return failure("malformed", "input sample movement axes must be signed 8-bit integers")
    end
    if not is_mask(sample.held, MAX_HELD_MASK) then
        return failure("malformed", "input sample held mask is invalid")
    end
    if not is_mask(sample.edges, MAX_EDGE_MASK) then
        return failure("malformed", "input sample edge mask is invalid")
    end
    local equipment_held = has_bit(sample.held, input_frame.HELD_BITS.equipment)
    local equipment_pressed = has_bit(sample.edges, input_frame.EDGE_BITS.equipment_pressed)
    local equipment_released = has_bit(sample.edges, input_frame.EDGE_BITS.equipment_released)
    if
        (equipment_pressed and not equipment_released and not equipment_held)
        or (equipment_released and equipment_held)
    then
        return failure("malformed", "input sample equipment transition combination is invalid")
    end
    return true
end

-- The compact packet codec uses one explicit byte for each version-2 sample
-- component. Axes are biased into [0, 254]; held and edge masks retain their
-- complete canonical bytes. Lowercase hexadecimal keeps the result ASCII-safe
-- without allowing a transport to infer or repeat combat edges.
---@param sample InputSample
---@return string?, string?, InputFrameErrorCode?
function input_frame.encode_sample(sample)
    local ok, err, code = input_frame.validate_sample(sample)
    if not ok then
        return nil, err, code
    end
    return ("%02x%02x%02x%02x"):format(
        sample.move_x + input_frame.MOVE_SCALE,
        sample.move_y + input_frame.MOVE_SCALE,
        sample.held,
        sample.edges
    )
end

---@param wire any
---@return InputSample?, string?, InputFrameErrorCode?
function input_frame.decode_sample(wire)
    if
        type(wire) ~= "string"
        or #wire ~= input_frame.MAX_SAMPLE_WIRE_BYTES
        or not wire:match("^[0-9a-f]+$")
    then
        return failure(
            "malformed",
            "input sample wire must be eight lowercase hex characters encoding four bytes"
        )
    end
    local sample = {
        move_x = assert(tonumber(wire:sub(1, 2), 16)) - input_frame.MOVE_SCALE,
        move_y = assert(tonumber(wire:sub(3, 4), 16)) - input_frame.MOVE_SCALE,
        held = assert(tonumber(wire:sub(5, 6), 16)),
        edges = assert(tonumber(wire:sub(7, 8), 16)),
    }
    local ok, err, code = input_frame.validate_sample(sample)
    if not ok then
        return nil, err, code
    end
    if input_frame.encode_sample(sample) ~= wire then
        return failure("malformed", "input sample wire is not canonical")
    end
    return sample
end

---@param options InputSample|InputSampleOptions|nil
---@return InputSample?, string?, InputFrameErrorCode?
function input_frame.new_sample(options)
    if options == nil then
        options = {}
    end
    if not has_only_fields(options, SAMPLE_FIELDS) then
        return failure("malformed", "input sample options must contain only canonical fields")
    end
    local sample = {
        move_x = options.move_x == nil and 0 or options.move_x,
        move_y = options.move_y == nil and 0 or options.move_y,
        held = options.held == nil and 0 or options.held,
        edges = options.edges == nil and 0 or options.edges,
    }
    local ok, err, code = input_frame.validate_sample(sample)
    if not ok then
        return nil, err, code
    end
    return sample
end

---@return InputSample
function input_frame.neutral_sample()
    return {
        move_x = 0,
        move_y = 0,
        held = 0,
        edges = 0,
    }
end

---@param index integer
---@return InputSlot?, string?, InputFrameErrorCode?
function input_frame.slot(index)
    if not is_integer(index) or index < 1 or index > input_frame.SLOT_COUNT then
        return failure("malformed", "input slot index must be between one and eight")
    end
    local slot = SLOT_ORDER[index]
    return {
        index = slot.index,
        id = slot.id,
        team = slot.team,
        outfield_index = slot.outfield_index,
    }
end

---@param team InputTeam
---@param outfield_index integer
---@return integer?, string?, InputFrameErrorCode?
function input_frame.slot_index(team, outfield_index)
    if
        (team ~= "home" and team ~= "away")
        or not is_integer(outfield_index)
        or outfield_index < 1
        or outfield_index > input_frame.HOME_SLOT_COUNT
    then
        return failure("malformed", "input team and outfield index must name a stable slot")
    end
    return team == "home" and outfield_index or input_frame.HOME_SLOT_COUNT + outfield_index
end

---@param raw_axis number
---@return integer?, string?, InputFrameErrorCode?
function input_frame.quantize_axis(raw_axis)
    if
        type(raw_axis) ~= "number"
        or raw_axis ~= raw_axis
        or raw_axis == math.huge
        or raw_axis == -math.huge
    then
        return failure("malformed", "raw movement axis must be finite")
    end
    local clamped = math.max(-1, math.min(1, raw_axis))
    local scaled = clamped * input_frame.MOVE_SCALE
    local quantized = scaled >= 0 and math.floor(scaled + 0.5) or math.ceil(scaled - 0.5)
    if quantized == 0 then
        return 0
    end
    ---@cast quantized integer
    return quantized
end

---@param raw_x number
---@param raw_y number
---@return integer?, integer?, string?, InputFrameErrorCode?
function input_frame.quantize_move(raw_x, raw_y)
    local move_x, x_err, x_code = input_frame.quantize_axis(raw_x)
    if move_x == nil then
        return nil, nil, x_err, x_code
    end
    local move_y, y_err, y_code = input_frame.quantize_axis(raw_y)
    if move_y == nil then
        return nil, nil, y_err, y_code
    end
    return move_x, move_y
end

---@param axis integer
---@return number?, string?, InputFrameErrorCode?
function input_frame.dequantize_axis(axis)
    if not is_axis(axis) then
        return failure("malformed", "quantized movement axis must be signed 8-bit")
    end
    return axis / input_frame.MOVE_SCALE
end

---@param sample InputSample
---@return number?, number?, string?, InputFrameErrorCode?
function input_frame.dequantize_move(sample)
    local ok, err, code = input_frame.validate_sample(sample)
    if not ok then
        return nil, nil, err, code
    end
    return sample.move_x / input_frame.MOVE_SCALE, sample.move_y / input_frame.MOVE_SCALE
end

---@param sample InputSample
---@param action InputHeldAction
---@return boolean?, string?, InputFrameErrorCode?
function input_frame.is_held(sample, action)
    local ok, err, code = input_frame.validate_sample(sample)
    if not ok then
        return nil, err, code
    end
    local bit = input_frame.HELD_BITS[action]
    if bit == nil then
        return failure("malformed", "unknown held input action")
    end
    return has_bit(sample.held, bit)
end

---@param sample InputSample
---@param action InputEdgeAction
---@return boolean?, string?, InputFrameErrorCode?
function input_frame.has_edge(sample, action)
    local ok, err, code = input_frame.validate_sample(sample)
    if not ok then
        return nil, err, code
    end
    local bit = input_frame.EDGE_BITS[action]
    if bit == nil then
        return failure("malformed", "unknown edge input action")
    end
    return has_bit(sample.edges, bit)
end

---@param frame any
---@return boolean?, string?, InputFrameErrorCode?
function input_frame.validate(frame)
    if type(frame) ~= "table" or not has_only_fields(frame, FRAME_FIELDS) then
        return failure("malformed", "input frame must contain only canonical fields")
    end
    if not is_integer(frame.version) then
        return failure("malformed", "input frame version must be an integer")
    end
    if frame.version ~= input_frame.VERSION then
        return failure("unsupported_version", "unsupported input frame version")
    end
    if not is_integer(frame.tick) or frame.tick < 0 or frame.tick > input_frame.MAX_TICK then
        return failure("malformed", "input frame tick must be a bounded non-negative integer")
    end
    if not is_canonical_array(frame.slots) then
        return failure("malformed", "input frame must contain exactly eight canonical slots")
    end
    for index = 1, input_frame.SLOT_COUNT do
        local ok, err, code = input_frame.validate_sample(frame.slots[index])
        if not ok then
            return nil, ("input slot %d: %s"):format(index, err or "invalid sample"), code
        end
    end
    return true
end

---@param tick integer
---@param slots InputSample[]?
---@return InputFrame?, string?, InputFrameErrorCode?
function input_frame.new(tick, slots)
    if slots ~= nil and not is_canonical_array(slots) then
        return failure(
            "malformed",
            "input frame slots must contain exactly eight canonical entries"
        )
    end
    local copied_slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        local sample, err, code = input_frame.new_sample(slots and slots[index] or nil)
        if not sample then
            return nil, err, code
        end
        copied_slots[index] = sample
    end
    local frame = {
        version = input_frame.VERSION,
        tick = tick,
        slots = copied_slots,
    }
    local ok, err, code = input_frame.validate(frame)
    if not ok then
        return nil, err, code
    end
    return frame
end

---@param tick integer
---@return InputFrame?, string?, InputFrameErrorCode?
function input_frame.neutral(tick)
    return input_frame.new(tick)
end

---@param frame InputFrame
---@return InputFrame?, string?, InputFrameErrorCode?
function input_frame.copy(frame)
    local ok, err, code = input_frame.validate(frame)
    if not ok then
        return nil, err, code
    end
    return input_frame.new(frame.tick, frame.slots)
end

---@param roster any
---@param team InputTeam
---@param players_by_id table<string, PlayerData>
---@param fixture_players table<string, boolean>
---@return table<string, boolean>?, string?, InputFrameErrorCode?
local function collect_roster_membership(roster, team, players_by_id, fixture_players)
    if not is_roster_array(roster) then
        return failure("malformed", ("%s fixture roster is invalid"):format(team))
    end
    local members = {}
    local keeper_count = 0
    for index = 1, #roster do
        local player_id = roster[index]
        if type(player_id) ~= "string" or player_id == "" then
            return failure("malformed", ("%s fixture roster has an invalid player id"):format(team))
        end
        if #player_id > input_frame.MAX_PLAYER_ID_BYTES then
            return failure("malformed", ("%s fixture roster player id is too long"):format(team))
        end
        local player = players_by_id[player_id]
        if player == nil then
            return failure("malformed", ("%s fixture roster names an unknown player"):format(team))
        end
        if fixture_players[player_id] then
            return failure("malformed", "one roster player cannot belong to both fixture sides")
        end
        fixture_players[player_id] = true
        members[player_id] = true
        if player.position == "keeper" then
            keeper_count = keeper_count + 1
        end
    end
    if keeper_count ~= 1 then
        return failure("malformed", ("%s fixture roster needs exactly one keeper"):format(team))
    end
    return members
end

---@param rosters any
---@param players_by_id table<string, PlayerData>
---@return InputRosterMembership?, string?, InputFrameErrorCode?
local function roster_memberships(rosters, players_by_id)
    if type(rosters) ~= "table" or not has_only_fields(rosters, ROSTER_FIELDS) then
        return failure("malformed", "input ownership needs canonical fixture rosters")
    end
    local fixture_players = {}
    local home, home_err, home_code =
        collect_roster_membership(rosters.home, "home", players_by_id, fixture_players)
    if home == nil then
        return nil, home_err, home_code
    end
    local away, away_err, away_code =
        collect_roster_membership(rosters.away, "away", players_by_id, fixture_players)
    if away == nil then
        return nil, away_err, away_code
    end
    return { home = home, away = away }
end

---@param rosters InputFixtureRosters
---@return InputFixtureRosters
local function copy_rosters(rosters)
    local home = {}
    local away = {}
    for index = 1, #rosters.home do
        home[index] = rosters.home[index]
    end
    for index = 1, #rosters.away do
        away[index] = rosters.away[index]
    end
    return { home = home, away = away }
end

---@param ownership any
---@param players_by_id table<string, PlayerData>
---@return boolean?, string?, InputFrameErrorCode?
function input_frame.validate_ownership(ownership, players_by_id)
    if type(ownership) ~= "table" or not has_only_fields(ownership, OWNERSHIP_FIELDS) then
        return failure("malformed", "input ownership must contain only canonical fields")
    end
    if not is_integer(ownership.version) then
        return failure("malformed", "input ownership version must be an integer")
    end
    if ownership.version ~= input_frame.VERSION then
        return failure("unsupported_version", "unsupported input ownership version")
    end
    if not is_canonical_array(ownership.slots) then
        return failure("malformed", "input ownership must contain exactly eight canonical slots")
    end
    if type(players_by_id) ~= "table" then
        return failure("malformed", "input ownership requires a roster player index")
    end
    local memberships, membership_err, membership_code =
        roster_memberships(ownership.rosters, players_by_id)
    if memberships == nil then
        return nil, membership_err, membership_code
    end

    local seen_players = {}
    for index = 1, input_frame.SLOT_COUNT do
        local assignment = ownership.slots[index]
        local slot = SLOT_ORDER[index]
        if type(assignment) ~= "table" or not has_only_fields(assignment, ASSIGNMENT_FIELDS) then
            return failure("malformed", ("input ownership slot %d is malformed"):format(index))
        end
        if assignment.slot ~= slot.id or assignment.team ~= slot.team then
            return failure(
                "malformed",
                ("input ownership slot %d violates canonical team order"):format(index)
            )
        end
        if type(assignment.player_id) ~= "string" or assignment.player_id == "" then
            return failure("malformed", ("input ownership slot %d needs a player id"):format(index))
        end
        if #assignment.player_id > input_frame.MAX_PLAYER_ID_BYTES then
            return failure(
                "malformed",
                ("input ownership slot %d player id is too long"):format(index)
            )
        end
        if seen_players[assignment.player_id] then
            return failure("malformed", "one roster player cannot own multiple input slots")
        end
        local player = players_by_id[assignment.player_id]
        if player == nil then
            return failure(
                "malformed",
                ("input ownership slot %d names an unknown player"):format(index)
            )
        end
        if not memberships[slot.team][assignment.player_id] then
            return failure(
                "malformed",
                ("input ownership slot %d binds a player from the other fixture side"):format(index)
            )
        end
        if player.position == "keeper" then
            return failure("malformed", "keepers cannot own input slots")
        end
        seen_players[assignment.player_id] = true
    end
    return true
end

---@param assignments InputSlotAssignment[]
---@param rosters InputFixtureRosters
---@param players_by_id table<string, PlayerData>
---@return InputOwnership?, string?, InputFrameErrorCode?
function input_frame.new_ownership(assignments, rosters, players_by_id)
    if not is_canonical_array(assignments) then
        return failure(
            "malformed",
            "input ownership assignments must contain exactly eight canonical entries"
        )
    end
    if type(players_by_id) ~= "table" then
        return failure("malformed", "input ownership requires a roster player index")
    end
    local memberships, membership_err, membership_code = roster_memberships(rosters, players_by_id)
    if memberships == nil then
        return nil, membership_err, membership_code
    end
    local copied_slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        local assignment = assignments[index]
        if type(assignment) ~= "table" or not has_only_fields(assignment, ASSIGNMENT_FIELDS) then
            return failure("malformed", ("input ownership slot %d is malformed"):format(index))
        end
        copied_slots[index] = {
            slot = assignment.slot,
            team = assignment.team,
            player_id = assignment.player_id,
        }
    end
    local ownership = {
        version = input_frame.VERSION,
        rosters = copy_rosters(rosters),
        slots = copied_slots,
    }
    local ok, err, code = input_frame.validate_ownership(ownership, players_by_id)
    if not ok then
        return nil, err, code
    end
    return ownership
end

---@param ownership InputOwnership
---@param players_by_id table<string, PlayerData>
---@return InputOwnership?, string?, InputFrameErrorCode?
function input_frame.copy_ownership(ownership, players_by_id)
    local ok, err, code = input_frame.validate_ownership(ownership, players_by_id)
    if not ok then
        return nil, err, code
    end
    return input_frame.new_ownership(ownership.slots, ownership.rosters, players_by_id)
end

---@param value string
---@return integer?
local function parse_unsigned(value)
    if value == "0" or value:match("^[1-9]%d*$") then
        local parsed = tonumber(value)
        if parsed == nil then
            return nil
        end
        ---@cast parsed integer
        return parsed
    end
    return nil
end

---@param value string
---@return integer?
local function parse_axis(value)
    if value == "0" then
        return 0
    end
    if value:match("^[1-9]%d*$") or value:match("^%-[1-9]%d*$") then
        local parsed = tonumber(value)
        if parsed == nil then
            return nil
        end
        ---@cast parsed integer
        return parsed
    end
    return nil
end

---@param value string
---@return string[]
local function split_pipe(value)
    local fields = {}
    local start = 1
    while true do
        local separator = value:find("|", start, true)
        if not separator then
            fields[#fields + 1] = value:sub(start)
            return fields
        end
        fields[#fields + 1] = value:sub(start, separator - 1)
        start = separator + 1
    end
end

---@param sample InputSample
---@return string
local function encode_frame_sample(sample)
    return table.concat({
        tostring(sample.move_x),
        tostring(sample.move_y),
        tostring(sample.held),
        tostring(sample.edges),
    }, ",")
end

---@param frame InputFrame
---@return string?, string?, InputFrameErrorCode?
function input_frame.encode(frame)
    local ok, err, code = input_frame.validate(frame)
    if not ok then
        return nil, err, code
    end
    local fields = { tostring(frame.version), tostring(frame.tick) }
    for index = 1, input_frame.SLOT_COUNT do
        fields[#fields + 1] = encode_frame_sample(frame.slots[index])
    end
    local wire = table.concat(fields, "|")
    if #wire > input_frame.MAX_WIRE_BYTES then
        return failure("wire_too_large", "input frame wire exceeds the byte limit")
    end
    return wire
end

---@param wire string
---@return InputFrame?, string?, InputFrameErrorCode?
function input_frame.decode(wire)
    if type(wire) ~= "string" then
        return failure("malformed", "input frame wire must be a string")
    end
    if #wire > input_frame.MAX_WIRE_BYTES then
        return failure("wire_too_large", "input frame wire exceeds the byte limit")
    end

    local fields = split_pipe(wire)
    if #fields ~= input_frame.SLOT_COUNT + 2 then
        return failure("malformed", "input frame wire has invalid fields")
    end
    local version = parse_unsigned(fields[1])
    local tick = parse_unsigned(fields[2])
    if version == nil or tick == nil then
        return failure("malformed", "input frame wire version and tick must be canonical integers")
    end

    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        local raw_x, raw_y, raw_held, raw_edges =
            fields[index + 2]:match("^([^,]*),([^,]*),([^,]*),([^,]*)$")
        if raw_x == nil then
            return failure("malformed", ("input frame wire slot %d is invalid"):format(index))
        end
        local move_x = parse_axis(raw_x)
        local move_y = parse_axis(raw_y)
        local held = parse_unsigned(raw_held)
        local edges = parse_unsigned(raw_edges)
        if move_x == nil or move_y == nil or held == nil or edges == nil then
            return failure("malformed", ("input frame wire slot %d is not canonical"):format(index))
        end
        slots[index] = {
            move_x = move_x,
            move_y = move_y,
            held = held,
            edges = edges,
        }
    end

    local frame = {
        version = version,
        tick = tick,
        slots = slots,
    }
    local ok, err, code = input_frame.validate(frame)
    if not ok then
        return nil, err, code
    end
    return input_frame.copy(frame)
end

return input_frame
