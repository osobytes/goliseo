import { fileURLToPath } from "node:url";

import { defineConfig } from "vitest/config";

// Headless unit tests. Mostly the pure modules (room_state, room_code,
// rate_limiter, turn_credentials) -- AGENTS.md §9 tier 1/2 equivalent for
// this self-contained deploy unit -- plus index.spec.ts, which exercises
// handleHostSignal/handleJoinSignal's routing and rate-limit-ordering logic
// against fake `Env` bindings (no real Workers runtime). The Durable
// Object / WebSocket glue in room_durable_object.ts itself, and the full
// Worker entry point wired to a real Durable Object, are exercised by hand
// via `wrangler dev` and scripted clients (see
// docs/online/hosting_runbook.md) -- a real Workers runtime for THAT is
// out of scope for a "no browser, no GPU" headless gate here.
export default defineConfig({
  resolve: {
    alias: {
      // See src/test/cloudflare_workers_stub.ts's own doc: index.spec.ts
      // transitively imports room_durable_object.ts's `cloudflare:workers`
      // import, which does not exist outside a real Workers runtime.
      "cloudflare:workers": fileURLToPath(
        new URL("./src/test/cloudflare_workers_stub.ts", import.meta.url),
      ),
    },
  },
  test: {
    include: ["src/**/*.spec.ts"],
    environment: "node",
  },
});
