-- Broadcast-style following camera.
--
-- The fixed whole-pitch view frames the stadium; a soccer game frames the play.
-- This holds the smoothed focus, updated once per frame from the same place
-- view_state is updated (where the authoritative dt lives), and read by the
-- renderer.
--
-- Three jobs, deliberately kept separate because an earlier version folded them
-- together and they fought:
--
--   1. TRACK   an eased focus on the ball/carrier, with a deadzone so small
--              touches move nothing.
--   2. LOOK    an offset ahead along travel, so a player running with the ball
--              can see who is in front of them rather than what they just left.
--   3. KEEP    a hard cap on how far the ball may sit from the focus, so it is
--              always on screen and near the middle whatever 1 and 2 decide.
--
-- Folding LOOK into the tracking target meant the deadzone swallowed it (the
-- deadzone deliberately lets the focus trail, the look-ahead deliberately pushes
-- it forward). Applying LOOK to the eased result instead gives both their full
-- effect, and KEEP is a backstop over the top.
--
-- Impure by design: it owns frame-to-frame state. The projection maths it feeds
-- stays pure in camera.lua, so the view it produces is still unit-testable.

local camera = require("game.render.camera")

local camera_follow = {}

camera_follow.config = {
    -- In perspective mode this scales the camera's distance and height rather
    -- than magnifying the image, so 1 is the tuned framing. Above 1 moves in.
    zoom = 1.0,
    -- How fast the focus closes on its target.
    ease = 3.2,
    -- Blend of ball position against the controlled player. Pure ball tracking
    -- is twitchy when the ball is loose, and abandons whoever you are steering.
    ball_weight = 0.62,

    -- 1. TRACK -----------------------------------------------------------
    -- How far the framing point may drift from the focus before the camera
    -- moves at all, as a fraction of the pitch.
    deadzone_x = 0.085,
    deadzone_y = 0.070,
    -- The deadzone exists to absorb jitter, not to hold the camera back from
    -- real play. Above this ball speed it collapses, so a moving ball is tracked
    -- tightly and the look-ahead below gets its full effect instead of being
    -- cancelled out by the trail the deadzone deliberately introduces.
    deadzone_release_speed = 180,
    deadzone_release = 0.85, -- how much of the box speed can remove

    -- 2. LOOK ------------------------------------------------------------
    -- Offset ahead along travel, so the carrier can see what they are running
    -- into. Below `look_min_speed` there is no offset at all, which keeps a
    -- jostling or slowly drifting ball from sliding the camera around.
    look_min_speed = 55,
    look_gain = 0.55, -- seconds of travel to look ahead by
    look_max = 210, -- world units
    look_ease = 2.4, -- how fast the offset itself follows, kept slow and smooth

    -- 3. KEEP ------------------------------------------------------------
    -- Hard cap on the ball's distance from the focus. Larger than the deadzone
    -- so normal play is governed by TRACK, but the ball can never leave frame.
    ball_keep_x = 0.115,
    ball_keep_y = 0.095,

    -- How far the focus stays inside each touchline, as a fraction of the pitch.
    -- Generous on purpose: a real camera only needs to not fly off the end of
    -- the pitch, and it must still be able to reach the goals.
    margin_x = 0.14,
    margin_y = 0.16,
}

local state = { x = nil, y = nil, bx = nil, by = nil, vx = 0, vy = 0, lx = 0, ly = 0 }

-- Drops the smoothed state so the next update snaps instead of sweeping across
-- the pitch. Call on kickoff and after a goal.
function camera_follow.reset()
    state.x, state.y, state.bx, state.by = nil, nil, nil, nil
    state.vx, state.vy, state.lx, state.ly = 0, 0, 0, 0
end

local function clamp(v, lo, hi)
    return math.max(lo, math.min(hi, v))
end

---@param s MatchState
---@param dt number
---@param pose CorrectionSmoothingPose?
function camera_follow.update(s, dt, pose)
    local cfg = camera_follow.config
    local field = s.field
    local ball = (pose and pose.ball) or s.ball

    -- MatchState carries no ball velocity (the sim stays pure), so derive it
    -- here the same way view_state derives player speed.
    if state.bx and dt > 0 then
        local k = math.min(dt * 9, 1)
        state.vx = state.vx + ((ball.x - state.bx) / dt - state.vx) * k
        state.vy = state.vy + ((ball.y - state.by) / dt - state.vy) * k
    end
    state.bx, state.by = ball.x, ball.y

    -- 1. TRACK
    local target_x, target_y = ball.x, ball.y
    local controlled = s.controlled and s.players[s.controlled]
    if controlled then
        local p = (pose and pose.players[controlled.id]) or controlled.pos
        local w = cfg.ball_weight
        target_x = ball.x * w + p.x * (1 - w)
        target_y = ball.y * w + p.y * (1 - w)
    end

    if not state.x then
        state.x, state.y = target_x, target_y
    else
        -- The camera keeps the framing point inside a box rather than chasing
        -- it, so only the amount by which it has escaped becomes something to
        -- move toward.
        local function escaped(pos, target, limit)
            local delta = target - pos
            if delta > limit then
                return target - limit
            elseif delta < -limit then
                return target + limit
            end
            return pos
        end
        local moving = math.sqrt(state.vx * state.vx + state.vy * state.vy)
        local release = 1 - math.min(moving / cfg.deadzone_release_speed, 1) * cfg.deadzone_release
        local want_x = escaped(state.x, target_x, field.w * cfg.deadzone_x * release)
        local want_y = escaped(state.y, target_y, field.h * cfg.deadzone_y * release)
        local k = 1 - math.exp(-cfg.ease * math.max(dt, 0))
        state.x = state.x + (want_x - state.x) * k
        state.y = state.y + (want_y - state.y) * k
    end

    -- 2. LOOK: an offset ahead along travel, eased separately so it grows and
    -- decays smoothly instead of snapping when the ball changes hands.
    local speed = math.sqrt(state.vx * state.vx + state.vy * state.vy)
    local want_lx, want_ly = 0, 0
    if speed > cfg.look_min_speed then
        -- Subtracting the threshold rather than testing against it means the
        -- offset grows from zero instead of popping in at the boundary.
        local reach = math.min((speed - cfg.look_min_speed) * cfg.look_gain, cfg.look_max)
        want_lx, want_ly = state.vx / speed * reach, state.vy / speed * reach
    end
    local lk = 1 - math.exp(-cfg.look_ease * math.max(dt, 0))
    state.lx = state.lx + (want_lx - state.lx) * lk
    state.ly = state.ly + (want_ly - state.ly) * lk
end

-- The focus actually framed: tracking plus look-ahead, with the ball kept close.
---@return number?, number?
function camera_follow.focus()
    if not state.x then
        return nil, nil
    end
    return state.x + state.lx, state.y + state.ly
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
    local fx, fy = state.x + state.lx, state.y + state.ly

    -- 3. KEEP: whatever tracking and look-ahead wanted, the ball stays on screen
    -- and near the middle. This is a backstop, not the main mechanism -- it only
    -- binds when play outruns the ease or the look-ahead overreaches.
    if state.bx then
        fx = clamp(fx, state.bx - field.w * cfg.ball_keep_x, state.bx + field.w * cfg.ball_keep_x)
        fy = clamp(fy, state.by - field.h * cfg.ball_keep_y, state.by + field.h * cfg.ball_keep_y)
    end

    return camera.view(fx, fy, field, cfg.zoom, {
        x = field.w * cfg.margin_x,
        y = field.h * cfg.margin_y,
    })
end

return camera_follow
