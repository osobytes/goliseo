-- `game.transport.fake_relay` and the sequencer-less probe behind #245.
--
-- Two halves, and they answer different questions.
--
-- The first half is the adapter: does a relay satisfy `StarTransportAdapter`,
-- and does it differ from `fake_star` in exactly the way the OMP-4 topology
-- decision says it does — one upload per `broadcast`, no privileged member, no
-- manual signaling, and a room that forwards opaque bytes without parsing them.
--
-- The second half is the part worth having. The decision's migration note says a
-- relay adapter "becomes a third implementation ... and the driver above it does
-- not change". These rows are what happens when the *sequencer* is removed as
-- well as the hub, and each one names the module and the mechanism that refuses.
-- They are recorded as pinned observations, not fixed:
-- `docs/online/relay_topology_probe.md` is the write-up.

local t = require("spec.support.runner")
local coordinator = require("game.online.coordinator")
local contract = require("game.transport.contract")
local fault_harness = require("game.online.fault_harness")
local input_frame = require("sim.input_frame")
local input_protocol = require("game.online.input_protocol")
local live_slot = require("game.online.live_slot")
local match_driver = require("game.online.match_driver")
local match_manifest = require("game.online.match_manifest")
local protocol_fixture = require("game.online.protocol_fixture")
local transport = require("game.transport")

local PROBE_SESSION = "session_relay_probe01"
local PROBE_MANIFEST = string.rep("a", 16)

---@param seq integer
---@param payload string?
---@return TransportMessage
local function control(seq, payload)
    return assert(contract.new({ type = "event", seq = seq, payload = payload or "" }))
end

---@param seq integer
---@param tick integer
---@param payload string?
---@return TransportMessage
local function input(seq, tick, payload)
    return assert(contract.new({ type = "input", seq = seq, tick = tick, payload = payload or "" }))
end

-- A real, encodable single-slot guest bundle: the exact shape every client
-- publishes for itself under a relay, so a byte figure taken from it is the
-- protocol's own and not an invented packet.
---@param peer_id string
---@param slot_index integer
---@param transport_tick integer
---@return TransportMessage
local function guest_bundle(peer_id, slot_index, transport_tick)
    ---@type InputAuthorityRow[]
    local rows = {}
    for tick = 0, input_protocol.HISTORY_ROWS do
        rows[#rows + 1] = {
            tick = transport_tick + tick,
            slot_index = slot_index,
            sample = assert(input_frame.new_sample({ edges = 0 })),
        }
    end
    local packet = assert(input_protocol.new_guest({
        session_id = PROBE_SESSION,
        manifest_id = PROBE_MANIFEST,
        sender_id = peer_id,
        sequence = transport_tick,
        transport_tick = transport_tick,
        first_input_tick = 0,
        rows = rows,
    }))
    return assert(contract.new({
        type = "input",
        seq = packet.sequence,
        tick = packet.transport_tick,
        payload = assert(input_protocol.encode(packet)),
    }))
end

---@param index integer
---@return string
local function member_id(index)
    return index == 1 and "host" or ("guest_" .. tostring(index - 1))
end

---@param count integer
---@return FakeRelayRoom, FakeRelayTransport[]
local function build_room(count)
    local room = transport.fake_relay_room()
    ---@type FakeRelayTransport[]
    local members = {}
    for index = 1, count do
        local endpoint = transport.fake_relay({
            role = index == 1 and "host" or "guest",
            peer_id = member_id(index),
            room = room,
        })
        assert(endpoint:initialize())
        members[index] = endpoint
    end
    return room, members
end

t.describe("relay transport adapter", function()
    t.it("joins every member to every other without a privileged opener", function()
        local _, members = build_room(8)
        for index, endpoint in ipairs(members) do
            t.eq(#endpoint:peer_ids(), 7, member_id(index) .. " should hold seven links")
            t.eq(endpoint:state(), "connected")
            for other = 1, 8 do
                if other ~= index then
                    t.eq(endpoint:peer_state(member_id(other)), "connected")
                end
            end
        end
    end)

    t.it("delivers an addressed envelope and never echoes a member's own line", function()
        local _, members = build_room(3)
        assert(members[1]:broadcast("input", input(1, 40, "authority")))
        members[1]:pump()
        t.eq(#members[1]:poll_batch(16), 0, "a relay never returns a member's own rows")
        for index = 2, 3 do
            local batch = members[index]:poll_batch(16)
            t.eq(#batch, 1)
            t.eq(batch[1].peer_id, "host")
            t.eq(batch[1].channel, "input")
            t.eq(batch[1].message.payload, "authority")
        end
    end)

    t.it("charges one upload per broadcast where the star charges one per link", function()
        local _, members = build_room(8)
        local message = input(1, 40, string.rep("x", 200))
        assert(members[1]:broadcast("input", message))
        members[1]:pump()
        local relay = members[1]:wire_counters()

        local star = transport.fake_star({ role = "host" })
        assert(star:initialize())
        for index = 2, 8 do
            local guest = transport.fake_star({ role = "guest", peer_id = member_id(index) })
            assert(guest:initialize())
            assert(star:open_peer(member_id(index)))
            assert(star:link(guest))
        end
        assert(star:broadcast("input", message))
        local hub = star:wire_counters()

        t.eq(relay.uplink_units, 1, "one relay upload")
        t.eq(hub.uplink_units, 7, "one star upload per guest link")
        t.eq(hub.uplink_bytes, relay.uplink_bytes * 7, "the star pays the fan-out seven times")
        -- The room forwards the same bytes to seven members, so total traffic is
        -- unchanged; only *where* it is paid moves.
        t.eq(hub.input_uplink_bytes, relay.input_uplink_bytes * 7)
    end)

    t.it("lets any member address any other, which the star forbids", function()
        local _, members = build_room(3)
        t.is_true(members[2]:send("guest_2", "control", control(0, "hello")) == true)

        local star = transport.fake_star({ role = "guest", peer_id = "guest_1" })
        assert(star:initialize())
        local _, _, code = star:send("guest_2", "control", control(0, "hello"))
        t.eq(code, "role_forbidden", "a star guest may only address the host")
    end)

    t.it("forwards a payload the room cannot possibly parse", function()
        local room, members = build_room(2)
        -- Deliberately not an input packet, and carrying the framing separators
        -- the room concatenates with. If the room parsed anything, this breaks.
        local payload = "not|an|input|packet\nwith a newline and % escapes"
        assert(members[1]:send("guest_1", "input", input(7, 12, payload)))
        members[1]:pump()
        local batch = members[2]:poll_batch(16)
        t.eq(#batch, 1)
        t.eq(batch[1].message.payload, payload, "the relay returns the bytes it was handed")
        t.eq(room.counters.lines, 1)
        t.eq(room.counters.dropped, 0)
    end)

    t.it("refuses manual signaling instead of faking it", function()
        local _, members = build_room(2)
        local _, _, offer = members[1]:request_offer("guest_1")
        t.eq(offer, "signal_error")
        local _, _, accepted = members[2]:accept_offer("offer:1")
        t.eq(accepted, "signal_error")
        local _, _, answered = members[1]:accept_answer("guest_1", "answer:offer:1")
        t.eq(answered, "signal_error")
        t.eq(members[1]:take_signal("guest_1"), nil)
    end)

    t.it("reports diagnostics the contract encoder round-trips", function()
        local _, members = build_room(3)
        assert(members[1]:send("guest_1", "control", control(0, "hello")))
        local original = members[1]:diagnostics()
        local decoded =
            assert(contract.decode_star_diagnostics(contract.encode_star_diagnostics(original)))
        t.eq(decoded.peer_count, 2)
        t.eq(#decoded.peers, 2)
        t.eq(decoded.peers[1].peer_id, "guest_1")
        t.eq(decoded.peers[1].slot, 1)
        t.eq(decoded.peers[1].state, "connected")
        t.eq(decoded.peers[1].control.outbound_depth, 1, "the addressed unit shows on that link")
        t.eq(decoded.peers[2].control.outbound_depth, 0, "and only on that link")
    end)

    t.it("closes one member link without disturbing the room", function()
        local _, members = build_room(3)
        assert(members[1]:close_peer("guest_1", "peer_left"))
        t.eq(members[1]:peer_state("guest_1"), "closed")
        t.eq(members[2]:peer_state("host"), "disconnected")
        t.eq(members[3]:peer_state("host"), "connected")
        assert(members[1]:broadcast("input", input(1, 40, "authority")))
        members[1]:pump()
        t.eq(#members[2]:poll_batch(16), 0, "a closed link receives nothing")
        t.eq(#members[3]:poll_batch(16), 1, "an untouched link is unaffected")
    end)

    t.it("drains and leaves the room on shutdown", function()
        local room, members = build_room(3)
        assert(members[1]:send("guest_1", "control", control(0, "hello")))
        assert(members[1]:shutdown())
        t.eq(members[1]:state(), "closed")
        t.eq(#members[1]:diagnostics().peers, 0, "no orphaned links survive teardown")
        t.eq(#room.members, 2, "the member left the room")
        t.eq(members[2]:peer_state("host"), "disconnected")
    end)
end)

-- ---------------------------------------------------------------------------
-- What the session stack does when no peer is the sequencer
-- ---------------------------------------------------------------------------

t.describe("relay topology probe: no peer is the sequencer", function()
    -- Finding 1. `match_driver`'s guest authority path accepts host batches and
    -- nothing else, so a client that receives another client's own bundle --
    -- which is all a framing relay can ever deliver -- kills the match.
    t.it("terminates a guest that receives a peer's own bundle", function()
        local harness = fault_harness.new({
            topology = "relay",
            mode = "2v2",
            profile = "clean",
            duration_ticks = 60,
        })
        assert(fault_harness.reach_start(harness))
        fault_harness.start_match(harness)
        for _ = 1, 12 do
            fault_harness.advance(harness)
        end
        local sender, target = harness.clients[2], harness.clients[3]
        local freeze = assert(sender.coordinator.freeze)
        local slot_index = live_slot.slot_index(assert(freeze.owned[sender.peer_id])[1])
        ---@type InputAuthorityRow[]
        local rows = {}
        for tick = 0, input_protocol.HISTORY_ROWS do
            rows[#rows + 1] = {
                tick = freeze.first_input_tick + tick,
                slot_index = slot_index,
                sample = assert(input_frame.new_sample({ edges = 0 })),
            }
        end
        local packet = assert(input_protocol.new_guest({
            session_id = harness.manifest.session_id,
            manifest_id = freeze.manifest_id,
            sender_id = sender.peer_id,
            sequence = 9100,
            transport_tick = harness.step + match_driver.DELAY_TICKS,
            first_input_tick = freeze.first_input_tick,
            rows = rows,
        }))
        -- The relay accepts it. That is the point: the wire imposes nothing.
        t.is_true(sender.fault:send(
            target.peer_id,
            "input",
            assert(contract.new({
                type = "input",
                seq = packet.sequence,
                tick = packet.transport_tick,
                payload = assert(input_protocol.encode(packet)),
            }))
        ) == true)
        for _ = 1, 6 do
            fault_harness.advance(harness)
        end
        local terminal = assert(match_driver.terminal(assert(target.driver)))
        t.eq(terminal.status, "ownership_violation")
        t.eq(terminal.detail, "a guest received authority that was not a host batch")
        fault_harness.teardown(harness)
    end)

    -- Finding 2. Ownership validation in `input_protocol.canonical_host_batch`
    -- survives the topology change only because the relay attributes each line
    -- to the member that handed it up. A relay that concatenated blobs without
    -- keeping origin -- which is how "opaque forwarding" is usually described --
    -- takes this check with it.
    t.it("keeps ownership validation bound to the transport origin", function()
        local manifest = match_manifest.template("4v4")
        local peer_ids = { "host" }
        for index = 1, 7 do
            peer_ids[#peer_ids + 1] = "guest_" .. tostring(index)
        end
        local assignments = assert(coordinator.plan_assignments(manifest, peer_ids))
        local slot_index = 2
        local producer = assignments[slot_index].producer_id
        ---@type InputAuthorityRow[]
        local rows = {}
        for tick = 0, input_protocol.HISTORY_ROWS do
            rows[#rows + 1] = {
                tick = tick,
                slot_index = slot_index,
                sample = assert(input_frame.new_sample({ edges = 0 })),
            }
        end
        local packet = assert(input_protocol.new_guest({
            session_id = manifest.session_id,
            manifest_id = require("game.online.protocol").manifest_id(manifest),
            sender_id = producer,
            sequence = 1,
            transport_tick = 0,
            first_input_tick = 0,
            rows = rows,
        }))
        local envelope = assert(contract.new({
            type = "input",
            seq = packet.sequence,
            tick = packet.transport_tick,
            payload = assert(input_protocol.encode(packet)),
        }))
        ---@param transport_peer_id string
        ---@return string?
        local function batch_with_origin(transport_peer_id)
            local _, _, code = input_protocol.canonical_host_batch({
                manifest = manifest,
                assignments = assignments,
                host_peer_id = "host",
                sequence = 1,
                transport_tick = 0,
                first_input_tick = 0,
            }, {
                {
                    packet = packet,
                    envelope = envelope,
                    arrival_tick = 0,
                    transport_peer_id = transport_peer_id,
                },
            })
            return code
        end
        t.eq(batch_with_origin(producer), nil, "the true origin canonicalises")
        t.eq(
            batch_with_origin("guest_7"),
            "ownership_mismatch",
            "a lost origin is an ownership violation, not a silent accept"
        )
    end)

    -- Finding 3. Declared bot fills are authored by the host and by nobody else
    -- (`match_driver.new`: `role == "host" and producer.producer_kind == "bot"`).
    -- Remove the host and those slots have no author at all.
    t.it("gives declared bot fills no author but the host", function()
        local harness = fault_harness.new({
            topology = "relay",
            mode = "4v4",
            humans = 4,
            profile = "clean",
            duration_ticks = 60,
        })
        assert(fault_harness.reach_start(harness))
        fault_harness.start_match(harness)
        local host = match_driver.diagnostics(assert(harness.clients[1].driver))
        local guest = match_driver.diagnostics(assert(harness.clients[2].driver))
        t.eq(#host.owned, 1, "the host owns one slot")
        t.eq(#host.authored, 5, "and authors its own plus all four bot fills")
        t.eq(#guest.owned, 1)
        t.eq(#guest.authored, 1, "a guest authors only what it owns")
        fault_harness.teardown(harness)
    end)

    -- Finding 4. The session lifecycle is host-authoritative in `coordinator`,
    -- independently of the wire. Moving input distribution to a relay does not
    -- touch any of these.
    t.it("keeps the session lifecycle host-authoritative", function()
        local manifest = match_manifest.template("2v2")
        local state = assert(coordinator.new({
            role = "guest",
            session_id = manifest.session_id,
            peer_id = "guest_1",
            host_peer_id = "host",
            host_link_id = "host",
            runtime = protocol_fixture.runtime(),
            expectation = {
                build_id = manifest.build_id,
                source_id = manifest.source_id,
                content_id = manifest.content_id,
                tuning_id = manifest.tuning_id,
                match_config_id = manifest.match_config_id,
                fixture_id = manifest.fixture_id,
                arena_id = manifest.arena_id,
                combat_rules_id = manifest.combat_rules_id,
                gameplay_ai_policy_id = manifest.gameplay_ai_policy_id,
                combat_status = manifest.combat_status,
            },
        }))
        local _, proposed = coordinator.step(state, {
            kind = "propose_manifest",
            manifest = manifest,
        })
        t.eq(proposed.accepted, false)
        t.eq(proposed.reason, "only the host proposes the session manifest")
        local _, assigned = coordinator.step(state, { kind = "assign_slots", assignments = {} })
        t.eq(assigned.reason, "only the host publishes slot assignments")
        local _, counted = coordinator.step(state, {
            kind = "begin_countdown",
            countdown_id = "countdown.1",
            remaining_ticks = 2,
            first_input_tick = 0,
        })
        t.eq(counted.reason, "only the host starts the countdown")
    end)

    -- Finding 5. The settle phase's host relay wait is host-only by
    -- construction. It exists because a player-host that stops relaying strands
    -- everyone else's tail; a relay that is not a player cannot leave, so this
    -- is one piece of complexity the topology genuinely deletes.
    --
    -- #243 re-pinned the *cost*, not the finding, and #255 finished the job. When
    -- this landed the wait was a heuristic -- four consecutive silent steps,
    -- `SETTLE_RELAY_QUIET_STEPS` -- and the host paid all four on every clean
    -- match. Guests now report their own confirmation in the bundles they already
    -- re-publish, so the host leaves when every author it has heard from has
    -- confirmed the final boundary, and the quiet count is gone entirely. Under
    -- clean delivery that is two steps rather than four-plus, which is why this
    -- asserts an upper bound where it used to assert a lower one.
    --
    -- The finding itself is untouched: the host branch of `tail_delivered` is
    -- still consulted by exactly one role, and a relay that is not a player still
    -- has no settle phase to scope it to.
    t.it("scopes the settle relay wait to the host alone", function()
        local harness = fault_harness.new({
            topology = "relay",
            mode = "1v1",
            profile = "clean",
            duration_ticks = 40,
        })
        assert(fault_harness.reach_start(harness))
        fault_harness.start_match(harness)
        for _ = 1, 200 do
            fault_harness.advance(harness)
            if fault_harness.finished(harness) then
                break
            end
        end
        local host = match_driver.diagnostics(assert(harness.clients[1].driver))
        local guest = match_driver.diagnostics(assert(harness.clients[2].driver))
        t.eq(host.status, "completed")
        t.eq(guest.status, "completed")
        t.is_true(
            host.settle_steps <= 2,
            ("the host settled slowly on a clean 1v1: %d steps"):format(host.settle_steps)
        )
        t.is_true(
            guest.settle_steps <= host.settle_steps,
            "a guest cannot outlast the relay it depends on"
        )
        fault_harness.teardown(harness)
    end)

    -- Finding 6. The per-node wire cost of the shape the decision actually
    -- proposes: every member publishes only its own bundle and receives the
    -- other seven, concatenated into one frame. Both figures are measured here
    -- rather than derived, because the decision's own table gets both wrong.
    t.it("measures sequencer-less per-node wire cost", function()
        local _, members = build_room(8)
        local ticks = 60
        for tick = 1, ticks do
            for index, endpoint in ipairs(members) do
                assert(endpoint:broadcast("input", guest_bundle(member_id(index), index, tick)))
            end
            members[1]:pump()
            for _, endpoint in ipairs(members) do
                endpoint:poll_batch(contract.MAX_QUEUE_LIMIT)
            end
        end
        for index, endpoint in ipairs(members) do
            local counters = endpoint:wire_counters()
            local up = counters.input_uplink_bytes / ticks
            local down = counters.input_downlink_bytes / ticks
            local framed = counters.downlink_framed_bytes / ticks
            t.eq(counters.uplink_units, ticks, member_id(index) .. " uploads once per tick")
            t.eq(counters.downlink_frames, ticks, "and receives one framed message per tick")
            -- One own bundle up; seven other bundles down. The uplink is an
            -- order of magnitude under the decision's predicted 1,190 B/tick,
            -- and the downlink is roughly double its predicted ~650 B/tick.
            t.is_true(up > 180 and up < 200, ("uplink %.1f B/tick"):format(up))
            t.is_true(down > 1300 and down < 1400, ("downlink %.1f B/tick"):format(down))
            t.near(down / up, 7, 0.05, "a member receives exactly the other seven bundles")
            -- `input_downlink_bytes` counts envelopes only, so that it compares
            -- with the star's figure. The wire also carries the per-line origin
            -- that finding 2 makes mandatory, plus the separators between lines,
            -- so the true downlink is strictly higher and the envelope figure is
            -- a floor. Pinned here so the doc's 1.76x-to-1.90x bracket cannot
            -- drift silently.
            t.is_true(
                framed > down,
                ("framed %.1f must exceed the envelope figure %.1f"):format(framed, down)
            )
            -- Re-pinned by #243, which added one `confirmed_span` header field to
            -- every input packet. The envelope figures above are unchanged
            -- because their brackets are wider than the field; this one is
            -- tighter and moved from ~1,430 to ~1,463 B/tick, which is the seven
            -- relayed bundles each carrying the new field.
            t.is_true(
                framed > 1450 and framed < 1475,
                ("framed downlink %.1f B/tick"):format(framed)
            )
        end
    end)
end)
