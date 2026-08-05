-- Capture a match as a stream of `RenderFrame` payloads, for renderers that do
-- not exist yet.
--
-- WHY THIS EXISTS (#341)
--
-- Babylon's cost to animate ten skinned characters does not depend on where the
-- poses come from. A file of captured frames drives a candidate renderer exactly
-- as a live simulation would, so the skeleton-scaling question -- the assumption
-- #330's decision rests on -- can be answered before the wasm boundary (#332)
-- lands. When it does land, the data source swaps and the rendering numbers stay
-- valid.
--
-- WHAT IS AND IS NOT IN THE FILE
--
-- Only the per-frame fields a character renderer consumes: displayed position,
-- facing, locomotion speed, the selected pose id, and the pose timers that shape
-- it. The HUD, the event batch and the combat telegraph model are deliberately
-- dropped -- they are not what a skinned-character benchmark measures, and
-- carrying them would triple the file for nothing.
--
-- The layout is FRAME-MAJOR FLAT STREAMS, not an array of frame objects. That is
-- the same reason `render/frame.lua` is structure-of-arrays: a JavaScript reader
-- gets contiguous numeric arrays it can index arithmetically instead of tens of
-- thousands of small objects to allocate and garbage-collect before the first
-- frame is drawn. Slot `s` of frame `f` lives at `(f - 1) * count + s`.
--
-- FIXTURE FAIRNESS
--
-- The fixture is #100's, field for field: same seed, same teams, same bot, same
-- fixed timestep, same 960x540 pitch. That is not tidiness -- it is what lets a
-- Babylon number sit in the same table as the native LOVE baseline in #328
-- without an asterisk. Warm-up frames are simulated and discarded so the capture
-- starts from a match in motion rather than from kickoff.
--
-- This module is pure sim + render: no `love`, no graphics, no window. It runs
-- under `love . --capture-frames` (headless, see conf.lua) and equally under a
-- bare Lua 5.1 interpreter, which is also what proves the boundary stayed clean.

local bot = require("sim.bot")
local fixed_clock = require("sim.fixed_clock")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local render_frame = require("render.frame")
local teams = require("data.teams")

---@class RenderFrameCaptureOptions
---@field seed integer?
---@field frames integer?
---@field warmup_frames integer?

-- Per-player scalar streams, each `frames * count` long, frame-major.
---@class RenderFrameCaptureStreams
---@field x number[]
---@field y number[]
---@field facing_x number[]
---@field facing_y number[]
---@field speed number[]
---@field pose integer[] -- 1-based index into `RenderFrameCapture.pose_ids`
---@field dive number[]
---@field dive_dir_x number[]
---@field dive_dir_y number[]
---@field grab number[]
---@field throw number[]
---@field windup number[]
---@field aerial number[]
---@field aerial_jump number[]
---@field flags integer[] -- bitfield; see CAPTURE_FLAG

---@class RenderFrameCaptureBall
---@field x number[]
---@field y number[]
---@field z number[]
---@field visible integer[] -- 0/1, one per frame

---@class RenderFrameCapture
---@field schema integer
---@field render_frame_version integer
---@field seed integer
---@field tick_rate integer
---@field warmup_frames integer
---@field frames integer
---@field count integer
---@field final_state_hash string
---@field pose_ids PlayerPoseId[]
---@field pose_counts integer[] -- parallel to pose_ids; how often each was selected
---@field field RenderFrameField
---@field roster RenderFrameRoster
---@field players RenderFrameCaptureStreams
---@field ball RenderFrameCaptureBall

---@class RenderFrameCaptureModule
local capture = {}

-- Bumped whenever the file layout changes. The reader asserts on it, because
-- unlike `RenderFrame` itself this payload genuinely does leave the process.
capture.SCHEMA = 1

-- #100's fixture, so the two benchmarks measure the same match.
capture.SEED = 20260803
capture.FIELD = { w = 960, h = 540 }
capture.DEFAULT_FRAMES = 1800 -- 30 s at 60 Hz
capture.DEFAULT_WARMUP_FRAMES = 300 -- 5 s at 60 Hz

-- Three booleans that would otherwise cost three full-length arrays of "0".
capture.CAPTURE_FLAG = { controlled = 1, dashing = 2, holding = 4 }

local DT = fixed_clock.TICK_SECONDS

---@param value number
---@return string
local function num(value)
    -- Three decimals is under a thousandth of a world unit on a 960x540 pitch
    -- and under a thousandth on every 0..1 timer -- far below anything a
    -- skeleton pose can express, and it roughly halves the file.
    local text = string.format("%.3f", value)
    -- Trim trailing zeros so the common integral case costs one character.
    text = text:gsub("0+$", ""):gsub("%.$", "")
    if text == "-0" or text == "" then
        return "0"
    end
    return text
end

---@param values number[]
---@return string
local function number_array(values)
    local parts = {}
    for index = 1, #values do
        parts[index] = num(values[index])
    end
    return "[" .. table.concat(parts, ",") .. "]"
end

---@param values integer[]
---@return string
local function integer_array(values)
    local parts = {}
    for index = 1, #values do
        parts[index] = string.format("%d", values[index])
    end
    return "[" .. table.concat(parts, ",") .. "]"
end

---@param value string
---@return string
local function quoted(value)
    -- The strings here are roster ids, authored names and pose ids: no control
    -- characters, so escaping quotes and backslashes is the whole job.
    return '"' .. value:gsub("\\", "\\\\"):gsub('"', '\\"') .. '"'
end

---@param values string[]
---@return string
local function string_array(values)
    local parts = {}
    for index = 1, #values do
        parts[index] = quoted(values[index])
    end
    return "[" .. table.concat(parts, ",") .. "]"
end

---@param values boolean[]
---@return string
local function boolean_array(values)
    local parts = {}
    for index = 1, #values do
        parts[index] = values[index] and "true" or "false"
    end
    return "[" .. table.concat(parts, ",") .. "]"
end

-- Run the fixture and collect the streams. Pure apart from the simulation's own
-- deterministic RNG: same options in, same table out.
---@param opts RenderFrameCaptureOptions?
---@return RenderFrameCapture
function capture.run(opts)
    opts = opts or {}
    local seed = opts.seed or capture.SEED
    local frames = opts.frames or capture.DEFAULT_FRAMES
    local warmup_frames = opts.warmup_frames or capture.DEFAULT_WARMUP_FRAMES
    assert(frames > 0, "capture needs at least one frame")
    assert(warmup_frames >= 0, "warm-up cannot be negative")

    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = capture.FIELD.w, h = capture.FIELD.h },
        human_controlled = true,
        seed = seed,
        -- Long enough that the captured window never runs into full time and
        -- starts recording a finished match standing still.
        duration = (warmup_frames + frames) * DT + 60,
    })
    local roster = render_frame.roster(state)
    local driver = bot.new({ seed = seed })
    local clock = fixed_clock.new()

    ---@type RenderFrameCaptureStreams
    local players = {
        x = {},
        y = {},
        facing_x = {},
        facing_y = {},
        speed = {},
        pose = {},
        dive = {},
        dive_dir_x = {},
        dive_dir_y = {},
        grab = {},
        throw = {},
        windup = {},
        aerial = {},
        aerial_jump = {},
        flags = {},
    }
    ---@type RenderFrameCaptureBall
    local ball = { x = {}, y = {}, z = {}, visible = {} }

    ---@type PlayerPoseId[]
    local pose_ids = {}
    ---@type table<string, integer>
    local pose_index = {}
    ---@type integer[]
    local pose_counts = {}
    local field ---@type RenderFrameField?

    local cursor = 0
    for frame_number = 1, warmup_frames + frames do
        local input = bot.input(driver, state, DT)
        fixed_clock.step(clock, input, function(_, tick_input)
            match.step(state, DT, tick_input)
            return not state.finished
        end)

        local frame = render_frame.build(state, { roster = roster })
        if frame_number > warmup_frames then
            field = field or frame.field
            for slot = 1, frame.players.count do
                cursor = cursor + 1
                local source = frame.players
                players.x[cursor] = source.x[slot]
                players.y[cursor] = source.y[slot]
                players.facing_x[cursor] = source.facing_x[slot]
                players.facing_y[cursor] = source.facing_y[slot]
                players.speed[cursor] = source.speed[slot]
                players.dive[cursor] = source.dive[slot]
                players.dive_dir_x[cursor] = source.dive_dir_x[slot]
                players.dive_dir_y[cursor] = source.dive_dir_y[slot]
                players.grab[cursor] = source.grab[slot]
                players.throw[cursor] = source.throw[slot]
                players.windup[cursor] = source.windup[slot]
                players.aerial[cursor] = source.aerial[slot]
                players.aerial_jump[cursor] = source.aerial_jump[slot]

                local id = source.pose_id[slot]
                local index = pose_index[id]
                if not index then
                    index = #pose_ids + 1
                    pose_ids[index] = id
                    pose_index[id] = index
                    pose_counts[index] = 0
                end
                pose_counts[index] = pose_counts[index] + 1
                players.pose[cursor] = index

                local flags = 0
                if source.controlled[slot] then
                    flags = flags + capture.CAPTURE_FLAG.controlled
                end
                if source.dashing[slot] then
                    flags = flags + capture.CAPTURE_FLAG.dashing
                end
                if source.holding[slot] then
                    flags = flags + capture.CAPTURE_FLAG.holding
                end
                players.flags[cursor] = flags
            end
            local index = frame_number - warmup_frames
            ball.x[index] = frame.ball.x
            ball.y[index] = frame.ball.y
            ball.z[index] = frame.ball.z
            ball.visible[index] = frame.ball.visible and 1 or 0
        end
    end

    return {
        schema = capture.SCHEMA,
        render_frame_version = render_frame.VERSION,
        seed = seed,
        tick_rate = math.floor(1 / DT + 0.5),
        warmup_frames = warmup_frames,
        frames = frames,
        count = roster.count,
        final_state_hash = match_snapshot.hash(match_snapshot.capture(state)),
        pose_ids = pose_ids,
        pose_counts = pose_counts,
        field = assert(field, "capture produced no frames"),
        roster = roster,
        players = players,
        ball = ball,
    }
end

-- JSON, hand-rolled rather than pulled in from anywhere: the shape is fixed and
-- entirely numeric, and a dependency to emit fifteen arrays would be worse than
-- the fifteen lines below.
---@param result RenderFrameCapture
---@return string
function capture.encode(result)
    local roster = result.roster
    local colors = {}
    for index = 1, roster.count do
        colors[index] = number_array(roster.species_color[index])
    end
    local rect = function(r)
        return string.format(
            '{"x":%s,"y":%s,"w":%s,"h":%s}',
            num(r.x),
            num(r.y),
            num(r.w),
            num(r.h)
        )
    end
    local out = {}
    local function put(text)
        out[#out + 1] = text
    end
    put("{")
    put(string.format('"schema":%d,', result.schema))
    put(string.format('"render_frame_version":%d,', result.render_frame_version))
    put(string.format('"seed":%d,', result.seed))
    put(string.format('"tick_rate":%d,', result.tick_rate))
    put(string.format('"warmup_frames":%d,', result.warmup_frames))
    put(string.format('"frames":%d,', result.frames))
    put(string.format('"count":%d,', result.count))
    put(string.format('"final_state_hash":%s,', quoted(result.final_state_hash)))
    put(string.format('"pose_ids":%s,', string_array(result.pose_ids)))
    put(string.format('"pose_counts":%s,', integer_array(result.pose_counts)))
    put(
        string.format(
            '"field":{"w":%s,"h":%s,"crossbar_h":%s,"penalty_box_depth":%s,'
                .. '"penalty_box_h":%s,"goal_home":%s,"goal_away":%s},',
            num(result.field.w),
            num(result.field.h),
            num(result.field.crossbar_h),
            num(result.field.penalty_box_depth),
            num(result.field.penalty_box_h),
            rect(result.field.goal_home),
            rect(result.field.goal_away)
        )
    )
    put(
        string.format(
            '"roster":{"count":%d,"ids":%s,"names":%s,"teams":%s,"is_keeper":%s,'
                .. '"radius":%s,"species_shape":%s,"species_color":[%s]},',
            roster.count,
            string_array(roster.ids),
            string_array(roster.names),
            string_array(roster.teams),
            boolean_array(roster.is_keeper),
            number_array(roster.radius),
            string_array(roster.species_shape),
            table.concat(colors, ",")
        )
    )
    local streams = {
        "x",
        "y",
        "facing_x",
        "facing_y",
        "speed",
        "dive",
        "dive_dir_x",
        "dive_dir_y",
        "grab",
        "throw",
        "windup",
        "aerial",
        "aerial_jump",
    }
    local parts = {}
    for _, name in ipairs(streams) do
        parts[#parts + 1] = string.format("%s:%s", quoted(name), number_array(result.players[name]))
    end
    parts[#parts + 1] = string.format('"pose":%s', integer_array(result.players.pose))
    parts[#parts + 1] = string.format('"flags":%s', integer_array(result.players.flags))
    put(string.format('"players":{%s},', table.concat(parts, ",")))
    put(
        string.format(
            '"ball":{"x":%s,"y":%s,"z":%s,"visible":%s}',
            number_array(result.ball.x),
            number_array(result.ball.y),
            number_array(result.ball.z),
            integer_array(result.ball.visible)
        )
    )
    put("}")
    return table.concat(out)
end

-- One pipe-delimited line, same convention as `GC_BENCH_*`, so a shell or a
-- Python runner can assert on the capture without parsing the payload it just
-- wrote. Coverage is reported, never asserted: a capture that only ever saw
-- locomotion is a thin benchmark, and the honest response is to say so rather
-- than to fail.
---@param result RenderFrameCapture
---@param path string
---@param bytes integer
---@return string
function capture.marker(result, path, bytes)
    local coverage = {}
    for index, id in ipairs(result.pose_ids) do
        coverage[index] = string.format("%s:%d", id, result.pose_counts[index])
    end
    table.sort(coverage)
    return string.format(
        "GC_CAPTURE|schema=%d|frame_version=%d|path=%s|seed=%d|frames=%d|warmup=%d"
            .. "|players=%d|bytes=%d|state_hash=%s|pose_families=%d|coverage=%s",
        result.schema,
        result.render_frame_version,
        path,
        result.seed,
        result.frames,
        result.warmup_frames,
        result.count,
        bytes,
        result.final_state_hash,
        #result.pose_ids,
        table.concat(coverage, ",")
    )
end

-- `love . --capture-frames [frames] [warmup] [path]`, and the same three
-- positional arguments under a bare interpreter.
---@param frames_arg string?
---@param warmup_arg string?
---@param path_arg string?
---@return boolean ok
function capture.main(frames_arg, warmup_arg, path_arg)
    local path = path_arg or ".bench/babylon/render_frames.json"
    local frames = tonumber(frames_arg)
    local warmup = tonumber(warmup_arg)
    local result = capture.run({
        frames = frames and math.floor(frames) or nil,
        warmup_frames = warmup and math.floor(warmup) or nil,
    })
    local encoded = capture.encode(result)
    local handle, err = io.open(path, "wb")
    if not handle then
        print("GC_CAPTURE|error|" .. tostring(err))
        return false
    end
    handle:write(encoded)
    handle:close()
    print(capture.marker(result, path, #encoded))
    return true
end

return capture
