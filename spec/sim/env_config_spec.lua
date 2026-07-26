local t = require("spec.support.runner")
local env_config = require("sim.env_config")
local input_frame = require("sim.input_frame")

---@param overrides table<string, any>?
---@return table<string, any>
local function raw(overrides)
    local config = {
        build = "spec-build",
        content = "showcase-content-v1",
        seed = 5,
        duration = 4,
    }
    for key, value in pairs(overrides or {}) do
        config[key] = value
    end
    return config
end

t.describe("env_config.normalize", function()
    t.it("stamps the version and fills documented defaults", function()
        local config = assert(env_config.normalize(raw()))
        t.eq(config.version, env_config.VERSION)
        t.eq(config.source, "spec-build", "source defaults to the build label")
        t.eq(config.max_goals, env_config.DEFAULT_MAX_GOALS)
        t.eq(config.field.w, env_config.DEFAULT_FIELD.w)
        t.eq(config.home_team_id, env_config.DEFAULT_HOME_TEAM_ID)
        t.eq(config.home_tactic_id, env_config.DEFAULT_TACTIC_ID)
        t.eq(config.combat, false)
        t.eq(config.observation_profile, "representative")
        t.eq(config.reward_team, "home")
        t.eq(#config.slot_sources, input_frame.SLOT_COUNT)
        t.eq(config.slot_sources[1].kind, "policy")
        t.eq(config.slot_sources[2].kind, "neutral")
        t.eq(config.objective_channels[1], "match_outcome")
        t.eq(#config.shaping_channels, 0)
        t.eq(config.max_episode_ticks, nil)
        t.eq(config.fixture, "nebula-v-orion;2-1-1-v-1-1-2;balanced-v-balanced;combat=false")
    end)

    t.it("copies rather than aliases the caller's tables", function()
        local sources = env_config.default_slot_sources()
        local field = { w = 800, h = 480 }
        local config = assert(env_config.normalize(raw({ slot_sources = sources, field = field })))
        sources[1].kind = "neutral"
        field.w = 1
        t.eq(config.slot_sources[1].kind, "policy")
        t.eq(config.field.w, 800)
    end)

    t.it("rejects unknown fields and unsupported versions", function()
        local unknown, _, unknown_code = env_config.normalize(raw({ learning_rate = 0.1 }))
        t.eq(unknown, nil)
        t.eq(unknown_code, "malformed")

        local versioned, _, versioned_code = env_config.normalize(raw({ version = 99 }))
        t.eq(versioned, nil)
        t.eq(versioned_code, "unsupported_version")
    end)

    t.it("rejects missing provenance and bad scalars", function()
        local no_build, _, no_build_code = env_config.normalize({ seed = 1 })
        t.eq(no_build, nil, "a config without provenance is rejected")
        t.eq(no_build_code, "malformed")

        local no_seed, _, no_seed_code = env_config.normalize({ build = "spec-build" })
        t.eq(no_seed, nil, "a config without a seed is rejected")
        t.eq(no_seed_code, "malformed")

        for _, case in ipairs({
            { overrides = { build = "" }, code = "malformed" },
            { overrides = { seed = 1.5 }, code = "malformed" },
            { overrides = { duration = 0 }, code = "malformed" },
            { overrides = { max_goals = 0 }, code = "malformed" },
            { overrides = { field = { w = 0, h = 10 } }, code = "malformed" },
            { overrides = { field = { w = 10, h = 10, d = 1 } }, code = "malformed" },
            { overrides = { max_episode_ticks = 0 }, code = "malformed" },
            { overrides = { reward_team = "both" }, code = "malformed" },
            { overrides = { observation_profile = "oracle" }, code = "malformed" },
        }) do
            local config, _, code = env_config.normalize(raw(case.overrides))
            t.eq(config, nil, "config must be rejected")
            t.eq(code, case.code)
        end
    end)

    t.it("rejects unknown authored content", function()
        for _, overrides in ipairs({
            { home_team_id = "void" },
            { away_team_id = "void" },
            { home_formation = "9-9-9" },
            { away_formation = "9-9-9" },
            { home_tactic_id = "chaos" },
            { away_tactic_id = "chaos" },
        }) do
            local config, _, code = env_config.normalize(raw(overrides))
            t.eq(config, nil)
            t.eq(code, "unknown_content")
        end
    end)

    t.it("validates slot sources strictly", function()
        -- The mutator sees the array as plain data so a spec can author the exact
        -- malformed rows the contract has to reject.
        ---@param mutate fun(sources: table<integer, any>)
        ---@return string?
        local function code_for(mutate)
            ---@type table<integer, any>
            local sources = env_config.default_slot_sources()
            mutate(sources)
            local _, _, code = env_config.normalize(raw({ slot_sources = sources }))
            return code
        end
        t.eq(
            code_for(function(sources)
                sources[2] = { kind = "bot" }
            end),
            "malformed",
            "a bot slot needs a seed"
        )
        t.eq(
            code_for(function(sources)
                sources[2] = { kind = "neutral", seed = 3 }
            end),
            "malformed",
            "only bot slots carry a seed"
        )
        t.eq(
            code_for(function(sources)
                sources[2] = { kind = "neutral", policy_id = "x" }
            end),
            "malformed",
            "only policy slots carry a policy id"
        )
        t.eq(
            code_for(function(sources)
                sources[2] = { kind = "oracle" }
            end),
            "malformed",
            "unknown kinds are rejected"
        )
        t.eq(
            code_for(function(sources)
                sources[8] = nil
            end),
            "malformed",
            "every canonical slot must be declared"
        )
        t.eq(
            code_for(function(sources)
                sources[1] = { kind = "neutral" }
            end),
            "empty_selection",
            "an environment with no policy slot is useless"
        )
    end)

    t.it("keeps the representative profile to exactly one controlled slot", function()
        local sources = env_config.default_slot_sources()
        sources[5] = { kind = "policy" }
        local config, _, code = env_config.normalize(raw({
            slot_sources = sources,
            observation_profile = "representative",
        }))
        t.eq(config, nil)
        t.eq(code, "profile_mismatch")

        local team = assert(env_config.normalize(raw({
            slot_sources = sources,
            observation_profile = "team",
        })))
        t.eq(#env_config.controlled_slots(team), 2)
    end)

    t.it("refuses diagnostic metrics as an optimization target", function()
        local objective, objective_err = env_config.normalize(raw({
            objective_channels = { "experience_proxy_metrics" },
        }))
        t.eq(objective, nil)
        t.is_true(tostring(objective_err):find("diagnostic", 1, true) ~= nil)

        local shaping = env_config.normalize(raw({
            shaping_channels = { "experience_proxy_metrics" },
        }))
        t.eq(shaping, nil)

        local misplaced = env_config.normalize(raw({ objective_channels = { "possession_gain" } }))
        t.eq(misplaced, nil, "a shaping channel is not an objective")
    end)

    t.it("classifies tape and policy slots", function()
        local sources = env_config.default_slot_sources()
        sources[5] = { kind = "tape" }
        sources[6] = { kind = "tape" }
        local config = assert(env_config.normalize(raw({ slot_sources = sources })))
        t.eq(#env_config.controlled_slots(config), 1)
        t.eq(#env_config.tape_slots(config), 2)
        t.eq(env_config.tape_slots(config)[2], 6)
    end)
end)

t.describe("env_config.digest", function()
    t.it("is stable for equal configs and sensitive to every knob", function()
        local base = assert(env_config.normalize(raw()))
        t.eq(env_config.digest(base), env_config.digest(assert(env_config.normalize(raw()))))
        for _, overrides in ipairs({
            { duration = 5 },
            { max_goals = 2 },
            { field = { w = 800, h = 540 } },
            { away_team_id = "nebula" },
            { home_tactic_id = "press_high" },
            { combat = true },
            { observation_profile = "privileged" },
            { reward_team = "away" },
            { max_episode_ticks = 10 },
            { shaping_channels = { "possession_gain" } },
        }) do
            local other = assert(env_config.normalize(raw(overrides)))
            t.is_true(
                env_config.digest(other) ~= env_config.digest(base),
                "the digest must react to a changed knob"
            )
        end
    end)

    t.it("records per-slot ownership and policy ids", function()
        local sources = env_config.default_slot_sources()
        sources[1] = { kind = "policy", policy_id = "ppo-42" }
        sources[3] = { kind = "bot", seed = 7 }
        local digest = env_config.digest(assert(env_config.normalize(raw({
            slot_sources = sources,
        }))))
        t.is_true(digest:find("1:policy#ppo-42", 1, true) ~= nil)
        t.is_true(digest:find("3:bot@7", 1, true) ~= nil)
    end)
end)

t.describe("env_config.resolve", function()
    t.it("resolves authored ids into the fixture tables the simulation consumes", function()
        local fixture = env_config.resolve(assert(env_config.normalize(raw())))
        t.eq(fixture.home.id, "nebula")
        t.eq(fixture.away.id, "orion")
        t.eq(fixture.home_tactic.id, "balanced")
        t.is_true(fixture.players_by_id["zyro_vex"] ~= nil)
    end)
end)
