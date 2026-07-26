---@class MatchEventBatchModule
local match_event_batch = {}

---@param diffs RollbackEventDiff[]
---@return MatchEvent[]
function match_event_batch.surviving(diffs)
    ---@type table<string, RollbackWrappedMatchEvent>
    local frame_match_events = {}
    for _, diff in ipairs(diffs) do
        for _, event in ipairs(diff.revoked) do
            frame_match_events[event.id] = nil
        end
        for _, replacement in ipairs(diff.replaced) do
            frame_match_events[replacement.before.id] = nil
            local event = replacement.after
            if event.domain:sub(1, 6) == "match/" then
                ---@cast event RollbackWrappedMatchEvent
                frame_match_events[event.id] = event
            end
        end
        for _, event in ipairs(diff.added) do
            if event.domain:sub(1, 6) == "match/" then
                ---@cast event RollbackWrappedMatchEvent
                frame_match_events[event.id] = event
            end
        end
    end

    local event_ids = {}
    for id in pairs(frame_match_events) do
        event_ids[#event_ids + 1] = id
    end
    table.sort(event_ids)

    local events = {}
    for _, id in ipairs(event_ids) do
        events[#events + 1] = frame_match_events[id].payload
    end
    return events
end

return match_event_batch
