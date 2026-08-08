# Issues #404 / #413 — visual evidence (PR #410)

All captures: headless Chrome, hardware GL via the repo's own
`CHROME_HARDWARE_GL_ARGS` (`--use-gl=angle --use-angle=gl-egl
--ignore-gpu-blocklist`) on `DISPLAY=:1`, renderer string verified before each
save as `ANGLE (NVIDIA Corporation, NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2,
OpenGL ES 3.2)`. `v2/tools/browser_match_harness`,
`?ratio=1&seed=1&duration=12`, every frame at **tick 720, 0-0**.

## #404 / PR #410 — the fix

- `hw_before.png` / `hw_bloom0_before.png` — `main` @ aaf9ae9 and its own `?bloom=0`
- `hw_after.png`  / `hw_bloom0.png`        — PR #410 and its own `?bloom=0`
- `zoom_hw_before.png`, `zoom_hw_after.png` — 5x crop, same region

## #413 — threshold-domain arms

Three arms differing in ONE thing: which pixels the bright pass selects.
Everything downstream is held identical.

- `arm1.png` — threshold linear `0.55` (what PR #410 ships; correct for the
  renderer as it exists today)
- `arm2.png` — threshold linear `0.2633`, knee `0.1847` — **what the constants
  must become if #413 is resolved by decoding content colours on entry**
- `arm3.png` — threshold the sRGB-encoded value at `0.55`/`0.15`, glow kept
  linear (the same rule as arm2, expressed in the other domain)
- `arm{1,2,3}_bloom0.png` — each arm's own `?bloom=0` baseline
- `zoom_arm{1,2,3}.png` — 5x crop, same region

Light added versus each arm's own `?bloom=0`, mean luminance delta (0-255), in
rings at increasing distance from character pixels. NOISE is two independent
captures of an identical configuration, i.e. the measurement floor:

| ring | NOISE | arm1 | arm2 | arm3 | arm2-arm3 |
| --- | --- | --- | --- | --- | --- |
| 1-4 px | -0.00 | +3.76 | +22.61 | +22.76 | -0.21 |
| 5-10 px | -0.00 | +0.61 | +16.83 | +16.97 | -0.16 |
| 11-20 px | -0.00 | -1.91 | +10.23 | +10.33 | -0.11 |
| 21-40 px | -0.00 | +4.52* | +4.59* | | |

(*) arms 2 and 3 at 21-40 px are +4.52 and +4.59; arm1 is -3.41 there, the MSAA
deficit on the hex-grid lines rather than glow.

Arms 2 and 3 agree within 0.21/255 — they are one rule in two domains, as
predicted. Both add ~6x arm1's light next to a character and are still adding
light at 21-40 px where arm1 has none.

## Supplementary — SwiftShader, NOT hardware

`sw_before.png`, `sw_after.png`, `sw_after_bloom0.png`. Software rasteriser, and
NOT tick-matched to each other (ticks 400 / 447 / 432). Kept only as a
second-rasteriser opinion.

---

Referenced from PR #410 and issue #413 by commit SHA so the pictures a reader
sees are the ones described. **Do not delete this branch** — that would break
the images in the merged PR's history and in #413.
