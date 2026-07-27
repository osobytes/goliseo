-- Reconciliation smoke for the online presentation timeline.
--
-- Real drivers, a real in-process star, real arrivals. What is asserted is the
-- presentation contract the renderer depends on: one stable identity per
-- confirmed event, published exactly once, and a correction that replaces the
-- whole speculative tail rather than layering a second copy over it.
--
-- What is *not* asserted, deliberately: the individual combat correction cases
-- (wind-up, guard, contact, projectile flight, stagger, ball spill, immunity
-- expiry). The rows are still produced by the pre-#112 deterministic bot, which
-- never drives the companion into those states, so pinning them here would pin
-- an absence. The mechanism — the companion surviving a correction — is covered.

local t = require("spec.support.runner")
local fixture = require("spec.fixtures.online_match_session")
local match_driver = require("game.online.match_driver")
local match_presentation = require("game.online.match_presentation")
local match_session = require("game.online.match_session")
local input_frame = require("sim.input_frame")
local match_snapshot = require("sim.match_snapshot")

---@class PresentationPeer
---@field request OnlineMatchRequest
---@field driver MatchDriver
---@field presentation OnlineMatchPresentation
---@field confirmed table<string, integer> -- Event id -> times published.
---@field revoked table<string, boolean>
---@field corrections integer
---@field confirmed_ticks integer[]

---@class PresentationHarness
---@field session OnlineMatchFixtureSession
---@field peers PresentationPeer[]

---@param mode SessionMatchMode
---@param duration_ticks integer?
---@return PresentationHarness
local function harness(mode, duration_ticks)
    local session = fixture.session(mode, nil, duration_ticks)
    ---@type PresentationPeer[]
    local peers = {}
    for index, peer_id in ipairs(session.peer_ids) do
        local role = index == 1 and "host" or "guest"
        local request = assert(match_session.request({
            role = role,
            peer_id = peer_id,
            manifest = session.manifest,
            freeze = session.freeze,
        }))
        local transport = index == 1 and session.host_transport
            or assert(session.guest_transports[peer_id])
        peers[index] = {
            request = request,
            driver = match_driver.new({
                role = role,
                peer_id = peer_id,
                freeze = session.freeze,
                manifest = session.manifest,
                transport = transport,
                initial_snapshot = request.initial_snapshot,
            }),
            presentation = match_presentation.new(
                request.initial_snapshot,
                request.first_input_tick
            ),
            confirmed = {},
            revoked = {},
            corrections = 0,
            confirmed_ticks = {},
        }
    end
    return { session = session, peers = peers }
end

---@param peer PresentationPeer
---@param batch RollbackPlayableLabBatch
local function record(peer, batch)
    peer.corrections = peer.corrections + #batch.corrections
    for _, diff in ipairs(batch.event_diffs) do
        for _, event in ipairs(diff.revoked) do
            peer.revoked[event.id] = true
        end
        for _, replacement in ipairs(diff.replaced) do
            peer.revoked[replacement.before.id] = true
        end
    end
    for _, step in ipairs(batch.confirmed_steps) do
        peer.confirmed_ticks[#peer.confirmed_ticks + 1] = step.tick
        for _, list in ipairs({
            step.match_events,
            step.combat_events or {},
            step.lifecycle_events,
        }) do
            for _, event in ipairs(list) do
                peer.confirmed[event.id] = (peer.confirmed[event.id] or 0) + 1
            end
        end
    end
end

---@param state PresentationHarness
---@param steps integer
---@param period integer? -- Deliver every `period` steps; nil delivers every step.
local function run(state, steps, period)
    for step = 1, steps do
        for _, peer in ipairs(state.peers) do
            local batch = match_driver.advance(peer.driver, input_frame.neutral_sample())
            record(peer, match_presentation.consume(peer.presentation, peer.driver, batch))
        end
        if period == nil or step % period == 0 then
            state.session.host_transport:pump()
        end
    end
end

t.describe("online match presentation", function()
    t.it("publishes each confirmed event exactly once under clean delivery", function()
        local state = harness("2v2")
        run(state, 40)
        for index, peer in ipairs(state.peers) do
            for id, count in pairs(peer.confirmed) do
                t.eq(count, 1, ("peer %d published %s more than once"):format(index, id))
            end
            for tick_index = 2, #peer.confirmed_ticks do
                t.eq(
                    peer.confirmed_ticks[tick_index],
                    peer.confirmed_ticks[tick_index - 1] + 1,
                    "confirmed steps must be contiguous"
                )
            end
        end
    end)

    t.it("tracks the driver's own confirmation ceiling", function()
        local state = harness("2v2")
        run(state, 40)
        for _, peer in ipairs(state.peers) do
            local diagnostics = match_driver.diagnostics(peer.driver)
            local timeline = match_presentation.diagnostics(peer.presentation)
            t.eq(
                timeline.confirmed_tick + peer.request.first_input_tick,
                diagnostics.confirmed_output_tick
            )
            t.eq(match_presentation.status(peer.presentation), "active")
        end
    end)

    t.it("replaces the speculative tail on a correction and never re-publishes it", function()
        local state = harness("2v2")
        run(state, 48, 6)
        local corrected = 0
        for index, peer in ipairs(state.peers) do
            corrected = corrected + peer.corrections
            for id in pairs(peer.revoked) do
                t.is_true(
                    peer.confirmed[id] == nil,
                    ("peer %d confirmed a revoked event %s"):format(index, id)
                )
            end
            for id, count in pairs(peer.confirmed) do
                t.eq(count, 1, ("peer %d published %s more than once"):format(index, id))
            end
        end
        t.is_true(corrected > 0, "bursty delivery must produce at least one correction")
    end)

    t.it("carries the combat companion through correction and resimulation", function()
        local state = harness("2v2")
        run(state, 48, 6)
        for _, peer in ipairs(state.peers) do
            local _, combat = match_snapshot.restore(match_driver.current_snapshot(peer.driver))
            t.is_true(combat ~= nil, "the combat companion must survive resimulation")
        end
    end)

    t.it("agrees between peers on every confirmed boundary it presented", function()
        local state = harness("2v2")
        run(state, 48, 4)
        -- The lowest confirmation ceiling in the session: every peer has full
        -- authority up to it, so a disagreement there is a real divergence
        -- rather than one peer still holding a prediction.
        local boundary = nil
        for _, peer in ipairs(state.peers) do
            local confirmed = match_driver.diagnostics(peer.driver).confirmed_output_tick
            boundary = boundary and math.min(boundary, confirmed) or confirmed
        end
        boundary = assert(boundary)
        t.is_true(boundary >= 0, "the run must confirm at least one boundary")
        local expected = nil
        for index, peer in ipairs(state.peers) do
            local lookup = match_driver.snapshot(peer.driver, boundary)
            t.is_true(lookup.status == "present" or lookup.status == "retained")
            local hash = match_snapshot.hash(assert(lookup.snapshot))
            expected = expected or hash
            t.eq(hash, expected, ("peer %d disagrees at a confirmed boundary"):format(index))
        end
    end)
end)
