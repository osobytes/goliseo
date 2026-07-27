-- Fixtures for OMP-3 network diagnostics: a full peer set, each with its own
-- recorder and its own diagnostic transport tap, driven step by step over the
-- in-process star.
--
-- The clock is injected and advances by a fixed amount per step. That is what
-- makes the *runtime* half of an export reproducible in a test at all -- and it
-- is also why the specs never generalise from that reproducibility. A frozen
-- clock proves the recorder is deterministic given its inputs; it proves nothing
-- about a real machine, where the same match yields different milliseconds every
-- time. The specs make byte-identity claims only about the canonical projection,
-- which does not contain a millisecond at all.

local match_driver = require("game.online.match_driver")
local match_driver_fixture = require("game.online.match_driver_fixture")
local net_diagnostics = require("game.online.net_diagnostics")
local DiagnosticTransport = require("game.online.diagnostic_transport")
local input_protocol = require("game.online.input_protocol")
local protocol = require("game.online.protocol")
local transport_contract = require("game.transport.contract")
local input_frame = require("sim.input_frame")

---@class NetDiagnosticsFixturePeer
---@field peer_id string
---@field role SessionRole
---@field recorder NetDiagnostics
---@field tap DiagnosticTransport
---@field driver MatchDriver

---@class NetDiagnosticsFixtureHarness
---@field session MatchDriverFixtureSession
---@field peers NetDiagnosticsFixturePeer[] -- Host first, then guests in seating order.
---@field step integer
---@field clock_ms number
---@field clock_step_ms number
---@field rtt_bias_ms number
---@field anchor_interval integer

---@class NetDiagnosticsFixtureOptions
---@field humans integer?
---@field duration number?
---@field combat boolean?
---@field hash_interval_ticks integer?
---@field max_rollback_ticks integer?
---@field clock_step_ms number?
---@field rtt_bias_ms number?
---@field anchor_interval integer?
---@field retain_wires integer?
---@field export_opt_in boolean?
---@field limits NetDiagnosticsLimits?

---@class NetDiagnosticsFixtureModule
local fixture = {}

-- One 60 Hz frame, to the microsecond the fixed clock actually implies.
fixture.DEFAULT_CLOCK_STEP_MS = 1000 / 60
fixture.DEFAULT_ANCHOR_INTERVAL = 15
fixture.DEFAULT_RETAIN_WIRES = 192

-- Half a frame: an anchor taken once per step can only place an input tick
-- within the frame it was sampled in, and saying so is the entire point of
-- carrying a mapping error next to the anchor.
fixture.MAPPING_ERROR_MS = fixture.DEFAULT_CLOCK_STEP_MS / 2

---@param mode SessionMatchMode
---@param options NetDiagnosticsFixtureOptions?
---@return NetDiagnosticsFixtureHarness
function fixture.harness(mode, options)
    options = options or {}
    local session = match_driver_fixture.session(mode, nil, options.humans)
    local snapshot = match_driver_fixture.initial_snapshot(options.duration, options.combat)
    ---@type NetDiagnosticsFixtureHarness
    local harness = {
        session = session,
        peers = {},
        step = 0,
        clock_ms = 0,
        clock_step_ms = options.clock_step_ms or fixture.DEFAULT_CLOCK_STEP_MS,
        rtt_bias_ms = options.rtt_bias_ms or 0,
        anchor_interval = options.anchor_interval or fixture.DEFAULT_ANCHOR_INTERVAL,
    }

    ---@param peer_id string
    ---@param role SessionRole
    ---@param transport StarTransportAdapter
    local function add(peer_id, role, transport)
        local recorder = net_diagnostics.new({
            role = role,
            peer_id = peer_id,
            manifest = session.manifest,
            freeze = session.freeze,
            input_delay_ticks = input_protocol.FAIRNESS_DELAY_TICKS,
            hash_interval_ticks = options.hash_interval_ticks
                or match_driver.DEFAULT_HASH_INTERVAL_TICKS,
            max_rollback_ticks = options.max_rollback_ticks or 0,
            limits = options.limits,
            export_opt_in = options.export_opt_in ~= false,
        })
        local tap = DiagnosticTransport.new({
            transport = transport,
            recorder = recorder,
            peer_id = peer_id,
            first_input_tick = session.freeze.first_input_tick,
            input_delay_ticks = match_driver.DELAY_TICKS,
            clock = function()
                return harness.clock_ms
            end,
            transport_tick = function()
                return harness.step + match_driver.DELAY_TICKS
            end,
            retain_wires = options.retain_wires or fixture.DEFAULT_RETAIN_WIRES,
        })
        local driver = match_driver.new({
            role = role,
            peer_id = peer_id,
            freeze = session.freeze,
            manifest = session.manifest,
            transport = tap,
            initial_snapshot = snapshot,
            hash_interval_ticks = options.hash_interval_ticks,
            max_rollback_ticks = options.max_rollback_ticks,
        })
        harness.peers[#harness.peers + 1] = {
            peer_id = peer_id,
            role = role,
            recorder = recorder,
            tap = tap,
            driver = driver,
        }
    end

    add(session.host_peer_id, "host", session.host_transport)
    for _, peer_id in ipairs(session.guest_peer_ids) do
        add(peer_id, "guest", assert(session.guest_transports[peer_id]))
    end
    return harness
end

-- The synthetic quality sample. It varies with the step so the recorder's
-- min/max/last folding is actually exercised, and it is a *function of the step*
-- so a repeated run with the same bias reproduces it -- which is the property the
-- runtime invariant checks lean on, not byte-identity of a real measurement.
---@param harness NetDiagnosticsFixtureHarness
---@return number rtt_ms, number jitter_ms
function fixture.quality(harness)
    local phase = harness.step % 5
    return 18 + phase * 2 + harness.rtt_bias_ms, phase * 0.5
end

---@param harness NetDiagnosticsFixtureHarness
---@param peer NetDiagnosticsFixturePeer
---@param batch MatchDriverBatch
local function fold(harness, peer, batch)
    -- Reading driver diagnostics also reads the tap's transport diagnostics,
    -- which is what folds the star's counters into the recorder.
    local diagnostics = match_driver.diagnostics(peer.driver)
    net_diagnostics.record_step(peer.recorder, diagnostics, batch)
    for _, addressed in ipairs(batch.control) do
        net_diagnostics.record_control(peer.recorder, addressed)
    end
    local rtt, jitter = fixture.quality(harness)
    for _, other in ipairs(harness.peers) do
        if other.peer_id ~= peer.peer_id then
            net_diagnostics.record_runtime_sample(peer.recorder, {
                peer_id = other.peer_id,
                monotonic_ms = harness.clock_ms,
                rtt_ms = rtt,
                jitter_ms = jitter,
            })
        end
    end
    if harness.anchor_interval > 0 and harness.step % harness.anchor_interval == 0 then
        net_diagnostics.record_anchor(peer.recorder, {
            input_tick = batch.input_tick,
            monotonic_ms = harness.clock_ms,
            mapping_error_ms = fixture.MAPPING_ERROR_MS,
        })
    end
end

-- One step for every peer, then one delivery. `deliver` false holds the star, so
-- every peer predicts and later corrects -- the impaired path.
---@param harness NetDiagnosticsFixtureHarness
---@param samples table<integer, InputSample>?
---@param deliver boolean?
---@return MatchDriverBatch[]
function fixture.advance(harness, samples, deliver)
    ---@type MatchDriverBatch[]
    local batches = {}
    for index, peer in ipairs(harness.peers) do
        local sample = samples and samples[index] or input_frame.neutral_sample()
        local batch = match_driver.advance(peer.driver, sample)
        batches[index] = batch
        fold(harness, peer, batch)
    end
    if deliver ~= false then
        harness.session.host_transport:pump()
    end
    harness.step = harness.step + 1
    harness.clock_ms = harness.clock_ms + harness.clock_step_ms
    return batches
end

---@param harness NetDiagnosticsFixtureHarness
---@param steps integer
---@param samples table<integer, InputSample>?
---@return MatchDriverBatch[]
function fixture.run(harness, steps, samples)
    ---@type MatchDriverBatch[]
    local last = {}
    for _ = 1, steps do
        last = fixture.advance(harness, samples)
    end
    return last
end

---@param harness NetDiagnosticsFixtureHarness
---@param steps integer
---@param period integer -- Deliver only every `period` steps.
---@param samples table<integer, InputSample>?
function fixture.run_bursty(harness, steps, period, samples)
    for step = 1, steps do
        fixture.advance(harness, samples, step % period == 0)
    end
end

---@param harness NetDiagnosticsFixtureHarness
---@param peer_id string
---@return NetDiagnosticsFixturePeer
function fixture.peer(harness, peer_id)
    for _, peer in ipairs(harness.peers) do
        if peer.peer_id == peer_id then
            return peer
        end
    end
    error("no fixture peer named " .. tostring(peer_id))
end

-- A real, decodable session control message, so `record_control` is exercised
-- against the protocol rather than against a stub payload.
---@param harness NetDiagnosticsFixtureHarness
---@param peer_id string
---@param sequence integer
---@param tick integer
---@param boundary_hash string
---@return TransportPeerMessage
function fixture.hash_report(harness, peer_id, sequence, tick, boundary_hash)
    local message = assert(
        protocol.new(
            "hash_report",
            harness.session.manifest.session_id,
            peer_id,
            sequence,
            { tick = tick, boundary_hash = boundary_hash }
        )
    )
    local envelope = assert(transport_contract.new({
        type = "event",
        seq = sequence,
        payload = assert(protocol.encode(message)),
    }))
    return { peer_id = peer_id, channel = "control", message = envelope, arrival_seq = sequence }
end

-- Forge an input bundle from `sender_id` naming `slot_index`. Ownership is a
-- frozen partition, so a slot outside the sender's owned set is an
-- `ownership_violation` and a second bundle on the same sequence with different
-- bytes is an `authority_conflict`. Both are reached this way rather than by
-- reaching into the driver.
---@param harness NetDiagnosticsFixtureHarness
---@param sender_id string
---@param slot_index integer
---@param sequence integer
---@param transport_tick integer
---@param edges integer
---@return TransportMessage
function fixture.forged_bundle(harness, sender_id, slot_index, sequence, transport_tick, edges)
    ---@type InputAuthorityRow[]
    local rows = {}
    for tick = 0, input_protocol.HISTORY_ROWS do
        rows[#rows + 1] = {
            tick = tick,
            slot_index = slot_index,
            sample = assert(input_frame.new_sample({
                edges = tick == input_protocol.HISTORY_ROWS and edges or 0,
            })),
        }
    end
    local packet = assert(input_protocol.new_guest({
        session_id = harness.session.manifest.session_id,
        manifest_id = harness.session.freeze.manifest_id,
        sender_id = sender_id,
        sequence = sequence,
        transport_tick = transport_tick,
        first_input_tick = harness.session.freeze.first_input_tick,
        rows = rows,
    }))
    return assert(transport_contract.new({
        type = "input",
        seq = packet.sequence,
        tick = packet.transport_tick,
        payload = assert(input_protocol.encode(packet)),
    }))
end

return fixture
