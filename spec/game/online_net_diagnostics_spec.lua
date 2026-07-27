local t = require("spec.support.runner")
local schema = require("game.online.diagnostics_schema")
local net_diagnostics = require("game.online.net_diagnostics")
local fixture = require("game.online.net_diagnostics_fixture")
local match_driver = require("game.online.match_driver")
local input_protocol = require("game.online.input_protocol")
local input_frame = require("sim.input_frame")
local transport_contract = require("game.transport.contract")
local desync_package = require("game.online.desync_package")
local live_slot = require("game.online.live_slot")

---@param options InputSampleOptions
---@return InputSample
local function sample(options)
    return assert(input_frame.new_sample(options))
end

---@param harness NetDiagnosticsFixtureHarness
---@return table
local function export_of(harness)
    return assert(net_diagnostics.export(harness.peers[1].recorder))
end

-- Every string anywhere in an artifact, so a redaction assertion can be about
-- the whole thing rather than about the handful of fields a test remembered.
---@param value any
---@param out string[]
local function collect_strings(value, out)
    if type(value) == "string" then
        out[#out + 1] = value
    elseif type(value) == "table" then
        for key, child in pairs(value) do
            collect_strings(key, out)
            collect_strings(child, out)
        end
    end
end

---@param artifact table
---@return string
local function all_text(artifact)
    ---@type string[]
    local parts = {}
    collect_strings(artifact, parts)
    table.sort(parts)
    return table.concat(parts, "\n")
end

-- Assert that no fragment of `poison` survives anywhere in `artifact`. Whole
-- strings are checked as well as the tokens inside them, so a partial scrub
-- would fail this rather than sneak through a whole-string comparison.
---@param artifact table
---@param poison string
local function assert_absent(artifact, poison)
    local text = all_text(artifact)
    t.is_true(text:find(poison, 1, true) == nil, poison .. " survived into the export")
    for token in poison:gmatch("[%w%.%-:@]+") do
        if #token >= 6 and token:find("[%.:@]") ~= nil then
            t.is_true(
                text:find(token, 1, true) == nil,
                "fragment " .. token .. " survived into the export"
            )
        end
    end
end

---@return TransportChannelDiagnostics
local function channel_diagnostics()
    return {
        state = "connected",
        outbound_depth = 0,
        inbound_depth = 0,
        buffered_amount = 0,
        sent = 1,
        received = 1,
        dropped_outbound = 0,
        dropped_inbound = 0,
    }
end

t.describe("diagnostics schema", function()
    t.it("refuses a canonical field that borrows wall-clock vocabulary", function()
        local ok = pcall(function()
            return schema.record("bad", "canonical", {
                { name = "confirmed_tick", kind = "integer" },
                { name = "rtt_ms", kind = "number" },
            })
        end)
        t.is_true(not ok, "a canonical section accepted an rtt field")
    end)

    t.it("refuses a runtime field that borrows simulation vocabulary", function()
        local ok = pcall(function()
            return schema.record("bad", "runtime", {
                { name = "monotonic_ms", kind = "number" },
                { name = "confirmed_tick", kind = "integer" },
            })
        end)
        t.is_true(not ok, "a runtime section accepted a confirmed tick")
    end)

    t.it("requires an anchor to bind both clocks and declare its error", function()
        local ok = pcall(function()
            return schema.record("bad_anchor", "anchor", {
                { name = "input_tick", kind = "integer" },
                { name = "monotonic_ms", kind = "number" },
            })
        end)
        t.is_true(not ok, "an anchor without a mapping error was accepted")

        local good = schema.record("good_anchor", "anchor", {
            { name = "input_tick", kind = "integer" },
            { name = "monotonic_ms", kind = "number" },
            { name = "mapping_error_ms", kind = "number" },
        })
        t.eq(good.kind, "record")
    end)

    -- Regression: the completeness check used to be gated on being the
    -- outermost call, so it never ran for the shape that actually ships --
    -- `EXPORT` is a domainless container and its `anchors` field is nested one
    -- level down. These build the anchor exactly the way `EXPORT` does.
    t.it("enforces anchor completeness on a nested anchor, as EXPORT nests it", function()
        -- The precondition this regression is about: the shipped export really
        -- is a domainless container with its anchor nested inside. If that ever
        -- stops being true, this test stops mirroring reality and should be
        -- rewritten rather than quietly kept passing.
        t.eq(net_diagnostics.EXPORT.domain, nil, "EXPORT is no longer a domainless container")
        local anchor_paths = 0
        for _, domain in pairs(schema.domains(net_diagnostics.EXPORT)) do
            if domain == "anchor" then
                anchor_paths = anchor_paths + 1
            end
        end
        t.is_true(anchor_paths > 0, "EXPORT declares no anchor field to enforce")

        ---@param fields DiagnosticsField[]
        ---@return boolean
        local function builds(fields)
            return (
                pcall(function()
                    return schema.record("nested_probe", nil, {
                        {
                            name = "session",
                            kind = "record",
                            domain = "identity",
                            fields = { { name = "peer_id", kind = "id" } },
                        },
                        {
                            name = "anchors",
                            kind = "array",
                            domain = "anchor",
                            element = { kind = "record", fields = fields },
                        },
                    })
                end)
            )
        end

        t.is_true(not builds({
            { name = "input_tick", kind = "integer" },
            { name = "monotonic_ms", kind = "number" },
        }), "a nested anchor without a mapping error was accepted")
        t.is_true(not builds({
            { name = "monotonic_ms", kind = "number" },
            { name = "mapping_error_ms", kind = "number" },
        }), "a nested anchor naming no simulation tick was accepted")
        t.is_true(
            builds({
                { name = "input_tick", kind = "integer" },
                { name = "mapping_error_ms", kind = "number" },
            }),
            "a nested anchor whose only wall-clock word is its own error term is still a binding"
        )
        t.is_true(
            builds({
                { name = "input_tick", kind = "integer" },
                { name = "monotonic_ms", kind = "number" },
                { name = "mapping_error_ms", kind = "number" },
            }),
            "a well-formed nested anchor was rejected"
        )
    end)

    -- The scope must be per-anchor-subtree. A sibling section naming a tick or a
    -- millisecond must not be able to satisfy the anchor's own requirements.
    t.it("does not let sibling sections satisfy an anchor's completeness", function()
        local ok = pcall(function()
            return schema.record("sibling_probe", nil, {
                {
                    name = "canonical",
                    kind = "record",
                    domain = "canonical",
                    fields = { { name = "confirmed_tick", kind = "integer" } },
                },
                {
                    name = "runtime",
                    kind = "record",
                    domain = "runtime",
                    fields = { { name = "mapping_error_ms", kind = "number" } },
                },
                {
                    name = "anchors",
                    kind = "array",
                    domain = "anchor",
                    element = {
                        kind = "record",
                        fields = { { name = "monotonic_ms", kind = "number" } },
                    },
                },
            })
        end)
        t.is_true(not ok, "an anchor borrowed completeness from its siblings")
    end)

    t.it("keeps the two vocabularies disjoint across the whole export shape", function()
        local domains = schema.domains(net_diagnostics.EXPORT)
        local canonical, runtime, anchor = 0, 0, 0
        for path, domain in pairs(domains) do
            local name = path:match("([^%.%[]+)%]?$") or path
            if domain == "canonical" or domain == "identity" then
                canonical = canonical + 1
                t.is_true(
                    not schema.is_wall_clock_name(name),
                    path .. " is deterministic evidence but reads as a wall-clock measurement"
                )
            elseif domain == "runtime" then
                runtime = runtime + 1
                t.is_true(
                    not schema.is_simulation_name(name),
                    path .. " is a runtime observation but reads as simulation truth"
                )
            elseif domain == "anchor" then
                anchor = anchor + 1
            end
        end
        t.is_true(canonical > 0 and runtime > 0 and anchor > 0)
    end)

    t.it("rejects direct identifiers, paths, and raw addresses as ids", function()
        local shape = schema.record("id_probe", "identity", { { name = "peer_id", kind = "id" } })
        for _, bad in ipairs({
            "player@example.com",
            "home/oscar/save.json",
            "c:/users/oscar",
            "../secrets",
            "192.168.1.14",
            "[fe80::1]",
        }) do
            local ok = schema.validate(shape, { peer_id = bad })
            t.is_true(not ok, bad .. " was accepted as a pseudonymous id")
        end
        t.is_true(schema.validate(shape, { peer_id = "guest_1" }))
    end)

    t.it("rejects non-finite and out-of-range values", function()
        local shape = schema.record("numbers", "canonical", {
            { name = "count", kind = "integer", min = 0 },
            { name = "ratio", kind = "number" },
        })
        t.is_true(not schema.validate(shape, { count = 1, ratio = 0 / 0 }))
        t.is_true(not schema.validate(shape, { count = 1, ratio = math.huge }))
        t.is_true(not schema.validate(shape, { count = -1, ratio = 0 }))
        t.is_true(not schema.validate(shape, { count = 1.5, ratio = 0 }))
        t.is_true(schema.validate(shape, { count = 1, ratio = 0.5 }))
    end)

    t.it("encodes maps in sorted key order, not table order", function()
        local shape = schema.record("m", "canonical", {
            { name = "live", kind = "map", element = { kind = "id" } },
        })
        local first =
            assert(schema.encode(shape, { live = { host = "home_1", guest_1 = "home_2" } }))
        local second =
            assert(schema.encode(shape, { live = { guest_1 = "home_2", host = "home_1" } }))
        t.eq(first, second)
    end)
end)

t.describe("net diagnostics collection", function()
    t.it("summarises a clean 2v2 run with both clocks kept apart", function()
        local harness = fixture.harness("2v2", { hash_interval_ticks = 6 })
        fixture.run(harness, 40, { [1] = sample({ move_x = 60 }) })
        local artifact = export_of(harness)

        t.eq(artifact.schema_version, net_diagnostics.SCHEMA_VERSION)
        t.eq(artifact.canonical.simulation.status, "active")
        t.eq(artifact.canonical.simulation.step_count, 40)
        t.eq(artifact.canonical.simulation.input_delay_ticks, input_protocol.FAIRNESS_DELAY_TICKS)
        t.is_true(artifact.canonical.simulation.confirmed_output_tick >= 0)
        t.is_true(artifact.canonical.simulation.checkpoint_count > 0)
        t.is_true(#artifact.canonical.checkpoints > 0)
        t.is_true(artifact.canonical.delivery.sent > 0)
        t.is_true(artifact.canonical.delivery.arrived > 0)

        -- Runtime is present and separate.
        t.is_true(artifact.runtime.star ~= nil)
        t.is_true(#artifact.runtime.peers > 0)
        t.is_true(#artifact.runtime.latency > 0)
        t.is_true(#artifact.anchors > 0)
        for _, anchor in ipairs(artifact.anchors) do
            t.is_true(anchor.mapping_error_ms > 0, "an anchor claimed a perfect clock mapping")
        end
    end)

    -- The runtime star/peers section was previously only asserted to be
    -- non-empty. It is the half of the artifact carrying transport counters and
    -- free text, so it gets its own coverage.
    t.it("folds star and per-channel transport counters into the runtime section", function()
        local harness = fixture.harness("2v2", { hash_interval_ticks = 6 })
        fixture.run(harness, 25)
        local artifact = export_of(harness)
        local star = assert(artifact.runtime.star)
        t.eq(star.role, "host")
        t.eq(star.state, "connected")
        t.is_true(star.sent > 0 and star.received > 0)
        t.eq(star.dropped_outbound, 0)
        t.eq(star.dropped_inbound, 0)
        t.eq(star.malformed, 0)
        t.eq(star.overflow, 0)
        t.eq(star.peer_count, #harness.peers - 1)
        t.eq(star.last_error, nil, "a clean run reported a transport error")

        t.eq(#artifact.runtime.peers, #harness.peers - 1)
        for _, peer in ipairs(artifact.runtime.peers) do
            t.eq(peer.state, "connected")
            t.eq(peer.sequence_gaps, 0)
            t.eq(peer.backpressure, 0)
            t.eq(peer.malformed, 0)
            for _, channel in ipairs({ peer.control, peer.input }) do
                t.is_true(channel.buffered_amount >= 0)
                t.is_true(channel.outbound_depth >= 0 and channel.inbound_depth >= 0)
                t.is_true(channel.dropped_outbound == 0 and channel.dropped_inbound == 0)
            end
            t.is_true(peer.input.sent > 0, "no input traffic was attributed to a peer channel")
        end

        -- The tap records transport lifecycle events, in order.
        t.is_true(#artifact.runtime.events > 0, "no runtime event was ever recorded")
        for index = 2, #artifact.runtime.events do
            t.is_true(
                artifact.runtime.events[index].ordinal > artifact.runtime.events[index - 1].ordinal,
                "runtime event ordinals are not monotonic"
            )
        end
    end)

    t.it("records the sample, send, arrival, and apply lifecycle of a packet", function()
        local harness = fixture.harness("2v2")
        fixture.run(harness, 20)
        local artifact = export_of(harness)
        local arrived = 0
        for _, packet in ipairs(artifact.canonical.delivery.packets) do
            t.eq(
                packet.authority_input_tick,
                harness.session.freeze.first_input_tick + packet.send_transport_tick,
                "authority tick did not follow the stated delay policy"
            )
            t.eq(packet.sample_step, packet.send_transport_tick - match_driver.DELAY_TICKS)
            if packet.disposition == "arrived" then
                arrived = arrived + 1
                t.is_true(packet.arrival_transport_tick ~= nil)
                t.is_true(packet.apply_input_tick ~= nil, "an arrival was never attributed a step")
                t.is_true(
                    packet.authority_input_tick > assert(packet.apply_input_tick),
                    "authority landed at or behind the tick already simulated"
                )
            end
        end
        t.is_true(arrived > 0, "a clean run recorded no arrivals at all")
    end)

    t.it("counts rollback, resimulation, and event reconciliation under impairment", function()
        local harness = fixture.harness("2v2", { hash_interval_ticks = 6 })
        fixture.run_bursty(harness, 60, 6, {
            [1] = sample({ move_x = 90 }),
            [2] = sample({ move_y = -70 }),
        })
        local artifact = export_of(harness)
        local simulation = artifact.canonical.simulation
        t.eq(simulation.status, "active")
        t.is_true(simulation.rollback_count > 0, "bursty delivery produced no rollback")
        t.is_true(simulation.resimulated_ticks > 0, "a rollback resimulated nothing")
        t.is_true(simulation.max_rollback_depth > 0)

        local worst = assert(artifact.canonical.worst_correction, "no worst correction recorded")
        t.is_true(worst.depth >= 1)
        t.is_true(worst.through_tick >= worst.causal_tick)
        t.is_true(worst.resimulated_ticks > 0)

        local events = artifact.canonical.events
        t.is_true(events.added > 0, "no tick was ever emitted for the first time")
        t.eq(
            events.resimulated_tick_count,
            events.unchanged + events.replaced + events.revoked,
            "resimulated ticks and reconciliation dispositions disagree"
        )

        -- "Convergent" has to mean something. Two peers' *exports* must agree on
        -- every boundary they both hashed, and on the live slot each human held
        -- there, which is the whole point of carrying the live map next to the
        -- hash rather than looking it up later.
        local other = assert(net_diagnostics.export(harness.peers[2].recorder))
        ---@type table<integer, table>
        local by_tick = {}
        for _, checkpoint in ipairs(other.canonical.checkpoints) do
            by_tick[checkpoint.tick] = checkpoint
        end
        local compared = 0
        for _, checkpoint in ipairs(artifact.canonical.checkpoints) do
            local mine = by_tick[checkpoint.tick]
            if mine ~= nil then
                compared = compared + 1
                t.eq(mine.hash, checkpoint.hash, "boundary " .. tostring(checkpoint.tick))
                for producer_id, slot in pairs(checkpoint.live) do
                    t.eq(mine.live[producer_id], slot, "live slot for " .. producer_id)
                end
            end
        end
        t.is_true(compared > 0, "two peers shared no confirmed boundary to compare")
    end)

    t.it("keeps canonical evidence byte-stable while runtime observation moves", function()
        ---@return NetDiagnosticsFixtureHarness
        local function build(rtt_bias_ms, clock_step_ms)
            local harness = fixture.harness("2v2", {
                hash_interval_ticks = 6,
                rtt_bias_ms = rtt_bias_ms,
                clock_step_ms = clock_step_ms,
            })
            fixture.run_bursty(harness, 45, 5, { [1] = sample({ move_x = 45 }) })
            return harness
        end

        local reference = build(0, fixture.DEFAULT_CLOCK_STEP_MS)
        local same_schedule_other_clock = build(37, fixture.DEFAULT_CLOCK_STEP_MS * 2)

        -- Deterministic half: byte-identical, and claimed as such.
        t.eq(
            assert(net_diagnostics.canonical_bytes(same_schedule_other_clock.peers[1].recorder)),
            assert(net_diagnostics.canonical_bytes(reference.peers[1].recorder)),
            "canonical evidence changed when only the clock changed"
        )

        -- Runtime half: not claimed to be identical, only to obey invariants.
        local a = assert(net_diagnostics.export(reference.peers[1].recorder))
        local b = assert(net_diagnostics.export(same_schedule_other_clock.peers[1].recorder))
        t.is_true(
            assert(net_diagnostics.digest(reference.peers[1].recorder))
                ~= assert(net_diagnostics.digest(same_schedule_other_clock.peers[1].recorder)),
            "the full export claimed byte-identity across two different clocks"
        )
        t.eq(#a.runtime.latency, #b.runtime.latency)
        for index, row in ipairs(a.runtime.latency) do
            local other = b.runtime.latency[index]
            t.eq(other.peer_id, row.peer_id)
            t.eq(other.sample_count, row.sample_count, "sample counts are a schedule, not a clock")
            t.is_true(
                assert(other.rtt_ms_min) > assert(row.rtt_ms_min),
                "the bias did not move rtt"
            )
            t.is_true(assert(row.rtt_ms_min) <= assert(row.rtt_ms_max))
            t.is_true(assert(row.monotonic_ms_first) <= assert(row.monotonic_ms_last))
        end
    end)

    t.it("reproduces the canonical projection across two identical runs", function()
        ---@return string
        local function digest()
            local harness = fixture.harness("1v1", { hash_interval_ticks = 5 })
            fixture.run_bursty(harness, 40, 4, { [1] = sample({ move_x = 30 }) })
            return assert(net_diagnostics.canonical_digest(harness.peers[1].recorder))
        end
        t.eq(digest(), digest())
    end)

    t.it("holds the export behind an explicit opt-in", function()
        local harness = fixture.harness("2v2", { export_opt_in = false })
        fixture.run(harness, 6)
        local recorder = harness.peers[1].recorder
        local artifact, err = net_diagnostics.export(recorder)
        t.eq(artifact, nil)
        t.is_true(err ~= nil and err:find("opted into") ~= nil)
        -- The summary is local-only and never gated: it does not leave the machine.
        t.is_true(#net_diagnostics.summary(recorder) > 0)
        net_diagnostics.opt_in_export(recorder)
        t.is_true(net_diagnostics.export(recorder) ~= nil)
    end)
end)

t.describe("net diagnostics privacy", function()
    t.it("keeps signalling blobs out of the export entirely", function()
        local harness = fixture.harness("2v2", { hash_interval_ticks = 6 })
        fixture.run(harness, 10)
        local host = harness.peers[1]
        -- A blob shaped like the real thing: SDP with ICE credentials and a
        -- candidate line carrying a host address.
        local secret = table.concat({
            "v=0",
            "a=ice-ufrag:F7gI",
            "a=ice-pwd:x9Yb3kQwPl2sVn8tRc4mHd",
            "a=candidate:1 1 udp 2130706431 192.168.1.14 54321 typ host",
            "a=fingerprint:sha-256 AB:CD:EF",
        }, "\r\n")
        net_diagnostics.record_signal(host.recorder, "guest_1", "inbound", secret)

        local artifact = assert(net_diagnostics.export(host.recorder))
        t.eq(#artifact.runtime.signals, 1)
        local signal = artifact.runtime.signals[1]
        t.eq(signal.content, schema.REDACTED)
        t.eq(signal.byte_length, #secret)
        t.eq(signal.direction, "inbound")

        local text = all_text(artifact)
        for _, forbidden in ipairs({
            "ice-ufrag",
            "ice-pwd",
            "x9Yb3kQwPl2sVn8tRc4mHd",
            "candidate",
            "192.168.1.14",
            "fingerprint",
            "v=0",
        }) do
            t.is_true(
                text:find(forbidden, 1, true) == nil,
                forbidden .. " survived into the diagnostic export"
            )
        end
        -- The encoded artifact is what actually gets pasted somewhere.
        local bytes = assert(net_diagnostics.encode(host.recorder))
        t.is_true(bytes:find("192.168.1.14", 1, true) == nil)
        t.is_true(bytes:find("ice-pwd", 1, true) == nil)
    end)

    -- Regression: `last_error` and event `detail` are free text produced by
    -- `String(error)` on a WebRTC/DOM exception and by the JS bridge itself.
    -- Nothing about that text is a controlled vocabulary, and once STUN/TURN is
    -- wired in it can carry a candidate line or an address verbatim. It used to
    -- pass through length truncation only.
    local POISON = {
        "ICE failed for candidate 192.168.1.14:54321 typ host",
        "connection reset by [fe80::1a2b:3c4d:5e6f:7890]",
        "bridge error: a=ice-pwd:x9Yb3kQwPl2sVn8tRc4mHd",
        "failed to apply sdp offer",
        "signalling failed for turn:turn.example.com:3478",
        "peer someone@example.com is unreachable",
    }

    t.it("redacts sensitive free text arriving as a transport error", function()
        local harness = fixture.harness("2v2")
        fixture.run(harness, 6)
        local recorder = harness.peers[1].recorder

        for _, poison in ipairs(POISON) do
            t.is_true(net_diagnostics.record_transport(recorder, {
                role = "host",
                state = "connected",
                capacity = 7,
                peer_count = 1,
                queue_limit = 64,
                buffered_amount_limit = 65536,
                event_depth = 0,
                sent = 1,
                received = 1,
                dropped_outbound = 0,
                dropped_inbound = 0,
                malformed = 0,
                unsupported_version = 0,
                overflow = 0,
                backpressure = 0,
                last_error = poison,
                peers = {
                    {
                        peer_id = "guest_1",
                        slot = 1,
                        state = "connected",
                        ice_state = "connected",
                        control = channel_diagnostics(),
                        input = channel_diagnostics(),
                        sequence_gaps = 0,
                        backpressure = 0,
                        malformed = 0,
                        last_error = poison,
                    },
                },
            }))
            local artifact = assert(net_diagnostics.export(recorder))
            t.eq(artifact.runtime.star.last_error, schema.REDACTED)
            t.eq(artifact.runtime.peers[1].last_error, schema.REDACTED)
            assert_absent(artifact, poison)
        end
    end)

    t.it("redacts sensitive free text arriving as a runtime event detail", function()
        local harness = fixture.harness("2v2")
        fixture.run(harness, 6)
        local recorder = harness.peers[1].recorder
        for index, poison in ipairs(POISON) do
            t.is_true(net_diagnostics.record_event(recorder, {
                kind = "peer_error",
                monotonic_ms = index * 10,
                peer_id = "guest_1",
                code = "signal_error",
                detail = poison,
            }))
        end
        local artifact = assert(net_diagnostics.export(recorder))
        local redacted = 0
        for _, event in ipairs(artifact.runtime.events) do
            if event.detail == schema.REDACTED then
                redacted = redacted + 1
            end
        end
        t.eq(redacted, #POISON, "a poisoned event detail survived unredacted")
        for _, poison in ipairs(POISON) do
            assert_absent(artifact, poison)
        end
    end)

    t.it("keeps benign transport error text readable", function()
        local harness = fixture.harness("2v2")
        fixture.run(harness, 6)
        local recorder = harness.peers[1].recorder
        t.is_true(net_diagnostics.record_event(recorder, {
            kind = "star_error",
            monotonic_ms = 5,
            code = "overflow",
            detail = "outbound queue reached its limit of 64 messages",
        }))
        local artifact = assert(net_diagnostics.export(recorder))
        local last = artifact.runtime.events[#artifact.runtime.events]
        t.eq(last.detail, "outbound queue reached its limit of 64 messages")
        t.eq(last.code, "overflow")
    end)

    t.it("stops poisoned free text reaching a desync package", function()
        local harness = fixture.harness("2v2", { hash_interval_ticks = 5 })
        fixture.run(harness, 30)
        local host = harness.peers[1]
        local poison = "ICE failed for candidate 192.168.1.14:54321 typ host"
        t.is_true(net_diagnostics.record_event(host.recorder, {
            kind = "peer_error",
            monotonic_ms = 12,
            peer_id = "guest_1",
            detail = poison,
        }))
        local checkpoints = match_driver.checkpoints(host.driver)
        local built = assert(desync_package.build({
            recorder = host.recorder,
            manifest = harness.session.manifest,
            freeze = harness.session.freeze,
            peer_id = host.peer_id,
            remote_peer_id = harness.session.guest_peer_ids[1],
            agreed_boundary_tick = checkpoints[1].tick,
            agreed_boundary_hash = checkpoints[1].hash,
            divergence_tick = checkpoints[#checkpoints].tick,
            local_hash = checkpoints[#checkpoints].hash,
            remote_hash = "deadbeefdeadbeef",
            input_wires = host.tap:wires(),
        }))
        -- The package embeds runtime events verbatim, so redaction has to have
        -- happened on the way in rather than on the way out.
        assert_absent(built, poison)
        t.is_true(assert(desync_package.encode(built)):find("192.168.1.14", 1, true) == nil)
    end)

    t.it("refuses to store a direct identifier as a peer id", function()
        local harness = fixture.harness("2v2")
        fixture.run(harness, 4)
        local recorder = harness.peers[1].recorder
        net_diagnostics.record_runtime_sample(recorder, {
            peer_id = "someone@example.com",
            monotonic_ms = 10,
            rtt_ms = 20,
        })
        local artifact, err = net_diagnostics.export(recorder)
        t.eq(artifact, nil, "an email address reached a validated export")
        t.is_true(err ~= nil and err:find("pseudonymous") ~= nil)
    end)

    t.it("carries no participant or playtest payload fields at all", function()
        local paths = schema.domains(net_diagnostics.EXPORT)
        for path in pairs(paths) do
            local lowered = path:lower()
            for _, forbidden in ipairs({
                "participant",
                "consent",
                "research",
                "clipboard",
                "address",
                "sdp",
                "candidate",
                "email",
            }) do
                t.is_true(
                    lowered:find(forbidden, 1, true) == nil,
                    path .. " names a field this schema must never carry"
                )
            end
        end
    end)
end)

t.describe("net diagnostics bounds", function()
    t.it("marks truncation explicitly instead of silently shrinking", function()
        local limits = {
            checkpoints = 2,
            packets = 4,
            control = 2,
            events = 2,
            signals = 1,
            mismatches = 2,
            anchors = 2,
            latency_peers = 2,
        }
        local harness = fixture.harness("2v2", { hash_interval_ticks = 4, limits = limits })
        fixture.run(harness, 40)
        local artifact = export_of(harness)

        t.eq(artifact.collection.retention.packets, schema.TRUNCATED)
        t.is_true(artifact.collection.dropped.packets > 0)
        t.is_true(#artifact.canonical.delivery.packets <= limits.packets)
        t.is_true(#artifact.canonical.checkpoints <= limits.checkpoints)
        t.is_true(#artifact.anchors <= limits.anchors)
        -- The count of published checkpoints is not the count retained, and the
        -- artifact says both rather than quietly conflating them.
        t.is_true(artifact.canonical.simulation.checkpoint_count >= #artifact.canonical.checkpoints)

        local summary = net_diagnostics.summary(harness.peers[1].recorder)
        local mentions = false
        for _, line in ipairs(summary) do
            mentions = mentions or line:find("truncated", 1, true) ~= nil
        end
        t.is_true(mentions, "a truncated ring did not show up in the human summary")
    end)

    t.it("keeps the oldest mismatches and the newest packets", function()
        local harness = fixture.harness("2v2", {
            limits = {
                checkpoints = 8,
                packets = 8,
                control = 8,
                events = 8,
                signals = 4,
                mismatches = 2,
                anchors = 4,
                latency_peers = 4,
            },
        })
        fixture.run(harness, 8)
        local recorder = harness.peers[1].recorder
        for index = 1, 5 do
            net_diagnostics.record_mismatch(recorder, {
                tick = index * 10,
                peer_id = "guest_1",
                local_hash = ("%016x"):format(index),
                remote_hash = ("%016x"):format(index + 100),
            })
        end
        local artifact = assert(net_diagnostics.export(recorder))
        t.eq(#artifact.canonical.mismatches, 2)
        -- The first divergence is the causal one, so it is the one kept.
        t.eq(artifact.canonical.mismatches[1].tick, 10)
        t.eq(artifact.canonical.mismatches[2].tick, 20)
        t.eq(artifact.collection.retention.mismatches, schema.TRUNCATED)
    end)

    t.it("counts rejected values instead of storing them", function()
        local harness = fixture.harness("2v2")
        fixture.run(harness, 4)
        local recorder = harness.peers[1].recorder
        t.eq(
            net_diagnostics.record_anchor(recorder, {
                input_tick = 0,
                monotonic_ms = 0 / 0,
                mapping_error_ms = 1,
            }),
            nil
        )
        t.eq(
            net_diagnostics.record_anchor(recorder, {
                input_tick = 0,
                monotonic_ms = 10,
                mapping_error_ms = -1,
            }),
            nil
        )
        t.eq(
            net_diagnostics.record_runtime_sample(recorder, {
                peer_id = "guest_1",
                monotonic_ms = 10,
                rtt_ms = math.huge,
            }),
            nil
        )
        local artifact = assert(net_diagnostics.export(recorder))
        t.eq(artifact.collection.rejected_values, 3)
    end)
end)

t.describe("net diagnostics failure fixtures", function()
    ---@param harness NetDiagnosticsFixtureHarness
    ---@param index integer
    ---@return table
    local function typed_summary(harness, index)
        local artifact = assert(net_diagnostics.export(harness.peers[index].recorder))
        return artifact.canonical.simulation
    end

    t.it("types an ownership violation", function()
        local harness = fixture.harness("2v2", { hash_interval_ticks = 6 })
        fixture.run(harness, 4)
        local guest_id = harness.session.guest_peer_ids[1]
        local guest = fixture.peer(harness, guest_id)
        -- away_1 (slot 5) belongs to another human, never to this guest.
        local forged = fixture.forged_bundle(harness, guest_id, 5, 9000, 8, 0)
        assert(guest.tap:send(transport_contract.HOST_PEER_ID, "input", forged))
        harness.session.host_transport:pump()
        fixture.advance(harness)

        local simulation = typed_summary(harness, 1)
        t.eq(simulation.status, "ownership_violation")
        t.eq(simulation.terminal_status, "ownership_violation")
        t.eq(simulation.terminal_failure, "input_channel")
        t.is_true(simulation.terminal_detail ~= nil)
    end)

    t.it("types an authority conflict", function()
        local harness = fixture.harness("2v2", { hash_interval_ticks = 6 })
        fixture.run(harness, 6)
        local guest_id = harness.session.guest_peer_ids[1]
        local guest = fixture.peer(harness, guest_id)
        local owned = harness.session.freeze.owned[guest_id]
        local slot_index = live_slot.slot_index(assert(owned)[1])

        ---@param edges integer
        local function send(edges)
            assert(
                guest.tap:send(
                    transport_contract.HOST_PEER_ID,
                    "input",
                    fixture.forged_bundle(harness, guest_id, slot_index, 4242, 9, edges)
                )
            )
        end

        -- One sender sequence with byte-identical authority is idempotent, so
        -- this establishes the rows rather than failing on them.
        send(0)
        send(0)
        harness.session.host_transport:pump()
        fixture.advance(harness)
        t.eq(typed_summary(harness, 1).status, "active")

        -- The same identity with different bytes conflicts with what history
        -- already holds.
        send(0)
        send(input_frame.EDGE_BITS.dash)
        harness.session.host_transport:pump()
        fixture.advance(harness)

        local simulation = typed_summary(harness, 1)
        t.eq(simulation.status, "authority_conflict")
        t.eq(simulation.terminal_failure, "input_channel")
    end)

    t.it("types over-window input and names the tick it was attributed to", function()
        local harness = fixture.harness("4v4", { hash_interval_ticks = 10 })
        fixture.run_bursty(harness, 60, 50)
        local terminal = 0
        for index in ipairs(harness.peers) do
            local simulation = typed_summary(harness, index)
            -- Since #241 the driver catches this on confirmation liveness at the
            -- step the tick becomes unconfirmable, rather than on the arrival
            -- that used to reveal it. The typed reason is sharper; the causal
            -- attribution the recorder carries -- the tick, and the failure the
            -- coordinator is told -- is unchanged.
            if simulation.status == "confirmation_stalled" then
                terminal = terminal + 1
                t.eq(simulation.terminal_failure, "late_input")
                t.is_true(simulation.terminal_tick ~= nil)
                t.is_true(simulation.late_input_tick ~= nil)
                t.is_true(
                    assert(simulation.late_input_tick) < simulation.retained_floor_tick + 1,
                    "a late input was attributed above the retained floor"
                )
            end
        end
        t.is_true(terminal > 0, "an over-window burst terminated nobody")
    end)

    t.it("types hash divergence and keeps the first divergent boundary", function()
        local harness = fixture.harness("2v2", { hash_interval_ticks = 5 })
        fixture.run(harness, 30)
        local host = harness.peers[1]
        local checkpoints = match_driver.checkpoints(host.driver)
        t.is_true(#checkpoints > 0, "no checkpoint was published to disagree about")
        local target = checkpoints[1]
        local remote = "deadbeefdeadbeef"
        for _ = 1, match_driver.MAX_HASH_MISMATCHES do
            local matched = match_driver.observe_checkpoint(host.driver, target.tick, remote)
            t.is_true(not matched)
            net_diagnostics.record_mismatch(host.recorder, {
                tick = target.tick,
                peer_id = harness.session.guest_peer_ids[1],
                local_hash = target.hash,
                remote_hash = remote,
                first_difference_path = "state.ball.pos.x",
            })
        end
        fixture.advance(harness)

        local artifact = assert(net_diagnostics.export(host.recorder))
        t.eq(artifact.canonical.simulation.status, "hash_mismatch")
        t.eq(artifact.canonical.simulation.terminal_failure, "desync")
        local mismatch = assert(artifact.canonical.mismatches[1])
        t.eq(mismatch.tick, target.tick)
        t.eq(mismatch.local_hash, target.hash)
        t.eq(mismatch.remote_hash, remote)
        t.eq(mismatch.first_difference_path, "state.ball.pos.x")
    end)

    t.it("types a guest disconnect and a host loss", function()
        local harness = fixture.harness("2v2", { hash_interval_ticks = 6 })
        fixture.run(harness, 8)
        local guest_id = harness.session.guest_peer_ids[1]
        assert(harness.session.host_transport:close_peer(guest_id, "peer_left"))
        harness.session.host_transport:pump()
        fixture.advance(harness)
        t.eq(typed_summary(harness, 1).status, "transport_lost")

        local other = fixture.harness("2v2", { hash_interval_ticks = 6 })
        fixture.run(other, 8)
        assert(other.session.host_transport:shutdown())
        fixture.advance(other)
        for index = 2, #other.peers do
            t.eq(typed_summary(other, index).status, "transport_lost")
        end
    end)

    t.it("reports orphaned links and residual queues at teardown", function()
        local harness = fixture.harness("2v2", { hash_interval_ticks = 6 })
        fixture.run(harness, 10)
        local host = harness.peers[1]
        assert(host.tap:shutdown())
        local artifact = assert(net_diagnostics.export(host.recorder))
        local teardown = assert(artifact.runtime.teardown, "shutdown recorded no teardown")
        t.is_true(teardown.requested)
        t.eq(#teardown.orphaned_peers, 0)
        t.eq(teardown.residual_outbound, 0)
        t.eq(teardown.residual_inbound, 0)
        t.is_true(teardown.complete, "a clean shutdown was not reported complete")
    end)

    t.it("reports an incomplete teardown as incomplete", function()
        local harness = fixture.harness("2v2")
        fixture.run(harness, 6)
        local recorder = harness.peers[1].recorder
        net_diagnostics.record_teardown(recorder, {
            requested = true,
            closed_peers = 1,
            orphaned_peers = { "guest_1" },
            residual_outbound = 3,
            residual_inbound = 0,
        })
        local artifact = assert(net_diagnostics.export(recorder))
        local teardown = assert(artifact.runtime.teardown)
        t.is_true(not teardown.complete)
        t.eq(teardown.orphaned_peers[1], "guest_1")
    end)

    t.it("records accepted control traffic in order", function()
        local harness = fixture.harness("2v2", { hash_interval_ticks = 6 })
        fixture.run(harness, 6)
        local recorder = harness.peers[1].recorder
        for index = 1, 3 do
            local addressed = fixture.hash_report(
                harness,
                harness.session.guest_peer_ids[1],
                index,
                index * 5,
                ("%016x"):format(index)
            )
            t.is_true(net_diagnostics.record_control(recorder, addressed))
        end
        local artifact = assert(net_diagnostics.export(recorder))
        t.eq(#artifact.canonical.control, 3)
        for index = 2, 3 do
            t.is_true(
                artifact.canonical.control[index].ordinal
                    > artifact.canonical.control[index - 1].ordinal,
                "control ordinals are not monotonic"
            )
            t.eq(artifact.canonical.control[index].kind, "hash_report")
        end
    end)
end)
