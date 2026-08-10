-- The one interface between the simulation and any renderer.
--
-- `render_frame.build` turns a `MatchState` into a flat, engine-free
-- description of ONE drawable frame. Nothing downstream of it needs to know
-- what a `MatchState` is, and nothing in it needs `love`.
--
-- Three rules shape this module, and none of them are style preferences:
--
-- 1. THE BOUNDARY IS CROSSED ONCE PER RENDERED FRAME, IN BATCH. Never per
--    entity, never per tick. Rollback re-simulates up to eight ticks inside a
--    single rendered frame; a per-tick crossing is the thing that would make a
--    non-Lua renderer unaffordable. `build` is therefore one call producing one
--    whole frame.
--
-- 2. PER-ENTITY DATA IS STRUCTURE-OF-ARRAYS. `frame.players` is a set of
--    parallel arrays indexed by roster slot, not an array of tables. An array
--    of tables cannot be written into a shared buffer without touching every
--    field individually across the boundary; parallel scalar arrays can be
--    copied wholesale. The optional arrays (`aerial_style`, ...) are typed as
--    `table<integer, T>` because they are sparse: an absent entry is `nil`, and
--    a buffer encoding maps that to a zero enum. Booleans encode as 0/1.
--
-- 3. PRESENTATION-DERIVED STATE STAYS ON THE RENDERER SIDE. Gait, lean, the
--    smoothed on-screen speed (`game.render.view_state`), the correction
--    smoothing state machine (`game.render.correction_smoothing`) and the
--    release follow-through window (`game.render.release_follow`) are NOT
--    simulation and are not derived here. Two of them feed in as explicit
--    inputs (`opts.render_pose`, `opts.kick_follow`) because the frame must
--    report the positions and poses actually shown; their state machines stay
--    where they are.
--
-- The frame splits into a static half and a per-frame half. `RenderFrameRoster`
-- is match-constant (ids, teams, species shape and palette) and crosses once —
-- pass it back in as `opts.roster` so it is not rebuilt. Everything else in
-- `RenderFrame` is rebuilt each frame.
--
-- Versioning borrows the STAMPING convention of `sim.input_frame` and
-- `sim.match_snapshot` -- an integer `VERSION` written into every payload and
-- bumped whenever the shape changes -- and only that half. Those two protocols
-- also hard-assert the version on every read, because they deserialize bytes
-- that came from somewhere else. Nothing validates `frame.version` today: the
-- payload never leaves the process, so there is no foreign producer to reject.
-- The read-side assertion belongs to the first consumer that deserializes this
-- across a boundary (#332), and it is that change's job to add it, not an
-- oversight to be inherited.

local keeper = require("sim.keeper")
local sim_match = require("sim.match")
local possession_transition = require("sim.possession_transition")
local identity = require("render.identity")
local player_pose = require("render.player_pose")

---@alias RenderTeam "home"|"away"
---@alias RenderSpeciesShape "round"|"broad"|"angular"|"cluster"
---@alias RenderChargeKind "shot"|"pass"

-- Match-constant per-player identity. Crosses the boundary once, not per frame.
---@class RenderFrameRoster
---@field version integer -- Exactly render_frame.VERSION.
---@field count integer
---@field ids string[]
---@field names string[]
---@field teams RenderTeam[]
---@field is_keeper boolean[]
---@field radius number[]
---@field species_shape RenderSpeciesShape[]
---@field species_color number[][] -- One rgb triple per slot; static, so nesting is free here.

-- Pitch geometry the renderer draws lines and goals from. Constant for a match,
-- carried on the frame because it is three numbers and a pair of rects.
---@class RenderFrameField
---@field w number
---@field h number
---@field crossbar_h number
---@field penalty_box_depth number
---@field penalty_box_h number
---@field goal_home Rect
---@field goal_away Rect

-- Structure of arrays, indexed 1..count by roster slot. Every array is the same
-- length; the sparse ones (`table<integer, T>`) leave holes where a value does
-- not apply. Timers arrive already normalised to 0..1 so no renderer has to
-- re-derive a duration constant.
---@class RenderFramePlayers
---@field count integer
---@field x number[] -- Displayed world position (correction smoothing applied).
---@field y number[]
---@field facing_x number[]
---@field facing_y number[]
---@field speed number[] -- Locomotion speed in world units/sec, straight off the sim.
---@field pose_id PlayerPoseId[]
---@field pose_priority integer[]
---@field pose_source PlayerPoseSource[]
---@field controlled boolean[]
---@field dashing boolean[]
---@field holding boolean[] -- Keeper carrying the ball in the hands.
---@field dive number[] -- 0..1
---@field dive_dir_x number[]
---@field dive_dir_y number[]
---@field grab number[] -- 0..1
---@field throw number[] -- 0..1
---@field windup number[] -- 0..1
---@field aerial number[] -- 0..1
---@field aerial_jump number[] -- 0..1
---@field aerial_style table<integer, AerialStyle> -- sparse
---@field aerial_outcome table<integer, AerialOutcome> -- sparse

---@class RenderFrameBall
---@field x number -- Displayed world position (correction smoothing applied).
---@field y number
---@field z number -- Height above the pitch.
---@field vx number
---@field vy number
---@field vz number
---@field visible boolean -- False while a keeper holds it in the hands.
---@field landing_x number? -- Ballistic landing point of a lofted loose ball, nil if none.
---@field landing_y number?

---@class RenderFramePossession
---@field owner integer? -- Roster slot holding the ball, nil if loose.
---@field owner_team RenderTeam?
---@field keeper_holds boolean -- Owner is a keeper carrying it in the hands.

-- What the locally controlled player is doing with the input. Distinct from
-- `possession`: a player can be charging a shot without the ball.
---@class RenderFrameControl
---@field controlled integer -- Roster slot.
---@field pass_target integer? -- Roster slot the pass would go to, nil if none.
---@field charge_kind RenderChargeKind? -- nil when nothing is charging.
---@field charge number -- 0..1, 0 when `charge_kind` is nil.

-- Scoreboard/identity facts a HUD needs, all derived from the simulation.
-- Match metadata (team names, arena, tactic) is authored content supplied by the
-- screen, not per-frame simulation state, so it stays out of the payload.
---@class RenderFrameHud
---@field home_score integer
---@field away_score integer
---@field time_left number
---@field finished boolean
---@field possession_team RenderTeam? -- nil while the ball is loose.
---@field controlled integer
---@field controlled_id string
---@field controlled_team RenderTeam
---@field controlled_is_keeper boolean
---@field controlled_owns_ball boolean
---@field controlled_stamina number -- 0..1
---@field species_shape RenderSpeciesShape
---@field species_color number[]

-- Optional booleans are TRI-STATE, not sparse. A sparse boolean array cannot be
-- decoded from the payload alone: absent ("this event kind does not report it")
-- and `false` ("it reports it, and the answer is no") would both encode to 0,
-- and both states genuinely occur -- `sim/match.lua` sets `on_target = false`
-- explicitly, `sim/aerial.lua` computes `jumping` as a real boolean. Kind alone
-- does not disambiguate either: a released outfield shot carries `on_target`
-- while a keeper's distribution kick, also `kind == "shot"`, does not.
---@alias RenderFrameTriState integer -- 0 = not reported, 1 = false, 2 = true.

-- This frame's discrete match events, flattened. This is the effect-trigger
-- channel: a renderer spawns particles, shakes and audio cues from it without
-- ever seeing a `MatchEvent` table.
--
-- The non-boolean optional arrays are sparse: an absent entry is `nil`, which a
-- buffer encoding maps to a zero enum. Which kinds report which field:
--
--   save_style                  catch, parry
--   keeper_state, keeper_depth  shot (released, keeper threatened), tip, catch, parry
--   shot_type, on_target        shot (released outfield shot only)
--   style, outcome,             header, volley, bicycle, reception
--     jumping, difficulty
--   player, slot                every kind whose emitter attributes one; a
--                               deflection or an unattributed event has neither
--
-- That table is documentation, not the decoder. `player`/`slot` being `nil` and
-- the tri-state booleans being 0 are each self-describing on their own.
---@class RenderFrameEvents
---@field count integer
---@field kind MatchEventKind[]
---@field x number[]
---@field y number[]
---@field player table<integer, string> -- sparse; roster id
---@field slot table<integer, integer> -- sparse; roster slot for `player`
---@field save_style table<integer, SaveStyle> -- sparse
---@field style table<integer, AerialStyle> -- sparse
---@field outcome table<integer, AerialOutcome> -- sparse
---@field difficulty table<integer, number> -- sparse
---@field shot_type table<integer, KeeperShotType> -- sparse
---@field keeper_state table<integer, KeeperBehaviorState> -- sparse
---@field keeper_depth table<integer, number> -- sparse
---@field jumping RenderFrameTriState[] -- dense; see RenderFrameTriState
---@field on_target RenderFrameTriState[] -- dense; see RenderFrameTriState

---@class RenderFrame
---@field version integer -- Exactly render_frame.VERSION.
---@field roster RenderFrameRoster
---@field field RenderFrameField
---@field players RenderFramePlayers
---@field ball RenderFrameBall
---@field possession RenderFramePossession
---@field control RenderFrameControl
---@field hud RenderFrameHud
---@field events RenderFrameEvents
---@field combat CombatPresentationModel? -- Combat telegraphs, carried unflattened (see below).

-- Inputs the frame cannot derive from `MatchState` alone.
---@class RenderFrameOptions
---@field roster RenderFrameRoster? -- Reuse a roster built earlier for this match.
---@field render_pose CorrectionSmoothingPose? -- Displayed positions from correction smoothing.
---@field events MatchEvent[]? -- Frame event batch; defaults to `state.events`.
---@field kick_follow table<string, boolean>? -- Renderer-owned release follow-through windows.
---@field combat CombatPresentationModel? -- Combat telegraph model for this frame.

---@class RenderFrameModule
local render_frame = {}

render_frame.VERSION = 1

-- Pose-timer normalisers. These are presentation eases, not simulation
-- durations: they only decide how fast a pose relaxes back to neutral.
local DIVE_EASE = 0.3
local GRAB_EASE = 0.25
local THROW_EASE = 0.25
local WINDUP_EASE = 0.15
local AERIAL_EASE = 0.22
local AERIAL_EASE_BICYCLE = 0.6
local AERIAL_EASE_JUMP = 0.35
local AERIAL_EASE_CONTROL = 0.18

-- Loose-ball ballistics for the landing reticle. The solve is presentation
-- (where will this cross come down?), so it lives here rather than making the
-- simulation compute something it never uses -- but it falls at the simulation's
-- own gravity, read from `sim.match`, so the two can never drift apart.
local BALL_GRAVITY = sim_match.GRAVITY
local RETICLE_MIN_HEIGHT = 20
local RETICLE_MIN_TIME = 0.05
local RETICLE_MAX_TIME = 3

-- Absent, false and true have to survive the crossing as three distinct values.
---@param value boolean?
---@return RenderFrameTriState
local function tri_state(value)
    if value == nil then
        return 0
    end
    return value and 2 or 1
end

---@param timer number?
---@param ease number
---@return number
local function eased(timer, ease)
    local value = timer or 0
    if value <= 0 then
        return 0
    end
    return math.min(1, value / ease)
end

---@param player MatchPlayer
---@return number
local function aerial_ease(player)
    if player.aerial_style == "bicycle" then
        return AERIAL_EASE_BICYCLE
    elseif (player.aerial_jump or 0) > 0 then
        return AERIAL_EASE_JUMP
    elseif player.aerial_style == "leg_control" or player.aerial_style == "chest_control" then
        return AERIAL_EASE_CONTROL
    end
    return AERIAL_EASE
end

-- Match-constant per-player identity. Build once and pass back in as
-- `opts.roster`: nothing in it can change while a match is running.
---@param state MatchState
---@return RenderFrameRoster
function render_frame.roster(state)
    local ids, names, teams = {}, {}, {}
    local is_keeper, radius, shapes, colors = {}, {}, {}, {}
    for index, player in ipairs(state.players) do
        local presentation =
            assert(identity.for_player(player.id), "missing pitch identity for " .. player.id)
        ids[index] = player.id
        -- The authored presentation name, not `MatchPlayer.name`: it is the one
        -- a HUD actually shows, and it survives a replay frame's partial copy.
        names[index] = presentation.name
        teams[index] = player.team
        is_keeper[index] = player.is_keeper
        radius[index] = player.radius
        shapes[index] = presentation.shape
        colors[index] = presentation.palette
    end
    return {
        version = render_frame.VERSION,
        count = #state.players,
        ids = ids,
        names = names,
        teams = teams,
        is_keeper = is_keeper,
        radius = radius,
        species_shape = shapes,
        species_color = colors,
    }
end

-- The scoreboard half of the payload. Exposed on its own because a broadcast
-- HUD outlives the frame the pitch is drawing: during a goal replay the pitch
-- shows a past frame while the HUD keeps reporting the live match.
---@param state MatchState
---@param roster RenderFrameRoster?
---@return RenderFrameHud
function render_frame.hud(state, roster)
    roster = roster or render_frame.roster(state)
    local controlled = state.players[state.controlled]
    local owner = state.owner and state.players[state.owner] or nil
    return {
        home_score = state.score.home,
        away_score = state.score.away,
        time_left = state.time_left,
        finished = state.finished,
        possession_team = owner and owner.team or nil,
        controlled = state.controlled,
        controlled_id = controlled.id,
        controlled_team = controlled.team,
        controlled_is_keeper = controlled.is_keeper,
        controlled_owns_ball = state.owner == state.controlled,
        controlled_stamina = math.max(0, math.min(1, controlled.sprint_meter)),
        species_shape = roster.species_shape[state.controlled],
        species_color = roster.species_color[state.controlled],
    }
end

---@param state MatchState
---@return RenderFrameField
local function build_field(state)
    return {
        w = state.field.w,
        h = state.field.h,
        crossbar_h = sim_match.CROSSBAR_H,
        penalty_box_depth = sim_match.PENALTY_BOX.depth,
        penalty_box_h = sim_match.PENALTY_BOX.h,
        goal_home = state.goal_home,
        goal_away = state.goal_away,
    }
end

-- The counter-press window is what separates a presser shepherding the carrier
-- from one hunting it at full speed: `sim.match` exempts a counter-pressing
-- presser from the contain slowdown and its ball-facing lock, so presentation
-- must not claim contain there either. Read once per team, not once per player.
---@param state MatchState
---@return table<RenderTeam, boolean>
local function counterpressing_teams(state)
    local owner_team = state.owner and state.players[state.owner].team or nil
    local out = {}
    for _, team in ipairs({ "home", "away" }) do
        out[team] = possession_transition.phase(
            state.transition,
            team,
            owner_team,
            state.transition_windows[team]
        ) == "counterpress"
    end
    return out
end

-- A tip is a keeper event that overrides the dive direction for one frame, so
-- it resolves here and the renderer never scans the event batch for it.
---@param events MatchEvent[]
---@return table<string, MatchEvent>
local function tip_events_by_player(events)
    local tips = {}
    for _, event in ipairs(events) do
        if event.kind == "tip" and event.player then
            tips[event.player] = event
        end
    end
    return tips
end

-- A KEEPER IS NOT DRAWN FACING ITS OWN DIVE (#449).
--
-- `MatchPlayer.facing` serves two jobs that had never been separated: it is
-- the direction the body is DRAWN pointing, and it is the aim the simulation
-- reads to decide who receives a keeper's throw (`sim/match.lua`'s
-- `keeper_throw` / `select_throw_target`). `move_offball_keeper` points it
-- along the dive while `dive_timer` runs, which is defensible for the second
-- job and wrong for the first — so the split happens here, at the
-- sim-to-renderer boundary, exactly where AGENTS.md §2 puts
-- presentation-derived state. The simulation keeps its own value untouched;
-- nothing downstream of `sim/match.lua` changes.
--
-- WHY IT IS NOT MERELY UNTIDY. `launch_dive` builds `dive_target` as
-- `Vec2.new(k.pos.x, y_cross)`, so `to_cross.x` is exactly 0 (IEEE-754
-- `a - a`) and `normalized()` keeps it exactly 0: a keeper's `dive_dir` is
-- ALWAYS (0, ±1), and the direction the dive branch writes to `facing` is the
-- same lateral unit vector. The rig decides which side a save rolls to from
-- the 2D cross product of those two (`game/render/rig3d/action_pose.lua`'s
-- `lateralSign`), and two exactly-parallel vectors have a zero cross product
-- — so the whole overlay, roll and travel together, was skipped for the large
-- majority of save frames. It was not even skipped consistently: locomotion
-- leaves a velocity-derived `facing` for the one tick before the dive branch
-- overwrites it, so a save that opened with a lean lost it partway through.
--
-- No count appears here on purpose. The impact tally came from one harness
-- session and nothing committed to this tree re-derives it, so a number in a
-- source comment would read as a measured fact no reader can check. #449 and
-- PR #452 carry the figures, dated and with the method stated.
--
-- WHY THE GOAL-LINE NORMAL rather than the facing latched at launch: the
-- latched value is whatever locomotion last left, and it can point back into
-- the keeper's own goal. A keeper faces up the pitch. Taken from the defended
-- goal's own rect rather than from the team name, so a side swap carries it.
--
-- WHY ALL THREE WINDOWS: the dive, its get-up recovery (same `lateralSign`,
-- same uncleared `dive_dir`, keeper flat on the floor so `facing` still holds
-- what the dive branch last wrote) and a tip (whose `dive_dir` this file
-- synthesises as (0, ±1) while `dive_timer` is already zero). One defect in
-- three states, not three defects.
--
-- WHY THIS OVERRIDES `facing_x`/`facing_y` RATHER THAN ADDING A SECOND
-- `drawn_facing_x`/`_y` PAIR. Redefining a field in a payload AGENTS.md §2
-- calls versioned for a future renderer is not free; the separate field was
-- considered and declined. No reader treats the FRAME's `facing` as
-- simulation truth -- rollback snapshots and both replay paths read
-- `MatchPlayer.facing` off the sim state directly, and the observation
-- encoders read the raw sim state too. This is a stateless per-tick
-- derivation from fields `render/` already reads, categorically unlike the
-- stateful presentation state (gait, lean, correction smoothing) §2 requires
-- be passed in as an explicit input -- which is why it may live here at all.
-- And a second field would widen the versioned wire for exactly one consumer.
-- If a reader ever does need the raw simulation aim per frame, that is the
-- moment to add the field.
--
-- PRECONDITION: NOTHING BUT A KEEPER CARRIES A `dive_timer`.
--
-- This function never tests `is_keeper`, and that is a decision rather than an
-- oversight -- the test would be dead code today. `sim/match.lua` sets
-- `dive_timer` nonzero in exactly one place, `launch_dive`, which has exactly
-- two call sites: one inside the keeper save path indexed by `keeper_idx`, and
-- one gated on `dive_delay > 0`, whose only nonzero assignment is
-- `s.players[ki].dive_delay` inside that same keeper save path.
-- `keeper_get_up_timer` is armed in one place too -- the dive-end transition,
-- which a player can only reach by having dived. So both windows imply
-- `is_keeper` by construction, and a guard here would never take its false arm.
--
-- WHAT WOULD BREAK IF THAT CHANGED: give an outfield player a dive and this
-- override reorients it too, drawing it facing up the pitch for the length of
-- that dive and its recovery -- right for a keeper defending a goal line,
-- wrong for anyone else. Bolting an `is_keeper` guard on at that point would
-- merely restore the ORIGINAL degenerate facing for outfield dives, so the fix
-- then is a real decision about what an outfield dive should face.
--
-- Pinned rather than assumed by "only ever gives a keeper a dive timer" in
-- `spec/sim/match_spec.lua` -- with the simulation it constrains, next to
-- `launch_dive`, so the person editing the dive logic meets it; this pointer
-- is the other half of that link. It sweeps stepped matches and goes red on
-- the first tick an outfield player holds either timer. An assertion here
-- would be the wrong shape: a hand-built fixture may legitimately set the
-- field on a non-keeper (the "normalises pose timers" spec does), and such a
-- player does receive the override -- harmless in a fixture, and not a
-- reachable simulation state.
--
-- Mirrors `v2/rust/crates/gc-render/src/frame.rs`'s `drawn_facing`; see that
-- function for the full argument.
---@param state MatchState
---@param player MatchPlayer
---@param tipping boolean
---@return number facing_x
---@return number facing_y
local function drawn_facing(state, player, tipping)
    if player.dive_timer <= 0 and player.keeper_get_up_timer <= 0 and not tipping then
        return player.facing.x, player.facing.y
    end
    local goal = player.team == "home" and state.goal_home or state.goal_away
    -- Into the field of play, away from the goal this keeper defends.
    local inward = (goal.x + goal.w / 2) < (state.field.w / 2) and 1 or -1
    return inward, 0
end

---@param state MatchState
---@param events MatchEvent[]
---@param roster RenderFrameRoster
---@return RenderFrameEvents
local function build_events(state, events, roster)
    local slot_of = {}
    for index = 1, roster.count do
        slot_of[roster.ids[index]] = index
    end
    ---@type RenderFrameEvents
    local out = {
        count = #events,
        kind = {},
        x = {},
        y = {},
        player = {},
        slot = {},
        save_style = {},
        style = {},
        outcome = {},
        jumping = {},
        difficulty = {},
        shot_type = {},
        keeper_state = {},
        keeper_depth = {},
        on_target = {},
    }
    for index, event in ipairs(events) do
        out.kind[index] = event.kind
        out.x[index] = event.x
        out.y[index] = event.y
        out.player[index] = event.player
        out.slot[index] = event.player and slot_of[event.player] or nil
        out.save_style[index] = event.save_style
        out.style[index] = event.style
        out.outcome[index] = event.outcome
        out.jumping[index] = tri_state(event.jumping)
        out.difficulty[index] = event.difficulty
        out.shot_type[index] = event.shot_type
        out.keeper_state[index] = event.keeper_state
        out.keeper_depth[index] = event.keeper_depth
        out.on_target[index] = tri_state(event.on_target)
    end
    return out
end

-- Where a lofted, loose ball will come down. Only for a genuinely airborne ball
-- (a cross or a lob), never a grounded pass, and only when it lands on the
-- pitch inside a readable window.
---@param state MatchState
---@param ball_x number
---@param ball_y number
---@return number?, number?
local function landing_point(state, ball_x, ball_y)
    local height = state.ball_z or 0
    if state.owner ~= nil or height <= RETICLE_MIN_HEIGHT then
        return nil, nil
    end
    local vz = state.ball_vz or 0
    local fall = (vz + math.sqrt(vz * vz + 2 * BALL_GRAVITY * height)) / BALL_GRAVITY
    if fall <= RETICLE_MIN_TIME or fall >= RETICLE_MAX_TIME then
        return nil, nil
    end
    local x = ball_x + state.ball_vel.x * fall
    local y = ball_y + state.ball_vel.y * fall
    if x <= 0 or x >= state.field.w or y <= 0 or y >= state.field.h then
        return nil, nil
    end
    return x, y
end

-- Turn one `MatchState` into one drawable frame. Pure: it reads the state and
-- allocates a new payload, and never mutates anything it was handed.
---@param state MatchState
---@param opts RenderFrameOptions?
---@return RenderFrame
function render_frame.build(state, opts)
    opts = opts or {}
    local roster = opts.roster or render_frame.roster(state)
    local render_pose = opts.render_pose
    local kick_follow = opts.kick_follow
    local combat = opts.combat
    local events = opts.events or state.events
    local tips = tip_events_by_player(events)
    local counterpressing = counterpressing_teams(state)
    local now = -state.time_left

    -- Held in the HANDS only: a keeper with a back-pass at its feet dribbles a
    -- ground ball like anyone else.
    local owner = state.owner and state.players[state.owner] or nil
    local keeper_holds = owner ~= nil and owner.is_keeper and not owner.feet_ball

    ---@type RenderFramePlayers
    local players = {
        count = roster.count,
        x = {},
        y = {},
        facing_x = {},
        facing_y = {},
        speed = {},
        pose_id = {},
        pose_priority = {},
        pose_source = {},
        controlled = {},
        dashing = {},
        holding = {},
        dive = {},
        dive_dir_x = {},
        dive_dir_y = {},
        grab = {},
        throw = {},
        windup = {},
        aerial = {},
        aerial_jump = {},
        aerial_style = {},
        aerial_outcome = {},
    }

    for index = 1, roster.count do
        local player = state.players[index]
        -- Displayed position: correction smoothing has already decided where
        -- this player is shown. Every distance the SIMULATION reasons about
        -- (smother range, tip direction) keeps reading the authoritative pos.
        local displayed = (render_pose and render_pose.players[player.id]) or player.pos

        local tip = tips[player.id]
        local dive_dir_x, dive_dir_y
        if tip then
            dive_dir_x, dive_dir_y = 0, (tip.y - player.pos.y) >= 0 and 1 or -1
        else
            dive_dir_x, dive_dir_y = player.dive_dir.x, player.dive_dir.y
        end

        -- Displayed facing: a keeper leaning along a dive is DRAWN facing up
        -- the pitch, whatever the simulation's `facing` says (#449). See
        -- `drawn_facing`.
        local facing_x, facing_y = drawn_facing(state, player, tip ~= nil)

        local keeper_context = nil
        if player.is_keeper then
            keeper_context = {
                near_ball = keeper.in_smother_range(player.pos:dist(state.ball)),
                shuffling = player.keeper_state == "base" and player.run_vel ~= nil and math.abs(
                    player.run_vel.y
                ) > 0,
                tip = tip ~= nil,
            }
        end

        -- Outfield pose inputs. The press mode is team-owned simulation state,
        -- the telegraph window is measured against the match clock, and the
        -- follow-through is the render-owned release window supplied by the
        -- caller. Both teams read from the same three sources.
        local outfield_context = nil
        if not player.is_keeper then
            local press = state.outfield_press[player.team]
            outfield_context = {
                now = now,
                containing = press.mode == "contain"
                    and press.presser_index == index
                    and not counterpressing[player.team],
                kick_follow = kick_follow ~= nil and kick_follow[player.id] == true,
            }
        end

        local combat_sample = combat and combat.players[index] or nil
        local pose = player_pose.select(player, combat_sample, keeper_context, outfield_context)

        players.x[index] = displayed.x
        players.y[index] = displayed.y
        players.facing_x[index] = facing_x
        players.facing_y[index] = facing_y
        players.speed[index] = player.run_vel and player.run_vel:length() or 0
        players.pose_id[index] = pose.id
        players.pose_priority[index] = pose.priority
        players.pose_source[index] = pose.source
        players.controlled[index] = index == state.controlled
        players.dashing[index] = player.slide_timer > 0
        players.holding[index] = index == state.owner and player.is_keeper and not player.feet_ball
        players.dive[index] = eased(player.dive_timer, DIVE_EASE)
        players.dive_dir_x[index] = dive_dir_x
        players.dive_dir_y[index] = dive_dir_y
        players.grab[index] = eased(player.grab_timer, GRAB_EASE)
        players.throw[index] = eased(player.throw_timer, THROW_EASE)
        -- The wind-up back-swing is deliberately unclamped: 0 = no windup,
        -- 1 = just committed, and a long charge reads above 1.
        players.windup[index] = player.windup_timer > 0 and player.windup_timer / WINDUP_EASE or 0
        players.aerial[index] = eased(player.aerial_timer, aerial_ease(player))
        players.aerial_jump[index] = player.aerial_jump or 0
        players.aerial_style[index] = player.aerial_style
        players.aerial_outcome[index] = player.aerial_outcome
    end

    local ball_point = (render_pose and render_pose.ball) or state.ball
    local landing_x, landing_y = landing_point(state, ball_point.x, ball_point.y)

    local controlled = state.players[state.controlled]
    local charge_kind, charge = nil, 0
    if controlled.charge > 0.02 then
        charge_kind, charge = "shot", controlled.charge
    elseif controlled.pass_charge > 0.02 then
        charge_kind, charge = "pass", controlled.pass_charge
    end

    return {
        version = render_frame.VERSION,
        roster = roster,
        field = build_field(state),
        players = players,
        ball = {
            x = ball_point.x,
            y = ball_point.y,
            z = state.ball_z or 0,
            vx = state.ball_vel.x,
            vy = state.ball_vel.y,
            vz = state.ball_vz or 0,
            visible = not keeper_holds,
            landing_x = landing_x,
            landing_y = landing_y,
        },
        possession = {
            owner = state.owner,
            owner_team = owner and owner.team or nil,
            keeper_holds = keeper_holds,
        },
        control = {
            controlled = state.controlled,
            pass_target = controlled.pass_target,
            charge_kind = charge_kind,
            charge = charge,
        },
        hud = render_frame.hud(state, roster),
        events = build_events(state, events, roster),
        combat = combat,
    }
end

return render_frame
