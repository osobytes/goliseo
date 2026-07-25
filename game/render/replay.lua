-- Goal replays: a bounded, boundary-keyed buffer of lightweight render snapshots.
-- When a goal goes in the sequence runs in two phases through the
-- normal renderer (same camera): first a brief CELEBRATION — the scorer wheels
-- away and teammates converge to mob them — then a slow-motion PLAYBACK of the
-- last few seconds of footage. Game-layer only — the sim knows nothing of it.
--
-- Snapshots copy exactly what pitch.draw touches. Vec2s are immutable in this
-- codebase so aliasing player fields is safe; the BALL is defensively copied
-- because the sim mutates s.ball / s.ball_vel in place in the bounce code.

local Vec2 = require("core.vec2")
local combat_sim = require("sim.combat")
local tuning = require("sim.tuning")

local replay = {}

local MAX_SECONDS = 8 -- ring capacity; playback window length is tunable
local EXTRAPOLATE = 20 -- synthetic tail frames: the sim resets for kickoff on
-- the goal frame, so we extend the final ball flight into the net ourselves
-- (long enough to show the netting catch the ball and rebound it back out).

-- Net-catch physics for the synthetic tail (mirrors the sim's loose-ball net
-- handling, which the replay never runs — see sim/match.lua). Hardcoded like
-- the gravity constant below: this is presentation-only ballistics.
local TAIL_BALL_R = 6 -- ball radius (sim BALL_RADIUS)
local TAIL_GRAVITY = 900 -- downward accel on ball_vz (sim GRAVITY)
local TAIL_NET_DAMP = 0.3 -- pace kept when the netting stops the ball (sim NET_DAMP)
local TAIL_CROSSBAR = 70 -- roof net height; a scored ball stays under the bar (sim CROSSBAR)

-- Celebration (presentation-only, plays before the replay cuts in).
local CELEBRATE_SECONDS = 1.6 -- the "brief delay" before playback begins
local MATE_DELAY = 0.25 -- teammates set off this fraction into the run (scorer leads)
local MOB_RADIUS = 30 -- how tightly teammates ring the scorer when they arrive
local STRIKE = { shot = true, header = true, volley = true, bicycle = true } -- events that score

---@class ReplayFrame: MatchState
---@field _replay_boundary integer
---@field _combat_state CombatMatchState?

---@type ReplayFrame[]
local buf = {}
local legacy_boundary = 0
local phase = nil -- nil | "celebrate" | "playback"
local playhead = 1
local emitted = 0
local window = {}
local active_end_boundary = nil
-- Celebration state (procedural: start poses + targets, animated by elapsed time).
local cel_elapsed = 0
---@type MatchState  -- the goal-moment snapshot (start poses + goals + field)
local cel_base
---@type string?  -- id of the player that scored
local cel_scorer
---@type table<string, Vec2>  -- id -> target the player runs to
local cel_targets
---@type Vec2  -- ball resting in the net
local cel_ball

local function cap()
    return MAX_SECONDS * 60
end

---@return integer
function replay.capacity()
    return cap()
end

local function clamp(x, a, b)
    return math.max(a, math.min(b, x))
end

-- Smootherstep-lite: ease-in-out on 0..1, flat at both ends.
local function smooth(u)
    u = clamp(u, 0, 1)
    return u * u * (3 - 2 * u)
end

function replay.reset()
    buf = {}
    legacy_boundary = 0
    phase = nil
    window = {}
    active_end_boundary = nil
end

function replay.active()
    return phase ~= nil
end

-- True only during the pre-replay celebration beat (the HUD shows GOAL, not REPLAY).
function replay.celebrating()
    return phase == "celebrate"
end

function replay.stop()
    phase = nil
    window = {}
    active_end_boundary = nil
end

---@param s MatchState
---@param boundary integer
---@param combat_state CombatMatchState?
---@return ReplayFrame
local function capture_frame(s, boundary, combat_state)
    local players = {}
    for i, p in ipairs(s.players) do
        players[i] = {
            id = p.id,
            team = p.team,
            pos = p.pos,
            facing = p.facing,
            radius = p.radius,
            is_keeper = p.is_keeper,
            slide_timer = p.slide_timer,
            dive_timer = p.dive_timer,
            dive_dir = p.dive_dir,
            grab_timer = p.grab_timer,
            throw_timer = p.throw_timer,
            windup_timer = p.windup_timer,
            aerial_timer = p.aerial_timer,
            aerial_style = p.aerial_style,
            aerial_outcome = p.aerial_outcome,
            aerial_jump = p.aerial_jump,
            sprint_meter = 1, -- keeps the HUD meter hidden during playback
            charge = 0,
            pass_charge = 0,
            pass_target = nil,
            jockey_timer = 0,
        }
    end
    local events = {}
    for i, e in ipairs(s.events) do
        events[i] = e
    end
    return {
        _replay_boundary = boundary,
        _combat_state = combat_state and combat_sim.snapshot(combat_state) or nil,
        field = s.field,
        goal_home = s.goal_home,
        goal_away = s.goal_away,
        score = { home = s.score.home, away = s.score.away },
        time_left = s.time_left,
        controlled = s.controlled,
        owner = s.owner,
        ball = Vec2.new(s.ball.x, s.ball.y),
        ball_vel = Vec2.new(s.ball_vel.x, s.ball_vel.y),
        ball_z = s.ball_z,
        ball_vz = s.ball_vz,
        finished = false,
        players = players,
        events = events,
    }
end

---@param boundary integer
---@return integer?, boolean
local function boundary_index(boundary)
    for index, frame in ipairs(buf) do
        if frame._replay_boundary == boundary then
            return index, true
        elseif frame._replay_boundary > boundary then
            return index, false
        end
    end
    return nil, false
end

-- Record or replace one canonical simulation boundary. Corrected rollback
-- footage uses this API so obsolete interval frames cannot survive by index.
---@param boundary integer
---@param s MatchState
---@param combat_state CombatMatchState?
function replay.record_boundary(boundary, s, combat_state)
    assert(
        type(boundary) == "number" and boundary == math.floor(boundary) and boundary >= 0,
        "replay boundary must be a non-negative integer"
    )
    local frame = capture_frame(s, boundary, combat_state)
    local index, found = boundary_index(boundary)
    if found then
        buf[assert(index)] = frame
    elseif index then
        table.insert(buf, index, frame)
    else
        buf[#buf + 1] = frame
    end
    while #buf > cap() do
        table.remove(buf, 1)
    end
end

-- Delete a corrected speculative tail before inserting replacement frames.
---@param boundary integer
function replay.truncate_from(boundary)
    local index = boundary_index(boundary)
    if index then
        for cursor = #buf, index, -1 do
            table.remove(buf, cursor)
        end
    end
    if phase ~= nil and active_end_boundary and boundary <= active_end_boundary then
        replay.stop()
    end
end

-- Record one legacy live frame (called before its simulation step). The
-- compatibility adapter supplies a private contiguous boundary sequence.
---@param s MatchState
---@param combat_state CombatMatchState?
function replay.record(s, combat_state)
    replay.record_boundary(legacy_boundary, s, combat_state)
    legacy_boundary = legacy_boundary + 1
end

-- Who scored: the most recent player on the scoring team to strike the ball
-- (shot/header/volley), scanning the buffer back from the goal frame. Falls
-- back to the scoring-team outfielder nearest the ball (deflections/own goals).
---@param scoring_team "home"|"away"
---@param last table  -- goal-moment snapshot
---@param end_index integer
---@return string? id
local function find_scorer(scoring_team, last, end_index)
    local team_of = {}
    for _, p in ipairs(last.players) do
        team_of[p.id] = p.team
    end
    for cursor = end_index, 1, -1 do
        local fr = buf[cursor]
        if fr and fr.events then
            for i = #fr.events, 1, -1 do
                local e = fr.events[i]
                if STRIKE[e.kind] and e.player and team_of[e.player] == scoring_team then
                    return e.player
                end
            end
        end
    end
    local best, best_d
    for _, p in ipairs(last.players) do
        if p.team == scoring_team and not p.is_keeper then
            local d = p.pos:dist(last.ball)
            if not best_d or d < best_d then
                best_d, best = d, p.id
            end
        end
    end
    return best
end

-- Set up the celebration: scorer wheels away toward the corner they scored in,
-- outfield teammates converge to mob them, everyone else holds (dejected).
---@param scoring_team "home"|"away"
---@param end_index integer
local function build_celebration(scoring_team, end_index)
    cel_base = buf[end_index]
    cel_scorer = find_scorer(scoring_team, cel_base, end_index)
    if not cel_scorer then
        return false
    end
    local field = cel_base.field
    local scorer_pos
    local mates = {}
    for _, p in ipairs(cel_base.players) do
        if p.id == cel_scorer then
            scorer_pos = p.pos
        elseif p.team == scoring_team and not p.is_keeper then
            mates[#mates + 1] = p.id
        end
    end
    -- Wheel toward the corner of the goal just scored in, nearest touchline.
    local corner_x = (scoring_team == "home") and (field.w - 70) or 70
    local corner_y = (scorer_pos.y < field.h / 2) and 80 or (field.h - 80)
    local corner = Vec2.new(corner_x, corner_y)
    cel_targets = { [cel_scorer] = corner }
    -- Teammates ring the corner so they arrive AROUND the scorer, not on top.
    for i, id in ipairs(mates) do
        local ang = (i / #mates) * 2 * math.pi
        cel_targets[id] = Vec2.new(
            clamp(corner_x + MOB_RADIUS * math.cos(ang), 20, field.w - 20),
            clamp(corner_y + MOB_RADIUS * math.sin(ang), 20, field.h - 20)
        )
    end
    -- Ball rests in the net behind the line it crossed.
    local g = (scoring_team == "home") and cel_base.goal_away or cel_base.goal_home
    cel_ball = Vec2.new(
        (scoring_team == "home") and (field.w + 12) or -12,
        clamp(cel_base.ball.y, g.y + 10, g.y + g.h - 10)
    )
    cel_elapsed = 0
    return true
end

-- Build the slow-motion window: the last REPLAY_SECONDS of footage plus a
-- synthetic net-bulge tail (the sim reset to kickoff on the goal frame).
---@param n integer
---@param end_index integer
local function build_window(n, end_index)
    window = {}
    local first = end_index - n + 1
    for index = first, end_index do
        window[#window + 1] = buf[index]
    end
    -- Synthetic tail: fly the ball on while everyone holds their final pose,
    -- and let the goal net catch it. The ball drives into the netting, sheds
    -- most of its pace, and rebounds gently back toward the line — so a goal
    -- reads clearly instead of the ball sailing straight out the back.
    local last = window[#window]
    -- The ball scored, so it's crossing one goal line; pick which by heading.
    local into_away = last.ball_vel.x >= 0
    local gg = into_away and last.goal_away or last.goal_home
    -- Back netting plane (ball centre clamp) and the y-extent of the mouth.
    local back_x = into_away and (gg.x + gg.w - TAIL_BALL_R) or (gg.x + TAIL_BALL_R)
    local y_lo, y_hi = gg.y + TAIL_BALL_R, gg.y + gg.h - TAIL_BALL_R
    local bx, by = last.ball.x, last.ball.y
    local bz, bvz = last.ball_z, last.ball_vz
    local vx, vy = last.ball_vel.x, last.ball_vel.y
    local dt = 1 / 60
    for _ = 1, EXTRAPOLATE do
        bx, by = bx + vx * dt, by + vy * dt
        bvz = bvz - TAIL_GRAVITY * dt
        bz = bz + bvz * dt
        if bz <= 0 then
            bz, bvz = 0, 0
        elseif bz > TAIL_CROSSBAR then
            bz = TAIL_CROSSBAR -- roof net holds it under the bar
            if bvz > 0 then
                bvz = 0
            end
        end
        -- Back net: clamp at the netting and rebound with much less force.
        if (into_away and bx > back_x) or (not into_away and bx < back_x) then
            bx = back_x
            vx = -vx * TAIL_NET_DAMP
        end
        -- Side nets: keep the ball inside the mouth as it settles.
        if by < y_lo then
            by, vy = y_lo, -vy * TAIL_NET_DAMP
        elseif by > y_hi then
            by, vy = y_hi, -vy * TAIL_NET_DAMP
        end
        local snap = {}
        for k, v in pairs(last) do
            snap[k] = v
        end
        snap.ball = Vec2.new(bx, by)
        snap.ball_z = bz
        snap.ball_vel = Vec2.new(vx, vy)
        snap.ball_vz = bvz
        snap.events = {}
        window[#window + 1] = snap
    end
end

-- Begin the goal sequence: a celebration beat, then slow-motion playback of
-- the last REPLAY_SECONDS (+ net-bulge tail).
---@param scoring_team "home"|"away"
---@param end_index integer
---@return boolean started
local function start_at_index(scoring_team, end_index)
    local n = math.min(end_index, math.floor(tuning.values.REPLAY_SECONDS * 60))
    if n < 30 then
        return false -- not enough footage to be worth showing
    end
    build_window(n, end_index)
    active_end_boundary = buf[end_index]._replay_boundary
    playhead = 1
    emitted = 0
    -- Celebrate first if we can name a scorer; otherwise cut straight to replay.
    if build_celebration(scoring_team, end_index) then
        phase = "celebrate"
    else
        phase = "playback"
    end
    return true
end

-- Begin the legacy goal sequence at the newest recorded frame.
---@param scoring_team "home"|"away"
---@return boolean started
function replay.start(scoring_team)
    return start_at_index(scoring_team, #buf)
end

-- Begin a confirmed rollback goal sequence at its corrected pre-goal boundary,
-- even when newer speculative/live simulation boundaries already exist.
---@param scoring_team "home"|"away"
---@param boundary integer
---@return boolean started
function replay.start_at(scoring_team, boundary)
    local index, found = boundary_index(boundary)
    if not found then
        return false
    end
    return start_at_index(scoring_team, assert(index))
end

---@return { count: integer, oldest_boundary: integer?, newest_boundary: integer?, boundaries: integer[], phase: string?, celebration_elapsed: number }
function replay.diagnostics()
    local boundaries = {}
    for index, frame in ipairs(buf) do
        boundaries[index] = frame._replay_boundary
    end
    return {
        count = #buf,
        oldest_boundary = buf[1] and buf[1]._replay_boundary or nil,
        newest_boundary = buf[#buf] and buf[#buf]._replay_boundary or nil,
        boundaries = boundaries,
        phase = phase,
        celebration_elapsed = cel_elapsed,
    }
end

---@param boundary integer
---@return { ball_x: number, ball_y: number, score_home: integer, score_away: integer }?
function replay.boundary_sample(boundary)
    local index, found = boundary_index(boundary)
    if not found then
        return nil
    end
    local frame = buf[assert(index)]
    return {
        ball_x = frame.ball.x,
        ball_y = frame.ball.y,
        score_home = frame.score.home,
        score_away = frame.score.away,
    }
end

-- One celebration frame: players ease from their goal-moment poses to their
-- targets (scorer leads, teammates set off a beat later), ball sat in the net.
---@return ReplayFrame
local function celebration_frame()
    local T = CELEBRATE_SECONDS
    local st = {}
    for k, v in pairs(cel_base) do
        st[k] = v
    end
    local players = {}
    for j, p in ipairs(cel_base.players) do
        local cp = {}
        for k, v in pairs(p) do
            cp[k] = v
        end
        local tgt = cel_targets[p.id]
        if tgt then
            local u
            if p.id == cel_scorer then
                u = smooth(cel_elapsed / T)
            else
                -- Teammates lead-in later so the scorer visibly sets off first.
                u = smooth((cel_elapsed - MATE_DELAY * T) / ((1 - MATE_DELAY) * T))
            end
            cp.pos = p.pos:add(tgt:sub(p.pos):scale(u))
            local d = tgt:sub(p.pos)
            if u > 0.02 and u < 0.98 and d:length() > 1 then
                cp.facing = d:normalized()
            end
        end
        players[j] = cp
    end
    st.players = players
    st.ball = cel_ball
    st.ball_z = 0
    st.ball_vel = Vec2.new(0, 0)
    st.ball_vz = 0
    st.events = {}
    ---@cast st ReplayFrame
    return st
end

-- Advance the sequence and return an interpolated, drawable MatchState-like
-- table — or nil when it has finished. During playback `state.events` carries
-- each recorded frame's events exactly once (for the effects layer), regardless
-- of how slowly the playhead crawls. (Typed as MatchState for the renderer's
-- benefit: it carries every field the draw path reads.)
---@param dt number
---@return ReplayFrame? state
function replay.step(dt)
    if phase == "celebrate" then
        cel_elapsed = cel_elapsed + dt
        local st = celebration_frame()
        if cel_elapsed >= CELEBRATE_SECONDS then
            phase = "playback" -- next step cuts to the slow-motion replay
        end
        return st
    end
    if phase ~= "playback" then
        return nil
    end
    playhead = playhead + dt * 60 * tuning.values.REPLAY_SLOWMO
    local i = math.floor(playhead)
    if i >= #window then
        phase = nil
        window = {}
        active_end_boundary = nil
        return nil
    end
    local a, b = window[i], window[i + 1]
    local t = playhead - i
    local st = {}
    for k, v in pairs(a) do
        st[k] = v
    end
    st.ball = a.ball:add(b.ball:sub(a.ball):scale(t))
    st.ball_z = a.ball_z + (b.ball_z - a.ball_z) * t
    local players = {}
    for j, pa in ipairs(a.players) do
        local pb = b.players[j]
        local cp = {}
        for k, v in pairs(pa) do
            cp[k] = v
        end
        cp.pos = pa.pos:add(pb.pos:sub(pa.pos):scale(t))
        players[j] = cp
    end
    st.players = players
    st.events = (i > emitted) and a.events or {}
    emitted = math.max(emitted, i)
    ---@cast st ReplayFrame
    return st
end

return replay
