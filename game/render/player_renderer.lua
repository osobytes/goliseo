-- Procedural billboard avatar drawn entirely from love.graphics primitives
-- (no sprite sheets). The figure stands upright facing the camera; body motion
-- lives in screen space (bob, limb pump, lean) while aim/facing stays a
-- ground-plane vector. All sizes scale off `r` (the projected body radius) so
-- far players shrink with the perspective.

local renderer = {}

---@class PlayerRenderOptions
---@field facing Vec2
---@field is_keeper boolean
---@field controlled boolean
---@field dashing boolean?
---@field dive number?
---@field dive_dir Vec2?
---@field holding boolean?
---@field grab number?
---@field throw number?
---@field windup number?
---@field aerial number?
---@field aerial_style AerialStyle?
---@field aerial_outcome AerialOutcome?
---@field aerial_jump number?
---@field species_shape "round"|"broad"|"angular"|"cluster"?
---@field species_color number[]?
---@field team "home"|"away"?
---@field combat CombatPlayerPresentation?
---@field pose PlayerPoseSelection?

---@class PlayerSilhouetteProfile
---@field torso_scale number
---@field limb_scale number
---@field head_kind "round"|"broad"|"angular"|"cluster"

---@type table<string, PlayerSilhouetteProfile>
local SILHOUETTES = {
    round = { torso_scale = 1.1, limb_scale = 1, head_kind = "round" },
    broad = { torso_scale = 1.5, limb_scale = 1.22, head_kind = "broad" },
    angular = { torso_scale = 0.82, limb_scale = 0.82, head_kind = "angular" },
    cluster = { torso_scale = 0.76, limb_scale = 1, head_kind = "cluster" },
}

---@param shape "round"|"broad"|"angular"|"cluster"
---@return PlayerSilhouetteProfile
function renderer.silhouette(shape)
    return assert(SILHOUETTES[shape], "unknown player silhouette")
end

-- Global alpha multiplier for the current figure pass. Dash afterimages set this
-- below 1 to draw faded ghosts; `set` honours it so every body part fades at once
-- without threading an alpha through each call.
local alpha_mul = 1

local function set(c, a)
    love.graphics.setColor(c[1], c[2], c[3], (a or 1) * alpha_mul)
end

local function clamp(x, a, b)
    return math.max(a, math.min(b, x))
end

-- Lighten a colour toward white for joint/visor accents.
local function lighten(c, t)
    return { c[1] + (1 - c[1]) * t, c[2] + (1 - c[2]) * t, c[3] + (1 - c[3]) * t }
end

---@param cx number
---@param sh_y number
---@param hip_y number
---@param r number
---@param opts PlayerRenderOptions
local function draw_equipment(cx, sh_y, hip_y, r, opts)
    local combat = opts.combat
    if not combat or not combat.equipment_presentation_id or not combat.family_id then
        return
    end
    local fx = opts.facing and opts.facing.x or 1
    local side = fx >= 0 and 1 or -1
    local active = combat.phase == "active"
    local raised = combat.phase == "guard" or combat.phase == "aim"
    local reach = active and 1.35 or (raised and 1.05 or 0.78)
    local hand_x = cx + side * r * reach
    local hand_y = raised and sh_y + r * 0.28 or hip_y - r * 0.3
    local id = combat.equipment_presentation_id

    if id == "toy_spring_gloves" then
        local extension = active and r * 0.9 or r * 0.25
        local front_x = hand_x + side * extension
        set({ 0.92, 0.96, 1 }, 0.95)
        love.graphics.setLineWidth(math.max(1, r * 0.12))
        love.graphics.line(
            hand_x - side * r * 0.22,
            hand_y,
            hand_x - side * r * 0.08,
            hand_y - r * 0.12,
            hand_x + side * r * 0.08,
            hand_y + r * 0.12,
            front_x,
            hand_y
        )
        set({ 0.4, 0.95, 1 }, 0.98)
        love.graphics.circle("fill", front_x, hand_y, r * 0.34)
        love.graphics.circle("line", cx - side * r * 0.72, hand_y + r * 0.12, r * 0.3)
    elseif id == "medieval_heater_shield" then
        local shield_x = hand_x + side * r * 0.08
        local shield_y = raised and sh_y + r * 0.42 or hip_y - r * 0.18
        local width = r * (raised and 0.72 or 0.58)
        local height = r * (raised and 1.05 or 0.85)
        set({ 0.92, 0.72, 0.28 }, 0.96)
        love.graphics.polygon(
            "fill",
            shield_x - width,
            shield_y - height * 0.45,
            shield_x + width,
            shield_y - height * 0.45,
            shield_x + width * 0.72,
            shield_y + height * 0.45,
            shield_x,
            shield_y + height * 0.72,
            shield_x - width * 0.72,
            shield_y + height * 0.45
        )
        set({ 0.18, 0.12, 0.08 }, 0.82)
        love.graphics.setLineWidth(math.max(1, r * 0.12))
        love.graphics.line(shield_x, shield_y - height * 0.35, shield_x, shield_y + height * 0.45)
        love.graphics.line(shield_x - width * 0.52, shield_y, shield_x + width * 0.52, shield_y)
    elseif id == "scifi_pulse_blaster" then
        local muzzle_x = hand_x + side * r * (active and 1.0 or 0.68)
        local left = math.min(hand_x - side * r * 0.18, muzzle_x)
        local width = math.abs(muzzle_x - hand_x) + r * 0.25
        set({ 0.24, 0.14, 0.32 }, 0.98)
        love.graphics.rectangle("fill", left, hand_y - r * 0.22, width, r * 0.44, r * 0.1)
        set({ 1, 0.45, 0.78 }, 0.98)
        love.graphics.setLineWidth(math.max(1, r * 0.13))
        love.graphics.line(hand_x, hand_y, muzzle_x, hand_y)
        love.graphics.polygon(
            "line",
            muzzle_x,
            hand_y - r * 0.28,
            muzzle_x + side * r * 0.35,
            hand_y,
            muzzle_x,
            hand_y + r * 0.28
        )
    else
        local blade_length = active and r * 1.8 or r * 1.35
        local tip_x = hand_x + side * blade_length
        local blade_color = { 0.92, 0.96, 1 }
        local blade_width = r * 0.14
        if id == "scifi_energy_blade" then
            blade_color = { 0.45, 1, 0.9 }
            blade_width = r * 0.2
        elseif id == "toy_foam_sword" then
            blade_color = { 1, 0.55, 0.28 }
            blade_width = r * 0.34
        end
        set({ 0.2, 0.14, 0.12 }, 0.98)
        love.graphics.setLineWidth(math.max(1.5, r * 0.26))
        love.graphics.line(hand_x - side * r * 0.2, hand_y, hand_x + side * r * 0.18, hand_y)
        love.graphics.line(hand_x, hand_y - r * 0.3, hand_x, hand_y + r * 0.3)
        set(blade_color, 0.98)
        love.graphics.setLineWidth(math.max(1.5, blade_width))
        love.graphics.line(hand_x + side * r * 0.15, hand_y, tip_x, hand_y)
        if id == "toy_foam_sword" then
            love.graphics.circle("fill", tip_x, hand_y, r * 0.2)
        end
    end
end

-- Draw just the standing body (legs, arms, torso, helmet) centred on screen-x
-- `bx`, feet at `gy`. No shadow / selection ring / facing tick — those are drawn
-- once by `renderer.draw` so afterimage ghosts don't duplicate them.
---@param bx number
---@param gy number
---@param r number
---@param color number[]
---@param v PlayerView?
---@param opts PlayerRenderOptions
local function figure(bx, gy, r, color, v, opts)
    local sp = v and v.speed or 0
    local ph = v and v.phase or 0
    local run = clamp(sp / 90, 0, 1) -- 0 idle .. 1 full sprint
    local lean = v and v.lean or 0
    local accent = opts.species_color or lighten(color, 0.55)
    local shape = opts.species_shape or "round"
    local silhouette = renderer.silhouette(shape)

    local swing = math.sin(ph) -- fore/aft limb phase
    -- Whole-body bounce: a gentle idle breath plus a run bob that peaks twice
    -- per stride.
    local bounce = run * math.abs(math.sin(ph)) * r * 0.16
    local breath = (1 - run) * math.sin(ph * 0.5 + bx) * r * 0.04

    -- Wind-up back-swing: lean the whole figure opposite the facing direction.
    local wu = opts.windup or 0
    local pose_id = opts.pose and opts.pose.id or nil
    if pose_id == "combat_windup" then
        wu = math.max(wu, 1 - (opts.combat and opts.combat.phase_progress or 0) * 0.45)
    end
    local fx = opts.facing and opts.facing.x or 0
    local windup_lean = -fx * wu * r * 0.6 -- leans back opposite facing
    local aerial = opts.aerial or 0
    local aerial_style = opts.aerial_style
    local action_lean = 0
    if aerial_style == "header" then
        action_lean = fx * aerial * r * 0.45
    elseif aerial_style == "chest_control" then
        action_lean = -fx * aerial * r * 0.25
    elseif pose_id == "combat_active" then
        action_lean = fx * r * 0.65
    elseif pose_id == "combat_recovery" then
        action_lean = -fx * r * 0.2
    end

    local cx = bx + lean * r * 0.5 + windup_lean + action_lean
    local foot_y = gy
    local hip_y = gy - r * 1.35 - bounce - breath
    local sh_y = gy - r * 2.15 - bounce - breath
    local head_y = gy - r * 2.75 - bounce - breath

    local stride = run * r * 0.65
    local hip_dx = r * 0.34

    -- Legs (pump in opposite phase). Boots are chunky blocks at the feet.
    local limb_scale = silhouette.limb_scale
    love.graphics.setLineWidth(math.max(1.5, r * 0.34 * limb_scale))
    set(color, opts.is_keeper and 0.8 or 1)
    local lfx = cx - hip_dx + swing * stride
    local rfx = cx + hip_dx - swing * stride
    local lfy, rfy = foot_y, foot_y
    if aerial_style == "volley" or aerial_style == "leg_control" then
        local strike_sign = (fx >= 0) and 1 or -1
        lfx = cx + strike_sign * r * 1.45 * aerial
        lfy = foot_y - r * 0.85 * aerial
    end
    love.graphics.line(cx - hip_dx, hip_y, lfx, lfy)
    love.graphics.line(cx + hip_dx, hip_y, rfx, rfy)
    set(accent, 1)
    love.graphics.setLineWidth(math.max(1.5, r * 0.42))
    love.graphics.line(lfx - r * 0.12, lfy, lfx + r * 0.18, lfy)
    love.graphics.line(rfx - r * 0.12, rfy, rfx + r * 0.18, rfy)

    -- Arms (opposite the legs, swinging the other way).
    love.graphics.setLineWidth(math.max(1.5, r * 0.26 * limb_scale))
    set(color, opts.is_keeper and 0.8 or 1)
    if pose_id == "combat_guard" then
        love.graphics.line(cx - r * 0.5, sh_y, cx + fx * r * 0.75, sh_y + r * 0.2)
        love.graphics.line(cx + r * 0.5, sh_y, cx + fx * r * 0.95, sh_y + r * 0.5)
    elseif pose_id == "combat_active" or pose_id == "combat_aim" then
        love.graphics.line(cx - r * 0.5, sh_y, cx + fx * r * 0.7, sh_y + r * 0.35)
        love.graphics.line(cx + r * 0.5, sh_y, cx + fx * r * 1.0, sh_y + r * 0.15)
    elseif aerial_style == "chest_control" then
        love.graphics.line(cx - r * 0.5, sh_y, cx - r * (0.55 + 0.55 * aerial), sh_y + r * 0.2)
        love.graphics.line(cx + r * 0.5, sh_y, cx + r * (0.55 + 0.55 * aerial), sh_y + r * 0.2)
    else
        love.graphics.line(
            cx - r * 0.5,
            sh_y,
            cx - r * 0.55 - swing * stride * 0.6,
            hip_y + r * 0.2
        )
        love.graphics.line(
            cx + r * 0.5,
            sh_y,
            cx + r * 0.55 + swing * stride * 0.6,
            hip_y + r * 0.2
        )
    end

    -- Torso (capsule: rounded rect from hips to shoulders).
    local tw = r * silhouette.torso_scale
    set(color, opts.is_keeper and 0.7 or 1)
    if shape == "angular" then
        love.graphics.polygon(
            "fill",
            cx - tw * 0.62,
            sh_y,
            cx + tw * 0.62,
            sh_y,
            cx + tw * 0.38,
            hip_y,
            cx - tw * 0.38,
            hip_y
        )
    else
        local roundness = shape == "broad" and r * 0.18 or r * 0.45
        love.graphics.rectangle("fill", cx - tw / 2, sh_y, tw, hip_y - sh_y, roundness, roundness)
    end
    -- Team joint band across the chest.
    set(accent, 0.9)
    love.graphics.setLineWidth(math.max(1, r * 0.18))
    local band_y = sh_y + (hip_y - sh_y) * 0.4
    if opts.team == "away" then
        love.graphics.line(cx - tw / 2, band_y, cx - tw * 0.12, band_y)
        love.graphics.line(cx + tw * 0.12, band_y, cx + tw / 2, band_y)
    else
        love.graphics.line(cx - tw / 2, band_y, cx + tw / 2, band_y)
    end

    -- Helmet + visor. The visor sits on the side the player aims toward (its
    -- ground-plane x), giving a readable facing cue without rotating the body.
    local hr = r * 0.62
    set(color, 1)
    if shape == "broad" then
        love.graphics.rectangle(
            "fill",
            cx - hr * 1.15,
            head_y - hr * 0.65,
            hr * 2.3,
            hr * 1.3,
            hr * 0.25,
            hr * 0.25
        )
    elseif shape == "angular" then
        love.graphics.polygon(
            "fill",
            cx,
            head_y - hr * 1.35,
            cx + hr * 0.9,
            head_y + hr * 0.7,
            cx,
            head_y + hr,
            cx - hr * 0.9,
            head_y + hr * 0.7
        )
    elseif shape == "cluster" then
        love.graphics.circle("fill", cx - hr * 0.7, head_y + hr * 0.2, hr * 0.68)
        love.graphics.circle("fill", cx + hr * 0.7, head_y + hr * 0.2, hr * 0.68)
        love.graphics.circle("fill", cx, head_y - hr * 0.55, hr * 0.72)
    else
        love.graphics.circle("fill", cx, head_y, hr)
    end
    set(accent, 0.95)
    if shape == "round" then
        love.graphics.arc(
            "fill",
            cx,
            head_y,
            hr * 0.82,
            math.rad(-40) + fx * 0.6,
            math.rad(90) + fx * 0.6
        )
    elseif shape == "cluster" then
        love.graphics.circle("fill", cx + fx * hr * 0.3, head_y, hr * 0.22)
    else
        love.graphics.line(
            cx - hr * 0.45,
            head_y + fx * hr * 0.14,
            cx + hr * 0.45,
            head_y + fx * hr * 0.14
        )
    end

    draw_equipment(cx, sh_y, hip_y, r, opts)
end

-- Draw one player.
---@param sx number  -- screen x of the ground point (feet)
---@param gy number  -- screen y of the ground point (feet)
---@param r number   -- projected body radius (px)
---@param color number[]
---@param v PlayerView?  -- nil = idle fallback
---@param opts PlayerRenderOptions
function renderer.draw(sx, gy, r, color, v, opts)
    local pose_id = opts.pose and opts.pose.id or nil
    -- Ground shadow (kept here so it tracks the figure).
    love.graphics.setColor(0, 0, 0, 0.35)
    love.graphics.ellipse("fill", sx, gy, r * 1.15, r * 0.5)

    -- Selection is geometry-first: two rings and a downward chevron remain
    -- readable in grayscale, during aerial poses, and during keeper dives.
    if opts.controlled then
        set({ 1, 1, 1 }, 0.92)
        love.graphics.setLineWidth(math.max(1, r * 0.12))
        love.graphics.ellipse("line", sx, gy, r * 1.25, r * 0.6)
        love.graphics.ellipse("line", sx, gy, r * 1.48, r * 0.72)
        love.graphics.polygon(
            "fill",
            sx,
            gy - r * 3.75,
            sx - r * 0.42,
            gy - r * 4.25,
            sx + r * 0.42,
            gy - r * 4.25
        )
    end

    -- Keeper dive: pivot the whole body at the feet toward the dive side and
    -- shove it laterally, so the figure lunges horizontally for the save. `dive`
    -- is 0..1 progress (1 = just launched). Reuses figure() under a transform.
    if
        (pose_id == "keeper_dive" or (pose_id == nil and opts.dive and opts.dive > 0))
        and opts.dive_dir
    then
        local d = opts.dive_dir ---@type Vec2
        local sign = (d.x >= 0) and 1 or -1
        local angle = sign * math.rad(72) * opts.dive
        love.graphics.push()
        love.graphics.translate(sx + d.x * r * 1.6 * opts.dive, gy)
        love.graphics.rotate(angle)
        love.graphics.translate(-sx, -gy)
        figure(sx, gy, r, color, v, opts)
        love.graphics.pop()
        -- Facing tick still helps read the dive direction.
        return
    end

    -- Aerial actions use the ground point for sorting/shadow but lift the
    -- billboard. A bicycle rotates the whole figure into a readable overhead
    -- silhouette; other styles pose individual limbs in figure().
    if
        (
            pose_id == "aerial_bicycle"
            or pose_id == "aerial_action"
            or (pose_id == nil and opts.aerial and opts.aerial > 0)
        ) and opts.aerial_style
    then
        local amount = clamp(opts.aerial, 0, 1)
        local lift = r * (0.35 + 1.65 * (opts.aerial_jump or 0)) * amount
        if opts.aerial_style == "bicycle" then
            local fx = (opts.facing and opts.facing.x) or 1
            local sign = (fx >= 0) and -1 or 1
            love.graphics.push()
            love.graphics.translate(sx, gy - lift - r * 0.7)
            love.graphics.rotate(sign * math.rad(78) * amount)
            love.graphics.translate(-sx, -gy)
            figure(sx, gy, r, color, v, opts)
            love.graphics.pop()
        else
            figure(sx, gy - lift, r, color, v, opts)
        end
        return
    end

    if pose_id == "combat_knockback" then
        love.graphics.push()
        love.graphics.translate(sx, gy - r * 0.45)
        love.graphics.rotate(math.rad(68))
        love.graphics.translate(-sx, -gy)
        figure(sx, gy, r, color, v, opts)
        love.graphics.pop()
        return
    elseif pose_id == "combat_stagger" then
        love.graphics.push()
        love.graphics.translate(sx, gy + r * 0.28)
        love.graphics.rotate(math.rad(8))
        love.graphics.translate(-sx, -gy)
        figure(sx, gy, r, color, v, opts)
        love.graphics.pop()
        return
    end

    -- Dash afterimage: faded copies trailing backward along the facing direction
    -- (which equals the move direction during a dash). Drawn before the figure so
    -- the solid body sits on top of its own smear.
    if opts.dashing and opts.facing then
        local fx, fy = opts.facing.x, opts.facing.y
        for n = 1, 2 do
            local k = n * 0.6
            alpha_mul = 0.24 / n
            figure(sx - fx * r * 1.1 * k, gy - fy * r * 0.55 * k, r, color, v, opts)
        end
        alpha_mul = 1
    end

    figure(sx, gy, r, color, v, opts)

    -- Keeper ball handling: cradle the ball in raised hands while holding it (lower
    -- while gathering), or thrust the arms forward on release (no ball — it's away).
    local fx = (opts.facing and opts.facing.x) or 0
    local sh_y = gy - r * 2.15
    if opts.holding then
        local gathering = (opts.grab or 0) > 0
        local hy = gathering and (gy - r * 1.6) or (gy - r * 2.1)
        local hx = sx + fx * r * 0.35
        set(lighten(color, 0.55), 0.95)
        love.graphics.setLineWidth(math.max(1.5, r * 0.26))
        love.graphics.line(sx - r * 0.5, sh_y, hx - r * 0.35, hy)
        love.graphics.line(sx + r * 0.5, sh_y, hx + r * 0.35, hy)
        love.graphics.setColor(1, 0.95, 0.7)
        love.graphics.circle("fill", hx, hy, r * 0.5)
    elseif (opts.throw or 0) > 0 then
        local hx = sx + fx * r * (0.6 + opts.throw * 0.8)
        set(lighten(color, 0.55), 0.9)
        love.graphics.setLineWidth(math.max(1.5, r * 0.26))
        love.graphics.line(sx - r * 0.5, sh_y, hx, gy - r * 1.9)
        love.graphics.line(sx + r * 0.5, sh_y, hx, gy - r * 1.9)
    end

    -- Ground-plane facing tick (kept from the old renderer as a clear aim cue).
    if opts.facing then
        set({ 1, 1, 1 }, 0.7)
        love.graphics.setLineWidth(math.max(1, r * 0.12))
        love.graphics.line(sx, gy, sx + opts.facing.x * r * 1.1, gy + opts.facing.y * r * 0.6)
    end
end

return renderer
