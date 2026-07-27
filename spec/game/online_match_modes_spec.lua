local coordinator = require("game.online.coordinator")
local driver = require("game.online.coordinator_driver")
local fixture = require("game.online.coordinator_fixture")
local protocol = require("game.online.protocol")
local input_frame = require("sim.input_frame")
local t = require("spec.support.runner")

local SESSION = fixture.manifest().session_id
local HOST = fixture.HOST_PEER_ID
---@type SessionMatchMode[]
local MODES = { "1v1", "2v2", "4v4" }

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

-- Ownership shaped by hand rather than by `plan_assignments`, so the validators
-- are exercised against inputs the planner would never produce.
---@param mode SessionMatchMode
---@param humans integer
---@return SessionManifest, SessionSlotProducer[]
local function seated(mode, humans)
    local manifest = fixture.manifest(mode)
    local shape = assert(protocol.MATCH_MODES[mode])
    ---@type SessionSlotProducer[]
    local assignments = {}
    for index = 1, input_frame.SLOT_COUNT do
        local slot = assert(input_frame.slot(index))
        local order = math.floor((index - 1) / shape.slots_per_human) + 1
        if order <= humans then
            assignments[index] = {
                slot = slot.id,
                team = slot.team,
                player_id = manifest.slots[index].player_id,
                producer_kind = "peer",
                producer_id = "peer." .. tostring(order),
            }
        else
            assignments[index] = {
                slot = slot.id,
                team = slot.team,
                player_id = manifest.slots[index].player_id,
                producer_kind = "bot",
                producer_id = "bot." .. slot.id,
                bot_seed = 1000 + index,
            }
        end
    end
    return manifest, assignments
end

---@param assignments SessionSlotProducer[]
---@param producer_id string
---@return string
local function owned_text(assignments, producer_id)
    return table.concat(protocol.owned_slots(assignments, producer_id), ",")
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

-- A host that has admitted `guest_count` guests and seen every acceptance of a
-- manifest in `mode`, ready for ownership to be published.
---@param guest_count integer
---@param mode SessionMatchMode
---@return CoordinatorState
local function accepted_host(guest_count, mode)
    local state = fixture.host()
    for index = 1, guest_count do
        local peer_id = fixture.guest_peer_id(index)
        state = (
            deliver(
                state,
                peer_id,
                assert(protocol.new("handshake", SESSION, peer_id, 0, {
                    role = "guest",
                    runtime = fixture.runtime(),
                }))
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
                assert(
                    protocol.new(
                        "manifest_accept",
                        SESSION,
                        peer_id,
                        1,
                        { manifest_id = manifest_id }
                    )
                )
            )
        )
    end
    return state
end

t.describe("OMP-3 match modes", function()
    t.it("supports exactly three sizes that share the same eight canonical slots", function()
        local count = 0
        for mode, shape in pairs(protocol.MATCH_MODES) do
            count = count + 1
            t.eq(shape.mode, mode)
            t.eq(shape.humans * shape.slots_per_human, input_frame.SLOT_COUNT)
            t.eq(shape.team_humans * shape.slots_per_human, input_frame.HOME_SLOT_COUNT)
            t.eq(shape.team_humans * 2, shape.humans)
        end
        t.eq(count, 3)
        t.eq(assert(protocol.match_mode("1v1")).slots_per_human, 4)
        t.eq(assert(protocol.match_mode("2v2")).slots_per_human, 2)
        t.eq(assert(protocol.match_mode("4v4")).slots_per_human, 1)
    end)

    t.it("rejects 3v3 and every other unsupported size with a typed reason", function()
        for _, unsupported in ipairs({ "3v3", "5v5", "2v3", "4V4", "", "1v1 " }) do
            local shape, reason, code = protocol.match_mode(unsupported)
            t.eq(shape, nil, "match mode " .. unsupported .. " must be refused")
            t.eq(code, "unsupported_match_mode")
            t.is_true(type(reason) == "string" and #reason > 0)
        end
        for _, malformed in ipairs({ 3, true, {} }) do
            local _, _, code = protocol.match_mode(malformed)
            t.eq(code, "unsupported_match_mode")
        end
        local _, _, nil_code = protocol.match_mode(nil)
        t.eq(nil_code, "unsupported_match_mode")
    end)

    t.it("refuses an unsupported mode at manifest validation, before readiness", function()
        for _, mode in ipairs(MODES) do
            t.is_true(assert(protocol.validate_manifest(fixture.manifest(mode))))
        end
        local manifest = fixture.manifest()
        ---@type any
        local three = manifest
        three.match_mode = "3v3"
        local ok, _, code = protocol.validate_manifest(three)
        t.eq(ok, nil)
        t.eq(code, "unsupported_match_mode")

        three.match_mode = nil
        ok, _, code = protocol.validate_manifest(three)
        t.eq(ok, nil)
        t.eq(code, "unsupported_match_mode")
    end)

    t.it("carries the mode in the deterministic manifest identity", function()
        local ids = {}
        for _, mode in ipairs(MODES) do
            local id = protocol.manifest_id(fixture.manifest(mode))
            t.eq(ids[id], nil, "two modes share a manifest digest")
            ids[id] = mode
        end
        local difference =
            assert(protocol.manifest_difference(fixture.manifest("4v4"), fixture.manifest("2v2")))
        t.eq(difference.path, "manifest.match_mode")
        t.eq(difference.expected, "4v4")
        t.eq(difference.actual, "2v2")
    end)

    t.it("refuses a 3v3 proposal on the wire and at the coordinator", function()
        local manifest = fixture.manifest()
        local valid_id = protocol.manifest_id(manifest)
        ---@type any
        local three = deep_copy(manifest)
        three.match_mode = "3v3"
        local message, _, code = protocol.new("manifest_proposal", SESSION, HOST, 0, {
            manifest_id = valid_id,
            manifest = three,
        })
        t.eq(message, nil)
        t.eq(code, "unsupported_match_mode")

        local _, outcome =
            coordinator.step(fixture.host(), { kind = "propose_manifest", manifest = three })
        t.eq(outcome.accepted, false)
        t.eq(outcome.code, "unsupported_match_mode")
    end)
end)

t.describe("OMP-3 multi-slot ownership", function()
    t.it("lets one human cover several slots while every slot keeps one source", function()
        for _, mode in ipairs(MODES) do
            local shape = assert(protocol.MATCH_MODES[mode])
            local manifest, assignments = seated(mode, shape.humans)
            t.is_true(
                assert(protocol.validate_assignment_manifest(manifest, assignments)),
                mode .. " ownership must validate"
            )
            local sources = assert(coordinator.slot_sources(manifest, assignments))
            local seen = 0
            for index = 1, input_frame.SLOT_COUNT do
                local slot = assert(input_frame.slot(index))
                t.eq(sources[slot.id].slot, slot.id)
                seen = seen + 1
            end
            t.eq(seen, input_frame.SLOT_COUNT, mode .. " must declare every canonical slot once")
            for order = 1, shape.humans do
                t.eq(
                    #protocol.owned_slots(assignments, "peer." .. tostring(order)),
                    shape.slots_per_human
                )
            end
        end
    end)

    t.it("pins the owned sets each mode produces, in canonical order", function()
        local _, duo = seated("1v1", 2)
        t.eq(owned_text(duo, "peer.1"), "home_1,home_2,home_3,home_4")
        t.eq(owned_text(duo, "peer.2"), "away_1,away_2,away_3,away_4")

        local _, pair = seated("2v2", 4)
        t.eq(owned_text(pair, "peer.1"), "home_1,home_2")
        t.eq(owned_text(pair, "peer.2"), "home_3,home_4")
        t.eq(owned_text(pair, "peer.3"), "away_1,away_2")
        t.eq(owned_text(pair, "peer.4"), "away_3,away_4")

        local _, quad = seated("4v4", 8)
        for index = 1, input_frame.SLOT_COUNT do
            t.eq(owned_text(quad, "peer." .. tostring(index)), assert(input_frame.slot(index)).id)
        end
        t.eq(owned_text(quad, "nobody"), "")
    end)

    t.it("rejects owned-set sizes that disagree with the frozen mode", function()
        -- The identical assignments array is legal under one mode and refused
        -- under another: the mode, not the array alone, decides.
        for _, pairing in ipairs({
            { "1v1", "4v4" },
            { "1v1", "2v2" },
            { "2v2", "4v4" },
            { "4v4", "1v1" },
            { "4v4", "2v2" },
            { "2v2", "1v1" },
        }) do
            local shaped_for, claimed = pairing[1], pairing[2]
            local shape = assert(protocol.MATCH_MODES[shaped_for])
            local _, assignments = seated(shaped_for, shape.humans)
            local manifest = fixture.manifest(claimed)
            local ok, reason, code = protocol.validate_assignment_manifest(manifest, assignments)
            t.eq(ok, nil, shaped_for .. " ownership must not pass as " .. claimed)
            t.eq(code, "invalid_ownership")
            t.is_true(type(reason) == "string" and reason:find(claimed, 1, true) ~= nil)
        end
    end)

    t.it("rejects a partially seated owned set inside one mode", function()
        local manifest, assignments = seated("2v2", 4)
        -- Hand one of peer.1's two slots to peer.2, leaving 1 and 3.
        assignments[2].producer_id = "peer.2"
        local ok, _, code = protocol.validate_assignment_manifest(manifest, assignments)
        t.eq(ok, nil)
        t.eq(code, "invalid_ownership")
    end)

    t.it("keeps every declared source unambiguous while humans span slots", function()
        local manifest, assignments = seated("4v4", 8)

        local shared_bot = deep_copy(assignments)
        shared_bot[7].producer_kind = "bot"
        shared_bot[7].producer_id = "bot.away_4"
        shared_bot[7].bot_seed = 5
        shared_bot[8].producer_kind = "bot"
        shared_bot[8].producer_id = "bot.away_4"
        shared_bot[8].bot_seed = 6
        local ok, _, code = protocol.validate_assignment_manifest(manifest, shared_bot)
        t.eq(ok, nil, "a bot producer may drive only one slot")
        t.eq(code, "malformed")

        local crossed = deep_copy(assignments)
        crossed[8].producer_kind = "bot"
        crossed[8].producer_id = "peer.1"
        crossed[8].bot_seed = 7
        ok, _, code = protocol.validate_assignment_manifest(manifest, crossed)
        t.eq(ok, nil, "a producer id may not be both peer and bot")
        t.eq(code, "malformed")

        local straddling = fixture.manifest("2v2")
        local _, pair = seated("2v2", 4)
        pair[4].producer_id = "peer.3"
        pair[5].producer_id = "peer.2"
        ok, _, code = protocol.validate_assignment_manifest(straddling, pair)
        t.eq(ok, nil, "one human's owned slots must sit on one team")
        t.eq(code, "malformed")
    end)

    t.it("carries multi-slot ownership across the canonical wire", function()
        local manifest, assignments = seated("1v1", 2)
        local message = assert(protocol.new("slot_assignment", SESSION, HOST, 0, {
            manifest_id = protocol.manifest_id(manifest),
            assignment_id = protocol.assignment_id(assignments, 1),
            assignments = assignments,
        }))
        local wire = assert(protocol.encode(message))
        local decoded = assert(protocol.decode(wire))
        local body = decoded.body
        ---@cast body SessionSlotAssignmentBody
        t.eq(#protocol.owned_slots(body.assignments, "peer.1"), 4)
        t.is_true(assert(protocol.validate_assignment_manifest(manifest, body.assignments)))
    end)

    t.it("mints a new ownership generation when a pair is repartitioned", function()
        local _, assignments = seated("2v2", 4)
        local before = protocol.assignment_id(assignments, 1)
        -- Swap which pair each home human owns without changing the mode, the
        -- roster, or how many slots anybody owns.
        local repartitioned = deep_copy(assignments)
        repartitioned[1].producer_id = "peer.2"
        repartitioned[2].producer_id = "peer.2"
        repartitioned[3].producer_id = "peer.1"
        repartitioned[4].producer_id = "peer.1"
        t.is_true(before ~= protocol.assignment_id(repartitioned, 1))
        -- Even byte-identical ownership republished is a distinct generation.
        t.is_true(before ~= protocol.assignment_id(assignments, 2))
    end)

    t.it("keeps both keepers unassignable in every mode", function()
        for _, mode in ipairs(MODES) do
            local manifest, assignments = seated(mode, 1)
            for _, keeper in ipairs({ "ozzo", "gax_oru" }) do
                local attempt = deep_copy(assignments)
                attempt[1].player_id = keeper
                local sources, _, code = coordinator.slot_sources(manifest, attempt)
                t.eq(sources, nil, mode .. " must refuse a keeper in a canonical slot")
                t.eq(code, "invalid_assignment")
            end
            -- The manifest itself never names a keeper in a slot either.
            local named = deep_copy(manifest)
            named.slots[1].player_id = "ozzo"
            local ok, _, code = protocol.validate_manifest(named)
            t.eq(ok, nil)
            t.eq(code, "malformed")
            -- Keepers stay slotless: eight slots for ten roster players.
            t.eq(#manifest.slots, input_frame.SLOT_COUNT)
            t.eq(#manifest.teams[1].roster, input_frame.FIXTURE_TEAM_SIZE)
        end
    end)
end)

t.describe("OMP-3 mode-aware coordinator ownership", function()
    t.it("seats humans in contiguous owned blocks per mode", function()
        local duo = assert(coordinator.plan_assignments(fixture.manifest("1v1"), { "a", "b" }))
        t.eq(owned_text(duo, "a"), "home_1,home_2,home_3,home_4")
        t.eq(owned_text(duo, "b"), "away_1,away_2,away_3,away_4")

        local pair =
            assert(coordinator.plan_assignments(fixture.manifest("2v2"), { "a", "b", "c", "d" }))
        t.eq(owned_text(pair, "a"), "home_1,home_2")
        t.eq(owned_text(pair, "d"), "away_3,away_4")

        -- Bots fill the unseated blocks, exactly as they always did.
        local lonely = assert(coordinator.plan_assignments(fixture.manifest("1v1"), { "a" }))
        t.eq(owned_text(lonely, "a"), "home_1,home_2,home_3,home_4")
        for index = 5, input_frame.SLOT_COUNT do
            t.eq(lonely[index].producer_kind, "bot")
            t.eq(lonely[index].producer_id, "bot." .. assert(input_frame.slot(index)).id)
        end
    end)

    t.it("keeps 4v4 seating byte-identical to one slot per human", function()
        local manifest = fixture.manifest()
        local plan = assert(coordinator.plan_assignments(manifest, fixture.peer_ids(7)))
        for index = 1, input_frame.SLOT_COUNT do
            local slot = assert(input_frame.slot(index))
            t.eq(plan[index].slot, slot.id)
            t.eq(plan[index].producer_kind, "peer")
            t.eq(#protocol.owned_slots(plan, plan[index].producer_id), 1)
        end
        t.eq(plan[1].producer_id, HOST)
        t.eq(plan[8].producer_id, fixture.guest_peer_id(7))
    end)

    t.it("refuses more humans than a mode seats", function()
        local _, reason, code =
            coordinator.plan_assignments(fixture.manifest("1v1"), { "a", "b", "c" })
        t.eq(code, "capacity")
        t.is_true(type(reason) == "string" and reason:find("1v1", 1, true) ~= nil)

        _, _, code =
            coordinator.plan_assignments(fixture.manifest("2v2"), { "a", "b", "c", "d", "e" })
        t.eq(code, "capacity")

        t.is_true(coordinator.plan_assignments(fixture.manifest("4v4"), fixture.peer_ids(7)) ~= nil)
    end)

    t.it("refuses a proposal whose mode cannot seat the admitted lobby", function()
        local state = fixture.host()
        for index = 1, 3 do
            local peer_id = fixture.guest_peer_id(index)
            state = (
                deliver(
                    state,
                    peer_id,
                    assert(protocol.new("handshake", SESSION, peer_id, 0, {
                        role = "guest",
                        runtime = fixture.runtime(),
                    }))
                )
            )
        end
        local next_state, outcome = coordinator.step(
            state,
            { kind = "propose_manifest", manifest = fixture.manifest("1v1") }
        )
        t.eq(outcome.accepted, false)
        t.eq(outcome.code, "capacity")
        t.is_true(next_state == state, "a refused proposal leaves no progress behind")

        -- The same lobby is fine at a mode that seats it.
        local _, ok_outcome = coordinator.step(
            state,
            { kind = "propose_manifest", manifest = fixture.manifest("2v2") }
        )
        t.eq(ok_outcome.accepted, true)
    end)

    t.it("refuses ownership that disagrees with the admitted lobby or the mode", function()
        local state = accepted_host(1, "1v1")
        local good = assert(
            coordinator.plan_assignments(
                fixture.manifest("1v1"),
                { HOST, fixture.guest_peer_id(1) }
            )
        )
        local _, outcome = coordinator.step(state, { kind = "assign_slots", assignments = good })
        t.eq(outcome.accepted, true)

        -- 4v4-shaped ownership under a 1v1 manifest.
        local _, wrong_shape = seated("4v4", 2)
        wrong_shape[1].producer_id = HOST
        wrong_shape[2].producer_id = fixture.guest_peer_id(1)
        local _, shape_outcome =
            coordinator.step(state, { kind = "assign_slots", assignments = wrong_shape })
        t.eq(shape_outcome.accepted, false)
        t.eq(shape_outcome.code, "invalid_ownership")

        -- Correctly shaped for the mode, but leaving an admitted peer unseated.
        local unseated = assert(coordinator.plan_assignments(fixture.manifest("1v1"), { HOST }))
        local _, unseated_outcome =
            coordinator.step(state, { kind = "assign_slots", assignments = unseated })
        t.eq(unseated_outcome.accepted, false)
        t.eq(unseated_outcome.code, "invalid_assignment")
    end)

    t.it("freezes the mode, the owned sets, and the opening live slot", function()
        for _, case in ipairs({ { "1v1", 1 }, { "2v2", 3 }, { "4v4", 7 } }) do
            local mode, guest_count = case[1], case[2]
            local shape = assert(protocol.MATCH_MODES[mode])
            local session = driver.new({ guest_count = guest_count, mode = mode })
            session:reach_start(2, 0)
            t.is_true(session:all_started(), mode .. " never reached its start boundary")
            local freeze = assert(session:host().state.freeze)
            t.eq(freeze.match_mode, mode)
            local humans = 0
            for _, node in ipairs(session.nodes) do
                local owned = assert(freeze.owned[node.peer_id], "a human has no owned set")
                humans = humans + 1
                t.eq(#owned, shape.slots_per_human)
                t.eq(freeze.live[node.peer_id], owned[1])
            end
            t.eq(humans, guest_count + 1)

            -- Every peer froze the same ownership model, not merely its own row.
            for _, node in ipairs(session.nodes) do
                local peer_freeze = assert(node.state.freeze)
                t.eq(peer_freeze.match_mode, freeze.match_mode)
                t.eq(peer_freeze.assignment_id, freeze.assignment_id)
                for _, other in ipairs(session.nodes) do
                    t.eq(
                        table.concat(peer_freeze.owned[other.peer_id] or {}, ","),
                        table.concat(freeze.owned[other.peer_id] or {}, ",")
                    )
                    t.eq(peer_freeze.live[other.peer_id], freeze.live[other.peer_id])
                end
            end
        end
    end)

    t.it("reports owned sets from published and frozen ownership alike", function()
        local state = accepted_host(1, "2v2")
        t.eq(#coordinator.owned_slots(state, HOST), 0, "unpublished ownership owns nothing")
        local plan = assert(
            coordinator.plan_assignments(
                fixture.manifest("2v2"),
                { HOST, fixture.guest_peer_id(1) }
            )
        )
        state = (coordinator.step(state, { kind = "assign_slots", assignments = plan }))
        t.eq(table.concat(coordinator.owned_slots(state, HOST), ","), "home_1,home_2")
        t.eq(#coordinator.owned_slots(state, "stranger"), 0)
        t.eq(assert(coordinator.slot_owner(state, "home_2")).producer_id, HOST)
    end)

    t.it("plays a full 1v1 and a full 2v2 session through to a result", function()
        for _, case in ipairs({ { "1v1", 1 }, { "2v2", 3 } }) do
            local mode, guest_count = case[1], case[2]
            local session = driver.new({ guest_count = guest_count, mode = mode })
            session:reach_start(3, 0)
            t.is_true(session:all_started(), mode .. " never started")
            session:play_out(2, 1)
            t.is_true(session:all_terminal("completed"), mode .. " never completed")
        end
    end)
end)

t.describe("OMP-3 live slot selection", function()
    t.it("auto-switches to an owned slot that wins the ball", function()
        local owned = { "home_1", "home_2", "home_3", "home_4" }
        t.eq(
            coordinator.next_live_slot(owned, "home_1", {
                switch = false,
                winner = "home_3",
                ranked = { "home_2" },
            }),
            "home_3"
        )
        -- Winning the ball outranks a simultaneous switch request.
        t.eq(
            coordinator.next_live_slot(owned, "home_1", {
                switch = true,
                winner = "home_4",
                ranked = { "home_2", "home_3" },
            }),
            "home_4"
        )
        -- A slot outside the owned set winning the ball changes nothing.
        t.eq(
            coordinator.next_live_slot(owned, "home_1", {
                switch = false,
                winner = "away_2",
                ranked = { "home_3" },
            }),
            "home_1"
        )
    end)

    t.it("switches to the owned slot nearest the ball, and only when eligible", function()
        local owned = { "home_1", "home_2", "home_3", "home_4" }
        t.eq(
            coordinator.next_live_slot(owned, "home_1", {
                switch = true,
                ranked = { "away_1", "home_3", "home_2" },
            }),
            "home_3",
            "the nearest owned slot wins, skipping unowned ones"
        )
        t.eq(
            coordinator.next_live_slot(owned, "home_1", { switch = false, ranked = { "home_4" } }),
            "home_1",
            "no switch edge, no transition"
        )
        t.eq(
            coordinator.next_live_slot(owned, "home_1", {
                switch = true,
                carrier = "home_1",
                ranked = { "home_4" },
            }),
            "home_1",
            "switching while carrying the ball is refused, as in solo play"
        )
        t.eq(
            coordinator.next_live_slot(owned, "home_1", { switch = true, ranked = { "away_3" } }),
            "home_1",
            "an entirely unowned ranking leaves the live slot alone"
        )
        t.eq(
            coordinator.next_live_slot(owned, "home_2", { switch = true }),
            "home_2",
            "an absent ranking leaves the live slot alone"
        )
    end)

    t.it("is inert in 4v4 without any mode special case", function()
        local owned = { "home_2" }
        for _, switch in ipairs({ true, false }) do
            for _, winner in ipairs({ "home_2", "home_1", "away_4" }) do
                for _, carrier in ipairs({ "home_2", "away_1" }) do
                    t.eq(
                        coordinator.next_live_slot(owned, "home_2", {
                            switch = switch,
                            winner = winner,
                            carrier = carrier,
                            ranked = { "away_1", "home_1", "home_2", "home_3" },
                        }),
                        "home_2",
                        "a one-slot owned set can never change who is live"
                    )
                end
            end
        end
    end)

    t.it("keeps exactly one live slot per human and drives the rest with AI", function()
        for _, case in ipairs({ { "1v1", 1 }, { "2v2", 3 }, { "4v4", 7 } }) do
            local mode, guest_count = case[1], case[2]
            local shape = assert(protocol.MATCH_MODES[mode])
            local session = driver.new({ guest_count = guest_count, mode = mode })
            session:reach_start(2, 0)
            local freeze = assert(session:host().state.freeze)
            local drivers = coordinator.slot_drivers(freeze)
            local human_slots = 0
            for index = 1, input_frame.SLOT_COUNT do
                local slot = assert(input_frame.slot(index))
                local kind = assert(drivers[slot.id], "a canonical slot has no driver")
                if kind == "human" then
                    human_slots = human_slots + 1
                end
            end
            t.eq(human_slots, guest_count + 1, mode .. " must expose one live slot per human")

            -- Moving a human's live slot moves exactly one human row with it.
            if shape.slots_per_human > 1 then
                local owned = assert(freeze.owned[HOST])
                local moved = {}
                for peer_id, slot in pairs(freeze.live) do
                    moved[peer_id] = slot
                end
                moved[HOST] = owned[#owned]
                local after = coordinator.slot_drivers(freeze, moved)
                t.eq(after[owned[1]], "ai", "the vacated owned slot falls back to AI")
                t.eq(after[owned[#owned]], "human")
                local still_human = 0
                for _, kind in pairs(after) do
                    if kind == "human" then
                        still_human = still_human + 1
                    end
                end
                t.eq(still_human, human_slots)
            end
        end
    end)
end)
