local combat_presentation = require("game.presentation.combat")

---@alias CombatFeedbackLinkKind "accepted_encounter"|"rejected_request"|"missing_feedback"
---@alias CombatFeedbackCueState "speculative"|"confirmed"|"revoked"
---@alias CombatFeedbackVfx
---| "none"
---| "commit"
---| "projectile_spawn"
---| "projectile_expire"
---| "contact_hit"
---| "contact_extended"
---| "contact_guarded"
---| "contact_immune"
---| "contact_superseded"
---| "ball_spill"
---| "forced"
---| "guard_recoil"
---| "rejected"

---@alias CombatRequestRejectionReason
---| "protected_keeper_or_no_loadout"
---| "kickoff_hold"
---| "soccer_commitment"
---| "aerial_state_or_recovery"
---| "forced_state"
---| "already_committed"
---| "cooldown"
---| "missing_press_edge"
---| "malformed_input"

---@class CombatFeedbackDisposition
---@field vfx CombatFeedbackVfx
---@field audio string?
---@field hud_text string?
---@field glyph string
---@field camera_strength number
---@field reversible boolean

---@class CombatFeedbackLink
---@field stable_id string
---@field link_kind CombatFeedbackLinkKind
---@field semantic_id string
---@field tick integer
---@field source_index integer?
---@field target_index integer?
---@field source_sequence integer?
---@field family_id ActionFamilyId?
---@field result CombatContactResult?
---@field reason CombatRequestRejectionReason?
---@field feedback_event_id string?
---@field x number?
---@field y number?
---@field disposition CombatFeedbackDisposition

---@class CombatFeedbackNotice
---@field id string
---@field text string
---@field glyph string
---@field source_index integer?
---@field target_index integer?
---@field life number
---@field max number

---@class CombatFeedbackImpulse
---@field id string
---@field strength number
---@field direction number
---@field life number
---@field max number

---@class CombatFeedbackState
---@field confirmed_ids table<string, boolean>
---@field notice CombatFeedbackNotice?
---@field impulse CombatFeedbackImpulse?
---@field reduced_motion boolean
---@field reduced_flash boolean

---@class CombatFeedbackObservation
---@field stable_id string
---@field link_kind CombatFeedbackLinkKind
---@field semantic_id string
---@field cue_state CombatFeedbackCueState
---@field feedback_event_id string?
---@field source_sequence integer?
---@field reason CombatRequestRejectionReason?
---@field vfx CombatFeedbackVfx
---@field audio string?
---@field hud_text string?
---@field camera_enabled boolean
---@field reduced_motion boolean
---@field reduced_flash boolean

---@class CombatFeedbackModule
local feedback = {}

local NOTICE_SECONDS = 0.72
local IMPULSE_SECONDS = 0.16
local MAX_CAMERA_OFFSET = 4
local default_reduced_motion = false
local default_reduced_flash = false

---@type table<CombatPresentationEventId, CombatFeedbackDisposition>
local DISPOSITIONS = {
    ["combat.commit"] = {
        vfx = "commit",
        audio = "combat_commit",
        hud_text = "EQUIPMENT COMMITTED",
        glyph = "chevron",
        camera_strength = 0,
        reversible = true,
    },
    ["combat.projectile.spawn"] = {
        vfx = "projectile_spawn",
        audio = "combat_projectile",
        glyph = "diamond",
        camera_strength = 0,
        reversible = true,
    },
    ["combat.projectile.expire"] = {
        vfx = "projectile_expire",
        audio = "combat_expire",
        hud_text = "RANGED EXPIRED",
        glyph = "hollow_diamond",
        camera_strength = 0,
        reversible = true,
    },
    ["combat.contact.hit"] = {
        vfx = "contact_hit",
        audio = "combat_hit",
        hud_text = "CLEAN HIT",
        glyph = "cross",
        camera_strength = 3,
        reversible = true,
    },
    ["combat.contact.extended"] = {
        vfx = "contact_extended",
        audio = "combat_extended",
        hud_text = "CHAIN CAPPED",
        glyph = "double_cross",
        camera_strength = 2.5,
        reversible = true,
    },
    ["combat.contact.guarded"] = {
        vfx = "contact_guarded",
        audio = "combat_guarded",
        hud_text = "GUARDED",
        glyph = "shield",
        camera_strength = 1,
        reversible = true,
    },
    ["combat.contact.immune"] = {
        vfx = "contact_immune",
        audio = "combat_immune",
        hud_text = "IMMUNE",
        glyph = "broken_ring",
        camera_strength = 0,
        reversible = true,
    },
    ["combat.contact.superseded"] = {
        vfx = "contact_superseded",
        audio = "combat_superseded",
        hud_text = "CONTACT DEFLECTED",
        glyph = "split",
        camera_strength = 0,
        reversible = true,
    },
    ["combat.ball_spill"] = {
        vfx = "ball_spill",
        audio = "combat_spill",
        hud_text = "BALL SPILLED",
        glyph = "burst",
        camera_strength = 2,
        reversible = true,
    },
    ["combat.forced"] = {
        vfx = "forced",
        audio = "combat_forced",
        hud_text = "CONTROL INTERRUPTED",
        glyph = "stagger",
        camera_strength = 3,
        reversible = true,
    },
    ["combat.guard_recoil"] = {
        vfx = "guard_recoil",
        audio = "combat_recoil",
        hud_text = "GUARD HELD",
        glyph = "shield",
        camera_strength = 1,
        reversible = true,
    },
}

local REJECTION_LABELS = {
    protected_keeper_or_no_loadout = "NO EQUIPMENT",
    kickoff_hold = "WAIT FOR KICKOFF",
    soccer_commitment = "SOCCER ACTION ACTIVE",
    aerial_state_or_recovery = "AERIAL ACTION ACTIVE",
    forced_state = "CONTROL INTERRUPTED",
    already_committed = "ACTION COMMITTED",
    cooldown = "COOLDOWN",
    missing_press_edge = "PRESS AGAIN",
    malformed_input = "INPUT INVALID",
}

---@return CombatFeedbackDisposition
local function rejected_disposition(reason)
    return {
        vfx = "rejected",
        audio = "combat_rejected",
        hud_text = "EQUIPMENT BLOCKED · " .. assert(REJECTION_LABELS[reason]),
        glyph = "blocked",
        camera_strength = 0,
        reversible = true,
    }
end

---@return CombatFeedbackDisposition
local function missing_disposition()
    return {
        vfx = "none",
        audio = nil,
        hud_text = nil,
        glyph = "none",
        camera_strength = 0,
        reversible = false,
    }
end

---@param event CombatEvent
---@return CombatFeedbackDisposition
function feedback.disposition(event)
    local presented = combat_presentation.event(event)
    return assert(DISPOSITIONS[presented.semantic_id], "unsupported combat feedback event")
end

---@param event CombatEvent
---@param stable_id string
---@return CombatFeedbackLink
function feedback.accepted(event, stable_id)
    assert(type(stable_id) == "string" and stable_id ~= "", "combat feedback id is required")
    local presented = combat_presentation.event(event, stable_id)
    assert(
        event.source_sequence ~= nil,
        "accepted combat feedback requires an authoritative source sequence"
    )
    return {
        stable_id = stable_id,
        link_kind = "accepted_encounter",
        semantic_id = presented.semantic_id,
        tick = event.tick,
        source_index = event.source_index,
        target_index = event.target_index,
        source_sequence = event.source_sequence,
        family_id = event.family_id,
        result = event.result,
        reason = nil,
        feedback_event_id = stable_id,
        x = event.x,
        y = event.y,
        disposition = assert(DISPOSITIONS[presented.semantic_id]),
    }
end

---@param stable_id string
---@param tick integer
---@param player_index integer
---@param reason CombatRequestRejectionReason
---@return CombatFeedbackLink
function feedback.rejected_request(stable_id, tick, player_index, reason)
    assert(type(stable_id) == "string" and stable_id ~= "", "rejected request id is required")
    assert(REJECTION_LABELS[reason], "unknown combat rejection reason")
    return {
        stable_id = stable_id,
        link_kind = "rejected_request",
        semantic_id = "combat.request.rejected." .. reason,
        tick = tick,
        source_index = player_index,
        target_index = nil,
        source_sequence = nil,
        family_id = nil,
        result = nil,
        reason = reason,
        feedback_event_id = stable_id,
        x = nil,
        y = nil,
        disposition = rejected_disposition(reason),
    }
end

---@param request_id string
---@param tick integer
---@param player_index integer
---@param reason CombatRequestRejectionReason
---@return CombatFeedbackLink
function feedback.missing_feedback(request_id, tick, player_index, reason)
    assert(
        type(request_id) == "string" and request_id ~= "",
        "missing feedback request id is required"
    )
    assert(REJECTION_LABELS[reason], "unknown combat rejection reason")
    return {
        stable_id = request_id,
        link_kind = "missing_feedback",
        semantic_id = "combat.request.missing_feedback",
        tick = tick,
        source_index = player_index,
        target_index = nil,
        source_sequence = nil,
        family_id = nil,
        result = nil,
        reason = reason,
        feedback_event_id = nil,
        x = nil,
        y = nil,
        disposition = missing_disposition(),
    }
end

---@return CombatFeedbackState
function feedback.new()
    return {
        confirmed_ids = {},
        notice = nil,
        impulse = nil,
        reduced_motion = default_reduced_motion,
        reduced_flash = default_reduced_flash,
    }
end

---@param settings GameSettings
function feedback.configure_defaults(settings)
    default_reduced_motion = not settings.screen_shake
    default_reduced_flash = not settings.bloom
end

---@param state CombatFeedbackState
---@param reduced_motion boolean
---@param reduced_flash boolean
function feedback.configure(state, reduced_motion, reduced_flash)
    state.reduced_motion = reduced_motion
    state.reduced_flash = reduced_flash
    if reduced_motion then
        state.impulse = nil
    end
end

---@param id string
---@return number
local function stable_direction(id)
    local value = 0
    for index = 1, #id do
        value = (value + id:byte(index) * index) % 2
    end
    return value == 0 and -1 or 1
end

---@param state CombatFeedbackState
---@param link CombatFeedbackLink
---@return boolean consumed
function feedback.confirm(state, link)
    if state.confirmed_ids[link.stable_id] then
        return false
    end
    state.confirmed_ids[link.stable_id] = true
    local disposition = link.disposition
    if disposition.hud_text then
        state.notice = {
            id = link.stable_id,
            text = disposition.hud_text,
            glyph = disposition.glyph,
            source_index = link.source_index,
            target_index = link.target_index,
            life = NOTICE_SECONDS,
            max = NOTICE_SECONDS,
        }
    end
    if disposition.camera_strength > 0 and not state.reduced_motion then
        local strength = math.min(MAX_CAMERA_OFFSET, disposition.camera_strength)
        local current = state.impulse and state.impulse.strength or 0
        state.impulse = {
            id = link.stable_id,
            strength = math.min(MAX_CAMERA_OFFSET, current + strength),
            direction = stable_direction(link.stable_id),
            life = IMPULSE_SECONDS,
            max = IMPULSE_SECONDS,
        }
    end
    return true
end

---@param state CombatFeedbackState
function feedback.reset_visuals(state)
    state.notice = nil
    state.impulse = nil
end

---@param state CombatFeedbackState
---@param dt number
function feedback.tick(state, dt)
    if state.notice then
        state.notice.life = math.max(0, state.notice.life - dt)
        if state.notice.life == 0 then
            state.notice = nil
        end
    end
    if state.impulse then
        state.impulse.life = math.max(0, state.impulse.life - dt)
        if state.impulse.life == 0 then
            state.impulse = nil
        end
    end
end

---@param state CombatFeedbackState
---@return { x: number, y: number }
function feedback.camera_offset(state)
    if state.reduced_motion or not state.impulse then
        return { x = 0, y = 0 }
    end
    local impulse = assert(state.impulse)
    local progress = impulse.life / impulse.max
    local magnitude = math.min(MAX_CAMERA_OFFSET, impulse.strength * progress)
    return {
        x = impulse.direction * magnitude,
        y = -magnitude * 0.35,
    }
end

---@param state CombatFeedbackState
---@param controlled_index integer
---@return CombatFeedbackNotice?
function feedback.notice(state, controlled_index)
    local notice = state.notice
    if
        notice
        and (notice.source_index == controlled_index or notice.target_index == controlled_index)
    then
        return notice
    end
    return nil
end

---@param state CombatFeedbackState
---@param link CombatFeedbackLink
---@param cue_state CombatFeedbackCueState
---@return CombatFeedbackObservation
function feedback.observation(state, link, cue_state)
    return {
        stable_id = link.stable_id,
        link_kind = link.link_kind,
        semantic_id = link.semantic_id,
        cue_state = cue_state,
        feedback_event_id = link.feedback_event_id,
        source_sequence = link.source_sequence,
        reason = link.reason,
        vfx = link.disposition.vfx,
        audio = link.disposition.audio,
        hud_text = link.disposition.hud_text,
        camera_enabled = link.disposition.camera_strength > 0 and not state.reduced_motion,
        reduced_motion = state.reduced_motion,
        reduced_flash = state.reduced_flash,
    }
end

---@param state CombatFeedbackState
---@return { confirmed_count: integer, notice_id: string?, impulse_id: string?, reduced_motion: boolean, reduced_flash: boolean }
function feedback.diagnostics(state)
    local confirmed_count = 0
    for _ in pairs(state.confirmed_ids) do
        confirmed_count = confirmed_count + 1
    end
    return {
        confirmed_count = confirmed_count,
        notice_id = state.notice and state.notice.id or nil,
        impulse_id = state.impulse and state.impulse.id or nil,
        reduced_motion = state.reduced_motion,
        reduced_flash = state.reduced_flash,
    }
end

return feedback
