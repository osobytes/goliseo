-- Broadcast-style following camera.
--
-- The fixed whole-pitch view frames the stadium; a soccer game frames the play.
-- This holds the smoothed focus, updated once per frame from the same place
-- view_state is updated (where the authoritative dt lives), and read by the
-- renderer.
--
-- Impure by design: it owns frame-to-frame state. The projection maths it feeds
-- stays pure in camera.lua, so the view it produces is still unit-testable.

local camera = require("game.render.camera")

local camera_follow = {}

camera_follow.config = {
    -- In perspective mode this scales the camera's distance and height rather
    -- than magnifying the image, so 1 is the tuned framing. Above 1 moves in.
    zoom = 1.0,
    -- How fast the focus closes on its target. Low values feel glued to the ball
    -- and make the pitch swim; high values lag behind play.
    ease = 3.2,
    -- Blend of ball position against the controlled player. Pure ball tracking
    -- is twitchy when the ball is loose, and abandons whoever you are steering.
    ball_weight = 0.62,
    -- Seconds of ball travel to lead by. This is what makes the camera watch the
    -- play rather than chase it: the frame arrives where the ball is going.
    lead_time = 0.45,
    -- Cap on that lead, so a long clearance does not throw the camera downfield
    -- ahead of the players.
    lead_max = 190,
    -- Deadzone: how far the framing point may drift from the current focus, as
    -- a fraction of the pitch, before the camera moves at all. This is the
    -- difference between a camera glued to the ball (which makes the pitch swim
    -- under every touch) and one that holds still and only reacts when play
    -- actually goes somewhere.
    deadzone_x = 0.085,
    deadzone_y = 0.070,
    -- How far the focus stays inside each touchline, as a fraction of the pitch.
    -- Generous on purpose: a real camera only needs to not fly off the end of
    -- the pitch, and it must still be able to reach the goals.
    margin_x = 0.14,
    margin_y = 0.16,
}

local state = { x = nil, y = nil, bx = nil, by = nil, vx = 0, vy = 0 }

-- Drops the smoothed state so the next update snaps instead of sweeping across
-- the pitch. Call on kickoff and after a goal.
function camera_follow.reset()
    state.x, state.y, state.bx, state.by = nil, nil, nil, nil
    state.vx, state.vy = 0, 0
end

---@param s MatchState
---@param dt number
---@param pose CorrectionSmoothingPose?
function camera_follow.update(s, dt, pose)
    local cfg = camera_follow.config
    local ball = (pose and pose.ball) or s.ball

    -- MatchState carries no ball velocity (the sim stays pure), so derive it
    -- here the same way view_state derives player speed.
    if state.bx and dt > 0 then
        local k = math.min(dt * 9, 1)
        state.vx = state.vx + ((ball.x - state.bx) / dt - state.vx) * k
        state.vy = state.vy + ((ball.y - state.by) / dt - state.vy) * k
    end
    state.bx, state.by = ball.x, ball.y

    local lx, ly = state.vx * cfg.lead_time, state.vy * cfg.lead_time
    local lead = math.sqrt(lx * lx + ly * ly)
    if lead > cfg.lead_max then
        lx, ly = lx * cfg.lead_max / lead, ly * cfg.lead_max / lead
    end

    local target_x, target_y = ball.x + lx, ball.y + ly

    local controlled = s.controlled and s.players[s.controlled]
    if controlled then
        local p = (pose and pose.players[controlled.id]) or controlled.pos
        local w = cfg.ball_weight
        target_x = target_x * w + p.x * (1 - w)
        target_y = target_y * w + p.y * (1 - w)
    end

    if not state.x then
        state.x, state.y = target_x, target_y
        return
    end

    -- Deadzone: the camera does not chase the framing point, it keeps it inside
    -- a box. Only the amount by which the point has escaped that box becomes
    -- something to move toward, so small touches and jostling move nothing.
    local function escaped(pos, target, limit)
        local delta = target - pos
        if delta > limit then
            return target - limit
        elseif delta < -limit then
            return target + limit
        end
        return pos
    end

    local want_x = escaped(state.x, target_x, s.field.w * cfg.deadzone_x)
    local want_y = escaped(state.y, target_y, s.field.h * cfg.deadzone_y)

    -- Exponential ease, framerate-independent.
    local k = 1 - math.exp(-cfg.ease * math.max(dt, 0))
    state.x = state.x + (want_x - state.x) * k
    state.y = state.y + (want_y - state.y) * k
end

-- The view to project through, or nil before the first update (in which case
-- the caller falls back to the whole-pitch view).
---@param field { w: number, h: number }
---@return CameraView?
function camera_follow.view(field)
    if not state.x then
        return nil
    end
    local cfg = camera_follow.config
    return camera.view(state.x, state.y, field, cfg.zoom, {
        x = field.w * cfg.margin_x,
        y = field.h * cfg.margin_y,
    })
end

-- The smoothed focus, for tests and diagnostics.
---@return number?, number?
function camera_follow.focus()
    return state.x, state.y
end

return camera_follow
