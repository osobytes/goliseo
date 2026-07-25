---@class CombatRenderModule
local combat_render = {}

local FAMILY_COLORS = {
    unarmed = { 0.55, 0.95, 1 },
    guard = { 0.75, 0.9, 1 },
    light_melee = { 1, 0.72, 0.28 },
    ranged = { 1, 0.45, 0.78 },
}

---@param direction Vec2
---@return number
---@return number
local function unit(direction)
    local length = math.sqrt(direction.x * direction.x + direction.y * direction.y)
    if length < 1e-9 then
        return 1, 0
    end
    return direction.x / length, direction.y / length
end

---@param sample CombatPlayerPresentation
---@return number
local function telegraph_alpha(sample)
    if sample.phase == "active" then
        return 0.95
    elseif sample.phase == "guard" or sample.phase == "aim" then
        return 0.72
    end
    return 0.35 + sample.phase_progress * 0.35
end

---@param project fun(wx: number, wy: number): number, number, number
---@param sample CombatPlayerPresentation
---@param position { x: number, y: number }
local function draw_arc(project, sample, position)
    local color = assert(FAMILY_COLORS[assert(sample.family_id)])
    local direction_x, direction_y = unit(sample.direction)
    local heading = math.atan2(direction_y, direction_x)
    local half_arc = math.rad(assert(sample.front_arc_degrees) / 2)
    local radius = sample.reach_px or 34
    local alpha = telegraph_alpha(sample)
    local center_x, center_y = project(position.x, position.y)
    local prior_x, prior_y ---@type number?, number?
    local first_x, first_y ---@type number?, number?
    local last_x, last_y ---@type number?, number?
    love.graphics.setColor(color[1], color[2], color[3], alpha)
    love.graphics.setLineWidth(sample.telegraph_kind == "guard_arc" and 2.5 or 1.8)
    for index = 0, 14 do
        local angle = heading - half_arc + 2 * half_arc * index / 14
        local world_x = position.x + math.cos(angle) * radius
        local world_y = position.y + math.sin(angle) * radius
        local screen_x, screen_y = project(world_x, world_y)
        if prior_x then
            love.graphics.line(prior_x, assert(prior_y), screen_x, screen_y)
        else
            first_x, first_y = screen_x, screen_y
        end
        prior_x, prior_y = screen_x, screen_y
        last_x, last_y = screen_x, screen_y
    end

    if sample.telegraph_kind == "guard_arc" then
        local inner_prior_x, inner_prior_y ---@type number?, number?
        for index = 0, 10 do
            local angle = heading - half_arc + 2 * half_arc * index / 10
            local screen_x, screen_y = project(
                position.x + math.cos(angle) * radius * 0.7,
                position.y + math.sin(angle) * radius * 0.7
            )
            if inner_prior_x then
                love.graphics.line(inner_prior_x, assert(inner_prior_y), screen_x, screen_y)
            end
            inner_prior_x, inner_prior_y = screen_x, screen_y
        end
        love.graphics.line(center_x, center_y, assert(first_x), assert(first_y))
        love.graphics.line(center_x, center_y, assert(last_x), assert(last_y))
    else
        local tip_x, tip_y =
            project(position.x + direction_x * radius, position.y + direction_y * radius)
        love.graphics.line(center_x, center_y, tip_x, tip_y)
        if sample.family_id == "unarmed" then
            love.graphics.circle("line", tip_x, tip_y, 4)
        else
            local normal_x, normal_y = -direction_y, direction_x
            local left_x, left_y = project(
                position.x + direction_x * radius + normal_x * 7,
                position.y + direction_y * radius + normal_y * 7
            )
            local right_x, right_y = project(
                position.x + direction_x * radius - normal_x * 7,
                position.y + direction_y * radius - normal_y * 7
            )
            love.graphics.line(left_x, left_y, right_x, right_y)
        end
    end
    love.graphics.setLineWidth(1)
end

---@param project fun(wx: number, wy: number): number, number, number
---@param sample CombatPlayerPresentation
---@param position { x: number, y: number }
local function draw_line(project, sample, position)
    local color = FAMILY_COLORS.ranged
    local direction_x, direction_y = unit(sample.direction)
    local range = sample.projectile_range_px or 300
    local alpha = telegraph_alpha(sample)
    local start_x, start_y = project(position.x, position.y)
    local end_x, end_y = project(position.x + direction_x * range, position.y + direction_y * range)
    love.graphics.setColor(color[1], color[2], color[3], alpha)
    love.graphics.setLineWidth(sample.phase == "active" and 3 or 1.5)
    love.graphics.line(start_x, start_y, end_x, end_y)

    -- Repeated transverse gates make range and direction legible without color.
    local normal_x, normal_y = -direction_y, direction_x
    for step = 1, 4 do
        local distance = range * step / 5
        local left_x, left_y = project(
            position.x + direction_x * distance + normal_x * 5,
            position.y + direction_y * distance + normal_y * 5
        )
        local right_x, right_y = project(
            position.x + direction_x * distance - normal_x * 5,
            position.y + direction_y * distance - normal_y * 5
        )
        love.graphics.line(left_x, left_y, right_x, right_y)
    end
    love.graphics.setLineWidth(1)
end

---@param model CombatPresentationModel
---@param project fun(wx: number, wy: number): number, number, number
---@param pose CorrectionSmoothingPose?
function combat_render.draw_under(model, project, pose)
    if not model.enabled then
        return
    end
    for _, sample in ipairs(model.players) do
        local position = (pose and pose.players[sample.player_id]) or sample.position
        if sample.telegraph_kind == "line" then
            draw_line(project, sample, position)
        elseif sample.telegraph_kind then
            draw_arc(project, sample, position)
        end
    end
end

---@param model CombatPresentationModel
---@param project fun(wx: number, wy: number): number, number, number
function combat_render.draw_over(model, project)
    if not model.enabled then
        return
    end
    for _, projectile in ipairs(model.projectiles) do
        local direction_x, direction_y = unit(projectile.direction)
        local tail_x, tail_y = project(
            projectile.position.x - direction_x * 18,
            projectile.position.y - direction_y * 18
        )
        local x, y, scale = project(projectile.position.x, projectile.position.y)
        local radius = math.max(3, 5 * scale)
        love.graphics.setColor(1, 0.45, 0.78, 0.58)
        love.graphics.setLineWidth(math.max(1, 3 * scale))
        love.graphics.line(tail_x, tail_y, x, y)
        love.graphics.setColor(1, 0.9, 1, 0.98)
        love.graphics.polygon("fill", x, y - radius, x + radius, y, x, y + radius, x - radius, y)
        love.graphics.setColor(0.15, 0.03, 0.18, 0.9)
        love.graphics.circle("fill", x, y, radius * 0.32)
    end
    love.graphics.setLineWidth(1)
end

return combat_render
