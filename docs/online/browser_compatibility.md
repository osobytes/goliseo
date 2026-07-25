# OMP-0 browser compatibility report

Status: **accepted for the current Linux development scope**. Stable Linux
Chrome and Firefox pass the automated flow, pacing, keyboard/input,
persistence, and letterboxing gates. Physical gamepad A/B and Firefox
JavaScript heap remain unverified, so this is not broader browser release
certification. Windows 11 is deferred to issue
[#30](https://github.com/osobytes/goliseo/issues/30). Missing evidence is
not treated as a pass.

## Artifact and durable evidence

- Original full-matrix source: `5f8e76cf46ce85f488be7a3ee8e88105cd43ab19`;
  package SHA-256:
  `c939d74873cb49fe8d587c66af9d7363c15580a3523846ee2ea210921c5aaef5`;
  [raw Linux baseline](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-16-evidence-5f8e76c).
- Reviewed Chrome audio/geometry probe:
  [source `806f7a3`](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-16-review-evidence-806f7a3).
- Corrected exact-source Chrome/Firefox audio probes:
  [source `c451727`](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-16-pr29-final-c451727)
  (supersedes the
  [intermediate `ee56d8a` packet](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-16-pr29-ee56d8a)).
- Persistence remediation:
  [#20 evidence](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-20-evidence-d2b175b).
- Letterboxing and pointer remediation:
  [#24 evidence](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-24-evidence-5813c53).
- Authoritative Chrome heap remediation:
  [#22 evidence](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-22-evidence-dab866b).
- Final Linux pacing/input campaign:
  [#21 evidence](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-21-evidence-d7fc8cf),
  clean source `d7fc8cfcd3ebf6bfc8a4ad6e54ed86c2afb1df75`, package
  `3542846f22b64249bdef454ddbfce07d84c9ccbe620435dc68c2bf557f2f8daa`.

Raw evidence remains release assets rather than committed generated output.
Packets contain browser/driver and OS/GPU metadata, served-file hashes,
capabilities, screenshots, console/service logs, memory samples, and summaries.

## Required environment matrix

| Row | 960×540 | 1280×720 | 1920×1080 | Remaining |
| --- | --- | --- | --- | --- |
| Linux Chrome 150 | Flow/pacing/input pass; 600 s stability and Chrome heap pass | Flow/pacing/input pass | Flow/pacing/input pass | Physical gamepad |
| Linux Firefox 152 | Flow/pacing/input pass; 600 s stability pass | Flow/pacing/input pass | Flow/pacing/input pass | Physical gamepad, JS heap |
| Windows 11 Chrome | Unavailable | Unavailable | Unavailable | Attended hardware campaign |
| Windows 11 Firefox | Unavailable | Unavailable | Unavailable | Attended hardware campaign plus manual JS heap |

Every final Linux row used hardware WebGL, completed Title → Result, passed
`web_report.py --require-flow`, retained a clean page-runtime console and
terminal health, and passed the unchanged update/draw/frame/input thresholds.
Both 600-second rows retained late input/settings and Match focus recovery.
Persistence now flushes and reloads `muted=true` in both browsers, while
storage-unavailable boot remains recoverable. Tall/wide canvas geometry and
real pointer hit-testing now pass in both browsers.

The final exact-head pacing campaign's worst complete samples had at most one
frame over 33 ms and none over 250 ms. Whole-row input p95/max remained below
100 ms in all six rows. Chrome's corrected one-document, forced-GC heap
measurement changed from the original apparent leak to
`2,639,224 → 2,632,180` bytes (-0.27%), passing the fixed 25% gate.

## Controls and memory interpretation

The runner observes exactly 11 ordered samples over roughly five seconds after
entering Match. A pass requires an unmuted setting before Match, user
activation, no autoplay warning, positive master volume, and at least one
active source. Malformed, missing, non-finite, duplicate, or out-of-order
samples fail the check without aborting evidence capture.

The full matrix, 600-second stability, and Chrome heap claims above are from
Chrome 150 remediation packets, including clean pacing source `d7fc8cf` and
the authoritative #22 heap packet. They are not attributed to the later
focused audio run.

The positive Chrome packet at source `806f7a3` is historical evidence.
Pre-fix source `4b446ceb` left the persistence probe's `muted=true` setting in
place, so its otherwise complete Chrome and Firefox probes correctly observed
zero sources. Corrected source `c451727` persists `muted=false` before the
product flow; its focused zero-stability Chrome 151 and Firefox 152 packets
each pass with all 11 samples positive, volume 1, user activation, and no
autoplay warning. Those focused packets prove audio only; they do not replace
the Chrome 150 full-matrix, stability, or heap provenance.

No physical standard-mapped controller was available for the Linux packets.
The attended operator must expose `mapping="standard"` and produce both
`gamepad_a` and `gamepad_b`; the ASRock LED controller at `/dev/input/js0` is
not evidence.

Firefox's recorded process-tree RSS is not JavaScript heap.
`performance.memory` is non-standard and Chromium-only, so the required
Firefox t0/t5/t10 heap companion remains manual. The concise procedure and
Mozilla sources are in [`browser_build.md`](browser_build.md).

## Deferred validation

- Issue [#30](https://github.com/osobytes/goliseo/issues/30) retains the
  serialized Windows 11 Chrome/Firefox campaign, physical controller, audible
  playback, and Firefox heap requirements for a later support expansion.
- Linux physical standard-gamepad coverage and Firefox t0/t5/t10 heap evidence
  are still required before making a broader public browser-support claim.
- Issue [#31](https://github.com/osobytes/goliseo/issues/31) tracks a
  self-contained Linux download separately from browser certification.

Issue [#16](https://github.com/osobytes/goliseo/issues/16) completed the
repository-owned evidence tooling and is closed. The owner-accepted delivery
policy and its narrower current support scope are recorded in
[`platform_decision.md`](platform_decision.md).
