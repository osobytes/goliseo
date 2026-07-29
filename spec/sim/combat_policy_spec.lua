-- `gameplay_ai/combat/v1` scoring, reason vocabulary, and availability.
--
-- Scoring is pure: it takes a candidate record, not a world. That is what lets
-- the soccer-value ordering be asserted directly instead of inferred from
-- whichever match happened to be simulated.

local Vec2 = require("core.vec2")
local brain = require("sim.brain")
local combat = require("sim.combat")
local combat_intent = require("sim.combat_intent")
local combat_observation = require("sim.combat_observation")
local combat_policy = require("sim.combat_policy")
local match = require("sim.match")
local teams = require("data.teams")
local t = require("spec.support.runner")

local SOURCE = 2
local TARGET = 7

-- `formation_risk_tradeoff` is a cost flag, never a purpose or a reason. Looking
-- it up through a variable keeps the absence assertions honest without asking
-- the type checker to accept a field that must not exist.
local COST_FLAG = "formation_risk_tradeoff"

---@param overrides table<string, any>?
---@return CombatPolicyCandidate
local function candidate(overrides)
    local base = {
        target_player = TARGET,
        purpose = "carrier_contest",
        family_id = "unarmed",
        target_ball_distance = 0,
        source_ball_distance = 30,
        target_is_carrier = true,
        team_owns_ball = false,
        spills_ball = true,
        teammate_coverage = 0,
        formation_risk = false,
        commitment_ticks = combat_policy.commitment_ticks("unarmed"),
        guard_threat_ticks = 0,
    }
    for key, value in pairs(overrides or {}) do
        base[key] = value
    end
    ---@cast base CombatPolicyCandidate
    return base
end

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
    for index, player in ipairs(state.players) do
        player.pos = Vec2.new(40 + index * 3, 40 + index * 3)
        player.vel = Vec2.new(0, 0)
        player.facing = Vec2.new(player.team == "home" and 1 or -1, 0)
    end
    state.ball = Vec2.new(900, 500)
    state.owner = nil
    return state, combat_state
end

---@param state MatchState
---@param combat_state CombatMatchState
---@return CombatObservation
local function observe(state, combat_state)
    return combat_observation.build(state, combat_state, SOURCE, combat_policy.POLICY_ID, nil)
end

t.describe("gameplay_ai/combat/v1 scoring", function()
    t.it("puts a carrier contest above every other purpose from the same geometry", function()
        local scores = {}
        for _, purpose in ipairs({
            "carrier_contest",
            "loose_ball_contest",
            "passing_lane_or_shot_denial",
            "carrier_protection",
        }) do
            scores[purpose] = combat_policy.score(candidate({ purpose = purpose }))
        end
        t.is_true(scores.carrier_contest > scores.loose_ball_contest)
        t.is_true(scores.loose_ball_contest > scores.passing_lane_or_shot_denial)
        t.is_true(scores.passing_lane_or_shot_denial > scores.carrier_protection)
        t.is_true(
            combat_policy.score(candidate({ purpose = "recovery_punish" })) > scores.carrier_contest
        )
    end)

    t.it("prefers the contest closer to the ball", function()
        local near = combat_policy.score(candidate({ target_ball_distance = 0 }))
        local far = combat_policy.score(candidate({ target_ball_distance = 400 }))
        t.is_true(near > far)
    end)

    t.it("discounts a target a teammate already covers", function()
        local uncovered = combat_policy.score(candidate({ teammate_coverage = 0 }))
        local covered = combat_policy.score(candidate({ teammate_coverage = 2 }))
        t.is_true(uncovered > covered)
    end)

    t.it("charges the whole commitment, so a cheap family beats an expensive one", function()
        local unarmed = combat_policy.score(candidate({
            family_id = "unarmed",
            commitment_ticks = combat_policy.commitment_ticks("unarmed"),
        }))
        local ranged = combat_policy.score(candidate({
            family_id = "ranged",
            commitment_ticks = combat_policy.commitment_ticks("ranged"),
        }))
        t.is_true(unarmed > ranged)
        t.eq(combat_policy.commitment_ticks("unarmed"), 6 + 4 + 12 + 24)
        t.eq(combat_policy.commitment_ticks("light_melee"), 12 + 5 + 21 + 42)
        t.eq(combat_policy.commitment_ticks("ranged"), 18 + 1 + 27 + 60)
        t.eq(combat_policy.commitment_ticks("guard"), 6 + 0 + 9 + 0)
    end)

    t.it("charges the formation-risk flag as a cost and never as a purpose", function()
        local safe = combat_policy.score(
            candidate({ purpose = "carrier_protection", formation_risk = false })
        )
        local risky = combat_policy.score(
            candidate({ purpose = "carrier_protection", formation_risk = true })
        )
        t.eq(safe - risky, combat_policy.FORMATION_RISK_PENALTY)
        t.is_true(combat_policy.PURPOSE_BASE[COST_FLAG] == nil)

        -- Chasing the ball IS leaving the anchor, so the flag is still raised
        -- and reported for an on-ball purpose but does not veto the contest.
        t.eq(
            combat_policy.score(candidate({ formation_risk = true })),
            combat_policy.score(candidate({ formation_risk = false }))
        )
    end)

    t.it("keeps a pure off-ball candidate below the decline baseline", function()
        -- Lane denial, away from the ball, with the source already pulled off
        -- its anchor: exactly the "purposeless harassment" shape the issue asks
        -- to make rare.
        local score = combat_policy.score(candidate({
            purpose = "passing_lane_or_shot_denial",
            target_ball_distance = 300,
            target_is_carrier = false,
            formation_risk = true,
        }))
        t.is_true(score < combat_policy.DECLINE_BASELINE)
        -- The same denial from a disciplined position, close to the ball, is a
        -- real option rather than a forbidden one.
        local disciplined = combat_policy.score(candidate({
            purpose = "passing_lane_or_shot_denial",
            target_ball_distance = 40,
            target_is_carrier = false,
            formation_risk = false,
        }))
        t.is_true(disciplined > combat_policy.DECLINE_BASELINE)
    end)

    t.it("raises a guard the closer its answered threat is", function()
        local imminent = combat_policy.score(candidate({
            family_id = "guard",
            guard_threat_ticks = 1,
            commitment_ticks = combat_policy.commitment_ticks("guard"),
        }))
        local distant = combat_policy.score(candidate({
            family_id = "guard",
            guard_threat_ticks = combat_policy.GUARD_URGENCY_TICKS,
            commitment_ticks = combat_policy.commitment_ticks("guard"),
        }))
        t.is_true(imminent > distant)
    end)

    t.it("loses to declining when the purpose is weak and far from the ball", function()
        local options = combat_policy.options({
            candidate({
                purpose = "carrier_protection",
                target_ball_distance = 500,
                target_is_carrier = false,
                teammate_coverage = 2,
                formation_risk = true,
            }),
        })
        local selected = brain.select_scored_option(options, 0, 1)
        t.eq(selected.kind, "decline")
    end)

    t.it("always offers a decline option and unique kind/id pairs", function()
        local options = combat_policy.options({
            candidate({ target_player = 7 }),
            candidate({ target_player = 8, purpose = "loose_ball_contest" }),
        })
        t.eq(#options, 3)
        local selected = brain.select_scored_option(options, 0, 1)
        t.eq(selected.kind, "commit")
        t.eq(selected.reference, 7)
    end)
end)

t.describe("gameplay_ai/combat/v1 decisions", function()
    t.it("reports the closed unavailability reason instead of guessing", function()
        local state, combat_state = bare_match()
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[TARGET].pos = Vec2.new(424, 270)
        state.owner = TARGET
        state.ball = state.players[TARGET].pos

        local runtime = combat_state.players[SOURCE]
        local original_family = runtime.family_id
        runtime.family_id = nil
        runtime.loadout_id = nil
        t.eq(
            combat_policy.decide(observe(state, combat_state), 1, 0).unavailable_reason,
            "no_loadout"
        )

        runtime.family_id = original_family
        runtime.loadout_id = "loadout_spring_gloves"
        runtime.cooldown_ticks = 12
        t.eq(
            combat_policy.decide(observe(state, combat_state), 1, 0).unavailable_reason,
            "cooldown"
        )

        runtime.cooldown_ticks = 0
        runtime.forced_ticks = 5
        t.eq(combat_policy.decide(observe(state, combat_state), 1, 0).unavailable_reason, "forced")

        runtime.forced_ticks = 0
        runtime.phase = "windup"
        runtime.phase_ticks = 3
        runtime.source_sequence = 1
        t.eq(
            combat_policy.decide(observe(state, combat_state), 1, 0).unavailable_reason,
            "already_committed"
        )

        runtime.phase = "ready"
        runtime.phase_ticks = 0
        runtime.source_sequence = nil
        state.players[SOURCE].slide_timer = 0.4
        t.eq(
            combat_policy.decide(observe(state, combat_state), 1, 0).unavailable_reason,
            "soccer_commitment"
        )
    end)

    t.it("reports no reachable opportunity as a feasibility unavailability", function()
        local state, combat_state = bare_match()
        combat_state.players[SOURCE].family_id = "unarmed"
        combat_state.players[SOURCE].loadout_id = "loadout_spring_gloves"
        state.players[SOURCE].pos = Vec2.new(60, 60)
        state.players[TARGET].pos = Vec2.new(900, 500)
        state.owner = TARGET
        state.ball = state.players[TARGET].pos
        local decision = combat_policy.decide(observe(state, combat_state), 1, 0)
        t.eq(decision.action, "unavailable")
        t.eq(decision.unavailable_reason, "family_commit_feasibility")
        t.eq(decision.reason, "none")
    end)

    t.it("commits with exactly one purpose reason and no context violation", function()
        local state, combat_state = bare_match()
        combat_state.players[SOURCE].family_id = "unarmed"
        combat_state.players[SOURCE].loadout_id = "loadout_spring_gloves"
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[SOURCE].facing = Vec2.new(1, 0)
        state.players[TARGET].pos = Vec2.new(424, 270)
        state.owner = TARGET
        state.ball = state.players[TARGET].pos
        local decision = combat_policy.decide(observe(state, combat_state), 1, 0)
        t.eq(decision.action, "commit")
        t.eq(decision.reason, "carrier_contest")
        t.eq(decision.target_player, TARGET)
        t.eq(decision.family_id, "unarmed")
        t.is_true(not decision.context_violation)
        t.is_true(combat_policy.is_commit_reason(decision.reason))
        t.eq(#decision.digest, 16)
    end)

    t.it("is byte-identical for the same observation and seed", function()
        local state, combat_state = bare_match()
        combat_state.players[SOURCE].family_id = "unarmed"
        combat_state.players[SOURCE].loadout_id = "loadout_spring_gloves"
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[SOURCE].facing = Vec2.new(1, 0)
        state.players[TARGET].pos = Vec2.new(424, 270)
        state.owner = TARGET
        state.ball = state.players[TARGET].pos
        local observation = observe(state, combat_state)
        local first = combat_policy.decide(observation, 20003, 3)
        local second = combat_policy.decide(observation, 20003, 3)
        t.eq(second.option_id, first.option_id)
        t.eq(second.reason, first.reason)
        t.eq(second.target_player, first.target_player)
        t.eq(second.rng_state, first.rng_state)
    end)

    t.it("spends no RNG at zero temperature", function()
        local state, combat_state = bare_match()
        combat_state.players[SOURCE].family_id = "unarmed"
        combat_state.players[SOURCE].loadout_id = "loadout_spring_gloves"
        state.players[SOURCE].pos = Vec2.new(400, 270)
        state.players[SOURCE].facing = Vec2.new(1, 0)
        state.players[TARGET].pos = Vec2.new(424, 270)
        state.owner = TARGET
        state.ball = state.players[TARGET].pos
        local decision = combat_policy.decide(observe(state, combat_state), 20003, 0)
        t.eq(decision.rng_state, 20003)
    end)

    t.it("rejects any observation that is not this schema", function()
        local foreign = { schema = "human_proxy_observation/v1", version = 1 }
        ---@cast foreign any
        t.is_true(not pcall(combat_policy.decide, foreign, 1, 0))
    end)
end)

t.describe("combat decision reasons", function()
    t.it("keeps decline out of the commit vocabulary and unattributed inside it", function()
        t.is_true(combat_intent.REASONS.decline)
        t.is_true(not combat_intent.COMMIT_REASONS.decline)
        t.is_true(combat_intent.COMMIT_REASONS.unattributed_off_ball)
        for _, purpose in ipairs({
            "carrier_contest",
            "carrier_protection",
            "loose_ball_contest",
            "passing_lane_or_shot_denial",
            "recovery_punish",
        }) do
            t.is_true(combat_intent.COMMIT_REASONS[purpose], purpose .. " is not a commit reason")
        end
        t.is_true(combat_intent.REASONS[COST_FLAG] == nil)
    end)

    t.it("refuses to label a commit with decline", function()
        local ok = pcall(combat_intent.commit, combat_intent.new_state(), "decline", 7, 0)
        t.is_true(not ok)
    end)

    t.it("materializes only legal equipment transitions", function()
        ---@param signals CombatIntentSignals
        local function legal(signals)
            local pressed = signals.equipment_pressed
            local released = signals.equipment_released
            local held = signals.equipment_held
            return not (pressed and not released and not held) and not (released and held)
        end

        t.is_true(legal(combat_intent.commit_signals()))
        local state = combat_intent.commit(combat_intent.new_state(), "carrier_contest", 7, 0)
        local signals, after = combat_intent.materialize(state)
        t.is_true(legal(assert(signals)))
        t.eq(assert(signals).equipment_released, true)
        t.eq(after.stage, "idle")

        local guard = combat_intent.commit(combat_intent.new_state(), "carrier_contest", 7, 3)
        t.eq(guard.stage, "hold")
        local current = guard
        for _ = 1, 3 do
            local held_signals
            held_signals, current = combat_intent.materialize(current)
            t.is_true(legal(assert(held_signals)))
            t.eq(assert(held_signals).equipment_held, true)
        end
        local release_signals
        release_signals, current = combat_intent.materialize(current)
        t.eq(assert(release_signals).equipment_released, true)
        t.eq(assert(release_signals).equipment_held, false)
        t.eq(current.stage, "idle")
        t.eq(select(1, combat_intent.materialize(current)), nil)
    end)

    t.it("derives a stat-scaled cadence and a per-tick decision seed", function()
        local slow = combat_intent.decision_period(0)
        local fast = combat_intent.decision_period(1)
        t.is_true(slow > fast)
        t.eq(slow, 27)
        t.eq(fast, 9)
        t.is_true(combat_intent.should_decide(0, slow, 0))
        t.is_true(not combat_intent.should_decide(1, slow, 0))
        -- Two players with the same scan rate do not decide on the same tick.
        t.is_true(combat_intent.should_decide(0, 9, 1))
        t.is_true(not combat_intent.should_decide(0, 10, 1))
        local seed = combat_intent.decision_seed(41, 3)
        t.eq(combat_intent.decision_seed(41, 3), seed)
        t.is_true(combat_intent.decision_seed(41, 4) ~= seed)
        t.is_true(seed >= 1 and seed == math.floor(seed))
    end)
end)
