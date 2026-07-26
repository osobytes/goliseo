-- The pure Lua learning-environment core.
--
-- reset / observe / act / step over the canonical fixed tick, built entirely
-- from the seams that already exist: sim.fixed_clock owns tick numbering,
-- sim.slot_input materializes the complete effective InputFrame for all eight
-- slots, sim.match is the only authority, sim.match_snapshot produces boundary
-- hashes, sim.metrics collects evaluation diagnostics, and sim.input_tape /
-- sim.replay verify the recording afterwards.
--
-- This module is pure: no love, no files, no networking, no learning framework.
-- Any transport, batching, or serialization layer lives outside sim/ and can
-- only drive this API, so it can never change simulation authority.
--
-- Reward and observation policy live in sim.env_reward and sim.env_observation;
-- the design record is docs/design/learning_environment.md.

local combat = require("sim.combat")
local combat_identity = require("sim.combat_identity")
local env_action = require("sim.env_action")
local env_config = require("sim.env_config")
local env_observation = require("sim.env_observation")
local env_reward = require("sim.env_reward")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local input_tape = require("sim.input_tape")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local metrics = require("sim.metrics")
local slot_input = require("sim.slot_input")
local tuning = require("sim.tuning")

---@alias EnvTerminationReason
---| "time_expired" -- Regulation time ran out.
---| "goal_cap" -- A side reached the configured goal cap.

---@alias EnvTruncationReason
---| "step_limit" -- The configured episode tick budget was spent.
---| "tape_exhausted" -- A tape-driven slot ran out of recorded rows.

---@alias EnvStoppageReason "kickoff_hold"

---@alias EnvErrorCode
---| "malformed"
---| "unsupported_version"
---| "unknown_content"
---| "empty_selection"
---| "profile_mismatch"
---| "missing_tape"
---| "tape_exhausted"
---| "episode_over"
---| "unknown_slot"
---| "missing_slot"
---| "illegal_action"
---| "sim_fault"
---| "faulted"

---@class EnvStepEvent
---@field tick integer -- Tick the event was confirmed on.
---@field team InputTeam? -- Absolute acting side, when identified.
---@field slot integer? -- Canonical input slot of the actor, when it has one.
---@field domain "soccer"|"combat"
---@field event MatchEvent|CombatEvent -- Full-fidelity confirmed record (evaluation data).

---@class EnvDiagnostics
---@field role "evaluation" -- Diagnostics are never an optimization target.
---@field channel EnvRewardChannelId -- The diagnostic channel these belong to.
---@field episode_ticks integer
---@field simulated_ticks integer
---@field event_counts table<string, integer>
---@field metrics MatchMetrics? -- Registered #128 metrics, produced once the episode ends.

---@class EnvStepResult
---@field version integer
---@field tick integer -- Boundary tick after the step.
---@field ticks_simulated integer -- Zero when the step could not advance.
---@field observation EnvObservation
---@field reward EnvRewardResult
---@field terminated boolean -- The match itself ended.
---@field truncated boolean -- The episode was cut short; the match did not end.
---@field termination EnvTerminationReason?
---@field truncation EnvTruncationReason?
---@field stoppage boolean -- Play is held (restart), distinct from termination.
---@field stoppage_reason EnvStoppageReason?
---@field events EnvStepEvent[]
---@field diagnostics EnvDiagnostics
---@field boundary_hash string -- Canonical snapshot hash after the step.
---@field boundary_hashes string[] -- One hash per simulated tick, in order.
---@field action_wires string[] -- Encoded effective InputFrames consumed, for audit.

---@class EnvManifest
---@field version integer
---@field env_version integer
---@field config_version integer
---@field observation_version integer
---@field action_version integer
---@field reward_version integer
---@field input_version integer
---@field snapshot_version integer
---@field tape_version integer
---@field tick_rate integer
---@field build string
---@field source string
---@field content string
---@field fixture string
---@field config string -- Canonical config digest.
---@field tuning string -- Serialized active tuning knobs.
---@field seed integer
---@field combat string? -- Combat mechanics identity, when combat is enabled.
---@field ownership InputOwnership
---@field observation_profile EnvObservationProfile
---@field controlled_slots integer[]
---@field policy_ids table<integer, string>
---@field slot_sources EnvSlotSourceConfig[]
---@field objective_channels EnvRewardChannelId[]
---@field shaping_channels EnvRewardChannelId[]
---@field diagnostic_channel EnvRewardChannelId
---@field initial_boundary_hash string
---@field boundary_hash string
---@field episode_ticks integer

---@class EnvInstance
---@field version integer
---@field config EnvConfig
---@field controlled_slots integer[]
---@field tape_slots integer[]
---@field tick integer -- Next tick to simulate; equals the current boundary tick.
---@field episode_ticks integer
---@field terminated boolean
---@field truncated boolean
---@field termination EnvTerminationReason?
---@field truncation EnvTruncationReason?
---@field boundary_hashes string[] -- Index one is the reset boundary.
---@field frames InputFrame[] -- Materialized effective rows, indexed by tick + 1.
---@field identity InputTapeIdentity
---@field _state MatchState
---@field _combat CombatMatchState?
---@field _producer SlotInputProducerState
---@field _metrics MetricsCollector
---@field _clock FixedClockState
---@field _initial MatchSnapshot
---@field _tape_frames InputFrame[]
---@field _final_metrics MatchMetrics?
---@field _fault string?

---@class EnvModule
local env = {}

env.VERSION = 1
env.MANIFEST_VERSION = 1
env.STEP_VERSION = 1

local DT = fixed_clock.TICK_SECONDS

---@type table<string, boolean>
env.REFERENCE_FIXTURES = {
    soccer_only = true,
    combat_all_families = true,
}

---@param code EnvErrorCode
---@param message string
---@return nil, string, EnvErrorCode
local function reject(code, message)
    return nil, message, code
end

-- Synthetic reference environments. `soccer_only` disables the combat companion
-- entirely; `combat_all_families` enables it on a fixture whose four outfield
-- loadouts per side cover unarmed, guard, light_melee, and ranged.
---@param id string
---@param overrides table<string, any>?
---@return table<string, any>?, string?, EnvErrorCode?
function env.reference_config(id, overrides)
    if not env.REFERENCE_FIXTURES[id] then
        return reject("unknown_content", "unknown reference fixture: " .. tostring(id))
    end
    local config = {
        version = env_config.VERSION,
        build = "reference-" .. id,
        content = env_config.DEFAULT_CONTENT,
        seed = 1,
        duration = 4,
        max_goals = 3,
        field = { w = env_config.DEFAULT_FIELD.w, h = env_config.DEFAULT_FIELD.h },
        home_team_id = env_config.DEFAULT_HOME_TEAM_ID,
        away_team_id = env_config.DEFAULT_AWAY_TEAM_ID,
        combat = id == "combat_all_families",
        observation_profile = "representative",
        slot_sources = env_config.default_slot_sources(),
        reward_team = "home",
        objective_channels = { "match_outcome" },
        shaping_channels = {},
    }
    if overrides ~= nil then
        if type(overrides) ~= "table" then
            return reject("malformed", "reference config overrides must be a table")
        end
        for key, value in pairs(overrides) do
            config[key] = value
        end
    end
    return config
end

---@param config EnvConfig
---@return MatchSlotSource[]
local function producer_sources(config)
    local sources = {}
    for index = 1, input_frame.SLOT_COUNT do
        local source = config.slot_sources[index]
        if source.kind == "bot" then
            sources[index] = { kind = "bot", seed = assert(source.seed) }
        elseif source.kind == "neutral" then
            sources[index] = { kind = "neutral" }
        else
            -- Policy and tape rows are both authored upstream of the producer:
            -- the environment writes them into the base frame it materializes.
            sources[index] = { kind = "frame" }
        end
    end
    return sources
end

-- `match.new` has no away-formation override, so an away formation is expressed
-- as a per-run TeamData view exactly the way sim.headless does it. Canonical team
-- data stays immutable and the existing team.formation path still builds the side.
---@param team TeamData
---@param formation string?
---@return TeamData
local function with_formation(team, formation)
    if not formation then
        return team
    end
    return {
        id = team.id,
        name = team.name,
        color = team.color,
        formation = formation,
        roster = team.roster,
    }
end

---@param frames any
---@return InputFrame[]?, string?, EnvErrorCode?
local function copy_tape_frames(frames)
    if frames == nil then
        return {}
    end
    if type(frames) ~= "table" then
        return reject("malformed", "tape frames must be an array")
    end
    local count = #frames
    for key in pairs(frames) do
        if type(key) ~= "number" or key ~= math.floor(key) or key < 1 or key > count then
            return reject("malformed", "tape frames must be a canonical array")
        end
    end
    local copied = {}
    for index = 1, count do
        local frame, err = input_frame.copy(frames[index])
        if not frame then
            return reject("malformed", "tape frame " .. index .. " is invalid: " .. tostring(err))
        end
        if frame.tick ~= index - 1 then
            return reject(
                "malformed",
                "tape frames must start at tick zero and be contiguous (frame " .. index .. ")"
            )
        end
        copied[index] = frame
    end
    return copied
end

---@param instance EnvInstance
---@return string
local function boundary_hash(instance)
    return match_snapshot.hash(match_snapshot.capture(instance._state, instance._combat))
end

-- Construct a fresh episode. Everything the episode depends on is named in the
-- config, so the manifest returned by `env.manifest` is sufficient to reproduce
-- this run tick for tick.
---@param config any
---@param tape_frames InputFrame[]? -- Required when any slot source is `tape`.
---@return EnvInstance?, string?, EnvErrorCode?
function env.reset(config, tape_frames)
    local normalized, config_err, config_code = env_config.normalize(config)
    if not normalized then
        return nil, config_err, config_code
    end
    local copied_frames, frames_err, frames_code = copy_tape_frames(tape_frames)
    if not copied_frames then
        return nil, frames_err, frames_code
    end
    local tape_slots = env_config.tape_slots(normalized)
    if #tape_slots > 0 and #copied_frames == 0 then
        return reject("missing_tape", "tape-driven slots require recorded frames")
    end
    if #tape_slots == 0 and #copied_frames > 0 then
        return reject("malformed", "recorded frames were supplied but no slot source is a tape")
    end

    local fixture = env_config.resolve(normalized)
    local away = with_formation(fixture.away, normalized.away_formation)
    local ownership = match.ownership_for_teams(fixture.home, away, fixture.players_by_id)
    local state = match.new({
        home = fixture.home,
        away = away,
        field = { w = normalized.field.w, h = normalized.field.h },
        home_formation = normalized.home_formation,
        tactic = fixture.home_tactic,
        away_tactic = fixture.away_tactic,
        players_by_id = fixture.players_by_id,
        human_controlled = false,
        duration = normalized.duration,
        max_goals = normalized.max_goals,
        seed = normalized.seed,
        input_ownership = ownership,
    })
    local combat_state = normalized.combat and combat.new_state(state, fixture.players_by_id) or nil
    local initial = match_snapshot.capture(state, combat_state)
    local identity = {
        tape_version = combat_state and input_tape.COMBAT_VERSION or input_tape.VERSION,
        input_version = input_frame.VERSION,
        snapshot_version = initial.version,
        build = normalized.build,
        source = normalized.source,
        content = normalized.content,
        tuning = tuning.serialize(),
        config = env_config.digest(normalized),
        fixture = normalized.fixture,
        seed = normalized.seed,
        tick_rate = fixed_clock.TICK_RATE,
        ownership = ownership,
        combat = combat_state and combat_identity.for_state(combat_state) or nil,
    }
    return {
        version = env.VERSION,
        config = normalized,
        controlled_slots = env_config.controlled_slots(normalized),
        tape_slots = tape_slots,
        tick = state.input_tick,
        episode_ticks = 0,
        terminated = false,
        truncated = false,
        termination = nil,
        truncation = nil,
        boundary_hashes = { match_snapshot.hash(initial) },
        frames = {},
        identity = identity,
        _state = state,
        _combat = combat_state,
        _producer = slot_input.new_producer(producer_sources(normalized)),
        _metrics = metrics.new(state),
        _clock = fixed_clock.new(),
        _initial = initial,
        _tape_frames = copied_frames,
        _final_metrics = nil,
        _fault = nil,
    }
end

-- The observation at the current boundary, built before any action for the next
-- tick exists.
---@param instance EnvInstance
---@return EnvObservation
function env.observe(instance)
    local observation = assert(
        env_observation.build(
            instance._state,
            instance._combat,
            instance.controlled_slots,
            instance.config.observation_profile
        )
    )
    return observation
end

-- Legality masks for the controlled slots, derived from the current observation.
---@param instance EnvInstance
---@return table<integer, EnvActionMask>
function env.action_masks(instance)
    local observation = env.observe(instance)
    local masks = {}
    for _, slot in ipairs(instance.controlled_slots) do
        masks[slot] = assert(env_action.mask(assert(observation.views[slot])))
    end
    return masks
end

---@param instance EnvInstance
---@return MatchSnapshot
function env.snapshot(instance)
    return match_snapshot.capture(instance._state, instance._combat)
end

---@param instance EnvInstance
---@param actions any
---@return table<integer, EnvSlotAction>?, string?, EnvErrorCode?
local function normalize_actions(instance, actions)
    if type(actions) ~= "table" then
        return reject("malformed", "step actions must be a table keyed by controlled slot")
    end
    local controlled = {}
    for _, slot in ipairs(instance.controlled_slots) do
        controlled[slot] = true
    end
    for key in pairs(actions) do
        if type(key) ~= "number" or not controlled[key] then
            return reject("unknown_slot", "slot " .. tostring(key) .. " is not a policy slot")
        end
    end
    local masks = env.action_masks(instance)
    local normalized = {}
    for _, slot in ipairs(instance.controlled_slots) do
        local action = actions[slot]
        if action == nil then
            return reject("missing_slot", "policy slot " .. slot .. " has no action this step")
        end
        local valid, valid_err = env_action.validate(action)
        if not valid then
            return reject("illegal_action", "slot " .. slot .. ": " .. tostring(valid_err))
        end
        local allowed, mask_err = env_action.check_mask(valid, assert(masks[slot]))
        if not allowed then
            return reject("illegal_action", "slot " .. slot .. ": " .. tostring(mask_err))
        end
        normalized[slot] = valid
    end
    return normalized
end

---@param instance EnvInstance
---@param tick integer
---@param actions table<integer, EnvSlotAction>
---@return InputFrame?, string?, EnvErrorCode?
local function base_frame(instance, tick, actions)
    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        local source = instance.config.slot_sources[index]
        if source.kind == "policy" then
            local sample, sample_err =
                env_action.to_sample(assert(actions[index], "policy slot action is missing"))
            if not sample then
                return reject("illegal_action", "slot " .. index .. ": " .. tostring(sample_err))
            end
            slots[index] = sample
        elseif source.kind == "tape" then
            local frame = instance._tape_frames[tick + 1]
            if not frame then
                return reject(
                    "tape_exhausted",
                    "tape slot " .. index .. " has no recorded row for tick " .. tick
                )
            end
            slots[index] = frame.slots[index]
        else
            -- Overwritten by the producer; the base row must still be complete.
            slots[index] = input_frame.neutral_sample()
        end
    end
    local frame, frame_err = input_frame.new(tick, slots)
    if not frame then
        return reject("malformed", frame_err or "effective frame is invalid")
    end
    return frame
end

---@param event MatchEvent
---@return MatchEvent
local function copy_match_event(event)
    return {
        kind = event.kind,
        x = event.x,
        y = event.y,
        player = event.player,
        save_style = event.save_style,
        style = event.style,
        outcome = event.outcome,
        jumping = event.jumping,
        difficulty = event.difficulty,
        shot_type = event.shot_type,
        keeper_state = event.keeper_state,
        keeper_depth = event.keeper_depth,
        on_target = event.on_target,
    }
end

---@param event CombatEvent
---@return CombatEvent
local function copy_combat_event(event)
    return {
        kind = event.kind,
        tick = event.tick,
        family_id = event.family_id,
        source_index = event.source_index,
        target_index = event.target_index,
        source_sequence = event.source_sequence,
        result = event.result,
        x = event.x,
        y = event.y,
        interruption_ticks = event.interruption_ticks,
        displacement_px = event.displacement_px,
    }
end

---@param instance EnvInstance
---@param confirmed_tick integer
---@param events EnvStepEvent[]
---@param reward_events EnvRewardEvent[]
local function collect_events(instance, confirmed_tick, events, reward_events)
    local state = instance._state
    local team_of_id = {}
    local slot_of_id = {}
    for index, player in ipairs(state.players) do
        team_of_id[player.id] = player.team
        slot_of_id[player.id] = state.slot_for_player[index]
    end
    for _, event in ipairs(state.events) do
        local team = event.player and team_of_id[event.player] or nil
        events[#events + 1] = {
            tick = confirmed_tick,
            team = team,
            slot = event.player and slot_of_id[event.player] or nil,
            domain = "soccer",
            event = copy_match_event(event),
        }
        reward_events[#reward_events + 1] = { kind = event.kind, team = team, result = nil }
    end
    local combat_state = instance._combat
    if combat_state then
        for _, event in ipairs(combat_state.events) do
            local source = event.source_index and state.players[event.source_index] or nil
            events[#events + 1] = {
                tick = event.tick,
                team = source and source.team or nil,
                slot = event.source_index and state.slot_for_player[event.source_index] or nil,
                domain = "combat",
                event = copy_combat_event(event),
            }
            reward_events[#reward_events + 1] = {
                kind = event.kind,
                team = source and source.team or nil,
                result = event.result,
            }
        end
    end
end

---@param instance EnvInstance
---@return EnvTerminationReason
local function termination_reason(instance)
    if instance._state.time_left <= 0 then
        return "time_expired"
    end
    return "goal_cap"
end

---@param instance EnvInstance
---@return MatchMetrics
local function final_metrics(instance)
    if not instance._final_metrics then
        instance._final_metrics = metrics.finish(instance._metrics, instance._state)
    end
    return assert(instance._final_metrics)
end

-- Advance the episode by `ticks` canonical fixed ticks (default one), holding the
-- supplied actions across them. One-shot edges only fire on the first tick: an
-- edge means "this tick only", so repeating it would fabricate presses the policy
-- never made. Held intents persist for every tick of the step.
--
-- Termination (the match ended) and truncation (the episode was cut short) are
-- returned separately and are never both true.
---@param instance EnvInstance
---@param actions table<integer, EnvSlotAction>
---@param ticks integer?
---@return EnvStepResult?, string?, EnvErrorCode?
function env.step(instance, actions, ticks)
    if instance._fault then
        return reject("faulted", "environment is faulted: " .. instance._fault)
    end
    if instance.terminated or instance.truncated then
        return reject("episode_over", "the episode has already ended")
    end
    local requested = ticks == nil and 1 or ticks
    if
        type(requested) ~= "number"
        or requested ~= math.floor(requested)
        or requested < 1
        or requested == math.huge
    then
        return reject("malformed", "step tick count must be a positive integer")
    end
    local normalized, actions_err, actions_code = normalize_actions(instance, actions)
    if not normalized then
        return nil, actions_err, actions_code
    end

    local state = instance._state
    local score_before = { home = state.score.home, away = state.score.away }
    local owner_before = state.owner and state.players[state.owner].team or nil
    ---@type EnvStepEvent[]
    local events = {}
    ---@type EnvRewardEvent[]
    local reward_events = {}
    local boundary_hashes = {}
    local action_wires = {}
    local simulated = 0
    local held_actions = normalized

    for index = 1, requested do
        if index > 1 then
            local repeated = {}
            for slot, action in pairs(held_actions) do
                repeated[slot] = env_action.without_edges(action)
            end
            held_actions = repeated
        end
        local tick = state.input_tick
        local frame, frame_err, frame_code = base_frame(instance, tick, held_actions)
        if not frame then
            if frame_code == "tape_exhausted" then
                -- An incomplete tape truncates the episode; it is not an error
                -- and it is never reported as the match having ended.
                instance.truncated = true
                instance.truncation = "tape_exhausted"
                break
            end
            return nil, frame_err, frame_code
        end
        -- The whole tick runs under pcall: an invariant violation anywhere between
        -- materialization and the boundary is a fault the caller must be able to
        -- reproduce, not a crash that takes the trainer down with it.
        local requested_wire = assert(input_frame.encode(frame))
        ---@type { frame: InputFrame, wire: string }?
        local advanced = nil
        local ok, fault = pcall(function()
            local effective = slot_input.materialize(instance._producer, state, frame)
            local encoded = assert(input_frame.encode(effective))
            fixed_clock.step(instance._clock, effective, function(_, tick_input)
                match.step(state, DT, tick_input, instance._combat)
                metrics.observe(instance._metrics, state, DT)
                return not state.finished
            end)
            advanced = { frame = effective, wire = encoded }
        end)
        if not ok or not advanced then
            instance._fault = tostring(fault)
            return reject(
                "sim_fault",
                ("simulation fault at tick %d (boundary %s, input %s): %s"):format(
                    tick,
                    instance.boundary_hashes[#instance.boundary_hashes],
                    requested_wire,
                    tostring(fault)
                )
            )
        end
        local effective = advanced.frame
        local wire = advanced.wire
        simulated = simulated + 1
        instance.episode_ticks = instance.episode_ticks + 1
        instance.tick = state.input_tick
        instance.frames[#instance.frames + 1] = effective
        local hash = boundary_hash(instance)
        instance.boundary_hashes[#instance.boundary_hashes + 1] = hash
        boundary_hashes[#boundary_hashes + 1] = hash
        action_wires[#action_wires + 1] = wire
        collect_events(instance, tick, events, reward_events)
        if state.finished then
            instance.terminated = true
            instance.termination = termination_reason(instance)
            break
        end
        local budget = instance.config.max_episode_ticks
        if budget and instance.episode_ticks >= budget then
            instance.truncated = true
            instance.truncation = "step_limit"
            break
        end
    end

    local reward = env_reward.evaluate({
        team = instance.config.reward_team,
        score_before = score_before,
        score_after = { home = state.score.home, away = state.score.away },
        owner_team_before = owner_before,
        owner_team_after = state.owner and state.players[state.owner].team or nil,
        events = reward_events,
        terminated = instance.terminated,
    }, {
        objectives = instance.config.objective_channels,
        shaping = instance.config.shaping_channels,
    })

    local event_counts = {}
    for _, record in ipairs(events) do
        local kind = record.event.kind
        event_counts[kind] = (event_counts[kind] or 0) + 1
    end
    local episode_over = instance.terminated or instance.truncated
    return {
        version = env.STEP_VERSION,
        tick = state.input_tick,
        ticks_simulated = simulated,
        observation = env.observe(instance),
        reward = reward,
        terminated = instance.terminated,
        truncated = instance.truncated,
        termination = instance.termination,
        truncation = instance.truncation,
        stoppage = state.kickoff_hold > 0,
        stoppage_reason = state.kickoff_hold > 0 and "kickoff_hold" or nil,
        events = events,
        diagnostics = {
            role = "evaluation",
            channel = env_reward.DIAGNOSTIC_METRIC_CHANNEL,
            episode_ticks = instance.episode_ticks,
            simulated_ticks = simulated,
            event_counts = event_counts,
            metrics = episode_over and final_metrics(instance) or nil,
        },
        boundary_hash = instance.boundary_hashes[#instance.boundary_hashes],
        boundary_hashes = boundary_hashes,
        action_wires = action_wires,
    }
end

-- Everything needed to reproduce this episode: contract versions, provenance,
-- tuning and fixture identity, seed, per-slot ownership with policy ids, reward
-- selection, and the boundary hashes that pin the trajectory.
---@param instance EnvInstance
---@return EnvManifest
function env.manifest(instance)
    local policy_ids = {}
    local sources = {}
    for index = 1, input_frame.SLOT_COUNT do
        local source = instance.config.slot_sources[index]
        sources[index] = {
            kind = source.kind,
            seed = source.seed,
            policy_id = source.policy_id,
        }
        if source.policy_id then
            policy_ids[index] = source.policy_id
        end
    end
    return {
        version = env.MANIFEST_VERSION,
        env_version = env.VERSION,
        config_version = instance.config.version,
        observation_version = env_observation.VERSION,
        action_version = env_action.VERSION,
        reward_version = env_reward.VERSION,
        input_version = input_frame.VERSION,
        snapshot_version = instance._initial.version,
        tape_version = instance.identity.tape_version,
        tick_rate = fixed_clock.TICK_RATE,
        build = instance.config.build,
        source = instance.config.source,
        content = instance.config.content,
        fixture = instance.config.fixture,
        config = instance.identity.config,
        tuning = instance.identity.tuning,
        seed = instance.config.seed,
        combat = instance.identity.combat,
        ownership = instance.identity.ownership,
        observation_profile = instance.config.observation_profile,
        controlled_slots = instance.controlled_slots,
        policy_ids = policy_ids,
        slot_sources = sources,
        objective_channels = instance.config.objective_channels,
        shaping_channels = instance.config.shaping_channels,
        diagnostic_channel = env_reward.DIAGNOSTIC_METRIC_CHANNEL,
        initial_boundary_hash = instance.boundary_hashes[1],
        boundary_hash = instance.boundary_hashes[#instance.boundary_hashes],
        episode_ticks = instance.episode_ticks,
    }
end

-- Export the episode as a canonical InputTape. The tape carries the effective
-- rows the simulation actually consumed and the boundary hashes the environment
-- recorded, so sim.replay re-deriving those hashes is an independent check that
-- the environment path and the replay path agree.
---@param instance EnvInstance
---@return InputTape?, string?, EnvErrorCode?
function env.tape(instance)
    local ok, tape = pcall(
        input_tape.from_frozen_recording,
        instance.identity,
        instance._initial,
        instance.frames,
        instance.boundary_hashes
    )
    if not ok then
        return reject("malformed", tostring(tape))
    end
    ---@cast tape InputTape
    return tape
end

return env
