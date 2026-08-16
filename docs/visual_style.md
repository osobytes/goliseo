# GOLISEO visual style

GOLISEO's non-match UI is a **match-day broadcast staged inside the coliseum**:
one headline, one obvious action, compact scouting cards, and bright focus
treatment, lit by the same braziers the renderer lights the arena with.

The implementation source of truth is `ts/packages/ui/src/theme.ts`; screens
describe semantic widgets and `ts/packages/ui/src/draw.ts` renders them. Screens must not invent
private palettes or duplicate component drawing.

## Palette

Nothing here is invented. Every value in `theme.ts` is sampled from the shipped
coliseum renderer — `stadium_bowl.ts`, `stadium_props.ts`, `stadium_sky.ts` in
`ts/packages/render/src/` — and each token carries a comment naming its source.
If one of those renderer constants moves, `theme.ts` is stale and should follow.

| Token | Sampled from | Purpose |
| --- | --- | --- |
| Zenith / Sky Mid / Sky | `stadium_sky.ts`'s fragment shader | Letterbox bars, transition wipe, the backdrop gradient |
| Sand | `stadium_bowl.ts`'s bowl material | The arcade arches, resting borders |
| Deep stone | `stadium_props.ts`'s brazier body | Disabled and recessed surfaces |
| Amber | `stadium_bowl.ts`'s `FASCIA_TRIM_COLOR` | The primary accent: selection, eyebrows, primary actions, MVP |
| Flame | `stadium_props.ts`'s brazier flame | Highlights on an amber surface |
| Cyan | unchanged from the previous theme | **Focus and navigation only** |
| Marble | — | Titles and primary copy |
| Panel / Raised Panel | — | Information cards and interactive controls |

Two rules that are not taste:

- **Amber leads, because fire is what lights the arena.** It carries selection
  and emphasis. It replaced cyan in that role when the coliseum landed.
- **Cyan is focus and navigation, and nothing else.** It used to double as a
  broadcast label colour, which made "where am I" and "what is chosen" the same
  signal. Do not reach for it as decoration.

Selected and focused states also change borders, fill, and markers; color is
never the only signal. Species accents come from `gc_data::species`
(`rust/crates/gc-data/src/species.rs`). Team
colors remain content data and should be reserved for team ownership, not
general navigation.

## Backdrop

`draw.ts`'s `drawBackdrop` is the coliseum seen from the pitch: a banded sky
gradient, a row of arches along the upper bowl with an amber fascia line under
them, and brazier light pooling on the arena floor ellipse. It replaced a
starfield of thirteen hardcoded stars and two nebula ellipses that predated the
building. The stadium still drifts in space, so the sky stays — it just stopped
being the whole backdrop.

`GraphicsBackend` has no gradient primitive, so the sky is quantized into
horizontal bands and radial pools are nested ellipses of decreasing alpha. Both
are approximations on purpose; adding a gradient primitive to the backend for
menu chrome is not worth the interface.

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

Panels use a dark fill, a restrained sand border, and a 6px corner radius.
Cards add a five-pixel identity rail; species cards also use a geometric
marker so identity survives grayscale viewing.

Buttons have four deliberate states:

- Resting: raised panel with a visible sand border.
- Focused: cyan border, brighter fill, and a left chevron.
- Selected: amber border, persistent amber-washed fill, and a selection marker.
- Disabled: muted fill and text, removed from focus and pointer activation.

Focus and selection are separate concepts, and they are separate colours for
exactly that reason. Moving focus never changes a formation, tactic, or team
sheet until the player confirms.

## Motion

Routes use a 180ms left-to-right broadcast reveal implemented by the shared
renderer. Screen reducers remain pure and know nothing about animation.
Motion never delays input, changes hit boxes, or carries gameplay meaning.

Avoid looping menu animation, parallax that competes with copy, camera shake
outside the match, and transition sequences longer than 250ms. A future
reduced-motion setting may set the reveal progress directly to complete
without changing route behavior.

## Screen-specific accents

- **Title**: strongest hero type and most open negative space. Four entries,
  not seven — see `ts/packages/screens/src/title.ts`'s header for what went and
  why.
- **Team sheet**: three columns — the five (species rail, geometric identity,
  role, five stats), the shape (one permanent pitch preview of the *chosen*
  formation, with its authored strength and risk), the plan (tactics in plain
  language, no raw tuning values, plus the combat toggle). One confirm, and a
  footer that restates the whole decision before it is committed.
- **Multiplayer**: two panels, host and join, and one honest sentence saying
  this is peer-to-peer with no matchmaking.
- **Lobby**: seats grouped into two team columns, not one flat list of eight.
  Signalling blobs are never rendered — an invite code carrying the model's own
  digest is the whole surface, and copy/paste happens behind it.
- **Result**: score first, trustworthy stats second, amber MVP card third. One
  screen for both contexts; a flag swaps the footer's middle action.
- **Session ended**: a plain-language headline, one sentence of consequence, the
  typed reason in a detail strip, and both exits one keypress away.
- **Help, Settings, Pause, Credits**: the same panels and focus states with
  lower information density. Credits is reached from Settings → About.

The Helios Crown broadcast presentation carries the same language into play:
compact broadcast rails, amber emphasis with cyan reserved for focus,
geometry-backed status, and dark raised panels that remain legible over the
arena.
