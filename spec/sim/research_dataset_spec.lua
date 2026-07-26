local example_package = require("spec.fixtures.research.example_package")
local research_dataset = require("sim.research_dataset")
local research_features = require("sim.research_features")
local research_schema = require("sim.research_schema")
local t = require("spec.support.runner")

---@return ResearchDatasetManifest
local function dataset()
    local gameplay = example_package.gameplay()
    return assert(example_package.dataset(gameplay.manifest))
end

---@param splits table
---@return ResearchDatasetManifest?, string?
local function dataset_with_splits(splits)
    local gameplay = example_package.gameplay()
    return example_package.dataset(gameplay.manifest, { splits = splits })
end

---@return table
local function participant_split()
    return {
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
    }
end

---@return table
local function build_split()
    return {
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
    }
end

t.describe("research dataset manifest", function()
    t.it("seals, validates, and round-trips", function()
        local manifest = dataset()
        t.is_true(research_dataset.validate(manifest))
        t.eq(manifest.dataset_hash, assert(research_dataset.derive_hash(manifest)))

        local bytes = assert(research_dataset.encode(manifest))
        local decoded = assert(research_dataset.decode(bytes))
        t.eq(assert(research_dataset.encode(decoded)), bytes)
        t.eq(decoded.dataset_id, manifest.dataset_id)
        t.eq(#decoded.splits, 2)
    end)

    t.it("detects any hand edit through the dataset hash", function()
        local manifest = dataset()
        local edited = research_schema.copy(manifest)
        edited.sources[1].condition_id = "condition-a-combat-off"
        local ok, err = research_dataset.validate(edited)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("dataset_hash", 1, true) ~= nil)
    end)

    t.it("pins the feature register it was extracted with", function()
        local manifest = dataset()
        local drifted = research_schema.copy(manifest)
        drifted.extraction.feature_registry_hash = "0000000000000000"
        drifted.dataset_hash = assert(research_dataset.derive_hash(drifted))
        local ok, err = research_dataset.validate(drifted)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("feature register", 1, true) ~= nil)
    end)

    t.it("rejects unknown features, version drift, and undefined aggregation levels", function()
        local gameplay = example_package.gameplay()
        local unknown, unknown_err = example_package.dataset(gameplay.manifest, {
            feature_versions = {
                { feature_id = "no-such-feature", version = 1, aggregation_level = "match" },
            },
        })
        t.is_true(unknown == nil)
        t.is_true(type(unknown_err) == "string")

        local drifted = example_package.dataset(gameplay.manifest, {
            feature_versions = {
                {
                    feature_id = "pxi_enjoyment_addon.enjoyment",
                    version = 99,
                    aggregation_level = "condition_block",
                },
            },
        })
        t.is_true(drifted == nil)

        local wrong_level = example_package.dataset(gameplay.manifest, {
            feature_versions = {
                {
                    feature_id = "pxi_enjoyment_addon.enjoyment",
                    version = 1,
                    aggregation_level = "tick",
                },
            },
        })
        t.is_true(wrong_level == nil)
    end)

    t.it("keeps the feature register hash reachable for lineage", function()
        local manifest = dataset()
        t.eq(manifest.extraction.feature_registry_hash, research_features.registry_hash())
    end)
end)

t.describe("research dataset splits", function()
    t.it("fails closed when participants overlap across folds", function()
        local overlapping = participant_split()
        overlapping.folds[2].participant_ids = { example_package.PARTICIPANT_ID }
        local manifest, err = dataset_with_splits({ overlapping })
        t.is_true(manifest == nil)
        t.is_true(type(err) == "string" and err:find("appears in both", 1, true) ~= nil)
    end)

    t.it("fails closed when builds overlap across folds", function()
        local overlapping = build_split()
        overlapping.folds[2].build_ids = { "spec-build" }
        local manifest, err = dataset_with_splits({ overlapping })
        t.is_true(manifest == nil)
        t.is_true(type(err) == "string" and err:find("appears in both", 1, true) ~= nil)
    end)

    t.it("supports a combined participant-and-build holdout", function()
        local combined = {
            split_id = "split-participant-and-build",
            grouping = "participant_and_build",
            folds = {
                {
                    fold_id = "train",
                    role = "train",
                    participant_ids = {
                        example_package.PARTICIPANT_ID,
                        example_package.SECOND_PARTICIPANT_ID,
                    },
                    build_ids = { "spec-build" },
                },
                {
                    fold_id = "test",
                    role = "test",
                    participant_ids = { example_package.THIRD_PARTICIPANT_ID },
                    build_ids = { "spec-build-next" },
                },
            },
        }
        t.is_true(dataset_with_splits({ combined }) ~= nil)

        local leaky = research_schema.copy(combined)
        leaky.folds[2].build_ids = { "spec-build", "spec-build-next" }
        t.is_true(dataset_with_splits({ leaky }) == nil)
    end)

    t.it("requires full coverage and known members", function()
        local incomplete = participant_split()
        incomplete.folds[1].participant_ids = { example_package.PARTICIPANT_ID }
        local manifest, err = dataset_with_splits({ incomplete })
        t.is_true(manifest == nil)
        t.is_true(type(err) == "string" and err:find("does not cover", 1, true) ~= nil)

        local unknown = participant_split()
        unknown.folds[2].participant_ids = { "p-ffffffffffffffff" }
        t.is_true(dataset_with_splits({ unknown }) == nil)

        local empty_fold = participant_split()
        empty_fold.folds[2].participant_ids = {}
        t.is_true(dataset_with_splits({ empty_fold }) == nil)
    end)

    t.it("rejects a split with no train or no held-out fold", function()
        local train_only = participant_split()
        train_only.folds[2].role = "train"
        t.is_true(dataset_with_splits({ train_only }) == nil)

        local test_only = participant_split()
        test_only.folds[1].role = "validation"
        t.is_true(dataset_with_splits({ test_only }) == nil)
    end)

    t.it("keeps participant and build groupings from bleeding into each other", function()
        local participant_with_builds = participant_split()
        participant_with_builds.folds[1].build_ids = { "spec-build" }
        t.is_true(dataset_with_splits({ participant_with_builds }) == nil)

        local build_with_participants = build_split()
        build_with_participants.folds[1].participant_ids = { example_package.PARTICIPANT_ID }
        t.is_true(dataset_with_splits({ build_with_participants }) == nil)
    end)

    t.it("requires a split manifest for model training", function()
        local gameplay = example_package.gameplay()
        local without_splits = example_package.dataset(gameplay.manifest, {
            purpose = "model_training",
            splits = {},
        })
        t.is_true(without_splits == nil)
        t.is_true(example_package.dataset(gameplay.manifest, { purpose = "model_training" }) ~= nil)
    end)
end)

t.describe("research dataset lineage", function()
    t.it("requires a parent hash whenever a transformation is recorded", function()
        local gameplay = example_package.gameplay()
        local orphan = example_package.dataset(gameplay.manifest, {
            transformations = {
                {
                    transformation_id = "drop-withdrawn",
                    description = "rebuild after a withdrawal tombstone",
                    input_hash = "1111111111111111",
                    output_hash = "2222222222222222",
                },
            },
        })
        t.is_true(orphan == nil)

        local child = example_package.dataset(gameplay.manifest, {
            dataset_id = "ds-m11-enjoyment-v2",
            parent_dataset_hash = "3333333333333333",
            transformations = {
                {
                    transformation_id = "drop-withdrawn",
                    description = "rebuild after a withdrawal tombstone",
                    input_hash = "1111111111111111",
                    output_hash = "2222222222222222",
                },
            },
        })
        t.is_true(child ~= nil)
        t.is_true(assert(child).parent_dataset_hash == "3333333333333333")

        local no_op = example_package.dataset(gameplay.manifest, {
            parent_dataset_hash = "3333333333333333",
            transformations = {
                {
                    transformation_id = "no-op",
                    description = "changes nothing",
                    input_hash = "1111111111111111",
                    output_hash = "1111111111111111",
                },
            },
        })
        t.is_true(no_op == nil)
    end)

    t.it("ties model-use permission to every source's accepted agreement", function()
        local gameplay = example_package.gameplay()
        t.is_true(example_package.dataset(gameplay.manifest, { purpose = "model_training" }) ~= nil)

        local uncovered_training = example_package.dataset(gameplay.manifest, {
            purpose = "model_training",
            model_use_covered = false,
        })
        t.is_true(uncovered_training == nil)

        local manifest = dataset()
        local retroactive = research_schema.copy(manifest)
        retroactive.sources[2].model_use_covered = false
        retroactive.dataset_hash = assert(research_dataset.derive_hash(retroactive))
        local ok, err = research_dataset.validate(retroactive)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("model_use_covered", 1, true) ~= nil)

        local undeclared_version = research_schema.copy(manifest)
        undeclared_version.sources[1].agreement_version = "playtest-agreement-v9"
        undeclared_version.dataset_hash = assert(research_dataset.derive_hash(undeclared_version))
        local version_ok, version_err = research_dataset.validate(undeclared_version)
        t.is_true(version_ok == nil)
        t.is_true(
            type(version_err) == "string" and version_err:find("does not declare", 1, true) ~= nil
        )
    end)

    t.it("verifies every source row against the session envelope it came from", function()
        local gameplay = example_package.gameplay()
        local manifest = assert(example_package.dataset(gameplay.manifest))
        local envelopes = example_package.dataset_sessions(gameplay.manifest)
        t.is_true(research_dataset.validate_against_sessions(manifest, envelopes))

        -- The row says model use was covered; the accepted agreement says it was
        -- not. Internal self-consistency cannot see this; the join can.
        local uncovered = example_package.dataset_sessions(gameplay.manifest, {
            third_model_use_covered = false,
        })
        local ok, raw_err = research_dataset.validate_against_sessions(manifest, uncovered)
        local err = assert(raw_err)
        t.is_true(ok == nil)
        t.is_true(err:find("model_use_covered", 1, true) ~= nil)
        t.is_true(err:find("accepted agreement", 1, true) ~= nil)

        local drifted_version = example_package.dataset_sessions(gameplay.manifest)
        drifted_version[2].agreement.agreement_version = "playtest-agreement-v2"
        local version_ok, version_err =
            research_dataset.validate_against_sessions(manifest, drifted_version)
        t.is_true(version_ok == nil)
        t.is_true(
            type(version_err) == "string"
                and version_err:find("but the session accepted", 1, true) ~= nil
        )

        local missing_session = example_package.dataset_sessions(gameplay.manifest)
        table.remove(missing_session, 3)
        local orphan_ok, orphan_err =
            research_dataset.validate_against_sessions(manifest, missing_session)
        t.is_true(orphan_ok == nil)
        t.is_true(type(orphan_err) == "string" and orphan_err:find("orphan join", 1, true) ~= nil)

        local duplicated = example_package.dataset_sessions(gameplay.manifest)
        duplicated[3] = research_schema.copy(duplicated[2])
        t.is_true(not research_dataset.validate_against_sessions(manifest, duplicated))
    end)

    t.it("refuses a dataset whose source session was withdrawn", function()
        local gameplay = example_package.gameplay()
        local manifest = assert(example_package.dataset(gameplay.manifest))
        local envelopes = example_package.dataset_sessions(gameplay.manifest)
        local withdrawn = example_package.withdrawn_session()
        withdrawn.session_id = envelopes[2].session_id
        withdrawn.participant_id = envelopes[2].participant_id
        envelopes[2] = withdrawn
        local ok, err = research_dataset.validate_against_sessions(manifest, envelopes)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("withdrawn session", 1, true) ~= nil)
    end)

    t.it("refuses a source row that pins a trace its session never referenced", function()
        local gameplay = example_package.gameplay()
        local manifest = assert(example_package.dataset(gameplay.manifest))
        local envelopes = example_package.dataset_sessions(gameplay.manifest)
        envelopes[1].trace_links[1].trace_id = "abcdefabcdefabcd"
        local ok, err = research_dataset.validate_against_sessions(manifest, envelopes)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("does not reference", 1, true) ~= nil)
    end)

    t.it("rejects non-finite numbers in this family", function()
        local manifest = dataset()
        local nan = research_schema.copy(manifest)
        nan.created_wall_clock_ms = 0 / 0
        t.is_true(not research_dataset.validate(nan))
    end)

    t.it("requires excluded sources to record a reason and stay out of folds", function()
        local manifest = dataset()
        local unexplained = research_schema.copy(manifest)
        unexplained.sources[1].excluded = true
        unexplained.dataset_hash = assert(research_dataset.derive_hash(unexplained))
        local ok, err = research_dataset.validate(unexplained)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string")

        local excluded_in_fold = research_schema.copy(manifest)
        excluded_in_fold.sources[3].excluded = true
        excluded_in_fold.sources[3].exclusion_reason = "technical_failure"
        excluded_in_fold.dataset_hash = assert(research_dataset.derive_hash(excluded_in_fold))
        local fold_ok, fold_err = research_dataset.validate(excluded_in_fold)
        t.is_true(fold_ok == nil)
        t.is_true(
            type(fold_err) == "string" and fold_err:find("excluded participant", 1, true) ~= nil
        )
    end)
end)
