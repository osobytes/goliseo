-- Bone masks: which part of the body a layer is allowed to drive.
--
-- GOLISEO is a fast soccer game, so a player striking, guarding, aiming or
-- reaching while still running is the NORMAL case, not an edge case. Authoring
-- a full-body clip per (locomotion x action) pair is combinatorial: 12
-- locomotion states x 10 actions is 120 clips that all have to be re-authored
-- when the run cycle changes.
--
-- The alternative is one clip per locomotion state and one clip per action,
-- composed at runtime with a mask deciding which bones each layer owns. 12 + 10
-- instead of 120, and the run cycle stays a single source of truth.
--
-- A mask is just a set of bone names, so it costs nothing to define -- but it
-- does mean bone naming is an interface, not an internal detail.
--
-- IMPORTANT: sockets must be inside any mask that includes the hand they hang
-- off. A socket left out of the mask keeps the base layer's transform while the
-- arm follows the overlay, and the weapon visibly detaches from the fist.

local masks = {}

local function set(list)
    local out = {}
    for _, name in ipairs(list) do
        out[name] = true
    end
    return out
end

local function sided(prefixes)
    local out = {}
    for _, prefix in ipairs(prefixes) do
        out[#out + 1] = prefix .. ".L"
        out[#out + 1] = prefix .. ".R"
    end
    return out
end

local function merge(...)
    local out = {}
    for i = 1, select("#", ...) do
        for name in pairs((select(i, ...))) do
            out[name] = true
        end
    end
    return out
end

-- Everything from the spine up. `hips` is deliberately NOT here: it is the
-- root of both halves and belongs to whatever drives locomotion, or the
-- character's stride starts fighting the action.
masks.UPPER_BODY = merge(
    set({ "spine", "chest", "neck", "head", "socket_ball" }),
    set(sided({ "shoulder", "upper_arm", "forearm", "hand", "socket_hand" })),
    set({ "socket_shield.L" })
)

-- Arms only: an action that must not disturb the torso's stride lean.
masks.ARMS = merge(
    set(sided({ "shoulder", "upper_arm", "forearm", "hand", "socket_hand" })),
    set({ "socket_shield.L" })
)

-- The sword arm alone -- a strike that leaves the shield arm holding guard.
masks.ARM_R = set({ "shoulder.R", "upper_arm.R", "forearm.R", "hand.R", "socket_hand.R" })

masks.LOWER_BODY = merge(set({ "hips" }), set(sided({ "thigh", "shin", "foot", "toe" })))

-- Single-leg masks, and the reason the naive upper/lower split is not enough
-- for this game: a PASS or a SHOT is a lower-body action performed while
-- running. It cannot be an upper-body overlay, and making it a full-body clip
-- would throw away the stride. Masking one leg plus the spine lets a player
-- strike the ball mid-run while the other leg keeps planting.
masks.KICK_R = merge(set({ "spine", "hips" }), set({ "thigh.R", "shin.R", "foot.R", "toe.R" }))
masks.KICK_L = merge(set({ "spine", "hips" }), set({ "thigh.L", "shin.L", "foot.L", "toe.L" }))

masks.FULL_BODY = merge(masks.UPPER_BODY, masks.LOWER_BODY, set({ "root" }))

return masks
