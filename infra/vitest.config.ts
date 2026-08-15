import { defineConfig } from "vitest/config";

// Headless unit tests for the pure modules only (room_state, room_code,
// rate_limiter, turn_credentials) -- AGENTS.md §9 tier 1/2 equivalent for
// this self-contained deploy unit. The Durable Object / WebSocket glue in
// room_durable_object.ts and the Worker entry in index.ts are exercised by
// hand via `wrangler dev` and scripted clients (see docs/online/hosting_runbook.md),
// not by this suite -- a real Workers runtime is out of scope for a "no
// browser, no GPU" headless gate here.
export default defineConfig({
  test: {
    include: ["src/**/*.spec.ts"],
    environment: "node",
  },
});
