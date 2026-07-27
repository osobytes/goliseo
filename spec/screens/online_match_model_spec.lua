-- Tier-2 coverage for the pure online match flow model (AGENTS.md §9).
--
-- `spec/screens/online_match_flow_spec.lua` drives this model end to end through
-- two mounted screens, which proves it works in the real flow but exercises it
-- only through whatever a real match happens to produce. This spec calls
-- `online_match_model.command` directly, one event kind at a time, against
-- hand-built states — which is the point of splitting a screen into a pure model
-- and a thin shell in the first place.
--
-- Two deliberate shortcuts, both of which keep the subject under test honest:
--
--  * The coordinator states come from `game.online.coordinator_driver`, real
--    instances driven to `running` over real canonical wires. That driver
--    proposes the *protocol fixture* manifest, while the request here is built
--    from the content-derived one. The model never reconciles the two — it reads
--    only the request's role, peer id, and first input tick — so pairing them is
--    safe, and it keeps this spec from re-testing the lobby.
--  * `RollbackEventStep`s are hand-built with only the fields the model reads
--    (`tick` and `lifecycle_events`). That *is* the model's input surface; a full
--    simulated step would add nothing but coupling.

local t = require("spec.support.runner")
local online_fixture = require("spec.fixtures.online_match_session")
local coordinator_driver = require("game.online.coordinator_driver")
local coordinator_fixture = require("game.online.coordinator_fixture")
local match_session = require("game.online.match_session")
local online_match_model = require("game.screens.online_match_model")
local protocol = require("game.online.protocol")

local MODE = "1v1"
-- The coordinator driver's own peer ids, so the freeze names the same humans the
-- coordinator instances do.
local SESSION_PEERS = { coordinator_fixture.HOST_PEER_ID, coordinator_fixture.guest_peer_id(1) }

---@class OnlineModelHarness
---@field host OnlineMatchModel
---@field guest OnlineMatchModel

---@return OnlineModelHarness
local function harness()
    local manifest = online_fixture.manifest(MODE)
    local freeze = online_fixture.freeze(manifest, SESSION_PEERS)
    local session = coordinator_driver.new({ guest_count = 1, mode = MODE }):reach_start()
    local nodes = session.nodes
    assert(nodes[1].state.phase == "running", "the host never reached running")
    assert(nodes[2].state.phase == "running", "the guest never reached running")

    ---@param index integer
    ---@param role SessionRole
    ---@return OnlineMatchModel
    local function model_for(index, role)
        local request = assert(match_session.request({
            role = role,
            peer_id = SESSION_PEERS[index],
            manifest = manifest,
            freeze = freeze,
        }))
        return online_match_model.new(request, nodes[index].state)
    end

    return { host = model_for(1, "host"), guest = model_for(2, "guest") }
end

---@param tick integer
---@param lifecycle RollbackWrappedLifecycleEvent[]?
---@return RollbackEventStep
local function step(tick, lifecycle)
    return {
        tick = tick,
        start_boundary = tick,
        end_boundary = tick + 1,
        state = { score = { home = 0, away = 0 }, time_left = 1, finished = false },
        match_events = {},
        combat_events = {},
        lifecycle_events = lifecycle or {},
    }
end

---@param kind RollbackLifecycleKind
---@param home integer
---@param away integer
---@param team InputTeam?
---@return RollbackWrappedLifecycleEvent
local function lifecycle(kind, home, away, team)
    return {
        id = "lifecycle/" .. kind .. "/1",
        tick = 0,
        domain = "lifecycle/" .. kind,
        ordinal = 1,
        payload = { kind = kind, team = team, score = { home = home, away = away } },
    }
end

---@param effects OnlineMatchEffect[]
---@return SessionMatchPhase[]
local function published_phases(effects)
    ---@type SessionMatchPhase[]
    local phases = {}
    for _, effect in ipairs(effects) do
        if effect.kind == "send" then
            local message = protocol.decode(effect.wire)
            if message and message.kind == "match_phase" then
                local body = message.body
                ---@cast body SessionMatchPhaseBody
                phases[#phases + 1] = body.phase
            end
        end
    end
    return phases
end

---@param effects OnlineMatchEffect[]
---@param kind string
---@return boolean
local function sent(effects, kind)
    for _, effect in ipairs(effects) do
        if effect.kind == "send" then
            local message = protocol.decode(effect.wire)
            if message and message.kind == kind then
                return true
            end
        end
    end
    return false
end

t.describe("online match model purity", function()
    t.it("returns a fresh model and never mutates the one it was given", function()
        local model = harness().host
        local before = model.coordinator
        local next_model, effects = online_match_model.command(model, { kind = "focus_lost" })
        t.is_true(next_model ~= model, "command must copy")
        t.eq(model.notice, nil, "the input model must be untouched")
        t.eq(model.abort_prompt, false)
        t.is_true(next_model.notice ~= nil)
        t.eq(#effects, 0, "a focus notice is local and sends nothing")
        t.is_true(next_model.coordinator == before, "a notice never steps the session")
    end)

    t.it("ignores an unknown event kind rather than failing", function()
        local model = harness().host
        local next_model, effects = online_match_model.command(model, { kind = "not_a_real_event" })
        t.eq(#effects, 0)
        t.eq(next_model.phase, "playing")
        t.eq(next_model.error, nil)
    end)
end)

t.describe("online match model local interruptions", function()
    -- The whole point: none of these may stop the driver, so none of them may
    -- produce a coordinator effect either.
    t.it("answers focus loss and controller loss with a notice and nothing else", function()
        for _, case in ipairs({
            { kind = "focus_lost", text = online_match_model.FOCUS_NOTICE },
            { kind = "controller_lost", text = online_match_model.CONTROLLER_NOTICE },
        }) do
            local model = harness().host
            local next_model, effects = online_match_model.command(model, { kind = case.kind })
            t.eq(#effects, 0, case.kind .. " must not reach the session")
            t.eq(assert(next_model.notice).text, case.text)
            t.eq(next_model.phase, "playing")
            t.eq(next_model.abort_prompt, false, case.kind .. " must not arm the abort prompt")
        end
    end)

    t.it("explains a pause request before it will abort", function()
        local model = harness().host
        local first, effects = online_match_model.command(model, { kind = "pause_request" })
        t.eq(#effects, 0, "the first pause request only explains itself")
        t.is_true(first.abort_prompt)
        t.eq(assert(first.notice).text, online_match_model.PAUSE_NOTICE)
        t.eq(first.phase, "playing")
        t.eq(first.terminal, nil)
    end)

    t.it("aborts on a second pause request and ends the session", function()
        local model = harness().host
        local armed = (online_match_model.command(model, { kind = "pause_request" }))
        local aborted, effects = online_match_model.command(armed, { kind = "pause_request" })
        t.is_true(sent(effects, "abort"), "an abort must be announced to the peers")
        t.eq(aborted.phase, "ended")
        t.eq(assert(aborted.terminal).reason, "local_abort")
        t.eq(aborted.result, nil, "an aborted session has no result to commit")
        t.eq(online_match_model.exit_route(aborted), "terminal")
        t.is_true(online_match_model.ended(aborted))
    end)

    t.it("disarms the prompt on any other input", function()
        local model = harness().host
        local armed = (online_match_model.command(model, { kind = "pause_request" }))
        local resumed, effects = online_match_model.command(armed, { kind = "resume_request" })
        t.eq(#effects, 0)
        t.eq(resumed.abort_prompt, false)
        t.eq(resumed.phase, "playing")
        t.eq(resumed.terminal, nil)
    end)

    t.it("decays a notice on the session clock", function()
        local model = harness().host
        local noticed = (online_match_model.command(model, { kind = "focus_lost" }))
        t.eq(assert(noticed.notice).remaining, online_match_model.NOTICE_SECONDS)
        local ticked = (online_match_model.command(noticed, { kind = "tick", dt = 1 }))
        t.eq(assert(ticked.notice).remaining, online_match_model.NOTICE_SECONDS - 1)
        local expired = (
            online_match_model.command(ticked, {
                kind = "tick",
                dt = online_match_model.NOTICE_SECONDS,
            })
        )
        t.eq(expired.notice, nil)
    end)
end)

t.describe("online match model confirmed publication", function()
    t.it("opens the session with kickoff then playing on the first confirmed step", function()
        local model = harness().host
        local next_model, effects = online_match_model.command(model, {
            kind = "confirmed",
            steps = { step(0) },
        })
        t.eq(table.concat(published_phases(effects), ","), "kickoff,playing")
        t.eq(assert(next_model.coordinator.match).phase, "playing")
        t.eq(next_model.phase, "playing")
    end)

    t.it("publishes a goal stoppage only after the confirmed score moved", function()
        local model = harness().host
        model = (online_match_model.command(model, { kind = "confirmed", steps = { step(0) } }))
        local scored, effects = online_match_model.command(model, {
            kind = "confirmed",
            steps = { step(1, { lifecycle("goal", 1, 0, "home"), lifecycle("kickoff", 1, 0) }) },
        })
        t.eq(table.concat(published_phases(effects), ","), "goal_stoppage,kickoff,playing")
        t.eq(scored.score.home, 1)
        t.eq(scored.score.away, 0)
        local match = assert(scored.coordinator.match)
        t.eq(match.home_score, 1, "the session carries the confirmed score, not a predicted one")
        t.eq(match.phase, "playing")
    end)

    t.it("walks the legal phase chain rather than publishing an illegal transition", function()
        local model = harness().host
        model = (online_match_model.command(model, { kind = "confirmed", steps = { step(0) } }))
        -- A goal leaves the session in `goal_stoppage`; full time cannot follow it
        -- directly, so the model has to route back through kickoff and playing.
        model = (
            online_match_model.command(model, {
                kind = "confirmed",
                steps = { step(1, { lifecycle("goal", 1, 0, "home") }) },
            })
        )
        t.eq(model.published, "playing")
        t.eq(model.error, nil, "no phase may be refused as out of order")
    end)

    t.it("never lets a guest publish a match phase", function()
        local model = harness().guest
        local next_model, effects = online_match_model.command(model, {
            kind = "confirmed",
            steps = { step(0, { lifecycle("goal", 1, 0, "home") }) },
        })
        t.eq(#published_phases(effects), 0, "only the host publishes match phases")
        t.eq(next_model.published, nil)
        t.eq(next_model.score.home, 0, "a guest does not restate the score either")
    end)

    t.it("reports a boundary checkpoint as a hash report", function()
        local model = harness().host
        local _, effects = online_match_model.command(model, {
            kind = "checkpoints",
            checkpoints = { { tick = 30, hash = "0123456789abcdef", live = {} } },
        })
        t.is_true(sent(effects, "hash_report"))
    end)

    t.it("surfaces a peer's boundary hash to the driver as well as the session", function()
        local model = harness().host
        local wire = assert(
            protocol.encode(
                assert(
                    protocol.new(
                        "hash_report",
                        model.coordinator.session_id,
                        coordinator_fixture.guest_peer_id(1),
                        9,
                        { tick = 30, boundary_hash = "0123456789abcdef" }
                    )
                )
            )
        )
        local _, effects = online_match_model.command(model, {
            kind = "control",
            link_id = coordinator_fixture.link_id(coordinator_fixture.guest_peer_id(1)),
            wire = wire,
        })
        local observed = nil
        for _, effect in ipairs(effects) do
            if effect.kind == "observe_hash" then
                observed = effect
            end
        end
        t.eq(assert(observed).tick, 30)
        t.eq(assert(observed).hash, "0123456789abcdef")
    end)
end)

t.describe("online match model full time and result", function()
    ---@param model OnlineMatchModel
    ---@return OnlineMatchModel, OnlineMatchEffect[]
    local function reach_full_time(model)
        model = (online_match_model.command(model, { kind = "confirmed", steps = { step(0) } }))
        return online_match_model.command(model, {
            kind = "full_time",
            final_tick = 90,
            home_score = 2,
            away_score = 1,
            final_hash = "0123456789abcdef",
        })
    end

    t.it("walks to full time and reports the simulation's own result", function()
        local finished, effects = reach_full_time(harness().host)
        t.eq(table.concat(published_phases(effects), ","), "full_time,result")
        t.is_true(sent(effects, "result_ack"), "the host offers the result for acknowledgement")
        t.eq(finished.phase, "full_time")
        t.eq(finished.score.home, 2)
        t.eq(finished.score.away, 1)
    end)

    t.it("does not commit a result while any peer has not acknowledged it", function()
        local finished = (reach_full_time(harness().host))
        t.eq(finished.result, nil, "the result waits for the session, not the simulation")
        t.eq(finished.phase, "full_time")
        t.is_true(not online_match_model.ended(finished))
    end)

    t.it("makes a guest report its own result rather than accept one on faith", function()
        local finished, effects = reach_full_time(harness().guest)
        t.eq(#published_phases(effects), 0, "a guest publishes no match phase")
        t.eq(finished.phase, "full_time")
        -- The guest's `finish` is local: it becomes `local_result`, which the
        -- coordinator later compares against the host's acknowledgement.
        local local_result = assert(finished.coordinator.local_result)
        t.eq(local_result.home_score, 2)
        t.eq(local_result.away_score, 1)
        t.eq(local_result.final_hash, "0123456789abcdef")
        t.eq(finished.result, nil)
    end)

    t.it("ignores a second full time", function()
        local first = (reach_full_time(harness().host))
        local second, effects = online_match_model.command(first, {
            kind = "full_time",
            final_tick = 91,
            home_score = 9,
            away_score = 9,
            final_hash = "ffffffffffffffff",
        })
        t.eq(#effects, 0, "full time happens once")
        t.eq(second.score.home, 2)
        t.eq(second.score.away, 1)
    end)
end)

t.describe("online match model terminal reporting", function()
    t.it("reports a netcode failure class into the session", function()
        for _, case in ipairs({
            { failure = "late_input", reason = "late_input" },
            { failure = "desync", reason = "hash_mismatch" },
            { failure = "input_channel", reason = "input_channel_failure" },
        }) do
            local model = harness().host
            local next_model = (
                online_match_model.command(model, {
                    kind = "failure",
                    status = "late_input",
                    failure = case.failure,
                    detail = "a driver terminal",
                })
            )
            t.eq(assert(next_model.terminal).reason, case.reason, case.failure)
            t.eq(next_model.phase, "ended")
            t.eq(next_model.result, nil, "a failed session commits no score")
            t.eq(online_match_model.exit_route(next_model), "terminal")
        end
    end)

    t.it("reports a lost star as the guest's own lost host link", function()
        local model = harness().guest
        local next_model, effects = online_match_model.command(model, {
            kind = "failure",
            status = "transport_lost",
            failure = nil,
            detail = "the star transport is no longer connected",
        })
        t.eq(assert(next_model.terminal).reason, "transport_lost")
        t.eq(next_model.phase, "ended")
        t.eq(#published_phases(effects), 0)
    end)

    t.it("ends a host session explicitly when it cannot name the lost peer", function()
        local model = harness().host
        local next_model, effects = online_match_model.command(model, {
            kind = "failure",
            status = "transport_lost",
            failure = nil,
            detail = "the star transport is no longer connected",
        })
        -- The driver folds every link event into one status, so a host has no
        -- peer id to disconnect. It ends the session rather than guessing.
        t.eq(assert(next_model.terminal).reason, "local_abort")
        t.is_true(sent(effects, "abort"))
        t.eq(next_model.phase, "ended")
    end)

    t.it("keeps the first terminal reason when late traffic follows it", function()
        local model = harness().host
        local ended = (
            online_match_model.command(model, {
                kind = "failure",
                status = "late_input",
                failure = "late_input",
            })
        )
        local after, effects = online_match_model.command(ended, {
            kind = "confirmed",
            steps = { step(0) },
        })
        t.eq(#effects, 0, "a terminated session makes no further progress")
        t.eq(assert(after.terminal).reason, "late_input")
    end)
end)
