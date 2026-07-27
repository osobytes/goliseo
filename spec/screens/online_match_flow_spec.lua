-- Headless screen-stack flows for the online match.
--
-- Two mounted lobby screens complete the real manual handshake over the
-- in-process star, reach a synchronized start, and hand their freeze, their
-- coordinator, and their link to two mounted online match screens. Those run a
-- real match to full time and complete the real result acknowledgement. No
-- browser, no display, no network.
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
---@param mode SessionMatchMode
---@return OnlineFlowHarness
local function started_lobby(mode)
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
                    manifest.duration_ticks = DURATION_TICKS
                    return manifest
                end,
            },
        })
    end

    local host = mount("host")
    local guest = mount("guest_1")
    local function pump(rounds)
        for _ = 1, rounds or 4 do
            for _, star in ipairs(stars) do
                star:pump()
            end
            host:update(0)
            guest:update(0)
        end
    end

    host:dispatch({ kind = "role", role = "host" })
    host:dispatch({ kind = "mode", mode = mode })
    -- Two mounted screens play every mode: 2v2 seats four humans and 4v4 seats
    -- eight, so the remaining seats are declared bot fills. That is the same
    -- short-lobby path the driver already supports, and it keeps the flow test
    -- honest about modes without mounting eight screens.
    if mode ~= "1v1" then
        host:dispatch({ kind = "bot_fill" })
    end
    guest:dispatch({ kind = "role", role = "guest" })
    host:dispatch({ kind = "invite" })
    pump(2)
    host:dispatch({ kind = "copy" })
    guest:dispatch({ kind = "paste_request" })
    pump(2)
    guest:dispatch({ kind = "copy" })
    host:dispatch({ kind = "paste_request" })
    pump(4)
    host:dispatch({ kind = "lock" })
    pump(4)
    host:dispatch({ kind = "ready", ready = true })
    guest:dispatch({ kind = "ready", ready = true })
    pump(2)
    host:dispatch({ kind = "start" })
    for _ = 1, lobby_model.COUNTDOWN_TICKS + 10 do
        for _, star in ipairs(stars) do
            star:pump()
        end
        host:update(1 / 60)
        guest:update(1 / 60)
    end
    assert(lobby_model.view(host.state.model).started, "the host never started")
    assert(lobby_model.view(guest.state.model).started, "the guest never started")
    return { lobbies = { host, guest }, stars = stars, matches = {}, actions = {} }
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
    -- The same stub the other renderer smoke uses: draw code really executes, so
    -- a nil field or a bad projection fails here rather than on a device.
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
