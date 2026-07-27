local coordinator = require("game.online.coordinator")
local fixture = require("game.online.coordinator_fixture")
local protocol = require("game.online.protocol")
local input_frame = require("sim.input_frame")
local t = require("spec.support.runner")

local SESSION = fixture.manifest().session_id
local HOST = fixture.HOST_PEER_ID

---@param value any
---@return any
local function deep_copy(value)
    if type(value) ~= "table" then
        return value
    end
    local result = {}
    for key, item in pairs(value) do
        result[key] = deep_copy(item)
    end
    return result
end

---@param kind SessionMessageKind
---@param peer_id string
---@param sequence integer
---@param body SessionMessageBody
---@return SessionControlMessage
local function message(kind, peer_id, sequence, body)
    return assert(protocol.new(kind, SESSION, peer_id, sequence, body))
end

---@param peer_id string
---@param sequence integer
---@param role SessionRole?
---@return SessionControlMessage
local function handshake(peer_id, sequence, role)
    return message("handshake", peer_id, sequence, {
        role = role or "guest",
        runtime = fixture.runtime(),
    })
end

---@param state CoordinatorState
---@param peer_id string
---@param control SessionControlMessage
---@return CoordinatorState, CoordinatorOutcome
local function deliver(state, peer_id, control)
    return coordinator.step(state, {
        kind = "control",
        link_id = fixture.link_id(peer_id),
        message = control,
    })
end

-- Host that has admitted `guest_count` guests, proposed the manifest, seen
-- every acceptance, and published canonical ownership.
---@param guest_count integer
---@return CoordinatorState, integer[]
local function assigned_host(guest_count)
    local state = fixture.host()
    ---@type integer[]
    local sequences = {}
    for index = 1, guest_count do
        local peer_id = fixture.guest_peer_id(index)
        state = (deliver(state, peer_id, handshake(peer_id, 0)))
        sequences[index] = 0
    end
    state = (coordinator.step(state, { kind = "propose_manifest", manifest = fixture.manifest() }))
    local manifest_id = assert(state.manifest_id)
    for index = 1, guest_count do
        local peer_id = fixture.guest_peer_id(index)
        sequences[index] = sequences[index] + 1
        state = (
            deliver(
                state,
                peer_id,
                message("manifest_accept", peer_id, sequences[index], { manifest_id = manifest_id })
            )
        )
    end
    state = (
        coordinator.step(state, {
            kind = "assign_slots",
            assignments = fixture.assignments(guest_count),
        })
    )
    return state, sequences
end

---@param state CoordinatorState
---@param guest_count integer
---@param sequences integer[]
---@return CoordinatorState
local function ready_host(state, guest_count, sequences)
    local manifest_id = assert(state.manifest_id)
    for index = 1, guest_count do
        local peer_id = fixture.guest_peer_id(index)
        -- A real guest answers a published assignment with an explicit negative
        -- readiness first; that falling edge is what binds its later positive
        -- readiness to this ownership generation.
        for _, ready in ipairs({ false, true }) do
            sequences[index] = sequences[index] + 1
            state = (
                deliver(
                    state,
                    peer_id,
                    message(
                        "ready",
                        peer_id,
                        sequences[index],
                        { manifest_id = manifest_id, ready = ready }
                    )
                )
            )
        end
    end
    return (coordinator.step(state, { kind = "set_ready", ready = true }))
end

t.describe("online coordinator construction", function()
    t.it("admits the host at construction and leaves a guest unheard", function()
        local host = fixture.host()
        t.eq(host.role, "host")
        t.eq(host.phase, "handshake")
        t.eq(#host.peers, 1)
        t.eq(host.peers[1].peer_id, HOST)
        t.eq(host.sequence, 0)

        local guest = fixture.guest(1)
        t.eq(guest.phase, "new")
        t.eq(#guest.peers, 2)
        t.eq(guest.peers[2].peer_id, HOST)
        t.eq(guest.peers[2].link_id, fixture.link_id("guest.1"))
    end)

    t.it("refuses malformed or contradictory identities", function()
        local runtime = fixture.runtime()
        ---@type any
        local unknown_role = {
            role = "spectator",
            session_id = SESSION,
            peer_id = "x",
            runtime = runtime,
        }
        local _, _, code = coordinator.new(unknown_role)
        t.eq(code, "malformed")

        _, _, code = coordinator.new({
            role = "guest",
            session_id = SESSION,
            peer_id = "g",
            host_peer_id = HOST,
            runtime = runtime,
        })
        t.eq(code, "malformed")

        _, _, code = coordinator.new({
            role = "guest",
            session_id = SESSION,
            peer_id = HOST,
            host_peer_id = HOST,
            host_link_id = "link.a",
            runtime = runtime,
        })
        t.eq(code, "role_conflict")

        _, _, code = coordinator.new({
            role = "host",
            session_id = "not a session id",
            peer_id = HOST,
            runtime = runtime,
        })
        t.eq(code, "malformed")
    end)

    t.it("names every lifecycle phase exactly once", function()
        local seen = {}
        for _, phase in ipairs(coordinator.PHASES) do
            t.is_true(not seen[phase], "phase " .. phase .. " is listed twice")
            seen[phase] = true
        end
        t.eq(#coordinator.PHASES, 9)
        t.is_true(seen.new and seen.terminal and seen.countdown and seen.running)
    end)

    t.it("maps every terminal reason to a closed protocol code", function()
        for _, reason in ipairs({
            "local_abort",
            "peer_abort",
            "guest_left",
            "host_left",
            "removed",
            "transport_lost",
            "protocol_violation",
            "manifest_mismatch",
            "invalid_assignment",
            "start_ack_timeout",
            "input_channel_failure",
            "late_input",
            "hash_mismatch",
        }) do
            local code = coordinator.TERMINAL_CODES[reason]
            t.is_true(code ~= nil, reason .. " has no protocol code")
            t.is_true(
                protocol.new("abort", SESSION, HOST, 0, { code = code }) ~= nil,
                reason .. " maps to an invalid abort code"
            )
        end
        t.eq(coordinator.TERMINAL_CODES.completed, nil)
    end)
end)

t.describe("online coordinator slot ownership", function()
    t.it("gives all eight canonical slots exactly one declared source", function()
        local manifest = fixture.manifest()
        for guest_count = 0, coordinator.MAX_GUESTS do
            local assignments =
                assert(coordinator.plan_assignments(manifest, fixture.peer_ids(guest_count)))
            local sources = assert(coordinator.slot_sources(manifest, assignments))
            local peers, bots, ids = 0, 0, {}
            for index = 1, input_frame.SLOT_COUNT do
                local slot = assert(input_frame.slot(index))
                local producer = assert(sources[slot.id])
                t.eq(producer.slot, slot.id)
                t.eq(producer.team, slot.team)
                t.is_true(not ids[producer.producer_id], "producer ids collide")
                ids[producer.producer_id] = true
                if producer.producer_kind == "peer" then
                    peers = peers + 1
                    t.eq(producer.bot_seed, nil)
                else
                    bots = bots + 1
                    t.is_true(producer.bot_seed ~= nil)
                end
            end
            t.eq(peers, guest_count + 1)
            t.eq(bots, input_frame.SLOT_COUNT - guest_count - 1)
        end
    end)

    t.it("never seats a combat-protected keeper", function()
        local manifest = fixture.manifest()
        local keepers = {}
        for _, team in ipairs(manifest.teams) do
            for _, player in ipairs(team.roster) do
                if player.position == "keeper" then
                    keepers[player.player_id] = true
                end
            end
        end
        t.eq(manifest.slots[1].player_id ~= nil, true)
        for _, slot in ipairs(manifest.slots) do
            t.is_true(not keepers[slot.player_id], "a manifest slot named a keeper")
        end

        local assignments = fixture.assignments(1)
        assignments[1].player_id = manifest.teams[1].roster[1].player_id
        local sources, err, code = coordinator.slot_sources(manifest, assignments)
        t.eq(sources, nil)
        t.eq(code, "invalid_assignment")
        t.is_true(assert(err):find("keepers") ~= nil)
    end)

    t.it("derives bot seeds deterministically from the frozen manifest seed", function()
        local manifest = fixture.manifest()
        local first = assert(coordinator.plan_assignments(manifest, { HOST }))
        local second = assert(coordinator.plan_assignments(manifest, { HOST }))
        for index = 2, input_frame.SLOT_COUNT do
            t.eq(first[index].bot_seed, second[index].bot_seed)
            t.eq(
                first[index].bot_seed,
                (manifest.seed + index * coordinator.BOT_SEED_STRIDE) % (protocol.MAX_SEED + 1)
            )
        end
    end)

    t.it("refuses to seat more humans than canonical slots", function()
        local ids = {}
        for index = 1, input_frame.SLOT_COUNT + 1 do
            ids[index] = "peer." .. tostring(index)
        end
        local _, _, code = coordinator.plan_assignments(fixture.manifest(), ids)
        t.eq(code, "capacity")

        _, _, code = coordinator.plan_assignments(fixture.manifest(), { HOST, HOST })
        t.eq(code, "duplicate_peer")
    end)
end)

t.describe("online coordinator admission", function()
    t.it("admits guests with stable identities", function()
        local state = fixture.host()
        local next_state, outcome = deliver(state, "guest.1", handshake("guest.1", 0))
        t.eq(outcome.disposition, "applied")
        t.eq(#next_state.peers, 2)
        t.eq(next_state.peers[2].peer_id, "guest.1")
        t.eq(next_state.peers[2].link_id, fixture.link_id("guest.1"))
        t.eq(next_state.phase, "handshake")
        t.eq(state.phase, "handshake", "the original state is untouched")
        t.eq(#state.peers, 1)
    end)

    t.it("refuses duplicate, over-capacity, role, and runtime violations per link", function()
        local state = fixture.host()
        for index = 1, coordinator.MAX_GUESTS do
            local peer_id = fixture.guest_peer_id(index)
            state = (deliver(state, peer_id, handshake(peer_id, 0)))
        end
        t.eq(#state.peers, coordinator.MAX_PEERS)

        local overflow, outcome = deliver(state, "guest.8", handshake("guest.8", 0))
        t.eq(outcome.code, "capacity")
        t.eq(#overflow.peers, coordinator.MAX_PEERS)
        t.eq(overflow.phase, "handshake")
        t.eq(outcome.actions[1].kind, "send")
        t.eq(outcome.actions[1].message.body.code, "capacity")
        t.eq(outcome.actions[2].kind, "close")

        local duplicate
        duplicate, outcome = coordinator.step(state, {
            kind = "control",
            link_id = fixture.link_id("guest.8"),
            message = handshake("guest.1", 0),
        })
        t.eq(outcome.code, "protocol_mismatch")
        t.eq(#duplicate.peers, coordinator.MAX_PEERS)

        -- The same handshake replayed on its own link is a plain duplicate.
        local replay
        replay, outcome = deliver(state, "guest.1", handshake("guest.1", 0))
        t.eq(outcome.disposition, "idempotent")
        t.is_true(replay == state)

        local host_claim
        host_claim, outcome = deliver(fixture.host(), "guest.9", handshake("guest.9", 0, "host"))
        t.eq(outcome.code, "protocol_mismatch")
        t.eq(#host_claim.peers, 1)

        local incompatible = fixture.runtime()
        incompatible.runtime_revision = "lovejs.11.5.other"
        local mismatch
        mismatch, outcome = deliver(
            fixture.host(),
            "guest.9",
            message("handshake", "guest.9", 0, { role = "guest", runtime = incompatible })
        )
        t.eq(outcome.code, "runtime_mismatch")
        t.eq(#mismatch.peers, 1)
        t.eq(mismatch.terminal, nil)
    end)

    t.it("closes admission once the manifest is proposed", function()
        local state = assigned_host(1)
        local next_state, outcome = deliver(state, "guest.7", handshake("guest.7", 0))
        t.eq(outcome.code, "invalid_phase")
        t.eq(#next_state.peers, 2)
        t.eq(next_state.terminal, nil)
    end)

    t.it("refuses non-handshake traffic from an unadmitted link", function()
        local state = fixture.host()
        local next_state, outcome = deliver(
            state,
            "guest.1",
            message("manifest_accept", "guest.1", 0, { manifest_id = "0123456789abcdef" })
        )
        t.eq(outcome.code, "invalid_phase")
        t.eq(#next_state.peers, 1)
        t.eq(next_state.terminal, nil)
    end)
end)

t.describe("online coordinator configuration", function()
    t.it("keeps the manifest immutable after proposal", function()
        local state = assigned_host(1)
        local manifest = fixture.manifest()
        local same, outcome = coordinator.step(state, {
            kind = "propose_manifest",
            manifest = manifest,
        })
        t.eq(outcome.disposition, "idempotent")
        t.is_true(same == state, "an idempotent proposal changes nothing")

        local changed = fixture.manifest()
        changed.seed = changed.seed + 1
        local rejected
        rejected, outcome = coordinator.step(state, {
            kind = "propose_manifest",
            manifest = changed,
        })
        t.eq(outcome.disposition, "rejected")
        t.eq(outcome.code, "identity_mismatch")
        t.is_true(rejected == state)
    end)

    t.it("requires manifest acceptance before ownership and readiness", function()
        local state = fixture.host()
        state = (deliver(state, "guest.1", handshake("guest.1", 0)))
        state = (
            coordinator.step(state, { kind = "propose_manifest", manifest = fixture.manifest() })
        )
        t.eq(state.peers[2].accepted_manifest_id, nil)

        local blocked, outcome = coordinator.step(state, {
            kind = "assign_slots",
            assignments = fixture.assignments(1),
        })
        t.eq(outcome.code, "invalid_phase")
        t.is_true(blocked == state, "ownership waits for every acceptance")

        local early = (
            deliver(
                state,
                "guest.1",
                message("ready", "guest.1", 1, {
                    manifest_id = assert(state.manifest_id),
                    ready = true,
                })
            )
        )
        t.eq(early.phase, "terminal")
        t.eq(assert(early.terminal).code, "invalid_phase")

        state = (
            deliver(
                state,
                "guest.1",
                message("manifest_accept", "guest.1", 1, {
                    manifest_id = assert(state.manifest_id),
                })
            )
        )
        state = (
            coordinator.step(state, {
                kind = "assign_slots",
                assignments = fixture.assignments(1),
            })
        )
        t.eq(state.phase, "assigned")
        local unconfirmed, outcome = deliver(
            state,
            "guest.1",
            message("ready", "guest.1", 2, {
                manifest_id = assert(state.manifest_id),
                ready = true,
            })
        )
        t.eq(outcome.code, "invalid_assignment", "readiness must answer for this ownership")
        t.eq(outcome.reason, coordinator.UNCONFIRMED_READY_REASON)
        t.is_true(unconfirmed == state)

        state = ready_host(state, 1, { 1 })
        t.eq(state.peers[2].ready, true)
        t.eq(state.phase, "ready")
    end)

    t.it("holds the acceptance invariant even if ownership was published early", function()
        -- White-box guard: the readiness barrier does not depend on the
        -- assignment-time acceptance rule for its own correctness.
        local state = assigned_host(1)
        state.peers[2].accepted_manifest_id = nil
        local next_state, outcome = deliver(
            state,
            "guest.1",
            message("ready", "guest.1", 2, {
                manifest_id = assert(state.manifest_id),
                ready = true,
            })
        )
        t.eq(outcome.disposition, "applied")
        t.eq(next_state.phase, "terminal")
        t.eq(assert(next_state.terminal).detail, "readiness preceded manifest acceptance")
    end)

    t.it("requires slot ownership before readiness", function()
        local state = fixture.host()
        local _, outcome = coordinator.step(state, { kind = "set_ready", ready = true })
        t.eq(outcome.code, "invalid_phase")

        state = (deliver(state, "guest.1", handshake("guest.1", 0)))
        state = (
            coordinator.step(state, { kind = "propose_manifest", manifest = fixture.manifest() })
        )
        _, outcome = coordinator.step(state, { kind = "set_ready", ready = true })
        t.eq(outcome.code, "invalid_phase")
    end)

    t.it("clears readiness whenever ownership changes", function()
        local state, sequences = assigned_host(2)
        state = ready_host(state, 2, sequences)
        t.eq(state.phase, "ready")

        local swapped = fixture.assignments(2)
        swapped[1].producer_id = "guest.1"
        swapped[2].producer_id = HOST
        local next_state, outcome = coordinator.step(state, {
            kind = "assign_slots",
            assignments = swapped,
        })
        t.eq(outcome.disposition, "applied")
        t.eq(next_state.phase, "assigned")
        for _, peer in ipairs(next_state.peers) do
            t.eq(peer.ready, false)
        end

        local same
        same, outcome = coordinator.step(next_state, {
            kind = "assign_slots",
            assignments = swapped,
        })
        t.eq(outcome.disposition, "idempotent")
        t.is_true(same == next_state)
    end)

    t.it("lets a peer revoke readiness before the countdown", function()
        local state, sequences = assigned_host(1)
        state = ready_host(state, 1, sequences)
        t.eq(state.phase, "ready")

        local next_state, outcome = deliver(
            state,
            "guest.1",
            message("ready", "guest.1", sequences[1] + 1, {
                manifest_id = assert(state.manifest_id),
                ready = false,
            })
        )
        t.eq(outcome.disposition, "applied")
        t.eq(next_state.phase, "assigned")
        t.eq(next_state.peers[2].ready, false)
    end)

    t.it("rejects a slot assignment that leaves a peer unseated", function()
        local state = assigned_host(2)
        local orphaned = fixture.assignments(1)
        local _, outcome = coordinator.step(state, {
            kind = "assign_slots",
            assignments = orphaned,
        })
        t.eq(outcome.code, "invalid_assignment")

        local impostor = fixture.assignments(2)
        impostor[3].producer_id = "guest.9"
        _, outcome = coordinator.step(state, { kind = "assign_slots", assignments = impostor })
        t.eq(outcome.code, "invalid_assignment")
    end)
end)

t.describe("online coordinator countdown and start", function()
    t.it("freezes ownership and names one start boundary", function()
        local state, sequences = assigned_host(1)
        state = ready_host(state, 1, sequences)
        local next_state, outcome = coordinator.step(state, {
            kind = "begin_countdown",
            countdown_id = fixture.COUNTDOWN_ID,
            remaining_ticks = 2,
            first_input_tick = 12,
        })
        t.eq(outcome.disposition, "applied")
        t.eq(next_state.phase, "countdown")
        local freeze = assert(next_state.freeze)
        t.eq(freeze.first_input_tick, 12)
        t.eq(freeze.seed, fixture.manifest().seed)
        t.eq(freeze.manifest_id, next_state.manifest_id)
        t.eq(freeze.combat_rules_id, fixture.manifest().combat_rules_id)
        t.eq(#freeze.assignments, input_frame.SLOT_COUNT)

        -- Freezing takes a copy: later edits to the live table cannot leak in.
        assert(next_state.assignments)[1].producer_id = "mutated"
        t.eq(freeze.assignments[1].producer_id, HOST)

        local blocked
        blocked, outcome = coordinator.step(next_state, {
            kind = "assign_slots",
            assignments = fixture.assignments(1),
        })
        t.eq(outcome.code, "invalid_phase")
        t.is_true(blocked == next_state)

        blocked, outcome = coordinator.step(next_state, { kind = "set_ready", ready = false })
        t.eq(outcome.code, "invalid_phase")
        t.is_true(blocked == next_state)
    end)

    t.it("refuses a countdown before every peer is ready", function()
        local state = assigned_host(1)
        local _, outcome = coordinator.step(state, {
            kind = "begin_countdown",
            countdown_id = fixture.COUNTDOWN_ID,
            remaining_ticks = 2,
            first_input_tick = 0,
        })
        t.eq(outcome.code, "invalid_phase")
    end)

    t.it("publishes start only when the fake clock drains the countdown", function()
        local state, sequences = assigned_host(1)
        state = ready_host(state, 1, sequences)
        state = (
            coordinator.step(state, {
                kind = "begin_countdown",
                countdown_id = fixture.COUNTDOWN_ID,
                remaining_ticks = 2,
                first_input_tick = 0,
            })
        )
        local outcome
        state, outcome = coordinator.step(state, { kind = "tick" })
        t.eq(#outcome.actions, 0)
        t.eq(state.countdown_remaining, 1)
        state, outcome = coordinator.step(state, { kind = "tick" })
        t.eq(state.countdown_remaining, 0)
        t.eq(state.phase, "countdown")
        t.eq(outcome.actions[1].message.kind, "start")
        t.eq(outcome.actions[1].message.body.first_input_tick, 0)

        local started
        started, outcome = deliver(
            state,
            "guest.1",
            message("start", "guest.1", sequences[1] + 1, {
                manifest_id = assert(state.manifest_id),
                countdown_id = fixture.COUNTDOWN_ID,
                first_input_tick = 0,
            })
        )
        t.eq(started.phase, "running")
        t.eq(outcome.actions[1].kind, "start_match")
        t.eq(assert(outcome.actions[1].freeze).first_input_tick, 0)
    end)

    t.it("does not double-start on a duplicated acknowledgement", function()
        local state, sequences = assigned_host(1)
        state = ready_host(state, 1, sequences)
        state = (
            coordinator.step(state, {
                kind = "begin_countdown",
                countdown_id = fixture.COUNTDOWN_ID,
                remaining_ticks = 0,
                first_input_tick = 0,
            })
        )
        local ack = message("start", "guest.1", sequences[1] + 1, {
            manifest_id = assert(state.manifest_id),
            countdown_id = fixture.COUNTDOWN_ID,
            first_input_tick = 0,
        })
        local outcome
        state, outcome = deliver(state, "guest.1", ack)
        t.eq(state.phase, "running")
        t.eq(outcome.actions[1].kind, "start_match")

        local repeated
        repeated, outcome = deliver(state, "guest.1", ack)
        t.eq(outcome.disposition, "idempotent")
        t.eq(#outcome.actions, 0)
        t.is_true(repeated == state)
    end)

    t.it("rejects a start that misnames the frozen boundary", function()
        local state, sequences = assigned_host(1)
        state = ready_host(state, 1, sequences)
        state = (
            coordinator.step(state, {
                kind = "begin_countdown",
                countdown_id = fixture.COUNTDOWN_ID,
                remaining_ticks = 0,
                first_input_tick = 0,
            })
        )
        local next_state = (
            deliver(
                state,
                "guest.1",
                message("start", "guest.1", sequences[1] + 1, {
                    manifest_id = assert(state.manifest_id),
                    countdown_id = fixture.COUNTDOWN_ID,
                    first_input_tick = 9,
                })
            )
        )
        t.eq(next_state.phase, "terminal")
        t.eq(assert(next_state.terminal).reason, "protocol_violation")
    end)
end)

t.describe("online coordinator match lifecycle", function()
    ---@return CoordinatorState, integer[]
    local function running_host()
        local state, sequences = assigned_host(1)
        state = ready_host(state, 1, sequences)
        state = (
            coordinator.step(state, {
                kind = "begin_countdown",
                countdown_id = fixture.COUNTDOWN_ID,
                remaining_ticks = 0,
                first_input_tick = 0,
            })
        )
        sequences[1] = sequences[1] + 1
        state = (
            deliver(
                state,
                "guest.1",
                message("start", "guest.1", sequences[1], {
                    manifest_id = assert(state.manifest_id),
                    countdown_id = fixture.COUNTDOWN_ID,
                    first_input_tick = 0,
                })
            )
        )
        return state, sequences
    end

    t.it("acknowledges simulation phases without authoring them", function()
        local state = running_host()
        local outcome
        state, outcome = coordinator.step(state, {
            kind = "match_phase",
            phase = "kickoff",
            tick = 0,
            home_score = 0,
            away_score = 0,
        })
        t.eq(outcome.disposition, "applied")
        t.eq(assert(state.match).phase, "kickoff")

        local rejected
        rejected, outcome = coordinator.step(state, {
            kind = "match_phase",
            phase = "goal_stoppage",
            tick = 30,
            home_score = 1,
            away_score = 0,
        })
        t.eq(outcome.code, "invalid_phase", "a goal cannot follow kickoff directly")
        t.is_true(rejected == state)

        state = (
            coordinator.step(state, {
                kind = "match_phase",
                phase = "playing",
                tick = 1,
                home_score = 0,
                away_score = 0,
            })
        )
        rejected, outcome = coordinator.step(state, {
            kind = "match_phase",
            phase = "goal_stoppage",
            tick = 30,
            home_score = 0,
            away_score = 0,
        })
        t.eq(outcome.code, "malformed", "a stoppage must follow a scored goal")

        rejected, outcome = coordinator.step(state, {
            kind = "match_phase",
            phase = "playing",
            tick = 0,
            home_score = 0,
            away_score = 0,
        })
        t.eq(outcome.code, "invalid_phase")

        state = (
            coordinator.step(state, {
                kind = "match_phase",
                phase = "goal_stoppage",
                tick = 30,
                home_score = 1,
                away_score = 0,
            })
        )
        rejected, outcome = coordinator.step(state, {
            kind = "match_phase",
            phase = "kickoff",
            tick = 31,
            home_score = 0,
            away_score = 0,
        })
        t.eq(outcome.code, "malformed", "scores never move backwards")
        t.is_true(rejected == state)
    end)

    t.it("never restates the simulation score at full time", function()
        local state = running_host()
        state = (
            coordinator.step(state, {
                kind = "match_phase",
                phase = "kickoff",
                tick = 0,
                home_score = 0,
                away_score = 0,
            })
        )
        local early, outcome = coordinator.step(state, {
            kind = "finish",
            final_tick = 10,
            home_score = 0,
            away_score = 0,
            final_hash = "fedcba9876543210",
        })
        t.eq(outcome.code, "invalid_phase")
        t.is_true(early == state)

        state = (
            coordinator.step(state, {
                kind = "match_phase",
                phase = "playing",
                tick = 1,
                home_score = 0,
                away_score = 0,
            })
        )
        state = (
            coordinator.step(state, {
                kind = "match_phase",
                phase = "full_time",
                tick = 7200,
                home_score = 2,
                away_score = 1,
            })
        )
        local lied
        lied, outcome = coordinator.step(state, {
            kind = "finish",
            final_tick = 7200,
            home_score = 3,
            away_score = 1,
            final_hash = "fedcba9876543210",
        })
        t.eq(outcome.code, "identity_mismatch")
        t.is_true(lied == state)

        local finished
        finished, outcome = coordinator.step(state, {
            kind = "finish",
            final_tick = 7200,
            home_score = 2,
            away_score = 1,
            final_hash = "fedcba9876543210",
        })
        t.eq(finished.phase, "result")
        t.eq(assert(finished.result).home_score, 2)
        t.eq(outcome.actions[1].message.kind, "match_phase")
        t.eq(outcome.actions[2].message.kind, "result_ack")
    end)

    t.it("completes only when every peer acknowledges the same result", function()
        local state, sequences = running_host()
        for _, phase in ipairs({ "kickoff", "playing", "full_time" }) do
            state = (
                coordinator.step(state, {
                    kind = "match_phase",
                    phase = phase,
                    tick = phase == "full_time" and 7200 or (phase == "playing" and 1 or 0),
                    home_score = phase == "full_time" and 1 or 0,
                    away_score = 0,
                })
            )
        end
        state = (
            coordinator.step(state, {
                kind = "finish",
                final_tick = 7200,
                home_score = 1,
                away_score = 0,
                final_hash = "fedcba9876543210",
            })
        )
        t.eq(state.phase, "result")

        local wrong = deliver(
            state,
            "guest.1",
            message("result_ack", "guest.1", sequences[1] + 1, {
                final_tick = 7200,
                home_score = 1,
                away_score = 1,
                final_hash = "fedcba9876543210",
            })
        )
        t.eq(assert(wrong.terminal).reason, "hash_mismatch")

        local done = deliver(
            state,
            "guest.1",
            message("result_ack", "guest.1", sequences[1] + 1, {
                final_tick = 7200,
                home_score = 1,
                away_score = 0,
                final_hash = "fedcba9876543210",
            })
        )
        t.eq(done.phase, "terminal")
        t.eq(assert(done.terminal).reason, "completed")
        t.eq(assert(done.terminal).code, nil)
    end)
end)

t.describe("online coordinator duplicate and invalid traffic", function()
    t.it("treats a byte-identical replay as a no-op", function()
        local state = fixture.host()
        local admission = handshake("guest.1", 0)
        local first = (deliver(state, "guest.1", admission))
        local second, outcome = deliver(first, "guest.1", admission)
        t.eq(outcome.disposition, "idempotent")
        t.eq(#outcome.actions, 0)
        t.is_true(second == first, "a duplicate never advances the session")
        t.eq(#second.peers, 2)
    end)

    t.it("treats reused transcript identity with new bytes as terminal", function()
        local state, sequences = assigned_host(1)
        local manifest_id = assert(state.manifest_id)
        local conflict = message("ready", "guest.1", sequences[1], {
            manifest_id = manifest_id,
            ready = true,
        })
        local next_state = (deliver(state, "guest.1", conflict))
        t.eq(next_state.phase, "terminal")
        t.eq(assert(next_state.terminal).reason, "protocol_violation")
        t.eq(assert(next_state.terminal).code, "malformed_message")
    end)

    t.it("treats an out-of-phase message as terminal", function()
        local state = fixture.host()
        state = (deliver(state, "guest.1", handshake("guest.1", 0)))
        local next_state = (
            deliver(
                state,
                "guest.1",
                message("ready", "guest.1", 1, {
                    manifest_id = "0123456789abcdef",
                    ready = true,
                })
            )
        )
        t.eq(next_state.phase, "terminal")
        t.eq(assert(next_state.terminal).code, "invalid_phase")
    end)

    t.it("treats a spoofed peer identity or wrong direction as terminal", function()
        local state = assigned_host(2)
        local spoofed = (
            coordinator.step(state, {
                kind = "control",
                link_id = fixture.link_id("guest.1"),
                message = message("manifest_accept", "guest.2", 5, {
                    manifest_id = assert(state.manifest_id),
                }),
            })
        )
        t.eq(spoofed.phase, "terminal")
        t.eq(assert(spoofed.terminal).detail, "control message claims another peer identity")

        local wrong_way = (
            deliver(
                state,
                "guest.1",
                message("manifest_proposal", "guest.1", 5, {
                    manifest_id = assert(state.manifest_id),
                    manifest = fixture.manifest(),
                })
            )
        )
        t.eq(wrong_way.phase, "terminal")
        t.eq(assert(wrong_way.terminal).reason, "protocol_violation")
    end)

    t.it("treats malformed or foreign-session wire as a per-link refusal", function()
        local state = fixture.host()
        local next_state, outcome =
            coordinator.receive(state, fixture.link_id("guest.1"), "GCOP;1;junk")
        t.eq(outcome.disposition, "rejected")
        t.eq(next_state.terminal, nil)
        t.eq(#next_state.peers, 1)

        local foreign = assert(protocol.new("handshake", "other_session", "guest.1", 0, {
            role = "guest",
            runtime = fixture.runtime(),
        }))
        local refused
        refused, outcome = deliver(state, "guest.1", foreign)
        t.eq(outcome.code, "protocol_mismatch")
        t.eq(refused.terminal, nil)
    end)

    t.it("refuses unknown events and post-terminal traffic", function()
        local state = fixture.host()
        local _, outcome = coordinator.step(state, { kind = "teleport" })
        t.eq(outcome.code, "unknown_message")

        local ended = (coordinator.step(state, { kind = "abort", code = "host_abort" }))
        t.eq(ended.phase, "terminal")
        local after
        after, outcome =
            coordinator.step(ended, { kind = "propose_manifest", manifest = fixture.manifest() })
        t.eq(outcome.code, "invalid_phase")
        t.is_true(after == ended)

        local ticked
        ticked, outcome = coordinator.step(ended, { kind = "tick" })
        t.eq(outcome.disposition, "applied")
        t.eq(ticked.clock, ended.clock + 1)
        t.eq(ticked.phase, "terminal")
    end)
end)

t.describe("online coordinator guest validation", function()
    t.it("accepts a matching manifest and refuses a foreign identity", function()
        local guest = fixture.guest(1)
        guest = (coordinator.step(guest, { kind = "connect" }))
        t.eq(guest.phase, "handshake")

        local manifest = fixture.manifest()
        local manifest_id = protocol.manifest_id(manifest)
        local accepted, outcome = deliver(
            guest,
            "guest.1",
            message(
                "manifest_proposal",
                HOST,
                0,
                { manifest_id = manifest_id, manifest = manifest }
            )
        )
        t.eq(accepted.phase, "manifest")
        t.eq(accepted.peers[1].accepted_manifest_id, manifest_id)
        t.eq(outcome.actions[1].message.kind, "manifest_accept")

        local other = fixture.manifest()
        other.content_id = "content.other.v1"
        local refused = (
            deliver(
                (coordinator.step(fixture.guest(1), { kind = "connect" })),
                "guest.1",
                message("manifest_proposal", HOST, 0, {
                    manifest_id = protocol.manifest_id(other),
                    manifest = other,
                })
            )
        )
        t.eq(refused.phase, "terminal")
        t.eq(assert(refused.terminal).reason, "manifest_mismatch")
        t.eq(assert(refused.terminal).detail, "local identity differs at manifest.content_id")
    end)

    t.it("reports the first differing expectation field", function()
        local manifest = fixture.manifest()
        t.eq(coordinator.expectation_difference(nil, manifest), nil)
        t.eq(coordinator.expectation_difference(fixture.expectation(), manifest), nil)
        local expectation = fixture.expectation()
        expectation.build_id = "build.other"
        expectation.arena_id = "arena.other"
        local difference = assert(coordinator.expectation_difference(expectation, manifest))
        t.eq(difference.path, "manifest.build_id")
        t.eq(difference.actual, manifest.build_id)
    end)

    t.it("refuses ownership that seats no local slot", function()
        local guest = fixture.guest(1)
        guest = (coordinator.step(guest, { kind = "connect" }))
        local manifest = fixture.manifest()
        local manifest_id = protocol.manifest_id(manifest)
        guest = (
            deliver(
                guest,
                "guest.1",
                message(
                    "manifest_proposal",
                    HOST,
                    0,
                    { manifest_id = manifest_id, manifest = manifest }
                )
            )
        )
        local next_state = (
            deliver(
                guest,
                "guest.1",
                message("slot_assignment", HOST, 1, {
                    manifest_id = manifest_id,
                    assignments = assert(coordinator.plan_assignments(manifest, { HOST })),
                })
            )
        )
        t.eq(next_state.phase, "terminal")
        t.eq(assert(next_state.terminal).reason, "invalid_assignment")
    end)

    t.it("revokes readiness when the host republishes ownership", function()
        local guest = fixture.guest(1)
        guest = (coordinator.step(guest, { kind = "connect" }))
        local manifest = fixture.manifest()
        local manifest_id = protocol.manifest_id(manifest)
        guest = (
            deliver(
                guest,
                "guest.1",
                message(
                    "manifest_proposal",
                    HOST,
                    0,
                    { manifest_id = manifest_id, manifest = manifest }
                )
            )
        )
        guest = (
            deliver(
                guest,
                "guest.1",
                message("slot_assignment", HOST, 1, {
                    manifest_id = manifest_id,
                    assignments = fixture.assignments(1),
                })
            )
        )
        t.eq(guest.phase, "assigned")
        local outcome
        guest, outcome = coordinator.step(guest, { kind = "set_ready", ready = true })
        t.eq(guest.phase, "ready")
        t.eq(outcome.actions[1].message.body.ready, true)

        local swapped = fixture.assignments(1)
        swapped[1].producer_id = "guest.1"
        swapped[2].producer_id = HOST
        guest, outcome = deliver(
            guest,
            "guest.1",
            message("slot_assignment", HOST, 2, {
                manifest_id = manifest_id,
                assignments = swapped,
            })
        )
        t.eq(guest.phase, "assigned")
        t.eq(guest.peers[1].ready, false)
        t.eq(outcome.actions[1].message.kind, "ready")
        t.eq(outcome.actions[1].message.body.ready, false)
    end)
end)

-- Systematic coverage of the two transition dimensions the reducer defines:
-- inbound (phase x control message kind) legality, and (phase x local event
-- kind) totality. These are cross-products, not hand-picked scenarios, so a
-- phase or kind added later cannot quietly escape coverage.
t.describe("online coordinator transition matrix", function()
    local GUEST = "guest.1"

    ---@param manifest_id string?
    ---@return table<SessionMessageKind, SessionMessageBody>
    local function guest_bodies(manifest_id)
        local id = manifest_id or "0123456789abcdef"
        return {
            handshake = { role = "guest", runtime = fixture.runtime() },
            manifest_accept = { manifest_id = id },
            ready = { manifest_id = id, ready = true },
            start = {
                manifest_id = id,
                countdown_id = fixture.COUNTDOWN_ID,
                first_input_tick = 0,
            },
            hash_report = { tick = 60, boundary_hash = "0123456789abcdef" },
            result_ack = {
                final_tick = 7200,
                home_score = 1,
                away_score = 0,
                final_hash = "fedcba9876543210",
            },
            abort = { code = "host_abort" },
            disconnect = { target_peer_id = GUEST, code = "peer_left" },
        }
    end

    ---@param manifest_id string?
    ---@return table<SessionMessageKind, SessionMessageBody>
    local function host_bodies(manifest_id)
        local id = manifest_id or "0123456789abcdef"
        local bodies = guest_bodies(id)
        return {
            manifest_proposal = {
                manifest_id = protocol.manifest_id(fixture.manifest()),
                manifest = fixture.manifest(),
            },
            peer_assignment = { assigned_peer_id = GUEST, role = "guest" },
            slot_assignment = { manifest_id = id, assignments = fixture.assignments(1) },
            countdown = {
                manifest_id = id,
                countdown_id = fixture.COUNTDOWN_ID,
                remaining_ticks = 1,
                first_input_tick = 0,
            },
            start = bodies.start,
            match_phase = { phase = "kickoff", tick = 0, home_score = 0, away_score = 0 },
            hash_report = bodies.hash_report,
            result_ack = bodies.result_ack,
            abort = bodies.abort,
            disconnect = { target_peer_id = GUEST, code = "host_left" },
        }
    end

    -- One host state per lifecycle phase, built through the real reducer.
    ---@return table<SessionLifecyclePhase, CoordinatorState>
    local function host_states()
        local states = {}
        states.handshake = (deliver(fixture.host(), GUEST, handshake(GUEST, 0)))
        local state, sequences = assigned_host(1)
        states.manifest = (
            coordinator.step(
                (deliver(fixture.host(), GUEST, handshake(GUEST, 0))),
                { kind = "propose_manifest", manifest = fixture.manifest() }
            )
        )
        states.assigned = state
        state = ready_host(state, 1, sequences)
        states.ready = state
        state = (
            coordinator.step(state, {
                kind = "begin_countdown",
                countdown_id = fixture.COUNTDOWN_ID,
                remaining_ticks = 0,
                first_input_tick = 0,
            })
        )
        states.countdown = state
        state = (
            deliver(
                state,
                GUEST,
                message("start", GUEST, 40, {
                    manifest_id = assert(state.manifest_id),
                    countdown_id = fixture.COUNTDOWN_ID,
                    first_input_tick = 0,
                })
            )
        )
        states.running = state
        for _, phase in ipairs({ "kickoff", "playing", "full_time" }) do
            state = (
                coordinator.step(state, {
                    kind = "match_phase",
                    phase = phase,
                    tick = phase == "full_time" and 7200 or (phase == "playing" and 1 or 0),
                    home_score = phase == "full_time" and 1 or 0,
                    away_score = 0,
                })
            )
        end
        states.result = (
            coordinator.step(state, {
                kind = "finish",
                final_tick = 7200,
                home_score = 1,
                away_score = 0,
                final_hash = "fedcba9876543210",
            })
        )
        return states
    end

    ---@return table<SessionLifecyclePhase, CoordinatorState>
    local function guest_states()
        local states = {}
        states.new = fixture.guest(1)
        local state = (coordinator.step(states.new, { kind = "connect" }))
        states.handshake = state
        local manifest = fixture.manifest()
        local manifest_id = protocol.manifest_id(manifest)
        state = (
            deliver(
                state,
                GUEST,
                message("manifest_proposal", HOST, 0, {
                    manifest_id = manifest_id,
                    manifest = manifest,
                })
            )
        )
        states.manifest = state
        state = (
            deliver(
                state,
                GUEST,
                message("slot_assignment", HOST, 1, {
                    manifest_id = manifest_id,
                    assignments = fixture.assignments(1),
                })
            )
        )
        states.assigned = state
        state = (coordinator.step(state, { kind = "set_ready", ready = true }))
        states.ready = state
        state = (
            deliver(
                state,
                GUEST,
                message("countdown", HOST, 2, {
                    manifest_id = manifest_id,
                    countdown_id = fixture.COUNTDOWN_ID,
                    remaining_ticks = 0,
                    first_input_tick = 0,
                })
            )
        )
        states.countdown = state
        state = (
            deliver(
                state,
                GUEST,
                message("start", HOST, 3, {
                    manifest_id = manifest_id,
                    countdown_id = fixture.COUNTDOWN_ID,
                    first_input_tick = 0,
                })
            )
        )
        states.running = state
        local sequence = 3
        for _, phase in ipairs({ "kickoff", "playing", "full_time" }) do
            sequence = sequence + 1
            state = (
                deliver(
                    state,
                    GUEST,
                    message("match_phase", HOST, sequence, {
                        phase = phase,
                        tick = phase == "full_time" and 7200 or (phase == "playing" and 1 or 0),
                        home_score = phase == "full_time" and 1 or 0,
                        away_score = 0,
                    })
                )
            )
        end
        states.result = state
        return states
    end

    ---@param state CoordinatorState
    ---@param sender string
    ---@param kind SessionMessageKind
    ---@param bodies table<SessionMessageKind, SessionMessageBody>
    local function assert_phase_cell(state, sender, kind, bodies)
        local control = message(kind, sender, 90, bodies[kind])
        -- The coordinator validates a republished assignment against `assigned`
        -- because ownership changes revoke readiness; the oracle mirrors that
        -- one documented remap and nothing else.
        local phase = state.phase
        if kind == "slot_assignment" and phase == "ready" then
            phase = "assigned"
        end
        local legal = protocol.validate_phase(control, phase) ~= nil
        local next_state = (deliver(state, GUEST, control))
        local label = ("%s in %s"):format(kind, state.phase)
        if legal then
            local terminal = next_state.terminal
            t.is_true(
                terminal == nil or terminal.code ~= "invalid_phase",
                label .. " is legal but was refused as out of phase"
            )
        else
            t.eq(next_state.phase, "terminal", label .. " must not be accepted")
            t.eq(assert(next_state.terminal).code, "invalid_phase", label)
        end
    end

    t.it("agrees with the protocol phase table for every host-received kind", function()
        local states = host_states()
        local checked = 0
        for _, phase in ipairs(coordinator.PHASES) do
            local state = states[phase]
            if state then
                t.eq(state.phase, phase, "fixture for phase " .. phase)
                for _, kind in ipairs({
                    "handshake",
                    "manifest_accept",
                    "ready",
                    "start",
                    "hash_report",
                    "result_ack",
                    "abort",
                    "disconnect",
                }) do
                    assert_phase_cell(state, GUEST, kind, guest_bodies(state.manifest_id))
                    checked = checked + 1
                end
            end
        end
        t.eq(checked, 7 * 8, "every host phase must be crossed with every guest-sent kind")
    end)

    t.it("agrees with the protocol phase table for every guest-received kind", function()
        local states = guest_states()
        local checked = 0
        for _, phase in ipairs(coordinator.PHASES) do
            local state = states[phase]
            if state then
                t.eq(state.phase, phase, "fixture for phase " .. phase)
                for _, kind in ipairs({
                    "manifest_proposal",
                    "peer_assignment",
                    "slot_assignment",
                    "countdown",
                    "start",
                    "match_phase",
                    "hash_report",
                    "result_ack",
                    "abort",
                    "disconnect",
                }) do
                    assert_phase_cell(state, HOST, kind, host_bodies(state.manifest_id))
                    checked = checked + 1
                end
            end
        end
        t.eq(checked, 8 * 10, "every guest phase must be crossed with every host-sent kind")
    end)

    t.it("is a total function over every phase and local event kind", function()
        local events = {
            { kind = "connect" },
            { kind = "control", link_id = fixture.link_id(GUEST) },
            { kind = "link_lost", link_id = fixture.link_id(GUEST), code = "transport_lost" },
            { kind = "propose_manifest", manifest = fixture.manifest() },
            { kind = "assign_slots", assignments = fixture.assignments(1) },
            { kind = "set_ready", ready = true },
            {
                kind = "begin_countdown",
                countdown_id = fixture.COUNTDOWN_ID,
                remaining_ticks = 1,
                first_input_tick = 0,
            },
            { kind = "tick" },
            { kind = "match_phase", phase = "kickoff", tick = 0, home_score = 0, away_score = 0 },
            { kind = "hash_report", tick = 60, boundary_hash = "0123456789abcdef" },
            {
                kind = "finish",
                final_tick = 7200,
                home_score = 1,
                away_score = 0,
                final_hash = "fedcba9876543210",
            },
            { kind = "netcode_failure", failure = "late_input" },
            { kind = "leave" },
            { kind = "abort", code = "host_abort" },
        }
        local dispositions = { applied = true, idempotent = true, rejected = true }
        local checked = 0
        for _, states in ipairs({ host_states(), guest_states() }) do
            local terminal = nil
            for _, phase in ipairs(coordinator.PHASES) do
                local state = states[phase]
                if state then
                    terminal = terminal
                        or (coordinator.step(state, { kind = "abort", code = "host_abort" }))
                    for _, base in ipairs(events) do
                        for _, subject in ipairs({ state, terminal }) do
                            local next_state, outcome = coordinator.step(subject, base)
                            t.is_true(
                                dispositions[outcome.disposition],
                                base.kind .. " in " .. subject.phase .. " returned no disposition"
                            )
                            t.is_true(
                                type(outcome.actions) == "table",
                                base.kind .. " in " .. subject.phase .. " returned no action list"
                            )
                            if outcome.disposition == "rejected" then
                                t.is_true(
                                    next_state == subject,
                                    base.kind
                                        .. " in "
                                        .. subject.phase
                                        .. " left progress behind on rejection"
                                )
                                t.is_true(
                                    outcome.code ~= nil,
                                    base.kind .. " rejected with no code"
                                )
                            end
                            checked = checked + 1
                        end
                    end
                end
            end
        end
        t.eq(checked, (7 + 8) * #events * 2)
    end)
end)
