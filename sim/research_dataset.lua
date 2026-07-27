-- Dataset, split, and lineage manifests.
--
-- A dataset manifest is the only artifact allowed to claim "these rows are what
-- we analysed". It therefore names its immutable sources by content hash, the
-- exact feature versions and register hash used, its transformations, the
-- agreement versions its sources were recorded under, and its parent dataset.
--
-- `dataset_hash` and the pinned `trace_manifest_hash` values are integrity
-- checks against accidental corruption and hand-edits. They are not a
-- tamper-proof audit trail: anyone who can rewrite a manifest can recompute
-- them.
--
-- Splits fail closed on overlap. Held-out *participants* stop leakage between
-- people; held-out *builds and tuning configurations* stop leakage between the
-- very configurations a model would be asked to generalize across. A split may
-- require both, and coverage is checked in each direction so a silently dropped
-- participant is an error rather than an unnoticed exclusion.

local research_features = require("sim.research_features")
local research_schema = require("sim.research_schema")
local research_session = require("sim.research_session")

---@class ResearchDatasetSourceRow
---@field trace_id string
---@field trace_manifest_hash string
---@field session_id string
---@field participant_id string
---@field condition_id string
---@field build_id string
---@field tuning_config_id string
---@field excluded boolean
---@field exclusion_reason string?
---@field agreement_version string
---@field model_use_covered boolean

---@class ResearchDatasetFold
---@field fold_id string
---@field role "train"|"validation"|"test"|"holdout"
---@field participant_ids string[]
---@field build_ids string[]

---@class ResearchDatasetSplit
---@field split_id string
---@field grouping "participant"|"build"|"participant_and_build"
---@field folds ResearchDatasetFold[]

---@class ResearchDatasetManifest
---@field schema_version integer
---@field manifest_kind "research_dataset_manifest"
---@field digest string
---@field dataset_id string
---@field dataset_version integer
---@field parent_dataset_hash string?
---@field created_wall_clock_ms number
---@field purpose "analysis"|"model_training"|"qa"
---@field usage table
---@field extraction table
---@field feature_versions table[]
---@field sources ResearchDatasetSourceRow[]
---@field transformations table[]
---@field splits ResearchDatasetSplit[]
---@field dataset_hash string

---@class ResearchDatasetModule
local research_dataset = {}

research_dataset.VERSION = 1
research_dataset.SUPPORTED_VERSIONS = { [1] = true }
research_dataset.KIND = "research_dataset_manifest"
research_dataset.HASH_LABEL = "research-dataset/v1"

research_dataset.FOLD_ROLES = research_schema.enum({ "train", "validation", "test", "holdout" })
research_dataset.GROUPINGS =
    research_schema.enum({ "participant", "build", "participant_and_build" })

local SOURCE_FIELDS = {
    { name = "trace_id", kind = "hash" },
    { name = "trace_manifest_hash", kind = "hash" },
    { name = "session_id", kind = "id", min_length = 16 },
    { name = "participant_id", kind = "id", min_length = 16 },
    { name = "condition_id", kind = "id" },
    { name = "build_id", kind = "id" },
    { name = "tuning_config_id", kind = "id" },
    { name = "excluded", kind = "boolean" },
    { name = "exclusion_reason", kind = "id", optional = true },
    { name = "agreement_version", kind = "id" },
    { name = "model_use_covered", kind = "boolean" },
}

local FOLD_FIELDS = {
    { name = "fold_id", kind = "id" },
    { name = "role", kind = "enum", values = research_dataset.FOLD_ROLES },
    {
        name = "participant_ids",
        kind = "array",
        element = { name = "participant_id", kind = "id", min_length = 16 },
    },
    {
        name = "build_ids",
        kind = "array",
        element = { name = "build_id", kind = "id" },
    },
}

local SPLIT_FIELDS = {
    { name = "split_id", kind = "id" },
    { name = "grouping", kind = "enum", values = research_dataset.GROUPINGS },
    {
        name = "folds",
        kind = "array",
        min_length = 2,
        max_length = 64,
        element = { name = "fold", kind = "record", fields = FOLD_FIELDS },
    },
}

-- Internal-analysis playtest data. The only permission question that survives is
-- whether the accepted agreement versions covered training and shipping a model,
-- and that cannot be granted after the fact.
local USAGE_FIELDS = {
    {
        name = "agreement_versions",
        kind = "array",
        min_length = 1,
        element = { name = "agreement_version", kind = "id" },
    },
    { name = "model_use_covered", kind = "boolean" },
}

local EXTRACTION_FIELDS = {
    { name = "extraction_commit", kind = "string" },
    { name = "extraction_config_id", kind = "id" },
    { name = "feature_registry_hash", kind = "hash" },
}

local FEATURE_VERSION_FIELDS = {
    { name = "feature_id", kind = "id" },
    { name = "version", kind = "integer", min = 1 },
    {
        name = "aggregation_level",
        kind = "enum",
        values = research_features.GRAINS,
    },
}

local TRANSFORMATION_FIELDS = {
    { name = "transformation_id", kind = "id" },
    { name = "description", kind = "string", max_length = 512 },
    { name = "input_hash", kind = "hash" },
    { name = "output_hash", kind = "hash" },
}

research_dataset.SHAPE = research_schema.record("research_dataset_manifest/v1", {
    { name = "schema_version", kind = "integer", min = 1 },
    {
        name = "manifest_kind",
        kind = "enum",
        values = research_schema.enum({ research_dataset.KIND }),
    },
    { name = "digest", kind = "enum", values = research_schema.enum({ research_schema.DIGEST }) },
    { name = "dataset_id", kind = "id" },
    { name = "dataset_version", kind = "integer", min = 1 },
    { name = "parent_dataset_hash", kind = "hash", optional = true },
    { name = "created_wall_clock_ms", kind = "number", min = 0 },
    {
        name = "purpose",
        kind = "enum",
        values = research_schema.enum({ "analysis", "model_training", "qa" }),
    },
    { name = "usage", kind = "record", fields = USAGE_FIELDS },
    { name = "extraction", kind = "record", fields = EXTRACTION_FIELDS },
    {
        name = "feature_versions",
        kind = "array",
        min_length = 1,
        max_length = 512,
        element = { name = "feature_version", kind = "record", fields = FEATURE_VERSION_FIELDS },
    },
    {
        name = "sources",
        kind = "array",
        min_length = 1,
        max_length = 8192,
        element = { name = "source", kind = "record", fields = SOURCE_FIELDS },
    },
    {
        name = "transformations",
        kind = "array",
        max_length = 256,
        element = { name = "transformation", kind = "record", fields = TRANSFORMATION_FIELDS },
    },
    {
        name = "splits",
        kind = "array",
        max_length = 32,
        element = { name = "split", kind = "record", fields = SPLIT_FIELDS },
    },
    { name = "dataset_hash", kind = "hash" },
})

---@param manifest ResearchDatasetManifest
---@return string?, string?
function research_dataset.derive_hash(manifest)
    local body = research_schema.copy(manifest)
    ---@cast body ResearchDatasetManifest
    body.dataset_hash = "0000000000000000"
    local content, err = research_schema.content_hash(research_dataset.SHAPE, body)
    if not content then
        return nil, err
    end
    return research_schema.tuple_hash(research_dataset.HASH_LABEL, {
        content,
        manifest.dataset_id,
        manifest.dataset_version,
    })
end

---@param split ResearchDatasetSplit
---@param path string
---@param known_participants table<string, boolean>
---@param known_builds table<string, boolean>
---@return boolean?, string?
local function validate_split(split, path, known_participants, known_builds)
    local checks_participants = split.grouping ~= "build"
    local checks_builds = split.grouping ~= "participant"
    local participant_groups = {}
    local build_groups = {}
    local fold_ids = {}
    local roles = {}
    for index, fold in ipairs(split.folds) do
        local fold_path = path .. ".folds." .. index
        if fold_ids[fold.fold_id] then
            return nil, fold_path .. " duplicates fold id " .. fold.fold_id
        end
        fold_ids[fold.fold_id] = true
        roles[fold.role] = true
        if checks_participants then
            if #fold.participant_ids == 0 then
                return nil, fold_path .. " must hold at least one participant"
            end
            local seen = {}
            for _, participant_id in ipairs(fold.participant_ids) do
                if seen[participant_id] then
                    return nil, fold_path .. " repeats participant " .. participant_id
                end
                seen[participant_id] = true
                if not known_participants[participant_id] then
                    return nil,
                        fold_path
                            .. " references participant "
                            .. participant_id
                            .. " with no source row"
                end
            end
            participant_groups[fold.fold_id] = fold.participant_ids
        elseif #fold.participant_ids > 0 then
            return nil, fold_path .. " declares participants in a build-only split"
        end
        if checks_builds then
            if #fold.build_ids == 0 then
                return nil, fold_path .. " must hold at least one build"
            end
            local seen = {}
            for _, build_id in ipairs(fold.build_ids) do
                if seen[build_id] then
                    return nil, fold_path .. " repeats build " .. build_id
                end
                seen[build_id] = true
                if not known_builds[build_id] then
                    return nil,
                        fold_path .. " references build " .. build_id .. " with no source row"
                end
            end
            build_groups[fold.fold_id] = fold.build_ids
        elseif #fold.build_ids > 0 then
            return nil, fold_path .. " declares builds in a participant-only split"
        end
    end
    if not roles.train then
        return nil, path .. " needs a train fold"
    end
    if not (roles.test or roles.holdout) then
        return nil, path .. " needs a test or holdout fold"
    end
    if checks_participants then
        local ok, err = research_schema.assert_disjoint(path .. " participants", participant_groups)
        if not ok then
            return nil, err
        end
        for participant_id in pairs(known_participants) do
            local found = false
            for _, ids in pairs(participant_groups) do
                for _, candidate in ipairs(ids) do
                    if candidate == participant_id then
                        found = true
                    end
                end
            end
            if not found then
                return nil, path .. " does not cover participant " .. participant_id
            end
        end
    end
    if checks_builds then
        local ok, err = research_schema.assert_disjoint(path .. " builds", build_groups)
        if not ok then
            return nil, err
        end
        for build_id in pairs(known_builds) do
            local found = false
            for _, ids in pairs(build_groups) do
                for _, candidate in ipairs(ids) do
                    if candidate == build_id then
                        found = true
                    end
                end
            end
            if not found then
                return nil, path .. " does not cover build " .. build_id
            end
        end
    end
    return true
end

---@param manifest any
---@return boolean?, string?
function research_dataset.validate(manifest)
    if type(manifest) ~= "table" then
        return nil, "research dataset manifest must be a table"
    end
    local versioned, version_err = research_schema.accepts_version(
        research_dataset.KIND,
        research_dataset.SUPPORTED_VERSIONS,
        research_dataset.VERSION,
        manifest.schema_version
    )
    if not versioned then
        return nil, version_err
    end
    local ok, err = research_schema.validate(research_dataset.SHAPE, manifest)
    if not ok then
        return nil, err
    end
    ---@cast manifest ResearchDatasetManifest

    if manifest.extraction.feature_registry_hash ~= research_features.registry_hash() then
        return nil,
            "research_dataset_manifest.extraction.feature_registry_hash does not match the active feature register"
    end
    local seen_features = {}
    for index, entry in ipairs(manifest.feature_versions) do
        local path = "research_dataset_manifest.feature_versions." .. index
        if seen_features[entry.feature_id] then
            return nil, path .. " duplicates feature " .. entry.feature_id
        end
        seen_features[entry.feature_id] = true
        local feature, feature_err = research_features.feature(entry.feature_id)
        if not feature then
            return nil, path .. ": " .. tostring(feature_err)
        end
        if feature.version ~= entry.version then
            return nil,
                path
                    .. " pins version "
                    .. tostring(entry.version)
                    .. " but the register defines version "
                    .. tostring(feature.version)
        end
        local level_ok, level_err =
            research_features.aggregation_allowed(entry.feature_id, entry.aggregation_level)
        if not level_ok then
            return nil, path .. ": " .. tostring(level_err)
        end
        if manifest.purpose == "model_training" then
            local use_ok, use_err = research_features.use_allowed(entry.feature_id, "model_input")
            if not use_ok then
                return nil, path .. ": " .. tostring(use_err)
            end
        end
    end

    local known_participants = {}
    local known_builds = {}
    local seen_traces = {}
    for index, row in ipairs(manifest.sources) do
        local path = "research_dataset_manifest.sources." .. index
        if seen_traces[row.trace_id] then
            return nil, path .. " duplicates trace " .. row.trace_id
        end
        seen_traces[row.trace_id] = true
        if row.excluded then
            if row.exclusion_reason == nil then
                return nil, path .. " must record why it is excluded"
            end
        elseif row.exclusion_reason ~= nil then
            return nil, path .. " records an exclusion reason without being excluded"
        else
            known_participants[row.participant_id] = true
            known_builds[row.build_id] = true
        end
    end
    if next(known_participants) == nil then
        return nil, "research_dataset_manifest.sources has no included rows"
    end

    local excluded_participants = {}
    for _, row in ipairs(manifest.sources) do
        if row.excluded and not known_participants[row.participant_id] then
            excluded_participants[row.participant_id] = true
        end
    end

    for index, split in ipairs(manifest.splits) do
        local path = "research_dataset_manifest.splits." .. index
        for _, fold in ipairs(split.folds) do
            for _, participant_id in ipairs(fold.participant_ids) do
                if excluded_participants[participant_id] then
                    return nil, path .. " includes fully excluded participant " .. participant_id
                end
            end
        end
        local split_ok, split_err = validate_split(split, path, known_participants, known_builds)
        if not split_ok then
            return nil, split_err
        end
    end
    if manifest.purpose == "model_training" and #manifest.splits == 0 then
        return nil, "research_dataset_manifest for model training requires a split manifest"
    end

    -- Model-use permission is a property of the agreement each participant
    -- accepted, so a dataset may only claim it when every included source row
    -- carries it. It can never be granted retroactively at the dataset level.
    local declared_versions = {}
    for _, version in ipairs(manifest.usage.agreement_versions) do
        declared_versions[version] = true
    end
    local every_source_covers = true
    for index, row in ipairs(manifest.sources) do
        local path = "research_dataset_manifest.sources." .. index
        if not declared_versions[row.agreement_version] then
            return nil,
                path
                    .. " was recorded under agreement version "
                    .. row.agreement_version
                    .. " which the dataset does not declare"
        end
        if not row.excluded and not row.model_use_covered then
            every_source_covers = false
        end
    end
    if manifest.usage.model_use_covered and not every_source_covers then
        return nil,
            "research_dataset_manifest.usage.model_use_covered requires every included source to be covered by its accepted agreement"
    end
    if manifest.purpose == "model_training" and not manifest.usage.model_use_covered then
        return nil,
            "research_dataset_manifest for model training requires agreement-covered model use"
    end

    if #manifest.transformations > 0 and manifest.parent_dataset_hash == nil then
        return nil,
            "research_dataset_manifest with transformations must name its parent dataset hash"
    end
    local seen_transformations = {}
    for index, transformation in ipairs(manifest.transformations) do
        local path = "research_dataset_manifest.transformations." .. index
        if seen_transformations[transformation.transformation_id] then
            return nil, path .. " is duplicated"
        end
        seen_transformations[transformation.transformation_id] = true
        if transformation.input_hash == transformation.output_hash then
            return nil, path .. " must change its payload or not be recorded"
        end
    end
    local expected, hash_err = research_dataset.derive_hash(manifest)
    if not expected then
        return nil, hash_err
    end
    if manifest.dataset_hash ~= expected then
        return nil, "research_dataset_manifest.dataset_hash does not cover its contents"
    end
    return true
end

-- Seal a manifest by deriving its dataset hash, then validate it.
---@param manifest ResearchDatasetManifest
---@return ResearchDatasetManifest?, string?
function research_dataset.seal(manifest)
    if type(manifest) ~= "table" then
        return nil, "research dataset manifest must be a table"
    end
    local sealed = research_schema.copy(manifest)
    ---@cast sealed ResearchDatasetManifest
    sealed.dataset_hash = "0000000000000000"
    local shape_ok, shape_err = research_schema.validate(research_dataset.SHAPE, sealed)
    if not shape_ok then
        return nil, shape_err
    end
    local hash, hash_err = research_dataset.derive_hash(sealed)
    if not hash then
        return nil, hash_err
    end
    sealed.dataset_hash = hash
    local ok, err = research_dataset.validate(sealed)
    if not ok then
        return nil, err
    end
    return sealed
end

-- Join every source row back to the session envelope it came from.
--
-- `agreement_version` and `model_use_covered` are *self-declared* on a source
-- row, which is only useful if it can be checked against the accepted agreement
-- it claims to reflect. This is that check: without it, a dataset builder could
-- assert model-use permission for a participant who never granted it, and the
-- non-retroactivity guarantee would be unverifiable rather than merely
-- unenforced.
---@param manifest ResearchDatasetManifest
---@param envelopes ResearchSessionEnvelope[]
---@return boolean?, string?
function research_dataset.validate_against_sessions(manifest, envelopes)
    local ok, err = research_dataset.validate(manifest)
    if not ok then
        return nil, err
    end
    if type(envelopes) ~= "table" then
        return nil, "research session envelopes must be an array"
    end
    local by_session = {}
    for index, envelope in ipairs(envelopes) do
        local envelope_ok, envelope_err = research_session.validate(envelope)
        if not envelope_ok then
            return nil, "research session envelope " .. index .. ": " .. tostring(envelope_err)
        end
        if by_session[envelope.session_id] then
            return nil, "research session envelope " .. envelope.session_id .. " is supplied twice"
        end
        by_session[envelope.session_id] = envelope
    end

    for index, row in ipairs(manifest.sources) do
        local path = "research_dataset_manifest.sources." .. index
        local envelope = by_session[row.session_id]
        if not envelope then
            return nil, path .. " is an orphan join"
        end
        if envelope.lifecycle.status == "withdrawn" then
            return nil, path .. " retains a withdrawn session"
        end
        if row.participant_id ~= envelope.participant_id then
            return nil, path .. " names another participant than its session"
        end
        if row.condition_id ~= envelope.condition_id then
            return nil, path .. " names another condition than its session"
        end
        if row.build_id ~= envelope.assignment.build_id then
            return nil, path .. " names another build than its session"
        end
        if row.tuning_config_id ~= envelope.assignment.tuning_config_id then
            return nil, path .. " names another tuning configuration than its session"
        end
        if row.agreement_version ~= envelope.agreement.agreement_version then
            return nil,
                path
                    .. " claims agreement version "
                    .. row.agreement_version
                    .. " but the session accepted "
                    .. envelope.agreement.agreement_version
        end
        local model_use_allowed = research_session.allows_model_use(envelope) == true
        if row.model_use_covered ~= model_use_allowed then
            return nil,
                path
                    .. " claims model_use_covered="
                    .. tostring(row.model_use_covered)
                    .. " but its accepted agreement says "
                    .. tostring(model_use_allowed)
        end
        local linked = false
        for _, link in ipairs(envelope.trace_links) do
            if link.trace_id == row.trace_id then
                linked = true
            end
        end
        if not linked then
            return nil, path .. " pins a trace its session does not reference"
        end
    end
    return true
end

-- A dataset may not contain a payload a withdrawal tombstone revoked. Derived
-- datasets are rebuilt from the tombstoned manifest, never hand-patched.
---@param manifest ResearchDatasetManifest
---@param tombstones ResearchWithdrawalTombstone[]
---@return boolean?, string?
function research_dataset.validate_against_tombstones(manifest, tombstones)
    local ok, err = research_dataset.validate(manifest)
    if not ok then
        return nil, err
    end
    if type(tombstones) ~= "table" then
        return nil, "research withdrawal tombstones must be an array"
    end
    local revoked_participants = {}
    local revoked_sessions = {}
    local revoked_hashes = {}
    for index, tombstone in ipairs(tombstones) do
        if type(tombstone) ~= "table" then
            return nil, "research withdrawal tombstone " .. index .. " must be a table"
        end
        -- Withdrawal is always full: the session goes and anything derived from
        -- it is regenerated. Revoking by participant and session as well as by
        -- payload hash is what keeps this robust against a source row that
        -- pinned a manifest hash at a different point in time.
        revoked_participants[tombstone.participant_id] = true
        revoked_sessions[tombstone.session_id] = true
        for _, hash in ipairs(tombstone.revoked_payload_hashes or {}) do
            revoked_hashes[hash] = true
        end
    end
    for index, row in ipairs(manifest.sources) do
        local path = "research_dataset_manifest.sources." .. index
        if revoked_participants[row.participant_id] then
            return nil, path .. " retains withdrawn participant " .. row.participant_id
        end
        if revoked_sessions[row.session_id] then
            return nil, path .. " retains withdrawn session " .. row.session_id
        end
        if revoked_hashes[row.trace_manifest_hash] then
            return nil, path .. " retains a revoked payload hash"
        end
    end
    return true
end

---@param manifest ResearchDatasetManifest
---@return string?, string?
function research_dataset.encode(manifest)
    local ok, err = research_dataset.validate(manifest)
    if not ok then
        return nil, err
    end
    return research_schema.encode(research_dataset.SHAPE, manifest)
end

---@param bytes string
---@return ResearchDatasetManifest?, string?
function research_dataset.decode(bytes)
    local manifest, err = research_schema.decode(research_dataset.SHAPE, bytes)
    if not manifest then
        return nil, err
    end
    local ok, validate_err = research_dataset.validate(manifest)
    if not ok then
        return nil, validate_err
    end
    return manifest
end

return research_dataset
