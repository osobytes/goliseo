-- Headless match batches preserve the legacy MatchInput fixture by default.
-- Supplying frames or slot_sources opts into fixed-slot input production. Both
-- paths fold into fun-proxy metrics (sim/metrics.lua). Pure — no love, no I/O;
-- `report` returns a string and the caller decides where it goes. Entry point:
-- `love . --sim [n]` in main.lua.

local Vec2 = require("core.vec2")
local match = require("sim.match")
local bot = require("sim.bot")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local metrics = require("sim.metrics")
local slot_input = require("sim.slot_input")
local tuning = require("sim.tuning")
local teams = require("data.teams")

local headless = {}

local FIELD = { w = 960, h = 540 } -- the real game's pitch (game/screens/match.lua)
local DT = fixed_clock.TICK_SECONDS
local DEFAULT_DURATION = 120
local DEFAULT_MAX_GOALS = match.NO_GOAL_LIMIT -- decided on score at full time
local MAX_STEPS_SLACK = 600 -- overtime guard: a stuck sim must not hang the batch

---@type MatchInput
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
    equipment_held = false,
    equipment_pressed = false,
    equipment_released = false,
}

---@alias HeadlessBot "home"|"none"

---@class HeadlessOpts
---@field seed number
---@field duration number?
---@field max_goals integer?
---@field reaction number?  -- bot latency override
---@field tuning_blob string?  -- knob overrides in sim/tuning serialize format
---@field home TeamData?
---@field away TeamData?
---@field home_formation string?
---@field away_formation string?
---@field tactic TacticData?
---@field away_tactic TacticData?
---@field players_by_id table<string, PlayerData>?
---@field species_by_id table<string, SpeciesData>?
---@field field { w: number, h: number }?
---@field bot HeadlessBot?  -- defaults to "home"; "none" is match AI vs match AI
---@field frames InputFrame[]? -- Complete canonical recorded rows, indexed by tick + 1.
---@field slot_sources MatchSlotSource[]? -- Upstream frame/bot/neutral producer policy.

---@class MatchResult
---@field seed number
---@field score { home: integer, away: integer }
---@field winner "home"|"away"|nil
---@field metrics MatchMetrics  -- includes `fun` (the composite score)
---@field desirability table<string, number>  -- per-banded-metric 0..1

-- `match.new` currently has no away-formation override. Keep that match-layer
-- API unchanged by making a per-run TeamData view; canonical team data remains
-- immutable and the existing team.formation path still builds the away side.
---@param team TeamData
---@param formation string?
---@return TeamData
local function with_formation(team, formation)
    if not formation then
        return team
    end
    return {
        id = team.id,
        name = team.name,
        color = team.color,
        formation = formation,
        roster = team.roster,
    }
end

-- Play one full match and measure it. Applies `tuning_blob` on top of the
-- defaults for the run and restores the previous knob values afterwards, so
-- batches never leak balance state into the live game or other configs.
---@param opts HeadlessOpts
---@return MatchResult
function headless.run_match(opts)
    local saved = opts.tuning_blob ~= nil and tuning.serialize() or nil
    if opts.tuning_blob then
        tuning.deserialize(opts.tuning_blob)
    end

    local bot_mode = opts.bot or "home"
    assert(bot_mode == "home" or bot_mode == "none", "unknown headless bot mode: " .. bot_mode)
    local field = opts.field or FIELD
    local duration = opts.duration or DEFAULT_DURATION
    local home = opts.home or teams.nebula
    local away = with_formation(opts.away or teams.orion, opts.away_formation)
    local slot_mode = opts.frames ~= nil or opts.slot_sources ~= nil
    local match_opts = {
        home = home,
        away = away,
        field = { w = field.w, h = field.h },
        home_formation = opts.home_formation,
        tactic = opts.tactic,
        away_tactic = opts.away_tactic,
        players_by_id = opts.players_by_id,
        species_by_id = opts.species_by_id,
        human_controlled = bot_mode == "home",
        duration = duration,
        max_goals = opts.max_goals or DEFAULT_MAX_GOALS,
        seed = opts.seed,
    }
    if slot_mode then
        match_opts.input_ownership = match.ownership_for_teams(home, away, opts.players_by_id)
    end
    local s = match.new(match_opts)
    ---@type SlotInputProducerState?
    local producer = nil
    if slot_mode then
        local sources = opts.slot_sources
        if not sources then
            sources = {}
            for index = 1, input_frame.SLOT_COUNT do
                -- A recording is complete by definition: without an explicit
                -- override, every canonical row comes from its tape.
                sources[index] = { kind = "frame" }
            end
        end
        producer = slot_input.new_producer(sources)
    end
    ---@type BotState?
    local b = nil
    if not slot_mode and bot_mode == "home" then
        b = bot.new({ seed = opts.seed, reaction = opts.reaction })
    end
    -- Metrics only fold MatchState/events; they never read bot or controlled-side
    -- state, so the same collector is valid for home-proxy and all-AI fixtures.
    local c = metrics.new(s)
    local clock = fixed_clock.new()

    local max_steps = math.ceil(duration / DT) + MAX_STEPS_SLACK
    for _ = 1, max_steps do
        if s.finished then
            break
        end
        local input
        if producer then
            local base_frame
            if opts.frames then
                base_frame =
                    assert(opts.frames[clock.tick + 1], "recorded frame missing for headless tick")
            else
                base_frame = assert(input_frame.neutral(clock.tick))
            end
            input = slot_input.materialize(producer, s, base_frame)
        else
            input = b and bot.input(b, s, DT) or NO_INPUT
        end
        local _, continue = fixed_clock.step(clock, input, function(_, tick_input)
            match.step(s, DT, tick_input)
            metrics.observe(c, s, DT)
            return not s.finished
        end)
        if not continue then
            break
        end
    end

    if saved ~= nil then
        tuning.deserialize(saved)
    end

    local m = metrics.finish(c, s)
    local fun, per = metrics.fun_score(m)
    m.fun = fun
    local winner = nil
    if s.score.home > s.score.away then
        winner = "home"
    elseif s.score.away > s.score.home then
        winner = "away"
    end
    return {
        seed = opts.seed,
        score = { home = s.score.home, away = s.score.away },
        winner = winner,
        metrics = m,
        desirability = per,
    }
end

---@class BatchOpts
---@field n integer?  -- number of matches (seeds 1..n) when `seeds` not given
---@field seeds number[]?
---@field duration number?
---@field max_goals integer?
---@field reaction number?
---@field tuning_blob string?
---@field home TeamData?
---@field away TeamData?
---@field home_formation string?
---@field away_formation string?
---@field tactic TacticData?
---@field away_tactic TacticData?
---@field players_by_id table<string, PlayerData>?
---@field species_by_id table<string, SpeciesData>?
---@field field { w: number, h: number }?
---@field bot HeadlessBot?
---@field frames InputFrame[]?
---@field slot_sources MatchSlotSource[]?

---@class BatchResult
---@field matches MatchResult[]
---@field agg table<string, MetricStats>

-- Play a batch over a fixed seed set. Compare knob configs on the SAME seeds
-- (common random numbers): differences then come from the knobs, not luck.
---@param opts BatchOpts
---@return BatchResult
function headless.run_batch(opts)
    local seeds = opts.seeds
    if not seeds then
        seeds = {}
        for i = 1, opts.n or 20 do
            seeds[i] = i
        end
    end
    local matches, all = {}, {}
    for _, seed in ipairs(seeds) do
        local r = headless.run_match({
            seed = seed,
            duration = opts.duration,
            max_goals = opts.max_goals,
            reaction = opts.reaction,
            tuning_blob = opts.tuning_blob,
            home = opts.home,
            away = opts.away,
            home_formation = opts.home_formation,
            away_formation = opts.away_formation,
            tactic = opts.tactic,
            away_tactic = opts.away_tactic,
            players_by_id = opts.players_by_id,
            species_by_id = opts.species_by_id,
            field = opts.field,
            bot = opts.bot,
            frames = opts.frames,
            slot_sources = opts.slot_sources,
        })
        matches[#matches + 1] = r
        all[#all + 1] = r.metrics
    end
    return { matches = matches, agg = metrics.aggregate(all) }
end

-- Display order: the banded fun-proxy metrics first, context counts after.
local REPORT_ROWS = {
    "fun",
    "goals_total",
    "shots_per_goal",
    "save_rate",
    "pass_completion",
    "turnovers_per_min",
    "possession_balance",
    "longest_drought_s",
    "decided_late",
    "lead_changes",
    "margin",
    "shots",
    "passes",
    "controlled_dribble_carry_s",
    "controlled_dribble_close_share",
    "controlled_dribble_sprint_share",
    "controlled_dribble_juke_share",
    "controlled_dribble_touches_per_min",
    "controlled_dribble_heavy_losses_per_min",
    "controlled_jukes",
    "ai_dribble_carry_s",
    "ai_dribble_close_share",
    "ai_dribble_sprint_share",
    "ai_dribble_juke_share",
    "ai_dribble_touches_per_min",
    "ai_dribble_heavy_losses_per_min",
    "ai_jukes",
    "keeper_base_s",
    "keeper_advance_s",
    "keeper_contain_s",
    "keeper_set_s",
    "keeper_retreat_s",
    "keeper_recover_s",
    "keeper_shot_depth_mean",
    "chip_shots",
    "chip_on_target",
    "chip_goals",
    "chip_conversion",
    "keeper_saves_base",
    "keeper_saves_advance",
    "keeper_saves_contain",
    "keeper_saves_set",
    "keeper_saves_retreat",
    "keeper_saves_recover",
    "keeper_saves_unclassified",
    "keeper_goals_base",
    "keeper_goals_advance",
    "keeper_goals_contain",
    "keeper_goals_set",
    "keeper_goals_retreat",
    "keeper_goals_recover",
    "keeper_goals_unclassified",
    "duration",
}

---@param batch BatchResult
---@return string
function headless.report(batch)
    local out = {}
    local n = #batch.matches
    -- Mean per-metric desirability across the batch: when the fun score is
    -- low, this column names the dimension that collapsed.
    local desir = {}
    for _, r in ipairs(batch.matches) do
        for k, d in pairs(r.desirability) do
            desir[k] = desir[k] or { sum = 0, n = 0 }
            desir[k].sum = desir[k].sum + d
            desir[k].n = desir[k].n + 1
        end
    end
    out[#out + 1] = ("fun-proxy metrics over %d matches (mean +/- sd [min .. max])"):format(n)
    out[#out + 1] = ("%-20s %10s %9s %9s %9s %7s  %s"):format(
        "metric",
        "mean",
        "sd",
        "min",
        "max",
        "desir",
        "band"
    )
    for _, key in ipairs(REPORT_ROWS) do
        local st = batch.agg[key]
        if st then
            local band = metrics.bands[key]
            local band_str = band and ("[%g .. %g]"):format(band[2], band[3]) or ""
            local d = desir[key]
            local d_str = d and ("%7.2f"):format(d.sum / d.n) or ("%7s"):format("-")
            local miss = st.n < n and (" (n=%d)"):format(st.n) or ""
            out[#out + 1] = ("%-20s %10.3f %9.3f %9.3f %9.3f %s  %s%s"):format(
                key,
                st.mean,
                st.sd,
                st.min,
                st.max,
                d_str,
                band_str,
                miss
            )
        end
    end
    return table.concat(out, "\n")
end

return headless
