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
-- Failure is always recoverable: `available()` reports false and the caller
-- keeps the procedural 2.5D renderer, which is the parity/fallback rule the
-- rigged-player contract requires.

local bloom = require("game.render.bloom")
local mat4 = require("core.mat4")
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

local state = { built = false, failed = false, rig = nil, teams = {} }

---@type fun(sx: number, sy: number, r: number, color: number[], view: PlayerView|nil, opts: table)
local draw_player

-- Builds the shared rig and one draw list per team. Called once, lazily, so a
-- headless or shader-less runtime never pays for it.
---@return boolean
local function build()
    if state.built then
        return not state.failed
    end
    state.built = true

    local ok, err = pcall(function()
        renderer.load()
        local rig = proportions.RIG_MEDIUM
        state.rig = skeleton.new(rig)
        state.height = proportions.height(rig)
        -- One theme for the first integration. Theme selection per player is
        -- presentation data that belongs with the roster, not in the renderer.
        local theme = themes.LIST[1]
        local figure = themes.FIGURES[1]
        for _, team in ipairs(themes.TEAMS) do
            state.teams[team.key] = body.build(rig, theme, team, figure)
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

-- Maps a presentation pose id onto the clips that exist today.
--
-- Coverage is deliberately honest: the contract defines 32 pose ids and this
-- library has four clips, so most ids resolve to idle. Anything unmapped simply
-- falls back rather than erroring, and the table is the single place new clips
-- get wired in as they are authored.
local POSE_CLIP = {
    locomotion = "locomotion",
    contain = "locomotion",
    run_telegraph = "locomotion",
    fatigue = "idle",
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
    return pose
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
    local parts = state.teams[team.key]
    if not parts then
        return
    end

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

    skeleton.apply(state.rig, poseFor(view, opts))

    -- Facing: the pitch's +y runs toward the near edge (toward the viewer), and
    -- the character's local +Z is its front, so a player running "down" the
    -- screen faces the camera.
    local facing = opts.facing
    local yaw = facing and math.atan2(facing.x, facing.y) or 0
    local world = mat4.rotationY(yaw)

    renderer.beginPass(cam)
    for _, part in ipairs(parts) do
        local bone_world = mat4.multiply(world, state.rig.world[part.bone])
        local model = part.attach and mat4.multiply(bone_world, part.attach) or bone_world
        renderer.draw(part.mesh, model, part.material)
    end
    renderer.endPass()
end

return player_renderer_3d
