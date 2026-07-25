local Vec2 = require("core.vec2")
local action_families = require("data.action_families")
local loadouts = require("data.loadouts")

---@class CombatSnapshotWriter
---@field literal fun(value: string)
---@field name fun(value: string)
---@field scalar fun(value: any)
---@field vec fun(value: Vec2)

---@class CombatSnapshotModule
local combat_snapshot = {}

combat_snapshot.VERSION = 1

combat_snapshot.STATE_FIELDS = {
    "version",
    "tick",
    "player_ids",
    "players",
    "projectiles",
    "events",
    "next_source_sequence",
}

combat_snapshot.PLAYER_FIELDS = {
    "loadout_id",
    "family_id",
    "phase",
    "phase_ticks",
    "cooldown_ticks",
    "source_sequence",
    "contacted",
    "release_latched",
    "control_held",
    "projectile_spawned",
    "forced_state",
    "forced_ticks",
    "chain_ticks",
    "immunity_ticks",
}

combat_snapshot.PROJECTILE_FIELDS = {
    "family_id",
    "source_index",
    "source_sequence",
    "pos",
    "dir",
    "remaining_ticks",
}

combat_snapshot.EVENT_FIELDS = {
    "kind",
    "tick",
    "family_id",
    "source_index",
    "target_index",
    "source_sequence",
    "result",
    "x",
    "y",
    "interruption_ticks",
    "displacement_px",
}

---@param values string[]
---@return table<string, boolean>
local function field_set(values)
    local result = {}
    for _, value in ipairs(values) do
        result[value] = true
    end
    return result
end

local STATE_FIELD_SET = field_set(combat_snapshot.STATE_FIELDS)
local PLAYER_FIELD_SET = field_set(combat_snapshot.PLAYER_FIELDS)
local PROJECTILE_FIELD_SET = field_set(combat_snapshot.PROJECTILE_FIELDS)
local EVENT_FIELD_SET = field_set(combat_snapshot.EVENT_FIELDS)
local VECTOR_FIELD_SET = field_set({ "x", "y" })
local PHASES = field_set({ "ready", "windup", "active", "aim", "guard", "recovery" })
local FORCED_STATES = field_set({ "stagger", "knockback" })
local EVENT_KINDS = field_set({
    "commit",
    "projectile_spawn",
    "projectile_expire",
    "contact",
    "ball_spill",
    "forced",
    "guard_recoil",
})
local CONTACT_RESULTS = field_set({ "hit", "extended", "guarded", "immune", "superseded" })
local MAX_INTEGER = 2147483647

---@param value any
---@return boolean
local function is_finite_number(value)
    return type(value) == "number" and value == value and value ~= math.huge and value ~= -math.huge
end

---@param value any
---@return boolean
local function is_non_negative_integer(value)
    return is_finite_number(value) and value == math.floor(value) and value >= 0
end

---@param value any
---@param allowed table<string, boolean>
---@param path string
local function assert_fields(value, allowed, path)
    assert(type(value) == "table", path .. " must be a table")
    for key in pairs(value) do
        assert(
            type(key) == "string" and allowed[key],
            path .. " has unknown field " .. tostring(key)
        )
    end
end

---@param value any
---@param path string
---@param expected integer?
local function assert_array(value, path, expected)
    assert(type(value) == "table", path .. " must be an array")
    local count = #value
    if expected then
        assert(count == expected, path .. " has the wrong length")
    end
    for key in pairs(value) do
        assert(
            type(key) == "number" and key == math.floor(key) and key >= 1 and key <= count,
            path .. " is not a canonical array"
        )
    end
    for index = 1, count do
        assert(value[index] ~= nil, path .. " has a hole")
    end
end

---@param value any
---@param path string
---@param make_vec boolean
---@return Vec2
local function copy_vec(value, path, make_vec)
    assert_fields(value, VECTOR_FIELD_SET, path)
    assert(is_finite_number(value.x), path .. ".x must be finite")
    assert(is_finite_number(value.y), path .. ".y must be finite")
    if make_vec then
        return Vec2.new(value.x, value.y)
    end
    return { x = value.x, y = value.y }
end

---@param value any
---@param path string
---@return integer
local function copy_counter(value, path)
    assert(
        is_non_negative_integer(value) and value <= MAX_INTEGER,
        path .. " must be a bounded non-negative integer"
    )
    return value
end

---@param value any
---@param path string
---@return integer?
local function copy_optional_sequence(value, path)
    if value == nil then
        return nil
    end
    assert(
        is_non_negative_integer(value) and value >= 1 and value <= MAX_INTEGER,
        path .. " must be a bounded positive integer"
    )
    return value
end

---@param value any
---@param path string
---@param count integer
---@return integer?
local function copy_optional_index(value, path, count)
    if value == nil then
        return nil
    end
    assert(
        is_non_negative_integer(value) and value >= 1 and value <= count,
        path .. " must be a valid player index"
    )
    return value
end

---@param source any
---@param path string
---@return CombatPlayerState
local function copy_player(source, path)
    assert_fields(source, PLAYER_FIELD_SET, path)
    assert(source.loadout_id == nil or type(source.loadout_id) == "string", path .. ".loadout_id")
    assert(source.family_id == nil or action_families[source.family_id], path .. ".family_id")
    assert(PHASES[source.phase], path .. ".phase is unsupported")
    assert(type(source.contacted) == "boolean", path .. ".contacted must be boolean")
    assert(type(source.release_latched) == "boolean", path .. ".release_latched must be boolean")
    assert(type(source.control_held) == "boolean", path .. ".control_held must be boolean")
    assert(
        type(source.projectile_spawned) == "boolean",
        path .. ".projectile_spawned must be boolean"
    )
    assert(
        source.forced_state == nil or FORCED_STATES[source.forced_state],
        path .. ".forced_state is unsupported"
    )
    if source.loadout_id then
        local loadout = assert(loadouts[source.loadout_id], path .. ".loadout_id is unknown")
        assert(
            loadout.family_id == source.family_id,
            path .. " loadout/family pair is inconsistent"
        )
    else
        assert(source.family_id == nil, path .. " family requires a loadout")
    end
    return {
        loadout_id = source.loadout_id,
        family_id = source.family_id,
        phase = source.phase,
        phase_ticks = copy_counter(source.phase_ticks, path .. ".phase_ticks"),
        cooldown_ticks = copy_counter(source.cooldown_ticks, path .. ".cooldown_ticks"),
        source_sequence = copy_optional_sequence(
            source.source_sequence,
            path .. ".source_sequence"
        ),
        contacted = source.contacted,
        release_latched = source.release_latched,
        control_held = source.control_held,
        projectile_spawned = source.projectile_spawned,
        forced_state = source.forced_state,
        forced_ticks = copy_counter(source.forced_ticks, path .. ".forced_ticks"),
        chain_ticks = copy_counter(source.chain_ticks, path .. ".chain_ticks"),
        immunity_ticks = copy_counter(source.immunity_ticks, path .. ".immunity_ticks"),
    }
end

---@param source any
---@param path string
---@param player_count integer
---@param make_vec boolean
---@return CombatProjectile
local function copy_projectile(source, path, player_count, make_vec)
    assert_fields(source, PROJECTILE_FIELD_SET, path)
    assert(action_families[source.family_id], path .. ".family_id is unknown")
    local source_index =
        assert(copy_optional_index(source.source_index, path .. ".source_index", player_count))
    local source_sequence =
        assert(copy_optional_sequence(source.source_sequence, path .. ".source_sequence"))
    local remaining_ticks = copy_counter(source.remaining_ticks, path .. ".remaining_ticks")
    assert(remaining_ticks >= 1, path .. ".remaining_ticks must be positive")
    return {
        family_id = source.family_id,
        source_index = source_index,
        source_sequence = source_sequence,
        pos = copy_vec(source.pos, path .. ".pos", make_vec),
        dir = copy_vec(source.dir, path .. ".dir", make_vec),
        remaining_ticks = remaining_ticks,
    }
end

---@param source any
---@param path string
---@param player_count integer
---@return CombatEvent
local function copy_event(source, path, player_count)
    assert_fields(source, EVENT_FIELD_SET, path)
    assert(EVENT_KINDS[source.kind], path .. ".kind is unsupported")
    local tick = copy_counter(source.tick, path .. ".tick")
    assert(source.family_id == nil or action_families[source.family_id], path .. ".family_id")
    assert(source.result == nil or CONTACT_RESULTS[source.result], path .. ".result is unsupported")
    assert(is_finite_number(source.x), path .. ".x must be finite")
    assert(is_finite_number(source.y), path .. ".y must be finite")
    if source.interruption_ticks ~= nil then
        copy_counter(source.interruption_ticks, path .. ".interruption_ticks")
    end
    if source.displacement_px ~= nil then
        assert(is_finite_number(source.displacement_px), path .. ".displacement_px must be finite")
    end
    return {
        kind = source.kind,
        tick = tick,
        family_id = source.family_id,
        source_index = copy_optional_index(
            source.source_index,
            path .. ".source_index",
            player_count
        ),
        target_index = copy_optional_index(
            source.target_index,
            path .. ".target_index",
            player_count
        ),
        source_sequence = copy_optional_sequence(
            source.source_sequence,
            path .. ".source_sequence"
        ),
        result = source.result,
        x = source.x,
        y = source.y,
        interruption_ticks = source.interruption_ticks,
        displacement_px = source.displacement_px,
    }
end

---@param source any
---@param path string
---@param match_state MatchState
---@param make_vec boolean
---@return CombatMatchState
function combat_snapshot.copy(source, path, match_state, make_vec)
    assert_fields(source, STATE_FIELD_SET, path)
    assert(source.version == combat_snapshot.VERSION, path .. ".version is unsupported")
    assert_array(source.player_ids, path .. ".player_ids", #match_state.players)
    assert_array(source.players, path .. ".players", #match_state.players)
    assert_array(source.projectiles, path .. ".projectiles")
    assert_array(source.events, path .. ".events")
    local result = {
        version = combat_snapshot.VERSION,
        tick = copy_counter(source.tick, path .. ".tick"),
        player_ids = {},
        players = {},
        projectiles = {},
        events = {},
        next_source_sequence = copy_counter(
            source.next_source_sequence,
            path .. ".next_source_sequence"
        ),
    }
    assert(result.next_source_sequence >= 1, path .. ".next_source_sequence must be positive")
    if match_state.slot_mode then
        assert(result.tick == match_state.input_tick, path .. ".tick must match state.input_tick")
    end
    for index, player in ipairs(match_state.players) do
        local player_id = source.player_ids[index]
        assert(type(player_id) == "string" and player_id ~= "", path .. ".player_ids is invalid")
        assert(player_id == player.id, path .. ".player_ids does not match the fixture")
        result.player_ids[index] = player_id
        result.players[index] = copy_player(source.players[index], path .. ".players." .. index)
        if player.is_keeper then
            assert(
                result.players[index].loadout_id == nil,
                path .. ".players." .. index .. " keeper cannot have a combat loadout"
            )
        end
    end
    for index, projectile in ipairs(source.projectiles) do
        result.projectiles[index] = copy_projectile(
            projectile,
            path .. ".projectiles." .. index,
            #match_state.players,
            make_vec
        )
    end
    for index, event in ipairs(source.events) do
        result.events[index] = copy_event(event, path .. ".events." .. index, #match_state.players)
        assert(
            result.events[index].tick == result.tick - 1,
            path .. ".events." .. index .. ".tick must name the causal input tick"
        )
    end
    return result
end

---@param source CombatMatchState
---@param make_vec boolean
---@return CombatMatchState
function combat_snapshot.copy_owned(source, make_vec)
    local result = {
        version = source.version,
        tick = source.tick,
        player_ids = {},
        players = {},
        projectiles = {},
        events = {},
        next_source_sequence = source.next_source_sequence,
    }
    for index, player_id in ipairs(source.player_ids) do
        result.player_ids[index] = player_id
    end
    for index, player in ipairs(source.players) do
        local copied = {}
        for _, field in ipairs(combat_snapshot.PLAYER_FIELDS) do
            copied[field] = player[field]
        end
        result.players[index] = copied
    end
    for index, projectile in ipairs(source.projectiles) do
        result.projectiles[index] = {
            family_id = projectile.family_id,
            source_index = projectile.source_index,
            source_sequence = projectile.source_sequence,
            pos = make_vec and Vec2.new(projectile.pos.x, projectile.pos.y)
                or { x = projectile.pos.x, y = projectile.pos.y },
            dir = make_vec and Vec2.new(projectile.dir.x, projectile.dir.y)
                or { x = projectile.dir.x, y = projectile.dir.y },
            remaining_ticks = projectile.remaining_ticks,
        }
    end
    for index, event in ipairs(source.events) do
        local copied = {}
        for _, field in ipairs(combat_snapshot.EVENT_FIELDS) do
            copied[field] = event[field]
        end
        result.events[index] = copied
    end
    return result
end

---@param writer CombatSnapshotWriter
---@param combat_state CombatMatchState
function combat_snapshot.append(writer, combat_state)
    writer.literal("GCCS;")
    for _, field in ipairs(combat_snapshot.STATE_FIELDS) do
        writer.name(field)
        local value = combat_state[field]
        if field == "player_ids" then
            writer.scalar(#value)
            for _, player_id in ipairs(value) do
                writer.scalar(player_id)
            end
        elseif field == "players" then
            writer.scalar(#value)
            for _, player in ipairs(value) do
                for _, player_field in ipairs(combat_snapshot.PLAYER_FIELDS) do
                    writer.name(player_field)
                    writer.scalar(player[player_field])
                end
            end
        elseif field == "projectiles" then
            writer.scalar(#value)
            for _, projectile in ipairs(value) do
                for _, projectile_field in ipairs(combat_snapshot.PROJECTILE_FIELDS) do
                    writer.name(projectile_field)
                    if projectile_field == "pos" or projectile_field == "dir" then
                        writer.vec(projectile[projectile_field])
                    else
                        writer.scalar(projectile[projectile_field])
                    end
                end
            end
        elseif field == "events" then
            writer.scalar(#value)
            for _, event in ipairs(value) do
                for _, event_field in ipairs(combat_snapshot.EVENT_FIELDS) do
                    writer.name(event_field)
                    writer.scalar(event[event_field])
                end
            end
        else
            writer.scalar(value)
        end
    end
end

---@param left any
---@param right any
---@return boolean
local function same_scalar(left, right)
    if left ~= right then
        return false
    end
    if left == 0 and right == 0 then
        return 1 / left == 1 / right
    end
    return true
end

---@param path string
---@param expected any
---@param actual any
---@return MatchSnapshotDifference
local function difference(path, expected, actual)
    return { path = path, expected = expected, actual = actual }
end

---@param left CombatMatchState?
---@param right CombatMatchState?
---@param path string
---@return MatchSnapshotDifference?
function combat_snapshot.first_difference(left, right, path)
    if (left == nil) ~= (right == nil) then
        return difference(path, left, right)
    end
    if left == nil or right == nil then
        return nil
    end
    for _, field in ipairs(combat_snapshot.STATE_FIELDS) do
        local field_path = path .. "." .. field
        local a = left[field]
        local b = right[field]
        if field == "player_ids" then
            if #a ~= #b then
                return difference(field_path .. ".length", #a, #b)
            end
            for index = 1, #a do
                if not same_scalar(a[index], b[index]) then
                    return difference(field_path .. "." .. index, a[index], b[index])
                end
            end
        elseif field == "players" or field == "events" then
            if #a ~= #b then
                return difference(field_path .. ".length", #a, #b)
            end
            local fields = field == "players" and combat_snapshot.PLAYER_FIELDS
                or combat_snapshot.EVENT_FIELDS
            for index = 1, #a do
                for _, child in ipairs(fields) do
                    if not same_scalar(a[index][child], b[index][child]) then
                        return difference(
                            field_path .. "." .. index .. "." .. child,
                            a[index][child],
                            b[index][child]
                        )
                    end
                end
            end
        elseif field == "projectiles" then
            if #a ~= #b then
                return difference(field_path .. ".length", #a, #b)
            end
            for index = 1, #a do
                for _, child in ipairs(combat_snapshot.PROJECTILE_FIELDS) do
                    if child == "pos" or child == "dir" then
                        for _, axis in ipairs({ "x", "y" }) do
                            if not same_scalar(a[index][child][axis], b[index][child][axis]) then
                                return difference(
                                    field_path .. "." .. index .. "." .. child .. "." .. axis,
                                    a[index][child][axis],
                                    b[index][child][axis]
                                )
                            end
                        end
                    elseif not same_scalar(a[index][child], b[index][child]) then
                        return difference(
                            field_path .. "." .. index .. "." .. child,
                            a[index][child],
                            b[index][child]
                        )
                    end
                end
            end
        elseif not same_scalar(a, b) then
            return difference(field_path, a, b)
        end
    end
    return nil
end

return combat_snapshot
