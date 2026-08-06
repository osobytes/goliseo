-- Rigged 3D player renderer: a drop-in alternative to `player_renderer.draw`.
--
-- The signature matches the 2.5D renderer exactly, because integration happens
-- at a single call site in pitch.lua. `sx`, `sy` and `r` still come from
-- game/render/camera.lua -- that projection stays the authority on where a
-- player is and how large they appear, so the broadcast trapezoid is unchanged.
-- This module only decides *how* the player is drawn at that spot: a rigged,
-- animated, depth-tested character instead of a billboard.
--
-- Everything is generated in code (game/render/rig3d/), so there are no asset
-- files to load and nothing to fail at startup beyond a shader compile.
--
-- Failure is USUALLY recoverable: `available()` reports false and the caller
-- keeps the procedural 2.5D renderer, which is the parity/fallback rule the
-- rigged-player contract requires.
--
-- "Always" is what this said until #360 measured it false. Under love.js a
-- shader the runtime will not take does NOT come back as a catchable Lua error:
-- the failure escapes the `pcall` in `build()` below through a secondary fault
-- inside LÖVE's own boot.lua error path, and love.js alerts and stops. A `pcall`
-- cannot make a runtime that aborts underneath it recoverable -- the same lesson
-- bloom.lua learned about `newCanvas`, arriving a second time through the shader
-- path.
--
-- So the contract holds wherever a compile failure is a Lua error, and does not
-- hold under love.js. That is the standing limitation, and it is UNCHANGED.
--
-- What HAS changed is that nothing triggers it today. The trigger #360 found was
-- Firefox, where no LÖVE shader declaring a `varying` compiled at all; that is
-- #391, fixed in #395 by hoisting those declarations above `main()` at the WebGL
-- boundary and by moving rig3d's vertex-only uniforms inside `#ifdef VERTEX`.
-- Re-measured on this branch in headed Chrome 151 and Firefox 153 on an RTX 2070
-- SUPER: the rig3d shader compiles and LINKS in both, and rigged players render
-- in both. The browser-build release hold that used to sit here is lifted.
--
-- Read that as "the known trigger is gone", not "the fallback works now". It
-- still does not, on that runtime, and the guard that would make it safe still
-- cannot be written from in here -- do not "fix" this by probing what compiles,
-- because the probe is the thing that crashes. `love . --gl-probe shader rig3d`
-- is the out-of-band form: a separate process whose death is the answer.
--
-- `pitch.rigged_players` is true by default; read that note before changing
-- anything here.

local action_pose = require("game.render.rig3d.action_pose")
local bloom = require("game.render.bloom")
local mat4 = require("core.mat4")
local match = require("sim.match")
local renderer = require("game.render.rig3d.renderer")
local skeleton = require("game.render.rig3d.skeleton")
local clips = require("game.render.rig3d.clips")
local masks = require("game.render.rig3d.masks")
local proportions = require("game.render.rig3d.proportions")
local themes = require("game.render.rig3d.themes")
local body = require("game.render.rig3d.body")
local view_state = require("game.render.view_state")

local player_renderer_3d = {}

-- Character height maps to roughly this many player-radii on screen. Tuned so a
-- rigged player reads at the same visual weight as the billboard it replaces.
local HEIGHT_IN_RADII = 3.0
-- Match camera looks down at the pitch; this is the apparent elevation.
local ELEVATION = math.rad(17)

local state = {
    built = false,
    failed = false,
    rig = nil,
    character = nil,
    palettes = {},
    -- Reused every frame by skeleton.boneRows. See its own note: allocating a
    -- fresh row buffer per character per frame is real GC pressure at ten
    -- players, and Shader:send copies immediately so one buffer is safe.
    bone_rows = {},
}

-- Opt-in per-character sub-phase instrumentation (#394), the same pattern as
-- pitch.phase_sink (#393) one level down: when a sink table is attached, every
-- drawn character splits its cost into pose evaluation (clip sampling +
-- layering + the action overlay), skeleton evaluation, the bone-row build, the
-- uniform/draw submission, and everything else (shadow, camera, yaw), each
-- ACCUMULATED into the sink -- the caller owns zeroing it per frame. nil (the
-- default) costs one branch per character and nothing else.
---@class CharPhaseSink
---@field pose_s number
---@field apply_s number
---@field rows_s number
---@field submit_s number
---@field other_s number
---@field characters integer

---@type CharPhaseSink?
player_renderer_3d.phase_sink = nil

-- The one conversion between the pitch's world units and the rig's metres.
--
-- Worth stating because it looks depth-dependent and is not. World-to-pixels is
-- the projection's `scale`, and pixels-to-metres is `ppm`, which is built from
-- `r = radius * scale`. The two `scale` terms cancel:
--
--     metres = world * height / (PLAYER_RADIUS * HEIGHT_IN_RADII * 2)
--
-- so anything sized through here is the same size at the halfway line as it is
-- on the goal line, which is what you want for a ball.
---@return number
local function metresPerWorldUnit()
    return state.height / (match.PLAYER_RADIUS * HEIGHT_IN_RADII * 2)
end

---@type fun(sx: number, sy: number, r: number, color: number[], view: PlayerView|nil, opts: table)
local draw_player

-- Builds the shared rig, the ONE shared character mesh (colour-free), and one
-- resolved palette per team. Called once, lazily, so a headless or
-- shader-less runtime never pays for it.
---@return boolean
local function build()
    if state.built then
        return not state.failed
    end
    state.built = true

    local ok, err = pcall(function()
        local rig = proportions.RIG_MEDIUM
        renderer.load(themes.SLOT_COUNT, skeleton.boneCount(rig))
        state.rig = skeleton.new(rig)
        state.height = proportions.height(rig)
        -- One theme for the first integration. Theme selection per player is
        -- presentation data that belongs with the roster, not in the renderer.
        local theme = themes.LIST[1]
        local figure = themes.FIGURES[1]
        -- The mesh carries a palette SLOT INDEX per vertex, not a literal
        -- colour (#337 slice 1), so it does not depend on team at all -- one
        -- build serves every team. Only the small resolved-colour array differs
        -- per team, and that is a uniform, not a mesh. Since slice 2 it also
        -- carries a bone index and a material per vertex, so the whole
        -- character is one static mesh and one draw call rather than ~28.
        state.character = body.build(rig, theme, figure)
        for _, team in ipairs(themes.TEAMS) do
            state.palettes[team.key] = themes.resolvedPalette(theme, team)
        end
    end)

    if not ok then
        state.failed = true
        print("rigged 3D players disabled (build failed): " .. tostring(err))
    end
    return not state.failed
end

-- True when a rigged player can actually be drawn this frame: the meshes and
-- shader built, and the current render target has a depth buffer.
---@return boolean
function player_renderer_3d.available()
    if not build() then
        return false
    end
    return bloom.hasDepth()
end

-- Maps a presentation pose id onto the LIMB clips that exist today.
--
-- Three mechanisms cover the pose contract between them, and which one owns an
-- id is not arbitrary:
--
--   * rig3d/action_pose.lua owns the ids the body performs as a whole -- dives,
--     aerials, knockback, stagger, stumble, get-up. Those are continuous
--     transforms driven by the simulation's own timers, not clips.
--   * The keeper's hands are driven off possession (`holding` / `throw`) rather
--     than a pose id, because a keeper carrying the ball has their arms around
--     it whatever else they are doing.
--   * This table owns the rest: stances and gaits that are genuinely limb
--     animation.
--
-- Coverage is still partial and deliberately honest about it. The keeper's
-- positioning family (ready-tall, ready-low, set, shuffle) and the outfield
-- contact family (slide, tackle, kick_follow, settle) have no authored clips
-- yet and resolve to idle or locomotion -- they are #101 and #102's scope.
-- Anything unmapped falls back rather than erroring, and this table is the
-- single place new clips get wired in as they land.
local POSE_CLIP = {
    locomotion = "locomotion",
    contain = "locomotion",
    run_telegraph = "locomotion",
    fatigue = "idle",
    -- A keeper shuffling across the goal is still travelling, so the gait
    -- blend reads better here than a standing idle would.
    keeper_shuffle = "locomotion",
    -- A settle and a kick follow-through both happen mid-stride.
    settle = "locomotion",
    kick_follow = "locomotion",
    -- Guard is a STANCE the player chooses, layered over whatever gait is
    -- playing -- not something baked into how they always run.
    combat_guard = "guard",
    combat_windup = "guard",
    combat_active = "charge",
    combat_recovery = "guard",
    combat_aim = "guard",
}

---@param pose_id string|nil
---@return string
local function clipFor(pose_id)
    return POSE_CLIP[pose_id or ""] or "idle"
end

-- Resolves the pose for one player.
---@param view PlayerView|nil
---@param opts table
---@return table
local function poseFor(view, opts)
    local speed = view and view.speed or 0
    local idle, walk, run = clips.ORDER[1], clips.ORDER[2], clips.RUN

    -- A run is not a fast walk, so the two are separate clips blended by speed
    -- rather than one clip played quicker. Playing a walk faster gives short,
    -- frantic steps with no knee lift and no lean -- which is exactly how it
    -- looked before.
    local walk_mix = math.min(speed / view_state.WALK_SPEED, 1)
    local run_mix = math.max(
        0,
        math.min(
            (speed - view_state.WALK_SPEED) / (view_state.RUN_SPEED - view_state.WALK_SPEED),
            1
        )
    )

    -- Both cycles are two steps with contacts at 0 and 0.5, so one normalised
    -- phase drives both and they stay in step through the blend. view_state
    -- accumulates it incrementally against a speed-dependent stride, which is
    -- what keeps the feet planted without the phase jumping when speed changes.
    local cycles = view and view.gait or 0

    local pose = clips.layer(
        clips.sample(idle, love.timer.getTime() * 0.35),
        clips.sample(walk, cycles * walk.duration),
        masks.FULL_BODY,
        walk_mix
    )
    if run_mix > 0 then
        pose = clips.layer(pose, clips.sample(run, cycles * run.duration), masks.FULL_BODY, run_mix)
    end

    local selected = clipFor(opts.pose and opts.pose.id)
    if selected == "guard" then
        pose = clips.layer(
            pose,
            clips.sample(clips.GUARD_STANCE, love.timer.getTime()),
            masks.UPPER_BODY,
            1
        )
    elseif selected == "charge" then
        -- The charge is a held pose, so it only needs a phase to breathe on;
        -- tying it to the stride keeps the sway in step with the legs.
        pose = clips.layer(
            pose,
            clips.sample(clips.CHARGE, cycles * clips.CHARGE.duration),
            masks.UPPER_BODY,
            1
        )
    elseif selected == "swing" then
        local swing = clips.ORDER[3]
        local t = (opts.windup and opts.windup > 0) and 0.3 or 0.55
        pose = clips.layer(pose, clips.sample(swing, t * swing.duration), masks.UPPER_BODY, 1)
    end

    -- The keeper's hands follow possession, not the pose id: arms wrapped
    -- around a ball outrank whatever else the keeper is doing, and they have to
    -- arrive at socket_ball or the held ball reads as floating.
    if (opts.throw or 0) > 0 then
        -- throw_timer counts DOWN, so 1 is the moment of commitment. Playing
        -- the sling as it drains runs cock -> release -> follow-through in the
        -- order a throw actually happens.
        local sling = clips.KEEPER_SLING
        pose = clips.layer(
            pose,
            clips.sample(sling, (1 - math.min(opts.throw, 1)) * sling.duration),
            masks.UPPER_BODY,
            1
        )
    elseif opts.holding then
        pose = clips.layer(
            pose,
            clips.sample(clips.KEEPER_GATHER, love.timer.getTime()),
            masks.UPPER_BODY,
            1
        )
    end

    -- Whole-body actions last: they move the root, so they ride on top of
    -- whatever gait and stance resolved instead of competing with them.
    return action_pose.apply(pose, opts)
end

-- Drop-in replacement for player_renderer.draw.
--
-- `sx`, `sy` are the projected screen position of the player's feet and `r` the
-- depth-scaled radius, exactly as the 2.5D renderer receives them.
---@param sx number
---@param sy number
---@param r number
---@param color number[]
---@param view PlayerView|nil
---@param opts table
function player_renderer_3d.draw(sx, sy, r, color, view, opts)
    if not build() then
        return
    end
    -- A per-frame draw error must not take the match down. `build()` being
    -- pcall'd only covered construction; an exception in here (a bad facing, a
    -- mesh/material mismatch) would otherwise propagate into the match loop.
    -- Latching `failed` means the fallback is permanent and the diagnostic is
    -- printed once rather than every frame.
    local ok, err = pcall(draw_player, sx, sy, r, color, view, opts)
    if not ok then
        state.failed = true
        print("rigged 3D players disabled (draw failed): " .. tostring(err))
    end
end

---@param sx number
---@param sy number
---@param r number
---@param color number[]
---@param view PlayerView|nil
---@param opts table
function draw_player(sx, sy, r, color, view, opts)
    local team = themes.TEAMS[(opts.team == "away") and 2 or 1]
    local character = state.character
    local palette = state.palettes[team.key]
    if not character or not palette then
        return
    end

    -- Sub-phase timer (#394): sink attached and a real clock available.
    local sink = player_renderer_3d.phase_sink
    if sink and not (love.timer and love.timer.getTime) then
        sink = nil
    end
    local clock = sink and love.timer.getTime
    local t_start = clock and clock() or 0

    -- Ground contact first, in 2D on the pitch plane, matching the billboard
    -- renderer's shadow and selection rings exactly. Without a shadow a rigged
    -- character reads as floating above the pitch rather than standing on it --
    -- it is the cheapest thing that sells the ground plane.
    love.graphics.setColor(0, 0, 0, 0.35)
    love.graphics.ellipse("fill", sx, sy, r * 1.15, r * 0.5)
    if opts.controlled then
        love.graphics.setColor(1, 1, 1, 0.92)
        love.graphics.setLineWidth(math.max(1, r * 0.12))
        love.graphics.ellipse("line", sx, sy, r * 1.25, r * 0.6)
        love.graphics.ellipse("line", sx, sy, r * 1.48, r * 0.72)
        love.graphics.setLineWidth(1)
    end
    love.graphics.setColor(1, 1, 1, 1)

    local vw, vh = love.graphics.getDimensions()
    -- Feet sit at the projected point, so the character grows upward from it.
    local ppm = (r * HEIGHT_IN_RADII * 2) / state.height
    local cam = renderer.characterCamera(sx, sy, ppm, vw, vh, ELEVATION)

    local t_pose = clock and clock() or 0
    local pose = poseFor(view, opts)
    local t_apply = clock and clock() or 0
    skeleton.apply(state.rig, pose)
    local t_rows = clock and clock() or 0
    skeleton.boneRows(state.rig, state.bone_rows)
    local t_submit = clock and clock() or 0

    -- Facing: the pitch's +y runs toward the near edge (toward the viewer), and
    -- the character's local +Z is its front, so a player running "down" the
    -- screen faces the camera.
    local facing = opts.facing
    local yaw = facing and math.atan2(facing.x, facing.y) or 0
    local world = mat4.rotationY(yaw)

    -- One character, one draw call (#337 slice 2). The bone each vertex rides
    -- and the material each vertex shades with are both baked into the vertex,
    -- so there is nothing left to iterate here.
    renderer.beginPass(cam, palette)
    renderer.draw(character.mesh, world, state.bone_rows)
    renderer.endPass()

    if sink then
        local t_done = clock()
        sink.pose_s = (sink.pose_s or 0) + (t_apply - t_pose)
        sink.apply_s = (sink.apply_s or 0) + (t_rows - t_apply)
        sink.rows_s = (sink.rows_s or 0) + (t_submit - t_rows)
        -- The yaw/world build rides in submit: it exists to feed the draw.
        sink.submit_s = (sink.submit_s or 0) + (t_done - t_submit)
        sink.other_s = (sink.other_s or 0) + (t_pose - t_start)
        sink.characters = (sink.characters or 0) + 1
    end
end

return player_renderer_3d
