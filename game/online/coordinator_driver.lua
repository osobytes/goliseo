local coordinator = require("game.online.coordinator")
local fixture = require("game.online.coordinator_fixture")
local protocol = require("game.online.protocol")

---@class CoordinatorDriverNode
---@field peer_id string
---@field role SessionRole
---@field state CoordinatorState
---@field link_id string -- The single host<->guest link this node owns.
---@field started boolean
---@field first_input_tick integer?
---@field terminal CoordinatorTerminal?

---@class CoordinatorDriverLink
---@field id string
---@field guest_peer_id string
---@field host_open boolean
---@field guest_open boolean

---@class CoordinatorDriverPacket
---@field link_id string
---@field to_host boolean
---@field wire string
---@field deliver_at integer

---@class CoordinatorDriverOptions
---@field session_id string?
---@field guest_count integer?
---@field latency_ticks integer?
---@field expectation CoordinatorManifestExpectation?
---@field mode SessionMatchMode?

---@class CoordinatorDriver
---@field clock integer
---@field latency_ticks integer
---@field session_id string
---@field mode SessionMatchMode
---@field nodes CoordinatorDriverNode[] -- Host first, then guests in order.
---@field links CoordinatorDriverLink[]
---@field queue CoordinatorDriverPacket[]
---@field transcript SessionControlMessage[]
---@field log string[]
---@field delivered integer
---@field dropped integer
local Driver = {}
Driver.__index = Driver

---@class SessionCoordinatorDriverModule
local driver = {}

driver.MAX_PUMP_ROUNDS = 64

---@param options CoordinatorDriverOptions?
---@return CoordinatorDriver
function driver.new(options)
    options = options or {}
    local guest_count = options.guest_count or 0
    local mode = options.mode or "4v4"
    local shape = assert(protocol.MATCH_MODES[mode], "unsupported driver match mode")
    assert(
        guest_count >= 0 and guest_count <= shape.humans - 1,
        ("a direct-host %s session seats at most %d guests"):format(mode, shape.humans - 1)
    )
    local session_id = options.session_id or fixture.manifest(mode).session_id
    ---@type CoordinatorDriver
    local self = setmetatable({
        clock = 0,
        latency_ticks = options.latency_ticks or 0,
        session_id = session_id,
        mode = mode,
        nodes = {},
        links = {},
        queue = {},
        transcript = {},
        log = {},
        delivered = 0,
        dropped = 0,
    }, Driver)
    self.nodes[1] = {
        peer_id = fixture.HOST_PEER_ID,
        role = "host",
        state = fixture.host(session_id),
        link_id = "",
        started = false,
    }
    for index = 1, guest_count do
        local state = fixture.guest(index, session_id, options.expectation)
        self.nodes[#self.nodes + 1] = {
            peer_id = state.peer_id,
            role = "guest",
            state = state,
            link_id = assert(state.host_link_id),
            started = false,
        }
        self.links[#self.links + 1] = {
            id = assert(state.host_link_id),
            guest_peer_id = state.peer_id,
            host_open = true,
            guest_open = true,
        }
    end
    return self
end

---@param peer_id string
---@return CoordinatorDriverNode?
function Driver:node(peer_id)
    for _, node in ipairs(self.nodes) do
        if node.peer_id == peer_id then
            return node
        end
    end
    return nil
end

---@return CoordinatorDriverNode
function Driver:host()
    return self.nodes[1]
end

---@param link_id string
---@return CoordinatorDriverLink?
function Driver:link(link_id)
    for _, link in ipairs(self.links) do
        if link.id == link_id then
            return link
        end
    end
    return nil
end

---@param node CoordinatorDriverNode
---@param link_id string
---@param message SessionControlMessage
function Driver:_enqueue(node, link_id, message)
    local link = self:link(link_id)
    local to_host = node.role == "guest"
    if not link or (to_host and not link.guest_open) or (not to_host and not link.host_open) then
        self.dropped = self.dropped + 1
        return
    end
    local wire = assert(protocol.encode(message))
    self.log[#self.log + 1] = ("%s>%s %s"):format(
        node.peer_id,
        to_host and fixture.HOST_PEER_ID or link.guest_peer_id,
        message.kind
    )
    self.queue[#self.queue + 1] = {
        link_id = link_id,
        to_host = to_host,
        wire = wire,
        deliver_at = self.clock + self.latency_ticks,
    }
end

---@param node CoordinatorDriverNode
---@param actions CoordinatorAction[]
function Driver:_apply(node, actions)
    for _, action in ipairs(actions) do
        if action.kind == "send" then
            -- One emitted message is one transcript entry, however many links
            -- it fans out to, so per-peer sequences stay strictly monotonic.
            self.transcript[#self.transcript + 1] = assert(action.message)
            for _, link_id in ipairs(assert(action.targets)) do
                self:_enqueue(node, link_id, assert(action.message))
            end
        elseif action.kind == "close" then
            local link = self:link(assert(action.link_id))
            if link then
                if node.role == "host" then
                    link.host_open = false
                else
                    link.guest_open = false
                end
            end
        elseif action.kind == "start_match" then
            node.started = true
            node.first_input_tick = assert(action.freeze).first_input_tick
        elseif action.kind == "terminate" then
            node.terminal = assert(action.terminal)
        end
    end
end

---@param peer_id string
---@param event table
---@return CoordinatorOutcome
function Driver:send(peer_id, event)
    local node = assert(self:node(peer_id), "unknown driver node " .. tostring(peer_id))
    local state, outcome = coordinator.step(node.state, event)
    node.state = state
    self:_apply(node, outcome.actions)
    return outcome
end

-- Deliver every packet whose fake-clock deadline has passed, including packets
-- produced by the deliveries themselves, so a pump reaches a stable state.
---@return integer delivered
function Driver:pump()
    local delivered = 0
    for _ = 1, driver.MAX_PUMP_ROUNDS do
        local due = {}
        local pending = {}
        for _, packet in ipairs(self.queue) do
            if packet.deliver_at <= self.clock then
                due[#due + 1] = packet
            else
                pending[#pending + 1] = packet
            end
        end
        if #due == 0 then
            return delivered
        end
        self.queue = pending
        for _, packet in ipairs(due) do
            local link = assert(self:link(packet.link_id))
            local target_peer = packet.to_host and fixture.HOST_PEER_ID or link.guest_peer_id
            local node = assert(self:node(target_peer))
            local open
            if packet.to_host then
                open = link.host_open
            else
                open = link.guest_open
            end
            if open and not coordinator.is_terminal(node.state) then
                delivered = delivered + 1
                self.delivered = self.delivered + 1
                local state, outcome = coordinator.receive(node.state, packet.link_id, packet.wire)
                node.state = state
                self:_apply(node, outcome.actions)
            else
                self.dropped = self.dropped + 1
            end
        end
    end
    error("coordinator driver did not settle within its pump bound")
end

---@param count integer?
---@return CoordinatorDriver
function Driver:tick(count)
    for _ = 1, count or 1 do
        self.clock = self.clock + 1
        self:pump()
        for _, node in ipairs(self.nodes) do
            local state, outcome = coordinator.step(node.state, { kind = "tick" })
            node.state = state
            self:_apply(node, outcome.actions)
        end
        self:pump()
    end
    return self
end

---@return CoordinatorDriver
function Driver:connect_all()
    for index = 2, #self.nodes do
        self:send(self.nodes[index].peer_id, { kind = "connect" })
    end
    self:pump()
    return self
end

-- Send a control message that the sender's own coordinator would never emit,
-- over the sender's real link and canonical wire format. This is how a
-- misbehaving or compromised peer is modelled: the receiving coordinator is
-- exercised exactly as in production, with no test-only entry point. The
-- sender's counter advances so the transcript stays monotonic for both ends.
---@param from_peer_id string
---@param to_peer_id string
---@param kind SessionMessageKind
---@param body SessionMessageBody
---@param session_id string?
---@return CoordinatorDriver
function Driver:inject(from_peer_id, to_peer_id, kind, body, session_id)
    local node = assert(self:node(from_peer_id), "unknown driver node " .. tostring(from_peer_id))
    local guest_peer_id = node.role == "host" and to_peer_id or from_peer_id
    local message = assert(
        protocol.new(kind, session_id or self.session_id, from_peer_id, node.state.sequence, body)
    )
    node.state.sequence = node.state.sequence + 1
    self:_apply(node, {
        { kind = "send", targets = { fixture.link_id(guest_peer_id) }, message = message },
    })
    self:pump()
    return self
end

-- Replay a wire that was already delivered, exactly as a reliable transport
-- retransmitting after loss would.
---@param from_peer_id string
---@param to_peer_id string
---@param wire string
---@return CoordinatorDriver
function Driver:replay(from_peer_id, to_peer_id, wire)
    local node = assert(self:node(from_peer_id))
    local guest_peer_id = node.role == "host" and to_peer_id or from_peer_id
    self.queue[#self.queue + 1] = {
        link_id = fixture.link_id(guest_peer_id),
        to_host = node.role == "guest",
        wire = wire,
        deliver_at = self.clock,
    }
    self:pump()
    return self
end

-- The canonical wire of the first message of `kind` this session carried.
---@param kind SessionMessageKind
---@return string
function Driver:first_wire(kind)
    for _, message in ipairs(self.transcript) do
        if message.kind == kind then
            return assert(protocol.encode(message))
        end
    end
    error("the session never carried a " .. kind .. " message")
end

-- Simulate a transport failure: both endpoints observe the loss of the link.
---@param guest_peer_id string
---@param code SessionDisconnectCode?
---@return CoordinatorDriver
function Driver:drop_link(guest_peer_id, code)
    local link = assert(self:link(fixture.link_id(guest_peer_id)))
    link.host_open = false
    link.guest_open = false
    local guest = self:node(guest_peer_id)
    if guest and not coordinator.is_terminal(guest.state) then
        self:send(guest_peer_id, { kind = "link_lost", link_id = link.id, code = code })
    end
    local host = self:host()
    if not coordinator.is_terminal(host.state) then
        self:send(host.peer_id, { kind = "link_lost", link_id = link.id, code = code })
    end
    self:pump()
    return self
end

---@return string
function Driver:transcript_id()
    return protocol.transcript_id(self.transcript)
end

---@return string
function Driver:trace()
    return table.concat(self.log, ";")
end

---@return boolean
function Driver:all_started()
    for _, node in ipairs(self.nodes) do
        if not node.started then
            return false
        end
    end
    return true
end

---@param reason CoordinatorTerminalReason?
---@return boolean
function Driver:all_terminal(reason)
    for _, node in ipairs(self.nodes) do
        if not node.terminal then
            return false
        end
        if reason and node.terminal.reason ~= reason then
            return false
        end
    end
    return true
end

-- Drive a session from manual connection to the frozen start boundary.
---@param countdown_ticks integer?
---@param first_input_tick integer?
---@return CoordinatorDriver
function Driver:reach_start(countdown_ticks, first_input_tick)
    local manifest = fixture.manifest(self.mode)
    local guest_count = #self.nodes - 1
    self:connect_all()
    self:send(fixture.HOST_PEER_ID, { kind = "propose_manifest", manifest = manifest })
    self:pump()
    self:send(fixture.HOST_PEER_ID, {
        kind = "assign_slots",
        assignments = assert(coordinator.plan_assignments(manifest, fixture.peer_ids(guest_count))),
    })
    self:pump()
    for _, node in ipairs(self.nodes) do
        self:send(node.peer_id, { kind = "set_ready", ready = true })
    end
    self:pump()
    local ticks = countdown_ticks or 3
    self:send(fixture.HOST_PEER_ID, {
        kind = "begin_countdown",
        countdown_id = fixture.COUNTDOWN_ID,
        remaining_ticks = ticks,
        first_input_tick = first_input_tick or 0,
    })
    self:pump()
    self:tick(ticks + 1)
    return self
end

-- Acknowledge a complete simulation: kickoff, play, full time, and result.
---@param home_score integer?
---@param away_score integer?
---@return CoordinatorDriver
function Driver:play_out(home_score, away_score)
    local home = home_score or 1
    local away = away_score or 0
    local host = fixture.HOST_PEER_ID
    self:send(
        host,
        { kind = "match_phase", phase = "kickoff", tick = 0, home_score = 0, away_score = 0 }
    )
    self:send(
        host,
        { kind = "match_phase", phase = "playing", tick = 1, home_score = 0, away_score = 0 }
    )
    self:pump()
    self:send(host, {
        kind = "match_phase",
        phase = "full_time",
        tick = 7200,
        home_score = home,
        away_score = away,
    })
    self:pump()
    self:send(host, {
        kind = "finish",
        final_tick = 7200,
        home_score = home,
        away_score = away,
        final_hash = "fedcba9876543210",
    })
    self:pump()
    return self
end

return driver
