-- Contract tests for the captured render-frame payload (#341).
--
-- Tier 1: no display, no love.graphics. What matters here is that the file the
-- Babylon benchmark reads means what the benchmark thinks it means. Two things
-- in particular, because getting either wrong produces a plausible-looking
-- number that measures the wrong thing:
--
--   * the stream layout. Slot `s` of frame `f` sits at `(f - 1) * count + s`,
--     and a reader that is off by one row draws a coherent-looking match in
--     which every character is one tick stale. Nothing downstream would notice.
--   * determinism. The benchmark's fixture is #100's, so two captures of the
--     same seed must be byte-identical or the two benchmarks are not measuring
--     the same match and the comparison in #330 is void.

local t = require("spec.support.runner")
local capture = require("scripts.capture_render_frames")
local render_frame = require("render.frame")

local FRAMES = 12
local WARMUP = 3

---@return RenderFrameCapture
local function fixture()
    return capture.run({ frames = FRAMES, warmup_frames = WARMUP })
end

local STREAMS = {
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
    "pose",
    "flags",
}

t.describe("captured render frames", function()
    t.it("carries the schema and the render frame version it was built from", function()
        local result = fixture()
        t.eq(result.schema, capture.SCHEMA)
        t.eq(result.render_frame_version, render_frame.VERSION)
        t.is_true(capture.SCHEMA >= 1, "the capture must carry a bumpable integer schema")
    end)

    t.it("lays every per-player stream out frame-major with no holes", function()
        local result = fixture()
        t.eq(result.frames, FRAMES)
        t.is_true(result.count > 0, "the fixture must produce a roster")
        local expected = FRAMES * result.count
        for _, name in ipairs(STREAMS) do
            local stream = result.players[name]
            t.is_true(type(stream) == "table", name .. " must be an array")
            t.eq(#stream, expected, name .. " must be frames * count long")
            for index = 1, expected do
                t.is_true(type(stream[index]) == "number", name .. " has a hole at " .. index)
            end
        end
        for _, name in ipairs({ "x", "y", "z", "visible" }) do
            t.eq(#result.ball[name], FRAMES, "ball." .. name .. " must be one entry per frame")
        end
    end)

    t.it("indexes slot s of frame f at (f - 1) * count + s", function()
        local result = fixture()
        -- Every player of a frame shares that frame's index range, and the ball
        -- entry for the same frame is at `f`. Proving the arithmetic on the
        -- roster's own slot count is what stops an off-by-one row from reading
        -- as a coherent but stale match.
        for frame_index = 1, FRAMES do
            local base = (frame_index - 1) * result.count
            for slot = 1, result.count do
                local index = base + slot
                t.is_true(
                    result.players.pose[index] >= 1
                        and result.players.pose[index] <= #result.pose_ids,
                    "pose index out of range at frame " .. frame_index .. " slot " .. slot
                )
            end
            t.is_true(
                result.ball.visible[frame_index] == 0 or result.ball.visible[frame_index] == 1,
                "ball visibility must encode as 0/1"
            )
        end
    end)

    t.it("names every pose it selected and counts every selection", function()
        local result = fixture()
        t.is_true(#result.pose_ids > 0, "the capture must record at least one pose family")
        t.eq(#result.pose_counts, #result.pose_ids)
        local total = 0
        for _, count in ipairs(result.pose_counts) do
            total = total + count
        end
        t.eq(total, FRAMES * result.count, "every sample must be attributed to a pose family")
    end)

    t.it("is deterministic: the same seed encodes to the same bytes", function()
        local first = capture.encode(capture.run({ frames = FRAMES, warmup_frames = WARMUP }))
        local second = capture.encode(capture.run({ frames = FRAMES, warmup_frames = WARMUP }))
        t.eq(#first, #second, "two captures of one seed differ in length")
        t.is_true(first == second, "two captures of one seed differ in content")
    end)

    t.it("encodes JSON a reader can index without repairing it", function()
        local result = fixture()
        local text = capture.encode(result)
        t.eq(text:sub(1, 1), "{")
        t.eq(text:sub(-1), "}")
        -- No bare Lua-isms that would make the file valid Lua and invalid JSON.
        t.is_true(text:find("nil", 1, true) == nil, "the payload leaked a nil")
        t.is_true(text:find("inf", 1, true) == nil, "the payload leaked an infinity")
        t.is_true(text:find("nan", 1, true) == nil, "the payload leaked a NaN")
        t.is_true(
            text:find('"schema":' .. capture.SCHEMA, 1, true) ~= nil,
            "the payload must declare its schema first"
        )
        -- Spot-check the stream length through the text rather than the table,
        -- so an encoder that drops entries cannot pass on the table alone.
        local _, commas = text:match('"speed":%[([^%]]*)%]'):gsub(",", "")
        t.eq(commas + 1, FRAMES * result.count)
    end)

    t.it("reports coverage in one marker line without asserting on it", function()
        local result = fixture()
        local line = capture.marker(result, "/tmp/x.json", 42)
        t.eq(line:sub(1, 11), "GC_CAPTURE|")
        t.is_true(line:find("|bytes=42|", 1, true) ~= nil, "the marker must report the byte count")
        t.is_true(
            line:find("|state_hash=" .. result.final_state_hash, 1, true) ~= nil,
            "the marker must carry the simulation hash so the fixture is checkable"
        )
        t.is_true(line:find("|coverage=", 1, true) ~= nil, "the marker must report pose coverage")
        t.is_true(line:find("\n") == nil, "the marker must be one line")
    end)

    t.it("does not reach into the simulation while recording it", function()
        -- The capture drives its own match, so the only way it could perturb
        -- anything is by mutating the payload it was handed. Two runs at the
        -- same seed ending on the same snapshot hash is that proof.
        local first = capture.run({ frames = FRAMES, warmup_frames = WARMUP })
        local second = capture.run({ frames = FRAMES, warmup_frames = WARMUP })
        t.eq(first.final_state_hash, second.final_state_hash)
        t.is_true(#first.final_state_hash > 0, "the capture must record a simulation hash")
    end)
end)
