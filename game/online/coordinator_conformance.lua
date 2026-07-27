local fnv1a64 = require("core.fnv1a64")
local driver = require("game.online.coordinator_driver")
local input_frame = require("sim.input_frame")

---@class SessionCoordinatorGolden
---@field full_transcript_id string
---@field full_trace_digest string
---@field full_message_count integer
---@field bot_transcript_id string
---@field bot_sources string
---@field solo_sources string

---@class SessionCoordinatorConformanceReport
---@field full_transcript_id string
---@field bot_transcript_id string
---@field message_count integer

---@class SessionCoordinatorConformanceModule
local conformance = {}

---@type SessionCoordinatorGolden
conformance.GOLDEN = {
    full_transcript_id = "0e75eac6c612418c",
    full_trace_digest = "2fa7235df619b00a",
    full_message_count = 51,
    bot_transcript_id = "861c73e35567ee15",
    bot_sources = "home_1=peer:host:nil|home_2=peer:guest.1:nil|home_3=peer:guest.2:nil|"
        .. "home_4=bot:bot.home_4:51677|away_1=bot:bot.away_1:59596|away_2=bot:bot.away_2:67515|"
        .. "away_3=bot:bot.away_3:75434|away_4=bot:bot.away_4:83353",
    solo_sources = "home_1=peer:host:nil|home_2=bot:bot.home_2:35839|home_3=bot:bot.home_3:43758|"
        .. "home_4=bot:bot.home_4:51677|away_1=bot:bot.away_1:59596|away_2=bot:bot.away_2:67515|"
        .. "away_3=bot:bot.away_3:75434|away_4=bot:bot.away_4:83353",
}

-- A flat, reviewable rendering of who owns each canonical slot at freeze.
---@param session CoordinatorDriver
---@return string
function conformance.sources(session)
    local freeze = assert(session:host().state.freeze, "the session never froze")
    local parts = {}
    for index = 1, input_frame.SLOT_COUNT do
        local slot = assert(input_frame.slot(index))
        local producer = assert(freeze.sources[slot.id], "a canonical slot has no source")
        parts[#parts + 1] = ("%s=%s:%s:%s"):format(
            slot.id,
            producer.producer_kind,
            producer.producer_id,
            tostring(producer.bot_seed)
        )
    end
    return table.concat(parts, "|")
end

---@param guest_count integer
---@return CoordinatorDriver
function conformance.session(guest_count)
    local session = driver.new({ guest_count = guest_count })
    session:reach_start(3, 0)
    assert(session:all_started(), "the canonical session never reached its start boundary")
    session:play_out(2, 1)
    assert(session:all_terminal("completed"), "the canonical session never completed")
    return session
end

---@return SessionCoordinatorConformanceReport
function conformance.verify()
    local full = conformance.session(7)
    local golden = conformance.GOLDEN
    local full_transcript_id = full:transcript_id()
    assert(
        full_transcript_id == golden.full_transcript_id,
        ("coordinator transcript golden changed: expected %s, got %s"):format(
            golden.full_transcript_id,
            full_transcript_id
        )
    )
    local trace_digest = fnv1a64.hash(full:trace())
    assert(
        trace_digest == golden.full_trace_digest,
        ("coordinator trace golden changed: expected %s, got %s"):format(
            golden.full_trace_digest,
            trace_digest
        )
    )
    assert(
        #full.transcript == golden.full_message_count,
        ("coordinator message count changed: expected %d, got %d"):format(
            golden.full_message_count,
            #full.transcript
        )
    )
    local full_sources = conformance.sources(full)

    local bots = conformance.session(2)
    local bot_transcript_id = bots:transcript_id()
    assert(
        bot_transcript_id == golden.bot_transcript_id,
        ("bot-filled transcript golden changed: expected %s, got %s"):format(
            golden.bot_transcript_id,
            bot_transcript_id
        )
    )
    local bot_sources = conformance.sources(bots)
    assert(
        bot_sources == golden.bot_sources,
        ("bot-filled ownership golden changed: expected %s, got %s"):format(
            golden.bot_sources,
            bot_sources
        )
    )

    local solo_sources = conformance.sources(conformance.session(0))
    assert(
        solo_sources == golden.solo_sources,
        ("solo ownership golden changed: expected %s, got %s"):format(
            golden.solo_sources,
            solo_sources
        )
    )
    assert(full_sources ~= bot_sources, "human and bot-filled ownership must differ")

    return {
        full_transcript_id = full_transcript_id,
        bot_transcript_id = bot_transcript_id,
        message_count = #full.transcript,
    }
end

---@param report SessionCoordinatorConformanceReport
---@return string
function conformance.marker(report)
    return table.concat({
        "GC_COORDINATOR",
        "golden",
        "schema=1",
        "full_transcript_id=" .. report.full_transcript_id,
        "bot_transcript_id=" .. report.bot_transcript_id,
        "messages=" .. tostring(report.message_count),
    }, "|")
end

return conformance
