-- Reconciliation smoke for the online presentation timeline.
--
-- Real drivers, a real in-process star, real arrivals. What is asserted is the
-- presentation contract the renderer depends on: one stable identity per
-- confirmed event, published exactly once, and a correction that replaces the
-- whole speculative tail rather than layering a second copy over it.
--
-- The individual combat correction cases (wind-up, guard, contact, projectile
-- flight, stagger, ball spill, immunity expiry) are pinned twice, against the
-- shared boundary zeroes in `spec/support/online_combat_phases.lua`. Bot fills
-- materialize `gameplay_ai/combat/v1` (#112), so those states are reachable and
-- pinning them no longer pins an absence.
--
--   * `spec/game/online_match_driver_spec.lua` pins the driver-layer claim: peers
--     converge on one snapshot hash through a correction taken in each phase.
--   * The second half of this file pins the *feedback* claim, which is a
--     different property -- a driver that converges perfectly can still leave a
--     swing drawn twice or a hit drawn that never happened.

local t = require("spec.support.runner")
local combat_phases = require("spec.support.online_combat_phases")
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
---@field revoked table<string, boolean> -- Ids the correction erased outright.
---@field latest table<string, any> -- Event id -> the newest payload the timeline holds.
---@field stale integer -- Confirmations that published a payload a correction had replaced.
---@field revoked_combat integer
---@field replaced_combat integer
---@field added_combat integer
---@field confirmed_combat integer
---@field corrections integer
---@field confirmed_ticks integer[]

---@class PresentationHarness
---@field session OnlineMatchFixtureSession
---@field peers PresentationPeer[]

---@class PresentationHarnessOptions
---@field duration_ticks integer?
---@field initial_snapshot MatchSnapshot? -- Boundary zero for every peer.

---@param value any
---@param other any
---@return boolean
local function equal_value(value, other)
    if type(value) ~= type(other) then
        return false
    end
    if type(value) ~= "table" then
        return value == other
    end
    for key, child in pairs(value) do
        if not equal_value(child, other[key]) then
            return false
        end
    end
    for key in pairs(other) do
        if value[key] == nil then
            return false
        end
    end
    return true
end

---@param mode SessionMatchMode
---@param options PresentationHarnessOptions?
---@return PresentationHarness
local function harness(mode, options)
    options = options or {}
    local session = fixture.session(mode, nil, options.duration_ticks)
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
        -- Every peer is handed the *same* boundary zero. A differing one is a
        -- desync fixture, not a correction fixture.
        local initial_snapshot = options.initial_snapshot or request.initial_snapshot
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
                initial_snapshot = initial_snapshot,
            }),
            presentation = match_presentation.new(initial_snapshot, request.first_input_tick),
            confirmed = {},
            revoked = {},
            latest = {},
            stale = 0,
            revoked_combat = 0,
            replaced_combat = 0,
            added_combat = 0,
            confirmed_combat = 0,
            corrections = 0,
            confirmed_ticks = {},
        }
    end
    return { session = session, peers = peers }
end

---@param domain string
---@return boolean
local function is_combat(domain)
    return domain:sub(1, 7) == "combat/"
end

-- Two different things a correction does to speculative feedback, kept apart
-- because the contract for each is different.
--
--   *Revoked* -- the id is gone from the corrected timeline. The cue never
--   happened, so it must never be confirmed. That is the `add`/`revoke` half of
--   the stable speculative consumer contract.
--
--   *Replaced* -- the id survives with a different payload
--   (`sim.rollback_events` derives identity from causality, so `before.id ==
--   after.id`). The cue happened but not the way it was drawn, so the
--   confirmation owes the *corrected* payload. Folding this into "revoked", as
--   the earlier shape of this spec did, would both under-test the replacement
--   and produce a false failure the moment a replaced event was confirmed.
---@param peer PresentationPeer
---@param batch RollbackPlayableLabBatch
local function record(peer, batch)
    peer.corrections = peer.corrections + #batch.corrections
    for _, diff in ipairs(batch.event_diffs) do
        for _, event in ipairs(diff.added) do
            peer.latest[event.id] = event.payload
            if is_combat(event.domain) then
                peer.added_combat = peer.added_combat + 1
            end
        end
        for _, event in ipairs(diff.revoked) do
            peer.revoked[event.id] = true
            peer.latest[event.id] = nil
            if is_combat(event.domain) then
                peer.revoked_combat = peer.revoked_combat + 1
            end
        end
        for _, replacement in ipairs(diff.replaced) do
            peer.latest[replacement.after.id] = replacement.after.payload
            if is_combat(replacement.after.domain) then
                peer.replaced_combat = peer.replaced_combat + 1
            end
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
                if is_combat(event.domain) then
                    peer.confirmed_combat = peer.confirmed_combat + 1
                end
                local latest = peer.latest[event.id]
                if latest ~= nil and not equal_value(latest, event.payload) then
                    peer.stale = peer.stale + 1
                end
            end
        end
    end
end

---@class PresentationRunOptions
---@field period integer? -- Deliver every `period` steps; nil delivers every step.
---@field sample (fun(step: integer, index: integer): InputSample)? -- Zero-based step.
---@field on_batch (fun(peer: PresentationPeer, index: integer, batch: RollbackPlayableLabBatch))?

---@param state PresentationHarness
---@param steps integer
---@param options PresentationRunOptions?
local function run(state, steps, options)
    options = options or {}
    for step = 1, steps do
        for index, peer in ipairs(state.peers) do
            local sample = options.sample and options.sample(step - 1, index)
                or input_frame.neutral_sample()
            local driver_batch = match_driver.advance(peer.driver, sample)
            local batch = match_presentation.consume(peer.presentation, peer.driver, driver_batch)
            record(peer, batch)
            -- Called inside the step on purpose: a caller that wants to look at
            -- the boundary a correction landed on has to do it now, before the
            -- driver evicts it.
            if options.on_batch then
                options.on_batch(peer, index, batch)
            end
        end
        if options.period == nil or step % options.period == 0 then
            state.session.host_transport:pump()
        end
    end
end

-- Every confirmed event published exactly once, in contiguous tick order, and
-- carrying the payload the last correction left behind.
---@param state PresentationHarness
local function assert_published_once(state)
    for index, peer in ipairs(state.peers) do
        for id, count in pairs(peer.confirmed) do
            t.eq(count, 1, ("peer %d published %s more than once"):format(index, id))
        end
        for id in pairs(peer.revoked) do
            t.is_true(
                peer.confirmed[id] == nil,
                ("peer %d confirmed a revoked event %s"):format(index, id)
            )
        end
        t.eq(
            peer.stale,
            0,
            ("peer %d published a payload a correction had already replaced"):format(index)
        )
        for tick_index = 2, #peer.confirmed_ticks do
            t.eq(
                peer.confirmed_ticks[tick_index],
                peer.confirmed_ticks[tick_index - 1] + 1,
                "confirmed steps must be contiguous"
            )
        end
    end
end

-- The lowest confirmation ceiling in the session: every peer has full authority
-- up to it, so a disagreement there is a real divergence rather than one peer
-- still holding a prediction.
---@param state PresentationHarness
---@return integer boundary
local function assert_confirmed_agreement(state)
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
    return boundary
end

t.describe("online match presentation", function()
    t.it("publishes each confirmed event exactly once under clean delivery", function()
        local state = harness("2v2")
        run(state, 40)
        assert_published_once(state)
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
        run(state, 48, { period = 6 })
        local corrected = 0
        for _, peer in ipairs(state.peers) do
            corrected = corrected + peer.corrections
        end
        assert_published_once(state)
        t.is_true(corrected > 0, "bursty delivery must produce at least one correction")
    end)

    t.it("agrees between peers on every confirmed boundary it presented", function()
        local state = harness("2v2")
        run(state, 48, { period = 4 })
        assert_confirmed_agreement(state)
    end)
end)

-- The same seven combat correction phases #166 pinned at the driver layer, taken
-- one layer up.
--
-- The driver-level claim is that peers converge on one snapshot hash through a
-- correction in each phase. This is a different property, and it is this issue's
-- own: *corrected-away feedback is revoked, and feedback that survives
-- confirmation publishes exactly once*. A screen consumes the timeline, not the
-- hash, so a driver that converges perfectly can still leave a swing drawn twice
-- or a hit drawn that never happened.
--
-- Each case rigs the phase's boundary zero, bursts delivery until real
-- corrections happen, checks that a correction really did resimulate a tick
-- *in* that phase -- against the boundary the rollback landed on and that tick's
-- own combat events, both read inside the step before the driver evicts them --
-- and then asserts the presentation contract across the whole run.
--
-- Delivery here is every 12th step rather than the scenario's own
-- `deliver_period`, and every run is 240 steps rather than the scenario's own
-- budget. The helper owns the fixture and the predicates; the caller owns its
-- delivery pattern, and this one is chosen from the other end:
--
--   * a shorter burst corrects too little to rewrite a cue at all, and past
--     roughly 16 the confirmation ceiling falls behind `sim.rollback_events`'
--     unconfirmed window, so the timeline gives up before the phase arrives;
--   * and because these bursts are longer than the driver spec's, every phase --
--     including the three the helper budgets 480 or 600 steps for -- is reached
--     inside 240. Nothing is assumed about that: `phase_ticks > 0` below is what
--     enforces it, so a fixture retune that stops reaching a phase inside the
--     budget fails here loudly rather than quietly asserting nothing.
local PHASE_DELIVER_PERIOD = 12
local PHASE_STEPS = 240

---@param state PresentationHarness
---@param phase_id OnlineCombatPhaseId
---@return integer[] observed -- Per peer: corrected ticks that ran through the phase.
local function run_phase(state, phase_id)
    ---@type integer[]
    local observed = {}
    for index = 1, #state.peers do
        observed[index] = 0
    end
    run(state, PHASE_STEPS, {
        period = PHASE_DELIVER_PERIOD,
        sample = function(step, index)
            return combat_phases.live_sample(phase_id, step, index)
        end,
        on_batch = function(peer, index, batch)
            local first = peer.request.first_input_tick
            -- `batch.outputs` mixes the ticks a correction re-derived with the
            -- ordinary forward tick the driver appends on nearly every call, and
            -- `RollbackTickOutput` does not distinguish them. This never reads a
            -- forward tick as a corrected one, because the lookup below is bounded
            -- by the correction's own reported interval and
            -- `match_presentation.consume` can only correct at or below the tick
            -- already applied before the batch -- strictly under the forward one.
            -- The assertion is here so that stays true by construction rather
            -- than by argument: a repeated tick would silently replace an entry.
            ---@type table<integer, RollbackTickOutput>
            local by_tick = {}
            for _, output in ipairs(batch.outputs) do
                assert(by_tick[output.tick] == nil, "one presentation batch reported a tick twice")
                by_tick[output.tick] = output
            end
            for _, correction in ipairs(batch.corrections) do
                for tick = correction.corrected_from_tick, correction.corrected_through_tick do
                    local before = match_driver.snapshot(peer.driver, tick + first).snapshot
                    local after = match_driver.snapshot(peer.driver, tick + 1 + first).snapshot
                    local output = by_tick[tick]
                    if
                        before ~= nil
                        and after ~= nil
                        and combat_phases.observed(
                            phase_id,
                            before,
                            after,
                            output and output.combat_events or {}
                        )
                    then
                        observed[index] = observed[index] + 1
                    end
                end
            end
        end,
    })
    return observed
end

t.describe("online match presentation combat phases", function()
    for _, phase_id in ipairs(combat_phases.PHASES) do
        t.it(("keeps feedback honest through a correction during %s"):format(phase_id), function()
            local state = harness("1v1", {
                initial_snapshot = combat_phases.boundary_zero(phase_id),
                duration_ticks = combat_phases.DURATION_SECONDS * 60,
            })
            local observed = run_phase(state, phase_id)

            local phase_ticks, corrections, replaced, added = 0, 0, 0, 0
            for index, peer in ipairs(state.peers) do
                t.eq(
                    match_presentation.status(peer.presentation),
                    "active",
                    ("peer %d's timeline gave up during %s"):format(index, phase_id)
                )
                phase_ticks = phase_ticks + observed[index]
                corrections = corrections + peer.corrections
                replaced = replaced + peer.replaced_combat
                added = added + peer.added_combat
            end
            t.is_true(corrections > 0, ("the %s burst never corrected anyone"):format(phase_id))
            t.is_true(
                phase_ticks > 0,
                ("no correction ever resimulated a %s tick"):format(phase_id)
            )
            -- The correction did not merely happen near the phase: it rewrote or
            -- introduced combat cues that presentation then had to reconcile. A
            -- run where the corrected tail produced byte-identical feedback would
            -- satisfy every assertion below for free.
            t.is_true(
                replaced + added > 0,
                ("no combat cue was rewritten or introduced during %s"):format(phase_id)
            )
            assert_published_once(state)
            assert_confirmed_agreement(state)
        end)
    end

    -- Revoking a *combat* cue is the rare half of the contract, and it has to be
    -- sought out rather than waited for. `sim.rollback_input_history` predicts a
    -- missing row by repeating its held bits with `edges = 0`, and every combat
    -- encounter is opened by a press edge, so no peer ever speculates a commit
    -- that later has to be taken back. What can be revoked is a cue *derived*
    -- from an already-open encounter -- the contact it lands, the forced state it
    -- inflicts, the ball it spills -- when the corrected geometry resolves the
    -- encounter differently.
    --
    -- The unarmed scrum is where that happens: eight bodies inside one 30 px
    -- reach of each other, so a corrected pixel is the difference between a
    -- contact and a miss. Without this case the "a revoked cue is never
    -- confirmed" check in `assert_published_once` would be vacuous, and it is
    -- asserted here rather than folded into the `contact` phase case above so
    -- that a future retune of that fixture cannot quietly make it vacuous again.
    t.it("never publishes a combat cue a correction took away", function()
        local state = harness("1v1", {
            initial_snapshot = combat_phases.boundary_zero("contact"),
            duration_ticks = combat_phases.DURATION_SECONDS * 60,
        })
        run_phase(state, "contact")
        local revoked = 0
        for _, peer in ipairs(state.peers) do
            revoked = revoked + peer.revoked_combat
        end
        t.is_true(revoked > 0, "the scrum never revoked a speculative combat cue")
        assert_published_once(state)
        assert_confirmed_agreement(state)
    end)

    -- The rest of the criterion's sweep: the same exactly-once contract has to
    -- hold across the lifecycle rows too, not only the combat ones, and it has to
    -- survive the end of the match rather than being read mid-run. Boundary zero
    -- is a combat-active scrum with a four-second clock, so kickoff, goals, and
    -- full time land in a timeline that is being corrected throughout.
    t.it("publishes the lifecycle exactly once through full time", function()
        local state = harness("1v1", {
            initial_snapshot = combat_phases.boundary_zero("contact", 4),
            duration_ticks = 4 * 60,
        })
        run(state, 4 * 60 + 90, { period = 6 })
        local full_time, combat_rows = 0, 0
        for index, peer in ipairs(state.peers) do
            t.eq(match_driver.status(peer.driver), "completed", ("peer %d"):format(index))
            t.eq(match_presentation.status(peer.presentation), "active")
            combat_rows = combat_rows + peer.confirmed_combat
            for id in pairs(peer.confirmed) do
                if id:find("lifecycle/full_time", 1, true) then
                    full_time = full_time + 1
                end
            end
            t.eq(
                full_time,
                index,
                ("peer %d published full time %d times"):format(index, full_time)
            )
        end
        t.is_true(combat_rows > 0, "a combat-active run confirmed no combat feedback at all")
        assert_published_once(state)
        assert_confirmed_agreement(state)
    end)
end)
