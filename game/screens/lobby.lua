-- Pure screen for the manual-connect online lobby.
--
-- `layout`, `hit`, and `update` touch no graphics, no transport, and no clock,
-- so the whole lobby — role choice, manual offer/answer exchange, mode choice,
-- ownership, readiness, countdown, and every failure — runs headlessly.
--
-- One deviation from the simplest screens is deliberate: a transition may need
-- side effects (open a peer, send a wire, write the clipboard). Those leave as
-- data on `state.effects`, an ordered list the owning screen drains after each
-- update. The transition itself stays pure.

local focus = require("game.ui.focus")
local lobby_model = require("game.screens.lobby_model")

---@class LobbyScreenContext
---@field model LobbyModel?
---@field options LobbyModelOptions?

---@class LobbyScreenState
---@field viewport { w: number, h: number }
---@field model LobbyModel
---@field focus string
---@field effects LobbyEffect[] -- Produced by the last update; drained by the owner.

---@class LobbyScreenModule
local lobby = {}

local LEFT_X = 24
local LEFT_W = 248
local ROSTER_X = 300
local ROSTER_W = 360
local RIGHT_X = 676
local RIGHT_W = 260
local ROW_TOP = 68
local ROW_STEP = 34
local ROW_H = 30

---@param viewport { w: number, h: number }
---@param context LobbyScreenContext?
---@return LobbyScreenState
function lobby.new_state(viewport, context)
    context = context or {}
    return {
        viewport = viewport,
        model = context.model or lobby_model.new(context.options),
        focus = "role_host",
        effects = {},
    }
end

---@param slot LobbySlotView
---@return string
local function slot_text(slot)
    local owner = slot.owner or "unassigned"
    local driver
    if slot.driver == "pending" then
        driver = "pending"
    elseif slot.owner_kind == "bot" then
        driver = "AI FILL"
    elseif slot.driver == "human" then
        driver = "LIVE"
    else
        driver = "AI (OWNED)"
    end
    return ("%s  %s  ->  %s  %s"):format(
        slot.slot:upper(),
        slot.player_id:upper(),
        owner:upper(),
        driver
    )
end

---@param seat LobbySeatView
---@return string
local function seat_text(seat)
    local slots = #seat.slots > 0 and table.concat(seat.slots, " ") or "unassigned"
    return ("%d  %s%s  %s  %s"):format(
        seat.index,
        seat.peer_id:upper(),
        seat.is_local and " (YOU)" or "",
        slots:upper(),
        seat.ready and "READY" or "-"
    )
end

---@param record LobbySignalRecord?
---@param fallback string
---@return string
local function signal_text(record, fallback)
    if not record then
        return fallback
    end
    -- Only the direction, size, and digest are ever rendered. The blob itself
    -- is handed to the clipboard and dropped; it is never held for display.
    return ("%s %s  %d BYTES  #%s"):format(
        record.direction:upper(),
        record.peer_id:upper(),
        record.bytes,
        record.fingerprint
    )
end

---@param view LobbyView
---@return string
local function identity_text(view)
    local lines = {}
    for _, row in ipairs(view.identity) do
        lines[#lines + 1] = ("%s  %s"):format(row.label, row.value:upper())
    end
    return table.concat(lines, "\n")
end

---@param state LobbyScreenState
---@return Layout
function lobby.layout(state)
    local view = lobby_model.view(state.model)
    ---@type Layout
    local layout = {
        {
            id = "title",
            kind = "title",
            text = "ONLINE LOBBY",
            rect = { x = 0, y = 18, w = state.viewport.w, h = 28 },
            data = { align = "center" },
        },
    }

    ---@param widget Widget
    local function push(widget)
        layout[#layout + 1] = widget
        return widget
    end

    local y = ROW_TOP
    ---@param id string
    ---@param text string
    ---@param height number
    ---@param options table?
    local function left(id, text, height, options)
        options = options or {}
        push({
            id = id,
            kind = options.kind or "button",
            text = text,
            selected = options.selected,
            focused = state.focus == id,
            rect = { x = options.x or LEFT_X, y = y, w = options.w or LEFT_W, h = height },
            data = {
                align = "left",
                tone = options.tone,
                disabled = options.disabled,
                focusable = options.focusable,
            },
        })
        if not options.inline then
            y = y + height + 6
        end
    end

    if not view.role then
        left("role_host", "HOST A SESSION", 40)
        left("identity", "JOIN AS  " .. view.peer_id:upper(), ROW_H)
        left("role_guest", "JOIN WITH AN OFFER", 40)
        left("hint", "MANUAL SIGNALING: BLOBS ARE EXCHANGED BY HAND.", 34, {
            kind = "label",
            tone = "muted",
            focusable = false,
        })
    elseif view.role == "host" then
        local mode_w = 76
        for index, mode in ipairs(lobby_model.MODES) do
            left("mode_" .. mode, mode:upper(), ROW_H, {
                x = LEFT_X + (index - 1) * (mode_w + 10),
                w = mode_w,
                selected = view.mode == mode,
                disabled = view.mode_locked,
                inline = index < #lobby_model.MODES,
            })
        end
        left(
            "bot_fill",
            view.bot_fill and "AI FILLS EMPTY SEATS: ON" or "AI FILLS EMPTY SEATS: OFF",
            ROW_H,
            { selected = view.bot_fill, disabled = view.mode_locked }
        )
        left("invite", "INVITE A PEER", ROW_H, { disabled = not view.can_invite })
        left("lock", "LOCK CONFIGURATION", ROW_H, { disabled = not view.can_lock })
    else
        left("mode_label", "MODE  " .. (view.mode_known and view.mode:upper() or "PENDING"), 22, {
            kind = "label",
            tone = "muted",
            focusable = false,
        })
    end

    if view.role then
        local half = (LEFT_W - 10) / 2
        left("copy_signal", "COPY SIGNAL", ROW_H, {
            w = half,
            inline = true,
            disabled = not view.has_outgoing,
        })
        left("paste_signal", "PASTE SIGNAL", ROW_H, { x = LEFT_X + half + 10, w = half })
        left("signal_out", signal_text(view.exported, "NO LOCAL SIGNAL"), 20, {
            kind = "label",
            tone = "muted",
            focusable = false,
        })
        left("signal_in", signal_text(view.imported, "NO IMPORTED SIGNAL"), 20, {
            kind = "label",
            tone = "muted",
            focusable = false,
        })
        left(
            "peer_count",
            ("PEERS  %d / %s"):format(
                view.connected,
                view.mode_known and tostring(view.required) or "?"
            ),
            20,
            { kind = "label", focusable = false }
        )
        if view.countdown then
            left("countdown", ("COUNTDOWN  %d TICKS"):format(view.countdown), 20, {
                kind = "label",
                focusable = false,
            })
        end
        if view.started then
            left("started", "START BOUNDARY REACHED", 20, { kind = "label", focusable = false })
        end
    end

    -- Roster: all eight canonical outfield slots, then both protected keepers.
    -- A slot the local peer could ask the host for carries its own control. In
    -- `1v1` and `4v4` no slot ever does, because there is no pair to choose.
    for index, slot in ipairs(view.slots) do
        local row_y = ROW_TOP + (index - 1) * ROW_STEP
        push({
            id = "slot_" .. slot.slot,
            kind = "card",
            text = slot_text(slot),
            selected = slot.local_owner,
            rect = { x = ROSTER_X, y = row_y, w = ROSTER_W - 66, h = ROW_H },
            data = { align = "left", focusable = false },
        })
        if slot.can_prefer then
            push({
                id = "prefer_" .. slot.slot,
                kind = "button",
                text = "TAKE",
                focused = state.focus == "prefer_" .. slot.slot,
                rect = { x = ROSTER_X + ROSTER_W - 62, y = row_y, w = 62, h = ROW_H },
                data = { align = "center" },
            })
        end
    end
    if view.preference then
        push({
            id = "preference",
            kind = "label",
            text = ("PAIR %s  %s"):format(
                table.concat(view.preference.slots, " "):upper(),
                view.preference.text:upper()
            ),
            rect = {
                x = ROSTER_X,
                y = ROW_TOP + 8 * ROW_STEP + 2 * 22 + 6,
                w = ROSTER_W,
                h = 40,
            },
            data = { align = "left", tone = "muted", focusable = false },
        })
    end
    for index, keeper in ipairs(view.keepers) do
        push({
            id = "keeper_" .. keeper.team,
            kind = "label",
            text = ("KEEPER %s  %s  PROTECTED AI"):format(
                keeper.team:upper(),
                keeper.player_id:upper()
            ),
            rect = {
                x = ROSTER_X,
                y = ROW_TOP + 8 * ROW_STEP + (index - 1) * 22,
                w = ROSTER_W,
                h = 20,
            },
            data = { align = "left", tone = "muted", focusable = false },
        })
    end

    -- Seats: one row per human, with the ownership swap that repartitions a
    -- 2v2 pair (and moves a human between teams) before the freeze.
    for index, seat in ipairs(view.seats) do
        local row_y = ROW_TOP + (index - 1) * ROW_STEP
        push({
            id = "seat_" .. index,
            kind = "card",
            text = seat_text(seat),
            selected = seat.is_local,
            rect = { x = RIGHT_X, y = row_y, w = RIGHT_W - 82, h = ROW_H },
            data = { align = "left", focusable = false },
        })
        if view.role == "host" and index < #view.seats then
            push({
                id = "swap_" .. index,
                kind = "button",
                text = "SWAP",
                focused = state.focus == "swap_" .. index,
                rect = { x = RIGHT_X + RIGHT_W - 74, y = row_y, w = 74, h = ROW_H },
                data = { align = "center", disabled = not view.can_configure },
            })
        end
    end

    push({
        id = "identity",
        kind = "label",
        text = identity_text(view),
        rect = { x = RIGHT_X, y = ROW_TOP + 8 * ROW_STEP + 8, w = RIGHT_W, h = 116 },
        data = { align = "left", tone = "muted", focusable = false },
    })

    push({
        id = "status",
        kind = "label",
        text = view.status:upper(),
        rect = { x = LEFT_X, y = 428, w = 620, h = 20 },
        data = { align = "left", focusable = false },
    })
    local trouble = view.error or view.terminal_text
    if view.terminal and view.terminal.detail then
        trouble = (trouble or "") .. "  (" .. view.terminal.detail .. ")"
    end
    push({
        id = "trouble",
        kind = "label",
        text = trouble and trouble:upper() or "",
        rect = { x = LEFT_X, y = 452, w = 620, h = 28 },
        data = { align = "left", tone = "muted", focusable = false },
    })

    push({
        id = "leave",
        kind = "button",
        text = "LEAVE LOBBY",
        focused = state.focus == "leave",
        rect = { x = LEFT_X, y = 488, w = 200, h = 36 },
    })
    if view.role then
        push({
            id = "ready",
            kind = "button",
            text = view.ready and "NOT READY" or "READY",
            selected = view.ready,
            focused = state.focus == "ready",
            rect = { x = 380, y = 488, w = 200, h = 36 },
            data = { disabled = not view.can_ready },
        })
    end
    if view.role == "host" then
        push({
            id = "start",
            kind = "button",
            text = "START COUNTDOWN",
            focused = state.focus == "start",
            rect = { x = 736, y = 488, w = 200, h = 36 },
            data = { disabled = not view.can_start },
        })
    end
    return layout
end

---@param id string
---@param view LobbyView
---@return table?
local function command_for(id, view)
    if id == "role_host" then
        return { kind = "role", role = "host" }
    elseif id == "role_guest" then
        return { kind = "role", role = "guest" }
    elseif id == "identity" then
        return { kind = "identity" }
    elseif id == "bot_fill" then
        return { kind = "bot_fill" }
    elseif id == "invite" then
        return { kind = "invite" }
    elseif id == "copy_signal" then
        return { kind = "copy" }
    elseif id == "paste_signal" then
        return { kind = "paste_request" }
    elseif id == "lock" then
        return { kind = "lock" }
    elseif id == "ready" then
        return { kind = "ready", ready = not view.ready }
    elseif id == "start" then
        return { kind = "start" }
    elseif id == "leave" then
        return { kind = "leave" }
    end
    local mode = id:match("^mode_(.+)$")
    if mode then
        return { kind = "mode", mode = mode }
    end
    local seat = id:match("^swap_(%d+)$")
    if seat then
        return { kind = "swap", index = tonumber(seat) }
    end
    local slot = id:match("^prefer_(.+)$")
    if slot then
        return { kind = "pair", slot = slot }
    end
    return nil
end

---@param effects LobbyEffect[]
---@return table?
local function action_for(effects)
    for _, effect in ipairs(effects) do
        if effect.kind == "start_match" then
            return { go = "online_match", freeze = effect.freeze }
        end
    end
    for _, effect in ipairs(effects) do
        if effect.kind == "leave" then
            return { go = "main_menu" }
        end
    end
    return nil
end

---@param state LobbyScreenState
---@param model LobbyModel
---@param next_focus string
---@param effects LobbyEffect[]
---@return LobbyScreenState
local function advance(state, model, next_focus, effects)
    return {
        viewport = state.viewport,
        model = model,
        focus = next_focus,
        effects = effects,
    }
end

---@param state LobbyScreenState
---@param event InputEvent|{ kind: "lobby", command: table }
---@return LobbyScreenState, table?
function lobby.update(state, event)
    if event.kind == "lobby" then
        ---@cast event { kind: "lobby", command: table }
        local model, effects = lobby_model.command(state.model, event.command)
        return advance(state, model, state.focus, effects), action_for(effects)
    end
    ---@cast event InputEvent
    local layout = lobby.layout(state)
    local next_focus = focus.navigate(layout, state.focus, event) or state.focus
    if event.kind == "action" and event.action == "back" then
        local model, effects = lobby_model.command(state.model, { kind = "leave" })
        return advance(state, model, next_focus, effects), action_for(effects)
    end
    local id = focus.activated(layout, next_focus, event)
    if not id then
        return advance(state, state.model, next_focus, {}), nil
    end
    local command = command_for(id, lobby_model.view(state.model))
    if not command then
        return advance(state, state.model, id, {}), nil
    end
    local model, effects = lobby_model.command(state.model, command)
    local next_state = advance(state, model, id, effects)
    -- Focus survives a layout that no longer offers the activated control.
    next_state.focus = focus.ensure(lobby.layout(next_state), id) or id
    return next_state, action_for(effects)
end

return lobby
