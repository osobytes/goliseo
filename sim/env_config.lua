-- Versioned learning-environment configuration.
--
-- A config is the complete, serializable identity of an episode: build/content
-- provenance, fixture and tuning identity, seed, duration, per-slot ownership,
-- observation profile, and reward channel selection. It resolves authored
-- content ids into the fixture tables sim.match already consumes; it never
-- reads files, never touches love, and never carries live simulation state.
--
-- Configs arrive from outside the simulation, so every rejection is a
-- recoverable `nil, err, code` return instead of an assert.

local env_reward = require("sim.env_reward")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local formations = require("data.formations")
local player_pool = require("data.players")
local tactics = require("data.tactics")
local teams = require("data.teams")

---@alias EnvObservationProfile "representative"|"team"|"privileged"

---@alias EnvSlotSourceKind
---| "policy" -- The caller supplies this slot's action every step.
---| "tape" -- The slot replays a supplied recorded InputFrame row.
---| "bot" -- The slot is filled by the deterministic built-in slot bot.
---| "neutral" -- The slot receives the canonical neutral row.

---@alias EnvConfigErrorCode
---| "malformed"
---| "unsupported_version"
---| "unknown_content"
---| "empty_selection"
---| "profile_mismatch"

---@class EnvSlotSourceConfig
---@field kind EnvSlotSourceKind
---@field seed integer? -- Required for `bot`, forbidden otherwise.
---@field policy_id string? -- Audit label for `policy` slots (checkpoint / population id).

---@class EnvConfig
---@field version integer -- Exactly EnvConfig.VERSION.
---@field build string -- Build/commit provenance supplied by the caller.
---@field source string -- Recording source label; defaults to `build`.
---@field content string -- Authored content revision label.
---@field fixture string -- Fixture label; derived from the teams when omitted.
---@field seed integer -- Canonical match seed.
---@field duration number -- Match length in seconds.
---@field max_goals integer -- Goal cap that terminates the match.
---@field field { w: number, h: number }
---@field home_team_id string -- Key into data.teams.
---@field away_team_id string
---@field home_formation string? -- Key into data.formations; nil keeps the team default.
---@field away_formation string?
---@field home_tactic_id string -- Key into data.tactics.
---@field away_tactic_id string
---@field combat boolean -- Enable the combat companion state (all four action families).
---@field observation_profile EnvObservationProfile
---@field slot_sources EnvSlotSourceConfig[] -- Exactly eight, canonical InputSlot order.
---@field reward_team InputTeam -- Perspective every reward channel is expressed from.
---@field objective_channels EnvRewardChannelId[]
---@field shaping_channels EnvRewardChannelId[]
---@field max_episode_ticks integer? -- Truncation budget; nil means only the match ends the episode.

---@class EnvResolvedFixture
---@field home TeamData
---@field away TeamData
---@field home_tactic TacticData
---@field away_tactic TacticData
---@field players_by_id table<string, PlayerData>

---@class EnvConfigModule
local env_config = {}

env_config.VERSION = 1
env_config.DEFAULT_CONTENT = "showcase-content-v1"
env_config.DEFAULT_DURATION = 30
env_config.DEFAULT_MAX_GOALS = 3
env_config.DEFAULT_FIELD = { w = 960, h = 540 }
env_config.DEFAULT_TACTIC_ID = "balanced"
env_config.DEFAULT_HOME_TEAM_ID = "nebula"
env_config.DEFAULT_AWAY_TEAM_ID = "orion"

local CONFIG_FIELDS = {
    version = true,
    build = true,
    source = true,
    content = true,
    fixture = true,
    seed = true,
    duration = true,
    max_goals = true,
    field = true,
    home_team_id = true,
    away_team_id = true,
    home_formation = true,
    away_formation = true,
    home_tactic_id = true,
    away_tactic_id = true,
    combat = true,
    observation_profile = true,
    slot_sources = true,
    reward_team = true,
    objective_channels = true,
    shaping_channels = true,
    max_episode_ticks = true,
}

local SOURCE_FIELDS = {
    kind = true,
    seed = true,
    policy_id = true,
}

local SOURCE_KINDS = {
    policy = true,
    tape = true,
    bot = true,
    neutral = true,
}

local PROFILES = {
    representative = true,
    team = true,
    privileged = true,
}

local FIELD_FIELDS = { w = true, h = true }

---@param code EnvConfigErrorCode
---@param message string
---@return nil, string, EnvConfigErrorCode
local function reject(code, message)
    return nil, message, code
end

---@param value any
---@return boolean
local function is_finite(value)
    return type(value) == "number" and value == value and value < math.huge and value > -math.huge
end

---@param value any
---@return boolean
local function is_integer(value)
    return is_finite(value) and value == math.floor(value)
end

---@param value any
---@return boolean
local function is_label(value)
    return type(value) == "string" and value ~= ""
end

---@param value any
---@param allowed table<string, boolean>
---@return boolean
local function only_known_fields(value, allowed)
    if type(value) ~= "table" then
        return false
    end
    for key in pairs(value) do
        if type(key) ~= "string" or not allowed[key] then
            return false
        end
    end
    return true
end

---@return table<string, PlayerData>
local function pool_by_id()
    local by_id = {}
    for _, player in ipairs(player_pool) do
        by_id[player.id] = player
    end
    return by_id
end

---@param sources any
---@return EnvSlotSourceConfig[]?, string?, EnvConfigErrorCode?
local function copy_slot_sources(sources)
    if type(sources) ~= "table" then
        return reject("malformed", "slot_sources must be an array")
    end
    if #sources ~= input_frame.SLOT_COUNT then
        return reject(
            "malformed",
            "slot_sources must have exactly " .. input_frame.SLOT_COUNT .. " entries"
        )
    end
    for key in pairs(sources) do
        if
            type(key) ~= "number"
            or key ~= math.floor(key)
            or key < 1
            or key > input_frame.SLOT_COUNT
        then
            return reject("malformed", "slot_sources must use canonical numeric indexes")
        end
    end
    local copied = {}
    for index = 1, input_frame.SLOT_COUNT do
        local source = sources[index]
        if not only_known_fields(source, SOURCE_FIELDS) then
            return reject("malformed", "slot source " .. index .. " has an unknown field")
        end
        if not SOURCE_KINDS[source.kind] then
            return reject("malformed", "slot source " .. index .. " has an unknown kind")
        end
        if source.kind == "bot" then
            if not is_integer(source.seed) then
                return reject("malformed", "bot slot source " .. index .. " needs an integer seed")
            end
        elseif source.seed ~= nil then
            return reject("malformed", "only bot slot sources may carry a seed")
        end
        if source.kind == "policy" then
            if source.policy_id ~= nil and not is_label(source.policy_id) then
                return reject(
                    "malformed",
                    "policy slot source " .. index .. " policy_id must be a non-empty string"
                )
            end
        elseif source.policy_id ~= nil then
            return reject("malformed", "only policy slot sources may carry a policy_id")
        end
        copied[index] = {
            kind = source.kind,
            seed = source.seed,
            policy_id = source.policy_id,
        }
    end
    return copied
end

---@return EnvSlotSourceConfig[]
function env_config.default_slot_sources()
    local sources = {}
    for index = 1, input_frame.SLOT_COUNT do
        sources[index] = index == 1 and { kind = "policy" } or { kind = "neutral" }
    end
    return sources
end

---@param config EnvConfig
---@return integer[]
function env_config.controlled_slots(config)
    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        if config.slot_sources[index].kind == "policy" then
            slots[#slots + 1] = index
        end
    end
    return slots
end

---@param config EnvConfig
---@return integer[]
function env_config.tape_slots(config)
    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        if config.slot_sources[index].kind == "tape" then
            slots[#slots + 1] = index
        end
    end
    return slots
end

-- Validate and complete a caller config. The result is a fresh table: nothing
-- is shared with the caller, so a config cannot mutate underneath an episode.
---@param config any
---@return EnvConfig?, string?, EnvConfigErrorCode?
function env_config.normalize(config)
    if not only_known_fields(config, CONFIG_FIELDS) then
        return reject("malformed", "config must be a table of known fields")
    end
    if config.version ~= nil and config.version ~= env_config.VERSION then
        return reject("unsupported_version", "unsupported env config version")
    end
    if not is_label(config.build) then
        return reject("malformed", "config.build must be a non-empty string")
    end
    if config.source ~= nil and not is_label(config.source) then
        return reject("malformed", "config.source must be a non-empty string")
    end
    if config.content ~= nil and not is_label(config.content) then
        return reject("malformed", "config.content must be a non-empty string")
    end
    if config.fixture ~= nil and not is_label(config.fixture) then
        return reject("malformed", "config.fixture must be a non-empty string")
    end
    if not is_integer(config.seed) then
        return reject("malformed", "config.seed must be a finite integer")
    end
    local duration = config.duration or env_config.DEFAULT_DURATION
    if not is_finite(duration) or duration <= 0 then
        return reject("malformed", "config.duration must be a positive finite number")
    end
    local max_goals = config.max_goals or env_config.DEFAULT_MAX_GOALS
    if not is_integer(max_goals) or max_goals < 1 then
        return reject("malformed", "config.max_goals must be a positive integer")
    end
    local field = config.field or env_config.DEFAULT_FIELD
    if not only_known_fields(field, FIELD_FIELDS) then
        return reject("malformed", "config.field must be a { w, h } table")
    end
    if not is_finite(field.w) or field.w <= 0 or not is_finite(field.h) or field.h <= 0 then
        return reject("malformed", "config.field dimensions must be positive finite numbers")
    end
    local home_team_id = config.home_team_id or env_config.DEFAULT_HOME_TEAM_ID
    local away_team_id = config.away_team_id or env_config.DEFAULT_AWAY_TEAM_ID
    if not teams[home_team_id] then
        return reject("unknown_content", "unknown home team: " .. tostring(home_team_id))
    end
    if not teams[away_team_id] then
        return reject("unknown_content", "unknown away team: " .. tostring(away_team_id))
    end
    for _, key in ipairs({ "home_formation", "away_formation" }) do
        local formation = config[key]
        if formation ~= nil and not formations[formation] then
            return reject("unknown_content", "unknown formation: " .. tostring(formation))
        end
    end
    local home_tactic_id = config.home_tactic_id or env_config.DEFAULT_TACTIC_ID
    local away_tactic_id = config.away_tactic_id or env_config.DEFAULT_TACTIC_ID
    if not tactics[home_tactic_id] then
        return reject("unknown_content", "unknown home tactic: " .. tostring(home_tactic_id))
    end
    if not tactics[away_tactic_id] then
        return reject("unknown_content", "unknown away tactic: " .. tostring(away_tactic_id))
    end
    if config.combat ~= nil and type(config.combat) ~= "boolean" then
        return reject("malformed", "config.combat must be a boolean")
    end
    local profile = config.observation_profile or "representative"
    if not PROFILES[profile] then
        return reject("malformed", "unknown observation profile: " .. tostring(profile))
    end
    local sources, sources_err, sources_code =
        copy_slot_sources(config.slot_sources or env_config.default_slot_sources())
    if not sources then
        return nil, sources_err, sources_code
    end
    local policy_slots = 0
    for _, source in ipairs(sources) do
        if source.kind == "policy" then
            policy_slots = policy_slots + 1
        end
    end
    if policy_slots == 0 then
        return reject("empty_selection", "at least one slot source must be a policy slot")
    end
    if profile == "representative" and policy_slots ~= 1 then
        return reject(
            "profile_mismatch",
            "the representative profile controls exactly one slot; use the team profile"
        )
    end
    local reward_team = config.reward_team or "home"
    if reward_team ~= "home" and reward_team ~= "away" then
        return reject("malformed", "config.reward_team must be home or away")
    end
    local objectives, objectives_err, objectives_code = env_reward.validate_selection(
        config.objective_channels or env_reward.DEFAULT_OBJECTIVES,
        "objective"
    )
    if not objectives then
        return reject(
            "malformed",
            assert(objectives_err) .. " (" .. tostring(objectives_code) .. ")"
        )
    end
    local shaping, shaping_err, shaping_code =
        env_reward.validate_selection(config.shaping_channels or {}, "shaping")
    if not shaping then
        return reject("malformed", assert(shaping_err) .. " (" .. tostring(shaping_code) .. ")")
    end
    if config.max_episode_ticks ~= nil then
        if not is_integer(config.max_episode_ticks) or config.max_episode_ticks < 1 then
            return reject("malformed", "config.max_episode_ticks must be a positive integer")
        end
    end

    local home = teams[home_team_id]
    local away = teams[away_team_id]
    local fixture = config.fixture
        or (
            home_team_id
            .. "-v-"
            .. away_team_id
            .. ";"
            .. (config.home_formation or home.formation)
            .. "-v-"
            .. (config.away_formation or away.formation)
            .. ";"
            .. home_tactic_id
            .. "-v-"
            .. away_tactic_id
            .. ";combat="
            .. tostring(config.combat == true)
        )

    return {
        version = env_config.VERSION,
        build = config.build,
        source = config.source or config.build,
        content = config.content or env_config.DEFAULT_CONTENT,
        fixture = fixture,
        seed = config.seed,
        duration = duration,
        max_goals = max_goals,
        field = { w = field.w, h = field.h },
        home_team_id = home_team_id,
        away_team_id = away_team_id,
        home_formation = config.home_formation,
        away_formation = config.away_formation,
        home_tactic_id = home_tactic_id,
        away_tactic_id = away_tactic_id,
        combat = config.combat == true,
        observation_profile = profile,
        slot_sources = sources,
        reward_team = reward_team,
        objective_channels = objectives,
        shaping_channels = shaping,
        max_episode_ticks = config.max_episode_ticks,
    }
end

-- Resolve authored ids into the tables sim.match consumes. Fixture content is
-- read-only here: the returned tables are the canonical data entries.
---@param config EnvConfig
---@return EnvResolvedFixture
function env_config.resolve(config)
    local home = assert(teams[config.home_team_id], "unknown home team")
    local away = assert(teams[config.away_team_id], "unknown away team")
    return {
        home = home,
        away = away,
        home_tactic = assert(tactics[config.home_tactic_id], "unknown home tactic"),
        away_tactic = assert(tactics[config.away_tactic_id], "unknown away tactic"),
        players_by_id = pool_by_id(),
    }
end

-- The canonical config digest recorded in the tape identity and the manifest.
-- Two runs that agree on this string plus build/content/tuning/seed agree on
-- everything the environment can decide.
---@param config EnvConfig
---@return string
function env_config.digest(config)
    local parts = {
        "env_config=" .. config.version,
        "field=" .. config.field.w .. "x" .. config.field.h,
        "duration=" .. config.duration,
        "max_goals=" .. config.max_goals,
        "tick_rate=" .. fixed_clock.TICK_RATE,
        "home=" .. config.home_team_id,
        "away=" .. config.away_team_id,
        "home_formation=" .. tostring(config.home_formation or "default"),
        "away_formation=" .. tostring(config.away_formation or "default"),
        "home_tactic=" .. config.home_tactic_id,
        "away_tactic=" .. config.away_tactic_id,
        "combat=" .. tostring(config.combat),
        "profile=" .. config.observation_profile,
        "reward_team=" .. config.reward_team,
        "max_episode_ticks=" .. tostring(config.max_episode_ticks or "none"),
    }
    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        local source = config.slot_sources[index]
        slots[index] = index
            .. ":"
            .. source.kind
            .. (source.seed and ("@" .. source.seed) or "")
            .. (source.policy_id and ("#" .. source.policy_id) or "")
    end
    parts[#parts + 1] = "slots=" .. table.concat(slots, ",")
    parts[#parts + 1] = "objectives=" .. table.concat(config.objective_channels, ",")
    parts[#parts + 1] = "shaping=" .. table.concat(config.shaping_channels, ",")
    return table.concat(parts, ";")
end

return env_config
