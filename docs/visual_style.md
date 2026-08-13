# Galactic Cup visual style

Galactic Cup's non-match UI is an **intergalactic sports broadcast**, not a
spaceship dashboard. It combines a quiet space backdrop with strong match-day
hierarchy: one headline, one obvious action, compact scouting cards, and bright
focus treatment.

The implementation source of truth is `ts/packages/ui/src/theme.ts`; screens
describe semantic widgets and `ts/packages/ui/src/draw.ts` renders them. Screens must not invent
private palettes or duplicate component drawing.

## Palette

| Token | Purpose |
| --- | --- |
| Void / Space | Letterbox and page backgrounds |
| Panel / Raised Panel | Information cards and interactive controls |
| Cyan | Focus, navigation, broadcast labels, Nebula emphasis |
| Amber | MVP moments, warnings, opponent or high-energy emphasis |
| White / Muted | Primary and secondary copy |

Selected and focused states also change borders, fill, and markers; color is
never the only signal. Species accents come from `gc_data::species`
(`rust/crates/gc-data/src/species.rs`). Team
colors remain content data and should be reserved for team ownership, not
general navigation.

## Type and copy

The code-native scale is 11px eyebrow, 13px body, 24px title, and 38px hero at
the 960×540 virtual resolution. Eyebrows and actions use concise uppercase
copy. Body copy uses sentence case and should explain a decision in one line
where possible.

Screen hierarchy is:

1. Optional broadcast eyebrow.
2. One title or hero score.
3. Decision cards or primary content.
4. A muted correction/help line.
5. Back and primary actions in a consistent footer.

## Spacing and layout

- All screens use a 960×540 internal canvas.
- Page gutters are at least 62px; related controls use 10–18px gaps.
- Interactive targets are at least 38px tall.
- Content must stay inside the virtual canvas at 960×540 and scale through
  `ts/packages/ui/src/viewport.ts` for larger or letterboxed windows.
- Pointer coordinates are converted into virtual space before hit-testing.

## Components and states

Panels use a dark fill, a restrained blue border, and a 6px corner radius.
Cards add a five-pixel identity rail; species cards also use a geometric
marker so identity survives grayscale viewing.

Buttons have four deliberate states:

- Resting: raised panel with a visible border.
- Focused: cyan border, brighter fill, and a left chevron.
- Selected: persistent brighter fill and selection marker.
- Disabled: muted fill and text, removed from focus and pointer activation.

Focus and selection are separate concepts. Moving focus never changes a
formation, tactic, or team sheet until the player confirms.

## Motion

Routes use a 180ms left-to-right broadcast reveal implemented by the shared
renderer. Screen reducers remain pure and know nothing about animation.
Motion never delays input, changes hit boxes, or carries gameplay meaning.

Avoid looping menu animation, parallax that competes with copy, camera shake
outside the match, and transition sequences longer than 250ms. A future
reduced-motion setting may set the reveal progress directly to complete
without changing route behavior.

## Screen-specific accents

- Title: strongest hero type and most open negative space.
- Squad: species rail, geometric identity, role, and five stats.
- Formation: selected-five species markers on a small pitch.
- Tactic: strength and risk in plain language; no raw tuning values.
- Result: score first, trustworthy stats second, amber MVP card third.
- Help, Settings, Pause, Credits: the same panels and focus states with lower
  information density.

The Helios Crown broadcast presentation carries the same language into play:
compact broadcast rails, cyan/amber state accents, geometry-backed status,
and dark raised panels that remain legible over the arena.
