-- Render differential capture: runs game.render.pitch's REAL draw code over a
-- fixed, hand-authored RenderFrame and records every geometry-affecting
-- love.graphics call as a normalized, JSON-encodable record -- comparable
-- field-for-field against packages/render/src/pitch.ts's `pitchDrawCommands`
-- output and against the (options, sx, sy, r, color) tuple pitch.lua hands to
-- game.render.player_renderer per player.
--
-- game.render.player_renderer and game.render.player_renderer_3d are replaced
-- in package.loaded BEFORE requiring game.render.pitch: the 3D module pulls in
-- sim.match and the whole rig3d tree for a code path pitch.rigged_players=false
-- never reaches, and the procedural module's own internal polygon soup (push/
-- translate/rotate limb transforms) is a second, much larger porting-fidelity
-- question already covered by that module's own spec -- this harness's job is
-- what pitch.lua itself computes and hands off (anchor position/scale/color/
-- the full PlayerRenderOptions payload -- pose, windup, aerial, dive, grab,
-- throw, holding, dashing, controlled, team, species), not what happens after.
--
-- HOW TO RUN (see README.md in this directory, and
-- v2/tools/lua_reference/README.md for the general pattern this follows):
--
--   1. Make a scratch directory (inside your own scratchpad, never the repo).
--   2. Copy in `game/`, `core/`, `data/` from the worktree root (NOT `sim/` --
--      this script never touches it; the RenderFrame below is a literal, not
--      simulated).
--   3. Drop in a conf.lua that disables window/graphics/audio (see
--      v2/tools/lua_reference/README.md's example).
--   4. Copy this file in as `main.lua`.
--   5. Run `love .` and capture stdout. The JSON array between
--      RENDER_REFERENCE_JSON_BEGIN/_END is the fixture; everything else on
--      stdout is not part of it.
--   6. Paste the captured JSON (verbatim, one line) into the
--      LUA_REFERENCE_JSON template literal in pitch.spec.ts's differential
--      describe block, and keep `luaDifferentialFrame()` there byte-for-byte
--      identical to the `frame` table below -- the whole comparison is only
--      meaningful if both sides consumed the SAME input.
--
-- Re-run this whenever pitch.lua's draw order, the fixture's field values, or
-- the set of love.graphics call kinds it issues changes.

local RECORD = {}

local function color_of(state)
    return { state.r, state.g, state.b }
end

local function push_geom(state, rec)
    rec.color = color_of(state)
    rec.alpha = state.a
    if state.blend ~= "alpha" then
        rec.blend = state.blend
    end
    RECORD[#RECORD + 1] = rec
end

-- LOVE's love.graphics.polygon/line accept EITHER a flat vararg list of
-- numbers OR a single table of numbers (game.render.pitch's draw_hex_floor
-- and draw_markings' circle helper use the table form; everything else in
-- this path uses the flat form) -- both have to normalize to the same flat
-- points array for this capture to be comparable to draw2d.ts's `points:
-- readonly number[]`.
local function flatten_points(...)
    local n = select("#", ...)
    if n == 1 and type((...)) == "table" then
        return (...)
    end
    return { ... }
end

local function stub_graphics()
    local state = { r = 1, g = 1, b = 1, a = 1, line_width = 1, blend = "alpha" }
    local g = {}
    g.setColor = function(r, gg, b, a)
        state.r, state.g, state.b, state.a = r, gg, b, (a == nil) and 1 or a
    end
    g.setLineWidth = function(w)
        state.line_width = w
    end
    g.setBlendMode = function(mode)
        state.blend = mode
    end
    g.polygon = function(mode, ...)
        local pts = flatten_points(...)
        local rec = { kind = "polygon", mode = mode, points = pts }
        if mode == "line" then
            rec.lineWidth = state.line_width
        end
        push_geom(state, rec)
    end
    g.circle = function(mode, x, y, r)
        local rec = { kind = "circle", mode = mode, x = x, y = y, r = r }
        if mode == "line" then
            rec.lineWidth = state.line_width
        end
        push_geom(state, rec)
    end
    g.ellipse = function(mode, x, y, rx, ry)
        local rec = { kind = "ellipse", mode = mode, x = x, y = y, rx = rx, ry = ry }
        if mode == "line" then
            rec.lineWidth = state.line_width
        end
        push_geom(state, rec)
    end
    g.line = function(...)
        local pts = flatten_points(...)
        push_geom(state, { kind = "line", points = pts, lineWidth = state.line_width })
    end
    g.rectangle = function(mode, x, y, w, h, rx, ry)
        local rec = { kind = "rect", mode = mode, x = x, y = y, w = w, h = h, rx = rx, ry = ry }
        if mode == "line" then
            rec.lineWidth = state.line_width
        end
        push_geom(state, rec)
    end
    g.printf = function(text, x, y, w, align)
        push_geom(
            state,
            { kind = "text", text = text, x = x, y = y, w = w, align = align or "left" }
        )
    end
    -- The goal-net mesh shader is a shading detail with no bearing on shape or
    -- position (pitch.ts's drawGoal has the same intentional omission) -- noop
    -- rather than recorded, so both sides agree on what is comparable.
    g.newShader = function()
        return { send = function() end }
    end
    g.setShader = function() end
    g.push = function()
        error(
            "unexpected love.graphics.push -- pitch.lua's own draw path should never call this directly"
        )
    end
    g.translate = function()
        error("unexpected love.graphics.translate")
    end
    g.rotate = function()
        error("unexpected love.graphics.rotate")
    end
    g.pop = function()
        error("unexpected love.graphics.pop")
    end
    return g
end

love.graphics = stub_graphics()
love.timer = {
    getTime = function()
        return 0
    end,
}

-- Replace the procedural and rigged player renderers BEFORE game.render.pitch
-- is required, so pitch.lua's own `require` calls resolve to these stand-ins.
package.loaded["game.render.player_renderer_3d"] = {
    available = function()
        return false
    end,
}
package.loaded["game.render.player_renderer"] = {
    draw = function(sx, sy, r, color, v, opts)
        RECORD[#RECORD + 1] = {
            kind = "player",
            sx = sx,
            sy = sy,
            r = r,
            color = color,
            opts = {
                facing = { opts.facing.x, opts.facing.y },
                is_keeper = opts.is_keeper,
                controlled = opts.controlled,
                dashing = opts.dashing,
                dive = opts.dive,
                dive_dir = opts.dive_dir and { opts.dive_dir.x, opts.dive_dir.y } or nil,
                holding = opts.holding,
                grab = opts.grab,
                throw = opts.throw,
                windup = opts.windup,
                aerial = opts.aerial,
                aerial_style = opts.aerial_style,
                aerial_outcome = opts.aerial_outcome,
                aerial_jump = opts.aerial_jump,
                species_shape = opts.species_shape,
                species_color = opts.species_color,
                team = opts.team,
                pose_id = opts.pose.id,
                pose_priority = opts.pose.priority,
                pose_source = opts.pose.source,
            },
        }
    end,
}

local pitch = require("game.render.pitch")
pitch.rigged_players = false
pitch.follow_camera = false

-- A deliberately SMALL field (200x120, versus the product's real 960x540).
-- game.render.pitch's hex floor tiles at a fixed 26-world-unit radius
-- regardless of field size, so the real pitch dimensions produce ~300 hex
-- polygon commands -- fine for a live game, unusable as an embedded test
-- fixture (v2/README.md #1 rules out reading fixture files from disk in this
-- package; the captured reference has to be a literal in the spec). Shrinking
-- the field is the only lever pitch.lua exposes to bring the hex count down;
-- every other code path below (backdrop, markings, goals, depth sort, the
-- overlay layer) runs identically regardless of field size.
---@type RenderFrame
-- Minimal by design: this fixture carries only the fields `pitch.lua` reads.
---@diagnostic disable-next-line: missing-fields
local frame = {
    field = {
        w = 200,
        h = 120,
        penalty_box_depth = 20,
        penalty_box_h = 60,
        crossbar_h = 16,
        goal_home = { x = -2, y = 52, w = 2, h = 16 },
        goal_away = { x = 200, y = 52, w = 2, h = 16 },
    },
    -- Minimal by design: this fixture carries only the fields `pitch.lua` reads.
    ---@diagnostic disable-next-line: missing-fields
    roster = {
        radius = { 2.5, 2.7, 2.5 },
        teams = { "home", "away", "home" },
        is_keeper = { false, true, false },
        species_shape = { "round", "round", "angular" },
        species_color = { { 1, 1, 1 }, { 0.8, 0.8, 1 }, { 0.9, 0.5, 0.2 } },
        ids = { "home-1", "away-kp", "home-2" },
    },
    -- Minimal by design: this fixture carries only the fields `pitch.lua` reads.
    ---@diagnostic disable-next-line: missing-fields
    players = {
        count = 3,
        x = { 108, 20, 146 },
        y = { 88, 18, 55 },
        facing_x = { 1, 0, -1 },
        facing_y = { 0, 1, 0 },
        controlled = { true, false, false },
        dashing = { true, nil, nil },
        dive = { nil, 0.35, nil },
        dive_dir_x = { nil, 1, nil },
        dive_dir_y = { nil, 0, nil },
        holding = { nil, true, nil },
        grab = { nil, nil, nil },
        throw = { nil, nil, nil },
        windup = { 0.42, nil, nil },
        aerial = { nil, nil, 0.6 },
        aerial_style = { nil, nil, "header" },
        aerial_outcome = { nil, nil, "clean" },
        aerial_jump = { nil, nil, 0.22 },
        -- `aerial_header` and `keeper` are NOT members of `PlayerPoseId` /
        -- `PlayerPoseSource`. That is deliberate here and must stay: `pitch.lua`
        -- passes both straight through to the player renderer without
        -- interpreting them, and `v2/ts/packages/render/src/pitch.spec.ts` pins
        -- these exact strings on the TypeScript side (see its `pose_id` /
        -- `pose_source` fixture and the captured reference JSON above it). The
        -- differential compares what the two languages DO with an opaque value;
        -- changing these to real enum members would mean recapturing the
        -- reference and editing the spec, for no gain in what is being tested.
        ---@diagnostic disable-next-line: assign-type-mismatch
        pose_id = { "tackle", "keeper_ready_tall", "aerial_header" },
        pose_priority = { 1, 2, 3 },
        ---@diagnostic disable-next-line: assign-type-mismatch
        pose_source = { "combat", "keeper", "combat" },
    },
    -- Minimal by design: this fixture carries only the fields `pitch.lua` reads.
    ---@diagnostic disable-next-line: missing-fields
    ball = { x = 100, y = 66, z = 4, visible = true, landing_x = 127, landing_y = 75 },
    control = { pass_target = 3, charge_kind = "shot", charge = 0.55, controlled = 1 },
}

local opts = {
    home_color = { 0.35, 0.75, 1.0 },
    away_color = { 1.0, 0.55, 0.25 },
}

-- The viewport is the FIELD's own size (#414). This capture previously used
-- 1280x720 and called it "the product's actual viewport", which was never true
-- of the LÖVE build: conf.lua pins a non-resizable 960x540 window and
-- sim/env_config.lua's DEFAULT_FIELD is 960x540, so `vp == field` in every
-- frame LÖVE has ever drawn.
--
-- Do NOT read that as "this fixture now captures the shipping configuration".
-- It does not, and cannot: the field above is a synthetic 200x120, shrunk to
-- keep the hex-tile count low enough to embed as a literal (see the block
-- comment above `frame` -- a pre-existing choice, unrelated to #414). LÖVE
-- never ships at 200x120. What `vp == field` buys HERE is narrower and purely
-- practical.
--
-- `game/render/camera.lua`'s fixed projection puts the world-to-pixel factor
-- into screen POSITIONS only, never into the depth scale that entity sizes are
-- derived from; v2's `packages/render/src/camera.ts` now carries one uniform
-- factor into both (see its `projectFixed` header). The two therefore agree
-- exactly when that factor is 1 -- i.e. at `vp == field` -- and diverge
-- otherwise. How they diverge depends on the field/vp aspect ratios:
--
--   * Same aspect (960x540 field at 1280x720): positions are still identical
--     and only `scale` differs, by a single constant. That is a one-part
--     divergence, and `camera.spec.ts` characterises it exactly -- its kept
--     1280x720 rows assert Lua's `sx`/`sy` verbatim and Lua's `scale` times
--     4/3.
--   * DIFFERENT aspect, which is this fixture's old case: 200x120 is 5:3 and
--     1280x720 is 16:9, so Lua's `vp.w/field.w` (6.4) and the uniform fit
--     `min(6.4, 6.0)` (6.0) disagree. Positions AND sizes then both diverge,
--     and pinning the fixture would mean characterising a two-part divergence
--     across every one of its 101 records.
--
-- Capturing at `vp == field` collapses that to zero divergence, so this stays
-- a plain equality differential over the whole draw list. Entity SIZES in the
-- capture are unchanged by the switch -- the old `scale` had no viewport term
-- at all -- so only screen positions moved.
--
-- This capture is therefore NOT the regression guard for #414: at `vp ==
-- field` the fixed and the buggy formula are byte-identical, so reverting
-- camera.ts leaves every record here passing (verified). Viewport-safety
-- coverage lives in two other places, both viewport-varying by construction:
-- `camera.spec.ts`'s kept 1280x720 differential rows, and `pitch.spec.ts`'s
-- Lua-independent "pitch entity sizes stay in proportion to the pitch at any
-- viewport" block.
local vp = { w = 200, h = 120 }

pitch.draw(frame, vp, opts)

-- ---------------------------------------------------------------------------
-- Minimal JSON encoder -- only the shapes this capture ever produces (numbers,
-- strings, booleans, nil, integer-keyed arrays, string-keyed objects).
-- ---------------------------------------------------------------------------

local function is_array(t)
    local n = 0
    for _ in pairs(t) do
        n = n + 1
    end
    if n == 0 then
        return true
    end
    for i = 1, n do
        if t[i] == nil then
            return false
        end
    end
    return true
end

local json_encode
json_encode = function(v)
    local tv = type(v)
    if v == nil then
        return "null"
    elseif tv == "boolean" then
        return v and "true" or "false"
    elseif tv == "number" then
        return string.format("%.17g", v)
    elseif tv == "string" then
        return (string.format("%q", v):gsub("\\\n", "n"))
    elseif tv == "table" then
        if is_array(v) then
            local parts = {}
            for i = 1, #v do
                parts[i] = json_encode(v[i])
            end
            return "[" .. table.concat(parts, ",") .. "]"
        else
            local keys = {}
            for k in pairs(v) do
                keys[#keys + 1] = k
            end
            table.sort(keys)
            local parts = {}
            for _, k in ipairs(keys) do
                parts[#parts + 1] = string.format("%q", k) .. ":" .. json_encode(v[k])
            end
            return "{" .. table.concat(parts, ",") .. "}"
        end
    end
    error("cannot JSON-encode a " .. tv)
end

print("RENDER_REFERENCE_JSON_BEGIN")
print(json_encode(RECORD))
print("RENDER_REFERENCE_JSON_END")

love.event.quit(0)
