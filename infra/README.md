# infra/

The Cloudflare Worker that hosts the built game and the two small server
pieces the online path needs: room-code signaling and TURN credentials.
Self-contained -- its own `package.json`, `tsconfig.json` and test setup;
not part of the `ts/` pnpm workspace, and never imported by any game
package.

- **Owner setup, deploy, and local dev**: see
  `docs/online/hosting_runbook.md`.
- **Design rationale** (why Cloudflare, why a Durable Object here does not
  contradict the OMP-4 relay decision): issue #551 and
  `docs/online/relay_topology_decision.md`.

## Layout

| Path                         | What                                                                                                         |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `wrangler.jsonc`             | Worker config: static assets, the `RoomDurableObject` binding + migration, the TURN rate limiter             |
| `src/index.ts`               | Worker entry: routes `/signal/*` and `/api/turn-credentials`; everything else falls through to static assets |
| `src/room_state.ts`          | The room-code signaling state machine -- pure, unit-tested, no I/O                                           |
| `src/room_durable_object.ts` | The Durable Object: hibernation-safe glue around `room_state.ts`                                             |
| `src/room_code.ts`           | Short human-friendly room code generation/validation                                                         |
| `src/rate_limiter.ts`        | A pure fixed-window limiter, used for room join attempts                                                     |
| `src/turn_credentials.ts`    | Calls Cloudflare Realtime's TURN key API; same-origin check                                                  |
| `*.spec.ts`                  | Headless vitest unit tests for the modules above                                                             |

## Quick start

```bash
pnpm install
pnpm exec wrangler types   # generates worker-configuration.d.ts (gitignored)
pnpm typecheck
pnpm lint
pnpm test
pnpm dev                   # wrangler dev -- needs ts/dist-app/ built first, see the runbook
```
