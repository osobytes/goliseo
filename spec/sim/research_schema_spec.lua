local research_schema = require("sim.research_schema")
local t = require("spec.support.runner")

local SHAPE = research_schema.record("spec_shape", {
    { name = "id", kind = "id" },
    { name = "count", kind = "integer", min = 0 },
    { name = "share", kind = "number", min = 0, max = 1 },
    { name = "flag", kind = "boolean" },
    { name = "status", kind = "enum", values = research_schema.enum({ "open", "closed" }) },
    { name = "digest", kind = "hash" },
    { name = "note", kind = "text", optional = true },
    { name = "tags", kind = "array", element = { name = "tag", kind = "id" } },
    {
        name = "scores",
        kind = "map",
        element = { name = "score", kind = "integer" },
    },
    {
        name = "nested",
        kind = "record",
        optional = true,
        fields = {
            { name = "label", kind = "string" },
            { name = "weight", kind = "number" },
        },
    },
})

---@return table
local function payload()
    return {
        id = "session.alpha-01",
        count = 3,
        share = 0.25,
        flag = true,
        status = "open",
        digest = "0123456789abcdef",
        tags = { "a", "b" },
        scores = { home = 1, away = 2 },
    }
end

t.describe("research schema validation", function()
    t.it("accepts a complete payload and reports the failing path otherwise", function()
        t.is_true(research_schema.validate(SHAPE, payload()))

        local missing = payload()
        missing.count = nil
        local ok, err = research_schema.validate(SHAPE, missing)
        t.is_true(ok == nil)
        t.eq(err, "spec_shape.count is required")
    end)

    t.it("rejects unknown fields instead of ignoring them", function()
        local unknown = payload()
        unknown.future_field = 1
        local ok, err = research_schema.validate(SHAPE, unknown)
        t.is_true(ok == nil)
        t.eq(err, "spec_shape has unknown field future_field")
    end)

    t.it("rejects bad enums, out-of-range and non-finite numbers", function()
        local bad_enum = payload()
        bad_enum.status = "paused"
        t.is_true(not research_schema.validate(SHAPE, bad_enum))

        local out_of_range = payload()
        out_of_range.share = 1.5
        t.is_true(not research_schema.validate(SHAPE, out_of_range))

        local nan = payload()
        nan.share = 0 / 0
        t.is_true(not research_schema.validate(SHAPE, nan))

        local infinite = payload()
        infinite.count = math.huge
        t.is_true(not research_schema.validate(SHAPE, infinite))

        local fractional = payload()
        fractional.count = 1.5
        t.is_true(not research_schema.validate(SHAPE, fractional))
    end)

    t.it("rejects malformed arrays and maps", function()
        local sparse = payload()
        sparse.tags = { "a", nil, "c" }
        t.is_true(not research_schema.validate(SHAPE, sparse))

        local keyed = payload()
        keyed.tags = { a = 1 }
        t.is_true(not research_schema.validate(SHAPE, keyed))

        local bad_map_key = payload()
        bad_map_key.scores = { ["Home Team"] = 1 }
        t.is_true(not research_schema.validate(SHAPE, bad_map_key))
    end)

    t.it("rejects direct identifiers and raw paths in join keys", function()
        for _, value in ipairs({
            "participant@example.com",
            "https://example.com/p",
            "../secrets",
            "c:\\users\\p",
            "Participant01",
            -- Forward-slash paths carry a username and contain no "://" or "\\",
            -- so the charset has to reject the separator itself.
            "home/oscar/matches/save1.json",
            "c:/users/oscar/appdata/save.json",
            "/var/log/goliseo",
            "users:oscar",
        }) do
            local leaked = payload()
            leaked.id = value
            local ok, err = research_schema.validate(SHAPE, leaked)
            t.is_true(ok == nil, "expected " .. value .. " to be rejected")
            t.is_true(type(err) == "string")
        end
    end)

    t.it("accepts the slug grammar the contracts actually use", function()
        for _, value in ipairs({
            "pxi_enjoyment_addon.enjoyment",
            "playtest-agreement-v3",
            "spec-build",
            "en-gb",
            "share_0_to_1",
        }) do
            local slug = payload()
            slug.id = value
            t.is_true(research_schema.validate(SHAPE, slug), value .. " should be a legal id")
        end
    end)

    t.it("rejects malformed digests and control characters", function()
        local short_digest = payload()
        short_digest.digest = "abc"
        t.is_true(not research_schema.validate(SHAPE, short_digest))

        local upper_digest = payload()
        upper_digest.digest = "0123456789ABCDEF"
        t.is_true(not research_schema.validate(SHAPE, upper_digest))

        local control = payload()
        control.nested = { label = "bad\nlabel", weight = 1 }
        t.is_true(not research_schema.validate(SHAPE, control))
    end)
end)

t.describe("research schema canonical serialization", function()
    t.it("round-trips through encode/decode byte-for-byte", function()
        local value = payload()
        value.note = "free text is bounded, never a join key"
        value.nested = { label = "nested", weight = -0.5 }
        local bytes = assert(research_schema.encode(SHAPE, value))
        local decoded = assert(research_schema.decode(SHAPE, bytes))
        local re_encoded = assert(research_schema.encode(SHAPE, decoded))
        t.eq(re_encoded, bytes)
        t.eq(decoded.id, value.id)
        t.eq(decoded.note, value.note)
        t.eq(decoded.nested.weight, -0.5)
        t.eq(decoded.scores.home, 1)
        t.eq(#decoded.tags, 2)
    end)

    t.it("round-trips awkward finite numbers exactly", function()
        for _, number in ipairs({ 0.1, -0.1, 1 / 3, 2 ^ -30, 1e17, -1e-17, 0, 6.02e23 }) do
            local value = payload()
            value.nested = { label = "n", weight = number }
            local bytes = assert(research_schema.encode(SHAPE, value))
            local decoded = assert(research_schema.decode(SHAPE, bytes))
            t.eq(decoded.nested.weight, number, "number round-trip")
        end
    end)

    t.it("hashes independently of table insertion order", function()
        local left = payload()
        left.scores = {}
        left.scores.away = 2
        left.scores.home = 1
        local right = payload()
        right.scores = {}
        right.scores.home = 1
        right.scores.away = 2
        t.eq(
            assert(research_schema.content_hash(SHAPE, left)),
            assert(research_schema.content_hash(SHAPE, right))
        )
    end)

    t.it("changes the content hash when any field changes", function()
        local base = assert(research_schema.content_hash(SHAPE, payload()))
        local changed = payload()
        changed.count = 4
        t.is_true(base ~= assert(research_schema.content_hash(SHAPE, changed)))
        t.eq(#base, research_schema.HASH_LENGTH)
    end)

    t.it("refuses to encode an invalid payload", function()
        local broken = payload()
        broken.status = "unknown"
        local bytes, err = research_schema.encode(SHAPE, broken)
        t.is_true(bytes == nil)
        t.is_true(type(err) == "string")
    end)

    t.it("fails closed on truncated, trailing, and foreign payloads", function()
        local bytes = assert(research_schema.encode(SHAPE, payload()))
        t.is_true(not research_schema.decode(SHAPE, bytes:sub(1, #bytes - 4)))
        t.is_true(not research_schema.decode(SHAPE, bytes .. "s1:x;"))
        t.is_true(not research_schema.decode(SHAPE, "not-a-research-payload"))

        local other = research_schema.record("other_shape", {
            { name = "id", kind = "id" },
        })
        local foreign = assert(research_schema.encode(other, { id = "x" }))
        t.is_true(not research_schema.decode(SHAPE, foreign))
    end)

    t.it("stops with a migration diagnostic on a future serialization version", function()
        local bytes = assert(research_schema.encode(SHAPE, payload()))
        local future = bytes:gsub("^GCRS1;", "GCRS9;")
        local value, err = research_schema.decode(SHAPE, future)
        t.is_true(value == nil)
        t.is_true(type(err) == "string" and err:find("no migration", 1, true) ~= nil)
    end)
end)

t.describe("research schema helpers", function()
    t.it("gates unsupported schema versions with a migration diagnostic", function()
        local supported = { [2] = true, [3] = true }
        t.is_true(research_schema.accepts_version("trace", supported, 3, 2))
        local ok, err = research_schema.accepts_version("trace", supported, 3, 1)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("no migration", 1, true) ~= nil)
        t.is_true(not research_schema.accepts_version("trace", supported, 3, "2"))
    end)

    t.it("hashes ordered tuples unambiguously", function()
        local left = research_schema.tuple_hash("run/v1", { "a", "bc" })
        local right = research_schema.tuple_hash("run/v1", { "ab", "c" })
        t.is_true(left ~= right)
        t.eq(left, research_schema.tuple_hash("run/v1", { "a", "bc" }))
        t.is_true(left ~= research_schema.tuple_hash("run/v2", { "a", "bc" }))
        t.is_true(
            research_schema.tuple_hash("run/v1", { 1 })
                ~= research_schema.tuple_hash("run/v1", { "1" })
        )
    end)

    t.it("detects overlapping membership groups", function()
        t.is_true(research_schema.assert_disjoint("split", {
            train = { "p1", "p2" },
            test = { "p3" },
        }))
        local ok, err = research_schema.assert_disjoint("split", {
            train = { "p1", "p2" },
            test = { "p2" },
        })
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("p2", 1, true) ~= nil)
    end)
end)
