local example_package = require("spec.fixtures.research.example_package")
local research_dataset = require("sim.research_dataset")
local research_response = require("sim.research_response")
local research_session = require("sim.research_session")
local research_timeline = require("sim.research_timeline")
local research_trace = require("sim.research_trace")
local t = require("spec.support.runner")

t.describe("research example package", function()
    t.it("assembles a completed session across every contract", function()
        local gameplay = example_package.gameplay()
        t.is_true(research_trace.validate(gameplay.manifest))
        t.is_true(research_timeline.validate_stream(gameplay.stream))
        t.is_true(research_trace.validate_against_stream(gameplay.manifest, gameplay.stream))

        local envelope = example_package.completed_session(gameplay.manifest.trace_id)
        t.is_true(research_session.validate(envelope))

        local responses = example_package.enjoyment_responses({
            trace_id = gameplay.manifest.trace_id,
        })
        t.is_true(research_response.validate(responses))
        t.eq(#responses.scores, 1)
        t.near(responses.scores[1].score, 2, 1e-9)

        local dataset = assert(example_package.dataset(gameplay.manifest))
        t.is_true(research_dataset.validate(dataset))
    end)

    t.it("keeps corrected-away rollback events out of the confirmed export", function()
        local gameplay = example_package.gameplay()
        for _, row in ipairs(gameplay.stream.rows) do
            t.is_true(
                row.rollback_event_id ~= gameplay.revoked_rollback_event_id,
                "revoked event leaked into the export"
            )
            t.is_true(
                row.event_kind ~= "tackle",
                "the corrected-away tackle leaked into the export"
            )
        end
        local kinds = {}
        for _, row in ipairs(gameplay.stream.rows) do
            kinds[row.event_kind] = true
        end
        t.is_true(kinds.touch, "the confirmed touch is missing")
        t.is_true(kinds.pass, "the corrected pass is missing")
    end)

    t.it("rejects a session or response set that names a trace nobody holds", function()
        local gameplay = example_package.gameplay()
        local envelope = example_package.completed_session(gameplay.manifest.trace_id)
        t.is_true(research_session.validate_against_traces(envelope, { gameplay.manifest }))

        local orphan_session = example_package.completed_session("1111111111111111")
        local ok, err = research_session.validate_against_traces(orphan_session, {
            gameplay.manifest,
        })
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("orphan join", 1, true) ~= nil)
        t.is_true(not research_session.validate_against_traces(envelope, {}))

        -- The seed cross-check compares the envelope's assignment against the
        -- resolved manifest, so prove the mismatch branch actually fires.
        local wrong_seed = example_package.completed_session(gameplay.manifest.trace_id)
        wrong_seed.assignment.seed = gameplay.manifest.simulation.seed + 1
        local seed_ok, seed_err = research_session.validate_against_traces(wrong_seed, {
            gameplay.manifest,
        })
        t.is_true(seed_ok == nil)
        t.is_true(type(seed_err) == "string" and seed_err:find("another seed", 1, true) ~= nil)

        local responses = example_package.enjoyment_responses({
            trace_id = gameplay.manifest.trace_id,
        })
        t.is_true(research_response.validate_against_trace(responses, gameplay.manifest))
        t.is_true(research_response.validate_against_session(responses, envelope))

        local orphan_responses = example_package.enjoyment_responses({
            trace_id = "1111111111111111",
        })
        local response_ok, response_err =
            research_response.validate_against_trace(orphan_responses, gameplay.manifest)
        t.is_true(response_ok == nil)
        t.is_true(
            type(response_err) == "string" and response_err:find("orphan join", 1, true) ~= nil
        )
    end)

    t.it("refuses a response set for an instrument the session says was skipped", function()
        local gameplay = example_package.gameplay()
        local interrupted = example_package.interrupted_session(gameplay.manifest.trace_id)
        local responses = example_package.enjoyment_responses({
            session_id = example_package.INTERRUPTED_SESSION_ID,
        })
        local ok, err = research_response.validate_against_session(responses, interrupted)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("not administered", 1, true) ~= nil)
    end)

    t.it("keeps model use tied to the accepted agreement version", function()
        local gameplay = example_package.gameplay()
        local envelope = example_package.completed_session(gameplay.manifest.trace_id)
        t.is_true(research_session.allows_model_use(envelope))

        local uncovered = example_package.completed_session(gameplay.manifest.trace_id)
        uncovered.agreement.model_use_covered = false
        local ok, err = research_session.allows_model_use(uncovered)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("did not cover", 1, true) ~= nil)
    end)

    t.it("joins two independent annotation sets to one gameplay run", function()
        local gameplay = example_package.gameplay()
        local event_id = gameplay.stream.rows[1].event_id
        local participant =
            example_package.participant_annotations(gameplay.stream.run_scope_id, event_id)
        local researcher = example_package.researcher_annotations(gameplay.stream.run_scope_id)
        t.is_true(research_timeline.join_annotations(gameplay.stream, { participant, researcher }))

        local orphan = example_package.participant_annotations(
            gameplay.stream.run_scope_id,
            "abcdefabcdefabcd"
        )
        orphan.annotation_set_id = "as-orphan"
        local ok, err = research_timeline.join_annotations(gameplay.stream, { orphan })
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("orphan", 1, true) ~= nil)
    end)

    t.it("records an interrupted session with structured missingness", function()
        local gameplay = example_package.gameplay("incomplete_interrupted")
        t.eq(gameplay.manifest.simulation.completion, "incomplete_interrupted")
        local envelope = example_package.interrupted_session(gameplay.manifest.trace_id)
        t.is_true(research_session.validate(envelope))
        t.eq(#envelope.lifecycle.interruptions, 1)
        t.eq(envelope.lifecycle.missingness[1].reason_code, "instrument_not_administered")

        local without_reason = example_package.interrupted_session(gameplay.manifest.trace_id)
        without_reason.lifecycle.interruptions = {}
        t.is_true(not research_session.validate(without_reason))
    end)

    t.it("scores nothing when a survey item is missing", function()
        local complete = example_package.enjoyment_responses()
        t.eq(#complete.scores, 1)

        local partial = example_package.enjoyment_responses({
            missing_item = "participant_skipped",
            response_set_id = "rs-pxi-enjoyment-b-partial",
        })
        t.is_true(research_response.validate(partial))
        t.eq(#partial.scores, 0)

        local scored_anyway = example_package.enjoyment_responses({
            missing_item = "participant_skipped",
            response_set_id = "rs-pxi-enjoyment-b-partial",
        })
        scored_anyway.scores = {
            { construct = "enjoyment", score = 1.5, rule = "mean_all_items", item_count = 2 },
        }
        local ok, err = research_response.validate(scored_anyway)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string")
    end)

    t.it("pairs a withdrawal tombstone with its emptied session envelope", function()
        local envelope = example_package.withdrawn_session()
        local tombstone = example_package.withdrawal_tombstone()
        t.is_true(research_session.validate(envelope))
        t.is_true(research_session.validate_tombstone(tombstone))
        t.is_true(research_session.validate_withdrawal(envelope, tombstone))

        local retained = example_package.withdrawn_session()
        retained.trace_links = {
            {
                trace_id = "1111111111111111",
                game_instance_id = "gi-0001",
                role = "condition_block",
            },
        }
        t.is_true(not research_session.validate(retained))

        local mismatched = example_package.withdrawal_tombstone()
        mismatched.tombstone_id = "tombstone-other"
        t.is_true(not research_session.validate_withdrawal(envelope, mismatched))
    end)

    t.it("refuses a dataset that retains a withdrawn participant", function()
        local gameplay = example_package.gameplay()
        local dataset = assert(example_package.dataset(gameplay.manifest))
        local tombstone = example_package.withdrawal_tombstone()
        local ok, err = research_dataset.validate_against_tombstones(dataset, { tombstone })
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("withdrawn", 1, true) ~= nil)

        local other = example_package.withdrawal_tombstone()
        other.participant_id = "p-000000000000000a"
        other.session_id = "s-000000000000000a"
        t.is_true(research_dataset.validate_against_tombstones(dataset, { other }))
    end)
end)
