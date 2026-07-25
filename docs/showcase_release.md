# Showcase release

## Release decision

The next release is a **small complete game and a source-available portfolio
showcase**, not the first layer of a large career mode.

Galactic Cup already has the expensive part: a typed, deterministic 5v5 match
simulation with substantial player verbs, 2.5D rendering, audio, replays,
headless balance tools, and a broad test suite. Building more simulation
systems now would make the game wider without fixing the experience a new
player or portfolio reviewer actually sees.

For this release, gameplay breadth is frozen. Work moves to the product frame
around the match: a front door, a meaningful but quick team setup, coherent
input, readable presentation, a satisfying finish, and a repository that is
easy to understand and run.

## Product pitch

**Galactic Cup is a 5v5 intergalactic arcade soccer game where you coach for
thirty seconds, then personally execute the plan.**

The appeal comes from a direct bridge:

1. Pick five characters with clear strengths and tradeoffs.
2. Choose a formation and a one-sentence tactical identity.
3. Play a two-minute match where those choices are visible.
4. Read the result, adjust, and go again.

The visual direction is **intergalactic sports broadcast**: dark space,
electric arena color, bold team identity, compact stat cards, and match
presentation that feels like a televised event rather than a debug view.

## Target experience

A first-time player should be able to clone or download the game, reach
kickoff without external instructions, finish a match, understand the result,
and choose what to do next.

Target session:

```text
Title
  -> Play
  -> Pick 5 of 8
  -> Choose formation
  -> Choose tactic
  -> Match
  -> Result + MVP
  -> Rematch / Change plan / Main menu
```

The complete loop should take roughly 5–10 minutes. It is framed as a
showcase fixture with a real beginning and ending, not as an unfinished
career save.

## The 20% that supplies 80% of the quality

### 1. A real product shell

- Title screen: Play, How to Play, Settings, Credits, Quit.
- Pause screen: Resume, Controls, Settings, Restart, Main Menu.
- Back navigation on every setup screen.
- No launch directly into a developer screen and no use of `Esc` as an
  unconditional quit while playing.

Why it stays: these surfaces establish intent and prevent the game from
feeling like a test harness.

### 2. One meaningful manager decision chain

- Choose exactly five starters from Nebula FC's eight-player roster.
- Keep the existing three formations.
- Keep the existing three tactics, each with plain-language strengths and
  risks plus a preview.
- Carry the selected roster, formation, and tactic into the match and result.

Why it stays: this is the smallest version of the project's north star,
“manager choices visibly change what happens on the pitch.”

### 3. Alien characters that read immediately

- Ship four roster archetypes: Terran, Gravling, Voltari, and Myceloid.
- Express identity through names, one-line copy, silhouettes/colors, and
  authored stat profiles that already affect the match.
- Show species and role on player cards and in the selected-player HUD.
- Do **not** add signature powers, energy, cooldowns, or another modifier
  layer for this release.

Why it stays: the space setting needs to be visible in the first minute, but
the existing stat system can carry the mechanical identity without a new
combat-like subsystem.

### 4. One polished match

The current match is the feature. Preserve its existing verbs and finish
their presentation:

- Keyboard and standard gamepad parity.
- Contextual first-match prompts and an always-available controls page.
- Clear controlled-player, possession, charge, stamina, tactic, score, and
  time feedback.
- Cohesive match HUD, audio mix, goal beat, replay, full-time transition, and
  one visually distinctive arena presentation.
- Release builds hide playtest-only controls and tuning surfaces.

Why it stays: confidence, feedback, and control consistency improve every
second of play. New rules do not.

### 5. A satisfying finish

- A dedicated result screen, not an overlay on the frozen pitch.
- Final score, winner/loser treatment, MVP, and a small set of trustworthy
  match stats such as shots, possession, saves, and pass completion.
- Clear actions: Rematch, Change Lineup/Plan, Main Menu.
- A short win presentation that makes completing the fixture feel
  intentional.

Why it stays: the result screen converts a simulation ending into a game
ending and closes the loop.

### 6. A repository that explains itself

- Honest public README with a short gameplay clip and screenshots.
- One-command setup and one-command quality gate.
- CI for format, types, tests, and gameplay tripwire.
- Architecture and contribution guidance.
- Explicit source-available license, release notes, and a downloadable Linux
  build.

Why it stays: for a portfolio project, setup quality and engineering
communication are part of the product.

## Scope ledger

### Commit for the showcase release

| Area | Commitment |
| --- | --- |
| Teams | Nebula FC and Orion Miners |
| Roster choice | Pick 5 starters from 8 Nebula players |
| Character identity | 4 species/archetypes, shown in cards and on the pitch |
| Formations | 2-1-1, 1-2-1, 1-1-2 |
| Tactics | Balanced, Press High, Counter Attack |
| Competition | One self-contained showcase fixture |
| Match | Existing 5v5 rules, two minutes / first to three |
| Input | Keyboard plus one standard gamepad layout |
| Screens | Title, How to Play, Settings, Squad, Formation, Tactic, Pause, Match, Result, Credits |
| Presentation | One cohesive UI theme and one distinctive arena treatment |
| Public project | README media, CI, license, contribution guide, packaged release |

### Explicitly defer

- League or season simulation, tables, calendars, brackets, and persistence.
- XP, levels, morale, form, fatigue across matches, and player growth.
- Signature skills, energy systems, species cooldowns, and active powers.
- Arena modifiers, hazards, weather, and multiple arena content.
- Transfers, contracts, money, scouting, owners, staff, and youth systems.
- Galactic Gazette, rivalries, procedural narrative, and career objectives.
- Local multiplayer, online play, create-a-club, and mod tooling.
- Additional teams beyond the opponent needed for the showcase fixture.

### Explicitly stop pursuing for this release

- New match verbs or rule systems: through-balls, one-twos, set pieces,
  fouls/cards, offside, halves, and weather.
- More simulation metrics unless they protect a shipping choice.
- Balance searches without a concrete observed playtest problem.
- UI for a future manager system.

Speculative manager systems and other future research are intentionally kept
outside the public release scope.

## Definition of done

The showcase is ready only when all of the following are true.

### Player experience

- A first-time player can reach kickoff in under 90 seconds without reading
  the repository documentation.
- The full title-to-result-to-rematch loop works with mouse/keyboard and with
  a gamepad.
- Every screen has a visible primary action and a working back path.
- Pause, focus loss, restart, return to menu, and quit behave deliberately.
- The player can explain what their roster, formation, and tactic changed
  after one or two matches.
- Three consecutive complete matches can be played without a crash, stuck
  state, debug UI, or stale result data.

### Presentation

- Menus, player cards, HUD, arena, results, and credits share one visual
  language.
- No placeholder copy, clipped text, debug labels, or unexplained controls
  are visible in a release build.
- Audio has useful relative levels and mute/volume settings persist for the
  session.
- The game remains readable at the supported window sizes and in fullscreen.

### Engineering

- `./scripts/check.sh` passes from a clean checkout.
- CI runs the same meaningful gate and reports failure clearly.
- Pure state transitions cover the complete screen flow in headless tests.
- Input mappings are centralized rather than duplicated per screen.
- Release and debug behavior are separated explicitly.
- The Linux release artifact launches without the source tree or developer
  tools.
- The browser artifact remains buildable and smoke-tested as a parallel
  delivery path.

### Source-available project and portfolio

- `README.md` opens with a strong screenshot or short GIF and an honest
  feature summary.
- Setup instructions have been tested on a clean environment.
- `LICENSE`, `CONTRIBUTING.md`, release notes, and repository topics are
  present.
- The README explains the pure-sim architecture, testing strategy, and
  balance tooling without overwhelming the game pitch.
- A tagged release contains the downloadable Linux build and a concise
  changelog.

## Release order

The surrounding product is built against typed contracts and a deterministic
fake match. The current match/aerial work is integrated last:

1. **Contracts and foundations** — session, match request/result, fake match
   adapter, abstract input, viewport, and settings.
2. **Complete application shell** — title, help, settings, pause, credits,
   squad selection, tactical setup, and result loop.
3. **Identity and visual system** — alien roster treatment and shared UI
   presentation across every non-match screen.
4. **Public release engineering** — PolyForm license, CI, packaging,
   contribution policy, and asset provenance.
5. **Real match integration** — connect the existing match/aerial work to the
   stable request/result/input seams, then apply HUD and arena presentation.
6. **Release QA and publishing** — clean builds, onboarding tests, media,
   release notes, tagged binaries, and corresponding source.

The architectural boundaries and contribution gates are documented in
`AGENTS.md` and `CONTRIBUTING.md`.

## Scope-change rule

A proposed feature enters the showcase only if it fixes a failure in the
definition of done and is cheaper than solving that failure through
presentation, copy, input, or flow. Otherwise it goes into the post-showcase
parking lot.

This rule is deliberately strict. The release should look small because it is
focused, not because it is unfinished.
