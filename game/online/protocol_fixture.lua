local protocol = require("game.online.protocol")
local input_frame = require("sim.input_frame")

---@class SessionProtocolFixtureModule
local fixture = {}

---@return SessionRuntimeIdentity
function fixture.runtime()
    return {
        version = protocol.RUNTIME_IDENTITY_VERSION,
        runtime_id = "lovejs",
        runtime_revision = "lovejs.11.5.omp0",
        presentation_id = "presentation.2026-07-25",
        capabilities = { "combat_feedback.v1", "control_channel.v1", "input_channel.v1" },
    }
end

---@return SessionManifest
function fixture.manifest()
    local home = {
        {
            player_id = "ozzo",
            position = "keeper",
        },
        {
            player_id = "zyro_vex",
            position = "forward",
            loadout_id = "loadout_spring_gloves",
            family_id = "unarmed",
        },
        {
            player_id = "mika_olu",
            position = "defender",
            loadout_id = "loadout_emberguard_shield",
            family_id = "guard",
        },
        {
            player_id = "rok_tann",
            position = "midfielder",
            loadout_id = "loadout_vector_blade",
            family_id = "light_melee",
        },
        {
            player_id = "sela_dwin",
            position = "forward",
            loadout_id = "loadout_prism_launcher",
            family_id = "ranged",
        },
    }
    local away = {
        {
            player_id = "gax_oru",
            position = "keeper",
        },
        {
            player_id = "drell",
            position = "defender",
            loadout_id = "loadout_spring_gloves",
            family_id = "unarmed",
        },
        {
            player_id = "morv",
            position = "defender",
            loadout_id = "loadout_emberguard_shield",
            family_id = "guard",
        },
        {
            player_id = "krag",
            position = "midfielder",
            loadout_id = "loadout_vector_blade",
            family_id = "light_melee",
        },
        {
            player_id = "tox_vren",
            position = "forward",
            loadout_id = "loadout_prism_launcher",
            family_id = "ranged",
        },
    }
    local slots = {}
    for index, player_id in ipairs({
        "zyro_vex",
        "mika_olu",
        "rok_tann",
        "sela_dwin",
        "drell",
        "morv",
        "krag",
        "tox_vren",
    }) do
        local slot = assert(input_frame.slot(index))
        slots[index] = { slot = slot.id, team = slot.team, player_id = player_id }
    end
    return {
        version = protocol.MANIFEST_VERSION,
        session_id = "session_alpha",
        protocol_version = protocol.VERSION,
        input_version = protocol.CURRENT_VERSIONS.input,
        snapshot_version = protocol.CURRENT_VERSIONS.snapshot,
        tape_version = protocol.CURRENT_VERSIONS.tape,
        combat_schema_version = protocol.CURRENT_VERSIONS.combat,
        build_id = "build.97b60ea",
        source_id = "source.97b60ea",
        content_id = "content.omp3.v1",
        tuning_id = "tuning.omp3.v1",
        match_config_id = "match_config.direct_host.v1",
        fixture_id = "fixture.default_mixed.v1",
        arena_id = "arena.goliseo",
        combat_rules_id = "combat_interaction.accepted_2026_07_23",
        gameplay_ai_policy_id = "gameplay_ai.provisional_issue_112",
        combat_status = "provisional_114",
        seed = 20001,
        tick_rate = 60,
        duration_ticks = 7200,
        max_goals = 5,
        teams = {
            { team = "home", team_id = "team_nova", roster = home },
            { team = "away", team_id = "team_void", roster = away },
        },
        slots = slots,
    }
end

---@return SessionSlotProducer[]
function fixture.assignments()
    local manifest = fixture.manifest()
    local result = {}
    for index, assignment in ipairs(manifest.slots) do
        if index <= 6 then
            result[index] = {
                slot = assignment.slot,
                team = assignment.team,
                player_id = assignment.player_id,
                producer_kind = "peer",
                producer_id = index == 1 and "host" or ("guest_" .. tostring(index)),
            }
        else
            result[index] = {
                slot = assignment.slot,
                team = assignment.team,
                player_id = assignment.player_id,
                producer_kind = "bot",
                producer_id = "bot_" .. assignment.slot,
                bot_seed = 21000 + index,
            }
        end
    end
    return result
end

---@return SessionControlMessage[]
function fixture.messages()
    local manifest = fixture.manifest()
    local manifest_id = protocol.manifest_id(manifest)
    local assignment_id = protocol.assignment_id(fixture.assignments(), 1)
    local bodies = {
        { "handshake", { role = "host", runtime = fixture.runtime() } },
        { "manifest_proposal", { manifest_id = manifest_id, manifest = manifest } },
        { "manifest_accept", { manifest_id = manifest_id } },
        { "peer_assignment", { assigned_peer_id = "host", role = "host" } },
        {
            "slot_assignment",
            {
                manifest_id = manifest_id,
                assignment_id = assignment_id,
                assignments = fixture.assignments(),
            },
        },
        { "ready", { manifest_id = manifest_id, assignment_id = assignment_id, ready = true } },
        {
            "countdown",
            {
                manifest_id = manifest_id,
                countdown_id = "countdown_1",
                remaining_ticks = 180,
                first_input_tick = 0,
            },
        },
        {
            "start",
            {
                manifest_id = manifest_id,
                countdown_id = "countdown_1",
                first_input_tick = 0,
            },
        },
        { "match_phase", { phase = "playing", tick = 1, home_score = 0, away_score = 0 } },
        { "hash_report", { tick = 60, boundary_hash = "0123456789abcdef" } },
        {
            "result_ack",
            {
                final_tick = 7200,
                home_score = 2,
                away_score = 1,
                final_hash = "fedcba9876543210",
            },
        },
        { "abort", { code = "host_abort" } },
        {
            "disconnect",
            { target_peer_id = "guest_2", code = "peer_left" },
        },
    }
    local result = {}
    for index, definition in ipairs(bodies) do
        result[index] = assert(
            protocol.new(definition[1], manifest.session_id, "host", index - 1, definition[2])
        )
    end
    return result
end

return fixture
