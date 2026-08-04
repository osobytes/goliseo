# Design: Aerial reception and acrobatic finishing

**Status:** implemented 2026-07-10

> This record names the keys that were bound when it was written. "Space" is the
> **ACTION** role and "L" the **MODIFIER** role; `docs/controls.md` and
> `game/input/bindings.lua` are authoritative for what they are bound to now.

## Why

Galactic Cup already gives the ball a vertical position and supports first-time headers and
volleys, but outfield collection is restricted to `GROUND_GRAB_HEIGHT`. A receiver who does not
shoot therefore watches a lob pass bounce until it reaches the floor. The current aerial strike
also checks only instantaneous height and horizontal distance: there is no explicit jump, no
reception quality, no contested-contact resolution, and the first eligible player in array order
wins the ball.

The new system should make a lofted ball a readable choice:

- Do nothing with the action button to cushion it with the chest or an extended leg.
- Use the action button to meet it first-time with a volley or header.
- Use the lob modifier with the action button to ask for an acrobatic finish, including a bicycle
  kick when the ball is overhead or behind the player.

Every choice uses the same contact model. Distance, ball pace, drop speed, required jump,
alignment, balance, pressure, and player skill determine whether contact is clean, heavy, or
missed.

## Design goals

1. A designated receiver can control a descending lob before it touches the floor.
2. Reception visibly has two touches where appropriate: chest or leg, then ball to foot.
3. Easy receptions are dependable; ambitious or pressured receptions remain risky.
4. Jumping headers, jumping volleys, and bicycle kicks are explicit outcomes of contact geometry.
5. The human chooses intent, not an animation. The sim chooses the physically valid action.
6. The resolver is pure and deterministic under the match seed.
7. Existing stats matter without adding a sixth authored stat.
8. Aerial contests are not decided by player array order.
9. Future jump-focused species can modify this one named aerial-contact seam.
10. Readability beats simulation detail. A failed touch must have an understandable cause.

## Non-goals

- Continuous player Z physics, free jumping, air steering, or landing collision physics.
- A dedicated jump key.
- Scissor-kick, diving-header, or first-time-pass input variants in the first implementation.
- Fouls or injuries caused by an acrobatic landing.
- Spin-aware aerial physics. Airborne `ball_spin` is not currently simulated.
- A new authored `aerial` or `first_touch` player attribute.
- Guaranteed success for a high-skill player or species.

## Player-facing controls

The controls remain contextual and use the existing keys.

| Situation | Input | Intent |
| --- | --- | --- |
| Descending aerial ball nearby | No Space | Receive and control |
| Descending aerial ball nearby | Hold Space | Conventional first-time strike |
| Descending aerial ball nearby | Hold L + Space | Acrobatic first-time strike |
| Any aerial action | Hold a direction | Aim the shot or choose the control exit direction |

Conventional strike selection is contextual:

- Lower contact: volley, jumping when extra height is required.
- Higher contact: header, jumping when extra height is required.
- `L + Space`: bicycle kick when its tighter geometry is valid. If it is not valid, fall back to
  the best conventional header or volley so the input is never eaten.

Reception is automatic because the meaningful choice is "control or strike," not "press a
button or inexplicably ignore the pass." A jump needed for reception is likewise automatic.

### Input boundary

`sim.match` should no longer infer aerial intent from the defensive `dash` or `jockey` flags.
Add abstract input fields:

```lua
---@field aerial_strike boolean  -- Space held while an aerial contact is available
---@field aerial_acrobatic boolean  -- L held with aerial_strike
```

`game/screens/match.lua` translates the physical keys. Defensive jockeying can remain active
outside an aerial opportunity, while the sim gives aerial contact precedence when a valid ball
is near the controlled player. While aerial intent has precedence, do not apply jockey slowdown
or its face-the-ball override. Preserve the player's approach facing through contact; bicycle
eligibility depends on knowing that the ball arrived overhead or behind that facing.

## Core model

### No full player Z axis

A jump is a short, committed contact action with:

- `jump_lift`: how much higher than the standing contact band the player must rise;
- `jump_ratio`: `jump_lift / max_jump_lift`, normalized to `0..1`;
- a contact instant resolved against the real `ball_z` and ground-plane distance;
- a transient pose and movement recovery timer.

The player's ground position remains a `Vec2`. The renderer lifts and poses the billboard while
its shadow remains on the pitch. This is enough to make jumping readable without introducing a
second movement simulation.

### Contact styles

The values below are initial tuning values in the current ball-height scale. Bands overlap on
purpose. The resolver evaluates every eligible style and chooses the lowest-difficulty one that
matches the player's intent.

| Style | Standing contact band | Max jump lift | Horizontal reach | Intent |
| --- | ---: | ---: | ---: | --- |
| Extended-leg control | 18-42 | 22 | 28 | Receive |
| Chest control | 34-62 | 24 | 26 | Receive |
| Volley | 18-38 | 16 | 30 | Strike |
| Header | 44-72 | 30 | 30 | Strike |
| Bicycle kick | 38-68 | 24 | 24 | Acrobatic strike |

For a style with standing maximum `max_z`:

```text
jump_lift  = max(0, ball_z - max_z)
jump_ratio = jump_lift / max_jump_lift
eligible   = ball_z >= min_z and jump_ratio <= 1 and horizontal_distance <= reach
```

An action is labeled `jumping = true` when `jump_lift` is more than a small pose threshold, not
merely because the contact band is aerial. A bicycle kick is always acrobatic and jumping.

`species.jump_reach(...)` should modify this envelope through an explicit result such as extra
horizontal reach and extra lift. It should not silently add one number to unrelated axes.

### Bicycle-kick geometry

A bicycle is eligible only when all of these are true:

- the player requested an acrobatic strike;
- the ball fits the bicycle height and jump envelope;
- the ball is overhead or behind the player's facing, measured by the dot product between
  `facing` and the ground-plane vector to the ball;
- the player is not sliding, stunned, holding the ball, or already recovering from an aerial;
- there is enough landing room inside the field boundary.

The ideal bicycle contact is slightly behind the player, not directly in front. Its alignment
difficulty measures deviation from that ideal. It has the smallest reach, highest base
difficulty, and longest recovery of the aerial actions.

## Contact prediction and candidate selection

Only descending balls above `GROUND_GRAB_HEIGHT` enter this resolver in v1. The ball keeps its
real trajectory; there is no teleport to the player.

Each step:

1. Predict a short contact horizon from the current ball position, horizontal velocity,
   `ball_z`, `ball_vz`, and gravity.
2. Let movement AI and the human aerial magnet steer toward that point.
3. When the real ball enters a style's height and horizontal envelope, build all eligible player
   candidates.
4. Resolve a contest if candidates from both teams can reach the same contact.
5. Resolve the winner's reception or strike.
6. Start a brief ball-level aerial lockout so several players cannot pinball the same ball on
   consecutive frames. Ground collection remains available.

The short prediction is assistance, not contact. Actual distance at the real contact frame is
part of difficulty, so a player stretching at the edge of reach has a worse touch than one under
the ball.

### Candidate priority and contests

The current first-player-in-array behavior must be replaced. Candidate priority uses:

```text
claim_score =
    0.45 * position_quality
  + 0.35 * action_skill
  + 0.10 * strength_norm
  + intent_bonus
  + jump_verb_edge
  + small_seeded_jitter
```

- `position_quality` is primarily `1 - stretch` with a smaller alignment contribution.
- The designated receiver gets a modest `intent_bonus`; it is not invulnerable.
- A human explicitly requesting a strike gets the same intent treatment.
- A same-team non-receiver should not steal a pass unless the receiver cannot reach it.
- Nearby opponents increase difficulty even when they lose the claim, representing body
  pressure at contact.
- `jump_verb_edge` is the future species hook. It stays probabilistic and can lose.
- Sort candidates by stable player id before consuming contest RNG, or derive jitter from the
  match seed plus player id. Reordering the runtime player array must not change the winner.

This contest decides who reaches the ball first. The winner must still roll the quality of the
touch or strike.

## Player skill

Add derived values in `sim/stats.lua`; do not add content fields to `PlayerData`.

```text
first_touch_skill = (0.75 * technique + 0.25 * mental) / 10
header_skill      = (0.35 * technique + 0.35 * mental + 0.30 * strength) / 10
volley_skill      = (0.65 * technique + 0.20 * mental + 0.15 * strength) / 10
bicycle_skill     = (0.70 * technique + 0.20 * mental + 0.10 * strength) / 10
```

All are clamped to `0..1` after species and, later, arena stat modifiers are applied.

- Technique owns touch cleanliness and acrobatic execution.
- Mental owns anticipation and composure under a dropping ball.
- Strength contributes to forceful headers and strikes; shot pace still comes from
  `shot_speed` so it is not counted twice for output power.
- Pace already matters by determining whether the player reaches the contact point.
- Stamina already matters through sprint availability. Future match-long fatigue can feed the
  balance term rather than changing these formulas.

The existing `dribble` factor should not stand in for all aerial skill. Its baseline and scaling
were tuned for ground control.

## Difficulty model

Difficulty is a normalized, inspectable value. Use the same inputs for all styles, with a style
base and small style-specific weight adjustments.

```text
difficulty = clamp(
    style_base
  + 0.24 * stretch^2
  + 0.18 * relative_horizontal_pace
  + 0.12 * vertical_drop_pace
  + 0.16 * jump_ratio
  + 0.10 * alignment_error
  + 0.10 * movement_instability
  + 0.10 * opponent_pressure
  - anticipation_bonus,
  0, 1)
```

Initial style bases:

| Style | Base difficulty |
| --- | ---: |
| Extended-leg control | 0.00 |
| Chest control | 0.05 |
| Header | 0.06 |
| Volley | 0.12 |
| Bicycle kick | 0.28 |

Inputs are defined as follows:

- `stretch`: horizontal distance divided by the style's contact reach.
- `relative_horizontal_pace`: ball velocity relative to player velocity, remapped from an easy
  pace near `80 px/s` to a hard pace near `600 px/s`.
- `vertical_drop_pace`: absolute downward speed, remapped from `80..500 px/s`.
- `jump_ratio`: required lift divided by the style's maximum lift.
- `alignment_error`: deviation from the style's ideal body contact. Control, headers, and
  volleys prefer the ball in front; bicycles prefer it overhead or slightly behind.
- `movement_instability`: sprint speed, a sharp last-second turn, active stumble, or other poor
  body balance. Sliding and stun remain hard eligibility failures.
- `opponent_pressure`: proximity of the nearest opponent inside roughly `64 px`, strongest at
  body contact.
- `anticipation_bonus`: small reduction for the designated receiver of an intentional pass.

There is deliberately no frame-perfect button timing term in v1. Holding the action communicates
intent; positioning and stretch express timing quality. This keeps aerial play arcade-readable.

### Outcome probabilities

Let `margin = action_skill - difficulty`. Use two seeded rolls:

```text
p_contact = clamp(0.82 + 0.35 * margin, 0.30, 0.995)
p_clean_given_contact = clamp(0.58 + 0.60 * margin, 0.08, 0.97)
```

1. Miss if the first roll is above `p_contact`.
2. Clean if contact occurs and the second roll is below `p_clean_given_contact`.
3. Otherwise produce a heavy reception or mishit strike.

These are starting values, not balance promises. Important invariants:

- Increasing skill never lowers either probability.
- Increasing any difficulty input never raises either probability.
- Even elite players can fail the hardest actions.
- Easy, unpressured control by a skilled receiver should be nearly automatic.
- Seed and inputs fully determine the outcome.

## Reception outcomes

Reception does not immediately teleport the ball into possession. Contact redirects the real
ball toward a foot target and lets the existing ground collection complete the second touch.

The foot target is `STICK_AHEAD` from the player in the held movement direction, falling back to
facing. This lets the user cushion into space without adding another button.

### Clean

- Emit `reception` with style, `jumping`, `difficulty`, and `outcome = "clean"`.
- Redirect the ball down toward the foot target over a short style-specific duration.
- Chest control takes about `0.18..0.24s`; extended-leg control takes `0.08..0.14s`.
- Match the horizontal exit pace mostly to player velocity, with a small controllable lead.
- Keep the receiver's `receive_timer` active so it can complete the follow-up at normal foot
  height even if the pass arrived quickly.
- Apply only a short settle window after ownership is gained.

### Heavy

- Emit `reception` with `outcome = "heavy"`.
- Deflect the ball downward but add seeded lateral and weight error.
- Keep the ball loose. The intended receiver continues chasing it, but opponents can win it.
- Use a longer settle window if the receiver recovers possession.

### Miss

- Emit `reception` with `outcome = "miss"` for animation and metrics.
- Leave the ball trajectory unchanged, or apply only a small glancing deflection when the style
  made physical contact.
- End the protected designated-receiver window; the ball is fully contestable.
- Apply aerial recovery so repeated attempts cannot occur immediately.

The actual chest-to-foot or leg-to-foot trajectory is important. It makes control visible and
creates the press-the-first-touch opportunity already promised by the game.

## Strike outcomes

The same resolver replaces the current always-connect header and separate volley sky roll.

### Header

- Standing or jumping based on `jump_lift`.
- Clean: current directed target behavior, moderate pace, generally downward `ball_vz`.
- Mishit: reduced pace plus angular or height error; it can glance wide or loop upward.
- Miss: unchanged ball plus landing recovery.

### Volley

- Standing or jumping based on `jump_lift`.
- Clean: high pace from `shot_speed`, small direction error, controlled vertical output.
- Mishit: slice, reduced power, or a skied ball into the cage.
- Miss: ball continues and the player lands off balance.

The current independent `VOLLEY_SKY_P` should be absorbed into mishit output selection so one
quality model owns aerial failure. `HEADER_SPEED` can remain the output-power knob.

### Bicycle kick

- Always uses `aerial_acrobatic` intent and valid behind/overhead geometry.
- Clean: powerful goal-directed strike with more angular variance than a clean volley.
- Mishit: commonly skied, sliced, or under-hit; never silently converted to a perfect header.
- Miss: ball continues; the player falls and has the longest aerial recovery.
- Recovery starts on attempt, not only on contact, so a bicycle is a committed risk.
- Direction input aims the target. With no direction, target the opponent goal as current aerial
  strikes do.

Suggested recovery values:

| Action | Movement during action | Recovery |
| --- | ---: | ---: |
| Chest / leg reception | 60% | 0.18s |
| Standing header / volley | 55% | 0.22s |
| Jumping header / volley | 35% | 0.35s |
| Bicycle kick | 0% | 0.60s |

## AI behavior

AI uses the same eligibility, difficulty, contest, and outcome functions.

- Intended receiver defaults to control.
- In attacking range, compare conventional strike quality with control value. Strike when a
  first-time attempt is credible or immediate pressure makes control worse.
- In the defensive third, clear dangerous high balls. Prefer a header, then a volley if the
  height demands it; control an unpressured delivery rather than heading every ball away.
- Attempt a bicycle only when its geometry is valid and it is better than the available
  conventional strike, usually because the ball is behind a goal-facing attacker. Do not gate it
  with an arbitrary player-id or position rule.
- Do not let AI use hidden reach or success bonuses. Difficulty level may later alter decision
  thresholds, not contact physics.

This should reduce the current failure mode where attacks fail to resolve while avoiding nonstop
low-value headers from midfield.

## Sim architecture

Add `sim/aerial.lua` as a pure module rather than growing the aerial block inside
`sim/match.lua`.

Suggested public shapes:

```lua
---@alias AerialIntent "receive"|"strike"|"acrobatic"
---@alias AerialStyle "leg_control"|"chest_control"|"volley"|"header"|"bicycle"
---@alias AerialOutcome "clean"|"heavy"|"miss"

---@class AerialContext
---@field ball_pos Vec2
---@field ball_vel Vec2
---@field ball_z number
---@field ball_vz number
---@field player_pos Vec2
---@field player_vel Vec2
---@field facing Vec2
---@field move_speed number
---@field skill number
---@field strength number
---@field opponent_distance number
---@field anticipated boolean
---@field extra_reach number
---@field extra_lift number

---@class AerialContact
---@field style AerialStyle
---@field jumping boolean
---@field jump_ratio number
---@field difficulty number
---@field reach_ratio number

---@class AerialResolution
---@field contact AerialContact
---@field outcome AerialOutcome
---@field rng integer
---@field angle_error number
---@field weight_error number
```

Suggested API:

```lua
aerial.contacts(context, intent) -> AerialContact[]
aerial.best_contact(context, intent) -> AerialContact?
aerial.difficulty(context, contact) -> number
aerial.resolve(context, contact, rng_state) -> AerialResolution
```

The geometry, difficulty, and outcome APIs above remain pure. To keep `sim/match.lua` below
LuaJIT's 200-local chunk limit and isolate concurrent work, `sim/aerial.lua` also exposes a thin
`resolve_play(state, input, config)` adapter that gathers candidates and applies the returned
ball velocity, ownership, timers, and events. `sim/match.lua` owns the state fields and invokes
that adapter once during loose-ball resolution.

### Runtime state

Rename the narrowly named `header_cd` to aerial state that every contact can use:

```lua
---@field aerial_cd number
---@field aerial_timer number
---@field aerial_style AerialStyle?
---@field aerial_outcome AerialOutcome?
---@field aerial_jump number  -- 0..1 for renderer lift
---@field aerial_recovery number
```

Add a short `MatchState.aerial_lock` timer for ball-level anti-ping-pong protection. Reset all
fields at kickoff and decrement them with the existing timers.

### Events

Extend `MatchEvent` with optional aerial detail:

```lua
---@field kind "shot"|...|"header"|"volley"|"bicycle"|"reception"
---@field style AerialStyle?
---@field outcome AerialOutcome?
---@field jumping boolean?
---@field difficulty number?
```

Keep `header` and `volley` event kinds so current shot metrics remain compatible. Add `bicycle`
to shot counting. `reception` records every attempted aerial control, including misses.

## Rendering and feedback

`player_renderer.lua` already owns transient procedural poses. Add one `aerial` option containing
style, jump amount, and normalized action progress.

- Keep the shadow at the projected ground point.
- Lift the figure in screen Y by `jump_ratio` while preserving depth sorting by ground position.
- Chest: torso back, arms out, ball redirected from chest height.
- Extended leg: one planted or jumping leg reaches toward contact.
- Header: torso and helmet drive toward the ball.
- Volley: striking leg extends through the contact point.
- Bicycle: torso rotates toward horizontal and legs scissor; recovery briefly shows the player
  down before standing.
- Clean, heavy, and miss use different sound weight and small contact effects. Do not put
  probability text over the player during normal play.

Because reception redirects the real ball rather than attaching it immediately, the chest-to-foot
motion is visible through the existing ball-height projection.

## Tuning

Keep the difficulty weights as code constants initially. Exposing every term in F1 would make the
panel harder to reason about. The useful high-level knobs are:

- `AERIAL_ASSIST`: steering/contact forgiveness for the human, with physical stretch still
  represented in difficulty;
- `AERIAL_MAGNET`: steering speed toward the predicted contact;
- `AERIAL_JUMP_LIFT`: global maximum-lift scale;
- `AERIAL_CONTROL_ERROR`: heavy-touch direction and weight spread;
- `BICYCLE_DIFFICULTY`: acrobatic style base;
- existing `HEADER_SPEED` and strike output speeds.

The current `AERIAL_ASSIST = 44` should not remain a raw addition to physical contact reach. Use
most of it for steering/anticipation and cap actual reach assistance, otherwise a player can kick
a ball several body widths away while the new distance difficulty becomes meaningless.

## Metrics and target behavior

Track:

- aerial receptions attempted / clean / heavy / missed;
- receptions won by the intended receiver and by an opponent;
- standing vs jumping contacts;
- header / volley / bicycle attempts, clean contacts, misses, and goals;
- average difficulty by action and outcome.

Initial playtest targets, to be validated rather than optimized blindly:

- Skilled, unpressured receiver on a normal lob: `90-98%` makes contact, mostly clean.
- Low-skill receiver stretching for a fast, steep, pressured lob: roughly `35-65%` contact, with
  heavy touches more common than clean ones.
- Jumping headers are common enough to read in one match when crosses are used.
- Bicycle kicks are memorable, not routine: valid attempts should be uncommon and misses visible.
- Adding reception should improve pass completion without making every lob safer than a ground
  pass.

## Testing

### Pure resolver specs: `spec/sim/aerial_spec.lua`

1. Each style accepts and rejects the correct height, lift, and horizontal reach boundaries.
2. Higher ball distance, relative pace, drop speed, jump requirement, instability, and pressure
   monotonically increase difficulty.
3. Higher relevant stats monotonically improve success across a fixed seed set.
4. Same context and seed produce the same contact, outcome, and error.
5. Bicycle requires acrobatic intent plus overhead/behind geometry.
6. Invalid bicycle geometry falls back to a conventional contact.

### Match integration specs

1. A human designated receiver who does not press Space chest-controls a descending lob and can
   gain foot possession before it lands naturally.
2. A lower ball uses an extended-leg reception.
3. Clean reception sends the real ball toward the chosen foot target; it does not teleport into
   ownership at chest height.
4. A forced heavy touch stays loose and is contestable.
5. A forced miss preserves the incoming trajectory and applies recovery.
6. Space produces a standing or jumping volley in the lower strike band.
7. Space produces a standing or jumping header in the higher strike band.
8. `L + Space` produces a bicycle on valid geometry, including a clean and a missed seeded case.
9. A bicycle attempt applies recovery even when it misses.
10. A better-positioned aerial candidate wins regardless of player array order.
11. A pressured intended receiver can lose the contest.
12. AI attackers, defenders, and receivers choose strike, clearance, and control respectively.
13. Existing ground pickup, keeper claim priority, pass completion, and goal-height behavior do
    not regress.

### Renderer specs

- Headless draw smoke covers every aerial pose option.
- One opt-in visual snapshot pins a jumping header and one pins a bicycle at readable silhouette
  angles after the renderer is tuned by eye.

## Implementation order

1. Add `sim/aerial.lua`, derived stats, pure geometry/difficulty/outcome specs.
2. Replace array-order aerial striking with candidate and contest resolution.
3. Add automatic leg/chest reception and real ball redirection to the foot target.
4. Add jumping header, jumping volley, and bicycle output/recovery.
5. Add abstract input flags and preserve the existing Space/L physical controls.
6. Add AI decisions and events/metrics.
7. Add procedural poses, audio/effects, and renderer smoke coverage.
8. Tune with fixed-seed scenarios, then run the full headless match suite and fun metrics.

This mechanic was explicitly reprioritized during the renderer-only visual spike. Its match
integration was kept narrow, with contact resolution and orchestration isolated in
`sim/aerial.lua`.

## Acceptance criteria

- A lob receiver can control the ball in the air without pressing the strike button.
- Chest and extended-leg receptions both occur and have clean, heavy, and miss outcomes.
- Distance, horizontal pace, drop pace, jump height, alignment, balance, pressure, technique,
  mental, and strength influence the documented parts of resolution.
- Jumping volleys and jumping headers reach balls above their standing contact bands.
- Bicycle kicks exist as explicit, high-risk acrobatic strikes with miss and recovery states.
- Aerial contests are independent of player array order.
- The sim remains pure, deterministic, and LuaJIT/5.1-compatible.
- Tests prove monotonic difficulty and skill behavior, seeded failures, and match integration.
- No new authored player stat or full player Z simulation is introduced.
