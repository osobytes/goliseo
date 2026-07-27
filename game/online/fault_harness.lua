-- The OMP-3 automated multi-client direct-host fault harness.
--
-- One `FaultHarness` owns N fully isolated clients — one host plus up to seven
-- guests — and drives them through the complete session lifecycle over the real
-- in-process star: handshake, manifest proposal and acceptance, slot assignment,
-- readiness, countdown, freeze, the mixed-family combat match, full time, the
-- acknowledged result, and teardown.
--
-- # Isolation is the whole point
--
-- Every client gets its own coordinator state, its own star endpoint, its own
-- impairment state, its own diagnostic recorder, its own rollback session, its
-- own presentation timeline, and its own session policy model. Nothing mutable
-- is shared: the only things two clients hold in common are the frozen manifest,
-- the freeze each derived independently, and the wire between them. Boundary
-- zero is *not* shared either — every client builds its own from
-- `match_session.request`, which is what makes an agreed hash evidence rather
-- than a tautology.
--
-- # What this reuses rather than reimplements
--
-- | Concern | Owner |
-- | --- | --- |
-- | Session state machine | `game.online.coordinator` |
-- | Session policy over a running match | `game.screens.online_match_model` |
-- | Control framing over the star | `game.online.lobby_link` |
-- | Match authority, rollback, live slot | `game.online.match_driver` |
-- | Presentation add/revoke/confirm | `game.online.match_presentation` |
-- | Resource and delivery measurement | `game.online.net_diagnostics` |
-- | Probabilistic impairment | `game.online.fault_transport` over `sim.network_conditions` |
--
-- The harness adds exactly two things of its own: the wiring that makes N of
-- those coexist, and the cross-client comparison that turns "it ran" into "they
-- agreed".
--
-- # What it cannot prove
--
-- Simulated profiles are simulated. They say nothing about NAT traversal, ICE,
-- TURN, or real-device frame scheduling; #170 covers real transport. Every
-- client here is a *logical* client in one operating-system process, so a
-- single-process run cannot catch a divergence caused by per-process hash-order
-- randomization — the separate-process campaign in `scripts/fault_harness.py`
-- exists for exactly that, and `docs/online/fault_harness.md` states the bound.
--
-- Rows contingent on #112 and #114 are declared and *skipped with a reason*
-- rather than omitted: the pre-#112 bot never reaches wind-up, guard, contact,
-- projectile flight, stagger, ball spill, or immunity expiry, so a spec for each
-- would pin an absence.

local coordinator = require("game.online.coordinator")
local DiagnosticTransport = require("game.online.diagnostic_transport")
local FaultTransport = require("game.online.fault_transport")
local lobby_link = require("game.online.lobby_link")
local match_driver = require("game.online.match_driver")
local match_manifest = require("game.online.match_manifest")
local match_presentation = require("game.online.match_presentation")
local match_session = require("game.online.match_session")
local net_diagnostics = require("game.online.net_diagnostics")
local online_match_model = require("game.screens.online_match_model")
local protocol = require("game.online.protocol")
local protocol_fixture = require("game.online.protocol_fixture")
local FakeStarTransport = require("game.transport.fake_star")
local transport_contract = require("game.transport.contract")
local input_frame = require("sim.input_frame")
local match_snapshot = require("sim.match_snapshot")
local rollback_input_history = require("sim.rollback_input_history")

---@class FaultHarnessClient
---@field index integer
---@field peer_id string
---@field role SessionRole
---@field star FakeStarTransport
---@field fault FaultTransport
---@field tap DiagnosticTransport? -- Created with the driver, once a freeze exists.
---@field recorder NetDiagnostics? -- Created with the driver, once a freeze exists.
---@field link LobbyLink
---@field coordinator CoordinatorState
---@field started boolean
---@field request OnlineMatchRequest?
---@field driver MatchDriver?
---@field presentation OnlineMatchPresentation?
---@field model OnlineMatchModel?
---@field buffers table<string, LobbyFrameBuffer>
---@field control TransportPeerMessage[]
---@field checkpoints MatchDriverCheckpoint[]
---@field published table<string, integer> -- Confirmed event id -> times published.
---@field revoked table<string, boolean>
---@field restored integer -- Revocations a later correction put back.
---@field final_hash string?
---@field errors string[]

---@class FaultHarnessOptions
---@field mode SessionMatchMode
---@field humans integer? -- Seated humans; fewer than the mode allows declares bot fills.
---@field profile NetworkProfileName?
---@field seed integer? -- Match seed override; defaults to the manifest template's.
---@field network_seed number? -- Impairment seed, never the match seed.
---@field duration_ticks integer?
---@field hash_interval_ticks integer?
---@field poll_order FaultTransportPollOrder?
---@field duplicate_control_every integer?
---@field queue_limit integer?
---@field buffered_amount_limit integer?
---@field countdown_ticks integer?
---@field script (fun(index: integer, step: integer): InputSample)?

---@class FaultHarness
---@field mode SessionMatchMode
---@field profile NetworkProfileName
---@field manifest SessionManifest
---@field peer_ids string[]
---@field clients FaultHarnessClient[]
---@field host_star FakeStarTransport
---@field transport_tick integer
---@field step integer
---@field clock_ms number
---@field hash_interval_ticks integer
---@field expect_backpressure boolean -- A clamped send buffer must actually latch.
---@field script fun(index: integer, step: integer): InputSample
---@field notes string[] -- Explicitly logged coverage bounds; never silent truncation.

---@class FaultHarnessModule
local fault_harness = {}

fault_harness.HOST_PEER_ID = transport_contract.HOST_PEER_ID
fault_harness.COUNTDOWN_ID = "countdown.1"
fault_harness.DEFAULT_DURATION_TICKS = 150
fault_harness.DEFAULT_NETWORK_SEED = 4703
fault_harness.DEFAULT_COUNTDOWN_TICKS = 2

-- One 60 Hz frame. The diagnostic recorder wants a monotonic millisecond source;
-- a fixed one makes the *runtime* half of an export reproducible in a test,
-- which is the only claim made about it.
fault_harness.CLOCK_STEP_MS = 1000 / 60

-- Control rounds the pre-match lifecycle is allowed for one exchange. Every
-- round is one star pump plus one poll on every client, so this is a liveness
-- bound and not a correctness one.
fault_harness.MAX_CONTROL_ROUNDS = 24

---@param index integer
---@return string
function fault_harness.guest_peer_id(index)
    return "guest_" .. tostring(index)
end

-- ---------------------------------------------------------------------------
-- Deterministic scripted input
-- ---------------------------------------------------------------------------

-- A pure function of `(client index, driver step)` only. No clock, no RNG, no
-- simulation read: two runs of the same matrix produce byte-identical authority,
-- which is the precondition for comparing deterministic markers at all.
--
-- The switch edge fires on a per-client phase so 1v1 and 2v2 actually move the
-- live slot, and it deliberately lands at different steps for different clients
-- so a correction can straddle one peer's switch while another is mid-window.
---@param index integer
---@param step integer
---@return InputSample
function fault_harness.scripted_sample(index, step)
    local phase = (step + index * 11) % 47
    local move_x = ((phase % 13) - 6) * 15
    local move_y = ((phase % 7) - 3) * 21
    local edges = 0
    if phase == 0 then
        edges = input_frame.EDGE_BITS.switch
    end
    return assert(input_frame.new_sample({ move_x = move_x, move_y = move_y, edges = edges }))
end

-- ---------------------------------------------------------------------------
-- Construction
-- ---------------------------------------------------------------------------

---@param manifest SessionManifest
---@return CoordinatorManifestExpectation
local function expectation_for(manifest)
    return {
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
    }
end

---@param harness FaultHarness
---@param message string
local function note(harness, message)
    harness.notes[#harness.notes + 1] = message
end

---@param options FaultHarnessOptions
---@return FaultHarness
function fault_harness.new(options)
    assert(type(options) == "table", "a fault harness needs options")
    local mode = assert(options.mode, "a fault harness needs a match mode")
    local shape = assert(protocol.match_mode(mode))
    local humans = options.humans or shape.humans
    assert(humans >= 1 and humans <= shape.humans, "the mode does not seat that many humans")

    local manifest = match_manifest.template(mode)
    manifest.duration_ticks = options.duration_ticks or fault_harness.DEFAULT_DURATION_TICKS
    if options.seed then
        manifest.seed = options.seed
    end
    local expectation = expectation_for(manifest)

    ---@type string[]
    local peer_ids = { fault_harness.HOST_PEER_ID }
    for index = 1, humans - 1 do
        peer_ids[#peer_ids + 1] = fault_harness.guest_peer_id(index)
    end

    local rendezvous = FakeStarTransport.new_rendezvous()
    local host_star = FakeStarTransport.new({
        role = "host",
        rendezvous = rendezvous,
        queue_limit = options.queue_limit,
        buffered_amount_limit = options.buffered_amount_limit,
    })
    assert(host_star:initialize())

    ---@type FaultHarness
    local harness = {
        mode = mode,
        profile = options.profile or "clean",
        manifest = manifest,
        peer_ids = peer_ids,
        clients = {},
        host_star = host_star,
        transport_tick = 0,
        step = 0,
        clock_ms = 0,
        hash_interval_ticks = options.hash_interval_ticks
            or match_driver.DEFAULT_HASH_INTERVAL_TICKS,
        -- Clamping the send buffer is only a fault if the clamp bites. A row
        -- that asks for one therefore has to observe it, or the row proves
        -- nothing beyond "the match still ran".
        expect_backpressure = options.buffered_amount_limit ~= nil,
        script = options.script or fault_harness.scripted_sample,
        notes = {},
    }

    local network_seed = options.network_seed or fault_harness.DEFAULT_NETWORK_SEED

    ---@param index integer
    ---@param peer_id string
    ---@param role SessionRole
    ---@param star FakeStarTransport
    ---@param legs string[]
    local function add(index, peer_id, role, star, legs)
        local fault = FaultTransport.new({
            transport = star,
            profile = harness.profile,
            -- One network seed per client, spread so two clients never replay
            -- the same impairment stream. The match seed is untouched.
            seed = network_seed + index * 101,
            legs = legs,
            poll_order = options.poll_order,
            duplicate_control_every = options.duplicate_control_every,
        })
        ---@type FaultHarnessClient
        local client = {
            index = index,
            peer_id = peer_id,
            role = role,
            star = star,
            fault = fault,
            tap = nil,
            recorder = nil,
            link = lobby_link.new(fault),
            coordinator = assert(coordinator.new({
                role = role,
                session_id = manifest.session_id,
                peer_id = peer_id,
                host_peer_id = role == "guest" and fault_harness.HOST_PEER_ID or nil,
                host_link_id = role == "guest" and fault_harness.HOST_PEER_ID or nil,
                runtime = protocol_fixture.runtime(),
                expectation = role == "guest" and expectation or nil,
            })),
            started = false,
            buffers = {},
            control = {},
            checkpoints = {},
            published = {},
            revoked = {},
            restored = 0,
            errors = {},
        }
        harness.clients[#harness.clients + 1] = client
    end

    ---@type string[]
    local guest_ids = {}
    for index = 2, #peer_ids do
        guest_ids[#guest_ids + 1] = peer_ids[index]
    end
    add(1, fault_harness.HOST_PEER_ID, "host", host_star, guest_ids)
    for index = 2, #peer_ids do
        local peer_id = peer_ids[index]
        local guest_star = FakeStarTransport.new({
            role = "guest",
            peer_id = peer_id,
            rendezvous = rendezvous,
            queue_limit = options.queue_limit,
            buffered_amount_limit = options.buffered_amount_limit,
        })
        assert(guest_star:initialize())
        assert(host_star:open_peer(peer_id))
        assert(host_star:link(guest_star))
        add(index, peer_id, "guest", guest_star, { fault_harness.HOST_PEER_ID })
    end

    if humans < shape.humans then
        note(
            harness,
            ("bot fill: %d of %d seats are humans; the rest are declared fills"):format(
                humans,
                shape.humans
            )
        )
    end
    return harness
end

-- ---------------------------------------------------------------------------
-- The pre-match lifecycle, over the real star
-- ---------------------------------------------------------------------------

---@param client FaultHarnessClient
---@param outcome CoordinatorOutcome
local function apply_outcome(client, outcome)
    if not outcome.accepted and outcome.reason then
        client.errors[#client.errors + 1] = outcome.reason
    end
    for _, action in ipairs(outcome.actions) do
        if action.kind == "send" then
            local wire = action.message and protocol.encode(action.message) or nil
            if wire == nil then
                client.errors[#client.errors + 1] = "a control message could not be encoded"
            else
                for _, target in ipairs(action.targets or {}) do
                    local ok, err = client.link:apply({
                        kind = "send",
                        link_id = target,
                        wire = wire,
                    })
                    if not ok and err then
                        client.errors[#client.errors + 1] = err
                    end
                end
            end
        elseif action.kind == "close" then
            client.link:apply({ kind = "close", link_id = assert(action.link_id) })
        elseif action.kind == "start_match" then
            client.started = true
        end
    end
end

---@param client FaultHarnessClient
---@param event table
local function dispatch(client, event)
    local state, outcome = coordinator.step(client.coordinator, event)
    client.coordinator = state
    apply_outcome(client, outcome)
end

-- Control envelopes still queued anywhere on the star. A clamped send buffer
-- drains a channel over several pumps, so "the exchange has settled" is a
-- question about the queues rather than a fixed round count.
---@param harness FaultHarness
---@return integer
local function pending_control(harness)
    local pending = 0
    for _, client in ipairs(harness.clients) do
        for _, peer in ipairs(client.star:diagnostics().peers) do
            pending = pending + peer.control.outbound_depth + peer.control.inbound_depth
        end
    end
    return pending
end

-- One control round: advance every impairment clock, let every client read its
-- mail, then flush the star. Control traffic is never impaired (a WebRTC control
-- channel is reliable and ordered), so this settles by construction -- but a
-- backpressure scenario clamps the send buffer on purpose, so the loop keeps
-- going while anything is still queued, up to a declared liveness bound.
---@param harness FaultHarness
---@param rounds integer?
function fault_harness.pump_control(harness, rounds)
    local minimum = rounds or 2
    for round = 1, fault_harness.MAX_CONTROL_ROUNDS do
        if round > minimum and pending_control(harness) == 0 then
            return
        end
        harness.transport_tick = harness.transport_tick + 1
        for _, client in ipairs(harness.clients) do
            client.fault:tick(harness.transport_tick)
        end
        for _, client in ipairs(harness.clients) do
            for _, event in ipairs(client.link:poll()) do
                if event.kind == "control" then
                    dispatch(client, {
                        kind = "control",
                        link_id = event.link_id,
                        wire = event.wire,
                    })
                elseif event.kind == "link_lost" then
                    dispatch(client, {
                        kind = "link_lost",
                        link_id = event.link_id,
                        code = "transport_lost",
                    })
                end
            end
        end
        harness.host_star:pump()
    end
end

-- Handshake through the frozen start boundary, using the same event vocabulary
-- the lobby screen uses and the same canonical wires.
---@param harness FaultHarness
---@param countdown_ticks integer?
---@param first_input_tick integer?
---@return boolean started
function fault_harness.reach_start(harness, countdown_ticks, first_input_tick)
    local host = harness.clients[1]
    for index = 2, #harness.clients do
        dispatch(harness.clients[index], { kind = "connect" })
    end
    fault_harness.pump_control(harness, 3)

    dispatch(host, { kind = "propose_manifest", manifest = harness.manifest })
    fault_harness.pump_control(harness, 3)

    local assignments =
        assert(coordinator.plan_assignments(harness.manifest, harness.peer_ids), "seating failed")
    dispatch(host, { kind = "assign_slots", assignments = assignments })
    fault_harness.pump_control(harness, 3)

    for _, client in ipairs(harness.clients) do
        dispatch(client, { kind = "set_ready", ready = true })
    end
    fault_harness.pump_control(harness, 3)

    local ticks = countdown_ticks or fault_harness.DEFAULT_COUNTDOWN_TICKS
    dispatch(host, {
        kind = "begin_countdown",
        countdown_id = fault_harness.COUNTDOWN_ID,
        remaining_ticks = ticks,
        first_input_tick = first_input_tick or 0,
    })
    fault_harness.pump_control(harness, 2)

    for _ = 1, ticks + fault_harness.MAX_CONTROL_ROUNDS do
        for _, client in ipairs(harness.clients) do
            dispatch(client, { kind = "tick" })
        end
        fault_harness.pump_control(harness, 2)
        local all = true
        for _, client in ipairs(harness.clients) do
            all = all and client.started
        end
        if all then
            return true
        end
    end
    return false
end

-- ---------------------------------------------------------------------------
-- The match
-- ---------------------------------------------------------------------------

-- Turn every frozen session into a running one. The request — including boundary
-- zero — is derived independently on each client from its own freeze.
---@param harness FaultHarness
function fault_harness.start_match(harness)
    for _, client in ipairs(harness.clients) do
        local state = client.coordinator
        local freeze = assert(state.freeze, "a client never froze a session")
        local manifest = assert(state.manifest)
        local request = assert(match_session.request({
            role = state.role,
            peer_id = state.peer_id,
            manifest = manifest,
            freeze = freeze,
        }))
        client.request = request
        -- The recorder needs the freeze, so the diagnostic tap can only exist
        -- once the session is frozen. Before that the link talks to the
        -- impairment decorator directly.
        local recorder = net_diagnostics.new({
            role = state.role,
            peer_id = state.peer_id,
            manifest = manifest,
            freeze = freeze,
            input_delay_ticks = match_driver.DELAY_TICKS,
            hash_interval_ticks = harness.hash_interval_ticks,
            max_rollback_ticks = 0,
            export_opt_in = true,
        })
        client.recorder = recorder
        client.tap = DiagnosticTransport.new({
            transport = client.fault,
            recorder = recorder,
            peer_id = state.peer_id,
            first_input_tick = freeze.first_input_tick,
            input_delay_ticks = match_driver.DELAY_TICKS,
            clock = function()
                return harness.clock_ms
            end,
            transport_tick = function()
                return harness.step + match_driver.DELAY_TICKS
            end,
            retain_wires = 0,
        })
        client.driver = match_driver.new({
            role = state.role,
            peer_id = state.peer_id,
            freeze = freeze,
            manifest = manifest,
            transport = client.tap,
            initial_snapshot = request.initial_snapshot,
            hash_interval_ticks = harness.hash_interval_ticks,
        })
        client.presentation =
            match_presentation.new(request.initial_snapshot, request.first_input_tick)
        client.model = online_match_model.new(request, state)
    end
end

---@param client FaultHarnessClient
---@param effects OnlineMatchEffect[]
local function apply_effects(client, effects)
    for _, effect in ipairs(effects) do
        if effect.kind == "send" then
            local ok, err = client.link:send(effect.link_id, effect.wire)
            if not ok and err then
                client.errors[#client.errors + 1] = err
            end
        elseif effect.kind == "close" then
            client.link:apply({ kind = "close", link_id = effect.link_id })
        elseif effect.kind == "observe_hash" then
            match_driver.observe_checkpoint(assert(client.driver), effect.tick, effect.hash)
        end
    end
end

---@param client FaultHarnessClient
---@param event table
local function command(client, event)
    local model, effects = online_match_model.command(assert(client.model), event)
    client.model = model
    apply_effects(client, effects)
end

-- Control traffic shares the transport with input: the driver keeps the input
-- envelopes and hands the rest back on its batch. Once it is terminal it stops
-- polling entirely, so from that point the star is drained directly into the
-- *same* reassembly buffers — a wire whose frames straddle the final tick must
-- not land in two buffer sets.
---@param client FaultHarnessClient
local function drain_control(client)
    if match_driver.status(assert(client.driver)) ~= "active" then
        for _, entry in ipairs(client.fault:poll_batch()) do
            if entry.channel == "control" then
                client.control[#client.control + 1] = entry
            end
        end
        local event = client.fault:poll_event()
        while event do
            if
                event.kind == "peer_state"
                and event.peer_id
                and (
                    event.state == "closed"
                    or event.state == "error"
                    or event.state == "disconnected"
                )
            then
                command(client, { kind = "link_lost", link_id = event.peer_id })
            end
            event = client.fault:poll_event()
        end
    end
    local entries = client.control
    client.control = {}
    for _, entry in ipairs(entries) do
        if entry.channel == "control" then
            local buffer = client.buffers[entry.peer_id] or lobby_link.new_buffer()
            client.buffers[entry.peer_id] = buffer
            local wire, err = lobby_link.absorb(buffer, entry.message.payload)
            if wire then
                command(client, { kind = "control", link_id = entry.peer_id, wire = wire })
            elseif err then
                client.buffers[entry.peer_id] = lobby_link.new_buffer()
                client.errors[#client.errors + 1] = err
            end
        end
    end
end

-- A correction can revoke an event and a *later* correction can put it back:
-- the timeline that removed it was itself corrected away. Such an event was
-- never "corrected away" in the sense the acceptance criterion means, so an
-- add clears the revocation rather than leaving the id permanently poisoned.
-- Restorations are counted so the report can say how often it happened instead
-- of hiding the subtlety behind a green tick.
---@param client FaultHarnessClient
---@param presented RollbackPlayableLabBatch
local function fold_presentation(client, presented)
    for _, diff in ipairs(presented.event_diffs) do
        for _, event in ipairs(diff.revoked) do
            client.revoked[event.id] = true
        end
        for _, replacement in ipairs(diff.replaced) do
            client.revoked[replacement.before.id] = true
        end
        for _, event in ipairs(diff.added) do
            if client.revoked[event.id] then
                client.revoked[event.id] = nil
                client.restored = client.restored + 1
            end
        end
        for _, replacement in ipairs(diff.replaced) do
            if client.revoked[replacement.after.id] then
                client.revoked[replacement.after.id] = nil
                client.restored = client.restored + 1
            end
        end
    end
    for _, step in ipairs(presented.confirmed_steps) do
        for _, event in ipairs(step.match_events) do
            client.published[event.id] = (client.published[event.id] or 0) + 1
        end
        for _, event in ipairs(step.lifecycle_events) do
            client.published[event.id] = (client.published[event.id] or 0) + 1
        end
        for _, event in ipairs(step.combat_events or {}) do
            client.published[event.id] = (client.published[event.id] or 0) + 1
        end
    end
end

---@param client FaultHarnessClient
local function report_driver(client)
    local driver = assert(client.driver)
    local terminal = match_driver.terminal(driver)
    if terminal == nil or client.final_hash ~= nil then
        return
    end
    local snapshot = match_driver.current_snapshot(driver)
    client.final_hash = match_snapshot.hash(snapshot)
    if terminal.status == "completed" then
        local state = snapshot.state
        command(client, {
            kind = "full_time",
            final_tick = state.input_tick + assert(client.request).first_input_tick,
            home_score = state.score.home,
            away_score = state.score.away,
            final_hash = client.final_hash,
        })
        return
    end
    command(client, {
        kind = "failure",
        status = terminal.status,
        failure = terminal.failure,
        detail = terminal.detail,
    })
end

-- One fixed 60 Hz step for every client: impair, advance, present, fold
-- diagnostics, exchange session mail, flush the star.
---@param harness FaultHarness
---@return MatchDriverBatch[]
function fault_harness.advance(harness)
    harness.transport_tick = harness.transport_tick + 1
    ---@type MatchDriverBatch[]
    local batches = {}
    for _, client in ipairs(harness.clients) do
        client.fault:tick(harness.transport_tick)
    end
    for index, client in ipairs(harness.clients) do
        local driver = assert(client.driver)
        local sample = harness.script(index, harness.step)
        local batch = match_driver.advance(driver, sample)
        batches[index] = batch
        for _, entry in ipairs(batch.control) do
            client.control[#client.control + 1] = entry
        end
        for _, checkpoint in ipairs(batch.checkpoints) do
            client.checkpoints[#client.checkpoints + 1] = checkpoint
        end
        local presented = match_presentation.consume(assert(client.presentation), driver, batch)
        fold_presentation(client, presented)
        -- Reading driver diagnostics also reads the tap's transport diagnostics,
        -- which is what folds the star's counters into the recorder.
        net_diagnostics.record_step(client.recorder, match_driver.diagnostics(driver), batch)
        if #presented.confirmed_steps > 0 then
            command(client, { kind = "confirmed", steps = presented.confirmed_steps })
        end
        if #batch.checkpoints > 0 then
            command(client, { kind = "checkpoints", checkpoints = batch.checkpoints })
        end
    end
    for _, client in ipairs(harness.clients) do
        drain_control(client)
        report_driver(client)
        command(client, { kind = "tick", dt = 1 / 60 })
    end
    harness.host_star:pump()
    harness.step = harness.step + 1
    harness.clock_ms = harness.clock_ms + fault_harness.CLOCK_STEP_MS
    return batches
end

-- Every driver has stopped. This is deliberately *not* the same question as
-- "every session ended": a guest that has not finished settling still accepts
-- the host's acknowledged result, so a loop that stopped at the session boundary
-- would tear a straggler down mid-settle and then report the final hash it never
-- produced as a divergence.
---@param harness FaultHarness
---@return boolean
function fault_harness.all_drivers_terminal(harness)
    for _, client in ipairs(harness.clients) do
        if client.driver == nil or match_driver.status(client.driver) == "active" then
            return false
        end
    end
    return true
end

-- The whole run is finished when every driver has stopped *and* every session
-- has ended.
---@param harness FaultHarness
---@return boolean
function fault_harness.finished(harness)
    return fault_harness.all_drivers_terminal(harness) and fault_harness.all_ended(harness)
end

---@param harness FaultHarness
---@return boolean
function fault_harness.all_ended(harness)
    for _, client in ipairs(harness.clients) do
        if client.model == nil or not online_match_model.ended(client.model) then
            return false
        end
    end
    return true
end

-- ---------------------------------------------------------------------------
-- Comparison and the deterministic marker set
-- ---------------------------------------------------------------------------

---@class FaultHarnessFinding
---@field id string
---@field ok boolean
---@field skipped boolean
---@field detail string

---@class FaultHarnessReport
---@field mode SessionMatchMode
---@field profile NetworkProfileName
---@field clients integer
---@field steps integer
---@field findings FaultHarnessFinding[]
---@field markers string[] -- Ordered, deterministic, comparable across processes.
---@field notes string[]
---@field ok boolean

-- Declared resource gates. A run that exceeds one produces a blocking finding
-- rather than a warning: the issue asks for bounded resources or an evidence
-- failure, and "we noticed but continued" is neither.
fault_harness.GATES = {
    max_rollback_depth = rollback_input_history.ROLLBACK_WINDOW_TICKS,
    max_channel_depth = transport_contract.MAX_QUEUE_LIMIT,
    max_residual_queue = 0,
    max_orphan_peers = 0,
    max_overflow = 0,
}

---@param live table<string, InputSlotId>
---@return string
local function live_marker(live)
    ---@type string[]
    local keys = {}
    for producer_id in pairs(live) do
        keys[#keys + 1] = producer_id
    end
    table.sort(keys)
    ---@type string[]
    local parts = {}
    for _, producer_id in ipairs(keys) do
        parts[#parts + 1] = producer_id .. "=" .. tostring(live[producer_id])
    end
    return table.concat(parts, ",")
end

---@param findings FaultHarnessFinding[]
---@param id string
---@param ok boolean
---@param detail string
local function finding(findings, id, ok, detail)
    findings[#findings + 1] = { id = id, ok = ok, skipped = false, detail = detail }
end

---@param findings FaultHarnessFinding[]
---@param id string
---@param detail string
local function skipped(findings, id, detail)
    findings[#findings + 1] = { id = id, ok = true, skipped = true, detail = detail }
end

-- Pairwise checkpoint comparison. Two clients are compared only at boundaries
-- *both* hashed: a peer that has not reached a checkpoint has not disagreed
-- about it, and treating a missing boundary as a mismatch would report a
-- delivery difference as a desync.
---@param harness FaultHarness
---@param findings FaultHarnessFinding[]
local function compare_checkpoints(harness, findings)
    local compared, hash_bad, live_bad = 0, 0, 0
    ---@type string[]
    local detail = {}
    for left = 1, #harness.clients do
        for right = left + 1, #harness.clients do
            local a, b = harness.clients[left], harness.clients[right]
            ---@type table<integer, MatchDriverCheckpoint>
            local by_tick = {}
            for _, checkpoint in ipairs(b.checkpoints) do
                by_tick[checkpoint.tick] = checkpoint
            end
            for _, checkpoint in ipairs(a.checkpoints) do
                local other = by_tick[checkpoint.tick]
                if other ~= nil then
                    compared = compared + 1
                    if other.hash ~= checkpoint.hash then
                        hash_bad = hash_bad + 1
                        if #detail < 4 then
                            detail[#detail + 1] = ("hash %s/%s@%d"):format(
                                a.peer_id,
                                b.peer_id,
                                checkpoint.tick
                            )
                        end
                    end
                    if live_marker(other.live) ~= live_marker(checkpoint.live) then
                        live_bad = live_bad + 1
                        if #detail < 8 then
                            detail[#detail + 1] = ("live %s/%s@%d"):format(
                                a.peer_id,
                                b.peer_id,
                                checkpoint.tick
                            )
                        end
                    end
                end
            end
        end
    end
    finding(
        findings,
        "converge.checkpoint_hash",
        hash_bad == 0 and compared > 0,
        ("compared %d shared checkpoints, %d disagreed %s"):format(
            compared,
            hash_bad,
            table.concat(detail, " ")
        )
    )
    -- 4v4 cannot exhibit a live-slot divergence at all: singleton owned sets make
    -- every branch of `next_live_slot` return the slot already live. The check is
    -- still run there, and it is *declared* as inert rather than counted as
    -- coverage it does not provide.
    if harness.mode == "4v4" then
        skipped(
            findings,
            "converge.live_slot",
            ("%d comparisons ran, but 4v4 owned sets are singletons so switching is inert"):format(
                compared
            )
        )
    else
        finding(
            findings,
            "converge.live_slot",
            live_bad == 0 and compared > 0,
            ("compared %d shared checkpoints, %d live-slot maps disagreed"):format(
                compared,
                live_bad
            )
        )
    end
end

---@param harness FaultHarness
---@param findings FaultHarnessFinding[]
local function compare_result(harness, findings)
    local reference = harness.clients[1]
    local hash_ok, result_ok = true, true
    ---@type string[]
    local detail = {}
    for _, client in ipairs(harness.clients) do
        if client.final_hash ~= reference.final_hash then
            hash_ok = false
            detail[#detail + 1] = client.peer_id .. " final hash differs"
        end
        local left = reference.model and reference.model.result or nil
        local right = client.model and client.model.result or nil
        if left == nil or right == nil then
            result_ok = false
            detail[#detail + 1] = client.peer_id .. " has no acknowledged result"
        elseif
            left.home_score ~= right.home_score
            or left.away_score ~= right.away_score
            or left.final_tick ~= right.final_tick
            or left.final_hash ~= right.final_hash
        then
            result_ok = false
            detail[#detail + 1] = client.peer_id .. " acknowledged a different result"
        end
    end
    finding(
        findings,
        "converge.final_hash",
        hash_ok and reference.final_hash ~= nil,
        table.concat(detail, "; ")
    )
    finding(findings, "converge.result", result_ok, table.concat(detail, "; "))
end

---@param harness FaultHarness
---@param findings FaultHarnessFinding[]
local function compare_presentation(harness, findings)
    local duplicates, resurrected, published, restored = 0, 0, 0, 0
    for _, client in ipairs(harness.clients) do
        restored = restored + client.restored
        for id, count in pairs(client.published) do
            published = published + 1
            if count > 1 then
                duplicates = duplicates + 1
            end
            if client.revoked[id] then
                resurrected = resurrected + 1
            end
        end
    end
    finding(
        findings,
        "presentation.published_once",
        duplicates == 0,
        ("%d confirmed events across %d clients, %d published more than once"):format(
            published,
            #harness.clients,
            duplicates
        )
    )
    finding(
        findings,
        "presentation.no_revoked_survivor",
        resurrected == 0,
        ("%d corrected-away events survived into a confirmed publication; "):format(resurrected)
            .. ("%d revocations were themselves corrected away and restored"):format(restored)
    )
end

---@param harness FaultHarness
---@param findings FaultHarnessFinding[]
---@param expect_completed boolean
local function compare_status(harness, findings, expect_completed)
    ---@type string[]
    local statuses = {}
    local completed, settle_timeouts = 0, 0
    for _, client in ipairs(harness.clients) do
        local status = client.driver and match_driver.status(client.driver) or "unstarted"
        statuses[#statuses + 1] = client.peer_id .. "=" .. status
        if status == "completed" then
            completed = completed + 1
        elseif status == "settle_timeout" then
            settle_timeouts = settle_timeouts + 1
        end
    end
    -- A false `settle_timeout` on a healthy match is the #237 defect inverted, so
    -- it is asserted separately from "everyone completed" and under every
    -- supported profile. It is asserted only on *healthy* rows: a scenario that
    -- deliberately kills the host strands the guests' tails by construction, and
    -- calling that a false settle timeout would make the check meaningless.
    if expect_completed then
        finding(
            findings,
            "lifecycle.no_settle_timeout",
            settle_timeouts == 0,
            ("%d peers timed out settling: %s"):format(settle_timeouts, table.concat(statuses, " "))
        )
    else
        skipped(
            findings,
            "lifecycle.no_settle_timeout",
            "an injected terminal fault strands the tail by construction: "
                .. table.concat(statuses, " ")
        )
    end
    if expect_completed then
        finding(
            findings,
            "lifecycle.completed",
            completed == #harness.clients,
            table.concat(statuses, " ")
        )
    else
        skipped(
            findings,
            "lifecycle.completed",
            "this scenario injects a terminal fault: " .. table.concat(statuses, " ")
        )
    end
end

-- Resource evidence comes from #168's recorder, not from a second observation
-- path of the harness's own. One detail is load bearing: queue **depth** is read
-- from `runtime.pressure`, the running peak across every transport snapshot, and
-- never from `runtime.peers[*]`. The peers array is the *last* snapshot, and the
-- last snapshot of a finished run is a quiescent one, so a gate written against
-- it reads zero however hard the transport was pushed. A blocking gate that
-- cannot fail is worse than no gate, so this one is checked for liveness too.
---@param harness FaultHarness
---@param findings FaultHarnessFinding[]
---@param markers string[]
local function measure_resources(harness, findings, markers)
    local worst_depth, worst_channel, residual, orphans = 0, 0, 0, 0
    local packets, bytes = 0, 0
    local backpressure, overflow, samples = 0, 0, 0
    for _, client in ipairs(harness.clients) do
        local recorder = client.recorder
        if recorder ~= nil then
            local artifact = net_diagnostics.export(recorder)
            if artifact == nil then
                finding(findings, "resources.export", false, client.peer_id .. " exported nothing")
            else
                local simulation = artifact.canonical.simulation
                worst_depth = math.max(worst_depth, simulation.max_rollback_depth)
                local delivery = artifact.canonical.delivery
                packets = packets + delivery.sent + delivery.arrived
                for _, packet in ipairs(delivery.packets) do
                    bytes = bytes + packet.payload_bytes
                end
                local pressure = artifact.runtime.pressure
                if pressure == nil then
                    finding(
                        findings,
                        "resources.pressure",
                        false,
                        client.peer_id .. " folded no transport snapshot at all"
                    )
                else
                    samples = samples + pressure.samples
                    worst_channel = math.max(
                        worst_channel,
                        pressure.peak_outbound_depth,
                        pressure.peak_inbound_depth
                    )
                    backpressure = backpressure + pressure.backpressure
                    overflow = overflow + pressure.overflow
                    markers[#markers + 1] = (
                        "pressure %s samples=%d outbound=%d inbound=%d buffered=%d "
                        .. "backpressure=%d peer_backpressure=%d overflow=%d "
                        .. "dropped_out=%d dropped_in=%d"
                    ):format(
                        client.peer_id,
                        pressure.samples,
                        pressure.peak_outbound_depth,
                        pressure.peak_inbound_depth,
                        pressure.peak_buffered_amount,
                        pressure.backpressure,
                        pressure.peer_backpressure,
                        pressure.overflow,
                        pressure.dropped_outbound,
                        pressure.dropped_inbound
                    )
                end
                local teardown = artifact.runtime.teardown
                if teardown ~= nil then
                    residual = residual + teardown.residual_outbound + teardown.residual_inbound
                    orphans = orphans + #teardown.orphaned_peers
                end
                markers[#markers + 1] = (
                    "resource %s rollbacks=%d corrections=%d depth=%d "
                    .. "sent=%d arrived=%d deferred=%d duplicate=%d rejected=%d applied=%d"
                ):format(
                    client.peer_id,
                    simulation.rollback_count,
                    simulation.correction_count,
                    simulation.max_rollback_depth,
                    delivery.sent,
                    delivery.arrived,
                    delivery.deferred,
                    delivery.duplicate,
                    delivery.rejected,
                    delivery.applied_rows
                )
            end
        end
    end
    finding(
        findings,
        "resources.rollback_depth",
        worst_depth <= fault_harness.GATES.max_rollback_depth,
        ("worst rollback depth %d against a gate of %d"):format(
            worst_depth,
            fault_harness.GATES.max_rollback_depth
        )
    )
    finding(
        findings,
        "resources.channel_depth",
        worst_channel <= fault_harness.GATES.max_channel_depth,
        ("peak channel depth %d over %d transport snapshots, against a gate of %d"):format(
            worst_channel,
            samples,
            fault_harness.GATES.max_channel_depth
        )
    )
    -- The gate above is only evidence if the quantity it gates was ever
    -- non-zero. This is the check that would have caught the earlier version,
    -- which read the last (quiescent) snapshot and reported zero everywhere.
    finding(
        findings,
        "resources.channel_depth_observed",
        worst_channel > 0 and samples > 0,
        ("the depth gate observed a peak of %d over %d snapshots; a zero peak would "):format(
            worst_channel,
            samples
        ) .. "mean the gate cannot fail"
    )
    finding(
        findings,
        "resources.no_overflow",
        overflow <= fault_harness.GATES.max_overflow,
        ("%d refused sends (star overflow) against a gate of %d"):format(
            overflow,
            fault_harness.GATES.max_overflow
        )
    )
    -- A clamped send buffer that never latched is a scenario that silently
    -- turned itself off.
    if harness.expect_backpressure then
        finding(
            findings,
            "faults.backpressure_observed",
            backpressure > 0,
            ("the clamped send buffer latched %d times across %d clients"):format(
                backpressure,
                #harness.clients
            )
        )
    else
        skipped(
            findings,
            "faults.backpressure_observed",
            ("this row does not clamp the send buffer; %d latches observed anyway"):format(
                backpressure
            )
        )
    end
    finding(
        findings,
        "teardown.clean",
        residual <= fault_harness.GATES.max_residual_queue
            and orphans <= fault_harness.GATES.max_orphan_peers,
        ("%d residual queued messages, %d orphaned peers"):format(residual, orphans)
    )
    markers[#markers + 1] = ("resource total packets=%d bytes=%d"):format(packets, bytes)
end

-- The contingent rows. They exist, they are named, and they are skipped with a
-- reason -- an absent row reads as "not applicable" and a silent pass reads as
-- "covered".
---@param findings FaultHarnessFinding[]
local function declare_contingent(findings)
    for _, phase in ipairs({
        "wind_up",
        "guard",
        "contact",
        "projectile_flight",
        "stagger",
        "ball_spill",
        "immunity_expiry",
    }) do
        skipped(
            findings,
            "combat.correction." .. phase,
            "blocked on #112: the pre-#112 bot never drives the companion into this phase, "
                .. "so a passing row here would pin an absence"
        )
    end
    skipped(
        findings,
        "combat.default_disposition",
        "blocked on #114: the manifest carries a placeholder combat disposition"
    )
    skipped(
        findings,
        "browser.multi_context",
        "not covered by this tier: the browser multi-context bridge is exercised by "
            .. "scripts/browser_matrix.py and by #170, and no claim about it is made here"
    )
end

-- Build the full report. `expect_completed` is false for scenarios that inject a
-- deliberately terminal fault, where "nobody completed" is the expected outcome
-- rather than a failure.
---@param harness FaultHarness
---@param expect_completed boolean?
---@return FaultHarnessReport
function fault_harness.report(harness, expect_completed)
    ---@type FaultHarnessFinding[]
    local findings = {}
    ---@type string[]
    local markers = {}

    markers[#markers + 1] = ("scenario mode=%s profile=%s clients=%d steps=%d"):format(
        harness.mode,
        harness.profile,
        #harness.clients,
        harness.step
    )
    for _, client in ipairs(harness.clients) do
        local status = client.driver and match_driver.status(client.driver) or "unstarted"
        local terminal = client.driver and match_driver.terminal(client.driver) or nil
        local diagnostics = client.driver and match_driver.diagnostics(client.driver) or nil
        markers[#markers + 1] = ("client %s status=%s terminal=%s final=%s confirmed_output=%s full_time=%s settle_steps=%s"):format(
            client.peer_id,
            status,
            terminal and terminal.status or "-",
            client.final_hash or "-",
            diagnostics and tostring(diagnostics.confirmed_output_tick) or "-",
            diagnostics and tostring(diagnostics.full_time_boundary) or "-",
            diagnostics and tostring(diagnostics.settle_steps) or "-"
        )
        for _, checkpoint in ipairs(client.checkpoints) do
            markers[#markers + 1] = ("checkpoint %s %d %s %s"):format(
                client.peer_id,
                checkpoint.tick,
                checkpoint.hash,
                live_marker(checkpoint.live)
            )
        end
        local counters = client.fault:counters()
        local model = client.fault:model_diagnostics()
        markers[#markers + 1] = (
            "impairment %s profile=%s scheduled=%d delivered=%d dropped=%d "
            .. "independent=%d burst=%d duplicated=%d reordered=%d withheld=%d held=%d "
            .. "control=%d control_dup=%d pending=%d peak_pending=%d peak_retained=%d"
        ):format(
            client.peer_id,
            counters.profile,
            counters.scheduled,
            counters.delivered,
            counters.dropped,
            counters.independent_lost,
            counters.burst_lost,
            counters.duplicated,
            counters.reordered,
            counters.withheld,
            counters.held,
            counters.control_delivered,
            counters.control_duplicated,
            counters.pending,
            model.peak_pending_envelopes,
            model.peak_retained_authoritative_records
        )
    end

    local started = true
    for _, client in ipairs(harness.clients) do
        started = started and client.started
    end
    finding(findings, "lifecycle.started", started, "every client reached the frozen start")
    finding(
        findings,
        "lifecycle.ended",
        fault_harness.all_ended(harness),
        "every client's session reached a terminal phase"
    )
    compare_status(harness, findings, expect_completed ~= false)
    compare_checkpoints(harness, findings)
    if expect_completed ~= false then
        compare_result(harness, findings)
    else
        skipped(findings, "converge.final_hash", "a terminal fault scenario has no agreed result")
        skipped(findings, "converge.result", "a terminal fault scenario has no agreed result")
    end
    compare_presentation(harness, findings)
    measure_resources(harness, findings, markers)
    declare_contingent(findings)

    local ok = true
    for _, entry in ipairs(findings) do
        ok = ok and entry.ok
    end
    return {
        mode = harness.mode,
        profile = harness.profile,
        clients = #harness.clients,
        steps = harness.step,
        findings = findings,
        markers = markers,
        notes = harness.notes,
        ok = ok,
    }
end

-- ---------------------------------------------------------------------------
-- Teardown
-- ---------------------------------------------------------------------------

---@param harness FaultHarness
function fault_harness.teardown(harness)
    for _, client in ipairs(harness.clients) do
        if client.tap then
            client.tap:shutdown()
        else
            client.fault:shutdown()
        end
    end
end

return fault_harness
