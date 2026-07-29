-- Frozen combat-disabled Outfield AI common-seed baseline (#59).
--
-- What this is: a checked-in, versioned recording of how the frozen gameplay-AI
-- policy (`sim.outfield_ai_policy`) plays a declared fixture over a declared
-- seed set, with full identity so #148/#149 can cite it instead of copying it.
-- It is the fixture-A control every "combat changed X" claim is measured
-- against; without it such a claim is unfalsifiable.
--
-- What this is NOT: the soccer fun tripwire. `sim/tripwire.lua` and
-- `data/fun_baseline.lua` are a human-proxy 30-seed smoke test with a 5%
-- tolerance band, and the locked evidence contract (#128 §4.4) forbids
-- refreshing them from a combat fixture. This artifact is separate on purpose:
-- different seeds, different fixture, all-AI sides, and an exact comparison.
--
-- Non-refresh rule. `love . --ai-baseline` verifies; it never writes. Writing
-- needs `love . --ai-baseline write --refreeze-ack`, which bumps
-- `baseline_version`, so a re-freeze is always a visible, attributable diff. A
-- failing verification is evidence, not a chore: refreshing to make it green
-- destroys the only record that the control moved.
--
-- Comparison is EXACT. The batch is deterministic per seed (see
-- docs/design/fun_metrics.md, "Determinism & variance"), values round-trip
-- through `%.17g`, and this is a frozen control rather than a drift band, so
-- any movement is a real finding. The report names the moved metric.
--
-- Pure module: no love, no I/O. `main.lua` owns reading and writing the file.

local fnv1a64 = require("core.fnv1a64")
local fixed_clock = require("sim.fixed_clock")
local headless = require("sim.headless")
local input_frame = require("sim.input_frame")
local match_snapshot = require("sim.match_snapshot")
local outfield_ai_policy = require("sim.outfield_ai_policy")
local tuning = require("sim.tuning")
local formations = require("data.formations")
local players = require("data.players")
local showcase = require("data.showcase_player_compatibility")
local species = require("data.species")
local tactics = require("data.tactics")
local teams = require("data.teams")

---@class OutfieldAiBaselineModule
local outfield_ai_baseline = {}

outfield_ai_baseline.SCHEMA = "outfield_ai_baseline"
outfield_ai_baseline.SCHEMA_VERSION = 1
outfield_ai_baseline.FIXTURE = "combat_disabled_control_a"

-- The locked paired calibration/common-seed block from the accepted evidence
-- contract (docs/design/combat_fun_evidence_contract.md §3.3). #149 runs its
-- combat-active arm on these same seeds, so this control is a paired control
-- under common random numbers rather than an independent sample. The soccer
-- tripwire's seeds 1..30 and the historical evaluation seeds 1001..1060 stay
-- out of it.
outfield_ai_baseline.SEED_FIRST = 20001
outfield_ai_baseline.SEED_COUNT = 60

outfield_ai_baseline.DURATION_SECONDS = 120
outfield_ai_baseline.MAX_GOALS = 3
outfield_ai_baseline.FIELD = { w = 960, h = 540 }

-- All-AI sides. The human-proxy bot in `sim.bot` is a separate policy with its
-- own weaknesses (docs/design/fun_metrics.md, "The human proxy"); mixing it in
-- would make this a baseline of the proxy, not of the gameplay AI.
outfield_ai_baseline.BOT = "none"

-- Tracked metrics: the soccer-integrity family the combat calibration must not
-- damage, plus the AI dribble diagnostics. Ordered; the order is hashed.
outfield_ai_baseline.TRACKED = {
    "fun",
    "goals_total",
    "goals_home",
    "goals_away",
    "shots",
    "shots_per_goal",
    "save_rate",
    "passes",
    "pass_completion",
    "turnovers_per_min",
    "possession_balance",
    "longest_drought_s",
    "decided_late",
    "lead_changes",
    "margin",
    "duration",
    "ai_dribble_carry_s",
    "ai_dribble_close_share",
    "ai_dribble_sprint_share",
    "ai_dribble_juke_share",
    "ai_dribble_touches_per_min",
    "ai_dribble_heavy_losses_per_min",
    "ai_jukes",
}

-- Per-metric fields, in hash and comparison order.
outfield_ai_baseline.STAT_FIELDS = { "n", "mean", "sd", "min", "max" }

local STAT_KEYS = { "pace", "strength", "technique", "stamina", "mental" }

---@class OutfieldAiBaselineStat
---@field n integer  -- matches contributing; 0 when no match had a denominator
---@field mean number
---@field sd number
---@field min number
---@field max number

---@class OutfieldAiBaselineIdentity
---@field schema string
---@field schema_version integer
---@field policy_id string
---@field fixture string
---@field fixture_hash string
---@field config string
---@field config_hash string
---@field content_hash string
---@field tuning_hash string
---@field snapshot_version integer
---@field input_version integer
---@field tick_rate integer
---@field seed_first integer
---@field seed_count integer
---@field seed_hash string  -- over the exact seed list, not just first/count

---@class OutfieldAiBaselineRecord
---@field baseline_version integer  -- bumped by every deliberate re-freeze
---@field identity OutfieldAiBaselineIdentity
---@field stats table<string, OutfieldAiBaselineStat>
---@field signature string  -- over identity + stats; excludes baseline_version

---@param parts string[]
---@param value number|string
local function append(parts, value)
    local kind = type(value)
    if kind == "number" then
        ---@cast value number
        parts[#parts + 1] = "n" .. match_snapshot.number_bytes(value) .. ";"
    else
        assert(kind == "string", "baseline identity values must be numbers or strings")
        ---@cast value string
        parts[#parts + 1] = "s" .. tostring(#value) .. ":" .. value .. ";"
    end
end

-- The declared seed set, materialized.
---@return integer[]
function outfield_ai_baseline.seeds()
    local out = {}
    for index = 0, outfield_ai_baseline.SEED_COUNT - 1 do
        out[index + 1] = outfield_ai_baseline.SEED_FIRST + index
    end
    return out
end

-- Everything about the run that is not the AI policy or the content.
---@return string
function outfield_ai_baseline.config()
    return ("field=%dx%d;duration=%d;max_goals=%d;tick_rate=%d;bot=%s;combat=%s;tactic=%s"):format(
        outfield_ai_baseline.FIELD.w,
        outfield_ai_baseline.FIELD.h,
        outfield_ai_baseline.DURATION_SECONDS,
        outfield_ai_baseline.MAX_GOALS,
        fixed_clock.TICK_RATE,
        outfield_ai_baseline.BOT,
        outfield_ai_policy.COMBAT_MODE,
        tactics.balanced.id
    )
end

-- Hash of the authored content the fixture actually instantiates: both teams,
-- every rostered player's mechanical stats, the species modifiers applied to
-- them, both formations' anchors, and the shared tactic. A content edit that
-- changes play therefore invalidates the baseline even though the AI policy is
-- untouched — which is the honest outcome, since the recorded numbers moved.
---@return string
function outfield_ai_baseline.content_hash()
    ---@type table<string, PlayerData>
    local by_id = {}
    for _, player in ipairs(players) do
        by_id[player.id] = player
    end
    local parts = { "GCOAC;1;" }
    local sides = {
        { key = "home", team = teams.nebula },
        { key = "away", team = teams.orion },
    }
    for _, side in ipairs(sides) do
        local team = side.team
        append(parts, side.key)
        append(parts, team.id)
        append(parts, team.formation)
        append(parts, #team.roster)
        for _, player_id in ipairs(team.roster) do
            local player = assert(by_id[player_id], "baseline roster names an unknown player")
            append(parts, player.id)
            append(parts, player.number)
            append(parts, player.position)
            for _, stat in ipairs(STAT_KEYS) do
                append(parts, stat)
                append(parts, player.stats[stat])
            end
            local compatibility = showcase[player_id]
            local species_id = compatibility and compatibility.species or "neutral"
            local species_data =
                assert(species[species_id], "baseline player names an unknown species")
            append(parts, "species")
            append(parts, species_data.id)
            for _, stat in ipairs(STAT_KEYS) do
                append(parts, species_data.modifiers[stat])
            end
        end
        local formation =
            assert(formations[team.formation], "baseline team names an unknown formation")
        append(parts, formation.id)
        append(parts, formation.keeper.x)
        append(parts, formation.keeper.y)
        append(parts, #formation.outfield)
        for _, anchor in ipairs(formation.outfield) do
            append(parts, anchor.role)
            append(parts, anchor.x)
            append(parts, anchor.y)
        end
    end
    local tactic = tactics.balanced
    append(parts, tactic.id)
    append(parts, tactic.press)
    append(parts, tactic.line_shift)
    append(parts, tactic.stamina_drain)
    append(parts, tactic.marking.scheme)
    append(parts, tactic.marking.man_marks)
    append(parts, tactic.marking.standoff)
    append(parts, tactic.marking.compactness)
    append(parts, tactic.marking.support)
    append(parts, tactic.transition.counterpress)
    append(parts, tactic.transition.counterattack)
    return fnv1a64.hash(table.concat(parts))
end

-- Hash over every shipped knob default, including the ones the policy id
-- deliberately excludes. Movement and dribble defaults are world rules rather
-- than AI decisions, but they still move the recorded numbers.
---@return string
function outfield_ai_baseline.tuning_hash()
    local parts = { "GCOAT;1;" }
    for _, knob in ipairs(tuning.knobs) do
        append(parts, knob.key)
        append(parts, knob.default)
    end
    return fnv1a64.hash(table.concat(parts))
end

-- The complete citable identity of the fixture, resolved from live modules.
-- `seeds` exists so a cheap probe run records what it ACTUALLY ran: its
-- identity then differs from the frozen 60-seed one and can never be mistaken
-- for the freeze.
---@param seeds integer[]?
---@return OutfieldAiBaselineIdentity
function outfield_ai_baseline.identity(seeds)
    seeds = seeds or outfield_ai_baseline.seeds()
    assert(#seeds > 0, "a baseline needs at least one seed")
    local seed_parts = { "GCOAS;1;" }
    for _, seed in ipairs(seeds) do
        append(seed_parts, seed)
    end
    local config = outfield_ai_baseline.config()
    ---@type OutfieldAiBaselineIdentity
    local identity = {
        schema = outfield_ai_baseline.SCHEMA,
        schema_version = outfield_ai_baseline.SCHEMA_VERSION,
        policy_id = outfield_ai_policy.id(),
        fixture = outfield_ai_baseline.FIXTURE,
        fixture_hash = "",
        config = config,
        config_hash = fnv1a64.hash(config),
        content_hash = outfield_ai_baseline.content_hash(),
        tuning_hash = outfield_ai_baseline.tuning_hash(),
        snapshot_version = match_snapshot.VERSION,
        input_version = input_frame.VERSION,
        tick_rate = fixed_clock.TICK_RATE,
        seed_first = seeds[1],
        seed_count = #seeds,
        seed_hash = fnv1a64.hash(table.concat(seed_parts)),
    }
    local parts = { "GCOAF;1;" }
    append(parts, identity.policy_id)
    append(parts, identity.fixture)
    append(parts, identity.config_hash)
    append(parts, identity.content_hash)
    append(parts, identity.tuning_hash)
    append(parts, identity.snapshot_version)
    append(parts, identity.input_version)
    append(parts, identity.seed_hash)
    identity.fixture_hash = fnv1a64.hash(table.concat(parts))
    return identity
end

-- Identity fields compared field by field, in report order.
outfield_ai_baseline.IDENTITY_FIELDS = {
    "schema",
    "schema_version",
    "policy_id",
    "fixture",
    "config",
    "config_hash",
    "content_hash",
    "tuning_hash",
    "snapshot_version",
    "input_version",
    "tick_rate",
    "seed_first",
    "seed_count",
    "seed_hash",
    "fixture_hash",
}

-- Content hash of the evidence itself. Deliberately excludes
-- `baseline_version`, so a re-freeze that changes nothing shows up in git as a
-- lone version bump instead of hiding inside a churned file.
---@param record OutfieldAiBaselineRecord
---@return string
function outfield_ai_baseline.signature(record)
    local parts = { "GCOAB;", tostring(outfield_ai_baseline.SCHEMA_VERSION), ";" }
    for _, field in ipairs(outfield_ai_baseline.IDENTITY_FIELDS) do
        local value = record.identity[field]
        assert(value ~= nil, "baseline identity is missing field " .. field)
        append(parts, value)
    end
    for _, key in ipairs(outfield_ai_baseline.TRACKED) do
        local stat = record.stats[key]
        assert(stat, "baseline is missing tracked metric " .. key)
        append(parts, key)
        for _, field in ipairs(outfield_ai_baseline.STAT_FIELDS) do
            append(parts, stat[field])
        end
    end
    return fnv1a64.hash(table.concat(parts))
end

---@class OutfieldAiBaselineMeasureOpts
---@field baseline_version integer?
---@field seeds integer[]?  -- probe override; the identity records what ran

-- Run the declared fixture over the declared seeds and record it.
---@param opts OutfieldAiBaselineMeasureOpts?
---@return OutfieldAiBaselineRecord
function outfield_ai_baseline.measure(opts)
    opts = opts or {}
    local seeds = opts.seeds or outfield_ai_baseline.seeds()
    local batch = headless.run_batch({
        seeds = seeds,
        duration = outfield_ai_baseline.DURATION_SECONDS,
        max_goals = outfield_ai_baseline.MAX_GOALS,
        field = outfield_ai_baseline.FIELD,
        bot = outfield_ai_baseline.BOT,
        -- Empty blob = every knob at its default, applied and restored per
        -- match, so a stray in-process nudge cannot leak into the freeze.
        tuning_blob = "",
    })
    ---@type table<string, OutfieldAiBaselineStat>
    local stats = {}
    for _, key in ipairs(outfield_ai_baseline.TRACKED) do
        -- A metric with no denominator in any match (shots per goal across
        -- goalless matches) is absent from the aggregate. Record it as a
        -- zero-support row rather than dropping it: `n` carries the fact, and
        -- the schema stays the same width whatever the seeds produced.
        local aggregate = batch.agg[key] or { n = 0, mean = 0, sd = 0, min = 0, max = 0 }
        stats[key] = {
            n = aggregate.n,
            mean = aggregate.mean,
            sd = aggregate.sd,
            min = aggregate.min,
            max = aggregate.max,
        }
    end
    ---@type OutfieldAiBaselineRecord
    local record = {
        baseline_version = opts.baseline_version or 1,
        identity = outfield_ai_baseline.identity(seeds),
        stats = stats,
        signature = "",
    }
    record.signature = outfield_ai_baseline.signature(record)
    return record
end

---@class OutfieldAiBaselineIdentityRow
---@field key string
---@field base string
---@field cur string
---@field ok boolean

---@class OutfieldAiBaselineMetricRow
---@field key string
---@field base number  -- baseline mean
---@field cur number  -- measured mean
---@field delta number
---@field moved string[]  -- stat fields that differ, e.g. {"mean", "sd"}
---@field ok boolean

---@class OutfieldAiBaselineComparison
---@field ok boolean
---@field identity_ok boolean
---@field signature_ok boolean
---@field identity_rows OutfieldAiBaselineIdentityRow[]
---@field rows OutfieldAiBaselineMetricRow[]

-- Compare a frozen record against a fresh measurement. Exact, per §"Comparison
-- is EXACT" above.
---@param baseline OutfieldAiBaselineRecord
---@param current OutfieldAiBaselineRecord
---@return OutfieldAiBaselineComparison
function outfield_ai_baseline.compare(baseline, current)
    ---@type OutfieldAiBaselineIdentityRow[]
    local identity_rows = {}
    local identity_ok = true
    for _, field in ipairs(outfield_ai_baseline.IDENTITY_FIELDS) do
        local base = tostring((baseline.identity or {})[field])
        local cur = tostring(current.identity[field])
        local row_ok = base == cur
        identity_ok = identity_ok and row_ok
        identity_rows[#identity_rows + 1] = { key = field, base = base, cur = cur, ok = row_ok }
    end

    ---@type OutfieldAiBaselineMetricRow[]
    local rows = {}
    local metrics_ok = true
    for _, key in ipairs(outfield_ai_baseline.TRACKED) do
        local base = baseline.stats and baseline.stats[key] or nil
        local cur = current.stats[key]
        local moved = {}
        for _, field in ipairs(outfield_ai_baseline.STAT_FIELDS) do
            if not base or base[field] ~= cur[field] then
                moved[#moved + 1] = field
            end
        end
        local row_ok = #moved == 0
        metrics_ok = metrics_ok and row_ok
        rows[#rows + 1] = {
            key = key,
            base = base and base.mean or 0,
            cur = cur.mean,
            delta = cur.mean - (base and base.mean or 0),
            moved = moved,
            ok = row_ok,
        }
    end

    local signature_ok = baseline.signature == current.signature
    return {
        ok = identity_ok and metrics_ok and signature_ok,
        identity_ok = identity_ok,
        signature_ok = signature_ok,
        identity_rows = identity_rows,
        rows = rows,
    }
end

---@param comparison OutfieldAiBaselineComparison
---@param baseline OutfieldAiBaselineRecord
---@param current OutfieldAiBaselineRecord
---@return string
function outfield_ai_baseline.report(comparison, baseline, current)
    local lines = {
        ("outfield AI baseline: %s, seeds %d..%d, combat %s"):format(
            outfield_ai_baseline.FIXTURE,
            outfield_ai_baseline.SEED_FIRST,
            outfield_ai_baseline.SEED_FIRST + outfield_ai_baseline.SEED_COUNT - 1,
            outfield_ai_policy.COMBAT_MODE
        ),
        ("frozen v%d vs data/outfield_ai_baseline.lua"):format(baseline.baseline_version or 0),
        ("policy   %s"):format(current.identity.policy_id),
        ("fixture  %s"):format(current.identity.fixture_hash),
        ("signature base=%s now=%s"):format(baseline.signature or "-", current.signature),
    }
    if not comparison.identity_ok then
        lines[#lines + 1] = "IDENTITY MISMATCH — the frozen fixture is not the one measured:"
        for _, row in ipairs(comparison.identity_rows) do
            if not row.ok then
                lines[#lines + 1] = ("  %-18s base=%s now=%s"):format(row.key, row.base, row.cur)
            end
        end
    end
    lines[#lines + 1] = ("%-32s %14s %14s %14s"):format("metric", "base mean", "now mean", "delta")
    for _, row in ipairs(comparison.rows) do
        lines[#lines + 1] = ("%-32s %14.6f %14.6f %+14.6f  %s"):format(
            row.key,
            row.base,
            row.cur,
            row.delta,
            row.ok and "ok" or ("MOVED[" .. table.concat(row.moved, ",") .. "]")
        )
    end
    if comparison.ok then
        lines[#lines + 1] = "AI BASELINE OK"
    else
        lines[#lines + 1] = "AI BASELINE MOVED — the frozen combat-disabled control is no longer"
        lines[#lines + 1] = "what this build produces. This is a finding, not a chore: #148/#149"
        lines[#lines + 1] = "cite this artifact, so refreshing it to go green deletes the evidence."
        lines[#lines + 1] = "Confirm the change is intended, record it in the drift log of"
        lines[#lines + 1] = "docs/design/fun_metrics.md, then re-freeze deliberately with"
        lines[#lines + 1] = "`love . --ai-baseline write --refreeze-ack` (bumps baseline_version)."
    end
    return table.concat(lines, "\n")
end

---@param value number
---@return string
local function number_literal(value)
    assert(value == value and value ~= math.huge and value ~= -math.huge, "baseline value is NaN")
    if value == math.floor(value) and math.abs(value) < 1e15 then
        return ("%d"):format(value)
    end
    return ("%.17g"):format(value)
end

-- Baseline file content (data/outfield_ai_baseline.lua). Stable order; the
-- numeric form round-trips exactly so verification can compare exactly.
---@param record OutfieldAiBaselineRecord
---@return string
function outfield_ai_baseline.serialize(record)
    local lines = {
        "-- Frozen combat-disabled Outfield AI baseline (#59). DO NOT hand-edit and",
        "-- DO NOT refresh to silence a failing `love . --ai-baseline`: #148/#149 cite",
        "-- this artifact as their control, so a moved baseline is evidence.",
        "--",
        "-- A deliberate re-freeze is:",
        "--   1. confirm the change is intended and record it in the drift log of",
        "--      docs/design/fun_metrics.md;",
        "--   2. `love . --ai-baseline write --refreeze-ack` (bumps baseline_version).",
        "--",
        "-- See sim/outfield_ai_baseline.lua and docs/design/fun_metrics.md.",
        "",
        "---@type OutfieldAiBaselineRecord",
        "return {",
        ("    baseline_version = %d,"):format(record.baseline_version),
        "    identity = {",
    }
    for _, field in ipairs(outfield_ai_baseline.IDENTITY_FIELDS) do
        local value = record.identity[field]
        if type(value) == "number" then
            lines[#lines + 1] = ("        %s = %s,"):format(field, number_literal(value))
        else
            lines[#lines + 1] = ("        %s = %q,"):format(field, tostring(value))
        end
    end
    lines[#lines + 1] = "    },"
    lines[#lines + 1] = "    stats = {"
    for _, key in ipairs(outfield_ai_baseline.TRACKED) do
        local stat = record.stats[key]
        -- One field per line: StyLua would expand a wide inline table anyway,
        -- and `./scripts/check.sh` must pass on generated output.
        lines[#lines + 1] = ("        %s = {"):format(key)
        for _, field in ipairs(outfield_ai_baseline.STAT_FIELDS) do
            lines[#lines + 1] = ("            %s = %s,"):format(field, number_literal(stat[field]))
        end
        lines[#lines + 1] = "        },"
    end
    lines[#lines + 1] = "    },"
    lines[#lines + 1] = ("    signature = %q,"):format(record.signature)
    lines[#lines + 1] = "}"
    lines[#lines + 1] = ""
    return table.concat(lines, "\n")
end

return outfield_ai_baseline
