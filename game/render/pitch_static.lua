-- Cached static pitch scene (#393). Everything on the pitch that cannot change
-- while a match runs -- the arena backdrop, the floor trapezoid, its glow and
-- hex tiling, the markings, the neon outline and the goals -- is drawn once
-- into two canvases and blitted per frame, instead of being re-derived through
-- a few thousand interpreter-side projection and love.graphics calls. That
-- Lua-side cost is what #393 measured dominating browser draw time, where the
-- shipped wasm runtime is a plain Lua 5.1 interpreter.
--
-- Two canvases, not one, to keep camera-shake semantics identical to the
-- direct path: the space backdrop is screen-fixed (it has never followed the
-- combat camera offset), while the projected scene shifts with it. So the
-- backdrop blits at the origin and the scene blits at the offset.
--
-- The scene canvas is composited with premultiplied-alpha blending. Content
-- rendered into a transparent canvas with LOVE's default "alphamultiply" mode
-- is stored premultiplied, and its "add" mode leaves destination alpha alone,
-- so one premultiplied blit reproduces both the alpha-blended markings and the
-- additive floor glow exactly as the direct path layers them.
--
-- BUILD DISCIPLINE: canvases are never built while another canvas pass is
-- active. pitch.draw runs inside bloom's scene pass, and rebinding that pass
-- after a setCanvas requires re-attaching its depth buffer, which
-- love.graphics.getCanvas cannot return -- get it wrong and the rigged
-- renderer depth-tests into nothing (game/render/benchmark.lua documents that
-- exact bug). So a cache miss inside the pass falls back to direct drawing and
-- records a pending build; the screen calls pitch_static.flush() after the
-- bloom pass, where no canvas is bound, and the next frame blits.
--
-- Defensive like bloom.lua: a failed canvas build latches state.failed, prints
-- one diagnostic, and the module degrades permanently to direct drawing.

local camera = require("game.render.camera")
local arena_render = require("game.render.arena")

local pitch_static = {}

-- Runtime lever: benchmark `nocache` mode and any emergency fallback. Off
-- means draw_cached refuses immediately and the direct path runs every frame.
pitch_static.enabled = true

local HEX_RADIUS = 26 -- world units, centre to corner
local NET_BACK_FRAC = 0.55 -- back frame height as a fraction of the crossbar

-- The static subset of PitchDrawOptions: per-match theming only, nothing that
-- changes per frame (arena_pulse and camera_offset stay outside the cache).
---@class PitchStaticOptions
---@field arena ArenaData
---@field home_color number[]
---@field away_color number[]

---@class PitchStaticPending
---@field key string
---@field field RenderFrameField
---@field vp { w: number, h: number }
---@field opts PitchStaticOptions

local state = {
    failed = false,
    ---@type string?
    key = nil,
    ---@type love.Canvas?
    backdrop = nil,
    ---@type love.Canvas?
    scene = nil,
    canvas_w = 0,
    canvas_h = 0,
    ---@type PitchStaticPending?
    pending = nil,
}

-- Screen-space mesh shader for the goal nets. Lazily created and fully
-- optional: headless tests stub love.graphics without newShader, and a failed
-- compile just falls back to a plain translucent fill.
local net_shader = nil
local net_shader_tried = false
local function get_net_shader()
    if not net_shader_tried then
        net_shader_tried = true
        if love.graphics.newShader then
            local ok, sh = pcall(
                love.graphics.newShader,
                [[
                extern float spacing;
                vec4 effect(vec4 color, Image tex, vec2 tc, vec2 sc) {
                    vec2 g = mod(sc, vec2(spacing));
                    float mesh = min(g.x, g.y);
                    float line = 1.0 - smoothstep(0.0, 1.6, mesh);
                    return vec4(color.rgb, color.a * (0.18 + 0.82 * line));
                }
            ]]
            )
            net_shader = ok and sh or nil
        end
    end
    return net_shader
end

---@param c number[]
---@param a number?
local function set(c, a)
    love.graphics.setColor(c[1], c[2], c[3], a or 1)
end

-- Build the screen-space points of a world-space circle (each sample projected).
---@param project fun(wx: number, wy: number): number, number, number
---@param cx number
---@param cy number
---@param r number
---@param segs integer
---@return number[]
local function projected_circle(project, cx, cy, r, segs)
    local pts = {}
    for i = 0, segs do
        local ang = (i / segs) * 2 * math.pi
        local sx, sy = project(cx + r * math.cos(ang), cy + r * math.sin(ang))
        pts[#pts + 1] = sx
        pts[#pts + 1] = sy
    end
    return pts
end

-- Soft additive luminance toward the pitch centre so the floor reads as lit.
---@param project fun(wx: number, wy: number): number, number, number
---@param field RenderFrameField
local function draw_floor_glow(project, field)
    local cx, cy = project(field.w / 2, field.h / 2)
    love.graphics.setBlendMode("add")
    for i = 4, 1, -1 do
        set({ 0.05, 0.16, 0.20 }, 0.06)
        love.graphics.ellipse("fill", cx, cy, 130 * i, 64 * i)
    end
    love.graphics.setBlendMode("alpha")
end

-- Bright, blooming pitch markings: halfway line + circle + spot, and goal boxes.
---@param project fun(wx: number, wy: number): number, number, number
---@param field RenderFrameField
local function draw_markings(project, field)
    love.graphics.setLineWidth(2)
    set({ 0.35, 0.72, 1.0 }, 0.85)

    local x1, y1 = project(field.w / 2, 0)
    local x2, y2 = project(field.w / 2, field.h)
    love.graphics.line(x1, y1, x2, y2)

    love.graphics.polygon("line", projected_circle(project, field.w / 2, field.h / 2, 70, 36))

    local sx, sy = project(field.w / 2, field.h / 2)
    love.graphics.circle("fill", sx, sy, 3)

    local depth, box_h = field.penalty_box_depth, field.penalty_box_h
    local top, bot = field.h / 2 - box_h / 2, field.h / 2 + box_h / 2
    ---@param xa number
    ---@param xb number
    local function box(xa, xb)
        local p1x, p1y = project(xa, top)
        local p2x, p2y = project(xb, top)
        local p3x, p3y = project(xb, bot)
        local p4x, p4y = project(xa, bot)
        love.graphics.polygon("line", p1x, p1y, p2x, p2y, p3x, p3y, p4x, p4y)
    end
    box(0, depth)
    box(field.w - depth, field.w)

    love.graphics.setLineWidth(1)
end

-- Draw a pointy-top hex tiling over the pitch, projected per-corner so the cells
-- follow the perspective. Corners are clamped to the field so edge cells meet the
-- touchlines instead of spilling onto the space backdrop.
---@param project fun(wx: number, wy: number): number, number, number
---@param field RenderFrameField
local function draw_hex_floor(project, field)
    local r = HEX_RADIUS
    local col_step = math.sqrt(3) * r
    local row_step = 1.5 * r

    set({ 0.16, 0.5, 0.6 }, 0.1)
    local row, cy = 0, 0
    while cy <= field.h + r do
        local x_off = (row % 2 == 1) and (col_step / 2) or 0
        local cx = x_off
        while cx <= field.w + r do
            local pts = {}
            for i = 0, 5 do
                local ang = math.rad(60 * i - 30)
                local wx = math.min(field.w, math.max(0, cx + r * math.cos(ang)))
                local wy = math.min(field.h, math.max(0, cy + r * math.sin(ang)))
                local sx, sy = project(wx, wy)
                pts[#pts + 1] = sx
                pts[#pts + 1] = sy
            end
            love.graphics.polygon("line", pts)
            cx = cx + col_step
        end
        row = row + 1
        cy = cy + row_step
    end
end

-- One goal: side/back/roof netting (screen-space mesh shader) inside a frame
-- of two posts and a crossbar. `line_x` is the goal-line plane, `back_x` the
-- net's back.
---@param project fun(wx: number, wy: number): number, number, number
---@param field RenderFrameField
---@param g Rect
---@param color number[]
---@param line_x number
---@param back_x number
local function draw_goal(project, field, g, color, line_x, back_x)
    local bar = field.crossbar_h
    local lfx, lfy, lfs = project(line_x, g.y) -- far post base (on the line)
    local lnx, lny, lns = project(line_x, g.y + g.h) -- near post base
    local bfx, bfy, bfs = project(back_x, g.y) -- back frame, far
    local bnx, bny, bns = project(back_x, g.y + g.h) -- back frame, near
    local back_h = bar * NET_BACK_FRAC

    local shader = get_net_shader()
    if shader then
        shader:send("spacing", 7)
        love.graphics.setShader(shader)
    end
    set(color, 0.30)
    -- Side nets: raked from full height at the posts down to the low back.
    love.graphics.polygon("fill", lfx, lfy, bfx, bfy, bfx, bfy - back_h * bfs, lfx, lfy - bar * lfs)
    love.graphics.polygon("fill", lnx, lny, bnx, bny, bnx, bny - back_h * bns, lnx, lny - bar * lns)
    -- Back net.
    love.graphics.polygon(
        "fill",
        bfx,
        bfy,
        bnx,
        bny,
        bnx,
        bny - back_h * bns,
        bfx,
        bfy - back_h * bfs
    )
    -- Roof net: crossbar down to the back frame.
    set(color, 0.22)
    love.graphics.polygon(
        "fill",
        lfx,
        lfy - bar * lfs,
        lnx,
        lny - bar * lns,
        bnx,
        bny - back_h * bns,
        bfx,
        bfy - back_h * bfs
    )
    if shader then
        love.graphics.setShader()
    end

    -- The frame: two posts + crossbar, bright so the bloom pass lights it.
    love.graphics.setLineWidth(3)
    set({ 0.92, 0.97, 1.0 }, 0.95)
    love.graphics.line(lfx, lfy, lfx, lfy - bar * lfs)
    love.graphics.line(lnx, lny, lnx, lny - bar * lns)
    love.graphics.line(lfx, lfy - bar * lfs, lnx, lny - bar * lns)
    -- Back frame, thinner and dimmer.
    love.graphics.setLineWidth(1)
    set({ 0.7, 0.85, 1.0 }, 0.5)
    love.graphics.line(bfx, bfy, bfx, bfy - back_h * bfs)
    love.graphics.line(bnx, bny, bnx, bny - back_h * bns)
    love.graphics.line(bfx, bfy - back_h * bfs, bnx, bny - back_h * bns)
end

-- The projected static scene, drawn through `project` into the current render
-- target: floor trapezoid, glow, hex tiling, markings, neon outline, goals.
-- The screen-fixed backdrop is NOT included -- callers layer it separately so
-- the cache can shift this layer under camera shake without moving space.
---@param project fun(wx: number, wy: number): number, number, number
---@param field RenderFrameField
---@param opts PitchStaticOptions
function pitch_static.draw_projected(project, field, opts)
    -- Pitch surface (projected trapezoid).
    local ax, ay = project(0, 0)
    local bx, by = project(field.w, 0)
    local cx, cy = project(field.w, field.h)
    local dx, dy = project(0, field.h)
    set(opts.arena.floor_color)
    love.graphics.polygon("fill", ax, ay, bx, by, cx, cy, dx, dy)

    -- Floor luminance (soft additive glow toward the centre).
    draw_floor_glow(project, field)

    -- Hex floor (faint texture).
    draw_hex_floor(project, field)

    -- Field markings (halfway line/circle/spot + goal boxes).
    draw_markings(project, field)

    -- Pitch outline (bright neon border).
    love.graphics.setLineWidth(2)
    set(opts.arena.rail_color, 0.9)
    love.graphics.polygon("line", ax, ay, bx, by, cx, cy, dx, dy)
    love.graphics.setLineWidth(1)

    -- Real goals standing behind the goal line, outside the field.
    local goal_home, goal_away = field.goal_home, field.goal_away
    draw_goal(project, field, goal_home, opts.home_color, goal_home.x + goal_home.w, goal_home.x)
    draw_goal(project, field, goal_away, opts.away_color, goal_away.x, goal_away.x + goal_away.w)
end

-- The whole static scene through the direct (uncached) path, in the exact
-- order the cached blits reproduce: backdrop first, projected scene second.
---@param project fun(wx: number, wy: number): number, number, number
---@param field RenderFrameField
---@param vp { w: number, h: number }
---@param opts PitchStaticOptions
function pitch_static.draw_scene(project, field, vp, opts)
    arena_render.draw_backdrop(opts.arena, vp)
    pitch_static.draw_projected(project, field, opts)
end

-- Everything the cached image depends on, as a comparable string. Complete on
-- purpose: a key that omitted an input would serve a stale scene rather than
-- rebuild, which is the one failure mode a cache must not have. camera state
-- is included because camera.project is the projection under the whole scene.
---@param field RenderFrameField
---@param vp { w: number, h: number }
---@param opts PitchStaticOptions
---@return string
function pitch_static.key(field, vp, opts)
    local gh, ga = field.goal_home, field.goal_away
    return string.format(
        "%g,%g|%g,%g,%g|%g,%g,%g,%g|%g,%g,%g,%g|%g,%g|%s|%s|%s|%s",
        field.w,
        field.h,
        field.penalty_box_depth,
        field.penalty_box_h,
        field.crossbar_h,
        gh.x,
        gh.y,
        gh.w,
        gh.h,
        ga.x,
        ga.y,
        ga.w,
        ga.h,
        vp.w,
        vp.h,
        tostring(opts.arena),
        table.concat(opts.home_color, ","),
        table.concat(opts.away_color, ","),
        tostring(camera.perspective_mode)
    )
end

---@return boolean
local function canvas_support()
    local g = love.graphics
    return (g.newCanvas and g.setCanvas and g.draw and g.clear and g.push and g.pop) and true
        or false
end

-- Blit the cached static scene if it is valid for these inputs. Returns false
-- (and records a pending build for flush()) when it is not; the caller must
-- then draw the scene directly. Never touches setCanvas -- see the header.
---@param field RenderFrameField
---@param vp { w: number, h: number }
---@param opts PitchStaticOptions
---@param offset { x: number, y: number }?  -- combat camera shake, applied at blit
---@return boolean drew
function pitch_static.draw_cached(field, vp, opts, offset)
    if not pitch_static.enabled or state.failed or not canvas_support() then
        return false
    end
    local key = pitch_static.key(field, vp, opts)
    if state.key ~= key or not state.backdrop or not state.scene then
        -- Snapshot the inputs rather than borrowing the frame's tables: the
        -- render frame is rebuilt every tick, but the build runs later, in
        -- flush(). Arena and colour tables are match-constant data, so those
        -- references are safe to hold.
        state.pending = {
            key = key,
            field = {
                w = field.w,
                h = field.h,
                penalty_box_depth = field.penalty_box_depth,
                penalty_box_h = field.penalty_box_h,
                crossbar_h = field.crossbar_h,
                goal_home = {
                    x = field.goal_home.x,
                    y = field.goal_home.y,
                    w = field.goal_home.w,
                    h = field.goal_home.h,
                },
                goal_away = {
                    x = field.goal_away.x,
                    y = field.goal_away.y,
                    w = field.goal_away.w,
                    h = field.goal_away.h,
                },
            },
            vp = { w = vp.w, h = vp.h },
            opts = {
                arena = opts.arena,
                home_color = opts.home_color,
                away_color = opts.away_color,
            },
        }
        return false
    end

    love.graphics.setColor(1, 1, 1, 1)
    love.graphics.draw(state.backdrop, 0, 0)
    love.graphics.setBlendMode("alpha", "premultiplied")
    love.graphics.draw(state.scene, offset and offset.x or 0, offset and offset.y or 0)
    love.graphics.setBlendMode("alpha")
    return true
end

-- Build any pending cache. MUST be called outside every canvas pass (screens
-- call it after bloom.draw returns): it binds its own canvases and restores
-- the default render target, which would silently detach a caller's depth
-- buffer if one were active.
function pitch_static.flush()
    local pending = state.pending
    if not pending or state.failed or not pitch_static.enabled then
        return
    end
    state.pending = nil
    local g = love.graphics
    local ok, err = pcall(function()
        local w = math.max(1, math.floor(pending.vp.w))
        local h = math.max(1, math.floor(pending.vp.h))
        if not state.backdrop or state.canvas_w ~= w or state.canvas_h ~= h then
            state.backdrop = g.newCanvas(w, h)
            state.scene = g.newCanvas(w, h)
            state.canvas_w, state.canvas_h = w, h
        end
        local field, vp, opts = pending.field, pending.vp, pending.opts
        -- The build projection: no follow view and no camera offset. The
        -- offset is applied at blit time; a follow view bypasses the cache
        -- entirely (pitch.draw never calls draw_cached with one active).
        local function project(wx, wy)
            return camera.project(wx, wy, field, vp)
        end
        g.push("all")
        g.setCanvas(state.backdrop)
        g.clear(0, 0, 0, 1)
        arena_render.draw_backdrop(opts.arena, vp)
        g.setCanvas(state.scene)
        g.clear(0, 0, 0, 0)
        pitch_static.draw_projected(project, field, opts)
        g.setCanvas()
        g.pop()
    end)
    if not ok then
        state.failed = true
        state.key, state.backdrop, state.scene = nil, nil, nil
        state.canvas_w, state.canvas_h = 0, 0
        print("static pitch cache disabled (build failed): " .. tostring(err))
        return
    end
    state.key = pending.key
end

-- Drop the cache and the failure latch. For tests and renderer reconfiguration
-- (the benchmark calls it so cache-off passes cannot blit a stale canvas).
function pitch_static.reset()
    state.failed = false
    state.key = nil
    state.backdrop = nil
    state.scene = nil
    state.canvas_w, state.canvas_h = 0, 0
    state.pending = nil
end

-- Introspection for tests and the benchmark's evidence line: whether a blit
-- can happen next frame, and whether the module has latched failure.
---@return { cached: boolean, pending: boolean, failed: boolean }
function pitch_static.status()
    return {
        cached = (state.key ~= nil and state.backdrop ~= nil and state.scene ~= nil),
        pending = state.pending ~= nil,
        failed = state.failed,
    }
end

return pitch_static
