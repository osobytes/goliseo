# Controls

## Match

Two contextual action buttons plus sprint. The same button does the natural
thing for the moment, and the first fixture teaches the essential contexts
without pausing play.

| Action | Keyboard | Gamepad |
|--------|----------|---------|
| Move | WASD or arrows | Left stick or D-pad |
| **ACTION** — shoot / tackle | Space | A |
| **PLAY** — pass / switch | K | X |
| Sprint | Shift (hold) | LB (hold) |
| Lob / chip modifier | L (hold) | Y (hold) |
| Juke / dodge | C | L3 |
| Equipment | J (hold/tap) | B (hold/tap) |
| Pause | P or Esc | Start |
| Skip replay / advance full time | Space or Enter | A |
| Toggle mute | M | — |
| Toggle fullscreen | F11 | — |

Rematches are chosen on the dedicated result screen. Full time owns the pitch
for a brief broadcast beat before that screen appears.

- **Shooting** (Space with the ball) aims at the goal; hold up/down to place it into a corner.
  Hold Space to **charge** a harder shot; holding left/right at release **curves** it.
- **Tackling** (Space without the ball) has two modes: **tap** (release quickly) fires a
  standing poke; **hold** enters **jockey stance** — you slow to 75 % and your facing locks
  toward the ball, shadowing the carrier. Releasing Space from jockey fires the poke with
  a **+6 bonus reach** as the reward for containing first. While **sprinting** instead of
  jockeying, the poke becomes a committed **slide tackle** whose speed scales with your pace.
  Slides reach further and knock the carrier off balance, but lock you in with a longer
  recovery — *sprint + Space* is the big play, and it can miss.
- **Sprint** (hold Shift) burns a **stamina meter** (shown above the help text when not full);
  it refills whenever you're not sprinting, and the **stamina** stat sets how big the tank is.
  An empty tank means no boost until it meaningfully recovers.
- **Switching** (K without the ball) hands control to the home outfielder **nearest the ball**;
  winning the ball auto-switches control to the winner.
- **Keeping the ball**: challenges reach for the *ball*, not your body — it sticks a step ahead
  of your facing, so turning between a defender and the ball **shields** it. Defenders commit
  to their pokes, go on cooldown when they miss, and **stumble** after lunging past a shielded
  ball — bait the poke, then break away. At kickoff the other team must stand off
  (centre-circle distance). But shielding is **active**: the presser hunts the ball itself and
  will work around your body, and a defender leaning on you shoves you off your spot — standing
  still is never safe, keep turning or move.
- **You control your keeper**: when your keeper gathers the ball, control switches to it.
  **K** (hold to charge) throws — the longer you hold, the further along your aim it picks a
  teammate; **Space** (hold to charge) is a **punt** off the foot, clearing high toward
  mid/front field as far as you charge it. You can walk it around inside your box; after ~5
  seconds the keeper distributes on its own (six-second rule).
- **Crosses**: a lofted pass (hold L + K) from wide in the attacking third becomes a **cross**
  aimed at your teammate in the box — AI wingers swing them in too. **Control follows every
  pass you make**: the moment the ball leaves your foot you're driving the receiver, so run
  onto the cross and meet it with Space — holding a direction at contact **aims the header or
  volley** wherever you point (undirected strikes go at goal).
- **Aerial reception**: do not press Space under a descending lob to control it. The player
  automatically chooses an extended-leg or chest touch, jumping when needed, and cushions the
  real ball toward their feet. Technique, mental, distance, ball pace, drop speed, required
  jump, body alignment, movement, and pressure decide whether the touch is clean, heavy, or
  missed. Hold a direction to cushion into that space.
- **Aerial finishing**: hold **Space** under a dropping ball for a standing or jumping header /
  volley. Hold **L + Space** to request a **bicycle kick** when the ball is overhead or behind;
  invalid bicycle geometry safely falls back to a conventional strike. Bicycles are powerful
  but difficult and leave the player recovering on the ground. The arena is a **cage**: a skied
  strike bounces off the ceiling and rains back into play.
- **The keeper is protected**: while a keeper holds the ball, everyone backs well off its ring
  (laws of the game — a keeper in possession can't be challenged), and markers drop off the
  outlets to guard the passing lanes instead of standing on the receiver's boots. Read the
  throw and step into the lane, or press the receiver's first touch.
- **Winning the ball**: your poke has generous reach and a forgiving window. Approach from the
  ball side for the extended reach — but at body-contact range your poke works from ANY angle
  (a toe through the legs), so chasing a carrier down is always winnable. When the opponent
  wins the ball, control auto-switches to your best-placed defender. AI players need a **settling touch** (~a third of a
  second) after receiving before they can pick a pressured pass — press them on the touch and
  the ball is winnable. Passes released into an adjacent defender get dinked over them; a ball
  dropping onto its receiver can't be walled off, only flat drilled balls can.
- **Juke** is a quick sidestep with brief tackle immunity — beat a defender or escape a challenge.
- Players have **bodies**: they block and bump each other, and a slide that connects shoves and
  briefly stuns the player it hits.
- Fast shots and driven passes **ricochet off bodies** — a defender in the lane blocks the shot,
  so shoot around them, chip over them (L), or pass for a better angle. Slow balls are trapped,
  not deflected, and lobs sail over heads.
- **Finishing**: charge and aim for a corner to beat the keeper. Whether a reached save is
  **held or parried** is a dice roll weighted by shot pace and the keeper's handling — soft,
  central shots stick in the gloves almost every time; hot or full-stretch balls usually get
  pushed away (rebounds!), and a charged rocket straight at the keeper is a genuine coin flip.
  AI strikers work the same way — the space you give them becomes shot power, so close
  shooters down.

Movement is continuous (read each frame as a direction vector) and **momentum-based**: players
accelerate to top speed over ~0.25 s and decelerate to a stop over ~0.18 s. Sprinting commits
you to your current direction — reversing while at full speed takes an arc, not an instant pivot.
Your **facing** follows your actual velocity so you can still aim while braking (the ball sticks
to wherever you're pointing, not where you're running).
Shooting is a discrete, edge-triggered event (queued on key press, consumed on the next
simulation step), so taps don't get lost between frames.

Equipment input likewise preserves held state plus distinct press and release edges at the
fixed-tick boundary. A fast tap between ticks reaches the simulation as an ordered press and
release on the next tick. Gamepad B remains the Back action outside a live match.

Equipment is active only after choosing **Combat Prototype** on the title screen. **Play
Showcase** remains the committed release path and constructs no combat state; its match rules
and onboarding are unchanged.

## Goal replays

Every goal rolls an automatic **slow-motion replay** of the last few seconds
from the broadcast camera — the buffer is recorded live every frame and played
back through the normal renderer with interpolation, ending with the ball in
the net. **Space/Enter or gamepad A** skips. Replay speed and length are on the
direct-playtest tuning panel (F1 → Replay).

## Tuning panel (direct playtest profile only)

The default product shell hides tuning, bloom, and internal rematch controls.
When the match screen is mounted directly with the `playtest` profile, press
**F1** to pause and open the live tuning panel — the
gameplay knobs from `sim/tuning.lua` (movement, attacking, defending, keeper,
AI), editable mid-session the way studio balance tools work:

- **↑/↓** select a knob, **←/→** adjust it (**Shift** = ×10 steps), **Tab**
  switches category. Changed knobs show a `*`.
- **Backspace** resets the selected knob (**Shift+Backspace** resets all).
- **F2** saves your tuning, **F3** loads it; saved tuning is auto-loaded when a
  match starts, so experiments persist across runs. Tests always run on
  defaults.
- **F1/Esc** closes and resumes play.

## Menus (pre-match flow)

Use the mouse, arrows/WASD plus Enter, or D-pad plus A. Every setup screen has
a visible Back action. The flow is Squad → Formation → Tactic → Match.
