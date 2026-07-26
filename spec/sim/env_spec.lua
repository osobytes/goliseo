local t = require("spec.support.runner")
local Vec2 = require("core.vec2")
local env = require("sim.env")
local env_action = require("sim.env_action")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local replay = require("sim.replay")
local tuning = require("sim.tuning")
local players = require("data.players")
local tactics = require("data.tactics")
local teams = require("data.teams")

---@return table<string, PlayerData>
local function players_by_id()
    local by_id = {}
    for _, player in ipairs(players) do
        by_id[player.id] = player
    end
    return by_id
end

---@param overrides table<string, any>?
---@return EnvInstance
local function reset(overrides)
    return assert(env.reset(assert(env.reference_config("soccer_only", overrides))))
end

---@param instance EnvInstance
---@param action EnvSlotAction?
---@return table<integer, EnvSlotAction>
local function actions_for(instance, action)
    local out = {}
    for _, slot in ipairs(instance.controlled_slots) do
        out[slot] = action or env_action.neutral()
    end
    return out
end

-- The fixture sim.env builds from a config, spelled out independently so a
-- silent change to that construction breaks the equivalence test.
---@param seed integer
---@param duration number
---@return MatchState
local function direct_match(seed, duration)
    local by_id = players_by_id()
    return match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        home_formation = nil,
        tactic = tactics.balanced,
        away_tactic = tactics.balanced,
        players_by_id = by_id,
        human_controlled = false,
        duration = duration,
        max_goals = 3,
        seed = seed,
        input_ownership = match.ownership_for_teams(teams.nebula, teams.orion, by_id),
    })
end

t.describe("env.reset", function()
    t.it("builds the configured fixture and pins the reset boundary", function()
        local instance = reset({ seed = 7, duration = 4 })
        t.eq(instance.tick, 0)
        t.eq(instance.episode_ticks, 0)
        t.eq(instance.terminated, false)
        t.eq(instance.truncated, false)
        t.eq(#instance.boundary_hashes, 1)
        t.eq(#instance.controlled_slots, 1)
        t.eq(instance.controlled_slots[1], 1)
        t.eq(instance.config.version, 1)
        t.eq(instance.identity.tick_rate, fixed_clock.TICK_RATE)
        t.eq(
            instance.boundary_hashes[1],
            match_snapshot.hash(match_snapshot.capture(direct_match(7, 4))),
            "the environment fixture is the plain sim fixture"
        )
    end)

    t.it("rejects a config that cannot be honored", function()
        local bad, err, code = env.reset({ build = "spec", seed = 1, home_team_id = "nope" })
        t.eq(bad, nil)
        t.eq(code, "unknown_content")
        t.is_true(tostring(err):find("nope", 1, true) ~= nil)
    end)

    t.it("requires recorded rows when a slot is tape-driven, and rejects stray rows", function()
        local config = assert(env.reference_config("soccer_only"))
        config.slot_sources[5] = { kind = "tape" }
        local missing, _, missing_code = env.reset(config)
        t.eq(missing, nil)
        t.eq(missing_code, "missing_tape")

        local stray = assert(env.reference_config("soccer_only"))
        local unused, _, unused_code = env.reset(stray, { assert(input_frame.neutral(0)) })
        t.eq(unused, nil)
        t.eq(unused_code, "malformed")
    end)
end)

t.describe("env.step", function()
    t.it("advances exactly one canonical tick by default", function()
        local instance = reset({ seed = 7, duration = 4 })
        local result = assert(env.step(instance, actions_for(instance)))
        t.eq(result.ticks_simulated, 1)
        t.eq(result.tick, 1)
        t.eq(instance.tick, 1)
        t.eq(#result.boundary_hashes, 1)
        t.eq(#result.action_wires, 1)
        t.eq(result.terminated, false)
        t.eq(result.truncated, false)
        t.eq(result.termination, nil)
        t.eq(result.truncation, nil)
        t.eq(result.observation.tick, 1)
        t.eq(result.diagnostics.role, "evaluation")
        t.eq(result.diagnostics.channel, "experience_proxy_metrics")
        t.eq(result.diagnostics.metrics, nil, "metrics are produced once the episode ends")
    end)

    t.it("advances a declared number of ticks, firing edges only on the first", function()
        local instance = reset({ seed = 7, duration = 4 })
        local result = assert(env.step(
            instance,
            actions_for(instance, {
                move = { x = 1, y = 0 },
                held = { sprint = true },
                edges = { dash = true },
            }),
            3
        ))
        t.eq(result.ticks_simulated, 3)
        t.eq(result.tick, 3)
        t.eq(#result.boundary_hashes, 3)
        for index, wire in ipairs(result.action_wires) do
            local frame = assert(input_frame.decode(wire))
            local sample = frame.slots[1]
            t.eq(frame.tick, index - 1)
            t.eq(input_frame.is_held(sample, "sprint"), true, "held intents persist")
            t.eq(
                input_frame.has_edge(sample, "dash"),
                index == 1,
                "a one-shot edge fires on the first tick only"
            )
        end
    end)

    t.it("materializes complete rows for every slot, including unowned ones", function()
        local config = assert(env.reference_config("soccer_only", { seed = 5, duration = 2 }))
        config.slot_sources[3] = { kind = "bot", seed = 99 }
        local instance = assert(env.reset(config))
        local result = assert(env.step(instance, actions_for(instance)))
        local frame = assert(input_frame.decode(result.action_wires[1]))
        t.eq(#frame.slots, input_frame.SLOT_COUNT)
        for index = 1, input_frame.SLOT_COUNT do
            t.is_true(frame.slots[index] ~= nil, "slot " .. index .. " has an effective row")
        end
        t.eq(#instance.frames, 1)
    end)

    t.it("refuses an illegal action with a reason and does not advance", function()
        local instance = reset({ seed = 7, duration = 4 })
        local refused, err, code =
            env.step(instance, actions_for(instance, { edges = { switch = true } }))
        t.eq(refused, nil)
        t.eq(code, "illegal_action")
        t.is_true(
            tostring(err):find("fixed-slot routing", 1, true) ~= nil,
            "the reason explains why player switching does not exist here"
        )
        t.eq(instance.tick, 0, "a refused action leaves the boundary untouched")

        local unknown, _, unknown_code =
            env.step(instance, { [1] = env_action.neutral(), [4] = env_action.neutral() })
        t.eq(unknown, nil)
        t.eq(unknown_code, "unknown_slot")

        local malformed, _, malformed_code =
            env.step(instance, actions_for(instance, { move = { x = 4, y = 4 } }))
        t.eq(malformed, nil)
        t.eq(malformed_code, "illegal_action")
    end)

    t.it("separates termination from truncation", function()
        local instance = reset({ seed = 7, duration = fixed_clock.TICK_SECONDS * 2.5 })
        local result
        for _ = 1, 5 do
            result = assert(env.step(instance, actions_for(instance)))
            if result.terminated or result.truncated then
                break
            end
        end
        local final = assert(result)
        t.eq(final.terminated, true)
        t.eq(final.truncated, false)
        t.eq(final.termination, "time_expired")
        t.eq(final.truncation, nil)
        t.is_true(final.diagnostics.metrics ~= nil, "the episode end carries #128 metrics")
        t.eq(
            assert(final.diagnostics.metrics).goals_total,
            final.observation.views[1].match.score_own
                + final.observation.views[1].match.score_opponent
        )

        local after, _, after_code = env.step(instance, actions_for(instance))
        t.eq(after, nil)
        t.eq(after_code, "episode_over")
    end)

    t.it("truncates on the episode tick budget without claiming the match ended", function()
        local instance = reset({ seed = 7, duration = 30, max_episode_ticks = 4 })
        local result
        for _ = 1, 6 do
            result = assert(env.step(instance, actions_for(instance)))
            if result.terminated or result.truncated then
                break
            end
        end
        local final = assert(result)
        t.eq(final.truncated, true)
        t.eq(final.terminated, false)
        t.eq(final.truncation, "step_limit")
        t.eq(final.termination, nil)
        t.eq(instance.episode_ticks, 4)
        t.is_true(final.diagnostics.metrics ~= nil, "truncation still reports diagnostics")
    end)

    t.it("truncates on an incomplete tape instead of faulting", function()
        local config = assert(env.reference_config("soccer_only", { seed = 8, duration = 30 }))
        config.slot_sources[5] = { kind = "tape" }
        local frames = { assert(input_frame.neutral(0)), assert(input_frame.neutral(1)) }
        local instance = assert(env.reset(config, frames))
        t.eq(assert(env.step(instance, actions_for(instance))).truncated, false)
        t.eq(assert(env.step(instance, actions_for(instance))).truncated, false)
        local exhausted = assert(env.step(instance, actions_for(instance)))
        t.eq(exhausted.truncated, true)
        t.eq(exhausted.truncation, "tape_exhausted")
        t.eq(exhausted.terminated, false)
        t.eq(exhausted.ticks_simulated, 0)
        t.eq(instance.episode_ticks, 2)
    end)

    t.it("reports a stoppage distinctly from termination", function()
        local instance = reset({ seed = 7, duration = 4 })
        local result = assert(env.step(instance, actions_for(instance)))
        t.eq(result.stoppage, true, "the fixture opens in the post-kickoff hold")
        t.eq(result.stoppage_reason, "kickoff_hold")
        t.eq(result.terminated, false)
        t.eq(result.observation.views[1].match.phase, "kickoff")
    end)

    t.it("surfaces a simulation fault as a reproducible diagnostic", function()
        local instance = reset({ seed = 7, duration = 4 })
        -- Corrupt the routing the simulation asserts on. A policy must never be
        -- able to crash the trainer silently: the fault is returned with the
        -- boundary hash and the exact input row that triggered it.
        instance._state.slot_players[3] = nil
        local faulted, err, code = env.step(instance, actions_for(instance))
        t.eq(faulted, nil)
        t.eq(code, "sim_fault")
        t.is_true(tostring(err):find("simulation fault at tick 0", 1, true) ~= nil)
        t.is_true(tostring(err):find(instance.boundary_hashes[1], 1, true) ~= nil)
        local again, _, again_code = env.step(instance, actions_for(instance))
        t.eq(again, nil)
        t.eq(again_code, "faulted")
    end)
end)

t.describe("env determinism and hash equivalence", function()
    t.it("reproduces boundary hashes for the same config and action tape", function()
        local script = {
            { move = { x = 1, y = 0 }, held = { sprint = true } },
            { move = { x = 0, y = 1 }, edges = { dash = true } },
            { move = { x = -1, y = 0 } },
            { move = { x = 0, y = 0 }, held = { jockey = true } },
            { move = { x = 0.5, y = 0.5 }, edges = { pass = true } },
        }
        ---@return EnvInstance
        local function run()
            local instance = reset({ seed = 31, duration = 4 })
            for _, action in ipairs(script) do
                assert(env.step(instance, actions_for(instance, action)))
            end
            return instance
        end
        local first, second = run(), run()
        t.eq(#first.boundary_hashes, #script + 1)
        for index, hash in ipairs(first.boundary_hashes) do
            t.eq(second.boundary_hashes[index], hash, "boundary " .. index .. " must reproduce")
        end

        local other = reset({ seed = 32, duration = 4 })
        for _, action in ipairs(script) do
            assert(env.step(other, actions_for(other, action)))
        end
        t.is_true(
            other.boundary_hashes[#other.boundary_hashes]
                ~= first.boundary_hashes[#first.boundary_hashes],
            "a different seed must diverge"
        )
    end)

    t.it("agrees with the direct sim and with replay on every boundary hash", function()
        local instance = reset({ seed = 17, duration = 4 })
        for index = 1, 6 do
            assert(env.step(
                instance,
                actions_for(instance, {
                    move = { x = index % 2 == 0 and 1 or -1, y = 0 },
                    held = { sprint = index % 3 == 0 },
                    edges = { dodge = index == 2 },
                })
            ))
        end

        -- Direct sim: the same fixture stepped with the effective rows the
        -- environment materialized, through sim.match alone.
        local state = direct_match(17, 4)
        local direct = { match_snapshot.hash(match_snapshot.capture(state)) }
        for index, frame in ipairs(instance.frames) do
            match.step(state, fixed_clock.TICK_SECONDS, frame)
            direct[index + 1] = match_snapshot.hash(match_snapshot.capture(state))
        end
        t.eq(#direct, #instance.boundary_hashes)
        for index, hash in ipairs(instance.boundary_hashes) do
            t.eq(direct[index], hash, "direct sim boundary " .. index .. " must match")
        end

        -- Replay: the exported tape carries the environment's own hashes, and
        -- sim.replay re-derives them from the snapshot and the rows.
        local tape = assert(env.tape(instance))
        local replayed = assert(replay.run(tape, instance.identity))
        t.eq(replayed.divergence, nil)
        t.eq(#replayed.boundaries, #instance.boundary_hashes)
        for index, row in ipairs(replayed.boundaries) do
            t.eq(row.hash, instance.boundary_hashes[index], "replay boundary " .. index)
        end
    end)
end)

t.describe("env observation profiles and masks", function()
    t.it("serves per-slot views for a multi-slot team fixture", function()
        local config = assert(env.reference_config("soccer_only", { seed = 12, duration = 4 }))
        config.observation_profile = "team"
        config.slot_sources[5] = { kind = "policy", policy_id = "away-probe" }
        local instance = assert(env.reset(config))
        t.eq(#instance.controlled_slots, 2)
        local observation = env.observe(instance)
        t.eq(observation.profile, "team")
        t.eq(#observation.slots, 2)
        local home_view = assert(observation.views[1])
        local away_view = assert(observation.views[5])
        t.eq(home_view.team, "home")
        t.eq(away_view.team, "away")
        t.eq(home_view.own.side, "own")
        t.eq(#home_view.teammates, 4)
        t.eq(#home_view.opponents, 5)
        t.eq(
            home_view.match.score_own,
            away_view.match.score_opponent,
            "the two sides see mirrored scores"
        )
        t.is_true(
            home_view.geometry.target_goal.x ~= away_view.geometry.target_goal.x,
            "each slot attacks its own target goal"
        )
    end)

    t.it("publishes a client-knowable mask per controlled slot", function()
        local instance = reset({ seed = 12, duration = 4 })
        local masks = env.action_masks(instance)
        local mask = assert(masks[1])
        t.eq(mask.privileged, false)
        t.eq(mask.profile, "representative")
        t.eq(mask.edges.switch, false)
        t.eq(mask.edges.shoot, true)
        t.eq(mask.held.equipment, false, "the soccer-only fixture has no combat loadout")
        t.eq(mask.held.aerial_strike, false, "no airborne ball, no aerial intent")
    end)

    t.it("tags a privileged mask as privileged", function()
        local config = assert(env.reference_config("soccer_only", { seed = 12 }))
        config.observation_profile = "privileged"
        local instance = assert(env.reset(config))
        t.eq(assert(env.action_masks(instance)[1]).privileged, true)
    end)
end)

t.describe("env reference fixtures", function()
    t.it("covers a soccer-only environment with no combat state", function()
        local instance = reset({ seed = 4, duration = 2 })
        t.eq(instance.config.combat, false)
        t.eq(instance._combat, nil)
        t.eq(instance.identity.combat, nil)
        t.eq(instance.identity.tape_version, 1)
        t.eq(assert(env.observe(instance).views[1]).own.equipment, nil)
    end)

    t.it("covers all four combat families in one environment", function()
        local config = assert(env.reference_config("combat_all_families", {
            seed = 91,
            duration = 4,
        }))
        config.observation_profile = "team"
        local sources = {}
        for index = 1, input_frame.SLOT_COUNT do
            sources[index] = { kind = "policy", policy_id = "family-probe-" .. index }
        end
        config.slot_sources = sources
        local instance = assert(env.reset(config))
        -- Equipment cannot be committed during the post-restart hold, which is a
        -- rule, not an environment limitation: clear it the way replay fixtures do.
        instance._state.kickoff_hold = 0
        t.is_true(instance._combat ~= nil, "the combat companion state exists")
        t.is_true(instance.identity.combat ~= nil, "the tape identity pins combat mechanics")
        t.eq(instance.identity.tape_version, 2)

        local observation = env.observe(instance)
        local families = {}
        for _, slot in ipairs(instance.controlled_slots) do
            local equipment = assert(assert(observation.views[slot]).own.equipment)
            families[equipment.family_id] = true
            t.eq(equipment.phase, "ready")
        end
        for _, family in ipairs({ "unarmed", "guard", "light_melee", "ranged" }) do
            t.is_true(families[family], "the reference fixture exposes the " .. family .. " family")
        end

        local result = assert(env.step(
            instance,
            actions_for(instance, {
                held = { equipment = true },
                edges = { equipment_pressed = true },
            })
        ))
        local acting = {}
        for _, slot in ipairs(instance.controlled_slots) do
            local equipment = assert(assert(result.observation.views[slot]).own.equipment)
            t.is_true(
                equipment.phase ~= "ready",
                "the " .. equipment.family_id .. " family left the ready phase"
            )
            acting[equipment.family_id] = equipment.phase
        end
        t.eq(#instance.controlled_slots, input_frame.SLOT_COUNT)
        for _, family in ipairs({ "unarmed", "guard", "light_melee", "ranged" }) do
            t.is_true(acting[family] ~= nil, family .. " committed through the action contract")
        end
    end)

    t.it("reports goal-cap termination separately from time expiry", function()
        local instance = reset({ seed = 3, duration = 30, max_goals = 1 })
        local state = instance._state
        -- Roll a loose ball across the away goal line: a real goal through the
        -- rules, reached without touching the resolver.
        state.kickoff_hold = 0
        state.owner = nil
        state.pickup_cd = 1
        state.ball = Vec2.new(state.field.w - 2, state.field.h / 2)
        state.ball_vel = Vec2.new(1200, 0)
        state.ball_z = 0
        state.ball_vz = 0
        local result = assert(env.step(instance, actions_for(instance)))
        t.eq(result.terminated, true)
        t.eq(result.termination, "goal_cap")
        t.eq(result.truncated, false)
        t.is_true(result.observation.views[1].match.time_left > 0, "regulation time remained")
    end)
end)

t.describe("env.manifest", function()
    t.it("names every input a reproduction needs", function()
        local config = assert(env.reference_config("soccer_only", { seed = 66, duration = 4 }))
        config.slot_sources[1] = { kind = "policy", policy_id = "scripted-v3" }
        config.slot_sources[6] = { kind = "bot", seed = 4242 }
        config.shaping_channels = { "possession_gain" }
        local instance = assert(env.reset(config))
        assert(env.step(instance, actions_for(instance)))
        local manifest = env.manifest(instance)
        t.eq(manifest.env_version, env.VERSION)
        t.eq(manifest.seed, 66)
        t.eq(manifest.tick_rate, fixed_clock.TICK_RATE)
        t.eq(manifest.observation_profile, "representative")
        t.eq(manifest.policy_ids[1], "scripted-v3")
        t.eq(manifest.slot_sources[6].seed, 4242)
        t.eq(manifest.diagnostic_channel, "experience_proxy_metrics")
        t.eq(manifest.objective_channels[1], "match_outcome")
        t.eq(manifest.shaping_channels[1], "possession_gain")
        t.eq(manifest.initial_boundary_hash, instance.boundary_hashes[1])
        t.eq(manifest.boundary_hash, instance.boundary_hashes[2])
        t.eq(manifest.episode_ticks, 1)
        t.is_true(manifest.config:find("seed") == nil, "the seed has its own manifest field")
        t.is_true(manifest.config:find("slots=1:policy#scripted-v3", 1, true) ~= nil)
        t.eq(manifest.tuning, tuning.serialize(), "active tuning is pinned")
    end)
end)
