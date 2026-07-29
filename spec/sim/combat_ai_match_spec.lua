-- Fixed-seed match scenarios for the deterministic combat AI.
--
-- These drive the whole seam: observation -> policy -> materialized MatchInput
-- -> `combat.prepare_inputs` -> resolver. Every scenario pins its geometry each
-- tick, so what is being tested is the decision, not where the movement AI
-- happened to wander.

local Vec2 = require("core.vec2")
local combat = require("sim.combat")
local combat_intent = require("sim.combat_intent")
local combat_policy = require("sim.combat_policy")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local players_data = require("data.players")
local slot_input = require("sim.slot_input")
local teams = require("data.teams")
local t = require("spec.support.runner")

local SOURCE = 2
local TARGET = 7

---@param family ActionFamilyId
---@return string
local function loadout_for(family)
    return ({
        unarmed = "loadout_spring_gloves",
        guard = "loadout_emberguard_shield",
        light_melee = "loadout_vector_blade",
        ranged = "loadout_pulse_blaster",
    })[family]
end

-- Every outfielder gets the same family, so a scenario exercises exactly one.
---@param family ActionFamilyId
---@param seed integer?
---@return MatchState
---@return CombatMatchState
local function family_match(family, seed)
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        seed = seed or 19,
    })
    state.kickoff_hold = 0
    -- A pure AI-versus-AI fixture. `human_controlled = false` stops the
    -- turnover auto-switch from handing `controlled` to whichever home player
    -- is nearest the ball, and parking `controlled` on the keeper keeps the one
    -- slot that still receives the passed MatchInput out of the outfield.
    state.human_controlled = false
    state.controlled = 1
    local pool = {}
    for _, player in ipairs(players_data) do
        local copied = {}
        for key, value in pairs(player) do
            copied[key] = value
        end
        if copied.loadout_id then
            copied.loadout_id = loadout_for(family)
        end
        pool[player.id] = copied
    end
    return state, combat.new_state(state, pool)
end

---@param state MatchState
local function park_everyone(state)
    for index, player in ipairs(state.players) do
        player.pos = Vec2.new(60 + index * 4, 60 + index * 4)
        player.vel = Vec2.new(0, 0)
        player.run_vel = Vec2.new(0, 0)
    end
end

---@param state MatchState
---@param source_pos Vec2
---@param target_pos Vec2
local function pin(state, source_pos, target_pos)
    state.players[SOURCE].pos = source_pos
    state.players[SOURCE].facing = Vec2.new(1, 0)
    state.players[SOURCE].vel = Vec2.new(0, 0)
    state.players[TARGET].pos = target_pos
    state.players[TARGET].facing = Vec2.new(-1, 0)
    state.players[TARGET].vel = Vec2.new(0, 0)
    state.owner = TARGET
    state.ball = target_pos
    state.ball_vel = Vec2.new(0, 0)
end

-- The whole team decides on its own cadence, so several players can commit on
-- one tick. A scenario asks about ITS source, never "whoever fired first".
---@param state MatchState
---@param player_id string?
---@return string? reason
local function commit_reason(state, player_id)
    local prefix = "combat_commit_"
    for _, event in ipairs(state.events) do
        if
            event.kind:sub(1, #prefix) == prefix
            and (player_id == nil or event.player == player_id)
        then
            return event.kind:sub(#prefix + 1)
        end
    end
    return nil
end

---@param family ActionFamilyId
---@param source_pos Vec2
---@param target_pos Vec2
---@param ticks integer
---@return string? reason
---@return CombatMatchState combat_state
---@return MatchState state
local function run_family(family, source_pos, target_pos, ticks)
    local state, combat_state = family_match(family)
    park_everyone(state)
    local reason = nil
    for _ = 1, ticks do
        pin(state, source_pos, target_pos)
        match.step(state, fixed_clock.TICK_SECONDS, slot_input.neutral_match_input(), combat_state)
        reason = reason or commit_reason(state, state.players[SOURCE].id)
    end
    return reason, combat_state, state
end

t.describe("deterministic combat AI in a match", function()
    t.it("contests an opposing carrier with unarmed", function()
        local reason, combat_state =
            run_family("unarmed", Vec2.new(400, 270), Vec2.new(424, 270), 40)
        t.eq(reason, "carrier_contest")
        t.eq(combat_state.players[SOURCE].family_id, "unarmed")
        -- The resolver, not just the reason: the materialized press reached the
        -- commit gate and was accepted.
        t.is_true(combat_state.next_source_sequence > 1)
    end)

    t.it("contests an opposing carrier with light melee at its longer reach", function()
        local reason, combat_state =
            run_family("light_melee", Vec2.new(400, 270), Vec2.new(440, 270), 40)
        t.eq(reason, "carrier_contest")
        t.eq(combat_state.players[SOURCE].family_id, "light_melee")
    end)

    t.it("releases a ranged shot at a distant carrier and spawns its projectile", function()
        local state, combat_state = family_match("ranged")
        park_everyone(state)
        local source_pos = Vec2.new(300, 270)
        local target_pos = Vec2.new(520, 270)
        local reason, spawned = nil, false
        for _ = 1, 80 do
            pin(state, source_pos, target_pos)
            match.step(
                state,
                fixed_clock.TICK_SECONDS,
                slot_input.neutral_match_input(),
                combat_state
            )
            reason = reason or commit_reason(state, state.players[SOURCE].id)
            for _, combat_event in ipairs(combat_state.events) do
                if
                    combat_event.kind == "projectile_spawn"
                    and combat_event.source_index == SOURCE
                then
                    spawned = true
                end
            end
        end
        t.eq(reason, "carrier_contest")
        t.is_true(spawned, "the AI never turned its ranged commit into a projectile")
    end)

    t.it("raises a guard against a telegraphed hostile melee it can attribute", function()
        local state, combat_state = family_match("guard")
        park_everyone(state)
        -- The threat source is also the carrier, so guarding it is a
        -- `carrier_contest` and never an unattributed off-ball action.
        local source_pos = Vec2.new(400, 270)
        local target_pos = Vec2.new(424, 270)
        local reason, guarded = nil, false
        for _ = 1, 60 do
            pin(state, source_pos, target_pos)
            -- Re-arm the hostile telegraph each tick so the window stays open.
            local hostile = combat_state.players[TARGET]
            hostile.family_id = "light_melee"
            hostile.loadout_id = loadout_for("light_melee")
            hostile.phase = "windup"
            hostile.phase_ticks = 10
            hostile.source_sequence = hostile.source_sequence or 1
            match.step(
                state,
                fixed_clock.TICK_SECONDS,
                slot_input.neutral_match_input(),
                combat_state
            )
            reason = reason or commit_reason(state, state.players[SOURCE].id)
            if combat_state.players[SOURCE].phase == "guard" then
                guarded = true
            end
        end
        t.eq(reason, "carrier_contest")
        t.is_true(guarded, "the AI committed a guard but never reached the guard phase")
    end)

    t.it("punishes a recovering carrier with the recovery-punish reason", function()
        local state, combat_state = family_match("unarmed")
        park_everyone(state)
        local reason = nil
        for _ = 1, 40 do
            pin(state, Vec2.new(400, 270), Vec2.new(424, 270))
            local victim = combat_state.players[TARGET]
            victim.family_id = "unarmed"
            victim.loadout_id = loadout_for("unarmed")
            victim.phase = "recovery"
            victim.phase_ticks = 10
            victim.source_sequence = victim.source_sequence or 1
            match.step(
                state,
                fixed_clock.TICK_SECONDS,
                slot_input.neutral_match_input(),
                combat_state
            )
            reason = reason or commit_reason(state, state.players[SOURCE].id)
        end
        t.eq(reason, "recovery_punish")
    end)

    t.it("sidesteps an in-flight projectile it has no reason to contest", function()
        local state, combat_state = family_match("unarmed")
        park_everyone(state)
        local source = state.players[SOURCE]
        local shooter = state.players[TARGET]
        local juked, dodged, contacted = false, false, false
        for _ = 1, 60 do
            -- No purpose predicate can hold: the ball is loose and nowhere near
            -- either body, so this player has nothing to contest and the policy
            -- has no candidate at all. Spacing is the only answer available.
            source.pos = Vec2.new(400, 270)
            source.facing = Vec2.new(1, 0)
            source.vel = Vec2.new(0, 0)
            shooter.pos = Vec2.new(900, 60)
            state.owner = nil
            state.ball = Vec2.new(900, 60)
            state.ball_vel = Vec2.new(0, 0)
            -- Re-arm the same inbound projectile every tick so the threat is
            -- always the same distance out whenever the personal cadence beat
            -- lands. At 300 px/s the 40 px gap closes in 7 ticks, which sits
            -- between EVADE_MIN_LEAD_TICKS and EVADE_WINDOW_TICKS: late enough
            -- to be real, early enough that a player could have read it.
            combat_state.projectiles[1] = {
                family_id = "ranged",
                source_index = TARGET,
                source_sequence = 1,
                pos = Vec2.new(440, 270),
                dir = Vec2.new(-1, 0),
                remaining_ticks = 40,
            }
            match.step(
                state,
                fixed_clock.TICK_SECONDS,
                slot_input.neutral_match_input(),
                combat_state
            )
            for _, event in ipairs(state.events) do
                if event.kind == "juke" and event.player == source.id then
                    juked = true
                end
            end
            if source.dodge_timer > 0 then
                dodged = true
            end
            for _, event in ipairs(combat_state.events) do
                if event.kind == "contact" and event.target_index == SOURCE then
                    contacted = true
                end
            end
        end
        t.is_true(juked, "the AI never emitted the juke its sidestep is made of")
        t.is_true(dodged, "the AI never entered the dodge window")
        -- The sidestep is mechanical, not a pose: `sim.combat` skips a dodging
        -- body when it selects projectile targets.
        t.is_true(not contacted, "the sidestep did not actually avoid the projectile")
    end)

    t.it("sidesteps away from the threat rather than into it", function()
        local state, combat_state = family_match("unarmed")
        park_everyone(state)
        local source = state.players[SOURCE]
        local shooter = state.players[TARGET]
        -- Facing +x with the threat arriving from below: the sidestep has to
        -- carry the body upward. Getting the perpendicular sign backwards would
        -- step into the projectile and still pass a "did it dodge" assertion.
        for _ = 1, 60 do
            source.pos = Vec2.new(400, 270)
            source.facing = Vec2.new(1, 0)
            source.vel = Vec2.new(0, 0)
            shooter.pos = Vec2.new(900, 60)
            state.owner = nil
            state.ball = Vec2.new(900, 60)
            state.ball_vel = Vec2.new(0, 0)
            combat_state.projectiles[1] = {
                family_id = "ranged",
                source_index = TARGET,
                source_sequence = 1,
                pos = Vec2.new(400, 310),
                dir = Vec2.new(0, -1),
                remaining_ticks = 40,
            }
            match.step(
                state,
                fixed_clock.TICK_SECONDS,
                slot_input.neutral_match_input(),
                combat_state
            )
            if source.dodge_timer > 0 then
                -- The projectile comes from +y, so a correct sidestep points at
                -- -y. `dodge_dir` is the authored juke direction the movement
                -- path then follows.
                t.is_true(
                    source.dodge_dir.y < 0,
                    "the sidestep pointed toward the incoming projectile"
                )
                return
            end
        end
        t.is_true(false, "the AI never sidestepped a threat arriving from the side")
    end)

    t.it("never commits against a protected keeper or lets a keeper commit", function()
        local state, combat_state = family_match("unarmed")
        park_everyone(state)
        for _ = 1, 90 do
            -- Every outfielder is crowded around the opposing keeper, which is
            -- the only body in reach.
            for index = 2, 5 do
                state.players[index].pos = Vec2.new(400 + index, 270)
                state.players[index].facing = Vec2.new(1, 0)
            end
            state.players[6].pos = Vec2.new(424, 270)
            for index = 7, 10 do
                state.players[index].pos = Vec2.new(60, 40 + index * 8)
            end
            state.owner = 6
            state.ball = state.players[6].pos
            match.step(
                state,
                fixed_clock.TICK_SECONDS,
                slot_input.neutral_match_input(),
                combat_state
            )
            t.is_true(
                commit_reason(state, nil) == nil,
                "the AI committed with only a protected keeper in reach"
            )
            for _, event in ipairs(combat_state.events) do
                t.is_true(event.target_index ~= 6, "a keeper was targeted")
            end
        end
        for _, index in ipairs({ 1, 6 }) do
            t.eq(combat_state.players[index].family_id, nil)
            t.eq(combat_state.players[index].phase, "ready")
            t.eq(combat_state.players[index].intent.stage, "idle")
            t.eq(combat_state.players[index].intent.reason, "none")
        end
    end)

    t.it("reproduces the same match byte for byte at every boundary", function()
        -- Capturing every boundary, not just the last, is what proves the
        -- retained intent survives strict snapshot capture: `capture` rejects a
        -- combat commitment that conflicts with a live run assignment, so a tick
        -- where the AI committed while holding a run would fail here rather than
        -- silently later.
        ---@return string[]
        local function run()
            local state, combat_state = family_match("light_melee", 8081)
            local hashes = {}
            for _ = 1, 360 do
                match.step(
                    state,
                    fixed_clock.TICK_SECONDS,
                    slot_input.neutral_match_input(),
                    combat_state
                )
                hashes[#hashes + 1] =
                    match_snapshot.hash(match_snapshot.capture(state, combat_state))
            end
            return hashes
        end
        local first = run()
        local second = run()
        t.eq(#second, #first)
        for index = 1, #first do
            t.eq(second[index], first[index], "boundary " .. index .. " diverged")
        end
    end)

    t.it("keeps off-ball commits rare over a whole match", function()
        local state, combat_state = family_match("unarmed", 20003)
        local by_reason = {}
        local total = 0
        for _ = 1, 3600 do
            match.step(
                state,
                fixed_clock.TICK_SECONDS,
                slot_input.neutral_match_input(),
                combat_state
            )
            for _, event in ipairs(state.events) do
                local prefix = "combat_commit_"
                if event.kind:sub(1, #prefix) == prefix then
                    local reason = event.kind:sub(#prefix + 1)
                    by_reason[reason] = (by_reason[reason] or 0) + 1
                    total = total + 1
                end
            end
        end
        t.is_true(total > 0, "a whole combat half produced no commit at all")
        -- Every commit carries one of the five purposes, never the closed
        -- zero-context outcome.
        t.eq(by_reason.unattributed_off_ball, nil)
        local on_ball = (by_reason.carrier_contest or 0)
            + (by_reason.loose_ball_contest or 0)
            + (by_reason.recovery_punish or 0)
        t.is_true(
            on_ball * 2 >= total,
            "most commits should be ball contests, not off-ball actions"
        )
    end)

    t.it("leaves the human-controlled slot's own input authoritative", function()
        local state, combat_state = family_match("unarmed")
        park_everyone(state)
        state.human_controlled = true
        for _ = 1, 60 do
            pin(state, Vec2.new(400, 270), Vec2.new(424, 270))
            -- The turnover auto-switch can move `controlled`; this scenario is
            -- about the slot the human actually holds at the decision boundary.
            state.controlled = SOURCE
            local input = slot_input.neutral_match_input()
            match.step(state, fixed_clock.TICK_SECONDS, input, combat_state)
            -- The human never asked for equipment, so the AI must not have
            -- asked on their behalf.
            t.eq(combat_state.players[SOURCE].phase, "ready")
            t.eq(combat_state.players[SOURCE].intent.stage, "idle")
        end
    end)
end)

t.describe("deterministic combat AI on fixed slots", function()
    ---@return MatchState
    ---@return CombatMatchState
    ---@return SlotInputProducerState
    local function slot_fixture()
        local state = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            seed = 733,
            input_ownership = match.ownership_for_teams(teams.nebula, teams.orion),
        })
        state.kickoff_hold = 0
        local sources = {}
        for index = 1, input_frame.SLOT_COUNT do
            sources[index] = { kind = "bot", seed = 1000 + index }
        end
        return state, combat.new_state(state), slot_input.new_producer(sources)
    end

    t.it("materializes legal equipment edges from declared bot fills", function()
        local state, combat_state, producer = slot_fixture()
        local seen = false
        for tick = 0, 200 do
            local base = assert(input_frame.neutral(tick))
            local frame = slot_input.materialize(producer, state, base, combat_state)
            t.is_true(input_frame.validate(frame))
            for index = 1, input_frame.SLOT_COUNT do
                local sample = frame.slots[index]
                t.is_true(input_frame.validate_sample(sample))
                if input_frame.has_edge(sample, "equipment_pressed") then
                    seen = true
                end
            end
            match.step(state, fixed_clock.TICK_SECONDS, frame, combat_state)
        end
        t.is_true(seen, "no declared bot fill ever requested equipment")
    end)

    t.it("replays a materialized bot tape without the producer", function()
        local state, combat_state, producer = slot_fixture()
        local frames = {}
        local hashes = {}
        for tick = 0, 120 do
            local base = assert(input_frame.neutral(tick))
            local frame = slot_input.materialize(producer, state, base, combat_state)
            frames[#frames + 1] = frame
            match.step(state, fixed_clock.TICK_SECONDS, frame, combat_state)
            hashes[#hashes + 1] = match_snapshot.hash(match_snapshot.capture(state, combat_state))
        end

        -- Replay through a producer that owns no bot at all: the tape alone is
        -- the authority, so every boundary must land on the same hash.
        local replay_state, replay_combat = slot_fixture()
        local frame_sources = {}
        for index = 1, input_frame.SLOT_COUNT do
            frame_sources[index] = { kind = "frame" }
        end
        local replay_producer = slot_input.new_producer(frame_sources)
        for index, frame in ipairs(frames) do
            local effective = slot_input.materialize(replay_producer, replay_state, frame, nil)
            match.step(replay_state, fixed_clock.TICK_SECONDS, effective, replay_combat)
            t.eq(
                match_snapshot.hash(match_snapshot.capture(replay_state, replay_combat)),
                hashes[index]
            )
        end
    end)

    t.it("leaves supplied frame rows byte-identical", function()
        local state, combat_state, _ = slot_fixture()
        local frame_sources = {}
        for index = 1, input_frame.SLOT_COUNT do
            frame_sources[index] = { kind = "frame" }
        end
        local producer = slot_input.new_producer(frame_sources)
        for tick = 0, 60 do
            local slots = {}
            for index = 1, input_frame.SLOT_COUNT do
                slots[index] = assert(input_frame.new_sample({
                    move_x = ((tick * 7 + index * 3) % 255) - 127,
                    move_y = ((tick * 11 + index * 5) % 255) - 127,
                    held = input_frame.HELD_BITS.equipment,
                }))
            end
            local base = assert(input_frame.new(tick, slots))
            local effective = slot_input.materialize(producer, state, base, combat_state)
            for index = 1, input_frame.SLOT_COUNT do
                t.eq(
                    assert(input_frame.encode_sample(effective.slots[index])),
                    assert(input_frame.encode_sample(base.slots[index]))
                )
            end
            match.step(state, fixed_clock.TICK_SECONDS, effective, combat_state)
        end
    end)

    t.it("reports the reason and digest behind every bot-fill commit", function()
        local state, combat_state, producer = slot_fixture()
        local commits, declines = 0, 0
        local seen_digest = nil
        for tick = 0, 300 do
            local base = assert(input_frame.neutral(tick))
            local frame, decisions = slot_input.materialize(producer, state, base, combat_state)
            for _, decision in ipairs(decisions) do
                t.eq(decision.player_index, state.slot_players[decision.slot])
                t.eq(#decision.digest, 16)
                if decision.action == "commit" then
                    commits = commits + 1
                    -- Exactly one stable reason from the five purposes, with the
                    -- frozen target it names.
                    t.is_true(
                        combat_intent.COMMIT_REASONS[decision.reason],
                        "a bot fill committed with reason " .. tostring(decision.reason)
                    )
                    t.is_true(decision.target_player ~= nil)
                    t.eq(decision.unavailable_reason, nil)
                    seen_digest = decision.digest
                    -- Readable back off the producer as well as off the return.
                    local recalled = assert(slot_input.last_decision(producer, decision.slot))
                    t.eq(recalled.reason, decision.reason)
                    t.eq(recalled.digest, decision.digest)
                elseif decision.action == "decline" then
                    declines = declines + 1
                    t.eq(decision.reason, "decline")
                    t.eq(decision.target_player, nil)
                else
                    t.eq(decision.action, "unavailable")
                    t.eq(decision.reason, "none")
                    t.is_true(decision.unavailable_reason ~= nil)
                end
            end
            match.step(state, fixed_clock.TICK_SECONDS, frame, combat_state)
        end
        t.is_true(commits > 0, "no bot fill ever committed, so nothing was reported")
        t.is_true(declines > 0, "declines are never surfaced")
        t.is_true(seen_digest ~= nil)

        -- `materialize` reports only the decisions made on the tick it just
        -- produced, while `last_decision` retains the most recent one across
        -- quiet ticks. A slot that has decided at least once keeps answering.
        local retained = 0
        for slot = 1, input_frame.SLOT_COUNT do
            if slot_input.last_decision(producer, slot) then
                retained = retained + 1
            end
        end
        t.is_true(retained > 0, "no slot retained its last decision")
        local base = assert(input_frame.neutral(301))
        local _, quiet = slot_input.materialize(producer, state, base, combat_state)
        t.is_true(#quiet <= input_frame.SLOT_COUNT)
        for slot = 1, input_frame.SLOT_COUNT do
            if slot_input.last_decision(producer, slot) then
                retained = retained - 1
            end
        end
        t.is_true(retained <= 0, "a quiet tick erased a retained decision")
    end)

    t.it("keeps the decision report out of the hashed simulation boundary", function()
        -- The report is a producer return value, never a MatchEvent: two peers
        -- must not diverge on whether their caller collected the diagnostic.
        ---@param collect boolean
        ---@return string
        local function run(collect)
            local state, combat_state, producer = slot_fixture()
            local hashes = {}
            for tick = 0, 120 do
                local base = assert(input_frame.neutral(tick))
                local frame, decisions = slot_input.materialize(producer, state, base, combat_state)
                if collect then
                    for _, decision in ipairs(decisions) do
                        t.is_true(decision.digest ~= nil)
                    end
                end
                match.step(state, fixed_clock.TICK_SECONDS, frame, combat_state)
                hashes[#hashes + 1] =
                    match_snapshot.hash(match_snapshot.capture(state, combat_state))
            end
            return table.concat(hashes, ";")
        end
        t.eq(run(true), run(false))
    end)

    t.it("uses the same policy id the session manifest names", function()
        t.eq(combat_policy.POLICY_ID, "gameplay_ai/combat/v1")
        t.eq(combat_policy.MANIFEST_POLICY_ID, "gameplay_ai.combat.v1")
        t.eq(
            require("game.online.match_manifest").GAMEPLAY_AI_POLICY_ID,
            combat_policy.MANIFEST_POLICY_ID
        )
    end)
end)
