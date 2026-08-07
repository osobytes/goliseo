-- Lua reference capture: an ordinary match in which EVERY player is driven by
-- an AI, including the one that takes the human-input branch.
--
-- WHY THIS EXISTS, and why the AFK capture beside it was not enough.
--
-- `capture_session_legacy_ordinary_match.lua` feeds
-- `slot_input.neutral_match_input()` every tick -- an idle player who never
-- presses anything. Its 7,201 bit-exact rows are real, and they prove the
-- inline match AI, the ball physics it produces, the keeper logic and the
-- whole `match.step` legacy branch. What they do not touch, at all, is the
-- code that runs when a player actually plays: shooting, charging a shot,
-- passing, lofting, dashing, dodging, jockeying, sprinting, and the aerial
-- strike intents. Every one of those is a branch of `sim/match.lua` that has
-- never been compared against anything, and "the ball physics feel wrong"
-- is a report about precisely that surface.
--
-- The controlled slot here is driven by `sim/bot.lua` instead. That is
-- deliberate rather than convenient:
--
--   * It is a REAL input source, not a scripted tape. It charges shots and
--     releases them, queues lobs, dashes, jukes as a carrier, and picks
--     outlets -- so the branches above are exercised in the combinations the
--     game actually produces, not in the combinations a human author thought
--     to write down.
--   * It is deterministic. `bot.new` seeds its own PRNG stream
--     (`seed * 7919 + 17`) and never touches the match's, so the same seed
--     replays identically and the capture is reproducible.
--   * It already exists on both sides (`sim/bot.lua` <-> `crates/gc-sim/
--     src/bot.rs`), so the differential does not need a second AI written
--     twice. It also means this capture tests `bot.rs` itself, which until
--     now had NO differential coverage at all -- it is reachable only from
--     `headless.rs`, and perturbing its `REACTION` constant changed nothing
--     in any existing test.
--
-- THE WIRE ROUND TRIP IS NOT OPTIONAL HERE. `crates/gc-wasm/src/session.rs`
-- does not hand a `MatchInput` to `match.step`; it encodes a canonical
-- `input_frame` wire string, decodes it, validates it, and dequantizes slot
-- `home_1` back into a `MatchInput`. That trip is LOSSY -- move axes are
-- quantized -- and with neutral input the loss is exactly zero, so the AFK
-- capture cannot see it. Under a bot that steers with real analogue
-- directions it is the difference between a player running where they meant
-- to and running somewhere else, so this capture routes every tick through
-- the same encode -> decode -> validate -> to_match_input path the browser
-- runs.
--
-- HOW TO RE-RUN (see v2/tools/lua_reference/README.md for the general
-- recipe): copy `core/`, `sim/`, `data/` into a scratch dir, drop in a
-- `conf.lua` disabling graphics/audio/window, copy this file in as
-- `main.lua`, run `love .`, and capture stdout. It takes well under a
-- second, so the capture is a full match rather than a pinned excerpt.
--
-- The row layout is IDENTICAL to
-- `capture_session_legacy_ordinary_match.lua`'s, so the Rust side can share
-- its parser: tick, six ball fields, owner, both scores, the match RNG, then
-- (x, y) per player for all ten.

local match = require("sim.match")
local bot = require("sim.bot")
local slot_input = require("sim.slot_input")
local input_frame = require("sim.input_frame")
local teams = require("data.teams")

local TICKS = 7200
local DT = 1 / 60
-- The match seed and the bot seed are deliberately DIFFERENT. Sharing one
-- would couple the bot's decision stream to the match's own RNG through the
-- seed alone, and a divergence in either would be harder to attribute.
local MATCH_SEED = 5
local BOT_SEED = 11

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

-- One tick of the browser's own input path, driven by the bot rather than by
-- a keyboard. Mirrors `Session::step` exactly: the bot's `MatchInput` becomes
-- a quantized slot sample, that sample is encoded to the canonical wire,
-- decoded, validated, and dequantized back into a `MatchInput` -- which is
-- what the simulation finally steps on. The value that comes out is NOT the
-- value that went in, and that is the point.
local function tick_input(b, s, tick)
    local raw = bot.input(b, s, DT)
    -- Slot 1 is `home_1`, the slot `Session::step` reads back out. The other
    -- seven must still be present and neutral: `input_frame.new` requires a
    -- canonical eight-entry array, and the browser's own encoder fills them
    -- the same way (`browser_sim_host.ts`'s `encodeInputFrameWire`).
    local slots = { slot_input.to_sample(raw) }
    for index = 2, input_frame.SLOT_COUNT do
        slots[index] = input_frame.neutral_sample()
    end
    local frame = assert(input_frame.new(tick, slots))
    local wire = assert(input_frame.encode(frame))
    local decoded = assert(input_frame.decode(wire))
    assert(input_frame.validate(decoded))
    return slot_input.to_match_input(decoded.slots[1])
end

function love.load()
    -- Built exactly as `game/screens/match.lua`'s `Match:restart` builds an
    -- ordinary match outside the rollback lab: no `input_ownership`, no
    -- `human_controlled` override (so it defaults true and one player takes
    -- the human-input branch -- the branch this capture exists to reach),
    -- and `match.step`'s own default duration and goal limit.
    local s = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        seed = MATCH_SEED,
    })
    local b = bot.new({ seed = BOT_SEED })
    dump_row(0, s)
    for tick = 1, TICKS do
        match.step(s, DT, tick_input(b, s, tick))
        dump_row(tick, s)
    end
    love.event.quit(0)
end
