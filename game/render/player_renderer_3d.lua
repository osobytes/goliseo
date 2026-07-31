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

local player_renderer_3d = {}

-- Character height maps to roughly this many player-radii on screen. Tuned so a
-- rigged player reads at the same visual weight as the billboard it replaces.
local HEIGHT_IN_RADII = 3.0
-- Match camera looks down at the pitch; this is the apparent elevation.
local ELEVATION = math.rad(17)
-- World units/sec at which the walk blend reaches full. view_state caps display
-- speed at 480, and a jog sits well below that.
local FULL_STRIDE_SPEED = 170

local state = { built = false, failed = false, rig = nil, teams = {} }

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
    combat_guard = "charge",
    combat_windup = "charge",
    combat_active = "swing",
    combat_recovery = "idle",
    combat_aim = "charge",
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
    local blend = math.min(speed / FULL_STRIDE_SPEED, 1)
    -- The gait phase advances with DISTANCE travelled (view_state derives it
    -- that way), which is what keeps the feet from sliding when a player
    -- accelerates. Scaling it into clip seconds preserves that property.
    local stride_time = (view and view.phase or 0) * 0.5

    local idle, walk = clips.ORDER[1], clips.ORDER[2]
    local pose = clips.layer(
        clips.sample(idle, love.timer.getTime() * 0.35),
        clips.sample(walk, stride_time),
        masks.FULL_BODY,
        blend
    )

    local selected = clipFor(opts.pose and opts.pose.id)
    if selected == "charge" then
        pose = clips.layer(pose, clips.sample(clips.CHARGE, stride_time), masks.UPPER_BODY, 1)
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

    local team = themes.TEAMS[(opts.team == "away") and 2 or 1]
    local parts = state.teams[team.key]
    if not parts then
        return
    end

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
