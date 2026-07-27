-- The online match request: everything one peer needs to start playing, derived
-- only from the frozen #161 manifest and the frozen #163 slot assignment.
--
-- Nothing here is local. Two peers holding the same freeze build byte-identical
-- requests, including boundary zero, which is the whole point: the driver's
-- rollback session is seeded from this snapshot on every peer, so a difference
-- would be an immediate desync rather than a cosmetic disagreement.
--
-- It draws nothing, opens no transport, and advances no simulation.
--
-- # Ownership, switching, and the keeper
--
-- A human owns a *set* of canonical outfield slots — four in 1v1, two in 2v2,
-- one in 4v4 — and controls one of them at a time. Switching inside that frozen
-- set is enabled and reuses the shipped single-player rules (`docs/controls.md`:
-- switch hands control to the outfielder nearest the ball, and winning the ball
-- auto-switches to the winner), intersected with the owned set by
-- `coordinator.next_live_slot`. In 4v4 the owned set is a singleton, so every
-- branch of that rule returns the slot already live and switching is inert.
-- That is a *consequence* of the general rule: there is no mode branch here or
-- anywhere below, and adding one would be the bug.
--
-- **Keeper control stays disabled in every mode.** Keepers are slotless and
-- AI-only, so a human's owned set can never contain one and no input path can
-- reach one. This is a deliberate divergence from solo play, where control
-- passes to your keeper when it gathers the ball; it is a design decision
-- reaffirmed by the repository owner for OMP-3, not an oversight to be "fixed"
-- later. `match_session.KEEPER_CONTROL` records it in one place so the intent
-- survives without a comment archaeology dig.

local coordinator = require("game.online.coordinator")
local live_slot = require("game.online.live_slot")
local match_manifest = require("game.online.match_manifest")
local protocol = require("game.online.protocol")
local combat = require("sim.combat")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local input_tape = require("sim.input_tape")
local match_snapshot = require("sim.match_snapshot")
local sim_match = require("sim.match")

---@class OnlineMatchRequestOptions
---@field role SessionRole
---@field peer_id string
---@field manifest SessionManifest
---@field freeze CoordinatorFreeze

---@class OnlineMatchRequest
---@field role SessionRole
---@field peer_id string
---@field manifest SessionManifest
---@field freeze CoordinatorFreeze
---@field mode SessionMatchMode
---@field owned InputSlotId[] -- This peer's frozen owned set, canonical order.
---@field live InputSlotId? -- Opening live slot; nil for a peer that owns nothing.
---@field slot_drivers table<InputSlotId, CoordinatorSlotDriver>
---@field home TeamData
---@field away TeamData
---@field arena ArenaData
---@field combat_enabled boolean
---@field snapshot_version integer -- Explicit combat-bearing MatchSnapshot contract.
---@field tape_version integer -- Explicit combat-bearing input tape contract.
---@field seed integer
---@field duration number -- Seconds, derived from the frozen tick budget.
---@field max_goals integer
---@field first_input_tick integer
---@field initial_snapshot MatchSnapshot

---@class OnlineMatchSessionModule
local match_session = {}

-- Keeper control is off in every online mode. See the header.
match_session.KEEPER_CONTROL = false

---@param freeze CoordinatorFreeze
---@param manifest SessionManifest
---@return boolean?, string?
local function identity_agrees(freeze, manifest)
    local manifest_id = protocol.manifest_id(manifest)
    if manifest_id ~= freeze.manifest_id then
        return nil, "the frozen manifest digest does not match the manifest"
    end
    for _, field in ipairs({
        "seed",
        "tick_rate",
        "duration_ticks",
        "max_goals",
        "content_id",
        "tuning_id",
        "combat_rules_id",
        "gameplay_ai_policy_id",
        "combat_status",
        "match_mode",
    }) do
        if freeze[field] ~= manifest[field] then
            return nil, "the freeze and the manifest disagree about " .. field
        end
    end
    return true
end

-- The online match always selects the combat-bearing contracts explicitly. The
-- protocol already refuses any other version, but stating it here means the
-- boundary-zero snapshot carries a `CombatMatchState` because this flow asked
-- for one, not because a default happened to line up.
---@param manifest SessionManifest
---@return boolean?, string?
local function contracts_agree(manifest)
    if manifest.snapshot_version ~= match_snapshot.COMBAT_VERSION then
        return nil, "the online match requires the combat-bearing snapshot contract"
    end
    if manifest.tape_version ~= input_tape.COMBAT_VERSION then
        return nil, "the online match requires the combat-bearing tape contract"
    end
    if manifest.tick_rate ~= fixed_clock.TICK_RATE then
        return nil, "the manifest tick rate is not this build's fixed tick rate"
    end
    return true
end

---@param freeze CoordinatorFreeze
---@param peer_id string
---@return InputSlotId[]?, string?
local function owned_slots(freeze, peer_id)
    local shape, err = protocol.match_mode(freeze.match_mode)
    if not shape then
        return nil, err
    end
    local owned = protocol.owned_slots(freeze.assignments, peer_id)
    if #owned == 0 then
        -- A spectating peer is not a supported OMP-3 shape: every seated human
        -- owns a block, and a peer with no block would author nothing while the
        -- partition still expected it to.
        return nil, "the freeze seats no slots for this peer"
    end
    if #owned ~= shape.slots_per_human then
        return nil, "the frozen owned set is the wrong size for the match mode"
    end
    local declared = freeze.owned[peer_id]
    if declared == nil or #declared ~= #owned then
        return nil, "the freeze does not record this peer's owned set"
    end
    for index, slot in ipairs(owned) do
        if declared[index] ~= slot then
            return nil, "the frozen owned set disagrees with the frozen assignments"
        end
        -- Structural, not defensive: canonical slots are outfield-only, so this
        -- can only fail if the slot vocabulary itself changed.
        local canonical = assert(input_frame.slot(live_slot.slot_index(slot)))
        if canonical.id ~= slot then
            return nil, "the frozen owned set names a slot outside the canonical eight"
        end
    end
    return owned
end

-- Boundary zero. Slot mode is mandatory — the online driver never runs the
-- legacy single-input path — and the combat companion is always present.
---@param content OnlineMatchContent
---@param freeze CoordinatorFreeze
---@return MatchSnapshot
local function initial_snapshot(content, freeze)
    local state = sim_match.new({
        home = content.home,
        away = content.away,
        field = { w = 960, h = 540 },
        duration = freeze.duration_ticks / freeze.tick_rate,
        max_goals = freeze.max_goals,
        seed = freeze.seed,
        input_ownership = content.ownership,
    })
    assert(state.slot_mode, "an online match requires a slot-mode simulation")
    assert(state.input_tick == 0, "an online match starts at boundary zero")
    return match_snapshot.capture(state, combat.new_state(state))
end

-- Build one peer's online match request. Returns `nil, err` for anything a
-- caller is meant to handle — a manifest this build cannot resolve, a freeze
-- that disagrees with it, an owned set the mode does not allow.
---@param options OnlineMatchRequestOptions
---@return OnlineMatchRequest?, string?
function match_session.request(options)
    if type(options) ~= "table" then
        return nil, "an online match request needs options"
    end
    local role = options.role
    if role ~= "host" and role ~= "guest" then
        return nil, "an online match request needs a session role"
    end
    local peer_id = options.peer_id
    if type(peer_id) ~= "string" or peer_id == "" then
        return nil, "an online match request needs a peer id"
    end
    local manifest = options.manifest
    local freeze = options.freeze
    if type(manifest) ~= "table" or type(freeze) ~= "table" then
        return nil, "an online match request needs the frozen manifest and freeze"
    end
    local ok, err = contracts_agree(manifest)
    if not ok then
        return nil, err
    end
    ok, err = identity_agrees(freeze, manifest)
    if not ok then
        return nil, err
    end
    local content
    content, err = match_manifest.resolve(manifest)
    if not content then
        return nil, err
    end
    local sources
    sources, err = coordinator.slot_sources(manifest, freeze.assignments)
    if not sources then
        return nil, err
    end
    local owned
    owned, err = owned_slots(freeze, peer_id)
    if not owned then
        return nil, err
    end
    local live = freeze.live[peer_id]
    if live == nil then
        return nil, "the freeze records no opening live slot for this peer"
    end
    local owns_live = false
    for _, slot in ipairs(owned) do
        owns_live = owns_live or slot == live
    end
    if not owns_live then
        return nil, "the opening live slot is outside the frozen owned set"
    end
    return {
        role = role,
        peer_id = peer_id,
        manifest = manifest,
        freeze = freeze,
        mode = freeze.match_mode,
        owned = owned,
        live = live,
        slot_drivers = coordinator.slot_drivers(freeze, freeze.live),
        home = content.home,
        away = content.away,
        arena = content.arena,
        combat_enabled = true,
        snapshot_version = manifest.snapshot_version,
        tape_version = manifest.tape_version,
        seed = freeze.seed,
        duration = freeze.duration_ticks / freeze.tick_rate,
        max_goals = freeze.max_goals,
        first_input_tick = freeze.first_input_tick,
        initial_snapshot = initial_snapshot(content, freeze),
    }
end

-- The match player index a slot drives, for presentation only. Slot routing in
-- the simulation is `slot_players`; this is what the camera follows and what the
-- HUD names, and it can never widen who a peer may author.
---@param state MatchState
---@param slot InputSlotId
---@return integer
function match_session.player_index(state, slot)
    local index = live_slot.slot_index(slot)
    return assert(state.slot_players[index], "canonical slot is unmapped")
end

-- True when the slot a human is live on is currently carrying the ball. The
-- offline switch rule ("K without the ball switches") reads exactly this.
---@param state MatchState
---@param slot InputSlotId?
---@return boolean
function match_session.carrying(state, slot)
    if slot == nil or state.owner == nil then
        return false
    end
    return state.owner == match_session.player_index(state, slot)
end

return match_session
