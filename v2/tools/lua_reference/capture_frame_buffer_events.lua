-- Capture script for the `s1_t*` rows in
-- `v2/rust/crates/gc-render/tests/fixtures/frame_buffer_lua_reference.txt`
-- (and its verbatim TypeScript copies). See that file's header comment and
-- `crates/gc-render/tests/frame_buffer_differential.rs`'s module doc for why
-- these rows exist: the original fixture's three frames (`t0`, `t37`, `t200`)
-- all carry `event_count = 0`, so the RenderFrame wire format's event section
-- had no differential coverage on either side of the wasm boundary.
--
-- HOW TO RUN
--
-- Per `v2/tools/lua_reference/README.md`: make a scratch directory, copy in
-- `core/`, `data/`, `sim/`, `render/` from the worktree root, drop in a
-- `conf.lua` that disables graphics/audio/window, copy this file in as
-- `main.lua`, and run `love .`. Capture stdout; the `LABEL<TAB>...` lines are
-- the fixture rows verbatim (the other lines are provenance for a human
-- reader and are not part of the fixture).
--
-- WHY SEED 1, NOT SEED 17
--
-- The existing fixture's match (teams nebula/orion, seed 17) produces zero
-- `MatchEvent`s in its captured window and, it turns out, stalls entirely
-- after tick ~1083 under `slot_input.neutral_match_input()` stepping (the
-- ball gets pinned to a player near a touchline and nothing further happens
-- for at least 6000 more ticks) -- not useful for finding eventful ticks.
--
-- A short survey of 22 seeds x 6000 ticks each (teams and stepping otherwise
-- identical) found seed 1 produced the richest spread: 14 distinct
-- `MatchEventKind`s across 3000 ticks, without stalling. The eight ticks
-- below were hand-picked from that seed's full event log (not included here;
-- regenerate by printing every event's fields on every nonempty tick if you
-- need to re-derive the selection) to cover multi-event frames, the aerial
-- field group (`style`/`outcome`/`jumping`/`difficulty`), the keeper field
-- group (`save_style`/`shot_type`/`keeper_state`/`keeper_depth`/`on_target`),
-- and -- the sharpest edge case found -- a `catch` at tick 2012 where
-- `keeper_depth` is exactly `0.0` (present, not absent) while `keeper_state`
-- is genuinely absent on that same event.
--
-- Roster is NOT re-captured here: `frame_buffer.encode_roster`'s output
-- depends on team/player data, not the match's RNG seed, so the existing
-- `roster` fixture row already covers this match too (verified: this
-- capture's roster block is byte-identical to it).
--
-- STILL NOT COVERED, EVEN WITH THIS CAPTURE: `combat_commit_*` events, which
-- never fire without a `CombatMatchState` (out of scope -- the differential
-- test this feeds passes `combat_state = None`, same as the rest of that
-- file); `on_target = true`; `tip`; `block`; `bicycle`; and
-- `press_commit_box_desperation` -- all either rare or entirely absent across
-- the 22-seed survey and judged not worth engineering a scenario for.

local match = require("sim.match")
local slot_input = require("sim.slot_input")
local teams = require("data.teams")
local frame = require("render.frame")
local frame_buffer = require("render.frame_buffer")

local function dump(label, buf)
    local parts = {}
    for i = 1, #buf do
        parts[#parts + 1] = string.format("%.17g", buf[i])
    end
    print(label .. "\t" .. #buf .. "\t" .. table.concat(parts, ","))
end

-- Cumulative tick targets (from kickoff), ascending. Each is stepped to from
-- the previous target -- not re-simulated from scratch -- so this is one
-- continuous match, matching how the differential test consumes it.
local TARGET_TICKS = { 226, 294, 374, 393, 1236, 1466, 2012, 2356 }

function love.load()
    local s = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        seed = 1,
    })
    local roster = frame.roster(s)

    local target_index = 1
    local next_target = TARGET_TICKS[target_index]
    local max_tick = TARGET_TICKS[#TARGET_TICKS]
    for tick = 1, max_tick do
        match.step(s, 1 / 60, slot_input.neutral_match_input())
        if tick == next_target then
            local label = "s1_t" .. tostring(tick)
            local built = frame.build(s, { roster = roster })
            assert(#s.events > 0, label .. ": expected at least one MatchEvent")
            dump(label, frame_buffer.encode(built))
            target_index = target_index + 1
            next_target = TARGET_TICKS[target_index]
        end
    end
    love.event.quit(0)
end
