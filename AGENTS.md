# AGENTS.md — Galactic Cup engineering constitution

This file is the source of truth for **how we write code** in this project. Humans and
AI agents both follow it. Keep it short, keep it enforced.

Game vision and public product scope live in `docs/`. This file is about
*practices*, not features.

---

## 1. Stack

| Concern        | Choice                          | Notes                                        |
| -------------- | ------------------------------- | -------------------------------------------- |
| Engine         | LÖVE (love2d) 11.5              | 2D, code-first                               |
| Language       | Lua, **LuaJIT / 5.1 semantics** | No 5.2+ syntax (`goto` ok, no `//`, no `<close>`) |
| Type checking  | lua-language-server (LuaLS)     | LuaCATS annotations, strict                  |
| Formatting     | StyLua                          | `stylua.toml` is law, never hand-format      |
| Testing        | LÖVE-native headless runner     | sudo-free; tiers in §9 (logic, UI, flows, visual) |

Run before every commit: `stylua --check .` then `lua-language-server --check .` then `busted`.
A wrapper lives at `./scripts/check.sh` (see §9).

---

## 2. Architecture — four layers, one direction

```
core/   pure utilities (vec2, math helpers) — no love, no game state
  ▲
data/   pure data tables (players, teams, formations, tactics, traits)
  ▲
sim/    pure logic (stats math, xp/leveling, tactic effects, match rules)
  ▲
render/ pure derivation of a drawable frame from simulation state — no love
  ▲
game/   LÖVE-specific: rendering, input, screens, the love.* callbacks
```

**The only allowed dependency direction is upward.** Concretely:

- `core/` may require other `core/` only. No `love`, no game state.
- `data/` requires **nothing**.
- `sim/` may require `core/`, `data/`, and other `sim/`. It must **never** `require("love")` or anything in `game/`.
- `render/` may require `core/`, `data/`, `sim/`, and other `render/`. It must **never**
  `require("love")` or anything in `game/`.
- `game/` may require anything.

Why: `sim/`, `data/` and `render/` stay pure, unit-testable without a window, and portable to
another engine later. If you feel the urge to draw or read input inside `sim/`, the boundary is
wrong — return data and let `game/` act on it.

`render/` is the sim-to-renderer boundary, not a renderer. `render.frame` turns one `MatchState`
into one versioned, flat `RenderFrame` payload; `game/render/` draws that payload with LÖVE and a
future renderer draws the same payload without it. The payload is crossed **once per rendered
frame, in batch** — never per entity, never per tick. Presentation-derived state (gait, lean,
correction smoothing, follow-through windows) is **not** simulation: it stays in `game/render/`
and feeds the builder as an explicit input. `scripts/phase0_sim_host.lua` loads `render/` under a
bare Lua interpreter, so an accidental `love` dependency fails there.

A function is "pure" here if it has no side effects and no I/O: same inputs → same outputs.
All gameplay math lives in pure functions; `game/` is the only place with mutation and effects.
Even inside `game/`, UI screens isolate a pure core (layout / hit-test / state transitions) from
drawing, so the UI is testable without a window — see §9.

---

## 3. Modules

One module per file. A module returns exactly one value (a table or a class).

```lua
-- sim/progression.lua
local progression = {}

---@param player PlayerState
---@param xp integer
---@return PlayerState player  -- same table, mutated
function progression.add_xp(player, xp)
    player.xp = player.xp + xp
    return player
end

return progression
```

- No global writes, ever, except LÖVE callbacks (`love.load`, `love.update`, ...).
  LuaLS will flag stray globals; treat that as an error.
- `require` paths are dotted from project root: `require("sim.progression")`.
- Order inside a file: `require`s → type annotations → constants → the module table →
  functions → `return`.

---

## 4. Classes (when you need state + behavior)

Metatable OOP with LuaCATS annotations. No class library — this pattern, copy it:

```lua
-- game/match/ball.lua
---@class Ball
---@field pos Vec2
---@field vel Vec2
---@field radius number
local Ball = {}
Ball.__index = Ball

---@param pos Vec2
---@return Ball
function Ball.new(pos)
    return setmetatable({
        pos = pos,
        vel = Vec2.new(0, 0),
        radius = 6,
    }, Ball)
end

---@param dt number
function Ball:update(dt)
    self.pos = self.pos:add(self.vel:scale(dt))
end

return Ball
```

- Constructor is `.new`, called with `.` — `Ball.new(...)`.
- Methods use `:` and `self`.
- Private fields are prefixed `_` and not part of the public `---@class`.

---

## 5. Typing rules

We treat LuaLS as a compiler. Untyped public code is a bug.

- Every **module-level table/class** gets a `---@class`.
- Every **public function** gets `---@param` for each arg and `---@return`.
- **Data shapes** are typed too, so data files are checked:

```lua
-- data/players.lua
---@alias Position "keeper"|"defender"|"midfielder"|"forward"

---@class StatBlock
---@field pace integer
---@field strength integer
---@field technique integer
---@field stamina integer
---@field mental integer

---@class PlayerData
---@field id string
---@field name string
---@field number integer
---@field position Position
---@field stats StatBlock
---@field presentation_id string
---@field cosmetic_variant_id string?
---@field loadout_id string?

---@type PlayerData[]
return {
    {
        id = "zyro_vex",
        name = "Zyro Vex",
        number = 9,
        position = "forward",
        stats = { pace = 8, strength = 6, technique = 7, stamina = 5, mental = 2 },
        presentation_id = "medieval_bramble_quickstep",
        cosmetic_variant_id = "bramble_berry",
        loadout_id = "loadout_spring_gloves",
    },
}
```

- Shared types (`Vec2`, `StatBlock`, `Position`, ...) are declared where they're defined and
  reused by name everywhere else. Don't redefine a shape.
- Prefer `---@enum` / `---@alias` over magic strings for closed sets (positions, tactics).
- `nil`-able returns must be annotated `---@return Foo?` and the caller must handle `nil`.

---

## 6. Naming & style

| Thing                     | Convention        | Example              |
| ------------------------- | ----------------- | -------------------- |
| Files / directories       | `snake_case`      | `match_rules.lua`    |
| Locals & functions        | `snake_case`      | `add_xp`, `goal_count` |
| Classes / type names      | `PascalCase`      | `Ball`, `StatBlock`  |
| Constants                 | `UPPER_SNAKE`     | `MAX_STAMINA`        |
| Private members           | `_leading`        | `self._cooldown`     |

StyLua owns whitespace: 4-space indent, double quotes, 100-col width, always parenthesize calls.
Never argue with the formatter; run it.

---

## 7. Errors

Two distinct mechanisms, used deliberately:

- **`assert(cond, msg)`** for *programmer errors / invariants* — things that should be
  impossible if the code is correct (missing required field, bad enum, broken state). Fail loud.
- **`return nil, err_string`** for *expected, recoverable failures* the caller is meant to handle
  (lookup miss, validation of external input). The caller **must** check it.

Never use `error()`/`assert` for normal control flow, and never silently swallow a `nil, err`.

---

## 8. Data is content, code is mechanism

New players, teams, formations, tactics, traits, and arenas are **data edits**, not code edits.
If adding content requires touching `sim/` or `game/`, the system isn't data-driven enough —
flag it. Keep `data/` free of logic (no functions in data tables).

---

## 9. UI: structure & testing

LÖVE has no DOM or accessibility tree, so "UI testing" means testing UI *logic*, not pixels.
We make that cheap by splitting every screen into a pure model and a thin renderer — the same
model/update/view seam that makes a reducer testable.

Each screen module exposes:

- `state` — a plain table (the screen's data).
- `layout(state, viewport) -> Layout` — **pure**. Produces positioned widgets, e.g.
  `{ id = "tactic_press", rect = { x, y, w, h }, text = "Press High" }`. No drawing.
- `update(state, event) -> state, action?` — **pure**. `event` is an abstracted input
  (`{ kind = "click", x, y }`, `{ kind = "key", key = "escape" }`), never a raw love callback.
  Returns the next state and an optional action (e.g. `{ go = "match" }`).
- `draw(state, layout)` — **impure**. The ONLY place `love.graphics` is called.

Hit-testing is a pure helper: `ui.hit(layout, x, y) -> id?`. Raw love callbacks
(`love.mousepressed`, ...) live in the screen stack and do nothing but translate input into
`event`s and dispatch to `update`. Because `layout` / `update` / `hit` touch no graphics and no
globals, they run in the headless test runner with zero display.

### Testing tiers

| Tier | What | Needs display? | When |
| ---- | ---- | -------------- | ---- |
| 1. Logic       | `sim/` math, xp/leveling, tactic effects                              | no  | always |
| 2. UI logic    | layout positions, hit-testing, `update` transitions & emitted actions | no  | always |
| 3. Interaction | drive the real screen stack with a scripted event sequence            | no  | for flows (squad → formation → match) |
| 4. Visual      | render a screen to an offscreen `Canvas`, hash `ImageData`, diff baseline | yes (`xvfb-run`) | opt-in, per pinned screen |

Tiers 1–3 are the contract — write them. Tier 4 is opt-in: it needs a GL context and baseline
images are brittle, so reserve it for screens whose layout we deliberately want pinned. Helpers
live in `spec/support/` (`harness.lua` mounts a screen and dispatches events;
`snapshot.lua` does canvas → hash → baseline compare).

Example UI-logic test (no display):

```lua
local formation = require("game.screens.formation")
local ui = require("game.ui.hit")

local s = formation.new()
local layout = formation.layout(s, { w = 800, h = 600 })
local btn = assert(ui.find(layout, "formation_1_2_1"))
local cx, cy = btn.rect.x + btn.rect.w / 2, btn.rect.y + btn.rect.h / 2
local s2 = formation.update(s, { kind = "click", x = cx, y = cy })
assert(s2.selected == "1-2-1")
```

### A harness self-test is not a harness run

A `--self-test` proves a harness *controller's* own logic — its parsing, its
fixtures. Only starting the harness proves the system it measures. The two are
never interchangeable, and a step is named for the one it actually runs: a heading
that says "fault harness" over a command that starts no harness is how a defect
breaking every online match passed nine green checks (#279). So:

- every gate in `scripts/check.sh` must also appear in `.github/workflows/ci.yml`,
  and vice versa — prefer a shared `scripts/check_*.sh` over hand-mirrored steps;
- every gate must come with a demonstration that it can go red, e.g.
  `scripts/check_fault_harness.sh --self-test`; and
- never trust one signal. A harness that prints failures and exits 0 must fail the
  gate anyway.

`docs/online/fault_harness.md` records the full case.

---

## 10. Workflow

- Format on save (StyLua) or run `stylua .` before committing.
- `./scripts/check.sh` runs format-check + type-check + tests; it must pass before commit.
- Tests go in `spec/` mirroring the source tree (`spec/sim/progression_spec.lua`,
  `spec/screens/formation_spec.lua`). Run them headless with `love . --test`.
- Branch names are `agent/<YYYY-MM-DD>-issue-<number>-<slug>`, e.g.
  `agent/2026-07-25-issue-180-rollback-shards`. Branches predating this rule use a `codex/`
  prefix — leave those alone; use `agent/` for anything new. No tooling keys off the prefix.
- Small, focused commits. Conventional-ish messages: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`.
- One change = one concern. Don't mix a refactor with a feature.
- **Never add a `Co-Authored-By` trailer (or any co-author / "Generated with" line)
  to commit messages.** Commits are authored as the repo owner, full stop.

---

## 11. Agent etiquette

- Read this file and `docs/` before writing code.
- Stay inside the committed scope in `docs/showcase_release.md`. Discuss
  substantial additions before building ahead.
- Respect the layer boundaries in §2 — they're the one rule that's expensive to fix later.
- When unsure about a shape, define the `---@class`/`---@alias` first, then implement.
- Use `scripts/gh-project` instead of the global `gh` CLI for any GitHub operation in this repo
  (issues, PRs, comments, checks). It scopes auth to a profile dedicated to this checkout instead
  of your default `gh` account. Authenticate once with
  `scripts/gh-project auth login --hostname github.com --git-protocol ssh`, then use
  `scripts/gh-project` anywhere you'd otherwise type `gh`.
