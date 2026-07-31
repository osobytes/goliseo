-- Broadcast-style following camera.
--
-- The fixed whole-pitch view frames the stadium; a soccer game frames the ball.
-- This holds the smoothed focus point and zoom, updated once per frame from the
-- same place view_state is updated (where the authoritative dt lives), and read
-- by the renderer.
--
-- Impure by design: it owns frame-to-frame state. The projection maths it feeds
-- stays pure in camera.lua, so the window it produces is still unit-testable.

local camera = require("game.render.camera")

local camera_follow = {}

camera_follow.config = {
    -- In perspective mode this scales the camera's distance and height rather
    -- than magnifying the image, so 1 is already the tuned framing. Values above
    -- 1 move in; below 1 pull back.
    zoom = 1.0,
    -- Seconds-ish to close most of the gap to the target. Low values feel
    -- glued to the ball and make the pitch swim; high values lag behind play.
    ease = 4.5,
    -- The camera leads slightly toward the attacking half rather than sitting
    -- centred on the ball, so a forward pass has somewhere to land on screen.
    lead = 0.18,
    -- Blend of ball position vs the controlled player. Pure ball tracking is
    -- twitchy when the ball is loose and bouncing.
    ball_weight = 0.7,
}

local state = { x = nil, y = nil }

-- Drops the smoothed state, so the next update snaps rather than sweeping the
-- camera across the pitch. Call on kickoff, goals, and renderer changes.
function camera_follow.reset()
    state.x, state.y = nil, nil
end

---@param s MatchState
---@param dt number
---@param pose CorrectionSmoothingPose?
function camera_follow.update(s, dt, pose)
    local field = s.field
    local ball = (pose and pose.ball) or s.ball
    local target_x, target_y = ball.x, ball.y

    -- Bias toward the controlled player so the camera does not abandon whoever
    -- the player is steering when the ball is knocked away.
    local controlled = s.controlled and s.players[s.controlled]
    if controlled then
        local p = (pose and pose.players[controlled.id]) or controlled.pos
        local w = camera_follow.config.ball_weight
        target_x = ball.x * w + p.x * (1 - w)
        target_y = ball.y * w + p.y * (1 - w)
        -- Lead ahead of the carrier, toward the goal they are attacking.
        local dir = (controlled.team == "home") and 1 or -1
        target_x = target_x + dir * field.w * camera_follow.config.lead * 0.5
    end

    if not state.x then
        state.x, state.y = target_x, target_y
        return
    end

    -- Exponential ease, framerate-independent.
    local k = 1 - math.exp(-camera_follow.config.ease * math.max(dt, 0))
    state.x = state.x + (target_x - state.x) * k
    state.y = state.y + (target_y - state.y) * k
end

-- The view to project through, or nil before the first update (in which case
-- the caller falls back to the whole-pitch view).
---@param field { w: number, h: number }
---@return CameraView?
function camera_follow.view(field)
    if not state.x then
        return nil
    end
    return camera.view(state.x, state.y, field, camera_follow.config.zoom)
end

return camera_follow
