local t = require("spec.support.runner")
local match = require("sim.match")
local teams = require("data.teams")
local Vec2 = require("core.vec2")

-- Oscar's scenario: home keeper with the ball IN HAND, one opposing attacker
-- pressing at the respect ring, two defenders offered as outlets, everyone
-- else parked far away. Throw in many directions at several charge levels and
-- count how often the home side actually keeps the ball. Hand distribution
-- must be reliable: the keeper sees the whole pitch and throws over or away
-- from a single presser.

---@param o table?
---@return MatchInput
local function input(o)
    o = o or {}
    return {
        move = o.move or Vec2.new(0, 0),
        shoot = false,
        shoot_held = false,
        pass = o.pass or false,
        pass_held = o.pass_held or false,
        switch = false,
        dash = false,
        dodge = false,
        lob = false,
        sprint = false,
        jockey = false,
    }
end

local NO_MOVE = input()

-- Build the scenario fresh (deterministic: same seed, same layout).
---@param attacker_pos Vec2
---@return MatchState
local function scenario(attacker_pos)
    local s = match.new({ home = teams.nebula, away = teams.orion, field = { w = 960, h = 540 } })
    s.owner = 1
    s.controlled = 1
    local keeper = s.players[1]
    keeper.pos = Vec2.new(60, 270)
    keeper.facing = Vec2.new(1, 0)
    keeper.hold_timer = 30 -- the test controls the release; no auto-distribute
    s.ball = Vec2.new(66, 270)

    local d1, d2, attacker
    local park_y = 40
    for i, p in ipairs(s.players) do
        if p.team == "home" and not p.is_keeper then
            if not d1 then
                d1 = i
                p.pos = Vec2.new(230, 160) -- outlet up the right channel
            elseif not d2 then
                d2 = i
                p.pos = Vec2.new(230, 380) -- outlet down the left channel
            else
                p.pos = Vec2.new(450, park_y) -- parked upfield, out of the play
                park_y = park_y + 90
            end
        elseif p.team == "away" then
            if not p.is_keeper and not attacker then
                attacker = i
                p.pos = attacker_pos
            else
                p.pos = Vec2.new(850, 40 + i * 55) -- rest of the away side far off
            end
        end
        p.anchor = p.pos
    end
    return s
end

-- Fire one throw (aim + charge), then play out up to 5 s WITHOUT any input:
-- the receive assist must do the receiver's work (a human who just threw is
-- still orienting). Returns "home" | "away" | "conceded" | "dead".
---@param attacker_pos Vec2
---@param aim Vec2
---@param charge number
---@return string outcome
local function play_throw(attacker_pos, aim, charge)
    local s = scenario(attacker_pos)
    s.players[s.controlled].pass_charge = charge
    match.step(s, 1 / 60, input({ pass = true, move = aim }))
    for _ = 1, 300 do
        match.step(s, 1 / 60, NO_MOVE)
        if s.score.away > 0 then
            return "conceded"
        end
        if s.owner then
            return s.players[s.owner].team
        end
    end
    return "dead"
end

local AIMS = {
    Vec2.new(1, 0),
    Vec2.new(1, -1),
    Vec2.new(0, -1),
    Vec2.new(1, 1),
    Vec2.new(0, 1),
    Vec2.new(-1, -1),
    Vec2.new(-1, 0),
    Vec2.new(-1, 1),
}
local CHARGES = { 0.2, 1.0 }
-- The nasty presser spots: dead ahead at the ring, and goal-side ON an outlet.
local SPOTS = {
    Vec2.new(125, 270),
    Vec2.new(205, 355),
}

---@return number kept, number total, number conceded
local function run_matrix()
    local kept, total, conceded = 0, 0, 0
    for _, spot in ipairs(SPOTS) do
        for _, aim in ipairs(AIMS) do
            for _, charge in ipairs(CHARGES) do
                local outcome = play_throw(spot, aim:normalized(), charge)
                total = total + 1
                if outcome == "home" then
                    kept = kept + 1
                elseif outcome == "conceded" then
                    conceded = conceded + 1
                end
            end
        end
    end
    return kept, total, conceded
end

t.describe("keeper hand-throw reliability (1 presser, 2 outlets)", function()
    t.it("keeps all 32 throws, any aim, and never concedes", function()
        local kept, total, conceded = run_matrix()
        t.eq(conceded, 0, "a hand throw must never gift a goal to the presser")
        t.eq(
            kept,
            total,
            ("home kept %d/%d throws — hand distribution regressed"):format(kept, total)
        )
    end)

    t.it("arcs a contested throw above the aerial strike envelope", function()
        -- Presser square on the lane to d2; throw at d2. The flight must be
        -- untouchable when it passes the presser: higher than any jump.
        local s = scenario(Vec2.new(160, 330))
        s.players[s.controlled].pass_charge = 0.2
        match.step(s, 1 / 60, input({ pass = true, move = Vec2.new(1, 1):normalized() }))
        local aerial = require("sim.aerial")
        local max_z_near_presser = 0
        for _ = 1, 300 do
            match.step(s, 1 / 60, NO_MOVE)
            local presser = s.players[7]
            if s.ball:dist(presser.pos) < 40 then
                max_z_near_presser = math.max(max_z_near_presser, s.ball_z)
            end
            if s.owner then
                break
            end
        end
        t.is_true(
            max_z_near_presser > aerial.MAX_TOUCH_Z,
            ("flight peaked at %.0f near the presser (envelope %.0f)"):format(
                max_z_near_presser,
                aerial.MAX_TOUCH_Z
            )
        )
    end)
end)
