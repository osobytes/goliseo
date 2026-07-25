-- Headless contract for game/audio.lua.
-- conf.lua sets t.modules.audio = false and t.modules.sound = false in test
-- mode, so love.audio and love.sound are nil when this runs. Every audio
-- entry point (load / update / reset / toggle_mute) must no-op cleanly.
--
-- Pattern mirrors spec/render/draw_smoke_spec.lua: we ensure love.audio and
-- love.sound are nil, require the module, and exercise its full public API
-- over a real MatchState — verifying no error is raised.

local t = require("spec.support.runner")
local match_sim = require("sim.match")
local teams = require("data.teams")
local combat_fixture = require("spec.fixtures.crowded_combat_feedback")

-- Sanity check: in headless mode these MUST be nil; the test is meaningless
-- if they aren't.
local function ensure_headless()
    -- love.audio is nil when t.modules.audio = false (conf.lua)
    return love.audio == nil and love.sound == nil
end

-- Force both to nil for the duration of the test block, even if somehow they
-- were loaded (belt-and-suspenders for any future conf.lua changes).
---@param fn fun()
local function with_nil_audio(fn)
    local saved_audio = love.audio
    local saved_sound = love.sound
    -- rawset avoids LuaCATS complaints about writing to love
    rawset(love, "audio", nil)
    rawset(love, "sound", nil)
    local ok, err = pcall(fn)
    rawset(love, "audio", saved_audio)
    rawset(love, "sound", saved_sound)
    return ok, err
end

t.describe("audio headless contract", function()
    t.it("love.audio and love.sound are nil in headless mode (sanity)", function()
        -- If this check fails the other tests are still meaningful because
        -- with_nil_audio forces nil. But flag it so we know.
        t.is_true(ensure_headless(), "expected headless: love.audio and love.sound should be nil")
    end)

    t.it("audio.load() no-ops without error when love.audio is nil", function()
        local ok, err = with_nil_audio(function()
            -- Re-require each time inside the nil scope to exercise the guard
            -- at the top of load(). (Module is cached; load() checks at entry.)
            local audio = require("game.audio")
            audio.load()
        end)
        t.is_true(ok, "audio.load() error: " .. tostring(err))
    end)

    t.it("audio.update() no-ops without error over a real MatchState", function()
        local ok, err = with_nil_audio(function()
            local audio = require("game.audio")
            audio.load()
            local state = match_sim.new({
                home = teams.nebula,
                away = teams.orion,
                field = { w = 960, h = 540 },
            })
            -- Simulate several frames of updates
            audio.update(state, 1 / 60)
            audio.update(state, 1 / 60)
            -- Advance time_left by stepping the sim a few ticks
            local input = {
                move = require("core.vec2").new(0, 0),
                shoot = false,
                shoot_held = false,
                pass = false,
                pass_held = false,
                switch = false,
                dash = false,
                dodge = false,
                lob = false,
                sprint = false,
            }
            for _ = 1, 10 do
                match_sim.step(state, 1 / 60, input)
                audio.update(state, 1 / 60)
            end
        end)
        t.is_true(ok, "audio.update() error: " .. tostring(err))
    end)

    t.it("audio.reset() no-ops without error when love.audio is nil", function()
        local ok, err = with_nil_audio(function()
            local audio = require("game.audio")
            audio.reset()
        end)
        t.is_true(ok, "audio.reset() error: " .. tostring(err))
    end)

    t.it("audio.toggle_mute() no-ops without error when love.audio is nil", function()
        local ok, err = with_nil_audio(function()
            local audio = require("game.audio")
            local muted = audio.toggle_mute()
            -- toggle_mute returns the new muted state (boolean)
            t.is_true(type(muted) == "boolean", "toggle_mute should return boolean")
            -- Toggle back
            audio.toggle_mute()
        end)
        t.is_true(ok, "audio.toggle_mute() error: " .. tostring(err))
    end)

    t.it("audio.update() handles score edge (goal) without error", function()
        local ok, err = with_nil_audio(function()
            local audio = require("game.audio")
            audio.load()
            local state = match_sim.new({
                home = teams.nebula,
                away = teams.orion,
                field = { w = 960, h = 540 },
            })
            -- Simulate initial update to set prev_score baseline
            audio.update(state, 1 / 60)
            -- Fake a goal by bumping the score
            state.score.home = state.score.home + 1
            audio.update(state, 1 / 60)
            -- And a second goal
            state.score.away = state.score.away + 1
            audio.update(state, 1 / 60)
        end)
        t.is_true(ok, "audio.update() score-edge error: " .. tostring(err))
    end)

    t.it("audio.update() handles kickoff edge (time_left reset) without error", function()
        local ok, err = with_nil_audio(function()
            local audio = require("game.audio")
            audio.load()
            local state = match_sim.new({
                home = teams.nebula,
                away = teams.orion,
                field = { w = 960, h = 540 },
            })
            -- First update seeds _prev_time_left
            audio.update(state, 1 / 60)
            -- Simulate a time reset (as after a goal restart)
            state.time_left = state.time_left + 30
            audio.update(state, 1 / 60)
        end)
        t.is_true(ok, "audio.update() kickoff-edge error: " .. tostring(err))
    end)

    t.it("ticks replay ambience and accepts the full-time edge headlessly", function()
        local ok, err = with_nil_audio(function()
            local audio = require("game.audio")
            local state = match_sim.new({
                home = teams.nebula,
                away = teams.orion,
                field = { w = 960, h = 540 },
            })
            audio.reset()
            audio.tick(0.5)
            state.finished = true
            audio.update(state, 1 / 60)
            audio.update(state, 1 / 60)
        end)
        t.is_true(ok, "audio full-time edge error: " .. tostring(err))
    end)

    t.it("full load→update→reset cycle no-ops cleanly", function()
        local ok, err = with_nil_audio(function()
            local audio = require("game.audio")
            audio.load()
            local state = match_sim.new({
                home = teams.nebula,
                away = teams.orion,
                field = { w = 960, h = 540 },
            })
            audio.update(state, 1 / 60)
            audio.reset()
            audio.load()
            audio.update(state, 1 / 60)
            audio.toggle_mute()
            audio.update(state, 1 / 60)
            audio.toggle_mute()
        end)
        t.is_true(ok, "full cycle error: " .. tostring(err))
    end)

    t.it("deduplicates and replay-suppresses confirmed combat cues headlessly", function()
        local ok, err = with_nil_audio(function()
            local audio = require("game.audio")
            audio.reset()
            local hit = combat_fixture.events[3]
            local guarded = combat_fixture.events[2]
            t.is_true(audio.consume_confirmed(hit))
            t.is_true(not audio.consume_confirmed(hit))
            t.is_true(audio.consume_confirmed(guarded, true))
            t.eq(audio.confirmed_cue_counts().combat_hit, 1)
            t.is_true(audio.confirmed_cue_counts().combat_guarded == nil)
            t.eq(audio.suppressed_confirmed_cue_counts().combat_guarded, 1)
            audio.consume_combat({ hit.payload, guarded.payload })
        end)
        t.is_true(ok, "confirmed combat audio error: " .. tostring(err))
    end)
end)
