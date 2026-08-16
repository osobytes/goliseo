# Menu & UX inventory — pre-redesign

> **This describes the shell as it stood on 2026-08-11, before the redesign in
> #562 acted on it.** It is kept as the record of what was counted, not as a
> description of the current tree: the thirteen routes below are now twelve,
> `squad`/`formation`/`tactic` are one `team_sheet` screen, `online_result` is a
> flag on `result`, `multiplayer` and `session_ended` exist, and `flow.ts` is
> deleted. `docs/visual_style.md` and `ARCHITECTURE.md` describe the tree that
> exists now.

- **Status:** inventory; acted on by #562
- **Taken:** 2026-08-11, on top of the Rust + TypeScript cutover (#467)
- **Scope:** every non-match screen the product shell can reach, plus the
  online screens it cannot
- **Related:** `docs/visual_style.md`, `docs/showcase_release.md`,
  `docs/design/goliseo_theme_pivot.md`, `docs/online/lobby.md`

This is a survey of what exists, read from source. It ends with three lists —
discard, keep, upgrade — and a list of the multiplayer screens that do not
exist yet. It deliberately proposes nothing that has not been counted first.

---

## 1. What the shell is made of

Two canvases, never both visible (`browser_main.ts`):

- **Menus** — Canvas2D. Screens describe semantic widgets; `@gc/ui`'s
  `draw.layout` paints them onto a fixed **960×540 virtual canvas**,
  letterboxed into the window. Every menu in the game goes through this one
  function.
- **Match** — three.js / WebGL, `@gc/render`'s `SceneRoot`.

The menu half is a model/view/update seam: pure `layout(state)` and
`update(state, event)` per screen, driven by the `Menu` adapter, routed by
`App` (`app.ts`). Nothing about that structure is a problem — it is the reason
a redesign is cheap. What follows is about *content and count*, not
architecture.

## 2. Screen census

**13 routes**, backed by **10 pure screen definitions** and **3 stateful screen
classes**. "Reachable" means from the shipped browser entry point
(`browser_main.ts`), not from a test.

| # | Route | Module | Reachable | Notes |
|---|---|---|---|---|
| 1 | `title` | `screens/title.ts` | yes | 7 menu entries |
| 2 | `squad` | `screens/squad.ts` | yes | pick 5 of 8 |
| 3 | `formation` | `screens/formation.ts` | yes | shape + preview |
| 4 | `tactic` | `screens/tactic.ts` | yes | one-line identity |
| 5 | `match` | `screens/real_match.ts` → `match.ts` | yes | the 3D screen |
| 6 | `result` | `screens/result.ts` | yes | score, stats, MVP |
| 7 | `pause` | `screens/pause.ts` | yes | 5 entries |
| 8 | `help` | `screens/help.ts` | yes | control reference |
| 9 | `settings` | `screens/settings.ts` | yes | 6 rows |
| 10 | `credits` | `screens/credits.ts` | yes | build info |
| 11 | `lobby` | `screens/online_lobby.ts` → `lobby.ts` | **no — throws** | see §3.1 |
| 12 | `online_match` | `screens/online_match.ts` | **no** | unreachable behind `lobby` |
| 13 | `online_result` | `screens/result.ts` again | **no** | second route, same screen |

Not routed at all: `screens/fake_match.ts` ("PRODUCT FLOW LABORATORY") — a
test fixture behind `matchAdapter.fake()`, which the browser shell never
selects.

So the player-facing count today is **10 screens**, and the online third of the
map is dark.

## 3. Findings

### 3.1 The only multiplayer entry point crashes

`title.ts` offers `ONLINE LOBBY (DEV)`. `App.handleAction` routes it to
`showLobby()`, which opens with:

```ts
if (!this.online) {
  throw new Error("no online ports were injected into this App");
}
```

`browser_main.ts` calls `bootstrap.new(...)` with `settingsStorage` and
`requestQuit` only — no `online`. Across the whole tree, `OnlinePorts` is
constructed in exactly two places: `app/src/*.spec.ts`, and the standalone
`tools/browser_online_match` harness. The product shell has no online wiring.

The lobby *screens* are real and well tested. Nothing joins them to the game.

### 3.2 Dead and duplicated code paths

| Thing | Status |
|---|---|
| `QUIT` on the title screen | No-op. `requestQuit` logs `"quit requested (no-op in a browser tab)"`. #467 dropped native; the browser is the only target. |
| `COMBAT PROTOTYPE` on the title screen | A second copy of `PLAY`. Both call `showSquad()`; the only difference is `session.setCombatEnabled`. |
| `app/src/flow.ts` (`Flow`) | A second router for squad → formation → tactic, duplicating `App`'s own. Exported from the package index; used only by `flow.spec.ts`. |
| `online_result` route | `result.ts` mounted a second time, purely so offline rematch is unreachable. One screen and one context flag would do it. |
| `fake_match.ts` | Test fixture, not a product screen. |

### 3.3 The setup flow costs three screens

`docs/showcase_release.md` sells the loop as "coach for thirty seconds, then
personally execute the plan" — **one setup beat**. The implementation spends
three full-screen route changes on it (squad → formation → tactic), each with
its own title, its own BACK/NEXT footer, and its own transition wipe. The
formation screen already draws a pitch preview; the squad screen already knows
the five starters; the tactic screen already displays the chosen formation as
an eyebrow. They are three views of one decision.

### 3.4 The menus are still in space; the world is not

The 3D world went coliseum (`render/src/stadium.ts`: an ancient Roman bowl,
sandstone and marble, cyan-teal stone and amber tech, twilight nebula sky).
The menu layer did not:

- `@gc/ui`'s `theme.ts` names its colors `void`, `space`, `nebula`.
- `draw.ts`'s `drawBackdrop` paints two nebula ellipses and a starfield.
- `title.ts`'s eyebrow reads `INTERGALACTIC 5v5`.
- `docs/visual_style.md` opens with "an **intergalactic sports broadcast**" and
  names `game/ui/theme.lua` as the source of truth — a file that no longer
  exists after #467.

Note the coliseum is *drifting in space*, so this is a shift of emphasis, not a
deletion: stone and amber lead, cyan stays as the focus/navigation signal, and
the starfield stops being the primary backdrop.

### 3.5 Copy inconsistency

`GOLISEO` is the game's name, but the title screen's eyebrow says
`INTERGALACTIC 5v5` and the tagline says
`PICK THE FIVE • SET THE SHAPE • PLAY THE PLAN` — which is the pre-combat,
pre-coliseum pitch. `docs/vision.md` now describes an arcade **combat**-soccer
crossover.

## 4. Discard / keep / upgrade

### Discard

- `QUIT` from the title screen.
- `COMBAT PROTOTYPE` as a top-level entry (becomes a toggle, see below).
- `app/src/flow.ts` and its export — a dead duplicate router.
- The `online_result` route as a separate route.
- `ONLINE LOBBY (DEV)` as-is: either wire it or remove the crash. It must not
  ship as a button that throws.
- The starfield backdrop as the default menu background.

### Keep (structure is right, content changes)

- The pure `layout`/`update` screen seam and the `Menu` adapter.
- The 960×540 virtual canvas + letterbox.
- `result` — score, stats, MVP is the right shape for a finish.
- `help` reachable from both title and pause.
- `settings`' six rows, plus its live-apply behavior.
- The lobby *model* (`lobby_model.ts`) — see §5; it is far ahead of its screen.

### Upgrade

| Upgrade | From → to |
|---|---|
| **Setup flow** | 3 routes → 1 "team sheet" screen with three panels (five / shape / plan), one confirm. |
| **Title screen** | 7 entries → 4: `PLAY`, `MULTIPLAYER`, `HOW TO PLAY`, `SETTINGS`. Combat becomes a match option, credits fold into settings/about. |
| **Theme** | Space tokens → coliseum tokens (stone, sand, amber; cyan retained for focus). New backdrop to match the arena. |
| **Copy** | Retire `INTERGALACTIC 5v5`; align the tagline with the combat-soccer crossover in `docs/vision.md`. |
| **`visual_style.md`** | Rewrite: it names a deleted Lua file as the source of truth. |
| **Result screen** | One screen serving both offline and online finishes via context. |

Route count if all of the above lands: **13 → 9** (title, setup, match, result,
pause, help, settings, multiplayer, online match), with online reachable for
the first time.

## 5. Multiplayer: what exists vs. what is missing

The lobby **model** (`screens/lobby_model.ts`, 1309 lines, heavily tested)
already covers far more than its one dev screen shows:

- host / guest roles;
- `1v1` / `2v2` / `4v4` match modes with per-mode manifest shapes;
- per-slot preference ("TAKE") and seat swapping;
- bot fill for empty slots;
- protected AI keepers per team;
- invite links and clipboard copy/paste of signal blobs;
- ready state, lock, and a tick countdown to a shared start boundary;
- typed terminal / departure / preference-rejection reasons.

What it is presented through is a single screen titled `ONLINE LOBBY`, whose
own hint line reads `MANUAL SIGNALING: BLOBS ARE EXCHANGED BY HAND.`

**Screens that do not exist yet** (candidates to build now, unplugged):

1. **Multiplayer front door** — Host / Join / (later) Quick match.
2. **Join by link** — paste-a-code landing, and the deep-link arrival state.
3. **Lobby proper** — roster, seats, mode picker, ready, countdown. A product
   redesign of the dev screen, over the model that already exists.
4. **Connecting / handshake** — the state between "join" and "in lobby", which
   today is a status string on the dev screen.
5. **Online result** — rematch, back to lobby, leave.
6. **Session ended** — peer left, desync, transport failure. The model already
   emits typed terminal reasons; nothing renders them as a screen.
7. **Connection readout** — latency, rollback frames. The data exists in the
   rollback lab; there is no player-facing surface.

Because every screen here is a pure `layout`/`update` pair, all seven can be
built and tested headless before any transport is attached — which is the point
of building them before they are plugged in.

## 6. Open questions for the redesign

1. Does the setup flow collapse to one screen, or one screen with three tabs?
   (Affects whether the pitch preview is always on screen.)
2. Is `MULTIPLAYER` a title entry, or does `PLAY` open a mode picker
   (solo / online)?
3. Does the coliseum theme keep a visible nebula sky in menus, or go fully
   interior (torchlight, stone, banners)?
4. Is the menu layer staying on Canvas2D, or should it move to DOM/CSS now that
   the browser is the only target? Canvas2D costs us text layout, accessibility
   and responsive reflow; it buys uniformity with the letterboxed 3D view.
5. Does combat stay a hidden option, or become a visible match setting?
