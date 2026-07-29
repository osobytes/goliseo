local t = require("spec.support.runner")
local frozen = require("data.outfield_ai_baseline")
local outfield_ai_baseline = require("sim.outfield_ai_baseline")
local outfield_ai_policy = require("sim.outfield_ai_policy")
local tripwire = require("sim.tripwire")

-- The frozen record is shared module state; never hand a mutable copy of it to
-- a comparison test.
---@param record OutfieldAiBaselineRecord
---@return OutfieldAiBaselineRecord
local function clone(record)
    local identity = {}
    for _, field in ipairs(outfield_ai_baseline.IDENTITY_FIELDS) do
        identity[field] = record.identity[field]
    end
    local stats = {}
    for _, key in ipairs(outfield_ai_baseline.TRACKED) do
        local stat = record.stats[key]
        stats[key] = { n = stat.n, mean = stat.mean, sd = stat.sd, min = stat.min, max = stat.max }
    end
    return {
        baseline_version = record.baseline_version,
        identity = identity --[[@as OutfieldAiBaselineIdentity]],
        stats = stats,
        signature = record.signature,
    }
end

t.describe("sim.outfield_ai_baseline", function()
    t.it("declares an explicit, contiguous seed set", function()
        local seeds = outfield_ai_baseline.seeds()
        t.eq(#seeds, outfield_ai_baseline.SEED_COUNT)
        t.eq(seeds[1], outfield_ai_baseline.SEED_FIRST)
        t.eq(seeds[#seeds], outfield_ai_baseline.SEED_FIRST + outfield_ai_baseline.SEED_COUNT - 1)
        for index = 2, #seeds do
            t.eq(seeds[index], seeds[index - 1] + 1, "the declared block is contiguous")
        end
    end)

    t.it("keeps its seeds clear of the soccer tripwire and historical evaluation sets", function()
        -- Locked in docs/design/combat_fun_evidence_contract.md §3.3: the
        -- tripwire owns 1..30 and 1001..1060 is spent evaluation history.
        for _, seed in ipairs(outfield_ai_baseline.seeds()) do
            t.is_true(seed > tripwire.DEFAULT_N, "seed " .. seed .. " collides with the tripwire")
            t.is_true(seed < 1001 or seed > 1060, "seed " .. seed .. " is a spent evaluation seed")
        end
    end)

    t.it("is a separate artifact from the soccer fun tripwire", function()
        -- #128 §4.4: data/fun_baseline.lua stays combat-disabled AND stays
        -- untouched by this work; the two must not share a file or a schema.
        local fun_baseline = require("data.fun_baseline")
        t.eq(fun_baseline.n, tripwire.DEFAULT_N, "the fun tripwire baseline is untouched")
        t.is_true(fun_baseline.identity == nil, "and carries none of this artifact's identity")
        t.is_true(frozen.identity ~= nil, "while this artifact is identity-bearing")
    end)

    t.it("freezes an identity that still describes this build", function()
        local live = outfield_ai_baseline.identity()
        for _, field in ipairs(outfield_ai_baseline.IDENTITY_FIELDS) do
            t.eq(
                tostring(frozen.identity[field]),
                tostring(live[field]),
                "frozen identity field " .. field .. " no longer matches this build"
            )
        end
        t.eq(frozen.identity.policy_id, outfield_ai_policy.id())
    end)

    t.it("records every tracked metric with usable variance", function()
        for _, key in ipairs(outfield_ai_baseline.TRACKED) do
            local stat = frozen.stats[key]
            t.is_true(stat ~= nil, "missing frozen metric " .. key)
            t.is_true(stat.n >= 1, key .. " has no contributing matches")
            t.is_true(stat.n <= frozen.identity.seed_count, key .. " counts more than it ran")
            for _, field in ipairs(outfield_ai_baseline.STAT_FIELDS) do
                local value = stat[field]
                t.is_true(type(value) == "number", key .. "." .. field .. " is numeric")
                t.is_true(value == value, key .. "." .. field .. " is not NaN")
            end
            t.is_true(stat.min <= stat.mean, key .. " min <= mean")
            t.is_true(stat.mean <= stat.max, key .. " mean <= max")
            t.is_true(stat.sd >= 0, key .. " has non-negative sd")
        end
    end)

    t.it("carries a self-consistent signature", function()
        -- Recomputing over the file's own contents catches a hand-edited number
        -- that never went through `--ai-baseline write`.
        t.eq(outfield_ai_baseline.signature(frozen), frozen.signature)
    end)

    t.it("passes when a record is compared against itself", function()
        local comparison = outfield_ai_baseline.compare(clone(frozen), clone(frozen))
        t.is_true(comparison.ok)
        t.is_true(comparison.identity_ok)
        t.is_true(comparison.signature_ok)
        t.eq(#comparison.rows, #outfield_ai_baseline.TRACKED)
    end)

    t.it("flags a moved metric instead of absorbing it", function()
        local current = clone(frozen)
        current.stats.pass_completion.mean = current.stats.pass_completion.mean + 1e-9
        current.signature = outfield_ai_baseline.signature(current)
        local comparison = outfield_ai_baseline.compare(clone(frozen), current)
        t.is_true(not comparison.ok, "an exact baseline must not absorb drift")
        t.is_true(comparison.identity_ok, "the fixture itself is unchanged")
        t.is_true(not comparison.signature_ok)
        local moved = {}
        for _, row in ipairs(comparison.rows) do
            if not row.ok then
                moved[#moved + 1] = row.key
                t.eq(table.concat(row.moved, ","), "mean", "and names the moved field")
            end
        end
        t.eq(#moved, 1, "only the moved metric is flagged")
        t.eq(moved[1], "pass_completion")
    end)

    t.it("flags a changed policy even when every metric matches", function()
        local current = clone(frozen)
        current.identity.policy_id = "outfield_ai_policy/v1/combat_disabled/deadbeefdeadbeef"
        current.signature = outfield_ai_baseline.signature(current)
        local comparison = outfield_ai_baseline.compare(clone(frozen), current)
        t.is_true(not comparison.ok, "identical numbers under a new policy is still a mismatch")
        t.is_true(not comparison.identity_ok)
        for _, row in ipairs(comparison.rows) do
            t.is_true(row.ok, "no metric moved")
        end
    end)

    t.it("tells the reader not to refresh a failure away", function()
        local current = clone(frozen)
        current.stats.goals_total.mean = current.stats.goals_total.mean + 0.5
        current.signature = outfield_ai_baseline.signature(current)
        local comparison = outfield_ai_baseline.compare(clone(frozen), current)
        local report = outfield_ai_baseline.report(comparison, clone(frozen), current)
        t.is_true(report:find("AI BASELINE MOVED", 1, true) ~= nil, "the report names the failure")
        t.is_true(report:find("deletes the evidence", 1, true) ~= nil, "and the reason not to")
        t.is_true(report:find("--refreeze-ack", 1, true) ~= nil, "and the deliberate path")
    end)

    t.it("serializes a loadable record that round-trips exactly", function()
        -- Exact comparison is only sound if the on-disk decimal form loses
        -- nothing, so this pins the serializer's precision, not its layout.
        local chunk = outfield_ai_baseline.serialize(clone(frozen))
        local loaded = assert(loadstring(chunk))() --[[@as OutfieldAiBaselineRecord]]
        t.eq(loaded.baseline_version, frozen.baseline_version)
        t.eq(loaded.signature, frozen.signature)
        for _, field in ipairs(outfield_ai_baseline.IDENTITY_FIELDS) do
            t.eq(tostring(loaded.identity[field]), tostring(frozen.identity[field]), field)
        end
        for _, key in ipairs(outfield_ai_baseline.TRACKED) do
            for _, field in ipairs(outfield_ai_baseline.STAT_FIELDS) do
                t.is_true(
                    loaded.stats[key][field] == frozen.stats[key][field],
                    key .. "." .. field .. " round-trips bit-for-bit"
                )
            end
        end
        t.eq(outfield_ai_baseline.signature(loaded), frozen.signature)
    end)

    t.it("reproduces a fresh run of the fixture exactly", function()
        -- A two-seed probe of the real code path: the full 60-seed
        -- verification is `love . --ai-baseline` in scripts/check.sh, but
        -- exact comparison is only defensible if measurement is reproducible.
        local seeds = { outfield_ai_baseline.SEED_FIRST, outfield_ai_baseline.SEED_FIRST + 1 }
        local first = outfield_ai_baseline.measure({ seeds = seeds })
        local second = outfield_ai_baseline.measure({ seeds = seeds })
        local comparison = outfield_ai_baseline.compare(first, second)
        t.is_true(comparison.ok, "the same seeds must produce the same recording")
        t.eq(first.signature, second.signature)
    end)

    t.it("cannot mistake a probe run for the frozen freeze", function()
        local probe = outfield_ai_baseline.measure({
            seeds = { outfield_ai_baseline.SEED_FIRST },
        })
        t.eq(probe.identity.seed_count, 1)
        t.is_true(probe.identity.seed_hash ~= frozen.identity.seed_hash, "different seed set")
        t.is_true(probe.identity.fixture_hash ~= frozen.identity.fixture_hash, "different fixture")
        local comparison = outfield_ai_baseline.compare(clone(frozen), probe)
        t.is_true(not comparison.ok, "a truncated run can never satisfy the freeze")
        t.is_true(not comparison.identity_ok)
    end)
end)
