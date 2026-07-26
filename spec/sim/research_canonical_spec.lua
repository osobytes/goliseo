local canonical_vectors = require("spec.fixtures.research.canonical_vectors")
local example_package = require("spec.fixtures.research.example_package")
local research_features = require("sim.research_features")
local research_response = require("sim.research_response")
local research_schema = require("sim.research_schema")
local research_session = require("sim.research_session")
local research_trace = require("sim.research_trace")
local t = require("spec.support.runner")

local SAMPLE_SHAPE = research_schema.record(canonical_vectors.SAMPLE_SHAPE_NAME, {
    { name = "id", kind = "id" },
    { name = "count", kind = "integer" },
    { name = "share", kind = "number" },
    {
        name = "tags",
        kind = "array",
        element = {
            name = "tag",
            kind = "enum",
            values = research_schema.enum({ "alpha", "beta" }),
        },
    },
})

---@return table
local function sample()
    return { id = "sample", count = 7, share = 0.5, tags = { "alpha" } }
end

t.describe("research canonical serialization vectors", function()
    t.it("pins the serialization version and digest name", function()
        t.eq(research_schema.SERIALIZATION_VERSION, canonical_vectors.SERIALIZATION_VERSION)
        t.eq(research_schema.DIGEST, canonical_vectors.DIGEST)
    end)

    t.it("pins the hand-written sample wire bytes and digest", function()
        local bytes = assert(research_schema.encode(SAMPLE_SHAPE, sample()))
        t.eq(bytes, canonical_vectors.SAMPLE_BYTES)
        t.eq(
            assert(research_schema.content_hash(SAMPLE_SHAPE, sample())),
            canonical_vectors.SAMPLE_HASH
        )
        local decoded = assert(research_schema.decode(SAMPLE_SHAPE, bytes))
        t.eq(assert(research_schema.encode(SAMPLE_SHAPE, decoded)), bytes)
    end)

    t.it("pins the example package content hashes", function()
        local gameplay = example_package.gameplay()
        t.eq(gameplay.manifest.simulation.tape_content_hash, canonical_vectors.TAPE_CONTENT_HASH)
        t.eq(gameplay.manifest.trace_id, canonical_vectors.TRACE_ID)
        t.eq(
            assert(research_trace.simulation_identity_hash(gameplay.manifest)),
            canonical_vectors.SIMULATION_IDENTITY_HASH
        )
        t.eq(
            assert(research_trace.content_hash(gameplay.manifest)),
            canonical_vectors.TRACE_MANIFEST_HASH
        )
        t.eq(gameplay.stream.stream_hash, canonical_vectors.EVENT_STREAM_HASH)

        local envelope = example_package.completed_session(gameplay.manifest.trace_id)
        t.eq(
            assert(research_session.content_hash(envelope)),
            canonical_vectors.SESSION_ENVELOPE_HASH
        )

        local responses = example_package.enjoyment_responses()
        t.eq(assert(research_response.content_hash(responses)), canonical_vectors.RESPONSE_SET_HASH)
        t.eq(research_features.registry_hash(), canonical_vectors.FEATURE_REGISTRY_HASH)
    end)
end)
