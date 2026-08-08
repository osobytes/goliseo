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
local pitch_static = require("game.render.pitch_static")
local combat_render = require("game.render.combat")
local player_renderer = require("game.render.player_renderer")
local player_renderer_3d = require("game.render.player_renderer_3d")
local view_state = require("game.render.view_state")
local effects = require("game.render.effects")
local arenas = require("data.arenas")

local pitch = {}

-- Rigged 3D players, ON by default. That is the project's direction -- retiring
-- the procedural 2.5D look is the point of the current work -- and the gate this
-- waited on has reported: #337 slice 2 draws ten rigged players in 33.2 draw
-- calls, inside #100's budget natively and rendering under love.js in Chrome AND
-- Firefox.
--
-- SUPERSEDED HAZARD, KEPT NARROWED. This note used to say that under love.js in
-- Firefox this default crashed the match on entry, and that a browser build must
-- not ship with it on. That was true and it was measured. It is no longer true:
-- #391 -- Firefox's WebGL translator emitting invalid GLSL for any LÖVE shader
-- that declares a `varying`, plus rig3d declaring its vertex-only uniforms
-- outside the stage blocks -- is fixed in #395, and #360 re-measured afterwards
-- in headed Firefox 153 on an RTX 2070 SUPER: the whole shader ladder compiles
-- AND links, and ten rigged players render. The release hold is lifted.
--
-- What did NOT get fixed, and is why this paragraph survives: the SHAPE of that
-- crash. Under love.js a shader the runtime will not take is still not a
-- catchable Lua error -- it escapes the `pcall` in `player_renderer_3d.build()`
-- through a secondary fault inside LÖVE's own boot.lua error path and takes the
-- page down. #391 removed the shader that triggered it, not the class, and
-- `pitch.draw` below still calls `available()` unconditionally. So a future edit
-- that makes any browser reject the rig3d shader is a crash on entering a match
-- again, not a fallback to the procedural renderer -- and there is still no
-- out-of-band guard, because learning whether a shader compiles requires
-- compiling it. Treat rig3d GLSL as code that has to be measured in a browser,
-- not only run natively; `love . --gl-probe shader rig3d` is that measurement.
pitch.rigged_players = true

-- Opt-in broadcast-style following camera. Off by default: it reframes the whole
-- match, so it stays behind a flag until it has been played.
pitch.follow_camera = false

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

    -- Static scene (backdrop, floor, glow, hex tiling, markings, outline,
    -- goals): a cached-canvas blit when the cache is valid, the direct path
    -- otherwise. The cache is bypassed whenever a follow view is active --
    -- the projection then changes every frame -- and on a miss the build is
    -- deferred to pitch_static.flush(), which the screen calls after the
    -- bloom pass (building here would detach bloom's depth buffer).
    ---@type PitchStaticOptions
    local scene_opts = {
        arena = arena,
        home_color = opts.home_color,
        away_color = opts.away_color,
    }
    local drew_cached = false
    if not view then
        drew_cached = pitch_static.draw_cached(field, vp, scene_opts, opts.camera_offset)
    end
    if not drew_cached then
        pitch_static.draw_scene(project, field, vp, scene_opts)
    end

    -- The arena frame chevrons pulse with the kickoff banner, so they stay
    -- live over the cached scene. They sit at the trapezoid corners, clear of
    -- the goals, so drawing them after the goals (the cached scene includes
    -- the goals) is visually identical to the old in-between order.
    local ax, ay = project(0, 0)
    local bx, by = project(field.w, 0)
    local cx, cy = project(field.w, field.h)
    local dx, dy = project(0, field.h)
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
