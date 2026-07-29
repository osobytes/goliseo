-- Presentation-only follow-through window for outfield ball releases.
--
-- A pass/cross/shot is a one-tick `MatchEvent`; the striking leg needs to stay
-- extended for a beat after it. That beat is a *render* fact, so it lives here
-- next to `view_state` instead of becoming persistent simulation state: nothing
-- in `sim/` reads it, it never enters MatchSnapshot, and it therefore cannot
-- move a hash, a replay, or a rollback resimulation.
--
-- The window is fed from the same frame batch the renderer already draws from
-- (`match_event_batch.surviving` under rollback, `MatchState.events` offline),
-- so an event revoked before it reaches presentation never opens one. A window
-- opened by an event that is revoked afterwards ages out on its own within
-- `DURATION`, exactly like an already-spawned particle in `game.render.effects`.
--
-- A cancelled wind-up emits no release event, so it never shows follow-through.

local release_follow = {}

-- Seconds a release stays visible after the ball leaves the boot.
release_follow.DURATION = 0.15

---@type table<string, number>
local remaining = {}

-- Age the open windows by the render dt, then latch any release in this frame's
-- batch. Ageing first keeps a same-frame release at its full duration.
---@param events MatchEvent[]
---@param dt number
function release_follow.update(events, dt)
    if dt > 0 then
        for id, left in pairs(remaining) do
            local next_left = left - dt
            remaining[id] = next_left > 0 and next_left or nil
        end
    end
    for _, event in ipairs(events) do
        if (event.kind == "shot" or event.kind == "pass") and event.player then
            remaining[event.player] = release_follow.DURATION
        end
    end
end

---@param id string
---@return boolean
function release_follow.active(id)
    return (remaining[id] or 0) > 0
end

-- Drop every window (fresh match, kickoff, correction reset, replay boundary)
-- so a follow-through can never survive the timeline that produced it.
function release_follow.reset()
    remaining = {}
end

return release_follow
