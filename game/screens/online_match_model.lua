-- Pure flow model for the online match.
--
-- It owns no simulation and no transport. The driver owns the match, the
-- coordinator owns the session, and this module is the deliberate policy between
-- them: which control-plane events a confirmed simulation step justifies, what a
-- terminal driver status reports, when a result may be committed, and how focus
-- loss, a pause request, a lost controller, and an abort are answered.
--
-- Every function is pure — `command` returns a fresh model plus an ordered
-- effect list — so a whole host/guest session replays headlessly with no display,
-- no clock, and no network.
--
-- # Two rules worth stating out loud
--
-- **Local focus loss cannot stop the match.** A peer that stops advancing stops
-- publishing authority, and every other peer stalls behind it: an online pause is
-- a pause for everybody, and OMP-3 has no host-arbitrated pause. So a focus loss
-- or a pause request produces a *notice* and nothing else, and the driver keeps
-- stepping. The escape hatch is an explicit abort, which ends the session for
-- everyone rather than silently freezing it.
--
-- **A predicted result is never committed.** Full time is published from the
-- simulation, but the result this peer records comes from the coordinator's
-- acknowledged `result` after the session completes. Until then the screen holds
-- on a "waiting for every peer" state, and a disagreement terminates the session
-- as a desync rather than showing two different scoreboards.

local coordinator = require("game.online.coordinator")
local protocol = require("game.online.protocol")

---@alias OnlineMatchPhase "playing"|"full_time"|"ended"

---@alias OnlineMatchEffect
---| { kind: "send", link_id: string, wire: string }
---| { kind: "close", link_id: string, detail: string? }
---| { kind: "observe_hash", tick: integer, hash: string }

---@class OnlineMatchNotice
---@field text string
---@field remaining number -- Seconds of display left.

---@class OnlineMatchModel
---@field request OnlineMatchRequest
---@field coordinator CoordinatorState
---@field phase OnlineMatchPhase
---@field published SessionMatchPhase? -- Host only: the last match phase it published.
---@field score { home: integer, away: integer }
---@field tick integer -- Highest confirmed simulation tick reported to the session.
---@field notice OnlineMatchNotice?
---@field abort_prompt boolean
---@field result CoordinatorResult?
---@field terminal CoordinatorTerminal?
---@field error string?

---@class OnlineMatchModelModule
local online_match_model = {}

online_match_model.NOTICE_SECONDS = 3.5

-- The longest legal walk between two match phases: goal_stoppage -> kickoff ->
-- playing -> full_time, plus the opening step from nothing.
---@type SessionMatchPhase[]
online_match_model.MATCH_PHASE_CHAIN = { "kickoff", "playing", "goal_stoppage", "full_time" }

online_match_model.PAUSE_NOTICE =
    "Online match: pausing would stall every peer. Press Escape again to abort."
online_match_model.FOCUS_NOTICE = "Window lost focus — the online match kept running."
online_match_model.CONTROLLER_NOTICE = "Controller disconnected — keyboard input still works."
online_match_model.ABORT_PROMPT =
    "Abort the session for every peer? Escape confirms, any other key resumes."

---@param model OnlineMatchModel
---@return OnlineMatchModel
local function copy(model)
    return {
        request = model.request,
        coordinator = model.coordinator,
        phase = model.phase,
        published = model.published,
        score = { home = model.score.home, away = model.score.away },
        tick = model.tick,
        notice = model.notice and {
            text = model.notice.text,
            remaining = model.notice.remaining,
        } or nil,
        abort_prompt = model.abort_prompt,
        result = model.result,
        terminal = model.terminal,
        error = model.error,
    }
end

---@param request OnlineMatchRequest
---@param state CoordinatorState
---@return OnlineMatchModel
function online_match_model.new(request, state)
    return {
        request = request,
        coordinator = state,
        phase = "playing",
        published = nil,
        score = { home = 0, away = 0 },
        tick = 0,
        notice = nil,
        abort_prompt = false,
        result = nil,
        terminal = nil,
        error = nil,
    }
end

---@param model OnlineMatchModel
---@param text string
local function notify(model, text)
    model.notice = { text = text, remaining = online_match_model.NOTICE_SECONDS }
end

---@param model OnlineMatchModel
---@param outcome CoordinatorOutcome
---@param effects OnlineMatchEffect[]
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
        elseif action.kind == "terminate" then
            model.terminal = assert(action.terminal)
        end
    end
end

---@param model OnlineMatchModel
---@param event table
---@param effects OnlineMatchEffect[]
local function step(model, event, effects)
    local state = model.coordinator
    -- A session that already ended keeps its reason. Late traffic after a
    -- termination must not overwrite it with "the session already ended".
    if state.phase == "terminal" and event.kind ~= "tick" then
        return
    end
    local next_state, outcome = coordinator.step(state, event)
    model.coordinator = next_state
    absorb(model, outcome, effects)
end

---@param model OnlineMatchModel
local function settle(model)
    local state = model.coordinator
    if state.phase == "terminal" then
        model.phase = "ended"
        model.terminal = model.terminal or state.terminal
        -- Only a completed session carries an acknowledged result. Anything else
        -- ends without one, deliberately: there is no half-agreed score.
        if state.terminal and state.terminal.reason == "completed" then
            model.result = state.result or state.local_result
        end
    end
end

---@param model OnlineMatchModel
---@param phase SessionMatchPhase
---@param effects OnlineMatchEffect[]
local function publish_phase(model, phase, effects)
    step(model, {
        kind = "match_phase",
        phase = phase,
        tick = model.tick,
        home_score = model.score.home,
        away_score = model.score.away,
    }, effects)
    model.published = phase
end

-- The coordinator's match-phase order is a chain: a running match opens with
-- `kickoff`, `playing` follows it, `goal_stoppage` and `full_time` follow
-- `playing`, and `kickoff` follows a stoppage. Walking that chain here means a
-- caller names the phase it *means* and cannot accidentally publish an illegal
-- transition — which would be refused, leaving the session stuck one step short
-- of the result.
---@param model OnlineMatchModel
---@param target SessionMatchPhase
---@param effects OnlineMatchEffect[]
local function advance_to(model, target, effects)
    for _ = 1, #online_match_model.MATCH_PHASE_CHAIN do
        if model.published == target then
            return
        end
        local current = model.published
        local next_phase
        if current == nil then
            next_phase = "kickoff"
        elseif current == "kickoff" then
            next_phase = "playing"
        elseif current == "goal_stoppage" then
            next_phase = "kickoff"
        elseif current == "playing" then
            next_phase = target
        else
            return -- Full time has no successor; the result path takes over.
        end
        publish_phase(model, next_phase, effects)
    end
end

-- Publish the control-plane consequences of one confirmed simulation step. The
-- ordering the coordinator enforces (kickoff, then playing, then a goal stoppage
-- only after the score actually moved, then kickoff again) is derived here from
-- the confirmed lifecycle records alone, so a predicted goal can never advance
-- the session.
---@param model OnlineMatchModel
---@param steps RollbackEventStep[]
---@param effects OnlineMatchEffect[]
local function publish_steps(model, steps, effects)
    if model.request.role ~= "host" then
        return
    end
    for _, confirmed in ipairs(steps) do
        model.tick = math.max(model.tick, confirmed.tick + model.request.first_input_tick)
        for _, event in ipairs(confirmed.lifecycle_events) do
            local payload = event.payload
            if payload.kind == "goal" then
                -- The score has to move *before* the stoppage is published: the
                -- coordinator refuses a goal stoppage that no goal preceded.
                model.score.home = payload.score.home
                model.score.away = payload.score.away
                advance_to(model, "goal_stoppage", effects)
            elseif payload.kind == "kickoff" then
                advance_to(model, "kickoff", effects)
            end
        end
        advance_to(model, "playing", effects)
    end
end

---@param model OnlineMatchModel
---@param event table
---@param effects OnlineMatchEffect[]
local function reach_full_time(model, event, effects)
    if model.phase ~= "playing" then
        return
    end
    model.phase = "full_time"
    model.score.home = event.home_score
    model.score.away = event.away_score
    model.tick = math.max(model.tick, event.final_tick)
    if model.request.role == "host" then
        advance_to(model, "full_time", effects)
    end
    -- Both roles report their own result. The host's becomes the session result;
    -- a guest's is retained locally so the host's acknowledgement is checked
    -- against a simulation this peer actually ran, rather than accepted on faith.
    step(model, {
        kind = "finish",
        final_tick = model.tick,
        home_score = model.score.home,
        away_score = model.score.away,
        final_hash = event.final_hash,
    }, effects)
end

---@param model OnlineMatchModel
---@param event table
---@param effects OnlineMatchEffect[]
local function report_failure(model, event, effects)
    if event.failure then
        step(model, {
            kind = "netcode_failure",
            failure = event.failure,
            detail = event.detail,
        }, effects)
        return
    end
    -- A lost star has no failure class of its own. A guest knows exactly which
    -- link died — the only one it has — so it reports the honest disconnect; a
    -- host cannot tell which guest went, because the driver folds every link
    -- event into one status, so it ends the session explicitly instead.
    local state = model.coordinator
    if state.role == "guest" and state.host_link_id then
        step(model, {
            kind = "link_lost",
            link_id = state.host_link_id,
            code = "transport_lost",
        }, effects)
        return
    end
    step(model, {
        kind = "abort",
        code = "peer_disconnect",
        detail = event.detail or "the star transport was lost",
    }, effects)
end

---@param model OnlineMatchModel
---@param link_id string
---@param wire string
---@param effects OnlineMatchEffect[]
local function absorb_control(model, link_id, wire, effects)
    local message = protocol.decode(wire)
    -- A peer's boundary hash is the driver's business as well as the session's:
    -- the coordinator counts consecutive disagreements, the driver compares the
    -- boundary it actually hashed. Both authorities see the same report.
    if message and message.kind == "hash_report" then
        local body = message.body
        ---@cast body SessionHashReportBody
        effects[#effects + 1] = {
            kind = "observe_hash",
            tick = body.tick,
            hash = body.boundary_hash,
        }
    end
    step(model, { kind = "control", link_id = link_id, wire = wire }, effects)
end

-- One flow event. `event` is abstracted the same way a screen event is: the
-- caller translates transport traffic, driver batches, and raw input into these
-- shapes and executes the returned effects.
---@param model OnlineMatchModel
---@param event table
---@return OnlineMatchModel, OnlineMatchEffect[]
function online_match_model.command(model, event)
    local next_model = copy(model)
    ---@type OnlineMatchEffect[]
    local effects = {}
    local kind = event.kind
    if kind == "control" then
        absorb_control(next_model, event.link_id, event.wire, effects)
    elseif kind == "tick" then
        step(next_model, { kind = "tick" }, effects)
        if next_model.notice then
            local remaining = next_model.notice.remaining - (event.dt or 0)
            next_model.notice = remaining > 0
                    and { text = next_model.notice.text, remaining = remaining }
                or nil
        end
    elseif kind == "confirmed" then
        publish_steps(next_model, event.steps or {}, effects)
    elseif kind == "checkpoints" then
        for _, checkpoint in ipairs(event.checkpoints or {}) do
            step(next_model, {
                kind = "hash_report",
                tick = checkpoint.tick,
                boundary_hash = checkpoint.hash,
            }, effects)
        end
    elseif kind == "full_time" then
        reach_full_time(next_model, event, effects)
    elseif kind == "failure" then
        report_failure(next_model, event, effects)
    elseif kind == "link_lost" then
        step(next_model, {
            kind = "link_lost",
            link_id = event.link_id,
            code = event.code or "transport_lost",
        }, effects)
    elseif kind == "pause_request" then
        -- Deliberate: no local pause. The first press explains why, the second
        -- offers the only honest alternative.
        if next_model.abort_prompt then
            step(next_model, {
                kind = "abort",
                code = "host_abort",
                detail = "a peer aborted the online match",
            }, effects)
        else
            next_model.abort_prompt = true
            notify(next_model, online_match_model.PAUSE_NOTICE)
        end
    elseif kind == "resume_request" then
        next_model.abort_prompt = false
    elseif kind == "focus_lost" then
        notify(next_model, online_match_model.FOCUS_NOTICE)
    elseif kind == "controller_lost" then
        notify(next_model, online_match_model.CONTROLLER_NOTICE)
    elseif kind == "abort" then
        step(next_model, {
            kind = "abort",
            code = "host_abort",
            detail = event.detail or "a peer aborted the online match",
        }, effects)
    end
    settle(next_model)
    return next_model, effects
end

---@param model OnlineMatchModel
---@return boolean
function online_match_model.ended(model)
    return model.phase == "ended"
end

-- What the flow should do once the session has ended. A completed session has an
-- acknowledged result to show; anything else leaves with its terminal reason and
-- no score, because a session that failed has no half-agreed outcome to display.
---@param model OnlineMatchModel
---@return "result"|"terminal"
function online_match_model.exit_route(model)
    return model.result ~= nil and "result" or "terminal"
end

return online_match_model
