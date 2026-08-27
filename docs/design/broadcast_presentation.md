# Broadcast presentation — Helios Crown Showcase

## Decision

The remaining match presentation ships as one system: **Helios Crown
Showcase**. It makes the existing simulation read like a finished
intergalactic sports product without adding rules, powers, or match verbs.

The design answers four questions at a glance:

1. What is the score and how much time remains?
2. Which player do I control, and what state are they in?
3. Who has the ball?
4. Which pre-match plan is active?

## Broadcast spine

The match HUD uses the shared product palette and contains:

- a top scorebug with team rails, score, and a separate clock chip;
- a text-and-diamond possession readout, so color is never the only signal;
- a lower-left controlled-player capsule with name, species, role, state, and
  a labeled, ticked stamina meter;
- a lower-right plan chip carrying tactic and formation into the match;
- a labeled, ticked world-local `SHOT` or `PASS` charge meter;
- one non-modal first-fixture prompt at a time.

The permanent developer control footer is removed. The pause Controls screen
is the complete reference.

## First-fixture onboarding

Prompts are presentation state, not simulation state. Each lesson appears
once, for at most six seconds, and retires when its taught action is used.
Pause, goal replay, and shell interruption naturally freeze its clock because
the match screen is not updated.

Priority:

1. identify the double-ringed controlled player and teach movement/sprint;
2. teach shoot/pass on first possession;
3. teach jockey/switch while defending;
4. teach throw/punt when the controlled keeper gathers.

`GameSession.first_match` is carried through the typed match request. Rematches
and later fixtures disable the reducer entirely.

## Pitch identity

The authored `presentation_species` field remains presentation-only. Mechanical
species stays neutral for the showcase release.

All players share the current animation rig:

- **Terran:** balanced torso and round helmet;
- **Gravling:** widest torso, thick limbs, broad low head;
- **Voltari:** narrow torso and angular crest;
- **Myceloid:** narrow stem and unmistakable three-lobed crown.

Team color remains the dominant kit color. Species palette is an accent.
Home uses a continuous chest band; away uses a split band. The controlled
player has two rings and a downward chevron, including during dives and aerial
poses.

## Helios Crown

There is exactly one arena treatment and no arena mechanic. Its procedural
presentation consists of:

- a large amber solar collector and orbital rings at the horizon;
- cyan and amber spectator/light ribbons;
- a dark neon hex pitch;
- four corner pylons framing the projected field;
- the venue slate `HELIOS CROWN · KAIRON-9 ORBIT`.

The arena record lives in pure data. Rendering depends on it; simulation does
not.

## Match beats

| Phase | Presentation |
| --- | --- |
| Kickoff | `SHOWCASE FIXTURE`, teams, venue, and active plan |
| Goal | `GOAL · TEAM`, current score, existing celebration |
| Replay | Compact `REPLAY` panel and one skip hint |
| Full time | 0.9-second pitch hold, final score, then Result |

Confirm may advance full time after a 0.25-second safety beat. Completion fires
once. The Result screen owns rematch decisions in the product profile.

## Audio hierarchy

The synthesized audio set remains asset-free and headless-safe. Relative cue
gains keep touches and passes below tackles and strikes, with goal and full
time at the top. A synthesized double whistle closes the fixture. Continuous
crowd ambience continues to decay during replay without replaying event cues.

## Product and playtest profiles

- `product`: used by the application shell; no tuning panel, bloom debug key,
  or internal full-time rematch.
- `playtest`: used by direct match tools and legacy characterization tests;
  preserves F1 tuning and internal restart behavior.

## Explicit cuts

This presentation package does not add multiple arenas, hazards, weather, species powers, portraits,
unique rigs, music, commentary, crowd simulation, minimaps, camera cuts,
cinematic zoom, shaders, new particles, or tutorials for every advanced verb.
Aerial mechanics remain documented in Controls rather than expanding the
first-fixture prompt sequence.

## Acceptance evidence

- Every authored player resolves a valid pitch identity.
- All four silhouettes draw through normal, keeper-dive, and aerial paths.
- HUD regions stay in bounds at 960×540, 1280×720, 1920×1080, and 1280×800.
- Possession, stamina, charge, selection, and replay state use text or geometry
  in addition to color.
- Onboarding is first-fixture-only, contextual, bounded, and non-modal.
- Full time remains visible for at least 0.9 seconds and completion fires once.
- Product mode rejects internal rematch; playtest mode preserves it.
- Headless audio accepts replay ambience and full-time transitions.
- The seeded gameplay baselines remain unchanged.
