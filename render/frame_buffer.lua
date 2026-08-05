-- The `RenderFrame` payload as a flat block a foreign runtime can read (#332).
--
-- `render.frame` produces the payload; this module is what makes it CROSSABLE.
-- A `RenderFrame` is Lua tables and Lua strings, and neither exists on the far
-- side of a wasm boundary. Handing it to JavaScript field by field would be one
-- boundary crossing per field per entity per frame -- exactly the shape
-- `render/frame.lua` was designed to avoid, undone at the last step. So the
-- payload is serialised once into one contiguous array of doubles, copied into
-- linear memory in one go, and read on the far side as a single typed-array
-- view. One crossing per rendered frame, in batch.
--
-- Doubles for EVERYTHING, including booleans, enums and counts. A mixed-width
-- layout would save bytes and cost the property that makes this cheap: a single
-- `Float64Array` over the whole block with no per-section re-slicing, no
-- alignment arithmetic, and no endian question. Lua 5.1 has no integer subtype
-- and no `string.pack`, so a double array is also the only encoding it can
-- produce without hand-rolled byte packing.
--
-- Four encoding rules, each of them forced by something in `render/frame.lua`:
--
-- 1. ENUMS BECOME SMALL INTEGERS, AND 0 ALWAYS MEANS ABSENT. Every closed
--    string set (`PlayerPoseId`, `MatchEventKind`, ...) has a numbering here.
--    Slot 0 is never a real member, so a sparse array's holes encode as 0 and
--    decode back to `nil` with no separate presence bit.
--
-- 2. SPARSE *NUMBERS* GET AN EXPLICIT PRESENCE FLAG. `difficulty` and
--    `keeper_depth` are sparse and numeric, so there is no unused value to
--    reserve -- 0 is a legitimate difficulty. `render/frame.lua` already
--    refused to let absent and false collapse together for booleans; the same
--    reasoning applies here, so `has_difficulty` / `has_keeper_depth` ride
--    alongside. `ball.landing_x` / `ball.landing_y` get `ball_landing_valid`
--    for the same reason.
--
-- 3. ONE-BASED SLOTS MAKE 0 A FREE SENTINEL. `possession.owner`,
--    `control.pass_target` and an event's `slot` are roster indices, so 0
--    encodes "none" without a flag.
--
-- 4. STRINGS DO NOT CROSS PER FRAME. The only per-frame strings in the payload
--    are `hud.controlled_id` and an event's `player`, and both are roster
--    lookups of an index that is already in the block (`hud.controlled`, the
--    event's `slot`). They are dropped from the frame block and recovered from
--    the roster, which crosses once. `RenderFrameRoster`'s own `ids` and
--    `names` cross once too, as a newline-joined blob beside its numeric block.
--
-- WHAT IS NOT CARRIED: `RenderFrame.combat`. It is the one disclosed nested
-- exception in `render/frame.lua` -- a `CombatPresentationModel` with `Vec2`
-- values -- and flattening it is that flattening's job, not this one's. Rather
-- than drop it silently, the header carries `combat_present`, so a reader can
-- tell "this frame had no combat telegraphs" from "this layout cannot carry
-- them yet" and fail loudly instead of drawing nothing.
--
-- VERSIONING. `render/frame.lua` stamps `version` on every payload and says the
-- read-side assertion belongs to the first consumer that deserializes it across
-- a boundary. That is this module. Two numbers are stamped and both are
-- checked, because they can change independently:
--
--   RENDER_FRAME_VERSION  the payload's meaning, `render_frame.VERSION`.
--   LAYOUT_VERSION        this block's shape -- field order, header size.
--
-- A field reordered here changes the second and not the first; a field added to
-- `RenderFrame` changes the first. `decode` hard-asserts both, and `encode`
-- hard-asserts the frame it was handed carries the version it claims. The
-- non-Lua reader (`wasm/sim-host/frame_payload.js`) checks the same two words
-- against its own constants, so a Lua producer and a JS reader cannot silently
-- disagree.

local render_frame = require("render.frame")

---@class RenderFrameBufferModule
local frame_buffer = {}

-- Shape of this block. Bump on any change to field order, header size or the
-- enum numberings below -- all of them are read positionally.
frame_buffer.LAYOUT_VERSION = 1

-- Ordinary sentinels, chosen so a misaligned or wrong-buffer read fails on the
-- first word instead of decoding garbage. "GOLF" and "GOLR" as big-endian ASCII.
frame_buffer.MAGIC = 0x474F4C46
frame_buffer.ROSTER_MAGIC = 0x474F4C52

-- Header words, 1-based, in order. `total_words` lets a reader size its view
-- from the header alone, which is what keeps a frame read at one crossing.
frame_buffer.HEADER_FIELDS = {
    "magic",
    "layout_version",
    "render_frame_version",
    "total_words",
    "header_words",
    "scalar_words",
    "player_count",
    "player_field_count",
    "event_count",
    "event_field_count",
    "combat_present",
    "reserved",
}
frame_buffer.HEADER_WORDS = #frame_buffer.HEADER_FIELDS

-- The once-per-frame scalars: field geometry, ball, possession, control, HUD.
-- Flat and prefixed rather than nested, because a nested section would need its
-- own offset word and buy nothing -- the whole run is fixed length.
frame_buffer.SCALAR_FIELDS = {
    "field_w",
    "field_h",
    "field_crossbar_h",
    "field_penalty_box_depth",
    "field_penalty_box_h",
    "goal_home_x",
    "goal_home_y",
    "goal_home_w",
    "goal_home_h",
    "goal_away_x",
    "goal_away_y",
    "goal_away_w",
    "goal_away_h",
    "ball_x",
    "ball_y",
    "ball_z",
    "ball_vx",
    "ball_vy",
    "ball_vz",
    "ball_visible",
    "ball_landing_valid",
    "ball_landing_x",
    "ball_landing_y",
    "possession_owner",
    "possession_owner_team",
    "possession_keeper_holds",
    "control_controlled",
    "control_pass_target",
    "control_charge_kind",
    "control_charge",
    "hud_home_score",
    "hud_away_score",
    "hud_time_left",
    "hud_finished",
    "hud_possession_team",
    "hud_controlled",
    "hud_controlled_team",
    "hud_controlled_is_keeper",
    "hud_controlled_owns_ball",
    "hud_controlled_stamina",
    "hud_species_shape",
    "hud_species_color_r",
    "hud_species_color_g",
    "hud_species_color_b",
}

-- Per-player arrays, field-major: all of `x`, then all of `y`, and so on. That
-- is the structure-of-arrays promise made concrete -- a reader that only draws
-- positions subarrays two runs and never touches the other nineteen.
frame_buffer.PLAYER_FIELDS = {
    "x",
    "y",
    "facing_x",
    "facing_y",
    "speed",
    "pose_id",
    "pose_priority",
    "pose_source",
    "controlled",
    "dashing",
    "holding",
    "dive",
    "dive_dir_x",
    "dive_dir_y",
    "grab",
    "throw",
    "windup",
    "aerial",
    "aerial_jump",
    "aerial_style",
    "aerial_outcome",
}

-- Per-event arrays, field-major. `player` (a roster id string) is deliberately
-- absent: `slot` indexes the roster, which already crossed.
frame_buffer.EVENT_FIELDS = {
    "kind",
    "x",
    "y",
    "slot",
    "save_style",
    "style",
    "outcome",
    "has_difficulty",
    "difficulty",
    "shot_type",
    "keeper_state",
    "has_keeper_depth",
    "keeper_depth",
    "jumping",
    "on_target",
}

-- The roster's numeric half. Its `ids` and `names` travel as a separate blob.
frame_buffer.ROSTER_HEADER_FIELDS = {
    "magic",
    "layout_version",
    "render_frame_version",
    "total_words",
    "header_words",
    "count",
    "field_count",
}
frame_buffer.ROSTER_HEADER_WORDS = #frame_buffer.ROSTER_HEADER_FIELDS

frame_buffer.ROSTER_FIELDS = {
    "team",
    "is_keeper",
    "radius",
    "species_shape",
    "species_color_r",
    "species_color_g",
    "species_color_b",
}

-- Enum numberings. Every one of these starts at 1 so that 0 stays reserved for
-- "absent". They are part of LAYOUT_VERSION: appending is safe, renumbering is
-- not, and either way the far side reads these same numbers.
---@type table<RenderTeam, integer>
frame_buffer.TEAM = { home = 1, away = 2 }

---@type table<RenderSpeciesShape, integer>
frame_buffer.SPECIES_SHAPE = { round = 1, broad = 2, angular = 3, cluster = 4 }

---@type table<RenderChargeKind, integer>
frame_buffer.CHARGE_KIND = { shot = 1, pass = 2 }

---@type table<PlayerPoseSource, integer>
frame_buffer.POSE_SOURCE = { soccer = 1, combat = 2, locomotion = 3 }

---@type table<AerialStyle, integer>
frame_buffer.AERIAL_STYLE = {
    leg_control = 1,
    chest_control = 2,
    volley = 3,
    header = 4,
    bicycle = 5,
}

---@type table<AerialOutcome, integer>
frame_buffer.AERIAL_OUTCOME = { clean = 1, heavy = 2, miss = 3 }

---@type table<SaveStyle, integer>
frame_buffer.SAVE_STYLE = { spread = 1, central = 2, stretch = 3 }

---@type table<KeeperBehaviorState, integer>
frame_buffer.KEEPER_STATE = {
    base = 1,
    advance = 2,
    contain = 3,
    set = 4,
    retreat = 5,
    recover = 6,
}

---@type table<KeeperShotType, integer>
frame_buffer.SHOT_TYPE = { ground = 1, chip = 2 }

---@type table<PlayerPoseId, integer>
frame_buffer.POSE_ID = {
    keeper_grab = 1,
    keeper_throw = 2,
    keeper_punt = 3,
    keeper_tip = 4,
    keeper_spread = 5,
    keeper_central = 6,
    keeper_stretch = 7,
    keeper_dive = 8,
    keeper_get_up = 9,
    keeper_set = 10,
    keeper_ready_low = 11,
    keeper_shuffle = 12,
    keeper_ready_tall = 13,
    aerial_bicycle = 14,
    aerial_action = 15,
    combat_knockback = 16,
    combat_stagger = 17,
    combat_guard = 18,
    combat_active = 19,
    combat_windup = 20,
    combat_aim = 21,
    combat_recovery = 22,
    soccer_windup = 23,
    slide = 24,
    tackle = 25,
    stumble = 26,
    kick_follow = 27,
    settle = 28,
    run_telegraph = 29,
    contain = 30,
    fatigue = 31,
    locomotion = 32,
}

---@type table<MatchEventKind, integer>
frame_buffer.EVENT_KIND = {
    shot = 1,
    pass = 2,
    touch = 3,
    tackle = 4,
    press_commit_heavy_touch = 5,
    press_commit_exposed_ball = 6,
    press_commit_cover = 7,
    press_commit_box_desperation = 8,
    press_commit_low_discipline = 9,
    combat_commit_carrier_contest = 10,
    combat_commit_carrier_protection = 11,
    combat_commit_loose_ball_contest = 12,
    combat_commit_passing_lane_or_shot_denial = 13,
    combat_commit_recovery_punish = 14,
    combat_commit_unattributed_off_ball = 15,
    catch = 16,
    parry = 17,
    tip = 18,
    claim = 19,
    block = 20,
    header = 21,
    volley = 22,
    bicycle = 23,
    reception = 24,
    juke = 25,
}

---@param codes table<string, integer>
---@return table<integer, string>
local function invert(codes)
    local out = {}
    for name, code in pairs(codes) do
        assert(code >= 1, "enum code 0 is reserved for absent")
        assert(out[code] == nil, "duplicate enum code " .. tostring(code))
        out[code] = name
    end
    return out
end

frame_buffer.TEAM_NAME = invert(frame_buffer.TEAM)
frame_buffer.SPECIES_SHAPE_NAME = invert(frame_buffer.SPECIES_SHAPE)
frame_buffer.CHARGE_KIND_NAME = invert(frame_buffer.CHARGE_KIND)
frame_buffer.POSE_SOURCE_NAME = invert(frame_buffer.POSE_SOURCE)
frame_buffer.AERIAL_STYLE_NAME = invert(frame_buffer.AERIAL_STYLE)
frame_buffer.AERIAL_OUTCOME_NAME = invert(frame_buffer.AERIAL_OUTCOME)
frame_buffer.SAVE_STYLE_NAME = invert(frame_buffer.SAVE_STYLE)
frame_buffer.KEEPER_STATE_NAME = invert(frame_buffer.KEEPER_STATE)
frame_buffer.SHOT_TYPE_NAME = invert(frame_buffer.SHOT_TYPE)
frame_buffer.POSE_ID_NAME = invert(frame_buffer.POSE_ID)
frame_buffer.EVENT_KIND_NAME = invert(frame_buffer.EVENT_KIND)

---@param value boolean?
---@return number
local function boolean_word(value)
    return value and 1 or 0
end

-- A closed-set member becomes its code; `nil` becomes 0. An unknown member is a
-- programmer error -- the alias grew and this numbering did not -- so it fails
-- loudly rather than encoding as absent, which would look like a sparse hole.
---@param codes table<string, integer>
---@param value string?
---@param what string
---@return number
local function enum_word(codes, value, what)
    if value == nil then
        return 0
    end
    local code = codes[value]
    assert(code ~= nil, ("unmapped %s %q; bump LAYOUT_VERSION"):format(what, tostring(value)))
    return code
end

---@param names table<integer, string>
---@param word number
---@param what string
---@return string?
local function enum_value(names, word, what)
    if word == 0 then
        return nil
    end
    local name = names[word]
    assert(name ~= nil, ("unknown %s code %s"):format(what, tostring(word)))
    return name
end

---@param frame RenderFrame
---@param words number[]
---@param at integer -- 1-based index of the first scalar word.
local function write_scalars(frame, words, at)
    local field = frame.field
    local ball = frame.ball
    local possession = frame.possession
    local control = frame.control
    local hud = frame.hud
    local color = hud.species_color
    local values = {
        field.w,
        field.h,
        field.crossbar_h,
        field.penalty_box_depth,
        field.penalty_box_h,
        field.goal_home.x,
        field.goal_home.y,
        field.goal_home.w,
        field.goal_home.h,
        field.goal_away.x,
        field.goal_away.y,
        field.goal_away.w,
        field.goal_away.h,
        ball.x,
        ball.y,
        ball.z,
        ball.vx,
        ball.vy,
        ball.vz,
        boolean_word(ball.visible),
        boolean_word(ball.landing_x ~= nil),
        ball.landing_x or 0,
        ball.landing_y or 0,
        possession.owner or 0,
        enum_word(frame_buffer.TEAM, possession.owner_team, "team"),
        boolean_word(possession.keeper_holds),
        control.controlled,
        control.pass_target or 0,
        enum_word(frame_buffer.CHARGE_KIND, control.charge_kind, "charge kind"),
        control.charge,
        hud.home_score,
        hud.away_score,
        hud.time_left,
        boolean_word(hud.finished),
        enum_word(frame_buffer.TEAM, hud.possession_team, "team"),
        hud.controlled,
        enum_word(frame_buffer.TEAM, hud.controlled_team, "team"),
        boolean_word(hud.controlled_is_keeper),
        boolean_word(hud.controlled_owns_ball),
        hud.controlled_stamina,
        enum_word(frame_buffer.SPECIES_SHAPE, hud.species_shape, "species shape"),
        color[1],
        color[2],
        color[3],
    }
    assert(
        #values == #frame_buffer.SCALAR_FIELDS,
        "the scalar writer and SCALAR_FIELDS disagree on length"
    )
    for index = 1, #values do
        words[at + index - 1] = values[index]
    end
end

-- Field-major fill of one structure-of-arrays section. `read` answers one
-- field for one entity; a nil answer is the caller's business, not this one's.
---@param words number[]
---@param at integer -- 1-based index of the first word of the section.
---@param fields string[]
---@param count integer
---@param read fun(field: string, index: integer): number
local function write_soa(words, at, fields, count, read)
    local cursor = at
    for _, name in ipairs(fields) do
        for index = 1, count do
            words[cursor] = read(name, index)
            cursor = cursor + 1
        end
    end
end

---@param players RenderFramePlayers
---@param name string
---@param index integer
---@return number
local function player_word(players, name, index)
    if name == "pose_id" then
        return enum_word(frame_buffer.POSE_ID, players.pose_id[index], "pose id")
    elseif name == "pose_source" then
        return enum_word(frame_buffer.POSE_SOURCE, players.pose_source[index], "pose source")
    elseif name == "aerial_style" then
        return enum_word(frame_buffer.AERIAL_STYLE, players.aerial_style[index], "aerial style")
    elseif name == "aerial_outcome" then
        return enum_word(
            frame_buffer.AERIAL_OUTCOME,
            players.aerial_outcome[index],
            "aerial outcome"
        )
    elseif name == "controlled" or name == "dashing" or name == "holding" then
        return boolean_word(players[name][index])
    end
    return players[name][index]
end

---@param events RenderFrameEvents
---@param name string
---@param index integer
---@return number
local function event_word(events, name, index)
    if name == "kind" then
        return enum_word(frame_buffer.EVENT_KIND, events.kind[index], "event kind")
    elseif name == "slot" then
        return events.slot[index] or 0
    elseif name == "save_style" then
        return enum_word(frame_buffer.SAVE_STYLE, events.save_style[index], "save style")
    elseif name == "style" then
        return enum_word(frame_buffer.AERIAL_STYLE, events.style[index], "aerial style")
    elseif name == "outcome" then
        return enum_word(frame_buffer.AERIAL_OUTCOME, events.outcome[index], "aerial outcome")
    elseif name == "shot_type" then
        return enum_word(frame_buffer.SHOT_TYPE, events.shot_type[index], "shot type")
    elseif name == "keeper_state" then
        return enum_word(frame_buffer.KEEPER_STATE, events.keeper_state[index], "keeper state")
    elseif name == "has_difficulty" then
        return boolean_word(events.difficulty[index] ~= nil)
    elseif name == "difficulty" then
        return events.difficulty[index] or 0
    elseif name == "has_keeper_depth" then
        return boolean_word(events.keeper_depth[index] ~= nil)
    elseif name == "keeper_depth" then
        return events.keeper_depth[index] or 0
    end
    return events[name][index]
end

-- Exact word count of a frame block. A reader derives its view length from the
-- header instead, but the producer needs it up front to truncate a reused array.
---@param count integer -- Roster slots.
---@param event_count integer
---@return integer
function frame_buffer.frame_words(count, event_count)
    return frame_buffer.HEADER_WORDS
        + #frame_buffer.SCALAR_FIELDS
        + #frame_buffer.PLAYER_FIELDS * count
        + #frame_buffer.EVENT_FIELDS * event_count
end

-- Serialise one whole frame into one flat array of doubles.
--
-- The array is the caller's to reuse: `into` is filled in place and returned,
-- so a per-frame producer allocates once and never again. It is truncated to
-- the frame's exact length, because a reader sizes its view off `total_words`
-- and a stale tail would be indistinguishable from live data if it did not.
---@param frame RenderFrame
---@param into number[]? -- Reused output array; a fresh one is made if absent.
---@return number[] words
function frame_buffer.encode(frame, into)
    -- The read-side assertion `render/frame.lua` deferred to its first
    -- cross-boundary consumer. A frame stamped with another protocol version
    -- has fields this layout would read positionally and get wrong.
    assert(
        frame.version == render_frame.VERSION,
        ("render frame version %s; expected %s"):format(
            tostring(frame.version),
            tostring(render_frame.VERSION)
        )
    )
    assert(
        frame.roster.version == render_frame.VERSION,
        ("render roster version %s; expected %s"):format(
            tostring(frame.roster.version),
            tostring(render_frame.VERSION)
        )
    )

    local players = frame.players
    local events = frame.events
    local count = players.count
    local event_count = events.count
    local total = frame_buffer.frame_words(count, event_count)

    local words = into or {}
    words[1] = frame_buffer.MAGIC
    words[2] = frame_buffer.LAYOUT_VERSION
    words[3] = render_frame.VERSION
    words[4] = total
    words[5] = frame_buffer.HEADER_WORDS
    words[6] = #frame_buffer.SCALAR_FIELDS
    words[7] = count
    words[8] = #frame_buffer.PLAYER_FIELDS
    words[9] = event_count
    words[10] = #frame_buffer.EVENT_FIELDS
    words[11] = boolean_word(frame.combat ~= nil)
    words[12] = 0

    local scalars_at = frame_buffer.HEADER_WORDS + 1
    write_scalars(frame, words, scalars_at)

    local players_at = scalars_at + #frame_buffer.SCALAR_FIELDS
    write_soa(words, players_at, frame_buffer.PLAYER_FIELDS, count, function(name, index)
        return player_word(players, name, index)
    end)

    local events_at = players_at + #frame_buffer.PLAYER_FIELDS * count
    write_soa(words, events_at, frame_buffer.EVENT_FIELDS, event_count, function(name, index)
        return event_word(events, name, index)
    end)

    for index = total + 1, #words do
        words[index] = nil
    end
    return words
end

-- Serialise the match-constant roster. Two returns because it has two halves
-- with different shapes: a numeric block and the id/name strings. Both cross
-- once per match, which is why the strings are affordable here and nowhere else.
--
-- The blob is newline-joined `id`, `name`, `id`, `name`, ... in slot order.
-- Newlines in either would make it unparseable, so they are rejected rather
-- than escaped: no authored id or presentation name contains one, and a scheme
-- with no escapes has no escaping bug.
---@param roster RenderFrameRoster
---@return number[] words
---@return string strings
function frame_buffer.encode_roster(roster)
    assert(
        roster.version == render_frame.VERSION,
        ("render roster version %s; expected %s"):format(
            tostring(roster.version),
            tostring(render_frame.VERSION)
        )
    )

    local count = roster.count
    local fields = frame_buffer.ROSTER_FIELDS
    local total = frame_buffer.ROSTER_HEADER_WORDS + #fields * count

    local words = {}
    words[1] = frame_buffer.ROSTER_MAGIC
    words[2] = frame_buffer.LAYOUT_VERSION
    words[3] = render_frame.VERSION
    words[4] = total
    words[5] = frame_buffer.ROSTER_HEADER_WORDS
    words[6] = count
    words[7] = #fields

    write_soa(words, frame_buffer.ROSTER_HEADER_WORDS + 1, fields, count, function(name, index)
        if name == "team" then
            return enum_word(frame_buffer.TEAM, roster.teams[index], "team")
        elseif name == "is_keeper" then
            return boolean_word(roster.is_keeper[index])
        elseif name == "radius" then
            return roster.radius[index]
        elseif name == "species_shape" then
            return enum_word(frame_buffer.SPECIES_SHAPE, roster.species_shape[index], "shape")
        end
        local channel = name == "species_color_r" and 1 or (name == "species_color_g" and 2 or 3)
        return roster.species_color[index][channel]
    end)

    local parts = {}
    for index = 1, count do
        local id, name = roster.ids[index], roster.names[index]
        assert(not id:find("\n", 1, true), "roster id must not contain a newline: " .. id)
        assert(not name:find("\n", 1, true), "roster name must not contain a newline: " .. name)
        parts[#parts + 1] = id
        parts[#parts + 1] = name
    end
    return words, table.concat(parts, "\n")
end

-- Decoded shapes. Deliberately NOT `RenderFrame`: a decoded frame has integer
-- roster slots where the payload had id strings, and no `combat`. Typing it as
-- the thing it is keeps a reader from assuming the round trip is lossless.
---@class DecodedRenderFrame
---@field layout_version integer
---@field render_frame_version integer
---@field combat_present boolean
---@field scalars table<string, number>
---@field players table<string, number[]>
---@field events table<string, number[]>

---@class DecodedRenderFrameRoster
---@field layout_version integer
---@field render_frame_version integer
---@field count integer
---@field ids string[]
---@field names string[]
---@field fields table<string, number[]>

---@param words number[]
---@param at integer
---@param fields string[]
---@param count integer
---@return table<string, number[]>
local function read_soa(words, at, fields, count)
    local out = {}
    local cursor = at
    for _, name in ipairs(fields) do
        local column = {}
        for index = 1, count do
            column[index] = words[cursor]
            cursor = cursor + 1
        end
        out[name] = column
    end
    return out
end

-- Read a block back. This is the Lua mirror of `wasm/sim-host/frame_payload.js`
-- and asserts exactly what that reader asserts, so a layout change that would
-- break the JS reader breaks the Lua spec suite first.
---@param words number[]
---@return DecodedRenderFrame
function frame_buffer.decode(words)
    assert(
        words[1] == frame_buffer.MAGIC,
        ("not a render frame block: magic %s"):format(tostring(words[1]))
    )
    assert(
        words[2] == frame_buffer.LAYOUT_VERSION,
        ("frame layout version %s; expected %s"):format(
            tostring(words[2]),
            tostring(frame_buffer.LAYOUT_VERSION)
        )
    )
    assert(
        words[3] == render_frame.VERSION,
        ("render frame version %s; expected %s"):format(
            tostring(words[3]),
            tostring(render_frame.VERSION)
        )
    )
    assert(
        words[5] == frame_buffer.HEADER_WORDS,
        ("header words %s; expected %s"):format(
            tostring(words[5]),
            tostring(frame_buffer.HEADER_WORDS)
        )
    )
    assert(
        words[6] == #frame_buffer.SCALAR_FIELDS,
        ("scalar words %s; expected %s"):format(
            tostring(words[6]),
            tostring(#frame_buffer.SCALAR_FIELDS)
        )
    )
    assert(
        words[8] == #frame_buffer.PLAYER_FIELDS,
        ("player field count %s; expected %s"):format(
            tostring(words[8]),
            tostring(#frame_buffer.PLAYER_FIELDS)
        )
    )
    assert(
        words[10] == #frame_buffer.EVENT_FIELDS,
        ("event field count %s; expected %s"):format(
            tostring(words[10]),
            tostring(#frame_buffer.EVENT_FIELDS)
        )
    )

    -- Header words are doubles on the wire; they are counts here.
    local count = words[7] --[[@as integer]]
    local event_count = words[9] --[[@as integer]]
    local total = frame_buffer.frame_words(count, event_count)
    assert(words[4] == total, ("total words %s; expected %s"):format(tostring(words[4]), total))

    local scalars_at = frame_buffer.HEADER_WORDS + 1
    local scalars = {}
    for index, name in ipairs(frame_buffer.SCALAR_FIELDS) do
        scalars[name] = words[scalars_at + index - 1]
    end

    local players_at = scalars_at + #frame_buffer.SCALAR_FIELDS
    local events_at = players_at + #frame_buffer.PLAYER_FIELDS * count
    return {
        layout_version = words[2],
        render_frame_version = words[3],
        combat_present = words[11] == 1,
        scalars = scalars,
        players = read_soa(words, players_at, frame_buffer.PLAYER_FIELDS, count),
        events = read_soa(words, events_at, frame_buffer.EVENT_FIELDS, event_count),
    }
end

---@param words number[]
---@param strings string
---@return DecodedRenderFrameRoster
function frame_buffer.decode_roster(words, strings)
    assert(
        words[1] == frame_buffer.ROSTER_MAGIC,
        ("not a render roster block: magic %s"):format(tostring(words[1]))
    )
    assert(
        words[2] == frame_buffer.LAYOUT_VERSION,
        ("roster layout version %s; expected %s"):format(
            tostring(words[2]),
            tostring(frame_buffer.LAYOUT_VERSION)
        )
    )
    assert(
        words[3] == render_frame.VERSION,
        ("render frame version %s; expected %s"):format(
            tostring(words[3]),
            tostring(render_frame.VERSION)
        )
    )
    assert(
        words[7] == #frame_buffer.ROSTER_FIELDS,
        ("roster field count %s; expected %s"):format(
            tostring(words[7]),
            tostring(#frame_buffer.ROSTER_FIELDS)
        )
    )

    local count = words[6] --[[@as integer]]
    local parts = {}
    for part in (strings .. "\n"):gmatch("([^\n]*)\n") do
        parts[#parts + 1] = part
    end
    assert(
        #parts == count * 2,
        ("roster blob holds %d strings; expected %d"):format(#parts, count * 2)
    )

    local ids, names = {}, {}
    for index = 1, count do
        ids[index] = parts[index * 2 - 1]
        names[index] = parts[index * 2]
    end
    return {
        layout_version = words[2],
        render_frame_version = words[3],
        count = count,
        ids = ids,
        names = names,
        fields = read_soa(
            words,
            frame_buffer.ROSTER_HEADER_WORDS + 1,
            frame_buffer.ROSTER_FIELDS,
            count
        ),
    }
end

-- Convenience for readers that want the string back rather than the code.
---@param decoded DecodedRenderFrame
---@param index integer
---@return PlayerPoseId?
function frame_buffer.pose_id(decoded, index)
    return enum_value(frame_buffer.POSE_ID_NAME, decoded.players.pose_id[index], "pose id")
    --[[@as PlayerPoseId?]]
end

---@param decoded DecodedRenderFrame
---@param index integer
---@return MatchEventKind?
function frame_buffer.event_kind(decoded, index)
    return enum_value(frame_buffer.EVENT_KIND_NAME, decoded.events.kind[index], "event kind")
    --[[@as MatchEventKind?]]
end

return frame_buffer
