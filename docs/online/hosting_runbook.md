# Hosting runbook: Cloudflare Workers

#551. `infra/` deploys the built game (`ts/dist-app/`) plus two tiny server
pieces the online path needs -- room-code signaling and short-lived TURN
credentials -- to Cloudflare Workers. This is the owner's one-time setup and
the day-to-day deploy shape; the engineering rationale (why Cloudflare, why a
Durable Object does not contradict the OMP-4 relay decision) lives in the
issue and in `docs/online/relay_topology_decision.md`.

`infra/` is deliberately self-contained: its own `package.json`, its own
`tsconfig.json`, never imported by any game package, and its own
`wrangler.jsonc` at `infra/wrangler.jsonc`.

## What ships

- **Static hosting.** `wrangler.jsonc`'s `assets` block serves
  `ts/dist-app/` (the same build `scripts/check.sh`'s `gate_app_bundle`
  stage verifies byte-for-byte). `ts/public/_headers` sets `.wasm`'s
  `Content-Type: application/wasm` explicitly.
- **Room-code signaling.** `RoomDurableObject`
  (`infra/src/room_durable_object.ts`), one instance per room code. A host
  opens a WebSocket at `/signal/host` and gets a short, human-friendly,
  single-use code; a guest opens one at `/signal/join?code=…`. Both routes
  are guarded by the `SIGNAL_RATE_LIMITER` binding (`wrangler.jsonc`'s
  `ratelimits`, 10 requests/60s per client IP), checked in `index.ts`
  **before** either handler addresses a Durable Object -- each attempt
  provisions or addresses a billed DO, and the room's own per-room
  join-attempt limit (`JOIN_RATE_LIMIT` in `room_durable_object.ts`) only
  throttles repeats against one already-known code, not a fresh code
  minted on every call. The DO relays opaque signaling blobs between host
  and guest(s) -- it never parses SDP/ICE content, only the small routing
  envelope a host uses to address a specific guest, and the exact wire
  shape (including a documented body-encoding asymmetry between the two
  directions) is in `room_durable_object.ts`'s module doc. Rooms expire
  (`ROOM_TTL_MS` in `room_state.ts`) and cap at one host plus
  `MAX_GUESTS` (7) guests.
- **TURN credentials.** `GET /api/turn-credentials` calls Cloudflare
  Realtime's `generate-ice-servers` API using the `TURN_KEY_ID` /
  `TURN_API_TOKEN` Worker secrets and returns short-TTL
  `{ iceServers: RTCIceServer[] }`. Guarded by a same-origin check and the
  `TURN_RATE_LIMITER` binding (`wrangler.jsonc`'s `ratelimits`). **With
  either secret unset it returns 404**, and the client is expected to
  degrade to STUN-only -- this exact contract is depended on by the
  client-side issue in this milestone; do not change the status code
  without updating that side too.

## One-time owner setup

One or two required repository secrets, one optional one, and one dashboard
action are everything left to do by hand. Nothing else here is manual --
the Durable Object's `[[migrations]]` block (`wrangler.jsonc`) ships
automatically with the first deploy, exactly like any other config change.

### 1. Add the zone and point nameservers

In the Cloudflare dashboard: **Add a site** → enter the domain → Cloudflare
assigns two nameservers → update them at the domain's registrar. This is
the only step that happens outside Cloudflare, and the only one with
propagation delay (minutes to hours).

### 2. Create the GitHub repository secrets

`.github/workflows/deploy.yml` reads these secret names --
**Settings → Secrets and variables → Actions** on the GitHub repo:

| Secret name              | Required? | What it is                                                     | Where to get it |
| -------------------------- | --------- | --------------------------------------------------------------- | ---------------- |
| `CLOUDFLARE_API_TOKEN`     | yes       | A Workers deploy token                                          | Cloudflare dashboard → **My Profile → API Tokens → Create Token** → template **"Edit Cloudflare Workers"** (or a custom token scoped to Workers Scripts:Edit, Workers Routes:Edit, and Durable Objects:Edit for the target account/zone) |
| `TURN_KEY_ID`              | no        | A Cloudflare Realtime TURN key's ID                              | Cloudflare dashboard → **Realtime → TURN → Create a TURN key** |
| `TURN_API_TOKEN`           | no        | That TURN key's API token                                       | Same screen, shown once at creation -- store it now |
| `CLOUDFLARE_ACCOUNT_ID`    | no*       | The target Cloudflare account's ID                               | Cloudflare dashboard → any zone/Workers overview page, right sidebar |

`CLOUDFLARE_API_TOKEN` is **required**: `deploy.yml`'s first step fails the
workflow immediately, with an explicit message, if it is absent. The
`TURN_KEY_ID` / `TURN_API_TOKEN` pair is **optional at deploy time**: if
either is missing, the workflow logs a warning and deploys anyway --
`/api/turn-credentials` then serves 404 (STUN-only) until both are added.
Once both exist, `deploy.yml` propagates them to the Worker automatically,
via `cloudflare/wrangler-action`'s `secrets:` input (which runs
`wrangler secret put` from the same-named environment variables on every
deploy) -- there is no separate local `wrangler secret put` step to
remember.

`CLOUDFLARE_ACCOUNT_ID` is optional **only if the API token from the row
above is scoped to a single Cloudflare account.** If that token can see
more than one account, `wrangler deploy` resolves the account to deploy to
non-interactively by picking whichever one sorts first alphabetically --
silently, with no error -- which is the wrong account exactly as often as
it is the right one. Do one of the two:

- Create `CLOUDFLARE_API_TOKEN` scoped to one account (the token creation
  screen lets you pick "Specific account" instead of "All accounts"), or
- Add `CLOUDFLARE_ACCOUNT_ID` as a fourth repository secret, which
  `deploy.yml` passes to `wrangler-action` as `accountId` whenever it is
  set.

### 3. First deploy

Push to `main`, or run the `Deploy` workflow manually
(**Actions → Deploy → Run workflow**). This builds `ts/dist-app/` the same
way `scripts/check.sh` does, runs `infra/`'s own typecheck/lint/test gate,
and deploys via `wrangler deploy`. The Durable Object's
`[[migrations]]` entry (`wrangler.jsonc`) is applied by this same deploy --
nothing extra to run for it.

The Worker is now live at its `workers.dev` subdomain
(`https://goliseo-online.<account-subdomain>.workers.dev`, visible in the
Cloudflare dashboard or the deploy workflow's own log).

### 4. Attach the custom domain

Once the zone is active (step 1) and the Worker has deployed at least once
(step 3): Cloudflare dashboard → **Workers & Pages → goliseo-online →
Settings → Domains & Routes → Add → Custom domain** → enter the
apex/subdomain from the added zone. Cloudflare provisions the certificate
and routes that hostname to the Worker; no further wrangler.jsonc change is
needed (custom domains are dashboard/API state, not config-file state).

That's it. `deploy.yml`'s `push: branches: [main]` trigger means every
merge to `main` after this redeploys automatically.

## Local development

```bash
cd infra
pnpm install
pnpm exec wrangler types   # regenerates worker-configuration.d.ts (gitignored)
pnpm exec tsc --noEmit
pnpm exec eslint .
pnpm exec vitest run       # the room state machine's headless unit tests

# Build the game once (from ts/), then serve it locally through the Worker:
cd ../ts && pnpm install && node packages/wasm/scripts/build_web.mjs && pnpm exec vite build
cd ../infra && pnpm exec wrangler dev
```

`wrangler dev` serves `ts/dist-app/` with the same static-asset routing and
`.wasm` content-type as production, and simulates the Durable Object and
rate-limit bindings locally -- no Cloudflare account or login needed for any
of this. Local secrets (for testing the TURN route with real credentials)
go in `infra/.dev.vars` (gitignored, never committed):

```
TURN_KEY_ID=...
TURN_API_TOKEN=...
```

Without a `.dev.vars`, `/api/turn-credentials` returns 404 locally, same as
production before the owner sets the two secrets.

## Residual risk / not yet verified

- **No real Cloudflare deploy has been run.** Everything above is verified
  through `wrangler dev`'s local simulation (scripted WebSocket clients:
  static site + wasm content-type, create-room → code, a second client
  joining that code and exchanging blobs both ways, `/api/turn-credentials`
  404 without secrets) -- never against a live account, per this issue's
  scope. The custom-domain attach and the real TURN credentials response
  shape are unverified until the owner completes the steps above.
- **Host-departure socket cleanup was inconclusive in local dev.**
  `RoomDurableObject.alarm()` closes any sockets still open once a room
  closes (host left, or the room's TTL elapsed) -- the documented
  Durable Objects hibernation pattern. Locally, a guest's own
  client-initiated disconnect (and the server noticing it, notifying the
  host, and rejecting a further join by the same code) all verified
  correctly; whether the server-initiated `close()` call itself is
  delivered promptly to an already-connected client's socket did not
  finish verifying against `wrangler dev`'s simulation in the time
  available. The room's server-side state closes correctly regardless (a
  join attempt against an already-host-departed code is rejected), so this
  does not affect the single-use/no-hijack guarantee -- only how quickly a
  still-open peer's socket visibly closes.
