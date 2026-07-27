local example_package = require("spec.fixtures.research.example_package")
local input_frame = require("sim.input_frame")
local input_tape = require("sim.input_tape")
local match_snapshot = require("sim.match_snapshot")
local research_schema = require("sim.research_schema")
local research_trace = require("sim.research_trace")
local short_match_tape = require("spec.fixtures.short_match_tape")
local t = require("spec.support.runner")

---@return ResearchTraceManifest, InputTape
local function manifest_and_tape()
    local gameplay = example_package.gameplay()
    return gameplay.manifest, gameplay.tape
end

---@param tape InputTape
---@return string
local function tape_signature(tape)
    local parts = {
        match_snapshot.hash(tape.initial),
        tostring(tape.version),
        tostring(#tape.frames),
    }
    for index = 1, #tape.frames do
        parts[#parts + 1] = assert(input_frame.encode(tape.frames[index]))
    end
    for index = 1, #tape.boundary_hashes do
        parts[#parts + 1] = tape.boundary_hashes[index]
    end
    return table.concat(parts, "|")
end

t.describe("gameplay trace manifest", function()
    t.it("derives its identity from the tape it references", function()
        local manifest, tape = manifest_and_tape()
        t.eq(manifest.simulation.initial_boundary_hash, tape.boundary_hashes[1])
        t.eq(manifest.simulation.final_boundary_hash, tape.boundary_hashes[#tape.boundary_hashes])
        t.eq(manifest.simulation.frame_count, #tape.frames)
        t.eq(manifest.simulation.last_boundary_tick, #tape.frames)
        t.eq(manifest.simulation.tape_content_hash, assert(research_trace.tape_content_hash(tape)))
        t.eq(manifest.trace_id, assert(research_trace.derive_trace_id(manifest)))
    end)

    t.it("keeps every manifest field in exactly one identity group", function()
        local groups = research_trace.FIELD_GROUPS
        t.is_true(research_schema.assert_disjoint("groups", groups))
        local covered = 0
        for _, members in pairs(groups) do
            covered = covered + #members
        end
        t.eq(covered, #research_trace.SHAPE.fields)
    end)
end)

t.describe("research metadata cannot move a simulation boundary hash", function()
    t.it("leaves the tape and its simulation identity untouched when links are added", function()
        local manifest, tape = manifest_and_tape()
        local before_tape = tape_signature(tape)
        local before_identity = assert(research_trace.simulation_identity_hash(manifest))
        local before_content = assert(research_trace.content_hash(manifest))

        local linked = assert(research_trace.with_research_link(manifest, {
            link_kind = "research_session",
            target_id = example_package.SESSION_ID,
            target_hash = "1234567890abcdef",
        }))
        local twice = assert(research_trace.with_research_link(linked, {
            link_kind = "annotation_set",
            target_id = "as-participant-debrief-1",
            target_hash = "fedcba0987654321",
        }))

        t.eq(tape_signature(tape), before_tape, "the tape changed")
        t.eq(#manifest.research_links, 0, "the source manifest was mutated")
        t.eq(#twice.research_links, 2)
        t.eq(assert(research_trace.simulation_identity_hash(twice)), before_identity)
        t.eq(twice.trace_id, manifest.trace_id)
        -- The whole-manifest hash *does* move: annotations are recorded, not hidden.
        t.is_true(assert(research_trace.content_hash(twice)) ~= before_content)
    end)

    t.it("rejects participant fields in the manifest and in the tape identity", function()
        local manifest = select(1, manifest_and_tape())
        local injected = research_schema.copy(manifest)
        injected.participant_id = "p-7f3a9c21b8e45d06"
        local ok, err = research_trace.validate(injected)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("unknown field", 1, true) ~= nil)

        local simulation_injected = research_schema.copy(manifest)
        simulation_injected.simulation.participant_id = "p-7f3a9c21b8e45d06"
        t.is_true(not research_trace.validate(simulation_injected))

        local _, identity = short_match_tape.make()
        local injected_identity = research_schema.copy(identity)
        injected_identity.participant_id = "p-7f3a9c21b8e45d06"
        t.is_true(
            not pcall(input_tape.copy_identity, injected_identity),
            "InputTapeIdentity accepted a participant field"
        )
    end)

    t.it("refuses duplicate research links", function()
        local manifest = select(1, manifest_and_tape())
        local link = {
            link_kind = "research_session",
            target_id = example_package.SESSION_ID,
            target_hash = "1234567890abcdef",
        }
        local linked = assert(research_trace.with_research_link(manifest, link))
        local again, err = research_trace.with_research_link(linked, link)
        t.is_true(again == nil)
        t.is_true(type(err) == "string" and err:find("duplicated", 1, true) ~= nil)
    end)
end)

t.describe("gameplay trace manifest serialization", function()
    t.it("round-trips byte-for-byte", function()
        local manifest = select(1, manifest_and_tape())
        local bytes = assert(research_trace.encode(manifest))
        local decoded = assert(research_trace.decode(bytes))
        t.eq(assert(research_trace.encode(decoded)), bytes)
        t.eq(decoded.trace_id, manifest.trace_id)
        t.eq(decoded.simulation.seed, manifest.simulation.seed)
        t.eq(#decoded.simulation.producers, input_frame.SLOT_COUNT)
    end)

    t.it("fails closed on malformed, unknown, and cross-version payloads", function()
        local manifest = select(1, manifest_and_tape())
        local bytes = assert(research_trace.encode(manifest))
        t.is_true(not research_trace.decode(bytes:sub(1, #bytes - 8)))
        t.is_true(not research_trace.decode(bytes .. "n;"))

        local malformed_hash = research_schema.copy(manifest)
        malformed_hash.simulation.tape_content_hash = "not-a-hash"
        t.is_true(not research_trace.validate(malformed_hash))

        local future = research_schema.copy(manifest)
        future.schema_version = research_trace.VERSION + 1
        local ok, err = research_trace.validate(future)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("no migration", 1, true) ~= nil)
    end)

    t.it("rejects non-finite numbers in this family", function()
        local manifest = select(1, manifest_and_tape())
        local nan = research_schema.copy(manifest)
        nan.runtime.mean_frame_ms = 0 / 0
        t.is_true(not research_trace.validate(nan))

        local infinite = research_schema.copy(manifest)
        infinite.simulation.seed = math.huge
        t.is_true(not research_trace.validate(infinite))
    end)

    t.it("detects a hand-edited trace id or simulation field", function()
        local manifest = select(1, manifest_and_tape())
        local retagged = research_schema.copy(manifest)
        retagged.simulation.seed = manifest.simulation.seed + 1
        local ok, err = research_trace.validate(retagged)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("trace_id", 1, true) ~= nil)

        local relabelled = research_schema.copy(manifest)
        relabelled.trace_id = "0000000000000000"
        t.is_true(not research_trace.validate(relabelled))
    end)
end)

t.describe("gameplay trace manifest invariants", function()
    t.it("requires canonical slot order and honest producer kinds", function()
        local manifest = select(1, manifest_and_tape())
        local shuffled = research_schema.copy(manifest)
        shuffled.simulation.producers[1].slot = shuffled.simulation.producers[2].slot
        t.is_true(not research_trace.validate(shuffled))

        local human_with_policy = research_schema.copy(manifest)
        human_with_policy.simulation.producers[1].producer_policy_id = "bot-baseline-v1"
        t.is_true(not research_trace.validate(human_with_policy))

        local bot_without_policy = research_schema.copy(manifest)
        bot_without_policy.simulation.producers[2].producer_policy_id = nil
        t.is_true(not research_trace.validate(bot_without_policy))
    end)

    t.it("rejects an inconsistent tick range, completion, or divergence", function()
        local manifest = select(1, manifest_and_tape())
        local bad_range = research_schema.copy(manifest)
        bad_range.simulation.last_boundary_tick = bad_range.simulation.last_boundary_tick + 1
        t.is_true(not research_trace.validate(bad_range))

        local completed_with_divergence = research_schema.copy(manifest)
        completed_with_divergence.simulation.divergence = {
            boundary_tick = 2,
            expected_hash = "1111111111111111",
            actual_hash = "2222222222222222",
            state_path = "state.ball.x",
        }
        t.is_true(not research_trace.validate(completed_with_divergence))
    end)

    t.it("keeps combat identity tied to the combat tape version", function()
        local manifest = select(1, manifest_and_tape())
        t.eq(manifest.simulation.tape_version, input_tape.VERSION)
        local soccer_with_combat = research_schema.copy(manifest)
        soccer_with_combat.simulation.combat_identity = "combat-identity"
        t.is_true(not research_trace.validate(soccer_with_combat))
    end)

    t.it("only accepts an event stream that describes the same run", function()
        local gameplay = example_package.gameplay()
        t.is_true(research_trace.validate_against_stream(gameplay.manifest, gameplay.stream))

        local other_scope = research_schema.copy(gameplay.stream)
        other_scope.run_scope_id = "abcdefabcdefabcd"
        t.is_true(not research_trace.validate_against_stream(gameplay.manifest, other_scope))

        local other_instance = research_schema.copy(gameplay.stream)
        other_instance.game_instance_id = "gi-0002"
        t.is_true(not research_trace.validate_against_stream(gameplay.manifest, other_instance))
    end)

    t.it("refuses a producer set that is not one row per canonical slot", function()
        local tape = short_match_tape.make()
        local short_producers = {}
        for index = 1, input_frame.SLOT_COUNT - 1 do
            short_producers[index] = { producer_kind = "bot", producer_policy_id = "bot-1" }
        end
        local manifest, err = research_trace.from_tape(tape, {
            game_instance_id = "gi-0001",
            ruleset_version = 1,
            event_schema_version = 1,
            confirmed_event_stream_hash = "1111111111111111",
            completion = "completed",
            producers = short_producers,
            runtime = {
                platform = "linux-native",
                renderer = "opengl",
                render_hz = 60,
                render_hz_mode = "fixed",
                input_device = "keyboard",
                mean_frame_ms = 8,
                p99_frame_ms = 14,
                dropped_frame_count = 0,
                pause_count = 0,
                goal_replay_count = 0,
                rollback_count = 0,
                max_rollback_ticks = 0,
                raw_device_event_policy = "not_collected",
                raw_device_event_clock = "none",
            },
        })
        t.is_true(manifest == nil)
        t.is_true(type(err) == "string")
    end)
end)
