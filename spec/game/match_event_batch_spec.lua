local match_event_batch = require("game.presentation.match_event_batch")
local t = require("spec.support.runner")

---@param id string
---@param y number
---@return RollbackWrappedMatchEvent
local function tip(id, y)
    return {
        id = id,
        tick = 12,
        domain = "match/tip",
        ordinal = 1,
        payload = { kind = "tip", x = 24, y = y, player = "home_keeper" },
    }
end

---@return RollbackEventDiff
local function empty_diff()
    return { added = {}, revoked = {}, replaced = {} }
end

t.describe("rollback match-event presentation batch", function()
    t.it("does not present a tip added then revoked in one render update", function()
        local event = tip("tip/old", 220)
        local added = empty_diff()
        added.added[1] = event
        local revoked = empty_diff()
        revoked.revoked[1] = event

        t.eq(#match_event_batch.surviving({ added, revoked }), 0)
    end)

    t.it("presents only the corrected tip after added then replaced", function()
        local before = tip("tip/old", 220)
        local after = tip("tip/new", 320)
        local added = empty_diff()
        added.added[1] = before
        local replaced = empty_diff()
        replaced.replaced[1] = { before = before, after = after }

        local events = match_event_batch.surviving({ added, replaced })
        t.eq(#events, 1)
        t.eq(events[1].kind, "tip")
        t.eq(events[1].y, 320)
    end)
end)
