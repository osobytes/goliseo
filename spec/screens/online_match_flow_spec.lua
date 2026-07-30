-- Headless screen-stack flows for the online match.
--
-- Mounted lobby screens -- two of them usually, four for the combat-family case
-- -- complete the real manual handshake over the in-process star, reach a
-- synchronized start, and hand their freeze, their coordinator, and their link to
-- the same number of mounted online match screens. Those run a real match to full
-- time and complete the real result acknowledgement. No browser, no display, no
-- network.
--
-- The manifest is this build's content-derived one with a shortened duration, so
-- the whole thing is the production path with one number turned down.

local t = require("spec.support.runner")
local lobby_model = require("game.screens.lobby_model")
local live_slot = require("game.online.live_slot")
local match_driver = require("game.online.match_driver")
local match_manifest = require("game.online.match_manifest")
local match_session = require("game.online.match_session")
local online_match_model = require("game.screens.online_match_model")
local OnlineLobby = require("game.screens.online_lobby")
local OnlineMatch = require("game.screens.online_match")
local match_hud = require("game.match_hud")
local match_hud_render = require("game.render.match_hud")
local pitch = require("game.render.pitch")
local combat_presentation = require("game.presentation.combat")
local protocol = require("game.online.protocol")
local action_families = require("data.action_families")
local loadouts = require("data.loadouts")
local input_frame = require("sim.input_frame")
local transport = require("game.transport")

local DURATION_TICKS = 90

---@param fn fun(down: table<string, boolean>)
local function with_keyboard(fn)
    local saved = love.keyboard
    local down = {}
    love.keyboard = {
        isDown = function(...)
            for _, key in ipairs({ ... }) do
                if down[key] then
                    return true
                end
            end
            return false
        end,
    }
    local ok, err = pcall(fn, down)
    love.keyboard = saved
    assert(ok, err)
end

---@class OnlineFlowHarness
---@field lobbies OnlineLobby[]
---@field stars FakeStarTransport[]
---@field matches OnlineMatchScreen[]
---@field actions table[][]

-- A full lobby that has already reached the synchronized start boundary.
--
-- `guest_count` mounts that many guest screens, each completing its own manual
-- offer/answer exchange through the one shared clipboard, exactly as a human
-- would: the host's `invite` opens one pending link at a time, so the guests are
-- admitted in sequence and the clipboard never holds two blobs at once.
-- `duration_ticks` shortens the frozen match; a test that needs live combat has
-- to outlast `sim.match`'s 2.5 s kickoff hold, which the default 90 ticks does
-- not.
---@param mode SessionMatchMode
---@param guest_count integer? -- Defaults to one guest.
---@param duration_ticks integer? -- Defaults to `DURATION_TICKS`.
---@return OnlineFlowHarness
local function started_lobby(mode, guest_count, duration_ticks)
    local rendezvous = transport.fake_star_rendezvous()
    ---@type table<string, string?>
    local clipboards = {}
    ---@type FakeStarTransport[]
    local stars = {}

    ---@param name string
    ---@return OnlineLobby
    local function mount(name)
        return OnlineLobby.new({ w = 960, h = 540 }, nil, {
            star_factory = function(role, peer_id)
                local star = transport.fake_star({
                    role = role,
                    peer_id = role == "guest" and peer_id or nil,
                    rendezvous = rendezvous,
                })
                assert(star:initialize())
                stars[#stars + 1] = star
                return star
            end,
            clipboard = {
                read = function()
                    return clipboards.shared
                end,
                write = function(text)
                    clipboards.shared = text
                end,
            },
            model_options = {
                peer_id = name,
                template = function(requested)
                    local manifest = match_manifest.template(requested)
                    manifest.duration_ticks = duration_ticks or DURATION_TICKS
                    return manifest
                end,
            },
        })
    end

    local host = mount("host")
    ---@type OnlineLobby[]
    local peers = { host }
    for index = 1, guest_count or 1 do
        peers[#peers + 1] = mount("guest_" .. tostring(index))
    end
    local function pump(rounds)
        for _ = 1, rounds or 4 do
            for _, star in ipairs(stars) do
                star:pump()
            end
            for _, peer in ipairs(peers) do
                peer:update(0)
            end
        end
    end

    host:dispatch({ kind = "role", role = "host" })
    host:dispatch({ kind = "mode", mode = mode })
    -- Fewer mounted screens than the mode seats: the remaining seats are
    -- declared bot fills. That is the same short-lobby path the driver already
    -- supports, and it keeps the flow tests honest about modes without mounting
    -- eight screens.
    if #peers < assert(protocol.match_mode(mode)).humans then
        host:dispatch({ kind = "bot_fill" })
    end
    for index = 2, #peers do
        local guest = peers[index]
        guest:dispatch({ kind = "role", role = "guest" })
        host:dispatch({ kind = "invite" })
        pump(2)
        host:dispatch({ kind = "copy" })
        guest:dispatch({ kind = "paste_request" })
        pump(2)
        guest:dispatch({ kind = "copy" })
        host:dispatch({ kind = "paste_request" })
        pump(4)
    end
    host:dispatch({ kind = "lock" })
    pump(4)
    for _, peer in ipairs(peers) do
        peer:dispatch({ kind = "ready", ready = true })
    end
    pump(2)
    host:dispatch({ kind = "start" })
    for _ = 1, lobby_model.COUNTDOWN_TICKS + 10 do
        for _, star in ipairs(stars) do
            star:pump()
        end
        for _, peer in ipairs(peers) do
            peer:update(1 / 60)
        end
    end
    for index, peer in ipairs(peers) do
        assert(
            lobby_model.view(peer.state.model).started,
            ("lobby peer %d never started"):format(index)
        )
    end
    return { lobbies = peers, stars = stars, matches = {}, actions = {} }
end

-- Mount the online match screen each lobby's start action would push. The freeze
-- is taken from the coordinator, which is exactly what the app router does.
---@param state OnlineFlowHarness
---@param mode SessionMatchMode
local function mount_matches(state, mode)
    for index, lobby in ipairs(state.lobbies) do
        local model = lobby.state.model
        local coordinator_state = assert(model.coordinator)
        local freeze = assert(coordinator_state.freeze, "the lobby never froze a session")
        local request = assert(match_session.request({
            role = coordinator_state.role,
            peer_id = coordinator_state.peer_id,
            manifest = assert(coordinator_state.manifest),
            freeze = freeze,
        }))
        assert(request.mode == mode)
        state.actions[index] = {}
        state.matches[index] = OnlineMatch.new({
            request = request,
            coordinator = coordinator_state,
            link = assert(lobby.link),
            on_action = function(action)
                state.actions[index][#state.actions[index] + 1] = action
            end,
        })
    end
end

---@param state OnlineFlowHarness
---@param frames integer
local function run(state, frames)
    for _ = 1, frames do
        for _, star in ipairs(state.stars) do
            star:pump()
        end
        for _, screen in ipairs(state.matches) do
            screen:update(1 / 60)
        end
    end
end

---@param state OnlineFlowHarness
local function teardown(state)
    for _, screen in ipairs(state.matches) do
        screen:teardown()
    end
    for _, lobby in ipairs(state.lobbies) do
        lobby:teardown()
    end
end

-- The renderer stub the family and smoke cases share: draw code really executes,
-- so a nil field or a bad projection fails here rather than on a device.
local function stub_graphics()
    local g = {}
    local noop = function() end
    for _, name in ipairs({
        "setColor",
        "setLineWidth",
        "setBlendMode",
        "rectangle",
        "polygon",
        "line",
        "circle",
        "ellipse",
        "arc",
        "push",
        "pop",
        "translate",
        "rotate",
        "print",
        "printf",
    }) do
        g[name] = noop
    end
    g.getDimensions = function()
        return 1280, 720
    end
    g.getWidth = function()
        return 1280
    end
    g.getHeight = function()
        return 720
    end
    return g
end

-- One gamepad with nothing held but the button the caller names. `b` inside a
-- match is the equipment button (`game.input.actions.from_gamepad`), and the
-- match screen *polls* it — an abstract action event alone would be overwritten
-- by the next poll, so a genuine gamepad hold has to come from here.
---@param held table<string, boolean>
---@param fn fun()
local function with_gamepad(held, fn)
    local saved = love.joystick
    local joystick = {
        getGamepadAxis = function()
            return 0
        end,
        isGamepadDown = function(_, button)
            return held[button] == true
        end,
    }
    love.joystick = {
        getJoysticks = function()
            return { joystick }
        end,
    }
    local ok, err = pcall(fn)
    love.joystick = saved
    assert(ok, err)
end

-- Which telegraph a family is *supposed* to put on the pitch. Distinct per
-- contact kind, so asserting it says more than "some telegraph appeared":
-- `game.presentation.combat.telegraph_kind` would have to be wrong about this
-- family specifically for the assertion to pass on the wrong shape.
---@type table<ActionFamilyId, CombatTelegraphKind>
local TELEGRAPH_FOR = {
    unarmed = "arc",
    light_melee = "arc",
    guard = "guard_arc",
    ranged = "line",
}

-- The fixed equipment a canonical slot carries, read out of the *frozen
-- manifest* rather than out of `data/`. That is the copy both peers agreed on,
-- and it is what makes the family expectations below content-derived: nothing
-- here chooses a loadout for the test.
---@param request OnlineMatchRequest
---@param slot InputSlotId
---@return string player_id, string loadout_id, ActionFamilyId family_id
local function slot_equipment(request, slot)
    local index = live_slot.slot_index(slot)
    local player_id = assert(request.manifest.slots[index], "canonical slot is missing").player_id
    for _, team in ipairs(request.manifest.teams) do
        for _, entry in ipairs(team.roster) do
            if entry.player_id == player_id then
                return player_id,
                    assert(entry.loadout_id, "an owned slot needs a fixed loadout"),
                    assert(entry.family_id, "an owned slot needs a fixed family")
            end
        end
    end
    error("the manifest slot names a player outside both rosters")
end

t.describe("online match screen flow", function()
    t.it("carries a 1v1 session from the lobby to an agreed result", function()
        with_keyboard(function()
            local state = started_lobby("1v1")
            mount_matches(state, "1v1")
            run(state, DURATION_TICKS + 90)

            for index, screen in ipairs(state.matches) do
                t.eq(
                    match_driver.status(screen.driver),
                    "completed",
                    ("peer %d did not reach full time"):format(index)
                )
                t.eq(screen.model.phase, "ended", ("peer %d never ended"):format(index))
                local terminal = assert(screen.model.terminal, "peer never recorded a terminal")
                t.eq(terminal.reason, "completed")
            end

            local first = assert(state.actions[1][1], "the host never routed anywhere")
            local second = assert(state.actions[2][1], "the guest never routed anywhere")
            t.eq(first.go, "online_result")
            t.eq(second.go, "online_result")
            -- Same authoritative score on every peer, taken from the coordinator's
            -- acknowledged result rather than either peer's prediction.
            t.eq(first.result.home_score, second.result.home_score)
            t.eq(first.result.away_score, second.result.away_score)
            t.eq(first.result.winner, second.result.winner)
            teardown(state)
        end)
    end)

    t.it("keeps control inside the frozen owned set and off both keepers", function()
        with_keyboard(function(down)
            local state = started_lobby("1v1")
            mount_matches(state, "1v1")
            local host = state.matches[1]
            ---@type table<string, boolean>
            local owned = {}
            for _, slot in ipairs(host.request.owned) do
                owned[slot] = true
            end
            t.eq(#host.request.owned, 4, "a 1v1 human owns the whole outfield line")

            -- Hold the switch key so the canonical switch edge is set on every
            -- sampled tick, then check where control can legally land.
            down.k = true
            run(state, 60)
            local seen = {}
            for _ = 1, 40 do
                run(state, 1)
                local live = assert(host.live[host.request.peer_id])
                t.is_true(owned[live], "control left the frozen owned set: " .. tostring(live))
                seen[live] = true
                local index = assert(
                    host.match.state.slot_players[live_slot.slot_index(live)],
                    "the live slot is unmapped"
                )
                t.eq(host.match.state.controlled, index, "the screen follows the live slot")
                t.is_true(
                    not host.match.state.players[index].is_keeper,
                    "keeper control must stay impossible online"
                )
            end
            -- The switch rule is the shipped one, so it only moves control when
            -- the live slot is off the ball; what is pinned here is that it never
            -- escapes the owned set, not that it fires on a particular tick.
            local count = 0
            for _ in pairs(seen) do
                count = count + 1
            end
            t.is_true(count >= 1)
            teardown(state)
        end)
    end)

    t.it("makes switching inert in 4v4 without branching on the mode", function()
        with_keyboard(function(down)
            local state = started_lobby("4v4")
            mount_matches(state, "4v4")
            local host = state.matches[1]
            t.eq(#host.request.owned, 1, "a 4v4 human owns exactly one slot")
            down.k = true
            run(state, 60)
            t.eq(
                host.live[host.request.peer_id],
                host.request.owned[1],
                "a singleton owned set makes every switch branch return the live slot"
            )
            teardown(state)
        end)
    end)

    t.it("keeps simulating through focus loss, a lost controller, and a pause request", function()
        with_keyboard(function()
            local state = started_lobby("2v2")
            mount_matches(state, "2v2")
            local host = state.matches[1]
            run(state, 20)
            local before = match_driver.diagnostics(host.driver).present_input_tick

            host:focus_lost()
            host:controller_lost()
            host:event({ kind = "action", action = "pause" })
            t.is_true(host.model.abort_prompt, "a pause request must explain itself first")
            run(state, 20)

            local after = match_driver.diagnostics(host.driver).present_input_tick
            t.is_true(after > before, "an online peer must keep simulating through focus loss")
            t.eq(match_driver.status(host.driver), "active")
            -- Any other input dismisses the prompt rather than aborting.
            host:event({ kind = "action", action = "confirm" })
            t.is_true(not host.model.abort_prompt)
            teardown(state)
        end)
    end)

    t.it("aborts deliberately and ends the session for every peer", function()
        with_keyboard(function()
            local state = started_lobby("2v2")
            mount_matches(state, "2v2")
            local host = state.matches[1]
            run(state, 20)
            host:event({ kind = "action", action = "pause" })
            host:event({ kind = "action", action = "pause" })
            run(state, 20)

            t.eq(host.model.phase, "ended")
            t.eq(assert(host.model.terminal).reason, "local_abort")
            local routed = assert(state.actions[1][1], "an abort must route somewhere")
            t.eq(routed.go, "online_ended")
            local guest = state.matches[2]
            t.eq(assert(guest.model.terminal, "the guest never saw the abort").reason, "peer_abort")
            teardown(state)
        end)
    end)

    t.it("shows the controlled player, its loadout, and the network state", function()
        with_keyboard(function()
            local state = started_lobby("2v2")
            mount_matches(state, "2v2")
            run(state, 30)
            local lines = state.matches[1]:overlay_lines()
            local text = table.concat(lines, "\n")
            t.is_true(text:find("control ") ~= nil, "the overlay names the controlled slot")
            t.is_true(text:find("owned ") ~= nil, "the overlay names the frozen owned set")
            t.is_true(text:find("family ") ~= nil, "the overlay names the fixed loadout family")
            t.is_true(text:find("net tick ") ~= nil, "the overlay reports the network state")
            t.eq(online_match_model.ended(state.matches[1].model), false)
            teardown(state)
        end)
    end)
end)

-- All four accepted families, driven and read through the online screen.
--
-- The seating is content, not a fixture. This build's canonical home line
-- carries exactly one accepted family per slot -- `home_1` guard, `home_2` light
-- melee, `home_3` ranged, `home_4` unarmed -- and a 4v4 seats one human per
-- slot. So four mounted online screens cover all four families without a single
-- loadout being chosen for the test, each peer's one owned slot is permanently
-- live (a singleton owned set makes switching inert), and the away line is bot
-- fill.
--
-- The run has three windows, and the first is what makes the other two mean
-- something:
--
--   quiet    -- nobody presses anything. `sim.match`'s 2.5 s kickoff hold
--               expires inside it, and from the moment it does every controlled
--               player reads `ready` -- equipped, off cooldown, in no forced
--               state, committed to no soccer action. Every human slot commits
--               zero times across the window anyway.
--   keyboard -- `j` is toggled on a fixed period. `game.screens.match` polls it.
--   gamepad  -- the gamepad's `b` is toggled instead, with `j` up.
--
-- A live slot is the one slot its peer authors for itself:
-- `match_driver.materialize_authored` hands the human sample straight to the
-- control slot and never asks `gameplay_ai/combat/v1` for that row. So a
-- confirmed `commit` carrying the live slot's player index can only have come
-- from local input. The quiet window shows that from the outside rather than by
-- reading the driver, and the readiness check is what makes the zero mean
-- something -- but only for the gates it actually reads, so it is worth being
-- exact about which those are.
--
-- `sim.combat`'s `request_rejection` refuses a press for seven reasons. The
-- quiet window covers five of them directly: `readiness == "ready"`
-- (`game/presentation/combat.lua`) is false unless the loadout exists,
-- `forced_ticks` is zero, the phase is `ready` -- so nothing is already
-- committed -- and `cooldown_ticks` is zero; and `kickoff_hold <= 0` is read
-- separately off the match state.
--
-- The remaining two, `soccer_commitment` and `aerial_state_or_recovery`, are
-- **not read**. They are unreachable instead, and by a property of the input
-- stub rather than of the readiness reading: `with_keyboard` and `with_gamepad`
-- below wire `j` and the pad's `b` and nothing else, so shoot, pass, dash,
-- jockey, dodge and the aerial actions can never be pressed and no
-- ground-commitment or aerial timer can arm on a live slot. A future edit that
-- teaches the stub another key has to re-check that, which is why it is written
-- down rather than left as an unexamined "every gate is open".
--
-- It deliberately does *not* lean on the AI-driven away line committing during
-- the window. It does not: from a real kickoff, over 700 quiet frames, the bot
-- fills produced no commit at all. `gameplay_ai/combat/v1` commits readily from
-- the rigged poses in `spec/support/online_combat_phases.lua` -- two lines 24 to
-- 36 px apart -- and rarely from open play, where a purpose target inside reach
-- and inside the front arc is a much scarcer event. That is a calibration
-- observation for #149, not something for this spec to work around.
--
-- Both routes are toggled rather than held because the four families activate
-- differently -- `press` for unarmed and light melee, `held` for guard,
-- `held_release` for ranged -- and only a press/hold/release cycle exercises all
-- three.
local FAMILY_QUIET_FRAMES = 210
local FAMILY_ROUTE_FRAMES = 200
local FAMILY_PRESS_PERIOD = 40
local FAMILY_DURATION_TICKS = FAMILY_QUIET_FRAMES + FAMILY_ROUTE_FRAMES * 2 + 180

t.describe("online combat families", function()
    t.it("drives and shows every accepted family from local keyboard and gamepad", function()
        ---@type table<string, boolean>
        local keys = {}
        ---@type table<string, boolean>
        local buttons = {}
        local saved_keyboard = love.keyboard
        love.keyboard = {
            isDown = function(...)
                for _, key in ipairs({ ... }) do
                    if keys[key] then
                        return true
                    end
                end
                return false
            end,
        }
        local ok, err = pcall(function()
            with_gamepad(buttons, function()
                local state = started_lobby("4v4", 3, FAMILY_DURATION_TICKS)
                mount_matches(state, "4v4")
                t.eq(#state.matches, 4, "a four-human 4v4 seats one peer per home slot")

                ---@class FamilyWitness
                ---@field slot InputSlotId
                ---@field family_id ActionFamilyId
                ---@field loadout_id string
                ---@field player_index integer
                ---@field commits table<string, integer> -- Window name -> own confirmed commits.
                ---@field quiet_open integer -- Quiet frames after the kickoff hold expired.
                ---@field quiet_ready integer -- Of those, frames the slot read `ready` on.
                ---@field telegraphs table<string, boolean>
                ---@field phases table<string, boolean>
                ---@field readiness table<string, boolean>
                ---@field projectiles integer -- Own projectiles seen in the presentation model.
                ---@field legible table? -- The first frame the family's telegraph was up.

                ---@type FamilyWitness[]
                local witnesses = {}
                ---@type table<ActionFamilyId, boolean>
                local seated = {}
                for index, screen in ipairs(state.matches) do
                    local request = screen.request
                    t.eq(#request.owned, 1, "a 4v4 human owns exactly one slot")
                    local slot = request.owned[1]
                    local player_id, loadout_id, family_id = slot_equipment(request, slot)
                    t.eq(
                        loadouts[loadout_id].family_id,
                        family_id,
                        "the manifest and this build's loadouts disagree about a family"
                    )
                    t.is_true(not seated[family_id], "two peers were seated on the same family")
                    seated[family_id] = true
                    local player_index =
                        assert(screen.match.state.slot_players[live_slot.slot_index(slot)])
                    t.eq(screen.match.state.players[player_index].id, player_id)
                    witnesses[index] = {
                        slot = slot,
                        family_id = family_id,
                        loadout_id = loadout_id,
                        player_index = player_index,
                        commits = { quiet = 0, keyboard = 0, gamepad = 0 },
                        quiet_open = 0,
                        quiet_ready = 0,
                        telegraphs = {},
                        phases = {},
                        readiness = {},
                        projectiles = 0,
                        legible = nil,
                    }
                end
                -- The criterion names four families; the seating covers exactly
                -- those four, so nothing below can quietly test three of them.
                for family_id in pairs(TELEGRAPH_FOR) do
                    t.is_true(
                        seated[family_id] == true,
                        ("no online peer was seated on %s"):format(family_id)
                    )
                end

                ---@param window string
                ---@param frames integer
                ---@param route string? -- "keyboard", "gamepad", or nil for silence.
                local function play(window, frames, route)
                    for frame = 1, frames do
                        local pressing = route ~= nil
                            and (frame - 1) % FAMILY_PRESS_PERIOD < FAMILY_PRESS_PERIOD / 2
                        keys.j = route == "keyboard" and pressing or false
                        buttons.b = route == "gamepad" and pressing or false
                        run(state, 1)
                        for index, screen in ipairs(state.matches) do
                            local witness = witnesses[index]
                            local match = screen.match
                            t.eq(
                                match.state.controlled,
                                witness.player_index,
                                "the screen must follow its own frozen owned slot"
                            )
                            for _, step in ipairs(match._rollback_confirmed_steps) do
                                for _, wrapped in ipairs(step.combat_events or {}) do
                                    local event = wrapped.payload
                                    if event.kind == "commit" then
                                        if event.source_index == witness.player_index then
                                            t.eq(
                                                event.family_id,
                                                witness.family_id,
                                                "a live slot committed outside its fixed family"
                                            )
                                            witness.commits[window] = witness.commits[window] + 1
                                        end
                                    end
                                end
                            end
                            local combat =
                                combat_presentation.model(match.state, match._combat_state)
                            local controlled = assert(combat.players[witness.player_index])
                            t.eq(controlled.family_id, witness.family_id)
                            t.eq(
                                controlled.equipment_presentation_id,
                                loadouts[witness.loadout_id].equipment_presentation_id,
                                "the presentation names the wrong equipment for this slot"
                            )
                            witness.readiness[controlled.readiness] = true
                            -- The control measurement: once the kickoff hold is
                            -- spent, was this slot actually able to commit while
                            -- it was being asked for nothing?
                            if window == "quiet" and match.state.kickoff_hold <= 0 then
                                witness.quiet_open = witness.quiet_open + 1
                                if controlled.readiness == "ready" then
                                    witness.quiet_ready = witness.quiet_ready + 1
                                end
                            end
                            witness.phases[controlled.phase] = true
                            if controlled.telegraph_kind then
                                witness.telegraphs[controlled.telegraph_kind] = true
                            end
                            for _, projectile in ipairs(combat.projectiles) do
                                if projectile.source_index == witness.player_index then
                                    witness.projectiles = witness.projectiles + 1
                                end
                            end
                            -- The first frame this family's own telegraph is on
                            -- the pitch: everything the criterion calls
                            -- "readable alongside" is captured from that one
                            -- frame, so the soccer, network, and selection
                            -- feedback asserted below is provably simultaneous
                            -- with the combat telegraph rather than merely
                            -- present at some other moment.
                            if
                                witness.legible == nil
                                and controlled.telegraph_kind
                                    == TELEGRAPH_FOR[witness.family_id]
                            then
                                witness.legible = {
                                    combat = combat,
                                    controlled = controlled,
                                    hud = match_hud.model(match.state, {
                                        home_name = match.home_name,
                                        away_name = match.away_name,
                                        arena_name = match.arena.name,
                                        arena_location = match.arena.location,
                                        tactic_name = "Balanced",
                                        formation_name = match.home_name,
                                        combat_enabled = combat.enabled,
                                        combat = controlled,
                                    }),
                                    overlay = table.concat(screen:overlay_lines(), "\n"),
                                    -- `Match:update` *replaces* `state` from the
                                    -- restored snapshot rather than mutating it,
                                    -- so keeping the reference keeps that frame.
                                    state = match.state,
                                    match = match,
                                }
                            end
                        end
                    end
                end

                play("quiet", FAMILY_QUIET_FRAMES, nil)
                play("keyboard", FAMILY_ROUTE_FRAMES, "keyboard")
                play("gamepad", FAMILY_ROUTE_FRAMES, "gamepad")

                for index, witness in ipairs(witnesses) do
                    local family = witness.family_id
                    local label = ("%s on %s"):format(family, witness.slot)
                    -- Usable: local input, and only local input, drove it.
                    t.eq(
                        witness.commits.quiet,
                        0,
                        ("%s committed with no local input at all"):format(label)
                    )
                    -- ...and it could have. Every quiet frame past the kickoff
                    -- hold read `ready`, so the zero above is the absence of a
                    -- press and not the absence of an opportunity.
                    t.is_true(
                        witness.quiet_open >= 30,
                        ("%s: the quiet window never outlasted the kickoff hold"):format(label)
                    )
                    t.eq(
                        witness.quiet_ready,
                        witness.quiet_open,
                        ("%s was not ready to commit throughout the quiet window, so "):format(
                            label
                        )
                            .. "committing zero times there proves nothing"
                    )
                    t.is_true(
                        witness.commits.keyboard > 0,
                        ("%s never committed from the keyboard"):format(label)
                    )
                    t.is_true(
                        witness.commits.gamepad > 0,
                        ("%s never committed from the gamepad"):format(label)
                    )
                    -- Readable: this family's own telegraph, not merely some
                    -- telegraph, and its own committed phase.
                    t.is_true(
                        witness.telegraphs[TELEGRAPH_FOR[family]] == true,
                        ("%s never showed its %s telegraph"):format(label, TELEGRAPH_FOR[family])
                    )
                    t.is_true(
                        witness.readiness.committed == true,
                        ("%s never read as committed"):format(label)
                    )
                    if family == "ranged" then
                        t.is_true(
                            witness.projectiles > 0,
                            "ranged never put a projectile of its own in the presentation model"
                        )
                    end
                    if family == "guard" then
                        t.is_true(
                            witness.phases.guard == true,
                            "guard never reached its held guard phase"
                        )
                    end

                    -- Readable *alongside* the rest, on one frame.
                    local legible = assert(witness.legible)
                    local hud = legible.hud
                    t.eq(
                        hud.equipment_label,
                        string.upper(assert(action_families[family].name)),
                        ("the HUD does not name %s"):format(label)
                    )
                    t.is_true(hud.equipment_state ~= nil, "the HUD hides the equipment state")
                    -- Soccer feedback the combat readout has to sit beside, not
                    -- replace: the scorebug, the clock, and possession.
                    t.eq(type(hud.home_score), "number")
                    t.eq(type(hud.away_score), "number")
                    t.is_true(#hud.clock > 0, "the clock vanished behind the combat readout")
                    t.is_true(#hud.possession > 0, "possession vanished")
                    -- Network and selection feedback on the same frame.
                    t.is_true(
                        legible.overlay:find("net tick ", 1, true) ~= nil,
                        "the network state vanished while a telegraph was up"
                    )
                    t.is_true(legible.overlay:find("rollbacks ", 1, true) ~= nil)
                    t.is_true(
                        legible.overlay:find("control " .. witness.slot, 1, true) ~= nil,
                        "the overlay stopped naming the controlled slot"
                    )
                    t.is_true(
                        legible.overlay:find("owned " .. witness.slot, 1, true) ~= nil,
                        "the overlay stopped naming the frozen owned set"
                    )
                    t.is_true(
                        legible.overlay:find(
                            "family " .. assert(action_families[family].name),
                            1,
                            true
                        ) ~= nil,
                        ("the overlay stopped naming %s"):format(label)
                    )

                    -- And it draws. Real pitch and HUD code runs over the frame
                    -- that carried this family's telegraph, so a telegraph shape
                    -- only this family produces cannot be broken in the renderer
                    -- while the model above still looks right.
                    local match = legible.match
                    local frame_state = legible.state
                    local saved = love.graphics
                    love.graphics = stub_graphics()
                    local drew, draw_err = pcall(function()
                        local viewport = { w = 1280, h = 720 }
                        pitch.draw(frame_state, viewport, {
                            home_color = match.home_color,
                            away_color = match.away_color,
                            arena = match.arena,
                            arena_pulse = 0,
                            combat = legible.combat,
                            events = frame_state.events,
                        })
                        match_hud_render.draw(hud, viewport)
                    end)
                    love.graphics = saved
                    t.is_true(drew, ("peer %d (%s): %s"):format(index, label, tostring(draw_err)))
                end

                teardown(state)
            end)
        end)
        love.keyboard = saved_keyboard
        assert(ok, err)
    end)
end)

t.describe("online match app routing", function()
    -- The lobby has emitted `{ go = "online_match", freeze = ... }` since it
    -- landed, and nothing routed it. This pins that it now does, through the real
    -- application router rather than a hand-mounted screen.
    t.it("routes the lobby's synchronized start into the online match", function()
        with_keyboard(function()
            local App = require("game.app")
            local rendezvous = transport.fake_star_rendezvous()
            ---@type table<string, string?>
            local clipboards = {}
            ---@type FakeStarTransport[]
            local stars = {}

            ---@param name string
            ---@return App
            local function mount(name)
                local app = App.new({ actual_w = 960, actual_h = 540 })
                app:show_lobby({
                    star_factory = function(role, peer_id)
                        local star = transport.fake_star({
                            role = role,
                            peer_id = role == "guest" and peer_id or nil,
                            rendezvous = rendezvous,
                        })
                        assert(star:initialize())
                        stars[#stars + 1] = star
                        return star
                    end,
                    clipboard = {
                        read = function()
                            return clipboards.shared
                        end,
                        write = function(text)
                            clipboards.shared = text
                        end,
                    },
                    model_options = {
                        peer_id = name,
                        template = function(requested)
                            local manifest = match_manifest.template(requested)
                            manifest.duration_ticks = DURATION_TICKS
                            return manifest
                        end,
                    },
                })
                return app
            end

            local host_app = mount("host")
            local guest_app = mount("guest_1")
            ---@param app App
            ---@param command table
            local function dispatch(app, command)
                local screen = assert(app.stack:current())
                ---@cast screen OnlineLobby
                screen:dispatch(command)
            end
            local function pump(rounds, dt)
                for _ = 1, rounds do
                    for _, star in ipairs(stars) do
                        star:pump()
                    end
                    host_app:update(dt or 0)
                    guest_app:update(dt or 0)
                end
            end

            t.eq(host_app:current_route(), "lobby")
            dispatch(host_app, { kind = "role", role = "host" })
            dispatch(host_app, { kind = "mode", mode = "2v2" })
            dispatch(host_app, { kind = "bot_fill" })
            dispatch(guest_app, { kind = "role", role = "guest" })
            dispatch(host_app, { kind = "invite" })
            pump(2)
            dispatch(host_app, { kind = "copy" })
            dispatch(guest_app, { kind = "paste_request" })
            pump(2)
            dispatch(guest_app, { kind = "copy" })
            dispatch(host_app, { kind = "paste_request" })
            pump(4)
            dispatch(host_app, { kind = "lock" })
            pump(4)
            dispatch(host_app, { kind = "ready", ready = true })
            dispatch(guest_app, { kind = "ready", ready = true })
            pump(2)
            dispatch(host_app, { kind = "start" })
            pump(lobby_model.COUNTDOWN_TICKS + 10, 1 / 60)

            t.eq(host_app:current_route(), "online_match")
            t.eq(guest_app:current_route(), "online_match")
            t.eq(host_app.online_error, nil)
            local screen = assert(host_app.stack:current())
            ---@cast screen OnlineMatchScreen
            t.eq(screen.request.mode, "2v2")
            t.eq(#screen.request.owned, 2)

            -- Focus loss on the online route must not push a pause screen.
            host_app:focus(false)
            t.eq(host_app:current_route(), "online_match")
            host_app:controller_removed()
            t.eq(host_app:current_route(), "online_match")

            host_app.stack:clear()
            guest_app.stack:clear()
        end)
    end)
end)

t.describe("online match renderer smoke", function()
    -- Combat legibility per family is pinned in "online combat families" above;
    -- what is left here is the frame's *soccer* shape -- that a live online frame
    -- draws at all and that no canonical slot names a keeper.
    t.it("draws a live online frame with its combat model and HUD", function()
        with_keyboard(function()
            local state = started_lobby("2v2")
            mount_matches(state, "2v2")
            run(state, 45)
            local screen = state.matches[1]
            local match = screen.match
            local saved = love.graphics
            love.graphics = stub_graphics()
            local ok, err = pcall(function()
                local viewport = { w = 1280, h = 720 }
                local combat = combat_presentation.model(match.state, match._combat_state)
                t.is_true(combat.enabled, "an online match always renders the combat model")
                pitch.draw(match.state, viewport, {
                    home_color = match.home_color,
                    away_color = match.away_color,
                    arena = match.arena,
                    arena_pulse = 0,
                    render_pose = match._render_pose,
                    combat = combat,
                    events = match.state.events,
                })
                match_hud_render.draw(
                    match_hud.model(match.state, {
                        home_name = match.home_name,
                        away_name = match.away_name,
                        arena_name = match.arena.name,
                        arena_location = match.arena.location,
                        tactic_name = "Balanced",
                        formation_name = match.home_name,
                        combat_enabled = combat.enabled,
                        combat = combat.players[match.state.controlled],
                    }),
                    viewport
                )
            end)
            love.graphics = saved
            t.is_true(ok, tostring(err))
            for slot = 1, input_frame.SLOT_COUNT do
                local index = assert(screen.match.state.slot_players[slot])
                t.is_true(
                    not screen.match.state.players[index].is_keeper,
                    "no canonical slot may name a keeper"
                )
            end
            teardown(state)
        end)
    end)
end)
