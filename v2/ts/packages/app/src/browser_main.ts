// The browser app shell's entry module -- v2/README.md §1's "a running app",
// this batch's primary task. Loads the wasm artifact, constructs the scene
// root against a real `<canvas>`, wires `bootstrap.ts`/`app.ts`/
// `screen_stack.ts` together, and runs the frame loop. Everything this file
// composes already existed in this package or a sibling one BEFORE this
// batch -- see this package's other new files this batch added
// (`canvas_graphics_backend.ts`, `browser_content.ts`, `browser_sim_host.ts`,
// `real_match_factory.ts`) for the pieces that did not.
//
// TWO CANVASES, not one. `@gc/ui`'s menu screens (title, squad, formation,
// tactic, pause, settings, credits, help, result, the fake-match fixture)
// draw through `draw.layout` onto a `GraphicsBackend`
// (`canvas_graphics_backend.ts` -- a Canvas2D implementation); the real
// match screen draws through `@gc/render`'s `SceneRoot`, which owns its own
// `THREE.WebGLRenderer`. Only one is ever visible at a time (toggled by
// `frame()` below on every render, keyed off whether the current screen has
// a menu layout) -- this is simpler and more robust than trying to run
// three.js and Canvas2D on the SAME canvas element.
//
// `bootstrap.new` (not `new App(...)` directly) is what wires
// `matchAdapter.real(realMatchFactory)` -- see bootstrap.ts's header for why
// `real()` needs an injected factory rather than importing
// `game.screens.real_match` itself. Every match this shell plays is
// therefore a REAL match, wasm-simulated -- there is no route to the fake
// adapter from this entry point (it exists purely for tests, see
// match_adapter.ts's header).
//
// KNOWN LIMITATION, not fixed here: `@gc/ui`'s `theme.ts` styling and
// `canvas_graphics_backend.ts`'s plain system-font word-wrap are a first
// pass, not tuned against a reference -- see that file's header. The
// rigged-player offscreen composite's frustum alignment and texture flipY
// orientation (`@gc/render`'s `scene.ts`/`pitch.ts` own "needs a live GL
// context" notes) are exactly the kind of thing this real canvas CAN check;
// see this port's report for what was actually verified versus not.

import * as THREE from "three";
import { SceneRoot, Stadium, camera, pitch } from "@gc/render";
import type { RenderPort } from "@gc/screens";
import { captureGamepad, captureKeyboard } from "@gc/input";
import { ok, err, type Result } from "@gc/core";
import { bootstrap } from "./bootstrap.ts";
import { hit, menuLayout, viewport } from "./ui_bridge.ts";
import { draw } from "@gc/ui";
import type { SettingsStorage } from "./settings.ts";
import { CanvasGraphicsBackend } from "./canvas_graphics_backend.ts";
import { createRealMatchFactory } from "./real_match_factory.ts";
import { createBrowserSimHost, ensureBrowserSimHostReady } from "./browser_sim_host.ts";
import { APP_CONTENT } from "./browser_content.ts";
import { buildInfo } from "./build_info.ts";

// `buildInfo.identity` ("goliseo") is this app's `love.filesystem
// .setIdentity` analog -- see build_info.ts's header. Namespaces every key
// this shell persists, the same way LÖVE's save identity namespaces a whole
// save directory.
const SETTINGS_STORAGE_KEY = `${buildInfo.identity}:settings`;
const MAX_FRAME_DT_SECONDS = 0.25; // clamp after a tab switch/stall, mirrors love's own dt clamp intent

// `sim/match.lua`'s `Match.new`'s own defaults (`opts.duration or 120`,
// `opts.max_goals or match.NO_GOAL_LIMIT` where `match.NO_GOAL_LIMIT == 99`)
// -- `game/screens/real_match.lua` never overrides either, so the product
// match screen always runs at these values.
const MATCH_DURATION_SECONDS = 120;
const MATCH_MAX_GOALS = 99;

// Placeholder team colors until `data/arenas.lua`'s per-team presentation
// palette has a TypeScript-reachable route (v2/README.md rule 6.7 -- see
// `browser_content.ts`'s header for the same content-pipeline gap).
const HOME_COLOR: readonly [number, number, number] = [0.35, 0.75, 1.0];
const AWAY_COLOR: readonly [number, number, number] = [1.0, 0.55, 0.25];

function required<T>(value: T | null, message: string): T {
  if (value === null) {
    throw new Error(message);
  }
  return value;
}

function localStorageSettings(): SettingsStorage {
  return {
    read(): string | undefined {
      try {
        return window.localStorage.getItem(SETTINGS_STORAGE_KEY) ?? undefined;
      } catch {
        return undefined;
      }
    },
    write(contents: string): Result<true, string> {
      try {
        window.localStorage.setItem(SETTINGS_STORAGE_KEY, contents);
        return ok(true);
      } catch (cause) {
        return err(`localStorage settings write failed: ${String(cause)}`);
      }
    },
  };
}

async function main(): Promise<void> {
  const root = required(document.getElementById("app-root"), "browser_main: #app-root is missing");
  const glCanvas = required(document.getElementById("gl-canvas"), "browser_main: #gl-canvas is missing") as HTMLCanvasElement;
  const menuCanvas = required(document.getElementById("menu-canvas"), "browser_main: #menu-canvas is missing") as HTMLCanvasElement;
  const bootStatus = document.getElementById("boot-status");

  const menuCtx = required(menuCanvas.getContext("2d"), "browser_main: 2D context unavailable");
  const menuBackend = new CanvasGraphicsBackend(menuCtx);

  // wasm must finish loading before any real match can be constructed
  // (`browser_sim_host.ts`'s header: calling into the wasm module before
  // `init()` resolves throws) -- block boot on it once, up front, rather
  // than surprising the player with a crash on kickoff.
  await ensureBrowserSimHostReady();

  const glRenderer = new THREE.WebGLRenderer({ canvas: glCanvas, antialias: true });
  const initialViewport = { w: window.innerWidth, h: window.innerHeight };
  const sceneRoot = new SceneRoot(glRenderer, { viewport: initialViewport });

  const keyboard = new captureKeyboard.BrowserKeyboardCapture(window);
  keyboard.attach();
  const gamepad = new captureGamepad.BrowserGamepadCapture(0);

  // The coliseum stadium + true-perspective broadcast camera (the product
  // look). Flags first -- `SceneRoot.render` only routes through the world
  // layer under `camera.perspective_mode`, and `pitch.stadium_mode` hands the
  // backdrop/floor/markings/goals over to the stadium (see scene.ts /
  // pitch.ts). The `Stadium` itself needs the field's real geometry (pitch
  // size, goal rects, crossbar height), which only exists once a match frame
  // arrives -- built lazily on the first `draw` below and reused for every
  // match after it (the field is constant across matches).
  camera.perspective_mode = true;
  pitch.stadium_mode = true;
  let stadium: Stadium | undefined;

  const renderer: RenderPort = {
    draw(frame, _roster) {
      // `frame`/`roster` are `@gc/screens`'s `match.ts`'s deliberately
      // narrow structural types (that file's own header: "only the fields
      // this module reads are declared locally") -- the concrete object
      // underneath is always `@gc/render`'s full `frame_buffer.ts`
      // `RenderFrame`/`DecodedRenderFrameRoster` (`browser_sim_host.ts`'s
      // `frame()`/`roster()` build them via that exact decoder, and embed
      // the roster into `frame.roster` -- `_roster` is unused here for
      // exactly that reason), a strict superset. `SceneRoot.render` needs
      // that superset (`field`/`players`/`ball`/`control`), so this cast
      // documents the gap rather than hiding it -- the same pattern
      // `app.ts`'s `asMenu`/`flow.ts`'s `asScreen` use for an analogous
      // "narrower published type, richer real value" seam.
      const renderFrame = frame as unknown as Parameters<SceneRoot["render"]>[0];
      if (stadium === undefined) {
        stadium = new Stadium({
          field: renderFrame.field,
          home_color: HOME_COLOR,
          away_color: AWAY_COLOR,
        });
        sceneRoot.setWorldLayer(stadium);
      }
      sceneRoot.render(renderFrame, {
        pitch: { home_color: HOME_COLOR, away_color: AWAY_COLOR },
        now: performance.now() / 1000,
      });
    },
  };

  const realMatchFactory = createRealMatchFactory({
    content: APP_CONTENT.matchContract,
    createHost: (request) => () =>
      createBrowserSimHost(
        request.home_team_id,
        request.away_team_id,
        request.seed ?? Math.floor(Date.now() % 1_000_000),
        MATCH_DURATION_SECONDS,
        MATCH_MAX_GOALS,
        // `browser_sim_host.ts` now also accepts `homeFormation`/`tactic`/
        // `homeStarterIds` (`crates/gc-wasm/src/session.rs`'s `Session::new`
        // grew all three this wave), and this is the only real call site --
        // before this, `combatEnabled` was the only field of a request that
        // reached a real session at all. `request.formation_id`/`tactic_id`/
        // `home_starter_ids` were being silently dropped here: a player's
        // squad-select/formation/tactic choices never reached the actual
        // simulated match, which always ran the home team's fixed authored
        // roster at "balanced". `awayTactic` is intentionally omitted --
        // `ProductMatchRequest` carries no away-side tactic field, matching
        // the Lua original (`game.match_contract`'s own request shape).
        {
          combatEnabled: request.combat_enabled,
          homeFormation: request.formation_id,
          tactic: request.tactic_id,
          homeStarterIds: request.home_starter_ids,
        },
      ),
    renderer,
    keyboard,
    gamepad,
  });

  const app = bootstrap.new(APP_CONTENT, realMatchFactory, initialViewport.w, initialViewport.h, {
    settingsStorage: localStorageSettings(),
    requestQuit: () => {
      // A browser tab cannot reliably self-close outside a script-opened
      // window -- nothing better to do here than log.
      console.info("[goliseo] quit requested (no-op in a browser tab)");
    },
  });

  // Dev-only inspection hook (stripped from a production `vite build` along
  // with every other `import.meta.env.DEV`-gated branch): lets a driver
  // script read live app/screen state (e.g. to compute an exact widget
  // click target from `app.transform` + the current layout) without
  // guessing pixel coordinates externally. Not part of the app's own
  // control flow -- nothing above or below this line reads it back.
  if (import.meta.env.DEV) {
    const devWindow = window as unknown as {
      __gcApp?: typeof app;
      __gcClickWidget?: (id: string) => boolean;
    };
    devWindow.__gcApp = app;
    // Drives a widget click through the exact same `app.event` seam a real
    // pointer event does (see the click listener below), but computed from
    // the CURRENT live layout instead of externally guessed pixel
    // coordinates -- for a driver script to click through screens reliably.
    devWindow.__gcClickWidget = (id: string): boolean => {
      const layout = menuLayout(app.stack.current());
      if (layout === undefined) {
        return false;
      }
      const widget = hit.find(layout, id);
      if (!widget?.rect) {
        return false;
      }
      const [x, y] = viewport.toActual(app.transform, widget.rect.x + widget.rect.w / 2, widget.rect.y + widget.rect.h / 2);
      app.event({ kind: "click", x, y, button: 1 });
      return true;
    };
  }

  window.addEventListener("blur", () => app.focus(false));
  window.addEventListener("focus", () => app.focus(true));
  window.addEventListener("gamepaddisconnected", () => app.controllerRemoved());

  function resize(): void {
    const w = window.innerWidth;
    const h = window.innerHeight;
    glCanvas.width = w;
    glCanvas.height = h;
    menuCanvas.width = w;
    menuCanvas.height = h;
    sceneRoot.resize({ w, h });
    app.resize(w, h);
  }
  window.addEventListener("resize", resize);
  resize();

  root.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) {
      return;
    }
    const rect = root.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    app.event({ kind: "click", x, y, button: 1 });
  });

  if (bootStatus) {
    bootStatus.remove();
  }

  let showingGl = false;
  function setLayerVisibility(gl: boolean): void {
    if (gl === showingGl) {
      return;
    }
    showingGl = gl;
    glCanvas.style.display = gl ? "block" : "none";
    menuCanvas.style.display = gl ? "none" : "block";
  }

  let lastFrameTime = performance.now();
  function frame(now: number): void {
    const dt = Math.min(Math.max((now - lastFrameTime) / 1000, 0), MAX_FRAME_DT_SECONDS);
    lastFrameTime = now;

    for (const evt of keyboard.drainKeyEvents()) {
      app.event(evt);
    }
    gamepad.poll();
    for (const evt of gamepad.drainGamepadEvents()) {
      app.event(evt);
    }

    app.update(dt);

    const layout = menuLayout(app.stack.current());
    if (layout !== undefined) {
      setLayerVisibility(false);
      menuCtx.clearRect(0, 0, menuCanvas.width, menuCanvas.height);
      draw.layout(menuBackend, layout, app.viewport);
    } else {
      setLayerVisibility(true);
      app.stack.current()?.draw?.();
    }

    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
}

main().catch((cause: unknown) => {
  console.error("[goliseo] failed to boot:", cause);
  const bootStatus = document.getElementById("boot-status");
  if (bootStatus) {
    bootStatus.textContent = "Failed to load. See console for details.";
  }
});
