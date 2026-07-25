local Vec2 = require("core.vec2")
local combat_presentation = require("game.presentation.combat")
local player_pose = require("game.presentation.player_pose")
local camera = require("game.render.camera")
local correction_smoothing = require("game.render.correction_smoothing")
local pitch = require("game.render.pitch")
local player_renderer = require("game.render.player_renderer")
local combat = require("sim.combat")
local match = require("sim.match")
local teams = require("data.teams")
local t = require("spec.support.runner")

---@class RecordedGeometryCall
---@field kind string
---@field args any[]
---@field color number[]
---@field line_width number

---@return table, RecordedGeometryCall[]
local function stub_graphics()
    local graphics = {}
    local calls = {}
    local color = { 1, 1, 1, 1 }
    local line_width = 1
    local function noop() end
    graphics.setColor = function(...)
        color = { ... }
    end
    graphics.setLineWidth = function(width)
        line_width = width
    end
    for _, name in ipairs({
        "setBlendMode",
        "setShader",
        "push",
        "pop",
        "translate",
        "rotate",
    }) do
        graphics[name] = noop
    end
    for _, name in ipairs({ "line", "circle", "ellipse", "arc", "polygon", "rectangle" }) do
        local kind = name
        graphics[kind] = function(...)
            calls[#calls + 1] = {
                kind = kind,
                args = { ... },
                color = { color[1], color[2], color[3], color[4] },
                line_width = line_width,
            }
        end
    end
    return graphics, calls
end

---@param fn fun()
---@return RecordedGeometryCall[]
local function with_graphics(fn)
    local saved = love.graphics
    local graphics, calls = stub_graphics()
    love.graphics = graphics
    local ok, err = pcall(fn)
    love.graphics = saved
    t.is_true(ok, "combat presentation draw error: " .. tostring(err))
    return calls
end

---@param call RecordedGeometryCall
---@param red number
---@param green number
---@param blue number
---@return boolean
local function has_color(call, red, green, blue)
    return math.abs(call.color[1] - red) < 1e-9
        and math.abs(call.color[2] - green) < 1e-9
        and math.abs(call.color[3] - blue) < 1e-9
end

---@param calls RecordedGeometryCall[]
---@param red number
---@param green number
---@param blue number
---@param line_width number
---@return boolean
local function has_styled_line(calls, red, green, blue, line_width)
    for _, call in ipairs(calls) do
        if
            call.kind == "line"
            and has_color(call, red, green, blue)
            and math.abs(call.line_width - line_width) < 1e-9
        then
            return true
        end
    end
    return false
end

---@param calls RecordedGeometryCall[]
---@param kind string
---@param mode string
---@param red number
---@param green number
---@param blue number
---@return boolean
local function has_colored_primitive(calls, kind, mode, red, green, blue)
    for _, call in ipairs(calls) do
        if call.kind == kind and call.args[1] == mode and has_color(call, red, green, blue) then
            return true
        end
    end
    return false
end

---@param calls RecordedGeometryCall[]
---@param x1 number
---@param y1 number
---@param x2 number
---@param y2 number
---@return boolean
local function has_ranged_segment(calls, x1, y1, x2, y2)
    for _, call in ipairs(calls) do
        if
            call.kind == "line"
            and has_color(call, 1, 0.45, 0.78)
            and math.abs(call.line_width - 1.5) < 1e-9
            and math.abs(call.args[1] - x1) < 1e-9
            and math.abs(call.args[2] - y1) < 1e-9
            and math.abs(call.args[3] - x2) < 1e-9
            and math.abs(call.args[4] - y2) < 1e-9
        then
            return true
        end
    end
    return false
end

t.describe("combat procedural renderer", function()
    t.it("draws all family telegraphs, a projectile, and the crowded ten-player fixture", function()
        local state = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
        })
        local combat_state = combat.new_state(state)
        local configured = {}
        for index, runtime in ipairs(combat_state.players) do
            if runtime.family_id and not configured[runtime.family_id] then
                configured[runtime.family_id] = true
                if runtime.family_id == "guard" then
                    runtime.phase = "guard"
                elseif runtime.family_id == "ranged" then
                    runtime.phase = "aim"
                else
                    runtime.phase = runtime.family_id == "unarmed" and "windup" or "active"
                    runtime.phase_ticks = 2
                end
            end
        end
        combat_state.projectiles[1] = {
            family_id = "ranged",
            source_index = 4,
            source_sequence = 3,
            pos = Vec2.new(480, 270),
            dir = Vec2.new(1, 0),
            remaining_ticks = 40,
        }
        local calls = with_graphics(function()
            pitch.draw(state, { w = 1280, h = 720 }, {
                home_color = teams.nebula.color,
                away_color = teams.orion.color,
                combat = combat_presentation.model(state, combat_state),
            })
        end)

        t.is_true(has_styled_line(calls, 0.55, 0.95, 1, 1.8), "unarmed arc primitive")
        t.is_true(has_styled_line(calls, 0.75, 0.9, 1, 2.5), "guard fan primitive")
        t.is_true(has_styled_line(calls, 1, 0.72, 0.28, 1.8), "melee arc primitive")
        t.is_true(has_styled_line(calls, 1, 0.45, 0.78, 1.5), "ranged line primitive")
        t.is_true(
            has_colored_primitive(calls, "polygon", "fill", 1, 0.9, 1),
            "projectile diamond primitive"
        )
        t.is_true(has_colored_primitive(calls, "circle", "fill", 1, 0.95, 0.7), "ball primitive")
    end)

    t.it("anchors rollback telegraphs to the correction-smoothed avatar pose", function()
        local viewport = { w = 1280, h = 720 }
        local state = match.new({
            home = teams.nebula,
            away = teams.orion,
            field = { w = 960, h = 540 },
            seed = 113,
        })
        local combat_state = combat.new_state(state)
        local ranged_index
        for index, runtime in ipairs(combat_state.players) do
            if runtime.family_id == "ranged" then
                ranged_index = index
                runtime.phase = "aim"
                break
            end
        end
        local index = assert(ranged_index)
        local player = state.players[index]
        player.facing = Vec2.new(1, 0)
        local smoothing = correction_smoothing.new(state)
        player.pos = Vec2.new(player.pos.x + 80, player.pos.y)
        smoothing = correction_smoothing.correct(smoothing, state)
        local pose = correction_smoothing.pose(smoothing)
        local smoothed = assert(pose.players[player.id])
        local authoritative = player.pos
        local model = combat_presentation.model(state, combat_state)
        local range = assert(model.players[index].projectile_range_px)
        local smooth_x, smooth_y = camera.project(smoothed.x, smoothed.y, state.field, viewport)
        local smooth_end_x, smooth_end_y =
            camera.project(smoothed.x + range, smoothed.y, state.field, viewport)
        local authority_x, authority_y =
            camera.project(authoritative.x, authoritative.y, state.field, viewport)
        local authority_end_x, authority_end_y =
            camera.project(authoritative.x + range, authoritative.y, state.field, viewport)

        local calls = with_graphics(function()
            pitch.draw(state, viewport, {
                home_color = teams.nebula.color,
                away_color = teams.orion.color,
                render_pose = pose,
                combat = model,
            })
        end)

        t.is_true(
            has_ranged_segment(calls, smooth_x, smooth_y, smooth_end_x, smooth_end_y),
            "telegraph origin must match the displayed avatar pose"
        )
        t.is_true(
            not has_ranged_segment(
                calls,
                authority_x,
                authority_y,
                authority_end_x,
                authority_end_y
            ),
            "telegraph must not jump to corrected authority ahead of the avatar"
        )
    end)

    t.it("draws all six procedural equipment proxies through compatible poses", function()
        with_graphics(function()
            local fixtures = {
                { "toy_spring_gloves", "unarmed", "combat_active" },
                { "medieval_heater_shield", "guard", "combat_guard" },
                { "medieval_tournament_sword", "light_melee", "combat_windup" },
                { "scifi_energy_blade", "light_melee", "combat_active" },
                { "toy_foam_sword", "light_melee", "combat_recovery" },
                { "scifi_pulse_blaster", "ranged", "combat_aim" },
            }
            for _, fixture in ipairs(fixtures) do
                local combat_sample = {
                    player_index = 2,
                    player_id = "fixture",
                    family_id = fixture[2],
                    family_name = fixture[2],
                    equipment_presentation_id = fixture[1],
                    equipment_name = fixture[1],
                    equipment_attachment = "right_hand",
                    phase = fixture[3] == "combat_recovery" and "recovery"
                        or (fixture[3] == "combat_aim" and "aim")
                        or (fixture[3] == "combat_guard" and "guard")
                        or (fixture[3] == "combat_windup" and "windup")
                        or "active",
                    phase_progress = 0.5,
                    phase_ticks = 3,
                    cooldown_ticks = 8,
                    cooldown_fraction = 0.5,
                    readiness = "committed",
                    forced_ticks = 0,
                    immunity_ticks = 0,
                    position = Vec2.new(100, 100),
                    direction = Vec2.new(1, 0),
                }
                player_renderer.draw(200, 300, 12, { 0.2, 0.8, 1 }, nil, {
                    facing = Vec2.new(1, 0),
                    is_keeper = false,
                    controlled = true,
                    species_shape = "round",
                    species_color = { 1, 0.7, 0.2 },
                    team = "home",
                    combat = combat_sample,
                    pose = {
                        id = fixture[3],
                        priority = player_pose.PRIORITY[fixture[3]],
                        source = "combat",
                    },
                })
            end
        end)
    end)

    t.it("draws distinct forced-state silhouettes without renderer-owned outcomes", function()
        with_graphics(function()
            for _, id in ipairs({ "combat_stagger", "combat_knockback" }) do
                player_renderer.draw(200, 300, 12, { 0.2, 0.8, 1 }, nil, {
                    facing = Vec2.new(1, 0),
                    is_keeper = false,
                    controlled = false,
                    team = "away",
                    pose = {
                        id = id,
                        priority = player_pose.PRIORITY[id],
                        source = "combat",
                    },
                })
            end
        end)
    end)
end)
