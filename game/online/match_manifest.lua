-- Content-derived session manifest for the online match flow.
--
-- `game.online.protocol_fixture` pins a manifest whose team and player ids exist
-- only in the fixture; it cannot be turned back into a playable match. This
-- module is the builder the lobby document said would replace it "when the
-- online match flow lands": every identity is derived from the shipped `data/`
-- content and `sim.tuning`, so two peers running the same build compute the same
-- manifest byte for byte, and the frozen manifest alone is enough to rebuild
-- boundary zero.
--
-- It is deliberately deterministic and free of clocks, randomness, and
-- transports. `template` is what the lobby proposes; `resolve` is what the match
-- flow reads back out of the frozen manifest.

local fnv1a64 = require("core.fnv1a64")
local arenas = require("data.arenas")
local loadouts = require("data.loadouts")
local players_data = require("data.players")
local teams_data = require("data.teams")
local build_info = require("game.build_info")
local protocol = require("game.online.protocol")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local sim_match = require("sim.match")
local tuning = require("sim.tuning")

---@class OnlineMatchContent
---@field home TeamData
---@field away TeamData
---@field arena ArenaData
---@field ownership InputOwnership

---@class OnlineMatchManifestModule
local match_manifest = {}

match_manifest.HOME_TEAM_ID = "nebula"
match_manifest.AWAY_TEAM_ID = "orion"
match_manifest.ARENA_ID = "helios_crown"
match_manifest.DEFAULT_SESSION_ID = "session_goliseo"
match_manifest.DEFAULT_SEED = 20003
match_manifest.DEFAULT_DURATION_TICKS = 3600
match_manifest.DEFAULT_MAX_GOALS = 5

-- The combat disposition is still provisional: #114 has not accepted a default,
-- so the manifest says so rather than pretending. When #114 lands this constant
-- and the fixture's are the only two places that change.
match_manifest.COMBAT_STATUS = "provisional_114"
match_manifest.COMBAT_RULES_ID = "combat_interaction.accepted_2026_07_23"
match_manifest.GAMEPLAY_AI_POLICY_ID = "gameplay_ai.provisional_issue_112"
match_manifest.MATCH_CONFIG_ID = "match_config.direct_host.v1"
match_manifest.FIXTURE_ID = "fixture.content_derived.v1"

---@return table<string, PlayerData>
local function players_by_id()
    local result = {}
    for _, player in ipairs(players_data) do
        result[player.id] = player
    end
    return result
end

---@param value string
---@return string
local function digest(value)
    return fnv1a64.hash(value):sub(1, 16)
end

---@param team TeamData
---@param pool table<string, PlayerData>
---@return string
local function team_content(team, pool)
    local parts = { team.id, team.name, team.formation }
    for _, id in ipairs(team.roster) do
        local player = assert(pool[id], "unknown roster player: " .. tostring(id))
        local loadout = player.loadout_id
                and assert(
                    loadouts[player.loadout_id],
                    "unknown loadout: " .. tostring(player.loadout_id)
                )
            or nil
        parts[#parts + 1] = ("%s/%s/%d/%s/%s/%s"):format(
            player.id,
            player.position,
            player.number,
            player.presentation_id,
            player.loadout_id or "-",
            loadout and loadout.family_id or "-"
        )
    end
    return table.concat(parts, ";")
end

-- One digest over exactly the content the simulation reads: both rosters, their
-- loadouts, and the arena. A content edit therefore mints a new `content_id`,
-- which is what a guest compares its build against.
---@return string
function match_manifest.content_id()
    local pool = players_by_id()
    local home = assert(teams_data[match_manifest.HOME_TEAM_ID], "unknown home team")
    local away = assert(teams_data[match_manifest.AWAY_TEAM_ID], "unknown away team")
    local arena = assert(arenas[match_manifest.ARENA_ID], "unknown arena")
    return "content."
        .. digest(table.concat({
            team_content(home, pool),
            team_content(away, pool),
            arena.id,
        }, "|"))
end

-- Tuning is already serialisable for the OMP-1 determinism fixture, so the
-- identity is the same string that campaign pins rather than a second one.
---@return string
function match_manifest.tuning_id()
    return "tuning." .. digest(tuning.serialize())
end

---@return string
function match_manifest.build_id()
    return "build."
        .. digest(build_info.name .. "/" .. build_info.version .. "/" .. build_info.channel)
end

---@param team TeamData
---@param side SessionTeam
---@param pool table<string, PlayerData>
---@return SessionTeamManifest
local function team_manifest(team, side, pool)
    ---@type SessionRosterPlayer[]
    local roster = {}
    for index, id in ipairs(team.roster) do
        local player = assert(pool[id], "unknown roster player: " .. tostring(id))
        if player.position == "keeper" then
            assert(index == 1, "a manifest roster places its keeper first")
            roster[index] = { player_id = player.id, position = player.position }
        else
            local loadout_id =
                assert(player.loadout_id, "an online outfielder needs a fixed loadout: " .. id)
            local loadout = assert(loadouts[loadout_id], "unknown loadout: " .. loadout_id)
            roster[index] = {
                player_id = player.id,
                position = player.position,
                loadout_id = loadout_id,
                family_id = loadout.family_id,
            }
        end
    end
    return { team = side, team_id = team.id, roster = roster }
end

-- The canonical manifest this build proposes. `mode` only changes how the eight
-- slots are shared; the content, the slots themselves, and every identity are
-- identical across modes, which is exactly why a 1v1 and a 4v4 lobby of the same
-- build agree on everything except `match_mode`.
---@param mode SessionMatchMode?
---@return SessionManifest
function match_manifest.template(mode)
    local pool = players_by_id()
    local home = assert(teams_data[match_manifest.HOME_TEAM_ID], "unknown home team")
    local away = assert(teams_data[match_manifest.AWAY_TEAM_ID], "unknown away team")
    -- The slot table is derived from the same helper the simulation uses, so the
    -- manifest cannot describe a seating the match would not build.
    local ownership = sim_match.ownership_for_teams(home, away, pool)
    ---@type SessionManifestSlot[]
    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        local slot = assert(input_frame.slot(index))
        local assignment = assert(ownership.slots[index], "canonical ownership slot is missing")
        slots[index] = {
            slot = slot.id,
            team = slot.team,
            player_id = assignment.player_id,
        }
    end
    return {
        version = protocol.MANIFEST_VERSION,
        session_id = match_manifest.DEFAULT_SESSION_ID,
        protocol_version = protocol.VERSION,
        input_version = protocol.CURRENT_VERSIONS.input,
        snapshot_version = protocol.CURRENT_VERSIONS.snapshot,
        tape_version = protocol.CURRENT_VERSIONS.tape,
        combat_schema_version = protocol.CURRENT_VERSIONS.combat,
        build_id = match_manifest.build_id(),
        source_id = match_manifest.build_id(),
        content_id = match_manifest.content_id(),
        tuning_id = match_manifest.tuning_id(),
        match_config_id = match_manifest.MATCH_CONFIG_ID,
        fixture_id = match_manifest.FIXTURE_ID,
        arena_id = match_manifest.ARENA_ID,
        combat_rules_id = match_manifest.COMBAT_RULES_ID,
        gameplay_ai_policy_id = match_manifest.GAMEPLAY_AI_POLICY_ID,
        combat_status = match_manifest.COMBAT_STATUS,
        seed = match_manifest.DEFAULT_SEED,
        tick_rate = fixed_clock.TICK_RATE,
        duration_ticks = match_manifest.DEFAULT_DURATION_TICKS,
        max_goals = match_manifest.DEFAULT_MAX_GOALS,
        match_mode = mode or "4v4",
        teams = {
            team_manifest(home, "home", pool),
            team_manifest(away, "away", pool),
        },
        slots = slots,
    }
end

-- Turn a frozen manifest back into the content the simulation needs. Everything
-- is looked up by the manifest's own ids and then checked against the manifest's
-- canonical slot table, so a manifest that names content this build does not
-- have fails here rather than producing a quietly different match.
---@param manifest SessionManifest
---@return OnlineMatchContent?, string?
function match_manifest.resolve(manifest)
    local ok, err = protocol.validate_manifest(manifest)
    if not ok then
        return nil, err
    end
    local home = teams_data[manifest.teams[1].team_id]
    local away = teams_data[manifest.teams[2].team_id]
    if not home or not away then
        return nil, "the manifest names a team this build does not have"
    end
    local arena = arenas[manifest.arena_id]
    if not arena then
        return nil, "the manifest names an arena this build does not have"
    end
    local pool = players_by_id()
    for _, team in ipairs(manifest.teams) do
        for _, player in ipairs(team.roster) do
            local known = pool[player.player_id]
            if not known then
                return nil, "the manifest names a player this build does not have"
            end
            if known.position ~= player.position then
                return nil, "the manifest disagrees about a player's position"
            end
            if player.loadout_id ~= nil and known.loadout_id ~= player.loadout_id then
                return nil, "the manifest disagrees about a player's fixed loadout"
            end
        end
    end
    local built = sim_match.ownership_for_teams(home, away, pool)
    for index = 1, input_frame.SLOT_COUNT do
        local declared = manifest.slots[index]
        local local_slot = assert(built.slots[index], "canonical ownership slot is missing")
        if declared.player_id ~= local_slot.player_id then
            return nil, "the manifest disagrees about canonical slot " .. declared.slot
        end
    end
    return { home = home, away = away, arena = arena, ownership = built }
end

return match_manifest
