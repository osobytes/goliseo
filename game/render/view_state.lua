-- Per-player view state derived from frame-to-frame motion. The sim stays pure
-- (MatchPlayer has no velocity); the renderer needs cadence, lean and speed to
-- animate, so we derive them here from position deltas. Keyed by player id.
--
-- `update` is called from the match screen's update (where the authoritative dt
-- lives); the renderer only reads via `get`. Drawing without an update first
-- (e.g. the smoke test) just yields nil -> the renderer falls back to idle.

local view_state = {}

view_state.MAX_DISPLAY_SPEED = 480

---@class PlayerView
---@field px number  -- last world x
---@field py number  -- last world y
---@field speed number  -- smoothed world-units/sec
---@field phase number  -- gait accumulator (radians), advances with distance
---@field lean number  -- smoothed screen-x lean, -1..1

---@type table<string, PlayerView>
local state = {}

local function clamp(x, a, b)
    return math.max(a, math.min(b, x))
end

-- Gait cadence: radians of limb swing per world-unit travelled. Tuned so a
-- full-speed runner pumps a few times a second.
--
-- Exported because `phase` is distance-derived, and a consumer that wants the
-- distance back (a clip whose stride is measured in world units) has to divide
-- it out. Hardcoding the constant in two places is how they drift apart.
view_state.CADENCE = 0.066
local CADENCE = view_state.CADENCE

---@param players MatchPlayer[]
---@param dt number
---@param pose CorrectionSmoothingPose?
function view_state.update(players, dt, pose)
    for _, p in ipairs(players) do
        local pos = pose and pose.players[p.id] or p.pos
        local v = state[p.id]
        if not v then
            state[p.id] = { px = pos.x, py = pos.y, speed = 0, phase = 0, lean = 0 }
        elseif dt > 0 then
            local vx = (pos.x - v.px) / dt
            local vy = (pos.y - v.py) / dt
            local sp = math.min(view_state.MAX_DISPLAY_SPEED, math.sqrt(vx * vx + vy * vy))
            -- Exponential smoothing so the gait doesn't strobe on jittery steps.
            local k = clamp(dt * 8, 0, 1)
            v.speed = v.speed + (sp - v.speed) * k
            v.phase = v.phase + sp * dt * CADENCE
            local target_lean = clamp(vx / 120, -1, 1)
            v.lean = v.lean + (target_lean - v.lean) * clamp(dt * 10, 0, 1)
            v.px, v.py = pos.x, pos.y
        end
    end
end

---@param id string
---@return PlayerView?
function view_state.get(id)
    return state[id]
end

-- Drop all tracking (call when starting a fresh match so ids don't carry over).
function view_state.reset()
    state = {}
end

return view_state
