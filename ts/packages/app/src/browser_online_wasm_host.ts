// Browser-target `@gc/wasm` loader for the ONLINE session coordinator and
// match driver -- the online counterpart of `browser_sim_host.ts`'s header
// (same reason: `@gc/wasm`'s Node-shaped `loadSimHost` reads its artifact
// with `require`/`node:fs`, neither of which exists in a browser).
//
// Deliberately shares `browser_sim_host.ts`'s own `init()` promise
// (`ensureBrowserSimHostReady`) rather than starting a second one. Two
// reasons, not one:
//
// - `Session`'s (and, by the same registry mechanism, a live
//   `MatchDriverBridge`'s) internal handle is only meaningful against the
//   `WebAssembly.Instance` that created it (`browser_sim_host.ts`'s own
//   header) -- a second, independently-initialized module instance would
//   silently produce handles neither loader could use interchangeably.
// - ES module resolution already caches `@gc/wasm/web` by specifier, so a
//   second `import * as gcWasmWeb from "@gc/wasm/web"` elsewhere in the same
//   page reaches the SAME module object, not a second instance -- there is
//   only ever one to initialize, and `ensureBrowserSimHostReady`'s own
//   cached promise is that initialization's single source of truth.
//
// Implements `online_wasm_host.ts`'s `OnlineWasmHost` -- see that file's
// header for why the interface is declared independently of this loader.

import * as gcWasmWeb from "@gc/wasm/web";
import { ensureBrowserSimHostReady } from "./browser_sim_host.ts";
import type {
  OnlineMatchDriverHandle,
  OnlineMatchMode,
  OnlineWasmHost,
} from "./online_wasm_host.ts";

/** Resolves once the browser wasm module (shared with the offline path) has
 * been fetched and instantiated. Callers MUST await this once before
 * {@link loadOnlineWasmHost} -- see this file's header. Idempotent. */
export const ensureOnlineWasmReady = ensureBrowserSimHostReady;

/**
 * Wraps the loaded `@gc/wasm/web` module as an {@link OnlineWasmHost}. Call
 * only after {@link ensureOnlineWasmReady} has resolved.
 */
export function loadOnlineWasmHost(): OnlineWasmHost {
  return {
    // `MatchDriverBridge`'s boundary cast is load-bearing (the real
    // wasm-bindgen constructor is a strict superset of `online_wasm_host.ts`'s
    // narrow, hand-declared interface -- see that file's header); `Coordinator`/
    // `Session` already satisfy their narrow interfaces structurally, so an
    // extra cast there is dead weight the lint below correctly flags.
    Coordinator: gcWasmWeb.Coordinator,
    Session: gcWasmWeb.Session,
    MatchDriverBridge:
      gcWasmWeb.MatchDriverBridge as unknown as OnlineWasmHost["MatchDriverBridge"],
    matchDriverFixtureManifestJson: (mode: OnlineMatchMode): string =>
      gcWasmWeb.matchDriverFixtureManifestJson(mode),
    inputFrameNewSample: (moveX: number, moveY: number, held: number, edges: number): string =>
      gcWasmWeb.inputFrameNewSample(moveX, moveY, held, edges),
    // Mirrors `@gc/wasm`'s own `SimHost.buildMatchDriverRenderFrame`
    // (`src/index.ts`) exactly, over the browser-target raw exports instead
    // of the Node ones -- see `browser_sim_host.ts`'s `frame()` for the
    // identical pattern on the offline (`SimSession`) side.
    buildMatchDriverRenderFrame(
      bridge: OnlineMatchDriverHandle,
      kickFollowSlots: number,
    ): Float64Array {
      // `bridge` is always the concrete class `Coordinator`/`MatchDriverBridge`
      // above constructed -- this cast documents that this loader's own
      // `OnlineMatchDriverHandle` is a structural narrowing of the real
      // wasm-bindgen class, not a different object.
      (bridge as unknown as InstanceType<typeof gcWasmWeb.MatchDriverBridge>).renderFrameBuild(
        kickFollowSlots,
      );
      const raw = gcWasmWeb.__getRawExports();
      const ptr = raw.driver_render_frame_ptr();
      const len = raw.driver_render_frame_len();
      // Fresh every call, never cached -- `raw.memory.buffer` is replaced
      // wholesale whenever wasm linear memory grows (`browser_sim_host.ts`'s
      // identical caveat on its own render-frame view).
      return new Float64Array(raw.memory.buffer, ptr, len);
    },
  };
}
