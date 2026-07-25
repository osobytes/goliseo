local determinism_evidence = require("sim.determinism_evidence")
local fixture = require("data.omp1_determinism")
local input_frame = require("sim.input_frame")
local input_tape = require("sim.input_tape")
local match_snapshot = require("sim.match_snapshot")
local placement = require("sim.placement")
local t = require("spec.support.runner")

t.describe("OMP-1 determinism evidence", function()
    t.it("pins the authoritative fixture to the current snapshot schema", function()
        t.eq(fixture.identity.snapshot_version, match_snapshot.VERSION)
        t.eq(match_snapshot.VERSION, 7)
    end)

    t.it("uses a total order for equal-distance match candidates", function()
        local before = placement.distance_candidate_before
        t.is_true(before({ idx = 9, d = 10 }, { idx = 2, d = 10 }))
        t.is_true(not before({ idx = 2, d = 10 }, { idx = 9, d = 10 }))
        t.is_true(before({ idx = 9, d = 9 }, { idx = 2, d = 10 }))
        t.is_true(not before({ idx = 2, d = 10 }, { idx = 9, d = 9 }))
    end)

    t.it("validates a current-input migration while changing only snapshot version", function()
        local prior = input_tape.copy_identity(fixture.identity)
        prior.snapshot_version = match_snapshot.VERSION - 1
        local migrated = determinism_evidence.migration_identity(prior)

        for _, field in ipairs(input_tape.IDENTITY_FIELDS) do
            if field ~= "snapshot_version" and field ~= "ownership" then
                t.eq(migrated[field], prior[field], "migration identity field " .. field)
            end
        end
        t.eq(migrated.snapshot_version, match_snapshot.VERSION)
        t.is_true(migrated.ownership ~= prior.ownership)
        local frozen_keeper = migrated.ownership.rosters.home[1]
        prior.ownership.rosters.home[1] = "mutated-after-copy"
        t.eq(migrated.ownership.rosters.home[1], frozen_keeper)

        local malformed = input_tape.copy_identity(fixture.identity)
        malformed.snapshot_version = match_snapshot.VERSION - 1
        rawset(malformed, "unexpected", true)
        t.is_true(not pcall(determinism_evidence.migration_identity, malformed))
        rawset(malformed, "unexpected", nil)
        rawset(malformed.ownership.rosters.home, 1, false)
        t.is_true(not pcall(determinism_evidence.migration_identity, malformed))
    end)

    t.it("explicitly migrates only the frozen v1 fixture identity to input v2", function()
        local legacy = input_tape.copy_identity(fixture.identity)
        legacy.fixture = "omp1-nebula-orion-eight-streams-v1"
        legacy.input_version = 1
        legacy.ownership.version = 1

        local migrated = determinism_evidence.migration_identity(legacy)
        t.eq(migrated.fixture, "omp1-nebula-orion-eight-streams-v2")
        t.eq(migrated.input_version, 2)
        t.eq(migrated.ownership.version, 2)
        t.eq(migrated.build, legacy.build)
        t.eq(migrated.source, legacy.source)
        t.eq(migrated.content, legacy.content)
        t.eq(migrated.config, legacy.config)
        t.eq(migrated.tuning, legacy.tuning)
        t.eq(migrated.seed, legacy.seed)
        t.eq(migrated.tick_rate, legacy.tick_rate)
        for _, team in ipairs({ "home", "away" }) do
            for index, player_id in ipairs(legacy.ownership.rosters[team]) do
                t.eq(migrated.ownership.rosters[team][index], player_id)
            end
        end
        for index, assignment in ipairs(legacy.ownership.slots) do
            t.eq(migrated.ownership.slots[index].slot, assignment.slot)
            t.eq(migrated.ownership.slots[index].team, assignment.team)
            t.eq(migrated.ownership.slots[index].player_id, assignment.player_id)
        end
    end)

    t.it("migrates only wires that were canonical within the v1 bounds", function()
        local current = assert(input_frame.encode(assert(input_frame.neutral(0))))
        local legacy = "1" .. current:sub(2)
        t.eq(determinism_evidence.migrate_legacy_fixture_wire(legacy), current)

        local v2_only_held = legacy:gsub("0,0,0,0", "0,0,128,0", 1)
        t.is_true(not pcall(determinism_evidence.migrate_legacy_fixture_wire, v2_only_held))

        local v2_only_edge = legacy:gsub("0,0,0,0", "0,0,0,64", 1)
        t.is_true(not pcall(determinism_evidence.migrate_legacy_fixture_wire, v2_only_edge))

        local oversized = legacy .. string.rep("0", 149)
        t.is_true(not pcall(determinism_evidence.migrate_legacy_fixture_wire, oversized))
    end)

    t.it("pins the full fixed-input match on the explicit evidence command", function()
        local result = determinism_evidence.verify()
        t.eq(result.ticks, 7201)
        t.eq(result.boundaries, 7202)
        t.eq(result.score_home, 0)
        t.eq(result.score_away, 0)
        t.eq(result.outcome, "draw")
        t.is_true(not result.coverage.goal_kickoff)
        t.is_true(result.coverage.tackle)
        t.is_true(result.coverage.aerial)
        t.is_true(result.coverage.keeper)
        t.is_true(result.coverage.full_time)
        local report = determinism_evidence.report(result)
        t.is_true(report:match("coverage=tackle,aerial,keeper,full_time") ~= nil)
        t.is_true(report:match("coverage=goal_kickoff") == nil)
    end)

    t.it("publishes the frozen fixture as a validated materialized tape", function()
        local tape = determinism_evidence.fixture_tape()
        t.is_true(input_tape.validate(tape))
        t.eq(tape.identity.fixture, fixture.fixture_id)
        t.eq(#tape.frames, fixture.frame_count)
        t.eq(#tape.boundary_hashes, fixture.boundary_count)
        t.eq(tape.boundary_hashes[#tape.boundary_hashes], fixture.expected_final_hash)
    end)
end)
