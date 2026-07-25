-- Synthesized audio for the match screen. All SFX are generated at load time
-- from mathematical waveforms — no asset files. Each SFX is ≤ 0.4 s at 22050 Hz
-- mono. Offline playback keeps the state-event adapter; rollback playback
-- consumes confirmed stable events. The crowd bed is a continuous low loop.
--
-- HEADLESS CONTRACT: conf.lua disables t.modules.audio and t.modules.sound in
-- test mode, making love.audio and love.sound nil. Every function in this module
-- must no-op cleanly in that case — this is the module's first-class contract,
-- not an afterthought.

local combat_feedback = require("game.presentation.combat_feedback")

---@class AudioModule
local audio = {}

local RATE = 22050
local BITS = 16
local CHANNELS = 1

-- Volume levels
local BASE_SFX_VOLUME = 0.5
local BASE_CROWD_VOLUME = 0.08
local BASE_GOAL_SWELL_VOLUME = 0.22
local CUE_GAIN = {
    touch = 0.45,
    reception = 0.5,
    pass = 0.62,
    block = 0.68,
    catch = 0.7,
    claim = 0.7,
    parry = 0.76,
    tackle = 0.84,
    header = 0.86,
    shot = 1.0,
    volley = 1.0,
    bicycle = 1.0,
    kickoff = 0.82,
    goal = 1.15,
    full_time = 1.12,
    combat_commit = 0.5,
    combat_projectile = 0.62,
    combat_expire = 0.42,
    combat_hit = 0.9,
    combat_extended = 0.82,
    combat_guarded = 0.78,
    combat_immune = 0.54,
    combat_superseded = 0.48,
    combat_spill = 0.84,
    combat_forced = 0.88,
    combat_recoil = 0.7,
    combat_rejected = 0.42,
}

local _muted = false
local _loaded = false
local _sfx_mix = 1
local _crowd_mix = 1

-- Sources (built at load; nil until loaded or when headless)
---@type love.Source?
local _crowd_src = nil

---@type table<string, love.Source>
local _sfx = {}

-- Goal/kickoff detection
local _prev_score_home = 0
local _prev_score_away = 0
local _prev_finished = false
local _prev_time_left = -1
local _crowd_swell_t = 0.0 -- countdown for crowd swell (seconds remaining)
local CROWD_SWELL_DUR = 3.0
---@type table<string, boolean>
local _confirmed_ids = {}
---@type table<string, integer>
local _confirmed_cues = {}
---@type table<string, integer>
local _suppressed_confirmed_cues = {}

---@return number
local function sfx_volume()
    return BASE_SFX_VOLUME * _sfx_mix
end

---@return number
local function crowd_volume()
    return BASE_CROWD_VOLUME * _crowd_mix
end

---@return number
local function goal_swell_volume()
    return math.min(1, BASE_GOAL_SWELL_VOLUME * _crowd_mix)
end

-- ---------------------------------------------------------------------------
-- Low-level sample builders (all pure, no love calls)
-- ---------------------------------------------------------------------------

--- Write a mono sine with exponential decay into a SoundData.
---@param sd love.SoundData
---@param freq number  Hz
---@param decay number  exponent (larger = faster fade)
---@param amp number  peak amplitude 0..1
local function write_sine_decay(sd, freq, decay, amp)
    local n = sd:getSampleCount()
    for i = 0, n - 1 do
        local t = i / RATE
        local s = math.sin(2 * math.pi * freq * t) * math.exp(-decay * t) * amp
        sd:setSample(i, math.max(-1, math.min(1, s)))
    end
end

--- Write a triangle wave with exponential decay (brighter timbre than sine).
---@param sd love.SoundData
---@param freq number
---@param decay number
---@param amp number
local function write_tri_decay(sd, freq, decay, amp)
    local n = sd:getSampleCount()
    local period = RATE / freq
    for i = 0, n - 1 do
        local t = i / RATE
        local phase = (i % period) / period -- 0..1
        local tri = 2 * math.abs(2 * phase - 1) - 1 -- triangle -1..1
        local s = tri * math.exp(-decay * t) * amp
        sd:setSample(i, math.max(-1, math.min(1, s)))
    end
end

--- Write seeded white noise with exponential decay.
---@param sd love.SoundData
---@param decay number
---@param amp number
local function write_noise_decay(sd, decay, amp)
    local n = sd:getSampleCount()
    -- Use a simple LCG so the noise is deterministic across runs.
    local seed = 12345
    for i = 0, n - 1 do
        local t = i / RATE
        seed = (seed * 1664525 + 1013904223) % (2 ^ 32)
        local r = (seed / (2 ^ 31)) - 1 -- -1..1
        local s = r * math.exp(-decay * t) * amp
        sd:setSample(i, math.max(-1, math.min(1, s)))
    end
end

--- Mix a second waveform additively into existing samples (clamps to -1..1).
---@param sd love.SoundData
---@param freq number
---@param decay number
---@param amp number
local function mix_sine(sd, freq, decay, amp)
    local n = sd:getSampleCount()
    for i = 0, n - 1 do
        local t = i / RATE
        local add = math.sin(2 * math.pi * freq * t) * math.exp(-decay * t) * amp
        local cur = sd:getSample(i)
        sd:setSample(i, math.max(-1, math.min(1, cur + add)))
    end
end

--- Build a SoundData with a given duration (seconds) and fill it via a callback.
---@param dur number  seconds (capped at 0.4)
---@param fill fun(sd: love.SoundData)
---@return love.Source
local function make_source(dur, fill)
    local samples = math.floor(math.min(dur, 0.4) * RATE)
    local sd = love.sound.newSoundData(samples, RATE, BITS, CHANNELS)
    fill(sd)
    local src = love.audio.newSource(sd, "static")
    src:setVolume(sfx_volume())
    return src
end

-- ---------------------------------------------------------------------------
-- SFX builders
-- ---------------------------------------------------------------------------

--- Soft tick — pass (quick sine tap, very short).
---@return love.Source
local function build_pass()
    return make_source(0.12, function(sd)
        write_sine_decay(sd, 880, 30, 0.7)
    end)
end

--- Touch — first touch / collection (softer tick, slight pitch drop via mix).
---@return love.Source
local function build_touch()
    return make_source(0.10, function(sd)
        write_sine_decay(sd, 660, 28, 0.6)
        mix_sine(sd, 330, 40, 0.15)
    end)
end

--- Low thump — shot.
---@return love.Source
local function build_shot()
    return make_source(0.25, function(sd)
        write_noise_decay(sd, 18, 0.55)
        mix_sine(sd, 90, 14, 0.45)
    end)
end

--- Thud — tackle (noise burst, low-mid body).
---@return love.Source
local function build_tackle()
    return make_source(0.18, function(sd)
        write_noise_decay(sd, 22, 0.6)
        mix_sine(sd, 120, 20, 0.35)
    end)
end

--- Block — body-block ricochet (sharp noise + mid tone).
---@return love.Source
local function build_block()
    return make_source(0.15, function(sd)
        write_noise_decay(sd, 25, 0.7)
        mix_sine(sd, 200, 22, 0.3)
    end)
end

--- Catch / claim — slap (crisp noise + high ping).
---@return love.Source
local function build_catch()
    return make_source(0.14, function(sd)
        write_noise_decay(sd, 30, 0.8)
        mix_sine(sd, 1200, 35, 0.3)
    end)
end

--- Parry — sharp deflect (high freq ping + noise pop).
---@return love.Source
local function build_parry()
    return make_source(0.18, function(sd)
        write_sine_decay(sd, 1400, 28, 0.5)
        write_noise_decay(sd, 35, 0.35)
    end)
end

--- Header — flick (mid triangle, short).
---@return love.Source
local function build_header()
    return make_source(0.15, function(sd)
        write_tri_decay(sd, 420, 24, 0.55)
    end)
end

--- Volley — heavy thump (low noise + deep sine).
---@return love.Source
local function build_volley()
    return make_source(0.3, function(sd)
        write_noise_decay(sd, 14, 0.65)
        mix_sine(sd, 70, 10, 0.5)
    end)
end

--- Goal — rising two-note + noise swell.
---@return love.Source
local function build_goal()
    -- Two rising sine tones + noise swell, 0.4 s
    return make_source(0.4, function(sd)
        local n = sd:getSampleCount()
        local seed = 99991
        for i = 0, n - 1 do
            local t = i / RATE
            local frac = t / 0.4
            -- Glide from 400 → 800 Hz over the duration
            local freq = 400 + 400 * frac
            local tone1 = math.sin(2 * math.pi * freq * t) * 0.4
            -- Second note a minor third above
            local freq2 = freq * 1.2
            local tone2 = math.sin(2 * math.pi * freq2 * t) * 0.3
            -- Noise swell that grows with time
            seed = (seed * 1664525 + 1013904223) % (2 ^ 32)
            local r = (seed / (2 ^ 31)) - 1
            local noise = r * frac * 0.25
            local env = math.exp(-3 * (1 - frac)) -- ramp in then slight decay
            local s = (tone1 + tone2 + noise) * env
            sd:setSample(i, math.max(-1, math.min(1, s)))
        end
    end)
end

--- Kickoff whistle — two short blasts of a high triangle.
---@return love.Source
local function build_kickoff()
    return make_source(0.35, function(sd)
        local n = sd:getSampleCount()
        local period1_end = math.floor(0.12 * RATE)
        local gap_end = math.floor(0.20 * RATE)
        local period = RATE / 880
        for i = 0, n - 1 do
            local s = 0.0
            if i < period1_end or i >= gap_end then
                local t = i / RATE
                local decay
                if i < period1_end then
                    decay = i / period1_end
                else
                    decay = (n - 1 - i) / (n - 1 - gap_end)
                end
                local phase = (i % period) / period
                local tri = 2 * math.abs(2 * phase - 1) - 1
                s = tri * decay * 0.65
            end
            sd:setSample(i, math.max(-1, math.min(1, s)))
        end
    end)
end

--- Full-time signal — two whistle bursts over a low closing tone.
---@return love.Source
local function build_full_time()
    return make_source(0.4, function(sd)
        local n = sd:getSampleCount()
        local period = RATE / 1040
        for i = 0, n - 1 do
            local t = i / RATE
            local whistle = 0
            if t < 0.13 or (t >= 0.2 and t < 0.38) then
                local local_t = t < 0.13 and t or (t - 0.2)
                local env = math.max(0, 1 - local_t / 0.18)
                local phase = (i % period) / period
                whistle = (2 * math.abs(2 * phase - 1) - 1) * env * 0.55
            end
            local closing = math.sin(2 * math.pi * 130 * t) * math.exp(-5 * t) * 0.2
            sd:setSample(i, math.max(-1, math.min(1, whistle + closing)))
        end
    end)
end

--- Quiet crowd bed — low-pass-like filtered noise loop.
---@return love.Source
local function build_crowd()
    -- 2 seconds of noise smoothed with a simple IIR to simulate crowd rumble
    local samples = 2 * RATE
    local sd = love.sound.newSoundData(samples, RATE, BITS, CHANNELS)
    local seed = 55555
    local prev = 0.0
    local alpha = 0.06 -- low-pass coefficient (smaller = more muffled)
    for i = 0, samples - 1 do
        seed = (seed * 1664525 + 1013904223) % (2 ^ 32)
        local r = (seed / (2 ^ 31)) - 1
        prev = prev + alpha * (r - prev)
        sd:setSample(i, math.max(-1, math.min(1, prev * 2.5)))
    end
    local src = love.audio.newSource(sd, "static")
    src:setLooping(true)
    src:setVolume(crowd_volume())
    return src
end

-- ---------------------------------------------------------------------------
-- Public API
-- ---------------------------------------------------------------------------

--- Load (synthesize) all SFX. Must be called once before update/reset.
--- No-ops when love.audio or love.sound is nil (headless test mode).
function audio.load()
    if not love.audio or not love.sound then
        return
    end
    if _loaded then
        return
    end
    _loaded = true

    _sfx["pass"] = build_pass()
    _sfx["touch"] = build_touch()
    _sfx["shot"] = build_shot()
    _sfx["tackle"] = build_tackle()
    _sfx["block"] = build_block()
    _sfx["catch"] = build_catch()
    _sfx["claim"] = build_catch() -- claim uses same profile as catch
    _sfx["parry"] = build_parry()
    _sfx["header"] = build_header()
    _sfx["volley"] = build_volley()
    _sfx["bicycle"] = build_volley()
    _sfx["reception"] = build_pass()
    _sfx["goal"] = build_goal()
    _sfx["kickoff"] = build_kickoff()
    _sfx["full_time"] = build_full_time()
    _sfx["combat_commit"] = build_pass()
    _sfx["combat_projectile"] = build_parry()
    _sfx["combat_expire"] = build_touch()
    _sfx["combat_hit"] = build_tackle()
    _sfx["combat_extended"] = build_tackle()
    _sfx["combat_guarded"] = build_block()
    _sfx["combat_immune"] = build_parry()
    _sfx["combat_superseded"] = build_pass()
    _sfx["combat_spill"] = build_catch()
    _sfx["combat_forced"] = build_volley()
    _sfx["combat_recoil"] = build_block()
    _sfx["combat_rejected"] = build_touch()

    _crowd_src = build_crowd()
    if not _muted then
        _crowd_src:play()
    end
end

--- Play a named SFX by cloning the static source (allows overlapping hits).
--- No-ops when muted or headless.
---@param name string
local function play(name)
    if _muted or not love.audio then
        return
    end
    local src = _sfx[name]
    if not src then
        return
    end
    local clone = src:clone()
    clone:setVolume(math.min(1, sfx_volume() * (CUE_GAIN[name] or 1)))
    clone:play()
end

---@param name string
local function count_confirmed_cue(name)
    _confirmed_cues[name] = (_confirmed_cues[name] or 0) + 1
end

local function start_goal_swell()
    _crowd_swell_t = CROWD_SWELL_DUR
    if _crowd_src and not _muted then
        _crowd_src:setVolume(goal_swell_volume())
    end
end

-- Consume one stable event only after its rollback step is confirmed. Event
-- identity, rather than mutable score/time edges, is the deduplication key.
---@param event RollbackWrappedEvent
---@param suppress_playback boolean?
---@return boolean consumed
function audio.consume_confirmed(event, suppress_playback)
    assert(
        type(event) == "table" and type(event.id) == "string",
        "confirmed audio event is invalid"
    )
    if _confirmed_ids[event.id] then
        return false
    end
    _confirmed_ids[event.id] = true

    local name = nil
    if event.domain:sub(1, 6) == "match/" then
        local kind = event.payload.kind
        if CUE_GAIN[kind] then
            name = kind
        end
    elseif event.domain:sub(1, 7) == "combat/" then
        local payload = event.payload --[[@as CombatEvent]]
        local link = combat_feedback.accepted(payload, event.id)
        name = link.disposition.audio
    elseif event.domain == "lifecycle/goal" then
        name = "goal"
    elseif event.domain == "lifecycle/kickoff" then
        name = "kickoff"
    elseif event.domain == "lifecycle/full_time" then
        name = "full_time"
    end
    if name == nil then
        return true
    end

    if suppress_playback then
        _suppressed_confirmed_cues[name] = (_suppressed_confirmed_cues[name] or 0) + 1
        return true
    end
    count_confirmed_cue(name)
    play(name)
    if name == "goal" then
        start_goal_swell()
    end
    return true
end

---@param events CombatEvent[]
function audio.consume_combat(events)
    for _, event in ipairs(events) do
        local name = combat_feedback.disposition(event).audio
        if name then
            play(name)
        end
    end
end

---@return table<string, integer>
function audio.confirmed_cue_counts()
    local result = {}
    for name, count in pairs(_confirmed_cues) do
        result[name] = count
    end
    return result
end

---@return table<string, integer>
function audio.suppressed_confirmed_cue_counts()
    local result = {}
    for name, count in pairs(_suppressed_confirmed_cues) do
        result[name] = count
    end
    return result
end

--- Advance continuous ambience without consuming or replaying match events.
---@param dt number
function audio.tick(dt)
    if not love.audio or not love.sound then
        return
    end
    if _crowd_swell_t > 0 then
        _crowd_swell_t = _crowd_swell_t - dt
        if _crowd_swell_t <= 0 then
            _crowd_swell_t = 0
            if _crowd_src and not _muted then
                _crowd_src:setVolume(crowd_volume())
            end
        end
    end
    if _crowd_src and not _muted and not _crowd_src:isPlaying() then
        _crowd_src:play()
    end
end

-- Consume one simulated state's events and score edges. Continuous ambience is
-- deliberately advanced separately by `tick` at the display's cadence.
---@param state MatchState
function audio.consume(state)
    if not love.audio or not love.sound then
        return
    end

    -- Drain event queue
    for _, evt in ipairs(state.events) do
        local kind = evt.kind
        if
            kind == "shot"
            or kind == "pass"
            or kind == "touch"
            or kind == "tackle"
            or kind == "block"
            or kind == "catch"
            or kind == "claim"
            or kind == "parry"
            or kind == "header"
            or kind == "volley"
            or kind == "bicycle"
            or kind == "reception"
        then
            play(kind)
        end
    end

    -- Goal detection via score edges
    local scored = false
    if state.score.home ~= _prev_score_home or state.score.away ~= _prev_score_away then
        scored = true
    end
    _prev_score_home = state.score.home
    _prev_score_away = state.score.away

    if scored then
        play("goal")
        start_goal_swell()
    end

    if state.finished and not _prev_finished then
        play("full_time")
    end
    _prev_finished = state.finished

    -- Kickoff edge: time reset upward (new match or post-goal)
    if _prev_time_left >= 0 and state.time_left > _prev_time_left + 1 then
        play("kickoff")
    end
    _prev_time_left = state.time_left
end

-- Compatibility helper for callers that have one simulation step per render
-- frame. Fixed-clock callers consume every tick then advance ambience once.
---@param state MatchState
---@param dt number
function audio.update(state, dt)
    audio.consume(state)
    audio.tick(dt)
end

--- Reset audio state for a new match (stop crowd swell, re-sync score refs).
--- Plays the kickoff whistle for the fresh start.
function audio.reset()
    _prev_score_home = 0
    _prev_score_away = 0
    _crowd_swell_t = 0
    _prev_time_left = -1
    _prev_finished = false
    _confirmed_ids = {}
    _confirmed_cues = {}
    _suppressed_confirmed_cues = {}

    if not love.audio or not love.sound then
        return
    end

    -- Return crowd to quiet level
    if _crowd_src then
        _crowd_src:setVolume(_muted and 0 or crowd_volume())
        if not _muted and not _crowd_src:isPlaying() then
            _crowd_src:play()
        end
    end

    -- Kickoff whistle for the fresh match start
    play("kickoff")
end

---@param value boolean
---@return boolean muted
function audio.set_muted(value)
    _muted = value
    if not love.audio then
        return _muted
    end
    if _crowd_src then
        if _muted then
            _crowd_src:setVolume(0)
        else
            _crowd_src:setVolume(_crowd_swell_t > 0 and goal_swell_volume() or crowd_volume())
            if not _crowd_src:isPlaying() then
                _crowd_src:play()
            end
        end
    end
    return _muted
end

--- Toggle mute. Returns the new muted state.
---@return boolean muted
function audio.toggle_mute()
    return audio.set_muted(not _muted)
end

---@param settings GameSettings
function audio.configure(settings)
    _sfx_mix = settings.sfx_volume / 0.8
    _crowd_mix = settings.crowd_volume / 0.55
    for _, source in pairs(_sfx) do
        source:setVolume(sfx_volume())
    end
    audio.set_muted(settings.muted)
end

return audio
