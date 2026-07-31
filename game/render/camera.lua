-- Pure 2.5D projection. Maps a point on the flat pitch (world space) to a screen
-- point plus a depth scale, producing a perspective trapezoid: the far edge
-- (world y = 0) is higher and narrower, the near edge (world y = field.h) is
-- lower and wider. No love calls — so the projection is unit-testable.

local mat4 = require("core.mat4")

local camera = {}

---@class CameraConfig
---@field far_scale number  -- sprite/spread scale at the far edge
---@field near_scale number  -- sprite/spread scale at the near edge
---@field horizon_frac number  -- screen-height fraction where the far edge sits
---@field bottom_frac number  -- screen-height fraction where the near edge sits

-- Tuned so the pitch is inset within the viewport (margins on all sides) rather
-- than filling the screen — the arena floats in space, like a broadcast frame.
-- Keep near_scale < 1 so even the widest (near) edge stays off the screen edges.
-- Opt-in: use the real perspective camera instead of the fixed trapezoid.
camera.perspective_mode = false

---@type CameraConfig
camera.DEFAULTS = {
    far_scale = 0.51, -- wide far edge (less of a sharp wedge), but inset
    near_scale = 0.84, -- < 1: near edge sits ~8% in from each side
    horizon_frac = 0.24, -- space/HUD band above the pitch
    bottom_frac = 0.88, -- margin below the pitch
}

---@class CameraView
---@field x number     -- focus world x
---@field y number     -- focus world y
---@field zoom number  -- 1 = whole pitch, >1 = magnified about the focus

-- Clamped focus for a following camera.
--
-- `margin` is how far, in world units, the focus must stay inside each edge of
-- the pitch. The caller supplies it because the right value depends on the
-- projection: a lens zoom needs the whole frame to stay over the pitch, while a
-- real camera only needs to not fly off the end of it -- and must still be able
-- to reach the goals.
--
-- Deriving the margin from `field / (2 * zoom)` (the previous behaviour) pins
-- the focus to the exact centre of the pitch at zoom 1, so the camera cannot
-- follow anything at all.
---@param fx number
---@param fy number
---@param field { w: number, h: number }
---@param zoom number
---@param margin { x: number, y: number }?
---@return CameraView
function camera.view(fx, fy, field, zoom, margin)
    zoom = math.max(0.25, zoom or 1)
    local mx = margin and margin.x or field.w / (2 * zoom)
    local my = margin and margin.y or field.h / (2 * zoom)
    mx = math.min(mx, field.w / 2)
    my = math.min(my, field.h / 2)
    return {
        x = math.max(mx, math.min(field.w - mx, fx)),
        y = math.max(my, math.min(field.h - my, fy)),
        zoom = zoom,
    }
end

-- ---------------------------------------------------------------------------
-- Perspective mode
-- ---------------------------------------------------------------------------
--
-- A real pinhole camera above and behind the focus, looking down at the pitch.
--
-- The fixed trapezoid interpolates scale LINEARLY with depth, which is not what
-- a camera does, and it maps whatever region it is given onto the same screen
-- shape. Neither survives moving in close: magnifying a fake perspective just
-- flattens it. This mode projects the ground plane properly, so convergence is
-- a consequence of where the camera is rather than a fixed screen shape, and
-- moving closer increases perspective the way it should.
--
-- Everything on the pitch draws through camera.project, so switching modes moves
-- lines, goals, players and effects together.
camera.PERSPECTIVE = {
    -- Tuned against arcade soccer reference framing: a player near the focus
    -- occupies roughly an eighth of the screen height, close enough to read a
    -- face while still showing enough pitch to aim a pass.
    height = 150, -- world units above the pitch
    distance = 180, -- world units back from the focus
    fov = 45, -- degrees, vertical
    scale_k = 300,
}

---@return number, number, number
local function project_perspective(wx, wy, field, vp, view)
    local cfg = camera.PERSPECTIVE
    local fx = view and view.x or field.w / 2
    local fy = view and view.y or field.h / 2

    -- Pitch (x, y) becomes world (x, 0, y): y runs from the far edge toward the
    -- camera, so the camera sits at +Z beyond the focus.
    -- Zoom pulls the camera in rather than magnifying the image, so closing in
    -- strengthens the perspective instead of flattening it.
    local zoom = math.max(0.25, (view and view.zoom) or 1)
    local eye = { fx, cfg.height / zoom, fy + cfg.distance / zoom }
    local vm = mat4.lookAt(eye, { fx, 0, fy })
    local pm = mat4.perspective(cfg.fov, vp.w / vp.h, 1, 8000)
    local m = mat4.multiply(pm, vm)

    local x, y, z, w =
        m[1] * wx + m[3] * wy + m[4],
        m[5] * wx + m[7] * wy + m[8],
        m[9] * wx + m[11] * wy + m[12],
        m[13] * wx + m[15] * wy + m[16]
    if w <= 0.0001 then
        w = 0.0001 -- behind the camera; clamp rather than divide by zero
    end
    local _ = z

    return (x / w + 1) * 0.5 * vp.w, (1 - y / w) * 0.5 * vp.h, cfg.scale_k / w
end

-- The fixed whole-pitch projection: world point -> screen point + depth scale.
---@return number, number, number
local function project_fixed(wx, wy, field, vp, cfg)
    local t = wy / field.h -- 0 = far, 1 = near
    local scale = cfg.far_scale + (cfg.near_scale - cfg.far_scale) * t
    local horizon = vp.h * cfg.horizon_frac
    local bottom = vp.h * cfg.bottom_frac
    local sy = horizon + (bottom - horizon) * t
    local sx = vp.w / 2 + (wx - field.w / 2) * scale * (vp.w / field.w)
    return sx, sy, scale
end

function camera.project(wx, wy, field, vp, cfg, view)
    cfg = cfg or camera.DEFAULTS
    if camera.perspective_mode then
        return project_perspective(wx, wy, field, vp, view)
    end
    local sx, sy, scale = project_fixed(wx, wy, field, vp, cfg)
    if not view or (view.zoom or 1) <= 1 then
        return sx, sy, scale
    end

    -- Magnify in SCREEN space about the focus, which is what a longer lens
    -- does. The earlier attempt re-mapped a sub-rectangle of the pitch onto the
    -- same fixed trapezoid, which forced full convergence onto a region that
    -- should look almost rectangular -- the pitch came out as a funnel.
    --
    -- Scaling the already-projected offsets keeps the perspective structure the
    -- fixed view establishes: parallel lines stay as straight as they were, the
    -- hex grid stays even, and only the framing changes.
    local z = view.zoom
    local fx, fy = project_fixed(view.x, view.y, field, vp, cfg)
    return vp.w * 0.5 + (sx - fx) * z, vp.h * 0.5 + (sy - fy) * z, scale * z
end

return camera
