local t = require("spec.support.runner")
local Vec2 = require("core.vec2")
local coordinator = require("game.online.coordinator")
local live_slot = require("game.online.live_slot")
local input_frame = require("sim.input_frame")
local match = require("sim.match")
local teams = require("data.teams")

---@return MatchState
local function slot_match()
    return match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        duration = 4,
        max_goals = 99,
        seed = 74,
        input_ownership = match.ownership_for_teams(teams.nebula, teams.orion),
    })
end

-- Park every outfielder on a distinct ring radius so the ranking is fully
-- determined, then let a caller collapse chosen slots onto the same radius.
---@param state MatchState
---@param radii table<integer, number>
local function place(state, radii)
    state.ball = Vec2.new(480, 270)
    state.owner = nil
    for slot = 1, input_frame.SLOT_COUNT do
        local player = state.players[assert(state.slot_players[slot])]
        local radius = radii[slot] or (100 + slot * 10)
        player.pos = Vec2.new(480 + radius, 270)
        player.vel = Vec2.new(0, 0)
        player.run_vel = Vec2.new(0, 0)
    end
end

---@param ranked InputSlotId[]
---@return string
local function joined(ranked)
    return table.concat(ranked, ",")
end

t.describe("online live-slot ranking", function()
    t.it("orders every canonical slot by distance to the ball", function()
        local state = slot_match()
        place(state, { [3] = 5, [7] = 15, [1] = 25 })
        local ranked = live_slot.rank(state)
        t.eq(#ranked, input_frame.SLOT_COUNT)
        t.eq(ranked[1], "home_3")
        t.eq(ranked[2], "away_3")
        t.eq(ranked[3], "home_1")
    end)

    t.it("resolves an exact distance tie by ascending canonical slot index", function()
        local state = slot_match()
        -- Two owned slots at exactly the same distance: the classic desync.
        place(state, { [2] = 40, [4] = 40 })
        local ranked = live_slot.rank(state)
        t.eq(ranked[1], "home_2")
        t.eq(ranked[2], "home_4")

        -- The tie is on opposite sides of the ball, so the vectors differ while
        -- the squared distances are bit-identical.
        local mirrored = slot_match()
        place(mirrored, { [2] = 40, [4] = 40 })
        mirrored.players[assert(mirrored.slot_players[4])].pos = Vec2.new(480 - 40, 270)
        t.eq(joined(live_slot.rank(mirrored)), joined(ranked))
    end)

    t.it("is independent of the order slots are inserted or sorted", function()
        local state = slot_match()
        -- Every outfielder equidistant: a comparator that fell through to table
        -- order would return an arbitrary permutation here.
        place(state, {})
        for slot = 1, input_frame.SLOT_COUNT do
            state.players[assert(state.slot_players[slot])].pos = Vec2.new(480 + 60, 270)
        end
        local expected = joined(live_slot.rank(state))
        t.eq(expected, "home_1,home_2,home_3,home_4,away_1,away_2,away_3,away_4")
        for _ = 1, 16 do
            t.eq(joined(live_slot.rank(state)), expected)
        end
    end)

    t.it("never iterates a hash table in the ranking path", function()
        -- `pairs` over a Lua hash part is the classic cross-peer divergence and
        -- two peers in one process would agree by accident. Assert the source,
        -- because no single-process test can observe it.
        local source = assert(love.filesystem.read("game/online/live_slot.lua"))
        local body = source:gsub("%-%-[^\n]*", "")
        -- `ipairs` is fine and `pairs` is not, so the match is anchored on the
        -- character before the call rather than on the bare substring.
        t.eq((" " .. body):find("[^%w_]pairs%s*%("), nil)
        t.is_true((" " .. body:gsub("pairs", "ipairs")):find("[^%w_]pairs%s*%(") == nil)
    end)

    t.it("excludes the live slot so switching actually switches", function()
        local state = slot_match()
        place(state, { [1] = 5, [2] = 15 })
        local ranked = live_slot.rank(state, "home_1")
        t.eq(ranked[1], "home_2")
        for _, slot in ipairs(ranked) do
            t.is_true(slot ~= "home_1")
        end
    end)

    t.it("reports the carrier and derives the winner from a possession change", function()
        local state = slot_match()
        place(state, {})
        t.eq(live_slot.carrier(state), nil)
        state.owner = assert(state.slot_players[6])
        t.eq(live_slot.carrier(state), "away_2")

        local held = live_slot.transition(state, { switch = false, previous_carrier = "away_2" })
        t.eq(held.winner, nil)
        local won = live_slot.transition(state, { switch = false, previous_carrier = nil })
        t.eq(won.winner, "away_2")
    end)

    t.it("keeps keepers slotless", function()
        local state = slot_match()
        place(state, {})
        state.owner = 1
        t.eq(live_slot.carrier(state), nil)
    end)

    t.it("feeds a total ordering into the frozen transition rule", function()
        local state = slot_match()
        place(state, { [2] = 40, [4] = 40 })
        local owned = { "home_2", "home_4" }
        local transition = live_slot.transition(state, {
            switch = true,
            live = "home_4",
            previous_carrier = nil,
        })
        t.eq(coordinator.next_live_slot(owned, "home_4", transition), "home_2")

        -- Same geometry, opposite starting slot: the rule still lands on the
        -- one member of the owned set the ranking put first.
        local other = live_slot.transition(state, {
            switch = true,
            live = "home_2",
            previous_carrier = nil,
        })
        t.eq(coordinator.next_live_slot(owned, "home_2", other), "home_4")
    end)
end)
