-- Impure 2.5D match renderer: draws a RenderFrame through the camera projection
-- as a perspective pitch with depth-sorted billboard players.
-- (Bloom/neon post-processing is a later pass; this is the geometry layer.)
--
-- This module never sees a `MatchState`. Everything it draws comes off the
-- versioned payload built by `render.frame`, which is the whole point: the same
-- payload can be handed to a renderer that is not written in Lua. The only
-- things read from outside it are renderer-owned presentation state
-- (`view_state` gait/lean, the particle systems) and per-match theming.

local camera = require("game.render.camera")
local camera_follow = require("game.render.camera_follow")
local arena_render = require("game.render.arena")
local combat_render = require("game.render.combat")
local player_renderer = require("game.render.player_renderer")
local player_renderer_3d = require("game.render.player_renderer_3d")
local view_state = require("game.render.view_state")
local effects = require("game.render.effects")
local arenas = require("data.arenas")

local pitch = {}

-- Opt-in rigged 3D players. Off by default: the procedural 2.5D renderer stays
-- the shipping path until the benchmark gate says otherwise.
pitch.rigged_players = false

-- Opt-in broadcast-style following camera. Off by default: it reframes the whole
-- match, so it stays behind a flag until it has been played.
pitch.follow_camera = false

local HEX_RADIUS = 26 -- world units, centre to corner
local NET_BACK_FRAC = 0.55 -- back frame height as a fraction of the crossbar

-- Opt-in per-phase instrumentation (#393). When a sink table is attached,
-- pitch.draw splits its cost into the static scene (backdrop, floor, markings,
-- goals -- everything that cannot change while a match runs) and the dynamic
-- rest (players, ball, effects, overlays), writing seconds into the sink every
-- call. nil (the default) costs two branches per frame and nothing else.
---@class PitchPhaseSink
---@field scene_static_s number?
---@field scene_dynamic_s number?

---@type PitchPhaseSink?
pitch.phase_sink = nil

-- Per-match theming and screen-space presentation. Deliberately NOT part of the
-- render frame: colours, arena art and the combat-feedback camera shake are
-- renderer concerns the simulation knows nothing about.
---@class PitchDrawOptions
---@field home_color number[]
---@field away_color number[]
---@field arena ArenaData?
---@field arena_pulse number?
---@field camera_offset { x: number, y: number }?

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

-- Render the whole pitch + entities for one frame.
---@param frame RenderFrame
---@param vp { w: number, h: number }
---@param opts PitchDrawOptions
function pitch.draw(frame, vp, opts)
    local field = frame.field
    local roster = frame.roster
    local players = frame.players
    local ball = frame.ball
    local arena = opts.arena or arenas.helios_crown

    -- One projection wrapper for the entire pitch: lines, goals, players,
    -- effects and the ball all go through it, so the follow window moves every
    -- one of them together and nothing has to know the camera exists.
    local view = pitch.follow_camera and camera_follow.view(field) or nil
    local function project(wx, wy)
        local sx, sy, scale = camera.project(wx, wy, field, vp, nil, view)
        local offset = opts.camera_offset
        return sx + (offset and offset.x or 0), sy + (offset and offset.y or 0), scale
    end

    -- Phase timer (#393): sink attached and a real clock available.
    local sink = pitch.phase_sink
    if sink and not (love.timer and love.timer.getTime) then
        sink = nil
    end
    local static_started = sink and love.timer.getTime() or 0

    arena_render.draw_backdrop(arena, vp)

    -- Pitch surface (projected trapezoid).
    local ax, ay = project(0, 0)
    local bx, by = project(field.w, 0)
    local cx, cy = project(field.w, field.h)
    local dx, dy = project(0, field.h)
    set(arena.floor_color)
    love.graphics.polygon("fill", ax, ay, bx, by, cx, cy, dx, dy)

    -- Floor luminance (soft additive glow toward the centre).
    draw_floor_glow(project, field)

    -- Hex floor (faint texture).
    draw_hex_floor(project, field)

    -- Field markings (halfway line/circle/spot + goal boxes).
    draw_markings(project, field)

    -- Pitch outline (bright neon border).
    love.graphics.setLineWidth(2)
    set(arena.rail_color, 0.9)
    love.graphics.polygon("line", ax, ay, bx, by, cx, cy, dx, dy)
    love.graphics.setLineWidth(1)
    arena_render.draw_frame(arena, {
        ax = ax,
        ay = ay,
        bx = bx,
        by = by,
        cx = cx,
        cy = cy,
        dx = dx,
        dy = dy,
    }, opts.arena_pulse)

    -- Real goals standing behind the goal line, outside the field: side/back/
    -- roof netting (screen-space mesh shader) inside a frame of two posts and
    -- a crossbar. `line_x` is the goal-line plane, `back_x` the net's back.
    ---@param g Rect
    ---@param color number[]
    ---@param line_x number
    ---@param back_x number
    local function draw_goal(g, color, line_x, back_x)
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
        love.graphics.polygon(
            "fill",
            lfx,
            lfy,
            bfx,
            bfy,
            bfx,
            bfy - back_h * bfs,
            lfx,
            lfy - bar * lfs
        )
        love.graphics.polygon(
            "fill",
            lnx,
            lny,
            bnx,
            bny,
            bnx,
            bny - back_h * bns,
            lnx,
            lny - bar * lns
        )
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
    local goal_home, goal_away = field.goal_home, field.goal_away
    draw_goal(goal_home, opts.home_color, goal_home.x + goal_home.w, goal_home.x)
    draw_goal(goal_away, opts.away_color, goal_away.x, goal_away.x + goal_away.w)

    -- Everything above this line is the static scene; everything below moves.
    local static_done = sink and love.timer.getTime() or 0

    -- Ball trail sits on the ground, under the entities.
    effects.draw_trail(project)
    combat_render.draw_under(frame, project)

    -- Depth-sorted drawables (far first). `index == nil` is the ball.
    local items = {}
    for index = 1, players.count do
        items[#items + 1] = { index = index, depth = players.y[index] }
    end
    items[#items + 1] = { index = nil, depth = ball.y }
    table.sort(items, function(a, b)
        return a.depth < b.depth
    end)

    for _, it in ipairs(items) do
        local index = it.index
        if index then
            local sx, sy, scale = project(players.x[index], players.y[index])
            local r = roster.radius[index] * scale
            local color = (roster.teams[index] == "home") and opts.home_color or opts.away_color
            ---@type PlayerRenderOptions
            local player_opts = {
                facing = { x = players.facing_x[index], y = players.facing_y[index] },
                is_keeper = roster.is_keeper[index],
                controlled = players.controlled[index],
                dashing = players.dashing[index],
                dive = players.dive[index],
                dive_dir = { x = players.dive_dir_x[index], y = players.dive_dir_y[index] },
                -- Keeper holding the ball: render it cradled in the hands (below).
                holding = players.holding[index],
                grab = players.grab[index],
                throw = players.throw[index],
                windup = players.windup[index],
                aerial = players.aerial[index],
                aerial_style = players.aerial_style[index],
                aerial_outcome = players.aerial_outcome[index],
                aerial_jump = players.aerial_jump[index],
                species_shape = roster.species_shape[index],
                species_color = roster.species_color[index],
                team = roster.teams[index],
                combat = frame.combat and frame.combat.players[index] or nil,
                pose = {
                    id = players.pose_id[index],
                    priority = players.pose_priority[index],
                    source = players.pose_source[index],
                },
            }
            -- One call site, two renderers. The rigged path reports
            -- unavailability (no depth buffer, failed shader) rather than
            -- throwing, and the procedural renderer stays the fallback.
            local v = view_state.get(roster.ids[index])
            if pitch.rigged_players and player_renderer_3d.available() then
                player_renderer_3d.draw(sx, sy, r, color, v, player_opts)
            else
                player_renderer.draw(sx, sy, r, color, v, player_opts)
            end
        elseif ball.visible then
            -- Loose / dribbled ball. (A keeper-held ball is drawn in its hands by the
            -- keeper avatar, so skip the ground ball then.) The shadow stays on the
            -- ground and shrinks/fades with height; the ball lifts by its height.
            local sx, sy, scale = project(ball.x, ball.y)
            local z = ball.z
            local hk = 1 / (1 + z / 80)
            love.graphics.setColor(0, 0, 0, 0.3 * hk)
            love.graphics.ellipse("fill", sx, sy, 6 * scale * hk, 3 * scale * hk)
            love.graphics.setColor(1, 0.95, 0.7)
            love.graphics.circle("fill", sx, sy - (z + 4) * scale, 5 * scale)
        end
    end

    combat_render.draw_over(frame, project)

    -- Landing reticle: a lofted, loose ball projects where it will come down, so
    -- a player can time a run to meet a cross. The ballistic solve belongs to the
    -- payload; this only draws the ring it hands back.
    local landing_x, landing_y = ball.landing_x, ball.landing_y
    if landing_x and landing_y then
        local sx, sy, scale = project(landing_x, landing_y)
        local t_now = (love.timer and love.timer.getTime and love.timer.getTime()) or 0
        local pulse = 0.6 + 0.4 * math.abs(math.sin(t_now * 6))
        love.graphics.setLineWidth(math.max(1, 1.5 * scale))
        love.graphics.setColor(1, 0.85, 0.35, 0.85 * pulse)
        love.graphics.circle("line", sx, sy, 12 * scale * pulse)
        love.graphics.setColor(1, 0.85, 0.35, 0.4)
        love.graphics.circle("line", sx, sy, 7 * scale)
        love.graphics.setLineWidth(1)
    end

    -- Pass-target preview: a small pulsing double-ring at the intended receiver's
    -- feet while the pass button is held. Guards love.timer access so the smoke
    -- test (which stubs love.graphics but not love.timer) stays green.
    local target = frame.control.pass_target
    if target then
        local tsx, tsy, tscale = project(players.x[target], players.y[target])
        local t_now = (love.timer and love.timer.getTime and love.timer.getTime()) or 0
        local pulse = 0.65 + 0.35 * math.abs(math.sin(t_now * 5))
        local team_color = (roster.teams[target] == "home") and opts.home_color or opts.away_color
        love.graphics.setLineWidth(math.max(1, 1.5 * tscale))
        love.graphics.setColor(team_color[1], team_color[2], team_color[3], 0.85 * pulse)
        love.graphics.circle("line", tsx, tsy, 10 * tscale * pulse)
        love.graphics.setColor(team_color[1], team_color[2], team_color[3], 0.45 * pulse)
        love.graphics.circle("line", tsx, tsy, 16 * tscale * pulse)
        love.graphics.setLineWidth(1)
    end

    -- Charge meter under the controlled player (soccer-game power bar):
    -- warm while charging a shot/punt, cool while charging a pass range.
    local charge_kind = frame.control.charge_kind
    if charge_kind then
        local amt = frame.control.charge
        local ccol = (charge_kind == "shot") and { 1, 0.72, 0.3 } or { 0.45, 0.85, 1 }
        local label = (charge_kind == "shot") and "SHOT" or "PASS"
        local controlled = frame.control.controlled
        local sx, sy, scale = project(players.x[controlled], players.y[controlled])
        local w, h = 34 * scale, math.max(3, 4 * scale)
        local y0 = sy + 12 * scale
        love.graphics.setColor(0, 0, 0, 0.55)
        love.graphics.rectangle("fill", sx - w / 2, y0, w, h)
        love.graphics.setColor(ccol[1], ccol[2], ccol[3], 0.95)
        love.graphics.rectangle("fill", sx - w / 2, y0, w * amt, h)
        love.graphics.setColor(1, 1, 1, 0.35)
        love.graphics.rectangle("line", sx - w / 2, y0, w, h)
        for i = 1, 4 do
            local tick_x = sx - w / 2 + w * i / 5
            love.graphics.line(tick_x, y0, tick_x, y0 + h)
        end
        love.graphics.setColor(ccol[1], ccol[2], ccol[3], 0.95)
        love.graphics.printf(label, sx - w / 2, y0 + h + 1, w, "center")
    end

    -- Flashes/sparks ride on top of everything.
    effects.draw_over(project)

    if sink then
        sink.scene_static_s = static_done - static_started
        sink.scene_dynamic_s = love.timer.getTime() - static_done
    end
end

return pitch
