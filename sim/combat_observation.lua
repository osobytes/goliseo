-- `combat_sim_observation/v1`: the only observation schema available inside
-- `sim/` and to the shipped gameplay AI.
--
-- It carries authoritative PUBLIC simulation state and nothing else. Every
-- field below is something a presented match already shows or a public catalog
-- already publishes. The exclusion list is enforced, not merely documented:
-- pixels, viewport, theme, presentation/loadout identity, render timing,
-- unresolved outcomes, hidden collision results, future input, and RNG state
-- can never enter, and `spec/sim/combat_observation_leakage_spec.lua` fails if
-- one does.
--
-- Two rules make the schema testable:
--
--   * Rows are canonically ordered. Teammate/opponent rows follow the canonical
--     player index; projectile rows follow `(source_sequence, source_player_id,
--     projectile_id)`. `validate` rejects a reordered array.
--   * No field is ever nil. Optional values materialize as explicit sentinels
--     (`0` for integers, `"none"` for enums, `false` for flags). Lua cannot hold
--     an explicit-nil key, so `nil` already means "missing" and `validate` can
--     reject it either way; what the sentinels buy is that a legitimately
--     INAPPLICABLE value -- no accepted action, so no source sequence -- does not
--     read as a missing field. Every row is therefore total, which is also what
--     lets the canonical encoding stay a fixed-width walk of the field list.
--
-- The digest is FNV-1a-64 over the canonical encoding below. Section 4.0 of
-- `docs/design/combat_fun_evidence_contract.md` names SHA-256 for #148's export
-- identity; this project ships only `core/fnv1a64.lua`, and adding a second
-- hash implementation to satisfy a per-schema digest would be a much larger
-- change than the digest is worth. Section 4.7 only requires a separate
-- canonical digest per schema, which FNV-1a over a canonical encoding provides.

local fnv1a64 = require("core.fnv1a64")
local fixed_clock = require("sim.fixed_clock")
local match_snapshot = require("sim.match_snapshot")
local action_families = require("data.action_families")
local combat_rules = require("sim.combat_rules")

---@alias CombatObservationSoccerAction
---|"none"
---|"slide"
---|"tackle"
---|"jockey"
---|"dodge"
---|"aerial"
---|"windup"
---|"receive"

---@class CombatObservationSelf
---@field player_index integer
---@field player_id string
---@field slot integer -- Canonical input slot, or 0 when this player owns none.
---@field team InputTeam
---@field is_keeper boolean
---@field family_id string -- ActionFamilyId, or "none" without a loadout.
---@field source_sequence integer -- 0 when no action is accepted.
---@field soccer_action CombatObservationSoccerAction
---@field soccer_committed boolean
---@field phase CombatActionPhase
---@field phase_ticks integer
---@field forced_state string -- CombatForcedState, or "none".
---@field forced_ticks integer
---@field immunity_ticks integer
---@field cooldown_ticks integer
---@field ready boolean
---@field control_held boolean
---@field release_latched boolean
---@field projected_spawn_tick integer -- 0 unless ranged and latched.
---@field x number
---@field y number
---@field vx number
---@field vy number
---@field facing_x number
---@field facing_y number
---@field radius number
---@field move_speed number -- Own base locomotion speed, world px/s.
---@field last_equipment_held boolean -- Own materialized input through the observed tick.
---@field last_equipment_pressed boolean
---@field last_equipment_released boolean

---@class CombatObservationPeer
---@field player_index integer
---@field player_id string
---@field slot integer
---@field team InputTeam
---@field is_keeper boolean
---@field soccer_action CombatObservationSoccerAction
---@field soccer_committed boolean
---@field family_id string
---@field source_sequence integer
---@field phase CombatActionPhase
---@field phase_ticks integer
---@field forced_state string
---@field forced_ticks integer
---@field immunity_ticks integer
---@field guard_active boolean
---@field telegraph_start_tick integer -- 0 when no action is public.
---@field telegraph_end_tick integer -- 0 when the public path has no bounded end.
---@field projected_reach_px number
---@field projected_arc_degrees number
---@field release_latched boolean
---@field projected_spawn_tick integer
---@field x number
---@field y number
---@field vx number
---@field vy number
---@field facing_x number
---@field facing_y number
---@field radius number

---@class CombatObservationProjectile
---@field projectile_id string
---@field source_player_index integer
---@field source_player_id string
---@field source_team InputTeam
---@field source_sequence integer
---@field family_id string
---@field x number
---@field y number
---@field dir_x number
---@field dir_y number
---@field vx number
---@field vy number
---@field in_flight boolean
---@field remaining_lifetime_ticks integer
---@field horizon_ticks integer

---@class CombatObservationBall
---@field x number
---@field y number
---@field z number
---@field vx number
---@field vy number
---@field vz number
---@field loose boolean
---@field owner_player_index integer
---@field owner_team string -- InputTeam, or "none".
---@field owner_is_keeper boolean

---@class CombatObservationMatch
---@field tick integer
---@field tick_rate integer
---@field field_w number
---@field field_h number
---@field goal_home_x number -- Static pitch geometry; the left goal mouth rect.
---@field goal_home_y number
---@field goal_home_w number
---@field goal_home_h number
---@field goal_away_x number
---@field goal_away_y number
---@field goal_away_w number
---@field goal_away_h number
---@field score_home integer
---@field score_away integer
---@field remaining_ticks integer
---@field stoppage boolean
---@field stoppage_ticks integer
---@field finished boolean

---@class CombatObservationAnchor
---@field player_index integer
---@field formation_slot_id string
---@field anchor_x number
---@field anchor_y number

---@class CombatObservation
---@field schema string
---@field version integer
---@field policy_id string
---@field producer_tick integer
---@field observed_tick integer
---@field family_catalog_version string
---@field collision_catalog_version string
---@field player_order integer[]
---@field projectile_order string[]
---@field self CombatObservationSelf
---@field teammates CombatObservationPeer[]
---@field opponents CombatObservationPeer[]
---@field projectiles CombatObservationProjectile[]
---@field ball CombatObservationBall
---@field match CombatObservationMatch
---@field anchors CombatObservationAnchor[]
---@field digest string

---@class CombatObservationModule
local combat_observation = {}

combat_observation.SCHEMA = "combat_sim_observation/v1"
combat_observation.VERSION = 1

-- The frozen static-collision rule this schema's projections assume: public
-- motion advances linearly at the fixed tick and is clamped to the pitch by the
-- player radius. `sim.combat_feasibility` uses exactly this rule, so the
-- feasibility predicate and the observation cannot drift apart silently.
combat_observation.COLLISION_CATALOG_VERSION = "pitch.linear_clamp.v1"

local FAMILY_ORDER = { "unarmed", "guard", "light_melee", "ranged" }
local CATALOG_FIELDS = {
    "id",
    "activation",
    "contact_kind",
    "windup_ticks",
    "active_ticks",
    "held_active",
    "recovery_ticks",
    "cooldown_ticks",
    "reach_px",
    "projectile_speed_px_per_second",
    "projectile_lifetime_ticks",
    "front_arc_degrees",
    "movement_multiplier",
    "guarded_recoil_px",
}
local CATALOG_OUTCOME_FIELDS = { "interruption_ticks", "displacement_px", "ball_spill" }
local CATALOG_RULE_FIELDS = { "MAX_DISABLE_TICKS", "IMMUNITY_TICKS", "KNOCKBACK_THRESHOLD_PX" }

-- Canonical encoded order. `digest` is deliberately absent: it is a digest OF
-- this body, so it cannot also be inside it.
combat_observation.ENCODED_FIELDS = {
    "schema",
    "version",
    "policy_id",
    "producer_tick",
    "observed_tick",
    "family_catalog_version",
    "collision_catalog_version",
    "player_order",
    "projectile_order",
    "self",
    "teammates",
    "opponents",
    "projectiles",
    "ball",
    "match",
    "anchors",
}

combat_observation.SELF_FIELDS = {
    "player_index",
    "player_id",
    "slot",
    "team",
    "is_keeper",
    "family_id",
    "source_sequence",
    "soccer_action",
    "soccer_committed",
    "phase",
    "phase_ticks",
    "forced_state",
    "forced_ticks",
    "immunity_ticks",
    "cooldown_ticks",
    "ready",
    "control_held",
    "release_latched",
    "projected_spawn_tick",
    "x",
    "y",
    "vx",
    "vy",
    "facing_x",
    "facing_y",
    "radius",
    "move_speed",
    "last_equipment_held",
    "last_equipment_pressed",
    "last_equipment_released",
}

combat_observation.PEER_FIELDS = {
    "player_index",
    "player_id",
    "slot",
    "team",
    "is_keeper",
    "soccer_action",
    "soccer_committed",
    "family_id",
    "source_sequence",
    "phase",
    "phase_ticks",
    "forced_state",
    "forced_ticks",
    "immunity_ticks",
    "guard_active",
    "telegraph_start_tick",
    "telegraph_end_tick",
    "projected_reach_px",
    "projected_arc_degrees",
    "release_latched",
    "projected_spawn_tick",
    "x",
    "y",
    "vx",
    "vy",
    "facing_x",
    "facing_y",
    "radius",
}

combat_observation.PROJECTILE_FIELDS = {
    "projectile_id",
    "source_player_index",
    "source_player_id",
    "source_team",
    "source_sequence",
    "family_id",
    "x",
    "y",
    "dir_x",
    "dir_y",
    "vx",
    "vy",
    "in_flight",
    "remaining_lifetime_ticks",
    "horizon_ticks",
}

combat_observation.BALL_FIELDS = {
    "x",
    "y",
    "z",
    "vx",
    "vy",
    "vz",
    "loose",
    "owner_player_index",
    "owner_team",
    "owner_is_keeper",
}

combat_observation.MATCH_FIELDS = {
    "tick",
    "tick_rate",
    "field_w",
    "field_h",
    "goal_home_x",
    "goal_home_y",
    "goal_home_w",
    "goal_home_h",
    "goal_away_x",
    "goal_away_y",
    "goal_away_w",
    "goal_away_h",
    "score_home",
    "score_away",
    "remaining_ticks",
    "stoppage",
    "stoppage_ticks",
    "finished",
}

combat_observation.ANCHOR_FIELDS = {
    "player_index",
    "formation_slot_id",
    "anchor_x",
    "anchor_y",
}

-- Fields a caller could plausibly try to smuggle in. `validate` already rejects
-- every undeclared key, so this is not a second enforcement path; it is the
-- named subset that section 4.7 explicitly excludes.
-- `spec/sim/combat_observation_leakage_spec.lua` consumes this list and adds its
-- own broader entries on top, so the two cannot drift apart.
combat_observation.FORBIDDEN_FIELDS = {
    "loadout_id",
    "presentation_id",
    "cosmetic_variant_id",
    "theme",
    "viewport",
    "pixels",
    "render_tick",
    "frame_time",
    "rng",
    "rng_state",
    "snapshot",
    "future_input",
    "pending_contact",
    "contact_result",
    "outcome",
}

---@param fields string[]
---@return table<string, boolean>
local function field_set(fields)
    local result = {}
    for _, field in ipairs(fields) do
        result[field] = true
    end
    return result
end

combat_observation.FIELD_SET = field_set(combat_observation.ENCODED_FIELDS)
combat_observation.FIELD_SET.digest = true
combat_observation.SELF_FIELD_SET = field_set(combat_observation.SELF_FIELDS)
combat_observation.PEER_FIELD_SET = field_set(combat_observation.PEER_FIELDS)
combat_observation.PROJECTILE_FIELD_SET = field_set(combat_observation.PROJECTILE_FIELDS)
combat_observation.BALL_FIELD_SET = field_set(combat_observation.BALL_FIELDS)
combat_observation.MATCH_FIELD_SET = field_set(combat_observation.MATCH_FIELDS)
combat_observation.ANCHOR_FIELD_SET = field_set(combat_observation.ANCHOR_FIELDS)

local PHASES = field_set({ "ready", "windup", "active", "aim", "guard", "recovery" })
local FORCED_STATES = field_set({ "none", "stagger", "knockback" })
local FAMILIES = field_set({ "none", "unarmed", "guard", "light_melee", "ranged" })
local TEAMS = field_set({ "home", "away" })
local SOCCER_ACTIONS = field_set({
    "none",
    "slide",
    "tackle",
    "jockey",
    "dodge",
    "aerial",
    "windup",
    "receive",
})

---@param parts string[]
---@param value any
local function append(parts, value)
    local kind = type(value)
    if kind == "boolean" then
        parts[#parts + 1] = value and "b1;" or "b0;"
    elseif kind == "number" then
        parts[#parts + 1] = "n" .. match_snapshot.number_bytes(value) .. ";"
    else
        assert(kind == "string", "combat observation values must be canonical scalars")
        parts[#parts + 1] = "s" .. tostring(#value) .. ":" .. value .. ";"
    end
end

---@return string
local function catalog_version()
    local parts = { "GCCV;1;" }
    for _, field in ipairs(CATALOG_RULE_FIELDS) do
        append(parts, field)
        append(parts, combat_rules[field])
    end
    for _, family_id in ipairs(FAMILY_ORDER) do
        local family = assert(action_families[family_id])
        append(parts, family_id)
        for _, field in ipairs(CATALOG_FIELDS) do
            append(parts, field)
            local value = family[field]
            append(parts, value == nil and 0 or value)
        end
        local outcome = family.unguarded_outcome
        append(parts, outcome ~= nil)
        if outcome then
            for _, field in ipairs(CATALOG_OUTCOME_FIELDS) do
                append(parts, field)
                append(parts, outcome[field])
            end
        end
    end
    -- `fnv1a64.hash` already returns exactly 16 lowercase hex characters.
    return "family_catalog." .. fnv1a64.hash(table.concat(parts))
end

combat_observation.FAMILY_CATALOG_VERSION = catalog_version()

---@param player MatchPlayer
---@return CombatObservationSoccerAction
local function soccer_action_of(player)
    if player.slide_timer > 0 then
        return "slide"
    end
    if player.dodge_timer > 0 then
        return "dodge"
    end
    if player.tackle_timer > 0 then
        return "tackle"
    end
    if player.aerial_timer > 0 or player.aerial_recovery > 0 then
        return "aerial"
    end
    if player.windup_timer > 0 or player.windup_shot ~= nil then
        return "windup"
    end
    if player.jockey_timer > 0 then
        return "jockey"
    end
    if player.receive_timer > 0 then
        return "receive"
    end
    return "none"
end

---@param player MatchPlayer
---@return boolean
local function soccer_committed_of(player)
    return player.slide_timer > 0
        or player.tackle_timer > 0
        or player.jockey_timer > 0
        or player.dodge_timer > 0
        or player.aerial_timer > 0
        or player.aerial_recovery > 0
        or player.windup_timer > 0
        or player.windup_shot ~= nil
end

-- Ticks already spent inside the current accepted action, derived only from the
-- public phase and its public remaining ticks.
---@param family ActionFamilyData
---@param phase CombatActionPhase
---@param phase_ticks integer
---@return integer
local function elapsed_action_ticks(family, phase, phase_ticks)
    if phase == "windup" then
        return family.windup_ticks - phase_ticks
    end
    if phase == "active" then
        return family.windup_ticks + (family.active_ticks or 1) - phase_ticks
    end
    if phase == "aim" or phase == "guard" then
        return family.windup_ticks
    end
    if phase == "recovery" then
        return family.windup_ticks
            + (family.active_ticks or 0)
            + (family.recovery_ticks - phase_ticks)
    end
    return 0
end

-- Remaining ticks of the public OUTGOING threat path. Zero means the row has no
-- bounded hostile path a defender could plan against: a guard is self-only, an
-- unlatched aim has no fixed release, and recovery threatens nobody.
---@param family ActionFamilyData
---@param phase CombatActionPhase
---@param phase_ticks integer
---@param release_latched boolean
---@return integer
local function threat_path_ticks(family, phase, phase_ticks, release_latched)
    if family.contact_kind == "guard" then
        return 0
    end
    if family.id == "ranged" then
        if phase == "windup" then
            return release_latched and (phase_ticks + 1) or 0
        end
        if phase == "active" then
            return phase_ticks
        end
        return 0
    end
    if phase == "windup" then
        return phase_ticks + (family.active_ticks or 0)
    end
    if phase == "active" then
        return phase_ticks
    end
    return 0
end

-- The public telegraph another player can see. `sim.env_observation` projects a
-- strict subset of this row so the two observation schemas derive from one
-- shared reading of presented combat state (docs/design/learning_environment.md
-- asks for exactly that convergence).
---@param combat_state CombatMatchState?
---@param player_index integer
---@return { family_id: ActionFamilyId, phase: CombatActionPhase, forced_state: CombatForcedState? }?
function combat_observation.telegraph(combat_state, player_index)
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

---@param state MatchState
---@param combat_state CombatMatchState?
---@param player_index integer
---@return CombatObservationPeer
local function peer_row(state, combat_state, player_index)
    local player = state.players[player_index]
    local runtime = combat_state and combat_state.players[player_index] or nil
    local family_id = runtime and runtime.family_id or nil
    local family = family_id and action_families[family_id] or nil
    local phase = runtime and runtime.phase or "ready"
    local phase_ticks = runtime and runtime.phase_ticks or 0
    local release_latched = (runtime and runtime.release_latched) == true
    local telegraph_start, telegraph_end = 0, 0
    local spawn_tick = 0
    if family and phase ~= "ready" then
        local tick = assert(combat_state).tick
        telegraph_start = tick - elapsed_action_ticks(family, phase, phase_ticks)
        local remaining = threat_path_ticks(family, phase, phase_ticks, release_latched)
        telegraph_end = remaining > 0 and (tick + remaining) or 0
        if family_id == "ranged" and release_latched then
            spawn_tick = phase == "active" and tick or (tick + phase_ticks)
        end
    end
    return {
        player_index = player_index,
        player_id = player.id,
        slot = state.slot_for_player[player_index] or 0,
        team = player.team,
        is_keeper = player.is_keeper,
        soccer_action = soccer_action_of(player),
        soccer_committed = soccer_committed_of(player),
        family_id = family_id or "none",
        source_sequence = (runtime and runtime.source_sequence) or 0,
        phase = phase,
        phase_ticks = phase_ticks,
        forced_state = (runtime and runtime.forced_state) or "none",
        forced_ticks = runtime and runtime.forced_ticks or 0,
        immunity_ticks = runtime and runtime.immunity_ticks or 0,
        guard_active = phase == "guard",
        telegraph_start_tick = telegraph_start,
        telegraph_end_tick = telegraph_end,
        projected_reach_px = (family and family.reach_px) or 0,
        projected_arc_degrees = (family and family.front_arc_degrees) or 0,
        release_latched = family_id == "ranged" and release_latched,
        projected_spawn_tick = spawn_tick,
        x = player.pos.x,
        y = player.pos.y,
        vx = player.vel.x,
        vy = player.vel.y,
        facing_x = player.facing.x,
        facing_y = player.facing.y,
        radius = player.radius,
    }
end

---@param source_player_id string
---@param source_sequence integer
---@return string
function combat_observation.projectile_id(source_player_id, source_sequence)
    return tostring(#source_player_id)
        .. ":"
        .. source_player_id
        .. "/"
        .. tostring(source_sequence)
end

---@param state MatchState
---@param projectile CombatProjectile
---@return integer
local function field_exit_ticks(state, projectile)
    local step = assert(action_families.ranged.projectile_speed_px_per_second)
        * fixed_clock.TICK_SECONDS
    local lifetime = assert(action_families.ranged.projectile_lifetime_ticks)
    local x, y = projectile.pos.x, projectile.pos.y
    for tick = 1, lifetime do
        x = x + projectile.dir.x * step
        y = y + projectile.dir.y * step
        if x < 0 or x > state.field.w or y < 0 or y > state.field.h then
            return tick
        end
    end
    return lifetime
end

---@param left CombatObservationProjectile
---@param right CombatObservationProjectile
---@return boolean
local function projectile_before(left, right)
    if left.source_sequence ~= right.source_sequence then
        return left.source_sequence < right.source_sequence
    end
    if left.source_player_id ~= right.source_player_id then
        return left.source_player_id < right.source_player_id
    end
    return left.projectile_id < right.projectile_id
end

---@param state MatchState
---@param player_index integer
---@return string
local function formation_slot_id(state, player_index)
    local player = state.players[player_index]
    local formation_id = state.formation[player.team]
    if player.is_keeper then
        return formation_id .. "/keeper"
    end
    local ordinal = 0
    for index, other in ipairs(state.players) do
        if other.team == player.team and not other.is_keeper then
            ordinal = ordinal + 1
            if index == player_index then
                break
            end
        end
    end
    return formation_id .. "/outfield/" .. tostring(ordinal)
end

---@class CombatObservationInput
---@field equipment_held boolean
---@field equipment_pressed boolean
---@field equipment_released boolean

-- Build one player's observation of the authoritative public boundary state.
---@param state MatchState
---@param combat_state CombatMatchState?
---@param player_index integer
---@param policy_id string
---@param last_input CombatObservationInput?
---@return CombatObservation
function combat_observation.build(state, combat_state, player_index, policy_id, last_input)
    assert(type(policy_id) == "string" and policy_id ~= "", "observations need a policy id")
    local player = assert(state.players[player_index], "unknown observation player")
    local runtime = combat_state and combat_state.players[player_index] or nil
    local tick = combat_state and combat_state.tick or state.input_tick

    local player_order = {}
    local teammates = {}
    local opponents = {}
    local anchors = {}
    for index, other in ipairs(state.players) do
        player_order[index] = index
        anchors[index] = {
            player_index = index,
            formation_slot_id = formation_slot_id(state, index),
            anchor_x = other.anchor.x,
            anchor_y = other.anchor.y,
        }
        if index ~= player_index then
            local row = peer_row(state, combat_state, index)
            if other.team == player.team then
                teammates[#teammates + 1] = row
            else
                opponents[#opponents + 1] = row
            end
        end
    end

    local projectiles = {}
    if combat_state then
        local step = assert(action_families.ranged.projectile_speed_px_per_second)
        for _, projectile in ipairs(combat_state.projectiles) do
            local source = state.players[projectile.source_index]
            projectiles[#projectiles + 1] = {
                projectile_id = combat_observation.projectile_id(
                    source.id,
                    projectile.source_sequence
                ),
                source_player_index = projectile.source_index,
                source_player_id = source.id,
                source_team = source.team,
                source_sequence = projectile.source_sequence,
                family_id = projectile.family_id,
                x = projectile.pos.x,
                y = projectile.pos.y,
                dir_x = projectile.dir.x,
                dir_y = projectile.dir.y,
                vx = projectile.dir.x * step,
                vy = projectile.dir.y * step,
                in_flight = true,
                remaining_lifetime_ticks = projectile.remaining_ticks,
                horizon_ticks = math.min(
                    projectile.remaining_ticks,
                    field_exit_ticks(state, projectile)
                ),
            }
        end
        table.sort(projectiles, projectile_before)
    end
    local projectile_order = {}
    for index, projectile in ipairs(projectiles) do
        projectile_order[index] = projectile.projectile_id
    end

    local owner = state.owner and state.players[state.owner] or nil
    local family_id = runtime and runtime.family_id or nil
    local observation = {
        schema = combat_observation.SCHEMA,
        version = combat_observation.VERSION,
        policy_id = policy_id,
        producer_tick = tick,
        observed_tick = tick,
        family_catalog_version = combat_observation.FAMILY_CATALOG_VERSION,
        collision_catalog_version = combat_observation.COLLISION_CATALOG_VERSION,
        player_order = player_order,
        projectile_order = projectile_order,
        self = {
            player_index = player_index,
            player_id = player.id,
            slot = state.slot_for_player[player_index] or 0,
            team = player.team,
            is_keeper = player.is_keeper,
            family_id = family_id or "none",
            source_sequence = (runtime and runtime.source_sequence) or 0,
            soccer_action = soccer_action_of(player),
            soccer_committed = soccer_committed_of(player),
            phase = runtime and runtime.phase or "ready",
            phase_ticks = runtime and runtime.phase_ticks or 0,
            forced_state = (runtime and runtime.forced_state) or "none",
            forced_ticks = runtime and runtime.forced_ticks or 0,
            immunity_ticks = runtime and runtime.immunity_ticks or 0,
            cooldown_ticks = runtime and runtime.cooldown_ticks or 0,
            ready = runtime ~= nil
                and runtime.family_id ~= nil
                and runtime.phase == "ready"
                and runtime.cooldown_ticks == 0
                and runtime.forced_ticks == 0,
            control_held = (runtime and runtime.control_held) == true,
            release_latched = family_id == "ranged"
                and (runtime and runtime.release_latched) == true,
            projected_spawn_tick = 0,
            x = player.pos.x,
            y = player.pos.y,
            vx = player.vel.x,
            vy = player.vel.y,
            facing_x = player.facing.x,
            facing_y = player.facing.y,
            radius = player.radius,
            move_speed = player.move_speed,
            last_equipment_held = (last_input and last_input.equipment_held) == true,
            last_equipment_pressed = (last_input and last_input.equipment_pressed) == true,
            last_equipment_released = (last_input and last_input.equipment_released) == true,
        },
        teammates = teammates,
        opponents = opponents,
        projectiles = projectiles,
        ball = {
            x = state.ball.x,
            y = state.ball.y,
            z = state.ball_z,
            vx = state.ball_vel.x,
            vy = state.ball_vel.y,
            vz = state.ball_vz,
            loose = state.owner == nil,
            owner_player_index = state.owner or 0,
            owner_team = owner and owner.team or "none",
            owner_is_keeper = owner ~= nil and owner.is_keeper == true,
        },
        match = {
            tick = tick,
            tick_rate = fixed_clock.TICK_RATE,
            field_w = state.field.w,
            field_h = state.field.h,
            goal_home_x = state.goal_home.x,
            goal_home_y = state.goal_home.y,
            goal_home_w = state.goal_home.w,
            goal_home_h = state.goal_home.h,
            goal_away_x = state.goal_away.x,
            goal_away_y = state.goal_away.y,
            goal_away_w = state.goal_away.w,
            goal_away_h = state.goal_away.h,
            score_home = state.score.home,
            score_away = state.score.away,
            remaining_ticks = math.floor(state.time_left * fixed_clock.TICK_RATE + 0.5),
            stoppage = state.kickoff_hold > 0,
            stoppage_ticks = math.floor(state.kickoff_hold * fixed_clock.TICK_RATE + 0.5),
            finished = state.finished,
        },
        anchors = anchors,
        digest = "",
    }
    if family_id == "ranged" and observation.self.release_latched then
        local phase = observation.self.phase
        observation.self.projected_spawn_tick = phase == "active" and tick
            or (tick + observation.self.phase_ticks)
    end
    ---@cast observation CombatObservation
    observation.digest = combat_observation.digest(observation)
    return observation
end

---@param parts string[]
---@param row table
---@param fields string[]
local function append_row(parts, row, fields)
    for _, field in ipairs(fields) do
        append(parts, field)
        append(parts, row[field])
    end
end

-- Canonical byte-exact encoding. Two observations encode identically exactly
-- when they carry the same public information.
---@param observation CombatObservation
---@return string
function combat_observation.encode(observation)
    local parts = { "GCCO;" .. tostring(combat_observation.VERSION) .. ";" }
    for _, field in ipairs(combat_observation.ENCODED_FIELDS) do
        append(parts, field)
        if field == "player_order" or field == "projectile_order" then
            local values = observation[field]
            append(parts, #values)
            for _, value in ipairs(values) do
                append(parts, value)
            end
        elseif field == "self" then
            append_row(parts, observation.self, combat_observation.SELF_FIELDS)
        elseif field == "teammates" or field == "opponents" then
            local rows = observation[field]
            append(parts, #rows)
            for _, row in ipairs(rows) do
                append_row(parts, row, combat_observation.PEER_FIELDS)
            end
        elseif field == "projectiles" then
            append(parts, #observation.projectiles)
            for _, row in ipairs(observation.projectiles) do
                append_row(parts, row, combat_observation.PROJECTILE_FIELDS)
            end
        elseif field == "anchors" then
            append(parts, #observation.anchors)
            for _, row in ipairs(observation.anchors) do
                append_row(parts, row, combat_observation.ANCHOR_FIELDS)
            end
        elseif field == "ball" then
            append_row(parts, observation.ball, combat_observation.BALL_FIELDS)
        elseif field == "match" then
            append_row(parts, observation.match, combat_observation.MATCH_FIELDS)
        else
            append(parts, observation[field])
        end
    end
    return table.concat(parts)
end

---@param observation CombatObservation
---@return string
function combat_observation.digest(observation)
    return fnv1a64.hash(combat_observation.encode(observation))
end

---@param value any
---@return boolean
local function is_integer(value)
    return type(value) == "number"
        and value == value
        and value ~= math.huge
        and value ~= -math.huge
        and value == math.floor(value)
end

---@param value any
---@return boolean
local function is_finite(value)
    return type(value) == "number" and value == value and value ~= math.huge and value ~= -math.huge
end

---@param row any
---@param fields string[]
---@param allowed table<string, boolean>
---@param path string
---@return string?
local function check_row(row, fields, allowed, path)
    if type(row) ~= "table" then
        return path .. " must be a table"
    end
    for key in pairs(row) do
        if type(key) ~= "string" or not allowed[key] then
            return path .. " has undeclared field " .. tostring(key)
        end
    end
    for _, field in ipairs(fields) do
        local value = row[field]
        if value == nil then
            return path .. " is missing field " .. field
        end
        local kind = type(value)
        if kind == "number" then
            if not is_finite(value) then
                return path .. "." .. field .. " must be finite"
            end
        elseif kind ~= "string" and kind ~= "boolean" then
            return path .. "." .. field .. " has an unsupported scalar type"
        end
    end
    return nil
end

---@param row table
---@param path string
---@return string?
local function check_enums(row, path)
    if row.family_id ~= nil and not FAMILIES[row.family_id] then
        return path .. " has an unknown family id"
    end
    if row.phase ~= nil and not PHASES[row.phase] then
        return path .. " has an unknown action phase"
    end
    if row.forced_state ~= nil and not FORCED_STATES[row.forced_state] then
        return path .. " has an unknown forced state"
    end
    if row.team ~= nil and not TEAMS[row.team] then
        return path .. " has an unknown team"
    end
    if row.soccer_action ~= nil and not SOCCER_ACTIONS[row.soccer_action] then
        return path .. " has an unknown soccer action"
    end
    return nil
end

-- Strict allowlist. Rejects a missing, duplicate, reordered, or undeclared
-- field, an unknown enum member, and a digest that does not cover the body.
---@param observation any
---@return boolean?, string?
function combat_observation.validate(observation)
    if type(observation) ~= "table" then
        return nil, "observation must be a table"
    end
    for key in pairs(observation) do
        if type(key) ~= "string" or not combat_observation.FIELD_SET[key] then
            return nil, "observation has undeclared field " .. tostring(key)
        end
    end
    for _, field in ipairs(combat_observation.ENCODED_FIELDS) do
        if observation[field] == nil then
            return nil, "observation is missing field " .. field
        end
    end
    if observation.schema ~= combat_observation.SCHEMA then
        return nil, "observation schema tag is not " .. combat_observation.SCHEMA
    end
    if observation.version ~= combat_observation.VERSION then
        return nil, "unsupported combat observation version"
    end
    if type(observation.policy_id) ~= "string" or observation.policy_id == "" then
        return nil, "observation needs a policy id"
    end
    if
        observation.family_catalog_version ~= combat_observation.FAMILY_CATALOG_VERSION
        or observation.collision_catalog_version ~= combat_observation.COLLISION_CATALOG_VERSION
    then
        return nil, "observation catalog versions do not match this build"
    end
    if not is_integer(observation.producer_tick) or not is_integer(observation.observed_tick) then
        return nil, "observation ticks must be integers"
    end

    local err = check_row(
        observation.self,
        combat_observation.SELF_FIELDS,
        combat_observation.SELF_FIELD_SET,
        "observation.self"
    ) or check_enums(observation.self, "observation.self")
    if err then
        return nil, err
    end
    err = check_row(
        observation.ball,
        combat_observation.BALL_FIELDS,
        combat_observation.BALL_FIELD_SET,
        "observation.ball"
    ) or check_row(
        observation.match,
        combat_observation.MATCH_FIELDS,
        combat_observation.MATCH_FIELD_SET,
        "observation.match"
    )
    if err then
        return nil, err
    end

    local seen_index = {}
    for _, key in ipairs({ "teammates", "opponents" }) do
        local rows = observation[key]
        if type(rows) ~= "table" then
            return nil, "observation." .. key .. " must be an array"
        end
        local same_team = key == "teammates"
        local previous = 0
        for index, row in ipairs(rows) do
            local path = "observation." .. key .. "." .. index
            err = check_row(
                row,
                combat_observation.PEER_FIELDS,
                combat_observation.PEER_FIELD_SET,
                path
            ) or check_enums(row, path)
            if err then
                return nil, err
            end
            -- Which array a row sits in IS its team, for every consumer:
            -- `combat_policy.candidates` only ever targets `opponents`,
            -- `teammate_coverage` only counts `teammates`, and
            -- `combat_feasibility.hostile_paths` reads `opponents` as hostile.
            -- Checking the `team` field against the closed vocabulary is
            -- therefore not enough; it has to agree with the array it is in, or
            -- a hand-built table could make the policy treat a teammate as a
            -- target. `build` cannot produce that, but `validate` is what an
            -- adversarial harness (section 4.7) leans on, so it must be the
            -- thing that says no.
            if (row.team == observation.self.team) ~= same_team then
                return nil, path .. " is in the wrong team array for its team"
            end
            if not is_integer(row.player_index) or row.player_index < 1 then
                return nil, path .. " needs a canonical player index"
            end
            if row.player_index <= previous then
                return nil, "observation." .. key .. " rows are not in canonical player order"
            end
            previous = row.player_index
            if seen_index[row.player_index] then
                return nil, "observation has a duplicate player row"
            end
            seen_index[row.player_index] = true
        end
    end
    if seen_index[observation.self.player_index] then
        return nil, "observation repeats the observing player as a peer"
    end

    if type(observation.projectiles) ~= "table" then
        return nil, "observation.projectiles must be an array"
    end
    local seen_projectile = {}
    local previous_projectile ---@type CombatObservationProjectile?
    for index, row in ipairs(observation.projectiles) do
        local path = "observation.projectiles." .. index
        err = check_row(
            row,
            combat_observation.PROJECTILE_FIELDS,
            combat_observation.PROJECTILE_FIELD_SET,
            path
        ) or check_enums(row, path)
        if err then
            return nil, err
        end
        if seen_projectile[row.projectile_id] then
            return nil, "observation has a duplicate projectile row"
        end
        seen_projectile[row.projectile_id] = true
        if previous_projectile and not projectile_before(previous_projectile, row) then
            return nil, "observation.projectiles rows are not in canonical order"
        end
        previous_projectile = row
    end

    if type(observation.anchors) ~= "table" then
        return nil, "observation.anchors must be an array"
    end
    local previous_anchor = 0
    for index, row in ipairs(observation.anchors) do
        err = check_row(
            row,
            combat_observation.ANCHOR_FIELDS,
            combat_observation.ANCHOR_FIELD_SET,
            "observation.anchors." .. index
        )
        if err then
            return nil, err
        end
        if not is_integer(row.player_index) or row.player_index <= previous_anchor then
            return nil, "observation.anchors rows are not in canonical player order"
        end
        previous_anchor = row.player_index
    end

    if type(observation.player_order) ~= "table" then
        return nil, "observation.player_order must be an array"
    end
    for index, value in ipairs(observation.player_order) do
        if value ~= index then
            return nil, "observation.player_order is not the canonical player index order"
        end
    end
    if #observation.player_order ~= #observation.anchors then
        return nil, "observation.player_order does not cover every anchor row"
    end
    if type(observation.projectile_order) ~= "table" then
        return nil, "observation.projectile_order must be an array"
    end
    if #observation.projectile_order ~= #observation.projectiles then
        return nil, "observation.projectile_order does not match the projectile rows"
    end
    for index, id in ipairs(observation.projectile_order) do
        if id ~= observation.projectiles[index].projectile_id then
            return nil, "observation.projectile_order does not follow the projectile rows"
        end
    end

    if type(observation.digest) ~= "string" then
        return nil, "observation digest must be a string"
    end
    if observation.digest ~= combat_observation.digest(observation) then
        return nil, "observation digest does not cover its body"
    end
    return true
end

return combat_observation
