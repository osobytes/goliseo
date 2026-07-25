-- Pure 5v5 match simulation. No love, no drawing, no input gathering.
--
-- Home attacks right (scores in the right goal); away attacks left. By default,
-- one home player is `controlled` by the human and everyone else is AI. Fully
-- simulated fixtures can set `human_controlled = false` so every player uses
-- the match AI. Possession is a single `owner` index (nil = loose ball). All
-- state lives in MatchState and `step` advances it deterministically.

local Vec2 = require("core.vec2")
local rng = require("core.rng")
local TUNE = require("sim.tuning").values -- live-tunable knobs (F1 panel)
local aerial = require("sim.aerial")
local keeper = require("sim.keeper")
local stats = require("sim.stats")
local species = require("sim.species")
local placement = require("sim.placement")
local ai = require("sim.ai")
local passing = require("sim.passing")
local fixed_clock = require("sim.fixed_clock")
local input_frame = require("sim.input_frame")
local slot_input = require("sim.slot_input")
local formations = require("data.formations")
local tactics = require("data.tactics")
local player_pool = require("data.players")
local species_pool = require("data.species")

local PLAYER_RADIUS = 12
local BALL_RADIUS = 6
local FRICTION = 1.2 -- fraction of ball speed shed per second
local STICK_AHEAD = PLAYER_RADIUS + BALL_RADIUS -- ball's resting offset at the feet
-- Touch-based dribble, DISCRETE: the carrier KICKS the ball ahead of their run
-- and it rolls free under grass friction — it never tracks the player. The
-- carrier in turn is HOOKED to it: while the touch is beyond playing reach
-- their movement steers back to the ball (see move_players), and only with the
-- ball at the feet do they move freely — the glue that keeps player and ball
-- one unit. When it slows back into playing reach they play the next touch,
-- struck harder than they move (TUNE.DRIBBLE_PUSH) so it runs on ahead again
-- (aim decides the direction: a carrier's facing obeys the stick). Every touch carries
-- direction/weight error scaled down by dribble skill (TUNE.DRIBBLE_ERR): a
-- clean toucher strokes it straight and tight, a sloppy one leathers it
-- off-line — past the control radius (skill-scaled) it's simply loose.
-- DRIBBLE_PUSH / DRIBBLE_ERR / DRIBBLE_TOUCH / DRIBBLE_CONTROL are live-tunable (F1 panel).
local DRIBBLE_LEAD_MIN = STICK_AHEAD -- resting point: the ball sits this far ahead when standing
-- CLOSE CONTROL vs KNOCK-ON: below DRIBBLE_CLOSE x move_speed (standing,
-- walking, an ordinary jog) the ball is kept glued to the feet with soft
-- corrective touches — natural control, nothing to lose. Only ABOVE it (a
-- sprint) does the carrier knock the ball on in discrete kicks, with all the
-- reach and risk that brings. DRIBBLE_CLOSE is the sweetspot lever (F1).
local DRIBBLE_TOUCH_REACH = STICK_AHEAD + 6 -- the ball is playable for the next touch inside this
local DRIBBLE_CATCH_PACE = 10 -- ...once it has slowed back to about the carrier's own pace
local DRIBBLE_ERR_SKILL = 0.85 -- top skill cancels this fraction of the touch error
local DRIBBLE_CONTROL_SKILL = 26 -- extra control radius a top-skill carrier earns (px)
local POSSESS_DIST = 22 -- outfield control radius
local KEEPER_DIST = 18 -- keeper catch radius (small enough that corners stay open)
local KEEPER_BOX_DEPTH = 160 -- how far off its line the keeper will come to claim
-- The PENALTY AREA (the drawn box): a keeper holding the ball may not carry it
-- out of here. Exported for the renderer so the rule and the paint agree.
local PENALTY_DEPTH = 95
local PENALTY_H = 200
local KEEPER_BOX_PAD = 30 -- vertical margin beyond the posts for the claim zone
local KEEPER_CLAIM_DIST = 40 -- grab radius when actively claiming a ball in the box (priority)
local KEEPER_LEAD = 0.01 -- anticipation lead when claiming a moving ball
-- Conservative first pass: a nearby passing option keeps the keeper on its normal arc.
local KEEPER_1V1_SUPPORT = 120
local POSSESS_MAX_SPEED = 350 -- outfield can only collect a slow-enough ball
local PASS_SPEED = 420 -- minimum pass pace; long passes are driven harder (see pass_speed_for)
local PASS_ARRIVE_PACE = 120 -- ball speed left when a pass reaches its receiver
local PASS_SPEED_MAX = 700 -- cap so long passes are driven, never rockets
local PASS_LEAD = 0.6 -- lead a moving receiver by this fraction of the flight time
-- PASS_CHARGE_RATE: live-tunable — see sim/tuning.lua (F1 panel)
local PASS_RANGE_MIN = 110 -- a tap pass prefers someone close
-- PASS_RANGE_MAX: live-tunable — see sim/tuning.lua (F1 panel)
local CROSS_CLEAR_H = 50 -- crosses arc high into the box (headers live here)
-- AI_SHOOT_RANGE: live-tunable — see sim/tuning.lua (F1 panel)
local GOAL_MOUTH = 110
local GOAL_DEPTH = 30 -- net box depth behind the goal line (outside the field)
local NET_DAMP = 0.3 -- velocity kept when bouncing off the net
local NET_ROLLOUT = 200 -- net "slope": a netted ball rolls back out toward the line (px/s^2)
local RELEASE_CD = 0.3 -- pickup lockout after a shot/pass (seconds)

local STEAL_DIST = 26 -- challenge range to the BALL to dislodge it
-- STEAL_ATTEMPT: live-tunable — see sim/tuning.lua (F1 panel)
-- WHIFF_STUMBLE: live-tunable — see sim/tuning.lua (F1 panel)
-- An AI player needs a beat to control a ball it just received before it can
-- pick out a pressured pass — the defender's window to actually rob them.
-- CARRIER_SETTLE: live-tunable — see sim/tuning.lua (F1 panel)
local KICKOFF_CLEAR = 120 -- opponents keep this centre-circle distance at kickoff
-- Defenders hold shape (no pressing) for this long after a restart, or until
-- the kicking team's first pass/shot releases the ball — whichever comes first.
local KICKOFF_HOLD = 2.5
local TACKLE_POP_SPEED = 150 -- speed the ball pops out on a tackle
-- AI_STEAL_CD: live-tunable — see sim/tuning.lua (F1 panel)
local KEEPER_SMOTHER = 26 -- keeper takes the ball off a carrier's feet at this range (in its box)

-- Body blocking: a fast loose ball ricochets off an outfield body it hits instead
-- of ghosting through — defenders between the shooter and the goal matter. Slow
-- balls (below POSSESS_MAX_SPEED) are handled by collection; high balls fly over.
local BLOCK_HEIGHT = 20 -- a flat/rising ball at/below this height hits a body
-- A DESCENDING ball only blocks at trap height: you can't wall off a ball
-- dropping over your shoulder, so lobbed deliveries reach their receiver while
-- drilled shots and passes still slam into bodies.
local BLOCK_HEIGHT_DESC = 12
-- A just-released ball can't be body-blocked for a beat: it is leaving at foot
-- level through the immediate crowd (including the passer's own teammates).
local BLOCK_GRACE = 0.08
local BLOCK_DAMP = 0.5 -- fraction of speed kept by the ricochet

-- AI on-ball decision making: an AI carrier passes out of pressure instead of
-- dribbling blindly into a challenge.
-- AI_PASS_PRESSURE: live-tunable — see sim/tuning.lua (F1 panel)
local AI_PASS_MIN_OPEN = 40 -- an outlet must have this much space to be worth it
local AI_PASS_MIN_DIST = 40 -- don't pass to someone standing on your toes
local AI_PASS_MAX_DIST = 420 -- or to someone the other side of the pitch

-- AI shooting: shot power scales with space (the deterministic stand-in for the
-- human's charge). An unpressured striker sets himself and beats the keeper; a
-- closed-down one can only snap off a saveable shot — so defending means
-- closing shooters down, and conceding space concedes goals.
local AI_CHARGE_MIN_SPACE = 25 -- no power bonus with a defender this close
local AI_CHARGE_SPACE_RANGE = 120 -- space beyond that for the full charge bonus
local AI_PASS_RISK_PENALTY = 80 -- scoring malus when a chaser could cut the ground ball

-- Player tackle: the same button does a standing poke when slow, or a committed
-- slide when moving (slide speed scales off current velocity). Slides reach
-- further but lock you in and have a long recovery.
local SLIDE_DURATION = 0.4 -- how long the slide lunge lasts
local SLIDE_MULT = 1.5 -- initial slide speed = current speed × this
local SLIDE_BASE_MIN = 200 -- ...but never slower than this (slide has punch)
local SLIDE_FRICTION = 2.5 -- slide speed decay per second
local SLIDE_REACH = 38 -- slide tackle ball-win range (extended leg)
local SLIDE_CD = 0.9 -- recovery before tackling again after a slide
local STAND_TIMER = 0.22 -- standing-poke active window (forgiving timing)
local STAND_REACH = 34 -- standing tackle ball-win range
local STAND_CD = 0.4 -- recovery after a standing tackle
local STUN_SLOW = 0.4 -- movement multiplier while stunned
local STUN_TIME = 0.5 -- seconds a player is knocked off balance by a slide hit

-- Jockey stance: hold Space off the ball to shadow the carrier at reduced speed, facing
-- the ball (or loose ball). Releasing Space from jockey fires the poke with bonus reach.
-- JOCKEY_SLOW: live-tunable — see sim/tuning.lua (F1 panel)
local JOCKEY_REACH_BONUS = 6 -- extra poke reach when jockey_timer > 0 at tackle time
local JOCKEY_HOLD = 0.2 -- seconds jockey_timer is set to each held frame (decays)

-- Ball Z-axis (height). The ball keeps a 2D ground position (ball/ball_vel) plus a
-- scalar height; height gates collection/goals so lobs fly over heads.
local GRAVITY = 900 -- downward accel on ball_vz (px/s^2)
local BOUNCE = 0.55 -- vertical restitution on landing (horizontal speed kept)
local AIR_FRICTION = 0.3 -- horizontal decay/s while airborne (vs ground FRICTION)
local GROUND_GRAB_HEIGHT = 14 -- ball collectable/tacklable only at/below this height
local KEEPER_AIR_GRAB = 60 -- a keeper can claim up to this height inside its box
local CROSSBAR = 70 -- ball at/above this height at the line = over the bar, no goal
-- KEEPER_RESPECT_DIST: live-tunable — see sim/tuning.lua (F1 panel)
local LAND_SETTLE_VZ = 60 -- below this |vz| on landing, the ball settles (stops bouncing)
local LOB_CLEAR_H = 24 -- a lob must clear roughly head height over a blocker
-- Cap a lob's horizontal speed so it isn't a flat rocket — and so a lobbed pass
-- lands below POSSESS_MAX_SPEED after air drag: the receiver waiting on the spot
-- must be able to collect it instead of watching it bounce through to a chaser.
local MAX_LOB_VH = 400
local CHIP_LINE_Z = 65 -- a chip shot is this high crossing the line (over keeper, under bar)

-- The arena is a CAGE: nothing ever leaves. A skied ball hits the ceiling and
-- rains back down onto the pitch.
local CAGE_CEILING = 170
local CEIL_BOUNCE = 0.55

-- Aerial play. Geometry and quality live in sim.aerial; match owns movement,
-- candidate intent, ball output, and transient state.
local AERIAL_ANTICIPATE = 84 -- the ball magnet engages within this of a dropping ball
-- Cross-finishing aid: hand control to the best-placed attacker when a lofted
-- ball flies into the human's attacking third, so one strike finishes it.
local CROSS_AID_Z = 30 -- only a genuinely lofted ball triggers the aid
local CROSS_AID_THIRD = 0.6 -- ... once it is this far into the attacking half (x fraction)
local CROSS_AID_RANGE = 150 -- ... and only if an attacker is this close to meet it
-- HEADER_SPEED: live-tunable — see sim/tuning.lua (F1 panel)
local CLEAR_HEADER_SPEED = 320 -- defensive header clearance pace
local VOLLEY_SPEED = 1.3 -- volley pace multiplier
-- VOLLEY_SKY_P: live-tunable — see sim/tuning.lua (F1 panel)
-- AI_HEADER_RANGE: live-tunable — see sim/tuning.lua (F1 panel)
-- CROSS_MIN_SPACE: live-tunable — see sim/tuning.lua (F1 panel)

-- Goalkeeper saves + distribution. Reach/handling are per-keeper (see sim.stats);
-- these are the shared thresholds.
-- Save quality = how close the ball is (within reach) + handling − pace. Whether
-- the keeper can get there at all (vs beaten) is geometric and deterministic;
-- whether a reached ball is GRABBED or PARRIED is probabilistic — a logistic
-- curve over quality, rolled from the match's seeded RNG (reproducible: same
-- seed, same match). Soft shots straight at the keeper stick in the gloves
-- almost every time; a blistering or full-stretch ball is usually pushed away.
-- Tuned so an uncharged shot placed at a corner is kept out (mostly a parry) by
-- a decent keeper, while a fully charged corner shot still beats it clean.
-- SAVE_SPEED_REF: live-tunable — see sim/tuning.lua (F1 panel)
-- CATCH_EVEN_QUALITY: live-tunable — see sim/tuning.lua (F1 panel)
local CATCH_SOFTNESS = 0.12 -- how fast the odds saturate either side of even
local PARRY_QUALITY = 0.1 -- at/above this the keeper at least gets a hand to it; below: beaten
local HANDLING_WEIGHT = 0.5 -- how much keeper handling lifts save quality
local PARRY_CD = 0.18 -- pickup lockout after a parry (stops instant re-grab/re-dive)
local PARRY_SPEED_MULT = 0.6 -- fraction of incoming speed kept on a deflection
local MIN_PARRY_CLEAR = 260 -- a parry is always punched at least this fast (clear of traffic)
-- A parry is tipped UP as well as out: the deflection sails over the shooter's
-- head instead of being served back to their feet (where it would ricochet off
-- their body straight back at the goal — a guaranteed rebound tap-in).
local PARRY_POP_VZ = 240
local KEEPER_HOLD = 0.9 -- seconds an AI keeper surveys/holds before distributing
-- KEEPER_HOLD_HUMAN: live-tunable — see sim/tuning.lua (F1 panel)
local PUNT_MIN = 240 -- keeper punt (clearance kick) distance floor
-- PUNT_MAX: live-tunable — see sim/tuning.lua (F1 panel)
local PUNT_CLEAR_H = 60 -- punts sail high over midfield
local KEEPER_DIVE_DURATION = 0.32 -- dive lunge / animation window
-- A committed save resolves when the ball ACTUALLY arrives at the keeper (real
-- trajectory, no teleport): contact radius, plus a crossing/timeout backstop.
local KEEPER_HANDS = 30 -- ball-contact radius that completes a committed save
local SAVE_TIMEOUT_PAD = 0.25 -- backstop beyond the predicted crossing time
local DEAD_SHOT_SPEED = 30 -- a pending save below this pace is a loose ball, not a shot
local KEEPER_SAFE_DIST = 60 -- a distribution outlet must be this clear of opponents
-- A floated throw needs the receiver a real step off its marker: body collision
-- keeps players ~24px apart and the AI steal range is 26, so a receiver marked
-- tighter than this would be dispossessed the moment the throw lands.
local THROW_MIN_OPEN = 30
local DROPKICK_DIST = 420 -- how far upfield a drop-kick clearance lands
local DROPKICK_CLEAR_H = 46 -- drop-kick loft: sails over every head on the way
local THROW_CLEAR_H = 34 -- hand throws arc clearly over heads (> LOB_CLEAR_H foot lobs)
-- A keeper's HANDS are accurate: a throw must not be interferable. Any
-- opponent near the flight lane raises the arc above the aerial strike
-- envelope (nobody can jump that high); a covered receiver gets the ball
-- landed to their SAFE side — away from the cover, close enough to run onto.
local THROW_SAFE_CLEAR = aerial.MAX_TOUCH_Z + 16 -- over a presser: beyond any jumping strike
local THROW_LANE_W = 60 -- an opponent this close to the throw lane can attack the flight
local THROW_LEAD_MAX = 55 -- land a covered outlet's throw at most this far to their safe side
local THROW_COVER_DIST = 140 -- an opponent this close to the receiver counts as cover
local RELEASE_DINK_DIST = 44 -- a defender this close on the release line gets dinked over
local SAVE_PAD = 18 -- on-target tolerance beyond the posts when projecting a shot
local SAVE_ZONE = 130 -- the keeper commits a dive once the shot is this close to its line
local KEEPER_GRAB_POSE = 0.25 -- seconds of the gather/reach pose after a grab
local KEEPER_THROW_POSE = 0.25 -- seconds of the release/throw pose after distributing
local RECEIVE_TIME = 1.3 -- seconds the intended receiver runs onto a keeper's distribution
-- A back-pass keeps the keeper coming to MEET it until it resolves: long enough
-- to outlive any under-hit roll (the window is cut short the moment anyone
-- gains the ball or a body/strike disrupts the pass — see match.step).
local KEEPER_RECEIVE_TIME = 4
-- Aim within ~23 degrees of the keeper — and closer-aligned than any other
-- teammate — and a pass is a deliberate back-pass: the keeper receives it.
local BACKPASS_AIM_COS = 0.92

-- Locomotion momentum. All walking/running movement (controlled player, AI owner
-- dribble, keeper positioning, off-ball AI) goes through apply_locomotion, which
-- accelerates run_vel toward a desired velocity. Slides, dodges, and dives keep
-- their own bespoke movement and bypass this helper.
-- MOVE_ACCEL: live-tunable — see sim/tuning.lua (F1 panel)
-- MOVE_DECEL: live-tunable — see sim/tuning.lua (F1 panel)
-- Facing follows run_vel when moving; below this threshold, keep the last facing
-- so a stationary player can still aim without snapping to zero.
local RUN_VEL_FACE_MIN = 20

-- SHOT_WINDUP: live-tunable — see sim/tuning.lua (F1 panel)
local WINDUP_MOVE = 0.3 -- movement multiplier during the wind-up plant phase

-- CHARGE_RATE: live-tunable — see sim/tuning.lua (F1 panel)
local CHARGE_POWER = 0.9 -- full charge adds this fraction to shot speed
local CURVE_MAX = 520 -- lateral acceleration of a full-charge curved shot
local SPIN_DECAY = 1.4 -- how fast curve bleeds off
local DODGE_DURATION = 0.16 -- length of a juke (seconds)
local DODGE_CD = 0.6 -- juke cooldown (seconds)
local DODGE_SPEED_MULT = 2.4 -- sideways speed during a juke

-- Sprint (controlled player): a hold-to-run burst from a stamina tank. The tank
-- size is stamina-derived (see sim.stats); it refills while not sprinting.
-- SPRINT_MULT: live-tunable — see sim/tuning.lua (F1 panel)
-- SPRINT_REFILL: live-tunable — see sim/tuning.lua (F1 panel)
local SPRINT_ENGAGE = 0.25 -- min meter to start a sprint (hysteresis: no flicker at empty)

---@class MatchPlayer
---@field id string
---@field name string
---@field team "home"|"away"
---@field pos Vec2
---@field vel Vec2  -- realized velocity (px/s) from last tick's movement; AI prediction source
---@field run_vel Vec2  -- locomotion velocity (px/s); accel/decel toward desired each tick
---@field facing Vec2
---@field anchor Vec2
---@field species_id string
---@field owned_verb SimVerb
---@field move_speed number
---@field shot_speed number
---@field dribble number  -- 0..1 ball control (higher = tighter touches, harder to nick)
---@field strength number  -- normalized 0..1, used by aerial contests
---@field first_touch number  -- 0..1 aerial reception quality
---@field header_skill number  -- 0..1 header contact quality
---@field volley_skill number  -- 0..1 volley contact quality
---@field bicycle_skill number  -- 0..1 bicycle-kick contact quality
---@field is_keeper boolean
---@field radius number
---@field dash_cd number  -- AI tackle cooldown (seconds until it can challenge again)
---@field dodge_cd number  -- seconds until a juke is ready again
---@field dodge_timer number  -- seconds of juke (sidestep + tackle immunity) remaining
---@field dodge_dir Vec2  -- sidestep direction for the active juke
---@field reach number  -- keeper dive/save radius in px (0 for outfield)
---@field handling number  -- keeper clean-catch factor 0..1 (0 for outfield)
---@field keeper_aggression number  -- positioning depth in px (0 for outfield)
---@field keeper_anticipation number  -- 0..1 shot-reading quality (0 for outfield)
---@field keeper_state KeeperBehaviorState  -- visible positioning/reaction state
---@field keeper_state_timer number  -- seconds left in the current timed state
---@field keeper_release_state KeeperBehaviorState?  -- state captured at the latest shot release
---@field keeper_release_motion number  -- normalized locomotion captured at shot release
---@field keeper_release_kind KeeperShotType?  -- latest inbound shot type
---@field keeper_release_depth number  -- line depth captured at shot release
---@field keeper_set number  -- seconds the pre-release set/lean has been visible (0 = inactive)
---@field dive_timer number  -- seconds of keeper dive lunge remaining
---@field dive_dir Vec2  -- unit direction of the active dive
---@field dive_delay number  -- countdown until a queued dive launches (synced to ball arrival)
---@field dive_target Vec2?  -- intercept point the dive converges on (movement stops there)
---@field hold_timer number  -- seconds a keeper holds the ball before distributing
---@field feet_ball boolean  -- keeper took a teammate's pass with the feet (no hands: dribbles, tackleable)
---@field slide_timer number  -- seconds of an active slide tackle remaining
---@field slide_dir Vec2  -- locked travel direction of the slide
---@field slide_vel number  -- current slide speed (px/s), decays over the slide
---@field tackle_timer number  -- seconds of a standing-tackle poke window remaining
---@field tackle_cd number  -- recovery before another tackle/slide can start
---@field stun_timer number  -- seconds knocked off balance (slowed, can't tackle)
---@field grab_timer number  -- keeper gather/reach pose remaining (visual)
---@field throw_timer number  -- keeper release/throw pose remaining (visual)
---@field receive_timer number  -- seconds this player is running onto an incoming pass
---@field sprint_meter number  -- 0..1 stamina tank for sprinting
---@field sprint_dur number  -- seconds a full tank lasts (stamina-derived)
---@field sprinting boolean  -- currently sprint-boosted (hysteresis state)
---@field save_pending "catch"|"parry"|nil  -- committed save verdict awaiting ball contact
---@field save_timer number  -- backstop countdown to force-resolve a pending save
---@field save_vx number  -- shot x-velocity at commit (sign flip = deflected, dive whiffs)
---@field save_style SaveStyle?  -- deterministic presentation style for the active attempt
---@field save_tip_emitted boolean  -- one-shot guard for the current released shot
---@field settle_timer number  -- AI first-touch control window (no pressured pass until settled)
---@field header_cd number  -- cooldown between aerial (header/volley) attempts
---@field aerial_timer number  -- transient aerial pose timer
---@field aerial_style AerialStyle?
---@field aerial_outcome AerialOutcome?
---@field aerial_jump number  -- 0..1 required lift for rendering
---@field aerial_recovery number  -- movement/action recovery after an aerial attempt
---@field charge number  -- this player's shot/punt charge, 0..1
---@field pass_charge number  -- this player's pass-range charge, 0..1
---@field pass_target integer?  -- player this owner would pass to if released now
---@field windup_timer number  -- seconds until a pending shot/punt releases (0 = none)
---@field windup_shot { dir: Vec2, speed: number, vz: number, spin: number, shot_type: KeeperShotType }?  -- payload captured at commit
---@field jockey_timer number  -- seconds of active jockey stance remaining (grants bonus poke reach)

---@alias Rect { x: number, y: number, w: number, h: number }

---@class MatchInput
---@field move Vec2  -- controlled player's desired direction
---@field shoot boolean  -- fire the shot (released this frame)
---@field shoot_held boolean  -- shoot key currently down (builds charge)
---@field pass boolean  -- release the pass (fired on key release)
---@field pass_held boolean  -- pass key currently down (builds pass range)
---@field switch boolean  -- hand control to the outfielder nearest the ball
---@field dash boolean  -- tackle attempt (slide when moving fast, poke when slow)
---@field dodge boolean  -- sidestep juke with brief tackle immunity
---@field lob boolean  -- loft modifier: chip a shot / lob a pass over a defender
---@field sprint boolean  -- hold to sprint (drains the sprint meter)
---@field jockey boolean  -- hold Space off the ball: slow shadow stance, bonus poke reach on release
---@field aerial_strike boolean?  -- abstract first-time strike intent
---@field aerial_acrobatic boolean?  -- abstract bicycle/acrobatic intent
---@field equipment_held boolean  -- equipment control is physically held
---@field equipment_pressed boolean  -- equipment press edge for this fixed tick
---@field equipment_released boolean  -- equipment release edge for this fixed tick

-- One-frame notifications of discrete actions, for the renderer's juice layer
-- (flashes, trails). Produced by the sim, cleared at the top of every step, so
-- a frame's events are whatever happened during that frame. Positions are world
-- space; `player` is the actor's id (nil for ball-only events).
---@class MatchEvent
---@field kind "shot"|"pass"|"touch"|"tackle"|"catch"|"parry"|"tip"|"claim"|"block"|"header"|"volley"|"bicycle"|"reception"|"juke"
---@field x number
---@field y number
---@field player string?
---@field save_style SaveStyle?
---@field style AerialStyle?
---@field outcome AerialOutcome?
---@field jumping boolean?
---@field difficulty number?
---@field shot_type KeeperShotType?
---@field keeper_state KeeperBehaviorState?
---@field keeper_depth number?
---@field on_target boolean?

---@class MatchState
---@field field { w: number, h: number }
---@field goal_home Rect  -- left goal; away scores here
---@field goal_away Rect  -- right goal; home scores here
---@field players MatchPlayer[]  -- home indices 1..5, away 6..10
---@field ball Vec2  -- ground position (x, y); height is ball_z
---@field ball_vel Vec2  -- horizontal velocity (ground plane)
---@field ball_z number  -- height above the pitch (0 = on the ground)
---@field ball_vz number  -- vertical velocity (+ = rising)
---@field owner integer?  -- index into players, nil if loose
---@field controlled integer  -- selected home player; valid even when every player is AI
---@field human_controlled boolean  -- whether `controlled` takes human-input branches
---@field score { home: integer, away: integer }
---@field time_left number
---@field max_goals integer
---@field finished boolean
---@field pickup_cd number
---@field press { home: integer, away: integer }  -- chasers per team (tactic-driven)
---@field marking { home: MarkingConfig, away: MarkingConfig }  -- off-ball scheme per team
---@field marks { home: table<integer, integer>, away: table<integer, integer> }  -- prev marking assignment (hysteresis)
---@field ball_spin number  -- lateral curve applied to the loose ball
---@field rng integer  -- seeded PRNG state (core.rng): same seed = same match
---@field block_grace number  -- body-blocking re-enabled when this hits 0 (set on release)
---@field aerial_lock number  -- seconds until this loose ball can take another aerial contact
---@field kickoff_hold number  -- seconds left of post-restart defensive hold (0 = press resumes)
---@field events MatchEvent[]  -- discrete actions this frame (see MatchEvent)
---@field slot_mode boolean -- True when the fixture uses stable eight-slot InputFrame routing.
---@field input_ownership InputOwnership? -- Stable fixture slot-to-player identity.
---@field slot_players table<integer, integer> -- Canonical slot index -> MatchState player index.
---@field slot_for_player table<integer, integer> -- Outfielder player index -> canonical slot index.
---@field input_tick integer -- InputFrame tick expected by the next slot-mode step.

local match = {}

---@return table<string, PlayerData>
local function pool_by_id()
    local by_id = {}
    for _, p in ipairs(player_pool) do
        by_id[p.id] = p
    end
    return by_id
end

---@param team TeamData
---@param players_by_id table<string, PlayerData>
---@return string[]
local function fixture_outfield_ids(team, players_by_id)
    local ids = {}
    for _, id in ipairs(team.roster) do
        local player = assert(players_by_id[id], "unknown player: " .. tostring(id))
        if player.position ~= "keeper" then
            ids[#ids + 1] = id
        end
    end
    assert(#ids == input_frame.HOME_SLOT_COUNT, team.id .. " must have four outfielders")
    return ids
end

-- Construct the canonical input ownership for the two authored fixture teams.
-- This is useful to local/headless adapters; callers with a selected fixture
-- roster may instead pass their validated InputOwnership directly to match.new.
---@param home TeamData
---@param away TeamData
---@param players_by_id table<string, PlayerData>?
---@return InputOwnership
function match.ownership_for_teams(home, away, players_by_id)
    local by_id = players_by_id or pool_by_id()
    local home_outfield = fixture_outfield_ids(home, by_id)
    local away_outfield = fixture_outfield_ids(away, by_id)
    local assignments = {}
    for index = 1, input_frame.SLOT_COUNT do
        local slot = assert(input_frame.slot(index))
        local ids = slot.team == "home" and home_outfield or away_outfield
        assignments[index] = {
            slot = slot.id,
            team = slot.team,
            player_id = ids[slot.outfield_index],
        }
    end
    return assert(input_frame.new_ownership(assignments, {
        home = home.roster,
        away = away.roster,
    }, by_id))
end

---@param team TeamData
---@param side "home"|"away"
---@param field { w: number, h: number }
---@param by_id table<string, PlayerData>
---@param species_by_id table<string, SpeciesData>
---@param showcase_by_id table<string, ShowcasePlayerCompatibilityData>
---@param formation_id string?  -- override team.formation
---@param line_shift number  -- tactic depth bias (fraction of pitch, toward attack)
---@return MatchPlayer[]
local function build_team(
    team,
    side,
    field,
    by_id,
    species_by_id,
    showcase_by_id,
    formation_id,
    line_shift
)
    local formation = formations[formation_id or team.formation]
    assert(formation, "unknown formation: " .. tostring(formation_id or team.formation))
    local anchors = placement.anchors(formation, side, field)
    local shift = (side == "home" and 1 or -1) * line_shift * field.w

    -- Keeper first, then outfield in roster order (matches formation order).
    local keeper_id, outfield = nil, {}
    for _, id in ipairs(team.roster) do
        local pd = by_id[id]
        assert(pd, "unknown player: " .. tostring(id))
        if pd.position == "keeper" and not keeper_id then
            keeper_id = id
        else
            outfield[#outfield + 1] = id
        end
    end
    assert(keeper_id, team.id .. " roster needs a keeper")

    local ordered = { keeper_id }
    for _, id in ipairs(outfield) do
        ordered[#ordered + 1] = id
    end

    local list = {}
    for i, id in ipairs(ordered) do
        local pd = by_id[id]
        local showcase = showcase_by_id[pd.id]
        local species_id = showcase and showcase.species or "neutral"
        local species_data = species_by_id[species_id]
        assert(species_data, "unknown species: " .. tostring(species_id))
        local effective_stats = species.apply(pd.stats, species_data)
        local base = anchors[i]
        -- Outfield anchors shift with the tactic; the keeper (i == 1) stays home.
        local ax = base.x
        if i > 1 then
            ax = math.max(PLAYER_RADIUS, math.min(field.w - PLAYER_RADIUS, base.x + shift))
        end
        local anchor = Vec2.new(ax, base.y)
        list[i] = {
            id = id,
            name = pd.name,
            team = side,
            pos = Vec2.new(anchor.x, anchor.y),
            vel = Vec2.new(0, 0),
            run_vel = Vec2.new(0, 0),
            facing = Vec2.new(side == "home" and 1 or -1, 0),
            anchor = anchor,
            species_id = species_data.id,
            owned_verb = species_data.verb,
            move_speed = stats.move_speed(effective_stats),
            shot_speed = stats.shot_speed(effective_stats),
            dribble = math.max(
                0,
                math.min(
                    1,
                    stats.dribble(effective_stats) + species.dribble_protection(species_data.verb)
                )
            ),
            strength = effective_stats.strength / 10,
            first_touch = stats.first_touch(effective_stats),
            header_skill = stats.header(effective_stats),
            volley_skill = stats.volley(effective_stats),
            bicycle_skill = stats.bicycle(effective_stats),
            is_keeper = pd.position == "keeper",
            radius = PLAYER_RADIUS,
            dash_cd = 0,
            dodge_cd = 0,
            dodge_timer = 0,
            dodge_dir = Vec2.new(0, 0),
            reach = (pd.position == "keeper") and stats.keeper_reach(effective_stats) or 0,
            handling = (pd.position == "keeper") and stats.keeper_handling(effective_stats) or 0,
            keeper_aggression = (pd.position == "keeper") and stats.keeper_aggression(
                effective_stats
            ) or 0,
            keeper_anticipation = (pd.position == "keeper") and stats.keeper_anticipation(
                effective_stats
            ) or 0,
            keeper_state = "base",
            keeper_state_timer = 0,
            keeper_release_state = nil,
            keeper_release_motion = 0,
            keeper_release_kind = nil,
            keeper_release_depth = 0,
            keeper_set = 0,
            dive_timer = 0,
            dive_dir = Vec2.new(0, 0),
            dive_delay = 0,
            dive_target = nil,
            hold_timer = 0,
            feet_ball = false,
            slide_timer = 0,
            slide_dir = Vec2.new(0, 0),
            slide_vel = 0,
            tackle_timer = 0,
            tackle_cd = 0,
            stun_timer = 0,
            grab_timer = 0,
            throw_timer = 0,
            receive_timer = 0,
            sprint_meter = 1,
            sprint_dur = stats.sprint_duration(effective_stats),
            sprinting = false,
            save_pending = nil,
            save_timer = 0,
            save_vx = 0,
            save_style = nil,
            save_tip_emitted = false,
            settle_timer = 0,
            header_cd = 0,
            aerial_timer = 0,
            aerial_style = nil,
            aerial_outcome = nil,
            aerial_jump = 0,
            aerial_recovery = 0,
            charge = 0,
            pass_charge = 0,
            pass_target = nil,
            windup_timer = 0,
            windup_shot = nil,
            jockey_timer = 0,
        }
    end
    return list
end

-- Index of the most advanced home outfield player (the default controlled one).
---@param players MatchPlayer[]
---@return integer
local function most_advanced_home(players)
    local best, best_x
    for i, p in ipairs(players) do
        if p.team == "home" and not p.is_keeper then
            if not best_x or p.pos.x > best_x then
                best_x = p.pos.x
                best = i
            end
        end
    end
    return best or 1
end

---@param s MatchState
---@param player_idx integer
---@return boolean
local function is_human_player(s, player_idx)
    if s.slot_mode then
        return s.slot_for_player[player_idx] ~= nil
    end
    return s.human_controlled and player_idx == s.controlled
end

-- Reset for a kickoff. `kicking` is the team restarting play (after conceding,
-- per the laws of the game); the opening kickoff is the home side's.
---@param s MatchState
---@param kicking "home"|"away"?
local function place_kickoff(s, kicking)
    kicking = kicking or "home"
    local half = s.field.w / 2
    for _, p in ipairs(s.players) do
        -- Kickoff law: everyone lines up in their own half (forwards whose
        -- formation anchor is upfield pull back to the halfway line and push
        -- up again once play restarts).
        local ax = p.anchor.x
        if p.team == "home" then
            ax = math.min(ax, half - PLAYER_RADIUS)
        else
            ax = math.max(ax, half + PLAYER_RADIUS)
        end
        p.pos = Vec2.new(ax, p.anchor.y)
        p.vel = Vec2.new(0, 0)
        p.run_vel = Vec2.new(0, 0)
        p.facing = Vec2.new(p.team == "home" and 1 or -1, 0)
        p.dive_timer = 0
        p.dive_dir = Vec2.new(0, 0)
        p.dive_delay = 0
        p.dive_target = nil
        p.keeper_state = "base"
        p.keeper_state_timer = 0
        p.keeper_release_state = nil
        p.keeper_release_motion = 0
        p.keeper_release_kind = nil
        p.keeper_release_depth = 0
        p.hold_timer = 0
        p.feet_ball = false
        p.slide_timer = 0
        p.slide_dir = Vec2.new(0, 0)
        p.slide_vel = 0
        p.tackle_timer = 0
        p.tackle_cd = 0
        p.stun_timer = 0
        p.grab_timer = 0
        p.throw_timer = 0
        p.receive_timer = 0
        p.sprint_meter = 1
        p.sprinting = false
        p.save_pending = nil
        p.save_timer = 0
        p.save_vx = 0
        p.keeper_set = 0
        p.save_style = nil
        p.save_tip_emitted = false
        p.settle_timer = 0
        p.header_cd = 0
        p.aerial_timer = 0
        p.aerial_style = nil
        p.aerial_outcome = nil
        p.aerial_jump = 0
        p.aerial_recovery = 0
        p.charge = 0
        p.pass_charge = 0
        p.pass_target = nil
        p.windup_timer = 0
        p.windup_shot = nil
        p.jockey_timer = 0
    end
    -- Give the kicking team the ball at the centre spot.
    local kicker
    if kicking == "home" then
        kicker = most_advanced_home(s.players)
        if not s.slot_mode then
            s.controlled = kicker
        end
    else
        -- The most advanced away outfielder (away attacks -x) takes the kickoff;
        -- the human gets their most advanced player to defend with.
        local best_x
        for i, p in ipairs(s.players) do
            if p.team == "away" and not p.is_keeper then
                if not best_x or p.pos.x < best_x then
                    best_x = p.pos.x
                    kicker = i
                end
            end
        end
        if not s.slot_mode then
            s.controlled = most_advanced_home(s.players)
        end
    end
    local c = s.players[kicker]
    c.facing = Vec2.new(kicking == "home" and 1 or -1, 0)
    c.pos = Vec2.new(s.field.w * (kicking == "home" and 0.45 or 0.55), s.field.h / 2)
    s.ball = c.pos:add(c.facing:scale(STICK_AHEAD))
    s.ball_vel = Vec2.new(0, 0)
    s.ball_z = 0
    s.ball_vz = 0
    s.owner = kicker
    s.pickup_cd = 0
    s.block_grace = 0
    s.aerial_lock = 0
    s.ball_spin = 0
    s.kickoff_hold = KICKOFF_HOLD
    -- Centre-circle rule: the non-kicking team keeps its distance from the
    -- ball at the restart — push any intruder straight back out.
    for _, p in ipairs(s.players) do
        if p.team ~= kicking and not p.is_keeper then
            local off = p.pos:sub(s.ball)
            local d = off:length()
            if d < KICKOFF_CLEAR then
                local dir = (d > 0) and off:normalized()
                    or Vec2.new(p.team == "home" and -1 or 1, 0)
                local np = s.ball:add(dir:scale(KICKOFF_CLEAR))
                p.pos = Vec2.new(
                    math.max(PLAYER_RADIUS, math.min(s.field.w - PLAYER_RADIUS, np.x)),
                    math.max(PLAYER_RADIUS, math.min(s.field.h - PLAYER_RADIUS, np.y))
                )
            end
        end
    end
end

---@param tactic TacticData
---@return MarkingConfig
local function marking_of(tactic)
    -- Keep the compatibility fallback function-local so this near-limit chunk
    -- does not spend a permanent local on legacy data.
    return tactic.marking
        or { scheme = "hybrid", man_marks = 1, standoff = 32, compactness = 0.5, support = 0.5 }
end

---@param opts { home: TeamData, away: TeamData, field: { w: number, h: number }, home_formation: string?, tactic: TacticData?, away_tactic: TacticData?, duration: number?, max_goals: integer?, seed: number?, players_by_id: table<string, PlayerData>?, species_by_id: table<string, SpeciesData>?, showcase_players_by_id: table<string, ShowcasePlayerCompatibilityData>?, human_controlled: boolean?, input_ownership: InputOwnership? }
---@return MatchState
function match.new(opts)
    local field = opts.field
    local by_id = opts.players_by_id or pool_by_id()
    local species_by_id = opts.species_by_id or species_pool
    local showcase_by_id = opts.showcase_players_by_id
        or require("data.showcase_player_compatibility")
    local home_tactic = opts.tactic or tactics.balanced
    local away_tactic = opts.away_tactic or tactics.balanced

    -- Seeded randomness (grab-vs-parry rolls). Warm the state up a few steps:
    -- minstd's first draws correlate with small seeds (seed 3 -> tiny sample).
    local rstate = rng.seed(opts.seed or 42)
    for _ = 1, 3 do
        rstate = rng.roll(rstate)
    end

    local home = build_team(
        opts.home,
        "home",
        field,
        by_id,
        species_by_id,
        showcase_by_id,
        opts.home_formation,
        home_tactic.line_shift
    )
    local away = build_team(
        opts.away,
        "away",
        field,
        by_id,
        species_by_id,
        showcase_by_id,
        nil,
        away_tactic.line_shift
    )
    local players = {}
    for _, p in ipairs(home) do
        players[#players + 1] = p
    end
    for _, p in ipairs(away) do
        players[#players + 1] = p
    end

    local slot_players = {}
    local slot_for_player = {}
    if opts.input_ownership then
        assert(input_frame.validate_ownership(opts.input_ownership, by_id))
        for _, team in ipairs({ "home", "away" }) do
            local expected = team == "home" and opts.home.roster or opts.away.roster
            local recorded = opts.input_ownership.rosters[team]
            assert(#recorded == #expected, "input ownership roster does not match fixture team")
            for index = 1, #expected do
                assert(
                    recorded[index] == expected[index],
                    "input ownership roster does not match fixture team"
                )
            end
        end
        local index_by_id = {}
        for index, player in ipairs(players) do
            index_by_id[player.id] = index
        end
        for slot_index = 1, input_frame.SLOT_COUNT do
            local assignment = opts.input_ownership.slots[slot_index]
            local player_index =
                assert(index_by_id[assignment.player_id], "slot player is not in match")
            local player = players[player_index]
            assert(player.team == assignment.team, "slot team does not match match player")
            assert(not player.is_keeper, "keeper cannot be mapped to an input slot")
            slot_players[slot_index] = player_index
            slot_for_player[player_index] = slot_index
        end
    end

    local mouth_y = field.h / 2 - GOAL_MOUTH / 2
    ---@type MatchState
    local s = {
        field = field,
        -- Real goals stand OUTSIDE the field: the goal line is the field
        -- boundary (x = 0 / x = field.w) and the net box extends behind it.
        goal_home = { x = -GOAL_DEPTH, y = mouth_y, w = GOAL_DEPTH, h = GOAL_MOUTH },
        goal_away = { x = field.w, y = mouth_y, w = GOAL_DEPTH, h = GOAL_MOUTH },
        players = players,
        human_controlled = opts.human_controlled ~= false,
        ball = Vec2.new(0, 0),
        ball_vel = Vec2.new(0, 0),
        ball_z = 0,
        ball_vz = 0,
        owner = nil,
        controlled = most_advanced_home(players),
        score = { home = 0, away = 0 },
        time_left = opts.duration or 120,
        max_goals = opts.max_goals or 3,
        finished = false,
        pickup_cd = 0,
        press = { home = home_tactic.press, away = away_tactic.press },
        marking = { home = marking_of(home_tactic), away = marking_of(away_tactic) },
        marks = { home = {}, away = {} },
        block_grace = 0,
        aerial_lock = 0,
        kickoff_hold = 0,
        ball_spin = 0,
        rng = rstate,
        events = {},
        slot_mode = opts.input_ownership ~= nil,
        input_ownership = opts.input_ownership and assert(
            input_frame.copy_ownership(opts.input_ownership, by_id)
        ) or nil,
        slot_players = slot_players,
        slot_for_player = slot_for_player,
        input_tick = 0,
    }
    place_kickoff(s)
    return s
end

---@param s MatchState
---@param pos Vec2
---@return Vec2
local function clamp_to_field(s, pos)
    local r = PLAYER_RADIUS
    local x = math.max(r, math.min(s.field.w - r, pos.x))
    local y = math.max(r, math.min(s.field.h - r, pos.y))
    return Vec2.new(x, y)
end

---@param ball Vec2
---@param goal Rect
---@return boolean
local function in_mouth(ball, goal)
    return ball.y >= goal.y and ball.y <= goal.y + goal.h
end

-- Set (index -> true) of the `count` non-keepers of `team` nearest the ball.
---@param s MatchState
---@param team "home"|"away"
---@param count integer
---@return table<integer, boolean>
local function nearest_n(s, team, count)
    local cand = {}
    for i, p in ipairs(s.players) do
        -- The controlled player is the human's business, not an AI resource:
        -- counting them here used to spend the whole chase allocation on the
        -- human, leaving every AI teammate statically watching a loose ball.
        if p.team == team and not p.is_keeper and not is_human_player(s, i) then
            cand[#cand + 1] = { idx = i, d = p.pos:dist(s.ball) }
        end
    end
    table.sort(cand, placement.distance_candidate_before)
    local set = {}
    for k = 1, math.min(count, #cand) do
        set[cand[k].idx] = true
    end
    return set
end

-- The home outfielder best placed to defend right now: nearest to the ball
-- (the current player included — no forced change if they're already it).
---@param s MatchState
---@return integer
local function best_defender(s)
    local best, best_d
    for i, p in ipairs(s.players) do
        if p.team == "home" and not p.is_keeper then
            local d = p.pos:dist(s.ball)
            if not best_d or d < best_d then
                best_d, best = d, i
            end
        end
    end
    return best or s.controlled
end

-- Manual switch: hand control to the home outfielder nearest the ball (other
-- than the current one) — the player you actually want when defending.
---@param s MatchState
---@param cur integer
---@return integer
local function next_home_outfield(s, cur)
    local best, best_d
    for i, p in ipairs(s.players) do
        if p.team == "home" and not p.is_keeper and i ~= cur then
            local d = p.pos:dist(s.ball)
            if not best_d or d < best_d then
                best_d, best = d, i
            end
        end
    end
    return best or cur
end

-- Launch a lob from `from` to `to` clearing height `h` over a blocker at lane
-- fraction `f`. Returns the horizontal velocity and the vertical launch speed so
-- the ball lands on `to`. Closed-form, deterministic.
---@param from Vec2
---@param to Vec2
---@param f number  -- blocker position along the lane, 0..1
---@param h number  -- height to clear at the blocker
---@return Vec2 vel
---@return number vz
local function lob_launch(from, to, f, h)
    local dir = to:sub(from)
    local d = dir:length()
    if d < 1 then
        return Vec2.new(0, 0), math.sqrt(2 * h * GRAVITY)
    end
    f = math.max(0.15, math.min(0.85, f))
    local tf = math.max(math.sqrt(2 * h / (GRAVITY * f * (1 - f))), d / MAX_LOB_VH)
    tf = math.min(tf, 1.0) -- keep lobs from becoming moon-balls
    return dir:normalized():scale(d / tf), 0.5 * GRAVITY * tf
end

---@param s MatchState
---@param owner MatchPlayer
---@param dir Vec2
---@param speed number?  -- defaults to the shooter's base shot speed
---@param vz number?  -- vertical launch (a chip); defaults to 0 (driven, on the ground)
---@param shot_type KeeperShotType?
local function release_shot(s, owner, dir, speed, vz, shot_type)
    local launch_speed = speed or owner.shot_speed
    local launch_vz = vz or 0
    local launch_dir = dir:normalized()
    local released_type = shot_type or ((launch_vz > 0) and "chip" or "ground")
    local threatened_keeper
    for _, player in ipairs(s.players) do
        if player.is_keeper then
            player.save_style = nil
            player.save_tip_emitted = false
            if player.team ~= owner.team then
                threatened_keeper = player
            end
        end
    end
    local on_target = false
    if threatened_keeper then
        local goal = threatened_keeper.team == "home" and s.goal_home or s.goal_away
        local goal_line_x = threatened_keeper.team == "home" and (goal.x + goal.w) or goal.x
        local infield_direction = threatened_keeper.team == "home" and 1 or -1
        threatened_keeper.keeper_release_state = threatened_keeper.keeper_state
        if threatened_keeper.keeper_state ~= "set" then
            threatened_keeper.keeper_release_motion = math.min(
                1,
                threatened_keeper.run_vel:length() / math.max(threatened_keeper.move_speed, 1)
            )
        end
        threatened_keeper.keeper_release_kind = released_type
        threatened_keeper.keeper_release_depth =
            math.max(0, (threatened_keeper.pos.x - goal_line_x) * infield_direction)
        local height = keeper.goal_line_height({
            origin = s.ball,
            direction = launch_dir,
            horizontal_speed = launch_speed,
            vertical_speed = launch_vz,
            defending_team = threatened_keeper.team,
            goal = goal,
            friction = launch_vz > 0 and AIR_FRICTION or FRICTION,
            gravity = GRAVITY,
        })
        on_target = height ~= nil
            and height >= 0
            and height < CROSSBAR
            and keeper.shot_targets_goal({
                defending_team = threatened_keeper.team,
                shooter_team = owner.team,
                origin = s.ball,
                direction = launch_dir,
                goal = goal,
            })
    end
    s.events[#s.events + 1] = {
        kind = "shot",
        x = s.ball.x,
        y = s.ball.y,
        player = owner.id,
        shot_type = released_type,
        keeper_state = threatened_keeper and threatened_keeper.keeper_release_state or nil,
        keeper_depth = threatened_keeper and threatened_keeper.keeper_release_depth or nil,
        on_target = on_target,
    }
    s.owner = nil
    s.kickoff_hold = 0
    s.ball_vel = launch_dir:scale(launch_speed)
    s.ball_z = 0
    s.ball_vz = launch_vz
    s.pickup_cd = RELEASE_CD
    s.block_grace = BLOCK_GRACE
end

-- Pace a ground pass so it actually arrives: friction sheds FRICTION of the
-- ball's speed per second, so a pass launched at v covers roughly v/FRICTION
-- before dying. Aim to reach the receiver with a touch of pace left.
---@param d number  -- distance to the receiver
---@return number speed
local function pass_speed_for(d)
    return math.min(PASS_SPEED_MAX, math.max(PASS_SPEED, PASS_ARRIVE_PACE + FRICTION * d))
end

-- Opposing outfielders as interception threats against a pass by `team`.
-- Keepers are excluded: they hold their box instead of chasing lanes.
---@param s MatchState
---@param team "home"|"away"
---@return Threat[]
local function pass_threats(s, team)
    local threats = {}
    for _, p in ipairs(s.players) do
        if p.team ~= team and not p.is_keeper then
            threats[#threats + 1] = { pos = p.pos, speed = p.move_speed }
        end
    end
    return threats
end

-- Earliest lane fraction where a chaser would cut out a driven ground pass
-- from->to (paced by pass_speed_for), or nil when the pass outruns everyone.
---@param from Vec2
---@param to Vec2
---@param threats Threat[]
---@return number? fraction
local function pass_risk(from, to, threats)
    local speed = pass_speed_for(from:dist(to))
    return ai.pass_intercept(from, to, speed, FRICTION, threats, POSSESS_DIST, POSSESS_MAX_SPEED)
end

-- Release a pass from `owner_idx` to teammate `target_idx`: fires the event,
-- paces the ball by distance (or lobs it over `blocker_f`), and sets the
-- receiver running onto it so passes are met instead of left to roll dead.
---@param s MatchState
---@param owner_idx integer
---@param target_idx integer
---@param blocker_f number?  -- lob over this lane fraction; nil = driven ground pass
---@param clear_h number?  -- loft clearance height (defaults to a foot lob)
---@param land_pos Vec2?  -- planned landing point (keeper throws); default: the receiver
local function release_pass(s, owner_idx, target_idx, blocker_f, clear_h, land_pos)
    local owner = s.players[owner_idx]
    local target = s.players[target_idx]
    -- Control follows a HUMAN pass to its receiver (standard soccer-game
    -- behavior): you take over the man the ball is travelling to — attack the
    -- cross, time the first touch — while it is still in flight. A back-pass
    -- is the exception: the keeper AI steps out to meet it, and control hands
    -- over in step() the moment the keeper traps it (see "Keeper control").
    if
        not s.slot_mode
        and is_human_player(s, owner_idx)
        and target.team == "home"
        and not target.is_keeper
    then
        s.controlled = target_idx
    end
    -- A defender right on the release point eats a driven ball — and even a lob
    -- is still low in its first strides (the lane check ignores segment ends).
    -- Dink over them: an arc that clears at 15% of the lane also stays above
    -- head height through the middle, so any mid-lane blocker is cleared too.
    -- (A planned throw — land_pos set — already cleared its own lane; the dink
    -- would LOWER its arc back into the presser's reach.)
    if not land_pos then
        local dirn = target.pos:sub(owner.pos):normalized()
        for qi, q in ipairs(s.players) do
            -- ANY body on the release line eats a driven ball — an adjacent
            -- TEAMMATE ricochets it just like a defender does. Dink over both
            -- (but never over the intended receiver).
            if qi ~= owner_idx and qi ~= target_idx and not q.is_keeper then
                local off = q.pos:sub(owner.pos)
                local d = off:length()
                if d < RELEASE_DINK_DIST and (off.x * dirn.x + off.y * dirn.y) > d * 0.2 then
                    blocker_f = 0.15 -- clear them just after the release
                    break
                end
            end
        end
    end
    -- A keeper receiver gets a long window: it must keep coming for the pass
    -- (or its dying roll) until the ball is actually resolved, not for a beat.
    target.receive_timer = target.is_keeper and KEEPER_RECEIVE_TIME or RECEIVE_TIME
    s.events[#s.events + 1] = { kind = "pass", x = s.ball.x, y = s.ball.y, player = owner.id }
    s.owner = nil
    s.kickoff_hold = 0
    s.ball_z = 0
    s.ball_spin = 0
    s.pickup_cd = RELEASE_CD
    s.block_grace = BLOCK_GRACE
    if blocker_f then
        s.ball_vel, s.ball_vz =
            lob_launch(owner.pos, land_pos or target.pos, blocker_f, clear_h or LOB_CLEAR_H)
    else
        local d = owner.pos:dist(target.pos)
        local pass_speed = pass_speed_for(d) * species.link_pass_speed(owner.owned_verb)
        -- Lead a MOVING receiver into their run (a fraction of the flight
        -- time) so the ball meets their stride instead of their heels.
        local aim_pt = target.pos:add(target.vel:scale(d / pass_speed * PASS_LEAD))
        s.ball_vel = aim_pt:sub(owner.pos):normalized():scale(pass_speed)
        s.ball_vz = 0
    end
end

-- Pure receiver selection for an outfield pass: returns the player index that
-- would receive a pass if released right now, or nil if nobody is available.
-- The own keeper is a valid receiver like anyone else — but only via the aim
-- cone (a deliberate back-pass); the openness fallback never panics it home.
-- Aim SQUARE at the keeper (best-aligned of all candidates, within
-- BACKPASS_AIM_COS) and the keeper wins outright: the generic scoring's
-- distance penalty must not hand a long deliberate back-pass to a mid-lane
-- defender instead.
-- Does NOT draw from s.rng — deterministic, safe to call every frame for preview.
---@param s MatchState
---@param owner_idx integer
---@param lofted boolean?
---@param aim Vec2?
---@param range number?
---@return integer? target_idx
local function select_pass_target(s, owner_idx, lofted, aim, range)
    local owner = s.players[owner_idx]
    aim = aim or owner.facing
    local cand, positions, opp_positions = {}, {}, {}
    for i, p in ipairs(s.players) do
        if p.team == owner.team and i ~= owner_idx then
            cand[#cand + 1] = i
            positions[#positions + 1] = p.pos
        elseif p.team ~= owner.team then
            opp_positions[#opp_positions + 1] = p.pos
        end
    end
    -- Deliberate back-pass: the keeper is the best-aligned candidate and the
    -- aim points near-square at it — it receives, however far it stands.
    do
        local naim = aim:normalized()
        if naim.x ~= 0 or naim.y ~= 0 then
            local best_cos, best_idx
            for k, pk in ipairs(positions) do
                local to = pk:sub(owner.pos)
                local d = to:length()
                if d > 1 then
                    local cos = (to.x * naim.x + to.y * naim.y) / d
                    if not best_cos or cos > best_cos then
                        best_cos, best_idx = cos, cand[k]
                    end
                end
            end
            if best_idx and s.players[best_idx].is_keeper and best_cos >= BACKPASS_AIM_COS then
                return best_idx
            end
        end
    end
    local rel, pick_cand, pick_pos
    if not lofted then
        local threats = pass_threats(s, owner.team)
        local safe_cand, safe_pos = {}, {}
        for k, idx in ipairs(cand) do
            if not pass_risk(owner.pos, positions[k], threats) then
                safe_cand[#safe_cand + 1] = idx
                safe_pos[#safe_pos + 1] = positions[k]
            end
        end
        rel = passing.target(owner.pos, aim, safe_pos, range)
        if rel then
            pick_cand, pick_pos = safe_cand, safe_pos
        end
    end
    if not rel then
        rel = passing.target(owner.pos, aim, positions, range)
        if not rel then
            local best_fb
            for k, pk in ipairs(positions) do
                -- Fallback (nobody in the cone) considers outfielders only: an
                -- unaimed pass must never dump the ball back at the keeper.
                if not s.players[cand[k]].is_keeper then
                    local open = math.huge
                    for _, qp in ipairs(opp_positions) do
                        open = math.min(open, qp:dist(pk))
                    end
                    local score = math.min(open, 80) - owner.pos:dist(pk) * 0.15
                    if not best_fb or score > best_fb then
                        best_fb, rel = score, k
                    end
                end
            end
        end
        if not rel then
            return nil
        end
        pick_cand, pick_pos = cand, positions
    end
    -- Cross override (lofted from wide in attacking third): redirect to box runner.
    if lofted then
        local third = (owner.team == "home") and (owner.pos.x > s.field.w * 0.62)
            or (owner.team == "away" and owner.pos.x < s.field.w * 0.38)
        local wide = math.abs(owner.pos.y - s.field.h / 2) > 120
        if third and wide then
            local best_k, best_d
            for k, i2 in ipairs(pick_cand) do
                local q = s.players[i2].pos
                local depth = (owner.team == "home") and (s.field.w - q.x) or q.x
                if depth < 220 and math.abs(q.y - s.field.h / 2) < 140 then
                    local gd = depth + math.abs(q.y - s.field.h / 2)
                    if not best_d or gd < best_d then
                        best_d, best_k = gd, k
                    end
                end
            end
            if best_k then
                rel = best_k
            end
        end
    end
    return pick_cand[rel]
end

-- Pure receiver selection for a keeper throw: returns the player index that
-- would receive the throw if released right now, or nil if nobody is available.
-- Does NOT draw from s.rng — deterministic, safe to call every frame for preview.
---@param s MatchState
---@param keeper_idx integer
---@param range number
---@param aim Vec2?
---@return integer? target_idx
local function select_throw_target(s, keeper_idx, range, aim)
    local keeper = s.players[keeper_idx]
    aim = aim or keeper.facing
    local cand, positions, opp_positions = {}, {}, {}
    for i, p in ipairs(s.players) do
        if p.team == keeper.team and i ~= keeper_idx and not p.is_keeper then
            cand[#cand + 1] = i
            positions[#positions + 1] = p.pos
        elseif p.team ~= keeper.team then
            opp_positions[#opp_positions + 1] = p.pos
        end
    end
    local naim = aim:normalized()
    local rel, best_score
    if naim.x ~= 0 or naim.y ~= 0 then
        for k, pk in ipairs(positions) do
            local to = pk:sub(keeper.pos)
            local d = to:length()
            if d > 1 then
                local cos = (to.x * naim.x + to.y * naim.y) / d
                if cos >= 0.5 then
                    local tf = math.min(
                        1,
                        math.max(math.sqrt(2 * THROW_CLEAR_H / (GRAVITY * 0.25)), d / MAX_LOB_VH)
                    )
                    local open = math.huge
                    for _, qp in ipairs(opp_positions) do
                        open = math.min(open, qp:dist(pk) - tf * 170)
                    end
                    local score = cos * 4
                        - math.abs(d - range) / 150
                        + math.max(0, math.min(open, 100)) / 40
                    if not best_score or score > best_score then
                        best_score, rel = score, k
                    end
                end
            end
        end
    end
    if not rel then
        local best_fb
        for k, pk in ipairs(positions) do
            local d = keeper.pos:dist(pk)
            local tf = math.min(
                1,
                math.max(math.sqrt(2 * THROW_CLEAR_H / (GRAVITY * 0.25)), d / MAX_LOB_VH)
            )
            local open = math.huge
            for _, qp in ipairs(opp_positions) do
                open = math.min(open, qp:dist(pk) - tf * 170)
            end
            local score = math.min(open, 100) - math.abs(d - range) * 0.2
            if not best_fb or score > best_fb then
                best_fb, rel = score, k
            end
        end
    end
    if not rel then
        return nil
    end
    return cand[rel]
end

---@param s MatchState
---@param owner_idx integer
---@param lofted boolean?  -- lob the pass over a defender on the lane
local function try_pass(s, owner_idx, lofted, aim)
    local owner = s.players[owner_idx]
    aim = aim or owner.facing
    -- Hold-to-charge picks the RANGE: a tap prefers someone close, a charged
    -- release picks out the long option along the aim.
    local range = (owner.pass_charge > 0.12)
            and (PASS_RANGE_MIN + owner.pass_charge * (TUNE.PASS_RANGE_MAX - PASS_RANGE_MIN))
        or nil
    local target_idx = select_pass_target(s, owner_idx, lofted, aim, range)
    if not target_idx then
        return
    end
    -- Determine loft: cross gets CROSS_CLEAR_H; regular lob gets lane-blocker fraction.
    local opp_positions = {}
    for _, p in ipairs(s.players) do
        if p.team ~= owner.team then
            opp_positions[#opp_positions + 1] = p.pos
        end
    end
    local target_pos = s.players[target_idx].pos
    local clear_h = nil
    local f = nil
    if lofted then
        local third = (owner.team == "home") and (owner.pos.x > s.field.w * 0.62)
            or (owner.team == "away" and owner.pos.x < s.field.w * 0.38)
        local wide = math.abs(owner.pos.y - s.field.h / 2) > 120
        if third and wide then
            local depth = (owner.team == "home") and (s.field.w - target_pos.x) or target_pos.x
            if depth < 220 and math.abs(target_pos.y - s.field.h / 2) < 140 then
                clear_h = CROSS_CLEAR_H
            end
        end
        f = ai.lane_blocker(owner.pos, target_pos, opp_positions, POSSESS_DIST) or 0.5
    end
    release_pass(s, owner_idx, target_idx, f, clear_h)
end

-- Plan a keeper HAND throw to `target_idx`. Hands see the whole pitch: a
-- throw is not a hopeful ball, it is placed. Two tools, composable:
--   - a COVERED receiver gets the ball landed to their safe side — away from
--     the nearest opponent but close enough to run onto (receive_timer);
--   - any opponent near the flight lane raises the arc ABOVE the aerial
--     strike envelope (aerial.MAX_TOUCH_Z): the flight cannot be met at all,
--     and the receiver takes it out of the air (aerial reception) or off the
--     bounce at the safe landing spot.
-- With nobody near the lane the throw stays the old low, quick float.
---@param s MatchState
---@param keeper MatchPlayer
---@param target_idx integer
---@return Vec2 land, number f, number clear_h
local function plan_throw(s, keeper, target_idx)
    local target = s.players[target_idx]
    local near_d, near_opp
    for _, q in ipairs(s.players) do
        if q.team ~= keeper.team then
            local d = q.pos:dist(target.pos)
            if not near_d or d < near_d then
                near_d, near_opp = d, q
            end
        end
    end
    local land = target.pos
    if near_opp and near_d < THROW_COVER_DIST then
        local away = target.pos:sub(near_opp.pos)
        -- Cover standing ON the receiver: push the landing toward the corner
        -- the keeper is facing away from goal — any consistent safe side.
        local dir = (away:length() > 1) and away:normalized() or keeper.facing
        local lead = math.min(THROW_LEAD_MAX, (THROW_COVER_DIST - near_d) * 0.5)
        land = Vec2.new(
            math.max(25, math.min(s.field.w - 25, target.pos.x + dir.x * lead)),
            math.max(25, math.min(s.field.h - 25, target.pos.y + dir.y * lead))
        )
    end
    local opp_positions = {}
    for _, q in ipairs(s.players) do
        if q.team ~= keeper.team then
            opp_positions[#opp_positions + 1] = q.pos
        end
    end
    local f = ai.lane_blocker(keeper.pos, land, opp_positions, THROW_LANE_W)
    if f then
        return land, math.max(0.2, math.min(0.8, f)), THROW_SAFE_CLEAR
    end
    return land, 0.5, THROW_CLEAR_H
end

-- Human keeper throw: aimed like a pass (facing cone), the charged range
-- picking WHICH teammate; the flight comes from plan_throw (uninterferable).
---@param s MatchState
---@param keeper_idx integer
---@param range number
local function keeper_throw(s, keeper_idx, range, aim)
    local keeper = s.players[keeper_idx]
    aim = aim or keeper.facing
    local target_idx = select_throw_target(s, keeper_idx, range, aim)
    if not target_idx then
        return
    end
    local land, f, clear_h = plan_throw(s, keeper, target_idx)
    release_pass(s, keeper_idx, target_idx, f, clear_h, land)
    keeper.throw_timer = KEEPER_THROW_POSE
end

-- AI carrier under pressure: pick the most open, most progressive teammate with
-- a workable lane and play it to them (lobbed if the lane is blocked). Returns
-- true if a pass was released. Deterministic; ties resolve to the lowest index.
---@param s MatchState
---@param owner_idx integer
---@return boolean passed
local function ai_try_pass(s, owner_idx)
    local owner = s.players[owner_idx]
    local fwd = (owner.team == "home") and 1 or -1
    local opp_positions = {}
    for _, p in ipairs(s.players) do
        if p.team ~= owner.team then
            opp_positions[#opp_positions + 1] = p.pos
        end
    end
    local threats = pass_threats(s, owner.team)
    local best, best_score, best_f
    for i, p in ipairs(s.players) do
        if p.team == owner.team and i ~= owner_idx and not p.is_keeper then
            local d = owner.pos:dist(p.pos)
            local open = math.huge
            for _, qp in ipairs(opp_positions) do
                open = math.min(open, qp:dist(p.pos))
            end
            if d >= AI_PASS_MIN_DIST and d <= AI_PASS_MAX_DIST and open >= AI_PASS_MIN_OPEN then
                -- Openness, upfield progress, and a mild preference for short. A
                -- statically clear lane a chaser could still cut is penalized; a
                -- blocked lane gets lobbed anyway, so it carries no extra malus.
                local blocked = ai.lane_blocker(owner.pos, p.pos, opp_positions, POSSESS_DIST)
                local risk = not blocked and pass_risk(owner.pos, p.pos, threats) or nil
                local score = open
                    + (p.pos.x - owner.pos.x) * fwd * 0.6
                    - d * 0.25
                    - (risk and AI_PASS_RISK_PENALTY or 0)
                if not best_score or score > best_score then
                    -- Lob over a static blocker — or over the point where a
                    -- chaser would step onto a ground ball.
                    best_score, best, best_f = score, i, blocked or risk
                end
            end
        end
    end
    if not best then
        return false
    end
    release_pass(s, owner_idx, best, best_f)
    return true
end

-- AI cross: from the flank, loft the ball to the teammate best placed in the
-- box. Returns true if a cross was released.
---@param s MatchState
---@param owner_idx integer
---@return boolean
local function ai_try_cross(s, owner_idx)
    local owner = s.players[owner_idx]
    local best, best_d
    for i, p in ipairs(s.players) do
        if p.team == owner.team and i ~= owner_idx and not p.is_keeper then
            local depth = (owner.team == "home") and (s.field.w - p.pos.x) or p.pos.x
            if depth < 220 and math.abs(p.pos.y - s.field.h / 2) < 140 then
                local gd = depth + math.abs(p.pos.y - s.field.h / 2)
                if not best_d or gd < best_d then
                    best_d, best = gd, i
                end
            end
        end
    end
    if not best then
        return false
    end
    release_pass(s, owner_idx, best, 0.5, CROSS_CLEAR_H)
    return true
end

---@param s MatchState
---@param team "home"|"away"
---@return Rect
local function attack_goal(s, team)
    return team == "home" and s.goal_away or s.goal_home
end

---@param s MatchState
---@param team "home"|"away"
---@return MatchPlayer?
local function team_keeper(s, team)
    for _, p in ipairs(s.players) do
        if p.team == team and p.is_keeper then
            return p
        end
    end
    return nil
end

-- World point in the opponent goal to aim at. `vbias` in [-1, 1] picks vertical
-- placement (0 = centre, +/-1 = the posts).
---@param s MatchState
---@param shooter MatchPlayer
---@param vbias number
---@return Vec2
local function shot_target(s, shooter, vbias)
    local g = attack_goal(s, shooter.team)
    -- The scoring plane: the goal line itself (field boundary).
    local gx = (shooter.team == "home") and g.x or (g.x + g.w)
    local cy = g.y + g.h / 2
    local half = g.h / 2 - 8
    return Vec2.new(gx, cy + math.max(-1, math.min(1, vbias)) * half)
end

-- True when the ball is inside the keeper's claim zone (its penalty area):
-- close to its own goal line and within the mouth ± a margin. The keeper comes
-- off its line to gather loose balls here and to close down a carrier.
---@param s MatchState
---@param keeper MatchPlayer
---@return boolean
local function in_claim_zone(s, keeper)
    local g = (keeper.team == "home") and s.goal_home or s.goal_away
    local depth = (keeper.team == "home") and s.ball.x or (s.field.w - s.ball.x)
    return depth <= KEEPER_BOX_DEPTH
        and s.ball.y >= g.y - KEEPER_BOX_PAD
        and s.ball.y <= g.y + g.h + KEEPER_BOX_PAD
end

-- Where a keeper holds a gathered ball: at its hands, but clamped safely inside
-- the line so the hold itself can never read as a goal in check_goal (which
-- counts ball + radius).
---@param s MatchState
---@param keeper MatchPlayer
---@return Vec2
local function keeper_hold_pos(s, keeper)
    local hold_x = keeper.pos.x
    if keeper.team == "home" then
        hold_x = math.max(hold_x, s.goal_home.x + s.goal_home.w + BALL_RADIUS + 1)
    else
        hold_x = math.min(hold_x, s.goal_away.x - BALL_RADIUS - 1)
    end
    return Vec2.new(hold_x, keeper.pos.y)
end

-- Knock the ball loose when a challenger reaches THE BALL — not the carrier's
-- body. The ball sticks a step ahead of the carrier's feet, so a carrier who
-- turns their body between the challenger and the ball SHIELDS it: challenges
-- from behind come up short. The human challenges with a standing poke or a
-- slide (longer reach); an AI defender COMMITS to a poke as soon as the ball
-- looks reachable and pays its cooldown even on a whiff, so a carrier who keeps
-- moving makes defenders miss. A stunned defender can't tackle. The ball pops
-- toward the challenger so a clean tackle tends to win possession; a slide also
-- knocks the carrier down.
---@param s MatchState
---@param combat_state CombatMatchState?
local function attempt_steals(s, combat_state)
    local combat_module = combat_state and require("sim.combat") or nil
    if not s.owner then
        return
    end
    local owner = s.players[s.owner]
    if owner.is_keeper and not owner.feet_ball then
        return -- a keeper has the ball in hand: it can't be tackled off them
    end
    if owner.dodge_timer > 0 then
        return -- juke i-frames: the carrier can't be tackled mid-dodge
    end
    if s.ball_z > GROUND_GRAB_HEIGHT then
        return -- ball is in the air, not at the carrier's feet (owned ball is grounded)
    end
    -- Keeper smother: a carrier who brings the ball into the keeper's box gets it
    -- picked straight off their feet (into the keeper's hands, not knocked loose).
    -- This is the 1v1 close-down; without it a carrier could walk the ball in.
    for i, p in ipairs(s.players) do
        if
            p.is_keeper
            and p.team ~= owner.team
            and p.stun_timer <= 0
            and in_claim_zone(s, p)
            and p.pos:dist(s.ball) <= KEEPER_SMOTHER
        then
            s.events[#s.events + 1] = { kind = "claim", x = s.ball.x, y = s.ball.y, player = p.id }
            -- Cancel any pending wind-up: the smother beats the shot.
            owner.windup_timer = 0
            owner.windup_shot = nil
            s.owner = i
            s.ball_vel = Vec2.new(0, 0)
            s.ball_spin = 0
            p.grab_timer = KEEPER_GRAB_POSE
            p.hold_timer = KEEPER_HOLD
            p.feet_ball = false
            return
        end
    end
    for i, p in ipairs(s.players) do
        if
            p.team ~= owner.team
            and not p.is_keeper
            and p.stun_timer <= 0
            and (not combat_module or not combat_module.blocks_actions(combat_state, i))
        then
            local human = is_human_player(s, i)
            local d = p.pos:dist(s.ball) -- reach for the ball: shielding matters
            local species_reach = species.collision_reach(p.owned_verb)
                - species.dribble_protection(owner.owned_verb)
            -- The human's poke also works at body-contact range from ANY angle
            -- (a toe through the legs): chasing a carrier is the default
            -- defensive situation and must be winnable. AI challenges stay
            -- strictly ball-side, so the human's own shielding keeps working.
            if human and p.pos:dist(owner.pos) <= STEAL_DIST + species_reach then
                d = math.min(d, STEAL_DIST)
            end
            local sliding = false
            local active, reach = false, STEAL_DIST
            if human then
                if p.slide_timer > 0 then
                    active, reach, sliding = true, SLIDE_REACH, true
                elseif p.tackle_timer > 0 then
                    -- A poke released from jockey stance gets bonus reach: the
                    -- defender committed to the shadow and earned a clean strike.
                    local jockey_bonus = (p.jockey_timer > 0) and JOCKEY_REACH_BONUS or 0
                    active, reach = true, STAND_REACH + jockey_bonus
                end
            elseif p.dash_cd <= 0 and d <= TUNE.STEAL_ATTEMPT then
                -- The AI pokes as soon as the ball looks reachable — and goes on
                -- cooldown whether or not it connects (a whiff is the carrier's
                -- window to escape).
                active = true
                p.dash_cd = TUNE.AI_STEAL_CD
                p.tackle_timer = STAND_TIMER -- poke pose for the renderer
                if d > STEAL_DIST then
                    -- Lunged past a shielded ball: stumble briefly. Baiting the
                    -- poke and breaking away is the carrier's core move.
                    p.stun_timer = math.max(p.stun_timer, TUNE.WHIFF_STUMBLE)
                end
            end
            if active and d <= reach + species_reach then
                local dir = p.pos:sub(owner.pos)
                if dir.x == 0 and dir.y == 0 then
                    dir = p.facing
                end
                s.events[#s.events + 1] =
                    { kind = "tackle", x = owner.pos.x, y = owner.pos.y, player = p.id }
                -- Cancel any pending wind-up on the carrier: the tackle beats the shot.
                owner.windup_timer = 0
                owner.windup_shot = nil
                for _, player in ipairs(s.players) do
                    player.keeper_set = 0
                end
                s.owner = nil
                s.ball_vel = dir:normalized():scale(TACKLE_POP_SPEED)
                s.pickup_cd = 0.12
                if sliding then
                    owner.stun_timer = math.max(owner.stun_timer, STUN_TIME) -- slide knocks them down
                end
                return
            end
        end
    end
end

-- Off-ball movement tuning (see docs/plan). All deterministic.
local TRIANGLE_JOIN = 320 -- supporters this close to the carrier triangulate around them
-- Positional calm: an off-ball player close enough to their role spot PLANTS
-- and stands (no robotic shuffling), only walking again once the spot drifts
-- meaningfully away. Urgent roles (chasing, receiving, pressing) are exempt.
local ARRIVE_RADIUS = 60 -- ease in below this distance (no full-speed overshoot)
local KEEPER_BASE_ARRIVE_RADIUS = 18 -- settle shallow base motion without erasing active tracking
local STAND_DEADBAND = 14 -- close enough: plant
local STAND_STILL_SPEED = 25 -- run_vel below this counts as standing (hysteresis memory)
local PURSUE_LEAD = 0.004 -- prediction horizon per px of distance (s/px)
local MARK_GOALSIDE = 16 -- px a marker stands goal-side of its man
local MARK_LANE_OFF = 44 -- ...but during a keeper's build-up they mark the LANE instead:
-- standing well off the outlet (toward goal) they can step into a pass, without
-- smothering the receiver's first touch the moment the throw arrives.
local COVER_FRAC = 0.3 -- cover sits this fraction from carrier toward own goal
local BLOCK_SHIFT = 0.45 -- how far the block slides toward the ball (× compactness)
local ATTACK_PUSH = 90 -- px an off-ball attacker pushes upfield (× support)
local SUPPORT_FAN = 70 -- candidate spread around an attacker's base
local SEP_RADIUS = 64 -- teammates within this repel each other
local SEP_PUSH = 16 -- px weight applied to the separation offset
local MARK_STICK = 20 -- hysteresis: keep a mark unless another is this much closer

---@param s MatchState
---@param team "home"|"away"
---@return Vec2
local function own_goal_center(s, team)
    local g = (team == "home") and s.goal_home or s.goal_away
    return Vec2.new(g.x + g.w / 2, g.y + g.h / 2)
end

-- Slide an anchor toward the ball without losing shape (block shift).
---@param anchor Vec2
---@param ball Vec2
---@param compactness number
---@return Vec2
local function block_shift(anchor, ball, compactness)
    return anchor:add(ball:sub(anchor):scale(BLOCK_SHIFT * compactness))
end

-- A marker stands just goal-side of its man, leading the man's motion.
---@param defpos Vec2
---@param opp_pos Vec2
---@param opp_vel Vec2
---@param goal Vec2
---@return Vec2
local function marker_target(defpos, opp_pos, opp_vel, goal, off)
    local aim = ai.pursue(defpos, opp_pos, opp_vel, PURSUE_LEAD)
    return aim:add(goal:sub(aim):normalized():scale(off or MARK_GOALSIDE))
end

-- Compute off-ball steering targets for every AI player NOT handled by the
-- controlled / owner / keeper branches. Pure function of the top-of-tick
-- snapshot `pos`. Returns player_index -> target Vec2 plus an URGENCY set
-- (roles needing full-speed precision, exempt from positional calm), and
-- refreshes s.marks for man-marking hysteresis. Roles: one presser + one cover
-- + scheme-driven rest when defending; support-spot runs when attacking;
-- press-set chase when loose.
---@param s MatchState
---@param pos Vec2[]
---@return table<integer, Vec2> targets
---@return table<integer, boolean> urgent
local function offball_targets(s, pos)
    local targets = {}
    local urgent = {} -- roles that need precision, exempt from positional calm
    local owner_team = s.owner and s.players[s.owner].team or nil

    for _, team in ipairs({ "home", "away" }) do
        local cfg = s.marking[team]
        local goal = own_goal_center(s, team)
        local atk = (team == "home") and 1 or -1

        -- This team's off-ball outfielders (exclude keeper, ball-owner, human).
        local mine = {}
        for i, p in ipairs(s.players) do
            if
                p.team == team
                and not p.is_keeper
                and i ~= s.owner
                and not is_human_player(s, i)
            then
                mine[#mine + 1] = i
            end
        end
        -- Opponents: all (for openness/lanes) and outfield-only (for marking).
        local opp_all_pos, opp_out, opp_out_pos = {}, {}, {}
        for i, p in ipairs(s.players) do
            if p.team ~= team then
                opp_all_pos[#opp_all_pos + 1] = pos[i]
                if not p.is_keeper then
                    opp_out[#opp_out + 1] = i
                    opp_out_pos[#opp_out_pos + 1] = pos[i]
                end
            end
        end

        -- Teammate positions for separation (spread out, don't stack).
        local function sep(idx, target)
            local others = {}
            for _, j in ipairs(mine) do
                if j ~= idx then
                    others[#others + 1] = pos[j]
                end
            end
            return target:add(ai.separation(target, others, SEP_RADIUS):scale(SEP_PUSH))
        end

        if owner_team and owner_team ~= team then
            -- DEFENDING: rank defenders by distance to the carrier.
            local carrier = s.players[s.owner]
            local cpos = pos[s.owner]
            local order = {}
            for _, idx in ipairs(mine) do
                order[#order + 1] = idx
            end
            table.sort(order, function(a, b)
                local da, db = pos[a]:dist(cpos), pos[b]:dist(cpos)
                if da ~= db then
                    return da < db
                end
                return a < b
            end)

            -- Kickoff law: the non-kicking team holds shape at a restart
            -- instead of pressing, until the first pass/shot (or the hold
            -- timer runs out as a failsafe).
            local held = s.kickoff_hold > 0
            local presser, cover = order[1], order[2]
            if presser then
                if held then
                    targets[presser] =
                        block_shift(s.players[presser].anchor, s.ball, cfg.compactness)
                else
                    -- Press the BALL, not the man: against a carrier who stands
                    -- still or turns to shield, the presser works around the body
                    -- to the ball side (collision makes it circle) and keeps
                    -- poking. Half the standoff keeps the press goal-side honest.
                    local aim = ai.pursue(pos[presser], s.ball, carrier.vel, PURSUE_LEAD)
                    targets[presser] = aim:add(goal:sub(aim):normalized():scale(cfg.standoff * 0.5))
                    urgent[presser] = true
                end
            end
            if cover then
                targets[cover] = held
                        and block_shift(s.players[cover].anchor, s.ball, cfg.compactness)
                    or ai.interpose(cpos, goal, COVER_FRAC)
            end

            -- The rest: which defenders should man-mark, which hold zone.
            local rest = {}
            for k = 3, #order do
                rest[#rest + 1] = order[k]
            end

            -- Pick the opponents to be man-marked (player indices into opp_out).
            local mark_locals = {}
            if cfg.scheme == "man" then
                for li = 1, #opp_out do
                    mark_locals[#mark_locals + 1] = li
                end
            elseif cfg.scheme == "hybrid" then
                local rank = {}
                for li = 1, #opp_out do
                    rank[#rank + 1] = li
                end
                table.sort(rank, function(a, b)
                    local da, db = opp_out_pos[a]:dist(goal), opp_out_pos[b]:dist(goal)
                    if da ~= db then
                        return da < db -- closest to our goal = most dangerous
                    end
                    return opp_out[a] < opp_out[b]
                end)
                for n = 1, math.min(cfg.man_marks, #rank) do
                    mark_locals[#mark_locals + 1] = rank[n]
                end
            end

            local newmarks = {}
            if #mark_locals > 0 and #rest > 0 then
                local restpos, markpos = {}, {}
                for _, idx in ipairs(rest) do
                    restpos[#restpos + 1] = pos[idx]
                end
                for _, li in ipairs(mark_locals) do
                    markpos[#markpos + 1] = opp_out_pos[li]
                end
                -- Build prev assignment in local indices for hysteresis.
                local prev_local = {}
                for di, pidx in ipairs(rest) do
                    local prev_opp = s.marks[team][pidx]
                    if prev_opp then
                        for mi, li in ipairs(mark_locals) do
                            if opp_out[li] == prev_opp then
                                prev_local[di] = mi
                            end
                        end
                    end
                end
                local map = ai.assign_marks(restpos, markpos, prev_local, MARK_STICK)
                for di, mi in pairs(map) do
                    local def_idx = rest[di]
                    local opp_idx = opp_out[mark_locals[mi]]
                    newmarks[def_idx] = opp_idx
                    -- Tight on a live carrier's teammates; lane distance while
                    -- the keeper surveys, so throws can actually be received.
                    local off = carrier.is_keeper and MARK_LANE_OFF or MARK_GOALSIDE
                    targets[def_idx] =
                        marker_target(pos[def_idx], pos[opp_idx], s.players[opp_idx].vel, goal, off)
                end
            end
            s.marks[team] = newmarks

            -- Any defender without a mark holds a ball-shifted zone.
            for _, idx in ipairs(rest) do
                if not targets[idx] then
                    targets[idx] = block_shift(s.players[idx].anchor, s.ball, cfg.compactness)
                end
            end
            for _, idx in ipairs(rest) do
                targets[idx] = sep(idx, targets[idx])
            end
        elseif owner_team == team then
            -- ATTACKING off the ball. When OUR KEEPER has it we're in build-up: hold
            -- stable, spread outlet positions (don't roam) so the keeper's throw
            -- reaches a teammate who's actually there. Otherwise make support runs.
            local build_up = s.players[s.owner].is_keeper
            local cpos = pos[s.owner]
            for _, idx in ipairs(mine) do
                if build_up then
                    targets[idx] = sep(idx, block_shift(s.players[idx].anchor, s.ball, 0.15))
                else
                    local a = s.players[idx].anchor
                    local base = Vec2.new(a.x + atk * ATTACK_PUSH * cfg.support, a.y)
                    local cands = {
                        base,
                        Vec2.new(base.x, base.y - SUPPORT_FAN),
                        Vec2.new(base.x, base.y + SUPPORT_FAN),
                        Vec2.new(base.x + atk * SUPPORT_FAN, base.y),
                        Vec2.new(base.x - atk * SUPPORT_FAN, base.y),
                    }
                    -- Triangulation: supporters near the play also consider
                    -- angular spots around the CARRIER at pass range (±40° and
                    -- ±80° off the attacking axis), so there is always a short
                    -- option to either side for a one-two.
                    if pos[idx]:dist(cpos) < TRIANGLE_JOIN then
                        for _, ang in ipairs({ -1.4, -0.7, 0.7, 1.4 }) do
                            local dirv = Vec2.new(math.cos(ang) * atk, math.sin(ang))
                            local c = cpos:add(dirv:scale(TUNE.TRIANGLE_DIST))
                            cands[#cands + 1] = Vec2.new(
                                math.max(20, math.min(s.field.w - 20, c.x)),
                                math.max(20, math.min(s.field.h - 20, c.y))
                            )
                        end
                    end
                    -- Clear the carrier's dribbling lane: drop candidates that
                    -- sit right ahead of them (don't clog the path).
                    local ahead = cpos:add(s.players[s.owner].facing:scale(120))
                    local open_cands = {}
                    for _, c in ipairs(cands) do
                        if c:dist(ahead) > 70 then
                            open_cands[#open_cands + 1] = c
                        end
                    end
                    if #open_cands == 0 then
                        open_cands = cands
                    end
                    targets[idx] =
                        sep(idx, ai.support_spot(cpos, open_cands, opp_all_pos, atk, s.field))
                end
            end
            s.marks[team] = {}
        else
            -- LOOSE ball: the press-set chases it with a pursuit lead, cutting
            -- off a rolling ball instead of trailing it. Passers already price
            -- this in: pass safety is interception-aware (ai.pass_intercept),
            -- not just a static lane check. Everyone else holds shape.
            local chasers = nearest_n(s, team, s.press[team])
            for _, idx in ipairs(mine) do
                -- The press-set chases — and so does ANYONE the ball lands
                -- near: a ball at your feet is yours to claim, whatever your
                -- assigned role (the ball magnet).
                if chasers[idx] or pos[idx]:dist(s.ball) < TUNE.LOOSE_MAGNET then
                    targets[idx] = ai.pursue(pos[idx], s.ball, s.ball_vel, PURSUE_LEAD)
                    urgent[idx] = true
                else
                    targets[idx] = block_shift(s.players[idx].anchor, s.ball, cfg.compactness)
                end
            end
            s.marks[team] = {}
        end
    end

    -- A designated receiver runs onto the incoming ball (overrides its other role)
    -- so a keeper's distribution is actually met and gathered, not left in space.
    for i, p in ipairs(s.players) do
        if p.receive_timer > 0 and targets[i] then
            targets[i] = Vec2.new(s.ball.x, s.ball.y)
            urgent[i] = true
        end
    end

    -- Hard retreat: you can't challenge a keeper holding the ball, so the opposing
    -- team must give it space — push any target inside the respect ring back out.
    -- A keeper playing a back-pass with the FEET gets no such protection.
    if s.owner and s.players[s.owner].is_keeper and not s.players[s.owner].feet_ball then
        local kpos = s.players[s.owner].pos
        local kteam = s.players[s.owner].team
        for i, tgt in pairs(targets) do
            if s.players[i].team ~= kteam then
                local off = tgt:sub(kpos)
                local d = off:length()
                if d < TUNE.KEEPER_RESPECT_DIST then
                    local dir = (d > 0) and off:normalized() or Vec2.new(1, 0)
                    targets[i] = kpos:add(dir:scale(TUNE.KEEPER_RESPECT_DIST))
                end
            end
        end
    end

    return targets, urgent
end

-- Resolve player-vs-player overlaps so bodies block instead of passing through.
-- Each pair pushed apart by its penetration; a sliding player barges through
-- (takes less of the push) and knocks the other off balance (stun). O(n^2)=45
-- pairs, deterministic.
---@param s MatchState
local function resolve_collisions(s)
    local pl = s.players
    for a = 1, #pl do
        for b = a + 1, #pl do
            local pa, pb = pl[a], pl[b]
            local delta = pa.pos:sub(pb.pos)
            local d = delta:length()
            local collision_reach = math.max(
                species.collision_reach(pa.owned_verb),
                species.collision_reach(pb.owned_verb)
            )
            local mind = pa.radius + pb.radius + collision_reach
            if d < mind then
                local dir = (d > 0) and delta:normalized() or Vec2.new(1, 0)
                local pen = mind - d
                local fa, fb = 0.5, 0.5 -- share the push evenly by default
                -- A defender leaning on the BALL CARRIER shoves them off their
                -- spot: standing still under pressure is never fully safe.
                if s.owner == a and pb.team ~= pa.team and pb.slide_timer <= 0 then
                    fa, fb = 0.7, 0.3
                elseif s.owner == b and pa.team ~= pb.team and pa.slide_timer <= 0 then
                    fa, fb = 0.3, 0.7
                end
                if pa.slide_timer > 0 and pb.slide_timer <= 0 then
                    fa, fb = 0.15, 0.85
                    if pb.stun_timer <= 0 then
                        pb.stun_timer = STUN_TIME
                    end
                elseif pb.slide_timer > 0 and pa.slide_timer <= 0 then
                    fa, fb = 0.85, 0.15
                    if pa.stun_timer <= 0 then
                        pa.stun_timer = STUN_TIME
                    end
                end
                pa.pos = clamp_to_field(s, pa.pos:add(dir:scale(pen * fa)))
                pb.pos = clamp_to_field(s, pb.pos:sub(dir:scale(pen * fb)))
            end
        end
    end
end

-- Locomotion helper: nudge `p.run_vel` toward `desired` by at most accel*dt
-- (DECEL when desired is zero), then move `p.pos` by run_vel*dt clamped to the
-- field. Updates `p.facing` to follow run_vel when the player is actually moving,
-- keeping the last facing when stationary so aim still works.
--
-- `desired` is the target velocity vector (direction × speed); passing zero
-- = stopping. Returns the new position (already applied to p.pos).
---@param s MatchState
---@param p MatchPlayer
---@param desired Vec2
---@param dt number
local function apply_locomotion(s, p, desired, dt)
    local dx = desired.x - p.run_vel.x
    local dy = desired.y - p.run_vel.y
    local diff_len = math.sqrt(dx * dx + dy * dy)
    local dlen = desired:length()
    -- Use DECEL when stopping (no input), ACCEL when steering toward any speed.
    local rate
    if dlen < 1 then
        rate = TUNE.MOVE_DECEL
    else
        -- Standing-start inertia: acceleration builds with momentum. From
        -- rest you push off at START_ACCEL and only reach full MOVE_ACCEL
        -- as speed builds — no 0-to-full-stride in an instant. A body
        -- already at speed redirects at full rate, so turns stay sharp.
        -- Normalized by the player's BASE speed (not the desired speed), so
        -- asking for a sprint never weakens the initial push-off.
        local momentum = math.min(1, p.run_vel:length() / math.max(1, p.move_speed))
        rate = TUNE.START_ACCEL + (TUNE.MOVE_ACCEL - TUNE.START_ACCEL) * momentum
    end
    local max_step = rate * dt
    if diff_len <= max_step then
        p.run_vel = desired
    else
        local scale = max_step / diff_len
        p.run_vel = Vec2.new(p.run_vel.x + dx * scale, p.run_vel.y + dy * scale)
    end
    p.pos = clamp_to_field(s, p.pos:add(p.run_vel:scale(dt)))
    -- Facing: follow run_vel when moving, keep last facing when stationary so
    -- a stopped player can aim without their facing snapping to zero.
    local rvlen = p.run_vel:length()
    if rvlen > RUN_VEL_FACE_MIN then
        p.facing = p.run_vel:normalized()
    end
end

---@param s MatchState
---@param carrier MatchPlayer
---@return number distance
---@return MatchPlayer? opponent
local function nearest_outfield_opponent(s, carrier)
    local best = math.huge
    local opponent = nil
    for _, p in ipairs(s.players) do
        if p.team ~= carrier.team and not p.is_keeper then
            local d = carrier.pos:dist(p.pos)
            if d < best then
                best = d
                opponent = p
            end
        end
    end
    return best, opponent
end

---@param p MatchPlayer
---@param want boolean
---@param dt number
local function update_sprint(p, want, dt)
    local can = p.sprint_meter > (p.sprinting and 0 or SPRINT_ENGAGE)
    p.sprinting = (want and can) or false
    if p.sprinting then
        p.sprint_meter = math.max(0, p.sprint_meter - dt / p.sprint_dur)
    else
        p.sprint_meter = math.min(1, p.sprint_meter + TUNE.SPRINT_REFILL * dt)
    end
end

---@param s MatchState
---@param player_index integer
---@param input MatchInput
---@return boolean
function match._aerial_active_for_input(s, player_index, input)
    local player = s.players[player_index]
    return player_index ~= s.owner
        and player.aerial_recovery <= 0
        and s.ball_z > GROUND_GRAB_HEIGHT
        and s.ball_vz < 0
        and player.pos:dist(s.ball) <= AERIAL_ANTICIPATE
        and (aerial.strike_requested(input) or player.receive_timer > 0)
end

---@param s MatchState
---@param dt number
---@param inputs table<integer, MatchInput>
---@param combat_state CombatMatchState?
local function move_players(s, dt, inputs, combat_state)
    local combat_module = combat_state and require("sim.combat") or nil
    -- Snapshot positions so role targets read one consistent world state and we
    -- can derive each player's realized velocity after everyone has moved. Vec2 is
    -- immutable, so aliasing p.pos here is safe.
    local prev = {}
    for i, p in ipairs(s.players) do
        prev[i] = p.pos
    end
    local targets, urgent = offball_targets(s, prev)

    for i, p in ipairs(s.players) do
        local combat_scale = combat_module and combat_module.movement_multiplier(combat_state, i)
            or 1
        local combat_blocked = combat_module and combat_module.blocks_actions(combat_state, i)
            or false
        if p.is_keeper and s.owner == i then
            p.keeper_state = "base"
            p.keeper_state_timer = 0
            p.keeper_release_state = nil
            p.keeper_release_motion = 0
            p.keeper_release_kind = nil
            p.keeper_release_depth = 0
        end
        if is_human_player(s, i) then
            local input = inputs[i] or slot_input.neutral_match_input()
            local aerial_active = match._aerial_active_for_input(s, i, input)
            -- Tackle button: a committed slide while SPRINTING, else a standing
            -- poke — one legible rule (sprint + tackle = the big slide). Slide
            -- speed scales off current velocity (p.vel) so it feels relative.
            if
                input.dash
                and not aerial_active
                and p.aerial_recovery <= 0
                and p.slide_timer <= 0
                and p.tackle_timer <= 0
                and p.tackle_cd <= 0
                and p.stun_timer <= 0
            then
                local sp = p.vel:length()
                if p.sprinting then
                    local d = (input.move.x ~= 0 or input.move.y ~= 0) and input.move:normalized()
                        or p.facing
                    p.slide_timer = SLIDE_DURATION
                    p.slide_dir = d
                    p.slide_vel = math.max(SLIDE_BASE_MIN, sp * SLIDE_MULT)
                    p.facing = d
                    p.tackle_cd = SLIDE_CD
                else
                    p.tackle_timer = STAND_TIMER
                    p.tackle_cd = STAND_CD
                end
            end
            -- Trigger a juke (not while sliding): a quick sidestep with tackle immunity.
            if
                input.dodge
                and p.dodge_cd <= 0
                and p.slide_timer <= 0
                and p.aerial_recovery <= 0
            then
                local perp = Vec2.new(-p.facing.y, p.facing.x)
                if input.move.x * perp.x + input.move.y * perp.y < 0 then
                    perp = perp:scale(-1)
                end
                p.dodge_timer = DODGE_DURATION
                p.dodge_cd = DODGE_CD
                p.dodge_dir = perp
                s.events[#s.events + 1] = { kind = "juke", x = p.pos.x, y = p.pos.y, player = p.id }
            end

            if p.slide_timer > 0 then
                -- Committed slide: locked direction, decaying speed, can't steer.
                -- Also drain run_vel so momentum doesn't carry after the slide ends.
                p.pos = clamp_to_field(s, p.pos:add(p.slide_dir:scale(p.slide_vel * dt)))
                p.slide_vel = p.slide_vel * math.max(0, 1 - SLIDE_FRICTION * dt)
                p.run_vel = Vec2.new(0, 0)
            elseif p.dodge_timer > 0 then
                -- Juke overrides steering: slide sideways fast.
                -- Drain run_vel so the exit from the juke starts from rest.
                p.pos = clamp_to_field(
                    s,
                    p.pos:add(p.dodge_dir:scale(p.move_speed * DODGE_SPEED_MULT * dt))
                )
                p.run_vel = Vec2.new(0, 0)
            else
                local dir = input.move
                local moving = dir.x ~= 0 or dir.y ~= 0
                -- Jockey stance (Space held off the ball): shadow the carrier at
                -- reduced speed, facing locked toward the ball. Mutually exclusive
                -- with sprint (jockey wins). Grants bonus poke reach on release.
                local jockeying = input.jockey
                    and i ~= s.owner
                    and p.stun_timer <= 0
                    and not aerial_active
                if jockeying then
                    p.jockey_timer = JOCKEY_HOLD
                end
                -- Sprint: needs a quarter tank to (re)engage, but once running it
                -- burns to empty — so a drained meter doesn't flicker the boost
                -- on and off at the refill rate.
                local want = input.sprint and moving and p.stun_timer <= 0 and not jockeying
                update_sprint(p, want, dt)
                local mv = p.move_speed * (p.stun_timer > 0 and STUN_SLOW or 1) * combat_scale
                if jockeying then
                    mv = mv * TUNE.JOCKEY_SLOW
                elseif p.sprinting then
                    mv = mv * TUNE.SPRINT_MULT * species.burst_speed(p.owned_verb)
                end
                -- Plant during wind-up: striker slows to 30% while winding up.
                if p.windup_timer > 0 then
                    mv = mv * WINDUP_MOVE
                end
                if p.aerial_recovery > 0 then
                    if p.aerial_style == "bicycle" then
                        mv = 0
                    elseif p.aerial_jump > 0 then
                        mv = mv * 0.35
                    elseif p.aerial_style == "leg_control" or p.aerial_style == "chest_control" then
                        mv = mv * 0.6
                    else
                        mv = mv * 0.55
                    end
                end
                -- Stationary-aiming exception: when input is held but the player
                -- hasn't built speed yet, facing should follow input, not run_vel.
                -- apply_locomotion handles facing via run_vel; for the input case
                -- we override after the call when run_vel is still tiny.
                local desired = moving and dir:normalized():scale(mv) or Vec2.new(0, 0)
                local had_input_facing = moving and p.run_vel:length() <= RUN_VEL_FACE_MIN
                -- Dribble hook: while the carrier's touch runs on ahead they are
                -- NOT free to run elsewhere — movement steers back to the ball
                -- (the chase half of kick-chase-kick), automatically when no
                -- input is held. The stick keeps choosing the FACING — where
                -- the NEXT touch goes — and free movement returns the moment
                -- the ball is back at the feet. Owners only: off the ball you
                -- run wherever you like.
                local hooked = i == s.owner
                    and not (p.is_keeper and not p.feet_ball)
                    and p.pos:dist(s.ball) > DRIBBLE_TOUCH_REACH
                if hooked then
                    local to_ball = s.ball:sub(p.pos)
                    if to_ball:length() > 1 then
                        desired = to_ball:normalized():scale(mv)
                    end
                elseif not moving and p.receive_timer > 0 and s.owner == nil then
                    -- Receive assist: the designated receiver of a pass works
                    -- to meet it by default — hold a direction to override
                    -- and attack a different spot instead.
                    local to_ball = s.ball:sub(p.pos)
                    if to_ball:length() > 1 then
                        desired = to_ball:normalized():scale(mv)
                    end
                end
                -- Aerial magnet: going up for a dropping ball nearby (holding the
                -- aerial button off the ball) glides the player toward it, so a
                -- cross is met without pixel-perfect positioning. It overrides
                -- the jockey slowdown and steers even from a standstill.
                local going_aerial = aerial_active
                local facing_before_aerial = p.facing
                if going_aerial and s.ball_z > GROUND_GRAB_HEIGHT and s.ball_vz < 0 then
                    local to_ball = s.ball:sub(p.pos)
                    local d = to_ball:length()
                    if d > 1 and d <= AERIAL_ANTICIPATE then
                        desired = desired:add(to_ball:normalized():scale(TUNE.AERIAL_MAGNET))
                        local cap = p.move_speed * 1.3
                        if desired:length() > cap then
                            desired = desired:normalized():scale(cap)
                        end
                    end
                end
                apply_locomotion(s, p, desired, dt)
                if going_aerial and aerial.acrobatic_requested(input) then
                    -- Bicycle geometry reads the approach facing; the contact
                    -- magnet must not rotate the player to face a ball behind.
                    p.facing = facing_before_aerial
                elseif jockeying then
                    -- Jockey stance: face the ball regardless of movement.
                    local ball_off = s.ball:sub(p.pos)
                    if ball_off:length() > 1 then
                        p.facing = ball_off:normalized()
                    end
                elseif (i == s.owner and moving) or had_input_facing then
                    -- A carrier's facing always obeys the stick — even while
                    -- hooked to a run-on ball — so the next touch turns the
                    -- dribble where you point, not where the chase ran.
                    p.facing = dir:normalized()
                end
            end
            -- A keeper holding the ball in its HANDS may not carry it out of the
            -- penalty area (the drawn box) — the laws, and the renderer, agree.
            -- Off the ball or with a back-pass at the feet it may roam.
            if p.is_keeper and s.owner == i and not p.feet_ball then
                local minx = (p.team == "home") and PLAYER_RADIUS or (s.field.w - PENALTY_DEPTH)
                local maxx = (p.team == "home") and PENALTY_DEPTH or (s.field.w - PLAYER_RADIUS)
                local top = s.field.h / 2 - PENALTY_H / 2 + PLAYER_RADIUS
                local bot = s.field.h / 2 + PENALTY_H / 2 - PLAYER_RADIUS
                p.pos = Vec2.new(
                    math.max(minx, math.min(maxx, p.pos.x)),
                    math.max(top, math.min(bot, p.pos.y))
                )
            end
        elseif i == s.owner then
            -- AI owner dribbles toward the opponent goal.
            if not p.is_keeper then
                local goal = (p.team == "home") and s.goal_away or s.goal_home
                local gc = Vec2.new(goal.x + goal.w / 2, goal.y + goal.h / 2)
                local pressure, threat = nearest_outfield_opponent(s, p)
                -- React on the NEXT tick after the tackle begins. Reading a
                -- same-frame human button here would make AI carriers psychic
                -- and invalidate a correctly timed challenge.
                local threat_committed = threat ~= nil
                    and (
                        (threat.tackle_timer > 0 and threat.tackle_timer < STAND_TIMER)
                        or (threat.slide_timer > 0 and threat.slide_timer < SLIDE_DURATION)
                    )
                if
                    threat_committed
                    and threat ~= nil
                    and not combat_blocked
                    and pressure <= TUNE.AI_JUKE_DIST
                    and p.dodge_cd <= 0
                    and p.dodge_timer <= 0
                    and p.stun_timer <= 0
                    and p.pos:dist(s.ball) <= DRIBBLE_TOUCH_REACH
                then
                    -- React to a defender who has actually committed: sidestep
                    -- away from their side, rather than spamming jukes on proximity.
                    local perp = Vec2.new(-p.facing.y, p.facing.x)
                    local to_threat = threat.pos:sub(p.pos)
                    if to_threat.x * perp.x + to_threat.y * perp.y > 0 then
                        perp = perp:scale(-1)
                    end
                    p.dodge_timer = DODGE_DURATION
                    p.dodge_cd = TUNE.AI_JUKE_CD
                    p.dodge_dir = perp
                    s.events[#s.events + 1] =
                        { kind = "juke", x = p.pos.x, y = p.pos.y, player = p.id }
                end
                local goal_dist = p.pos:dist(gc)
                local want_sprint = pressure >= TUNE.AI_SPRINT_SPACE
                    and not combat_blocked
                    and goal_dist > TUNE.AI_SHOOT_RANGE
                    and p.windup_timer <= 0
                    and p.dodge_timer <= 0
                    and p.stun_timer <= 0
                update_sprint(p, want_sprint, dt)
                local mv = p.move_speed * (p.stun_timer > 0 and STUN_SLOW or 1) * combat_scale
                if p.sprinting then
                    mv = mv * TUNE.SPRINT_MULT * species.burst_speed(p.owned_verb)
                end
                -- Plant during wind-up: AI striker slows to 30% while winding up.
                if p.windup_timer > 0 then
                    mv = mv * WINDUP_MOVE
                end
                -- Derive desired direction from ai.steer (which returns a clamped
                -- position and a unit direction), then feed apply_locomotion.
                local _, dir = ai.steer(p.pos, gc, mv * dt)
                local desired = (dir.x ~= 0 or dir.y ~= 0) and dir:scale(mv) or Vec2.new(0, 0)
                -- Dribble hook (same rule as the human carrier): chase the
                -- run-on touch before anything else, facing kept on the
                -- dribble line so the next touch continues toward goal.
                if p.pos:dist(s.ball) > DRIBBLE_TOUCH_REACH then
                    local to_ball = s.ball:sub(p.pos)
                    if to_ball:length() > 1 then
                        desired = to_ball:normalized():scale(mv)
                    end
                end
                if p.dodge_timer > 0 then
                    p.pos = clamp_to_field(
                        s,
                        p.pos:add(p.dodge_dir:scale(p.move_speed * DODGE_SPEED_MULT * dt))
                    )
                    p.run_vel = Vec2.new(0, 0)
                else
                    apply_locomotion(s, p, desired, dt)
                end
                if dir.x ~= 0 or dir.y ~= 0 then
                    p.facing = dir
                end
            else
                -- A keeper holding the ball faces upfield; if an opponent is camped
                -- right in front of it, step laterally to open a throwing angle.
                p.facing = Vec2.new((p.team == "home") and 1 or -1, 0)
                local camper
                for _, q in ipairs(s.players) do
                    if q.team ~= p.team and q.pos:dist(p.pos) < TUNE.KEEPER_RESPECT_DIST then
                        camper = q
                        break
                    end
                end
                if camper then
                    -- Sidestep away from the camper's side to open a throwing angle.
                    -- Use apply_locomotion for the keeper lateral step too.
                    local side = (camper.pos.y >= p.pos.y) and -1 or 1
                    apply_locomotion(s, p, Vec2.new(0, side * p.move_speed), dt)
                    -- Override facing: keeper always faces upfield.
                    p.facing = Vec2.new((p.team == "home") and 1 or -1, 0)
                else
                    apply_locomotion(s, p, Vec2.new(0, 0), dt)
                    p.facing = Vec2.new((p.team == "home") and 1 or -1, 0)
                end
            end
        elseif p.is_keeper then
            update_sprint(p, false, dt)
            local entry_motion = math.min(1, p.run_vel:length() / math.max(p.move_speed, 1))
            local goal = (p.team == "home") and s.goal_home or s.goal_away
            local goal_line_x = p.team == "home" and (goal.x + goal.w) or goal.x
            local infield_direction = p.team == "home" and 1 or -1
            local carrier_idx = s.owner
            local carrier = carrier_idx and s.players[carrier_idx] or nil
            if carrier and (carrier.is_keeper or carrier.team == p.team) then
                carrier = nil
            end

            local support_near = false
            local defender_engaged = false
            local carrier_pos ---@type Vec2?
            if carrier and carrier_idx then
                carrier_pos = prev[carrier_idx]
                for other_idx, other in ipairs(s.players) do
                    if
                        other_idx ~= carrier_idx
                        and not other.is_keeper
                        and other.team == carrier.team
                        and prev[other_idx]:dist(carrier_pos) <= KEEPER_1V1_SUPPORT
                    then
                        support_near = true
                    elseif
                        other.team == p.team
                        and not other.is_keeper
                        and prev[other_idx]:dist(carrier_pos) <= KEEPER_1V1_SUPPORT * 0.6
                    then
                        defender_engaged = true
                    end
                end
            end

            local loose_touch = carrier_pos ~= nil
                and carrier_pos:dist(s.ball) > DRIBBLE_TOUCH_REACH
            local attacker_controlled = carrier ~= nil and not loose_touch
            local attacker_in_front = (s.ball.x - p.pos.x) * infield_direction > 0
            local position_context = {
                in_claim_zone = in_claim_zone(s, p),
                attacker_controlled = attacker_controlled,
                loose_touch = loose_touch,
                support_near = support_near,
                defender_engaged = defender_engaged,
                threat_distance = p.pos:dist(s.ball),
            }
            local contain_eligible = carrier ~= nil
                and attacker_in_front
                and keeper.should_contain(position_context)
            local advance_eligible = contain_eligible and keeper.should_advance(position_context)

            local toward_goal = (p.team == "home") and s.ball_vel.x < 0 or s.ball_vel.x > 0
            if s.owner or not toward_goal then
                p.keeper_release_state = nil
                p.keeper_release_motion = 0
                p.keeper_release_kind = nil
                p.keeper_release_depth = 0
            end

            local ground_cue = p.keeper_release_kind == "ground" and toward_goal
            local lob_cue = p.keeper_release_kind == "chip" and toward_goal
            if carrier and carrier.windup_shot then
                local pending = assert(carrier.windup_shot)
                if pending.shot_type == "chip" then
                    lob_cue = true
                else
                    local shot_context = {
                        defending_team = p.team,
                        shooter_team = carrier.team,
                        origin = carrier.pos,
                        direction = pending.dir,
                        goal = goal,
                    }
                    if
                        (carrier.windup_timer <= 0 and keeper.shot_targets_goal(shot_context))
                        or keeper.should_set({
                            defending_team = shot_context.defending_team,
                            shooter_team = shot_context.shooter_team,
                            origin = shot_context.origin,
                            direction = shot_context.direction,
                            goal = shot_context.goal,
                            anticipation = p.keeper_anticipation,
                            windup_duration = TUNE.SHOT_WINDUP,
                            windup_remaining = carrier.windup_timer,
                        })
                    then
                        ground_cue = true
                        if carrier.windup_timer <= 0 then
                            -- Planting on the release tick must not erase the
                            -- velocity carried into it. The captured debt
                            -- survives until the save attempt.
                            p.keeper_release_motion = entry_motion
                        end
                    end
                end
            end

            local through_ball_cue = false
            if not s.owner and toward_goal and not p.keeper_release_kind then
                for _, receiver in ipairs(s.players) do
                    if receiver.team ~= p.team and receiver.receive_timer > 0 then
                        local receiver_depth = (receiver.pos.x - goal_line_x) * infield_direction
                        if receiver_depth <= KEEPER_BOX_DEPTH + KEEPER_1V1_SUPPORT * 0.5 then
                            through_ball_cue = true
                            break
                        end
                    end
                end
            end

            local behavior = keeper.behavior({
                current_state = p.keeper_state,
                state_timer = p.keeper_state_timer,
                keeper_pos = prev[i],
                ball_pos = s.ball,
                goal = goal,
                team = p.team,
                aggression = p.keeper_aggression,
                advance_eligible = advance_eligible,
                contain_eligible = contain_eligible,
                ground_cue = ground_cue,
                lob_cue = lob_cue,
                through_ball_cue = through_ball_cue,
                dt = dt,
            })
            p.keeper_state = behavior.state
            p.keeper_state_timer = behavior.state_timer
            if p.dive_timer > 0 or p.save_pending or p.dive_delay > 0 then
                p.keeper_set = 0
            elseif p.keeper_state == "set" then
                p.keeper_set = p.keeper_set + dt
            else
                p.keeper_set = 0
            end

            if p.dive_timer > 0 then
                -- Diving: lunge hard toward the intercept point — and STOP
                -- there. Unclamped, a near-straight shot (a 2px correction)
                -- became a full-speed lunge PAST the ball: gloves closing on
                -- empty air while the save resolved elsewhere. Dives bypass
                -- locomotion (bespoke movement — keep as-is).
                local step = p.move_speed * 1.6 * dt
                local to_target = p.dive_target and p.dive_target:sub(p.pos)
                if to_target and to_target:length() > 0.5 then
                    local dir = to_target:normalized()
                    p.pos =
                        clamp_to_field(s, p.pos:add(dir:scale(math.min(step, to_target:length()))))
                    p.facing = dir
                elseif not p.dive_target then
                    -- No known intercept (legacy path): the old straight lunge.
                    p.pos = clamp_to_field(s, p.pos:add(p.dive_dir:scale(step)))
                    p.facing = p.dive_dir
                end
                p.run_vel = Vec2.new(0, 0)
            elseif p.save_pending or p.dive_delay > 0 then
                -- A committed reaction owns the keeper until it launches or
                -- resolves. Decelerate through locomotion instead of layering
                -- positioning movement under the queued save.
                apply_locomotion(s, p, Vec2.new(0, 0), dt)
            elseif s.owner == nil and p.receive_timer > 0 then
                -- Meet a teammate's back-pass at the ball. Generic predictive
                -- pursuit is wrong here: its horizon grows with distance, so an
                -- incoming pass projects behind the keeper and sends it backward
                -- through the goal instead of forward to receive.
                local _, dir = ai.steer(p.pos, s.ball, p.move_speed * dt)
                local desired = (dir.x ~= 0 or dir.y ~= 0) and dir:scale(p.move_speed)
                    or Vec2.new(0, 0)
                apply_locomotion(s, p, desired, dt)
            elseif
                s.owner == nil
                and in_claim_zone(s, p)
                and not p.keeper_release_kind
                and not through_ball_cue
            then
                -- Come off the line to claim a loose ball in the box.
                -- Predictive pursuit remains useful for non-designated claims.
                local aim = ai.pursue(p.pos, s.ball, s.ball_vel, KEEPER_LEAD)
                local _, dir = ai.steer(p.pos, aim, p.move_speed * dt)
                local desired = (dir.x ~= 0 or dir.y ~= 0) and dir:scale(p.move_speed)
                    or Vec2.new(0, 0)
                apply_locomotion(s, p, desired, dt)
            else
                -- Ordinary keeper movement is owned by the explicit behavior
                -- state. Base uses shallow dynamic depth plus the lateral corner
                -- concession; contain/advance can commit further before recovery.
                local _, dir = ai.steer(p.pos, behavior.target, p.move_speed * dt)
                local movement_speed = p.move_speed * behavior.movement_scale
                if p.keeper_state == "base" then
                    -- Neutral targets are deliberately shallow. Ease into them
                    -- so ordinary acceleration cannot oscillate past the 18 px
                    -- base cap and accidentally advertise a committed high line.
                    local distance = p.pos:dist(behavior.target)
                    movement_speed = movement_speed
                        * math.min(1, distance / KEEPER_BASE_ARRIVE_RADIUS)
                end
                local desired = (dir.x ~= 0 or dir.y ~= 0) and dir:scale(movement_speed)
                    or Vec2.new(0, 0)
                apply_locomotion(s, p, desired, dt)
            end
        else
            update_sprint(p, false, dt)
            -- Off-ball AI: role-assigned target (press/cover/mark/support/zone).
            -- Positional roles have CALM: ease in on approach, plant inside the
            -- deadband, and once standing, stay planted until the spot drifts
            -- beyond the wake radius — no robotic shuffling on the spot.
            local target = targets[i] or p.anchor
            local mv = p.move_speed * (p.stun_timer > 0 and STUN_SLOW or 1) * combat_scale
            local dist = p.pos:dist(target)
            local standing = p.run_vel:length() < STAND_STILL_SPEED
            local desired = Vec2.new(0, 0)
            if urgent[i] or dist > (standing and TUNE.STAND_WAKE or STAND_DEADBAND) then
                local _, dir = ai.steer(p.pos, target, mv * dt)
                if dir.x ~= 0 or dir.y ~= 0 then
                    local speed = urgent[i] and mv or mv * math.min(1, dist / ARRIVE_RADIUS)
                    desired = dir:scale(speed)
                end
            end
            apply_locomotion(s, p, desired, dt)
        end
    end

    -- A keeper in possession is PHYSICALLY protected (laws of the game: you
    -- cannot challenge a keeper holding the ball). AI targets already retreat;
    -- this ring catches the human-controlled player and any straggler. Ball at
    -- the keeper's FEET (a received back-pass) is fair game — no ring.
    if s.owner and s.players[s.owner].is_keeper and not s.players[s.owner].feet_ball then
        local k = s.players[s.owner]
        for _, p in ipairs(s.players) do
            if p.team ~= k.team then
                local off = p.pos:sub(k.pos)
                local d = off:length()
                if d < TUNE.KEEPER_RESPECT_DIST then
                    local dir = (d > 0) and off:normalized() or Vec2.new(1, 0)
                    p.pos = clamp_to_field(s, k.pos:add(dir:scale(TUNE.KEEPER_RESPECT_DIST)))
                end
            end
        end
    end

    -- Push apart any overlapping bodies before deriving velocity, so a shove
    -- registers as motion and players never occupy the same point.
    resolve_collisions(s)

    -- Realized velocity (px/s) from this tick's movement; AI prediction source.
    if dt > 0 then
        for i, p in ipairs(s.players) do
            p.vel = p.pos:sub(prev[i]):scale(1 / dt)
        end
    end
end

-- Keeper distribution, in three tiers (all from the hands, deterministic,
-- ties by ascending index):
--   1. SAFE outlet (clear of opponents): a short throw — bowled along the ground
--      when the lane is clear, floated over the blocker when it isn't.
--   2. No safe outlet but someone is at least a step off their marker: a floated
--      throw that sails over the opponents to that most-open teammate.
--   3. Everyone swarmed: drop the ball and drop-kick it long upfield — a high
--      clearance over every head, not a flat drilled ball at the opponent goal.
---@param s MatchState
---@param keeper_idx integer
local function keeper_distribute(s, keeper_idx)
    local keeper = s.players[keeper_idx]
    -- Ball at the feet (received back-pass): distribution is KICKED — a normal
    -- foot pass (no throw pose, ordinary lob height), released immediately.
    local kicked = keeper.feet_ball
    local fwd = (keeper.team == "home") and 1 or -1 -- +x is upfield for home
    local opp = {}
    for _, q in ipairs(s.players) do
        if q.team ~= keeper.team then
            opp[#opp + 1] = q.pos
        end
    end

    local threats = pass_threats(s, keeper.team)
    local best, best_score, best_f
    local open_best, open_best_d
    for i, p in ipairs(s.players) do
        if p.team == keeper.team and not p.is_keeper then
            local opp_d = math.huge
            for _, qp in ipairs(opp) do
                opp_d = math.min(opp_d, qp:dist(p.pos))
            end
            if opp_d >= KEEPER_SAFE_DIST then
                -- A ground lane only counts as clear when nobody stands on it AND
                -- no chaser can cut the rolling ball out mid-flight; a cuttable
                -- lane is floated over the interception point instead.
                local f = ai.lane_blocker(keeper.pos, p.pos, opp, POSSESS_DIST)
                    or pass_risk(keeper.pos, p.pos, threats)
                -- Prefer a clear ground lane, then a short safe range, then openness
                -- and a little upfield progress. The clear-lane bonus dominates so a
                -- reliable ground pass always beats a risky lob.
                local score = (f and 0 or 1000)
                    - keeper.pos:dist(p.pos)
                    + opp_d * 0.5
                    + (p.pos.x - keeper.pos.x) * fwd * 0.2
                if not best_score or score > best_score then
                    best_score, best, best_f = score, i, f
                end
            end
            -- Tier-2 fallback candidate: the most-open teammate overall.
            if opp_d >= THROW_MIN_OPEN and (not open_best_d or opp_d > open_best_d) then
                open_best_d, open_best = opp_d, i
            end
        end
    end

    if not kicked then
        keeper.throw_timer = KEEPER_THROW_POSE -- release/throw pose (visual)
    end
    -- Hands place their lobs via plan_throw (safe-side landing + an arc no
    -- jump can reach); a kicked distribution keeps ordinary foot-lob flight.
    ---@param idx integer
    ---@param f number?
    local function release_throw(idx, f)
        if kicked then
            release_pass(s, keeper_idx, idx, f, LOB_CLEAR_H)
        elseif f then
            local land, pf, clear_h = plan_throw(s, keeper, idx)
            release_pass(s, keeper_idx, idx, pf, clear_h, land)
        else
            release_pass(s, keeper_idx, idx, nil, nil) -- clear ground lane: bowl it
        end
    end
    if best then
        release_throw(best, best_f)
    elseif open_best then
        -- Float it over the traffic to the least-marked teammate: the flight
        -- stays out of everyone's reach, so camped opponents can't pick it off.
        local f = ai.lane_blocker(keeper.pos, s.players[open_best].pos, opp, POSSESS_DIST) or 0.5
        release_throw(open_best, f)
    else
        -- Everyone swarmed: drop-kick a high clearance upfield (lands around
        -- DROPKICK_DIST away, toward the middle of the pitch).
        local tx = math.max(40, math.min(s.field.w - 40, keeper.pos.x + fwd * DROPKICK_DIST))
        local target = Vec2.new(tx, s.field.h / 2)
        s.events[#s.events + 1] = { kind = "shot", x = s.ball.x, y = s.ball.y, player = keeper.id }
        s.owner = nil
        s.ball_z = 0
        s.ball_spin = 0
        s.pickup_cd = RELEASE_CD
        s.block_grace = BLOCK_GRACE
        s.ball_vel, s.ball_vz = lob_launch(keeper.pos, target, 0.5, DROPKICK_CLEAR_H)
    end
end

-- HUMAN-CONTROLLED keeper distribution: you pick it. Space (hold + release)
-- is a charged PUNT off the foot — the longer the hold, the further upfield it
-- sails. K (hold + release) is a charged THROW: the range picks which teammate
-- along your aim receives it. The hold clock still runs as the six-second-rule
-- fallback so play can't stall. With the ball at the FEET (a received
-- back-pass) the throw becomes a normal outfield-style pass, and there is no
-- six-second fallback — feet are exempt, and you're in control.
---@param s MatchState
---@param dt number
---@param input MatchInput
---@param owner MatchPlayer
local function human_keeper_actions(s, dt, input, owner)
    -- A full meter releases on its own (predictable); early release fires at
    -- the current charge.
    local fire_shot, fire_pass = input.shoot, input.pass
    if input.shoot_held then
        owner.charge = math.min(1, owner.charge + TUNE.CHARGE_RATE * dt)
        owner.pass_target = nil
        fire_shot = fire_shot or owner.charge >= 1
    elseif input.pass_held then
        owner.pass_charge = math.min(1, owner.pass_charge + TUNE.PASS_CHARGE_RATE * dt)
        fire_pass = fire_pass or owner.pass_charge >= 1
    end
    if fire_shot and owner.windup_timer == 0 then
        local dist = PUNT_MIN + owner.charge * (TUNE.PUNT_MAX - PUNT_MIN)
        local dir = (input.move.x ~= 0 or input.move.y ~= 0) and input.move:normalized()
            or owner.facing
        if dir.x == 0 and dir.y == 0 then
            dir = Vec2.new((owner.team == "home") and 1 or -1, 0)
        end
        local tgt = owner.pos:add(dir:scale(dist))
        tgt = Vec2.new(
            math.max(40, math.min(s.field.w - 40, tgt.x)),
            math.max(40, math.min(s.field.h - 40, tgt.y))
        )
        -- Parameters captured at commit; ball releases after the wind-up.
        local vel, vz = lob_launch(owner.pos, tgt, 0.5, PUNT_CLEAR_H)
        owner.charge = 0
        owner.pass_target = nil
        owner.windup_timer = TUNE.SHOT_WINDUP
        owner.windup_shot = {
            dir = vel:normalized(),
            speed = vel:length(),
            vz = vz,
            spin = 0,
            shot_type = "ground",
        }
    elseif fire_pass then
        local aim = (input.move.x ~= 0 or input.move.y ~= 0) and input.move:normalized()
            or owner.facing
        if owner.feet_ball then
            try_pass(s, s.owner, input.lob, aim)
        else
            local range = PASS_RANGE_MIN
                + owner.pass_charge * (TUNE.PASS_RANGE_MAX - PASS_RANGE_MIN)
            keeper_throw(s, s.owner, range, aim)
        end
        owner.pass_charge = 0
        owner.pass_target = nil
    end
    if s.owner and not owner.feet_ball and owner.hold_timer <= 0 and owner.windup_timer == 0 then
        keeper_distribute(s, s.owner)
    end
end

-- Fire the actual dive lunge: aim at the freshest prediction of where the
-- shot crosses the keeper's line. The movement clamps to dive_target (see
-- move_players), so a near-straight shot is a small step, not a full-speed
-- 100px lunge past the ball.
---@param s MatchState
---@param keeper MatchPlayer
local function launch_dive(s, keeper)
    local y_cross = s.ball.y
    if s.ball_vel.x ~= 0 then
        local t = (keeper.pos.x - s.ball.x) / s.ball_vel.x
        if t > 0 then
            y_cross = s.ball.y + s.ball_vel.y * t
        end
    end
    keeper.dive_timer = KEEPER_DIVE_DURATION
    keeper.dive_target = Vec2.new(keeper.pos.x, y_cross)
    local to_cross = keeper.dive_target:sub(keeper.pos)
    keeper.dive_dir = (to_cross:length() > 1) and to_cross:normalized() or Vec2.new(0, 0)
end

-- The keeper of the threatened goal COMMITS against an on-target shot: it picks
-- its verdict now (catch / parry / beaten — pure and deterministic, a function
-- of reach, handling, pace and angle), but the ball is NOT touched: it keeps
-- flying its real trajectory and the save completes on contact in
-- resolve_pending_save. The dive itself is QUEUED (dive_delay) so the lunge
-- lands when the ball does — committing early must not mean diving early. So a
-- shot always visibly travels from the shooter's boot to the keeper's glove —
-- no teleports, no gloves closing on empty air.
---@param s MatchState
local function attempt_save(s)
    local speed = s.ball_vel:length()
    if speed < 1 or s.ball_vel.x == 0 then
        return -- a dead or purely-vertical ball is not an on-target shot
    end
    for _, keeper in ipairs(s.players) do
        if
            keeper.is_keeper
            and keeper.receive_timer <= 0 -- a teammate's back-pass is RECEIVED (feet), never saved
            and keeper.dive_timer <= 0
            and keeper.dive_delay <= 0
            and not keeper.save_pending
        then
            local goal = (keeper.team == "home") and s.goal_home or s.goal_away
            local toward = (keeper.team == "home") and (s.ball_vel.x < 0) or (s.ball_vel.x > 0)
            -- Time for the ball to reach the keeper's line. Must be ahead of the ball
            -- (keeper between ball and goal) and close enough that the keeper commits.
            local t = (keeper.pos.x - s.ball.x) / s.ball_vel.x
            if toward and t >= 0 and math.abs(keeper.pos.x - s.ball.x) <= SAVE_ZONE then
                local y_cross = s.ball.y + s.ball_vel.y * t -- where it crosses the keeper's line
                local plane_x = (keeper.team == "home") and (goal.x + goal.w) or goal.x
                local tg = (plane_x - s.ball.x) / s.ball_vel.x
                local y_goal = s.ball.y + s.ball_vel.y * tg -- where it crosses the goal plane
                -- Height of the shot when it reaches the keeper's line. A ball over
                -- the keeper's aerial reach (a chip) sails past it; over the bar isn't
                -- When the ball ACTUALLY arrives, friction included. dist/speed
                -- lies for slow shots (they decelerate the whole way), and a
                -- dying ball never arrives at all — that one is claimed off the
                -- grass by the normal keeper logic, never dived at.
                local dxa = math.abs(keeper.pos.x - s.ball.x)
                local x_frac = speed / math.abs(s.ball_vel.x) -- path px per x px
                local k_fric = (s.ball_z > 0.5 or s.ball_vz > 0) and AIR_FRICTION or FRICTION
                local eta = require("sim.keeper").travel_time(dxa * x_frac, speed, k_fric)
                -- The dive is timed to GLOVE CONTACT (hands' radius short of
                -- the line), not the line itself — so the lunge window covers
                -- the moment the save actually resolves.
                local eta_contact = require("sim.keeper").travel_time(
                    math.max(0, dxa - KEEPER_HANDS) * x_frac,
                    speed,
                    k_fric
                )
                -- Height when it reaches the keeper's line, at the real arrival
                -- time (the geometric t is fine for y — friction shrinks both
                -- velocity components equally, so the path stays straight —
                -- but gravity runs on the clock).
                local tz = eta or t
                local z_cross = s.ball_z + s.ball_vz * tz - 0.5 * GRAVITY * tz * tz
                local on_target = y_goal >= goal.y - SAVE_PAD
                    and y_goal <= goal.y + goal.h + SAVE_PAD
                    and z_cross < CROSSBAR
                    and z_cross <= KEEPER_AIR_GRAB
                -- How far the keeper has to dive along its line to reach the shot.
                local dive_dist = math.abs(keeper.pos.y - y_cross)
                local block_reach = species.block_reach(keeper.owned_verb)
                -- Physical save eligibility already includes the species block-reach
                -- seam. Use that same effective reach for style and tip geometry;
                -- catch/parry quality deliberately keeps its existing base-reach math.
                local effective_reach = keeper.reach + block_reach
                local reaction_reach = require("sim.keeper").reaction_reach(
                    effective_reach,
                    keeper.keeper_release_motion,
                    KEEPER_DIVE_DURATION
                )
                if on_target and eta then
                    -- Any pre-release set has now reached the real save-processing
                    -- seam. The existing verdict and contact-timed dive own the
                    -- presentation from here.
                    keeper.keeper_set = 0
                    local dist_to_keeper = keeper.pos:dist(s.ball)
                    local save_style
                    if dist_to_keeper > KEEPER_SMOTHER then
                        save_style = require("sim.keeper").save_style(
                            dist_to_keeper,
                            dive_dist,
                            effective_reach
                        )
                    end

                    if dive_dist > reaction_reach then
                        if
                            dive_dist > effective_reach
                            and dive_dist <= effective_reach * 1.1
                            and not keeper.save_tip_emitted
                        then
                            keeper.save_tip_emitted = true
                            s.events[#s.events + 1] = {
                                kind = "tip",
                                x = keeper.pos.x,
                                y = y_cross,
                                player = keeper.id,
                                keeper_state = keeper.keeper_release_state,
                                keeper_depth = keeper.keeper_release_depth,
                            }
                        end
                        return
                    end

                    keeper.save_style = save_style
                    -- Queue the dive so the lunge window covers the arrival:
                    -- a shot still half a second out gets a set keeper first,
                    -- then a dive that meets the ball, not one that finished
                    -- while the shot was still traveling.
                    keeper.dive_delay = math.max(0, (eta_contact or eta) - KEEPER_DIVE_DURATION)
                    if keeper.dive_delay == 0 then
                        launch_dive(s, keeper)
                    end

                    -- Closeness of the dive + handling, minus pace. A shot straight at
                    -- the keeper (dive_dist ~ 0) is gathered even when hard; only wide
                    -- or blistering shots drop to a parry or beat the keeper.
                    local quality = (1 - dive_dist / keeper.reach)
                        + keeper.handling * HANDLING_WEIGHT
                        - speed / TUNE.SAVE_SPEED_REF

                    if quality >= PARRY_QUALITY then
                        -- Grab or parry: probabilistic, from the match's seeded
                        -- RNG. The catch odds are a logistic curve over quality
                        -- — soft and central sticks in the gloves, hot or at
                        -- full stretch usually gets pushed away.
                        local p_catch = 1
                            / (1 + math.exp(-(quality - TUNE.CATCH_EVEN_QUALITY) / CATCH_SOFTNESS))
                        local sample
                        s.rng, sample = rng.roll(s.rng)
                        keeper.save_pending = (sample < p_catch) and "catch" or "parry"
                        keeper.save_timer = eta + SAVE_TIMEOUT_PAD
                        keeper.save_vx = s.ball_vel.x
                    end
                    -- Beaten: the dive is committed but the ball flies through.
                    return
                end
            end
        end
    end
end

-- Complete a committed save when the ball actually arrives: at hands' reach, on
-- crossing the keeper's plane (a fast far-corner ball met right at the line),
-- or on the timeout backstop. The dive is abandoned (a whiff) if the shot got
-- deflected away in flight — a body block or bounce reversing its direction.
---@param s MatchState
---@param dt number
---@return "catch"|"parry"|nil
local function resolve_pending_save(s, dt)
    for ki, keeper in ipairs(s.players) do
        local pend = keeper.save_pending
        if pend then
            keeper.save_timer = keeper.save_timer - dt
            local reversed = s.ball_vel.x * keeper.save_vx <= 0
            local crossed = (keeper.save_vx > 0 and s.ball.x >= keeper.pos.x)
                or (keeper.save_vx < 0 and s.ball.x <= keeper.pos.x)
            local contact = keeper.pos:dist(s.ball) <= KEEPER_HANDS
            if reversed then
                keeper.save_pending = nil -- the shot was deflected: the dive whiffs
                keeper.dive_delay = 0 -- and a still-queued lunge stays holstered
                keeper.save_style = nil
                keeper.save_tip_emitted = false
            elseif s.ball_vel:length() < DEAD_SHOT_SPEED and not contact then
                -- The shot died short of the gloves: it is a loose ball now.
                -- Drop the commitment so the normal claim logic gathers it —
                -- never vacuum a stationary ball across open grass.
                keeper.save_pending = nil
                keeper.dive_delay = 0
                keeper.save_style = nil
                keeper.save_tip_emitted = false
            elseif contact or crossed or keeper.save_timer <= 0 then
                keeper.save_pending = nil
                keeper.dive_delay = 0
                local save_style = keeper.save_style
                keeper.save_style = nil
                keeper.save_tip_emitted = false
                if pend == "catch" then
                    s.events[#s.events + 1] = {
                        kind = "catch",
                        x = s.ball.x,
                        y = s.ball.y,
                        player = keeper.id,
                        save_style = save_style,
                        keeper_state = keeper.keeper_release_state,
                        keeper_depth = keeper.keeper_release_depth,
                    }
                    s.ball = keeper_hold_pos(s, keeper)
                    s.owner = ki
                    s.ball_vel = Vec2.new(0, 0)
                    s.ball_z = 0
                    s.ball_vz = 0
                    s.ball_spin = 0
                    keeper.grab_timer = KEEPER_GRAB_POSE
                    keeper.hold_timer = KEEPER_HOLD
                    keeper.feet_ball = false
                    return "catch"
                end
                -- Parry from the actual contact point: punch it clear — out AND
                -- up, so the deflection sails over the shooter, never served
                -- into their body. Keep the ball safely outside the goal line.
                s.events[#s.events + 1] = {
                    kind = "parry",
                    x = s.ball.x,
                    y = s.ball.y,
                    player = keeper.id,
                    save_style = save_style,
                    keeper_state = keeper.keeper_release_state,
                    keeper_depth = keeper.keeper_release_depth,
                }
                local goal = (keeper.team == "home") and s.goal_home or s.goal_away
                local bx = s.ball.x
                if keeper.team == "home" then
                    bx = math.max(bx, goal.x + goal.w + BALL_RADIUS + 1)
                else
                    bx = math.min(bx, goal.x - BALL_RADIUS - 1)
                end
                s.ball = Vec2.new(bx, s.ball.y)
                local gc = Vec2.new(goal.x + goal.w / 2, goal.y + goal.h / 2)
                local dir = s.ball:sub(gc):normalized()
                if dir.x == 0 and dir.y == 0 then
                    dir = keeper.facing
                end
                local speed = s.ball_vel:length()
                s.ball_vel = dir:scale(math.max(MIN_PARRY_CLEAR, speed * PARRY_SPEED_MULT))
                s.ball_vz = PARRY_POP_VZ
                s.ball_spin = 0
                s.pickup_cd = PARRY_CD
                s.block_grace = BLOCK_GRACE
                return "parry"
            end
        end
    end
    return nil
end

-- Recompute pass_target for a human outfield carrier holding the pass button.
-- Pure: no RNG draws. Safe to call every frame while pass_held is true.
---@param s MatchState
---@param owner_idx integer
---@param input MatchInput
local function update_pass_target_outfield(s, owner_idx, input)
    local owner = s.players[owner_idx]
    local range = (owner.pass_charge > 0.12)
            and (PASS_RANGE_MIN + owner.pass_charge * (TUNE.PASS_RANGE_MAX - PASS_RANGE_MIN))
        or nil
    local aim = (input.move.x ~= 0 or input.move.y ~= 0) and input.move:normalized() or nil
    owner.pass_target = select_pass_target(s, owner_idx, input.lob, aim, range)
end

-- Recompute pass_target for a human keeper holding the pass button.
-- Pure: no RNG draws. Safe to call every frame while pass_held is true.
---@param s MatchState
---@param keeper_idx integer
---@param input MatchInput
local function update_pass_target_keeper(s, keeper_idx, input)
    local keeper = s.players[keeper_idx]
    local range = PASS_RANGE_MIN + keeper.pass_charge * (TUNE.PASS_RANGE_MAX - PASS_RANGE_MIN)
    local aim = (input.move.x ~= 0 or input.move.y ~= 0) and input.move:normalized() or nil
    keeper.pass_target = select_throw_target(s, keeper_idx, range, aim)
end

-- Human outfield controlled shot commit: build charge and, on fire, store a
-- wind-up payload (parameters captured now, released after TUNE.SHOT_WINDUP seconds).
-- Extracted from update_ball to keep it under LuaJIT's 60-upvalue cap.
---@param s MatchState
---@param dt number
---@param input MatchInput
---@param owner MatchPlayer
local function human_outfield_actions(s, dt, input, owner)
    local fire_shot, fire_pass = input.shoot, input.pass
    if input.shoot_held then
        owner.charge = math.min(1, owner.charge + TUNE.CHARGE_RATE * dt)
        fire_shot = fire_shot or owner.charge >= 1
        owner.pass_target = nil
    elseif input.pass_held then
        owner.pass_charge = math.min(1, owner.pass_charge + TUNE.PASS_CHARGE_RATE * dt)
        fire_pass = fire_pass or owner.pass_charge >= 1
        -- Preview: recompute intended receiver every frame (pure, no RNG).
        update_pass_target_outfield(s, s.owner, input)
    else
        owner.pass_target = nil
    end
    if fire_shot then
        -- Aim at the goal; vertical of `facing` picks the corner. Charge
        -- (held shoot) scales power; lateral input bends the shot.
        -- Parameters are CAPTURED NOW and released after the wind-up.
        local vbias = math.max(-1, math.min(1, owner.facing.y * 1.4))
        local speed = owner.shot_speed * (1 + owner.charge * CHARGE_POWER)
        local target = shot_target(s, owner, vbias)
        local vz = 0
        local shot_type = input.lob and "chip" or "ground"
        if input.lob then
            -- Human chip intent is locked now, like direction and power. A
            -- feasible chip clears the keeper's current committed plane; an
            -- infeasible request remains a deterministic poor chip rather
            -- than silently becoming a ground-shot decoy during the wind-up.
            local defending_team = owner.team == "home" and "away" or "home"
            local defending_keeper = assert(team_keeper(s, defending_team))
            local goal = defending_team == "home" and s.goal_home or s.goal_away
            vz = keeper.committed_chip_launch({
                origin = owner.pos,
                target = target,
                keeper_pos = defending_keeper.pos,
                defending_team = defending_team,
                goal = goal,
                horizontal_speed = speed,
                friction = AIR_FRICTION,
                gravity = GRAVITY,
                keeper_clearance = KEEPER_AIR_GRAB,
                crossbar = CROSSBAR,
                desired_goal_height = CHIP_LINE_Z,
            })
        end
        local side = (input.move.x > 0 and 1) or (input.move.x < 0 and -1) or 0
        local spin = shot_type == "ground" and side * owner.charge * CURVE_MAX or 0
        owner.charge = 0
        owner.pass_target = nil
        owner.windup_timer = TUNE.SHOT_WINDUP
        owner.windup_shot = {
            dir = target:sub(owner.pos),
            speed = speed,
            vz = vz,
            spin = spin,
            shot_type = shot_type,
        }
    elseif fire_pass then
        local aim = (input.move.x ~= 0 or input.move.y ~= 0) and input.move:normalized() or nil
        try_pass(s, s.owner, input.lob, aim)
        owner.pass_charge = 0
        owner.pass_target = nil
    elseif not (input.shoot_held or input.pass_held) then
        owner.charge = 0
    end
end

-- AI outfield owner decision: shoot (with wind-up), cross, or pass out of pressure.
-- Extracted to keep update_ball under the LuaJIT 60-upvalue cap.
---@param s MatchState
---@param owner_idx integer
---@param owner MatchPlayer
local function ai_outfield_decision(s, owner_idx, owner)
    local g = attack_goal(s, owner.team)
    local gc = Vec2.new((owner.team == "home") and g.x or (g.x + g.w), g.y + g.h / 2)
    if owner.pos:dist(gc) < TUNE.AI_SHOOT_RANGE then
        -- Shoot to the corner away from the defending keeper, with power
        -- scaled by the space the striker has been given (see constants).
        local keeper_player = team_keeper(s, owner.team == "home" and "away" or "home")
        local vbias = 0.85
        if keeper_player then
            vbias = (keeper_player.pos.y < gc.y) and 0.85 or -0.85
        end
        local space = math.huge
        for _, q in ipairs(s.players) do
            if q.team ~= owner.team and not q.is_keeper then
                space = math.min(space, q.pos:dist(owner.pos))
            end
        end
        -- Closed down with no power available: look for the square ball
        -- to a better-placed teammate first; only shoot if nobody's on.
        local passed = false
        if space < AI_CHARGE_MIN_SPACE and owner.settle_timer <= 0 then
            passed = ai_try_pass(s, owner_idx)
        end
        if not passed then
            local frac =
                math.max(0, math.min(1, (space - AI_CHARGE_MIN_SPACE) / AI_CHARGE_SPACE_RANGE))
            local speed = owner.shot_speed * (1 + frac * CHARGE_POWER)
            local shot_end = shot_target(s, owner, vbias)
            local sdir = shot_end:sub(owner.pos)
            local vz = 0
            if
                keeper_player
                and owner.settle_timer <= 0
                and owner.pos:dist(s.ball) <= DRIBBLE_TOUCH_REACH
                and keeper.chip_is_visible(keeper_player.pos, keeper_player.team, g)
            then
                vz = keeper.chip_launch({
                    origin = owner.pos,
                    target = shot_end,
                    keeper_pos = keeper_player.pos,
                    defending_team = keeper_player.team,
                    goal = g,
                    horizontal_speed = speed,
                    friction = AIR_FRICTION,
                    gravity = GRAVITY,
                    keeper_clearance = KEEPER_AIR_GRAB,
                    crossbar = CROSSBAR,
                    desired_goal_height = CHIP_LINE_Z,
                }) or 0
            end
            owner.windup_timer = TUNE.SHOT_WINDUP
            owner.windup_shot = {
                dir = sdir,
                speed = speed,
                vz = vz,
                spin = 0,
                shot_type = vz > 0 and "chip" or "ground",
            }
        end
    else
        -- From wide in the attacking third, swing a CROSS to a teammate
        -- in the box (who can meet it with a header).
        local crossed = false
        local third = (owner.team == "home") and (owner.pos.x > s.field.w * 0.62)
            or (owner.team == "away" and owner.pos.x < s.field.w * 0.38)
        local wide = math.abs(owner.pos.y - s.field.h / 2) > 130
        if third and wide and owner.settle_timer <= 0 then
            -- Only with a step of space: a pressured winger shouldn't spam
            -- hopeful crosses.
            local space = math.huge
            for _, q in ipairs(s.players) do
                if q.team ~= owner.team and not q.is_keeper then
                    space = math.min(space, q.pos:dist(owner.pos))
                end
            end
            if space > TUNE.CROSS_MIN_SPACE then
                crossed = ai_try_cross(s, owner_idx)
            end
        end
        if not crossed then
            -- Out of range: pass out of pressure rather than dribble
            -- into a challenge. If nobody is open, keep carrying.
            local pressure = math.huge
            for _, q in ipairs(s.players) do
                if q.team ~= owner.team and not q.is_keeper then
                    pressure = math.min(pressure, q.pos:dist(owner.pos))
                end
            end
            if pressure <= TUNE.AI_PASS_PRESSURE and owner.settle_timer <= 0 then
                ai_try_pass(s, owner_idx)
            end
        end
    end
end

---@param s MatchState
---@param dt number
---@param inputs table<integer, MatchInput>
---@param combat_state CombatMatchState?
local function update_ball(s, dt, inputs, combat_state)
    local combat_module = combat_state and require("sim.combat") or nil
    -- Controller transients belong to the input owner. Losing possession
    -- cancels that player's charge, preview, and any committed wind-up; a
    -- later possession can never inherit stale state from another slot.
    for index, player in ipairs(s.players) do
        if index ~= s.owner or not is_human_player(s, index) then
            player.charge = 0
            player.pass_charge = 0
            player.pass_target = nil
        end
        if s.slot_mode and index ~= s.owner and player.windup_shot then
            player.windup_timer = 0
            player.windup_shot = nil
        end
    end

    -- Wind-up resolution: a player whose timer just hit 0 and still owns the ball
    -- fires the stored shot payload. If they lost possession during the wind-up
    -- (tackle, smother) the payload was already cleared in attempt_steals.
    if s.owner then
        local wowner = s.players[s.owner]
        if wowner.windup_timer == 0 and wowner.windup_shot then
            local ws = assert(wowner.windup_shot)
            wowner.windup_shot = nil
            release_shot(s, wowner, ws.dir, ws.speed, ws.vz, ws.shot_type)
            s.ball_spin = ws.spin
            -- Keeper punt gets a throw pose; outfield shot doesn't (handled below).
            if wowner.is_keeper then
                wowner.throw_timer = KEEPER_THROW_POSE
            end
            return
        end
    end

    if s.owner then
        -- An owned ball invalidates any committed save still waiting on contact.
        for _, q in ipairs(s.players) do
            q.save_pending = nil
            q.save_style = nil
            q.save_tip_emitted = false
        end
        local owner = s.players[s.owner]
        local input = inputs[s.owner] or slot_input.neutral_match_input()
        if owner.is_keeper and not owner.feet_ball then
            -- A keeper holds the ball in its hands, clamped clear of its own line.
            s.ball = keeper_hold_pos(s, owner)
            s.ball_vel = Vec2.new(0, 0)
            s.ball_z = 0 -- an owned ball is grounded (at feet / in hands)
            s.ball_vz = 0
        else
            -- Touch-based dribble, DISCRETE (see the constants block): kick,
            -- chase, kick again. Between touches the ball runs free under
            -- grass friction; each new touch is a visible, audible kick ahead
            -- of the run with skill-scaled direction/weight error. Push one
            -- past the (skill-scaled) control radius and possession breaks —
            -- a heavy touch, robbed.
            local skill = owner.dribble
            -- REALIZED speed (actual motion), not run_vel: a carrier body-
            -- checked to a stop must not keep pushing the ball at the pace
            -- their legs are asking for — the ball rides what the body DOES.
            local speed = owner.vel:length()
            local at_feet = owner.pos:dist(s.ball) <= DRIBBLE_TOUCH_REACH
            s.ball_z = 0
            s.ball_vz = 0
            if not at_feet then
                -- The ball is away from the feet: it rolls free — the PLAYER
                -- goes to the BALL (the hook in move_players), never the
                -- other way around.
                s.ball_vel = s.ball_vel:scale(math.max(0, 1 - FRICTION * dt))
            elseif speed < owner.move_speed * TUNE.DRIBBLE_CLOSE then
                -- CLOSE CONTROL (standing through an ordinary jog): the ball
                -- stays glued to the feet with soft corrective touches —
                -- natural, safe, nothing knocked away. Sprinting breaks into
                -- the kick-and-chase below.
                local rest = owner.pos:add(owner.facing:scale(DRIBBLE_LEAD_MIN))
                local correct = rest:sub(s.ball):scale(TUNE.DRIBBLE_TOUCH * (0.5 + 0.5 * skill))
                s.ball_vel = owner.vel:add(correct)
            elseif s.ball_vel:length() <= speed + DRIBBLE_CATCH_PACE then
                -- The ball has slowed back to the feet: play the next touch —
                -- a kick ahead of the run, struck harder than the carrier
                -- moves so it runs on and returns. Sloppier feet (low skill)
                -- spray the angle and the weight; the seeded rolls keep the
                -- sim reproducible.
                local roll_a, roll_w
                s.rng, roll_a = rng.roll(s.rng)
                s.rng, roll_w = rng.roll(s.rng)
                local slop = 1 - DRIBBLE_ERR_SKILL * skill
                local ang = (roll_a * 2 - 1) * TUNE.DRIBBLE_ERR * slop
                local ca, sa = math.cos(ang), math.sin(ang)
                local dir = Vec2.new(
                    owner.facing.x * ca - owner.facing.y * sa,
                    owner.facing.x * sa + owner.facing.y * ca
                )
                local weight = 1 + (roll_w * 2 - 1) * TUNE.DRIBBLE_ERR * 0.8 * slop
                s.ball_vel = dir:scale(speed * TUNE.DRIBBLE_PUSH * weight)
                s.events[#s.events + 1] =
                    { kind = "touch", x = s.ball.x, y = s.ball.y, player = owner.id }
            else
                -- At the feet but still leaving the boot (just kicked): let it
                -- run, shedding pace on the grass.
                s.ball_vel = s.ball_vel:scale(math.max(0, 1 - FRICTION * dt))
            end
            s.ball = s.ball:add(s.ball_vel:scale(dt))
            local control = TUNE.DRIBBLE_CONTROL + DRIBBLE_CONTROL_SKILL * skill
            if owner.pos:dist(s.ball) > control then
                -- Clear at the ownership-loss transition so this tick is
                -- self-contained: reacquisition cannot revive stale input state.
                owner.charge = 0
                owner.pass_charge = 0
                owner.pass_target = nil
                -- The fixed-slot contract requires same-tick wind-up
                -- cancellation. Legacy match AI keeps its tripwire-pinned
                -- heavy-touch behavior at the explicit offline boundary.
                if s.slot_mode then
                    owner.windup_timer = 0
                    owner.windup_shot = nil
                end
                for _, player in ipairs(s.players) do
                    player.keeper_set = 0
                end
                s.owner = nil -- the touch got away from the feet: it's loose now
                return -- no owner actions this frame; the ball plays loose next
            end
        end

        if owner.is_keeper then
            if is_human_player(s, s.owner) then
                -- Preview: while pass_held, show which teammate would receive.
                -- Ball at the feet passes like an outfielder; in the hands it throws.
                if input.pass_held then
                    if owner.feet_ball then
                        update_pass_target_outfield(s, s.owner, input)
                    else
                        update_pass_target_keeper(s, s.owner, input)
                    end
                else
                    owner.pass_target = nil
                end
                human_keeper_actions(s, dt, input, owner)
            else
                owner.pass_target = nil
                if owner.hold_timer <= 0 then
                    -- AI keeper: survey, then distribute to a safe outlet (build
                    -- from the back) instead of hoofing it upfield every frame.
                    keeper_distribute(s, s.owner)
                end
            end
        elseif is_human_player(s, s.owner) then
            -- A full meter LETS FLY on its own (predictable, like the meter
            -- promises); release fires early at the current charge.
            -- During wind-up: inputs are locked out (shot is committed, params
            -- already captured), so skip all action logic this frame.
            if
                owner.windup_timer == 0
                and (not combat_module or not combat_module.blocks_actions(combat_state, s.owner))
            then
                human_outfield_actions(s, dt, input, owner)
            end
        elseif
            owner.windup_timer == 0
            and (not combat_module or not combat_module.blocks_actions(combat_state, s.owner))
        then
            -- AI owner: decide what to do (shoot/cross/pass/carry).
            -- Guarded: while winding up, the shot is already committed; no re-decisions.
            ai_outfield_decision(s, s.owner, owner)
        end
        return
    end

    -- Loose ball: integrate, decay, curve, bounce off touchlines/back walls.
    s.ball = s.ball:add(s.ball_vel:scale(dt))
    -- Full grass friction only near the ground; a lofted ball glides (light drag).
    local airborne = s.ball_z > GROUND_GRAB_HEIGHT
    local hfric = airborne and AIR_FRICTION or FRICTION
    s.ball_vel = s.ball_vel:scale(math.max(0, 1 - hfric * dt))
    local trajectory_bounced = false
    -- Spin only bends a ball rolling on the grass.
    if not airborne and s.ball_spin ~= 0 and s.ball_vel:length() > 1 then
        local v = s.ball_vel
        local perp = Vec2.new(-v.y, v.x):normalized()
        s.ball_vel = s.ball_vel:add(perp:scale(s.ball_spin * dt))
        s.ball_spin = s.ball_spin * math.max(0, 1 - SPIN_DECAY * dt)
    end

    -- Vertical: integrate under gravity, land and rebound (keeping horizontal pace).
    s.ball_z = s.ball_z + s.ball_vz * dt
    s.ball_vz = s.ball_vz - GRAVITY * dt
    -- Cage ceiling: a skied ball bounces back down into the arena.
    if s.ball_z >= CAGE_CEILING then
        s.ball_z = CAGE_CEILING
        if s.ball_vz > 0 then
            s.ball_vz = -s.ball_vz * CEIL_BOUNCE
            trajectory_bounced = true
        end
    end
    if s.ball_z <= 0 then
        s.ball_z = 0
        if s.ball_vz < 0 then
            if -s.ball_vz <= LAND_SETTLE_VZ then
                s.ball_vz = 0 -- settle: stop micro-bouncing
            else
                s.ball_vz = -s.ball_vz * BOUNCE -- rebound up; ball_vel unchanged
            end
        end
    end

    if s.ball.y < BALL_RADIUS then
        s.ball.y = BALL_RADIUS
        s.ball_vel.y = -s.ball_vel.y
        trajectory_bounced = true
    elseif s.ball.y > s.field.h - BALL_RADIUS then
        s.ball.y = s.field.h - BALL_RADIUS
        s.ball_vel.y = -s.ball_vel.y
        trajectory_bounced = true
    end
    -- X walls: through a goal mouth the ball plays on into the net box behind
    -- the line; anywhere else it bounces back in. Inside a net box the side
    -- netting clamps y, the back net kills most pace, and a gentle "slope"
    -- rolls a dead ball back out toward the line so it can't strand there.
    if s.ball.x < BALL_RADIUS then
        if in_mouth(s.ball, s.goal_home) then
            local g = s.goal_home
            if s.ball.x < g.x + BALL_RADIUS then
                s.ball.x = g.x + BALL_RADIUS
                s.ball_vel = Vec2.new(-s.ball_vel.x * NET_DAMP, s.ball_vel.y * NET_DAMP)
            end
            if s.ball.y < g.y + BALL_RADIUS then
                s.ball.y = g.y + BALL_RADIUS
                s.ball_vel.y = -s.ball_vel.y * NET_DAMP
            elseif s.ball.y > g.y + g.h - BALL_RADIUS then
                s.ball.y = g.y + g.h - BALL_RADIUS
                s.ball_vel.y = -s.ball_vel.y * NET_DAMP
            end
            s.ball_vel.x = s.ball_vel.x + NET_ROLLOUT * dt
        else
            s.ball.x = BALL_RADIUS
            s.ball_vel.x = -s.ball_vel.x
            trajectory_bounced = true
        end
    elseif s.ball.x > s.field.w - BALL_RADIUS then
        if in_mouth(s.ball, s.goal_away) then
            local g = s.goal_away
            if s.ball.x > g.x + g.w - BALL_RADIUS then
                s.ball.x = g.x + g.w - BALL_RADIUS
                s.ball_vel = Vec2.new(-s.ball_vel.x * NET_DAMP, s.ball_vel.y * NET_DAMP)
            end
            if s.ball.y < g.y + BALL_RADIUS then
                s.ball.y = g.y + BALL_RADIUS
                s.ball_vel.y = -s.ball_vel.y * NET_DAMP
            elseif s.ball.y > g.y + g.h - BALL_RADIUS then
                s.ball.y = g.y + g.h - BALL_RADIUS
                s.ball_vel.y = -s.ball_vel.y * NET_DAMP
            end
            s.ball_vel.x = s.ball_vel.x - NET_ROLLOUT * dt
        else
            s.ball.x = s.field.w - BALL_RADIUS
            s.ball_vel.x = -s.ball_vel.x
            trajectory_bounced = true
        end
    end
    if trajectory_bounced then
        for _, player in ipairs(s.players) do
            player.keeper_set = 0
        end
    end

    -- Body blocking: a fast, low ball that runs into an outfield body ricochets
    -- off it. Only a ball moving TOWARD the body blocks, so a shooter never
    -- blocks their own release. Keepers are excluded — they interact with the
    -- ball through saves and claims, never as a passive wall.
    do
        local speed = s.ball_vel:length()
        local block_h = (s.ball_vz < 0) and BLOCK_HEIGHT_DESC or BLOCK_HEIGHT
        if speed >= POSSESS_MAX_SPEED and s.ball_z <= block_h and s.block_grace == 0 then
            for _, p in ipairs(s.players) do
                -- The designated receiver never walls a ball off: they let it
                -- arrive and take the touch.
                if not p.is_keeper and p.receive_timer <= 0 then
                    local off = s.ball:sub(p.pos)
                    local d = off:length()
                    local contact = p.radius + BALL_RADIUS + species.block_reach(p.owned_verb)
                    if d < contact then
                        local n = (d > 0) and off:normalized() or Vec2.new(1, 0)
                        local vn = s.ball_vel.x * n.x + s.ball_vel.y * n.y
                        if vn < 0 then
                            s.events[#s.events + 1] =
                                { kind = "block", x = s.ball.x, y = s.ball.y, player = p.id }
                            -- Reflect off the body normal, damped, and push the
                            -- ball clear so it can't re-block next frame.
                            s.ball_vel = s.ball_vel:sub(n:scale(2 * vn)):scale(BLOCK_DAMP)
                            s.ball = p.pos:add(n:scale(contact))
                            s.ball_spin = 0
                            -- The ricochet ends the pass: nobody is receiving this
                            -- ball any more (a keeper's save reflexes included).
                            for _, q in ipairs(s.players) do
                                q.receive_timer = 0
                                q.keeper_set = 0
                                q.save_style = nil
                                q.save_tip_emitted = false
                            end
                            break
                        end
                    end
                end
            end
        end
    end

    -- Keeper save: commit against an on-target shot, then complete the save when
    -- the ball actually arrives (real trajectory, no teleport). NOT gated on
    -- pickup_cd: that cooldown is the SHOOTER's re-collection lockout, and a
    -- close-range shot reaches the line well inside it — the keeper must still
    -- be allowed to react. A resolved catch ends the frame here; a parry sets
    -- pickup_cd so the deflection isn't re-grabbed instantly (the parried ball
    -- travels away from goal, so it can't trigger an immediate re-save); being
    -- beaten falls through to a possible goal.
    attempt_save(s)
    if resolve_pending_save(s, dt) == "catch" then
        return
    end

    -- Descending high-ball reception and first-time strikes. Geometry,
    -- difficulty, and seeded quality live in sim.aerial.
    local aerial_ineligible
    if combat_state then
        aerial_ineligible = {}
        for player_index, runtime in ipairs(combat_state.players) do
            aerial_ineligible[player_index] = runtime.forced_ticks > 0
        end
    end
    local aerial_redirected = aerial.resolve_play(s, inputs, {
        ground_grab_height = GROUND_GRAB_HEIGHT,
        stick_ahead = STICK_AHEAD,
        gravity = GRAVITY,
        release_cd = RELEASE_CD,
        clear_header_speed = CLEAR_HEADER_SPEED,
        volley_speed = VOLLEY_SPEED,
    }, aerial_ineligible)
    if aerial_redirected then
        -- A successful first touch or strike owns a new presentation trajectory.
        -- Clear style and the one-shot guard even when x direction is unchanged,
        -- but preserve the pre-existing verdict/dive timing: #44 must not change
        -- whether the keeper catches, parries, or is beaten.
        for _, player in ipairs(s.players) do
            player.keeper_set = 0
            player.save_style = nil
            player.save_tip_emitted = false
        end
    end

    -- Collection. A keeper has PRIORITY in its own box: it claims any loose ball it
    -- can reach there (with its hands), beating outfielders even if they are a touch
    -- closer. Otherwise the nearest eligible player grabs it.
    if s.pickup_cd == 0 then
        local speed = s.ball_vel:length()
        local best, best_dist

        for i, p in ipairs(s.players) do
            if
                p.is_keeper
                and (not combat_state or assert(combat_state.players[i]).forced_ticks == 0)
                and not p.save_pending -- a committed save resolves on contact instead
                and p.receive_timer <= 0 -- a teammate's pass is taken with the FEET below
                and in_claim_zone(s, p)
                and s.ball_z <= KEEPER_AIR_GRAB
                and p.pos:dist(s.ball) <= KEEPER_CLAIM_DIST + species.jump_reach(p.owned_verb)
            then
                best = i
                break
            end
        end

        if not best then
            for i, p in ipairs(s.players) do
                -- A keeper meeting a teammate's pass traps it with an outfield
                -- reach; hand grabs use the tighter keeper radius.
                local reach = (p.is_keeper and p.receive_timer <= 0) and KEEPER_DIST or POSSESS_DIST
                -- A ball above head height flies over everyone — not collectable.
                -- The DESIGNATED receiver traps a driven pass at full pace (the
                -- first touch is theirs); everyone else needs it slowed down.
                local eligible = (p.is_keeper or p.receive_timer > 0 or speed < POSSESS_MAX_SPEED)
                    and s.ball_z <= GROUND_GRAB_HEIGHT
                    and (not combat_state or assert(combat_state.players[i]).forced_ticks == 0)
                local d = p.pos:dist(s.ball)
                if eligible and d <= reach and (not best_dist or d < best_dist) then
                    best_dist = d
                    best = i
                end
            end
        end

        if best then
            local bp = s.players[best]
            for _, player in ipairs(s.players) do
                player.keeper_set = 0
                player.save_style = nil
                player.save_tip_emitted = false
            end
            if bp.is_keeper and bp.receive_timer <= 0 then
                -- A keeper gather: claim event + gather pose, then it surveys/holds.
                -- Snap the ball into its hands so a claim right on the line can't
                -- still register as a goal this frame.
                s.events[#s.events + 1] =
                    { kind = "claim", x = s.ball.x, y = s.ball.y, player = bp.id }
                s.ball = keeper_hold_pos(s, bp)
                bp.grab_timer = KEEPER_GRAB_POSE
                bp.hold_timer = KEEPER_HOLD
                bp.feet_ball = false
            else
                -- A back-pass rule of sorts: a keeper receiving a teammate's
                -- deliberate pass takes it with the FEET — it can dribble, pass,
                -- or punt, and is tackleable like any carrier.
                bp.feet_ball = bp.is_keeper
                if speed > 1 then
                    -- A moving ball trapped by a player reads as a "touch";
                    -- an AI needs a beat of control before it can pass on.
                    s.events[#s.events + 1] =
                        { kind = "touch", x = s.ball.x, y = s.ball.y, player = bp.id }
                    bp.settle_timer = TUNE.CARRIER_SETTLE
                end
            end
            s.owner = best
            s.ball_vel = Vec2.new(0, 0)
            s.ball_spin = 0
            -- Auto-switch: the human takes over whichever home outfielder wins the
            -- ball (like FIFA / Mario Strikers). Keepers stay AI.
            if
                not s.slot_mode
                and s.human_controlled
                and bp.team == "home"
                and not bp.is_keeper
            then
                s.controlled = best
            end
        end
    end
end

-- A goal per the laws of the game: the WHOLE ball crosses the goal line,
-- between the posts, under the bar — judged on the frame it crosses (edge
-- triggered on prev_x). A ball that sailed over the bar and drops inside the
-- net box afterwards never counts; a ball nestling in the net can't re-count.
---@param s MatchState
---@param prev_x number  -- ball x at the top of this step
---@return "home"|"away"? scorer
local function check_goal(s, prev_x)
    local line_a = s.goal_away.x -- right goal line: home scores crossing it
    if s.ball.x - BALL_RADIUS > line_a and prev_x - BALL_RADIUS <= line_a then
        if in_mouth(s.ball, s.goal_away) and s.ball_z < CROSSBAR then
            s.score.home = s.score.home + 1
            return "home"
        end
    end
    local line_h = s.goal_home.x + s.goal_home.w -- left goal line: away scores
    if s.ball.x + BALL_RADIUS < line_h and prev_x + BALL_RADIUS >= line_h then
        if in_mouth(s.ball, s.goal_home) and s.ball_z < CROSSBAR then
            s.score.away = s.score.away + 1
            return "away"
        end
    end
    return nil
end

---@param s MatchState
---@param dt number
---@param input InputFrame|MatchInput
---@param combat_state CombatMatchState?
---@return MatchState
function match.step(s, dt, input, combat_state)
    if s.finished then
        for _, player in ipairs(s.players) do
            player.keeper_set = 0
        end
        return s
    end
    local combat_module = combat_state and require("sim.combat") or nil
    if combat_state then
        assert(dt == fixed_clock.TICK_SECONDS, "combat matches require the canonical fixed tick")
        if s.slot_mode then
            assert(combat_state.tick == s.input_tick, "combat boundary does not match input tick")
        end
    end

    local inputs ---@type table<integer, MatchInput>
    if s.slot_mode then
        -- Slot mode has no legacy-input fallback. A complete, tick-numbered
        -- effective InputFrame is the simulation boundary; producers must
        -- materialize bots or neutral rows before calling the simulation.
        assert(input_frame.validate(input), "slot-mode match requires an InputFrame")
        ---@cast input InputFrame
        assert(input.tick == s.input_tick, "input frame tick does not match match state")
        assert(dt == fixed_clock.TICK_SECONDS, "slot-mode matches require the canonical fixed tick")
        inputs = {}
        for index = 1, input_frame.SLOT_COUNT do
            local player_idx = assert(s.slot_players[index], "slot mapping is incomplete")
            inputs[player_idx] = slot_input.to_match_input(input.slots[index])
        end
        s.input_tick = s.input_tick + 1
    else
        ---@cast input MatchInput
        inputs = {}
    end

    -- Discrete events are per-frame: clear last frame's before producing this one's.
    for i = #s.events, 1, -1 do
        s.events[i] = nil
    end
    if combat_state then
        assert(combat_module).clear_events(combat_state)
    end

    s.time_left = s.time_left - dt
    if s.time_left <= 0 then
        s.time_left = 0
        s.finished = true
        if combat_state then
            assert(combat_module).advance_boundary(combat_state)
        end
        for _, player in ipairs(s.players) do
            player.keeper_set = 0
        end
        return s
    end

    if s.pickup_cd > 0 then
        s.pickup_cd = math.max(0, s.pickup_cd - dt)
    end
    if s.block_grace > 0 then
        s.block_grace = math.max(0, s.block_grace - dt)
    end
    if s.aerial_lock > 0 then
        s.aerial_lock = math.max(0, s.aerial_lock - dt)
    end
    if s.kickoff_hold > 0 then
        s.kickoff_hold = math.max(0, s.kickoff_hold - dt)
    end
    for _, p in ipairs(s.players) do
        if p.dash_cd > 0 then
            p.dash_cd = math.max(0, p.dash_cd - dt)
        end
        if p.dodge_cd > 0 then
            p.dodge_cd = math.max(0, p.dodge_cd - dt)
        end
        if p.dodge_timer > 0 then
            p.dodge_timer = math.max(0, p.dodge_timer - dt)
        end
        if p.dive_timer > 0 then
            p.dive_timer = math.max(0, p.dive_timer - dt)
            if p.dive_timer == 0 then
                p.dive_target = nil
                p.save_style = nil
                p.save_tip_emitted = false
            end
        end
        if p.dive_delay > 0 then
            p.dive_delay = math.max(0, p.dive_delay - dt)
            if p.dive_delay == 0 then
                -- The queued dive fires — unless the shot is no longer inbound
                -- (deflected away mid-flight): then the keeper stays home.
                local inbound = (p.team == "home") and (s.ball_vel.x < 0) or (s.ball_vel.x > 0)
                if inbound then
                    launch_dive(s, p)
                else
                    p.save_style = nil
                    p.save_tip_emitted = false
                end
            end
        end
        if p.hold_timer > 0 then
            p.hold_timer = math.max(0, p.hold_timer - dt)
        end
        if p.slide_timer > 0 then
            p.slide_timer = math.max(0, p.slide_timer - dt)
        end
        if p.tackle_timer > 0 then
            p.tackle_timer = math.max(0, p.tackle_timer - dt)
        end
        if p.tackle_cd > 0 then
            p.tackle_cd = math.max(0, p.tackle_cd - dt)
        end
        if p.stun_timer > 0 then
            p.stun_timer = math.max(0, p.stun_timer - dt)
        end
        if p.grab_timer > 0 then
            p.grab_timer = math.max(0, p.grab_timer - dt)
        end
        if p.throw_timer > 0 then
            p.throw_timer = math.max(0, p.throw_timer - dt)
        end
        if p.receive_timer > 0 then
            p.receive_timer = math.max(0, p.receive_timer - dt)
        end
        if p.settle_timer > 0 then
            p.settle_timer = math.max(0, p.settle_timer - dt)
        end
        if p.header_cd > 0 then
            p.header_cd = math.max(0, p.header_cd - dt)
        end
        if p.aerial_timer > 0 then
            p.aerial_timer = math.max(0, p.aerial_timer - dt)
            if p.aerial_timer == 0 then
                p.aerial_style = nil
                p.aerial_outcome = nil
                p.aerial_jump = 0
            end
        end
        if p.aerial_recovery > 0 then
            p.aerial_recovery = math.max(0, p.aerial_recovery - dt)
        end
        if p.windup_timer > 0 then
            p.windup_timer = math.max(0, p.windup_timer - dt)
        end
        if p.jockey_timer > 0 then
            p.jockey_timer = math.max(0, p.jockey_timer - dt)
        end
    end

    if
        not s.slot_mode
        and s.human_controlled
        and input.switch
        and (not combat_module or not combat_module.blocks_actions(combat_state, s.controlled))
    then
        s.controlled = next_home_outfield(s, s.controlled)
    end
    if not s.slot_mode then
        inputs[s.controlled] = input
    end
    if combat_state then
        local equipment_ineligible = {}
        for player_index, player_input in pairs(inputs) do
            equipment_ineligible[player_index] =
                match._aerial_active_for_input(s, player_index, player_input)
        end
        inputs = assert(combat_module).prepare_inputs(s, combat_state, inputs, equipment_ineligible)
    end

    local prev_ball_x = s.ball.x -- for edge-triggered goal-line crossing
    local prev_owner = s.owner
    local prev_owner_team = s.owner and s.players[s.owner].team or nil
    move_players(s, dt, inputs, combat_state)
    local combat_contacts = combat_state and assert(combat_module).collect_contacts(s, combat_state)
        or nil
    attempt_steals(s, combat_state)
    if combat_state and combat_contacts then
        assert(combat_module).resolve_contacts(s, combat_state, combat_contacts)
    end
    update_ball(s, dt, inputs, combat_state)
    if combat_state then
        assert(combat_module).sanitize_forced_players(s, combat_state)
        assert(combat_module).finish_tick(combat_state)
    end

    -- A gained ball resolves any in-flight pass: nobody is "running onto" it
    -- any more. In particular an INTERCEPTED back-pass ends the keeper's
    -- receive window, so its save reflexes come straight back online.
    if s.owner and s.owner ~= prev_owner then
        for _, p in ipairs(s.players) do
            p.receive_timer = 0
        end
    end

    -- Auto-switch on turnover: the moment the opponent wins the ball, hand
    -- control to the home outfielder best placed to defend (nearest the ball)
    -- — mirroring the existing auto-switch when a home player wins it.
    local owner_team = s.owner and s.players[s.owner].team or nil
    if
        not s.slot_mode
        and s.human_controlled
        and owner_team == "away"
        and prev_owner_team ~= "away"
    then
        s.controlled = best_defender(s)
    end

    -- Cross aid: when a lofted ball flies into the human's attacking third and
    -- the human isn't already on it, hand control to the attacker best placed to
    -- meet it — so a single strike (with the aerial magnet) finishes the cross.
    if
        not s.slot_mode
        and s.human_controlled
        and owner_team ~= "home"
        and s.ball_z > CROSS_AID_Z
        and s.ball.x > s.field.w * CROSS_AID_THIRD
    then
        local best, best_d
        for i, p in ipairs(s.players) do
            if p.team == "home" and not p.is_keeper then
                local d = p.pos:dist(s.ball)
                if not best_d or d < best_d then
                    best_d, best = d, i
                end
            end
        end
        if best and best_d <= CROSS_AID_RANGE then
            s.controlled = best
        end
    end

    -- Keeper control: the human takes over the HOME keeper while it holds the
    -- ball (to pick the distribution), and control returns to an outfielder
    -- the moment the keeper no longer has it.
    if not s.slot_mode and s.human_controlled then
        if s.owner and s.players[s.owner].team == "home" and s.players[s.owner].is_keeper then
            if s.owner ~= prev_owner then
                s.controlled = s.owner
                -- The six-second clock only runs on a ball held in the HANDS;
                -- a back-pass trapped at the feet plays on at your own pace.
                if not s.players[s.owner].feet_ball then
                    s.players[s.owner].hold_timer = TUNE.KEEPER_HOLD_HUMAN
                end
            end
        elseif s.players[s.controlled].is_keeper then
            s.controlled = best_defender(s)
        end
    end

    local scorer = check_goal(s, prev_ball_x)
    if scorer then
        if combat_state then
            assert(combat_module).reset_for_kickoff(combat_state)
        end
        for _, player in ipairs(s.players) do
            player.keeper_set = 0
            player.save_style = nil
            player.save_tip_emitted = false
        end
        if s.score.home >= s.max_goals or s.score.away >= s.max_goals then
            s.finished = true
        else
            -- The team that conceded restarts play.
            place_kickoff(s, scorer == "home" and "away" or "home")
        end
    end

    return s
end

-- Renderer data: the goal frame height (posts/crossbar) and the penalty area
-- (world units) — drawn from the same numbers the rules use.
match.CROSSBAR_H = CROSSBAR
match.PENALTY_BOX = { depth = PENALTY_DEPTH, h = PENALTY_H }

-- Test seam: expose the pure off-ball target computation for assertions.
match._offball_targets = offball_targets
match._resolve_collisions = resolve_collisions
-- Test seam: pure receiver selectors (used for pass preview and acceptance specs).
match._select_pass_target = select_pass_target
match._select_throw_target = select_throw_target

return match
