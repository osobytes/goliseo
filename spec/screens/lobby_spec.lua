-- Tier-2 UI logic for the online lobby: layout, hit-testing, focus, and the
-- transitions each control produces. Nothing here opens a transport or a
-- window; the model is driven through the same pure seam the screen uses.

local hit = require("game.ui.hit")
local lobby = require("game.screens.lobby")
local lobby_model = require("game.screens.lobby_model")
local t = require("spec.support.runner")
local ui_draw = require("game.ui.draw")

local VP = { w = 960, h = 540 }

---@param layout Layout
---@param id string
---@return InputEvent
local function click_on(layout, id)
    local widget = assert(hit.find(layout, id), "missing widget " .. id)
    return {
        kind = "click",
        x = widget.rect.x + widget.rect.w / 2,
        y = widget.rect.y + widget.rect.h / 2,
    }
end

---@param state LobbyScreenState
---@param id string
---@return LobbyScreenState, table?
local function click(state, id)
    return lobby.update(state, click_on(lobby.layout(state), id))
end

---@param state LobbyScreenState
---@param command table
---@return LobbyScreenState, table?
local function dispatch(state, command)
    return lobby.update(state, { kind = "lobby", command = command })
end

---@param state LobbyScreenState
---@return LobbyView
local function view(state)
    return lobby_model.view(state.model)
end

---@param layout Layout
local function assert_within_canvas(layout)
    for _, widget in ipairs(layout) do
        t.is_true(widget.rect.x >= 0, widget.id .. " starts left of the viewport")
        t.is_true(widget.rect.y >= 0, widget.id .. " starts above the viewport")
        t.is_true(widget.rect.x + widget.rect.w <= VP.w, widget.id .. " exceeds viewport width")
        t.is_true(widget.rect.y + widget.rect.h <= VP.h, widget.id .. " exceeds viewport height")
    end
end

---@param fn fun()
---@return boolean, string?
local function with_stub_graphics(fn)
    local saved = love.graphics
    local graphics = {}
    for _, name in ipairs({
        "setColor",
        "setLineWidth",
        "rectangle",
        "polygon",
        "line",
        "circle",
        "ellipse",
        "push",
        "pop",
        "translate",
        "scale",
        "printf",
        "setFont",
    }) do
        graphics[name] = function() end
    end
    graphics.getDimensions = function()
        return 1280, 720
    end
    love.graphics = graphics
    local ok, err = pcall(fn)
    love.graphics = saved
    return ok, err
end

---@return LobbyScreenState
local function hosting()
    local state = lobby.new_state(VP)
    state = (click(state, "role_host"))
    return state
end

t.describe("online lobby screen", function()
    t.it("opens on an explicit host or guest choice", function()
        local state = lobby.new_state(VP)
        local layout = lobby.layout(state)
        t.is_true(hit.find(layout, "role_host") ~= nil)
        t.is_true(hit.find(layout, "role_guest") ~= nil)
        t.eq(hit.find(layout, "invite"), nil)
        t.eq(view(state).phase, "role")
    end)

    t.it("cycles the guest identity that answers the host's Nth invitation", function()
        local state = lobby.new_state(VP)
        t.eq(view(state).peer_id, "guest_1")
        state = (click(state, "identity"))
        t.eq(view(state).peer_id, "guest_2")
        state = (click(state, "role_guest"))
        t.eq(view(state).role, "guest")
        -- Identity is baked into the coordinator, so it stops moving.
        state = (dispatch(state, { kind = "identity" }))
        t.eq(view(state).peer_id, "guest_2")
    end)

    t.it("offers exactly the supported match modes to a host", function()
        local state = hosting()
        local layout = lobby.layout(state)
        for _, mode in ipairs({ "1v1", "2v2", "4v4" }) do
            t.is_true(hit.find(layout, "mode_" .. mode) ~= nil, mode .. " is missing")
        end
        t.eq(hit.find(layout, "mode_3v3"), nil)
    end)

    t.it("derives the required peer count from the selected mode", function()
        local state = hosting()
        t.eq(view(state).required, 8)
        state = (click(state, "mode_2v2"))
        t.eq(view(state).required, 4)
        t.eq(view(state).mode, "2v2")
        state = (click(state, "mode_1v1"))
        t.eq(view(state).required, 2)
        local peers = assert(hit.find(lobby.layout(state), "peer_count"))
        t.is_true(assert(peers.text):find("1 / 2", 1, true) ~= nil)
    end)

    t.it("shows all eight canonical slots and both protected keepers", function()
        local state = hosting()
        local layout = lobby.layout(state)
        for _, slot in ipairs({
            "home_1",
            "home_2",
            "home_3",
            "home_4",
            "away_1",
            "away_2",
            "away_3",
            "away_4",
        }) do
            t.is_true(hit.find(layout, "slot_" .. slot) ~= nil, slot .. " is not shown")
        end
        local home_keeper = assert(hit.find(layout, "keeper_home"))
        t.is_true(assert(home_keeper.text):find("PROTECTED AI", 1, true) ~= nil)
        t.is_true(hit.find(layout, "keeper_away") ~= nil)
    end)

    t.it("never renders a pasted or exported signaling blob", function()
        local state = hosting()
        state = (dispatch(state, { kind = "paste", text = string.rep("SDPSECRET", 40) }))
        for _, widget in ipairs(lobby.layout(state)) do
            if widget.text then
                t.is_true(
                    widget.text:find("SDPSECRET", 1, true) == nil,
                    widget.id .. " rendered signaling text"
                )
            end
        end
    end)

    t.it("keeps only a digest of a signaling blob it has handed over", function()
        local state = hosting()
        state = (dispatch(state, { kind = "signal", peer_id = "guest_1", signal = "offer:blob" }))
        t.is_true(view(state).has_outgoing)
        local next_state, _ = dispatch(state, { kind = "copy" })
        local clipboard
        for _, effect in ipairs(next_state.effects) do
            if effect.kind == "clipboard" then
                clipboard = effect.text
            end
        end
        t.eq(clipboard, "offer:blob")
        t.is_true(not view(next_state).has_outgoing)
        t.eq(next_state.model.outgoing, nil)
        local record = assert(view(next_state).exported)
        t.eq(record.direction, "offer")
        t.eq(record.bytes, #"offer:blob")
    end)

    t.it("routes the paste control through a clipboard request", function()
        local state = hosting()
        local next_state = (dispatch(state, { kind = "paste_request" }))
        t.eq(#next_state.effects, 1)
        t.eq(next_state.effects[1].kind, "paste_request")
    end)

    t.it("emits transport effects for an invitation", function()
        local state = hosting()
        state = (click(state, "mode_1v1"))
        local next_state = (click(state, "invite"))
        local kinds = {}
        for _, effect in ipairs(next_state.effects) do
            kinds[#kinds + 1] = effect.kind
        end
        t.eq(table.concat(kinds, ","), "open_peer,request_offer")
    end)

    t.it("disables controls that the current phase forbids", function()
        local state = hosting()
        local layout = lobby.layout(state)
        t.is_true(assert(hit.find(layout, "copy_signal")).data.disabled)
        t.is_true(assert(hit.find(layout, "ready")).data.disabled)
        t.is_true(assert(hit.find(layout, "start")).data.disabled)
        -- A disabled control is not activated by a click on it.
        local next_state = (click(state, "ready"))
        t.eq(view(next_state).error, nil)
    end)

    t.it("leaves on back and on the leave control", function()
        local state = hosting()
        local _, action = lobby.update(state, { kind = "action", action = "back" })
        t.eq(assert(action).go, "main_menu")
        local _, clicked = click(state, "leave")
        t.eq(assert(clicked).go, "main_menu")
    end)

    t.it("moves focus with directional actions", function()
        local state = lobby.new_state(VP)
        t.eq(state.focus, "role_host")
        local moved = (lobby.update(state, { kind = "action", action = "down" }))
        t.is_true(moved.focus ~= state.focus)
        local focused = assert(hit.find(lobby.layout(moved), moved.focus))
        t.is_true(focused.focused)
    end)

    t.it("keeps every state inside the virtual canvas", function()
        local role_state = lobby.new_state(VP)
        local host_state = hosting()
        local guest_state = (click(lobby.new_state(VP), "role_guest"))
        local ready_state = (dispatch(hosting(), { kind = "bot_fill" }))
        ready_state = (dispatch(ready_state, { kind = "lock" }))
        for _, state in ipairs({ role_state, host_state, guest_state, ready_state }) do
            assert_within_canvas(lobby.layout(state))
        end
    end)

    t.it("renders every state without touching a real display", function()
        local states = { lobby.new_state(VP), hosting() }
        local locked = (dispatch(hosting(), { kind = "bot_fill" }))
        states[#states + 1] = (dispatch(locked, { kind = "lock" }))
        for _, state in ipairs(states) do
            local ok, err = with_stub_graphics(function()
                ui_draw.layout(lobby.layout(state), VP)
            end)
            t.is_true(ok, tostring(err))
        end
    end)

    t.it("surfaces a terminal reason in the layout", function()
        local state = hosting()
        state = (dispatch(state, { kind = "leave" }))
        local trouble = assert(hit.find(lobby.layout(state), "trouble"))
        t.is_true(assert(trouble.text):find("YOU ENDED THE SESSION", 1, true) ~= nil)
    end)

    t.it("surfaces a dropped guest's build on a lobby that is still standing", function()
        local state = hosting()
        -- The coordinator's own record, written where a real drop writes it.
        -- The session is not terminal, so this is the only line there is.
        assert(state.model.coordinator).departure = {
            peer_id = "guest_1",
            reason = "build_mismatch",
            code = "protocol_error",
            detail = "a guest aborted with manifest_mismatch",
        }
        local text = assert(assert(hit.find(lobby.layout(state), "trouble")).text)
        t.is_true(text:find("DIFFERENT BUILD", 1, true) ~= nil, text)
        t.is_true(text:find("INSTALL THE SAME BUILD ON BOTH", 1, true) ~= nil, text)
        t.is_true(text:find("MANIFEST_MISMATCH", 1, true) ~= nil, "the detail is kept")
    end)

    t.it("lets a finished session outrank a dropped guest", function()
        local state = hosting()
        assert(state.model.coordinator).departure = {
            peer_id = "guest_1",
            reason = "build_mismatch",
            code = "protocol_error",
        }
        state = (dispatch(state, { kind = "leave" }))
        local text = assert(assert(hit.find(lobby.layout(state), "trouble")).text)
        t.is_true(text:find("YOU ENDED THE SESSION", 1, true) ~= nil)
        t.is_true(text:find("DIFFERENT BUILD", 1, true) == nil)
    end)

    t.it("names the AI-driven slots inside a human's owned set", function()
        local state = hosting()
        state = (click(state, "mode_1v1"))
        state = (dispatch(state, { kind = "bot_fill" }))
        state = (dispatch(state, { kind = "lock" }))
        local layout = lobby.layout(state)
        t.is_true(assert(assert(hit.find(layout, "slot_home_1")).text):find("LIVE", 1, true) ~= nil)
        for _, slot in ipairs({ "home_2", "home_3", "home_4" }) do
            local text = assert(assert(hit.find(layout, "slot_" .. slot)).text)
            t.is_true(text:find("AI (OWNED)", 1, true) ~= nil, slot .. " must read as owned AI")
            t.is_true(text:find("HOST", 1, true) ~= nil, slot .. " must name its human owner")
        end
        t.is_true(
            assert(assert(hit.find(layout, "slot_away_1")).text):find("AI FILL", 1, true) ~= nil
        )
    end)

    t.it("offers a pair control only where there is a pair to choose", function()
        local state = hosting()
        state = (click(state, "mode_2v2"))
        state = (dispatch(state, { kind = "bot_fill" }))
        state = (dispatch(state, { kind = "lock" }))
        local layout = lobby.layout(state)
        t.eq(hit.find(layout, "prefer_home_1"), nil, "a slot already owned needs no control")
        t.is_true(hit.find(layout, "prefer_home_3") ~= nil, "the rest of the line is a choice")
        t.eq(hit.find(layout, "prefer_away_1"), nil, "the other team is never a choice")

        state = (click(state, "prefer_home_3"))
        local preference = assert(view(state).preference, "the request must reach the view")
        t.eq(table.concat(preference.slots, ","), "home_1,home_3")
        t.eq(preference.status, "granted")
        local label = assert(hit.find(lobby.layout(state), "preference"))
        t.is_true(assert(label.text):find("HOME_1 HOME_3", 1, true) ~= nil)
        local taken = assert(hit.find(lobby.layout(state), "slot_home_3"))
        t.is_true(assert(taken.text):find("HOST", 1, true) ~= nil, "the pair moved on the roster")
    end)

    t.it("offers no pair control at all in 1v1 or 4v4", function()
        for _, mode in ipairs({ "1v1", "4v4" }) do
            local state = hosting()
            state = (click(state, "mode_" .. mode))
            state = (dispatch(state, { kind = "bot_fill" }))
            state = (dispatch(state, { kind = "lock" }))
            for _, widget in ipairs(lobby.layout(state)) do
                t.is_true(
                    widget.id:match("^prefer_") == nil,
                    mode .. " must offer no pair control: " .. widget.id
                )
            end
        end
    end)
end)
