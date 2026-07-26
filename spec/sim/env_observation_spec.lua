local t = require("spec.support.runner")
local env = require("sim.env")
local env_observation = require("sim.env_observation")
local input_frame = require("sim.input_frame")

---@param overrides table<string, any>?
---@return EnvInstance
local function reset(overrides)
    return assert(env.reset(assert(env.reference_config("soccer_only", overrides))))
end

-- Read a field the contract deliberately does not declare. Absence is the
-- assertion, so the lookup has to go through plain data.
---@param record table
---@param name string
---@return any
local function field(record, name)
    return record[name]
end

---@type any
local UNKNOWN_PROFILE = "oracle"

t.describe("env_observation profiles", function()
    t.it("tags observability and human-proxy validity per profile", function()
        t.eq(env_observation.PROFILES.representative.player_observable, true)
        t.eq(env_observation.PROFILES.representative.human_proxy_valid, true)
        t.eq(env_observation.PROFILES.representative.multi_slot, false)
        t.eq(env_observation.PROFILES.team.player_observable, true)
        t.eq(env_observation.PROFILES.team.multi_slot, true)
        t.eq(env_observation.PROFILES.privileged.player_observable, false)
        t.eq(env_observation.PROFILES.privileged.human_proxy_valid, false)
        for id, profile in pairs(env_observation.PROFILES) do
            t.eq(profile.id, id)
            t.is_true(profile.description ~= "")
        end
    end)

    t.it("rejects unknown profiles and slot/profile mismatches", function()
        local instance = reset({ seed = 2, duration = 2 })
        local unknown, _, unknown_code =
            env_observation.build(instance._state, nil, { 1 }, UNKNOWN_PROFILE)
        t.eq(unknown, nil)
        t.eq(unknown_code, "unknown_profile")

        local mismatch, _, mismatch_code =
            env_observation.build(instance._state, nil, { 1, 5 }, "representative")
        t.eq(mismatch, nil)
        t.eq(mismatch_code, "profile_mismatch")

        local empty, _, empty_code = env_observation.build(instance._state, nil, {}, "team")
        t.eq(empty, nil)
        t.eq(empty_code, "malformed")

        local unsorted, _, unsorted_code =
            env_observation.build(instance._state, nil, { 5, 1 }, "team")
        t.eq(unsorted, nil)
        t.eq(unsorted_code, "malformed")

        local off_slot, _, off_slot_code =
            env_observation.build(instance._state, nil, { 9 }, "representative")
        t.eq(off_slot, nil)
        t.eq(off_slot_code, "malformed")
    end)
end)

t.describe("env_observation.view", function()
    t.it("describes the fixture from the controlled slot's side", function()
        local instance = reset({ seed = 2, duration = 4 })
        local view = assert(env_observation.view(instance._state, nil, 1, "representative"))
        t.eq(view.version, env_observation.VERSION)
        t.eq(view.slot, 1)
        t.eq(view.slot_id, "home_1")
        t.eq(view.team, "home")
        t.eq(view.own.slot, 1)
        t.eq(view.own.side, "own")
        t.eq(view.own.is_keeper, false, "keepers never own an input slot")
        t.eq(#view.teammates, 4, "four teammates including the AI keeper")
        t.eq(#view.opponents, 5)
        t.eq(view.match.tick, 0)
        t.eq(view.match.tick_rate, 60)
        t.eq(view.match.phase, "kickoff")
        t.eq(view.match.finished, false)
        t.eq(view.geometry.field_w, 960)
        t.eq(view.geometry.own_goal.x, instance._state.goal_home.x)
        t.eq(view.geometry.target_goal.x, instance._state.goal_away.x)
        t.eq(#view.events, 0)
        t.eq(view.privileged, nil)

        local keepers = 0
        for _, player in ipairs(view.teammates) do
            t.eq(player.side, "own")
            if player.is_keeper then
                keepers = keepers + 1
                t.eq(player.slot, nil, "a keeper has no input slot to report")
            end
        end
        t.eq(keepers, 1)
        for _, player in ipairs(view.opponents) do
            t.eq(player.side, "opponent")
        end
    end)

    t.it("mirrors sides for an away slot", function()
        local instance = reset({ seed = 2, duration = 4 })
        local view = assert(env_observation.view(instance._state, nil, 5, "representative"))
        t.eq(view.team, "away")
        t.eq(view.slot_id, "away_1")
        t.eq(view.geometry.own_goal.x, instance._state.goal_away.x)
        t.eq(view.geometry.target_goal.x, instance._state.goal_home.x)
        t.eq(view.match.score_own, instance._state.score.away)
    end)

    t.it("reports the ball and its carrier as relative cues", function()
        local instance = reset({ seed = 2, duration = 4 })
        local state = instance._state
        local view = assert(env_observation.view(state, nil, 1, "representative"))
        t.eq(view.ball.x, state.ball.x)
        t.eq(view.ball.z, state.ball_z)
        t.eq(view.ball.airborne, false)
        t.eq(view.ball.loose, state.owner == nil)
        if state.owner then
            local carrier = state.players[state.owner]
            t.eq(view.ball.owner_side, carrier.team == "home" and "own" or "opponent")
            t.eq(view.ball.owner_slot, state.slot_for_player[state.owner])
        end
    end)

    t.it("shows only the readiness a player can see on themselves", function()
        local instance = reset({ seed = 2, duration = 4 })
        local state = instance._state
        local player = state.players[assert(state.slot_players[1])]
        player.dash_cd = 0.25
        player.dodge_cd = 0
        player.sprint_meter = 0.5
        local view = assert(env_observation.view(state, nil, 1, "representative"))
        t.eq(view.own.dash_cooldown_s, 0.25)
        t.eq(view.own.dash_ready, false)
        t.eq(view.own.dodge_ready, true)
        t.eq(view.own.sprint_meter, 0.5)
        -- Other players expose no remaining timers and no meters at all.
        for _, other in ipairs(view.opponents) do
            t.eq(field(other, "dash_cooldown_s"), nil)
            t.eq(field(other, "sprint_meter"), nil)
            t.eq(field(other, "charge"), nil)
            t.eq(field(other, "pass_charge"), nil)
        end
    end)

    -- The guard the previous version of this suite lacked. A `type(x) == "boolean"`
    -- assertion passes whether or not the field should exist at all, so it cannot
    -- catch a visibility violation. This pins the exact field set instead: adding a
    -- field to a non-self player record fails here until it is also justified in
    -- env_observation.PLAYER_FIELDS with a citation for where it is rendered.
    t.it("exposes no non-self field without a rendered analogue", function()
        local combat_instance =
            assert(env.reset(assert(env.reference_config("combat_all_families", { seed = 5 }))))
        local views = {
            assert(env_observation.view(reset({ seed = 5 })._state, nil, 1, "representative")),
            assert(
                env_observation.view(
                    combat_instance._state,
                    combat_instance._combat,
                    1,
                    "representative"
                )
            ),
        }
        local seen = {}
        local records = 0
        for _, view in ipairs(views) do
            for _, group in ipairs({ view.teammates, view.opponents }) do
                for _, other in ipairs(group) do
                    records = records + 1
                    for name in pairs(other) do
                        seen[name] = true
                        t.is_true(
                            env_observation.PLAYER_FIELDS[name],
                            "non-self field '"
                                .. tostring(name)
                                .. "' is not in env_observation.PLAYER_FIELDS: either it has a"
                                .. " rendered analogue and belongs there with a citation, or it"
                                .. " must not be observable"
                        )
                    end
                    local telegraph = field(other, "equipment")
                    if telegraph then
                        for name in pairs(telegraph) do
                            t.is_true(
                                env_observation.EQUIPMENT_TELEGRAPH_FIELDS[name],
                                "equipment telegraph field '" .. tostring(name) .. "' is not pinned"
                            )
                        end
                    end
                end
            end
        end
        t.eq(records, 18, "both fixtures contributed nine other-player records each")
        -- Positive control: the scan really did walk populated records.
        t.is_true(seen.x and seen.side and seen.equipment, "the scan reached real fields")
        -- And the five states with no rendered analogue are gone for good.
        for _, name in ipairs({ "charging", "jockeying", "tackling", "dodging", "stunned" }) do
            t.is_true(
                not seen[name],
                name
                    .. " has no rendered analogue for a non-local player and must stay unobservable"
            )
            t.is_true(
                not env_observation.PLAYER_FIELDS[name],
                name .. " must not be re-added to the permitted non-self field set"
            )
        end
        -- Own-view readiness is unaffected: these remain visible about yourself.
        local own =
            assert(env_observation.view(reset({ seed = 5 })._state, nil, 1, "representative")).own
        t.eq(type(own.charging), "boolean")
        t.eq(type(own.stunned), "boolean")
        t.eq(type(own.jockeying), "boolean")
    end)

    t.it("shows equipment as a telegraph for others and a readout for self", function()
        local instance =
            assert(env.reset(assert(env.reference_config("combat_all_families", { seed = 9 }))))
        local view =
            assert(env_observation.view(instance._state, instance._combat, 1, "representative"))
        local own = assert(view.own.equipment)
        t.eq(own.phase, "ready")
        t.eq(own.ready, true)
        t.eq(type(own.cooldown_ticks), "number")
        t.is_true(own.loadout_id ~= nil, "own loadout identity is visible on the HUD")
        local telegraphs = 0
        for _, other in ipairs(view.opponents) do
            if other.equipment then
                telegraphs = telegraphs + 1
                t.eq(other.equipment.phase, "ready")
                t.eq(
                    field(other.equipment, "cooldown_ticks"),
                    nil,
                    "others expose no private tick counts"
                )
                t.eq(field(other.equipment, "loadout_id"), nil)
            end
        end
        t.eq(telegraphs, 4, "the four opposing outfielders carry visible equipment")
    end)

    t.it("requires a fixed-slot match", function()
        local instance = reset({ seed = 2, duration = 2 })
        instance._state.slot_mode = false
        local view, _, code = env_observation.view(instance._state, nil, 1, "representative")
        t.eq(view, nil)
        t.eq(code, "malformed")
    end)
end)

t.describe("env_observation.encode", function()
    t.it("is stable for identical observations and reacts to any change", function()
        local left = reset({ seed = 2, duration = 4 })
        local right = reset({ seed = 2, duration = 4 })
        local encoded = env_observation.encode(env.observe(left))
        t.eq(env_observation.encode(env.observe(right)), encoded)

        right._state.ball_z = right._state.ball_z + 1
        t.is_true(
            env_observation.encode(env.observe(right)) ~= encoded,
            "a changed cue must change the encoding"
        )
    end)

    t.it("distinguishes profiles", function()
        local representative = reset({ seed = 2, duration = 4 })
        local privileged_config = assert(env.reference_config("soccer_only", { seed = 2 }))
        privileged_config.observation_profile = "privileged"
        local privileged = assert(env.reset(privileged_config))
        t.is_true(
            env_observation.encode(env.observe(privileged))
                ~= env_observation.encode(env.observe(representative)),
            "the privileged profile carries strictly more"
        )
    end)
end)

t.describe("env_observation.view_for", function()
    t.it("returns the requested slot view or nil", function()
        local config = assert(env.reference_config("soccer_only", { seed = 2, duration = 4 }))
        config.observation_profile = "team"
        config.slot_sources[5] = { kind = "policy" }
        local instance = assert(env.reset(config))
        local observation = env.observe(instance)
        t.eq(assert(env_observation.view_for(observation, 5)).slot, 5)
        t.eq(env_observation.view_for(observation, 2), nil)
        t.eq(#observation.slots, 2)
        t.eq(observation.slots[2], 5)
        t.eq(#instance.config.slot_sources, input_frame.SLOT_COUNT)
    end)
end)
