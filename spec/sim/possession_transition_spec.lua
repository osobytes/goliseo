local t = require("spec.support.runner")
local metrics = require("sim.metrics")
local tactics = require("data.tactics")
local transitions = require("sim.possession_transition")

local TICK = 1 / 60

---@return table<"home"|"away", TransitionConfig>
local function balanced_windows()
    return {
        home = transitions.copy_windows(tactics.balanced.transition),
        away = transitions.copy_windows(tactics.balanced.transition),
    }
end

-- Hold `owner_team` until its possession is authoritative.
---@param state PossessionTransitionState
---@param owner_team "home"|"away"
local function establish(state, owner_team)
    for _ = 1, math.ceil(transitions.ESTABLISH_SECONDS / TICK) do
        transitions.observe(state, owner_team, TICK)
    end
end

t.describe("possession transition state", function()
    t.it("starts idle and copies defensively", function()
        local state = transitions.new_state()
        t.eq(state.version, transitions.VERSION)
        t.eq(state.last_team, nil)
        t.eq(state.holding_team, nil)
        t.eq(state.hold, 0)
        t.eq(state.turnover_team, nil)
        t.eq(state.elapsed, 0)
        t.is_true(not transitions.active(state))

        establish(state, "home")
        local copy = transitions.copy_state(state)
        state.hold = 0
        state.holding_team = nil
        state.last_team = nil
        t.eq(copy.last_team, "home")
        t.eq(copy.holding_team, "home")
        t.eq(copy.hold, transitions.ESTABLISH_SECONDS)
    end)

    t.it("uses the same settled-possession threshold the metrics collector counts", function()
        -- One counted turnover is exactly one transition window. If these ever
        -- diverge the tripwire's turnover metric stops describing the phases.
        t.eq(transitions.ESTABLISH_SECONDS, metrics.SETTLE_HOLD)
    end)

    t.it("remembers a first established possession without inventing a turnover", function()
        local state = transitions.new_state()
        establish(state, "home")
        t.eq(state.last_team, "home")
        t.eq(state.turnover_team, nil)
        t.eq(state.elapsed, 0)
        t.is_true(not transitions.active(state))
    end)

    t.it("waits for authoritative possession before opening a transition", function()
        local state = transitions.new_state()
        establish(state, "home")

        -- A touch is not possession: ownership flicker cannot open a window.
        transitions.observe(state, "away", TICK)
        transitions.observe(state, nil, TICK)
        transitions.observe(state, "home", TICK)
        t.eq(state.turnover_team, nil)
        t.eq(state.last_team, "home")

        establish(state, "away")
        t.eq(state.turnover_team, "away")
        t.eq(state.last_team, "away")
        t.eq(state.elapsed, 0)
        t.is_true(transitions.active(state))
    end)

    t.it("saturates the hold streak at the establish threshold", function()
        local state = transitions.new_state()
        for _ = 1, 600 do
            transitions.observe(state, "home", TICK)
        end
        t.eq(state.hold, transitions.ESTABLISH_SECONDS)
    end)

    t.it("lets a loose ball pause the streak without erasing the prior team", function()
        local state = transitions.new_state()
        establish(state, "home")
        transitions.observe(state, "away", TICK)
        local partial = state.hold
        t.is_true(partial > 0 and partial < transitions.ESTABLISH_SECONDS)

        for _ = 1, 300 do
            transitions.observe(state, nil, TICK)
        end
        t.eq(state.hold, partial, "loose frames must bridge, not accrue")
        t.eq(state.holding_team, "away")
        t.eq(state.last_team, "home")
        t.eq(state.turnover_team, nil)
    end)

    t.it("resets the streak when the other team touches it", function()
        local state = transitions.new_state()
        establish(state, "home")
        for _ = 1, 20 do
            transitions.observe(state, "away", TICK)
        end
        transitions.observe(state, "home", TICK)
        t.eq(state.holding_team, "home")
        t.near(state.hold, TICK, 1e-12)
        establish(state, "away")
        t.eq(state.turnover_team, "away", "a reset streak still reaches a real turnover")
    end)

    t.it("does not restart the same transition while the ball keeps changing hands", function()
        local state = transitions.new_state()
        establish(state, "home")
        establish(state, "away")
        local windows = balanced_windows()
        transitions.advance(state, 1.0, windows)
        t.near(state.elapsed, 1.0, 1e-12)

        -- Flicker back and forth: neither touch is authoritative, so the live
        -- counter-press keeps decaying instead of resetting.
        for _ = 1, 10 do
            transitions.observe(state, "home", TICK)
            transitions.observe(state, "away", TICK)
        end
        t.eq(state.turnover_team, "away")
        t.near(state.elapsed, 1.0, 1e-12)
    end)

    t.it("decays and retires a transition at the widest live window", function()
        local state = transitions.new_state()
        establish(state, "home")
        establish(state, "away")
        local windows = {
            home = transitions.copy_windows(tactics.press_high.transition),
            away = transitions.copy_windows(tactics.balanced.transition),
        }
        -- Away won it, so the limit is max(away counterattack, home counterpress).
        t.eq(transitions.window_limit(state, windows), 3.0)

        transitions.advance(state, 2.9, windows)
        t.is_true(transitions.active(state))
        transitions.advance(state, 0.1, windows)
        t.is_true(not transitions.active(state))
        t.eq(state.elapsed, 0)
        t.eq(state.last_team, "away", "retiring a window keeps the possession memory")
        t.eq(transitions.window_limit(state, windows), 0)
    end)

    t.it("clears every possession memory for a lifecycle boundary", function()
        local state = transitions.new_state()
        establish(state, "home")
        establish(state, "away")
        transitions.clear(state)
        t.eq(state.last_team, nil)
        t.eq(state.holding_team, nil)
        t.eq(state.hold, 0)
        t.eq(state.turnover_team, nil)
        t.eq(state.elapsed, 0)

        -- A cleared state cannot manufacture a turnover from the next possession.
        establish(state, "home")
        t.eq(state.turnover_team, nil)
    end)

    t.it("decays each team through brain.phase into ordinary attack and defend", function()
        local state = transitions.new_state()
        establish(state, "home")
        establish(state, "away")
        local balanced = transitions.copy_windows(tactics.balanced.transition)
        t.eq(transitions.phase(state, "away", "away", balanced), "counterattack")
        t.eq(transitions.phase(state, "home", "away", balanced), "counterpress")
        -- A spill inside the window keeps both phases: possession is loose, the
        -- transition is not.
        t.eq(transitions.phase(state, "away", nil, balanced), "counterattack")
        t.eq(transitions.phase(state, "home", nil, balanced), "counterpress")

        transitions.advance(state, 2.5, { home = balanced, away = balanced })
        t.eq(transitions.phase(state, "away", "away", balanced), "attack")
        t.eq(transitions.phase(state, "home", "away", balanced), "defend")
        t.eq(transitions.phase(state, "home", nil, balanced), "loose")
    end)

    t.it("honours the authored tactic ordering", function()
        local state = transitions.new_state()
        establish(state, "home")
        establish(state, "away")
        local balanced = transitions.copy_windows(tactics.balanced.transition)
        local press = transitions.copy_windows(tactics.press_high.transition)
        local counter = transitions.copy_windows(tactics.counter.transition)

        t.is_true(press.counterpress > balanced.counterpress, "Press High presses longer")
        t.eq(counter.counterpress, 0, "Counter Attack does not counter-press")
        t.is_true(counter.counterattack > balanced.counterattack, "Counter Attack breaks longer")

        state.elapsed = 2.7
        t.eq(transitions.phase(state, "home", "away", press), "counterpress")
        t.eq(transitions.phase(state, "home", "away", balanced), "defend")
        state.elapsed = 0
        t.eq(transitions.phase(state, "home", "away", counter), "defend")
        t.eq(transitions.phase(state, "away", "away", counter), "counterattack")
        state.elapsed = 2.7
        t.eq(transitions.phase(state, "away", "away", counter), "counterattack")
        t.eq(transitions.phase(state, "away", "away", balanced), "attack")
    end)

    t.it("selects at most the nearest two eligible counter-pressers", function()
        local candidates = {
            { player_index = 5, distance_cost = 30 },
            { player_index = 2, distance_cost = 10 },
            { player_index = 3, distance_cost = 20 },
            { player_index = 4, distance_cost = 5, eligible = false },
        }
        local chosen = transitions.select_pressers(candidates, transitions.MAX_PRESSERS)
        t.eq(transitions.MAX_PRESSERS, 2)
        t.eq(#chosen, 2)
        t.eq(chosen[1], 2)
        t.eq(chosen[2], 3)

        -- Fewer eligible players than the cap uses only what exists.
        t.eq(#transitions.select_pressers({ { player_index = 9, distance_cost = 1 } }, 2), 1)
        t.eq(#transitions.select_pressers({}, 2), 0)
        t.eq(
            #transitions.select_pressers(
                { { player_index = 9, distance_cost = 1, eligible = false } },
                2
            ),
            0
        )
    end)

    t.it("breaks equal counter-press distances by player index", function()
        local chosen = transitions.select_pressers({
            { player_index = 8, distance_cost = 12 },
            { player_index = 3, distance_cost = 12 },
            { player_index = 5, distance_cost = 12 },
        }, 2)
        t.eq(chosen[1], 3)
        t.eq(chosen[2], 5)
    end)

    t.it("pushes forwards hardest and defenders not at all on a counter-attack", function()
        t.is_true(transitions.support_push("fwd") > transitions.support_push("wide"))
        t.is_true(transitions.support_push("wide") > transitions.support_push("mid"))
        t.is_true(transitions.support_push("mid") > transitions.support_push("def"))
        t.eq(transitions.support_push("def"), 1)
        ---@type any
        local unauthored_role = "keeper"
        t.eq(
            transitions.support_push(unauthored_role),
            1,
            "an unknown role keeps the ordinary push"
        )
    end)

    t.it("rejects malformed, unsupported, and leaked state", function()
        local state = transitions.new_state()
        t.is_true(not pcall(transitions.copy_state, nil))
        local wrong_version = transitions.copy_state(state)
        wrong_version.version = transitions.VERSION + 1
        t.is_true(not pcall(transitions.copy_state, wrong_version))

        local bad_team = transitions.copy_state(state)
        rawset(bad_team, "last_team", "neutral")
        t.is_true(not pcall(transitions.copy_state, bad_team))

        local leaked = transitions.copy_state(state)
        leaked.elapsed = 1
        t.is_true(
            not pcall(transitions.copy_state, leaked),
            "idle windows cannot hold elapsed time"
        )

        local unheld = transitions.copy_state(state)
        unheld.hold = 1
        t.is_true(not pcall(transitions.copy_state, unheld))

        local overheld = transitions.copy_state(state)
        overheld.holding_team = "home"
        overheld.hold = transitions.ESTABLISH_SECONDS + 1
        t.is_true(not pcall(transitions.copy_state, overheld))

        local orphaned = transitions.copy_state(state)
        orphaned.holding_team = "home"
        orphaned.hold = transitions.ESTABLISH_SECONDS
        orphaned.last_team = "home"
        orphaned.turnover_team = "away"
        t.is_true(not pcall(transitions.copy_state, orphaned))

        local detached = transitions.copy_state(state)
        detached.last_team = "home"
        t.is_true(not pcall(transitions.copy_state, detached))

        t.is_true(not pcall(transitions.observe, state, "middle", TICK))
        t.is_true(not pcall(transitions.observe, state, "home", -1))
        t.is_true(not pcall(transitions.advance, state, -1, balanced_windows()))
        t.is_true(not pcall(transitions.phase, state, "middle", nil, balanced_windows().home))
        t.is_true(not pcall(transitions.copy_windows, { counterpress = -1, counterattack = 1 }))
        ---@type any
        local partial_windows = { counterpress = 1 }
        t.is_true(not pcall(transitions.copy_windows, partial_windows))
        t.is_true(not pcall(transitions.select_pressers, {}, -1))
        t.is_true(not pcall(transitions.select_pressers, {
            { player_index = 2, distance_cost = 1 },
            { player_index = 2, distance_cost = 2 },
        }, 2))
    end)
end)
