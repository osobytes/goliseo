-- Guest-chosen pairs: the additive `pair_preference` request, the host's typed
-- verdict, and the ownership generation a grant mints through the one existing
-- `protocol.assignment_id` path.
--
-- The inertness argument is asserted rather than described: every request a
-- `1v1` or `4v4` session can express is enumerated and proven ungrantable, and
-- the same enumeration over `2v2` finds grants, so the rule -- not a mode
-- branch -- is what makes the difference.

local conformance = require("game.online.protocol_conformance")
local coordinator = require("game.online.coordinator")
local driver = require("game.online.coordinator_driver")
local fixture = require("game.online.coordinator_fixture")
local input_frame = require("sim.input_frame")
local protocol = require("game.online.protocol")
local protocol_fixture = require("game.online.protocol_fixture")
local t = require("spec.support.runner")

local HOST = fixture.HOST_PEER_ID
local SESSION = fixture.manifest().session_id

-- The thirteen wire digests that shipped before pair preferences existed. Two
-- message kinds were appended to the conformance fixture, never inserted, so
-- every one of these must still be byte-identical.
local SHIPPED_DIGESTS = {
    handshake = "839047279d13a02d",
    manifest_proposal = "6d3818a72c39032b",
    manifest_accept = "a292fa4a99393456",
    peer_assignment = "fa48b31571dfe543",
    slot_assignment = "aae4def8b6d64d92",
    ready = "d1a83352b259e76b",
    countdown = "3c40b7699ce5861e",
    start = "d4fff9c7d4ba31b7",
    match_phase = "1671940891b78f1f",
    hash_report = "4405d9323b1e5b0f",
    result_ack = "5f466e6740c6d4cf",
    abort = "9db9c05e9728c4c1",
    disconnect = "a7599b154bb86cec",
}

---@param kind SessionMessageKind
---@param peer_id string
---@param sequence integer
---@param body SessionMessageBody
---@return SessionControlMessage
local function message(kind, peer_id, sequence, body)
    return assert(protocol.new(kind, SESSION, peer_id, sequence, body))
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

-- A host that has admitted `guest_count` guests, proposed the mode's manifest,
-- seen every acceptance, and published the planned block ownership.
---@param mode SessionMatchMode
---@param guest_count integer
---@return CoordinatorState
local function assigned_host(mode, guest_count)
    local state = fixture.host()
    for index = 1, guest_count do
        local peer_id = fixture.guest_peer_id(index)
        state = (
            deliver(
                state,
                peer_id,
                message("handshake", peer_id, 0, {
                    role = "guest",
                    runtime = fixture.runtime(),
                })
            )
        )
    end
    state = (
        coordinator.step(state, { kind = "propose_manifest", manifest = fixture.manifest(mode) })
    )
    local manifest_id = assert(state.manifest_id)
    for index = 1, guest_count do
        local peer_id = fixture.guest_peer_id(index)
        state = (
            deliver(
                state,
                peer_id,
                message("manifest_accept", peer_id, 1, {
                    manifest_id = manifest_id,
                })
            )
        )
    end
    return (
        coordinator.step(state, {
            kind = "assign_slots",
            assignments = fixture.assignments(guest_count, mode),
        })
    )
end

---@param state CoordinatorState
---@param producer_id string
---@return string
local function owned_text(state, producer_id)
    return table.concat(coordinator.owned_slots(state, producer_id), ",")
end

-- Every canonical slot has exactly one producer, and no human owns more or
-- fewer slots than the mode seats. Called after every interleaving so a double
-- ownership cannot hide behind a passing assertion elsewhere.
---@param state CoordinatorState
---@param mode SessionMatchMode
local function assert_partition(state, mode)
    local assignments = assert(state.assignments, "ownership is unpublished")
    local shape = assert(protocol.MATCH_MODES[mode])
    ---@type table<string, integer>
    local counts = {}
    for index = 1, input_frame.SLOT_COUNT do
        local producer = assignments[index]
        t.eq(producer.slot, assert(input_frame.slot(index)).id, "canonical order broke")
        if producer.producer_kind == "peer" then
            counts[producer.producer_id] = (counts[producer.producer_id] or 0) + 1
        end
    end
    for _, peer in ipairs(state.peers) do
        t.eq(
            counts[peer.peer_id] or 0,
            shape.slots_per_human,
            peer.peer_id .. " owns the wrong number of slots"
        )
    end
    t.is_true(
        protocol.validate_assignment_manifest(assert(state.manifest), assignments) == true,
        "ownership stopped validating against the manifest"
    )
end

-- Every set of `size` canonical slots, so a mode's whole request space can be
-- swept instead of sampled.
---@param size integer
---@return InputSlotId[][]
local function slot_sets(size)
    ---@type InputSlotId[][]
    local sets = {}
    ---@param start integer
    ---@param chosen InputSlotId[]
    local function walk(start, chosen)
        if #chosen == size then
            local copy = {}
            for index, slot in ipairs(chosen) do
                copy[index] = slot
            end
            sets[#sets + 1] = copy
            return
        end
        for index = start, input_frame.SLOT_COUNT do
            chosen[#chosen + 1] = assert(input_frame.slot(index)).id
            walk(index + 1, chosen)
            chosen[#chosen] = nil
        end
    end
    walk(1, {})
    return sets
end

t.describe("pair preference wire", function()
    t.it("keeps every digest that shipped before it, and the manifest id", function()
        local report = conformance.verify()
        t.eq(report.manifest_id, "ea39ebe78423e0a0", "the manifest is untouched by this change")
        for kind, digest in pairs(SHIPPED_DIGESTS) do
            t.eq(conformance.GOLDEN.wire_digests[kind], digest, kind .. " wire digest moved")
        end
        t.eq(conformance.GOLDEN.wire_digests.pair_preference, "a8f84f2616677df5")
        t.eq(conformance.GOLDEN.wire_digests.pair_preference_result, "df611e121f1ce5ba")
    end)

    t.it("round-trips a request and a verdict through the canonical codec", function()
        local request = message("pair_preference", "guest.1", 4, {
            manifest_id = ("a"):rep(16),
            assignment_id = ("b"):rep(16),
            slots = { "home_1", "home_3" },
        })
        local wire = assert(protocol.encode(request))
        t.eq(assert(protocol.encode(assert(protocol.decode(wire)))), wire)

        local verdict = message("pair_preference_result", HOST, 4, {
            manifest_id = ("a"):rep(16),
            assignment_id = ("b"):rep(16),
            slots = { "home_1", "home_3" },
            status = "rejected",
            reason = "already_taken",
        })
        local verdict_wire = assert(protocol.encode(verdict))
        t.eq(assert(protocol.encode(assert(protocol.decode(verdict_wire)))), verdict_wire)
    end)

    t.it("refuses a slot set that is not canonical, unique, and bounded", function()
        local cases = {
            { name = "descending", slots = { "home_2", "home_1" } },
            { name = "duplicate", slots = { "home_1", "home_1" } },
            { name = "unknown slot", slots = { "home_9" } },
            -- Keepers hold no canonical slot, so the vocabulary cannot name one.
            { name = "keeper player", slots = { "ozzo" } },
            { name = "empty", slots = {} },
            { name = "sparse", slots = { [1] = "home_1", [3] = "home_3" } },
            { name = "not strings", slots = { 1, 2 } },
            { name = "not an array", slots = { home_1 = true } },
        }
        for _, case in ipairs(cases) do
            local value, _, code = protocol.validate_slot_set(case.slots)
            t.eq(value, nil, case.name)
            t.eq(code, "malformed", case.name)
        end
        t.is_true(protocol.validate_slot_set({ "home_1", "away_4" }) == true)
    end)

    t.it("binds the typed reason to the rejected status and to nothing else", function()
        ---@param status any
        ---@param reason any
        ---@return any
        local function validate(status, reason)
            local body = {
                manifest_id = ("a"):rep(16),
                assignment_id = ("b"):rep(16),
                slots = { "home_1" },
                status = status,
                reason = reason,
            }
            return (
                protocol.validate({
                    version = protocol.VERSION,
                    kind = "pair_preference_result",
                    session_id = SESSION,
                    peer_id = HOST,
                    sequence = 0,
                    message_id = assert(protocol.message_id(SESSION, HOST, 0)),
                    body = body,
                })
            )
        end
        t.eq(validate("rejected", nil), nil, "a refusal must carry a reason")
        t.eq(validate("rejected", "no_thanks"), nil, "the reason vocabulary is closed")
        -- Silence is only observable at the end that waited, so a host claiming
        -- it would be reporting something it cannot know. Keeping the one
        -- locally minted reason off the wire is what leaves the accepted
        -- message vocabulary -- and both digests above -- exactly as they were.
        t.is_true(protocol.PREFERENCE_REJECTIONS.no_response == true)
        t.eq(validate("rejected", "no_response"), nil, "a host cannot report silence")
        t.eq(validate("granted", "already_taken"), nil, "only a refusal carries a reason")
        t.eq(validate("maybe", nil), nil, "the status vocabulary is closed")
        t.is_true(validate("granted", nil) == true)
        t.is_true(validate("unchanged", nil) == true)
        t.is_true(validate("rejected", "wrong_team") == true)
    end)

    t.it("is legal exactly in the pre-start phases, countdown included", function()
        local request = message("pair_preference", "guest.1", 4, {
            manifest_id = ("a"):rep(16),
            assignment_id = ("b"):rep(16),
            slots = { "home_1" },
        })
        local result = message("pair_preference_result", HOST, 4, {
            manifest_id = ("a"):rep(16),
            assignment_id = ("b"):rep(16),
            slots = { "home_1" },
            status = "granted",
        })
        local legal = { assigned = true, ready = true, countdown = true }
        for _, phase in ipairs(coordinator.PHASES) do
            for _, control in ipairs({ request, result }) do
                local ok, _, code = protocol.validate_phase(control, phase)
                if legal[phase] then
                    t.is_true(ok == true, control.kind .. " must be legal in " .. phase)
                else
                    t.eq(ok, nil, control.kind .. " must be illegal in " .. phase)
                    t.eq(code, "invalid_phase", phase)
                end
            end
        end
    end)
end)

t.describe("pair preference rules", function()
    t.it("grants a same-team pair and pays the displaced peer in vacated slots", function()
        local state = assigned_host("2v2", 3)
        t.eq(owned_text(state, HOST), "home_1,home_2")
        t.eq(owned_text(state, "guest.1"), "home_3,home_4")

        local verdict = coordinator.evaluate_preference(state, HOST, { "home_1", "home_3" })
        t.eq(verdict.status, "granted")
        local granted = assert(verdict.assignments)
        t.eq(table.concat(protocol.owned_slots(granted, HOST), ","), "home_1,home_3")
        t.eq(table.concat(protocol.owned_slots(granted, "guest.1"), ","), "home_2,home_4")
        t.eq(table.concat(protocol.owned_slots(granted, "guest.2"), ","), "away_1,away_2")
        t.eq(table.concat(protocol.owned_slots(granted, "guest.3"), ","), "away_3,away_4")
    end)

    t.it("answers the set a peer already owns with unchanged", function()
        local state = assigned_host("2v2", 3)
        local verdict = coordinator.evaluate_preference(state, HOST, { "home_1", "home_2" })
        t.eq(verdict.status, "unchanged")
        t.eq(verdict.assignments, nil, "an unchanged verdict publishes nothing")
    end)

    t.it("refuses each ungrantable request with its own typed reason", function()
        local state = assigned_host("2v2", 3)
        ---@param slots any
        ---@param reason SessionPreferenceRejection
        ---@param peer_id string?
        local function refused(slots, reason, peer_id)
            local verdict = coordinator.evaluate_preference(state, peer_id or HOST, slots)
            t.eq(verdict.status, "rejected", reason)
            t.eq(verdict.reason, reason)
            t.eq(verdict.assignments, nil, "a refusal publishes nothing")
        end
        refused({ "home_1", "away_1" }, "wrong_team")
        refused({ "home_1" }, "invalid_slot")
        refused({ "home_1", "home_2", "home_3" }, "invalid_slot")
        refused({ "home_2", "home_1" }, "invalid_slot")
        refused({ "home_3", "home_4" }, "detached")
        refused({ "home_1", "home_3" }, "not_seated", "nobody")
    end)

    t.it("protects a pair its owner chose, and only that", function()
        local state = assigned_host("2v2", 3)
        -- `guest.1` claims the pair the host's plan gave it.
        state = (
            deliver(
                state,
                "guest.1",
                message("pair_preference", "guest.1", 2, {
                    manifest_id = assert(state.manifest_id),
                    assignment_id = assert(state.assignment_id),
                    slots = { "home_3", "home_4" },
                })
            )
        )
        local claimed = coordinator.evaluate_preference(state, HOST, { "home_1", "home_3" })
        t.eq(claimed.status, "rejected")
        t.eq(claimed.reason, "already_taken")
        -- The away pair claimed nothing, so it is still exchangeable.
        local open = coordinator.evaluate_preference(state, "guest.2", { "away_1", "away_3" })
        t.eq(open.status, "granted")
    end)

    t.it("refuses a preference once ownership is frozen", function()
        local session = driver.new({ guest_count = 3, mode = "2v2" }):reach_start()
        local host = session:host().state
        t.is_true(coordinator.is_frozen(host))
        local verdict = coordinator.evaluate_preference(host, "guest.1", { "home_3", "home_1" })
        t.eq(verdict.status, "rejected")
        t.eq(verdict.reason, "after_freeze")
    end)
end)

t.describe("pair preference inertness", function()
    -- The whole request space of each mode, swept. In `1v1` and `4v4` nothing
    -- is grantable; in `2v2` the same rule finds grants, which is what makes
    -- the first two results a property of the rule and not of a mode branch.
    ---@param mode SessionMatchMode
    ---@param guest_count integer
    ---@param peer_id string
    ---@return integer granted, table<SessionPreferenceRejection|"unchanged", integer>
    local function sweep(mode, guest_count, peer_id)
        local state = assigned_host(mode, guest_count)
        local shape = assert(protocol.MATCH_MODES[mode])
        local granted = 0
        ---@type table<string, integer>
        local answers = {}
        for _, slots in ipairs(slot_sets(shape.slots_per_human)) do
            local verdict = coordinator.evaluate_preference(state, peer_id, slots)
            if verdict.status == "granted" then
                granted = granted + 1
            else
                local key = verdict.reason or verdict.status
                answers[key] = (answers[key] or 0) + 1
            end
        end
        return granted, answers
    end

    t.it("cannot move a 1v1 human, whose owned set is the whole outfield line", function()
        local granted, answers = sweep("1v1", 1, HOST)
        t.eq(granted, 0, "1v1 must have nothing to choose")
        t.eq(answers.unchanged, 1, "exactly one 1v1 request is the line already owned")
    end)

    t.it("cannot move a 4v4 human, whose owned set is a single slot", function()
        local granted, answers = sweep("4v4", 7, HOST)
        t.eq(granted, 0, "4v4 must have nothing to choose")
        t.eq(answers.unchanged, 1)
        t.eq(answers.detached, 3, "the three same-team slots keep none of the owned set")
        t.eq(answers.wrong_team, 4)
    end)

    t.it("is equally inert in a 4v4 that AI is still filling", function()
        local granted = sweep("4v4", 2, HOST)
        t.eq(granted, 0, "a bot fill is not an opening in 4v4 either")
    end)

    t.it("finds real choices in 2v2 under the same rule", function()
        local granted = sweep("2v2", 3, HOST)
        t.is_true(granted > 0, "2v2 must be able to choose a pair")
    end)
end)

t.describe("pair preference generations", function()
    t.it("mints a grant through the existing assignment_id path and clears readiness", function()
        local state = assigned_host("2v2", 3)
        state = (coordinator.step(state, { kind = "set_ready", ready = true }))
        t.is_true(state.peers[1].ready)
        local epoch = state.assignment_epoch
        local previous = assert(state.assignment_id)

        local next_state, outcome =
            coordinator.step(state, { kind = "prefer_pair", slots = { "home_1", "home_3" } })
        t.is_true(outcome.accepted)
        t.eq(next_state.assignment_epoch, epoch + 1)
        t.eq(
            next_state.assignment_id,
            protocol.assignment_id(assert(next_state.assignments), epoch + 1),
            "a grant must mint its generation through the one existing path"
        )
        t.is_true(next_state.assignment_id ~= previous)
        t.eq(next_state.phase, "assigned")
        t.is_true(not next_state.peers[1].ready, "a granted pair clears readiness")
        assert_partition(next_state, "2v2")

        local published = false
        for _, action in ipairs(outcome.actions) do
            published = published
                or (action.kind == "send" and assert(action.message).kind == "slot_assignment")
        end
        t.is_true(published, "a grant republishes ownership on the ordinary path")
    end)

    t.it("mints nothing when the request is the pair already owned", function()
        local state = assigned_host("2v2", 3)
        state = (coordinator.step(state, { kind = "set_ready", ready = true }))
        local epoch = state.assignment_epoch
        local previous = assert(state.assignment_id)

        local next_state, outcome =
            coordinator.step(state, { kind = "prefer_pair", slots = { "home_1", "home_2" } })
        t.is_true(outcome.accepted)
        t.eq(#outcome.actions, 0, "an unchanged verdict publishes nothing")
        t.eq(next_state.assignment_epoch, epoch)
        t.eq(next_state.assignment_id, previous)
        t.is_true(next_state.peers[1].ready, "an unchanged verdict cannot clear readiness")
        t.eq(next_state.preference.status, "unchanged")
    end)

    t.it("drops every claim when the host republishes ownership itself", function()
        local state = assigned_host("2v2", 3)
        state = (coordinator.step(state, { kind = "prefer_pair", slots = { "home_1", "home_3" } }))
        t.eq(table.concat(assert(state.peers[1].pair_choice), ","), "home_1,home_3")
        state = (
            coordinator.step(state, {
                kind = "assign_slots",
                assignments = fixture.assignments(3, "2v2"),
            })
        )
        t.eq(state.peers[1].pair_choice, nil, "the host overruled every choice")
        t.eq(owned_text(state, HOST), "home_1,home_2", "the host's own plan is back in force")
    end)
end)

t.describe("pair preference sessions", function()
    ---@param mode SessionMatchMode
    ---@param guest_count integer
    ---@param latency integer?
    ---@return CoordinatorDriver
    local function assigned_session(mode, guest_count, latency)
        local session =
            driver.new({ guest_count = guest_count, mode = mode, latency_ticks = latency or 0 })
        local manifest = fixture.manifest(mode)
        -- With latency in the link, nothing is delivered inside one pump, so
        -- every setup step is settled on the fake clock before the next.
        local function settle()
            if session.latency_ticks > 0 then
                session:tick(session.latency_ticks + 1)
            else
                session:pump()
            end
        end
        session:connect_all()
        settle()
        session:send(HOST, { kind = "propose_manifest", manifest = manifest })
        settle()
        session:send(HOST, {
            kind = "assign_slots",
            assignments = assert(
                coordinator.plan_assignments(manifest, fixture.peer_ids(guest_count))
            ),
        })
        settle()
        return session
    end

    ---@param session CoordinatorDriver
    ---@param peer_id string
    ---@return CoordinatorPreference
    local function preference(session, peer_id)
        return assert(assert(session:node(peer_id)).state.preference)
    end

    t.it("carries a guest's chosen pair to the host and the grant back", function()
        local session = assigned_session("2v2", 3)
        session:send("guest.2", { kind = "prefer_pair", slots = { "away_1", "away_3" } })
        t.eq(preference(session, "guest.2").status, "pending")
        session:pump()

        local host = session:host().state
        t.eq(owned_text(host, "guest.2"), "away_1,away_3")
        t.eq(owned_text(host, "guest.3"), "away_2,away_4")
        assert_partition(host, "2v2")
        t.eq(preference(session, "guest.2").status, "granted")
        t.eq(
            owned_text(assert(session:node("guest.2")).state, "guest.2"),
            "away_1,away_3",
            "the requester holds the ownership before it reads the verdict"
        )
        t.eq(owned_text(assert(session:node("guest.3")).state, "guest.3"), "away_2,away_4")
        for _, node in ipairs(session.nodes) do
            t.is_true(not coordinator.is_terminal(node.state), node.peer_id .. " ended")
        end
    end)

    t.it("answers an ungrantable request with a typed reason and moves nothing", function()
        local session = assigned_session("2v2", 3)
        local before = owned_text(session:host().state, "guest.2")
        session:send("guest.2", { kind = "prefer_pair", slots = { "home_1", "away_1" } })
        session:pump()

        local answer = preference(session, "guest.2")
        t.eq(answer.status, "rejected")
        t.eq(answer.reason, "wrong_team")
        t.eq(owned_text(session:host().state, "guest.2"), before)
        t.eq(owned_text(assert(session:node("guest.2")).state, "guest.2"), before)
        t.eq(session:host().state.assignment_epoch, 1, "a refusal mints no generation")
        t.is_true(not coordinator.is_terminal(session:host().state))
    end)

    t.it("is idempotent when a reliable transport retransmits the request", function()
        local session = assigned_session("2v2", 3)
        session:send("guest.2", { kind = "prefer_pair", slots = { "away_1", "away_3" } })
        session:pump()
        local epoch = session:host().state.assignment_epoch
        local owned = owned_text(session:host().state, "guest.2")

        session:replay("guest.2", HOST, session:first_wire("pair_preference"))
        t.eq(session:host().state.assignment_epoch, epoch, "a retransmission cannot re-apply")
        t.eq(owned_text(session:host().state, "guest.2"), owned)
        assert_partition(session:host().state, "2v2")
    end)

    t.it("repeats without doubling when the guest asks again for what it holds", function()
        local session = assigned_session("2v2", 3)
        for _ = 1, 3 do
            session:send("guest.2", { kind = "prefer_pair", slots = { "away_1", "away_3" } })
            session:pump()
        end
        t.eq(session:host().state.assignment_epoch, 2, "only the first request moved ownership")
        t.eq(owned_text(session:host().state, "guest.2"), "away_1,away_3")
        t.eq(preference(session, "guest.2").status, "unchanged")
        assert_partition(session:host().state, "2v2")
    end)

    t.it("never lets two guests racing for one slot both own it", function()
        -- Both away humans want `away_3`, and both speak before either has seen
        -- the other's ownership: the requests cross on the wire.
        local session = assigned_session("2v2", 3, 1)
        session:send("guest.2", { kind = "prefer_pair", slots = { "away_1", "away_3" } })
        session:send("guest.3", { kind = "prefer_pair", slots = { "away_3", "away_4" } })
        t.eq(
            preference(session, "guest.2").assignment_id,
            preference(session, "guest.3").assignment_id
        )
        session:tick(2)

        local host = session:host().state
        assert_partition(host, "2v2")
        t.eq(owned_text(host, "guest.2"), "away_1,away_3", "the first request won")
        t.eq(owned_text(host, "guest.3"), "away_2,away_4")
        t.eq(preference(session, "guest.2").status, "granted")
        local loser = preference(session, "guest.3")
        t.eq(loser.status, "rejected")
        t.eq(loser.reason, "superseded")

        -- Asking again against the ownership now in force is refused too: the
        -- winner claimed the slot, so it is no longer up for exchange.
        session:send("guest.3", { kind = "prefer_pair", slots = { "away_3", "away_4" } })
        session:tick(2)
        t.eq(preference(session, "guest.3").reason, "already_taken")
        assert_partition(session:host().state, "2v2")
        t.eq(owned_text(session:host().state, "guest.2"), "away_1,away_3")
    end)

    -- The freeze lands at the countdown, and a preference already in flight when
    -- it does is an ordinary race. It is answered, not fatal.
    ---@return CoordinatorDriver
    local function counting_down_session()
        local session = assigned_session("2v2", 3)
        for _, node in ipairs(session.nodes) do
            session:send(node.peer_id, { kind = "set_ready", ready = true })
        end
        session:pump()
        session:send(HOST, {
            kind = "begin_countdown",
            countdown_id = fixture.COUNTDOWN_ID,
            remaining_ticks = 30,
            first_input_tick = 0,
        })
        session:pump()
        return session
    end

    t.it("refuses a preference that arrives after the freeze without ending the session", function()
        local session = counting_down_session()
        local host = session:host().state
        t.eq(host.phase, "countdown")
        local freeze = assert(host.freeze)
        session:inject("guest.2", HOST, "pair_preference", {
            manifest_id = freeze.manifest_id,
            assignment_id = freeze.assignment_id,
            slots = { "away_1", "away_3" },
        })

        local after = session:host().state
        t.eq(after.assignment_id, freeze.assignment_id, "the frozen generation stands")
        t.eq(assert(after.freeze).assignment_id, freeze.assignment_id)
        t.eq(
            table.concat(assert(after.freeze).owned["guest.2"], ","),
            table.concat(freeze.owned["guest.2"], ","),
            "a frozen pair cannot move"
        )
        t.is_true(not coordinator.is_terminal(after), "a late preference is not fatal")
        local answered = false
        for _, control in ipairs(session.transcript) do
            if control.kind == "pair_preference_result" then
                answered = true
                t.eq(control.body.status, "rejected")
                t.eq(control.body.reason, "after_freeze")
            end
        end
        t.is_true(answered, "the host must answer the late request")
    end)

    -- Past the countdown every peer has seen `start`, and the phase table draws
    -- the line there: a preference is then as much a violation as a late
    -- `ready`, and ends the session the same way. Pinned because it is the
    -- boundary the `countdown` allowance was chosen against.
    t.it("treats a preference during the match as the protocol violation it is", function()
        local session = driver.new({ guest_count = 3, mode = "2v2" }):reach_start()
        local freeze = assert(session:host().state.freeze)
        session:inject("guest.2", HOST, "pair_preference", {
            manifest_id = freeze.manifest_id,
            assignment_id = freeze.assignment_id,
            slots = { "away_1", "away_3" },
        })
        local terminal = assert(session:host().state.terminal)
        t.eq(terminal.reason, "protocol_violation")
        t.eq(terminal.code, "invalid_phase")
    end)

    t.it("keeps a full 2v2 session reaching its start boundary after a granted pair", function()
        local session = assigned_session("2v2", 3)
        session:send("guest.2", { kind = "prefer_pair", slots = { "away_1", "away_3" } })
        session:pump()
        for _, node in ipairs(session.nodes) do
            session:send(node.peer_id, { kind = "set_ready", ready = true })
        end
        session:pump()
        session:send(HOST, {
            kind = "begin_countdown",
            countdown_id = fixture.COUNTDOWN_ID,
            remaining_ticks = 3,
            first_input_tick = 0,
        })
        session:pump()
        session:tick(4)
        t.is_true(session:all_started(), "every peer must reach the start boundary")
        local freeze = assert(session:host().state.freeze)
        t.eq(table.concat(freeze.owned["guest.2"], ","), "away_1,away_3")
        t.eq(freeze.live["guest.2"], "away_1")
    end)

    -- A host that is up but silent. The guest's link is blackholed in the
    -- guest->host direction only: neither end sees a `link_lost`, so neither
    -- takes the `transport_lost` path that already terminates correctly, and
    -- the request simply never lands. This is the case an unbounded `pending`
    -- never escaped.
    ---@param session CoordinatorDriver
    ---@param peer_id string
    ---@return CoordinatorDriverLink
    local function silence(session, peer_id)
        local link = assert(session:link(fixture.link_id(peer_id)))
        link.guest_open = false
        return link
    end

    t.it("gives up on a request a live host never answers", function()
        local session = assigned_session("2v2", 3)
        local before = owned_text(assert(session:node("guest.2")).state, "guest.2")
        silence(session, "guest.2")
        session:send("guest.2", { kind = "prefer_pair", slots = { "away_1", "away_3" } })
        t.eq(preference(session, "guest.2").status, "pending")

        session:tick(coordinator.PREFERENCE_TIMEOUT_TICKS)
        t.eq(preference(session, "guest.2").status, "pending", "the wait is not cut short")
        session:tick(1)

        local answer = preference(session, "guest.2")
        t.eq(answer.status, "rejected")
        t.eq(answer.reason, "no_response")
        t.eq(answer.deadline, nil, "an answered request waits on nothing")
        t.eq(table.concat(answer.slots, ","), "away_1,away_3", "the request stays legible")
        t.eq(owned_text(assert(session:node("guest.2")).state, "guest.2"), before)
        t.eq(owned_text(session:host().state, "guest.2"), before)
        t.eq(session:host().state.assignment_epoch, 1, "an expiry mints no generation")
        assert_partition(session:host().state, "2v2")
        for _, node in ipairs(session.nodes) do
            t.is_true(not coordinator.is_terminal(node.state), node.peer_id .. " ended")
        end
    end)

    t.it("cannot be moved by a verdict that arrives after the wait expired", function()
        local session = assigned_session("2v2", 3)
        silence(session, "guest.2")
        session:send("guest.2", { kind = "prefer_pair", slots = { "away_1", "away_3" } })
        session:tick(coordinator.PREFERENCE_TIMEOUT_TICKS + 1)
        t.eq(preference(session, "guest.2").reason, "no_response")

        local guest = assert(session:node("guest.2")).state
        local before = owned_text(guest, "guest.2")
        local generation = assert(guest.assignment_id)
        local epoch = guest.assignment_epoch
        -- The host finally speaks, and grants exactly what was asked for. The
        -- requester stopped waiting, so this answers no pending request and is
        -- dropped: ownership only ever moves on `slot_assignment`.
        session:inject(HOST, "guest.2", "pair_preference_result", {
            manifest_id = assert(guest.manifest_id),
            assignment_id = generation,
            slots = { "away_1", "away_3" },
            status = "granted",
        })

        local after = assert(session:node("guest.2")).state
        t.eq(owned_text(after, "guest.2"), before, "a late grant cannot move ownership")
        t.eq(after.assignment_id, generation, "a late grant mints no generation")
        t.eq(after.assignment_epoch, epoch)
        t.eq(assert(after.preference).status, "rejected")
        t.eq(assert(after.preference).reason, "no_response", "a terminal outcome is not reopened")
        t.is_true(not coordinator.is_terminal(after), "a late grant is not fatal")
    end)

    t.it("frees the guest to ask again once the wait has expired", function()
        local session = assigned_session("2v2", 3)
        local link = silence(session, "guest.2")
        session:send("guest.2", { kind = "prefer_pair", slots = { "away_1", "away_3" } })
        session:tick(coordinator.PREFERENCE_TIMEOUT_TICKS + 1)
        t.eq(preference(session, "guest.2").reason, "no_response")

        link.guest_open = true
        session:send("guest.2", { kind = "prefer_pair", slots = { "away_1", "away_3" } })
        t.eq(preference(session, "guest.2").status, "pending")
        session:pump()
        t.eq(preference(session, "guest.2").status, "granted")
        t.eq(owned_text(session:host().state, "guest.2"), "away_1,away_3")
        t.eq(owned_text(session:host().state, "guest.3"), "away_2,away_4")
        assert_partition(session:host().state, "2v2")
    end)

    -- `superseded` is what protects ownership when the answer is late rather
    -- than absent, so it is re-proven now that a wait can also end by itself:
    -- the host answers the loser well inside the deadline, and running the
    -- clock past that deadline afterwards must not reach back and overwrite
    -- the typed reason the host actually gave.
    t.it("still refuses a request that outlived its ownership generation", function()
        local session = assigned_session("2v2", 3, 1)
        session:send("guest.2", { kind = "prefer_pair", slots = { "away_1", "away_3" } })
        session:send("guest.3", { kind = "prefer_pair", slots = { "away_3", "away_4" } })
        session:tick(2)
        t.eq(preference(session, "guest.2").status, "granted")
        local loser = preference(session, "guest.3")
        t.eq(loser.status, "rejected")
        t.eq(loser.reason, "superseded")
        t.eq(loser.deadline, nil, "an answered request stops waiting")
        t.eq(owned_text(session:host().state, "guest.2"), "away_1,away_3")
        assert_partition(session:host().state, "2v2")

        session:tick(coordinator.PREFERENCE_TIMEOUT_TICKS + 1)
        t.eq(preference(session, "guest.3").reason, "superseded")
        t.eq(owned_text(session:host().state, "guest.2"), "away_1,away_3")
    end)
end)

t.describe("pair preference keeper protection", function()
    t.it("cannot name a keeper, because a keeper holds no canonical slot", function()
        local manifest = protocol_fixture.manifest("2v2")
        ---@type table<string, boolean>
        local keepers = {}
        for _, team in ipairs(manifest.teams) do
            for _, player in ipairs(team.roster) do
                if player.position == "keeper" then
                    keepers[player.player_id] = true
                end
            end
        end
        for _, keeper in ipairs({ "ozzo", "gax_oru" }) do
            t.is_true(keepers[keeper], keeper .. " must be a fixture keeper")
            t.eq(protocol.slot_index(keeper), nil, "a keeper has no canonical slot")
        end
        local state = assigned_host("2v2", 3)
        for index = 1, input_frame.SLOT_COUNT do
            t.is_true(
                not keepers[assert(state.assignments)[index].player_id],
                "a keeper reached a canonical slot"
            )
        end
    end)
end)
