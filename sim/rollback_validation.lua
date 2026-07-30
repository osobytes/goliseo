-- Pure OMP-2 validation campaign and deterministic scenario registry adapter.
-- Runtime clocks, process memory, browser identity, and game-layer consumers
-- remain outside this module.

local fnv1a64 = require("core.fnv1a64")
local Vec2 = require("core.vec2")
local config = require("data.omp2_rollback_validation")
local network_profiles = require("data.network_profiles")
local player_pool = require("data.players")
local teams = require("data.teams")
local combat = require("sim.combat")
local combat_identity = require("sim.combat_identity")
local determinism_evidence = require("sim.determinism_evidence")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local input_tape = require("sim.input_tape")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local rollback_lab = require("sim.rollback_lab")
local tuning = require("sim.tuning")

---@alias RollbackValidationSuite
---| "native"
---| "browser-full"
---| "browser-stress"
---| "late-window"
---| "soak"

---@class RollbackValidationOptions
---@field profile_name string?
---@field network_seed integer?
---@field measure RollbackSessionMeasure?

---@class RollbackValidationCaseSpec
---@field id string
---@field scenario string
---@field tape InputTape
---@field options RollbackLabOptions
---@field expected_failure boolean
---@field sample string?

---@class RollbackValidationCompletedCase
---@field id string
---@field scenario string
---@field initial_snapshot MatchSnapshot
---@field result RollbackLabResult
---@field expected_failure boolean
---@field accepted boolean
---@field hidden_progress boolean
---@field scenario_pass boolean
---@field sample string?

---@class RollbackValidationCampaign
---@field suite RollbackValidationSuite
---@field cases RollbackValidationCaseSpec[]
---@field next_case integer
---@field active RollbackLabCampaign?
---@field active_spec RollbackValidationCaseSpec?
---@field completed integer
---@field failed boolean
---@field logical Fnv1a64State
---@field result RollbackValidationResult?

---@class RollbackValidationResult
---@field schema integer
---@field suite RollbackValidationSuite
---@field success boolean
---@field case_count integer
---@field logical_digest string

--- Per-slot equipment cadence for a combat load fixture.
--- `offset` is the first tick that presses, `period` the ticks between presses, and
--- `hold` how long equipment stays held before the release edge. One shape drives all
--- four families: a press family ignores the hold, `guard` spends it guarding, and
--- `ranged` needs it to outlast the windup so the release lands in the aim phase.
--- `move_x` is a quantized lateral push applied only for the first `approach` ticks of
--- each cycle, which is what keeps a scrum crowded. Neither extreme works: with no push
--- at all the first landed hit displaces its target out of reach and the tape spends its
--- remaining ticks idle, while a push held every tick walks the two lines straight
--- through each other -- bodies do not collide -- and they separate for good. A short
--- burst per cycle recovers roughly the displacement the cycle just inflicted.
---@class Omp2CombatLoadSlotPlan
---@field offset integer
---@field period integer
---@field hold integer
---@field move_x integer
---@field approach integer

--- Start-of-tape body positions, in `MatchState.players` order: home keeper, the four
--- home outfielders in roster order, then the away side the same way.
---
--- `crowded` is the ten-player mixed-family scrum. It follows the crowding pattern of
--- `game/presentation/combat_feedback_fixture.lua` -- opposing families interleaved
--- inside one contested pocket rather than spread over the pitch -- without depending
--- on it, because `sim/` may not require `game/`.
---
--- The rows are paired deliberately rather than merely crowded. `select_melee_target`
--- takes the nearest legal target along the attacker's facing, so pairing each melee
--- family opposite a *guard* is the only way the guarded-contact and guard-recoil paths
--- are exercised at all; a symmetric layout gives four unguarded exchanges and never
--- touches them. Rows read: guard against light melee, light melee against guard, a
--- ranged duel, and an unarmed exchange. Every gap is 38 px, inside light melee's
--- 42 px reach plus the 12 px target radius and inside unarmed's 30 px plus the same.
---
--- `pocket` packs all eight outfielders into one 40 px-deep contest, four a side, for
--- the repeated-family load. Every body carries the same loadout there, so unlike
--- `crowded` there is no pairing to respect -- the point is the density of simultaneous
--- same-family resolutions rather than the variety of them.
---@type table<string, number[][]>
local COMBAT_LOAD_LAYOUTS = {
    crowded = {
        { 95, 270 },
        { 452, 240 },
        { 452, 290 },
        { 452, 200 },
        { 452, 340 },
        { 865, 270 },
        { 490, 288 },
        { 490, 242 },
        { 490, 202 },
        { 490, 338 },
    },
    pocket = {
        { 95, 270 },
        { 460, 210 },
        { 460, 250 },
        { 460, 290 },
        { 460, 330 },
        { 865, 270 },
        { 500, 212 },
        { 500, 252 },
        { 500, 292 },
        { 500, 332 },
    },
}

--- Where the ball starts. It sits clear of the contest on purpose, so the measured load
--- is combat rather than a possession scramble, and because a carried ball costs
--- retained snapshot bytes the pinned budget has no room for -- see the budget note on
--- `combat_load_tape`. The cost of parking it is one `ball_spill` event the crowded
--- fixture would otherwise cover.
---@type number[]
local COMBAT_LOAD_BALL = { 480, 470 }

--- Input cadence per slot, in canonical slot order (four home, then four away).
---@type table<string, Omp2CombatLoadSlotPlan[]>
local COMBAT_LOAD_SLOT_PLANS = {
    crowded = {
        { offset = 0, period = 40, hold = 30, move_x = 38, approach = 5 },
        { offset = 4, period = 46, hold = 1, move_x = 38, approach = 5 },
        { offset = 8, period = 60, hold = 20, move_x = 38, approach = 4 },
        { offset = 2, period = 30, hold = 1, move_x = 38, approach = 6 },
        { offset = 0, period = 40, hold = 30, move_x = -38, approach = 5 },
        { offset = 4, period = 46, hold = 1, move_x = -38, approach = 5 },
        { offset = 12, period = 60, hold = 20, move_x = -38, approach = 4 },
        { offset = 6, period = 30, hold = 1, move_x = -38, approach = 6 },
    },
    pocket = {
        { offset = 0, period = 30, hold = 1, move_x = 38, approach = 6 },
        { offset = 5, period = 30, hold = 1, move_x = 38, approach = 6 },
        { offset = 10, period = 30, hold = 1, move_x = 38, approach = 6 },
        { offset = 15, period = 30, hold = 1, move_x = 38, approach = 6 },
        { offset = 20, period = 30, hold = 1, move_x = -38, approach = 6 },
        { offset = 25, period = 30, hold = 1, move_x = -38, approach = 6 },
        { offset = 0, period = 30, hold = 1, move_x = -38, approach = 6 },
        { offset = 5, period = 30, hold = 1, move_x = -38, approach = 6 },
    },
}

---@class RollbackValidationModule
local rollback_validation = {}

rollback_validation.SCHEMA = 1

---@param value any
---@return boolean
local function is_integer(value)
    return type(value) == "number"
        and value == value
        and value ~= math.huge
        and value ~= -math.huge
        and value == math.floor(value)
end

---@return RollbackInputSource[]
local function sources()
    local result = { "local" }
    for slot = 2, input_frame.SLOT_COUNT do
        result[slot] = "remote"
    end
    return result
end

---@param source InputTape
---@param first_boundary integer
---@return MatchState
---@return CombatMatchState?
local function state_at(source, first_boundary)
    local state, combat_state = match_snapshot.restore(source.initial)
    for index = 1, first_boundary do
        match.step(state, fixed_clock.TICK_SECONDS, source.frames[index], combat_state)
    end
    return state, combat_state
end

---@param source InputTape
---@param first_boundary integer
---@param last_boundary integer
---@param scenario string
---@return InputTape
local function normalized_window(source, first_boundary, last_boundary, scenario)
    assert(
        first_boundary >= 0 and last_boundary > first_boundary and last_boundary <= #source.frames,
        "rollback validation window is outside the frozen tape"
    )
    local state, combat_state = state_at(source, first_boundary)
    state.input_tick = 0
    state.events = {}
    if combat_state then
        combat_state.tick = 0
        combat_state.events = {}
    end
    local initial = match_snapshot.capture(state, combat_state)
    local frames = {}
    for boundary = first_boundary, last_boundary - 1 do
        local frame = assert(input_frame.copy(source.frames[boundary + 1]))
        frame.tick = #frames
        assert(input_frame.validate(frame))
        frames[#frames + 1] = frame
    end
    local identity = input_tape.copy_identity(source.identity)
    identity.build = "omp2-rollback-validation-v1"
    identity.source = "omp2-normalized-" .. scenario
    identity.fixture = "omp2-" .. scenario
    identity.config = identity.config .. ";normalized_boundary=" .. first_boundary
    return input_tape.new(identity, initial, frames)
end

---@return InputTape
local function synthetic_goal_tape()
    local ownership = match.ownership_for_teams(teams.nebula, teams.orion)
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        duration = 2,
        max_goals = 3,
        seed = 83,
        input_ownership = ownership,
    })
    local away_keeper = state.players[6]
    away_keeper.keeper_state = "retreat"
    away_keeper.keeper_state_timer = 0.1
    away_keeper.keeper_release_state = "advance"
    away_keeper.keeper_release_motion = 0.5
    away_keeper.keeper_release_kind = "chip"
    away_keeper.keeper_release_depth = 42
    away_keeper.receive_timer = 1
    state.owner = nil
    state.ball = Vec2.new(965, 270)
    state.ball_vel = Vec2.new(600, 0)
    state.ball_z = 0
    state.ball_vz = 0
    state.pickup_cd = 1
    state.block_grace = 1
    local frames = {
        assert(input_frame.neutral(0)),
        assert(input_frame.neutral(1)),
        assert(input_frame.neutral(2)),
    }
    local identity = {
        tape_version = input_tape.VERSION,
        input_version = input_frame.VERSION,
        snapshot_version = match_snapshot.VERSION,
        build = "omp2-rollback-validation-v1",
        source = "omp2-synthetic-goal-kickoff-v1",
        content = "nebula-orion-showcase-content-v1",
        tuning = tuning.serialize(),
        config = "field=960x540;duration=2;max_goals=3;tick_rate=60",
        fixture = "omp2-goal-kickoff",
        seed = 83,
        tick_rate = fixed_clock.TICK_RATE,
        ownership = ownership,
    }
    return input_tape.new(identity, match_snapshot.capture(state), frames)
end

---@return InputTape
local function combat_validation_tape()
    local fixture = config.combat_fixture
    local ownership = match.ownership_for_teams(teams.nebula, teams.orion)
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        duration = 20,
        max_goals = 99,
        seed = fixture.seed,
        input_ownership = ownership,
    })
    state.kickoff_hold = 0
    local combat_state = combat.new_state(state)
    local initial = match_snapshot.capture(state, combat_state)
    local frames = {}
    for tick = 0, fixture.frame_count - 1 do
        local frame = assert(input_frame.neutral(tick))
        for slot = 1, input_frame.SLOT_COUNT do
            if tick == 0 then
                frame.slots[slot] = assert(input_frame.new_sample({
                    held = input_frame.HELD_BITS.equipment,
                    edges = input_frame.EDGE_BITS.equipment_pressed,
                }))
            elseif tick < 20 then
                frame.slots[slot] = assert(input_frame.new_sample({
                    held = input_frame.HELD_BITS.equipment,
                }))
            elseif tick == 20 then
                frame.slots[slot] = assert(input_frame.new_sample({
                    edges = input_frame.EDGE_BITS.equipment_released,
                }))
            end
        end
        frames[#frames + 1] = frame
    end
    local identity = {
        tape_version = input_tape.COMBAT_VERSION,
        input_version = input_frame.VERSION,
        snapshot_version = match_snapshot.COMBAT_VERSION,
        build = "omp2-combat-rollback-v1",
        source = "issue-111-bounded-combat-v1",
        content = "nebula-orion-showcase-content-v1",
        tuning = tuning.serialize(),
        config = ("field=960x540;duration=20;max_goals=99;tick_rate=60;ticks=%d"):format(
            fixture.frame_count
        ),
        fixture = fixture.id,
        seed = fixture.seed,
        tick_rate = fixed_clock.TICK_RATE,
        ownership = ownership,
        combat = combat_identity.for_state(combat_state),
    }
    local tape = input_tape.new(identity, initial, frames)
    local actual_initial = tape.boundary_hashes[1]
    local actual_final = tape.boundary_hashes[#tape.boundary_hashes]
    local actual_digest = rollback_lab.tape_digest(tape)
    assert(
        actual_initial == fixture.initial_hash
            and actual_final == fixture.final_hash
            and actual_digest == fixture.tape_digest,
        ("combat validation identity changed: initial=%s final=%s digest=%s"):format(
            actual_initial,
            actual_final,
            actual_digest
        )
    )
    return tape
end

--- A roster index whose outfielders all carry one loadout, so a fixture can demand a
--- repeated action family the authored mixed roster deliberately never produces.
--- Keepers keep their absent loadout: `sim.combat` refuses them an action anyway, and
--- rewriting them would put a combat identity on a body that can never use it.
---@param loadout_id string
---@return table<string, PlayerData>
local function repeated_family_roster(loadout_id)
    local by_id = {}
    for _, player in ipairs(player_pool) do
        ---@type PlayerData
        local copy = {
            id = player.id,
            name = player.name,
            number = player.number,
            position = player.position,
            stats = player.stats,
            presentation_id = player.presentation_id,
            cosmetic_variant_id = player.cosmetic_variant_id,
            loadout_id = player.loadout_id ~= nil and loadout_id or nil,
        }
        by_id[copy.id] = copy
    end
    return by_id
end

---@param fixture Omp2RollbackCombatLoadFixture
---@return InputFrame[]
local function combat_load_frames(fixture)
    local plans = assert(
        COMBAT_LOAD_SLOT_PLANS[fixture.layout],
        "unknown combat load layout: " .. tostring(fixture.layout)
    )
    local frames = {}
    for tick = 0, fixture.frame_count - 1 do
        local frame = assert(input_frame.neutral(tick))
        for slot = 1, input_frame.SLOT_COUNT do
            local plan = assert(plans[slot], "combat load layout is missing a slot plan")
            assert(plan.hold < plan.period, "combat load hold must end inside its period")
            local held = 0
            local edges = 0
            local move_x = 0
            local elapsed = tick - plan.offset
            if elapsed >= 0 then
                local cycle = elapsed % plan.period
                if cycle == 0 then
                    held = input_frame.HELD_BITS.equipment
                    edges = input_frame.EDGE_BITS.equipment_pressed
                elseif cycle < plan.hold then
                    held = input_frame.HELD_BITS.equipment
                elseif cycle == plan.hold then
                    edges = input_frame.EDGE_BITS.equipment_released
                end
                if cycle < plan.approach then
                    move_x = plan.move_x
                end
            end
            frame.slots[slot] = assert(input_frame.new_sample({
                move_x = move_x,
                held = held,
                edges = edges,
            }))
        end
        frames[#frames + 1] = frame
    end
    return frames
end

--- Build one crowded combat load tape, or the same-seed combat-disabled twin of one.
---
--- The twin is not a second fixture with similar settings: it is this function with
--- `fixture.combat` false, so the match options, seed, body layout, ball and the whole
--- frame sequence are shared by construction and only the CombatMatchState companion
--- differs. That is what makes the paired cost attributable to combat rather than to
--- two workloads that merely resemble each other. Without a companion the equipment
--- bits are inert, and the tape is an ordinary soccer tape on InputTape v1 /
--- MatchSnapshot v11.
---
--- These fixtures were designed against a hard ceiling. `budgets.snapshot_bytes` caps the
--- 31-boundary retained window and `main.lua` applies that gate to every case regardless
--- of profile. At 768 KiB the existing `omp2-combat-rollback-v1` measured 779,362 bytes of
--- it, leaving about 7 KB across 31 boundaries -- a couple of hundred bytes per snapshot.
--- Three consequences are baked in above: the ball is parked rather than carried, the tapes
--- are 160 ticks rather than longer (retained bytes grow with tick count), and neither
--- fixture sustains more than about two concurrent projectiles.
---
--- #209 has since raised the gate to 896 KiB, so those three constraints are no longer
--- forced -- but they are still what these committed fixtures measure, and the pinned
--- hashes above describe exactly that shape. Relaxing any of them is new fixture work with
--- its own evidence, not an edit here. The cost model and the lever decision are in
--- docs/online/omp2_rollback_validation.md; narrowing the encoding is #282.
---@param fixture Omp2RollbackCombatLoadFixture
---@return InputTape
local function combat_load_tape(fixture)
    local layout = assert(
        COMBAT_LOAD_LAYOUTS[fixture.layout],
        "unknown combat load layout: " .. tostring(fixture.layout)
    )
    local players_by_id = fixture.repeated_loadout_id
        and repeated_family_roster(fixture.repeated_loadout_id)
    local ownership = match.ownership_for_teams(teams.nebula, teams.orion)
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        duration = fixture.duration,
        max_goals = 99,
        seed = fixture.seed,
        input_ownership = ownership,
    })
    state.kickoff_hold = 0
    state.owner = nil
    state.ball = Vec2.new(COMBAT_LOAD_BALL[1], COMBAT_LOAD_BALL[2])
    state.ball_vel = Vec2.new(0, 0)
    for index, player in ipairs(state.players) do
        local position = assert(layout[index], "combat load layout does not cover the match")
        player.pos = Vec2.new(position[1], position[2])
        player.facing = Vec2.new(player.team == "home" and 1 or -1, 0)
    end

    local combat_state = fixture.combat and combat.new_state(state, players_by_id) or nil
    local initial = match_snapshot.capture(state, combat_state)
    local identity = {
        tape_version = combat_state and input_tape.COMBAT_VERSION or input_tape.VERSION,
        input_version = input_frame.VERSION,
        snapshot_version = combat_state and match_snapshot.COMBAT_VERSION or match_snapshot.VERSION,
        build = fixture.id,
        source = "issue-150-combat-load-v1",
        content = "nebula-orion-showcase-content-v1",
        tuning = tuning.serialize(),
        config = ("field=960x540;duration=%d;max_goals=99;tick_rate=60;ticks=%d;layout=%s;loadout=%s"):format(
            fixture.duration,
            fixture.frame_count,
            fixture.layout,
            fixture.repeated_loadout_id or "roster"
        ),
        fixture = fixture.id,
        seed = fixture.seed,
        tick_rate = fixed_clock.TICK_RATE,
        ownership = ownership,
        combat = combat_state and combat_identity.for_state(combat_state) or nil,
    }
    local tape = input_tape.new(identity, initial, combat_load_frames(fixture))
    local actual_initial = tape.boundary_hashes[1]
    local actual_final = tape.boundary_hashes[#tape.boundary_hashes]
    local actual_digest = rollback_lab.tape_digest(tape)
    assert(
        actual_initial == fixture.initial_hash
            and actual_final == fixture.final_hash
            and actual_digest == fixture.tape_digest,
        ("%s identity changed: initial=%s final=%s digest=%s"):format(
            fixture.id,
            actual_initial,
            actual_final,
            actual_digest
        )
    )
    return tape
end

---@return InputTape
local function late_window_tape()
    local source = determinism_evidence.fixture_tape()
    local initial = match_snapshot.capture(match_snapshot.restore(source.initial))
    local frames = {}
    for tick = 0, 39 do
        local slots = {}
        for slot = 1, input_frame.SLOT_COUNT do
            slots[slot] = input_frame.neutral_sample()
        end
        slots[2] = assert(input_frame.new_sample({
            move_x = tick % 2 == 0 and input_frame.MOVE_SCALE or -input_frame.MOVE_SCALE,
        }))
        frames[#frames + 1] = assert(input_frame.new(tick, slots))
    end
    local identity = input_tape.copy_identity(source.identity)
    identity.build = "omp2-rollback-validation-v1"
    identity.source = "omp2-late-window-v1"
    identity.fixture = "omp2-late-window"
    identity.config = "field=960x540;duration=120;max_goals=3;tick_rate=60;ticks=40"
    return input_tape.new(identity, initial, frames)
end

---@param profile_name string
---@param network_seed integer
---@param measure RollbackSessionMeasure?
---@return RollbackLabOptions
local function lab_options(profile_name, network_seed, measure)
    return {
        profile_name = profile_name,
        network_seed = network_seed,
        sources = sources(),
        measure = measure,
        prevalidated_tape = true,
    }
end

---@param id string
---@param scenario string
---@param tape InputTape
---@param options RollbackLabOptions
---@param expected_failure boolean?
---@param sample string?
---@return RollbackValidationCaseSpec
local function case_spec(id, scenario, tape, options, expected_failure, sample)
    return {
        id = id,
        scenario = scenario,
        tape = tape,
        options = options,
        expected_failure = expected_failure == true,
        sample = sample,
    }
end

---@param measure RollbackSessionMeasure?
---@param profile_name string
---@param network_seed integer
---@return RollbackValidationCaseSpec[]
local function scenario_cases(measure, profile_name, network_seed)
    local source = determinism_evidence.fixture_tape()
    local goal = synthetic_goal_tape()
    local cases = {}
    for _, scenario in ipairs(config.scenarios) do
        local tape
        if scenario.kind == "synthetic_goal" then
            tape = goal
        else
            tape = normalized_window(
                source,
                assert(scenario.first_boundary),
                assert(scenario.last_boundary),
                scenario.id
            )
        end
        cases[#cases + 1] = case_spec(
            ("scenario-%s-%s-%d"):format(scenario.id, profile_name, network_seed),
            scenario.id,
            tape,
            lab_options(profile_name, network_seed, measure)
        )
    end
    return cases
end

---@type table<string, Omp2RollbackCombatLoadFixture>
local combat_load_by_scenario = {}
for _, fixture in ipairs(config.combat_load_fixtures) do
    assert(
        combat_load_by_scenario[fixture.scenario] == nil,
        "combat load scenarios must be unique: " .. fixture.scenario
    )
    combat_load_by_scenario[fixture.scenario] = fixture
end

---@param scenario string
---@return string
local function combat_load_case_prefix(scenario)
    return (scenario:gsub("_", "-"))
end

---@param measure RollbackSessionMeasure?
---@param network_seed integer
---@return RollbackValidationCaseSpec[]
local function combat_load_cases(measure, network_seed)
    local cases = {}
    for _, fixture in ipairs(config.combat_load_fixtures) do
        cases[#cases + 1] = case_spec(
            ("%s-%s-%d"):format(
                combat_load_case_prefix(fixture.scenario),
                config.stress_profile,
                network_seed
            ),
            fixture.scenario,
            combat_load_tape(fixture),
            lab_options(config.stress_profile, network_seed, measure)
        )
    end
    return cases
end

---@param target RollbackValidationCaseSpec[]
---@param added RollbackValidationCaseSpec[]
local function append_cases(target, added)
    for _, row in ipairs(added) do
        target[#target + 1] = row
    end
end

---@param suite RollbackValidationSuite
---@param options RollbackValidationOptions
---@return RollbackValidationCaseSpec[]
local function plan_cases(suite, options)
    local cases = {}
    local full = determinism_evidence.fixture_tape()
    local combat_tape = combat_validation_tape()
    if suite == "native" then
        for _, profile_name in ipairs(config.full_profiles) do
            for _, network_seed in ipairs(config.network_seeds) do
                cases[#cases + 1] = case_spec(
                    ("full-%s-%d"):format(profile_name, network_seed),
                    "complete_fixture",
                    full,
                    lab_options(profile_name, network_seed, options.measure)
                )
                cases[#cases + 1] = case_spec(
                    ("combat-%s-%d"):format(profile_name, network_seed),
                    "combat",
                    combat_tape,
                    lab_options(profile_name, network_seed, options.measure)
                )
            end
        end
        for _, network_seed in ipairs(config.network_seeds) do
            append_cases(
                cases,
                scenario_cases(options.measure, config.stress_profile, network_seed)
            )
            cases[#cases + 1] = case_spec(
                ("combat-stress-evidence-%d"):format(network_seed),
                "combat",
                combat_tape,
                lab_options(config.stress_profile, network_seed, options.measure)
            )
            append_cases(cases, combat_load_cases(options.measure, network_seed))
        end
    elseif suite == "browser-full" then
        local profile_name = assert(options.profile_name, "browser-full requires a profile")
        local network_seed = assert(options.network_seed, "browser-full requires a network seed")
        cases[1] = case_spec(
            ("full-%s-%d"):format(profile_name, network_seed),
            "complete_fixture",
            full,
            lab_options(profile_name, network_seed, options.measure)
        )
        cases[2] = case_spec(
            ("combat-%s-%d"):format(profile_name, network_seed),
            "combat",
            combat_tape,
            lab_options(profile_name, network_seed, options.measure)
        )
    elseif suite == "browser-stress" then
        local profile_name = options.profile_name or config.stress_profile
        local network_seed = assert(options.network_seed, "browser-stress requires a network seed")
        append_cases(cases, scenario_cases(options.measure, profile_name, network_seed))
        cases[#cases + 1] = case_spec(
            ("combat-stress-evidence-%d"):format(network_seed),
            "combat",
            combat_tape,
            lab_options(profile_name, network_seed, options.measure)
        )
        append_cases(cases, combat_load_cases(options.measure, network_seed))
    elseif suite == "late-window" then
        local tape = late_window_tape()
        for _, delay in ipairs({ 30, 31 }) do
            local profile = {
                base_delay_ticks = delay,
                jitter_min_ticks = 0,
                jitter_max_ticks = 0,
                independent_loss_rate = 0,
                duplication_rate = 0,
                burst_start_rate = 0,
                burst_length_ticks = 0,
            }
            local case_options = lab_options("delay_" .. delay, delay, options.measure)
            case_options.profile = profile
            cases[#cases + 1] =
                case_spec("delay-" .. delay, "late_window", tape, case_options, delay == 31)
        end
    elseif suite == "soak" then
        for index, network_seed in ipairs(config.soak_network_seeds) do
            cases[#cases + 1] = case_spec(
                ("combat-soak-%d-%d"):format(index, network_seed),
                "combat",
                combat_tape,
                lab_options("playable", network_seed, options.measure)
            )
            cases[#cases + 1] = case_spec(
                ("soak-%d-%d"):format(index, network_seed),
                "complete_fixture",
                full,
                lab_options("playable", network_seed, options.measure),
                false,
                config.soak_samples[index]
            )
        end
    end
    assert(#cases > 0, "rollback validation suite has no cases")
    return cases
end

--- A combat load case is covered when it replayed the exact pinned artifact and when
--- combat was present or absent as its fixture declares. Both directions are
--- load-bearing: an active fixture that confirmed nothing measured no combat at all,
--- and a twin that confirmed something is not a control -- the paired comparison would
--- silently become combat-against-combat and report no overhead.
---
--- Spell the two directions out separately, because `a and b or c` is not a conditional
--- in Lua. It parses as `(a and b) or c`, so an active fixture with zero confirmed
--- events fell through to the `combat_events == 0` arm and reported itself covered,
--- defeating the check exactly when it was needed. `scenario_pass` feeds
--- `completed.accepted` and the `success=` marker field that CI trusts as a gate.
---@param result RollbackLabResult
---@param fixture Omp2RollbackCombatLoadFixture
---@return boolean
local function combat_load_covered(result, fixture)
    local expected_version = fixture.combat and match_snapshot.COMBAT_VERSION
        or match_snapshot.VERSION
    local combat_events = result.event_metrics.confirmed_combat_events
    local combat_present = (fixture.combat and combat_events > 0)
        or (not fixture.combat and combat_events == 0)
    return result.input_ticks == fixture.frame_count
        and result.initial_hash == fixture.initial_hash
        and result.reference_final_hash == fixture.final_hash
        and result.tape_digest == fixture.tape_digest
        and combat_present
        and result.reference_final_snapshot.version == expected_version
        and result.client_final_snapshot.version == expected_version
end

---@param result RollbackLabResult
---@param scenario string
---@return boolean
local function scenario_covered(result, scenario)
    if scenario == "complete_fixture" or scenario == "late_window" then
        return true
    end
    local load_fixture = combat_load_by_scenario[scenario]
    if load_fixture then
        return combat_load_covered(result, load_fixture)
    end
    if scenario == "combat" then
        local fixture = config.combat_fixture
        return result.input_ticks == fixture.frame_count
            and result.initial_hash == fixture.initial_hash
            and result.reference_final_hash == fixture.final_hash
            and result.tape_digest == fixture.tape_digest
            and result.event_metrics.confirmed_combat_events > 0
            and result.reference_final_snapshot.version == match_snapshot.COMBAT_VERSION
            and result.client_final_snapshot.version == match_snapshot.COMBAT_VERSION
    end
    if scenario == "repeated_rollback" then
        return result.metrics.rollback_count >= 2
    end
    local previous_owner = nil
    for _, row in ipairs(result.event_trace) do
        if row.kind == "reference_confirmed" then
            local step = assert(row.step)
            if scenario == "possession_change" then
                local current = step.state.owner_id
                if previous_owner ~= nil and current ~= previous_owner then
                    return true
                end
                previous_owner = current
            end
            for _, event in ipairs(step.match_events) do
                local kind = event.payload.kind
                if
                    (scenario == "tackle" and kind == "tackle")
                    or (scenario == "shot" and kind == "shot")
                    or (scenario == "aerial" and kind == "header")
                    or (scenario == "keeper_action" and kind == "catch")
                then
                    return true
                end
            end
            for _, event in ipairs(step.lifecycle_events) do
                local kind = event.payload.kind
                if
                    (scenario == "goal" and kind == "goal")
                    or (scenario == "kickoff" and kind == "kickoff")
                    or (scenario == "full_time" and kind == "full_time")
                then
                    return true
                end
            end
        end
    end
    return false
end

---@param spec RollbackValidationCaseSpec
---@param active RollbackLabCampaign
---@param result RollbackLabResult
---@return RollbackValidationCompletedCase
local function complete_case(spec, active, result)
    local expected_terminal = spec.expected_failure
        and not result.success
        and result.status == "late_input_unrecoverable"
        and result.late_input_tick == 0
    local scenario_pass = scenario_covered(result, spec.scenario)
    local hidden_progress = expected_terminal and rollback_lab.probe_terminal_stability(active)
        or false
    local accepted = (result.success or expected_terminal) and scenario_pass and not hidden_progress
    return {
        id = spec.id,
        scenario = spec.scenario,
        initial_snapshot = spec.tape.initial,
        result = result,
        expected_failure = spec.expected_failure,
        accepted = accepted,
        hidden_progress = hidden_progress,
        scenario_pass = scenario_pass,
        sample = spec.sample,
    }
end

---@param suite RollbackValidationSuite
---@param options RollbackValidationOptions?
---@return RollbackValidationCampaign
function rollback_validation.new_campaign(suite, options)
    options = options or {}
    assert(
        suite == "native"
            or suite == "browser-full"
            or suite == "browser-stress"
            or suite == "late-window"
            or suite == "soak",
        "unknown rollback validation suite"
    )
    if options.network_seed ~= nil then
        assert(is_integer(options.network_seed), "rollback validation network seed must be integer")
    end
    return {
        suite = suite,
        cases = plan_cases(suite, options),
        next_case = 1,
        active = nil,
        active_spec = nil,
        completed = 0,
        failed = false,
        logical = fnv1a64.new(),
        result = nil,
    }
end

---@param campaign RollbackValidationCampaign
---@param max_ticks integer
---@return RollbackValidationResult?
---@return RollbackValidationCompletedCase?
function rollback_validation.step_campaign(campaign, max_ticks)
    assert(is_integer(max_ticks) and max_ticks > 0, "validation step must be positive")
    if campaign.result then
        return campaign.result, nil
    end
    if campaign.active == nil then
        local spec = assert(campaign.cases[campaign.next_case])
        campaign.active_spec = spec
        campaign.active = rollback_lab.new_campaign(spec.tape, spec.options)
    end
    local active = assert(campaign.active)
    local result = rollback_lab.step_campaign(active, max_ticks)
    if result == nil then
        return nil, nil
    end
    local spec = assert(campaign.active_spec)
    local completed = complete_case(spec, active, result)
    campaign.completed = campaign.completed + 1
    campaign.failed = campaign.failed or not completed.accepted
    fnv1a64.update(
        campaign.logical,
        completed.id .. "\n" .. rollback_lab.logical_marker(result) .. "\n"
    )
    campaign.next_case = campaign.next_case + 1
    campaign.active = nil
    campaign.active_spec = nil
    if campaign.next_case > #campaign.cases then
        campaign.result = {
            schema = rollback_validation.SCHEMA,
            suite = campaign.suite,
            success = not campaign.failed,
            case_count = campaign.completed,
            logical_digest = fnv1a64.hex(campaign.logical),
        }
    end
    return campaign.result, completed
end

---@param completed RollbackValidationCompletedCase
---@return string
function rollback_validation.case_marker(completed)
    local result = completed.result
    local metrics = result.metrics
    local events = result.event_metrics
    local tape_version = completed.initial_snapshot.version == match_snapshot.COMBAT_VERSION
            and input_tape.COMBAT_VERSION
        or input_tape.VERSION
    return table.concat({
        "GC_ROLLBACK_VALIDATION",
        "case",
        "schema=1",
        "case=" .. completed.id,
        "scenario=" .. completed.scenario,
        "fixture=" .. result.fixture,
        "profile=" .. result.profile,
        "network_seed=" .. result.network_seed,
        "success=" .. (completed.accepted and "1" or "0"),
        "lab_success=" .. (result.success and "1" or "0"),
        "expected_failure=" .. (completed.expected_failure and "1" or "0"),
        "status=" .. result.status,
        "late_tick=" .. tostring(result.late_input_tick or "none"),
        "hidden_progress=" .. (completed.hidden_progress and "1" or "0"),
        "scenario_pass=" .. (completed.scenario_pass and "1" or "0"),
        "tape_version=" .. tape_version,
        "snapshot_version=" .. completed.result.reference_final_snapshot.version,
        "tape_digest=" .. result.tape_digest,
        "initial_hash=" .. result.initial_hash,
        "reference_hash=" .. result.reference_final_hash,
        "client_hash=" .. result.client_final_hash,
        "rollbacks=" .. metrics.rollback_count,
        "max_depth=" .. metrics.max_rollback_depth,
        "resimulated=" .. metrics.resimulated_ticks,
        "peak_snapshots=" .. metrics.peaks.snapshot_count,
        "peak_snapshot_bytes=" .. metrics.peaks.snapshot_bytes,
        "peak_history_bytes=" .. metrics.peaks.history_bytes,
        "event_reference_digest=" .. events.reference_digest,
        "event_confirmed_digest=" .. events.confirmed_digest,
        "event_confirmed_combat=" .. events.confirmed_combat_events,
        "event_residue=" .. events.speculative_residue,
        "sample=" .. tostring(completed.sample or "none"),
    }, "|")
end

---@param result RollbackValidationResult
---@return string
function rollback_validation.result_marker(result)
    return table.concat({
        "GC_ROLLBACK_VALIDATION",
        "result",
        "schema=" .. result.schema,
        "suite=" .. result.suite,
        "success=" .. (result.success and "1" or "0"),
        "logical_digest=" .. result.logical_digest,
        "case_count=" .. result.case_count,
    }, "|")
end

---@return Omp2RollbackValidationData
function rollback_validation.config()
    return config
end

--- Whether a lab result covers the combat load scenario it claims to. This is the same
--- predicate `scenario_covered` applies when it decides `scenario_pass`, exposed so a
--- test can drive both fixture kinds against both zero and non-zero confirmed combat
--- events. A campaign only ever produces the two passing corners of that truth table,
--- so the rejecting corners are unreachable from a normal run and go unchecked unless
--- they are asserted directly.
---@param result RollbackLabResult
---@param scenario string
---@return boolean
function rollback_validation.combat_load_covered(result, scenario)
    local fixture = assert(
        combat_load_by_scenario[scenario],
        "unknown combat load scenario: " .. tostring(scenario)
    )
    return combat_load_covered(result, fixture)
end

--- Build a pinned combat load tape by scenario id. Exposed so the fixtures can be
--- exercised and their pinned identity checked without standing up a whole campaign.
---@param scenario string
---@return InputTape
function rollback_validation.combat_load_tape(scenario)
    local fixture = assert(
        combat_load_by_scenario[scenario],
        "unknown combat load scenario: " .. tostring(scenario)
    )
    return combat_load_tape(fixture)
end

---@return string
function rollback_validation.profile_digest()
    local state = fnv1a64.new()
    for _, name in ipairs(config.full_profiles) do
        local profile = assert(network_profiles[name])
        fnv1a64.update(state, table.concat({
            name,
            tostring(profile.base_delay_ticks),
            tostring(profile.jitter_min_ticks),
            tostring(profile.jitter_max_ticks),
            tostring(profile.independent_loss_rate),
            tostring(profile.duplication_rate),
            tostring(profile.burst_start_rate),
            tostring(profile.burst_length_ticks),
        }, "|") .. "\n")
    end
    return fnv1a64.hex(state)
end

return rollback_validation
