-- Match-grain liveness for manager levers. Each comparison runs fixture A and
-- fixture B on the same seeds (common random numbers), then reports effect
-- sizes: home win-rate percentage points and mean shifts normalized by each
-- match metric's good-band width. No standard-error threshold is used.
--
-- This is only half 1 of manager-mode's headless-first ship-gate. Passing
-- here means a lever is perceptible, not that it is a decision: a lever is
-- ships-eligible only after decision_contingency (#0005) passes as well.

local headless = require("sim.headless")
local metrics = require("sim.metrics")
local players = require("data.players")
local presets = require("data.tuning_presets")
local tactics = require("data.tactics")
local teams = require("data.teams")

---@class LeverMetricDelta
---@field key string
---@field n integer  -- seeds with finite observations for both A and B
---@field mean_a number
---@field mean_b number
---@field delta number  -- signed raw mean shift, A - B
---@field band_width number  -- good_hi - good_lo
---@field band_widths number  -- signed normalized shift, (A - B) / band_width

---@class LeverLivenessResult
---@field seeds integer
---@field win_rate_a number  -- home-win share for fixture A, 0..1
---@field win_rate_b number  -- home-win share for fixture B, 0..1
---@field dwin_pts number  -- signed percentage points, A home wins - B home wins
---@field win_in_band boolean  -- |dwin_pts| is in the 3..20 pp good band
---@field metric_deltas LeverMetricDelta[]
---@field moved_metrics LeverMetricDelta[]  -- |band_widths| >= 0.5
---@field passes boolean  -- win_in_band AND at least one moved metric

---@class LeverDefinition
---@field id string
---@field name string
---@field option_a string
---@field option_b string
---@field fixture_a BatchOpts
---@field fixture_b BatchOpts

---@class LeverRun
---@field lever LeverDefinition
---@field result LeverLivenessResult

---@class LeverMetrics
---@field WIN_GOOD_LO number
---@field WIN_GOOD_HI number
---@field METRIC_MOVE_MIN number
local lever_metrics = {
    WIN_GOOD_LO = 3,
    WIN_GOOD_HI = 20,
    METRIC_MOVE_MIN = 0.5,
}

---@param v any
---@return boolean
local function is_finite_number(v)
    return type(v) == "number" and v == v and v > -math.huge and v < math.huge
end

---@param opts BatchOpts
---@param seeds number[]
---@return BatchOpts
local function with_seeds(opts, seeds)
    local copy = {}
    for key, value in pairs(opts) do
        copy[key] = value
    end
    copy.seeds = seeds
    return copy
end

---@param batch BatchResult
---@return number rate
local function home_win_rate(batch)
    local wins = 0
    for _, match_result in ipairs(batch.matches) do
        if match_result.winner == "home" then
            wins = wins + 1
        end
    end
    return wins / #batch.matches
end

---@param a BatchResult
---@param b BatchResult
---@return LeverMetricDelta[] deltas
local function metric_deltas(a, b)
    local keys = {}
    for key in pairs(metrics.bands) do
        keys[#keys + 1] = key
    end
    table.sort(keys)

    local deltas = {}
    for _, key in ipairs(keys) do
        local band = metrics.bands[key]
        local width = band[3] - band[2]
        if is_finite_number(width) and width > 0 then
            local n, sum_a, sum_b = 0, 0, 0
            for i = 1, #a.matches do
                local value_a = a.matches[i].metrics[key]
                local value_b = b.matches[i].metrics[key]
                -- Rate metrics can be nil when their denominator did not
                -- occur. Keep the common-seed pairing honest by admitting a
                -- seed only when both fixture observations are valid.
                if is_finite_number(value_a) and is_finite_number(value_b) then
                    n = n + 1
                    sum_a = sum_a + value_a
                    sum_b = sum_b + value_b
                end
            end
            if n > 0 then
                local mean_a = sum_a / n
                local mean_b = sum_b / n
                local delta = mean_a - mean_b
                deltas[#deltas + 1] = {
                    key = key,
                    n = n,
                    mean_a = mean_a,
                    mean_b = mean_b,
                    delta = delta,
                    band_width = width,
                    band_widths = delta / width,
                }
            end
        end
    end
    return deltas
end

-- Compare two alternatives applied to the home side. The sign is always
-- fixture A minus fixture B: draws count as no home win, so dwin_pts is
-- 100 * (share of seeds A's home side wins - share B's home side wins).
-- Liveness is direction-agnostic and gates on |dwin_pts|.
---@param fixture_a BatchOpts
---@param fixture_b BatchOpts
---@param seeds number[]
---@return LeverLivenessResult
function lever_metrics.lever_liveness(fixture_a, fixture_b, seeds)
    assert(#seeds > 0, "lever_liveness needs at least one seed")
    local a = headless.run_batch(with_seeds(fixture_a, seeds))
    local b = headless.run_batch(with_seeds(fixture_b, seeds))
    assert(#a.matches == #b.matches, "paired fixtures produced different batch sizes")

    local win_rate_a = home_win_rate(a)
    local win_rate_b = home_win_rate(b)
    local dwin_pts = (win_rate_a - win_rate_b) * 100
    local win_magnitude = math.abs(dwin_pts)
    local win_in_band = win_magnitude >= lever_metrics.WIN_GOOD_LO
        and win_magnitude <= lever_metrics.WIN_GOOD_HI
    local deltas = metric_deltas(a, b)
    local moved = {}
    for _, delta in ipairs(deltas) do
        if math.abs(delta.band_widths) >= lever_metrics.METRIC_MOVE_MIN then
            moved[#moved + 1] = delta
        end
    end

    return {
        seeds = #seeds,
        win_rate_a = win_rate_a,
        win_rate_b = win_rate_b,
        dwin_pts = dwin_pts,
        win_in_band = win_in_band,
        metric_deltas = deltas,
        moved_metrics = moved,
        passes = win_in_band and #moved > 0,
    }
end

---@param source table
---@param overrides table
---@return table
local function merged(source, overrides)
    local result = {}
    for key, value in pairs(source) do
        result[key] = value
    end
    for key, value in pairs(overrides) do
        result[key] = value
    end
    return result
end

---@param id string
---@return TuningPreset
local function preset_by_id(id)
    for _, preset in ipairs(presets) do
        if preset.id == id then
            return preset
        end
    end
    assert(false, "unknown tuning preset: " .. id)
    return presets[1]
end

---@return table<string, PlayerData>
local function players_by_id()
    local result = {}
    for _, player in ipairs(players) do
        result[player.id] = player
    end
    return result
end

---@param roster string[]
---@return TeamData
local function nebula_with_roster(roster)
    return {
        id = teams.nebula.id,
        name = teams.nebula.name,
        color = teams.nebula.color,
        formation = teams.nebula.formation,
        roster = roster,
    }
end

-- Candidate A is intentionally explicit: defaults have a broken scoring
-- signature, so a dead-looking lever there is not valid liveness evidence.
---@return string config_name
---@return LeverDefinition[] levers
function lever_metrics.built_ins()
    local preset = preset_by_id("candidate_a")
    local base = {
        home = teams.nebula,
        away = teams.orion,
        tactic = tactics.balanced,
        away_tactic = tactics.balanced,
        players_by_id = players_by_id(),
        bot = "none",
        tuning_blob = preset.blob,
    }
    local star_in = nebula_with_roster({
        "ozzo",
        "brakka",
        "veil_nyx",
        "rok_tann",
        "zyro_vex",
    })
    local star_benched = nebula_with_roster({
        "ozzo",
        "brakka",
        "veil_nyx",
        "rok_tann",
        "mika_olu",
    })
    return preset.name,
        {
            {
                id = "formation",
                name = "Formation",
                option_a = "2-1-1 Balanced",
                option_b = "1-1-2 Aggressive",
                fixture_a = merged(base, { home_formation = "2-1-1" }),
                fixture_b = merged(base, { home_formation = "1-1-2" }),
            },
            {
                id = "tactic",
                name = "Tactic",
                option_a = "Press High",
                option_b = "Counter Attack",
                fixture_a = merged(base, { tactic = tactics.press_high }),
                fixture_b = merged(base, { tactic = tactics.counter }),
            },
            {
                id = "star_swap",
                name = "Star swap",
                option_a = "Zyro Vex starts",
                option_b = "Mika Olu starts",
                fixture_a = merged(base, { home = star_in }),
                fixture_b = merged(base, { home = star_benched }),
            },
        }
end

---@param seeds number[]
---@param log fun(msg: string)?
---@return string config_name
---@return LeverRun[] runs
function lever_metrics.run_built_ins(seeds, log)
    local config_name, levers = lever_metrics.built_ins()
    local runs = {}
    for i, lever in ipairs(levers) do
        if log then
            log(("levers: %d/%d %s"):format(i, #levers, lever.name))
        end
        runs[#runs + 1] = {
            lever = lever,
            result = lever_metrics.lever_liveness(lever.fixture_a, lever.fixture_b, seeds),
        }
    end
    return config_name, runs
end

---@param config_name string
---@param runs LeverRun[]
---@return string report
function lever_metrics.report(config_name, runs)
    local out = {
        ("lever liveness — %s; AI/AI; paired common seeds"):format(config_name),
        "dWin is home win-rate percentage points (A - B); the gate uses |dWin| 3..20 pp.",
        "PASS here is only ship-gate half 1; decision_contingency (#0005) is still required.",
        ("%-12s %-21s %-21s %9s %17s %6s"):format(
            "lever",
            "A",
            "B",
            "dWin pp",
            "moved (band-w)",
            "gate"
        ),
    }
    for _, run in ipairs(runs) do
        local moved = {}
        for _, delta in ipairs(run.result.moved_metrics) do
            moved[#moved + 1] = ("%s %+.2f"):format(delta.key, delta.band_widths)
        end
        out[#out + 1] = ("%-12s %-21s %-21s %+9.1f %17s %6s"):format(
            run.lever.name,
            run.lever.option_a,
            run.lever.option_b,
            run.result.dwin_pts,
            #moved > 0 and table.concat(moved, ",") or "none",
            run.result.passes and "PASS" or "FAIL"
        )
        local all_deltas = {}
        for _, delta in ipairs(run.result.metric_deltas) do
            all_deltas[#all_deltas + 1] = ("%s=%+.2f(n=%d)"):format(
                delta.key,
                delta.band_widths,
                delta.n
            )
        end
        out[#out + 1] = ("  home win rates A/B: %.1f%% / %.1f%%; all band-width deltas A-B: %s"):format(
            run.result.win_rate_a * 100,
            run.result.win_rate_b * 100,
            table.concat(all_deltas, ", ")
        )
    end
    return table.concat(out, "\n")
end

return lever_metrics
