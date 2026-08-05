-- Contract tests for the flat render frame block (#332).
--
-- These are tier-1 logic tests: no display, no wasm, no browser. What they pin
-- is the WIRE, and they are the fast half of a two-part gate. The other half
-- (`wasm/sim-host/verify_payload.js` and `verify_payload_browser.py`) runs the
-- same protocol through real linear memory in node, Chrome and Firefox with the
-- JavaScript reader, which is the only place a Lua/JS disagreement can show up.
--
-- So this file deliberately checks things that would ALSO break the JavaScript
-- reader if they regressed -- header field counts, section offsets, enum codes,
-- version stamps -- rather than only things Lua can see. A layout change made
-- without touching `wasm/sim-host/frame_payload.js` fails here first, in four
-- minutes, instead of in a browser run nobody launched.

local t = require("spec.support.runner")
local frame_buffer = require("render.frame_buffer")
local render_frame = require("render.frame")
local match = require("sim.match")
local match_snapshot = require("sim.match_snapshot")
local teams = require("data.teams")

---@return MatchState
local function fixture(seed)
    return match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = 960, h = 540 },
        seed = seed or 17,
    })
end

-- Put one event of `kind` into a built frame's event channel. Kickoff produces
-- none, and the sparse/tri-state encoding rules only have anything to say once
-- there is one.
---@param frame RenderFrame
---@param kind MatchEventKind
local function with_event(frame, kind)
    local events = frame.events
    events.count = 1
    events.kind[1] = kind
    events.x[1] = 0
    events.y[1] = 0
    events.jumping[1] = 0
    events.on_target[1] = 0
end

t.describe("render frame buffer", function()
    t.it("round-trips a whole frame through one flat array of doubles", function()
        local state = fixture()
        local frame = render_frame.build(state)
        local words = frame_buffer.encode(frame)

        for index = 1, #words do
            t.eq(type(words[index]), "number", ("word %d must be a double"):format(index))
        end

        local decoded = frame_buffer.decode(words)
        t.eq(decoded.render_frame_version, render_frame.VERSION)
        t.eq(decoded.layout_version, frame_buffer.LAYOUT_VERSION)
        t.near(decoded.scalars.ball_x, frame.ball.x)
        t.near(decoded.scalars.ball_y, frame.ball.y)
        t.near(decoded.scalars.field_w, frame.field.w)
        t.eq(decoded.scalars.hud_home_score, frame.hud.home_score)
        t.eq(#decoded.players.x, frame.players.count)
        for index = 1, frame.players.count do
            t.near(decoded.players.x[index], frame.players.x[index])
            t.near(decoded.players.y[index], frame.players.y[index])
            t.eq(frame_buffer.pose_id(decoded, index), frame.players.pose_id[index])
        end
    end)

    t.it("stamps both versions and rejects a frame carrying another one", function()
        local frame = render_frame.build(fixture())
        local words = frame_buffer.encode(frame)
        t.eq(words[2], frame_buffer.LAYOUT_VERSION)
        t.eq(words[3], render_frame.VERSION)

        -- The read-side assertion `render/frame.lua` deferred to its first
        -- cross-boundary consumer. A payload from a future producer must be
        -- refused, not read positionally and misinterpreted.
        frame.version = render_frame.VERSION + 1
        t.is_true(
            not pcall(frame_buffer.encode, frame),
            "encoding a frame stamped with another protocol version must fail"
        )
        frame.version = render_frame.VERSION
        frame.roster.version = render_frame.VERSION + 1
        t.is_true(
            not pcall(frame_buffer.encode, frame),
            "encoding against a roster from another protocol version must fail"
        )
    end)

    t.it("detects a corrupted header rather than misreading the block", function()
        local frame = render_frame.build(fixture())
        local words = frame_buffer.encode(frame)
        -- Every header word the JavaScript reader also checks. If one of these
        -- stops being checked here, the two readers have drifted apart.
        for _, word in ipairs({ 1, 2, 3, 5, 6, 8, 10 }) do
            local original = words[word]
            words[word] = original + 1
            t.is_true(
                not pcall(frame_buffer.decode, words),
                ("a corrupted header word %d was read as valid"):format(word)
            )
            words[word] = original
        end
        t.is_true(pcall(frame_buffer.decode, words), "the intact block must still decode")
    end)

    t.it("agrees with the header it writes about its own geometry", function()
        local frame = render_frame.build(fixture())
        local words = frame_buffer.encode(frame)

        t.eq(words[1], frame_buffer.MAGIC)
        t.eq(words[4], #words, "total_words must size the whole block")
        t.eq(words[5], frame_buffer.HEADER_WORDS)
        t.eq(words[6], #frame_buffer.SCALAR_FIELDS)
        t.eq(words[7], frame.players.count)
        t.eq(words[8], #frame_buffer.PLAYER_FIELDS)
        t.eq(words[9], frame.events.count)
        t.eq(words[10], #frame_buffer.EVENT_FIELDS)
        t.eq(
            words[4],
            frame_buffer.frame_words(frame.players.count, frame.events.count),
            "a reader sizing its view from the header must get the whole block"
        )
    end)

    t.it("lays per-entity data out field-major, so one field is one run", function()
        local state = fixture()
        local frame = render_frame.build(state)
        local words = frame_buffer.encode(frame)
        local count = frame.players.count
        local at = frame_buffer.HEADER_WORDS + #frame_buffer.SCALAR_FIELDS + 1

        -- Structure of arrays is the whole reason this encoding is affordable:
        -- a reader that wants positions takes two contiguous runs and never
        -- touches the other nineteen fields.
        for field, name in ipairs(frame_buffer.PLAYER_FIELDS) do
            for index = 1, count do
                local word = words[at + (field - 1) * count + index - 1]
                local source = frame.players[name][index]
                if type(source) == "number" then
                    t.near(word, source, 1e-9, ("%s[%d]"):format(name, index))
                end
            end
        end
    end)

    t.it("maps an absent enum to zero and a present one to its code", function()
        local state = fixture()
        local frame = render_frame.build(state)

        -- Nothing is airborne at kickoff, so every aerial style is absent.
        for index = 1, frame.players.count do
            frame.players.aerial_style[index] = nil
            frame.players.aerial_outcome[index] = nil
        end
        frame.players.aerial_style[1] = "bicycle"
        frame.players.aerial_outcome[1] = "heavy"

        local decoded = frame_buffer.decode(frame_buffer.encode(frame))
        t.eq(decoded.players.aerial_style[1], frame_buffer.AERIAL_STYLE.bicycle)
        t.eq(decoded.players.aerial_outcome[1], frame_buffer.AERIAL_OUTCOME.heavy)
        for index = 2, frame.players.count do
            t.eq(decoded.players.aerial_style[index], 0, "an absent enum must encode as zero")
            t.eq(decoded.players.aerial_outcome[index], 0, "an absent enum must encode as zero")
        end
    end)

    t.it("keeps absent, false and true distinguishable for a sparse number", function()
        local state = fixture()
        local frame = render_frame.build(state)
        with_event(frame, "header")
        frame.events.difficulty[1] = nil

        local decoded = frame_buffer.decode(frame_buffer.encode(frame))
        t.eq(decoded.events.has_difficulty[1], 0, "an absent sparse number must say so")
        t.eq(decoded.events.difficulty[1], 0)

        -- A difficulty of exactly zero is a real, reported value, and it must
        -- not read back as "not reported". This is why the presence flag
        -- exists at all: unlike an enum, a number has no spare slot.
        frame.events.difficulty[1] = 0
        local present = frame_buffer.decode(frame_buffer.encode(frame))
        t.eq(present.events.has_difficulty[1], 1, "a reported zero must not read as absent")
        t.eq(present.events.difficulty[1], 0)
    end)

    t.it("carries the tri-state booleans through unflattened", function()
        local frame = render_frame.build(fixture())
        with_event(frame, "shot")
        for _, value in ipairs({ 0, 1, 2 }) do
            frame.events.jumping[1] = value
            frame.events.on_target[1] = value
            local decoded = frame_buffer.decode(frame_buffer.encode(frame))
            t.eq(decoded.events.jumping[1], value, "a tri-state must survive the crossing")
            t.eq(decoded.events.on_target[1], value)
        end
    end)

    t.it("uses slot zero for none, so a one-based index needs no flag", function()
        local state = fixture()
        local frame = render_frame.build(state)
        frame.possession.owner = nil
        frame.possession.owner_team = nil
        frame.control.pass_target = nil
        frame.control.charge_kind = nil

        local loose = frame_buffer.decode(frame_buffer.encode(frame))
        t.eq(loose.scalars.possession_owner, 0, "a loose ball has no owner slot")
        t.eq(loose.scalars.possession_owner_team, 0)
        t.eq(loose.scalars.control_pass_target, 0)
        t.eq(loose.scalars.control_charge_kind, 0)

        frame.possession.owner = 3
        frame.possession.owner_team = "away"
        frame.control.pass_target = 5
        local held = frame_buffer.decode(frame_buffer.encode(frame))
        t.eq(held.scalars.possession_owner, 3)
        t.eq(held.scalars.possession_owner_team, frame_buffer.TEAM.away)
        t.eq(held.scalars.control_pass_target, 5)
    end)

    t.it("reports whether combat telegraphs were dropped instead of hiding it", function()
        local frame = render_frame.build(fixture())
        t.eq(frame_buffer.decode(frame_buffer.encode(frame)).combat_present, false)

        -- This layout does not carry the nested combat model. A frame that HAD
        -- one must say so, so a renderer can refuse a half-frame rather than
        -- quietly draw no telegraphs.
        frame.combat = { enabled = true, players = {}, projectiles = {} }
        t.eq(
            frame_buffer.decode(frame_buffer.encode(frame)).combat_present,
            true,
            "a dropped combat model must be disclosed in the header"
        )
    end)

    t.it("crosses the roster once, with its ids and names", function()
        local state = fixture()
        local roster = render_frame.roster(state)
        local words, strings = frame_buffer.encode_roster(roster)
        local decoded = frame_buffer.decode_roster(words, strings)

        t.eq(decoded.count, roster.count)
        t.eq(words[4], #words, "total_words must size the roster block too")
        for index = 1, roster.count do
            t.eq(decoded.ids[index], roster.ids[index])
            t.eq(decoded.names[index], roster.names[index])
            t.eq(decoded.fields.team[index], frame_buffer.TEAM[roster.teams[index]])
            t.eq(decoded.fields.is_keeper[index], roster.is_keeper[index] and 1 or 0)
            t.near(decoded.fields.radius[index], roster.radius[index])
            t.near(decoded.fields.species_color_r[index], roster.species_color[index][1])
        end
    end)

    t.it("refuses a roster string it could not parse back", function()
        local roster = render_frame.roster(fixture())
        roster.names[1] = "Bad\nName"
        t.is_true(
            not pcall(frame_buffer.encode_roster, roster),
            "a newline in a roster string would make the blob unparseable"
        )
    end)

    t.it("reuses the caller's array and truncates it to this frame", function()
        local state = fixture()
        local frame = render_frame.build(state)
        local words = frame_buffer.encode(frame)
        local length = #words

        -- A stale tail from a longer previous frame is indistinguishable from
        -- live data once a reader sizes its view off the header.
        words[length + 1] = 12345
        words[length + 2] = 67890
        local again = frame_buffer.encode(frame, words)
        t.is_true(again == words, "the caller's array must be reused, not replaced")
        t.eq(#again, length, "a reused array must be truncated to this frame's length")
        t.eq(again[length + 1], nil)
    end)

    t.it("does not perturb the simulation", function()
        local state = fixture()
        local before = match_snapshot.hash(match_snapshot.capture(state))
        frame_buffer.encode(render_frame.build(state))
        frame_buffer.encode_roster(render_frame.roster(state))
        t.eq(
            match_snapshot.hash(match_snapshot.capture(state)),
            before,
            "serialising a frame must not touch the state it read"
        )
    end)

    t.it("numbers every enum from one, leaving zero for absent", function()
        local tables = {
            TEAM = frame_buffer.TEAM,
            SPECIES_SHAPE = frame_buffer.SPECIES_SHAPE,
            CHARGE_KIND = frame_buffer.CHARGE_KIND,
            POSE_SOURCE = frame_buffer.POSE_SOURCE,
            POSE_ID = frame_buffer.POSE_ID,
            AERIAL_STYLE = frame_buffer.AERIAL_STYLE,
            AERIAL_OUTCOME = frame_buffer.AERIAL_OUTCOME,
            SAVE_STYLE = frame_buffer.SAVE_STYLE,
            KEEPER_STATE = frame_buffer.KEEPER_STATE,
            SHOT_TYPE = frame_buffer.SHOT_TYPE,
            EVENT_KIND = frame_buffer.EVENT_KIND,
        }
        for name, codes in pairs(tables) do
            local seen = {}
            local highest = 0
            local size = 0
            for member, code in pairs(codes) do
                t.is_true(code >= 1, ("%s.%s must not use the absent code"):format(name, member))
                t.is_true(seen[code] == nil, ("%s has a duplicate code %d"):format(name, code))
                seen[code] = true
                highest = math.max(highest, code)
                size = size + 1
            end
            -- Dense from 1, so the JavaScript reader can decode with an array
            -- literal rather than a map -- which is how it is written.
            t.eq(highest, size, ("%s must be numbered densely from one"):format(name))
        end
    end)
end)
