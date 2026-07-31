-- Keyframed animation clips, authored directly in Lua.
--
-- A clip is a list of keyframes sorted by time. Each keyframe holds a *sparse*
-- pose: `rot` gives bone rotations in degrees, `move` gives additive
-- translations. A bone a clip never mentions stays at its rest transform.
--
-- Sign conventions (the warrior faces +Z):
--   rot.x  The sign depends on which way the bone POINTS, which is the single
--          easiest thing to get wrong here:
--            * Limbs hang DOWN (-Y), so negative x swings them FORWARD.
--              A knee only bends backward, so shin.x stays positive; an elbow
--              only bends forward, so forearm.x stays negative.
--            * The spine chain points UP (+Y), so the sign INVERTS:
--              POSITIVE x on spine / chest / neck leans the torso FORWARD.
--          Verified by rendering, not assumed -- an early charge pose leaned
--          backwards for exactly this reason.
--   rot.y negative -> twists toward his right;  positive -> toward his left.
--   rot.z          -> splays a limb away from the body's midline.
--
-- `loop` and `root_motion` are properties of the CLIP, not assumptions baked
-- into the player. Every clip here is in-place: simulation owns position,
-- facing and contact, per #101.
--
-- Channels are interpolated independently and eased with smoothstep, which is
-- enough to keep a 4-6 key clip from looking like straight-line lerp. Euler
-- angles are interpolated component-wise: fine here because no joint passes
-- through a gimbal-locking orientation.

local quat = require("core.quat")

local clips = {}

-- Shallow-merges `overrides` over `base` into a fresh table, so each keyframe
-- can state only what differs from the warrior's guard stance.
---@param base table
---@param overrides table
---@return table
local function stance(base, overrides)
    local out = {}
    for k, v in pairs(base) do
        out[k] = v
    end
    for k, v in pairs(overrides) do
        out[k] = v
    end
    return out
end

-- The combat guard every clip returns to: elbows bent, sword hand forward,
-- shield arm carrying the shield at his left side.
--
-- The sword's blade angle is simply (upper_arm.R.x + forearm.R.x + 145) degrees
-- swept from straight up, because the hand attachment adds a fixed 145 to
-- the arm chain. That single number is what the swing clip animates: -35 at the
-- windup (blade cocked overhead and back) through to +120 (blade driven down
-- and forward).
local GUARD = {
    ["upper_arm.R"] = { -18, 0, 6 },
    ["forearm.R"] = { -72, 0, 0 },
    ["hand.R"] = { 0, -8, 0 },
    ["upper_arm.L"] = { -12, 0, 18 },
    ["forearm.L"] = { -58, 0, 0 },
    ["thigh.R"] = { -4, 0, 0 },
    ["thigh.L"] = { 6, 0, 0 },
    ["shin.L"] = { 6, 0, 0 },
}

-- ---------------------------------------------------------------------------
-- Clip 1: IDLE -- breathing and a slow weight shift from one foot to the other
-- ---------------------------------------------------------------------------
local IDLE = {
    name = "idle",
    loop = true,
    root_motion = false, -- in-place; simulation owns displacement (#101)
    duration = 3.4,
    keys = {
        {
            t = 0.0,
            rot = stance(GUARD, {}),
            move = {},
        },
        {
            t = 0.85,
            rot = stance(GUARD, {
                spine = { -1.5, 0, 0 },
                chest = { -2.5, 0, 0 },
                head = { -2, 3, 0 },
                ["hips"] = { 0, 0, 1.5 },
                ["upper_arm.R"] = { -16, 0, 7 },
                ["forearm.R"] = { -70, 0, 0 },
                ["upper_arm.L"] = { -10, 0, 19 },
            }),
            move = { root = { 0.015, 0.012, 0 } },
        },
        {
            t = 1.70,
            rot = stance(GUARD, {
                spine = { 0.5, 0, 0 },
                chest = { 1.0, 0, 0 },
                head = { 1.5, -5, 0 },
            }),
            move = { root = { 0, 0, 0 } },
        },
        {
            t = 2.55,
            rot = stance(GUARD, {
                spine = { -1.2, 0, 0 },
                chest = { -2.0, 0, 0 },
                head = { -1.5, -4, 0 },
                ["hips"] = { 0, 0, -1.5 },
                ["upper_arm.R"] = { -20, 0, 5 },
                ["forearm.R"] = { -74, 0, 0 },
                ["upper_arm.L"] = { -14, 0, 17 },
            }),
            move = { root = { -0.015, 0.010, 0 } },
        },
        {
            t = 3.4,
            rot = stance(GUARD, {}),
            move = {},
        },
    },
}

-- ---------------------------------------------------------------------------
-- Clip 2: WALK -- a two-step cycle, contact / passing / contact / passing
-- ---------------------------------------------------------------------------
local function step(thigh_f, shin_f, foot_f, toe_f, thigh_b, shin_b, foot_b, toe_b, extra)
    local t = {
        ["thigh.R"] = { thigh_f, 0, -2 },
        ["shin.R"] = { shin_f, 0, 0 },
        ["foot.R"] = { foot_f, 0, 0 },
        ["toe.R"] = { toe_f, 0, 0 },
        ["thigh.L"] = { thigh_b, 0, 2 },
        ["shin.L"] = { shin_b, 0, 0 },
        ["foot.L"] = { foot_b, 0, 0 },
        ["toe.L"] = { toe_b, 0, 0 },
    }
    for k, v in pairs(extra or {}) do
        t[k] = v
    end
    return t
end

-- Mirrors a leg pose left/right, so a cycle is authored once and the opposite
-- step comes for free -- and cannot drift out of sync with its own mirror.
local function mirror(pose)
    local out = {}
    for name, v in pairs(pose) do
        local base, side = name:match("^(.*)%.([LR])$")
        local swapped = base and (base .. "." .. (side == "R" and "L" or "R")) or name
        -- Y and Z are the twist and splay axes, so both flip with the side.
        out[swapped] = { v[1], -v[2], -v[3] }
    end
    return out
end

-- ---------------------------------------------------------------------------
-- Clip 2: WALK
--
-- Timings and joint ranges are matched to reference locomotion: a 0.80 s cycle
-- with the knee peaking near 70 degrees through the passing pose. The previous
-- version ran a 0.77 s cycle with a 46 degree knee at every speed including a
-- sprint, which is what made it read as short, fast, robotic steps.
-- ---------------------------------------------------------------------------
local WALK = {
    name = "walk",
    loop = true,
    root_motion = false,
    -- World units covered by one full cycle. Phase advances with distance, so
    -- this is what keeps the feet planted rather than sliding.
    stride = 130,
    duration = 0.80,
    keys = {
        {
            t = 0.0,
            rot = stance(
                GUARD,
                step(-26, 6, 10, -16, 22, 30, -16, -30, {
                    hips = { 0, -5, 2 },
                    chest = { 3, 5, 0 },
                })
            ),
            move = { root = { 0, 0, 0 } },
        },
        {
            t = 0.20,
            rot = stance(
                GUARD,
                step(6, 12, -6, -4, -10, 70, -16, -12, {
                    hips = { 0, 0, 0 },
                    chest = { 3, 0, 0 },
                })
            ),
            move = { root = { 0, 0.036, 0 } },
        },
        {
            t = 0.40,
            rot = stance(GUARD, mirror(step(-26, 6, 10, -16, 22, 30, -16, -30, {}))),
            move = { root = { 0, 0, 0 } },
        },
        {
            t = 0.60,
            rot = stance(GUARD, mirror(step(6, 12, -6, -4, -10, 70, -16, -12, {}))),
            move = { root = { 0, 0.036, 0 } },
        },
        {
            t = 0.80,
            rot = stance(
                GUARD,
                step(-26, 6, 10, -16, 22, 30, -16, -30, {
                    hips = { 0, -5, 2 },
                    chest = { 3, 5, 0 },
                })
            ),
            move = { root = { 0, 0, 0 } },
        },
    },
}

-- ---------------------------------------------------------------------------
-- Clip 3: RUN
--
-- A run is not a fast walk. Against the same reference: a 0.60 s cycle, the
-- knee peaking near 120 degrees, a longer thigh swing, a forward torso lean and
-- a flight phase the walk does not have. Playing a walk faster gets none of
-- that, which is the whole reason this is a separate clip.
-- ---------------------------------------------------------------------------
local RUN = {
    name = "run",
    loop = true,
    root_motion = false,
    stride = 285,
    duration = 0.60,
    keys = {
        {
            t = 0.0,
            rot = stance(
                GUARD,
                step(-42, 18, 14, -22, 34, 78, -24, -38, {
                    hips = { 0, -7, 3 },
                    spine = { 9, 0, 0 },
                    chest = { 4, 7, 0 },
                })
            ),
            move = { root = { 0, 0, 0 } },
        },
        {
            t = 0.15,
            rot = stance(
                GUARD,
                step(12, 34, -10, -8, -22, 118, -18, -16, {
                    hips = { 0, 0, 0 },
                    spine = { 11, 0, 0 },
                    chest = { 4, 0, 0 },
                })
            ),
            move = { root = { 0, 0.058, 0 } },
        },
        {
            t = 0.30,
            rot = stance(
                GUARD,
                mirror(step(-42, 18, 14, -22, 34, 78, -24, -38, {
                    spine = { 9, 0, 0 },
                }))
            ),
            move = { root = { 0, 0, 0 } },
        },
        {
            t = 0.45,
            rot = stance(
                GUARD,
                mirror(step(12, 34, -10, -8, -22, 118, -18, -16, {
                    spine = { 11, 0, 0 },
                }))
            ),
            move = { root = { 0, 0.058, 0 } },
        },
        {
            t = 0.60,
            rot = stance(
                GUARD,
                step(-42, 18, 14, -22, 34, 78, -24, -38, {
                    hips = { 0, -7, 3 },
                    spine = { 9, 0, 0 },
                    chest = { 4, 7, 0 },
                })
            ),
            move = { root = { 0, 0, 0 } },
        },
    },
}

-- ---------------------------------------------------------------------------
-- Clip 3: SWING -- an overhead light_melee strike, with a long recovery
-- ---------------------------------------------------------------------------
local SWING = {
    name = "swing",
    loop = false,
    root_motion = false, -- in-place; simulation owns displacement (#101)
    fallback = "idle",
    duration = 1.4,
    keys = {
        { -- guard
            t = 0.0,
            rot = stance(GUARD, {}),
            move = {},
        },
        { -- windup: torso coils to his right, sword arm lifts overhead and back
            t = 0.32,
            rot = stance(GUARD, {
                spine = { -7, -10, 0 },
                chest = { -4, -24, 0 },
                head = { 7, -12, 0 },
                ["hips"] = { 0, -8, 0 },
                ["upper_arm.R"] = { -125, 0, 16 },
                ["forearm.R"] = { -55, 0, 0 },
                ["hand.R"] = { 0, -14, 0 },
                ["upper_arm.L"] = { -14, 0, 20 },
                ["forearm.L"] = { -58, 0, 0 },
                ["thigh.R"] = { 8, 0, -2 },
                ["thigh.L"] = { -6, 0, 2 },
            }),
            move = { root = { 0, 0.02, -0.03 } },
        },
        { -- strike: everything uncoils forward into a braced lunge
            t = 0.50,
            rot = stance(GUARD, {
                spine = { 15, 10, 0 },
                chest = { 10, 22, 0 },
                head = { -11, 10, 0 },
                ["hips"] = { 0, 10, 0 },
                ["upper_arm.R"] = { -45, 0, 6 },
                ["forearm.R"] = { -15, 0, 0 },
                ["hand.R"] = { 0, -4, 0 },
                ["upper_arm.L"] = { -18, 0, 20 },
                ["forearm.L"] = { -60, 0, 0 },
                ["thigh.L"] = { -30, 0, 2 },
                ["shin.L"] = { 10, 0, 0 },
                ["thigh.R"] = { 18, 0, -2 },
                ["shin.R"] = { 24, 0, 0 },
                ["foot.R"] = { -14, 0, 0 },
            }),
            move = { root = { 0, -0.025, 0.05 } },
        },
        { -- follow-through
            t = 0.66,
            rot = stance(GUARD, {
                spine = { 19, 12, 0 },
                chest = { 13, 26, 0 },
                head = { -13, 12, 0 },
                ["hips"] = { 0, 12, 0 },
                ["upper_arm.R"] = { -22, 0, 4 },
                ["forearm.R"] = { -3, 0, 0 },
                ["upper_arm.L"] = { -16, 0, 22 },
                ["forearm.L"] = { -58, 0, 0 },
                ["thigh.L"] = { -32, 0, 2 },
                ["shin.L"] = { 14, 0, 0 },
                ["thigh.R"] = { 20, 0, -2 },
                ["shin.R"] = { 28, 0, 0 },
                ["foot.R"] = { -16, 0, 0 },
            }),
            move = { root = { 0, -0.032, 0.06 } },
        },
        { -- settle back toward the guard
            t = 0.98,
            rot = stance(GUARD, {
                spine = { 7, 4, 0 },
                chest = { 5, 10, 0 },
                ["upper_arm.R"] = { -30, 0, 6 },
                ["forearm.R"] = { -55, 0, 0 },
                ["thigh.L"] = { -10, 0, 2 },
            }),
            move = { root = { 0, -0.01, 0.02 } },
        },
        { -- loop
            t = 1.4,
            rot = stance(GUARD, {}),
            move = {},
        },
    },
}

-- ---------------------------------------------------------------------------
-- Clip 4: CHARGE -- a held attack pose, not a swing.
--
-- Design note: in a fast soccer game, an attack that requires stopping to swing
-- fights the pace. A CHARGE instead fuses the attack into the run: the weapon
-- is levelled and held, the shield comes up, the player commits forward, and
-- the "attack" is the CONTACT rather than an animation event.
--
-- That is much cheaper to build as well. A swing needs windup/active/recovery
-- authored against every locomotion state it can start from; a charge is one
-- looping upper-body hold plus one impact one-shot. It also reads as
-- commitment, which makes it telegraphed and counterable -- good for the game,
-- not just for the budget.
--
-- Authored as an upper-body OVERLAY: it deliberately says nothing about the
-- legs, so whatever locomotion is playing underneath keeps the stride.
-- ---------------------------------------------------------------------------
local CHARGE = {
    name = "charge",
    loop = true,
    root_motion = false,
    duration = 0.70,
    keys = {
        {
            t = 0.0,
            rot = {
                spine = { 16, 0, 0 },
                chest = { 10, -6, 0 },
                head = { -17, 4, 0 },
                ["upper_arm.R"] = { -66, 0, 10 },
                ["forearm.R"] = { -22, 0, 0 },
                ["hand.R"] = { 0, -6, 0 },
                ["upper_arm.L"] = { -34, 0, 22 },
                ["forearm.L"] = { -74, 0, 0 },
            },
            move = {},
        },
        {
            t = 0.35,
            rot = {
                spine = { 19, 0, 0 },
                chest = { 12, 6, 0 },
                head = { -20, -4, 0 },
                ["upper_arm.R"] = { -62, 0, 8 },
                ["forearm.R"] = { -26, 0, 0 },
                ["hand.R"] = { 0, -6, 0 },
                ["upper_arm.L"] = { -30, 0, 24 },
                ["forearm.L"] = { -70, 0, 0 },
            },
            move = {},
        },
        {
            t = 0.70,
            rot = {
                spine = { 16, 0, 0 },
                chest = { 10, -6, 0 },
                head = { -17, 4, 0 },
                ["upper_arm.R"] = { -66, 0, 10 },
                ["forearm.R"] = { -22, 0, 0 },
                ["hand.R"] = { 0, -6, 0 },
                ["upper_arm.L"] = { -34, 0, 22 },
                ["forearm.L"] = { -74, 0, 0 },
            },
            move = {},
        },
    },
}

clips.ORDER = { IDLE, WALK, SWING }
clips.RUN = RUN
clips.CHARGE = CHARGE

-- Precomputes the set of bones a clip touches, so the sampler knows which
-- channels to emit without re-scanning every keyframe each frame.
local function prepare(clip)
    assert(clip.keys[1].t == 0, clip.name .. ": first key must be at t = 0")
    assert(clip.loop ~= nil, clip.name .. ": must declare loop")
    assert(clip.root_motion == false, clip.name .. ": clips are in-place (#101)")
    -- A one-shot has to say where playback lands when it finishes, or the
    -- controller has nothing to fall back to.
    assert(clip.loop or clip.fallback, clip.name .. ": one-shot needs a fallback")
    assert(
        clip.keys[#clip.keys].t == clip.duration,
        clip.name .. ": last key must be at the duration"
    )
    clip.rot_bones, clip.move_bones = {}, {}
    -- Bake authored Euler into quaternions once. Everything downstream --
    -- sampling, layering, crossfading -- works on quaternions; humans keep
    -- editing degrees above.
    for _, key in ipairs(clip.keys) do
        key.q = {}
        for name, e in pairs(key.rot or {}) do
            key.q[name] = quat.fromEuler(math.rad(e[1]), math.rad(e[2]), math.rad(e[3]))
        end
    end
    for _, key in ipairs(clip.keys) do
        for name in pairs(key.rot or {}) do
            clip.rot_bones[name] = true
        end
        for name in pairs(key.move or {}) do
            clip.move_bones[name] = true
        end
    end
end

for _, clip in ipairs(clips.ORDER) do
    prepare(clip)
end
prepare(CHARGE)
prepare(RUN)

local ZERO = { 0, 0, 0 }
local IDENTITY = quat.identity()

local function lerp3(a, b, u)
    return {
        a[1] + (b[1] - a[1]) * u,
        a[2] + (b[2] - a[2]) * u,
        a[3] + (b[3] - a[3]) * u,
    }
end

-- Composes an overlay layer onto a base pose through a bone mask.
--
-- Override semantics, which is what a masked layer means: for every bone the
-- mask owns, the result comes from the OVERLAY -- and if the overlay clip never
-- touches that bone, it goes to rest rather than leaking the base pose through.
-- Anything outside the mask is untouched. `weight` fades the layer in and out,
-- which is how an action blends over a run instead of popping onto it.
---@param base table    -- pose from the locomotion layer
---@param overlay table -- pose from the action layer
---@param mask table    -- set of bone names the overlay owns
---@param weight number -- 0 = base only, 1 = overlay fully applied
---@return table
function clips.layer(base, overlay, mask, weight)
    local out = { rot = {}, move = {} }
    for name, value in pairs(base.rot) do
        out.rot[name] = value
    end
    for name, value in pairs(base.move) do
        out.move[name] = value
    end
    for name in pairs(mask) do
        -- Rotations blend on the short arc; translations are linear, which is
        -- correct -- only rotation has the wrap-around problem.
        if overlay.rot[name] or base.rot[name] then
            out.rot[name] =
                quat.slerp(base.rot[name] or IDENTITY, overlay.rot[name] or IDENTITY, weight)
        end
        if overlay.move[name] or base.move[name] then
            out.move[name] = lerp3(base.move[name] or ZERO, overlay.move[name] or ZERO, weight)
        end
    end
    return out
end

-- Samples a clip at `time`, wrapping so every clip loops seamlessly.
---@param clip table
---@param time number
---@return table  -- { rot = {...}, move = {...} }, ready for skeleton.apply
function clips.sample(clip, time)
    local t = time % clip.duration
    local keys = clip.keys

    local i = 1
    while i < #keys - 1 and keys[i + 1].t <= t do
        i = i + 1
    end
    local a, b = keys[i], keys[i + 1]

    local span = b.t - a.t
    local u = span > 1e-6 and (t - a.t) / span or 0
    u = u * u * (3 - 2 * u) -- smoothstep ease in/out

    local pose = { rot = {}, move = {} }
    for name in pairs(clip.rot_bones) do
        pose.rot[name] = quat.slerp(a.q[name] or IDENTITY, b.q[name] or IDENTITY, u)
    end
    for name in pairs(clip.move_bones) do
        pose.move[name] = lerp3((a.move or {})[name] or ZERO, (b.move or {})[name] or ZERO, u)
    end
    return pose
end

return clips
