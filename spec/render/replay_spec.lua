local t = require("spec.support.runner")
local player_pose = require("render.player_pose")
local replay = require("game.render.replay")
local match = require("sim.match")
local teams = require("data.teams")
local tuning = require("sim.tuning")
local Vec2 = require("core.vec2")
local combat = require("sim.combat")

local NO_INPUT = {
    move = Vec2.new(0, 0),
    shoot = false,
    shoot_held = false,
    pass = false,
    pass_held = false,
    switch = false,
    dash = false,
    dodge = false,
    lob = false,
    sprint = false,
    jockey = false,
}

t.describe("goal replay buffer", function()
    t.it("records live frames and plays them back in slow motion", function()
        tuning.reset()
        replay.reset()
        local s =
            match.new({ home = teams.nebula, away = teams.orion, field = { w = 960, h = 540 } })
        for _ = 1, 90 do
            replay.record(s)
            match.step(s, 1 / 60, NO_INPUT)
        end
        t.is_true(replay.start("home"), "enough footage to start")
        t.is_true(replay.active())
        t.is_true(replay.celebrating(), "opens on the celebration beat")

        local frames, last = 0, nil
        for _ = 1, 2000 do
            local st = replay.step(1 / 60)
            if not st then
                break
            end
            frames = frames + 1
            t.is_true(st.players ~= nil and #st.players == 10, "drawable state")
            t.is_true(st.ball ~= nil)
            last = st
        end
        t.is_true(not replay.active(), "playback finishes on its own")
        t.is_true(last ~= nil)
        -- Slow motion: playing ~90 recorded frames (+ tail) at 0.35x must take
        -- meaningfully MORE display frames than were recorded.
        t.is_true(frames > 90 * 2, "playback is slower than real time: " .. frames)
    end)

    t.it("can be skipped", function()
        tuning.reset()
        replay.reset()
        local s =
            match.new({ home = teams.nebula, away = teams.orion, field = { w = 960, h = 540 } })
        for _ = 1, 60 do
            replay.record(s)
            match.step(s, 1 / 60, NO_INPUT)
        end
        t.is_true(replay.start("home"))
        replay.stop()
        t.is_true(not replay.active())
        t.is_true(replay.step(1 / 60) == nil)
    end)

    t.it("refuses to start without enough footage", function()
        replay.reset()
        t.is_true(not replay.start("home"), "no footage, no replay")
    end)

    t.it("retains the authoritative combat companion in recorded frames", function()
        tuning.reset()
        replay.reset()
        local state =
            match.new({ home = teams.nebula, away = teams.orion, field = { w = 960, h = 540 } })
        local combat_state = combat.new_state(state)
        for _ = 1, 60 do
            replay.record(state, combat_state)
            match.step(state, 1 / 60, NO_INPUT, combat_state)
        end
        t.is_true(replay.start("home"))
        local frame = assert(replay.step(1 / 60))
        t.is_true(frame._combat_state ~= nil)
        t.eq(assert(frame._combat_state).player_ids[2], frame.players[2].id)
    end)

    t.it("retains the corrected combat companion when replacing a rollback tail", function()
        tuning.reset()
        replay.reset()
        local state =
            match.new({ home = teams.nebula, away = teams.orion, field = { w = 960, h = 540 } })
        local combat_state = combat.new_state(state)
        for boundary = 0, 39 do
            replay.record_boundary(boundary, state, combat_state)
        end

        replay.truncate_from(20)
        combat_state.players[2].phase = "recovery"
        combat_state.players[2].phase_ticks = 9
        combat_state.players[2].cooldown_ticks = 47
        for boundary = 20, 39 do
            replay.record_boundary(boundary, state, combat_state)
        end

        t.is_true(replay.start_at("home", 39))
        local frame = assert(replay.step(1 / 60))
        local corrected = assert(frame._combat_state)
        t.eq(corrected.players[2].phase, "recovery")
        t.eq(corrected.players[2].phase_ticks, 9)
        t.eq(corrected.players[2].cooldown_ticks, 47)
    end)

    t.it("celebrates before cutting to the slow-motion replay", function()
        tuning.reset()
        replay.reset()
        local s =
            match.new({ home = teams.nebula, away = teams.orion, field = { w = 960, h = 540 } })
        for _ = 1, 90 do
            replay.record(s)
            match.step(s, 1 / 60, NO_INPUT)
        end
        t.is_true(replay.start("home"))
        -- The celebration runs first (real time), then playback takes over.
        local celeb_frames = 0
        for _ = 1, 2000 do
            local st = replay.step(1 / 60)
            if not st or not replay.celebrating() then
                break
            end
            celeb_frames = celeb_frames + 1
            t.is_true(#st.players == 10 and st.ball ~= nil, "drawable celebration frame")
        end
        t.is_true(celeb_frames > 30, "celebration lasts a beat: " .. celeb_frames)
        t.is_true(replay.active() and not replay.celebrating(), "then it is the replay")
    end)

    -- pitch.draw hands buffered replay players straight to player_pose.select,
    -- so every field that selector reads must survive the capture. A missing one
    -- is a nil comparison — a crash on the goal sequence, not a wrong pose.
    t.it("carries every pose input through capture, celebration, and playback", function()
        tuning.reset()
        replay.reset()
        local s =
            match.new({ home = teams.nebula, away = teams.orion, field = { w = 960, h = 540 } })
        local keeper_index
        for index, p in ipairs(s.players) do
            if p.is_keeper then
                keeper_index = index
                break
            end
        end
        keeper_index = assert(keeper_index)
        for _ = 1, 90 do
            -- Hold the post-dive get-up window open across the recording. The
            -- buffered struct must carry it; the selector must not default it.
            s.players[keeper_index].keeper_get_up_timer = 0.18
            replay.record(s)
            match.step(s, 1 / 60, NO_INPUT)
        end

        t.is_true(replay.start("home"))
        local neutral = { near_ball = false, shuffling = false, tip = false }
        local frames, get_up_frames = 0, 0
        for _ = 1, 4000 do
            local st = replay.step(1 / 60)
            if not st then
                break
            end
            frames = frames + 1
            for _, p in ipairs(st.players) do
                local selection = player_pose.select(p, nil, p.is_keeper and neutral or nil)
                t.is_true(player_pose.PRIORITY[selection.id] ~= nil, "unknown replay pose")
                if p.is_keeper and selection.id == "keeper_get_up" then
                    get_up_frames = get_up_frames + 1
                end
            end
        end
        t.is_true(frames > 0, "replay produced no frames")
        t.is_true(get_up_frames > 0, "the get-up window never reached a replay frame")
    end)
end)
