-- Pure 2.5D projection. Maps a point on the flat pitch (world space) to a screen
-- point plus a depth scale, producing a perspective trapezoid: the far edge
-- (world y = 0) is higher and narrower, the near edge (world y = field.h) is
-- lower and wider. No love calls — so the projection is unit-testable.

local camera = {}

---@class CameraConfig
---@field far_scale number  -- sprite/spread scale at the far edge
---@field near_scale number  -- sprite/spread scale at the near edge
---@field horizon_frac number  -- screen-height fraction where the far edge sits
---@field bottom_frac number  -- screen-height fraction where the near edge sits

-- Tuned so the pitch is inset within the viewport (margins on all sides) rather
-- than filling the screen — the arena floats in space, like a broadcast frame.
-- Keep near_scale < 1 so even the widest (near) edge stays off the screen edges.
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
-- The focus is pulled back from the touchlines so a zoomed frame still lands on
-- the pitch rather than off the end of it.
---@param fx number
---@param fy number
---@param field { w: number, h: number }
---@param zoom number
---@return CameraView
function camera.view(fx, fy, field, zoom)
    zoom = math.max(1, zoom or 1)
    local half_w, half_h = field.w / (2 * zoom), field.h / (2 * zoom)
    return {
        x = math.max(half_w, math.min(field.w - half_w, fx)),
        y = math.max(half_h, math.min(field.h - half_h, fy)),
        zoom = zoom,
    }
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
