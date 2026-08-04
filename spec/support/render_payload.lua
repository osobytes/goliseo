-- Build the render payload the way the match screen does, so a renderer spec
-- exercises the same boundary the game does instead of hand-assembling one.
--
-- The only thing this adds over `render_frame.build` is the renderer-owned
-- release-follow window: it is module state a spec has usually just driven with
-- `release_follow.update`, and forgetting to thread it would silently drop the
-- `kick_follow` pose from every projection test.

local render_frame = require("render.frame")
local release_follow = require("game.render.release_follow")

---@class RenderPayloadSupport
local support = {}

---@param state MatchState
---@param opts RenderFrameOptions?
---@return RenderFrame
function support.frame(state, opts)
    opts = opts or {}
    return render_frame.build(state, {
        roster = opts.roster,
        render_pose = opts.render_pose,
        events = opts.events,
        kick_follow = opts.kick_follow or release_follow.windows(),
        combat = opts.combat,
    })
end

return support
