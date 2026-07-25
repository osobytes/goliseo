local t = require("spec.support.runner")
local brain = require("sim.brain")
local rng = require("core.rng")

---@param overrides table?
---@return BrainPhaseContext
local function phase_context(overrides)
    local context = {
        possession = "team",
        transition = nil,
        transition_elapsed = 0,
        counterpress_window = 2.5,
        counterattack_window = 2.5,
    }
    for key, value in pairs(overrides or {}) do
        context[key] = value
    end
    return context
end

---@param overrides table?
---@return BrainPressContext
local function press_context(overrides)
    local context = {
        heavy_touch = false,
        exposed_ball = false,
        cover_available = false,
        box_desperation = false,
        press_discipline = 0.8,
        low_discipline_threshold = 0.35,
    }
    for key, value in pairs(overrides or {}) do
        context[key] = value
    end
    return context
end

---@param id string
---@param score number
---@param kind string?
---@return BrainScoredOption
local function option(id, score, kind)
    return {
        id = id,
        kind = kind or "pass",
        score = score,
        reference = id,
    }
end

---@param player_index integer
---@param granted_at number
---@param expires_at number
---@return RunSlot
local function run_slot(player_index, granted_at, expires_at)
    return {
        player_index = player_index,
        run_type = "come_short",
        score = 1,
        target_x = 450,
        target_y = 270,
        granted_at = granted_at,
        expires_at = expires_at,
    }
end

t.describe("brain.phase", function()
    t.it("returns ordinary phases without an active transition", function()
        t.eq(brain.phase(phase_context()), "attack")
        t.eq(brain.phase(phase_context({ possession = "opponent" })), "defend")
        t.eq(brain.phase(phase_context({ possession = "loose" })), "loose")
    end)

    t.it("keeps transition phases only inside their caller-owned windows", function()
        t.eq(
            brain.phase(phase_context({
                possession = "loose",
                transition = "lost",
                transition_elapsed = 2.49,
                counterpress_window = 2.5,
            })),
            "counterpress"
        )
        t.eq(
            brain.phase(phase_context({
                transition = "won",
                transition_elapsed = 2.49,
                counterattack_window = 2.5,
            })),
            "counterattack"
        )
        t.eq(
            brain.phase(phase_context({
                possession = "opponent",
                transition = "lost",
                transition_elapsed = 2.5,
                counterpress_window = 2.5,
            })),
            "defend"
        )
        t.eq(
            brain.phase(phase_context({
                transition = "won",
                transition_elapsed = 2.5,
                counterattack_window = 2.5,
            })),
            "attack"
        )
    end)

    t.it("disables a transition when its supplied window is zero", function()
        t.eq(
            brain.phase(phase_context({
                possession = "opponent",
                transition = "lost",
                counterpress_window = 0,
            })),
            "defend"
        )
    end)
end)

t.describe("brain.refresh_interval", function()
    t.it("maps scan rate linearly from the slow endpoint to the fast endpoint", function()
        t.near(brain.refresh_interval(0, 0.45, 0.15), 0.45)
        t.near(brain.refresh_interval(0.5, 0.45, 0.15), 0.3)
        t.near(brain.refresh_interval(1, 0.45, 0.15), 0.15)
    end)

    t.it("saturates out-of-range and non-finite scan rates", function()
        t.near(brain.refresh_interval(-2, 0.45, 0.15), 0.45)
        t.near(brain.refresh_interval(2, 0.45, 0.15), 0.15)
        t.near(brain.refresh_interval(0 / 0, 0.45, 0.15), 0.45)
        t.near(brain.refresh_interval(math.huge, 0.45, 0.15), 0.15)
    end)

    t.it("accepts an equal fixed interval", function()
        t.near(brain.refresh_interval(0, 0.3, 0.3), 0.3)
        t.near(brain.refresh_interval(1, 0.3, 0.3), 0.3)
    end)

    t.it("rejects semantically reversed interval endpoints", function()
        local ok = pcall(function()
            brain.refresh_interval(0.5, 0.15, 0.45)
        end)
        t.is_true(not ok)
    end)
end)

t.describe("brain.assign_presser", function()
    t.it("uses distance then player index as a deterministic total order", function()
        local candidates = {
            { player_index = 7, distance_cost = 20 },
            { player_index = 3, distance_cost = 20 },
            { player_index = 2, distance_cost = 30 },
        }
        t.eq(brain.assign_presser(candidates, nil, 0.15), 3)
    end)

    t.it("keeps the current eligible presser through marginal changes", function()
        local candidates = {
            { player_index = 1, distance_cost = 100 },
            { player_index = 2, distance_cost = 86 },
        }
        t.eq(brain.assign_presser(candidates, 1, 0.15), 1)
        candidates[2].distance_cost = 85
        t.eq(brain.assign_presser(candidates, 1, 0.15), 2)
    end)

    t.it("replaces an ineligible current presser and returns nil without candidates", function()
        local candidates = {
            { player_index = 1, distance_cost = 5, eligible = false },
            { player_index = 2, distance_cost = 20 },
        }
        t.eq(brain.assign_presser(candidates, 1, 0.15), 2)
        t.is_true(
            brain.assign_presser(
                { { player_index = 1, distance_cost = 5, eligible = false } },
                1,
                0.15
            ) == nil
        )
    end)
end)

t.describe("brain run arbitration", function()
    local context = {
        players = {
            {
                player_index = 4,
                in_behind = { score = 8, x = 800, y = 200, duration = 1.8 },
                come_short = { score = 6, x = 500, y = 240, duration = 1.5 },
            },
            {
                player_index = 2,
                hold_width = { score = 8, x = 620, y = 50, duration = 1.8 },
            },
            {
                player_index = 3,
                eligible = false,
                in_behind = { score = 99, x = 900, y = 270, duration = 1.8 },
            },
        },
    }

    t.it("builds a deterministic total order across all run types", function()
        local candidates = brain.run_candidates(context)
        t.eq(#candidates, 3)
        t.eq(candidates[1].player_index, 2)
        t.eq(candidates[1].run_type, "hold_width")
        t.eq(candidates[2].player_index, 4)
        t.eq(candidates[2].run_type, "in_behind")
        t.eq(candidates[3].run_type, "come_short")
    end)

    t.it("respects the cap and grants only the best request per player", function()
        local slots = brain.grant_runs(brain.run_candidates(context), {}, 2, 10)
        t.eq(#slots, 2)
        t.eq(slots[1].player_index, 2)
        t.eq(slots[2].player_index, 4)
        t.eq(slots[1].expires_at, 11.8)
    end)

    t.it("preserves unexpired active slots instead of re-litigating them", function()
        local active = {
            {
                player_index = 4,
                run_type = "come_short",
                score = 1,
                target_x = 450,
                target_y = 270,
                granted_at = 8,
                expires_at = 12,
            },
        }
        local candidates = {
            {
                player_index = 2,
                run_type = "in_behind",
                score = 100,
                target_x = 900,
                target_y = 200,
                duration = 1.8,
            },
        }
        local slots = brain.grant_runs(candidates, active, 1, 10)
        t.eq(#slots, 1)
        t.eq(slots[1].player_index, 4)
        t.eq(slots[1].run_type, "come_short")
        t.eq(slots[1].expires_at, 12)
        slots[1].expires_at = 0
        t.eq(active[1].expires_at, 12, "the resolver does not mutate caller state")
    end)

    t.it("replaces expired slots from the ranked candidate list", function()
        local active = {
            {
                player_index = 4,
                run_type = "come_short",
                score = 1,
                target_x = 450,
                target_y = 270,
                granted_at = 8,
                expires_at = 10,
            },
        }
        local candidates = {
            {
                player_index = 2,
                run_type = "in_behind",
                score = 9,
                target_x = 900,
                target_y = 200,
                duration = 1.8,
            },
        }
        local slots = brain.grant_runs(candidates, active, 1, 10)
        t.eq(#slots, 1)
        t.eq(slots[1].player_index, 2)
        t.eq(slots[1].granted_at, 10)
        t.eq(slots[1].expires_at, 11.8)
    end)

    t.it("returns no slots when the configured maximum is zero", function()
        local slots = brain.grant_runs({
            {
                player_index = 2,
                run_type = "in_behind",
                score = 9,
                target_x = 900,
                target_y = 200,
                duration = 1.8,
            },
        }, { run_slot(4, 8, 12) }, 0, 10)
        t.eq(#slots, 0)
    end)

    t.it("preserves the earliest grants when active slots exceed a lowered cap", function()
        local active = {
            run_slot(4, 7, 20),
            run_slot(3, 5, 20),
            run_slot(2, 6, 20),
        }
        local slots = brain.grant_runs({}, active, 2, 10)
        t.eq(#slots, 2)
        t.eq(slots[1].player_index, 3)
        t.eq(slots[2].player_index, 2)
    end)
end)

t.describe("brain.press_mode", function()
    t.it("attributes every disciplined commit to one stable reason", function()
        local cases = {
            { field = "heavy_touch", reason = "heavy_touch" },
            { field = "exposed_ball", reason = "exposed_ball" },
            { field = "cover_available", reason = "cover" },
            { field = "box_desperation", reason = "box_desperation" },
        }
        for _, case in ipairs(cases) do
            local mode, reason = brain.press_mode(press_context({ [case.field] = true }))
            t.eq(mode, "commit", case.field)
            t.eq(reason, case.reason, case.field)
        end
    end)

    t.it("uses a distinct low-discipline fallback", function()
        local mode, reason = brain.press_mode(press_context({ press_discipline = 0.2 }))
        t.eq(mode, "commit")
        t.eq(reason, "low_discipline")
    end)

    t.it("contains without a commit trigger", function()
        local mode, reason = brain.press_mode(press_context())
        t.eq(mode, "contain")
        t.eq(reason, "no_trigger")
    end)

    t.it("contains at the low-discipline threshold boundary", function()
        local mode, reason = brain.press_mode(press_context({
            press_discipline = 0.35,
            low_discipline_threshold = 0.35,
        }))
        t.eq(mode, "contain")
        t.eq(reason, "no_trigger")
    end)

    t.it("uses a stable trigger precedence", function()
        local mode, reason = brain.press_mode(press_context({
            heavy_touch = true,
            exposed_ball = true,
            cover_available = true,
            box_desperation = true,
            press_discipline = 0,
        }))
        t.eq(mode, "commit")
        t.eq(reason, "heavy_touch")
    end)
end)

t.describe("brain scored option selection", function()
    t.it("returns exact argmax without consuming RNG at zero temperature", function()
        local state = rng.seed(71)
        local selected, next_state = brain.select_scored_option({
            option("safe", 4),
            option("best", 9),
            option("risky", 7),
        }, 0, state)
        t.eq(selected.id, "best")
        t.eq(next_state, state)
    end)

    t.it("breaks argmax ties by stable kind and id", function()
        local selected = brain.select_scored_option({
            option("z", 9, "shoot"),
            option("b", 9, "pass"),
            option("a", 9, "pass"),
        }, 0, rng.seed(1))
        t.eq(selected.kind, "pass")
        t.eq(selected.id, "a")
    end)

    t.it("is reproducible for the same options and seed", function()
        local options = {
            option("short", 1),
            option("through", 1.1),
            option("carry", 0.9, "dribble"),
        }
        local first, first_state = brain.select_scored_option(options, 0.8, rng.seed(904))
        local second, second_state = brain.select_scored_option(options, 0.8, rng.seed(904))
        t.eq(first.kind, second.kind)
        t.eq(first.id, second.id)
        t.eq(first_state, second_state)
    end)

    t.it("is stable when caller option order changes", function()
        local ordered = {
            option("short", 1),
            option("through", 1.1),
            option("carry", 0.9, "dribble"),
        }
        local reversed = { ordered[3], ordered[2], ordered[1] }
        local first, first_state = brain.select_scored_option(ordered, 0.8, rng.seed(904))
        local second, second_state = brain.select_scored_option(reversed, 0.8, rng.seed(904))
        t.eq(first.kind, second.kind)
        t.eq(first.id, second.id)
        t.eq(first_state, second_state)
    end)

    t.it("treats delimiter-like kind and id bytes as distinct identity fields", function()
        local selected = brain.select_scored_option({
            option("c", 2, "a\0b"),
            option("b\0c", 1, "a"),
        }, 0, rng.seed(1))
        t.eq(selected.kind, "a\0b")
        t.eq(selected.id, "c")
    end)

    t.it("keeps the generic selector open to non-soccer kinds and payloads", function()
        local selected = brain.select_scored_option({
            {
                id = "guard-break",
                kind = "equipment",
                score = 10,
                payload = { family = "light_melee", target = 4 },
            },
            option("pass", 2),
        }, 0, rng.seed(15))
        t.eq(selected.kind, "equipment")
        t.eq(assert(selected.payload).family, "light_melee")
    end)

    t.it("makes a fully composed carrier deterministic under pressure", function()
        local state = rng.seed(15)
        local selected, next_state = brain.decide_carrier({
            option("best", 10),
            option("other", 9),
        }, 1, 1, 2, state)
        t.eq(selected.id, "best")
        t.eq(next_state, state)
    end)

    t.it("threads explicit RNG state when soft selection is active", function()
        local state = rng.seed(15)
        local _, next_state = brain.decide_carrier({
            option("best", 10),
            option("other", 9),
        }, 0, 1, 2, state)
        t.is_true(next_state ~= state)
        local expected_state = rng.roll(state)
        t.eq(next_state, expected_state)
    end)

    t.it("uses uniform seeded selection for direct positive-infinite temperature", function()
        local state = rng.seed(1)
        local selected, next_state = brain.select_scored_option({
            option("a_low", -100),
            option("z_best", 100),
        }, math.huge, state)
        local expected_state = rng.roll(state)
        t.eq(selected.id, "a_low")
        t.eq(next_state, expected_state)
    end)

    t.it("uses uniform seeded selection when finite carrier temperature overflows", function()
        local state = rng.seed(1)
        local selected, next_state = brain.decide_carrier({
            option("a_low", -100),
            option("z_best", 100),
        }, 0, 1, 1e308, state)
        local expected_state = rng.roll(state)
        t.eq(selected.id, "a_low")
        t.eq(next_state, expected_state)
    end)
end)
