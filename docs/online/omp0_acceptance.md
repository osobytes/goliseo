# OMP-0 acceptance matrix

> **Pre-port record (LÖVE/Lua), kept as history.** Everything below was written
> against the Lua tree on LÖVE that commit `2c0d449` (#467) deleted when the
> Rust + TypeScript port reached parity. Its file paths, module names, commands
> and measurements describe that tree: they are accurate for the work they
> record and **name nothing you can open or run today**. The live tree is
> `rust/crates/gc-*` and `ts/packages/*` — see `ARCHITECTURE.md`.

Status: baseline criteria fixed before browser implementation.

This file remains the historical predeclared OMP-0 contract. On 2026-07-19 the
repository owner accepted a narrower current support scope so gameplay work
could proceed: browser-first for online development, native LÖVE on Linux as
the maintained fallback, and Windows/macOS support deferred. That decision
does not turn missing rows into passes or change the thresholds below; see
[`platform_decision.md`](platform_decision.md).

OMP-0 answers one question: can the existing Galactic Cup product run as a
desktop browser client and carry the input traffic needed for a later
multiplayer slice? It does not prove rollback, production rooms, signaling,
eight-player synchronization, or mobile support.

The rules in this document are fixed before issues #2–#5 produce evidence.
The final decision in `platform_decision.md` must evaluate these rules without
changing a threshold to fit an observed result.

## Decision vocabulary

- **Hard blocker** — a failure that prevents the browser proof from being a
  viable client path. One reproducible hard-blocker failure in a required
  environment is enough to prevent `browser-first`.
- **Soft target** — evidence that improves confidence but may be accepted as a
  documented limitation of the spike. Soft-target misses count in the decision
  score and must have a follow-up or explicit owner.
- **Required environment** — a platform/browser pair that must be tested for a
  decision. An unavailable required environment makes the result
  `inconclusive`; it is not silently treated as a pass.

## Test matrix

### Required product environments

| ID | Operating system | Browser/runtime | Purpose | Required for decision |
| --- | --- | --- | --- | --- |
| L-C | Primary Linux desktop (Zorin OS 18.1 / Ubuntu 24.04-compatible) | Current stable Chromium-based browser | Primary browser artifact, graphics, input, and WebRTC proof | Yes |
| L-F | Same Linux desktop | Current stable Firefox | Cross-engine browser compatibility | Yes |
| W-C | Windows 11 desktop | Current stable Chromium-based browser | Windows packaging and browser compatibility | Yes |
| W-F | Windows 11 desktop | Current stable Firefox | Cross-engine Windows compatibility | Yes |
| M-C | macOS desktop | Current stable Chromium-based browser | Additional confidence only; not a release gate for OMP-0 | No |

“Current stable” means the version installed on the day of the run and is
recorded with the report. Mobile browsers, Safari, touch input, and browser
extensions are outside this spike unless a result exposes a specific blocker.

Each required environment runs at 960×540, 1280×720, and 1920×1080 browser
viewport sizes. The first size is the internal game coordinate system; the
larger sizes exercise scaling and letterboxing. Tests use a clean browser
profile with hardware acceleration enabled and no extensions.

### Native comparison baseline

The native baseline is measured on the primary development machine before the
browser artifact exists. It is a comparison point, not a claim that native
performance automatically makes native the product choice.

Captured 2026-07-17:

| Property | Observation |
| --- | --- |
| OS | Zorin OS 18.1, Ubuntu-compatible desktop |
| CPU | AMD Ryzen 7 5800X, 8 cores / 16 threads |
| GPU | NVIDIA GeForce RTX 2070 SUPER, NVIDIA driver 595.71.05 |
| Runtime | LÖVE 11.5 (Mysterious Mysteries) |
| Window | 960×540, resizable false, vsync 1 |
| Display | 3440×1440 at 74.98 Hz |
| Boot to title | 483–516 ms across 5 launches; p50 509 ms, p95 516 ms |
| Scripted title to kickoff | 491–675 ms across 5 runs; p50 605 ms, p95 675 ms |
| Title RSS | 67,000–83,000 KiB across 5 launches; p50 approximately 67,000 KiB |
| Frame cadence target | 60 Hz product budget: 16.7 ms per frame; native run uses vsync |

The title-to-kickoff measurement sends the same keyboard navigation events a
test harness uses: Play, the existing five-player team sheet, Set the Shape,
and Kick Off. It measures route responsiveness, not a first-time human's
90-second onboarding target. The native frame-cadence observation is recorded
against the 60 Hz budget; issue #3 must add the per-frame update/draw sample
and report it in the same format for native and browser runs.

To repeat the non-visual native checks from a clean checkout:

```sh
love --version
./scripts/check.sh
```

The window-start and scripted navigation timings were collected on the
primary desktop with the native window visible. They are included as a
machine-specific baseline and must not be reused as a cross-machine promise.

## Metrics and thresholds

All measurements use a warm-up period of 10 seconds, then a 60-second sample
unless a row specifies a longer observation. Reports include browser version,
OS, viewport, artifact/runtime revision, sample count, p50, p95, maximum, and
the collection timestamp. A missing metric is a missing result, not zero.

| Metric | Hard gate | Soft target | Collection method | Why it matters |
| --- | --- | --- | --- | --- |
| Artifact boot | Title screen renders from a clean build in every required environment; no uncaught startup exception | First title pixels within 2.0 s of navigation to the served artifact | #2 smoke check plus browser console and screen recording | A browser client that cannot boot is not a delivery path |
| Product flow | Title → squad → formation → tactic → match → result completes once in every required environment | First-time player reaches kickoff in ≤90 s; returning path in ≤30 s | #3 scripted flow plus manual stopwatch; record route and result | Compatibility must cover the actual product, not only a blank canvas |
| Update time | p95 update ≤8 ms and max ≤33 ms at 960×540 during the complete flow | p95 update ≤4 ms | #3 frame instrumentation; collect `love.update` duration | Leaves time for rendering and browser scheduling at 60 Hz |
| Draw time | p95 draw ≤8 ms and max ≤33 ms at 960×540 | p95 draw ≤6 ms | #3 frame instrumentation; collect `love.draw` duration | Protects input and simulation time from rendering cost |
| Frame pacing | No sustained stall: fewer than 3 frames over 33 ms in any 60-second sample and no frame over 250 ms | p95 frame interval ≤16.7 ms | #3 frame timestamps plus browser performance trace | Jitter is visible as control delay even when average FPS looks high |
| Input response | p95 from keyboard/gamepad event to the next visible state or simulation tick ≤100 ms; no lost confirm/back action | p95 ≤50 ms | #3 scripted key/gamepad event timestamps and browser trace | Online play needs local input to feel immediate before networking is added |
| Browser console | No uncaught exception, unhandled rejection, WebAssembly trap, or repeated fatal runtime error | Warnings are classified and non-repeating | Browser console export for each environment | Console failures often precede a stuck or partially working client |
| Runtime stability | No crash, tab kill, unrecoverable WebGL context loss, or dead input during the 10-minute product run | No unexplained warning growth; settings and focus recovery remain usable | #3 run log, console, focus-loss/resize checks | A short demo can hide leaks and lifecycle failures |
| Memory | Browser tab remains alive for 10 minutes and RSS/JS heap has no monotonic growth that exceeds 25% after a forced GC sample | Growth ≤10% after warm-up | Browser task manager/heap snapshots at 0, 5, and 10 minutes | Unbounded growth makes a longer match or session unsafe |
| Tick cadence | Each peer sends and receives 60 tick-numbered input samples per second for 10 minutes; diagnostics remain live | ≥99% of expected samples observed after history recovery | #5 diagnostics, with one row per peer and timestamped counters | This is the traffic shape later rollback will need |
| Input delivery | Reliable control messages are complete and ordered; input-channel gaps are visible and recovered by recent history | ≤1% unrecovered input samples in the 10-minute run | #5 sequence/gap counters and exported summary | Loss must be measurable before it can be compensated for |
| Queue depth | No unbounded queue; queue overflow is never reached in a passing run | p95 queue depth ≤8 messages and max ≤32 | #4/#5 bridge diagnostics sampled once per tick | Backpressure must not freeze the LÖVE update loop |
| Transport latency | Handshake completes; median RTT ≤150 ms and p95 RTT ≤250 ms in the baseline network profile | p95 jitter ≤50 ms | #5 timestamped ping/echo and input diagnostics | A peer proof needs an explicit latency envelope |

The input and transport rows use a 60 Hz tick target. An input sample is a
compact state for one simulation tick; the proof may repeat the most recent
six tick numbers as recovery history, but it must report unique ticks and
retransmitted history separately. The envelope and payload limits are fixed
for this spike at 512 bytes per message and 128 bytes for one input sample.

## Network profile and observation period

The first run uses two browser contexts on the same trusted local network over
HTTPS or localhost with a secure context. The second run uses a repeatable
network shaper with 100 ms round-trip latency and 1% random packet loss. The
shaper profile is not a production quality-of-service guarantee; it exercises
the diagnostic and bounded-queue behavior required by the proof.

The minimum two-client observation is 10 uninterrupted minutes after the
handshake. Each browser context must be refreshed once and repeat the
handshake/runbook successfully. Manual offer/answer exchange is allowed.
Signaling, STUN/TURN selection, room UX, authentication, and public hosting
are not evaluated here.

## Hard blockers versus tolerable limitations

The following are hard blockers:

- the artifact cannot boot or the complete product flow cannot reach Result in
  a required environment;
- any uncaught startup/runtime exception, WebAssembly trap, or unrecoverable
  graphics/input failure during a required run;
- update/draw/frame limits fail in a way that causes the game to miss the
  stated 60 Hz hard gate;
- either peer cannot complete the version/build handshake or cannot sustain
  the 60 Hz diagnostic traffic for the full observation period;
- queues grow without a bound, disconnects are not observable, or malformed /
  unsupported messages are silently accepted;
- the transport proof requires importing JavaScript, WebRTC, or LÖVE APIs into
  `core/`, `data/`, or `sim/`.

The following are tolerable for the spike when reproducible and documented:

- a non-required browser or macOS-specific limitation;
- a classified console warning that does not repeat or affect the flow;
- a soft-target miss while all hard gates pass;
- manual signaling and a local-network-only proof;
- a browser feature that has a native fallback but is not needed by the
  required flow.

Tolerable limitations still need a follow-up issue when they affect the next
  milestone, and they cannot be used to hide a hard-gate failure.

## Evidence ownership

Issue #3 must produce:

- one compatibility table for every required OS/browser/viewport row;
- native-versus-browser update, draw, frame-interval, input, startup, and
  memory measurements using the metric definitions above;
- browser console exports and a classification of every warning/error;
- a run log showing focus loss, resize, fullscreen, audio gesture, gamepad,
  and clean result transition;
- links to any follow-up issue for a reproducible compatibility failure.

Issue #5 must produce:

- host/guest version and build-identity handshake logs;
- the 10-minute tick exchange summary for both network profiles;
- send/receive counts, unique ticks, history retransmits, sequence gaps,
  reordering, RTT, jitter, disconnect reason, and queue depth;
- a second successful run after a clean refresh in both browser contexts;
- a mismatch run proving protocol/build rejection is useful and explicit.

Issue #2 owns the immutable web-runtime revision, artifact hash/package
contents, local serving command, and browser smoke-check evidence. Issue #6
owns the final decision and must link all of these artifacts without changing
this matrix.

## Decision rules for #6

Evaluate the rules in this order:

1. If any required environment or required metric is missing, contradictory,
   or not reproducible, the outcome is **inconclusive**.
2. If the native baseline cannot boot and complete the native product flow,
   the outcome is **inconclusive**; there is no trustworthy fallback to
   compare against.
3. Choose **browser-first** only when every browser hard gate passes in all
   required environments, #5 completes both observation runs, and at least
   80% of the soft targets pass overall with no single required environment
   missing more than two soft targets.
4. Choose **native-download** when evidence is complete, the native baseline
   passes its product-flow gate, and any browser hard gate fails or the browser
   soft-target score is below the browser-first rule.

The fallback is the other client path: `native-download` falls back to a
browser investigation after the named browser blocker is fixed and the full
matrix is rerun; `browser-first` keeps native builds as a compatibility and
diagnostics fallback. The trigger for reconsideration is a reproducible change
in the required runtime, a new browser support target, or a follow-up issue
that removes the named blocker—not a changed interpretation of these rules.
