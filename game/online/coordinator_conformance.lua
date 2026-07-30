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
---@field duo_transcript_id string -- A full 1v1: two humans, four owned slots each.
---@field duo_sources string
---@field duo_owned string
---@field pair_transcript_id string -- A full 2v2: four humans, two owned slots each.
---@field pair_sources string
---@field pair_owned string
---@field full_owned string

---@class SessionCoordinatorConformanceReport
---@field full_transcript_id string
---@field bot_transcript_id string
---@field message_count integer

---@class SessionCoordinatorConformanceModule
local conformance = {}

---@type SessionCoordinatorGolden
conformance.GOLDEN = {
    full_transcript_id = "c45f138f0f80ace5",
    full_trace_digest = "2fa7235df619b00a",
    full_message_count = 51,
    bot_transcript_id = "8589a4b030e9ad12",
    bot_sources = "home_1=peer:host:nil|home_2=peer:guest.1:nil|home_3=peer:guest.2:nil|"
        .. "home_4=bot:bot.home_4:51677|away_1=bot:bot.away_1:59596|away_2=bot:bot.away_2:67515|"
        .. "away_3=bot:bot.away_3:75434|away_4=bot:bot.away_4:83353",
    solo_sources = "home_1=peer:host:nil|home_2=bot:bot.home_2:35839|home_3=bot:bot.home_3:43758|"
        .. "home_4=bot:bot.home_4:51677|away_1=bot:bot.away_1:59596|away_2=bot:bot.away_2:67515|"
        .. "away_3=bot:bot.away_3:75434|away_4=bot:bot.away_4:83353",
    duo_transcript_id = "1a2b3fd0a0881042",
    duo_sources = "home_1=peer:host:nil|home_2=peer:host:nil|home_3=peer:host:nil|"
        .. "home_4=peer:host:nil|away_1=peer:guest.1:nil|away_2=peer:guest.1:nil|"
        .. "away_3=peer:guest.1:nil|away_4=peer:guest.1:nil",
    duo_owned = "host=[home_1,home_2,home_3,home_4]>home_1|"
        .. "guest.1=[away_1,away_2,away_3,away_4]>away_1",
    pair_transcript_id = "f1801a4c5c6cfa31",
    pair_sources = "home_1=peer:host:nil|home_2=peer:host:nil|home_3=peer:guest.1:nil|"
        .. "home_4=peer:guest.1:nil|away_1=peer:guest.2:nil|away_2=peer:guest.2:nil|"
        .. "away_3=peer:guest.3:nil|away_4=peer:guest.3:nil",
    pair_owned = "host=[home_1,home_2]>home_1|guest.1=[home_3,home_4]>home_3|"
        .. "guest.2=[away_1,away_2]>away_1|guest.3=[away_3,away_4]>away_3",
    full_owned = "host=[home_1]>home_1|guest.1=[home_2]>home_2|guest.2=[home_3]>home_3|"
        .. "guest.3=[home_4]>home_4|guest.4=[away_1]>away_1|guest.5=[away_2]>away_2|"
        .. "guest.6=[away_3]>away_3|guest.7=[away_4]>away_4",
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

-- A flat rendering of every human's owned set and the slot that is live at the
-- first tick. In 4v4 each owned set has one member and the live slot is it,
-- which is exactly what makes switching inert there.
---@param session CoordinatorDriver
---@return string
function conformance.owned(session)
    local freeze = assert(session:host().state.freeze, "the session never froze")
    local parts = {}
    for _, node in ipairs(session.nodes) do
        local owned = freeze.owned[node.peer_id]
        if owned then
            parts[#parts + 1] = ("%s=[%s]>%s"):format(
                node.peer_id,
                table.concat(owned, ","),
                tostring(freeze.live[node.peer_id])
            )
        end
    end
    return table.concat(parts, "|")
end

---@param guest_count integer
---@param mode SessionMatchMode?
---@return CoordinatorDriver
function conformance.session(guest_count, mode)
    local session = driver.new({ guest_count = guest_count, mode = mode })
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

    -- The same eight canonical slots, shared three different ways. Pinning all
    -- three keeps 4v4 a regression target while 1v1 and 2v2 gain evidence of
    -- their own.
    local full_owned = conformance.owned(full)
    assert(
        full_owned == golden.full_owned,
        ("4v4 owned-set golden changed: expected %s, got %s"):format(golden.full_owned, full_owned)
    )
    for _, case in ipairs({
        { "1v1", 1, golden.duo_transcript_id, golden.duo_sources, golden.duo_owned },
        { "2v2", 3, golden.pair_transcript_id, golden.pair_sources, golden.pair_owned },
    }) do
        local mode, guest_count = case[1], case[2]
        local session = conformance.session(guest_count, mode)
        for _, check in ipairs({
            { "transcript", session:transcript_id(), case[3] },
            { "ownership", conformance.sources(session), case[4] },
            { "owned-set", conformance.owned(session), case[5] },
        }) do
            assert(
                check[2] == check[3],
                ("%s %s golden changed: expected %s, got %s"):format(
                    mode,
                    check[1],
                    check[3],
                    check[2]
                )
            )
        end
    end

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
