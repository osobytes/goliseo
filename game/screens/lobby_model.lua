-- Pure lobby model for the manual-connect online session.
--
-- It owns no truth of its own. The session coordinator decides admission, the
-- manifest, ownership, readiness, the countdown, and the terminal reason; this
-- module presents that truth, records the few genuinely local choices (role,
-- match mode, seating order, bot fill), and turns everything into data the
-- screen can lay out and the link layer can execute.
--
-- Every function here is pure: `command` returns a fresh model plus an ordered
-- effect list, and `view` derives presentation from the model alone. Transport
-- calls, clipboard access, and rendering live outside.

local coordinator = require("game.online.coordinator")
local fnv1a64 = require("core.fnv1a64")
local input_frame = require("sim.input_frame")
local protocol = require("game.online.protocol")
local protocol_fixture = require("game.online.protocol_fixture")
local transport_contract = require("game.transport.contract")

---@alias LobbyRole "host"|"guest"
---@alias LobbySignalDirection "offer"|"answer"

---@class LobbySignalRecord
---@field direction LobbySignalDirection
---@field peer_id string
---@field bytes integer
---@field fingerprint string -- Short digest; the blob itself is never retained.

---@alias LobbyEffect
---| { kind: "open_star", role: LobbyRole, peer_id: string }
---| { kind: "open_peer", peer_id: string }
---| { kind: "request_offer", peer_id: string }
---| { kind: "accept_offer", signal: string }
---| { kind: "accept_answer", peer_id: string, signal: string }
---| { kind: "send", link_id: string, wire: string }
---| { kind: "close", link_id: string, detail: string? }
---| { kind: "clipboard", text: string }
---| { kind: "paste_request" }
---| { kind: "start_match", freeze: CoordinatorFreeze }
---| { kind: "shutdown" }
---| { kind: "leave" }

---@class LobbyModelOptions
---@field peer_id string? -- Coordinator identity claimed by this peer.
---@field session_id string?
---@field seed integer?
---@field template (fun(mode: SessionMatchMode): SessionManifest)?

---@class LobbyModel
---@field role LobbyRole?
---@field peer_id string
---@field session_id string
---@field seed integer?
---@field mode SessionMatchMode
---@field bot_fill boolean
---@field seating string[] -- Host-side human seating order; index N owns block N.
---@field coordinator CoordinatorState?
---@field guests integer -- Host-side count of opened guest links.
---@field pending_link string? -- Host-side link awaiting its answer blob.
---@field outgoing string? -- Local signaling blob awaiting export; cleared on copy.
---@field exported LobbySignalRecord?
---@field imported LobbySignalRecord?
---@field status string
---@field error string?
---@field started boolean -- A synchronized start action has been observed.
---@field template fun(mode: SessionMatchMode): SessionManifest

---@class LobbyModelModule
local lobby_model = {}

lobby_model.DEFAULT_MODE = "4v4"
lobby_model.COUNTDOWN_ID = "countdown.1"
lobby_model.COUNTDOWN_TICKS = 180
lobby_model.FIRST_INPUT_TICK = 0
lobby_model.MAX_SIGNAL_BYTES = 16384
lobby_model.GUEST_LINK_PREFIX = "guest_"

---@type SessionMatchMode[]
lobby_model.MODES = { "1v1", "2v2", "4v4" }

-- Human-readable equivalents of the coordinator's terminal reasons. The wire
-- codes stay closed; these strings exist only so a tester can act on a failure
-- without reading the protocol document.
---@type table<CoordinatorTerminalReason, string>
lobby_model.TERMINAL_TEXT = {
    completed = "The session finished.",
    local_abort = "You ended the session.",
    peer_abort = "A peer ended the session.",
    guest_left = "A guest left the session.",
    host_left = "The host left the session.",
    removed = "The host disconnected you.",
    transport_lost = "The connection to a peer was lost.",
    protocol_violation = "A peer sent traffic this session cannot accept.",
    manifest_mismatch = "Match identity differs between peers.",
    invalid_assignment = "Published slot ownership was unusable.",
    start_ack_timeout = "A peer never reached the start boundary.",
    input_channel_failure = "The input channel failed.",
    late_input = "Input arrived too late to resimulate.",
    hash_mismatch = "Peers disagreed about the simulation.",
}

-- The developer lobby proposes the pinned fixture manifest. It is the only
-- complete, protocol-valid manifest in the tree today; a content-derived
-- builder replaces it when the online match flow lands, which is why the
-- source is injectable rather than hard-wired.
---@param mode SessionMatchMode
---@return SessionManifest
local function default_template(mode)
    return protocol_fixture.manifest(mode)
end

---@param text string
---@return string
local function fingerprint(text)
    return fnv1a64.hash(text):sub(1, 8)
end

---@param model LobbyModel
---@return LobbyModel
local function copy(model)
    local seating = {}
    for index, id in ipairs(model.seating) do
        seating[index] = id
    end
    return {
        role = model.role,
        peer_id = model.peer_id,
        session_id = model.session_id,
        seed = model.seed,
        mode = model.mode,
        bot_fill = model.bot_fill,
        seating = seating,
        coordinator = model.coordinator,
        guests = model.guests,
        pending_link = model.pending_link,
        outgoing = model.outgoing,
        exported = model.exported,
        imported = model.imported,
        status = model.status,
        error = model.error,
        started = model.started,
        template = model.template,
    }
end

---@param model LobbyModel
---@param mode SessionMatchMode
---@return SessionManifest
local function manifest_for(model, mode)
    local manifest = model.template(mode)
    manifest.session_id = model.session_id
    manifest.match_mode = mode
    if model.seed then
        manifest.seed = model.seed
    end
    return manifest
end

---@param model LobbyModel
---@return CoordinatorManifestExpectation
local function expectation_for(model)
    local manifest = manifest_for(model, model.mode)
    return {
        build_id = manifest.build_id,
        source_id = manifest.source_id,
        content_id = manifest.content_id,
        tuning_id = manifest.tuning_id,
        match_config_id = manifest.match_config_id,
        fixture_id = manifest.fixture_id,
        arena_id = manifest.arena_id,
        combat_rules_id = manifest.combat_rules_id,
        gameplay_ai_policy_id = manifest.gameplay_ai_policy_id,
        combat_status = manifest.combat_status,
    }
end

---@param options LobbyModelOptions?
---@return LobbyModel
function lobby_model.new(options)
    options = options or {}
    local template = options.template or default_template
    local session_id = options.session_id or template(lobby_model.DEFAULT_MODE).session_id
    return {
        role = nil,
        -- A guest's coordinator identity and its transport link identity are
        -- the same string: the host's Nth invitation opens link `guest_N`, and
        -- the guest must answer as that peer for the star to bind them.
        peer_id = options.peer_id or (lobby_model.GUEST_LINK_PREFIX .. "1"),
        session_id = session_id,
        seed = options.seed,
        mode = lobby_model.DEFAULT_MODE,
        bot_fill = false,
        seating = {},
        coordinator = nil,
        guests = 0,
        pending_link = nil,
        outgoing = nil,
        exported = nil,
        imported = nil,
        status = "Host a session or join one with a pasted offer.",
        error = nil,
        started = false,
        template = template,
    }
end

---@param model LobbyModel
---@return SessionMatchMode
local function effective_mode(model)
    local state = model.coordinator
    if state and state.manifest then
        return state.manifest.match_mode
    end
    return model.mode
end

---@param model LobbyModel
---@return integer
local function required_humans(model)
    return assert(protocol.MATCH_MODES[effective_mode(model)]).humans
end

-- Ownership can only be published once the roster is stable, so seating is
-- rebuilt from the coordinator roster while preserving any order the host has
-- already chosen. A departed peer drops out; a new one is appended.
---@param model LobbyModel
local function refresh_seating(model)
    local state = model.coordinator
    if not state or state.role ~= "host" then
        return
    end
    ---@type table<string, boolean>
    local present = {}
    for _, peer in ipairs(state.peers) do
        present[peer.peer_id] = true
    end
    ---@type string[]
    local seating = {}
    for _, id in ipairs(model.seating) do
        if present[id] then
            seating[#seating + 1] = id
            present[id] = nil
        end
    end
    for _, peer in ipairs(state.peers) do
        if present[peer.peer_id] then
            seating[#seating + 1] = peer.peer_id
            present[peer.peer_id] = nil
        end
    end
    model.seating = seating
end

---@param model LobbyModel
---@param outcome CoordinatorOutcome
---@param effects LobbyEffect[]
local function absorb(model, outcome, effects)
    if not outcome.accepted and outcome.reason then
        model.error = outcome.reason
    end
    for _, action in ipairs(outcome.actions) do
        if action.kind == "send" then
            local wire = action.message and protocol.encode(action.message) or nil
            if wire then
                for _, target in ipairs(action.targets or {}) do
                    effects[#effects + 1] = { kind = "send", link_id = target, wire = wire }
                end
            else
                model.error = "a control message could not be encoded"
            end
        elseif action.kind == "close" then
            effects[#effects + 1] = { kind = "close", link_id = assert(action.link_id) }
        elseif action.kind == "start_match" then
            model.started = true
            effects[#effects + 1] = { kind = "start_match", freeze = assert(action.freeze) }
        end
    end
end

---@param model LobbyModel
---@param event table
---@param effects LobbyEffect[]
---@return CoordinatorOutcome?
local function step(model, event, effects)
    local state = model.coordinator
    if not state then
        return nil
    end
    -- A session that already ended keeps its reason. Late transport traffic is
    -- expected after a termination and must not overwrite it with "the session
    -- already ended".
    if state.phase == "terminal" and event.kind ~= "tick" then
        return nil
    end
    local next_state, outcome = coordinator.step(state, event)
    model.coordinator = next_state
    absorb(model, outcome, effects)
    refresh_seating(model)
    return outcome
end

---@param model LobbyModel
---@return SessionSlotProducer[]?
local function planned_assignments(model)
    local state = model.coordinator
    if not state or state.role ~= "host" or not state.manifest then
        return nil
    end
    return (coordinator.plan_assignments(state.manifest, model.seating))
end

-- Publishing is deferred until every admitted peer has accepted the manifest,
-- because #163 refuses ownership before that. Byte-identical ownership is
-- refused as idempotent by the coordinator, so this is safe to call after any
-- roster or seating change without churning readiness.
---@param model LobbyModel
---@param effects LobbyEffect[]
local function publish_assignments(model, effects)
    local state = model.coordinator
    if not state or state.role ~= "host" then
        return
    end
    if state.phase ~= "manifest" and state.phase ~= "assigned" and state.phase ~= "ready" then
        return
    end
    for _, peer in ipairs(state.peers) do
        if peer.accepted_manifest_id ~= state.manifest_id then
            return
        end
    end
    local assignments = planned_assignments(model)
    if not assignments then
        return
    end
    step(model, { kind = "assign_slots", assignments = assignments }, effects)
end

---@param model LobbyModel
---@return boolean
local function configurable(model)
    local state = model.coordinator
    if not state then
        return false
    end
    return state.phase ~= "countdown"
        and state.phase ~= "running"
        and state.phase ~= "result"
        and state.phase ~= "terminal"
end

---@param model LobbyModel
---@param role LobbyRole
---@param effects LobbyEffect[]
local function choose_role(model, role, effects)
    if model.coordinator then
        model.error = "the session role is already chosen"
        return
    end
    model.role = role
    if role == "host" then
        model.peer_id = transport_contract.HOST_PEER_ID
    end
    effects[#effects + 1] = { kind = "open_star", role = role, peer_id = model.peer_id }
    if role == "host" then
        model.coordinator = assert(coordinator.new({
            role = "host",
            session_id = model.session_id,
            peer_id = model.peer_id,
            runtime = protocol_fixture.runtime(),
        }))
        model.status = "Pick a match mode, then invite peers."
    else
        model.coordinator = assert(coordinator.new({
            role = "guest",
            session_id = model.session_id,
            peer_id = model.peer_id,
            host_peer_id = transport_contract.HOST_PEER_ID,
            host_link_id = transport_contract.HOST_PEER_ID,
            runtime = protocol_fixture.runtime(),
            expectation = expectation_for(model),
        }))
        model.status = "Paste the host's offer to connect."
    end
    refresh_seating(model)
end

---@param model LobbyModel
---@param mode SessionMatchMode
local function set_mode(model, mode)
    if model.role ~= "host" then
        model.error = "only the host chooses the match mode"
        return
    end
    if not protocol.MATCH_MODES[mode] then
        model.error = "unsupported match mode"
        return
    end
    local state = assert(model.coordinator)
    -- The mode lives in the immutable manifest, so it locks at proposal rather
    -- than at countdown. Every configuration choice downstream of it — seating,
    -- and therefore readiness — is discarded when it changes.
    if state.manifest_id then
        model.error = "the match mode is fixed once the manifest is proposed"
        return
    end
    if model.mode ~= mode then
        model.seating = {}
        refresh_seating(model)
    end
    model.mode = mode
    model.status = ("%s seats %d humans."):format(mode, required_humans(model))
end

---@param model LobbyModel
---@param effects LobbyEffect[]
local function invite(model, effects)
    if model.role ~= "host" then
        model.error = "only the host invites peers"
        return
    end
    local state = assert(model.coordinator)
    if state.manifest_id then
        model.error = "admission closed when the manifest was proposed"
        return
    end
    if model.pending_link then
        model.error = "finish the pending invitation first"
        return
    end
    if model.guests >= required_humans(model) - 1 then
        model.error = ("%s seats %d humans"):format(effective_mode(model), required_humans(model))
        return
    end
    if model.guests >= transport_contract.MAX_GUESTS then
        model.error = "the star transport is at guest capacity"
        return
    end
    model.guests = model.guests + 1
    local link_id = lobby_model.GUEST_LINK_PREFIX .. tostring(model.guests)
    model.pending_link = link_id
    effects[#effects + 1] = { kind = "open_peer", peer_id = link_id }
    effects[#effects + 1] = { kind = "request_offer", peer_id = link_id }
    model.status = "Creating an offer for " .. link_id .. "."
end

---@param model LobbyModel
---@param effects LobbyEffect[]
local function export_signal(model, effects)
    if not model.outgoing then
        model.error = "no signaling blob is waiting"
        return
    end
    effects[#effects + 1] = { kind = "clipboard", text = model.outgoing }
    -- The blob leaves the model the moment it is handed over: nothing renders
    -- it, nothing logs it, and a later screenshot cannot leak it.
    model.outgoing = nil
    model.status = "Signal copied. Send it to your peer."
end

---@param model LobbyModel
---@param text any
---@param effects LobbyEffect[]
local function import_signal(model, text, effects)
    if type(text) ~= "string" or #text == 0 then
        model.error = "the pasted signal is empty"
        return
    end
    if #text > lobby_model.MAX_SIGNAL_BYTES then
        model.error = "the pasted signal is too large to be a signaling blob"
        return
    end
    if model.role ~= "host" and model.role ~= "guest" then
        model.error = "choose host or guest before pasting a signal"
        return
    end
    if model.role == "host" and not model.pending_link then
        model.error = "invite a peer before pasting an answer"
        return
    end
    local host_side = model.role == "host"
    -- Only the shape of the blob is retained. The bytes go straight to the
    -- transport and are never held, rendered, or logged.
    model.imported = {
        direction = host_side and "answer" or "offer",
        peer_id = host_side and assert(model.pending_link) or transport_contract.HOST_PEER_ID,
        bytes = #text,
        fingerprint = fingerprint(text),
    }
    if host_side then
        effects[#effects + 1] =
            { kind = "accept_answer", peer_id = assert(model.pending_link), signal = text }
        model.status = "Answer accepted for " .. assert(model.pending_link) .. "."
    else
        effects[#effects + 1] = { kind = "accept_offer", signal = text }
        model.status = "Offer accepted. Copy your answer back to the host."
    end
end

---@param model LobbyModel
---@param effects LobbyEffect[]
local function lock_session(model, effects)
    if model.role ~= "host" then
        model.error = "only the host proposes the manifest"
        return
    end
    local state = assert(model.coordinator)
    if state.manifest_id then
        model.error = "the manifest is already proposed"
        return
    end
    local required = required_humans(model)
    if #state.peers < required and not model.bot_fill then
        model.error = ("%s needs %d humans; %d are connected"):format(
            effective_mode(model),
            required,
            #state.peers
        )
        return
    end
    local outcome = step(
        model,
        { kind = "propose_manifest", manifest = manifest_for(model, model.mode) },
        effects
    )
    if outcome and outcome.accepted then
        model.status = "Manifest proposed. Waiting for peers to accept."
    end
    publish_assignments(model, effects)
end

---@param model LobbyModel
---@param index any
---@param effects LobbyEffect[]
local function swap_seats(model, index, effects)
    if model.role ~= "host" then
        model.error = "only the host reassigns ownership"
        return
    end
    if not configurable(model) then
        model.error = "ownership is frozen"
        return
    end
    if type(index) ~= "number" or index < 1 or index + 1 > #model.seating then
        model.error = "there is no seat to swap with"
        return
    end
    model.seating[index], model.seating[index + 1] = model.seating[index + 1], model.seating[index]
    model.status = "Ownership republished; readiness cleared."
    publish_assignments(model, effects)
end

---@param model LobbyModel
---@param ready any
---@param effects LobbyEffect[]
local function set_ready(model, ready, effects)
    if type(ready) ~= "boolean" then
        model.error = "readiness must be a boolean"
        return
    end
    step(model, { kind = "set_ready", ready = ready }, effects)
end

---@param model LobbyModel
---@param effects LobbyEffect[]
local function begin_countdown(model, effects)
    if model.role ~= "host" then
        model.error = "only the host starts the countdown"
        return
    end
    local outcome = step(model, {
        kind = "begin_countdown",
        countdown_id = lobby_model.COUNTDOWN_ID,
        remaining_ticks = lobby_model.COUNTDOWN_TICKS,
        first_input_tick = lobby_model.FIRST_INPUT_TICK,
    }, effects)
    if outcome and outcome.accepted then
        model.status = "Countdown started. Configuration is frozen."
    end
end

---@param model LobbyModel
---@param effects LobbyEffect[]
local function leave(model, effects)
    local state = model.coordinator
    if state and state.phase ~= "terminal" then
        if state.role == "guest" then
            step(model, { kind = "leave" }, effects)
        else
            step(
                model,
                { kind = "abort", code = "host_abort", detail = "host left the lobby" },
                effects
            )
        end
    end
    -- The departure notice is still queued on the transport, so the link is
    -- deliberately left open here: the owning screen tears it down once the
    -- notice has had its chance to leave. Shutting down now would drop it.
    effects[#effects + 1] = { kind = "leave" }
end

---@param model LobbyModel
---@param command table
local function on_signal(model, command)
    model.outgoing = command.signal
    local direction = model.role == "host" and "offer" or "answer"
    model.exported = {
        direction = direction,
        peer_id = command.peer_id,
        bytes = #command.signal,
        fingerprint = fingerprint(command.signal),
    }
    model.status = direction == "offer" and "Offer ready. Copy it to your peer."
        or "Answer ready. Copy it back to the host."
end

---@param model LobbyModel
---@param command table
---@param effects LobbyEffect[]
local function on_peer_connected(model, command, effects)
    if model.role == "host" then
        if model.pending_link == command.peer_id then
            model.pending_link = nil
        end
        model.status = command.peer_id .. " connected."
    else
        model.status = "Connected to the host."
        step(model, { kind = "connect" }, effects)
    end
end

---@type table<string, fun(model: LobbyModel, command: table, effects: LobbyEffect[])>
local COMMANDS = {
    role = function(model, command, effects)
        choose_role(model, command.role, effects)
    end,
    mode = function(model, command)
        set_mode(model, command.mode)
    end,
    -- Which invitation this peer is answering. It must agree with the host's
    -- Nth opened link, so it is chosen before the role is locked in.
    identity = function(model)
        if model.coordinator then
            model.error = "identity is fixed once the session starts"
            return
        end
        local index = tonumber(
            model.peer_id:match("^" .. lobby_model.GUEST_LINK_PREFIX .. "(%d+)$")
        ) or 1
        index = index % transport_contract.MAX_GUESTS + 1
        model.peer_id = lobby_model.GUEST_LINK_PREFIX .. tostring(index)
        model.status = "Joining as " .. model.peer_id .. "."
    end,
    bot_fill = function(model)
        if model.role ~= "host" then
            model.error = "only the host approves bot fills"
            return
        end
        model.bot_fill = not model.bot_fill
        model.status = model.bot_fill and "Empty seats will be filled with AI."
            or "Every seat must be a connected human."
    end,
    invite = function(model, _, effects)
        invite(model, effects)
    end,
    copy = function(model, _, effects)
        export_signal(model, effects)
    end,
    paste_request = function(model, _, effects)
        effects[#effects + 1] = { kind = "paste_request" }
    end,
    paste = function(model, command, effects)
        import_signal(model, command.text, effects)
    end,
    lock = function(model, _, effects)
        lock_session(model, effects)
    end,
    swap = function(model, command, effects)
        swap_seats(model, command.index, effects)
    end,
    ready = function(model, command, effects)
        set_ready(model, command.ready, effects)
    end,
    start = function(model, _, effects)
        begin_countdown(model, effects)
    end,
    leave = function(model, _, effects)
        leave(model, effects)
    end,
    tick = function(model, _, effects)
        step(model, { kind = "tick" }, effects)
    end,
    signal = function(model, command)
        on_signal(model, command)
    end,
    peer_connected = function(model, command, effects)
        on_peer_connected(model, command, effects)
    end,
    control = function(model, command, effects)
        step(model, { kind = "control", link_id = command.link_id, wire = command.wire }, effects)
        publish_assignments(model, effects)
    end,
    link_lost = function(model, command, effects)
        step(model, { kind = "link_lost", link_id = command.link_id }, effects)
    end,
    link_error = function(model, command)
        model.error = command.detail or "the transport reported a failure"
    end,
}

-- The single pure entry point. Unknown commands are ignored rather than fatal
-- so a future screen control cannot crash a live session.
---@param model LobbyModel
---@param command table
---@return LobbyModel, LobbyEffect[]
function lobby_model.command(model, command)
    ---@type LobbyEffect[]
    local effects = {}
    local handler = type(command) == "table"
            and type(command.kind) == "string"
            and COMMANDS[command.kind]
        or nil
    if not handler then
        return model, effects
    end
    local next_model = copy(model)
    next_model.error = nil
    handler(next_model, command, effects)
    return next_model, effects
end

-- ---------------------------------------------------------------------------
-- Derived presentation
-- ---------------------------------------------------------------------------

---@class LobbySlotView
---@field slot InputSlotId
---@field team "home"|"away"
---@field player_id string
---@field owner string? -- Producer id, or nil while ownership is unpublished.
---@field owner_kind "peer"|"bot"|nil
---@field driver CoordinatorSlotDriver|"pending"
---@field live boolean -- The opening live slot of its human owner.
---@field local_owner boolean

---@class LobbySeatView
---@field index integer
---@field peer_id string
---@field is_local boolean
---@field ready boolean
---@field slots InputSlotId[]

---@class LobbyKeeperView
---@field team "home"|"away"
---@field player_id string

---@class LobbyIdentityRow
---@field label string
---@field value string

---@class LobbyView
---@field role LobbyRole?
---@field peer_id string
---@field phase SessionLifecyclePhase|"role"
---@field mode SessionMatchMode
---@field mode_locked boolean
---@field mode_known boolean -- False while a guest has not yet seen the manifest.
---@field required integer
---@field connected integer
---@field ready_count integer
---@field bot_fill boolean
---@field slots LobbySlotView[]
---@field keepers LobbyKeeperView[]
---@field seats LobbySeatView[]
---@field identity LobbyIdentityRow[]
---@field countdown integer?
---@field terminal CoordinatorTerminal?
---@field terminal_text string?
---@field exported LobbySignalRecord?
---@field imported LobbySignalRecord?
---@field has_outgoing boolean
---@field status string
---@field error string?
---@field can_invite boolean
---@field can_lock boolean
---@field can_configure boolean -- Host-side ownership can still be republished.
---@field can_ready boolean
---@field ready boolean
---@field can_start boolean
---@field started boolean

---@param model LobbyModel
---@return SessionSlotProducer[]?
local function visible_assignments(model)
    local state = model.coordinator
    if state and state.assignments then
        return state.assignments
    end
    return planned_assignments(model)
end

---@param model LobbyModel
---@return SessionManifest
local function visible_manifest(model)
    local state = model.coordinator
    if state and state.manifest then
        return state.manifest
    end
    return manifest_for(model, model.mode)
end

---@param model LobbyModel
---@return LobbySlotView[], LobbyKeeperView[]
local function roster_view(model)
    local manifest = visible_manifest(model)
    local assignments = visible_assignments(model)
    -- The opening live slot is the coordinator's rule, not the lobby's; the
    -- freeze records the same table for the same assignments.
    local live = coordinator.preview_live(assignments)
    ---@type LobbySlotView[]
    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        local slot = assert(input_frame.slot(index))
        local entry = manifest.slots[index]
        local producer = assignments and assignments[index] or nil
        local is_live = producer ~= nil
            and producer.producer_kind == "peer"
            and live[producer.producer_id] == producer.slot
        slots[index] = {
            slot = slot.id,
            team = slot.team,
            player_id = entry and entry.player_id or "?",
            owner = producer and producer.producer_id or nil,
            owner_kind = producer and producer.producer_kind or nil,
            driver = producer and (is_live and "human" or "ai") or "pending",
            live = is_live,
            local_owner = producer ~= nil
                and producer.producer_kind == "peer"
                and producer.producer_id == model.peer_id,
        }
    end
    ---@type LobbyKeeperView[]
    local keepers = {}
    for _, team in ipairs(manifest.teams) do
        for _, player in ipairs(team.roster) do
            if player.position == "keeper" then
                keepers[#keepers + 1] = { team = team.team, player_id = player.player_id }
            end
        end
    end
    return slots, keepers
end

---@param model LobbyModel
---@return LobbySeatView[]
local function seats_view(model)
    local state = model.coordinator
    ---@type LobbySeatView[]
    local seats = {}
    if not state then
        return seats
    end
    local assignments = visible_assignments(model)
    ---@type string[]
    local order = {}
    if state.role == "host" then
        for index, id in ipairs(model.seating) do
            order[index] = id
        end
    else
        for index, peer in ipairs(state.peers) do
            order[index] = peer.peer_id
        end
    end
    for index, peer_id in ipairs(order) do
        ---@type InputSlotId[]
        local owned = {}
        for _, producer in ipairs(assignments or {}) do
            if producer.producer_kind == "peer" and producer.producer_id == peer_id then
                owned[#owned + 1] = producer.slot
            end
        end
        local ready = false
        for _, peer in ipairs(state.peers) do
            if peer.peer_id == peer_id then
                ready = peer.ready
            end
        end
        seats[index] = {
            index = index,
            peer_id = peer_id,
            is_local = peer_id == state.peer_id,
            ready = ready,
            slots = owned,
        }
    end
    return seats
end

---@param model LobbyModel
---@return LobbyIdentityRow[]
local function identity_view(model)
    local manifest = visible_manifest(model)
    local state = model.coordinator
    return {
        { label = "MODE", value = manifest.match_mode },
        {
            label = "MANIFEST",
            value = state and state.manifest_id and state.manifest_id:sub(1, 12) or "unproposed",
        },
        { label = "BUILD", value = manifest.build_id },
        { label = "CONTENT", value = manifest.content_id },
        { label = "TUNING", value = manifest.tuning_id },
        { label = "RULES", value = manifest.combat_rules_id },
        { label = "COMBAT", value = manifest.combat_status },
    }
end

---@param model LobbyModel
---@return LobbyView
function lobby_model.view(model)
    local state = model.coordinator
    local slots, keepers = roster_view(model)
    local required = required_humans(model)
    local connected = state and #state.peers or 0
    local ready_count = 0
    local ready = false
    if state then
        for _, peer in ipairs(state.peers) do
            if peer.ready then
                ready_count = ready_count + 1
                if peer.peer_id == state.peer_id then
                    ready = true
                end
            end
        end
    end
    local phase = state and state.phase or "role"
    local terminal = state and state.terminal or nil
    return {
        role = model.role,
        peer_id = model.peer_id,
        phase = phase,
        mode = effective_mode(model),
        mode_locked = state ~= nil and state.manifest_id ~= nil,
        mode_known = model.role == "host" or (state ~= nil and state.manifest ~= nil),
        required = required,
        connected = connected,
        ready_count = ready_count,
        bot_fill = model.bot_fill,
        slots = slots,
        keepers = keepers,
        seats = seats_view(model),
        identity = identity_view(model),
        countdown = state and state.countdown_remaining or nil,
        terminal = terminal,
        terminal_text = terminal and lobby_model.TERMINAL_TEXT[terminal.reason] or nil,
        exported = model.exported,
        imported = model.imported,
        has_outgoing = model.outgoing ~= nil,
        status = model.status,
        error = model.error,
        can_invite = model.role == "host"
            and phase == "handshake"
            and model.pending_link == nil
            and connected < required,
        can_lock = model.role == "host"
            and phase == "handshake"
            and (connected >= required or model.bot_fill),
        can_configure = model.role == "host" and configurable(model),
        can_ready = phase == "assigned" or phase == "ready",
        ready = ready,
        can_start = model.role == "host" and phase == "ready",
        started = model.started,
    }
end

return lobby_model
