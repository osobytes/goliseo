-- Contract tests for the simulation side of the wasm payload boundary (#332).
--
-- The module itself is driven from Rust and normally only runs under the wasm
-- host, which no gate exercises. These are the parts that can be tested without
-- one: they are pure Lua, and they are where a silent wrong answer would hide.
--
-- Tier 1: no display, no wasm, no browser.

local t = require("spec.support.runner")
local host = require("scripts.wasm_frame_payload_host")
local frame_buffer = require("render.frame_buffer")
local fixed_clock = require("sim.fixed_clock")

t.describe("wasm frame payload host", function()
    t.it("reports a tick that matches the state after a rollback", function()
        host.boot()
        local before_tick = host.advance(60)
        t.eq(before_tick, 60, "advancing 60 ticks must report tick 60")

        local before_hash = host.hash()
        local after_tick = host.rollback(8)

        -- The counter rewinds with the state and steps forward with the resim,
        -- so it must land back where it started. It reported a stale value
        -- before this was fixed: the resim reproduces the same eight ticks, so
        -- a counter that only counted forward drifted by the rollback depth
        -- every single burst.
        t.eq(after_tick, before_tick, "a rollback must not advance the tick counter")
        t.eq(host.hash(), before_hash, "a rollback must reproduce the state it rolled back over")
    end)

    t.it("keeps the tick honest across repeated bursts", function()
        host.boot()
        local tick = host.advance(40)
        for _ = 1, 5 do
            t.eq(host.rollback(8), tick, "every burst must return to the same tick")
        end
    end)

    t.it("re-simulates at most the fixed clock's maximum burst", function()
        host.boot()
        host.advance(40)
        t.eq(host.MAX_RESIM_TICKS, fixed_clock.MAX_TICKS_PER_UPDATE)
        t.is_true(
            not pcall(host.rollback, host.MAX_RESIM_TICKS + 1),
            "rolling back past the retained history must fail loudly, not silently clamp"
        )
    end)

    t.it("hands out a decodable frame block for the live match", function()
        local count = host.boot()
        host.advance(30)
        local decoded = frame_buffer.decode(host.frame())
        t.eq(#decoded.players.x, count)
        t.eq(decoded.render_frame_version, frame_buffer.decode(host.frame()).render_frame_version)
    end)

    t.it("reuses one output array instead of allocating per frame", function()
        host.boot()
        host.advance(10)
        -- The steady-state promise: a per-frame producer allocates once. If this
        -- ever returns a fresh table each call, the boundary is still correct
        -- but the frame budget quietly pays for it.
        t.is_true(host.frame() == host.frame(), "the frame block array must be reused")
    end)

    t.it("crosses the roster once, with parseable ids and names", function()
        local count = host.boot()
        local words, strings = host.roster()
        local roster = frame_buffer.decode_roster(words, strings)
        t.eq(roster.count, count)
        for index = 1, count do
            t.is_true(#roster.ids[index] > 0, "every slot must carry an id")
            t.is_true(#roster.names[index] > 0, "every slot must carry a name")
        end
    end)
end)
