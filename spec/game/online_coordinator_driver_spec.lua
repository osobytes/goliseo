local coordinator = require("game.online.coordinator")
local driver = require("game.online.coordinator_driver")
local fixture = require("game.online.coordinator_fixture")
local input_frame = require("sim.input_frame")
local t = require("spec.support.runner")

local HOST = fixture.HOST_PEER_ID

---@param session CoordinatorDriver
---@return integer, integer
local function count_sources(session)
    local freeze = assert(session:host().state.freeze)
    local peers, bots = 0, 0
    for index = 1, input_frame.SLOT_COUNT do
        local slot = assert(input_frame.slot(index))
        local producer = assert(freeze.sources[slot.id])
        if producer.producer_kind == "peer" then
            peers = peers + 1
        else
            bots = bots + 1
        end
    end
    return peers, bots
end

t.describe("online coordinator driver", function()
    t.it("runs a host plus seven humans from connect to acknowledged result", function()
        local session = driver.new({ guest_count = 7 })
        session:reach_start(3, 0)

        t.eq(session:host().state.phase, "running")
        t.is_true(session:all_started())
        for _, node in ipairs(session.nodes) do
            t.eq(node.state.phase, "running")
            t.eq(node.first_input_tick, 0)
            t.eq(assert(node.state.freeze).countdown_id, fixture.COUNTDOWN_ID)
        end
        local peers, bots = count_sources(session)
        t.eq(peers, 8)
        t.eq(bots, 0)

        session:play_out(2, 1)
        t.is_true(session:all_terminal("completed"))
        t.eq(assert(session:host().state.result).home_score, 2)
        t.eq(assert(session:host().state.terminal).code, nil)
    end)

    t.it("fills unoccupied development slots with deterministic bots", function()
        local session = driver.new({ guest_count = 2 })
        session:reach_start(2, 0)
        local peers, bots = count_sources(session)
        t.eq(peers, 3)
        t.eq(bots, 5)

        local freeze = assert(session:host().state.freeze)
        local seen = {}
        for index = 1, input_frame.SLOT_COUNT do
            local slot = assert(input_frame.slot(index))
            local producer = assert(freeze.sources[slot.id])
            t.is_true(not seen[producer.producer_id], "producer ids must be unique")
            seen[producer.producer_id] = true
            if producer.producer_kind == "bot" then
                t.is_true(producer.bot_seed ~= nil, "bot producers declare a seed")
            end
        end
        local twin = assert(coordinator.plan_assignments(fixture.manifest(), fixture.peer_ids(2)))
        t.eq(twin[8].bot_seed, freeze.assignments[8].bot_seed)

        session:play_out(0, 3)
        t.is_true(session:all_terminal("completed"))
    end)

    t.it("runs a solo host session with no control traffic at all", function()
        local session = driver.new({ guest_count = 0 })
        session:reach_start(1, 0)
        t.eq(session:host().state.phase, "running")
        t.eq(#session.transcript, 0)
        session:play_out(1, 1)
        t.eq(assert(session:host().state.terminal).reason, "completed")
    end)

    t.it("is deterministic across identical runs", function()
        local first = driver.new({ guest_count = 3 })
        first:reach_start(2, 0):play_out(1, 0)
        local second = driver.new({ guest_count = 3 })
        second:reach_start(2, 0):play_out(1, 0)
        t.eq(first:transcript_id(), second:transcript_id())
        t.eq(first:trace(), second:trace())
        t.is_true(#first:transcript_id() == 16)
    end)

    t.it("keeps the session alive when a guest leaves before the countdown", function()
        local session = driver.new({ guest_count = 2 })
        session:connect_all()
        session:send(HOST, { kind = "propose_manifest", manifest = fixture.manifest() })
        session:pump()
        session:send(HOST, { kind = "assign_slots", assignments = fixture.assignments(2) })
        session:pump()
        for _, node in ipairs(session.nodes) do
            session:send(node.peer_id, { kind = "set_ready", ready = true })
        end
        session:pump()
        t.eq(session:host().state.phase, "ready")

        session:send(fixture.guest_peer_id(2), { kind = "leave" })
        session:pump()
        local host = session:host()
        t.eq(host.terminal, nil)
        t.eq(host.state.phase, "assigned")
        t.eq(#host.state.peers, 2)
        t.eq(host.state.assignments, nil)
        t.eq(host.state.peers[1].ready, false)
        t.eq(assert(session:node(fixture.guest_peer_id(2))).terminal.reason, "guest_left")
        local survivor = assert(session:node(fixture.guest_peer_id(1)))
        t.eq(survivor.state.phase, "assigned")
        t.eq(survivor.state.peers[1].ready, false)

        -- The remaining humans can reconfigure and start a bot-filled session.
        session:send(HOST, {
            kind = "assign_slots",
            assignments = assert(
                coordinator.plan_assignments(fixture.manifest(), { HOST, fixture.guest_peer_id(1) })
            ),
        })
        session:pump()
        session:send(HOST, { kind = "set_ready", ready = true })
        session:send(fixture.guest_peer_id(1), { kind = "set_ready", ready = true })
        session:pump()
        t.eq(session:host().state.phase, "ready")
    end)

    t.it("ends the session when a frozen guest departs", function()
        local session = driver.new({ guest_count = 2 })
        session:reach_start(1, 0)
        session:send(fixture.guest_peer_id(1), { kind = "leave" })
        session:pump()
        t.eq(assert(session:host().terminal).reason, "guest_left")
        t.eq(assert(session:host().terminal).code, "peer_disconnect")
        t.eq(assert(session:node(fixture.guest_peer_id(2))).terminal.reason, "peer_abort")
    end)

    t.it("ends the session when a frozen guest link drops", function()
        local session = driver.new({ guest_count = 2 })
        session:reach_start(1, 0)
        session:drop_link(fixture.guest_peer_id(2), "transport_lost")
        t.eq(assert(session:host().terminal).reason, "transport_lost")
        t.eq(assert(session:node(fixture.guest_peer_id(2))).terminal.reason, "transport_lost")
    end)

    t.it("gives a guest a stable reason when the host disappears", function()
        local session = driver.new({ guest_count = 1 })
        session:reach_start(1, 0)
        local guest = fixture.guest_peer_id(1)
        session:send(guest, {
            kind = "link_lost",
            link_id = fixture.link_id(guest),
            code = "host_left",
        })
        t.eq(assert(session:node(guest)).terminal.reason, "host_left")
        t.eq(assert(session:node(guest)).terminal.origin, "remote")
    end)

    t.it("aborts when a peer never acknowledges the start boundary", function()
        local session = driver.new({ guest_count = 2 })
        session:connect_all()
        session:send(HOST, { kind = "propose_manifest", manifest = fixture.manifest() })
        session:pump()
        session:send(HOST, { kind = "assign_slots", assignments = fixture.assignments(2) })
        session:pump()
        for _, node in ipairs(session.nodes) do
            session:send(node.peer_id, { kind = "set_ready", ready = true })
        end
        session:pump()
        session:send(HOST, {
            kind = "begin_countdown",
            countdown_id = fixture.COUNTDOWN_ID,
            remaining_ticks = 1,
            first_input_tick = 0,
        })
        session:pump()
        -- Silence one guest's uplink before the start boundary is published.
        local silenced = assert(session:link(fixture.link_id(fixture.guest_peer_id(2))))
        silenced.guest_open = false
        session:tick(coordinator.START_ACK_TIMEOUT_TICKS + 3)

        local terminal = assert(session:host().terminal)
        t.eq(terminal.reason, "start_ack_timeout")
        t.eq(terminal.origin, "timeout")
        t.eq(terminal.peer_id, fixture.guest_peer_id(2))
        t.eq(session:host().started, false)
    end)

    t.it("ends the session on a persistent boundary hash mismatch", function()
        local session = driver.new({ guest_count = 1 })
        session:reach_start(1, 0)
        local guest = fixture.guest_peer_id(1)
        for index = 1, coordinator.MAX_HASH_MISMATCHES do
            local tick = index * 60
            session:send(HOST, {
                kind = "hash_report",
                tick = tick,
                boundary_hash = "0123456789abcdef",
            })
            session:send(guest, {
                kind = "hash_report",
                tick = tick,
                boundary_hash = "ffffffffffffffff",
            })
            session:pump()
        end
        local terminal = assert(session:host().terminal)
        t.eq(terminal.reason, "hash_mismatch")
        t.eq(terminal.code, "desync")
        t.eq(terminal.peer_id, guest)
        -- Detection is symmetric: the guest reaches the same verdict locally.
        t.eq(assert(session:node(guest)).terminal.reason, "hash_mismatch")
    end)

    t.it("tolerates a single boundary hash disagreement", function()
        local session = driver.new({ guest_count = 1 })
        session:reach_start(1, 0)
        local guest = fixture.guest_peer_id(1)
        session:send(HOST, { kind = "hash_report", tick = 60, boundary_hash = "0123456789abcdef" })
        session:send(guest, { kind = "hash_report", tick = 60, boundary_hash = "ffffffffffffffff" })
        session:pump()
        t.eq(session:host().terminal, nil)
        t.eq(session:host().state.peers[2].hash_mismatches, 1)
        session:send(HOST, { kind = "hash_report", tick = 120, boundary_hash = "0123456789abcdef" })
        session:send(
            guest,
            { kind = "hash_report", tick = 120, boundary_hash = "0123456789abcdef" }
        )
        session:pump()
        t.eq(session:host().state.peers[2].hash_mismatches, 0)
    end)

    t.it("turns netcode terminal failures into stable session reasons", function()
        for _, case in ipairs({
            {
                failure = "input_channel",
                reason = "input_channel_failure",
                code = "peer_disconnect",
            },
            { failure = "late_input", reason = "late_input", code = "desync" },
            { failure = "desync", reason = "hash_mismatch", code = "desync" },
        }) do
            local session = driver.new({ guest_count = 1 })
            session:reach_start(1, 0)
            session:send(HOST, { kind = "netcode_failure", failure = case.failure })
            session:pump()
            local terminal = assert(session:host().terminal)
            t.eq(terminal.reason, case.reason)
            t.eq(terminal.code, case.code)
            t.eq(assert(session:node(fixture.guest_peer_id(1))).terminal.reason, "peer_abort")
        end
    end)

    t.it("aborts every peer when the host quits", function()
        local session = driver.new({ guest_count = 3 })
        session:reach_start(1, 0)
        session:send(HOST, { kind = "abort", code = "host_abort" })
        session:pump()
        t.eq(assert(session:host().terminal).reason, "local_abort")
        for index = 1, 3 do
            local guest = assert(session:node(fixture.guest_peer_id(index)))
            t.eq(assert(guest.terminal).reason, "peer_abort")
            t.eq(assert(guest.terminal).code, "host_abort")
        end
    end)

    t.it("survives duplicated and delayed control delivery", function()
        local session = driver.new({ guest_count = 2, latency_ticks = 2 })
        session:connect_all()
        session:tick(3)
        session:send(HOST, { kind = "propose_manifest", manifest = fixture.manifest() })
        session:tick(3)
        -- Replay every queued wire once more; a reliable channel may retransmit.
        local replay = {}
        for _, packet in ipairs(session.queue) do
            replay[#replay + 1] = packet
        end
        for _, packet in ipairs(replay) do
            session.queue[#session.queue + 1] = {
                link_id = packet.link_id,
                to_host = packet.to_host,
                wire = packet.wire,
                deliver_at = packet.deliver_at,
            }
        end
        session:tick(4)
        t.eq(session:host().terminal, nil)
        t.eq(#session:host().state.peers, 3)
        for index = 2, 3 do
            t.eq(
                session:host().state.peers[index].accepted_manifest_id,
                session:host().state.manifest_id
            )
        end
    end)
end)

t.describe("online coordinator conformance", function()
    t.it("matches the pinned canonical session goldens", function()
        local conformance = require("game.online.coordinator_conformance")
        local report = conformance.verify()
        t.eq(#report.full_transcript_id, 16)
        t.is_true(report.message_count > 0)
        t.is_true(conformance.marker(report):find("GC_COORDINATOR|golden|") == 1)
    end)
end)
