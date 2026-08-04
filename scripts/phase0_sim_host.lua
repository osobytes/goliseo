-- Phase 0 of the Babylon migration spike: prove the simulation runs OUTSIDE LOVE.
--
-- This is the load-bearing experiment. If `sim/` cannot run under a bare Lua
-- interpreter and reproduce the frozen determinism hashes, then no renderer
-- choice matters -- the portable-sim-core architecture the whole migration
-- rests on does not hold, and we should find that out in an afternoon rather
-- than after committing to a rewrite.
--
-- Deliberately a plain Lua 5.1 script with no LOVE, no shim and no stubs. Any
-- accidental engine dependency in sim/ core/ data/ shows up here as a hard
-- error, which is exactly what we want to learn.
--
-- Lua 5.1 specifically, NOT 5.4: Lua 5.3 introduced a distinct integer subtype,
-- which changes numeric semantics against the doubles-only model this codebase
-- is written for. Running the sim on 5.4 would silently change arithmetic and
-- invalidate the hash comparison this script exists to make.
--
--   docker run --rm -v "$PWD:/app" -w /app nickblah/lua:5.1-alpine \
--     lua scripts/phase0_sim_host.lua

package.path = "./?.lua;" .. package.path

local function heading(text)
    print("")
    print("== " .. text)
end

-- 1. Does the pure layer load at all outside the engine?
heading("loading sim/ core/ data/ render/ under bare Lua " .. _VERSION)

local modules = {
    "core.vec2",
    "core.fnv1a64",
    "core.deterministic_math",
    "core.rng",
    "data.teams",
    "data.players",
    "data.species",
    "sim.fixed_clock",
    "sim.match",
    "sim.match_snapshot",
    "sim.bot",
    "sim.metrics",
    "sim.headless",
    -- The sim-to-renderer boundary. It is pure by construction, and this is the
    -- step that proves it: a stray `love` reference in the payload builder or in
    -- the pose/identity derivations it calls fails here, not in a browser.
    "render.identity",
    "render.player_pose",
    "render.frame",
}

for _, name in ipairs(modules) do
    local ok, err = pcall(require, name)
    if not ok then
        print(("FAIL  %s\n      %s"):format(name, tostring(err)))
        os.exit(1)
    end
    print("ok    " .. name)
end

-- 2. THE CORRECTNESS TEST: does the frozen determinism contract still hold?
--
-- Loading proves there are no engine imports. This proves something much
-- stronger -- that the simulation computes bit-identical results on a VM it has
-- never run on. The expected hashes are the same ones native LuaJIT, Chrome and
-- Firefox already agree on, so a match here means a fourth independent runtime
-- joins that set without a single line of sim/ changing.
heading("frozen determinism contract")

local evidence = require("sim.determinism_evidence")
local verified = evidence.verify()
local frozen = require("data.omp1_determinism")

local hash_ok = verified.final_hash == frozen.expected_final_hash
print(("final hash       %s"):format(tostring(verified.final_hash)))
print(("expected         %s"):format(tostring(frozen.expected_final_hash)))
print(("sequence digest  %s"):format(tostring(verified.sequence_digest)))
print(("ticks            %d"):format(verified.ticks))
print(("verdict          %s"):format(hash_ok and "MATCH" or "DIVERGED"))
if not hash_ok then
    print("")
    print("PHASE0 FAILED: the simulation is loadable here but not deterministic here.")
    os.exit(1)
end

-- 3. Performance, measured with the bot driving so the sim does realistic work.
--
-- NOTE the bot is deliberately NOT part of the determinism contract above, and
-- its state hash does differ between LuaJIT and PUC Lua -- almost certainly
-- `pairs()` iteration order, which the Lua spec leaves unspecified. That is
-- fine for unattended play but would matter if bot-driven matches ever needed
-- to be reproducible across VMs. Recorded input tapes, which is what the
-- contract and the netcode actually use, are unaffected.
heading("stepping a match (performance only)")

local fixed_clock = require("sim.fixed_clock")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local bot = require("sim.bot")
local teams = require("data.teams")

local DT = fixed_clock.TICK_SECONDS
local SEED = 20260803
local TICKS = 600

local state = match.new({
    home = teams.nebula,
    away = teams.orion,
    field = { w = 960, h = 540 },
    human_controlled = true,
    seed = SEED,
    duration = 600,
})
local brain = bot.new({ seed = SEED })
local clock = fixed_clock.new()

local started = os.clock()
for _ = 1, TICKS do
    if state.finished then
        break
    end
    local input = bot.input(brain, state, DT)
    fixed_clock.step(clock, input, function(_, tick_input)
        match.step(state, DT, tick_input)
        return not state.finished
    end)
end
local elapsed = os.clock() - started

local hash = match_snapshot.hash(match_snapshot.capture(state))
print(
    ("ran %d ticks in %.1f ms (%.4f ms/tick)"):format(TICKS, elapsed * 1000, elapsed * 1000 / TICKS)
)
print("state hash: " .. tostring(hash))

-- 3. The budget question. Rollback re-simulates several ticks inside one
-- rendered frame, so the number that matters is not one tick but a worst-case
-- resim burst against the 8 ms update budget from omp0_acceptance.
heading("rollback budget")

local per_tick_ms = elapsed * 1000 / TICKS
local burst = fixed_clock.MAX_TICKS_PER_UPDATE
print(("per tick        %.4f ms"):format(per_tick_ms))
print(("%d-tick resim    %.4f ms"):format(burst, per_tick_ms * burst))
print(
    ("update budget    8.0000 ms  -> %s"):format(
        per_tick_ms * burst <= 8 and "WITHIN BUDGET" or "OVER BUDGET"
    )
)

print("")
print("PHASE0 OK: the simulation runs outside LOVE.")
