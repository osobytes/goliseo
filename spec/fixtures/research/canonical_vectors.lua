-- Frozen canonical serialization vectors for the research contracts.
--
-- These pins exist so that any change to the length-prefixed encoding, to
-- canonical field order, or to the fnv1a64 digest is caught as a deliberate
-- serialization-version decision instead of an accidental silent break. They
-- also pin the small hand-written payload byte-for-byte, so a reviewer can read
-- the exact wire format in the diff.
--
-- Updating a value here is a contract change: bump
-- `research_schema.SERIALIZATION_VERSION` (or the owning contract version) and
-- record the migration in docs/design/player_evidence_contracts.md.

---@class ResearchCanonicalVectors
local canonical_vectors = {}

canonical_vectors.SERIALIZATION_VERSION = 1
canonical_vectors.DIGEST = "fnv1a64/v1"

-- Hand-written minimal payload; see spec/sim/research_canonical_spec.lua.
canonical_vectors.SAMPLE_SHAPE_NAME = "research_canonical_sample/v1"
canonical_vectors.SAMPLE_BYTES = table.concat({
    "GCRS1;",
    "28:research_canonical_sample/v1;",
    "r1:4;",
    "k2:id;s6:sample;",
    "k5:count;i1:7;",
    "k5:share;d14:p:0:33554432:0;",
    "k4:tags;a1:1;e5:alpha;",
})
canonical_vectors.SAMPLE_HASH = "2c43b30a590f0da8"

-- Derived from the checked-in short match tape and the research example package.
canonical_vectors.TAPE_CONTENT_HASH = "bb9f685463450a85"
canonical_vectors.TRACE_ID = "f29c674011a3c120"
canonical_vectors.SIMULATION_IDENTITY_HASH = "865e13fec60ffb05"
canonical_vectors.TRACE_MANIFEST_HASH = "27e1ae117826fe6f"
canonical_vectors.EVENT_STREAM_HASH = "30d828e1df9ead57"
canonical_vectors.SESSION_ENVELOPE_HASH = "538ddc4bdc387539"
canonical_vectors.RESPONSE_SET_HASH = "04b559ff59cea90e"
-- Covers every authored value in the feature register, including the prose in
-- `goodhart_failure` and `confounds`. Editing that prose is a register change and
-- moves this hash, which is the intended behaviour: the register is content, and
-- a reader that trusted a stale hash could not tell a reworded caveat from a
-- reclassified feature.
canonical_vectors.FEATURE_REGISTRY_HASH = "7a42fe98b1bc784c"

return canonical_vectors
