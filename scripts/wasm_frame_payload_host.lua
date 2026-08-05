-- The simulation side of the wasm render-frame payload boundary (#332).
--
-- `scripts/phase0_sim_host.lua` proved the simulation RUNS off LOVE. This makes
-- it OBSERVABLE: it owns a live match, advances it, rolls it back, and hands
-- out one flat `render.frame_buffer` block per rendered frame. The Rust host
-- (`wasm/sim-host/src/main.rs`) calls exactly the four functions below and
-- copies the block into linear memory; JavaScript reads it as one typed-array
-- view. No other Lua surface is exposed across the boundary.
--
-- WHY THE ROLLBACK LIVES HERE AND NOT IN JS. `sim.match_snapshot` capture and
-- restore never leave this file. `rollback` restores a snapshot from up to
-- `sim.fixed_clock.MAX_TICKS_PER_UPDATE` ticks ago and replays the recorded
-- inputs, all inside one call -- so an eight-tick resim burst costs the
-- boundary the same single crossing a quiet frame does. If JS could see a
-- snapshot, the per-tick marshalling this whole design exists to avoid would be
-- back, eight times per frame.
--
-- Deliberately a plain Lua 5.1 module with no LOVE and no stubs, for the same
-- reason the phase-0 probe is: an accidental engine dependency in the payload
-- path has to fail here rather than in a browser.

package.path = "./?.lua;" .. package.path

local bot = require("sim.bot")
local fixed_clock = require("sim.fixed_clock")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local render_frame = require("render.frame")
local frame_buffer = require("render.frame_buffer")
local teams = require("data.teams")

---@class WasmFramePayloadHost
local host = {}

-- Same fixture the phase-0 probe steps, so the tick cost measured beside the
-- payload cost is comparable with the number #327 already published.
host.SEED = 20260803
host.FIELD = { w = 960, h = 540 }
host.DURATION = 600

-- The rollback depth the fixed clock allows inside one rendered frame. The
-- worst case this host is built to measure, not a tuning knob.
host.MAX_RESIM_TICKS = fixed_clock.MAX_TICKS_PER_UPDATE

---@class WasmFramePayloadEntry
---@field input MatchInput
---@field snapshot MatchSnapshot -- Taken BEFORE `input` was applied.

---@class WasmFramePayloadState
---@field state MatchState
---@field brain BotState
---@field roster RenderFrameRoster
---@field words number[] -- Reused frame block; never reallocated per frame.
---@field history WasmFramePayloadEntry[] -- Newest last.
---@field tick integer
local live = nil

---@return WasmFramePayloadState
local function require_live()
    return assert(live, "boot() has not been called")
end

-- Start a fresh match. Idempotent by construction: a second call discards the
-- first match entirely, so a harness can re-boot without leaking state.
---@return integer count -- Roster slots.
function host.boot()
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = host.FIELD,
        human_controlled = true,
        seed = host.SEED,
        duration = host.DURATION,
    })
    live = {
        state = state,
        brain = bot.new({ seed = host.SEED }),
        roster = render_frame.roster(state),
        words = {},
        history = {},
        tick = 0,
    }
    return live.roster.count
end

---@param current WasmFramePayloadState
---@return MatchInput
local function step_once(current)
    -- `capture_owned` / `restore_owned` deliberately: that is the pair
    -- `sim.rollback_session` uses, so the cost measured here is the cost
    -- rollback actually pays rather than the slower validating variant.
    local snapshot = match_snapshot.capture_owned(current.state)
    local input = bot.input(current.brain, current.state, fixed_clock.TICK_SECONDS)
    match.step(current.state, fixed_clock.TICK_SECONDS, input)
    current.tick = current.tick + 1

    -- Keep exactly the depth a rollback can reach back to. An unbounded history
    -- would be a slow leak in a module that is meant to run for a whole match.
    local history = current.history
    history[#history + 1] = { input = input, snapshot = snapshot }
    while #history > host.MAX_RESIM_TICKS do
        table.remove(history, 1)
    end
    return input
end

-- Advance the simulation by `ticks` fixed ticks.
---@param ticks integer
---@return integer tick
function host.advance(ticks)
    local current = require_live()
    for _ = 1, ticks do
        if current.state.finished then
            break
        end
        step_once(current)
    end
    return current.tick
end

-- Restore the state from `ticks` ticks ago and re-simulate it forward with the
-- inputs that were recorded then. This is the rollback burst, entire, inside
-- the module. The replayed inputs are the recorded ones rather than freshly
-- sampled, so the resim is a genuine re-execution, and `host.hash` before and
-- after must agree.
--
-- It deliberately does NOT hash. Hashing means a full canonical encode of the
-- match state, which costs several times the resim it would be measuring --
-- the first draft of this function returned a hash and made an eight-tick
-- burst look like 10 ms of rollback when 7.5 ms of that was the diagnostic.
-- The caller asks for a hash when it wants one, outside the timed region.
---@param ticks integer
---@return integer tick
function host.rollback(ticks)
    local current = require_live()
    local history = current.history
    assert(ticks >= 1 and ticks <= #history, "rollback depth outside the retained history")

    local first = #history - ticks + 1
    current.state = match_snapshot.restore_owned(history[first].snapshot)
    -- The tick counter rewinds with the state and steps forward with the resim.
    -- It reported a stale value before: harmless while every caller discarded
    -- it, and exactly the kind of thing that stops being harmless the first
    -- time someone believes it.
    current.tick = current.tick - ticks
    for index = first, #history do
        match.step(current.state, fixed_clock.TICK_SECONDS, history[index].input)
        current.tick = current.tick + 1
    end
    -- The retained snapshots stay valid across the resim precisely because the
    -- resim reproduces the states they were taken from, so history is untouched.
    return current.tick
end

---@return string hash
function host.hash()
    local current = require_live()
    return match_snapshot.hash(match_snapshot.capture_owned(current.state))
end

-- Build and serialise one rendered frame. This is the whole per-frame cost the
-- boundary pays, and it is deliberately one function so the host can time it
-- without timing anything else.
---@return number[] words
function host.frame()
    local current = require_live()
    local frame = render_frame.build(current.state, { roster = current.roster })
    return frame_buffer.encode(frame, current.words)
end

---@return number[] words
---@return string strings
function host.roster()
    local current = require_live()
    return frame_buffer.encode_roster(current.roster)
end

return host
