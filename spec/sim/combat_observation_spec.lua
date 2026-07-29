-- `combat_sim_observation/v1` schema, allowlist, canonical order, and digest.
--
-- The allowlist is the point of the schema: a policy that could read one extra
-- field would no longer be constrained by the contract at all. Every rejection
-- below is paired with the accepting case, so a check that silently stopped
-- checking fails instead of passing quietly.

local Vec2 = require("core.vec2")
local combat = require("sim.combat")
local combat_observation = require("sim.combat_observation")
local combat_policy = require("sim.combat_policy")
local match = require("sim.match")
local teams = require("data.teams")
local t = require("spec.support.runner")

---@param seed integer?
---@return MatchState
---@return CombatMatchState
local function new_match(seed)
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        seed = seed or 19,
    })
    state.kickoff_hold = 0
    return state, combat.new_state(state)
end

---@param state MatchState
---@param combat_state CombatMatchState
---@param index integer?
---@return CombatObservation
local function build(state, combat_state, index)
    return combat_observation.build(state, combat_state, index or 2, combat_policy.POLICY_ID, nil)
end

t.describe("combat_sim_observation/v1", function()
    t.it("builds a valid observation whose digest covers its body", function()
        local state, combat_state = new_match()
        local observation = build(state, combat_state)
        t.eq(observation.schema, "combat_sim_observation/v1")
        t.eq(observation.version, 1)
        t.eq(observation.policy_id, combat_policy.POLICY_ID)
        t.is_true(assert(combat_observation.validate(observation)))
        t.eq(observation.digest, combat_observation.digest(observation))
        t.eq(#observation.digest, 16)
    end)

    t.it("orders teammate, opponent, and anchor rows by canonical player index", function()
        local state, combat_state = new_match()
        local observation = build(state, combat_state, 3)
        local previous = 0
        for _, row in ipairs(observation.teammates) do
            t.is_true(row.player_index > previous)
            t.eq(row.team, observation.self.team)
            previous = row.player_index
        end
        previous = 0
        for _, row in ipairs(observation.opponents) do
            t.is_true(row.player_index > previous)
            t.is_true(row.team ~= observation.self.team)
            previous = row.player_index
        end
        for index, row in ipairs(observation.anchors) do
            t.eq(row.player_index, index)
        end
        t.eq(#observation.player_order, #state.players)
        t.eq(#observation.teammates + #observation.opponents + 1, #state.players)
    end)

    t.it("rejects a reordered peer row array", function()
        local state, combat_state = new_match()
        local observation = build(state, combat_state)
        observation.opponents[1], observation.opponents[2] =
            observation.opponents[2], observation.opponents[1]
        local ok, err = combat_observation.validate(observation)
        t.is_true(ok == nil)
        t.is_true(tostring(err):find("canonical player order") ~= nil, err)
    end)

    t.it("rejects a row filed under the wrong team array", function()
        local state, combat_state = new_match()
        local observation = build(state, combat_state)
        -- Home is 1..5 and away is 6..10 and the observer is player 2, so moving
        -- opponent 6 onto the end of the teammate array keeps the canonical
        -- ascending player order intact. Only the team disagrees.
        local moved = table.remove(observation.opponents, 1)
        t.eq(moved.player_index, 6)
        observation.teammates[#observation.teammates + 1] = moved
        -- Recompute the digest so a stale hash cannot mask the real rejection:
        -- this has to fail on the team, not on the digest.
        observation.digest = combat_observation.digest(observation)
        local ok, err = combat_observation.validate(observation)
        t.is_true(ok == nil, "a team-swapped row was accepted")
        t.is_true(tostring(err):find("wrong team array") ~= nil, err)

        -- Every consumer reads array membership as team ground truth, so the
        -- mirror case has to be rejected too.
        local mirrored = build(state, combat_state)
        local mate = table.remove(mirrored.teammates, 1)
        table.insert(mirrored.opponents, 1, mate)
        mirrored.digest = combat_observation.digest(mirrored)
        t.is_true(combat_observation.validate(mirrored) == nil)
    end)

    t.it("rejects a missing declared field", function()
        local state, combat_state = new_match()
        local observation = build(state, combat_state)
        observation.self.facing_x = nil
        local ok, err = combat_observation.validate(observation)
        t.is_true(ok == nil)
        t.is_true(tostring(err):find("missing field facing_x") ~= nil, err)
    end)

    t.it("rejects an undeclared field anywhere in the schema", function()
        local state, combat_state = new_match()
        for _, mutate in ipairs({
            function(observation)
                observation.loadout_id = "loadout_spring_gloves"
            end,
            function(observation)
                observation.self.presentation_id = "medieval_bramble_quickstep"
            end,
            function(observation)
                observation.opponents[1].theme = "medieval"
            end,
            function(observation)
                observation.ball.rng = 12345
            end,
            function(observation)
                observation.match.viewport = 800
            end,
        }) do
            local observation = build(state, combat_state)
            mutate(observation)
            local ok, err = combat_observation.validate(observation)
            t.is_true(ok == nil, "an undeclared field was accepted")
            t.is_true(tostring(err):find("undeclared field") ~= nil, err)
        end
    end)

    t.it("rejects a duplicated peer row and a duplicated projectile row", function()
        local state, combat_state = new_match()
        -- A peer row cannot be duplicated without ALSO breaking something else:
        -- within one array it breaks the strict player-index ordering, and
        -- across arrays it breaks team membership. Both rejections are asserted
        -- here; `seen_index` stays as defence in depth behind them.
        local across = build(state, combat_state)
        across.teammates[#across.teammates + 1] = across.opponents[1]
        t.is_true(combat_observation.validate(across) == nil)
        local within = build(state, combat_state)
        within.teammates[#within.teammates + 1] = within.teammates[1]
        t.is_true(combat_observation.validate(within) == nil)

        combat_state.projectiles[1] = {
            family_id = "ranged",
            source_index = 7,
            source_sequence = 1,
            pos = Vec2.new(400, 270),
            dir = Vec2.new(-1, 0),
            remaining_ticks = 40,
        }
        local with_projectile = build(state, combat_state)
        t.eq(#with_projectile.projectiles, 1)
        t.is_true(assert(combat_observation.validate(with_projectile)))
        with_projectile.projectiles[2] = with_projectile.projectiles[1]
        with_projectile.projectile_order[2] = with_projectile.projectiles[1].projectile_id
        local ok, err = combat_observation.validate(with_projectile)
        t.is_true(ok == nil)
        t.is_true(tostring(err):find("duplicate projectile") ~= nil, err)
    end)

    t.it("orders projectiles by source sequence, source player, then id", function()
        local state, combat_state = new_match()
        combat_state.projectiles = {
            {
                family_id = "ranged",
                source_index = 8,
                source_sequence = 5,
                pos = Vec2.new(300, 200),
                dir = Vec2.new(-1, 0),
                remaining_ticks = 20,
            },
            {
                family_id = "ranged",
                source_index = 7,
                source_sequence = 2,
                pos = Vec2.new(320, 210),
                dir = Vec2.new(-1, 0),
                remaining_ticks = 30,
            },
        }
        local observation = build(state, combat_state)
        t.eq(#observation.projectiles, 2)
        t.eq(observation.projectiles[1].source_sequence, 2)
        t.eq(observation.projectiles[2].source_sequence, 5)
        for index, id in ipairs(observation.projectile_order) do
            t.eq(id, observation.projectiles[index].projectile_id)
        end
        t.is_true(assert(combat_observation.validate(observation)))

        observation.projectiles[1], observation.projectiles[2] =
            observation.projectiles[2], observation.projectiles[1]
        observation.projectile_order[1] = observation.projectiles[1].projectile_id
        observation.projectile_order[2] = observation.projectiles[2].projectile_id
        local ok, err = combat_observation.validate(observation)
        t.is_true(ok == nil)
        t.is_true(tostring(err):find("canonical order") ~= nil, err)
    end)

    t.it("gives every projectile a collision-free id from its source and sequence", function()
        t.eq(combat_observation.projectile_id("zyro_vex", 3), "8:zyro_vex/3")
        -- Without the length prefix, `("a", 11)` and `("a/1", 1)` would collide.
        t.is_true(
            combat_observation.projectile_id("a", 11) ~= combat_observation.projectile_id("a/1", 1)
        )
    end)

    t.it("publishes the bounded public threat path of a committed melee row", function()
        local state, combat_state = new_match()
        local runtime = combat_state.players[7]
        runtime.family_id = "light_melee"
        runtime.loadout_id = "loadout_vector_blade"
        runtime.phase = "windup"
        runtime.phase_ticks = 5
        runtime.source_sequence = 4
        combat_state.tick = 40
        local observation = build(state, combat_state, 2)
        local row = assert(combat_observation.telegraph(combat_state, 7))
        t.eq(row.phase, "windup")
        for _, peer in ipairs(observation.opponents) do
            if peer.player_index == 7 then
                t.eq(peer.family_id, "light_melee")
                -- Twelve windup ticks, five of which are gone; the remaining
                -- five plus five active ticks end the public path at tick 50.
                t.eq(peer.telegraph_start_tick, 33)
                t.eq(peer.telegraph_end_tick, 50)
                t.eq(peer.projected_reach_px, 42)
                t.eq(peer.release_latched, false)
                t.eq(peer.projected_spawn_tick, 0)
            end
        end
    end)

    t.it("holds the same sentinel discipline on the self row", function()
        local state, combat_state = new_match()
        -- A player with no accepted action: every optional value is present as
        -- its sentinel rather than absent, which is what keeps the row total.
        local idle = build(state, combat_state, 2)
        t.eq(idle.self.source_sequence, 0)
        t.eq(idle.self.forced_state, "none")
        t.eq(idle.self.release_latched, false)
        t.eq(idle.self.projected_spawn_tick, 0)
        t.is_true(assert(combat_observation.validate(idle)))

        -- A ranged self row publishes its spawn tick only once its own release
        -- latch is set, exactly as a peer row does.
        local runtime = combat_state.players[2]
        runtime.family_id = "ranged"
        runtime.loadout_id = "loadout_pulse_blaster"
        runtime.phase = "windup"
        runtime.phase_ticks = 6
        runtime.source_sequence = 3
        combat_state.tick = 100
        local unlatched = build(state, combat_state, 2)
        t.eq(unlatched.self.release_latched, false)
        t.eq(unlatched.self.projected_spawn_tick, 0)

        runtime.release_latched = true
        local latched = build(state, combat_state, 2)
        t.eq(latched.self.release_latched, true)
        t.eq(latched.self.projected_spawn_tick, 106)

        -- A non-ranged family never claims a latch or a spawn tick.
        runtime.family_id = "light_melee"
        runtime.loadout_id = "loadout_vector_blade"
        local melee = build(state, combat_state, 2)
        t.eq(melee.self.release_latched, false)
        t.eq(melee.self.projected_spawn_tick, 0)
    end)

    t.it("publishes a ranged spawn tick only once the release latch is public", function()
        local state, combat_state = new_match()
        local runtime = combat_state.players[7]
        runtime.family_id = "ranged"
        runtime.loadout_id = "loadout_pulse_blaster"
        runtime.phase = "windup"
        runtime.phase_ticks = 6
        runtime.source_sequence = 2
        combat_state.tick = 100

        local unlatched = build(state, combat_state, 2)
        for _, peer in ipairs(unlatched.opponents) do
            if peer.player_index == 7 then
                t.eq(peer.release_latched, false)
                t.eq(peer.projected_spawn_tick, 0)
                t.eq(peer.telegraph_end_tick, 0)
            end
        end

        runtime.release_latched = true
        local latched = build(state, combat_state, 2)
        for _, peer in ipairs(latched.opponents) do
            if peer.player_index == 7 then
                t.eq(peer.release_latched, true)
                t.eq(peer.projected_spawn_tick, 106)
                t.eq(peer.telegraph_end_tick, 107)
            end
        end
    end)
end)
