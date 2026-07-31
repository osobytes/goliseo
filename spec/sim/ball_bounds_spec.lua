local t = require("spec.support.runner")
local match = require("sim.match")
local teams = require("data.teams")
local Vec2 = require("core.vec2")

---@type MatchInput
local NO_INPUT = {
    move = Vec2.new(0, 0),
    shoot = false,
    shoot_held = false,
    pass = false,
    pass_held = false,
    switch = false,
    dash = false,
    dodge = false,
    lob = false,
    sprint = false,
    jockey = false,
    equipment_held = false,
    equipment_pressed = false,
    equipment_released = false,
}

local FIELD = { w = 960, h = 540 }

local function neutral(s)
    local out = {}
    for index in ipairs(s.players) do
        out[index] = NO_INPUT
    end
    return out
end

local function scenario()
    local s = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = FIELD,
        human_controlled = false,
        seed = 11,
    })
    local carrier = 2
    local p = s.players[carrier]
    p.pos = Vec2.new(FIELD.w * 0.5, FIELD.h - 2)
    p.facing = Vec2.new(0, 1)
    s.owner = carrier
    s.ball_vel = Vec2.new(0, 0)
    s.ball_z, s.ball_vz = 0, 0
    return s, carrier
end

t.describe("ball bounds", function()
    t.it("pulls a possessed ball back onto the pitch", function()
        -- The touchline clamp lives on the loose-ball path, and the possession
        -- branch returns before reaching it. A ball that ended up outside the
        -- pitch while owned was therefore never corrected: it sat there, the
        -- carrier could not walk out to it (players are clamped to the field),
        -- and the only way to recover it was to shoot it back in.
        local s = scenario()
        s.ball = Vec2.new(FIELD.w * 0.5, FIELD.h + 25)

        for _ = 1, 30 do
            match.step(s, 1 / 60, neutral(s))
        end

        t.is_true(
            s.ball.y <= FIELD.h,
            string.format("ball stayed outside the touchline at y=%.1f", s.ball.y)
        )
    end)

    t.it("keeps a possessed ball inside on every side", function()
        for _, corner in ipairs({
            { x = -40, y = FIELD.h / 2 },
            { x = FIELD.w + 40, y = FIELD.h / 2 },
            { x = FIELD.w / 2, y = -40 },
            { x = FIELD.w / 2, y = FIELD.h + 40 },
        }) do
            local s = scenario()
            s.ball = Vec2.new(corner.x, corner.y)
            for _ = 1, 20 do
                match.step(s, 1 / 60, neutral(s))
            end
            t.is_true(
                s.ball.x >= 0 and s.ball.x <= FIELD.w and s.ball.y >= 0 and s.ball.y <= FIELD.h,
                string.format("ball escaped at (%.1f, %.1f)", s.ball.x, s.ball.y)
            )
        end
    end)
end)
