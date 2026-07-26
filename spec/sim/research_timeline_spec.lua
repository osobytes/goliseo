local example_package = require("spec.fixtures.research.example_package")
local fixed_clock = require("sim.fixed_clock")
local research_schema = require("sim.research_schema")
local research_timeline = require("sim.research_timeline")
local research_trace = require("sim.research_trace")
local t = require("spec.support.runner")

local RUN_SCOPE = "0123456789abcdef"

---@return ResearchTimeline, ResearchExampleRollbackSequence
local function fresh_timeline()
    local sequence = example_package.rollback_sequence()
    local run_scope_id = assert(research_trace.tape_content_hash(sequence.tape))
    return assert(research_timeline.new(run_scope_id, "gi-0001")), sequence
end

---@return ResearchBoundaryClock
local function clock()
    return {
        origin_wall_clock_ms = 1000,
        tick_rate = fixed_clock.TICK_RATE,
        first_boundary_tick = 0,
        last_boundary_tick = 3,
        non_simulated_ms = 0,
    }
end

t.describe("research timeline construction", function()
    t.it("requires a hash-shaped run scope and a bounded instance id", function()
        t.is_true(research_timeline.new(RUN_SCOPE, "gi-0001") ~= nil)
        t.is_true(not research_timeline.new("nope", "gi-0001"))
        t.is_true(not research_timeline.new(RUN_SCOPE, "Game Instance"))
    end)

    t.it("exports an empty stream before anything is confirmed", function()
        local timeline = assert(research_timeline.new(RUN_SCOPE, "gi-0001"))
        local stream = assert(research_timeline.export(timeline))
        t.eq(#stream.rows, 0)
        t.eq(stream.confirmed_through_tick, -1)
        t.eq(stream.confirmed_boundary, 0)
        t.is_true(research_timeline.validate_stream(stream))
    end)
end)

t.describe("confirmed versus speculative rollback semantics", function()
    t.it("exports only confirmed events and drops the corrected-away one", function()
        local timeline, sequence = fresh_timeline()
        assert(research_timeline.observe_diff(timeline, sequence.speculative_diff))
        assert(research_timeline.observe_diff(timeline, sequence.correction_diff))
        local added = assert(research_timeline.confirm(timeline, sequence.confirmed))
        t.eq(added, 2)
        local stream = assert(research_timeline.export(timeline))
        t.eq(#stream.rows, 2)
        t.eq(stream.rows[1].event_kind, "touch")
        t.eq(stream.rows[1].canonical_tick, 0)
        t.eq(stream.rows[2].event_kind, "pass")
        t.eq(stream.rows[2].canonical_tick, 1)
        t.eq(stream.confirmed_through_tick, 1)
        t.eq(stream.confirmed_boundary, 2)
        for _, row in ipairs(stream.rows) do
            t.is_true(row.rollback_event_id ~= sequence.revoked_rollback_event_id)
        end
    end)

    t.it("refuses to confirm a step that still carries a revoked event", function()
        local timeline, sequence = fresh_timeline()
        assert(research_timeline.observe_diff(timeline, sequence.speculative_diff))
        assert(research_timeline.observe_diff(timeline, sequence.correction_diff))
        local stale = research_schema.copy(sequence.confirmed)
        stale[2].match_events[1] = research_schema.copy(sequence.correction_diff.revoked[1])
        local added, err = research_timeline.confirm(timeline, stale)
        t.is_true(added == nil)
        t.is_true(type(err) == "string" and err:find("revoked", 1, true) ~= nil)
    end)

    t.it("refuses to revoke or replace an already-confirmed event", function()
        local timeline, sequence = fresh_timeline()
        assert(research_timeline.confirm(timeline, sequence.confirmed))
        local confirmed_id = sequence.confirmed[1].match_events[1]
        local ok, err = research_timeline.observe_diff(timeline, {
            added = {},
            replaced = {},
            revoked = { research_schema.copy(confirmed_id) },
        })
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("already-confirmed", 1, true) ~= nil)

        local replace_ok = research_timeline.observe_diff(timeline, {
            added = {},
            replaced = {
                {
                    before = research_schema.copy(confirmed_id),
                    after = research_schema.copy(confirmed_id),
                },
            },
            revoked = {},
        })
        t.is_true(replace_ok == nil)
    end)

    t.it("requires contiguous confirmation from the research boundary", function()
        local timeline, sequence = fresh_timeline()
        local skipped = { research_schema.copy(sequence.confirmed[2]) }
        local added, err = research_timeline.confirm(timeline, skipped)
        t.is_true(added == nil)
        t.is_true(type(err) == "string" and err:find("noncontiguous", 1, true) ~= nil)
    end)

    t.it("refuses to confirm the same event twice", function()
        local timeline, sequence = fresh_timeline()
        assert(research_timeline.confirm(timeline, sequence.confirmed))
        local again, err = research_timeline.confirm(timeline, sequence.confirmed)
        t.is_true(again == nil)
        t.is_true(type(err) == "string")
    end)
end)

t.describe("confirmed event stream canonicalization", function()
    t.it("orders rows by tick then domain rank and hashes them", function()
        local timeline, sequence = fresh_timeline()
        assert(research_timeline.observe_diff(timeline, sequence.correction_diff))
        assert(research_timeline.confirm(timeline, sequence.confirmed))
        local stream = assert(research_timeline.export(timeline))
        local previous = nil
        for _, row in ipairs(stream.rows) do
            t.eq(row.domain, "soccer")
            t.eq(row.domain_rank, research_timeline.DOMAIN_RANKS.soccer)
            if previous then
                t.is_true(previous.canonical_tick <= row.canonical_tick)
            end
            previous = row
        end
        t.eq(stream.stream_hash, assert(research_timeline.stream_hash(stream)))
    end)

    t.it("fails closed when a row or hash is tampered with", function()
        local gameplay = example_package.gameplay()
        local tampered = research_schema.copy(gameplay.stream)
        tampered.rows[1].event_kind = "goal"
        t.is_true(not research_timeline.validate_stream(tampered))

        local rehashed = research_schema.copy(gameplay.stream)
        rehashed.stream_hash = "0000000000000000"
        t.is_true(not research_timeline.validate_stream(rehashed))

        local unconfirmed = research_schema.copy(gameplay.stream)
        unconfirmed.confirmed_through_tick = 0
        unconfirmed.confirmed_boundary = 1
        t.is_true(not research_timeline.validate_stream(unconfirmed))

        local reordered = research_schema.copy(gameplay.stream)
        reordered.rows[1], reordered.rows[2] = reordered.rows[2], reordered.rows[1]
        t.is_true(not research_timeline.validate_stream(reordered))
    end)

    t.it("detects a duplicate canonical key before it reaches an export", function()
        local timeline, sequence = fresh_timeline()
        assert(research_timeline.confirm(timeline, sequence.confirmed))
        timeline._rows[#timeline._rows + 1] = research_schema.copy(timeline._rows[1])
        local stream, err = research_timeline.export(timeline)
        t.is_true(stream == nil)
        t.is_true(type(err) == "string" and err:find("duplicate total key", 1, true) ~= nil)
    end)
end)

t.describe("wall-clock cue mapping", function()
    t.it("maps a cue to the nearest boundary and keeps the signed error", function()
        local exact = assert(research_timeline.map_to_boundary(clock(), 1000))
        t.eq(exact.canonical_tick, 0)
        t.near(exact.mapping_error_ms, 0)

        local mid = assert(research_timeline.map_to_boundary(clock(), 1000 + 1000 / 60 * 1.4))
        t.eq(mid.canonical_tick, 1)
        t.near(mid.mapping_error_ms, 0.4 * 1000 / 60, 1e-6)

        local behind = assert(research_timeline.map_to_boundary(clock(), 1000 + 1000 / 60 * 1.6))
        t.eq(behind.canonical_tick, 2)
        t.is_true(behind.mapping_error_ms < 0)
    end)

    t.it("subtracts pause and replay wall time before mapping", function()
        local paused = clock()
        paused.non_simulated_ms = 5000
        local mapping = assert(research_timeline.map_to_boundary(paused, 6000))
        t.eq(mapping.canonical_tick, 0)
        t.near(mapping.mapping_error_ms, 0)
    end)

    t.it("fails closed outside the trace window and on an unsupported tick rate", function()
        local outside, err = research_timeline.map_to_boundary(clock(), 5000)
        t.is_true(outside == nil)
        t.is_true(type(err) == "string" and err:find("outside", 1, true) ~= nil)
        t.is_true(not research_timeline.map_to_boundary(clock(), 0))

        local wrong_rate = clock()
        wrong_rate.tick_rate = fixed_clock.TICK_RATE + 1
        t.is_true(not research_timeline.map_to_boundary(wrong_rate, 1000))
    end)
end)

t.describe("research annotation sets", function()
    t.it("requires cue provenance and participant-authored free text", function()
        local gameplay = example_package.gameplay()
        local set = example_package.participant_annotations(gameplay.stream.run_scope_id)
        t.is_true(research_timeline.validate_annotation_set(set))

        local canonical_with_error =
            example_package.participant_annotations(gameplay.stream.run_scope_id)
        canonical_with_error.annotations[1].boundary_source = "canonical_tick"
        t.is_true(not research_timeline.validate_annotation_set(canonical_with_error))

        local mapped_without_clock =
            example_package.participant_annotations(gameplay.stream.run_scope_id)
        mapped_without_clock.annotations[1].wall_clock_ms = nil
        t.is_true(not research_timeline.validate_annotation_set(mapped_without_clock))

        local tool_authored = example_package.participant_annotations(gameplay.stream.run_scope_id)
        tool_authored.author_role = "tool"
        t.is_true(not research_timeline.validate_annotation_set(tool_authored))

        local duplicate = example_package.participant_annotations(gameplay.stream.run_scope_id)
        duplicate.annotations[2] = research_schema.copy(duplicate.annotations[1])
        t.is_true(not research_timeline.validate_annotation_set(duplicate))
    end)

    t.it("rejects non-finite mapping errors", function()
        local gameplay = example_package.gameplay()
        local nan = example_package.participant_annotations(gameplay.stream.run_scope_id)
        nan.annotations[1].mapping_error_ms = 0 / 0
        t.is_true(not research_timeline.validate_annotation_set(nan))
    end)

    t.it("gives free-text annotations the same withdrawal path as survey answers", function()
        local gameplay = example_package.gameplay()
        local set = example_package.participant_annotations(gameplay.stream.run_scope_id)
        local envelope = example_package.completed_session(gameplay.manifest.trace_id)
        t.is_true(research_timeline.validate_against_session(set, envelope))
        t.is_true(set.annotations[1].free_text ~= nil, "this fixture must carry free text")

        local withdrawn = example_package.withdrawn_session()
        withdrawn.session_id = set.session_id
        local ok, err = research_timeline.validate_against_session(set, withdrawn)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("withdrawn session", 1, true) ~= nil)

        local other_session = example_package.completed_session(gameplay.manifest.trace_id)
        other_session.session_id = "s-000000000000000a"
        local orphan_ok, orphan_err = research_timeline.validate_against_session(set, other_session)
        t.is_true(orphan_ok == nil)
        t.is_true(type(orphan_err) == "string" and orphan_err:find("orphan join", 1, true) ~= nil)

        local drifted = example_package.completed_session(gameplay.manifest.trace_id)
        drifted.agreement.agreement_version = "playtest-agreement-v2"
        t.is_true(not research_timeline.validate_against_session(set, drifted))
    end)

    t.it("rejects sets from another run, duplicates, and unconfirmed cues", function()
        local gameplay = example_package.gameplay()
        local set = example_package.participant_annotations(gameplay.stream.run_scope_id)
        local other = example_package.researcher_annotations("abcdefabcdefabcd")
        t.is_true(not research_timeline.join_annotations(gameplay.stream, { set, other }))

        t.is_true(not research_timeline.join_annotations(gameplay.stream, { set, set }))

        local late = example_package.participant_annotations(gameplay.stream.run_scope_id)
        late.annotations[1].canonical_tick = gameplay.stream.confirmed_through_tick + 5
        local ok, err = research_timeline.join_annotations(gameplay.stream, { late })
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("confirmed boundary", 1, true) ~= nil)
    end)
end)
