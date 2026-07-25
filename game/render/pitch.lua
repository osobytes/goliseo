-- Impure 2.5D match renderer: draws the simulation state through the camera
-- projection as a perspective pitch with depth-sorted billboard players.
-- (Bloom/neon post-processing is a later pass; this is the geometry layer.)

local camera = require("game.render.camera")
local arena_render = require("game.render.arena")
local combat_render = require("game.render.combat")
local player_renderer = require("game.render.player_renderer")
local player_pose = require("game.presentation.player_pose")
local identity = require("game.presentation.identity")
local view_state = require("game.render.view_state")
local effects = require("game.render.effects")
local arenas = require("data.arenas")
local sim_match = require("sim.match") -- CROSSBAR_H: the goal frame height

local pitch = {}

local HEX_RADIUS = 26 -- world units, centre to corner
local NET_BACK_FRAC = 0.55 -- back frame height as a fraction of the crossbar

---@class PitchDrawOptions
---@field home_color number[]
---@field away_color number[]
---@field arena ArenaData?
---@field arena_pulse number?
---@field render_pose CorrectionSmoothingPose?
---@field combat CombatPresentationModel?

---@param player MatchPlayer
---@param pose CorrectionSmoothingPose?
---@return { x: number, y: number }
local function player_position(player, pose)
    return (pose and pose.players[player.id]) or player.pos
end

---@param state MatchState
---@param pose CorrectionSmoothingPose?
---@return { x: number, y: number }
local function ball_position(state, pose)
    return (pose and pose.ball) or state.ball
end

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
---@param field { w: number, h: number }
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
---@param field { w: number, h: number }
local function draw_markings(project, field)
    love.graphics.setLineWidth(2)
    set({ 0.35, 0.72, 1.0 }, 0.85)

    local x1, y1 = project(field.w / 2, 0)
    local x2, y2 = project(field.w / 2, field.h)
    love.graphics.line(x1, y1, x2, y2)

    love.graphics.polygon("line", projected_circle(project, field.w / 2, field.h / 2, 70, 36))

    local sx, sy = project(field.w / 2, field.h / 2)
    love.graphics.circle("fill", sx, sy, 3)

    local depth, box_h = sim_match.PENALTY_BOX.depth, sim_match.PENALTY_BOX.h
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
---@param field { w: number, h: number }
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
---@param s MatchState
---@param vp { w: number, h: number }
---@param opts PitchDrawOptions
function pitch.draw(s, vp, opts)
    local field = s.field
    local arena = opts.arena or arenas.helios_crown
    local render_pose = opts.render_pose
    local rendered_ball = ball_position(s, render_pose)
    local function project(wx, wy)
        return camera.project(wx, wy, field, vp)
    end

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
        local bar = sim_match.CROSSBAR_H
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
    draw_goal(s.goal_home, opts.home_color, s.goal_home.x + s.goal_home.w, s.goal_home.x)
    draw_goal(s.goal_away, opts.away_color, s.goal_away.x, s.goal_away.x + s.goal_away.w)

    -- Ball trail sits on the ground, under the entities.
    effects.draw_trail(project)
    if opts.combat then
        combat_render.draw_under(opts.combat, project, render_pose)
    end

    -- Depth-sorted drawables (far first).
    local items = {}
    for i, p in ipairs(s.players) do
        local pos = player_position(p, render_pose)
        items[#items + 1] = { kind = "player", p = p, pos = pos, idx = i, depth = pos.y }
    end
    items[#items + 1] = { kind = "ball", depth = rendered_ball.y }
    table.sort(items, function(a, b)
        return a.depth < b.depth
    end)

    -- Held in the HANDS only: a keeper with a back-pass at its feet dribbles a
    -- ground ball like anyone else.
    local keeper_holds = s.owner ~= nil
        and s.players[s.owner].is_keeper
        and not s.players[s.owner].feet_ball
    for _, it in ipairs(items) do
        if it.kind == "player" then
            local p = it.p
            local pos = it.pos
            local sx, sy, scale = project(pos.x, pos.y)
            local r = p.radius * scale
            local color = (p.team == "home") and opts.home_color or opts.away_color
            local presentation =
                assert(identity.for_player(p.id), "missing pitch identity for " .. p.id)
            local combat_sample = opts.combat and opts.combat.players[it.idx] or nil
            local aerial_duration = 0.22
            if p.aerial_style == "bicycle" then
                aerial_duration = 0.6
            elseif (p.aerial_jump or 0) > 0 then
                aerial_duration = 0.35
            elseif p.aerial_style == "leg_control" or p.aerial_style == "chest_control" then
                aerial_duration = 0.18
            end
            player_renderer.draw(sx, sy, r, color, view_state.get(p.id), {
                facing = p.facing,
                is_keeper = p.is_keeper,
                controlled = (it.idx == s.controlled),
                dashing = p.slide_timer > 0,
                -- 0.3 is a visual normalizer (~ the sim's dive duration); exactness
                -- doesn't matter, it just eases the lunge back upright.
                dive = (p.dive_timer > 0) and math.min(1, p.dive_timer / 0.3) or 0,
                dive_dir = p.dive_dir,
                -- Keeper holding the ball: render it cradled in the hands (below).
                holding = (it.idx == s.owner and p.is_keeper and not p.feet_ball),
                grab = (p.grab_timer > 0) and math.min(1, p.grab_timer / 0.25) or 0,
                throw = (p.throw_timer > 0) and math.min(1, p.throw_timer / 0.25) or 0,
                -- Wind-up back-swing: 0 = no windup, 1 = just committed.
                windup = (p.windup_timer > 0) and (p.windup_timer / 0.15) or 0,
                aerial = ((p.aerial_timer or 0) > 0)
                        and math.min(1, p.aerial_timer / aerial_duration)
                    or 0,
                aerial_style = p.aerial_style,
                aerial_outcome = p.aerial_outcome,
                aerial_jump = p.aerial_jump or 0,
                species_shape = presentation.shape,
                species_color = presentation.palette,
                team = p.team,
                combat = combat_sample,
                pose = player_pose.select(p, combat_sample),
            })
        elseif not keeper_holds then
            -- Loose / dribbled ball. (A keeper-held ball is drawn in its hands by the
            -- keeper avatar, so skip the ground ball then.) The shadow stays on the
            -- ground and shrinks/fades with height; the ball lifts by its height.
            local sx, sy, scale = project(rendered_ball.x, rendered_ball.y)
            local z = s.ball_z or 0
            local hk = 1 / (1 + z / 80)
            love.graphics.setColor(0, 0, 0, 0.3 * hk)
            love.graphics.ellipse("fill", sx, sy, 6 * scale * hk, 3 * scale * hk)
            love.graphics.setColor(1, 0.95, 0.7)
            love.graphics.circle("fill", sx, sy - (z + 4) * scale, 5 * scale)
        end
    end

    if opts.combat then
        combat_render.draw_over(opts.combat, project)
    end

    -- Landing reticle: a lofted, loose ball projects where it will come down, so
    -- a player can time a run to meet a cross. Only for a genuinely airborne ball
    -- (a cross/lob), not a grounded pass. Ballistic solve to z = 0.
    do
        local bz = s.ball_z or 0
        if not s.owner and bz > 20 then
            local g = 900 -- matches the sim's GRAVITY
            local vz = s.ball_vz or 0
            local tland = (vz + math.sqrt(vz * vz + 2 * g * bz)) / g
            local lx = rendered_ball.x + s.ball_vel.x * tland
            local ly = rendered_ball.y + s.ball_vel.y * tland
            if
                tland > 0.05
                and tland < 3
                and lx > 0
                and lx < field.w
                and ly > 0
                and ly < field.h
            then
                local sx, sy, scale = project(lx, ly)
                local t_now = (love.timer and love.timer.getTime and love.timer.getTime()) or 0
                local pulse = 0.6 + 0.4 * math.abs(math.sin(t_now * 6))
                love.graphics.setLineWidth(math.max(1, 1.5 * scale))
                love.graphics.setColor(1, 0.85, 0.35, 0.85 * pulse)
                love.graphics.circle("line", sx, sy, 12 * scale * pulse)
                love.graphics.setColor(1, 0.85, 0.35, 0.4)
                love.graphics.circle("line", sx, sy, 7 * scale)
                love.graphics.setLineWidth(1)
            end
        end
    end

    -- Pass-target preview: a small pulsing double-ring at the intended receiver's
    -- feet while the pass button is held. Guards love.timer access so the smoke
    -- test (which stubs love.graphics but not love.timer) stays green.
    local cp = s.players[s.controlled]
    if cp.pass_target then
        local tp = s.players[cp.pass_target]
        local target_pos = player_position(tp, render_pose)
        local tsx, tsy, tscale = project(target_pos.x, target_pos.y)
        local t_now = (love.timer and love.timer.getTime and love.timer.getTime()) or 0
        local pulse = 0.65 + 0.35 * math.abs(math.sin(t_now * 5))
        local team_color = (tp.team == "home") and opts.home_color or opts.away_color
        love.graphics.setLineWidth(math.max(1, 1.5 * tscale))
        love.graphics.setColor(team_color[1], team_color[2], team_color[3], 0.85 * pulse)
        love.graphics.circle("line", tsx, tsy, 10 * tscale * pulse)
        love.graphics.setColor(team_color[1], team_color[2], team_color[3], 0.45 * pulse)
        love.graphics.circle("line", tsx, tsy, 16 * tscale * pulse)
        love.graphics.setLineWidth(1)
    end

    -- Charge meter under the controlled player (soccer-game power bar):
    -- warm while charging a shot/punt, cool while charging a pass range.
    local amt, ccol, label
    if cp.charge > 0.02 then
        amt, ccol, label = cp.charge, { 1, 0.72, 0.3 }, "SHOT"
    elseif cp.pass_charge > 0.02 then
        amt, ccol, label = cp.pass_charge, { 0.45, 0.85, 1 }, "PASS"
    end
    if amt then
        local controlled_pos = player_position(cp, render_pose)
        local sx, sy, scale = project(controlled_pos.x, controlled_pos.y)
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
end

return pitch
