-- Turns the online driver's raw tick outputs into the stable presentation
-- timeline the match renderer already consumes.
--
-- `sim.rollback_playable_lab` owns an event timeline because it owns its own
-- rollback session. The online driver deliberately does not: it publishes
-- authority, corrections, and outputs, and leaves presentation to the game
-- layer. This module is that layer, and it is the only new presentation
-- machinery the online flow adds — everything downstream (speculative
-- add/revoke/replace, confirmed exactly-once consumers, VFX, audio, camera,
-- HUD, replay, statistics) is the shipped #113/#147 path, unchanged.
--
-- It is pure: no transport, no love, no clock. It reads a `MatchDriverBatch`
-- and the driver's snapshot lookups, and returns a batch in exactly the shape
-- `game.screens.match` already knows how to consume.
--
-- # Append versus replacement
--
-- The driver emits corrected outputs (from its single reconciliation) before
-- this step's fresh outputs, both in session-tick space. An output at or below
-- the highest tick already applied is therefore the head of a correction run
-- that replaces the complete speculative tail; anything above it is an ordinary
-- append. `sim.rollback_events` asserts both shapes, so a mis-split fails loudly
-- here rather than silently publishing a cue twice.
--
-- Confirmation runs after the correction run and again at the end of the batch,
-- mirroring the laboratory: freeing the oldest event slot before the next
-- predicted output is appended is the difference between a supported correction
-- and a false unconfirmed-window terminal at a long delay.

local match_driver = require("game.online.match_driver")
local rollback_events = require("sim.rollback_events")
local rollback_input_history = require("sim.rollback_input_history")

---@class OnlineMatchPresentation
---@field _events RollbackEventTimeline
---@field _first integer -- The frozen first input tick; session tick zero.
---@field _applied integer -- Highest session tick applied to the timeline, -1 when none.
---@field _status RollbackPlayableLabStatus

---@class OnlineMatchPresentationModule
local match_presentation = {}

---@param value any
---@return any
local function copy_value(value)
    if type(value) ~= "table" then
        return value
    end
    local result = {}
    for key, child in pairs(value) do
        result[copy_value(key)] = copy_value(child)
    end
    return result
end

---@param initial_snapshot MatchSnapshot -- Canonical boundary zero.
---@param first_input_tick integer
---@param max_unconfirmed_ticks integer?
---@return OnlineMatchPresentation
function match_presentation.new(initial_snapshot, first_input_tick, max_unconfirmed_ticks)
    return {
        _events = rollback_events.new(
            initial_snapshot,
            max_unconfirmed_ticks or rollback_input_history.ROLLBACK_WINDOW_TICKS
        ),
        _first = first_input_tick,
        _applied = -1,
        _status = "active",
    }
end

---@return RollbackPlayableLabBatch
local function new_batch()
    return {
        outputs = {},
        event_diffs = {},
        confirmed_steps = {},
        corrections = {},
        status = "active",
    }
end

---@param presentation OnlineMatchPresentation
---@param driver MatchDriver
---@param output RollbackTickOutput
---@return RollbackEventStepInput
local function event_step(presentation, driver, output)
    local lookup = match_driver.snapshot(driver, output.end_boundary + presentation._first)
    assert(
        lookup.status == "present" or lookup.status == "retained",
        "the online presentation needs a retained boundary snapshot"
    )
    return {
        output = output,
        snapshot = assert(lookup.snapshot, "retained boundary snapshot is missing"),
    }
end

---@param presentation OnlineMatchPresentation
---@param batch RollbackPlayableLabBatch
---@param from integer
---@param through integer
---@param steps RollbackEventStepInput[]
---@return boolean
local function apply_steps(presentation, batch, from, through, steps)
    local diff, err, code = rollback_events.apply(presentation._events, from, through, steps)
    if diff == nil then
        assert(code == "unconfirmed_window_exceeded", err or "online event application failed")
        presentation._status = "unconfirmed_window_exceeded"
        return false
    end
    batch.event_diffs[#batch.event_diffs + 1] = copy_value(diff)
    return true
end

---@param presentation OnlineMatchPresentation
---@param batch RollbackPlayableLabBatch
---@param confirmed_output_tick integer -- Input-tick space.
---@return boolean
local function publish_confirmation(presentation, batch, confirmed_output_tick)
    if rollback_events.diagnostics(presentation._events).status ~= "active" then
        presentation._status = "unconfirmed_window_exceeded"
        return false
    end
    -- Never confirm past what has been applied. The driver's confirmation
    -- ceiling covers the whole batch, but the mid-batch confirmation after a
    -- correction runs before this batch's fresh appends exist.
    local session_tick =
        math.min(confirmed_output_tick - presentation._first, presentation._applied)
    for _, step in ipairs(rollback_events.confirm(presentation._events, session_tick)) do
        batch.confirmed_steps[#batch.confirmed_steps + 1] = copy_value(step)
    end
    return true
end

---@param from integer
---@param through integer
---@param corrected_through integer
---@return RollbackPlayableCorrection
local function correction_view(from, through, corrected_through)
    -- The driver reports how many corrections and rollbacks a step contained but
    -- not the reconciliation record itself, so the causal tick is reported as
    -- the first replaced tick. That is exactly what presentation uses it for:
    -- the boundary replay must be truncated from.
    return {
        causal_tick = from,
        restored_boundary = from,
        old_present_boundary = through + 1,
        new_present_boundary = corrected_through + 1,
        replaced_from_tick = from,
        replaced_through_tick = through,
        corrected_from_tick = from,
        corrected_through_tick = corrected_through,
        old_present_hash = nil,
        new_present_hash = nil,
        first_difference = nil,
    }
end

-- Consume one driver batch. `driver` is read only for its retained boundary
-- snapshots and its confirmation ceiling; nothing here can move the simulation.
---@param presentation OnlineMatchPresentation
---@param driver MatchDriver
---@param driver_batch MatchDriverBatch
---@return RollbackPlayableLabBatch
function match_presentation.consume(presentation, driver, driver_batch)
    local batch = new_batch()
    if presentation._status ~= "active" then
        batch.status = presentation._status
        return batch
    end
    local confirmed_output_tick = match_driver.diagnostics(driver).confirmed_output_tick
    local index = 1
    local outputs = driver_batch.outputs
    while index <= #outputs do
        local output = outputs[index]
        batch.outputs[#batch.outputs + 1] = copy_value(output)
        if output.tick > presentation._applied then
            if
                not apply_steps(presentation, batch, output.tick, output.tick, {
                    event_step(presentation, driver, output),
                })
            then
                batch.status = presentation._status
                return batch
            end
            presentation._applied = output.tick
            index = index + 1
        else
            -- A correction run: contiguous corrected outputs replacing the whole
            -- speculative tail from this tick through everything already applied.
            local from = output.tick
            local through = presentation._applied
            ---@type RollbackEventStepInput[]
            local steps = {}
            local expected = from
            -- Bounded by `through`: the corrected run replaces the speculative
            -- tail and stops there. A fresh output for the next tick may follow
            -- it in the same driver batch, and swallowing that into the
            -- replacement would extend it past the interval being replaced.
            while index <= #outputs and expected <= through and outputs[index].tick == expected do
                if expected ~= from then
                    batch.outputs[#batch.outputs + 1] = copy_value(outputs[index])
                end
                steps[#steps + 1] = event_step(presentation, driver, outputs[index])
                expected = expected + 1
                index = index + 1
            end
            if not apply_steps(presentation, batch, from, through, steps) then
                batch.status = presentation._status
                return batch
            end
            -- A correction may legally shorten the tail at full time, so the
            -- applied ceiling follows the corrected run rather than the replaced
            -- one; claiming a tick the timeline no longer holds would break the
            -- next append.
            local corrected_through = from + #steps - 1
            presentation._applied = corrected_through
            batch.corrections[#batch.corrections + 1] =
                correction_view(from, through, corrected_through)
            if not publish_confirmation(presentation, batch, confirmed_output_tick) then
                batch.status = presentation._status
                return batch
            end
        end
    end
    publish_confirmation(presentation, batch, confirmed_output_tick)
    batch.status = presentation._status
    return batch
end

---@param presentation OnlineMatchPresentation
---@return RollbackEventsDiagnostics
function match_presentation.diagnostics(presentation)
    return rollback_events.diagnostics(presentation._events)
end

---@param presentation OnlineMatchPresentation
---@return RollbackPlayableLabStatus
function match_presentation.status(presentation)
    return presentation._status
end

return match_presentation
