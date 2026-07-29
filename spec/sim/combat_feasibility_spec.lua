-- `family_commit_feasibility/v1` and `intervention_candidate/v2`.
--
-- These are pure temporal predicates: they answer "could this family reach that
-- frozen identity from here" and "does a soccer purpose actually hold", never
-- "will it hit". Each family gets its own catalogued proof shape, so each one is
-- driven separately below.

local Vec2 = require("core.vec2")
local combat = require("sim.combat")
local combat_feasibility = require("sim.combat_feasibility")
local combat_observation = require("sim.combat_observation")
local combat_policy = require("sim.combat_policy")
local match = require("sim.match")
local teams = require("data.teams")
local t = require("spec.support.runner")

-- Home is 1..5 (1 is the keeper), away is 6..10 (6 is the keeper).
local SOURCE = 2
local TARGET = 7
local OTHER_OPPONENT = 8

---@return MatchState
---@return CombatMatchState
local function bare_match()
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        seed = 19,
    })
    state.kickoff_hold = 0
    local combat_state = combat.new_state(state)
    -- Park everyone far apart so each scenario states its own geometry.
    for index, player in ipairs(state.players) do
        player.pos = Vec2.new(40 + index * 3, 40 + index * 3)
        player.vel = Vec2.new(0, 0)
        player.facing = Vec2.new(player.team == "home" and 1 or -1, 0)
        combat_state.players[index].family_id = nil
        combat_state.players[index].loadout_id = nil
    end
    state.ball = Vec2.new(900, 500)
    state.ball_vel = Vec2.new(0, 0)
    state.owner = nil
    return state, combat_state
end

---@param state MatchState
---@param combat_state CombatMatchState
---@param index integer?
---@return CombatObservation
local function observe(state, combat_state, index)
    return combat_observation.build(
        state,
        combat_state,
        index or SOURCE,
        combat_policy.POLICY_ID,
        nil
    )
end

t.describe("family_commit_feasibility/v1", function()
    t.it("proves an unarmed swept melee contact inside reach and arc", function()
        local state, combat_state = bare_match()
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[SOURCE].facing = Vec2.new(1, 0)
        state.players[TARGET].pos = Vec2.new(424, 270)
        local observation = observe(state, combat_state)

        local witness = combat_feasibility.family_commit(observation, TARGET, "unarmed", nil)
        t.is_true(witness.feasible)
        t.eq(witness.family_id, "unarmed")
        t.eq(witness.target_player, TARGET)
        -- Six windup ticks, then the first of four active ticks.
        t.eq(witness.contact_tick, 7)
        t.eq(witness.horizon_ticks, 10)
    end)

    t.it("refuses an unarmed commit behind the source and beyond its reach", function()
        local state, combat_state = bare_match()
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[SOURCE].facing = Vec2.new(1, 0)
        state.players[TARGET].pos = Vec2.new(376, 270)
        t.is_true(
            not combat_feasibility.family_commit(
                observe(state, combat_state),
                TARGET,
                "unarmed",
                nil
            ).feasible
        )

        state.players[TARGET].pos = Vec2.new(520, 270)
        t.is_true(
            not combat_feasibility.family_commit(
                observe(state, combat_state),
                TARGET,
                "unarmed",
                nil
            ).feasible
        )
    end)

    t.it("reaches further with light melee than unarmed from the same pose", function()
        local state, combat_state = bare_match()
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[SOURCE].facing = Vec2.new(1, 0)
        state.players[TARGET].pos = Vec2.new(444, 270)
        local observation = observe(state, combat_state)
        t.is_true(
            not combat_feasibility.family_commit(observation, TARGET, "unarmed", nil).feasible
        )
        local witness = combat_feasibility.family_commit(observation, TARGET, "light_melee", nil)
        t.is_true(witness.feasible)
        -- Twelve windup ticks, then the first of five active ticks.
        t.eq(witness.contact_tick, 13)
    end)

    t.it("lets a searched movement tape make an out-of-reach target reachable", function()
        local state, combat_state = bare_match()
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[SOURCE].facing = Vec2.new(1, 0)
        state.players[TARGET].pos = Vec2.new(470, 270)
        local observation = observe(state, combat_state)
        t.is_true(
            not combat_feasibility.family_commit(observation, TARGET, "unarmed", nil).feasible
        )
        local moved = combat_feasibility.family_commit(observation, TARGET, "unarmed", {
            move_x = 1,
            move_y = 0,
            ticks = 20,
        })
        t.is_true(moved.feasible)
        t.eq(moved.commit_tick, 20)
    end)

    t.it("proves a ranged commit only along a clear projected line", function()
        local state, combat_state = bare_match()
        state.players[SOURCE].pos = Vec2.new(200, 270)
        state.players[SOURCE].facing = Vec2.new(1, 0)
        state.players[TARGET].pos = Vec2.new(400, 270)
        local clear = observe(state, combat_state)
        t.is_true(combat_feasibility.family_commit(clear, TARGET, "ranged", nil).feasible)

        -- Another opponent standing in front of the target takes the shot.
        state.players[OTHER_OPPONENT].pos = Vec2.new(320, 270)
        local blocked = observe(state, combat_state)
        t.is_true(not combat_feasibility.family_commit(blocked, TARGET, "ranged", nil).feasible)
        t.is_true(combat_feasibility.family_commit(blocked, OTHER_OPPONENT, "ranged", nil).feasible)
    end)

    t.it("never proves a commit against a protected keeper", function()
        local state, combat_state = bare_match()
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[SOURCE].facing = Vec2.new(1, 0)
        state.players[6].pos = Vec2.new(424, 270)
        local observation = observe(state, combat_state)
        for _, family in ipairs({ "unarmed", "light_melee", "ranged" }) do
            t.is_true(
                not combat_feasibility.family_commit(observation, 6, family, nil).feasible,
                family .. " proved a commit against a keeper"
            )
        end
    end)

    t.it("guards a hostile melee windup and nothing without a public threat", function()
        local state, combat_state = bare_match()
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[SOURCE].facing = Vec2.new(1, 0)
        state.players[TARGET].pos = Vec2.new(424, 270)
        state.players[TARGET].facing = Vec2.new(-1, 0)

        local idle = observe(state, combat_state)
        t.is_true(not combat_feasibility.family_commit(idle, TARGET, "guard", nil).feasible)

        local hostile = combat_state.players[TARGET]
        hostile.family_id = "light_melee"
        hostile.loadout_id = "loadout_vector_blade"
        hostile.phase = "windup"
        hostile.phase_ticks = 10
        hostile.source_sequence = 1
        local telegraphed = observe(state, combat_state)
        local witness = combat_feasibility.family_commit(telegraphed, TARGET, "guard", nil)
        t.is_true(witness.feasible)
        t.eq(witness.family_id, "guard")
        t.eq(witness.target_player, TARGET)
        t.is_true(witness.contact_tick >= 6, "guard cannot intersect before it is raised")
    end)

    t.it("ignores an aimed ranged row until its release latch is public", function()
        local state, combat_state = bare_match()
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[SOURCE].facing = Vec2.new(1, 0)
        state.players[TARGET].pos = Vec2.new(470, 270)
        state.players[TARGET].facing = Vec2.new(-1, 0)
        local hostile = combat_state.players[TARGET]
        hostile.family_id = "ranged"
        hostile.loadout_id = "loadout_pulse_blaster"
        hostile.phase = "windup"
        hostile.phase_ticks = 8
        hostile.source_sequence = 3

        local unlatched = observe(state, combat_state)
        t.eq(#combat_feasibility.hostile_paths(unlatched, TARGET), 0)
        t.is_true(not combat_feasibility.family_commit(unlatched, TARGET, "guard", nil).feasible)

        hostile.release_latched = true
        local latched = observe(state, combat_state)
        t.eq(#combat_feasibility.hostile_paths(latched, TARGET), 1)
        t.is_true(combat_feasibility.family_commit(latched, TARGET, "guard", nil).feasible)
    end)

    t.it("guards an already in-flight hostile projectile inside its horizon", function()
        local state, combat_state = bare_match()
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[SOURCE].facing = Vec2.new(1, 0)
        state.players[TARGET].pos = Vec2.new(600, 270)
        combat_state.projectiles[1] = {
            family_id = "ranged",
            source_index = TARGET,
            source_sequence = 9,
            pos = Vec2.new(480, 270),
            dir = Vec2.new(-1, 0),
            remaining_ticks = 40,
        }
        local observation = observe(state, combat_state)
        t.eq(#observation.projectiles, 1)
        t.is_true(observation.projectiles[1].horizon_ticks > 0)
        local witness = combat_feasibility.family_commit(observation, TARGET, "guard", nil)
        t.is_true(witness.feasible)

        local ticks, source, threat_x, threat_y =
            combat_feasibility.incoming_threat(observation, 30)
        t.is_true(ticks ~= nil)
        t.eq(source, TARGET)
        -- The threat position is the PROJECTILE's, not the shooter's. The
        -- shooter stands at x=600, well behind the body it is about to hit, so a
        -- caller that stepped away from the shooter would step into the shot.
        t.eq(threat_x, 480)
        t.eq(threat_y, 270)
        t.is_true(threat_x ~= state.players[TARGET].pos.x)
    end)
end)

t.describe("combat purpose predicates", function()
    t.it("names the opposing carrier a carrier contest", function()
        local state, combat_state = bare_match()
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[TARGET].pos = Vec2.new(424, 270)
        state.owner = TARGET
        state.ball = state.players[TARGET].pos
        local bitset, count =
            combat_feasibility.purpose_bitset(observe(state, combat_state), TARGET)
        t.is_true(bitset.carrier_contest)
        t.is_true(count >= 1)
        t.eq(combat_feasibility.dominant_purpose(bitset), "carrier_contest")
    end)

    t.it("names the nearest opponent to our own carrier a carrier protection", function()
        local state, combat_state = bare_match()
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[3].pos = Vec2.new(410, 270)
        state.owner = 3
        state.ball = state.players[3].pos
        state.players[TARGET].pos = Vec2.new(440, 270)
        state.players[OTHER_OPPONENT].pos = Vec2.new(700, 270)
        local bitset = combat_feasibility.purpose_bitset(observe(state, combat_state), TARGET)
        t.is_true(bitset.carrier_protection)
        -- The far opponent is neither near the owner nor the nearest.
        local other =
            combat_feasibility.purpose_bitset(observe(state, combat_state), OTHER_OPPONENT)
        t.is_true(not other.carrier_protection)
    end)

    t.it("names a shared loose ball a loose-ball contest", function()
        local state, combat_state = bare_match()
        state.owner = nil
        state.ball = Vec2.new(400, 270)
        state.players[SOURCE].pos = Vec2.new(360, 270)
        state.players[TARGET].pos = Vec2.new(440, 270)
        local bitset = combat_feasibility.purpose_bitset(observe(state, combat_state), TARGET)
        t.is_true(bitset.loose_ball_contest)

        -- Move the source out of the 96 px window: the pair stops being a
        -- contest even though the target has not moved.
        state.players[SOURCE].pos = Vec2.new(100, 270)
        local far = combat_feasibility.purpose_bitset(observe(state, combat_state), TARGET)
        t.is_true(not far.loose_ball_contest)
    end)

    t.it("names the sole blocker of one of our passing lanes a lane denial", function()
        local state, combat_state = bare_match()
        state.owner = SOURCE
        state.players[SOURCE].pos = Vec2.new(300, 270)
        state.ball = state.players[SOURCE].pos
        state.players[3].pos = Vec2.new(600, 270)
        state.players[TARGET].pos = Vec2.new(450, 270)
        state.players[OTHER_OPPONENT].pos = Vec2.new(120, 60)
        local bitset = combat_feasibility.purpose_bitset(observe(state, combat_state), TARGET)
        t.is_true(bitset.passing_lane_or_shot_denial)

        -- A second body in the same lane means neither is the SOLE blocker.
        state.players[OTHER_OPPONENT].pos = Vec2.new(500, 270)
        local shared = combat_feasibility.purpose_bitset(observe(state, combat_state), TARGET)
        t.is_true(not shared.passing_lane_or_shot_denial)
    end)

    t.it("upgrades a ball-context target in recovery to a recovery punish", function()
        local state, combat_state = bare_match()
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[TARGET].pos = Vec2.new(424, 270)
        state.owner = TARGET
        state.ball = state.players[TARGET].pos
        local runtime = combat_state.players[TARGET]
        runtime.family_id = "unarmed"
        runtime.loadout_id = "loadout_spring_gloves"
        runtime.phase = "recovery"
        runtime.phase_ticks = 8
        runtime.source_sequence = 2
        local bitset, count =
            combat_feasibility.purpose_bitset(observe(state, combat_state), TARGET)
        t.is_true(bitset.recovery_punish)
        t.is_true(bitset.carrier_contest)
        t.eq(count, 2)
        t.eq(combat_feasibility.dominant_purpose(bitset), "recovery_punish")
    end)

    t.it("keeps recovery alone diagnostic rather than a purpose", function()
        local state, combat_state = bare_match()
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[TARGET].pos = Vec2.new(424, 270)
        state.owner = nil
        state.ball = Vec2.new(900, 500)
        local runtime = combat_state.players[TARGET]
        runtime.family_id = "unarmed"
        runtime.loadout_id = "loadout_spring_gloves"
        runtime.phase = "recovery"
        runtime.phase_ticks = 8
        runtime.source_sequence = 2
        local bitset, count =
            combat_feasibility.purpose_bitset(observe(state, combat_state), TARGET)
        t.eq(count, 0)
        t.is_true(not bitset.recovery_punish)
        t.is_true(combat_feasibility.dominant_purpose(bitset) == nil)
    end)

    t.it("never names a keeper or a teammate", function()
        local state, combat_state = bare_match()
        state.owner = nil
        state.ball = Vec2.new(400, 270)
        state.players[SOURCE].pos = Vec2.new(390, 270)
        state.players[6].pos = Vec2.new(410, 270)
        state.players[3].pos = Vec2.new(412, 270)
        local observation = observe(state, combat_state)
        t.eq(select(2, combat_feasibility.purpose_bitset(observation, 6)), 0)
        t.eq(select(2, combat_feasibility.purpose_bitset(observation, 3)), 0)
    end)
end)

t.describe("intervention_candidate/v2", function()
    t.it("admits a reachable purpose pair and records its family bitset", function()
        local state, combat_state = bare_match()
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[SOURCE].facing = Vec2.new(1, 0)
        state.players[TARGET].pos = Vec2.new(424, 270)
        state.owner = TARGET
        state.ball = state.players[TARGET].pos
        local envelope = combat_feasibility.intervention_candidates(
            observe(state, combat_state),
            { search_ticks = 0 }
        )
        t.eq(#envelope, 1)
        t.eq(envelope[1].target_player, TARGET)
        t.eq(envelope[1].purpose, "carrier_contest")
        t.is_true(envelope[1].family_bitset.unarmed)
        t.is_true(envelope[1].family_bitset.light_melee)
    end)

    t.it("keeps a true purpose that no searched pose can reach out of the envelope", function()
        local state, combat_state = bare_match()
        -- A carrier on the far touchline: the purpose predicate is true, but no
        -- movement inside the search window brings any family into contact.
        -- Section 4.6 calls this `context_only_remote`: a diagnostic, never an
        -- opportunity.
        state.players[SOURCE].pos = Vec2.new(60, 60)
        state.players[TARGET].pos = Vec2.new(900, 500)
        state.owner = TARGET
        state.ball = state.players[TARGET].pos
        local observation = observe(state, combat_state)
        t.is_true(combat_feasibility.purpose_bitset(observation, TARGET).carrier_contest)
        t.eq(#combat_feasibility.intervention_candidates(observation, { search_ticks = 4 }), 0)
    end)

    t.it("returns pairs in a stable target-then-purpose order", function()
        local state, combat_state = bare_match()
        state.owner = nil
        state.ball = Vec2.new(400, 270)
        state.players[SOURCE].pos = Vec2.new(390, 270)
        state.players[SOURCE].facing = Vec2.new(1, 0)
        state.players[TARGET].pos = Vec2.new(414, 272)
        state.players[OTHER_OPPONENT].pos = Vec2.new(412, 268)
        local envelope = combat_feasibility.intervention_candidates(
            observe(state, combat_state),
            { search_ticks = 0 }
        )
        t.is_true(#envelope >= 2)
        local previous = 0
        for _, pair in ipairs(envelope) do
            t.is_true(pair.target_player >= previous)
            previous = pair.target_player
        end
    end)
end)

t.describe("formation_risk_tradeoff", function()
    t.it("flags a source far from its authored anchor and clears one at home", function()
        local state, combat_state = bare_match()
        local player = state.players[SOURCE]
        player.pos = Vec2.new(player.anchor.x, player.anchor.y)
        t.is_true(not combat_feasibility.formation_risk(observe(state, combat_state), "unarmed"))

        player.pos =
            Vec2.new(math.min(940, player.anchor.x + 200), math.min(520, player.anchor.y + 150))
        t.is_true(combat_feasibility.formation_risk(observe(state, combat_state), "unarmed"))
    end)
end)
