local t = require("spec.support.runner")
local frozen = require("data.outfield_ai_baseline")
local outfield_ai_baseline = require("sim.outfield_ai_baseline")
local outfield_ai_policy = require("sim.outfield_ai_policy")
local tripwire = require("sim.tripwire")
local tuning = require("sim.tuning")

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

    t.it("keeps its seeds clear of every other locked block", function()
        -- Locked in docs/design/combat_fun_evidence_contract.md §3.3. The
        -- holdout blocks matter most: a control that quietly overlapped
        -- 30001..30060 would spend the untouched confirmatory set.
        local reserved = {
            { name = "adversarial", first = 21001, last = 21060 },
            { name = "untouched holdout", first = 30001, last = 30060 },
            { name = "replacement holdout", first = 31001, last = 31060 },
        }
        for _, seed in ipairs(outfield_ai_baseline.seeds()) do
            t.is_true(seed > tripwire.DEFAULT_N, "seed " .. seed .. " collides with the tripwire")
            t.is_true(seed < 1001 or seed > 1060, "seed " .. seed .. " is a spent evaluation seed")
            for _, block in ipairs(reserved) do
                t.is_true(
                    seed < block.first or seed > block.last,
                    "seed " .. seed .. " collides with the " .. block.name .. " block"
                )
            end
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

    t.it("keeps baseline_version out of the signature", function()
        -- The freeze counter must not enter the evidence hash, or a re-freeze
        -- that changes nothing would look like a changed recording.
        local bumped = clone(frozen)
        bumped.baseline_version = frozen.baseline_version + 7
        t.eq(outfield_ai_baseline.signature(bumped), frozen.signature)
    end)

    t.it("passes when a record is compared against itself", function()
        local comparison = outfield_ai_baseline.compare(clone(frozen), clone(frozen))
        t.is_true(comparison.ok)
        t.is_true(comparison.metrics_ok)
        t.is_true(comparison.identity_ok)
        t.is_true(comparison.signature_ok)
        t.is_true(not comparison.stale)
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

    t.it("warns about a stale identity without blocking unrelated work", function()
        -- Identity covers all 40 knob defaults and every authored roster, so it
        -- moves for edits that provably cannot change combat-disabled play.
        -- Those must stay visible without taxing an unrelated branch with the
        -- `--refreeze-ack` ceremony.
        local current = clone(frozen)
        current.identity.tuning_hash = "deadbeefdeadbeef"
        current.signature = outfield_ai_baseline.signature(current)
        local comparison = outfield_ai_baseline.compare(clone(frozen), current)
        t.is_true(comparison.ok, "identical play must not fail the shared gate")
        t.is_true(comparison.stale, "but the stale identity is still called out")
        t.is_true(not comparison.identity_ok)
        t.is_true(not comparison.signature_ok)
        for _, row in ipairs(comparison.rows) do
            t.is_true(row.ok, "no metric moved")
        end
        local report = outfield_ai_baseline.report(comparison, clone(frozen), current)
        t.is_true(report:find("AI BASELINE STALE", 1, true) ~= nil, "the report names staleness")
        t.is_true(report:find("tuning_hash", 1, true) ~= nil, "and which identity field moved")
        t.is_true(report:find("drift-log entry", 1, true) ~= nil, "and still demands the drift log")
        t.is_true(report:find("AI BASELINE MOVED", 1, true) == nil, "without claiming a failure")
    end)

    t.it("still fails when a changed policy actually moves a metric", function()
        local current = clone(frozen)
        current.identity.policy_id = "outfield_ai_policy/v1/combat_disabled/deadbeefdeadbeef"
        current.stats.goals_total.mean = current.stats.goals_total.mean + 0.25
        current.signature = outfield_ai_baseline.signature(current)
        local comparison = outfield_ai_baseline.compare(clone(frozen), current)
        t.is_true(not comparison.ok, "moved play under a new policy is a hard failure")
        t.is_true(not comparison.stale, "which is a failure, not a staleness warning")
        t.is_true(not comparison.identity_ok)
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

    t.it("detects a real policy change by re-running the fixture", function()
        -- The end-to-end half of the guarantee: mutate a hashed policy
        -- constant, play the SAME seeds again, and require the recording to
        -- move. `tuning_blob = ""` resets live values from the defaults, so
        -- moving a default really does change how the match is played.
        local seeds = { outfield_ai_baseline.SEED_FIRST, outfield_ai_baseline.SEED_FIRST + 1 }
        local before = outfield_ai_baseline.measure({ seeds = seeds })
        local knob = assert(tuning.by_key["AI_SHOOT_RANGE"])
        local previous = knob.default
        knob.default = knob.max
        local ok, after = pcall(outfield_ai_baseline.measure, { seeds = seeds })
        knob.default = previous
        tuning.reset()
        assert(ok, after)
        ---@cast after OutfieldAiBaselineRecord

        local comparison = outfield_ai_baseline.compare(before, after)
        t.is_true(
            not comparison.ok,
            "doubling AI shoot range must move the recording, not be absorbed"
        )
        t.is_true(not comparison.identity_ok, "and it is visible as a policy change")
        t.is_true(
            before.identity.policy_id ~= after.identity.policy_id,
            "the policy id moves with the declared constant"
        )
        local moved = 0
        for _, row in ipairs(comparison.rows) do
            if not row.ok then
                moved = moved + 1
            end
        end
        t.is_true(moved > 0, "at least one tracked metric reports MOVED")

        -- And the mutation left nothing behind for later specs.
        t.eq(outfield_ai_baseline.identity().policy_id, frozen.identity.policy_id)
    end)

    t.it("cannot mistake a probe run for the frozen freeze", function()
        local probe = outfield_ai_baseline.measure({
            seeds = { outfield_ai_baseline.SEED_FIRST },
        })
        t.eq(probe.identity.seed_count, 1)
        t.is_true(probe.identity.seed_hash ~= frozen.identity.seed_hash, "different seed set")
        t.is_true(probe.identity.fixture_hash ~= frozen.identity.fixture_hash, "different fixture")
        local comparison = outfield_ai_baseline.compare(clone(frozen), probe)
        t.is_true(not comparison.identity_ok, "a truncated run can never satisfy the freeze")
        t.is_true(
            probe.identity.seed_count ~= frozen.identity.seed_count,
            "the recorded seed count is what actually ran"
        )
    end)
end)
