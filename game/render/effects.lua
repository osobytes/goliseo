-- The "juice" layer: turns one-frame sim events (shots, passes, traps) into
-- short-lived particles, and samples a fading trail behind a fast loose ball.
-- Renderer-side state only — the sim stays pure. Rollback actions use stable
-- event keys; `update` remains the offline adapter. Drawing projects and paints.

local combat_feedback = require("game.presentation.combat_feedback")

local effects = {}

-- Match the ball's billboard lift in pitch.lua so flashes/trails sit on the ball.
local BALL_LIFT = 4
local TRAIL_LIFE = 0.32 -- base seconds a trail dot lingers (hot dots last longer)
local TRAIL_SPACING = 7 -- world units between samples (fps-independent spacing)
local TRAIL_MIN_SPEED = 80 -- only trail a ball moving faster than this
local TRAIL_HOT_SPEED = 900 -- ball speed that reads as full "heat" (charged shot)

---@class Particle
---@field x number
---@field y number
---@field vx number
---@field vy number
---@field life number
---@field max number
---@field size number
---@field color number[]
---@field kind "spark"|"ring"|"glyph"
---@field event_id string?
---@field glyph string?

---@class CombatFeedbackReadabilityObservation
---@field active_feedback_count integer
---@field concurrent_event_count integer
---@field overlap_pair_count integer
---@field occluded_count integer
---@field ball_masked boolean
---@field hud_masked boolean
---@field non_color_only boolean

---@type Particle[]
local particles = {}
---@type { x: number, y: number, z: number, heat: number, life: number, max: number }[]
local trail = {}
local last_sample ---@type { x: number, y: number }?
---@type table<string, boolean>
local speculative_events = {}
---@type table<string, boolean>
local suppressed_events = {}
---@type table<string, boolean>
local confirmed_events = {}
local reduced_motion = false
local reduced_flash = false

local function add(x, y, vx, vy, life, size, color, kind, event_id, glyph)
    particles[#particles + 1] = {
        x = x,
        y = y,
        vx = vx,
        vy = vy,
        life = life,
        max = life,
        size = size,
        color = color,
        kind = kind,
        event_id = event_id,
        glyph = glyph,
    }
end

---@param id string
---@return integer
local function stable_seed(id)
    local value = 2166136261
    for index = 1, #id do
        value = (value * 16777619 + id:byte(index)) % 2147483647
    end
    return value
end

---@param seed integer
---@return integer
---@return number
local function deterministic_unit(seed)
    local next_seed = (seed * 48271) % 2147483647
    return next_seed, next_seed / 2147483647
end

---@param x number
---@param y number
---@param n integer
---@param speed number
---@param life number
---@param size number
---@param color number[]
---@param event_id string
local function combat_burst(x, y, n, speed, life, size, color, event_id)
    local count = reduced_flash and math.min(2, n) or n
    local seed = stable_seed(event_id)
    for _ = 1, count do
        local angle_unit, speed_unit
        seed, angle_unit = deterministic_unit(seed)
        seed, speed_unit = deterministic_unit(seed)
        local angle = angle_unit * math.pi * 2
        local scalar = speed * (0.45 + speed_unit * 0.55)
        if reduced_motion then
            scalar = scalar * 0.25
        end
        add(
            x,
            y,
            math.cos(angle) * scalar,
            math.sin(angle) * scalar,
            reduced_flash and life * 0.55 or life,
            size,
            color,
            "spark",
            event_id
        )
    end
end

---@param x number
---@param y number
---@param life number
---@param size number
---@param color number[]
---@param event_id string
---@param glyph string
local function glyph(x, y, life, size, color, event_id, glyph)
    add(x, y, 0, 0, life, size, color, "glyph", event_id, glyph)
end

-- A radial spray of sparks. Direction is randomized; index-free so it's fine in
-- normal game code (unlike workflow scripts, math.random is available here).
local function burst(x, y, n, speed, life, size, color, event_id)
    for _ = 1, n do
        local a = math.random() * math.pi * 2
        local sp = speed * (0.45 + math.random() * 0.55)
        add(
            x,
            y,
            math.cos(a) * sp,
            math.sin(a) * sp,
            life * (0.6 + math.random() * 0.4),
            size,
            color,
            "spark",
            event_id
        )
    end
end

local function ring(x, y, life, size, color, event_id)
    add(x, y, 0, 0, life, size, color, "ring", event_id)
end

---@type table<ActionFamilyId, number[]>
local FAMILY_COLORS = {
    unarmed = { 0.55, 0.95, 1 },
    guard = { 0.75, 0.9, 1 },
    light_melee = { 1, 0.72, 0.28 },
    ranged = { 1, 0.45, 0.78 },
}

---@param event CombatEvent
---@param event_id string
local function spawn_combat_event(event, event_id)
    local link = combat_feedback.accepted(event, event_id)
    local disposition = link.disposition
    local color = (event.family_id and FAMILY_COLORS[event.family_id]) or { 1, 0.9, 0.55 }
    local life = reduced_flash and 0.16 or 0.32
    local size = reduced_flash and 8 or 13

    if disposition.vfx == "none" then
        return
    elseif disposition.vfx == "commit" then
        ring(event.x, event.y, life, size, color, event_id)
        glyph(event.x, event.y, life, size, color, event_id, disposition.glyph)
    elseif disposition.vfx == "projectile_spawn" then
        glyph(event.x, event.y, life, size, color, event_id, disposition.glyph)
        combat_burst(event.x, event.y, 4, 120, life, 2, color, event_id)
    elseif disposition.vfx == "projectile_expire" then
        glyph(event.x, event.y, life, size, color, event_id, disposition.glyph)
    elseif disposition.vfx == "contact_hit" or disposition.vfx == "contact_extended" then
        ring(event.x, event.y, life, size + 5, color, event_id)
        glyph(event.x, event.y, life, size + 2, color, event_id, disposition.glyph)
        combat_burst(event.x, event.y, 8, 230, life, 3, color, event_id)
    elseif disposition.vfx == "contact_guarded" or disposition.vfx == "guard_recoil" then
        ring(event.x, event.y, life, size + 2, color, event_id)
        ring(event.x, event.y, life * 1.15, size + 8, color, event_id)
        glyph(event.x, event.y, life, size + 2, color, event_id, disposition.glyph)
    elseif disposition.vfx == "contact_immune" or disposition.vfx == "contact_superseded" then
        glyph(event.x, event.y, life, size + 2, color, event_id, disposition.glyph)
    elseif disposition.vfx == "ball_spill" then
        ring(event.x, event.y, life, size + 6, { 1, 0.95, 0.7 }, event_id)
        -- Keep the authoritative event anchored to the ball while placing the
        -- semantic glyph just above it so the ball remains readable.
        glyph(event.x, event.y - 32, life, size + 3, { 1, 0.95, 0.7 }, event_id, disposition.glyph)
        combat_burst(event.x, event.y, 6, 180, life, 2, { 1, 0.95, 0.7 }, event_id)
    elseif disposition.vfx == "forced" then
        glyph(event.x, event.y, life, size + 3, color, event_id, disposition.glyph)
        combat_burst(event.x, event.y, 5, 150, life, 2, color, event_id)
    end
end

---@param settings GameSettings
function effects.configure(settings)
    reduced_motion = not settings.screen_shake
    reduced_flash = not settings.bloom
end

-- Drop drawable state while retaining stable-event bookkeeping. Replay exit
-- uses this so an action suppressed while past footage was visible cannot
-- reappear late if that same speculative ID is subsequently replaced.
function effects.reset_visuals()
    particles = {}
    trail = {}
    last_sample = nil
end

-- Drop all live effects (call on a fresh match / kickoff so nothing carries over).
function effects.reset()
    effects.reset_visuals()
    speculative_events = {}
    suppressed_events = {}
    confirmed_events = {}
end

---@param e MatchEvent
---@param event_id string?
local function spawn_event(e, event_id)
    if e.kind == "shot" then
        ring(e.x, e.y, 0.30, 16, { 1, 0.95, 0.7 }, event_id)
        burst(e.x, e.y, 10, 230, 0.35, 3, { 1, 0.9, 0.55 }, event_id)
    elseif e.kind == "pass" then
        ring(e.x, e.y, 0.24, 11, { 0.8, 0.95, 1 }, event_id)
        burst(e.x, e.y, 5, 150, 0.26, 2, { 0.8, 0.95, 1 }, event_id)
    elseif e.kind == "touch" then
        ring(e.x, e.y, 0.28, 13, { 1, 1, 1 }, event_id)
    elseif e.kind == "tackle" then
        -- A hard hit: punchier and warmer than the rest.
        ring(e.x, e.y, 0.34, 18, { 1, 0.6, 0.3 }, event_id)
        burst(e.x, e.y, 12, 260, 0.4, 3, { 1, 0.7, 0.4 }, event_id)
    elseif e.kind == "catch" or e.kind == "claim" then
        -- Clean grab/gather: a tight, confident double ring, no spray.
        ring(e.x, e.y, 0.3, 12, { 0.8, 1, 0.9 }, event_id)
        ring(e.x, e.y, 0.38, 20, { 0.6, 1, 0.8 }, event_id)
    elseif e.kind == "parry" then
        -- Deflection: a sharp cool spark fan.
        ring(e.x, e.y, 0.3, 16, { 0.7, 0.9, 1 }, event_id)
        burst(e.x, e.y, 9, 220, 0.34, 3, { 0.7, 0.9, 1 }, event_id)
    elseif e.kind == "header" then
        -- Aerial flick: a crisp high ring.
        ring(e.x, e.y, 0.28, 15, { 0.9, 1, 1 }, event_id)
        burst(e.x, e.y, 5, 170, 0.28, 2, { 0.9, 1, 1 }, event_id)
    elseif e.kind == "volley" then
        -- A volley is violence: big hot flash.
        ring(e.x, e.y, 0.34, 20, { 1, 0.8, 0.45 }, event_id)
        burst(e.x, e.y, 12, 280, 0.4, 3, { 1, 0.8, 0.45 }, event_id)
    elseif e.kind == "bicycle" then
        -- Acrobatic strike: a larger double flash, whether clean or wild.
        ring(e.x, e.y, 0.38, 24, { 1, 0.65, 0.35 }, event_id)
        ring(e.x, e.y, 0.28, 14, { 0.85, 0.95, 1 }, event_id)
        burst(e.x, e.y, 15, 310, 0.44, 3, { 1, 0.7, 0.4 }, event_id)
    elseif e.kind == "reception" then
        -- First touch: clean is compact; a heavy or missed touch splashes wider.
        local clean = e.outcome == "clean"
        ring(e.x, e.y, clean and 0.2 or 0.3, clean and 9 or 16, { 0.65, 1, 0.85 }, event_id)
        if not clean then
            burst(e.x, e.y, 5, 150, 0.26, 2, { 0.65, 1, 0.85 }, event_id)
        end
    elseif e.kind == "block" then
        -- Body block: a blunt thud off a defender, warmer and smaller than a parry.
        ring(e.x, e.y, 0.26, 14, { 1, 0.85, 0.6 }, event_id)
        burst(e.x, e.y, 6, 180, 0.3, 2, { 1, 0.85, 0.6 }, event_id)
    end
end

---@param id string
local function revoke_event(id)
    speculative_events[id] = nil
    suppressed_events[id] = nil
    for index = #particles, 1, -1 do
        if particles[index].event_id == id then
            table.remove(particles, index)
        end
    end
end

---@param event RollbackWrappedEvent
local function add_speculative(event)
    if
        (event.domain:sub(1, 6) ~= "match/" and event.domain:sub(1, 7) ~= "combat/")
        or speculative_events[event.id]
        or suppressed_events[event.id]
        or confirmed_events[event.id]
    then
        return
    end
    speculative_events[event.id] = true
    if event.domain:sub(1, 6) == "match/" then
        local payload = event.payload --[[@as MatchEvent]]
        spawn_event(payload, event.id)
    else
        local payload = event.payload --[[@as CombatEvent]]
        spawn_combat_event(payload, event.id)
    end
end

-- Apply a complete stable-event delta. Speculative action visuals are keyed by
-- event ID, so a correction can revoke or atomically replace their particles.
---@param diff RollbackEventDiff
function effects.apply_event_diff(diff)
    for _, event in ipairs(diff.revoked) do
        revoke_event(event.id)
    end
    for _, replacement in ipairs(diff.replaced) do
        local was_suppressed = suppressed_events[replacement.before.id]
            or suppressed_events[replacement.after.id]
        revoke_event(replacement.before.id)
        if was_suppressed then
            suppressed_events[replacement.after.id] = true
        else
            add_speculative(replacement.after)
        end
    end
    for _, event in ipairs(diff.added) do
        add_speculative(event)
    end
end

-- Consume live/corrected event deltas without creating drawable state while a
-- confirmed replay owns the screen. Suppressed IDs remain reversible and keep
-- replacements from surfacing late after the replay transition.
---@param diff RollbackEventDiff
function effects.discard_event_diff(diff)
    for _, event in ipairs(diff.revoked) do
        revoke_event(event.id)
    end
    for _, replacement in ipairs(diff.replaced) do
        revoke_event(replacement.before.id)
        if
            replacement.after.domain:sub(1, 6) == "match/"
            or replacement.after.domain:sub(1, 7) == "combat/"
        then
            suppressed_events[replacement.after.id] = true
        end
    end
    for _, event in ipairs(diff.added) do
        revoke_event(event.id)
        if event.domain:sub(1, 6) == "match/" or event.domain:sub(1, 7) == "combat/" then
            suppressed_events[event.id] = true
        end
    end
end

-- Once an event is confirmed it cannot be revoked. Existing particles keep
-- aging naturally, while the rollback key is retired from bounded state.
---@param event RollbackWrappedEvent
---@param suppress_spawn boolean?
---@return boolean consumed
function effects.confirm_event(event, suppress_spawn)
    if confirmed_events[event.id] then
        return false
    end
    confirmed_events[event.id] = true
    if
        event.domain:sub(1, 7) == "combat/"
        and not suppress_spawn
        and not speculative_events[event.id]
        and not suppressed_events[event.id]
    then
        local payload = event.payload --[[@as CombatEvent]]
        spawn_combat_event(payload, event.id)
    end
    speculative_events[event.id] = nil
    suppressed_events[event.id] = nil
    return true
end

---@param events CombatEvent[]
---@param stable_id fun(event: CombatEvent, ordinal: integer): string
function effects.consume_combat(events, stable_id)
    for ordinal, event in ipairs(events) do
        spawn_combat_event(event, stable_id(event, ordinal))
    end
end

-- A corrected authoritative boundary invalidates renderer-owned loose-ball
-- path samples. Action particles remain reconciled separately by stable ID.
function effects.reset_trail()
    trail = {}
    last_sample = nil
end

---@param s MatchState
function effects.sample_ball(s)
    -- Sample the ball trail only when it's a fast loose ball, spaced by distance
    -- so the trail looks the same regardless of framerate and never blobs at
    -- rest. Each dot remembers the ball's speed ("heat") and height, so a
    -- charged shot streaks hotter, longer and along its true flight path.
    local speed = s.ball_vel:length()
    if s.owner == nil and speed > TRAIL_MIN_SPEED then
        local dx = last_sample and (s.ball.x - last_sample.x) or math.huge
        local dy = last_sample and (s.ball.y - last_sample.y) or math.huge
        if dx * dx + dy * dy >= TRAIL_SPACING * TRAIL_SPACING then
            local heat = math.min(1, speed / TRAIL_HOT_SPEED)
            local life = TRAIL_LIFE * (0.7 + 0.9 * heat)
            trail[#trail + 1] =
                { x = s.ball.x, y = s.ball.y, z = s.ball_z, heat = heat, life = life, max = life }
            last_sample = { x = s.ball.x, y = s.ball.y }
        end
    else
        last_sample = nil
    end
end

-- Consume one simulated state's events and position sample. This is the
-- compatibility adapter used by the non-rollback match path.
---@param s MatchState
function effects.consume(s)
    for _, event in ipairs(s.events) do
        spawn_event(event, nil)
    end
    effects.sample_ball(s)
end

---@return { particle_count: integer, trail_count: integer, speculative_ids: string[], suppressed_ids: string[], confirmed_ids: string[] }
function effects.diagnostics()
    local speculative_ids = {}
    for id in pairs(speculative_events) do
        speculative_ids[#speculative_ids + 1] = id
    end
    local suppressed_ids = {}
    for id in pairs(suppressed_events) do
        suppressed_ids[#suppressed_ids + 1] = id
    end
    local confirmed_ids = {}
    for id in pairs(confirmed_events) do
        confirmed_ids[#confirmed_ids + 1] = id
    end
    table.sort(speculative_ids)
    table.sort(suppressed_ids)
    table.sort(confirmed_ids)
    return {
        particle_count = #particles,
        trail_count = #trail,
        speculative_ids = speculative_ids,
        suppressed_ids = suppressed_ids,
        confirmed_ids = confirmed_ids,
    }
end

---@param x number
---@param y number
---@param radius number
---@param rect Rect
---@return boolean
local function circle_intersects_rect(x, y, radius, rect)
    local nearest_x = math.max(rect.x, math.min(x, rect.x + rect.w))
    local nearest_y = math.max(rect.y, math.min(y, rect.y + rect.h))
    local dx = x - nearest_x
    local dy = y - nearest_y
    return dx * dx + dy * dy <= radius * radius
end

-- Observational hook for #148/#150: this reports the currently presented
-- geometry only. It does not create an authoritative outcome or timestamp.
---@param project fun(wx: number, wy: number): number, number, number
---@param ball { x: number, y: number }
---@param hud_rects Rect[]
---@param occluders Rect[]
---@return CombatFeedbackReadabilityObservation
function effects.readability_observation(project, ball, hud_rects, occluders)
    local rows = {}
    local ids = {}
    local non_color_only = true
    for _, particle in ipairs(particles) do
        if particle.kind == "glyph" and particle.event_id then
            local x, y, scale = project(particle.x, particle.y)
            rows[#rows + 1] = {
                x = x,
                y = y - BALL_LIFT * scale,
                radius = particle.size * scale,
            }
            ids[particle.event_id] = true
            non_color_only = non_color_only and particle.glyph ~= nil
        end
    end

    local ball_x, ball_y, ball_scale = project(ball.x, ball.y)
    local ball_radius = math.max(4, 7 * ball_scale)
    local overlap_pair_count = 0
    local occluded_count = 0
    local ball_masked = false
    local hud_masked = false
    for index, row in ipairs(rows) do
        local dx = row.x - ball_x
        local dy = row.y - ball_y
        ball_masked = ball_masked
            or dx * dx + dy * dy <= (row.radius + ball_radius) * (row.radius + ball_radius)
        for _, rect in ipairs(hud_rects) do
            hud_masked = hud_masked or circle_intersects_rect(row.x, row.y, row.radius, rect)
        end
        for _, rect in ipairs(occluders) do
            if circle_intersects_rect(row.x, row.y, row.radius, rect) then
                occluded_count = occluded_count + 1
                break
            end
        end
        for other_index = index + 1, #rows do
            local other = rows[other_index]
            local other_dx = row.x - other.x
            local other_dy = row.y - other.y
            if
                other_dx * other_dx + other_dy * other_dy
                <= (row.radius + other.radius) * (row.radius + other.radius)
            then
                overlap_pair_count = overlap_pair_count + 1
            end
        end
    end
    local concurrent_event_count = 0
    for _ in pairs(ids) do
        concurrent_event_count = concurrent_event_count + 1
    end
    return {
        active_feedback_count = #rows,
        concurrent_event_count = concurrent_event_count,
        overlap_pair_count = overlap_pair_count,
        occluded_count = occluded_count,
        ball_masked = ball_masked,
        hud_masked = hud_masked,
        non_color_only = non_color_only,
    }
end

-- Advance renderer-owned particles at display cadence.
---@param dt number
function effects.tick(dt)
    for i = #trail, 1, -1 do
        trail[i].life = trail[i].life - dt
        if trail[i].life <= 0 then
            table.remove(trail, i)
        end
    end

    for i = #particles, 1, -1 do
        local p = particles[i]
        p.life = p.life - dt
        p.x = p.x + p.vx * dt
        p.y = p.y + p.vy * dt
        local drag = math.max(0, 1 - 4 * dt)
        p.vx = p.vx * drag
        p.vy = p.vy * drag
        if p.life <= 0 then
            table.remove(particles, i)
        end
    end
end

-- Compatibility helper for callers that have one simulation step per render
-- frame. Fixed-clock callers consume each tick then animate once per render.
---@param s MatchState
---@param dt number
function effects.update(s, dt)
    effects.consume(s)
    effects.tick(dt)
end

-- Ball trail. Draw under the ball (call before the depth-sorted entities).
-- Heat (sample-time ball speed) drives size, warmth and brightness: a tap-pass
-- leaves a faint wisp, a charged shot a hot comet tail the bloom pass lights up.
---@param project fun(wx: number, wy: number): number, number, number
function effects.draw_trail(project)
    for _, t in ipairs(trail) do
        local sx, sy, scale = project(t.x, t.y)
        local a = t.life / t.max
        local heat = t.heat or 0
        love.graphics.setColor(1, 0.95 - 0.35 * heat, 0.7 - 0.45 * heat, a * (0.35 + 0.4 * heat))
        local r = (2.5 + 3 * a) * (1 + 0.8 * heat) * scale
        love.graphics.circle("fill", sx, sy - (BALL_LIFT + (t.z or 0)) * scale, r)
    end
end

-- Bursts and rings. Draw on top (call after the entities).
---@param project fun(wx: number, wy: number): number, number, number
function effects.draw_over(project)
    for _, p in ipairs(particles) do
        local sx, sy, scale = project(p.x, p.y)
        local t = p.life / p.max
        local c = p.color
        if p.kind == "ring" then
            -- Expands as it fades.
            local rad = p.size * scale * (1.0 + (1 - t) * 0.6)
            love.graphics.setColor(c[1], c[2], c[3], t * 0.8)
            love.graphics.setLineWidth(math.max(1, 2 * scale))
            love.graphics.circle("line", sx, sy - BALL_LIFT * scale, rad)
        elseif p.kind == "glyph" then
            local radius = p.size * scale * (0.85 + 0.15 * t)
            local x = sx
            local y = sy - BALL_LIFT * scale
            love.graphics.setColor(c[1], c[2], c[3], t)
            love.graphics.setLineWidth(math.max(1, 2 * scale))
            if p.glyph == "chevron" then
                love.graphics.line(x - radius, y + radius * 0.5, x, y - radius * 0.5)
                love.graphics.line(x, y - radius * 0.5, x + radius, y + radius * 0.5)
            elseif p.glyph == "diamond" or p.glyph == "hollow_diamond" then
                love.graphics.polygon(
                    "line",
                    x,
                    y - radius,
                    x + radius,
                    y,
                    x,
                    y + radius,
                    x - radius,
                    y
                )
            elseif p.glyph == "shield" then
                love.graphics.arc("line", x, y, radius, math.rad(190), math.rad(350))
                love.graphics.line(x - radius, y - radius * 0.2, x, y + radius)
                love.graphics.line(x, y + radius, x + radius, y - radius * 0.2)
            elseif p.glyph == "broken_ring" then
                love.graphics.arc("line", x, y, radius, 0, math.rad(135))
                love.graphics.arc("line", x, y, radius, math.rad(180), math.rad(315))
            elseif p.glyph == "split" then
                love.graphics.line(x - radius, y - radius, x - radius * 0.2, y)
                love.graphics.line(x + radius, y + radius, x + radius * 0.2, y)
            elseif p.glyph == "burst" or p.glyph == "stagger" then
                for index = 0, 3 do
                    local angle = math.pi * index / 2
                    love.graphics.line(
                        x + math.cos(angle) * radius * 0.3,
                        y + math.sin(angle) * radius * 0.3,
                        x + math.cos(angle) * radius,
                        y + math.sin(angle) * radius
                    )
                end
            elseif p.glyph == "double_cross" then
                love.graphics.line(x - radius, y - radius, x + radius, y + radius)
                love.graphics.line(x - radius, y + radius, x + radius, y - radius)
                love.graphics.circle("line", x, y, radius * 0.55)
            else
                love.graphics.line(x - radius, y - radius, x + radius, y + radius)
                love.graphics.line(x - radius, y + radius, x + radius, y - radius)
            end
        else
            love.graphics.setColor(c[1], c[2], c[3], t)
            love.graphics.circle("fill", sx, sy - BALL_LIFT * scale, p.size * scale * t + 0.5)
        end
    end
end

return effects
