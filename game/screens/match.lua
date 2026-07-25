-- The playable 5v5 match screen. Gathers input, delegates all simulation to
-- `sim.match`, and renders the result. Drawing lives ONLY here (AGENTS.md §2).

local sim_match = require("sim.match")
local combat_sim = require("sim.combat")
local fixed_clock = require("sim.fixed_clock")
local match_snapshot = require("sim.match_snapshot")
local rollback_playable_lab = require("sim.rollback_playable_lab")
local slot_input = require("sim.slot_input")
local arenas = require("data.arenas")
local teams = require("data.teams")
local tactics = require("data.tactics")
local pitch = require("game.render.pitch")
local bloom = require("game.render.bloom")
local correction_smoothing = require("game.render.correction_smoothing")
local effects = require("game.render.effects")
local match_hud_render = require("game.render.match_hud")
local view_state = require("game.render.view_state")
local combat_presentation = require("game.presentation.combat")
local audio = require("game.audio")
local match_hud = require("game.match_hud")
local onboarding = require("game.match_onboarding")
local tuning_panel = require("game.ui.tuning_panel")
local replay = require("game.render.replay")
local match_input_adapter = require("game.match_input_adapter")
local Vec2 = require("core.vec2")

local FIELD_W = 960
local FIELD_H = 540

---@class MatchRollbackLabOptions
---@field local_slot integer?
---@field profile_name string?
---@field network_profile NetworkProfile?
---@field network_seed integer?
---@field bot_seed integer?
---@field max_rollback_ticks integer?
---@field settlement_ticks integer?
---@field duration number?
---@field max_goals integer?
---@field initial_snapshot MatchSnapshot? -- Optional canonical development fixture seam.

---@class MatchRollbackDebugModel : RollbackPlayableLabDebugModel
---@field correction_magnitude number
---@field active_smoothing_count integer

---@class MatchScreenOptions
---@field formation string?
---@field tactic string?
---@field home TeamData?
---@field away TeamData?
---@field seed integer?
---@field arena_id string?
---@field show_onboarding boolean?
---@field combat_enabled boolean? -- Explicit post-showcase prototype opt-in.
---@field profile "product"|"playtest"?
---@field rollback_lab MatchRollbackLabOptions? -- Explicit development-only opt-in.

---@class MatchScreen : Screen
---@field state MatchState
---@field _combat_state CombatMatchState?
---@field home_color number[]
---@field away_color number[]
---@field home_name string
---@field away_name string
---@field arena ArenaData
---@field _opts MatchScreenOptions
---@field _profile "product"|"playtest"
---@field _onboarding MatchOnboardingState
---@field _kickoff_banner number
---@field _last_scoring_team "home"|"away"?
---@field _shoot_held_prev boolean
---@field _pass_held_prev boolean
---@field _lob_latch boolean
---@field _pass boolean
---@field _switch boolean
---@field _dash boolean
---@field _dodge boolean
---@field _space_held_prev boolean  -- tracks Space held off the ball for jockey stance
---@field _equipment_keyboard_down boolean
---@field _equipment_gamepad_down boolean
---@field _equipment_abstract_down boolean
---@field _equipment_pressed boolean
---@field _equipment_released boolean
---@field _clock FixedClockState
---@field _input_adapter MatchInputAdapterState
---@field _frame_events MatchEvent[] -- Events produced by every simulation tick in the latest render update.
---@field _rollback_lab RollbackPlayableLab?
---@field _rollback_debug MatchRollbackDebugModel?
---@field _rollback_outputs RollbackTickOutput[]
---@field _rollback_event_diffs RollbackEventDiff[]
---@field _rollback_confirmed_steps RollbackEventStep[]
---@field _rollback_corrections RollbackPlayableCorrection[]
---@field _presentation_full_time boolean
---@field _pending_confirmed_kickoff boolean
---@field _confirmed_lifecycle_ids table<string, boolean>
---@field _render_smoothing CorrectionSmoothingState
---@field _render_pose CorrectionSmoothingPose
---@field _replay_state ReplayFrame?
local Match = {}
Match.__index = Match

---@param self MatchScreen
---@return boolean
local function match_is_over(self)
    if self._rollback_debug then
        local status = self._rollback_debug.status
        return status ~= "active" and status ~= "settling"
    end
    return self.state.finished
end

---@param target any[]
---@param source any[]
local function append_values(target, source)
    for _, value in ipairs(source) do
        target[#target + 1] = value
    end
end

---@param self MatchScreen
local function refresh_smoothing_debug(self)
    if self._rollback_debug == nil then
        return
    end
    local diagnostics = correction_smoothing.diagnostics(self._render_smoothing)
    self._rollback_debug.correction_magnitude = diagnostics.maximum_magnitude
    self._rollback_debug.active_smoothing_count = diagnostics.active_count
end

---@param self MatchScreen
---@param reset_view boolean?
local function clear_render_smoothing(self, reset_view)
    self._render_smoothing = correction_smoothing.clear(self._render_smoothing, self.state)
    self._render_pose = correction_smoothing.pose(self._render_smoothing)
    if reset_view ~= false then
        view_state.reset()
    end
    refresh_smoothing_debug(self)
end

---@param self MatchScreen
---@param dt number
---@param corrected boolean
---@param lifecycle_reset boolean
---@param update_view boolean
local function update_render_smoothing(self, dt, corrected, lifecycle_reset, update_view)
    if lifecycle_reset then
        clear_render_smoothing(self, update_view)
    elseif corrected then
        self._render_smoothing =
            correction_smoothing.reconcile(self._render_smoothing, self.state, dt)
        self._render_pose = correction_smoothing.pose(self._render_smoothing)
        refresh_smoothing_debug(self)
    else
        self._render_smoothing =
            correction_smoothing.advance(self._render_smoothing, self.state, dt)
        self._render_pose = correction_smoothing.pose(self._render_smoothing)
        refresh_smoothing_debug(self)
    end
    if update_view then
        view_state.update(self.state.players, dt, self._render_pose)
    end
end

---@param debug MatchRollbackDebugModel?
---@return boolean
local function synchronization_failed(debug)
    if debug == nil then
        return false
    end
    return debug.status ~= "active" and debug.status ~= "settling" and debug.status ~= "converged"
end

---@param self MatchScreen
local function reconcile_rollback_replay(self)
    local lab = assert(self._rollback_lab)
    local first_boundary = nil
    for _, correction in ipairs(self._rollback_corrections) do
        first_boundary = first_boundary and math.min(first_boundary, correction.replaced_from_tick)
            or correction.replaced_from_tick
    end
    if first_boundary then
        replay.truncate_from(first_boundary)
        effects.reset_trail()
    end
    for _, output in ipairs(self._rollback_outputs) do
        first_boundary = first_boundary and math.min(first_boundary, output.start_boundary)
            or output.start_boundary
    end
    if first_boundary == nil then
        return
    end
    for boundary = first_boundary, self.state.input_tick do
        local lookup = rollback_playable_lab.snapshot(lab, boundary)
        if lookup.status == "present" or lookup.status == "retained" then
            replay.record_boundary(
                boundary,
                match_snapshot.restore(assert(lookup.snapshot, "replay snapshot is missing"))
            )
        end
    end
end

---@param self MatchScreen
local function finish_replay(self)
    replay.stop()
    self._replay_state = nil
    effects.reset_visuals()
    clear_render_smoothing(self)
    view_state.update(self.state.players, 0, self._render_pose)
    if self._pending_confirmed_kickoff and not self._presentation_full_time then
        self._kickoff_banner = 1.15
    end
    self._pending_confirmed_kickoff = false
end

---@param self MatchScreen
---@param dt number
---@return boolean was_active
local function advance_rollback_replay(self, dt)
    if self._rollback_lab == nil or not replay.active() then
        return false
    end
    self._replay_state = replay.step(dt)
    if self._replay_state then
        view_state.update(self._replay_state.players, dt)
    else
        finish_replay(self)
    end
    return true
end

-- Consume one confirmed lifecycle record exactly once. The screen-owned ledger
-- gates every associated side effect; audio deduplication is a second guard,
-- not the authority that decides whether replay/HUD/result state may restart.
---@param event RollbackWrappedLifecycleEvent
---@return boolean consumed
function Match:consume_confirmed_lifecycle(event)
    if self._confirmed_lifecycle_ids[event.id] then
        return false
    end
    self._confirmed_lifecycle_ids[event.id] = true
    audio.consume_confirmed(event)
    local payload = event.payload
    if payload.kind == "goal" then
        local scoring_team = assert(payload.team, "confirmed goal is missing its scoring team")
        self._last_scoring_team = scoring_team
        if replay.start_at(scoring_team, event.tick) then
            effects.reset()
            clear_render_smoothing(self)
            self._replay_state = nil
        end
    elseif payload.kind == "kickoff" then
        self._pending_confirmed_kickoff = true
        if not replay.active() and not self._presentation_full_time then
            self._kickoff_banner = 1.15
            self._pending_confirmed_kickoff = false
        end
    elseif payload.kind == "full_time" then
        self._presentation_full_time = true
        self._pending_confirmed_kickoff = false
    end
    return true
end

-- Route rollback action deltas through the currently drawn timeline. Live
-- effects are consumed reversibly but never spawned over confirmed past
-- footage.
---@param diff RollbackEventDiff
---@return boolean presented
function Match:consume_rollback_event_diff(diff)
    if self._rollback_lab ~= nil and replay.active() then
        effects.discard_event_diff(diff)
        return false
    end
    effects.apply_event_diff(diff)
    return true
end

-- Publish one stable rollback step to presentation consumers. Match and combat
-- action records share the same confirmation ledger; lifecycle records retain
-- their screen-owned transitions on top.
---@param step RollbackEventStep
---@return integer consumed_count
function Match:consume_confirmed_step(step)
    local consumed_count = 0
    for _, event in ipairs(step.match_events) do
        effects.confirm_event(event.id)
        if audio.consume_confirmed(event) then
            consumed_count = consumed_count + 1
        end
    end
    for _, event in ipairs(step.combat_events or {}) do
        effects.confirm_event(event.id)
        if audio.consume_confirmed(event) then
            consumed_count = consumed_count + 1
        end
    end
    for _, event in ipairs(step.lifecycle_events) do
        if self:consume_confirmed_lifecycle(event) then
            consumed_count = consumed_count + 1
        end
    end
    return consumed_count
end

---@param self MatchScreen
local function consume_rollback_presentation(self)
    for _, diff in ipairs(self._rollback_event_diffs) do
        self:consume_rollback_event_diff(diff)
    end
    for _, step in ipairs(self._rollback_confirmed_steps) do
        self:consume_confirmed_step(step)
    end
    if self._pending_confirmed_kickoff and not replay.active() then
        self._kickoff_banner = 1.15
        self._pending_confirmed_kickoff = false
    end
end

---@param opts MatchScreenOptions?
---@return MatchScreen
function Match.new(opts)
    local self = setmetatable({}, Match)
    self._opts = opts or {}
    self._profile = self._opts.profile or "playtest"
    assert(
        self._profile ~= "product" or self._opts.rollback_lab == nil,
        "rollback laboratory mode is development-only"
    )
    if self._profile == "playtest" and love.window then
        tuning_panel.load() -- playtest tuning persists across runs (F1 panel)
    end
    local home = self._opts.home or teams.nebula
    local away = self._opts.away or teams.orion
    self.home_color = home.color
    self.away_color = away.color
    self.home_name = home.name
    self.away_name = away.name
    self.arena = assert(arenas[self._opts.arena_id or "helios_crown"], "unknown match arena")
    self:restart()
    return self
end

-- Start a fresh match with the same pre-match choices (formation/tactic).
-- Used at construction and for the full-time rematch.
function Match:restart()
    local home = self._opts.home or teams.nebula
    local away = self._opts.away or teams.orion
    local rollback_options = self._opts.rollback_lab
    local ownership = rollback_options and sim_match.ownership_for_teams(home, away) or nil
    local initial ---@type MatchState
    local initial_combat ---@type CombatMatchState?
    if rollback_options and rollback_options.initial_snapshot then
        initial, initial_combat = match_snapshot.restore(rollback_options.initial_snapshot)
    else
        initial = sim_match.new({
            home = home,
            away = away,
            field = { w = FIELD_W, h = FIELD_H },
            home_formation = self._opts.formation,
            tactic = self._opts.tactic and tactics[self._opts.tactic] or nil,
            seed = self._opts.seed,
            duration = rollback_options and rollback_options.duration or nil,
            max_goals = rollback_options and rollback_options.max_goals or nil,
            input_ownership = ownership,
        })
        if self._opts.combat_enabled then
            initial_combat = combat_sim.new_state(initial)
        end
    end
    assert(
        (initial_combat ~= nil) == (self._opts.combat_enabled == true),
        initial_combat ~= nil and "combat-bearing rollback snapshots require combat_enabled = true"
            or "combat-enabled matches require a CombatMatchState companion"
    )
    if rollback_options then
        self._rollback_lab =
            rollback_playable_lab.new(match_snapshot.capture(initial, initial_combat), {
                local_slot = rollback_options.local_slot,
                profile_name = rollback_options.profile_name,
                profile = rollback_options.network_profile,
                network_seed = rollback_options.network_seed,
                bot_seed = rollback_options.bot_seed,
                max_rollback_ticks = rollback_options.max_rollback_ticks,
                settlement_ticks = rollback_options.settlement_ticks,
            })
        self.state, self._combat_state =
            match_snapshot.restore(rollback_playable_lab.current_snapshot(self._rollback_lab))
        local debug = rollback_playable_lab.debug_model(self._rollback_lab)
        ---@cast debug MatchRollbackDebugModel
        self._rollback_debug = debug
    else
        self._rollback_lab = nil
        self._rollback_debug = nil
        self.state = initial
        self._combat_state = initial_combat
    end
    self._render_smoothing = correction_smoothing.new(self.state)
    self._render_pose = correction_smoothing.pose(self._render_smoothing)
    self._pass, self._switch, self._dash, self._dodge = false, false, false, false
    self._shoot_held_prev = false
    self._pass_held_prev = false
    self._lob_latch = false
    self._space_held_prev = false
    self._equipment_keyboard_down = false
    self._equipment_gamepad_down = false
    self._equipment_abstract_down = false
    self._equipment_pressed = false
    self._equipment_released = false
    self._clock = fixed_clock.new()
    self._input_adapter = match_input_adapter.new()
    self._frame_events = {}
    self._rollback_outputs = {}
    self._rollback_event_diffs = {}
    self._rollback_confirmed_steps = {}
    self._rollback_corrections = {}
    self._presentation_full_time = false
    self._pending_confirmed_kickoff = false
    self._confirmed_lifecycle_ids = {}
    self._last_score = 0
    self._last_home = 0
    self._last_scoring_team = nil
    self._kickoff_banner = 1.15
    self._onboarding = onboarding.new(self._opts.show_onboarding == true, self._combat_state ~= nil)
    self._replay_state = nil
    replay.reset()
    if self._rollback_lab then
        replay.record_boundary(0, self.state, self._combat_state)
    end
    view_state.reset()
    effects.reset()
    audio.load()
    audio.reset()
    refresh_smoothing_debug(self)
end

-- ScreenStack calls this before discarding the laboratory. Presentation state
-- is cleared explicitly even though the authoritative match remains untouched.
function Match:teardown()
    self._replay_state = nil
    clear_render_smoothing(self)
end

-- Product/offline matches retain their state-edge behavior. Rollback matches
-- expose full time only after the stable lifecycle step is confirmed.
---@return boolean
function Match:full_time_confirmed()
    if self._rollback_lab then
        return self._presentation_full_time
    end
    return self.state.finished
end

-- A confirmed goal sequence owns the screen until it finishes or the player
-- explicitly skips it. Result navigation must not replace an active replay.
---@return boolean
function Match:result_completion_blocked()
    return self._rollback_lab ~= nil and replay.active()
end

---@param self MatchScreen
---@return boolean
local function equipment_down(self)
    return self._equipment_keyboard_down
        or self._equipment_gamepad_down
        or self._equipment_abstract_down
end

---@param self MatchScreen
---@param source "keyboard"|"gamepad"?
---@param pressed boolean
local function record_equipment_transition(self, source, pressed)
    local before = equipment_down(self)
    if source == "keyboard" then
        self._equipment_keyboard_down = pressed
    elseif source == "gamepad" then
        self._equipment_gamepad_down = pressed
    else
        self._equipment_abstract_down = pressed
    end
    local after = equipment_down(self)
    if before ~= after then
        if after then
            self._equipment_pressed = true
        else
            self._equipment_released = true
        end
    end
end

---@param evt InputEvent
function Match:event(evt)
    if evt.kind == "action" then
        if replay.active() then
            if evt.action == "confirm" or evt.action == "pass_switch" then
                finish_replay(self)
            end
            return
        end
        if match_is_over(self) then
            if self._profile == "playtest" and evt.action == "confirm" then
                self:restart()
            end
            return
        end
        if evt.action == "equipment" then
            record_equipment_transition(self, evt.source, evt.pressed ~= false)
            return
        end
        local carrying = self.state.owner == self.state.controlled
        if evt.action == "pass_switch" and not carrying then
            self._switch = true
        elseif evt.action == "juke" then
            self._dodge = true
        end
        return
    end
    if evt.kind ~= "key" then
        return
    end
    if self._rollback_lab and evt.key == "r" then
        self:restart()
        return
    end
    -- A confirmed goal replay remains skippable after the rollback simulation
    -- has converged. The next Return may then use the normal rematch behavior.
    if replay.active() then
        if evt.key == "space" or evt.key == "return" or evt.key == "k" then
            finish_replay(self)
        end
        return
    end
    -- After full time only the rematch keys act; match inputs stop buffering.
    if match_is_over(self) then
        if self._profile == "playtest" and (evt.key == "r" or evt.key == "return") then
            self:restart()
        end
        return
    end
    -- Tuning panel (playtest): F1 toggles; while open it owns the keyboard
    -- and the match is paused (see update).
    if self._profile == "playtest" and evt.key == "f1" then
        tuning_panel.toggle()
        return
    end
    if self._profile == "playtest" and tuning_panel.open then
        tuning_panel.key(evt.key, love.keyboard.isDown("lshift", "rshift"))
        return
    end
    -- Contextual actions: the same key means the natural thing for the moment.
    -- Space = shoot with the ball (polled: hold to charge) / jockey off the ball
    --   (hold to contain; release fires the poke — mirroring shoot's hold/release).
    -- K = pass with the ball / switch player without it.
    local carrying = self.state.owner == self.state.controlled
    if evt.key == "k" then
        -- Passing is polled while carrying (hold to charge the range, release
        -- to play it); off the ball K switches player on the press.
        if not carrying then
            self._switch = true
        end
    elseif evt.key == "c" then
        self._dodge = true
    elseif self._profile == "playtest" and evt.key == "b" then
        bloom.config.enabled = not bloom.config.enabled
    elseif evt.key == "m" then
        audio.toggle_mute()
    end
end

---@return love.Joystick?
local function active_gamepad()
    if not love.joystick or not love.joystick.getJoysticks then
        return nil
    end
    local joysticks = love.joystick.getJoysticks()
    return joysticks[1]
end

---@param button love.GamepadButton
---@return boolean
local function gamepad_down(button)
    local joystick = active_gamepad()
    return joystick ~= nil and joystick:isGamepadDown(button)
end

---@return Vec2
local function read_move_axis()
    local x, y = 0, 0
    if love.keyboard.isDown("left", "a") then
        x = x - 1
    end
    if love.keyboard.isDown("right", "d") then
        x = x + 1
    end
    if love.keyboard.isDown("up", "w") then
        y = y - 1
    end
    if love.keyboard.isDown("down", "s") then
        y = y + 1
    end
    local joystick = active_gamepad()
    if joystick then
        local gx = joystick:getGamepadAxis("leftx")
        local gy = joystick:getGamepadAxis("lefty")
        if math.abs(gx) >= 0.2 then
            x = x + gx
        end
        if math.abs(gy) >= 0.2 then
            y = y + gy
        end
        if joystick:isGamepadDown("dpleft") then
            x = x - 1
        end
        if joystick:isGamepadDown("dpright") then
            x = x + 1
        end
        if joystick:isGamepadDown("dpup") then
            y = y - 1
        end
        if joystick:isGamepadDown("dpdown") then
            y = y + 1
        end
    end
    return Vec2.new(x, y)
end

---@param dt number
function Match:update(dt)
    -- These are one-render handoff deltas. Clear them even when this update is
    -- paused or terminal so presentation reconciliation can never consume a
    -- prior render's batch twice.
    self._rollback_outputs = {}
    self._rollback_event_diffs = {}
    self._rollback_confirmed_steps = {}
    self._rollback_corrections = {}
    if self._profile == "playtest" and tuning_panel.open then
        return -- paused for tuning: tweak, close, resume
    end
    -- Slow-motion goal replay: the sim freezes while the buffer plays back
    -- through the normal renderer (same camera).
    if self._rollback_lab == nil and replay.active() then
        self._replay_state = replay.step(dt)
        if self._replay_state then
            view_state.update(self._replay_state.players, dt)
            effects.update(self._replay_state, dt)
        else
            finish_replay(self)
        end
        audio.tick(dt)
        return
    end
    if match_is_over(self) then
        if not advance_rollback_replay(self, dt) then
            clear_render_smoothing(self)
        end
        audio.tick(dt)
        return
    end
    self._kickoff_banner = math.max(0, self._kickoff_banner - dt)
    local equipment_before_poll = equipment_down(self)
    self._equipment_keyboard_down = love.keyboard.isDown("j")
    self._equipment_gamepad_down = gamepad_down("b")
    local equipment_held = equipment_down(self)
    if equipment_before_poll ~= equipment_held then
        if equipment_held then
            self._equipment_pressed = true
        else
            self._equipment_released = true
        end
    end
    -- Space reads as "shoot" while carrying (hold to charge, release to fire);
    -- off the ball it is "jockey" while held and fires the poke on release
    -- — mirroring the on-ball hold/release pattern so muscle memory transfers.
    local carrying = self.state.owner == self.state.controlled
    local shoot_down = love.keyboard.isDown("space") or gamepad_down("a")
    local held = carrying and shoot_down
    local space_down_offball = (not carrying) and shoot_down
    local k_held = carrying and (love.keyboard.isDown("k") or gamepad_down("x"))
    -- Jockey release: Space was held last frame off the ball and is now up.
    if self._space_held_prev and not space_down_offball and not carrying then
        self._dash = true
    end
    -- L is a modifier, and fingers naturally lift it a frame before the action
    -- key on release. LATCH it across the hold so "L + K/Space" always lofts,
    -- even when L comes up first.
    local l_down = love.keyboard.isDown("l") or gamepad_down("y")
    local firing = (self._shoot_held_prev and not held) or (self._pass_held_prev and not k_held)
    local lob = l_down or (firing and self._lob_latch) or false
    if held or k_held then
        self._lob_latch = self._lob_latch or l_down
    else
        self._lob_latch = false
    end
    local move = read_move_axis()
    ---@type MatchInput
    local frame_input = {
        move = move,
        shoot = self._shoot_held_prev and not held, -- fire on release
        shoot_held = held,
        pass = self._pass_held_prev and not k_held, -- pass fires on release too
        pass_held = k_held,
        switch = self._switch,
        dash = self._dash,
        dodge = self._dodge,
        lob = lob,
        sprint = love.keyboard.isDown("lshift", "rshift") or gamepad_down("leftshoulder"),
        jockey = space_down_offball, -- hold Space off the ball: slow shadow stance
        aerial_strike = space_down_offball,
        aerial_acrobatic = space_down_offball and l_down,
        equipment_held = equipment_held,
        equipment_pressed = self._equipment_pressed,
        equipment_released = self._equipment_released,
    }
    self._shoot_held_prev = held
    self._pass_held_prev = k_held
    self._space_held_prev = space_down_offball
    self._pass, self._switch, self._dash, self._dodge = false, false, false, false
    self._equipment_pressed, self._equipment_released = false, false

    -- Render input is sampled every update, then the adapter holds one-shot
    -- edges until a canonical simulation tick consumes them. A fast display
    -- therefore cannot drop an action merely because this render update makes
    -- zero simulation progress.
    self._input_adapter = match_input_adapter.sample(self._input_adapter, frame_input)
    self._frame_events = {}
    local score_before = self.state.score.home + self.state.score.away
    local kickoff_before = self.state.kickoff_hold
    if self._rollback_lab then
        local lab = assert(self._rollback_lab)
        fixed_clock.advance(self._clock, dt, function(_)
            if not rollback_playable_lab.needs_local_sample(lab) then
                return nil
            end
            local next, tick_input = match_input_adapter.next_tick(self._input_adapter)
            self._input_adapter = next
            return slot_input.to_sample(tick_input)
        end, function(tick, sample)
            local batch = rollback_playable_lab.advance(lab, tick, sample)
            append_values(self._rollback_outputs, batch.outputs)
            append_values(self._rollback_event_diffs, batch.event_diffs)
            append_values(self._rollback_confirmed_steps, batch.confirmed_steps)
            append_values(self._rollback_corrections, batch.corrections)
            self.state, self._combat_state =
                match_snapshot.restore(rollback_playable_lab.current_snapshot(lab))
            return batch.status == "active" or batch.status == "settling"
        end)
        local debug = rollback_playable_lab.debug_model(lab)
        ---@cast debug MatchRollbackDebugModel
        self._rollback_debug = debug
        reconcile_rollback_replay(self)
        consume_rollback_presentation(self)
        if not replay.active() then
            effects.sample_ball(self.state)
        end
    else
        fixed_clock.advance(self._clock, dt, function(_)
            local next, tick_input = match_input_adapter.next_tick(self._input_adapter)
            self._input_adapter = next
            return tick_input
        end, function(_, tick_input)
            replay.record(self.state, self._combat_state) -- Pre-step, so the goal flight remains in the buffer.
            local score_before = self.state.score.home + self.state.score.away
            sim_match.step(self.state, fixed_clock.TICK_SECONDS, tick_input, self._combat_state)
            for _, event in ipairs(self.state.events) do
                self._frame_events[#self._frame_events + 1] = event
            end
            effects.consume(self.state)
            audio.consume(self.state)
            -- Preserve the existing goal beat: stop this catch-up batch as soon as
            -- a live goal is scored, then the render update starts its replay.
            local scored = self.state.score.home + self.state.score.away > score_before
            return not self.state.finished and not scored
        end)
    end

    -- Presentation is never a second simulation authority. It follows normal
    -- render dt even on frames that consume zero or several simulation ticks.
    -- A correction begins at the preceding displayed pose, then sheds only its
    -- render-owned offset. Confirmed replay footage keeps its own view timeline
    -- while hidden live corrections continue to reconcile independently.
    local drawing_rollback_replay = self._rollback_lab ~= nil and replay.active()
    local score_after = self.state.score.home + self.state.score.away
    local lifecycle_reset = score_after ~= score_before
        or (self._rollback_lab == nil and score_after > self._last_score)
        or self.state.kickoff_hold > kickoff_before
        or self.state.finished
        or synchronization_failed(self._rollback_debug)
    update_render_smoothing(
        self,
        dt,
        #self._rollback_corrections > 0,
        lifecycle_reset,
        not drawing_rollback_replay
    )
    if not drawing_rollback_replay then
        effects.tick(dt)
    end
    audio.tick(dt)
    advance_rollback_replay(self, dt)
    local controlled = self.state.players[self.state.controlled]
    local owner = self.state.owner and self.state.players[self.state.owner] or nil
    self._onboarding = onboarding.update(self._onboarding, {
        carrying = self.state.owner == self.state.controlled,
        defending = owner ~= nil and owner.team == "away",
        keeper_holding = self.state.owner == self.state.controlled
            and controlled.is_keeper
            and not controlled.feet_ball,
        moved = move:length() > 0.2 or frame_input.sprint,
        shot = frame_input.shoot or frame_input.shoot_held,
        passed = frame_input.pass or frame_input.pass_held,
        defended = frame_input.jockey or frame_input.dash or frame_input.switch,
        equipment_available = self._combat_state ~= nil
            and assert(self._combat_state.players[self.state.controlled]).family_id ~= nil,
        equipment_used = frame_input.equipment_pressed,
    }, dt)

    -- A goal just went in (score edge, match still live): celebrate, then roll
    -- the replay. Which side scored picks the celebrating team.
    if self._rollback_lab == nil then
        local sh, sa = self.state.score.home, self.state.score.away
        if sh + sa > self._last_score and not self.state.finished then
            local scoring_team = sh > self._last_home and "home" or "away"
            self._last_scoring_team = scoring_team
            if replay.start(scoring_team) then
                effects.reset() -- the scene jumps back in time; drop live particles
                view_state.reset()
                self._replay_state = nil
            end
        end
        self._last_home, self._last_score = sh, sh + sa
    end
end

---@return BroadcastPhase?
function Match:broadcast_phase()
    if replay.celebrating() then
        return "goal"
    end
    if replay.active() then
        return "replay"
    end
    if self:full_time_confirmed() then
        return "full_time"
    end
    if self._kickoff_banner > 0 then
        return "kickoff"
    end
    return nil
end

---@param s MatchState
---@param vp { w: number, h: number }
---@param combat_state CombatMatchState?
function Match:draw_frame(s, vp, combat_state)
    local combat_model = combat_presentation.model(s, combat_state)
    -- World: 2.5D perspective pitch + billboard players.
    pitch.draw(s, vp, {
        home_color = self.home_color,
        away_color = self.away_color,
        arena = self.arena,
        arena_pulse = math.min(1, self._kickoff_banner),
        render_pose = s == self.state and self._render_pose or nil,
        combat = combat_model,
    })

    local phase = self:broadcast_phase()
    local tactic = self._opts.tactic and tactics[self._opts.tactic] or nil
    local model = match_hud.model(self.state, {
        home_name = self.home_name,
        away_name = self.away_name,
        arena_name = self.arena.name,
        arena_location = self.arena.location,
        tactic_name = tactic and tactic.name or "Balanced",
        formation_name = self._opts.formation or (self._opts.home or teams.nebula).formation,
        prompt = onboarding.prompt(self._onboarding),
        phase = phase,
        scoring_team = self._last_scoring_team,
        combat_enabled = combat_model.enabled,
    })
    match_hud_render.draw(model, vp)

    if self._profile == "playtest" then
        tuning_panel.draw(vp)
    end
end

---@param vp { w: number, h: number }
function Match:draw_rollback_overlay(vp)
    local model = self._rollback_debug
    if model == nil then
        return
    end
    local lines = {
        ("ROLLBACK LAB  %s  [%s]"):format(model.profile, model.status),
        ("ref/client/net  %d / %d / %d"):format(
            model.reference_tick,
            model.current_tick,
            model.transport_tick
        ),
        ("confirmed in/out  %d / %d"):format(
            model.confirmed_input_tick,
            model.confirmed_output_tick
        ),
        ("rollback %d  depth %d/%d  resim %d"):format(
            model.rollback_count,
            model.latest_rollback_depth,
            model.max_rollback_depth,
            model.resimulated_ticks
        ),
        ("smoothing %d  max %.1f wu"):format(
            model.active_smoothing_count,
            model.correction_magnitude
        ),
        ("predicted %d rows / %d ticks"):format(
            model.predicted_slot_samples,
            model.predicted_ticks
        ),
        ("snapshots %d / %d B  pending %d"):format(
            model.retained_snapshot_count,
            model.retained_snapshot_bytes,
            model.network_pending
        ),
        ("net sent/delivered/lost  %d / %d / %d"):format(
            model.network_counters.sent,
            model.network_counters.delivered,
            model.network_counters.independent_lost + model.network_counters.burst_lost
        ),
        ("net peak queue/refs  %d / %d"):format(
            model.network_high_water.peak_pending_envelopes,
            model.network_high_water.peak_pending_record_references
        ),
        ("convergence %s @ %d"):format(model.convergence.status, model.convergence.boundary),
    }
    local width = math.min(360, vp.w - 24)
    local height = #lines * 16 + 16
    love.graphics.push("all")
    love.graphics.setColor(0.015, 0.025, 0.06, 0.88)
    love.graphics.rectangle("fill", 12, 12, width, height, 6, 6)
    love.graphics.setColor(0.35, 0.95, 1, 1)
    for index, line in ipairs(lines) do
        love.graphics.print(line, 22, 16 + index * 16)
    end
    love.graphics.pop()
end

function Match:draw()
    local vp = { w = love.graphics.getWidth(), h = love.graphics.getHeight() }
    if replay.active() and self._replay_state then
        local replay_state = assert(self._replay_state)
        bloom.draw(function()
            self:draw_frame(replay_state, vp, replay_state._combat_state)
        end)
    else
        bloom.draw(function()
            self:draw_frame(self.state, vp, self._combat_state)
        end)
    end
    self:draw_rollback_overlay(vp)
end

return Match
