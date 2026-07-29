# OMP-0 WebRTC input proof

This is the issue #5 spike. It proves the browser transport shape needed by a
later rollback client without putting WebRTC or JavaScript in `core/`, `data/`,
or `sim/`. It does not drive a real match, provide signaling, or select
production STUN/TURN services.

## Status: historical evidence

The game-facing online path no longer uses this proof. Issue #164 replaced it
with the host-star transport in `docs/online/transport_bridge.md`
(`game.transport.browser_star` over `window.GoliseoStarTransport`), which
carries #161 session messages and #162 input packets instead of a
proof-specific vocabulary.

Everything below is retained deliberately as OMP-0 evidence and as the
two-peer measurement rig: `scripts/webrtc_proof_host.js`, the proof and suite
pages, `game/webrtc_proof.lua`, and this runbook stay green and unchanged.
What the host-star transport reuses from it is the *shape* — a reliable
ordered control channel plus an unordered `maxRetransmits: 0` input channel,
manual offer/answer exchange, and per-link queue/gap/drop diagnostics — not
the code path. Latency and jitter percentiles, the deterministic shaper, and
the 60 Hz observation runbook remain here; the host-star bridge deliberately
does not duplicate them.

## Artifact and API

Build and serve the pinned browser artifact in a secure context:

```sh
scripts/web_build.sh build/web
scripts/web_serve.sh build/web 8000
```

The generated `player.js` exposes `window.GoliseoWebRTCProof`. It uses the
version-1 envelope from `game.transport`:

```text
version | type | seq | tick-or-empty | percent-escaped payload
```

The proof host creates two channels:

| Channel | Delivery | Use |
| --- | --- | --- |
| `control` | reliable, ordered | handshake, ping/pong, lifecycle, rejection |
| `input` | unordered, `maxRetransmits: 0` | compact tick-numbered input samples |

The input payload is at most 128 bytes and contains a six-tick recovery
history. The host refuses to add input when its observed `bufferedAmount` would
exceed a 64-message queue budget; drops and queue depth remain visible in
`diagnostics()`.

The pure Lua contract in `game/webrtc_proof.lua` mirrors the handshake, input
limits, mismatch codes, and diagnostic counters for headless tests. The
browser-only implementation lives in `scripts/webrtc_proof_host.js` and is
embedded into `player.js` by `scripts/web_build.py`.

## Browser proof page

The generated artifact includes `webrtc-proof.html`, a browser-visible control
surface that does not require DevTools console mutation. Open a host and guest
tab with the same profile:

```text
http://127.0.0.1:8000/webrtc-proof.html?role=host&profile=baseline&duration_ms=600000
http://127.0.0.1:8000/webrtc-proof.html?role=guest&profile=baseline&duration_ms=600000
```

1. Select **Create offer** in the host tab and copy its local signal into the
   guest tab's remote signal field.
2. Select **Accept offer** in the guest tab and copy its local signal into the
   host tab's remote signal field.
3. Select **Accept answer** in the host tab. Wait for both statuses to show
   `handshake-complete`.
4. Select **Start 60 Hz traffic** in both tabs. The status changes to
   `complete` after the requested duration.
5. Save the diagnostics JSON from both tabs, refresh both tabs, and repeat.

For the second OMP-0 profile, replace `profile=baseline` with
`profile=shaped`. The proof-level shaper applies a deterministic 50 ms
one-way delay to both channels and 1% loss to input messages only, producing
the required 100 ms round-trip/1% loss traffic profile while preserving the
reliable ordered control channel. Diagnostics include the configured profile,
delay, loss, shaper drops, and pending shaped messages. This is a repeatable
spike impairment, not a production quality-of-service guarantee.

The suite page runs both profiles concurrently in four separate nested browser
contexts and includes a build-mismatch pair:

```text
http://127.0.0.1:8000/webrtc-proof-suite.html?duration_ms=600000&run_id=first
```

Select **Connect and run all profiles**, wait for `complete`, and save the suite
diagnostics JSON. Refresh the page, change `run_id=first` to `run_id=refresh`,
and repeat. The individual host/guest pages remain the manual fallback.

## Manual two-context runbook

Open the same served artifact in two clean browser contexts. Keep the console
open in both contexts. The examples below use `host` and `guest` as console
variables; reload both contexts before a repeat.

1. In context A, create the host and export its offer:

   ```js
   host = GoliseoWebRTCProof.create({ role: "host" });
   offer = await host.create_offer();
   copy(JSON.stringify(offer));
   ```

2. In context B, paste the offer, create the guest, and export its answer:

   ```js
   guest = GoliseoWebRTCProof.create({ role: "guest" });
   answer = await guest.accept_offer(JSON.parse(prompt("Paste host offer")));
   copy(JSON.stringify(answer));
   ```

3. Back in context A, paste the answer and wait for both handshakes:

   ```js
   await host.accept_answer(JSON.parse(prompt("Paste guest answer")));
   await Promise.all([host.wait_for_handshake(), guest.wait_for_handshake()]);
   ```

4. Start the fixed 60 Hz proof for the ten-minute OMP-0 observation period in
   both contexts:

   ```js
   host.start_input({ hz: 60, duration_ms: 600000 });
   guest.start_input({ hz: 60, duration_ms: 600000 });
   ```

5. Export diagnostics at the end, then repeat after a clean refresh:

   ```js
   console.table(host.diagnostics());
   copy(JSON.stringify(host.diagnostics(), null, 2));
   console.table(guest.diagnostics());
   copy(JSON.stringify(guest.diagnostics(), null, 2));
   ```

The expected report includes handshake/build identity, role, send and receive
rates, unique ticks, history retransmits, sequence gaps, out-of-order samples,
drops, shaper drops, current and maximum queue depth, RTT p50/p95/max, jitter
p50/p95/max, and disconnect reason. Send/receive rates use only the bounded
traffic window; queue diagnostics include p50, p95, and maximum depth sampled
once per input tick. `GC_WEBRTC|...` console markers provide a line-oriented
capture alongside the JSON diagnostics.

## Mismatch proof

Repeat the offer/answer flow with a deliberately incompatible guest. The host
must report a useful error and must not set `handshake_complete`:

```js
guest = GoliseoWebRTCProof.create({ role: "guest", build_id: "mismatch" });
```

Use `protocol_version: 2` instead to exercise the version rejection. These are
expected failures; do not continue to input traffic after rejection.

## Evidence status

The repository contains the executable proof host, browser control page, pure
contract tests, build smoke assertions, and this runbook. A WebRTC pass is
claimed only after separate browser contexts complete the required 10-minute
baseline and shaped runs plus the clean-refresh repeat. Record browser version,
artifact `manifest.json`, both JSON summaries, console exports, and the
clean-refresh repeat on issue #5 when the run is executed.
