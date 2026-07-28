-- OMP-3 online match driver.
--
-- One instance drives one peer's match: it polls the star transport, authors the
-- rows it owns, sequences host authority, feeds real arrivals into the OMP-2
-- rollback session, advances the fixed 60 Hz clock, recomputes the live-slot
-- timeline, publishes boundary hashes, and ends with one typed terminal status.
--
-- It is the only place in the online stack that touches both a transport and a
-- rollback session. `sim/`, `data/`, and `core/` stay free of transport,
-- browser, and LOVE dependencies: everything WebRTC-shaped enters through the
-- injected `StarTransportAdapter`.
--
-- # Clocks
--
-- Three clocks are deliberately distinct:
--
--  * the **driver step** `T`, one per `advance` call, one per 60 Hz frame;
--  * the **transport tick** on the wire, `T + DELAY`, which exists only so the
--    host's own bundles can spend the versioned three-tick fairness delay in
--    the collector before the first simulated tick; and
--  * the **input tick** `first_input_tick + T`, the simulation authority tick
--    stepped during driver step `T`.
--
-- A sample taken during driver step `T` is authority for input tick
-- `first_input_tick + T + DELAY`. That is the documented input delay, and the
-- host pays it exactly like every guest: its own bundles are queued in the
-- collector and are not readable by `canonical_host_batch` until three transport
-- ticks later, so the host cannot bypass canonical sequencing.
--
-- # Authorship
--
-- Authorship is a frozen partition of the eight canonical slots, so no two peers
-- can ever author the same `(slot, input tick)`:
--
--  * a peer authors **every slot in its frozen owned set** — the human sample on
--    its control slot, deterministic AI rows on the rest;
--  * the host additionally authors every declared bot fill.
--
-- Validation is therefore set membership, not equality: a peer may author any
-- slot it owns and nothing outside it.
--
-- # The live slot
--
-- `live(N)` is the slot a human is controlling at input tick `N`. It is a pure
-- function of the frozen owned set, the canonical input stream, and the
-- deterministic simulation:
--
--     live(first)  = freeze.live
--     live(N + 1)  = coordinator.next_live_slot(owned, live(N), transition)
--
-- where the transition's `switch` edge is read from the *effective* canonical
-- row of `control_slot(N)` at tick `N` — the exact row `sim.match` consumed, and
-- the one the human's bits are actually on — and
-- `carrier`/`winner`/`ranked` come from the boundary `N + 1` simulation state
-- through `game.online.live_slot`. Nothing local reaches it. A rollback that
-- replaces ticks `[a, b]` replaces `live(a + 1 .. b + 1)` with it, so the
-- timeline is corrected by exactly the same mechanism as the simulation.
--
-- Because a sample is authority `DELAY` ticks after it is taken, the row that
-- carries a human's bits at input tick `N` is the one for `live(N - DELAY)`,
-- clamped to the opening live slot before the delay has elapsed. That
-- `control_slot` is itself a pure function of the same timeline, so every peer
-- computes it, not just the author. A correction can retroactively move
-- `live`, which can leave a human's bits on a slot the corrected timeline no
-- longer calls live for at most `DELAY` ticks. Every peer agrees on that
-- outcome, and the simulation is unaffected: it consumes the eight rows it is
-- given, whoever authored them.
--
-- # Full time settles before it completes
--
-- Reaching full time is not the same as agreeing on it. The last `DELAY` ticks
-- can still be unconfirmed at the moment the final tick is simulated, so a
-- driver that terminated there would report a final hash taken over predicted
-- authority. Under a burst straddling full time two peers stop at different
-- confirmation depths and disagree about a match in which nothing actually
-- diverged.
--
-- So full time opens a **settle phase** instead of terminating: the simulation
-- stops for good, and the driver keeps draining, applying, and re-publishing the
-- last authored redundancy window until `confirmed_output_tick + 1` reaches the
-- final boundary. Only then does it terminate `completed`, over a final state
-- that is a function of confirmed authority alone. Boundary hashes stop here:
-- from full time the acknowledged final hash is the cross-check, and a report
-- for a settling peer's boundary could reach one that has already moved on to
-- the result, where a hash report is no longer legal.
--
-- Settling is bounded twice, and it waits on nothing else. It ends after
-- `SETTLE_TIMEOUT_TICKS` further driver steps, or after `SETTLE_TIMEOUT_SECONDS`
-- of monotonic wall clock, whichever comes first, with the typed terminal
-- `settle_timeout`. It is deliberately *not* gated on hash agreement: the driver
-- cannot see another peer's hash, so waiting for one would be an unbounded wait
-- on a fact that never arrives. A disagreement reported through
-- `observe_checkpoint` while settling still terminates `hash_mismatch`, exactly
-- as it does while playing -- settling never swallows a divergence, it only
-- refuses to report a result over authority it has not confirmed.

local coordinator = require("game.online.coordinator")
local input_protocol = require("game.online.input_protocol")
local live_slot = require("game.online.live_slot")
local protocol = require("game.online.protocol")
local transport_contract = require("game.transport.contract")
local input_frame = require("sim.input_frame")
local match_snapshot = require("sim.match_snapshot")
local rollback_input_history = require("sim.rollback_input_history")
local rollback_session = require("sim.rollback_session")
local slot_input = require("sim.slot_input")

---@alias MatchDriverStatus
---| "active"
---| "completed"
---| "settle_timeout"
---| "confirmation_stalled"
---| "late_input"
---| "hash_mismatch"
---| "ownership_violation"
---| "authority_conflict"
---| "input_channel_failure"
---| "transport_lost"

---@class MatchDriverTerminal
---@field status MatchDriverStatus
---@field failure CoordinatorNetcodeFailure? -- What to report into the coordinator, if anything.
---@field detail string
---@field tick integer? -- Input tick the failure was attributed to.

---@class MatchDriverCheckpoint
---@field tick integer -- Confirmed boundary tick, in input-tick space.
---@field hash string
---@field live table<string, InputSlotId> -- Live slot per human at that boundary.

---@class MatchDriverOptions
---@field role SessionRole
---@field peer_id string
---@field freeze CoordinatorFreeze
---@field manifest SessionManifest
---@field transport StarTransportAdapter
---@field initial_snapshot MatchSnapshot -- Canonical slot-mode boundary zero.
---@field max_rollback_ticks integer?
---@field hash_interval_ticks integer?
---@field settle_timeout_ticks integer? -- Driver steps the settle phase may span.
---@field settle_timeout_seconds number? -- Monotonic seconds the settle phase may span.
---@field clock (fun(): number)? -- Monotonic seconds source; injected so the bound is testable.

---@class MatchDriverBatch
---@field step integer -- Driver step that produced this batch.
---@field input_tick integer -- Input tick simulated during this step.
---@field outputs RollbackTickOutput[]
---@field reconciliations integer -- Always at most one per driver step.
---@field applied_rows integer
---@field corrections integer
---@field rollbacks integer
---@field sent_packets integer
---@field checkpoints MatchDriverCheckpoint[]
---@field control TransportPeerMessage[] -- Control-channel traffic for the coordinator.
---@field live table<string, InputSlotId>
---@field status MatchDriverStatus

---@class MatchDriverDiagnostics
---@field role SessionRole
---@field peer_id string
---@field status MatchDriverStatus
---@field terminal MatchDriverTerminal?
---@field step integer
---@field transport_tick integer
---@field present_input_tick integer
---@field confirmed_input_tick integer -- Raw all-input authority confirmation.
---@field confirmed_output_tick integer -- Confirmation capped to simulated outputs.
---@field retained_floor_tick integer -- Oldest input tick the rollback window still holds.
---@field owned InputSlotId[]
---@field authored InputSlotId[]
---@field live table<string, InputSlotId>
---@field control_slot InputSlotId?
---@field rollback_count integer
---@field correction_count integer
---@field predicted_slot_samples integer
---@field max_rollback_depth integer
---@field late_input_tick integer?
---@field hash_mismatches integer
---@field checkpoint_count integer
---@field settling boolean -- Full time reached, final boundary not confirmed yet.
---@field settled boolean -- The final boundary was confirmed before the driver completed.
---@field full_time_boundary integer? -- Final boundary, in input-tick space.
---@field settle_steps integer -- Settle steps taken; zero when full time settled immediately.
---@field dropped_outbound integer
---@field dropped_inbound integer

---@class MatchDriverPending
---@field due integer -- Driver step on which the collector may read this packet.
---@field packet InputPacket
---@field envelope TransportMessage
---@field producer_id string

---@class MatchDriver
---@field _role SessionRole
---@field _peer_id string
---@field _freeze CoordinatorFreeze
---@field _manifest SessionManifest
---@field _manifest_id string
---@field _transport StarTransportAdapter
---@field _session RollbackSession
---@field _sources RollbackInputSource[]
---@field _producer SlotInputProducerState
---@field _owned InputSlotId[]
---@field _authored integer[] -- Canonical slot indexes this peer authors, ascending.
---@field _authored_set table<integer, boolean>
---@field _owned_set table<integer, boolean>
---@field _peer_slots table<string, integer[]> -- Peer producer id -> owned slot indexes.
---@field _humans string[] -- Human producer ids, in frozen assignment order.
---@field _bot_slots integer[] -- Host only: declared bot fills, ascending.
---@field _first integer
---@field _step integer
---@field _live table<integer, table<string, InputSlotId>>
---@field _carrier table<integer, InputSlotId?>
---@field _live_tick integer -- Highest input tick with a computed live map.
---@field _history table<integer, table<integer, InputSample>> -- Slot index -> tick -> authored sample.
---@field _sequences table<string, integer>
---@field _pending MatchDriverPending[] -- Host only: the delayed local collector path.
---@field _deferred InputPacketArrival[] -- Host only: arrivals held back by the row bound.
---@field _relay table<integer, InputPacketArrival> -- Host only: newest accepted bundle per slot.
---@field _peer_confirmed table<integer, integer> -- Host only: slot index -> that author's newest reported confirmed input tick.
---@field _host_confirmed integer? -- Guest only: the host's newest reported confirmed input tick.
---@field _checkpoints MatchDriverCheckpoint[]
---@field _checkpoint_by_tick table<integer, string>
---@field _hash_interval integer
---@field _next_checkpoint integer
---@field _hash_mismatches integer
---@field _status MatchDriverStatus
---@field _terminal MatchDriverTerminal?
---@field _late_input_tick integer?
---@field _authored_tick integer -- Highest input tick this peer has authored rows for.
---@field _settle_ticks integer
---@field _settle_seconds number
---@field _clock fun(): number
---@field _full_time_boundary integer? -- Final boundary; non-nil once full time is reached.
---@field _settle_steps integer -- Settle steps taken so far, capped by `_settle_ticks`.
---@field _settle_deadline number? -- Monotonic second the settle phase expires at.
---@field _settle_quiet_steps integer -- Consecutive settle steps with no inbound input traffic.
---@field _settled boolean

---@class OnlineMatchDriverModule
local match_driver = {}

match_driver.DELAY_TICKS = input_protocol.FAIRNESS_DELAY_TICKS
match_driver.DEFAULT_HASH_INTERVAL_TICKS = 30
match_driver.MAX_HASH_MISMATCHES = 3
match_driver.POLL_BATCH_LIMIT = transport_contract.MAX_QUEUE_LIMIT

-- How many contiguous input ticks one targeted repair covers. See `plan_repair`
-- for why this is a span rather than the single tick a stalled guest is blocked
-- on, and `input_protocol.HOST_WINDOW_ROWS` for the byte budget that pays for it.
match_driver.REPAIR_SPAN_TICKS = 4

-- The settle phase drains at most one second of further driver steps. The tail
-- it is waiting for is at most `DELAY_TICKS` of authority, already carried by
-- the seven-row redundancy window, and settling adds one further re-send of that
-- window per step: sixty of them is far past the point where a recoverable burst
-- has been recovered, and short enough that nobody stares at a dead peer. It is
-- a liveness bound, not a correctness one -- nothing is simulated while
-- settling, so the retained window stops sliding and waiting can never push the
-- tail out of it.
match_driver.SETTLE_TIMEOUT_TICKS = 60

-- The same bound in real time, for a caller whose frames are not arriving at
-- 60 Hz. A driver that is advanced slowly would otherwise spend far longer than
-- a second inside a bounded number of steps; at a healthy frame rate the tick
-- bound always expires first. Half the coordinator's start-acknowledgement
-- patience, which is the closest comparable wait in the session.
match_driver.SETTLE_TIMEOUT_SECONDS = 2

-- Consecutive silent settle steps after which the host stops relaying.
--
-- The host is the star's only relay: a guest that is still missing a tail row
-- can only ever receive it as part of a canonical host batch. A settling guest
-- re-publishes its redundancy window on *every* settle step, so inbound input
-- traffic is exactly the signal that somebody still needs the relay, and silence
-- is the signal that nobody does. Four steps is the fairness delay plus one --
-- the longest a link that is still being used can legitimately go quiet -- so a
-- clean match costs the host four extra steps (67 ms) and a match with a
-- straggler keeps the relay alive for as long as the straggler keeps asking,
-- bounded by the settle phase's existing deadlines and by nothing new.
--
-- **This one is a heuristic, and it is the only bound in this phase that is.**
-- `SETTLE_TIMEOUT_TICKS` and `SETTLE_TIMEOUT_SECONDS` are hard: they end the
-- phase unconditionally, and nothing here can wait past them. This margin is
-- not of that class, and a future reader must not read it as though it were.
--
-- What "burst length plus one" actually covers is **one** worst-case burst. The
-- `stress` profile's maximum burst is three consecutive losses, so a straggler
-- that loses a whole burst of re-sends still breaks the silence on the fourth
-- step. What it does **not** cover is a worst-case burst immediately followed by
-- worst-case jitter: the next re-send after a three-loss burst can draw up to
-- three further ticks of positive delay, which pushes that straggler's silent
-- window past four steps and lets the host conclude "drained" while somebody is
-- still asking.
--
-- That compounding edge is real, low-probability, and does not appear anywhere
-- in the seed sweep this landed against. Its blast radius is bounded and is
-- deliberately not a new failure class: the straggler ends `settle_timeout`,
-- which is exactly the typed terminal it would have had before this relay
-- existed, just rarer. Widening the margin trades a longer whistle on every
-- clean match for a smaller tail here; making it a guarantee instead of a
-- heuristic needs the guest to be able to *say* it is still missing rows, which
-- is the same protocol addition #243 tracks.
match_driver.SETTLE_RELAY_QUIET_STEPS = match_driver.DELAY_TICKS + 1

---@param value any
---@return boolean
local function is_integer(value)
    return type(value) == "number"
        and value == value
        and value ~= math.huge
        and value ~= -math.huge
        and value == math.floor(value)
end

-- Monotonic seconds for the settle phase's wall-clock bound. LOVE's timer is the
-- honest source when there is one; the fallback keeps the driver constructible
-- in a bare Lua host, and every test that asserts on the bound injects its own.
-- Nothing simulated reads this: it can only end a settle phase early, never
-- change a tick.
---@return number
local function default_clock()
    if love ~= nil and love.timer ~= nil and love.timer.getTime ~= nil then
        return love.timer.getTime()
    end
    return os.time()
end

---@param sample InputSample
---@return InputSample
local function copy_sample(sample)
    return assert(input_frame.new_sample(sample))
end

---@param seed integer
---@param slot_index integer
---@return integer
local function derived_ai_seed(seed, slot_index)
    -- A non-live owned slot needs a bot stream just like a declared fill does,
    -- and every peer must be able to name it, so it is derived from the frozen
    -- seed rather than negotiated. Park-Miller states are 1..2^31-2.
    local mixed = (seed % 2147483629) * 31 + slot_index * 7919
    return 1 + (mixed % 2147483645)
end

-- ---------------------------------------------------------------------------
-- Terminal status
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@param status MatchDriverStatus
---@param detail string
---@param failure CoordinatorNetcodeFailure?
---@param tick integer?
local function terminate(driver, status, detail, failure, tick)
    if driver._status ~= "active" then
        return
    end
    driver._status = status
    driver._terminal = {
        status = status,
        failure = failure,
        detail = detail,
        tick = tick,
    }
end

---@param driver MatchDriver
---@return boolean
local function running(driver)
    return driver._status == "active"
end

-- ---------------------------------------------------------------------------
-- Tick spaces
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@param input_tick integer
---@return integer
local function session_tick(driver, input_tick)
    return input_tick - driver._first
end

---@param driver MatchDriver
---@param tick integer
---@return integer
local function to_input_tick(driver, tick)
    return tick + driver._first
end

---@param driver MatchDriver
---@return integer -- Present boundary, in input-tick space.
local function present_input_tick(driver)
    return to_input_tick(driver, rollback_session.diagnostics(driver._session).present_boundary)
end

-- Input-authority confirmation: the highest contiguous tick whose eight rows are
-- all authoritative. It says nothing about whether that tick has been simulated.
---@param driver MatchDriver
---@return integer
local function confirmed_input_tick(driver)
    local confirmed = rollback_session.diagnostics(driver._session).confirmed_tick
    if confirmed < 0 then
        return driver._first - 1
    end
    return to_input_tick(driver, confirmed)
end

-- Confirmation capped to the simulated-output ceiling. A sample is authority
-- `DELAY` ticks before it is consumed, so `confirmed_tick` routinely runs ahead
-- of the present boundary even under perfectly clean delivery. Any boundary that
-- keys a *snapshot* lookup must come from here instead, because
-- `confirmed_output_tick + 1 <= present_boundary` is exactly the guarantee that
-- the snapshot was captured. Using the raw confirmation aborts healthy matches
-- whenever a checkpoint lands on the race point.
---@param driver MatchDriver
---@return integer
local function confirmed_output_input_tick(driver)
    local confirmed = rollback_session.diagnostics(driver._session).confirmed_output_tick
    if confirmed < 0 then
        return driver._first - 1
    end
    return to_input_tick(driver, confirmed)
end

---@param driver MatchDriver
---@return integer
local function retained_floor_input_tick(driver)
    local diagnostics = rollback_session.diagnostics(driver._session)
    return to_input_tick(driver, diagnostics.input_history.oldest_retained_tick)
end

-- ---------------------------------------------------------------------------
-- Live-slot timeline
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@param live table<string, InputSlotId>
---@return table<string, InputSlotId>
local function copy_live_map(driver, live)
    local copied = {}
    for _, producer_id in ipairs(driver._humans) do
        copied[producer_id] = live[producer_id]
    end
    return copied
end

---@param driver MatchDriver
---@param boundary integer -- Input-tick space.
---@return MatchState
local function boundary_state(driver, boundary)
    local lookup = rollback_session.snapshot(driver._session, session_tick(driver, boundary))
    assert(
        lookup.status == "present" or lookup.status == "retained",
        "the live-slot timeline needs a retained boundary snapshot"
    )
    return assert(lookup.snapshot, "retained boundary snapshot is missing").state
end

-- A real `MatchState` — Vec2 fields and all — for the boundary this peer is
-- about to simulate. The deterministic AI needs the live shape, not a
-- snapshot's plain vector tables.
---@param driver MatchDriver
---@return MatchState
local function present_state(driver)
    return (match_snapshot.restore(rollback_session.current_snapshot(driver._session)))
end

-- Extend the live timeline through the boundary that has just been simulated.
-- `record` is the effective input record `sim.match` consumed at `tick`, which
-- is where the canonical `switch` edge is read from.
---@param driver MatchDriver
---@param tick integer -- Input tick that was simulated.
---@param record RollbackInputTickRecord
local function extend_live(driver, tick, record)
    local previous = assert(driver._live[tick], "the live-slot timeline has a gap")
    local after = boundary_state(driver, tick + 1)
    local carrier = live_slot.carrier(after)
    local previous_carrier = driver._carrier[tick]
    ---@type table<string, InputSlotId>
    local next_live = {}
    for _, producer_id in ipairs(driver._humans) do
        local current = assert(previous[producer_id], "a human has no live slot")
        -- The `switch` edge is read from the row the human's bits are actually
        -- on. Because a sample is authority `DELAY` ticks after it is taken,
        -- that is the control slot, not necessarily the live slot: reading the
        -- live slot's row would read an AI row whenever the two differ, and
        -- switching would silently stall.
        local slot_index =
            live_slot.slot_index(match_driver.control_slot(driver, producer_id, tick))
        local sample = assert(record.slots[slot_index], "effective frame row is missing").sample
        local transition = live_slot.transition(after, {
            switch = live_slot.switch_edge(sample),
            live = current,
            previous_carrier = previous_carrier,
        })
        next_live[producer_id] =
            coordinator.next_live_slot(driver._freeze.owned[producer_id], current, transition)
    end
    driver._live[tick + 1] = next_live
    driver._carrier[tick + 1] = carrier
    if tick + 1 > driver._live_tick then
        driver._live_tick = tick + 1
    end
end

-- The live-slot timeline is pruned `DELAY_TICKS` *below* the authority floor, not
-- to it.
--
-- `extend_live` re-derives the timeline for a corrected tick, and the switch edge
-- it reads comes from the human's *control* slot -- the live slot from
-- `DELAY_TICKS` earlier, because a sample is authority that long after it is
-- taken. So correcting the oldest retained tick reads the timeline entry
-- `DELAY_TICKS` beneath it, and pruning flush with the authority floor deletes
-- exactly that entry.
--
-- Before #243 nothing reached far enough down to notice: the open-loop
-- redundancy window only ever re-sent rows from the last seven ticks, so a
-- correction landed nowhere near the floor. The targeted repair delivers
-- authority from anywhere above the floor, including the floor itself, and turned
-- the latent gap into a hard `the live-slot timeline has no entry for this tick`
-- assertion on `4v4.stress`. The overlap is what the correction path always
-- needed; only its reachability is new.
---@param driver MatchDriver
---@param floor integer -- Oldest input tick still retained.
local function prune_live(driver, floor)
    local oldest = math.max(driver._first, floor - match_driver.DELAY_TICKS)
    for tick = driver._first, oldest - 1 do
        driver._live[tick] = nil
        driver._carrier[tick] = nil
    end
end

-- The slot that carries a human's authored bits at `input_tick`. The input
-- delay means it is the live slot from `DELAY` ticks earlier.
---@param driver MatchDriver
---@param producer_id string
---@param input_tick integer
---@return InputSlotId
function match_driver.control_slot(driver, producer_id, input_tick)
    local source = math.max(driver._first, input_tick - match_driver.DELAY_TICKS)
    local live = assert(driver._live[source], "the live-slot timeline has no entry for this tick")
    return assert(live[producer_id], "the producer owns no live slot")
end

---@param driver MatchDriver
---@param input_tick integer?
---@return table<string, InputSlotId>
function match_driver.live(driver, input_tick)
    local tick = input_tick or driver._live_tick
    local live = assert(driver._live[tick], "the live-slot timeline has no entry for this tick")
    return copy_live_map(driver, live)
end

-- ---------------------------------------------------------------------------
-- Authoring
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@param sender_id string
---@return integer
local function next_sequence(driver, sender_id)
    local sequence = driver._sequences[sender_id] or 0
    driver._sequences[sender_id] = sequence + 1
    return sequence
end

-- Authored samples are retained down to this peer's own rollback floor, not to
-- the seven ticks one bundle carries.
--
-- The extra history is what makes `settle_tail_tick` able to re-publish a window
-- the host is actually missing rather than only the newest one. Pruning flush
-- with the bundle meant that once a tick fell out of the newest window it could
-- never be re-authored by anyone: the guest that wrote it had thrown it away, and
-- it existed nowhere else if the host had not received it. `ROLLBACK_WINDOW_TICKS`
-- is the right floor because authority older than that cannot be placed in any
-- peer's window anyway, so retaining more would buy nothing and retaining less
-- discards rows that are still placeable.
---@param driver MatchDriver
---@param slot_index integer
---@param tick integer
---@param sample InputSample
local function record_authored(driver, slot_index, tick, sample)
    local slot_history = driver._history[slot_index]
    slot_history[tick] = copy_sample(sample)
    local oldest = tick - rollback_input_history.ROLLBACK_WINDOW_TICKS - 1
    if oldest >= driver._first then
        slot_history[oldest] = nil
    end
end

-- The redundant bundle for one slot: the current row plus exactly the retained
-- prior rows, oldest first, as `input_protocol.validate` requires.
---@param driver MatchDriver
---@param slot_index integer
---@param tick integer
---@return InputAuthorityRow[]
local function redundant_rows(driver, slot_index, tick)
    local slot_history = driver._history[slot_index]
    local first = math.max(driver._first, tick - input_protocol.HISTORY_ROWS)
    local rows = {}
    for row_tick = first, tick do
        local sample = assert(slot_history[row_tick], "authored redundancy row is missing")
        rows[#rows + 1] = { tick = row_tick, slot_index = slot_index, sample = copy_sample(sample) }
    end
    return rows
end

-- Deterministic AI rows for every slot this peer authors that is not its
-- control slot. The bot stream advances once per authored tick per slot
-- regardless of which slot is live, so the stream is a function of the tick
-- count alone and never of the live-slot history.
--
-- `geometry` lets a caller that already restored the present `MatchState` reuse
-- it. Deriving one costs a `capture_owned` plus a `restore` — two full deep
-- copies on top of the one the session already performs per tick — so it is
-- derived lazily and only when this peer actually authors an AI row.
--
-- When the peer authors exactly one slot it is, necessarily, the control slot:
-- `control_slot` always returns a member of the frozen owned set, and a
-- one-element authored set means a one-element owned set and no bot fills. The
-- materialized frame would then be discarded unread, so it is never built. That
-- is the common case — every seat in a full 4v4 lobby, and every guest in any
-- lobby with no declared fills.
---@param driver MatchDriver
---@param input_tick integer
---@param human_sample InputSample?
---@param geometry MatchState? -- A present-boundary state the caller already holds.
---@return table<integer, InputSample>
local function materialize_authored(driver, input_tick, human_sample, geometry)
    local control = nil
    if #driver._owned > 0 then
        control =
            live_slot.slot_index(match_driver.control_slot(driver, driver._peer_id, input_tick))
    end
    local human = copy_sample(human_sample or input_frame.neutral_sample())
    if #driver._authored == 1 and driver._authored[1] == control then
        return { [control] = human }
    end

    geometry = geometry or present_state(driver)
    local slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        slots[index] = input_frame.neutral_sample()
    end
    local base = assert(input_frame.new(geometry.input_tick, slots))
    local frame = slot_input.materialize(driver._producer, geometry, base)
    ---@type table<integer, InputSample>
    local authored = {}
    for _, slot_index in ipairs(driver._authored) do
        if slot_index == control then
            authored[slot_index] = human
        else
            authored[slot_index] = copy_sample(frame.slots[slot_index])
        end
    end
    return authored
end

---@param driver MatchDriver
---@param slot_index integer
---@return string
local function producer_id_for(driver, slot_index)
    local producer = assert(driver._freeze.assignments[slot_index], "canonical slot is unassigned")
    return producer.producer_id
end

---@param driver MatchDriver
---@param slot_index integer
---@param tick integer
---@param transport_tick integer
---@return InputPacket, TransportMessage
local function build_packet(driver, slot_index, tick, transport_tick)
    local sender_id = producer_id_for(driver, slot_index)
    local sequence = next_sequence(driver, sender_id)
    local packet = assert(input_protocol.new_guest({
        session_id = driver._manifest.session_id,
        manifest_id = driver._manifest_id,
        sender_id = sender_id,
        sequence = sequence,
        transport_tick = transport_tick,
        first_input_tick = driver._first,
        -- The confirmation feedback #243 added. Every bundle a peer already sends
        -- every tick now says how far its own confirmation has actually reached,
        -- so the host stops fanning out blind. It costs one header field and no
        -- round trip: the sender never waits on it and never asks for anything.
        confirmed_span = input_protocol.confirmed_span(driver._first, confirmed_input_tick(driver)),
        rows = redundant_rows(driver, slot_index, tick),
    }))
    local wire = assert(input_protocol.encode(packet))
    local envelope = assert(transport_contract.new({
        type = "input",
        seq = packet.sequence,
        tick = packet.transport_tick,
        payload = wire,
    }))
    return packet, envelope
end

-- ---------------------------------------------------------------------------
-- Applying authority
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@param rows InputAuthorityRow[]
---@param batch MatchDriverBatch
---@param arrival boolean? -- True for authority that arrived over the transport.
local function apply_rows(driver, rows, batch, arrival)
    if #rows == 0 then
        return
    end
    local arrivals = {}
    -- Authority at or below the confirmed boundary is already final. The
    -- redundant six-row history re-sends it every tick, and replaying it must
    -- not re-enter the window as a late correction, so it is skipped here.
    --
    -- Everything else goes to `rollback_input_history`, which owns the retained
    -- floor. There is deliberately no driver-level floor pre-check: it would be
    -- provably redundant. The history rejects `tick < oldest_retained_tick` with
    -- `outside_window`, this driver reads the same value, and every row that
    -- survives the skip above and is below the floor is also the *lowest* tick
    -- in `arrivals` (valid rows are all at or above the floor). So a pre-check
    -- and the history's own rejection would terminate on the same batch and
    -- attribute the same causal tick. One owner of the floor is better than two.
    local confirmed = confirmed_input_tick(driver)
    for _, row in ipairs(rows) do
        if row.tick > confirmed then
            arrivals[#arrivals + 1] = {
                tick = session_tick(driver, row.tick),
                slot_index = row.slot_index,
                sample = copy_sample(row.sample),
            }
        end
    end
    if #arrivals == 0 then
        return
    end
    -- A peer's own rows are always for a tick it has not simulated yet, and the
    -- redundant re-sends behind them are byte-identical duplicates, so they can
    -- never open a divergence. They are inserted without a reconciliation pass,
    -- which is what makes "one arrival batch, one reconciliation" literally
    -- true rather than "one, plus a no-op". The guarantee is checked rather
    -- than assumed: if a local insert ever did report a divergence, the
    -- reconciliation still runs below.
    local result, err, code
    if not arrival then
        local accepted, insert_err, insert_code =
            rollback_session.add_authoritative_batch(driver._session, arrivals)
        if accepted ~= nil and accepted.earliest_divergence == nil then
            batch.applied_rows = batch.applied_rows + accepted.inserted
            return
        end
        if accepted ~= nil then
            result = {
                arrival = accepted,
                reconciliation = rollback_session.reconcile(driver._session),
            }
        else
            err, code = insert_err, insert_code
        end
    else
        result, err, code = rollback_session.apply_authoritative_batch(driver._session, arrivals)
    end
    if result == nil then
        if code == "outside_window" then
            driver._late_input_tick = driver._late_input_tick
                or to_input_tick(driver, arrivals[1].tick)
            terminate(
                driver,
                "late_input",
                err or "authority arrived outside the retained rollback window",
                "late_input",
                driver._late_input_tick
            )
            return
        end
        if code == "conflicting_authoritative" then
            terminate(
                driver,
                "authority_conflict",
                err or "canonical authority conflicted with retained history",
                "input_channel"
            )
            return
        end
        terminate(
            driver,
            "input_channel_failure",
            err or "canonical authority was rejected",
            "input_channel"
        )
        return
    end
    if arrival then
        batch.reconciliations = batch.reconciliations + 1
    end
    batch.applied_rows = batch.applied_rows + result.arrival.inserted
    batch.corrections = batch.corrections + result.arrival.corrections
    local reconciliation = result.reconciliation
    if reconciliation.status == "late_input_unrecoverable" then
        driver._late_input_tick = driver._late_input_tick
            or (reconciliation.causal_tick and to_input_tick(driver, reconciliation.causal_tick))
        terminate(
            driver,
            "late_input",
            "the rollback window overflowed while reconciling",
            "late_input",
            driver._late_input_tick
        )
        return
    end
    if not reconciliation.changed then
        return
    end
    batch.rollbacks = batch.rollbacks + 1
    for _, output in ipairs(reconciliation.corrected_outputs) do
        local tick = to_input_tick(driver, output.tick)
        extend_live(driver, tick, output.input)
        batch.outputs[#batch.outputs + 1] = output
    end
end

-- ---------------------------------------------------------------------------
-- Transport
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
local function drain_events(driver)
    while running(driver) do
        local event = driver._transport:poll_event()
        if event == nil then
            return
        end
        if event.kind == "star_error" or event.kind == "peer_error" then
            if event.code == "overflow" or event.code == "backpressure" then
                terminate(
                    driver,
                    "input_channel_failure",
                    event.message or "the input channel exceeded its queue budget",
                    "input_channel"
                )
            else
                terminate(
                    driver,
                    "input_channel_failure",
                    event.message or "the transport reported a terminal error",
                    "input_channel"
                )
            end
        elseif event.kind == "star_state" then
            if
                event.state == "closed"
                or event.state == "error"
                or event.state == "disconnected"
            then
                terminate(driver, "transport_lost", "the star transport is no longer connected")
            end
        elseif event.kind == "peer_state" then
            -- Every link is frozen at the countdown, so losing one after the
            -- freeze ends the match rather than shrinking the roster.
            if
                event.state == "closed"
                or event.state == "error"
                or event.state == "disconnected"
            then
                terminate(driver, "transport_lost", "a frozen peer link was lost")
            end
        end
    end
end

-- Every input envelope available on one transport tick, in the transport's own
-- deterministic drain order. Order is recorded, never used as authority: rows
-- are unioned and canonically sorted before a single reconciliation.
--
-- One transport carries both channels, and the reliable control channel belongs
-- to the session coordinator, not here. Draining it and discarding it would eat
-- the coordinator's traffic, so anything that is not an input envelope is handed
-- back on the batch for the owner to dispatch.
---@param driver MatchDriver
---@param batch MatchDriverBatch
---@return TransportPeerMessage[]
local function poll_input(driver, batch)
    local polled = driver._transport:poll_batch(match_driver.POLL_BATCH_LIMIT)
    local messages = {}
    for _, entry in ipairs(polled) do
        if entry.channel == "input" then
            messages[#messages + 1] = entry
        else
            batch.control[#batch.control + 1] = entry
        end
    end
    return messages
end

-- ---------------------------------------------------------------------------
-- Host collection
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@param messages TransportPeerMessage[]
---@param transport_tick integer
---@return InputPacketArrival[]?
local function collect_arrivals(driver, messages, transport_tick)
    ---@type InputPacketArrival[]
    local arrivals = {}
    for _, entry in ipairs(messages) do
        local packet, err, code = input_protocol.decode(entry.message.payload, {
            session_id = driver._manifest.session_id,
            manifest_id = driver._manifest_id,
            sender_id = entry.peer_id,
        })
        if packet == nil then
            terminate(
                driver,
                code == "ownership_mismatch" and "ownership_violation" or "input_channel_failure",
                err or "a guest input bundle failed to decode",
                "input_channel"
            )
            return nil
        end
        local slot_index = packet.rows[1].slot_index
        local owned = driver._peer_slots[entry.peer_id]
        local permitted = false
        for _, index in ipairs(owned or {}) do
            if index == slot_index then
                permitted = true
            end
        end
        if not permitted then
            terminate(
                driver,
                "ownership_violation",
                "a peer authored a slot outside its frozen owned set",
                "input_channel"
            )
            return nil
        end
        arrivals[#arrivals + 1] = {
            packet = packet,
            envelope = entry.message,
            arrival_tick = transport_tick,
            transport_peer_id = entry.peer_id,
        }
    end
    -- The host's own human and bot bundles enter through the same collector and
    -- only become readable once they have spent the versioned fairness delay.
    local remaining = {}
    for _, pending in ipairs(driver._pending) do
        if pending.due <= driver._step then
            arrivals[#arrivals + 1] = {
                packet = pending.packet,
                envelope = pending.envelope,
                arrival_tick = transport_tick,
                transport_peer_id = driver._peer_id,
            }
        else
            remaining[#remaining + 1] = pending
        end
    end
    driver._pending = remaining
    return arrivals
end

-- One host batch is bounded at `MAX_HOST_ROWS` distinct `(input tick, slot)`
-- rows, which is exactly eight slots times the seven-row redundancy window. A
-- delivery burst wider than that window cannot fit in one batch, so the excess
-- is carried to the next transport tick rather than dropped: dropping it would
-- strand authority the peers still need to confirm, and splitting it into two
-- batches on one tick would break the one-batch-one-reconciliation contract.
--
-- Selection is deterministic and independent of poll order: arrivals are sorted
-- by `(transport tick, sender, sequence)` first, then taken greedily.
---@param driver MatchDriver
---@return fun(left: InputPacketArrival, right: InputPacketArrival): boolean
local function arrival_order(driver)
    return function(left, right)
        -- The host's own collector path is never deferred. Its slots are the
        -- ones its own `materialize` demands be authoritative, so starving them
        -- would stall the host on its own input rather than on the network.
        local left_local = left.transport_peer_id == driver._peer_id
        local right_local = right.transport_peer_id == driver._peer_id
        if left_local ~= right_local then
            return left_local
        end
        if left.packet.transport_tick ~= right.packet.transport_tick then
            return left.packet.transport_tick < right.packet.transport_tick
        end
        if left.packet.sender_id ~= right.packet.sender_id then
            return left.packet.sender_id < right.packet.sender_id
        end
        return left.packet.sequence < right.packet.sequence
    end
end

---@param driver MatchDriver
---@param arrivals InputPacketArrival[]
---@param budget integer -- Rows this selection may consume, at most `MAX_HOST_ROWS`.
---@return InputPacketArrival[] selected
---@return table<string, boolean> keys -- Distinct `(tick, slot)` keys the selection already carries.
---@return integer count -- How many of `budget` those keys consume.
local function select_within_bound(driver, arrivals, budget)
    table.sort(arrivals, arrival_order(driver))
    ---@type InputPacketArrival[]
    local selected = {}
    ---@type InputPacketArrival[]
    local deferred = {}
    ---@type table<string, boolean>
    local keys = {}
    local count = 0
    for _, arrival in ipairs(arrivals) do
        local added = {}
        local additional = 0
        for _, row in ipairs(arrival.packet.rows) do
            local key = tostring(row.tick) .. ":" .. tostring(row.slot_index)
            if not keys[key] and not added[key] then
                added[key] = true
                additional = additional + 1
            end
        end
        if #deferred == 0 and count + additional <= budget then
            for _, row in ipairs(arrival.packet.rows) do
                keys[tostring(row.tick) .. ":" .. tostring(row.slot_index)] = true
            end
            count = count + additional
            selected[#selected + 1] = arrival
        else
            -- Once one arrival is held back, everything after it is held back
            -- too, so the carried-over stream keeps its canonical order.
            deferred[#deferred + 1] = arrival
        end
    end
    driver._deferred = deferred
    if #deferred > transport_contract.MAX_QUEUE_LIMIT then
        terminate(
            driver,
            "input_channel_failure",
            "the host authority backlog exceeded its bounded queue",
            "input_channel"
        )
    end
    return selected, keys, count
end

-- The newest bundle the host has accepted for each canonical slot.
--
-- A guest bundle is single-slot by construction, so `rows[1].slot_index` names
-- it. Newest wins, measured on the bundle's newest row rather than on its
-- arrival order, so a reordered old bundle can never displace a fresher one.
---@param driver MatchDriver
---@param selected InputPacketArrival[]
local function remember_relay(driver, selected)
    for _, arrival in ipairs(selected) do
        local rows = arrival.packet.rows
        local slot_index = rows[1].slot_index
        local newest = rows[#rows].tick
        local held = driver._relay[slot_index]
        if held == nil or newest > held.packet.rows[#held.packet.rows].tick then
            -- A copy: the deferred queue re-stamps `arrival_tick` in place, and
            -- the relay must not observe that mutation.
            driver._relay[slot_index] = {
                packet = arrival.packet,
                envelope = arrival.envelope,
                arrival_tick = arrival.arrival_tick,
                transport_peer_id = arrival.transport_peer_id,
            }
        end
    end
end

-- Record what each remote author says its own confirmation has reached.
--
-- The bundle every guest already sends every tick now carries `confirmed_span`
-- (#243), so this costs nothing beyond reading a field that already arrived. It
-- is recorded per canonical slot rather than per peer because the rest of the
-- host path is already keyed that way, and because a peer that owns several
-- slots reports one confirmation for all of them: the same value lands under
-- each of its slots and the minimum below is unchanged either way.
--
-- Newest wins, so a reordered old bundle can never walk a confirmation
-- backwards. The host's own collector path is skipped: its confirmation is not
-- feedback, it is the reference the feedback is measured against.
---@param driver MatchDriver
---@param arrivals InputPacketArrival[]
local function remember_confirmation(driver, arrivals)
    for _, arrival in ipairs(arrivals) do
        if arrival.transport_peer_id ~= driver._peer_id then
            local slot_index = arrival.packet.rows[1].slot_index
            local reported = input_protocol.confirmed_tick(arrival.packet)
            local held = driver._peer_confirmed[slot_index]
            if held == nil or reported > held then
                driver._peer_confirmed[slot_index] = reported
            end
        end
    end
end

-- The contiguous span of input ticks the most-behind guest is waiting on, as
-- rows read straight out of the host's own retained authority. `nil` means
-- nobody is behind far enough for a repair to be the right answer.
--
-- Every row here is authority the host already validated, canonicalized and
-- confirmed. Authorship is a frozen partition, so a re-sent row is
-- byte-identical to the one it repeats and can never open a divergence.
--
-- Three conditions gate the span, and each one is doing work:
--
--   * `tick <= host_confirmed`. The host must hold all eight rows of a tick as
--     authority, or it would be inventing some of them. A tick that is short
--     even one slot ends the span rather than being sent partial: a partial tick
--     cannot unblock confirmation, so it would be budget spent for nothing.
--   * `tick >= floor`. Below the host's own retained floor the rows are gone,
--     and a guest that far behind is already `confirmation_stalled` on its own
--     side. Repair cannot resurrect it and must not pretend to.
--   * `host_confirmed - needed > HISTORY_ROWS`. This is the gate that keeps the
--     mechanism off in a healthy match. Inside the redundancy window the
--     open-loop fan-out is still re-sending that tick every transport tick, so a
--     repair would spend budget on a row already in flight. Only once the window
--     has moved past it has blind redundancy provably failed for that guest.
--
-- ### Why a span and not the single tick the guest is stuck on
--
-- Confirmation advances only from `confirmed_tick + 1`, so a guest stuck at `C`
-- is blocked on tick `C + 1` alone, and repairing exactly that tick is the
-- minimal correct answer. It is also measurably too slow, and the reason is a
-- race the single-tick version loses:
--
--   * the guest's report reaches the host about three transport ticks after its
--     confirmation moves, and the repair takes several more to land, so the host
--     learns the *next* tick to repair roughly six steps later;
--   * the guest's retained floor climbs one tick per step regardless.
--
-- One tick recovered per six steps against a floor that eats one tick per step
-- is a race the guest loses from any real deficit. Captured on the default seed:
-- the single-tick repair walked the frontier 9 -> 10 and was still waiting for
-- the report when the floor passed 11, and the guest that stranded was missing
-- exactly two rows -- `(11, slot 5)` and `(12, slot 5)`. A span covers the ticks
-- behind the report latency instead of discovering them one round trip at a
-- time, which is what makes the mechanism outrun the floor rather than trail it.
--
-- `REPAIR_SPAN_TICKS` is that span, and its cost is why the row bound moved.
-- Eight slots times four ticks is 32 of the 72 rows, reserved *before* selection
-- rather than taken from what selection leaves over -- under `stress` the batch
-- saturates on arrivals alone, so a repair competing for leftovers is a repair
-- that never happens. The reservation is not a standing one: it is zero on every
-- tick where no guest is measurably stuck, which is every tick of a healthy
-- match.
---@param driver MatchDriver
---@param capacity_ticks integer -- Whole ticks the batch can still carry.
---@return InputAuthorityRow[]?
local function plan_repair(driver, capacity_ticks)
    local frontier = nil
    for slot_index = 1, input_frame.SLOT_COUNT do
        local confirmed = driver._peer_confirmed[slot_index]
        if confirmed ~= nil and (frontier == nil or confirmed < frontier) then
            frontier = confirmed
        end
    end
    if frontier == nil or capacity_ticks <= 0 then
        return nil
    end
    local needed = frontier + 1
    local host_confirmed = confirmed_input_tick(driver)
    if needed > host_confirmed or host_confirmed - needed <= input_protocol.HISTORY_ROWS then
        return nil
    end
    if needed < retained_floor_input_tick(driver) then
        return nil
    end
    ---@type InputAuthorityRow[]
    local rows = {}
    local span = math.min(match_driver.REPAIR_SPAN_TICKS, capacity_ticks)
    local last = math.min(needed + span - 1, host_confirmed)
    for tick = needed, last do
        ---@type InputAuthorityRow[]
        local complete = {}
        for slot_index = 1, input_frame.SLOT_COUNT do
            local sample = rollback_session.authoritative_sample(
                driver._session,
                session_tick(driver, tick),
                slot_index
            )
            if sample == nil then
                break
            end
            complete[slot_index] = {
                tick = tick,
                slot_index = slot_index,
                sample = copy_sample(sample),
            }
        end
        if #complete < input_frame.SLOT_COUNT then
            break
        end
        for _, row in ipairs(complete) do
            rows[#rows + 1] = row
        end
    end
    if #rows == 0 then
        return nil
    end
    return rows
end

-- Fill the batch out to the full eight-slot redundancy window the row bound is
-- sized for, from authority the host already holds.
--
-- `MAX_HOST_ROWS` is `SLOT_COUNT * RETAINED_ROWS` precisely because one host
-- batch is meant to carry a seven-tick window for every slot: that is the
-- redundancy every guest's confirmation depends on, because the host→guest leg
-- has no retransmission. Selecting only the bundles that arrived on *this*
-- transport tick silently spends far less of it. A slot whose author's bundle
-- was lost or delayed on the guest→host leg contributes nothing at all, so that
-- leg's losses multiply into the host→guest leg instead of being absorbed by the
-- one peer that already has the rows. Measured on the #241 capture, a row that
-- the design fans out seven times was fanned out five, and the guest that
-- stranded received a batch inside its loss burst whose tick span covered the
-- missing row but which carried no row for that slot.
--
-- So every slot the selection does not already cover is topped up from the
-- newest bundle the host accepted for it, re-stamped onto this transport tick.
-- Those rows are authority the host has already validated and canonicalized:
-- authorship is a frozen partition, so a re-sent row is byte-identical to the
-- one it repeats and can never open a divergence.
--
-- Two things bound it. The top-up runs *after* selection and only inside the
-- remaining row budget, so it can never displace a real arrival or push one into
-- the deferred queue. And a bundle whose oldest row has fallen below the host's
-- own retained floor is dropped rather than relayed: authority that old can no
-- longer be placed in any peer's rollback window, and re-sending it would turn a
-- silent gap into a spurious `late_input` on a healthy peer.
--
-- This repairs the *leak*, not the *bound*. It has nothing to spend when the
-- batch is already saturated, which is exactly what the `stress` profile does:
-- `docs/online/fault_harness.md` records that measurement, the `4v4.stress` row
-- it leaves open, and the three allocation schemes that were tried against it and
-- rejected.
---@param driver MatchDriver
---@param selected InputPacketArrival[]
---@param keys table<string, boolean>
---@param count integer
---@param budget integer
---@param transport_tick integer
---@return integer count -- Rows the batch carries after the top-up.
local function fill_relay_window(driver, selected, keys, count, budget, transport_tick)
    ---@type table<integer, boolean>
    local covered = {}
    for _, arrival in ipairs(selected) do
        covered[arrival.packet.rows[1].slot_index] = true
    end
    local floor = retained_floor_input_tick(driver)
    for slot_index = 1, input_frame.SLOT_COUNT do
        local held = not covered[slot_index] and driver._relay[slot_index] or nil
        if held ~= nil and held.packet.rows[1].tick >= floor then
            local additional = 0
            for _, row in ipairs(held.packet.rows) do
                if not keys[tostring(row.tick) .. ":" .. tostring(row.slot_index)] then
                    additional = additional + 1
                end
            end
            if count + additional <= budget then
                for _, row in ipairs(held.packet.rows) do
                    keys[tostring(row.tick) .. ":" .. tostring(row.slot_index)] = true
                end
                count = count + additional
                selected[#selected + 1] = {
                    packet = held.packet,
                    envelope = held.envelope,
                    arrival_tick = transport_tick,
                    transport_peer_id = held.transport_peer_id,
                }
            end
        end
    end
    return count
end

---@param driver MatchDriver
---@param batch MatchDriverBatch
---@param transport_tick integer
---@param messages TransportPeerMessage[]
local function host_sequence_authority(driver, batch, transport_tick, messages)
    local arrivals = collect_arrivals(driver, messages, transport_tick)
    if arrivals == nil then
        return
    end
    -- Anything held back by the row bound last tick is re-stamped onto this
    -- transport tick and sequenced ahead of the new arrivals.
    local carried = driver._deferred
    driver._deferred = {}
    for _, arrival in ipairs(carried) do
        arrival.arrival_tick = transport_tick
        arrivals[#arrivals + 1] = arrival
    end
    -- Confirmation feedback is read before selection, so a bundle the row bound
    -- defers still tells the host where its author is stuck. Deferring authority
    -- and ignoring the report attached to it would be the same blindness this
    -- field exists to remove.
    remember_confirmation(driver, arrivals)
    local budget = input_protocol.MAX_HOST_ROWS
    -- Selection re-sorts, so appending here cannot change which arrivals win.
    local keys, count
    arrivals, keys, count = select_within_bound(driver, arrivals, budget)
    if not running(driver) then
        return
    end
    remember_relay(driver, arrivals)
    -- Three claims on one budget, allocated in this order, and the order *is* the
    -- policy #243 asked for:
    --
    --   1. arrivals, which carry authority nobody else holds;
    --   2. targeted repair, which re-sends what a guest has *said* it is missing;
    --   3. the open-loop top-up, which re-sends a slot's newest window on the
    --      chance that some guest missed it.
    --
    -- Repair outranking the top-up is the whole point: once a guest has reported
    -- where it is stuck, spending the same rows on a guess is strictly worse than
    -- spending them on the answer. Measured -- with the top-up taking the
    -- headroom first, the repair was left between 6 and 16 rows and `4v4.stress`
    -- stayed red on the default seed.
    --
    -- Repair never outranks arrivals, and that matters too: displacing an arrival
    -- defers it to the next transport tick, which delays fresh authority for
    -- *every* peer to help one. Reserving repair budget ahead of selection was
    -- measured as well and is worse at every span tried.
    local repairs = plan_repair(driver, math.floor((budget - count) / input_frame.SLOT_COUNT))
    count = count + (repairs and #repairs or 0)
    count = fill_relay_window(driver, arrivals, keys, count, budget, transport_tick)
    if #arrivals == 0 then
        return
    end
    local packet, err, code = input_protocol.canonical_host_batch({
        manifest = driver._manifest,
        assignments = driver._freeze.assignments,
        host_peer_id = driver._peer_id,
        sequence = next_sequence(driver, driver._peer_id .. ".batch"),
        transport_tick = transport_tick,
        first_input_tick = driver._first,
        confirmed_span = input_protocol.confirmed_span(driver._first, confirmed_input_tick(driver)),
        repair_rows = repairs,
    }, arrivals)
    if packet == nil then
        local status = "input_channel_failure"
        if code == "ownership_mismatch" then
            status = "ownership_violation"
        elseif code == "authority_conflict" or code == "packet_conflict" then
            status = "authority_conflict"
        end
        terminate(
            driver,
            status,
            err or "the host batch could not be canonicalized",
            "input_channel"
        )
        return
    end
    apply_rows(driver, packet.rows, batch, true)
    if not running(driver) then
        return
    end
    local wire = assert(input_protocol.encode(packet))
    local envelope = assert(transport_contract.new({
        type = "input",
        seq = packet.sequence,
        tick = packet.transport_tick,
        payload = wire,
    }))
    local delivered, broadcast_err, broadcast_code = driver._transport:broadcast("input", envelope)
    if delivered == nil then
        terminate(
            driver,
            broadcast_code == "backpressure" and "input_channel_failure" or "transport_lost",
            broadcast_err or "the host could not fan out canonical authority",
            "input_channel"
        )
        return
    end
    batch.sent_packets = batch.sent_packets + 1
end

---@param driver MatchDriver
---@param batch MatchDriverBatch
---@param messages TransportPeerMessage[]
local function guest_apply_authority(driver, batch, messages)
    ---@type InputAuthorityRow[]
    local rows = {}
    ---@type table<string, InputSample>
    local seen = {}
    for _, entry in ipairs(messages) do
        local packet, err, code = input_protocol.decode(entry.message.payload, {
            session_id = driver._manifest.session_id,
            manifest_id = driver._manifest_id,
            sender_id = entry.peer_id,
        })
        if packet == nil then
            terminate(
                driver,
                "input_channel_failure",
                err or "a host authority batch failed to decode",
                "input_channel"
            )
            return
        end
        if packet.kind ~= "host" then
            terminate(
                driver,
                "ownership_violation",
                "a guest received authority that was not a host batch",
                "input_channel"
            )
            return
        end
        -- The host reports its own confirmation in the same field guests use, so
        -- the feedback runs in both directions over the traffic that already
        -- exists. A guest needs it to know whether the tail it authored ever
        -- reached the sequencer -- see `tail_delivered`.
        local host_confirmed = input_protocol.confirmed_tick(packet)
        if driver._host_confirmed == nil or host_confirmed > driver._host_confirmed then
            driver._host_confirmed = host_confirmed
        end
        for _, row in ipairs(packet.rows) do
            local key = tostring(row.tick) .. ":" .. tostring(row.slot_index)
            local prior = seen[key]
            if prior == nil then
                seen[key] = row.sample
                rows[#rows + 1] = row
            elseif
                prior.move_x ~= row.sample.move_x
                or prior.move_y ~= row.sample.move_y
                or prior.held ~= row.sample.held
                or prior.edges ~= row.sample.edges
            then
                terminate(
                    driver,
                    "authority_conflict",
                    "two host batches on one transport tick disagreed on authority",
                    "input_channel"
                )
                return
            end
        end
    end
    -- Callback order cannot reach the simulation: the union is sorted into the
    -- canonical (tick, slot) order before one atomic application.
    table.sort(rows, function(left, right)
        if left.tick ~= right.tick then
            return left.tick < right.tick
        end
        return left.slot_index < right.slot_index
    end)
    apply_rows(driver, rows, batch, true)
end

-- ---------------------------------------------------------------------------
-- Checkpoints
-- ---------------------------------------------------------------------------

-- Boundary hashes are a *running* session's cross-check, and the caller stops
-- publishing them at full time -- see `advance`. From there the final boundary's
-- hash is the result acknowledgement's business, and it subsumes every earlier
-- one: the final state is a function of every confirmed tick, so a boundary that
-- disagreed anywhere cannot agree there.
---@param driver MatchDriver
---@param batch MatchDriverBatch
local function publish_checkpoints(driver, batch)
    -- Output-capped confirmation, never the raw input confirmation: this
    -- boundary keys a snapshot lookup.
    local confirmed = confirmed_output_input_tick(driver)
    while running(driver) and driver._next_checkpoint <= confirmed + 1 do
        local boundary = driver._next_checkpoint
        local lookup = rollback_session.snapshot(driver._session, session_tick(driver, boundary))
        if lookup.status ~= "present" and lookup.status ~= "retained" then
            terminate(
                driver,
                "late_input",
                "a confirmed hash checkpoint fell out of the retained window",
                "late_input",
                boundary
            )
            return
        end
        local hash = match_snapshot.hash(assert(lookup.snapshot))
        -- The live slot is captured with the hash rather than looked up later:
        -- the timeline is pruned with the rollback window, and a checkpoint is
        -- exactly the boundary at which peers must agree on both.
        local live = copy_live_map(driver, assert(driver._live[boundary]))
        driver._checkpoints[#driver._checkpoints + 1] =
            { tick = boundary, hash = hash, live = live }
        driver._checkpoint_by_tick[boundary] = hash
        batch.checkpoints[#batch.checkpoints + 1] =
            { tick = boundary, hash = hash, live = copy_live_map(driver, live) }
        driver._next_checkpoint = boundary + driver._hash_interval
    end
end

-- Compare a peer's published checkpoint. A single disagreement is tolerated and
-- cleared by the next agreement, matching the coordinator's hash policy;
-- `MAX_HASH_MISMATCHES` consecutive disagreements end the match.
---@param driver MatchDriver
---@param tick integer
---@param hash string
---@return boolean matched -- False when the boundary was hashed and disagreed.
function match_driver.observe_checkpoint(driver, tick, hash)
    local mine = driver._checkpoint_by_tick[tick]
    if mine == nil then
        return true
    end
    if mine == hash then
        driver._hash_mismatches = 0
        return true
    end
    driver._hash_mismatches = driver._hash_mismatches + 1
    if driver._hash_mismatches >= match_driver.MAX_HASH_MISMATCHES then
        terminate(
            driver,
            "hash_mismatch",
            "boundary hashes disagreed at "
                .. tostring(match_driver.MAX_HASH_MISMATCHES)
                .. " consecutive checkpoints",
            "desync",
            tick
        )
    end
    return false
end

---@param driver MatchDriver
---@return MatchDriverCheckpoint[]
function match_driver.checkpoints(driver)
    local copied = {}
    for index, checkpoint in ipairs(driver._checkpoints) do
        copied[index] = {
            tick = checkpoint.tick,
            hash = checkpoint.hash,
            live = copy_live_map(driver, checkpoint.live),
        }
    end
    return copied
end

-- ---------------------------------------------------------------------------
-- Confirmation liveness
-- ---------------------------------------------------------------------------

-- Confirmation can stop advancing without anything being raised, and the driver
-- has to say so at the point it happens.
--
-- `rollback_input_history` confirms tick `N` when all eight of its rows are
-- authoritative, and it prunes below the retained floor as the present boundary
-- slides. Those two are independent: nothing stops the floor from passing a tick
-- that never got its eighth row. When it does, that tick's authority is deleted,
-- and because confirmation only ever advances from `confirmed_tick + 1`, it can
-- never cross the hole again -- not later in this match, not ever. The peer
-- keeps simulating on authority it will never confirm.
--
-- Nothing detects that today. The reactive `late_input` path fires when a row
-- *arrives* below the floor; a row that simply never arrives trips nothing, so
-- the first symptom is `settle_timeout` up to a whole match later, reported by a
-- mechanism that exists for a lost peer and carrying a final hash that
-- disagrees for a reason nobody named.
--
-- `confirmed_tick + 1 < oldest_retained_tick` is exactly that state, it is
-- permanent, and it is one comparison. It is deliberately its own terminal:
-- `settle_timeout` means a tail that did not arrive in time, `hash_mismatch`
-- means peers that disagree about a tick they both confirmed, and this means a
-- peer that can no longer confirm at all. It reports as `late_input` into the
-- coordinator because that is the same causal family -- authority this peer can
-- no longer place in its rollback window -- while the local reason stays exact.
---@param driver MatchDriver
local function detect_confirmation_stall(driver)
    if not running(driver) then
        return
    end
    local diagnostics = rollback_session.diagnostics(driver._session)
    local floor = diagnostics.input_history.oldest_retained_tick
    local gap = diagnostics.confirmed_tick + 1
    if gap >= floor then
        return
    end
    local stalled = to_input_tick(driver, gap)
    driver._late_input_tick = driver._late_input_tick or stalled
    terminate(
        driver,
        "confirmation_stalled",
        ("confirmation stopped advancing at input tick %d: tick %d never became authoritative "):format(
            stalled - 1,
            stalled
        )
            .. ("and has fallen below the retained floor at tick %d"):format(
                to_input_tick(driver, floor)
            ),
        "late_input",
        stalled
    )
end

-- ---------------------------------------------------------------------------
-- Full time and settling
-- ---------------------------------------------------------------------------

-- Full time stops the simulation and opens the settle phase. It does not
-- terminate: up to `DELAY_TICKS` of the match can still be unconfirmed here, and
-- a result reported over predicted authority is exactly the disagreement this
-- phase exists to prevent.
---@param driver MatchDriver
local function reach_full_time(driver)
    if driver._full_time_boundary ~= nil then
        return
    end
    driver._full_time_boundary = present_input_tick(driver)
    driver._settle_deadline = driver._clock() + driver._settle_seconds
end

-- Which seven-tick window a settling guest re-publishes.
--
-- One bundle is a fixed seven rows ending at the tick it names, so naming a tick
-- chooses the whole window. Re-sending the newest one every step is the obvious
-- choice and is wrong whenever the host is more than a window behind: the rows
-- the sequencer is actually missing have already fallen out of it, and no number
-- of re-sends of the newest window will ever contain them.
--
-- The host now says where its confirmation is, so the window can be anchored just
-- above it: `host_confirmed + 1 + HISTORY_ROWS` ends a bundle whose oldest row is
-- exactly the tick the host needs next. Capped at the newest authored tick, so a
-- caught-up host still gets the ordinary newest window and nothing changes for a
-- healthy settle.
--
-- Measured on `4v4.stress` seed 4738: the host was confirmed at 114 with the
-- final boundary at 121, and the two rows nobody in the session held were ticks
-- 115 and 116 of `guest_7`'s slot. `guest_7` re-published `[118, 124]` sixty
-- times and never once carried them. Anchored at the host's report it publishes
-- `[115, 121]` instead, which is the same seven rows' worth of traffic aimed at
-- the gap rather than past it.
---@param driver MatchDriver
---@return integer
local function settle_tail_tick(driver)
    local newest = driver._authored_tick
    local host_confirmed = driver._role == "host" and confirmed_input_tick(driver)
        or driver._host_confirmed
    if host_confirmed == nil then
        return newest
    end
    local anchored = host_confirmed + 1 + input_protocol.HISTORY_ROWS
    -- A bundle carries `HISTORY_ROWS` rows *behind* the tick it names, so the
    -- anchor cannot reach below the oldest authored sample still retained or
    -- `redundant_rows` would be asked for a row that was pruned. A host that far
    -- behind is past saving by re-publication anyway: those ticks are below every
    -- peer's rollback floor.
    local oldest = math.max(
        driver._first,
        newest - rollback_input_history.ROLLBACK_WINDOW_TICKS + input_protocol.HISTORY_ROWS
    )
    return math.max(oldest, math.min(newest, anchored))
end

-- One settle step's outbound traffic: an authored redundancy window, re-published
-- unchanged.
--
-- Nothing new is authored, because nothing is simulated any more -- there is no
-- boundary state to materialize an AI row from and no tick left for a human
-- sample to be authority for. What a peer can still be missing is the tail of
-- what was already authored, and that is precisely this window. Re-sending it
-- once per settle step gives the last ticks the same redundancy every earlier
-- tick got; without it the final rows would be published fewer times than any
-- others, which is why the tail is the part that fails to confirm.
--
-- The host's re-sends go back through its own collector on the ordinary
-- `DELAY_TICKS` due date. `canonical_host_batch` refuses host-local input that
-- has not spent the fairness delay, and settling is not a licence to bypass
-- canonical sequencing.
--
-- Which window gets re-sent is `settle_tail_tick`'s answer, not simply the newest
-- one -- see there for why the newest is the wrong window when the host is behind.
---@param driver MatchDriver
---@param batch MatchDriverBatch
---@param transport_tick integer
local function resend_tail(driver, batch, transport_tick)
    local tick = settle_tail_tick(driver)
    for _, slot_index in ipairs(driver._authored) do
        local packet, envelope = build_packet(driver, slot_index, tick, transport_tick)
        if driver._role == "host" then
            driver._pending[#driver._pending + 1] = {
                due = driver._step + match_driver.DELAY_TICKS,
                packet = packet,
                envelope = envelope,
                producer_id = packet.sender_id,
            }
        else
            local ok, err, code =
                driver._transport:send(transport_contract.HOST_PEER_ID, "input", envelope)
            if ok == nil then
                terminate(
                    driver,
                    code == "backpressure" and "input_channel_failure" or "transport_lost",
                    err or "a guest could not re-publish its settling input bundle",
                    "input_channel"
                )
                return
            end
            batch.sent_packets = batch.sent_packets + 1
        end
    end
end

-- Close the settle phase, one way or the other.
--
-- It completes when the final boundary is confirmed -- `confirmed_output_tick`,
-- the output-capped confirmation, because a settled boundary is exactly one that
-- was both simulated and fully authorized. The state captured there is then a
-- function of confirmed authority alone, which is what makes the final hash the
-- session acknowledges an agreement rather than a coincidence.
--
-- Otherwise it expires, in ticks or in wall clock, whichever comes first. A peer
-- that has genuinely gone must not hold the match open, and there is no path
-- here that waits on anything unbounded: both deadlines are fixed the moment
-- full time is reached, and each `advance` re-checks them exactly once.
-- Whether this peer may leave the settle phase, over and above having confirmed
-- its own final boundary. Both roles owe the star something, and #243's
-- confirmation feedback is what lets each of them *know* when the debt is paid
-- instead of inferring it.
--
-- **The host owes a relay.** A guest's settle re-sends carry rows it already
-- holds; the rows it is *missing* belong to other peers and can only reach it
-- inside a canonical host batch, so the instant the host stops the star stops.
-- The host is also structurally the first peer to confirm the final boundary --
-- it is the sequencer -- which is why "confirmed, therefore done" let it leave
-- three steps into a sixty-step settle window while three of its guests were
-- still asking (#237).
--
-- Until now the host could only *infer* that nobody was still asking, from
-- `SETTLE_RELAY_QUIET_STEPS` consecutive steps with no inbound traffic. That
-- heuristic is documented above as the only one in a phase whose other bounds
-- are hard, and as covering everything except a worst-case burst followed by
-- worst-case jitter. Every settling guest now *states* its confirmation every
-- step, so the ordinary case is a bound: the host leaves when every author it
-- has heard from has confirmed the final boundary. The quiet count survives only
-- as the fallback for a peer that has stopped speaking altogether, which is the
-- one case a report cannot cover -- and which the settle deadline already bounds.
--
-- **A guest owes its tail.** Its authored rows for the last ticks exist nowhere
-- else until the host has them, and the host is the only peer that can fan them
-- out to the other six. A guest that confirms its own boundary and leaves takes
-- that authority with it. Measured on `4v4.stress` seed 4738: `guest_7` completed
-- while ticks 115 and 116 of its slot had never reached the host, and the other
-- seven peers -- the host included -- spent the whole settle window missing
-- exactly those two rows and then timed out. This is #241's tail stall in the
-- other direction, and the same field closes it: the host's batches carry the
-- host's own confirmation, so a guest can see that the sequencer is still behind
-- the final boundary and keep re-publishing until it is not.
--
-- Neither wait is unbounded. The settle deadline expires both, and a peer that
-- has confirmed its own boundary completes at expiry rather than failing, so
-- waiting here can delay a whistle but can never invent a `settle_timeout`.
---@param driver MatchDriver
---@return boolean
local function tail_delivered(driver)
    local boundary = driver._full_time_boundary
    if boundary == nil then
        return true
    end
    if driver._role ~= "host" then
        -- Silence is not consent here: a guest that has never heard a host batch
        -- has no evidence its tail landed, and must keep re-publishing.
        return driver._host_confirmed ~= nil and driver._host_confirmed + 1 >= boundary
    end
    if driver._settle_quiet_steps >= match_driver.SETTLE_RELAY_QUIET_STEPS then
        return true
    end
    for slot_index = 1, input_frame.SLOT_COUNT do
        local confirmed = driver._peer_confirmed[slot_index]
        if confirmed ~= nil and confirmed + 1 < boundary then
            return false
        end
    end
    return true
end

---@param driver MatchDriver
local function settle(driver)
    local boundary = driver._full_time_boundary
    if boundary == nil or not running(driver) then
        return
    end
    local confirmed = confirmed_output_input_tick(driver) + 1 >= boundary
    if confirmed and tail_delivered(driver) then
        driver._settled = true
        terminate(
            driver,
            "completed",
            "the match reached full time with the final boundary confirmed"
        )
        return
    end
    -- Both deadlines end the phase, but they do not decide the status: a peer
    -- that confirmed its own final boundary completes over confirmed authority
    -- whether or not it was still relaying for somebody else. `settle_timeout`
    -- keeps meaning exactly one thing -- the phase expired with this peer's own
    -- final boundary still unconfirmed.
    local expired = driver._settle_steps >= driver._settle_ticks
    local detail = "full time was reached but the final boundary was still unconfirmed after "
        .. tostring(driver._settle_ticks)
        .. " settle ticks"
    if not expired and driver._clock() >= assert(driver._settle_deadline) then
        expired = true
        detail = "full time was reached but the final boundary was still unconfirmed after "
            .. tostring(driver._settle_seconds)
            .. " seconds of settling"
    end
    if not expired then
        driver._settle_steps = driver._settle_steps + 1
        return
    end
    if confirmed then
        driver._settled = true
        terminate(
            driver,
            "completed",
            "the match reached full time with the final boundary confirmed"
        )
        return
    end
    terminate(driver, "settle_timeout", detail, "input_channel", boundary)
end

-- True once the final boundary was confirmed. This is the settled
-- full time, as opposed to the tick the simulation happened to stop on, and it
-- is what presentation must key the final whistle off so the visible whistle and
-- the confirmed result agree.
---@param driver MatchDriver
---@return boolean
function match_driver.settled(driver)
    return driver._settled
end

-- The final boundary, in input-tick space. `nil` until full time is reached.
---@param driver MatchDriver
---@return integer?
function match_driver.full_time_boundary(driver)
    return driver._full_time_boundary
end

-- ---------------------------------------------------------------------------
-- Construction
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
local function prime(driver)
    -- Nobody could sample before the start boundary, so the first `DELAY` input
    -- ticks carry neutral human rows. AI rows are materialized from boundary
    -- zero, which every peer shares.
    -- Nothing is simulated between these ticks, so the boundary-zero state is
    -- restored once and reused rather than re-derived per priming tick.
    local geometry = #driver._authored > 1 and present_state(driver) or nil
    for tick = driver._first, driver._first + match_driver.DELAY_TICKS - 1 do
        local authored = materialize_authored(driver, tick, nil, geometry)
        for _, slot_index in ipairs(driver._authored) do
            record_authored(driver, slot_index, tick, authored[slot_index])
        end
    end
    if driver._role ~= "host" then
        -- A guest's step-zero bundle already carries these rows as redundancy,
        -- and a guest applies its own rows without a collector.
        return
    end
    -- The host's collector is where its fairness delay is spent, so its
    -- pre-start rows must be placed in it on transport tick zero to be readable
    -- on the first step. They cannot bypass anything: nobody had sampled yet.
    local last = driver._first + match_driver.DELAY_TICKS - 1
    for _, slot_index in ipairs(driver._authored) do
        local packet, envelope = build_packet(driver, slot_index, last, 0)
        driver._pending[#driver._pending + 1] = {
            due = 0,
            packet = packet,
            envelope = envelope,
            producer_id = packet.sender_id,
        }
    end
end

---@param options MatchDriverOptions
---@return MatchDriver
function match_driver.new(options)
    assert(type(options) == "table", "match driver options are required")
    local freeze = assert(options.freeze, "match driver requires a coordinator freeze")
    local manifest = assert(options.manifest, "match driver requires the frozen manifest")
    local transport = assert(options.transport, "match driver requires a star transport")
    local role = options.role
    assert(role == "host" or role == "guest", "match driver role must be host or guest")
    local peer_id = options.peer_id
    assert(type(peer_id) == "string" and peer_id ~= "", "match driver requires a peer id")
    local manifest_id = protocol.manifest_id(manifest)
    assert(
        manifest_id == freeze.manifest_id,
        "match driver manifest does not match the frozen session"
    )
    local hash_interval = options.hash_interval_ticks or match_driver.DEFAULT_HASH_INTERVAL_TICKS
    assert(
        is_integer(hash_interval) and hash_interval >= 1,
        "match driver hash interval must be a positive integer"
    )
    local settle_ticks = options.settle_timeout_ticks or match_driver.SETTLE_TIMEOUT_TICKS
    assert(
        is_integer(settle_ticks) and settle_ticks >= 1,
        "match driver settle window must be a positive integer number of ticks"
    )
    local settle_seconds = options.settle_timeout_seconds or match_driver.SETTLE_TIMEOUT_SECONDS
    assert(
        type(settle_seconds) == "number" and settle_seconds > 0 and settle_seconds < math.huge,
        "match driver settle window must be a positive number of seconds"
    )
    local clock = options.clock or default_clock
    assert(type(clock) == "function", "match driver clock must be a monotonic seconds function")
    local maximum = options.max_rollback_ticks or rollback_input_history.ROLLBACK_WINDOW_TICKS
    assert(
        is_integer(maximum)
            and maximum >= 1
            and maximum <= rollback_input_history.ROLLBACK_WINDOW_TICKS,
        "match driver rollback window must be a bounded positive integer"
    )

    local owned = protocol.owned_slots(freeze.assignments, peer_id)
    ---@type table<integer, boolean>
    local owned_set = {}
    for _, slot in ipairs(owned) do
        owned_set[live_slot.slot_index(slot)] = true
    end

    ---@type table<string, integer[]>
    local peer_slots = {}
    ---@type string[]
    local humans = {}
    ---@type integer[]
    local bot_slots = {}
    for index = 1, input_frame.SLOT_COUNT do
        local producer = assert(freeze.assignments[index], "the freeze has an unassigned slot")
        if producer.producer_kind == "peer" then
            if peer_slots[producer.producer_id] == nil then
                peer_slots[producer.producer_id] = {}
                humans[#humans + 1] = producer.producer_id
            end
            local slots = peer_slots[producer.producer_id]
            slots[#slots + 1] = index
        else
            bot_slots[#bot_slots + 1] = index
        end
    end

    ---@type integer[]
    local authored = {}
    ---@type table<integer, boolean>
    local authored_set = {}
    ---@type MatchSlotSource[]
    local sources = {}
    ---@type RollbackInputSource[]
    local rollback_sources = {}
    for index = 1, input_frame.SLOT_COUNT do
        local producer = freeze.assignments[index]
        local mine = owned_set[index] or (role == "host" and producer.producer_kind == "bot")
        if mine then
            authored[#authored + 1] = index
            authored_set[index] = true
            local seed = producer.bot_seed or derived_ai_seed(freeze.seed, index)
            sources[index] = { kind = "bot", seed = seed }
            rollback_sources[index] = "local"
        else
            sources[index] = { kind = "neutral" }
            rollback_sources[index] = "remote"
        end
    end

    local session = rollback_session.new(options.initial_snapshot, rollback_sources, maximum)
    ---@type table<integer, table<integer, InputSample>>
    local history = {}
    for index = 1, input_frame.SLOT_COUNT do
        history[index] = {}
    end

    ---@type MatchDriver
    local driver = {
        _role = role,
        _peer_id = peer_id,
        _freeze = freeze,
        _manifest = manifest,
        _manifest_id = manifest_id,
        _transport = transport,
        _session = session,
        _sources = rollback_sources,
        _producer = slot_input.new_producer(sources),
        _owned = owned,
        _authored = authored,
        _authored_set = authored_set,
        _owned_set = owned_set,
        _peer_slots = peer_slots,
        _humans = humans,
        _bot_slots = bot_slots,
        _first = freeze.first_input_tick,
        _step = 0,
        _live = {},
        _carrier = {},
        _live_tick = freeze.first_input_tick,
        _history = history,
        _sequences = {},
        _pending = {},
        _deferred = {},
        _relay = {},
        _peer_confirmed = {},
        _host_confirmed = nil,
        _checkpoints = {},
        _checkpoint_by_tick = {},
        _hash_interval = hash_interval,
        _next_checkpoint = freeze.first_input_tick,
        _hash_mismatches = 0,
        _status = "active",
        _terminal = nil,
        _late_input_tick = nil,
        _authored_tick = freeze.first_input_tick + match_driver.DELAY_TICKS - 1,
        _settle_ticks = settle_ticks,
        _settle_seconds = settle_seconds,
        _clock = clock,
        _full_time_boundary = nil,
        _settle_steps = 0,
        _settle_deadline = nil,
        _settle_quiet_steps = 0,
        _settled = false,
    }
    ---@type table<string, InputSlotId>
    local opening = {}
    for _, producer_id in ipairs(humans) do
        opening[producer_id] = assert(freeze.live[producer_id], "a human has no opening live slot")
    end
    driver._live[driver._first] = opening
    driver._carrier[driver._first] = live_slot.carrier(boundary_state(driver, driver._first))
    prime(driver)
    return driver
end

-- ---------------------------------------------------------------------------
-- The fixed-tick step
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@param batch MatchDriverBatch
---@param input_tick integer
---@param transport_tick integer
---@param sample InputSample?
local function author_and_send(driver, batch, input_tick, transport_tick, sample)
    local authored = materialize_authored(driver, input_tick, sample)
    for _, slot_index in ipairs(driver._authored) do
        record_authored(driver, slot_index, input_tick, authored[slot_index])
    end
    driver._authored_tick = input_tick
    ---@type InputAuthorityRow[]
    local local_rows = {}
    for _, slot_index in ipairs(driver._authored) do
        local packet, envelope = build_packet(driver, slot_index, input_tick, transport_tick)
        if driver._role == "host" then
            driver._pending[#driver._pending + 1] = {
                due = driver._step + match_driver.DELAY_TICKS,
                packet = packet,
                envelope = envelope,
                producer_id = packet.sender_id,
            }
        else
            for _, row in ipairs(packet.rows) do
                local_rows[#local_rows + 1] = row
            end
            local ok, err, code =
                driver._transport:send(transport_contract.HOST_PEER_ID, "input", envelope)
            if ok == nil then
                terminate(
                    driver,
                    code == "backpressure" and "input_channel_failure" or "transport_lost",
                    err or "a guest could not publish its input bundle",
                    "input_channel"
                )
                return
            end
            batch.sent_packets = batch.sent_packets + 1
        end
    end
    -- A guest's own rows are local authority immediately, applied as one batch
    -- so its own authoring never costs a second reconciliation. The host's
    -- canonical echo is byte-identical and therefore idempotent.
    table.sort(local_rows, function(left, right)
        if left.tick ~= right.tick then
            return left.tick < right.tick
        end
        return left.slot_index < right.slot_index
    end)
    apply_rows(driver, local_rows, batch, false)
end

---@param driver MatchDriver
---@param batch MatchDriverBatch
---@param input_tick integer
local function step_to(driver, batch, input_tick)
    while running(driver) do
        local diagnostics = rollback_session.diagnostics(driver._session)
        if to_input_tick(driver, diagnostics.present_boundary) > input_tick then
            return
        end
        if diagnostics.status == "finished" then
            reach_full_time(driver)
            return
        end
        local output, err, code = rollback_session.step(driver._session)
        if output == nil then
            if code == "match_finished" then
                reach_full_time(driver)
                return
            end
            terminate(
                driver,
                "late_input",
                err or "the rollback session cannot progress",
                "late_input",
                driver._late_input_tick
            )
            return
        end
        local tick = to_input_tick(driver, output.tick)
        extend_live(driver, tick, output.input)
        batch.outputs[#batch.outputs + 1] = output
        local floor =
            rollback_session.diagnostics(driver._session).input_history.oldest_retained_tick
        prune_live(driver, to_input_tick(driver, floor))
        if output.finished then
            reach_full_time(driver)
            return
        end
    end
end

-- One fixed 60 Hz driver step: poll, apply one canonical arrival batch through
-- one reconciliation, author this step's rows, publish them, simulate one input
-- tick, and hash any confirmed checkpoint that came due.
--
-- After full time the same step keeps its first half and loses its second: the
-- driver still polls, still applies one arrival batch through one
-- reconciliation, and still re-publishes the last authored window, but it
-- authors nothing new and simulates nothing. It ends when the final boundary
-- confirms or the settle phase expires.
---@param driver MatchDriver
---@param sample InputSample? -- The local human's sample; nil for a peer that owns nothing.
---@return MatchDriverBatch
function match_driver.advance(driver, sample)
    local input_tick = driver._first + driver._step
    local transport_tick = driver._step + match_driver.DELAY_TICKS
    ---@type MatchDriverBatch
    local batch = {
        step = driver._step,
        input_tick = input_tick,
        outputs = {},
        reconciliations = 0,
        applied_rows = 0,
        corrections = 0,
        rollbacks = 0,
        sent_packets = 0,
        checkpoints = {},
        control = {},
        live = {},
        status = driver._status,
    }
    if not running(driver) then
        -- No hidden progress after a terminal status: nothing is polled, sent,
        -- applied, or simulated.
        batch.live = copy_live_map(driver, driver._live[driver._live_tick] or {})
        return batch
    end

    drain_events(driver)
    local messages = running(driver) and poll_input(driver, batch) or {}
    if driver._full_time_boundary ~= nil then
        -- Inbound input traffic during the settle phase is the only evidence the
        -- host has that a guest is still asking it to relay.
        if #messages > 0 then
            driver._settle_quiet_steps = 0
        else
            driver._settle_quiet_steps = driver._settle_quiet_steps + 1
        end
    end
    if running(driver) then
        if driver._role == "host" then
            host_sequence_authority(driver, batch, transport_tick, messages)
        else
            guest_apply_authority(driver, batch, messages)
        end
    end
    if running(driver) then
        if driver._full_time_boundary ~= nil then
            resend_tail(driver, batch, transport_tick)
        else
            author_and_send(
                driver,
                batch,
                input_tick + match_driver.DELAY_TICKS,
                transport_tick,
                sample
            )
        end
    end
    if running(driver) and driver._full_time_boundary == nil then
        step_to(driver, batch, input_tick)
    end
    -- Boundary hashes stop at full time, including on the step that reaches it.
    -- Settling peers no longer terminate on the same step, so a checkpoint
    -- published while one peer settles can reach another that has already left
    -- `running` for the result -- and a hash report is only legal in a running
    -- session. It costs no detection: from here the acknowledged final hash is
    -- the cross-check, and every earlier boundary is folded into it.
    if driver._full_time_boundary == nil then
        publish_checkpoints(driver, batch)
    end
    -- Before settling, because a peer whose confirmation is already dead must say
    -- so with the reason that is true rather than wait out a phase whose deadline
    -- would describe a different fault.
    detect_confirmation_stall(driver)
    settle(driver)

    driver._step = driver._step + 1
    batch.status = driver._status
    batch.live = copy_live_map(driver, driver._live[driver._live_tick] or {})
    return batch
end

-- ---------------------------------------------------------------------------
-- Pure diagnostics
-- ---------------------------------------------------------------------------

---@param driver MatchDriver
---@return MatchDriverStatus
function match_driver.status(driver)
    return driver._status
end

---@param driver MatchDriver
---@return MatchDriverTerminal?
function match_driver.terminal(driver)
    local terminal = driver._terminal
    if terminal == nil then
        return nil
    end
    return {
        status = terminal.status,
        failure = terminal.failure,
        detail = terminal.detail,
        tick = terminal.tick,
    }
end

---@param driver MatchDriver
---@return MatchSnapshot
function match_driver.current_snapshot(driver)
    return rollback_session.current_snapshot(driver._session)
end

---@param driver MatchDriver
---@param boundary integer -- Input-tick space.
---@return RollbackSnapshotLookup
function match_driver.snapshot(driver, boundary)
    return rollback_session.snapshot(driver._session, session_tick(driver, boundary))
end

---@param driver MatchDriver
---@return table<InputSlotId, CoordinatorSlotDriver>
function match_driver.slot_drivers(driver)
    return coordinator.slot_drivers(driver._freeze, driver._live[driver._live_tick])
end

---@param driver MatchDriver
---@return MatchDriverDiagnostics
function match_driver.diagnostics(driver)
    local session = rollback_session.diagnostics(driver._session)
    local transport = driver._transport:diagnostics()
    ---@type InputSlotId[]
    local authored = {}
    for index, slot_index in ipairs(driver._authored) do
        authored[index] = live_slot.slot_id(slot_index)
    end
    ---@type InputSlotId[]
    local owned = {}
    for index, slot in ipairs(driver._owned) do
        owned[index] = slot
    end
    local control = nil
    if #driver._owned > 0 then
        control = match_driver.control_slot(driver, driver._peer_id, driver._live_tick)
    end
    return {
        role = driver._role,
        peer_id = driver._peer_id,
        status = driver._status,
        terminal = match_driver.terminal(driver),
        step = driver._step,
        transport_tick = driver._step + match_driver.DELAY_TICKS,
        present_input_tick = present_input_tick(driver),
        confirmed_input_tick = confirmed_input_tick(driver),
        confirmed_output_tick = confirmed_output_input_tick(driver),
        retained_floor_tick = retained_floor_input_tick(driver),
        owned = owned,
        authored = authored,
        live = copy_live_map(driver, driver._live[driver._live_tick] or {}),
        control_slot = control,
        rollback_count = session.rollback_count,
        correction_count = session.correction_count,
        predicted_slot_samples = session.predicted_slot_samples,
        max_rollback_depth = session.max_rollback_depth,
        late_input_tick = driver._late_input_tick,
        hash_mismatches = driver._hash_mismatches,
        checkpoint_count = #driver._checkpoints,
        settling = driver._full_time_boundary ~= nil and driver._status == "active",
        settled = driver._settled,
        full_time_boundary = driver._full_time_boundary,
        settle_steps = driver._settle_steps,
        dropped_outbound = transport.dropped_outbound,
        dropped_inbound = transport.dropped_inbound,
    }
end

return match_driver
