local example_package = require("spec.fixtures.research.example_package")
local research_schema = require("sim.research_schema")
local research_session = require("sim.research_session")
local t = require("spec.support.runner")

local TRACE_ID = "0123456789abcdef"

---@return table
local function envelope()
    return example_package.completed_session(TRACE_ID)
end

t.describe("research session envelope", function()
    t.it("accepts the completed example and round-trips byte-for-byte", function()
        local value = envelope()
        t.is_true(research_session.validate(value))
        local bytes = assert(research_session.encode(value))
        local decoded = assert(research_session.decode(bytes))
        t.eq(assert(research_session.encode(decoded)), bytes)
        t.eq(decoded.participant_id, value.participant_id)
        t.eq(decoded.experience.derived_label.label, "intermediate")
        t.eq(decoded.environment.readability_settings.cue_scale, "1.25")
        t.eq(decoded.agreement.agreement_version, "playtest-agreement-v3")
    end)

    t.it("requires bounded, distinct pseudonymous ids (length and charset, not entropy)", function()
        local short_participant = envelope()
        short_participant.participant_id = "p-1"
        t.is_true(not research_session.validate(short_participant))

        local shared = envelope()
        shared.session_id = shared.participant_id
        local ok, err = research_session.validate(shared)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("must differ", 1, true) ~= nil)
    end)

    t.it("rejects direct identifiers anywhere in a join key", function()
        local email = envelope()
        email.recruitment_channel = "friend@example.com"
        t.is_true(not research_session.validate(email))

        local url = envelope()
        url.cohort_id = "https://example.com/cohort"
        t.is_true(not research_session.validate(url))
    end)

    t.it("rejects unknown fields and unsupported schema versions", function()
        local unknown = envelope()
        unknown.participant_email = "nope"
        local ok, err = research_session.validate(unknown)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("unknown field", 1, true) ~= nil)

        local future = envelope()
        future.schema_version = research_session.VERSION + 1
        local version_ok, version_err = research_session.validate(future)
        t.is_true(version_ok == nil)
        t.is_true(
            type(version_err) == "string" and version_err:find("no migration", 1, true) ~= nil
        )
    end)
end)

t.describe("research session agreement", function()
    t.it("requires an accepted agreement recorded before the session started", function()
        local declined = envelope()
        declined.agreement.accepted = false
        local ok, err = research_session.validate(declined)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("must be accepted", 1, true) ~= nil)

        local late = envelope()
        late.agreement.accepted_wall_clock_ms = late.lifecycle.started_wall_clock_ms + 1
        local late_ok, late_err = research_session.validate(late)
        t.is_true(late_ok == nil)
        t.is_true(
            type(late_err) == "string"
                and late_err:find("after the session started", 1, true) ~= nil
        )

        local missing = envelope()
        missing.agreement = nil
        t.is_true(not research_session.validate(missing))
    end)

    t.it("answers the model-use question from the accepted version only", function()
        local value = envelope()
        t.is_true(research_session.allows_model_use(value))

        local uncovered = envelope()
        uncovered.agreement.model_use_covered = false
        local ok, raw_err = research_session.allows_model_use(uncovered)
        local err = assert(raw_err)
        t.is_true(ok == nil)
        t.is_true(err:find("did not cover", 1, true) ~= nil)
        t.is_true(err:find("playtest-agreement-v3", 1, true) ~= nil)
    end)

    t.it("keeps readability settings optional and free of a permission matrix", function()
        local without = envelope()
        without.environment.readability_settings = nil
        t.is_true(research_session.validate(without))

        local unknown_key = envelope()
        unknown_key.environment.readability_settings = { ["High Contrast"] = "on" }
        t.is_true(not research_session.validate(unknown_key))
    end)
end)

t.describe("research session lifecycle", function()
    t.it("requires an end timestamp for every terminal status", function()
        local unfinished = envelope()
        unfinished.lifecycle.ended_wall_clock_ms = nil
        t.is_true(not research_session.validate(unfinished))

        local in_progress = envelope()
        in_progress.lifecycle.status = "in_progress"
        in_progress.lifecycle.ended_wall_clock_ms = nil
        t.is_true(research_session.validate(in_progress))

        local reversed = envelope()
        reversed.lifecycle.ended_wall_clock_ms = reversed.lifecycle.started_wall_clock_ms - 1
        t.is_true(not research_session.validate(reversed))
    end)

    t.it("refuses a completed session that also reports a stop", function()
        local contradictory = envelope()
        contradictory.lifecycle.interruptions = {
            { kind = "process_exit", wall_clock_ms = 5000, duration_ms = 0 },
        }
        t.is_true(not research_session.validate(contradictory))

        local paused = envelope()
        paused.lifecycle.interruptions = {
            { kind = "pause", wall_clock_ms = 5000, duration_ms = 12000 },
        }
        t.is_true(research_session.validate(paused))
    end)

    t.it("requires a recorded reason for interrupted and excluded sessions", function()
        local interrupted = envelope()
        interrupted.lifecycle.status = "interrupted"
        t.is_true(not research_session.validate(interrupted))

        local excluded = envelope()
        excluded.lifecycle.status = "excluded"
        t.is_true(not research_session.validate(excluded))
        excluded.lifecycle.exclusions = {
            {
                exclusion_id = "ex-1",
                scope = "session",
                reason_code = "protocol_violation",
                preregistered = true,
            },
        }
        t.is_true(research_session.validate(excluded))
    end)

    t.it("rejects duplicate missingness rows and duplicate trace links", function()
        local duplicated_missing = envelope()
        duplicated_missing.lifecycle.missingness = {
            {
                target_id = "pxi-enjoyment-addon",
                target_kind = "instrument",
                reason_code = "participant_skipped",
            },
            {
                target_id = "pxi-enjoyment-addon",
                target_kind = "instrument",
                reason_code = "operator_error",
            },
        }
        t.is_true(not research_session.validate(duplicated_missing))

        local duplicated_link = envelope()
        duplicated_link.trace_links[2] = research_schema.copy(duplicated_link.trace_links[1])
        t.is_true(not research_session.validate(duplicated_link))
    end)

    t.it("keeps practice traces inside practice blocks", function()
        local misdeclared = envelope()
        misdeclared.trace_links[1].role = "practice"
        t.is_true(not research_session.validate(misdeclared))
        misdeclared.assignment.practice_block = true
        t.is_true(research_session.validate(misdeclared))
    end)
end)

t.describe("research session experience measures", function()
    t.it("stores continuous and ordinal sources, never a bare label", function()
        local value = envelope()
        t.eq(value.experience.football_games_ordinal, 4)

        local unbounded = envelope()
        unbounded.experience.football_games_ordinal = 6
        t.is_true(not research_session.validate(unbounded))

        local uncalibrated = envelope()
        uncalibrated.experience.derived_label = { label = "experienced" }
        local ok, err = research_session.validate(uncalibrated)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("calibration_id", 1, true) ~= nil)

        local unlabelled = envelope()
        unlabelled.experience.derived_label = nil
        t.is_true(research_session.validate(unlabelled))
    end)

    t.it("rejects non-finite numbers in this family", function()
        local nan = envelope()
        nan.experience.play_hours_per_week = 0 / 0
        t.is_true(not research_session.validate(nan))

        local infinite = envelope()
        infinite.lifecycle.started_wall_clock_ms = math.huge
        t.is_true(not research_session.validate(infinite))
    end)
end)

t.describe("research withdrawal tombstone", function()
    t.it("requires a tombstone and an emptied envelope for a withdrawal", function()
        local withdrawn = example_package.withdrawn_session()
        t.is_true(research_session.validate(withdrawn))

        local without_tombstone = example_package.withdrawn_session()
        without_tombstone.tombstone_id = nil
        t.is_true(not research_session.validate(without_tombstone))

        local without_missingness = example_package.withdrawn_session()
        without_missingness.lifecycle.missingness = {}
        t.is_true(not research_session.validate(without_missingness))

        local stray_tombstone = envelope()
        stray_tombstone.tombstone_id = "tombstone-4d7c1a9e"
        t.is_true(not research_session.validate(stray_tombstone))
    end)

    t.it("requires the tombstone to name payloads and force a rebuild", function()
        local tombstone = example_package.withdrawal_tombstone()
        t.is_true(research_session.validate_tombstone(tombstone))

        local empty = example_package.withdrawal_tombstone({})
        t.is_true(not research_session.validate_tombstone(empty))

        local hand_patched = example_package.withdrawal_tombstone()
        hand_patched.rebuild_required = false
        local ok, err = research_session.validate_tombstone(hand_patched)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("rebuild", 1, true) ~= nil)

        local duplicated = example_package.withdrawal_tombstone({
            "1111111111111111",
            "1111111111111111",
        })
        t.is_true(not research_session.validate_tombstone(duplicated))
    end)

    t.it("only pairs a tombstone with its own session and participant", function()
        local withdrawn = example_package.withdrawn_session()
        local tombstone = example_package.withdrawal_tombstone()
        t.is_true(research_session.validate_withdrawal(withdrawn, tombstone))

        local other_participant = example_package.withdrawal_tombstone()
        other_participant.participant_id = "p-000000000000000a"
        t.is_true(not research_session.validate_withdrawal(withdrawn, other_participant))

        local live_session = envelope()
        live_session.tombstone_id = nil
        t.is_true(not research_session.validate_withdrawal(live_session, tombstone))
    end)
end)
