-- Representative research example package.
--
-- Covers the four cases issue #132 asks for: a completed session, an
-- interrupted session, a session with missing survey items, and a withdrawal
-- tombstone. Everything is built from the real modules — the gameplay side uses
-- the checked-in short match tape and the real rollback event timeline, so the
-- package cannot drift away from the simulation spine it claims to describe.

local input_frame = require("sim.input_frame")
local match_snapshot = require("sim.match_snapshot")
local research_dataset = require("sim.research_dataset")
local research_features = require("sim.research_features")
local research_schema = require("sim.research_schema")
local research_timeline = require("sim.research_timeline")
local research_trace = require("sim.research_trace")
local rollback_events = require("sim.rollback_events")
local short_match_tape = require("spec.fixtures.short_match_tape")

---@class ResearchExamplePackage
local example_package = {}

example_package.PARTICIPANT_ID = "p-7f3a9c21b8e45d06"
example_package.SECOND_PARTICIPANT_ID = "p-2b8e45d067f3a9c1"
example_package.THIRD_PARTICIPANT_ID = "p-45d067f3a9c12b8e"
example_package.SESSION_ID = "s-1c9d4f7a3b6e2058"
example_package.SECOND_SESSION_ID = "s-6e2058f7a3b91c9d"
example_package.THIRD_SESSION_ID = "s-a3b91c9d6e2058f7"
example_package.WITHDRAWN_SESSION_ID = "s-58f7a3b6e20591c9"
example_package.INTERRUPTED_SESSION_ID = "s-91c9d4f7a3b6e205"
example_package.GAME_INSTANCE_ID = "gi-0001"

---@param source MatchEvent
---@return MatchEvent
local function copy_event(source)
    local event = {}
    for key, value in pairs(source) do
        event[key] = value
    end
    ---@cast event MatchEvent
    return event
end

---@param before MatchSnapshot
---@param events MatchEvent[]
---@return MatchSnapshot
local function next_snapshot(before, events)
    local state = match_snapshot.restore(before)
    state.input_tick = state.input_tick + 1
    state.events = {}
    for index, event in ipairs(events) do
        state.events[index] = copy_event(event)
    end
    state.time_left = math.max(0, state.time_left - 1 / 60)
    return match_snapshot.capture(state)
end

---@param tick integer
---@return RollbackInputTickRecord
local function input_record(tick)
    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        slots[index] = {
            source = "local",
            status = "authoritative",
            sample = input_frame.neutral_sample(),
        }
    end
    return { tick = tick, slots = slots }
end

---@param snapshot MatchSnapshot
---@return RollbackEventStepInput
local function step_input(snapshot)
    local tick = snapshot.state.input_tick - 1
    local events = {}
    for index, event in ipairs(snapshot.state.events) do
        events[index] = copy_event(event)
    end
    return {
        output = {
            tick = tick,
            start_boundary = tick,
            end_boundary = tick + 1,
            input = input_record(tick),
            events = events,
            state = {
                score = {
                    home = snapshot.state.score.home,
                    away = snapshot.state.score.away,
                },
                time_left = snapshot.state.time_left,
                finished = snapshot.state.finished,
            },
            finished = snapshot.state.finished,
        },
        snapshot = snapshot,
    }
end

---@class ResearchExampleGameplay
---@field tape InputTape
---@field stream ResearchEventStream
---@field manifest ResearchTraceManifest
---@field revoked_rollback_event_id string

---@class ResearchExampleRollbackSequence
---@field tape InputTape
---@field first_diff RollbackEventDiff
---@field speculative_diff RollbackEventDiff
---@field correction_diff RollbackEventDiff
---@field confirmed RollbackEventStep[]
---@field revoked_rollback_event_id string

-- A real rollback sequence: tick 0 confirmed as a touch, tick 1 first predicted
-- as a tackle and then corrected to a pass. The tackle is therefore an event
-- that presentation saw and research must never see.
---@return ResearchExampleRollbackSequence
function example_package.rollback_sequence()
    local tape = short_match_tape.make()
    local initial = tape.initial
    local rollback_timeline = rollback_events.new(initial)
    local first = next_snapshot(initial, { { kind = "touch", x = 100, y = 100 } })
    local first_diff = assert(rollback_events.apply(rollback_timeline, 0, 0, { step_input(first) }))
    local speculative = next_snapshot(first, { { kind = "tackle", x = 120, y = 110 } })
    local speculative_diff =
        assert(rollback_events.apply(rollback_timeline, 1, 1, { step_input(speculative) }))
    local corrected = next_snapshot(first, { { kind = "pass", x = 130, y = 115 } })
    local correction_diff =
        assert(rollback_events.apply(rollback_timeline, 1, 1, { step_input(corrected) }))
    assert(#correction_diff.revoked == 1, "example package expects one revoked event")
    return {
        tape = tape,
        first_diff = first_diff,
        speculative_diff = speculative_diff,
        correction_diff = correction_diff,
        confirmed = rollback_events.confirm(rollback_timeline, 1),
        revoked_rollback_event_id = correction_diff.revoked[1].id,
    }
end

-- Build the gameplay half of the package: a real tape, a real confirmed event
-- stream whose second tick was corrected by rollback, and a trace manifest that
-- references both.
---@param completion ResearchTraceCompletion?
---@return ResearchExampleGameplay
function example_package.gameplay(completion)
    local sequence = example_package.rollback_sequence()
    local tape = sequence.tape
    local speculative_diff = sequence.speculative_diff
    local correction_diff = sequence.correction_diff
    local confirmed = sequence.confirmed
    local revoked_id = sequence.revoked_rollback_event_id

    -- The event stream is scoped by tape content, which is knowable before any
    -- recorder diagnostics exist. That ordering is what keeps the trace manifest
    -- (which references the stream hash) free of a circular dependency.
    local run_scope_id = assert(research_trace.tape_content_hash(tape))
    local research_stream_builder =
        assert(research_timeline.new(run_scope_id, example_package.GAME_INSTANCE_ID))
    assert(research_timeline.observe_diff(research_stream_builder, speculative_diff))
    assert(research_timeline.observe_diff(research_stream_builder, correction_diff))
    assert(research_timeline.confirm(research_stream_builder, confirmed))
    local stream = assert(research_timeline.export(research_stream_builder))

    local manifest = assert(example_package.trace_manifest(tape, stream.stream_hash, completion))
    assert(research_trace.validate_against_stream(manifest, stream))

    return {
        tape = tape,
        stream = stream,
        manifest = manifest,
        revoked_rollback_event_id = revoked_id,
    }
end

---@param tape InputTape
---@param confirmed_event_stream_hash string
---@param completion ResearchTraceCompletion?
---@return ResearchTraceManifest?, string?
function example_package.trace_manifest(tape, confirmed_event_stream_hash, completion)
    local producers = {}
    for index = 1, input_frame.SLOT_COUNT do
        producers[index] = index == 1 and { producer_kind = "human" }
            or { producer_kind = "bot", producer_policy_id = "bot-baseline-v1" }
    end
    return research_trace.from_tape(tape, {
        game_instance_id = example_package.GAME_INSTANCE_ID,
        ruleset_version = 1,
        event_schema_version = 1,
        confirmed_event_stream_hash = confirmed_event_stream_hash,
        completion = completion or "completed",
        producers = producers,
        runtime = {
            platform = "linux-native",
            renderer = "opengl",
            render_hz = 60,
            render_hz_mode = "fixed",
            input_device = "keyboard",
            mean_frame_ms = 8.2,
            p99_frame_ms = 14.5,
            dropped_frame_count = 3,
            pause_count = 1,
            goal_replay_count = 0,
            rollback_count = 1,
            max_rollback_ticks = 2,
            raw_device_event_policy = "minimized_diagnostic",
            raw_device_event_clock = "wall_clock_monotonic",
        },
    })
end

---@param trace_id string
---@param overrides table?
---@return table
local function base_envelope(trace_id, overrides)
    overrides = overrides or {}
    local envelope = {
        schema_version = 1,
        manifest_kind = "research_session_envelope",
        digest = research_schema.DIGEST,
        session_id = example_package.SESSION_ID,
        participant_id = example_package.PARTICIPANT_ID,
        study_id = "m11-combat-fun",
        protocol_version = "m11-protocol-v3",
        cohort_id = "confirmatory-wave-1",
        recruitment_channel = "local-football-club",
        block_id = "block-b",
        trial_id = "trial-2",
        condition_id = "condition-b-combat-on",
        condition_order = 2,
        sequence_label = "ab",
        counterbalancing_cell = "cell-ab-home-seed38",
        assignment = {
            build_id = "spec-build",
            tuning_config_id = "tuning-default-v1",
            seed = 38,
            side = "home",
            role_id = "outfield-slot-1",
            loadout_id = "loadout-spring-gloves",
            opponent_population = "bot",
            opponent_policy_id = "bot-baseline-v1",
            practice_block = false,
        },
        environment = {
            device_class = "desktop",
            control_device = "keyboard",
            control_mapping_id = "kb-default-v2",
            viewport_w = 1280,
            viewport_h = 720,
            render_hz = 60,
            input_latency_ms = 24.5,
            language_tag = "en-gb",
            readability_settings = { high_contrast_cues = "on", cue_scale = "1.25" },
        },
        lifecycle = {
            status = "completed",
            started_wall_clock_ms = 1000,
            ended_wall_clock_ms = 900000,
            observer_mode = "same_room",
            assistance = "none",
            interruptions = {},
            exclusions = {},
            missingness = {},
        },
        experience = {
            play_hours_per_week = 6.5,
            years_playing = 12,
            football_games_ordinal = 4,
            action_games_ordinal = 2,
            self_reported_familiarity_ordinal = 3,
            derived_label = {
                label = "intermediate",
                calibration_id = "experience-calibration-v1",
                calibration_version = 1,
            },
        },
        agreement = {
            agreement_version = "playtest-agreement-v3",
            accepted = true,
            accepted_wall_clock_ms = 500,
            model_use_covered = true,
        },
        trace_links = {
            {
                trace_id = trace_id,
                game_instance_id = example_package.GAME_INSTANCE_ID,
                role = "condition_block",
            },
        },
    }
    envelope.session_id = overrides.session_id or envelope.session_id
    envelope.participant_id = overrides.participant_id or envelope.participant_id
    envelope.condition_id = overrides.condition_id or envelope.condition_id
    envelope.assignment.build_id = overrides.build_id or envelope.assignment.build_id
    envelope.assignment.tuning_config_id = overrides.tuning_config_id
        or envelope.assignment.tuning_config_id
    if overrides.model_use_covered ~= nil then
        envelope.agreement.model_use_covered = overrides.model_use_covered
    end
    return envelope
end

-- The session envelopes that back `example_package.dataset`. Source rows are
-- derived from these, so a dataset can never claim an agreement or a model-use
-- permission that no envelope actually recorded.
---@param manifest ResearchTraceManifest
---@param overrides table?
---@return table[] envelopes
function example_package.dataset_sessions(manifest, overrides)
    overrides = overrides or {}
    return {
        base_envelope(manifest.trace_id),
        base_envelope(research_schema.tuple_hash("example-trace/v1", { "second" }), {
            session_id = example_package.SECOND_SESSION_ID,
            participant_id = example_package.SECOND_PARTICIPANT_ID,
            condition_id = "condition-a-combat-off",
        }),
        base_envelope(research_schema.tuple_hash("example-trace/v1", { "third" }), {
            session_id = example_package.THIRD_SESSION_ID,
            participant_id = example_package.THIRD_PARTICIPANT_ID,
            build_id = "spec-build-next",
            tuning_config_id = "tuning-candidate-v2",
            model_use_covered = overrides.third_model_use_covered,
        }),
    }
end

-- One dataset source row derived from the session envelope it came from.
---@param envelope table
---@param trace_manifest_hash string
---@return table
local function source_row(envelope, trace_manifest_hash)
    return {
        trace_id = envelope.trace_links[1].trace_id,
        trace_manifest_hash = trace_manifest_hash,
        session_id = envelope.session_id,
        participant_id = envelope.participant_id,
        condition_id = envelope.condition_id,
        build_id = envelope.assignment.build_id,
        tuning_config_id = envelope.assignment.tuning_config_id,
        excluded = false,
        agreement_version = envelope.agreement.agreement_version,
        model_use_covered = envelope.agreement.model_use_covered,
    }
end

---@param trace_id string
---@return table
function example_package.completed_session(trace_id)
    return base_envelope(trace_id)
end

---@param trace_id string
---@return table
function example_package.interrupted_session(trace_id)
    local envelope = base_envelope(trace_id)
    envelope.session_id = example_package.INTERRUPTED_SESSION_ID
    envelope.lifecycle.status = "interrupted"
    envelope.lifecycle.ended_wall_clock_ms = 420000
    envelope.lifecycle.interruptions = {
        {
            kind = "technical_failure",
            wall_clock_ms = 410000,
            duration_ms = 9000,
            canonical_tick = 2,
            reason_code = "technical_failure",
        },
    }
    envelope.lifecycle.missingness = {
        {
            target_id = "pxi_enjoyment_addon",
            target_kind = "instrument",
            reason_code = "instrument_not_administered",
        },
    }
    return envelope
end

---@return table
function example_package.withdrawn_session()
    local envelope = base_envelope("0000000000000000")
    envelope.session_id = example_package.WITHDRAWN_SESSION_ID
    envelope.lifecycle.status = "withdrawn"
    envelope.lifecycle.ended_wall_clock_ms = 500000
    envelope.lifecycle.missingness = {
        {
            target_id = "session-payload",
            target_kind = "trace",
            reason_code = "participant_withdrew",
        },
    }
    envelope.trace_links = {}
    envelope.tombstone_id = "tombstone-4d7c1a9e"
    return envelope
end

---@param revoked_payload_hashes string[]?
---@return table
function example_package.withdrawal_tombstone(revoked_payload_hashes)
    return {
        schema_version = 1,
        manifest_kind = "research_withdrawal_tombstone",
        digest = research_schema.DIGEST,
        tombstone_id = "tombstone-4d7c1a9e",
        session_id = example_package.WITHDRAWN_SESSION_ID,
        participant_id = example_package.PARTICIPANT_ID,
        request_wall_clock_ms = 1200000,
        revoked_payload_hashes = revoked_payload_hashes or { "1111111111111111" },
        rebuild_required = true,
    }
end

---@param overrides table?
---@return table
function example_package.enjoyment_responses(overrides)
    overrides = overrides or {}
    local responses = {
        { item_id = "pxi_enjoy_1", raw_response = 2, response_ms = 3400 },
        { item_id = "pxi_enjoy_2", raw_response = 1, response_ms = 2900 },
        { item_id = "pxi_enjoy_3", raw_response = 3, response_ms = 4100 },
    }
    local scores = {
        { construct = "enjoyment", score = 2, rule = "mean_all_items", item_count = 3 },
    }
    if overrides.missing_item then
        responses[3] = {
            item_id = "pxi_enjoy_3",
            missing_reason = overrides.missing_item,
            response_ms = 8000,
        }
        scores = {}
    end
    return {
        schema_version = 1,
        manifest_kind = "research_response_set",
        digest = research_schema.DIGEST,
        response_set_id = overrides.response_set_id or "rs-pxi-enjoyment-b-1",
        session_id = overrides.session_id or example_package.SESSION_ID,
        participant_id = example_package.PARTICIPANT_ID,
        condition_id = "condition-b-combat-on",
        instrument_id = "pxi_enjoyment_addon",
        instrument_version = "pxi-2021.1-enjoyment-addon",
        scoring_key_version = "pxi-enjoyment-mean-1",
        analysis_role = "primary",
        validated_instrument = true,
        partial_administration = false,
        locale = { language_tag = "en-gb", translation_provenance = "original" },
        timing = {
            relative_to_play = "post_block",
            offset_ms = 4200,
            canonical_boundary_tick = 3,
            trace_id = overrides.trace_id,
        },
        presentation_order = { "pxi_enjoy_2", "pxi_enjoy_1", "pxi_enjoy_3" },
        randomized_presentation = true,
        responses = responses,
        scores = scores,
    }
end

---@param run_scope_id string
---@param event_id string?
---@return table
function example_package.participant_annotations(run_scope_id, event_id)
    return {
        schema_version = 1,
        manifest_kind = "research_annotation_set",
        digest = research_schema.DIGEST,
        annotation_set_id = "as-participant-debrief-1",
        run_scope_id = run_scope_id,
        session_id = example_package.SESSION_ID,
        author_role = "participant",
        agreement_version = "playtest-agreement-v3",
        coding_scheme_version = "debrief-codes-v2",
        annotations = {
            {
                annotation_id = "an-1",
                canonical_tick = 1,
                boundary_source = "wall_clock_mapped",
                mapping_error_ms = 4.5,
                wall_clock_ms = 1030,
                event_id = event_id,
                code_id = "felt-unfair",
                confidence = 0.6,
                free_text = "I did not see the tackle coming",
            },
        },
    }
end

---@param run_scope_id string
---@return table
function example_package.researcher_annotations(run_scope_id)
    return {
        schema_version = 1,
        manifest_kind = "research_annotation_set",
        digest = research_schema.DIGEST,
        annotation_set_id = "as-researcher-coding-1",
        run_scope_id = run_scope_id,
        session_id = example_package.SESSION_ID,
        author_role = "researcher",
        agreement_version = "playtest-agreement-v3",
        coding_scheme_version = "debrief-codes-v2",
        annotations = {
            {
                annotation_id = "an-r1",
                canonical_tick = 1,
                boundary_source = "canonical_tick",
                mapping_error_ms = 0,
                code_id = "counterplay-unreadable",
                confidence = 0.4,
                disagreement_group = "dg-tackle-readability",
            },
            {
                annotation_id = "an-r2",
                canonical_tick = 0,
                boundary_source = "render_frame_mapped",
                mapping_error_ms = -6.25,
                wall_clock_ms = 1002,
                code_id = "cue-missed",
            },
        },
    }
end

---@param manifest ResearchTraceManifest
---@param overrides table?
---@return ResearchDatasetManifest?, string?
function example_package.dataset(manifest, overrides)
    overrides = overrides or {}
    local manifest_hash = assert(research_trace.content_hash(manifest))
    local envelopes = example_package.dataset_sessions(manifest, overrides)
    local manifest_hashes = {
        manifest_hash,
        research_schema.tuple_hash("example-manifest/v1", { "second" }),
        research_schema.tuple_hash("example-manifest/v1", { "third" }),
    }
    local sources = {}
    for index, envelope in ipairs(envelopes) do
        sources[index] = source_row(envelope, manifest_hashes[index])
    end
    local splits = overrides.splits
        or {
            {
                split_id = "split-participant-holdout",
                grouping = "participant",
                folds = {
                    {
                        fold_id = "train",
                        role = "train",
                        participant_ids = {
                            example_package.PARTICIPANT_ID,
                            example_package.SECOND_PARTICIPANT_ID,
                        },
                        build_ids = {},
                    },
                    {
                        fold_id = "test",
                        role = "test",
                        participant_ids = { example_package.THIRD_PARTICIPANT_ID },
                        build_ids = {},
                    },
                },
            },
            {
                split_id = "split-build-holdout",
                grouping = "build",
                folds = {
                    {
                        fold_id = "train",
                        role = "train",
                        participant_ids = {},
                        build_ids = { "spec-build" },
                    },
                    {
                        fold_id = "holdout",
                        role = "holdout",
                        participant_ids = {},
                        build_ids = { "spec-build-next" },
                    },
                },
            },
        }
    return research_dataset.seal({
        schema_version = 1,
        manifest_kind = "research_dataset_manifest",
        digest = research_schema.DIGEST,
        dataset_id = overrides.dataset_id or "ds-m11-enjoyment-v1",
        dataset_version = 1,
        parent_dataset_hash = overrides.parent_dataset_hash,
        created_wall_clock_ms = 1700000000000,
        purpose = overrides.purpose or "analysis",
        usage = {
            agreement_versions = { "playtest-agreement-v3" },
            model_use_covered = overrides.model_use_covered ~= false,
        },
        extraction = {
            extraction_commit = "0000000000000000000000000000000000000000",
            extraction_config_id = "extraction-config-v1",
            feature_registry_hash = research_features.registry_hash(),
        },
        feature_versions = overrides.feature_versions or {
            {
                feature_id = "pxi_enjoyment_addon.enjoyment",
                version = 1,
                aggregation_level = "condition_block",
            },
            {
                feature_id = "soccer_shape_proxy_score",
                version = 1,
                aggregation_level = "match",
            },
        },
        sources = sources,
        transformations = overrides.transformations or {},
        splits = splits,
        dataset_hash = "0000000000000000",
    })
end

return example_package
