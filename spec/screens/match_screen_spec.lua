local t = require("spec.support.runner")
local Match = require("game.screens.match")
local fixed_clock = require("sim.fixed_clock")
local sim_match = require("sim.match")

t.describe("match screen rematch (tier 2)", function()
    t.it("R restarts a finished match with the same pre-match choices", function()
        local m = Match.new({ formation = "2-1-1", tactic = "press_high" })
        t.eq(m.state.press.home, 2, "tactic applied to the first match")
        m.state.finished = true
        m.state.score.home = 3
        m:event({ kind = "key", key = "r" })
        t.is_true(not m.state.finished, "a fresh match is underway")
        t.eq(m.state.score.home, 0, "the score is reset")
        t.eq(#m.state.players, 10)
        t.eq(m.state.press.home, 2, "the tactic choice carries into the rematch")
    end)

    t.it("Enter also triggers the rematch", function()
        local m = Match.new()
        m.state.finished = true
        m:event({ kind = "key", key = "return" })
        t.is_true(not m.state.finished)
    end)

    t.it("ignores the rematch keys while the match is live", function()
        local m = Match.new()
        local before = m.state
        m:event({ kind = "key", key = "r" })
        t.is_true(m.state == before, "no restart mid-match")
    end)

    t.it("ignores match inputs after full time", function()
        local m = Match.new()
        m.state.finished = true
        m:event({ kind = "key", key = "k" })
        t.is_true(not m._pass, "pass input does not buffer on the full-time screen")
    end)

    t.it("leaves rematch ownership to the result screen in product mode", function()
        local m = Match.new({ profile = "product" })
        m.state.finished = true
        m:event({ kind = "key", key = "r" })
        m:event({ kind = "action", action = "confirm" })
        t.is_true(m.state.finished)
    end)
end)

t.describe("match screen fixed simulation clock (tier 2)", function()
    t.it("samples render frames but only steps the match at the canonical interval", function()
        local saved_keyboard = love.keyboard
        local original_step = sim_match.step
        local down = {}
        local dts = {}
        love.keyboard = {
            isDown = function(...)
                for _, key in ipairs({ ... }) do
                    if down[key] then
                        return true
                    end
                end
                return false
            end,
        }
        sim_match.step = function(state, dt, input)
            dts[#dts + 1] = dt
            return original_step(state, dt, input)
        end

        local ok, err = pcall(function()
            local screen = Match.new()
            local initial_time = screen.state.time_left
            screen:update(1 / 120)
            t.eq(#dts, 0, "a zero-tick render update never steps the simulator")
            screen:update(1 / 120)
            screen:update(1 / 30)
            t.eq(#dts, 3)
            for _, dt in ipairs(dts) do
                t.eq(dt, fixed_clock.TICK_SECONDS)
            end
            t.eq(screen._clock.tick, 3)
            t.near(screen.state.time_left, initial_time - 3 * fixed_clock.TICK_SECONDS, 1e-9)
        end)
        sim_match.step = original_step
        love.keyboard = saved_keyboard
        assert(ok, err)
    end)
end)

t.describe("match screen contextual controls (tier 2)", function()
    t.it("constructs and drives combat only behind the explicit option", function()
        local showcase = Match.new()
        t.is_true(showcase._combat_state == nil)

        local prototype = Match.new({ combat_enabled = true })
        local combat_state = assert(prototype._combat_state)
        prototype.state.kickoff_hold = 0
        prototype:event({
            kind = "action",
            action = "equipment",
            pressed = true,
        })
        prototype:update(fixed_clock.TICK_SECONDS)
        local runtime = combat_state.players[prototype.state.controlled]
        t.eq(runtime.phase, "windup")
        t.is_true(runtime.source_sequence ~= nil)
    end)

    t.it("K never switches while carrying the ball (it charges a pass)", function()
        local m = Match.new() -- at kickoff the controlled player has the ball
        m:event({ kind = "key", key = "k" })
        t.is_true(not m._switch, "on the ball, K is the (polled) pass charge, not a switch")
    end)

    t.it("K switches player when not carrying", function()
        local m = Match.new()
        m.state.owner = nil
        m:event({ kind = "key", key = "k" })
        t.is_true(m._switch, "K is a switch off the ball")
        t.is_true(not m._pass)
    end)

    t.it("Space hold = jockey, release = poke (off the ball)", function()
        -- Drive polled inputs with a stubbed keyboard (same pattern as lob-latch test).
        local saved = love.keyboard
        local down = {}
        love.keyboard = {
            isDown = function(...)
                for _, k in ipairs({ ... }) do
                    if down[k] then
                        return true
                    end
                end
                return false
            end,
        }
        -- Off the ball: holding Space produces jockey stance (jockey_timer > 0).
        local m = Match.new()
        m.state.owner = nil
        m.state.pickup_cd = 5 -- nobody picks the ball up during this test
        local me = m.state.players[m.state.controlled]
        down.space = true
        m:update(1 / 60) -- hold one frame
        t.is_true(me.jockey_timer > 0, "holding Space off the ball engages jockey stance")
        -- _space_held_prev is set; release now → poke fires next update.
        -- The poke manifests as tackle_timer > 0 on the controlled player.
        me.tackle_cd = 0 -- ensure the poke isn't blocked by cooldown
        me.slide_timer = 0
        down.space = false
        m:update(1 / 60) -- release
        t.is_true(me.tackle_timer > 0, "releasing Space off the ball fires the poke")
        love.keyboard = saved
    end)

    t.it("Space never produces a poke while carrying (it charges the shot)", function()
        local saved = love.keyboard
        local down = {}
        love.keyboard = {
            isDown = function(...)
                for _, k in ipairs({ ... }) do
                    if down[k] then
                        return true
                    end
                end
                return false
            end,
        }
        local m = Match.new() -- carrying at kickoff
        down.space = true
        m:update(1 / 60) -- hold while carrying
        down.space = false
        local me = m.state.players[m.state.controlled]
        local tackle_before = me.tackle_timer
        m:update(1 / 60) -- release while still carrying
        t.is_true(
            me.tackle_timer == tackle_before,
            "Space release while carrying does not fire a poke"
        )
        love.keyboard = saved
    end)
end)

t.describe("match screen lob latch (tier 2)", function()
    -- Drive the polled inputs with a stubbed keyboard.
    local function with_keys(fn)
        local saved = love.keyboard
        local down = {}
        love.keyboard = {
            isDown = function(...)
                for _, k in ipairs({ ... }) do
                    if down[k] then
                        return true
                    end
                end
                return false
            end,
        }
        local ok, err = pcall(fn, down)
        love.keyboard = saved
        assert(ok, err)
    end

    t.it("L held during a charged pass lofts it even if L lifts a frame early", function()
        with_keys(function(down)
            local m = Match.new() -- carrying at kickoff
            down.k, down.l = true, true
            m:update(1 / 60) -- charging the pass with L held
            m:update(1 / 60)
            down.l = false
            m:update(1 / 60) -- L released a beat before K...
            down.k = false
            m:update(1 / 60) -- ...K release fires the pass
            t.is_true(m.state.owner ~= m.state.controlled, "the pass released")
            t.is_true(m.state.ball_vz > 0, "and it was lofted: the latch held L for us")
        end)
    end)

    t.it("holding K charges the pass range for an outfielder", function()
        with_keys(function(down)
            local m = Match.new()
            down.k = true
            for _ = 1, 20 do -- a third of a second of holding K
                m:update(1 / 60) -- (a full meter would auto-fire and reset)
            end
            t.is_true(
                m.state.players[m.state.controlled].pass_charge > 0.5,
                "the pass range charged up"
            )
        end)
    end)
end)

t.describe("match screen goal replay (tier 2)", function()
    local replay = require("game.render.replay")

    local function with_keys(fn)
        local saved = love.keyboard
        local down = {}
        love.keyboard = {
            isDown = function(...)
                for _, k in ipairs({ ... }) do
                    if down[k] then
                        return true
                    end
                end
                return false
            end,
        }
        local ok, err = pcall(fn, down)
        love.keyboard = saved
        assert(ok, err)
    end

    t.it("a goal freezes the sim into a slow-mo replay; skipping resumes", function()
        with_keys(function()
            local m = Match.new()
            for _ = 1, 40 do -- build up recorded footage
                m:update(1 / 60)
            end
            m.state.score.home = m.state.score.home + 1 -- goal edge
            m:update(1 / 60)
            t.is_true(replay.active(), "the replay rolls after a goal")
            local t0 = m.state.time_left
            m:update(1 / 60)
            t.eq(m.state.time_left, t0, "the sim is frozen during the replay")
            m:event({ kind = "key", key = "space" })
            t.is_true(not replay.active(), "Space skips the replay")
            m:update(1 / 60)
            t.is_true(m.state.time_left < t0, "live play resumes")
        end)
    end)
end)
