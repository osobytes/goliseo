-- Capture script for
-- `v2/rust/crates/gc-sim/tests/fixtures/session_legacy_ordinary_lua_reference.txt`,
-- feeding `crates/gc-sim/tests/session_legacy_differential.rs`.
--
-- Unlike `match_step_ai_ai_lua_reference.txt` (captured with
-- `human_controlled = false`, i.e. no player at all takes the human-input
-- branch), this captures the shape `game/screens/match.lua`'s `Match:restart`
-- actually builds for an ORDINARY product match: no `input_ownership` (so
-- `sim.match` stays on the legacy path) and no `human_controlled` override
-- either, so it defaults `true` -- one player (`s.controlled`, starting as
-- the kickoff taker) takes the human-input branch every tick, fed
-- `slot_input.neutral_match_input()` every tick (an idle/AFK local player,
-- the same scenario `crates/gc-wasm/src/session.rs`'s own
-- `a_full_match_on_an_idle_local_wire_is_not_a_permanent_scoreless_zero_event_stalemate`
-- regression test drives). Every other player runs `sim.match`'s inline AI
-- branch, unconditionally.
--
-- Same field layout as `match_step_ai_ai_lua_reference.txt` (see that
-- fixture's own header in `match_differential.rs`): tick, ball.x, ball.y,
-- ball_vel.x, ball_vel.y, ball_z, ball_vz, owner (-1 for loose), score.home,
-- score.away, rng, then players[i].pos.{x,y} for i = 1..10 -- floats via
-- `%.17g`.
--
-- HOW TO RUN: per `v2/tools/lua_reference/README.md` -- copy `core/`,
-- `sim/`, `data/` into a scratch dir, drop in a `conf.lua` disabling
-- graphics/audio/window, copy this file in as `main.lua`, run `love .`,
-- capture stdout.
--
-- TICK COUNT: the full 7200 (120 seconds), matching
-- `match_step_ai_ai_lua_reference.txt` -- a complete match, not a pinned
-- excerpt. Capturing this took well under a second (see this script's own
-- run notes), so there was no reason to pin a shorter run.
local match = require("sim.match")
local slot_input = require("sim.slot_input")
local teams = require("data.teams")

local TICKS = 7200

local function dump_row(tick, s)
    local parts = {}
    parts[#parts + 1] = string.format("%d", tick)
    parts[#parts + 1] = string.format("%.17g", s.ball.x)
    parts[#parts + 1] = string.format("%.17g", s.ball.y)
    parts[#parts + 1] = string.format("%.17g", s.ball_vel.x)
    parts[#parts + 1] = string.format("%.17g", s.ball_vel.y)
    parts[#parts + 1] = string.format("%.17g", s.ball_z)
    parts[#parts + 1] = string.format("%.17g", s.ball_vz)
    parts[#parts + 1] = string.format("%d", s.owner or -1)
    parts[#parts + 1] = string.format("%d", s.score.home)
    parts[#parts + 1] = string.format("%d", s.score.away)
    parts[#parts + 1] = string.format("%d", s.rng)
    for i = 1, 10 do
        local p = s.players[i]
        parts[#parts + 1] = string.format("%.17g", p.pos.x)
        parts[#parts + 1] = string.format("%.17g", p.pos.y)
    end
    print(table.concat(parts, "\t"))
end

function love.load()
    local s = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        seed = 5,
    })
    dump_row(0, s)
    local input = slot_input.neutral_match_input()
    for tick = 1, TICKS do
        match.step(s, 1 / 60, input)
        dump_row(tick, s)
    end
    love.event.quit(0)
end
