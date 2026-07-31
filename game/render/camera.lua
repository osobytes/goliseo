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

---@class CameraWindow
---@field x number  -- world x of the window's left edge
---@field y number  -- world y of the window's far edge
---@field w number  -- window width in world units
---@field h number  -- window depth in world units

-- The sub-rectangle of the pitch that fills the trapezoid.
--
-- Zoom 1 shows the whole pitch (the original fixed view). Above 1 the camera
-- moves in and follows a focus point, the way a broadcast soccer game frames
-- the ball rather than the stadium. The window is clamped to the pitch so the
-- view never runs past the touchline -- which is what stops the framing from
-- lurching when play reaches a corner.
--
-- Pure: no love calls, no frame state. Smoothing lives in camera_follow.
---@param fx number  -- focus world x
---@param fy number  -- focus world y
---@param field { w: number, h: number }
---@param zoom number  -- 1 = whole pitch, 2 = half of it
---@return CameraWindow
function camera.window(fx, fy, field, zoom)
    zoom = math.max(1, zoom or 1)
    local w, h = field.w / zoom, field.h / zoom
    local x = math.max(0, math.min(field.w - w, fx - w / 2))
    local y = math.max(0, math.min(field.h - h, fy - h / 2))
    return { x = x, y = y, w = w, h = h }
end

-- Project a world point onto the screen.
--
-- With a `window`, the projection frames that sub-rectangle instead of the whole
-- pitch. The trapezoid maths is unchanged -- the window simply becomes the field
-- as far as the projection is concerned, so every caller that draws through this
-- (pitch lines, goals, players, effects) follows the camera for free.
---@param wx number
---@param wy number
---@param field { w: number, h: number }
---@param vp { w: number, h: number }
---@param cfg CameraConfig?
---@param window CameraWindow?
---@return number sx
---@return number sy
---@return number scale
function camera.project(wx, wy, field, vp, cfg, window)
    cfg = cfg or camera.DEFAULTS
    if window then
        wx, wy = wx - window.x, wy - window.y
        field = { w = window.w, h = window.h }
    end
    local t = wy / field.h -- 0 = far, 1 = near
    local scale = cfg.far_scale + (cfg.near_scale - cfg.far_scale) * t
    local horizon = vp.h * cfg.horizon_frac
    local bottom = vp.h * cfg.bottom_frac
    local sy = horizon + (bottom - horizon) * t
    local sx = vp.w / 2 + (wx - field.w / 2) * scale * (vp.w / field.w)
    return sx, sy, scale
end

return camera
