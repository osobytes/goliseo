-- Driver-level boundary zeroes for the seven combat correction phases #166
-- names: wind-up, guard, contact, projectile flight, stagger/knockback, ball
-- spill, and immunity expiry.
--
-- Why this exists. `game.online.match_driver_fixture` pins one combat-active
-- boundary zero, and from it the AI-driven slots reach *some* combat, eventually,
-- somewhere on the pitch. That is enough to prove the companion survives a
-- correction; it is not enough to pin a named phase, because nothing in the
-- fixture decides which phase happens or when. Each scenario here rigs boundary
-- zero -- equipped families, opening pose, ball ownership -- so the phase it is
-- named for actually occurs inside a headless run, and supplies the predicate
-- that recognises it.
--
-- Two routes reach a phase, and each scenario declares which one it uses:
--
--   `policy`          -- the phase is produced by `gameplay_ai/combat/v1` itself,
--                        through `sim.slot_input`'s `bot_combat_signals`. Nothing
--                        but the opening pose is arranged; the decision, the
--                        commit and the resolution are the shipped policy's.
--   `canonical_input` -- the phase is produced by a live slot's own equipment
--                        input, carried on the canonical input stream like any
--                        other authority. Used only where the policy cannot be
--                        steered into the phase deterministically from a fixed
--                        boundary zero (see `guard`).
--
-- Neither route force-sets a combat runtime field. Every scenario's boundary
-- zero is a `ready` combat state, so what a correction resimulates through is a
-- phase the simulation entered on its own, from authority every peer holds.
--
-- Nothing here iterates a hash-keyed table in a way that reaches an output.
-- Player pools are walked with `ipairs` over `data.players`, and the per-scenario
-- tables are read by explicit key. Two peers in one process share `pairs()`
-- order, so agreement produced by it would be an accident rather than a proof.
--
-- Usage. A caller owns its own driver harness and delivery pattern; this module
-- owns the fixture and the predicates:
--
--   local phases = require("spec.support.online_combat_phases")
--   local snapshot = phases.boundary_zero("projectile_flight")   -- every peer
--   ... drive drivers built on `snapshot`, bursting delivery ...
--   -- for a resimulated input tick T on some peer:
--   phases.observed("projectile_flight", snapshot_at(T), snapshot_at(T + 1), events_at(T))
--
-- `live_sample(id, step, index)` is the live slot's input program for the
-- scenario; it is `input_frame.neutral_sample()` for every `policy` scenario.

local Vec2 = require("core.vec2")
local combat = require("sim.combat")
local input_frame = require("sim.input_frame")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local players_data = require("data.players")
local teams = require("data.teams")

---@alias OnlineCombatPhaseId
---| "windup"
---| "guard"
---| "contact"
---| "projectile_flight"
---| "stagger"
---| "ball_spill"
---| "immunity_expiry"

---@alias OnlineCombatPhaseRoute "policy"|"canonical_input"

---@class OnlineCombatPhaseScenario
---@field id OnlineCombatPhaseId
---@field route OnlineCombatPhaseRoute
---@field home_family ActionFamilyId -- Loadout family every home outfielder carries.
---@field away_family ActionFamilyId -- Loadout family every away outfielder carries.
---@field separation_px number -- Opening gap between the two facing lines.
---@field row_spacing_px number -- Vertical gap between rows; a small one is a scrum.
---@field steps integer -- Driver steps the scenario needs to reach the phase.
---@field deliver_period integer -- Transport drains every Nth step; the burst that corrects.
---@field hold_equipment boolean -- The host's live slot holds equipment on the canonical stream.
---@field note string -- Why this scenario reaches its phase by the route it declares.

---@class OnlineCombatPhaseModule
local combat_phases = {}

-- Player indexes in a nebula-versus-orion `sim.match` state. 1 and 6 are the
-- protected keepers, which are slotless and never combat-capable.
local HOME_OUTFIELD = { 2, 3, 4, 5 }
local AWAY_OUTFIELD = { 7, 8, 9, 10 }

-- One loadout per family, matching `spec/sim/combat_ai_match_spec.lua` so a
-- scenario here and a scenario there mean the same thing by "light_melee".
---@type table<ActionFamilyId, string>
local LOADOUT_FOR = {
    unarmed = "loadout_spring_gloves",
    guard = "loadout_emberguard_shield",
    light_melee = "loadout_vector_blade",
    ranged = "loadout_pulse_blaster",
}

combat_phases.FIELD = { w = 960, h = 540 }
-- Long enough that no scenario can reach full time and turn a correction
-- question into a settle question.
combat_phases.DURATION_SECONDS = 14
combat_phases.SEED = 74
-- The opening lane the two facing lines are laid out along.
local LINE_X = 400

-- Canonical order, and the order a caller should report results in.
---@type OnlineCombatPhaseId[]
combat_phases.PHASES = {
    "windup",
    "guard",
    "contact",
    "projectile_flight",
    "stagger",
    "ball_spill",
    "immunity_expiry",
}

---@type table<OnlineCombatPhaseId, OnlineCombatPhaseScenario>
local SCENARIOS = {
    windup = {
        id = "windup",
        route = "policy",
        home_family = "light_melee",
        away_family = "light_melee",
        separation_px = 36,
        row_spacing_px = 120,
        steps = 240,
        deliver_period = 5,
        hold_equipment = false,
        note = "Light melee telegraphs the longest melee wind-up (12 ticks), so the "
            .. "policy's own commit leaves a wide window for a burst to land inside.",
    },
    guard = {
        id = "guard",
        route = "canonical_input",
        home_family = "guard",
        away_family = "light_melee",
        separation_px = 30,
        row_spacing_px = 120,
        steps = 240,
        deliver_period = 5,
        hold_equipment = true,
        note = "The policy raises a guard only while it can attribute a telegraphed "
            .. "hostile path to a purpose target (`sim.combat_feasibility`'s guard "
            .. "witness), which needs the threat re-armed inside the deciding player's scan "
            .. "cadence. `spec/sim/combat_ai_match_spec.lua` does that by re-pinning the "
            .. "hostile every tick; a driver-level scenario cannot, because boundary zero "
            .. "is the only thing it controls. So the guard here is raised by a live "
            .. "slot's own held equipment, which reaches the resolver through the "
            .. "canonical input stream rather than by mutating a runtime field.",
    },
    contact = {
        id = "contact",
        route = "policy",
        home_family = "unarmed",
        away_family = "unarmed",
        separation_px = 24,
        row_spacing_px = 28,
        steps = 480,
        deliver_period = 8,
        hold_equipment = false,
        note = "A scrum: eight unarmed outfielders inside one 30 px reach of each "
            .. "other and of the ball. Unarmed has the shortest wind-up and the tightest "
            .. "reach, so the policy's commits land contacts instead of missing.",
    },
    projectile_flight = {
        id = "projectile_flight",
        route = "policy",
        home_family = "ranged",
        away_family = "ranged",
        separation_px = 200,
        row_spacing_px = 120,
        steps = 240,
        deliver_period = 5,
        hold_equipment = false,
        note = "Two ranged lines at shooting distance. A projectile lives 60 ticks, "
            .. "so flight covers most of the run once the first shot is released.",
    },
    stagger = {
        id = "stagger",
        route = "policy",
        home_family = "light_melee",
        away_family = "light_melee",
        separation_px = 36,
        row_spacing_px = 120,
        steps = 240,
        deliver_period = 5,
        hold_equipment = false,
        note = "Light melee displaces 18 px on an unguarded hit, over the knockback "
            .. "threshold, and interrupts for 18 ticks -- the widest forced window any "
            .. "family produces.",
    },
    ball_spill = {
        id = "ball_spill",
        route = "policy",
        home_family = "unarmed",
        away_family = "unarmed",
        separation_px = 24,
        row_spacing_px = 28,
        steps = 600,
        deliver_period = 8,
        hold_equipment = false,
        note = "A spill is the narrowest event of the seven: it needs an unguarded "
            .. "contact on the carrier specifically, not on whoever is nearest. The "
            .. "contact scrum produces those, and this runs it longer so a burst covers "
            .. "several of them rather than the one.",
    },
    immunity_expiry = {
        id = "immunity_expiry",
        route = "policy",
        home_family = "unarmed",
        away_family = "unarmed",
        separation_px = 24,
        row_spacing_px = 28,
        steps = 480,
        deliver_period = 8,
        hold_equipment = false,
        note = "The same scrum as `contact`, read one tick later: immunity is granted "
            .. "by a landed contact and counts down to zero over `combat.IMMUNITY_TICKS`, "
            .. "so the family that lands the most contacts also expires the most "
            .. "immunities.",
    },
}

---@param id OnlineCombatPhaseId
---@return OnlineCombatPhaseScenario
function combat_phases.scenario(id)
    return assert(SCENARIOS[id], "unknown online combat phase: " .. tostring(id))
end

-- `PlayerData`'s complete field set, copied by name rather than by iteration.
-- A new field added to `data/players.lua` would otherwise be dropped silently
-- from every scenario's pool, so the copy asserts against this list instead.
local PLAYER_FIELDS = {
    "id",
    "name",
    "number",
    "position",
    "stats",
    "presentation_id",
    "cosmetic_variant_id",
    "loadout_id",
}

-- Every outfielder on a side carries the same family, so a scenario exercises
-- exactly one pair of them. The copy is built by walking `PLAYER_FIELDS` with
-- `ipairs`; the `pairs` pass below only rejects an unknown key and cannot
-- influence the result, whatever order it runs in.
---@param scenario OnlineCombatPhaseScenario
---@return table<string, PlayerData>
local function pool_for(scenario)
    ---@type table<string, boolean>
    local known = {}
    for _, key in ipairs(PLAYER_FIELDS) do
        known[key] = true
    end
    ---@type table<string, boolean>
    local home_ids = {}
    for _, id in ipairs(teams.nebula.roster) do
        home_ids[id] = true
    end
    ---@type table<string, PlayerData>
    local by_id = {}
    for _, player in ipairs(players_data) do
        for key in pairs(player) do
            assert(known[key], "data.players grew an unhandled field: " .. tostring(key))
        end
        ---@type table<string, any>
        local copied = {}
        for _, key in ipairs(PLAYER_FIELDS) do
            copied[key] = player[key]
        end
        if copied.loadout_id ~= nil then
            local family = home_ids[player.id] and scenario.home_family or scenario.away_family
            copied.loadout_id = assert(LOADOUT_FOR[family], "no loadout for family " .. family)
        end
        ---@cast copied PlayerData
        by_id[player.id] = copied
    end
    return by_id
end

-- Two facing lines centred on the pitch, one outfielder per row, with the ball
-- on the first away outfielder. Every scenario opens from the same shape and
-- differs only in the families carrying it, how far apart the lines stand, and
-- how tightly the rows are stacked -- a wide `row_spacing_px` is four separate
-- duels, a narrow one is a scrum where every body is inside every reach. What a
-- scenario proves is therefore attributable to the family rather than to a pose
-- invented for it.
---@param state MatchState
---@param scenario OnlineCombatPhaseScenario
local function pose(state, scenario)
    local spacing = scenario.row_spacing_px
    local top = combat_phases.FIELD.h / 2 - spacing * 1.5
    for order, index in ipairs(HOME_OUTFIELD) do
        local player = state.players[index]
        player.pos = Vec2.new(LINE_X, top + (order - 1) * spacing)
        player.facing = Vec2.new(1, 0)
        player.vel = Vec2.new(0, 0)
        player.run_vel = Vec2.new(0, 0)
    end
    for order, index in ipairs(AWAY_OUTFIELD) do
        local player = state.players[index]
        player.pos = Vec2.new(LINE_X + scenario.separation_px, top + (order - 1) * spacing)
        player.facing = Vec2.new(-1, 0)
        player.vel = Vec2.new(0, 0)
        player.run_vel = Vec2.new(0, 0)
    end
    local carrier = AWAY_OUTFIELD[1]
    state.owner = carrier
    state.ball = state.players[carrier].pos
    state.ball_vel = Vec2.new(0, 0)
    -- The kickoff hold refuses every equipment request, so a scenario that spent
    -- it would spend its whole run waiting instead of fighting.
    state.kickoff_hold = 0
end

-- The shared, combat-active boundary zero for one phase. Every peer in a session
-- must be given the *same* snapshot: a differing one is a desync fixture, not a
-- correction fixture.
---@param id OnlineCombatPhaseId
---@param duration number? -- Match seconds; defaults to `combat_phases.DURATION_SECONDS`.
---@return MatchSnapshot
function combat_phases.boundary_zero(id, duration)
    local scenario = combat_phases.scenario(id)
    local by_id = pool_for(scenario)
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = combat_phases.FIELD.w, h = combat_phases.FIELD.h },
        duration = duration or combat_phases.DURATION_SECONDS,
        max_goals = 99,
        seed = combat_phases.SEED,
        players_by_id = by_id,
        input_ownership = match.ownership_for_teams(teams.nebula, teams.orion, by_id),
    })
    pose(state, scenario)
    return match_snapshot.capture(state, combat.new_state(state, by_id))
end

-- The live slot's input for one driver step. Driver index 1 is the host, whose
-- opening live slot is `home_1`; every other index gets a neutral sample.
--
-- Only the `guard` scenario authors anything: it presses equipment once and then
-- holds it, which is a human raising a shield and keeping it up. The press edge
-- is re-sent periodically so an interrupted guard is raised again rather than
-- leaving the rest of the run neutral.
---@param id OnlineCombatPhaseId
---@param step integer -- Zero-based driver step.
---@param index integer -- Driver index; 1 is the host.
---@return InputSample
function combat_phases.live_sample(id, step, index)
    local scenario = combat_phases.scenario(id)
    if not scenario.hold_equipment or index ~= 1 then
        return input_frame.neutral_sample()
    end
    return assert(input_frame.new_sample({
        held = input_frame.HELD_BITS.equipment,
        edges = step % 30 == 0 and input_frame.EDGE_BITS.equipment_pressed or 0,
    }))
end

---@param snapshot MatchSnapshot
---@return CombatMatchState
local function companion(snapshot)
    return assert(snapshot.combat, "a combat phase scenario needs a combat-bearing snapshot")
end

---@param snapshot MatchSnapshot
---@param phase CombatActionPhase
---@return boolean
local function any_phase(snapshot, phase)
    for _, runtime in ipairs(companion(snapshot).players) do
        if runtime.phase == phase then
            return true
        end
    end
    return false
end

---@param events CombatEvent[]
---@param kind CombatEventKind
---@return boolean
local function any_event(events, kind)
    for _, event in ipairs(events) do
        if event.kind == kind then
            return true
        end
    end
    return false
end

-- Did input tick `tick` run through the named phase?
--
-- `before` is the snapshot at boundary `tick` -- the state the tick was
-- simulated *from* -- and `after` is the snapshot at boundary `tick + 1`.
-- `events` are the combat events that tick emitted. A caller asks this about a
-- tick a correction resimulated, so a true answer means the correction carried
-- the companion through that phase and landed on the state every peer agrees on.
---@param id OnlineCombatPhaseId
---@param before MatchSnapshot
---@param after MatchSnapshot
---@param events CombatEvent[]
---@return boolean
function combat_phases.observed(id, before, after, events)
    if id == "windup" then
        return any_phase(before, "windup")
    elseif id == "guard" then
        return any_phase(before, "guard")
    elseif id == "contact" then
        return any_event(events, "contact")
    elseif id == "projectile_flight" then
        -- Already in flight when the tick began, so the tick advanced it. A
        -- projectile that appears only in `after` was spawned by this tick and
        -- has not flown a pixel yet, which is a spawn rather than flight.
        return #companion(before).projectiles > 0
    elseif id == "stagger" then
        for _, runtime in ipairs(companion(before).players) do
            if runtime.forced_state ~= nil and runtime.forced_ticks > 0 then
                return true
            end
        end
        return false
    elseif id == "ball_spill" then
        return any_event(events, "ball_spill")
    end
    assert(id == "immunity_expiry", "unknown online combat phase: " .. tostring(id))
    -- The expiry itself, not merely a live immunity: the counter has to reach
    -- zero inside the resimulated tick.
    local entering = companion(before).players
    local leaving = companion(after).players
    for index, runtime in ipairs(entering) do
        local next_runtime = leaving[index]
        if
            next_runtime ~= nil
            and runtime.immunity_ticks > 0
            and next_runtime.immunity_ticks == 0
        then
            return true
        end
    end
    return false
end

return combat_phases
