-- Versioned learning-environment observations.
--
-- Three explicitly tagged profiles:
--
--   representative  one controlled slot, restricted to cues the presented match
--                   shows that slot's player at the current boundary tick.
--   team            several controlled slots, each still restricted to its own
--                   player-observable cues, with explicit slot ownership.
--   privileged      the representative view plus the authoritative canonical
--                   snapshot and RNG state. `player_observable` and
--                   `human_proxy_valid` are false: policies trained on this
--                   profile may never be reported as human proxies.
--
-- A view is built from the boundary state BEFORE any action for the next tick
-- is accepted, so it structurally cannot contain a future tick, another slot's
-- same-tick choice, or a resolver verdict that has not happened yet. Confirmed
-- events describe the tick that already completed.
--
-- Everything here is a fresh plain-data copy: an observation holds no reference
-- to MatchState, so a policy cannot reach through it into simulation authority.
--
-- Field-by-field source, unit, visibility, and history rules live in
-- docs/design/learning_environment.md.

local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local match_snapshot = require("sim.match_snapshot")

---@alias EnvRelativeSide "own"|"opponent"
---@alias EnvMatchPhase "kickoff"|"live"|"finished"
---@alias EnvObservationErrorCode "malformed"|"unknown_profile"|"profile_mismatch"

---@class EnvObservationProfileData
---@field id EnvObservationProfile
---@field player_observable boolean -- True only when every field has a presented analogue.
---@field human_proxy_valid boolean -- False for oracle/debug/adversarial profiles.
---@field multi_slot boolean
---@field description string

---@class EnvObservedGeometry
---@field field_w number -- Pitch width in world px.
---@field field_h number
---@field own_goal Rect -- Goal this slot defends, world px.
---@field target_goal Rect -- Goal this slot attacks, world px.

---@class EnvObservedMatch
---@field tick integer -- Boundary tick this observation describes.
---@field tick_rate integer
---@field phase EnvMatchPhase
---@field stoppage boolean -- True while the post-restart hold suppresses pressing.
---@field time_left number -- Seconds remaining.
---@field score_own integer
---@field score_opponent integer
---@field max_goals integer
---@field finished boolean

---@class EnvObservedBall
---@field x number -- World px.
---@field y number
---@field z number -- Height above the pitch, world px.
---@field vx number -- World px/s.
---@field vy number
---@field vz number
---@field spin number -- Lateral curve applied to the loose ball.
---@field airborne boolean
---@field loose boolean -- No player owns the ball.
---@field owner_slot integer? -- Canonical input slot of the carrier, when it has one.
---@field owner_side EnvRelativeSide? -- Carrier's side relative to the viewer.
---@field owner_is_keeper boolean

---@class EnvObservedEquipment
---@field family_id ActionFamilyId -- Visible equipment silhouette.
---@field phase CombatActionPhase -- Visible animation phase; remaining ticks are withheld.
---@field forced_state CombatForcedState? -- Visible stagger/knockback.

---@class EnvObservedSelfEquipment
---@field loadout_id string
---@field family_id ActionFamilyId
---@field phase CombatActionPhase
---@field phase_ticks integer -- Own readiness readout.
---@field cooldown_ticks integer
---@field ready boolean
---@field control_held boolean
---@field forced_state CombatForcedState?
---@field forced_ticks integer
---@field immunity_ticks integer

---@class EnvObservedPlayer
---@field slot integer? -- Canonical input slot; nil for keepers.
---@field side EnvRelativeSide
---@field is_keeper boolean
---@field x number
---@field y number
---@field vx number -- Realized velocity from the previous tick, world px/s.
---@field vy number
---@field facing_x number -- Unit facing.
---@field facing_y number
---@field radius number
---@field has_ball boolean
---@field sprinting boolean
---@field jockeying boolean
---@field sliding boolean
---@field tackling boolean
---@field dodging boolean
---@field stunned boolean
---@field diving boolean
---@field airborne boolean
---@field winding_up boolean -- A shot/punt windup pose is visible.
---@field charging boolean -- A charge meter pose is visible.
---@field equipment EnvObservedEquipment?

---@class EnvObservedSelf
---@field player_id string
---@field slot integer
---@field side EnvRelativeSide
---@field is_keeper boolean
---@field x number
---@field y number
---@field vx number
---@field vy number
---@field facing_x number
---@field facing_y number
---@field radius number
---@field move_speed number -- Own base locomotion speed, world px/s.
---@field has_ball boolean
---@field sprinting boolean
---@field jockeying boolean
---@field sliding boolean
---@field tackling boolean
---@field dodging boolean
---@field stunned boolean
---@field diving boolean
---@field airborne boolean
---@field winding_up boolean
---@field charging boolean
---@field sprint_meter number -- 0..1 own stamina readout.
---@field charge number -- 0..1 own shot charge readout.
---@field pass_charge number -- 0..1 own pass-range charge readout.
---@field dash_ready boolean
---@field dodge_ready boolean
---@field tackle_ready boolean
---@field header_ready boolean
---@field dash_cooldown_s number
---@field dodge_cooldown_s number
---@field tackle_cooldown_s number
---@field header_cooldown_s number
---@field stun_s number
---@field dodge_s number
---@field slide_s number
---@field tackle_s number
---@field jockey_s number
---@field windup_s number
---@field aerial_recovery_s number
---@field pass_preview_slot integer? -- Presented pass preview target, own carries only.
---@field pass_preview_keeper boolean
---@field equipment EnvObservedSelfEquipment?

---@class EnvObservedEvent
---@field kind string -- MatchEventKind or CombatEventKind.
---@field tick integer -- Tick the event was confirmed on; always in the past.
---@field x number
---@field y number
---@field actor_slot integer?
---@field actor_side EnvRelativeSide?
---@field on_target boolean? -- Soccer strikes only.
---@field result CombatContactResult? -- Confirmed combat verdict.
---@field family_id ActionFamilyId?

---@class EnvPrivilegedView
---@field rng integer -- Authoritative PRNG state.
---@field snapshot MatchSnapshot -- Canonical deep copy; never the live state table.
---@field boundary_hash string

---@class EnvSlotView
---@field version integer
---@field profile EnvObservationProfile
---@field slot integer -- Canonical input slot this view belongs to.
---@field slot_id InputSlotId
---@field team InputTeam -- Absolute side of this slot (public fixture fact).
---@field match EnvObservedMatch
---@field geometry EnvObservedGeometry
---@field ball EnvObservedBall
---@field own EnvObservedSelf
---@field teammates EnvObservedPlayer[] -- Canonical player order.
---@field opponents EnvObservedPlayer[]
---@field events EnvObservedEvent[] -- Confirmed events of the previous tick only.
---@field privileged EnvPrivilegedView? -- Privileged profile only.

---@class EnvObservation
---@field version integer
---@field profile EnvObservationProfile
---@field player_observable boolean
---@field human_proxy_valid boolean
---@field tick integer
---@field slots integer[] -- Controlled slots covered, ascending.
---@field views table<integer, EnvSlotView>

---@class EnvObservationModule
local env_observation = {}

env_observation.VERSION = 1

---@type table<EnvObservationProfile, EnvObservationProfileData>
env_observation.PROFILES = {
    representative = {
        id = "representative",
        player_observable = true,
        human_proxy_valid = true,
        multi_slot = false,
        description = "One controlled slot restricted to presented, current cues.",
    },
    team = {
        id = "team",
        player_observable = true,
        human_proxy_valid = true,
        multi_slot = true,
        description = "Per-slot player-observable views with explicit slot ownership.",
    },
    privileged = {
        id = "privileged",
        player_observable = false,
        human_proxy_valid = false,
        multi_slot = true,
        description = "Authoritative oracle/debug profile; never a human proxy.",
    },
}

---@param code EnvObservationErrorCode
---@param message string
---@return nil, string, EnvObservationErrorCode
local function reject(code, message)
    return nil, message, code
end

---@param team InputTeam
---@param view_team InputTeam
---@return EnvRelativeSide
local function side_of(team, view_team)
    return team == view_team and "own" or "opponent"
end

---@param rect Rect
---@return Rect
local function copy_rect(rect)
    return { x = rect.x, y = rect.y, w = rect.w, h = rect.h }
end

---@param state MatchState
---@return EnvMatchPhase
local function phase_of(state)
    if state.finished then
        return "finished"
    end
    if state.kickoff_hold > 0 then
        return "kickoff"
    end
    return "live"
end

---@param state MatchState
---@param combat_state CombatMatchState?
---@param player_index integer
---@return EnvObservedEquipment?
local function equipment_telegraph(state, combat_state, player_index)
    if not combat_state then
        return nil
    end
    local runtime = combat_state.players[player_index]
    if not runtime or not runtime.family_id then
        return nil
    end
    return {
        family_id = runtime.family_id,
        phase = runtime.phase,
        forced_state = runtime.forced_state,
    }
end

---@param combat_state CombatMatchState?
---@param player_index integer
---@return EnvObservedSelfEquipment?
local function own_equipment(combat_state, player_index)
    if not combat_state then
        return nil
    end
    local runtime = combat_state.players[player_index]
    if not runtime or not runtime.family_id or not runtime.loadout_id then
        return nil
    end
    return {
        loadout_id = runtime.loadout_id,
        family_id = runtime.family_id,
        phase = runtime.phase,
        phase_ticks = runtime.phase_ticks,
        cooldown_ticks = runtime.cooldown_ticks,
        ready = runtime.phase == "ready"
            and runtime.cooldown_ticks == 0
            and runtime.forced_state == nil,
        control_held = runtime.control_held,
        forced_state = runtime.forced_state,
        forced_ticks = runtime.forced_ticks,
        immunity_ticks = runtime.immunity_ticks,
    }
end

---@param state MatchState
---@param combat_state CombatMatchState?
---@param player_index integer
---@param view_team InputTeam
---@return EnvObservedPlayer
local function observed_player(state, combat_state, player_index, view_team)
    local player = state.players[player_index]
    return {
        slot = state.slot_for_player[player_index],
        side = side_of(player.team, view_team),
        is_keeper = player.is_keeper,
        x = player.pos.x,
        y = player.pos.y,
        vx = player.vel.x,
        vy = player.vel.y,
        facing_x = player.facing.x,
        facing_y = player.facing.y,
        radius = player.radius,
        has_ball = state.owner == player_index,
        sprinting = player.sprinting,
        jockeying = player.jockey_timer > 0,
        sliding = player.slide_timer > 0,
        tackling = player.tackle_timer > 0,
        dodging = player.dodge_timer > 0,
        stunned = player.stun_timer > 0,
        diving = player.dive_timer > 0,
        airborne = player.aerial_jump > 0,
        winding_up = player.windup_timer > 0,
        charging = player.charge > 0 or player.pass_charge > 0,
        equipment = equipment_telegraph(state, combat_state, player_index),
    }
end

---@param state MatchState
---@param combat_state CombatMatchState?
---@param player_index integer
---@param slot integer
---@return EnvObservedSelf
local function observed_self(state, combat_state, player_index, slot)
    local player = state.players[player_index]
    local preview_index = state.owner == player_index and player.pass_target or nil
    local preview = preview_index and state.players[preview_index] or nil
    return {
        player_id = player.id,
        slot = slot,
        side = "own",
        is_keeper = player.is_keeper,
        x = player.pos.x,
        y = player.pos.y,
        vx = player.vel.x,
        vy = player.vel.y,
        facing_x = player.facing.x,
        facing_y = player.facing.y,
        radius = player.radius,
        move_speed = player.move_speed,
        has_ball = state.owner == player_index,
        sprinting = player.sprinting,
        jockeying = player.jockey_timer > 0,
        sliding = player.slide_timer > 0,
        tackling = player.tackle_timer > 0,
        dodging = player.dodge_timer > 0,
        stunned = player.stun_timer > 0,
        diving = player.dive_timer > 0,
        airborne = player.aerial_jump > 0,
        winding_up = player.windup_timer > 0,
        charging = player.charge > 0 or player.pass_charge > 0,
        sprint_meter = player.sprint_meter,
        charge = player.charge,
        pass_charge = player.pass_charge,
        dash_ready = player.dash_cd <= 0,
        dodge_ready = player.dodge_cd <= 0,
        tackle_ready = player.tackle_cd <= 0,
        header_ready = player.header_cd <= 0,
        dash_cooldown_s = player.dash_cd,
        dodge_cooldown_s = player.dodge_cd,
        tackle_cooldown_s = player.tackle_cd,
        header_cooldown_s = player.header_cd,
        stun_s = player.stun_timer,
        dodge_s = player.dodge_timer,
        slide_s = player.slide_timer,
        tackle_s = player.tackle_timer,
        jockey_s = player.jockey_timer,
        windup_s = player.windup_timer,
        aerial_recovery_s = player.aerial_recovery,
        pass_preview_slot = preview_index and state.slot_for_player[preview_index] or nil,
        pass_preview_keeper = preview ~= nil and preview.is_keeper == true,
        equipment = own_equipment(combat_state, player_index),
    }
end

---@param state MatchState
---@param view_team InputTeam
---@return EnvObservedBall
local function observed_ball(state, view_team)
    local owner = state.owner and state.players[state.owner] or nil
    return {
        x = state.ball.x,
        y = state.ball.y,
        z = state.ball_z,
        vx = state.ball_vel.x,
        vy = state.ball_vel.y,
        vz = state.ball_vz,
        spin = state.ball_spin,
        airborne = state.ball_z > 0,
        loose = state.owner == nil,
        owner_slot = state.owner and state.slot_for_player[state.owner] or nil,
        owner_side = owner and side_of(owner.team, view_team) or nil,
        owner_is_keeper = owner ~= nil and owner.is_keeper == true,
    }
end

-- Confirmed events of the tick that already completed. `state.events` is
-- cleared at the top of every match step, so this is never a speculative or
-- rollback-presentation event, and its tick is always strictly in the past.
---@param state MatchState
---@param combat_state CombatMatchState?
---@param view_team InputTeam
---@return EnvObservedEvent[]
local function observed_events(state, combat_state, view_team)
    local events = {}
    if state.input_tick <= 0 then
        return events
    end
    local confirmed_tick = state.input_tick - 1
    local slot_of_id = {}
    local team_of_id = {}
    for index, player in ipairs(state.players) do
        slot_of_id[player.id] = state.slot_for_player[index]
        team_of_id[player.id] = player.team
    end
    for _, event in ipairs(state.events) do
        local actor_team = event.player and team_of_id[event.player] or nil
        events[#events + 1] = {
            kind = event.kind,
            tick = confirmed_tick,
            x = event.x,
            y = event.y,
            actor_slot = event.player and slot_of_id[event.player] or nil,
            actor_side = actor_team and side_of(actor_team, view_team) or nil,
            on_target = event.on_target,
            result = nil,
            family_id = nil,
        }
    end
    if combat_state then
        for _, event in ipairs(combat_state.events) do
            local source = event.source_index and state.players[event.source_index] or nil
            events[#events + 1] = {
                kind = event.kind,
                tick = event.tick,
                x = event.x,
                y = event.y,
                actor_slot = event.source_index and state.slot_for_player[event.source_index]
                    or nil,
                actor_side = source and side_of(source.team, view_team) or nil,
                on_target = nil,
                result = event.result,
                family_id = event.family_id,
            }
        end
    end
    return events
end

---@param state MatchState
---@param combat_state CombatMatchState?
---@return EnvPrivilegedView
local function privileged_view(state, combat_state)
    local snapshot = match_snapshot.capture(state, combat_state)
    return {
        rng = state.rng,
        snapshot = snapshot,
        boundary_hash = match_snapshot.hash(snapshot),
    }
end

-- Build one slot's view. `profile` only widens the result: the player-observable
-- body is identical for every profile, and `privileged` appends the
-- authoritative block on top of it.
---@param state MatchState
---@param combat_state CombatMatchState?
---@param slot integer
---@param profile EnvObservationProfile
---@return EnvSlotView?, string?, EnvObservationErrorCode?
function env_observation.view(state, combat_state, slot, profile)
    if not env_observation.PROFILES[profile] then
        return reject("unknown_profile", "unknown observation profile: " .. tostring(profile))
    end
    local canonical_slot, slot_err = input_frame.slot(slot)
    if not canonical_slot then
        return reject("malformed", assert(slot_err))
    end
    if not state.slot_mode then
        return reject("malformed", "observations require a fixed-slot match")
    end
    local player_index = state.slot_players[slot]
    if not player_index then
        return reject("malformed", "slot " .. slot .. " is not routed to a player")
    end
    local view_team = canonical_slot.team
    local teammates = {}
    local opponents = {}
    for index, player in ipairs(state.players) do
        if index ~= player_index then
            local observed = observed_player(state, combat_state, index, view_team)
            if player.team == view_team then
                teammates[#teammates + 1] = observed
            else
                opponents[#opponents + 1] = observed
            end
        end
    end
    local own_goal = view_team == "home" and state.goal_home or state.goal_away
    local target_goal = view_team == "home" and state.goal_away or state.goal_home
    return {
        version = env_observation.VERSION,
        profile = profile,
        slot = slot,
        slot_id = canonical_slot.id,
        team = view_team,
        match = {
            tick = state.input_tick,
            tick_rate = fixed_clock.TICK_RATE,
            phase = phase_of(state),
            stoppage = state.kickoff_hold > 0,
            time_left = state.time_left,
            score_own = view_team == "home" and state.score.home or state.score.away,
            score_opponent = view_team == "home" and state.score.away or state.score.home,
            max_goals = state.max_goals,
            finished = state.finished,
        },
        geometry = {
            field_w = state.field.w,
            field_h = state.field.h,
            own_goal = copy_rect(own_goal),
            target_goal = copy_rect(target_goal),
        },
        ball = observed_ball(state, view_team),
        own = observed_self(state, combat_state, player_index, slot),
        teammates = teammates,
        opponents = opponents,
        events = observed_events(state, combat_state, view_team),
        privileged = profile == "privileged" and privileged_view(state, combat_state) or nil,
    }
end

-- Build the observation for a profile over the controlled slots. Each slot's
-- view is built from the same boundary state, independently: no view can carry
-- another slot's pending action because no action has been accepted yet.
---@param state MatchState
---@param combat_state CombatMatchState?
---@param slots integer[]
---@param profile EnvObservationProfile
---@return EnvObservation?, string?, EnvObservationErrorCode?
function env_observation.build(state, combat_state, slots, profile)
    local profile_data = env_observation.PROFILES[profile]
    if not profile_data then
        return reject("unknown_profile", "unknown observation profile: " .. tostring(profile))
    end
    if type(slots) ~= "table" or #slots == 0 then
        return reject("malformed", "observations need at least one controlled slot")
    end
    if not profile_data.multi_slot and #slots ~= 1 then
        return reject(
            "profile_mismatch",
            "the " .. profile .. " profile covers exactly one controlled slot"
        )
    end
    local views = {}
    local ordered = {}
    local previous = 0
    for index = 1, #slots do
        local slot = slots[index]
        if type(slot) ~= "number" or slot ~= math.floor(slot) or slot <= previous then
            return reject("malformed", "controlled slots must be ascending unique integers")
        end
        previous = slot
        local view, view_err, view_code = env_observation.view(state, combat_state, slot, profile)
        if not view then
            return nil, view_err, view_code
        end
        views[slot] = view
        ordered[index] = slot
    end
    return {
        version = env_observation.VERSION,
        profile = profile,
        player_observable = profile_data.player_observable,
        human_proxy_valid = profile_data.human_proxy_valid,
        tick = state.input_tick,
        slots = ordered,
        views = views,
    }
end

---@param observation EnvObservation
---@param slot integer
---@return EnvSlotView?
function env_observation.view_for(observation, slot)
    return observation.views[slot]
end

---@param parts string[]
---@param value any
local function encode_value(parts, value)
    local kind = type(value)
    if value == nil then
        parts[#parts + 1] = "z;"
    elseif kind == "boolean" then
        parts[#parts + 1] = value and "b1;" or "b0;"
    elseif kind == "number" then
        parts[#parts + 1] = "n" .. match_snapshot.number_bytes(value) .. ";"
    elseif kind == "string" then
        parts[#parts + 1] = "s" .. #value .. ":" .. value .. ";"
    else
        assert(kind == "table", "observations contain only plain data")
        local numeric, strings = {}, {}
        for key in pairs(value) do
            if type(key) == "number" then
                numeric[#numeric + 1] = key
            else
                assert(type(key) == "string", "observation keys must be numbers or strings")
                strings[#strings + 1] = key
            end
        end
        table.sort(numeric)
        table.sort(strings)
        parts[#parts + 1] = "t" .. (#numeric + #strings) .. ";"
        for _, key in ipairs(numeric) do
            encode_value(parts, key)
            encode_value(parts, value[key])
        end
        for _, key in ipairs(strings) do
            encode_value(parts, key)
            encode_value(parts, value[key])
        end
    end
end

-- Canonical byte-exact encoding. Two observations encode identically exactly
-- when they carry the same information, which is what the leakage tests assert.
---@param observation EnvObservation|EnvSlotView
---@return string
function env_observation.encode(observation)
    local parts = { "GCEO;" .. env_observation.VERSION .. ";" }
    encode_value(parts, observation)
    return table.concat(parts)
end

return env_observation
