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
--
-- These hashes cover the tape's simulation identity, which includes
-- `match_snapshot.VERSION` and `COMBAT_VERSION`. A bump to either legitimately
-- moves every hash below, so the versions they were computed against are pinned
-- here: a stale vector then reports "computed against snapshot version N" rather
-- than an opaque 16-hex mismatch that looks like corruption. Same lesson as #196,
-- where input-packet goldens pinned at snapshot version 9 met a version-10 encoder
-- and took `main` red.
canonical_vectors.SNAPSHOT_VERSION = 11
canonical_vectors.COMBAT_VERSION = 12
canonical_vectors.TAPE_CONTENT_HASH = "6f21da271b5a4603"
canonical_vectors.TRACE_ID = "d7491ed5cc4cd10b"
canonical_vectors.SIMULATION_IDENTITY_HASH = "4a9637a871966a67"
canonical_vectors.TRACE_MANIFEST_HASH = "b0247e882a7a63ef"
canonical_vectors.EVENT_STREAM_HASH = "c9a43ff1a3657659"
canonical_vectors.SESSION_ENVELOPE_HASH = "0da43aba0805a72b"
canonical_vectors.RESPONSE_SET_HASH = "04b559ff59cea90e"
-- Covers every authored value in the feature register, including the prose in
-- `goodhart_failure` and `confounds`. Editing that prose is a register change and
-- moves this hash, which is the intended behaviour: the register is content, and
-- a reader that trusted a stale hash could not tell a reworded caveat from a
-- reclassified feature.
canonical_vectors.FEATURE_REGISTRY_HASH = "7a42fe98b1bc784c"

return canonical_vectors
