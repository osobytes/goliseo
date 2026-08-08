-- Tier-1/2 tests for the #393 static-scene cache: key completeness, the
-- miss -> pending -> flush -> blit lifecycle, the failure latch, and the
-- camera-shake blit offset. All headless: love.graphics is a counting stub,
-- so these prove the cache logic, not the pixels (the visual snapshot scripts
-- own those).

local t = require("spec.support.runner")
local pitch_static = require("game.render.pitch_static")

---@return RenderFrameField
local function make_field()
    return {
        w = 960,
        h = 540,
        penalty_box_depth = 110,
        penalty_box_h = 260,
        crossbar_h = 40,
        goal_home = { x = -28, y = 220, w = 28, h = 100 },
        goal_away = { x = 960, y = 220, w = 28, h = 100 },
    }
end

---@type ArenaData
local ARENA = {
    id = "spec_arena",
    name = "spec",
    location = "spec orbit",
    floor_color = { 0.1, 0.2, 0.3 },
    rail_color = { 0.2, 0.9, 1.0 },
    marking_color = { 0.3, 0.7, 1.0 },
    highlight_color = { 0.9, 0.6, 0.2 },
}

---@type ArenaData
local OTHER_ARENA = {
    id = "spec_other",
    name = "other",
    location = "spec orbit b",
    floor_color = { 0, 0, 0 },
    rail_color = { 1, 1, 1 },
    marking_color = { 1, 1, 1 },
    highlight_color = { 1, 1, 1 },
}

---@return PitchStaticOptions
local function make_opts()
    return {
        arena = ARENA,
        home_color = { 1, 0.2, 0.2 },
        away_color = { 0.2, 0.2, 1 },
    }
end

local VP = { w = 960, h = 540 }

-- A love.graphics stub with canvas support. Records what the cache does with
-- it: canvases created, blits issued (with positions), current render target.
local function stub_graphics()
    local g = {}
    local noop = function() end
    for _, name in ipairs({
        "setColor",
        "setLineWidth",
        "setBlendMode",
        "rectangle",
        "polygon",
        "line",
        "circle",
        "ellipse",
        "push",
        "pop",
    }) do
        g[name] = noop
    end
    g.calls = { new_canvas = 0, blits = {}, set_canvas = {} }
    g.newCanvas = function(w, h)
        g.calls.new_canvas = g.calls.new_canvas + 1
        return { canvas = true, w = w, h = h }
    end
    g.setCanvas = function(canvas)
        g.calls.set_canvas[#g.calls.set_canvas + 1] = canvas or "none"
    end
    g.clear = noop
    g.draw = function(canvas, x, y)
        g.calls.blits[#g.calls.blits + 1] = { canvas = canvas, x = x, y = y }
    end
    return g
end

---@param fn fun(g: table)
local function with_stub(fn)
    local saved = love.graphics
    local g = stub_graphics()
    love.graphics = g
    pitch_static.reset()
    pitch_static.enabled = true
    local ok, err = pcall(fn, g)
    pitch_static.reset()
    love.graphics = saved
    if not ok then
        error(err, 0)
    end
end

t.describe("pitch_static cache", function()
    t.it("keys every input the cached image depends on", function()
        local base = pitch_static.key(make_field(), VP, make_opts())
        t.eq(pitch_static.key(make_field(), VP, make_opts()), base, "same inputs, same key")

        -- One mutation per component pitch_static.key() serializes. A
        -- component missing here could go stale without any test noticing,
        -- which is the one failure mode a cache must not have.
        ---@type table<string, fun(field: RenderFrameField, vp: { w: number, h: number }, opts: PitchStaticOptions)>
        local mutations = {
            ["field.w"] = function(field)
                field.w = 961
            end,
            ["field.h"] = function(field)
                field.h = 541
            end,
            ["field.penalty_box_depth"] = function(field)
                field.penalty_box_depth = 111
            end,
            ["field.penalty_box_h"] = function(field)
                field.penalty_box_h = 261
            end,
            ["field.crossbar_h"] = function(field)
                field.crossbar_h = 41
            end,
            ["goal_home.x"] = function(field)
                field.goal_home.x = -29
            end,
            ["goal_home.y"] = function(field)
                field.goal_home.y = 221
            end,
            ["goal_home.w"] = function(field)
                field.goal_home.w = 29
            end,
            ["goal_home.h"] = function(field)
                field.goal_home.h = 101
            end,
            ["goal_away.x"] = function(field)
                field.goal_away.x = 961
            end,
            ["goal_away.y"] = function(field)
                field.goal_away.y = 221
            end,
            ["goal_away.w"] = function(field)
                field.goal_away.w = 29
            end,
            ["goal_away.h"] = function(field)
                field.goal_away.h = 101
            end,
            ["vp.w"] = function(_, vp)
                vp.w = 1280
            end,
            ["vp.h"] = function(_, vp)
                vp.h = 720
            end,
            ["opts.arena"] = function(_, _, opts)
                opts.arena = OTHER_ARENA
            end,
            ["opts.home_color"] = function(_, _, opts)
                opts.home_color = { 0.5, 0.5, 0.5 }
            end,
            ["opts.away_color"] = function(_, _, opts)
                opts.away_color = { 0.5, 0.5, 0.5 }
            end,
        }
        for name, mutate in pairs(mutations) do
            local field, vp, opts = make_field(), { w = VP.w, h = VP.h }, make_opts()
            mutate(field, vp, opts)
            t.is_true(pitch_static.key(field, vp, opts) ~= base, name .. " must invalidate")
        end

        -- The projection mode is global camera state rather than an argument,
        -- so it gets a save/restore rather than a mutator above.
        local camera = require("game.render.camera")
        local saved = camera.perspective_mode
        camera.perspective_mode = not saved
        local flipped = pitch_static.key(make_field(), VP, make_opts())
        camera.perspective_mode = saved
        t.is_true(flipped ~= base, "camera.perspective_mode must invalidate")
    end)

    t.it("misses, builds on flush, then blits", function()
        with_stub(function(g)
            local drew = pitch_static.draw_cached(make_field(), VP, make_opts(), nil)
            t.is_true(not drew, "first frame must miss")
            t.is_true(pitch_static.status().pending, "miss must record a pending build")
            t.eq(g.calls.new_canvas, 0, "no canvas may be created inside the frame")

            pitch_static.flush()
            t.eq(g.calls.new_canvas, 2, "flush builds backdrop + scene canvases")
            t.is_true(pitch_static.status().cached, "flush must leave a usable cache")
            t.eq(g.calls.set_canvas[#g.calls.set_canvas], "none", "flush must unbind its canvas")

            drew = pitch_static.draw_cached(make_field(), VP, make_opts(), nil)
            t.is_true(drew, "second frame must blit")
            t.eq(#g.calls.blits, 2, "one backdrop blit + one scene blit")
            t.eq(g.calls.blits[1].x, 0, "backdrop blits at the origin")

            pitch_static.flush()
            t.eq(g.calls.new_canvas, 2, "an idle flush must not rebuild")
        end)
    end)

    t.it("applies camera shake at the scene blit only", function()
        with_stub(function(g)
            pitch_static.draw_cached(make_field(), VP, make_opts(), nil)
            pitch_static.flush()
            g.calls.blits = {}
            local drew = pitch_static.draw_cached(make_field(), VP, make_opts(), { x = 7, y = -3 })
            t.is_true(drew, "offset must not defeat the cache")
            t.eq(g.calls.blits[1].x, 0, "backdrop stays screen-fixed")
            t.eq(g.calls.blits[1].y, 0, "backdrop stays screen-fixed")
            t.eq(g.calls.blits[2].x, 7, "scene shifts with the shake")
            t.eq(g.calls.blits[2].y, -3, "scene shifts with the shake")
        end)
    end)

    t.it("rebuilds when an input changes", function()
        with_stub(function(g)
            pitch_static.draw_cached(make_field(), VP, make_opts(), nil)
            pitch_static.flush()

            local other_vp = { w = 1280, h = 720 }
            local drew = pitch_static.draw_cached(make_field(), other_vp, make_opts(), nil)
            t.is_true(not drew, "a changed viewport must miss")
            pitch_static.flush()
            t.eq(g.calls.new_canvas, 4, "a new viewport needs new canvases")
            t.is_true(
                pitch_static.draw_cached(make_field(), other_vp, make_opts(), nil),
                "rebuilt cache must serve the new viewport"
            )
        end)
    end)

    t.it("draws direct when the runtime has no canvas support", function()
        local saved = love.graphics
        love.graphics = { polygon = function() end } -- no newCanvas
        pitch_static.reset()
        local ok, err = pcall(function()
            local drew = pitch_static.draw_cached(make_field(), VP, make_opts(), nil)
            t.is_true(not drew, "no canvas support must refuse the cache")
            t.is_true(not pitch_static.status().pending, "and must not queue a build")
        end)
        pitch_static.reset()
        love.graphics = saved
        t.is_true(ok, tostring(err))
    end)

    t.it("latches failure when the canvas build throws, diagnosing exactly once", function()
        with_stub(function(g)
            g.newCanvas = function()
                error("out of memory")
            end
            -- Count the diagnostic rather than silencing it: one latch, one
            -- printed line, no matter how many frames and flushes follow.
            local saved_print = print
            local diagnostics = 0
            _G.print = function(message, ...)
                if
                    type(message) == "string"
                    and message:find("static pitch cache disabled", 1, true)
                then
                    diagnostics = diagnostics + 1
                end
                return saved_print(message, ...)
            end
            local ok, err = pcall(function()
                pitch_static.draw_cached(make_field(), VP, make_opts(), nil)
                pitch_static.flush()
                t.is_true(pitch_static.status().failed, "a failed build must latch")
                t.eq(diagnostics, 1, "the latch must print its diagnostic")

                local drew = pitch_static.draw_cached(make_field(), VP, make_opts(), nil)
                t.is_true(not drew, "a latched cache must draw direct")
                t.is_true(not pitch_static.status().pending, "and must stop queueing builds")
                pitch_static.flush()
                pitch_static.flush()
                t.eq(diagnostics, 1, "repeated flushes must not repeat the diagnostic")
            end)
            _G.print = saved_print
            t.is_true(ok, tostring(err))
        end)
    end)

    t.it("drops the cache and the latch on reset", function()
        with_stub(function()
            pitch_static.draw_cached(make_field(), VP, make_opts(), nil)
            pitch_static.flush()
            t.is_true(pitch_static.status().cached, "precondition: a built cache")

            pitch_static.reset()
            local status = pitch_static.status()
            t.is_true(not status.cached, "reset must drop the canvases")
            t.is_true(not status.pending, "reset must drop any pending build")
            t.is_true(not status.failed, "reset must clear the failure latch")
        end)
    end)

    t.it("stays off when disabled", function()
        with_stub(function()
            pitch_static.enabled = false
            local drew = pitch_static.draw_cached(make_field(), VP, make_opts(), nil)
            t.is_true(not drew, "disabled cache must draw direct")
            t.is_true(not pitch_static.status().pending, "disabled cache must not queue builds")
            pitch_static.enabled = true
        end)
    end)
end)
