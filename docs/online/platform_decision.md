# OMP-0 platform decision

> **Pre-port record (LÖVE/Lua), kept as history.** Everything below was written
> against the Lua tree on LÖVE that commit `2c0d449` (#467) deleted when the
> Rust + TypeScript port reached parity. Its file paths, module names, commands
> and measurements describe that tree: they are accurate for the work they
> record and **name nothing you can open or run today**. The live tree is
> `rust/crates/gc-*` and `ts/packages/*` — see `ARCHITECTURE.md`.

Status: **accepted for the current scope** on 2026-07-19.

## Decision

Galactic Cup will proceed **browser-first for online development** while
preserving native LÖVE execution on Linux.

- The browser artifact and JavaScript–Lua WebRTC boundary are the primary
  online client path.
- `love .` remains the native Linux development and compatibility path.
- A self-contained Linux download is planned in
  [issue #31](https://github.com/osobytes/goliseo/issues/31), but it does
  not block gameplay development.
- Windows browser certification remains in
  [issue #30](https://github.com/osobytes/goliseo/issues/30). Windows and
  macOS native packages are deferred until after the Linux package is proven.

The fixed acceptance matrix in
[`omp0_acceptance.md`](omp0_acceptance.md) remains the historical evidence
contract. Its full cross-platform result is still inconclusive because
Windows 11, physical standard-gamepad A/B, and Firefox JavaScript heap
evidence are unavailable. The owner explicitly accepted those items as
deferred support expansion rather than treating them as passes. No Windows or
macOS support is claimed by this decision.

## Current delivery matrix

| Path | Current status | Intended use |
| --- | --- | --- |
| Linux with LÖVE 11.5 | Runnable from source with `love .` | Native development and compatibility fallback |
| Linux Chrome | Automated product flow, pacing, persistence, letterboxing, stability, audio, and Chrome heap evidence pass | Primary browser development path |
| Linux Firefox | Automated product flow, pacing, persistence, letterboxing, stability, and audio evidence pass | Maintained cross-engine browser path; heap and physical gamepad remain unverified |
| Self-contained Linux package | Not implemented | Future downloadable native build tracked by #31 |
| Windows browser | Not verified or supported yet | Deferred attended campaign tracked by #30 |
| Windows/macOS native package | Not implemented or supported yet | Future OS-specific work after Linux packaging |

Browser compatibility and a downloadable native build are separate
deliverables. Keeping `love .` and the browser artifact runnable preserves both
options without requiring every operating-system package before game
development continues.

## Evidence inventory

| Area | Evidence | Status |
| --- | --- | --- |
| Historical criteria | [Issue #1](https://github.com/osobytes/goliseo/issues/1), [`omp0_acceptance.md`](omp0_acceptance.md) | Fixed before implementation; thresholds unchanged |
| Browser artifact | [PR #8](https://github.com/osobytes/goliseo/pull/8), [`browser_build.md`](browser_build.md) | Reproducible and pinned |
| Browser matrix | [Issue #16](https://github.com/osobytes/goliseo/issues/16), [`browser_compatibility.md`](browser_compatibility.md) | Linux automated gates pass; broader certification deferred |
| Linux remediations | [#20 persistence](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-20-evidence-d2b175b), [#21 pacing/input](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-21-evidence-d7fc8cf), [#22 Chrome heap](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-22-evidence-dab866b), [#24 letterboxing](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-24-evidence-5813c53) | Pass |
| Corrected audio | [Focused Chrome 151/Firefox 152 source `c451727`](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-16-pr29-final-c451727) | Pass on Linux; audio scope only |
| Transport seam | [PR #10](https://github.com/osobytes/goliseo/pull/10), [`transport_bridge.md`](transport_bridge.md) | Bounded asynchronous contract available |
| WebRTC proof | [Issue #5](https://github.com/osobytes/goliseo/issues/5), [PR #14](https://github.com/osobytes/goliseo/pull/14), [`webrtc_input_proof.md`](webrtc_input_proof.md) | Both 10-minute network profiles pass |
| Native runtime | `README.md`, native LÖVE test and simulation commands | Maintained on the Linux development machine |
| Native/browser comparison | [Issue #3](https://github.com/osobytes/goliseo/issues/3) | Complete side-by-side product-flow comparison is missing and owner-deferred |

## Historical criterion evaluation

| Criterion | Current evidence | Historical result |
| --- | --- | --- |
| Artifact boot and product flow | Linux Chrome 150/Firefox 152 remediation packets boot and complete Title → Result at all required viewports | Pass on Linux; Windows unavailable |
| Update/draw/frame/input | All six Linux Chrome 150/Firefox 152 remediation rows pass unchanged thresholds; long rows retain stability/liveness | Pass on Linux; Windows unavailable |
| Console and lifecycle | Clean page runtime, terminal health, focus recovery, fullscreen, keyboard, and clean Result | Pass on Linux |
| Persistence and letterboxing | Reload/populate, recoverable storage failure, tall/wide geometry, and pointer mapping pass in Chrome/Firefox | Pass on Linux |
| Audio and gamepad | Focused source `c451727` passes Chrome 151/Firefox 152 audio; physical standard-mapped A/B is unavailable | Audio passes; gamepad deferred |
| Memory | Authoritative Chrome 150 post-GC heap -0.27%; Firefox RSS is supplemental and Firefox JS heap is unavailable | Chrome passes; Firefox heap deferred |
| Transport | Both fixed issue #5 profiles pass bounded queue/input/latency requirements | Pass |
| Native comparison | The native runtime boots and is tested, but the complete side-by-side issue #3 product-flow report was not captured | Incomplete; owner-deferred |
| Cross-platform matrix | Windows Chrome/Firefox are unavailable; macOS was optional in the historical contract | Inconclusive; deferred rather than passed |

The 600-second claims retain the browser versions and sources of the full
remediation packets: Chrome 150 and Firefox 152, including pacing source
`d7fc8cf` and the separate authoritative #22 Chrome heap packet. The later
Chrome 151/Firefox 152 source `c451727` is a focused zero-stability audio probe
and is cited only for audio.

## Accepted limitations and deferred work

- The current support scope is Linux. Windows and macOS are neither tested nor
  advertised as supported.
- Physical standard-gamepad browser evidence and Firefox JavaScript heap
  evidence are still missing. They must be collected before broadening the
  browser support claim.
- The pinned browser runtime still requires a local-spike CSP containing
  `unsafe-eval`; that is not a production security decision.
- The native path is currently a source checkout plus LÖVE 11.5, not a
  downloadable standalone package.
- The historical native/browser side-by-side product-flow comparison is
  incomplete. A runnable native fallback is not presented as equivalent
  performance evidence.
- A native downloaded client does not yet have a proven internet transport.
  The transport-neutral Lua boundary preserves that option without claiming
  it is complete.
- Rollback, signaling, host topology, and production STUN/TURN remain
  unproven.

## Assumptions for later work

- OMP-1 may proceed with deterministic simulation and the transport-neutral
  input seam. It must keep the existing native Linux single-player path and
  browser artifact runnable.
- OMP-2 may reuse the issue #5 envelope and diagnostics while remaining
  transport-independent.
- OMP-3 may use the browser WebRTC path as its primary online integration
  target. It must not assume Windows/macOS support, native/browser cross-play,
  or a packaged native online client.
- Linux packaging issue #31 and Windows validation issue #30 do not block
  gameplay or deterministic-netcode development. They become release gates
  only when their respective distribution targets enter scope.

Reconsider the delivery choice if the maintained Linux browser path develops a
reproducible hard blocker, a native online transport is proven, or the project
adds a Windows or macOS support target.
