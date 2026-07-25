local hud = require("game.match_hud")
local theme = require("game.ui.theme")

---@class MatchHudRenderModule
local renderer = {}

local COLORS = theme.colors

---@type table<string, love.Font>
local fonts = {}

---@param color number[]
---@param alpha number?
local function set(color, alpha)
    love.graphics.setColor(color[1], color[2], color[3], alpha or 1)
end

---@param kind "body"|"eyebrow"|"title"
---@param scale number
local function set_font(kind, scale)
    if not love.graphics.newFont or not love.graphics.setFont then
        return
    end
    local size = math.max(10, math.floor((theme.fonts[kind] or theme.fonts.body) * scale + 0.5))
    local key = kind .. "_" .. size
    if not fonts[key] then
        fonts[key] = love.graphics.newFont(size)
    end
    love.graphics.setFont(fonts[key])
end

---@param rect Rect
---@param fill number[]
---@param border number[]
---@param alpha number?
local function panel(rect, fill, border, alpha)
    set(fill, alpha or 0.94)
    love.graphics.rectangle("fill", rect.x, rect.y, rect.w, rect.h, theme.radius, theme.radius)
    set(border, 0.86)
    love.graphics.setLineWidth(theme.border_width)
    love.graphics.rectangle("line", rect.x, rect.y, rect.w, rect.h, theme.radius, theme.radius)
end

---@param shape "round"|"broad"|"angular"|"cluster"
---@param x number
---@param y number
---@param color number[]
---@param size number
local function species_mark(shape, x, y, color, size)
    set(color)
    if shape == "broad" then
        love.graphics.rectangle("fill", x - size, y - size * 0.72, size * 2, size * 1.44, 2, 2)
    elseif shape == "angular" then
        love.graphics.polygon(
            "fill",
            x,
            y - size,
            x + size,
            y + size * 0.86,
            x - size,
            y + size * 0.86
        )
    elseif shape == "cluster" then
        love.graphics.circle("fill", x - size * 0.5, y + size * 0.25, size * 0.5)
        love.graphics.circle("fill", x + size * 0.5, y + size * 0.25, size * 0.5)
        love.graphics.circle("fill", x, y - size * 0.48, size * 0.5)
    else
        love.graphics.circle("fill", x, y, size)
    end
end

---@param rect Rect
---@param amount number
---@param scale number
local function stamina(rect, amount, scale)
    local x = rect.x + 64 * scale
    local y = rect.y + 46 * scale
    local w = rect.w - 78 * scale
    local h = math.max(6, 7 * scale)
    set(COLORS.void, 0.9)
    love.graphics.rectangle("fill", x, y, w, h)
    set(amount <= 0.2 and COLORS.amber or COLORS.cyan, 0.95)
    love.graphics.rectangle("fill", x, y, w * amount, h)
    set(COLORS.text, 0.7)
    love.graphics.rectangle("line", x, y, w, h)
    for i = 1, 4 do
        local tick_x = x + w * i / 5
        love.graphics.line(tick_x, y, tick_x, y + h)
    end
end

---@param rect Rect
---@param amount number
---@param scale number
local function readiness(rect, amount, scale)
    local segments = 5
    local gap = math.max(1, 2 * scale)
    local width = (rect.w - 16 * scale - gap * (segments - 1)) / segments
    local y = rect.y + rect.h - 6 * scale
    for index = 1, segments do
        local threshold = index / segments
        set(amount + 1e-9 >= threshold and COLORS.cyan or COLORS.border_soft, 0.9)
        love.graphics.rectangle(
            "fill",
            rect.x + 8 * scale + (index - 1) * (width + gap),
            y,
            width,
            math.max(2, 3 * scale)
        )
    end
end

---@param model MatchHudModel
---@param viewport { w: number, h: number }
function renderer.draw(model, viewport)
    local layout = hud.layout(viewport)
    local scale = layout.scale

    set_font("eyebrow", scale)
    set(COLORS.text_muted)
    love.graphics.printf(model.venue, layout.venue.x, layout.venue.y, layout.venue.w, "center")

    panel(layout.scorebug, COLORS.panel, COLORS.border)
    set(COLORS.cyan)
    love.graphics.rectangle(
        "fill",
        layout.scorebug.x,
        layout.scorebug.y,
        layout.scorebug.w / 2,
        3 * scale
    )
    set(COLORS.amber)
    love.graphics.rectangle(
        "fill",
        layout.scorebug.x + layout.scorebug.w / 2,
        layout.scorebug.y,
        layout.scorebug.w / 2,
        3 * scale
    )
    set_font("body", scale)
    set(COLORS.text)
    love.graphics.printf(
        model.home_name,
        layout.scorebug.x + 14 * scale,
        layout.scorebug.y + 17 * scale,
        170 * scale,
        "left"
    )
    love.graphics.printf(
        model.away_name,
        layout.scorebug.x + layout.scorebug.w - 184 * scale,
        layout.scorebug.y + 17 * scale,
        170 * scale,
        "right"
    )
    set_font("title", scale)
    love.graphics.printf(
        ("%d  —  %d"):format(model.home_score, model.away_score),
        layout.scorebug.x + 170 * scale,
        layout.scorebug.y + 9 * scale,
        160 * scale,
        "center"
    )

    panel(layout.clock, COLORS.panel_raised, COLORS.border)
    set_font("body", scale)
    set(COLORS.text)
    love.graphics.printf(
        model.clock,
        layout.clock.x,
        layout.clock.y + 8 * scale,
        layout.clock.w,
        "center"
    )

    set_font("eyebrow", scale)
    set(COLORS.text)
    love.graphics.printf(
        model.possession,
        layout.status.x + 18 * scale,
        layout.status.y + 2 * scale,
        layout.status.w - 18 * scale,
        "center"
    )
    local diamond_x = layout.status.x + 20 * scale
    local diamond_y = layout.status.y + 8 * scale
    local diamond_r = 5 * scale
    set(COLORS.cyan)
    love.graphics.polygon(
        model.possession_marker == "filled" and "fill" or "line",
        diamond_x,
        diamond_y - diamond_r,
        diamond_x + diamond_r,
        diamond_y,
        diamond_x,
        diamond_y + diamond_r,
        diamond_x - diamond_r,
        diamond_y
    )

    panel(layout.identity, COLORS.panel, COLORS.border)
    set(model.species_color)
    love.graphics.rectangle(
        "fill",
        layout.identity.x,
        layout.identity.y,
        5 * scale,
        layout.identity.h,
        theme.radius,
        theme.radius
    )
    species_mark(
        model.species_shape,
        layout.identity.x + 27 * scale,
        layout.identity.y + 24 * scale,
        model.species_color,
        8 * scale
    )
    set_font("body", scale)
    set(COLORS.text)
    love.graphics.printf(
        model.player_name .. " · " .. model.player_detail,
        layout.identity.x + 46 * scale,
        layout.identity.y + 10 * scale,
        layout.identity.w - 58 * scale,
        "left"
    )
    set_font("eyebrow", scale)
    set(COLORS.text_muted)
    love.graphics.printf(
        "STAMINA",
        layout.identity.x + 10 * scale,
        layout.identity.y + 43 * scale,
        52 * scale,
        "left"
    )
    set(model.possession_marker == "filled" and COLORS.cyan or COLORS.text_muted)
    love.graphics.printf(
        model.player_state,
        layout.identity.x + layout.identity.w - 100 * scale,
        layout.identity.y + 29 * scale,
        88 * scale,
        "right"
    )
    stamina(layout.identity, model.stamina, scale)

    if model.equipment_label then
        panel(
            layout.combat,
            COLORS.panel,
            model.feedback_text and COLORS.amber or COLORS.border_soft
        )
        set_font("eyebrow", scale)
        set(model.feedback_text and COLORS.amber or COLORS.text)
        local text = model.feedback_text
                and ("[" .. string.upper(model.feedback_glyph or "FEEDBACK") .. "] " .. model.feedback_text)
            or ("EQUIPMENT · " .. model.equipment_label .. " · " .. assert(model.equipment_state))
        love.graphics.printf(
            text,
            layout.combat.x + 8 * scale,
            layout.combat.y + 4 * scale,
            layout.combat.w - 16 * scale,
            "center"
        )
        readiness(layout.combat, model.equipment_progress, scale)
    end

    panel(layout.plan, COLORS.panel, COLORS.border_soft)
    set_font("eyebrow", scale)
    set(COLORS.amber)
    love.graphics.printf(
        model.plan,
        layout.plan.x + 8 * scale,
        layout.plan.y + 15 * scale,
        layout.plan.w - 16 * scale,
        "center"
    )

    if model.prompt then
        panel(layout.prompt, COLORS.panel_raised, COLORS.cyan)
        set_font("eyebrow", scale)
        set(COLORS.cyan)
        love.graphics.printf(
            model.prompt.title,
            layout.prompt.x + 10 * scale,
            layout.prompt.y + 8 * scale,
            layout.prompt.w - 20 * scale,
            "center"
        )
        set_font("body", scale)
        set(COLORS.text)
        love.graphics.printf(
            model.prompt.body,
            layout.prompt.x + 10 * scale,
            layout.prompt.y + 27 * scale,
            layout.prompt.w - 20 * scale,
            "center"
        )
    end

    if model.announcement_title then
        if model.announcement_kind == "full_time" then
            set(COLORS.void, 0.78)
            love.graphics.rectangle("fill", 0, 0, viewport.w, viewport.h)
        end
        panel(
            layout.announcement,
            COLORS.panel_raised,
            model.announcement_kind == "replay" and COLORS.cyan or COLORS.amber,
            0.96
        )
        set_font("title", scale)
        set(model.announcement_kind == "replay" and COLORS.cyan or COLORS.amber)
        love.graphics.printf(
            model.announcement_title,
            layout.announcement.x + 12 * scale,
            layout.announcement.y + 24 * scale,
            layout.announcement.w - 24 * scale,
            "center"
        )
        set_font("body", scale)
        set(COLORS.text)
        love.graphics.printf(
            model.announcement_detail or "",
            layout.announcement.x + 12 * scale,
            layout.announcement.y + 62 * scale,
            layout.announcement.w - 24 * scale,
            "center"
        )
    end
end

return renderer
