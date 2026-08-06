-- Pose level-of-detail policy (#394): which characters may reuse last frame's
-- evaluated pose, and when.
--
-- Why this exists: under the wasm-hosted interpreter the per-character draw
-- cost is CPU-side Lua, and the measured split (issue #394) puts ~88% of it in
-- pose evaluation -- clips.sample/clips.layer, skeleton.apply and the bone-row
-- build -- against ~9% in the whole uniform/draw submission. A character that
-- stands roughly 60 px tall under the whole-pitch camera does not need its
-- limbs re-evaluated 60 times a second; re-posing it every other frame is
-- invisible at that size while halving the dominant cost. Placement (screen
-- position, yaw, palette) stays per-frame -- only the LIMB POSE is held.
--
-- This module is the pure policy AND the cache-entry lifecycle: interval
-- selection, the refresh schedule, and the per-entry bookkeeping (`step`).
-- player_renderer_3d owns only the cache table itself and the GPU-facing rows.
-- Keeping all decisions here, with zero love.* calls, keeps them testable
-- headless (AGENTS.md section 9) and keeps the contract visible:
--
--   * DETERMINISTIC IN ITS INPUTS. The schedule is a function of a
--     per-character draw counter and a stagger index, never the wall clock, so
--     the same draw sequence always refreshes on the same frames.
--   * PRESENTATION-ONLY. Nothing here reads or writes simulation state; the
--     inputs are the same PlayerRenderOptions the renderer already receives.
--   * CONSERVATIVE. Anything the eye tracks -- the controlled player, dives,
--     aerials, throws, combat, any pose the simulation drives through its own
--     timers -- refreshes at full rate. Only the plain gait/idle family is
--     ever held, and only while the character is small on screen.

local pose_lod = {}

-- Characters taller than this on screen re-evaluate every frame: at close-up
-- sizes a held gait frame starts to read as a hitch. The whole-pitch camera
-- draws characters at roughly 60 px, comfortably under it; a follow camera
-- zooming in pushes past it and turns the LOD off by itself.
pose_lod.FULL_RATE_HEIGHT_PX = 120

-- Refresh interval for held characters: every other frame. Halves the
-- dominant per-character cost while keeping limb animation at 30 Hz on a
-- ~60 px figure, which is beneath notice. Deliberately NOT deeper -- a third
-- frame of hold starts to stutter the gait cycle.
pose_lod.REDUCED_INTERVAL = 2

-- The pose ids that may be held: the plain locomotion/idle family, i.e. the
-- ids player_renderer_3d resolves to the looping gait clips. Everything else
-- -- action_pose's whole-body ids (dives, aerials, knockback, stumble), the
-- combat stances, anything new and unknown -- refreshes at full rate, so an
-- unlisted id can never be degraded by accident. Public so the spec can pin
-- the set exhaustively: a dropped or mistyped entry must fail the suite.
---@type table<string, boolean>
pose_lod.RELAXED_POSE = {
    locomotion = true,
    contain = true,
    run_telegraph = true,
    fatigue = true,
    keeper_shuffle = true,
    settle = true,
    kick_follow = true,
}

-- How often one character's pose must be re-evaluated, in frames. 1 means
-- every frame (no hold). Pure: same options and size, same answer.
---@param opts PlayerRenderOptions
---@param height_px number  -- the character's on-screen height in pixels
---@return integer
function pose_lod.interval(opts, height_px)
    if height_px > pose_lod.FULL_RATE_HEIGHT_PX then
        return 1
    end
    -- The player being steered is the one pair of eyes always on a character.
    if opts.controlled then
        return 1
    end
    -- Possession-driven keeper arms and every simulation-timer-driven action
    -- (dive, aerial, throw, wind-up) animate continuously; holding a frame of
    -- them visibly desynchronises the body from the simulation's own ramp.
    if
        (opts.dive or 0) > 0
        or (opts.aerial or 0) > 0
        or (opts.throw or 0) > 0
        or (opts.windup or 0) > 0
        or opts.holding
    then
        return 1
    end
    local id = opts.pose and opts.pose.id
    if id ~= nil and not pose_lod.RELAXED_POSE[id] then
        return 1
    end
    return pose_lod.REDUCED_INTERVAL
end

-- Whether this character's pose is due for re-evaluation on this draw.
--
-- `tick` counts the character's own draws; `stagger` is a small per-character
-- constant. Offsetting the schedule by the stagger spreads the refreshes of
-- different characters across frames, so ten held characters cost five
-- re-evaluations every frame instead of ten on alternate frames -- the mean is
-- the same, the p95 is not.
---@param tick integer  -- per-character draw counter
---@param stagger integer  -- per-character schedule offset
---@param interval integer  -- from pose_lod.interval
---@return boolean
function pose_lod.due(tick, stagger, interval)
    if interval <= 1 then
        return true
    end
    return (tick + stagger) % interval == 0
end

-- One cached character: its held bone rows, its draw counter, its schedule
-- offset, and whether the rows have ever been filled.
---@class PoseLodEntry
---@field rows number[][]  -- the character's held bone rows (renderer-owned content)
---@field tick integer     -- this character's own draw counter
---@field stagger integer  -- schedule offset, assigned round-robin at creation
---@field fresh boolean    -- false until the rows have been written once

-- Round-robin stagger for the next entry created. Consecutive characters get
-- consecutive offsets, which at interval 2 lands them on opposite frames.
local next_stagger = 0

-- The whole cache-entry lifecycle for one character's draw: look up (or
-- create) its entry in `cache`, advance its draw counter, and decide whether
-- this draw must re-evaluate the pose.
--
-- Returns `refresh = true` when the caller must evaluate and write
-- `entry.rows` before submitting them; the entry is marked fresh here because
-- that write is the caller's unconditional next step on a refresh. A brand-new
-- (or never-filled) entry always refreshes regardless of schedule -- cached
-- rows that were never written must never reach the GPU.
--
-- Pure apart from the entry bookkeeping itself: no love.*, no clock, no
-- knowledge of what a "view" is beyond being a stable table key. The caller
-- chooses the cache table (weak-keyed in the renderer, plain in tests).
---@param cache table<table, PoseLodEntry>
---@param key table  -- stable per-character identity (the renderer's PlayerView)
---@param interval integer  -- from pose_lod.interval
---@return PoseLodEntry entry
---@return boolean refresh
function pose_lod.step(cache, key, interval)
    local entry = cache[key]
    if not entry then
        entry = { rows = {}, tick = -1, stagger = next_stagger, fresh = false }
        next_stagger = (next_stagger + 1) % 1024
        cache[key] = entry
    end
    entry.tick = entry.tick + 1
    local refresh = not entry.fresh or pose_lod.due(entry.tick, entry.stagger, interval)
    if refresh then
        entry.fresh = true
    end
    return entry, refresh
end

return pose_lod
