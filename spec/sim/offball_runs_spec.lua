local t = require("spec.support.runner")
local Vec2 = require("core.vec2")
local offball_runs = require("sim.offball_runs")
local stats = require("sim.stats")

---@param team "home"|"away"
---@param overrides table?
---@return OffballRunContext
local function context(team, overrides)
    local mirror = team == "home" and 1 or -1
    local function x(home_x)
        return team == "home" and home_x or 960 - home_x
    end
    local result = {
        team = team,
        field = { w = 960, h = 540 },
        carrier_pos = Vec2.new(x(300), 270),
        carrier_settled = true,
        carrier_pressure = 70,
        pressure_distance = 120,
        players = {
            {
                player_index = team == "home" and 4 or 9,
                role = "mid",
                run_drive = 0.55,
                pos = Vec2.new(x(500), 190),
                anchor_y = 0.5,
            },
            {
                player_index = team == "home" and 5 or 10,
                role = "fwd",
                run_drive = 0.8,
                pos = Vec2.new(x(570), 270),
                anchor_y = 0.5,
            },
        },
        teammates = {
            { player_index = team == "home" and 4 or 9, pos = Vec2.new(x(500), 190) },
            { player_index = team == "home" and 5 or 10, pos = Vec2.new(x(570), 270) },
        },
        opponents = {
            { pos = Vec2.new(x(650), 80), is_keeper = false },
            { pos = Vec2.new(x(620), 170), is_keeper = false },
            { pos = Vec2.new(x(610), 370), is_keeper = false },
            { pos = Vec2.new(x(640), 460), is_keeper = false },
            { pos = Vec2.new(x(920), 270), is_keeper = true },
        },
    }
    for key, value in pairs(overrides or {}) do
        result[key] = value
    end
    t.eq(mirror * mirror, 1)
    ---@cast result OffballRunContext
    return result
end

---@param slots RunSlot[]
---@param player_index integer
---@return RunSlot?
local function slot_for(slots, player_index)
    for _, slot in ipairs(slots) do
        if slot.player_index == player_index then
            return slot
        end
    end
    return nil
end

t.describe("role-gated off-ball runs", function()
    t.it("derives the authored role from formation and stable outfield ordinal", function()
        local expected = {
            ["2-1-1"] = { "def", "def", "mid", "fwd" },
            ["1-2-1"] = { "def", "wide", "wide", "fwd" },
            ["1-1-2"] = { "def", "mid", "fwd", "fwd" },
        }
        for formation_id, roles in pairs(expected) do
            for ordinal, role in ipairs(roles) do
                t.eq(offball_runs.formation_role(formation_id, ordinal), role)
            end
        end
    end)

    t.it("grants a driven forward an in-behind target beyond the last defender", function()
        local slots = offball_runs.grant(context("home"), {}, 10)
        local slot = assert(slot_for(slots, 5))
        t.eq(slot.run_type, "in_behind")
        t.is_true(slot.target_x > 650)
        t.is_true(slot.target_x > 480)
        t.eq(slot.expires_at, 11.8)
    end)

    t.it("mirrors in-behind geometry and arbitration for the away team", function()
        local home = assert(slot_for(offball_runs.grant(context("home"), {}, 10), 5))
        local away = assert(slot_for(offball_runs.grant(context("away"), {}, 10), 10))
        t.eq(home.run_type, away.run_type)
        t.near(home.target_x + away.target_x, 960)
        t.near(home.target_y, away.target_y)
        t.near(home.score, away.score)
    end)

    t.it("rejects backward or immaterial in-behind targets for both teams", function()
        local home = context("home", {
            players = {
                {
                    player_index = 4,
                    role = "fwd",
                    run_drive = 0.8,
                    pos = Vec2.new(730, 240),
                    anchor_y = 0.5,
                },
                {
                    player_index = 5,
                    role = "fwd",
                    run_drive = 0.8,
                    pos = Vec2.new(695, 300),
                    anchor_y = 0.5,
                },
            },
            teammates = {
                { player_index = 4, pos = Vec2.new(730, 240) },
                { player_index = 5, pos = Vec2.new(695, 300) },
            },
        })
        local away = context("away", {
            players = {
                {
                    player_index = 9,
                    role = "fwd",
                    run_drive = 0.8,
                    pos = Vec2.new(230, 240),
                    anchor_y = 0.5,
                },
                {
                    player_index = 10,
                    role = "fwd",
                    run_drive = 0.8,
                    pos = Vec2.new(265, 300),
                    anchor_y = 0.5,
                },
            },
            teammates = {
                { player_index = 9, pos = Vec2.new(230, 240) },
                { player_index = 10, pos = Vec2.new(265, 300) },
            },
        })

        t.eq(#offball_runs.grant(home, {}, 0), 0)
        t.eq(#offball_runs.grant(away, {}, 0), 0)
    end)

    t.it("requires the conservative drive threshold and a clear settled lane", function()
        local function authored_drive(pace, mental)
            return stats.run_drive({
                pace = pace,
                strength = 5,
                technique = 5,
                stamina = 5,
                mental = mental,
            })
        end
        t.eq(offball_runs.RUN_DRIVE_THRESHOLD, 0.55)
        t.is_true(authored_drive(8, 2) >= offball_runs.RUN_DRIVE_THRESHOLD)
        for _, stat_pair in ipairs({ { 7, 3 }, { 6, 3 }, { 5, 5 } }) do
            t.is_true(authored_drive(stat_pair[1], stat_pair[2]) < offball_runs.RUN_DRIVE_THRESHOLD)
        end

        local low_drive = context("home")
        low_drive.players[2].run_drive = offball_runs.RUN_DRIVE_THRESHOLD - 0.01
        t.is_true(slot_for(offball_runs.grant(low_drive, {}, 0), 5) == nil)

        local unsettled = context("home", { carrier_settled = false })
        t.eq(#offball_runs.grant(unsettled, {}, 0), 0)

        local blocked = context("home")
        blocked.opponents[1].pos = Vec2.new(500, 270)
        t.is_true(slot_for(offball_runs.grant(blocked, {}, 0), 5) == nil)
    end)

    t.it(
        "checks a midfield runner short under pressure and moves goal-side of its marker",
        function()
            local fixture = context("home")
            fixture.players = {
                {
                    player_index = 4,
                    role = "mid",
                    run_drive = 0.5,
                    pos = Vec2.new(500, 190),
                    anchor_y = 0.5,
                },
            }
            fixture.teammates = { { player_index = 4, pos = Vec2.new(500, 190) } }
            fixture.opponents[1].pos = Vec2.new(400, 205)
            local slot = assert(slot_for(offball_runs.grant(fixture, {}, 0), 4))
            t.eq(slot.run_type, "come_short")
            t.is_true(slot.target_x > fixture.opponents[1].pos.x)
            local distance = fixture.carrier_pos:dist(Vec2.new(slot.target_x, slot.target_y))
            t.is_true(distance >= offball_runs.MIN_SUPPORT_DISTANCE)
            t.is_true(distance <= 139)

            fixture.carrier_pressure = fixture.pressure_distance + 1
            t.eq(#offball_runs.grant(fixture, {}, 0), 0)
        end
    )

    t.it("mirrors come-short geometry and scoring for the away team", function()
        local home = context("home")
        home.players = {
            {
                player_index = 4,
                role = "mid",
                run_drive = 0.5,
                pos = Vec2.new(500, 190),
                anchor_y = 0.5,
            },
        }
        home.teammates = { { player_index = 4, pos = Vec2.new(500, 190) } }
        home.opponents[1].pos = Vec2.new(400, 205)

        local away = context("away")
        away.players = {
            {
                player_index = 9,
                role = "mid",
                run_drive = 0.5,
                pos = Vec2.new(460, 190),
                anchor_y = 0.5,
            },
        }
        away.teammates = { { player_index = 9, pos = Vec2.new(460, 190) } }
        away.opponents[1].pos = Vec2.new(560, 205)

        local home_slot = assert(slot_for(offball_runs.grant(home, {}, 0), 4))
        local away_slot = assert(slot_for(offball_runs.grant(away, {}, 0), 9))
        t.eq(home_slot.run_type, "come_short")
        t.eq(away_slot.run_type, "come_short")
        t.near(home_slot.target_x + away_slot.target_x, 960)
        t.near(home_slot.target_y, away_slot.target_y)
        t.near(home_slot.score, away_slot.score)
    end)

    t.it("rejects come-short targets without meaningful carrier progress", function()
        local projected_home = context("home", {
            players = {
                {
                    player_index = 4,
                    role = "mid",
                    run_drive = 0.5,
                    pos = Vec2.new(390, 270),
                    anchor_y = 0.5,
                },
            },
            teammates = { { player_index = 4, pos = Vec2.new(390, 270) } },
        })
        local projected_away = context("away", {
            players = {
                {
                    player_index = 9,
                    role = "mid",
                    run_drive = 0.5,
                    pos = Vec2.new(570, 270),
                    anchor_y = 0.5,
                },
            },
            teammates = { { player_index = 9, pos = Vec2.new(570, 270) } },
        })
        t.eq(#offball_runs.grant(projected_home, {}, 0), 0)
        t.eq(#offball_runs.grant(projected_away, {}, 0), 0)

        local marked_home = context("home", {
            players = {
                {
                    player_index = 4,
                    role = "mid",
                    run_drive = 0.5,
                    pos = Vec2.new(445, 270),
                    anchor_y = 0.5,
                },
            },
            teammates = { { player_index = 4, pos = Vec2.new(445, 270) } },
        })
        marked_home.opponents[1].pos = Vec2.new(415, 270)
        local marked_away = context("away", {
            players = {
                {
                    player_index = 9,
                    role = "mid",
                    run_drive = 0.5,
                    pos = Vec2.new(515, 270),
                    anchor_y = 0.5,
                },
            },
            teammates = { { player_index = 9, pos = Vec2.new(515, 270) } },
        })
        marked_away.opponents[1].pos = Vec2.new(545, 270)
        t.eq(#offball_runs.grant(marked_home, {}, 0), 0)
        t.eq(#offball_runs.grant(marked_away, {}, 0), 0)
    end)

    t.it("mirrors multi-candidate grant order while enforcing the two-run cap", function()
        local home = context("home", {
            players = {
                {
                    player_index = 4,
                    role = "fwd",
                    run_drive = 0.9,
                    pos = Vec2.new(500, 180),
                    anchor_y = 0.3,
                },
                {
                    player_index = 5,
                    role = "fwd",
                    run_drive = 0.9,
                    pos = Vec2.new(500, 360),
                    anchor_y = 0.7,
                },
                {
                    player_index = 3,
                    role = "mid",
                    run_drive = 0.6,
                    pos = Vec2.new(450, 230),
                    anchor_y = 0.5,
                },
            },
            teammates = {
                { player_index = 3, pos = Vec2.new(450, 230) },
                { player_index = 4, pos = Vec2.new(500, 180) },
                { player_index = 5, pos = Vec2.new(500, 360) },
            },
        })
        local away = context("away", {
            players = {
                {
                    player_index = 9,
                    role = "fwd",
                    run_drive = 0.9,
                    pos = Vec2.new(460, 180),
                    anchor_y = 0.3,
                },
                {
                    player_index = 10,
                    role = "fwd",
                    run_drive = 0.9,
                    pos = Vec2.new(460, 360),
                    anchor_y = 0.7,
                },
                {
                    player_index = 8,
                    role = "mid",
                    run_drive = 0.6,
                    pos = Vec2.new(510, 230),
                    anchor_y = 0.5,
                },
            },
            teammates = {
                { player_index = 8, pos = Vec2.new(510, 230) },
                { player_index = 9, pos = Vec2.new(460, 180) },
                { player_index = 10, pos = Vec2.new(460, 360) },
            },
        })
        local home_slots = offball_runs.grant(home, {}, 0)
        local away_slots = offball_runs.grant(away, {}, 0)
        t.eq(#home_slots, 2)
        t.eq(#away_slots, 2)
        for index = 1, 2 do
            t.eq(away_slots[index].player_index, home_slots[index].player_index + 5)
            t.eq(away_slots[index].run_type, home_slots[index].run_type)
            t.near(away_slots[index].target_x + home_slots[index].target_x, 960)
            t.near(away_slots[index].target_y, home_slots[index].target_y)
            t.near(away_slots[index].score, home_slots[index].score)
        end
    end)

    t.it("sends a wide role to an unoccupied outer lane at a legal support distance", function()
        local fixture = context("home", {
            carrier_pressure = 200,
            players = {
                {
                    player_index = 3,
                    role = "wide",
                    run_drive = 0.5,
                    pos = Vec2.new(500, 150),
                    anchor_y = 0.3,
                },
            },
            teammates = {
                { player_index = 3, pos = Vec2.new(500, 150) },
            },
        })
        local slot = assert(slot_for(offball_runs.grant(fixture, {}, 0), 3))
        t.eq(slot.run_type, "hold_width")
        t.is_true(slot.target_y < fixture.field.h / 3)
        local distance = fixture.carrier_pos:dist(Vec2.new(slot.target_x, slot.target_y))
        t.is_true(distance >= offball_runs.MIN_SUPPORT_DISTANCE)
        t.is_true(distance <= offball_runs.MAX_SUPPORT_DISTANCE)

        fixture.teammates[#fixture.teammates + 1] = {
            player_index = 4,
            pos = Vec2.new(fixture.carrier_pos.x + 70, fixture.field.h / 6),
        }
        t.eq(#offball_runs.grant(fixture, {}, 0), 0)
    end)

    t.it("treats a mirrored wide carrier as occupying the same width lane", function()
        local home = context("home", {
            carrier_pos = Vec2.new(300, 90),
            carrier_pressure = 200,
            players = {
                {
                    player_index = 3,
                    role = "wide",
                    run_drive = 0.5,
                    pos = Vec2.new(500, 150),
                    anchor_y = 0.3,
                },
            },
            teammates = { { player_index = 3, pos = Vec2.new(500, 150) } },
        })
        local away = context("away", {
            carrier_pos = Vec2.new(660, 450),
            carrier_pressure = 200,
            players = {
                {
                    player_index = 8,
                    role = "wide",
                    run_drive = 0.5,
                    pos = Vec2.new(460, 390),
                    anchor_y = 0.7,
                },
            },
            teammates = { { player_index = 8, pos = Vec2.new(460, 390) } },
        })

        t.eq(#offball_runs.grant(home, {}, 0), 0)
        t.eq(#offball_runs.grant(away, {}, 0), 0)
    end)

    t.it(
        "mirrors the opposite hold-width flank and rejects an empty distance intersection",
        function()
            local home = context("home", {
                carrier_pressure = 200,
                players = {
                    {
                        player_index = 3,
                        role = "wide",
                        run_drive = 0.5,
                        pos = Vec2.new(500, 400),
                        anchor_y = 0.3,
                    },
                },
                teammates = {
                    { player_index = 3, pos = Vec2.new(500, 150) },
                },
            })
            local away = context("away", {
                carrier_pressure = 200,
                players = {
                    {
                        player_index = 8,
                        role = "wide",
                        run_drive = 0.5,
                        pos = Vec2.new(460, 140),
                        anchor_y = 0.7,
                    },
                },
                teammates = {
                    { player_index = 8, pos = Vec2.new(460, 390) },
                },
            })
            local home_slot = assert(slot_for(offball_runs.grant(home, {}, 0), 3))
            local away_slot = assert(slot_for(offball_runs.grant(away, {}, 0), 8))
            t.eq(home_slot.run_type, "hold_width")
            t.eq(away_slot.run_type, "hold_width")
            t.near(home_slot.target_x + away_slot.target_x, 960)
            t.near(home_slot.target_y + away_slot.target_y, 540)
            t.near(home_slot.score, away_slot.score)

            local impossible = context("home", {
                carrier_pos = Vec2.new(480, 450),
                carrier_pressure = 200,
                players = {
                    {
                        player_index = 3,
                        role = "wide",
                        run_drive = 0.5,
                        pos = Vec2.new(500, 60),
                        anchor_y = 0.3,
                    },
                },
                teammates = {
                    { player_index = 3, pos = Vec2.new(500, 60) },
                },
            })
            t.eq(#offball_runs.grant(impossible, {}, 0), 0)
        end
    )

    t.it("rejects come-short and hold-width targets when field clamping breaks spacing", function()
        local come_home = context("home", {
            carrier_pos = Vec2.new(20, 270),
            players = {
                {
                    player_index = 4,
                    role = "mid",
                    run_drive = 0.5,
                    pos = Vec2.new(12, 270),
                    anchor_y = 0.5,
                },
            },
            teammates = {
                { player_index = 4, pos = Vec2.new(12, 270) },
            },
        })
        local come_away = context("away", {
            carrier_pos = Vec2.new(940, 270),
            players = {
                {
                    player_index = 9,
                    role = "mid",
                    run_drive = 0.5,
                    pos = Vec2.new(948, 270),
                    anchor_y = 0.5,
                },
            },
            teammates = {
                { player_index = 9, pos = Vec2.new(948, 270) },
            },
        })
        t.eq(#offball_runs.grant(come_home, {}, 0), 0)
        t.eq(#offball_runs.grant(come_away, {}, 0), 0)

        local width_home = context("home", {
            carrier_pos = Vec2.new(936, 90),
            carrier_pressure = 200,
            players = {
                {
                    player_index = 3,
                    role = "wide",
                    run_drive = 0.5,
                    pos = Vec2.new(900, 80),
                    anchor_y = 0.3,
                },
            },
            teammates = {
                { player_index = 3, pos = Vec2.new(900, 80) },
            },
        })
        local width_away = context("away", {
            carrier_pos = Vec2.new(24, 450),
            carrier_pressure = 200,
            players = {
                {
                    player_index = 8,
                    role = "wide",
                    run_drive = 0.5,
                    pos = Vec2.new(60, 460),
                    anchor_y = 0.7,
                },
            },
            teammates = {
                { player_index = 8, pos = Vec2.new(60, 460) },
            },
        })
        t.eq(#offball_runs.grant(width_home, {}, 0), 0)
        t.eq(#offball_runs.grant(width_away, {}, 0), 0)
    end)

    t.it("caps a team at two and retains marginal active assignments until expiry", function()
        local fixture = context("home", {
            players = {
                {
                    player_index = 2,
                    role = "wide",
                    run_drive = 0.4,
                    pos = Vec2.new(460, 110),
                    anchor_y = 0.3,
                },
                {
                    player_index = 3,
                    role = "wide",
                    run_drive = 0.9,
                    pos = Vec2.new(500, 430),
                    anchor_y = 0.7,
                },
                {
                    player_index = 4,
                    role = "mid",
                    run_drive = 0.7,
                    pos = Vec2.new(500, 190),
                    anchor_y = 0.5,
                },
                {
                    player_index = 5,
                    role = "fwd",
                    run_drive = 0.9,
                    pos = Vec2.new(570, 270),
                    anchor_y = 0.5,
                },
            },
            teammates = {
                { player_index = 2, pos = Vec2.new(460, 110) },
                { player_index = 3, pos = Vec2.new(500, 430) },
                { player_index = 4, pos = Vec2.new(500, 190) },
                { player_index = 5, pos = Vec2.new(570, 270) },
            },
        })
        local active = {
            {
                player_index = 2,
                run_type = "hold_width",
                score = 1,
                target_x = 410,
                target_y = 90,
                granted_at = 8,
                expires_at = 11,
            },
        }
        local slots = offball_runs.grant(fixture, active, 10)
        t.eq(#slots, offball_runs.MAX_ACTIVE_PER_TEAM)
        local retained = assert(slot_for(slots, 2))
        t.eq(retained.target_x, 410)
        t.eq(retained.expires_at, 11)
    end)

    t.it("derives the fixed telegraph window from expiry without persistent state", function()
        local expires_at = 11.8
        t.is_true(not offball_runs.telegraphing(9.99, expires_at))
        t.is_true(offball_runs.telegraphing(10, expires_at))
        t.is_true(offball_runs.telegraphing(10.199999, expires_at))
        t.is_true(not offball_runs.telegraphing(10.2, expires_at))
        t.is_true(not offball_runs.telegraphing(expires_at, expires_at))

        local negative_now = -0.2
        local negative_expiry = negative_now + offball_runs.RUN_LIFETIME_SECONDS
        t.is_true(offball_runs.telegraphing(negative_now, negative_expiry))
        t.is_true(
            not offball_runs.telegraphing(
                negative_now + offball_runs.TELEGRAPH_SECONDS,
                negative_expiry
            )
        )
    end)
end)
