# Native desktop route decision (#329)

> **Pre-port record (LÖVE/Lua), kept as history.** Everything below was written
> against the Lua tree on LÖVE that commit `2c0d449` (#467) deleted when the
> Rust + TypeScript port reached parity. Its file paths, module names, commands
> and measurements describe that tree: they are accurate for the work they
> record and **name nothing you can open or run today**. The live tree is
> `rust/crates/gc-*` and `ts/packages/*` — see `ARCHITECTURE.md`.

Status: **recommended, pending the #330 migrate-or-optimise decision** — 2026-08-04.

Decides [issue #329](https://github.com/osobytes/goliseo/issues/329). Feeds
[#330](https://github.com/osobytes/goliseo/issues/330). Builds on
[#342](https://github.com/osobytes/goliseo/issues/342)
([`wasm_webview_determinism.md`](../online/wasm_webview_determinism.md)),
[#341](https://github.com/osobytes/goliseo/issues/341)
([`babylon_skinned_benchmark.md`](babylon_skinned_benchmark.md)) and
[#337 slice 2](https://github.com/osobytes/goliseo/issues/337).

## What a player gets out of this

Nothing, today — and that is the point of writing it down now. This decides
which desktop application a player would eventually download, before any of the
work that would be wasted by getting it wrong. The two shippable candidates
differ by a factor of **72 in download size**, and — **on Linux, which is the
only platform this project supports and the only one measured** — by a factor of
**4.4 in how much frame budget the window itself eats**. The third candidate
turns out not to be a shippable product at all yet.

A player on Linux feels the frame cost immediately: on the same scene, the same
GPU and the same 87 draw calls, ten characters cost **1.23 ms** of draw time in
Electron and **5.42 ms** in Tauri.

**That frame number is a WebKitGTK number, not a Tauri number.** Tauri renders
in whatever webview the platform ships: WebKitGTK on Linux, **WebView2 on
Windows — which is Chromium/V8, the same engine as Electron** — and WKWebView on
macOS. Only the Linux arm was measured, and it is plausibly Tauri's worst case.
Every restatement of the 4.4x below carries that scope, and the recommendation
in §4 is built so it does not need the number at all.

## Verdict

**Electron, on Linux — the only platform this project currently supports**
(`docs/online/platform_decision.md`), and the only one measured here. On the one
machine that can measure any of this, Electron renders the #341 benchmark scene
at **1.23 ms draw p50** and **Tauri-on-WebKitGTK** at **5.42 ms — 4.4x more, for
an identical 87 draw calls, on the same GPU, in the same interleaved session** —
and Tauri costs 1.9x the cold start on top.

**Lead on `draw`, not on `frame`.** `draw` is CPU time inside `scene.render()`;
it cannot contain vsync or compositor pacing, so it is immune to the one
confound this comparison could plausibly have had (see
[the pacing control](#the-pacing-control)). `frame` brackets a `gl.finish()` and
the inter-tick gap, so it can. Both are reported; the argument rests on `draw`. Tauri wins only on the Debian package (1.3 MiB against 94.2 MiB), and it
wins that by *not shipping a renderer*, which is the same fact that produces the
frame cost.

**The 4.4x is scoped to WebKitGTK and does not generalise.** On Windows, Tauri's
webview is WebView2 — Chromium and V8, the same engine as Electron — so the
frame-cost gap would likely shrink or vanish there, and it was **not measured**.
The recommendation below is deliberately not built on this number: Electron wins
on engine uniformity, cold start and maturity, all of which hold whatever
WebView2 would have measured.

**Babylon Native is not a candidate on this evidence, and it is not close
either.** It builds, and its capability spike passes every check this game needs
— including `BoneIKController`, the feature Babylon was chosen for in #328. But
its own validation suite **segmentation-faults on this machine after 9 of 720
tests**, and again after 196 more once the first crash is skipped: about 28% of
its own suite runs before the process dies. It publishes **no releases and no
version tags since 2020**, its README calls it "public preview in source form
only" with no backward-compatibility contract, and it does not yet support
**audio** or **particles** at all. It has no packaging story to measure an
installer from.

**That last finding is the one #330 needs.** The 2026-08-04 decision to drop
LÖVE rested on two assumptions. #341 falsified the first. This falsifies the
second: *"Babylon Native works well enough to carry the native build"* is not
true today. See [What this does to #330](#what-this-does-to-330).

## Evidence discipline

This document distinguishes three things and never blurs them:

- **Measured** — run on this machine, on this date, with the command recorded.
- **Read from a primary source** — the upstream repository's own files, API or
  CI configuration, cited by path or endpoint. A README's *claim about
  maturity* is quoted as a claim, not as evidence; a README's *statement of
  what is unsupported* is a primary source about scope, which is different.
- **Inferred** — reasoning over the two above, labelled as such.

Machine for every measured number: Linux 7.0.0-28-generic x86_64, 16 cores,
NVIDIA GeForce RTX 2070 SUPER, driver 595.71.05, `DISPLAY=:1`. Other agents were
building in sibling worktrees throughout; `/proc/loadavg` is recorded with each
measurement because of it.

---

## 1. Babylon Native: what it actually is

### 1.1 Primary sources, read 2026-08-04

All of this is from the GitHub API and from files in
`BabylonJS/BabylonNative@aa244ec98c00660ee1832f68d4fdaa7f2620128e` (committed
2026-08-04T21:27:31Z — the same day this was read).

| Fact | Value | Source |
| --- | --- | --- |
| Licence, age | MIT, created 2019-05-28 | `GET /repos/BabylonJS/BabylonNative` |
| Popularity | 913 stars, 158 forks | same |
| Open issues / PRs | 74 issues, 19 pull requests | `GET /search/issues` |
| Closed issues | 495 | same |
| **GitHub releases** | **0** | `GET /repos/BabylonJS/BabylonNative/releases` |
| **Version tags** | **4, all dated 2020-06-08** (`0.0.1`, `0.0.2`, `0.03`, `0.0.4`) | `GET .../tags`, then each tag's commit date |
| Commits per month, 2025-09 → 2026-08 | 1, 4, 11, 9, 11, 17, 15, 25, 22, 34, 15, 3 | `GET /search/commits` |
| Babylon.js pinned at | `^9.15.0` (`Apps/package.json`) | repository file |
| Linux CI | 4 configurations (Clang/JSC, Clang/Hermes, GCC/JSC, Clang/QuickJS) plus a Linux install test, all `success` on master at this commit | `.github/workflows/ci.yml`, `GET /actions/runs/30952414795/jobs` |

Two of those rows point in opposite directions and both are true. **The project
is alive**: 34 commits in June 2026, a commit landed the day this was read, four
Linux build configurations green in CI, and Babylon.js pinned only four minor
versions behind the 9.19.1 the #341 benchmark uses. **The project is not
shipped**: no release has ever been published and the newest version tag is six
years old. Those are compatible — it is a source-form component, and its README
says so.

Four of the oldest open issues are load-bearing rather than cosmetic:
[#31 Sound support](https://github.com/BabylonJS/BabylonNative/issues/31) has
been open since 2019-08-19, and
[#218 Support texture loaders](https://github.com/BabylonJS/BabylonNative/issues/218),
[#440 The Graphics component is inadequately documented](https://github.com/BabylonJS/BabylonNative/issues/440)
and
[#538 GLTF Loader non-cached HTTP requests](https://github.com/BabylonJS/BabylonNative/issues/538)
since 2020.

### 1.2 What the README says is missing

The README's "Project Status" is a primary source about **scope**, and it is
blunt. Quoting only the load-bearing sentence: Babylon Native is "currently
available as a public preview in source form only", and "this project is not at
the point where updates are fully backward compatible yet, and thus the contract
for consuming Babylon Native can still and probably will change in the future."

Its own feature table, against what this game needs:

| Babylon.js feature | Babylon Native status (README) | Does Goliseo need it? |
| --- | --- | --- |
| 3D assets (glTF), Animations, Materials, Meshes, Lights, Shaders, Textures | Supported | Yes — and measured working in §1.4 |
| **Audio** | **Not yet supported** | **Yes** — `docs/design/sound.md` is a committed design |
| **Particles** | **Not yet supported** | Yes — `docs/visual_style.md` and the combat telegraphs |
| Texture loaders (KTX, DDS) | Not yet supported | Not today; PNG/JPEG is enough |
| Serializers | Not yet supported | No |
| **Input** | **Partially — "only single pointer supported"** | Keyboard and gamepad are the primary input |
| GUI | Partially — "text rendering experimentally supported" | Yes — every screen in `game/screens/` |
| Instancing | Partially — "only thin instances supported" | Probably — it is the #337 optimisation shape |
| Post processing | Partially — "some are supported" | Yes — bloom is in the current renderer |
| Inspector, Node Material Editor, GUI Editor, Performance Profiler | No plan to support | No — those are authoring tools |

Audio and particles being absent is not a rendering detail. A football game with
no crowd and no ball-strike audio is not the product, and the work to add them
is upstream C++ work in someone else's repository, not work this project can
schedule.

### 1.3 Building it — measured

Timeboxed to one afternoon; it did not need the box.

```bash
# PINNED. `git clone --depth 1` takes whatever HEAD is on the day you run it,
# and §1.1 measured upstream landing 15-34 commits a month -- so an unpinned
# clone builds different C++ from the one this document reports on, and a
# reader could not tell a failure to reproduce from a different subject.
# Fetching the SHA directly keeps the clone shallow AND pinned; `--depth 1` on
# `git clone` and a checkout of an arbitrary historical commit are otherwise in
# tension, because the shallow clone does not contain that commit.
git init BabylonNative && cd BabylonNative
git remote add origin https://github.com/BabylonJS/BabylonNative.git
git fetch --depth 1 origin aa244ec98c00660ee1832f68d4fdaa7f2620128e
git checkout FETCH_HEAD
git submodule update --init --recursive   # see the note below: currently a no-op

cmake -G Ninja -B build/Linux \
  -D JAVASCRIPTCORE_LIBRARY=/usr/lib/x86_64-linux-gnu/libjavascriptcoregtk-4.1.so \
  -D NAPI_JAVASCRIPT_ENGINE=JavaScriptCore -D CMAKE_BUILD_TYPE=RelWithDebInfo \
  -D OpenGL_GL_PREFERENCE=GLVND .
ninja -C build/Linux
```

Verified: that sequence lands on `HEAD = aa244ec98c00660ee1832f68d4fdaa7f2620128e`
and costs the same 228 MB as the unpinned clone did.
`git submodule update --init --recursive` is included because a checkout must
re-sync submodules to the commit being built — but **at this commit it is a
no-op**: `git submodule status` is empty, and every dependency (bgfx, glslang,
googletest, JsRuntimeHost, UrlLib, SPIRV-Cross) is fetched by CMake at configure
time instead. Keep the line anyway; it costs nothing and stops being a no-op the
moment upstream adds a submodule.

| Step | Result |
| --- | --- |
| Clone (pinned, shallow) | 228 MB, no git submodules — dependencies are fetched by CMake |
| CMake configure | 30.6 s, fetches bgfx, glslang, googletest, JsRuntimeHost, UrlLib, SPIRV-Cross, and runs `npm install` |
| Build | 1157 targets, **107 s** wall (load average 18–27; other agents were building) |
| `Apps/Playground/Playground` | 63,156,688 bytes (RelWithDebInfo) |
| Release, `-s` stripped | see §3 |

**`BUILDING.md` is stale in one way that stops the build and one that does not.**
Measured, not read:

- It lists `libcurl4-openssl-dev` as required, and the build genuinely needs
  `curl/curl.h` (`Dependencies/UrlLib/Source/UrlRequest_Unix.cpp:3`). But
  `UrlLib` includes it directly instead of going through `find_package(CURL)`,
  so `-D CURL_INCLUDE_DIR` and `-D CURL_LIBRARY` are ignored — CMake reports
  them under "Manually-specified variables were not used by the project" and
  the build then fails with `fatal error: 'curl/curl.h' file not found`. There
  is **no supported way to point the build at a curl outside the system
  include path.** This box has no passwordless sudo, so the package was
  obtained with `apt-get download libcurl4-openssl-dev`, extracted with
  `dpkg-deb -x` into a local prefix, and injected through `CMAKE_CXX_FLAGS` /
  `CMAKE_EXE_LINKER_FLAGS`. That works, and it is a workaround for a missing
  knob rather than a missing package.
- It omits `libwayland-dev`, which `.github/workflows/build-linux.yml` installs.
  This box had it already, so it did not bite here.

### 1.4 The capability spike — measured, and it passes

`bench/babylon_native/spike.js` loads the **same CC0 KayKit Knight** the #341
browser benchmark uses (sha256 `60428e3a…`, pinned in
`scripts/babylon_bench.py`) and exercises the four things a replacement
presentation layer has to do.

```
cp bench/babylon_native/spike.js  <build>/Apps/Playground/Scripts/spike.js
cp character.glb                  <build>/Apps/Playground/Scripts/character.glb
DISPLAY=:1 ./Playground app:///Scripts/spike.js
```

The evidence of record is a **retained five-run batch** against the stripped
`Release` build, on an idle box, not a single quoted transcript. All five runs
returned `status=OK` for all six checks. One run in full, and the across-run
spread underneath it:

```
GC_BN|check=engine|status=OK|babylon=9.15.0|graphics_api=OpenGL|engine_description=Native2 - Parallel shader compilation
GC_BN|check=gltf_load|status=OK|meshes=16|skinned_meshes=6|skeletons=1|bones=41|animation_groups=76|total_vertices=0
GC_BN|check=shadows|status=OK|shadow_map_size=1024|casters=6|detail=shadow map allocated
GC_BN|check=skeletal_animation|status=OK|clip=1H_Melee_Attack_Chop|clips_available=76|frames=30|bones_moved=34|bones_total=41|max_bone_delta=0.49609
GC_BN|check=bone_ik|status=OK|bone=upperarm.l|bone_parent=chest|effector=lowerarm.l|frames_per_pose=30|effector_travel=0.29209|distance_to_target_pose_a=0.48586|distance_to_target_pose_b=1.11977
GC_BN|check=render|status=OK|width=600|height=400|clear_color=0,0,0|lit_threshold=24|lit_pixels=187800|total_pixels=240000|lit_fraction=0.78250|accepted_fraction_window=0.001..0.98|brightest_channel=255|prove_flat_fill=no
GC_BN|check=summary|status=OK|checks=6
```

| across 5 runs | value |
| --- | --- |
| `summary` | `OK` in 5 of 5 |
| `bones_moved` | 34 of 41, in every run |
| `effector_travel` | 0.29209 – 0.29283 |
| `lit_fraction` | 0.78250, identical in every run |

Read that line by line, because it settles the #328 question:

- **glTF loading works.** 41 joints, 6 skinned meshes and all 76 animation
  clips came off the same `.glb` the browser bench loads.
- **Skeletal animation works.** 34 of 41 bones moved over 30 frames of a real
  clip. `max_bone_delta` varies between runs (0.49 here, 0.66 on a loaded box)
  because Babylon advances animation by wall-clock time, so a slower machine
  samples a later pose at the same frame index. The bone *count* does not move.
- **Shadows work.** A 1024 shadow map was allocated with six casters, and the
  scene rendered with it.
- **`BoneIKController` works.** This is the decisive one. The controller
  attached to `upperarm.l`, and when the IK target was moved the end effector
  travelled ~0.292 units to follow it, in all five runs. **Built-in IK is the
  stated reason Babylon was chosen over three.js in #328, and it runs
  natively.** That is not a surprise once you look at the implementation —
  `BoneIKController` is arithmetic over `Bone`, `Matrix` and `Vector3` with no
  DOM and no engine call — but "should work" and "did work" are different
  claims and this is the second one.
- **It drew** — and this line is now worth the paper it is written on, which it
  previously was not. See below.

#### The render check was broken, and what "it drew" now rests on

The first version of this check counted a pixel as drawn if any channel exceeded
8. **Babylon's default `Scene.clearColor` is (0.2, 0.2, 0.3, 1) — R=51, G=51,
B=76 — so every pixel of an empty background already passed.** It scored a blank
frame 240000 / 240000 and reported `status=OK`. It could not go red. An earlier
revision of this document quoted exactly that `240000|240000` and said "It
drew"; that claim was unsupported, and this section replaces it.

The check now pins the background to pure black, counts pixels above 24, and
requires the lit fraction to fall **inside** a window — something must be lit,
and the frame must not be *entirely* lit, because a uniform fill is the
degenerate case the old check accepted. The real scene measures **0.78250**,
identically in all five runs.

The demonstration that it can now go red is `bench/babylon_native/prove_flat_fill.js`,
which reproduces the original bug against a real renderer — it turns the frame
into a uniform fill immediately before capture:

```
$ ./Playground app:///Scripts/spike.js                                    # real scene
GC_BN|check=render|status=OK  |lit_pixels=187800|lit_fraction=0.78250|prove_flat_fill=no
GC_BN|check=summary|status=OK                                              exit 0

$ ./Playground app:///Scripts/prove_flat_fill.js app:///Scripts/spike.js   # falsified
GC_BN|check=prove_flat_fill|status=ARMED|expect=render FAIL and a non-zero exit
GC_BN|check=render|status=FAIL|lit_pixels=240000|lit_fraction=1.00000|prove_flat_fill=yes
GC_BN|check=summary|status=FAIL                                            exit 1
```

The red case prints `240000` — **the exact number the broken check published as
success**. Two other falsifications were tried first and rejected as dishonest
demonstrations: `setEnabled(false)` threw inside `scene.render()` (red for the
wrong reason — an exception proves the harness fails, not that the threshold
discriminates), and both `isVisible = false` and an emptied camera frustum left
the ground plane drawing and made the frame *brighter* than the real scene
(0.847 lit against 0.782), because they removed the character's shadow. Neither
blanked anything; both are recorded in `spike.js` so nobody re-tries them.

**What survives, and what does not.** The skeleton and IK results never depended
on the pixel check — `bones_moved=34` and `effector_travel` come from bone
matrices, and they are unchanged. What changed is that "Babylon Native rendered
this scene" is now backed by a measurement that fails when it should.

Two smaller things the spike surfaced. `scene.getTotalVertices()` returned **0**
where the browser reports a real count, so at least one scene statistic is not
wired up on the native path. And the graphics API is reported as plain
`OpenGL` — bgfx's GL backend, not Vulkan.

**Builds exercised.** The spike ran on two independently configured builds — the
`RelWithDebInfo` one in §1.3 and a stripped `Release` one — for six runs in
total, with every check `OK` in every run. So the result is not an artefact of a
single build configuration.

### 1.5 The validation suite — measured, and it does not pass

This is the finding that decides the route. Babylon Native's own pixel-comparison
suite, run exactly as `.github/workflows/build-linux.yml` runs it:

```
DISPLAY=:1 ./Playground app:///Scripts/validation_native.js
```

| Run | Build | GL path | Tests validated | Outcome |
| --- | --- | --- | ---: | --- |
| 1 | RelWithDebInfo, my flags | NVIDIA 595.71.05 | **9 of 720** | `SIGSEGV` in `GLTF Extension KHR_materials_volume with attenuation`, exit 3 |
| 2 | RelWithDebInfo, my flags | `LIBGL_ALWAYS_SOFTWARE=1` (llvmpipe) | **9 of 720** | identical crash, same test |
| 3 | **exact CI flags** (`BX_CONFIG_DEBUG`, `BABYLON_DEBUG_TRACE`, `NATIVEDRACO`, `NATIVEMESHOPT` all `ON`) | NVIDIA | **9 of 720** | identical crash, same test |
| 4 | RelWithDebInfo, first 10 tests removed from `config.json` | NVIDIA | **196 of 710** | `SIGSEGV` in `Test code inlining`, exit 3 |

So: **about 205 of 720 tests — 28% — run before the process dies, at two
independent crash sites**, reproducibly, across two builds, both GL paths, and
the upstream CI's own compile flags.

Two things must be said about that immediately, because a reader will
(correctly) ask both.

**Upstream CI is green.** Every Linux job of run `30952414795` on
`aa244ec9` — the same commit — reports `success`. So this is not an upstream
regression, and this document does not claim it is. What it is: upstream CI runs
on GitHub's `ubuntu-latest` under `xvfb-run`, and the crash reproduces here both
on a real NVIDIA driver **and** under `LIBGL_ALWAYS_SOFTWARE=1`, on Ubuntu 24.04
with Mesa 25.2.8. The honest conclusion is the one that matters for a
ship decision: *a green Babylon Native CI does not predict a clean run on a
developer's actual Linux desktop*, and the failure mode when it diverges is a
segmentation fault with an empty callstack rather than a JavaScript exception.
For a project whose whole value proposition is running the same JS everywhere,
that is the wrong failure mode.

**The experiment that would settle it, and why it was not run.** Upstream CI
runs the suite under **`xvfb-run`**, and `LIBGL_ALWAYS_SOFTWARE=1` under a real
X server is a reasonable proxy for that but not the same thing — Xvfb is a
different X server, not just a different GL driver. Neither `xvfb-run` nor
`Xvfb` is installed on this box and installing them needs root, which is not
available here. **`xvfb-run ./Playground app:///Scripts/validation_native.js` is
therefore the single named, untried experiment that would discriminate "upstream
is immature" from "this environment differs"**, and it is cheap for anyone who
has Xvfb. Until someone runs it, this document's reading of the crash is bounded
by that.

**The suite could not be bisected with the tool provided.** `--test`,
`--test-index`, `--once` and `--list` are documented in
`Apps/Playground/Shared/CommandLine.cpp` and were silently ignored on every run.
Reading the source explains it:
`Apps/Playground/Win32/App.cpp` parses the options and publishes them to
JavaScript (`QueuePlaygroundOptions`, which sets `_playgroundOptions`);
**`Apps/Playground/X11/App.cpp` does neither** — it loops over `argv` and calls
`LoadScript` on each entry. Run 4 above only exists because the test list was
edited out of `config.json` by hand. That is a Linux-versus-Windows parity gap
in the host application, found by using it.

### 1.6 What Babylon Native is worth, stated fairly

The architectural case for it is real and this evidence does not damage it: the
same TypeScript, the same `Scene`, the same `BoneIKController`, no browser, one
renderer for web and desktop. Everything the spike touched worked, including the
feature the framework was chosen for.

The shipping case is what fails. No releases, no stable contract, no audio, no
particles, single-pointer input, no packaging, and a validation suite that
segfaults after a quarter of itself on the machine this game is developed on.
Adopting it means owning C++ work in someone else's repository on the critical
path of a game's presentation layer, and #330 is already a decision about
whether this project can afford a presentation rewrite at all.

**Inferred, and stated as inference:** this is a reasonable bet in 12–24 months
and an unreasonable one now. Nothing here says it will not get there — the
commit cadence says the opposite.

---

## 2. The webview determinism question was already settled — correctly

#329's second acceptance criterion — *"wasm hash parity across the three desktop
webviews is measured"* — was delivered by #342 (PR #344, `1c5da82`) and is
recorded in [`wasm_webview_determinism.md`](../online/wasm_webview_determinism.md).
It is not repeated here.

**Reviewed for overstatement, and it is not overstated.** It claims parity only
across the runtimes it ran (JavaScriptCore via real WebKitGTK 2.52.3, V8 via
Chrome 150, SpiderMonkey via Firefox 153, plus node), it labels WebView2 and
WKWebView `NOT RUN` in the result table itself rather than in a footnote, it
records the sha256 of the module it served in every row so "the same binary"
is checkable rather than asserted, and it names NaN bit patterns as
explicitly unsettled. `scripts/check_verify_webview.sh` is honestly titled: it
runs `--self-test` and starts no webview, and says so in the file. The verdict
regex is `(?<!MIS)MATCH`, so a `MISMATCH` cannot be read as a pass.

One small independent corroboration falls out of this issue's own measurements.
The Tauri row in §3 is WebKitGTK rendering the #341 scene, and it reports
**87.0 draw calls — bit-identical to Electron's V8/Chromium and to #341's
Chrome** for the same configuration, with the runner failing the route outright
if draw calls differ between passes. That is a *rendering-side* agreement across
two engines, not a simulation one, and it does not extend #342's claim. It just
fails to contradict it.

**For this decision:** the "three JS engines" objection to Tauri is not a
determinism objection. Tauri loses here on frame cost, which is a different
argument entirely, and one that would apply just as much if it shipped one
engine everywhere.

---

## 3. The route table

Measured with `scripts/native_shell_bench.py`, which serves the **#341 benchmark
page verbatim** — same scene, same 994 KB captured `RenderFrame` payload
(`state_hash 51875e4b2a3adac1`, seed 20260803), same markers — to each shell and
reads the results back over HTTP. Ten characters, `merged` variant, 600 measured
frames after 300 warm-up, **five fresh launches per route, interleaved one pass
at a time**, median across launches with the min–max span under each figure.
Cold start is five interleaved launches, same treatment.

Host: Linux 7.0.0-28-generic, RTX 2070 SUPER, `DISPLAY=:1`, measured 2026-08-04.
**Load average 0.56–0.80 across the cold-start passes and 0.80–1.06 across the
frame-cost passes** — an effectively idle box, recorded per pass in
`report.json`. See [the load caveat](#the-load-caveat) and
[the pacing control](#the-pacing-control), neither of which is boilerplate.

| route | installer | cold start `dom_ready` | cold start `scene_ready` | draw calls | **draw p50** | **draw p95** | frame p50 | frame p95 | GPU evidence |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| **Electron** 43.3.0 (Chromium/V8, reports `Chrome/150.0`) | **94.2 MiB** AppImage (312.1 MiB unpacked) | **622.6 ms** <sub>572.3–654.0</sub> | **876.6 ms** <sub>839.0–1132.0</sub> | 87.0 | **1.23 ms** <sub>1.17–1.24</sub> | **1.96 ms** <sub>1.69–2.12</sub> | 2.68 ms <sub>2.52–2.72</sub> | 4.09 ms <sub>3.43–4.49</sub> | `ANGLE (NVIDIA Corporation, NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2, OpenGL 4.5.0)`; `/dev/nvidia0` + `/dev/dri/renderD128` open |
| **Tauri** 2.11.5 (WebKitGTK 2.52.3 / JavaScriptCore, reports `Version/60.5 Safari/605.1.15`) | **1.3 MiB** `.deb` <br>73.8 MiB AppImage <br>3.2 MiB binary | **1328.3 ms** <sub>1305.3–1340.5</sub> | **1657.3 ms** <sub>1624.3–1673.5</sub> | 87.0 | **5.42 ms** <sub>5.38–5.48</sub> | **6.22 ms** <sub>5.92–6.32</sub> | 17.62 ms <sub>17.42–17.78</sub> | 20.44 ms <sub>20.08–20.90</sub> | engine string rejected as spoofed; mapped `libEGL_nvidia`, `libnvidia-eglcore`; `/dev/nvidia0` + `/dev/dri/renderD128` open |
| **Babylon Native** (bgfx GL / JavaScriptCore) | **NOT MEASURED** — no installer exists | — | 1592 ms *(different, smaller scene — excluded from every ratio)* | NOT MEASURED | NOT MEASURED | NOT MEASURED | NOT MEASURED | NOT MEASURED | — |
| **WebView2** (Windows) | NOT MEASURED | NOT MEASURED | NOT MEASURED | NOT MEASURED | NOT MEASURED | NOT MEASURED | NOT MEASURED | NOT MEASURED | no Windows hardware |
| **WKWebView** (macOS) | NOT MEASURED | NOT MEASURED | NOT MEASURED | NOT MEASURED | NOT MEASURED | NOT MEASURED | NOT MEASURED | NOT MEASURED | no macOS hardware |

Ratios: **Tauri costs 4.41x Electron's draw p50, 3.17x its draw p95, 6.59x its
frame p50, 5.00x its frame p95, and 1.89x its cold start to a rendered scene.**
Both routes reported the same payload `state_hash 51875e4b2a3adac1` and the same
87 draw calls in all five passes, so they rendered the same thing.

**Which of those columns the argument uses.** `draw` is CPU time inside
`scene.render()` and cannot contain vsync or compositor pacing; `frame` brackets
a `gl.finish()` and the inter-tick gap and can. **The decision rests on the two
`draw` columns**, where Tauri is 3.2–4.4x more expensive. The `frame` columns
are reported because a player feels them, and read with
[the pacing caveat](#the-pacing-control).

**On the 60 fps question.** Tauri-on-WebKitGTK's frame p50 is **17.62 ms**,
which is over the 16.7 ms a 60 fps frame allows — but only by about 6%, and
frame time is the pacing-inclusive column. Read it as *"WebKitGTK has no
headroom at ten characters"* rather than as a large margin: the defensible
version of the claim is the draw column, where Tauri spends 4.2 ms more CPU per
frame than Electron before any of the game's own work.

#### Babylon Native's two measurable cells

Its `scene_ready` figure above is **not comparable to the shells' and is marked
so**. It is spawn → first rendered frame for `bench/babylon_native/spike.js`
(five launches: 1466 / 1514 / 1592 / 1732 / 2235 ms, median 1592, load 10–13) —
a scene with **one** character, a ground plane and a shadow map, not the ten
characters, pitch, goals and ball the shells rendered. It is recorded because
"the process starts and draws in about a second and a half" is a real fact about
the route, and omitted from the ratios because comparing it to the shells would
be comparing two different scenes.

For size, the closest honest figure is the **stripped Release `Playground`
binary: 7,468,064 bytes (7.1 MiB)** — but that is not an installer. It
dynamically links 69 shared objects including the system
`libjavascriptcoregtk-4.1.so.0` (31,959,344 bytes / 30.5 MiB), which a
distributable would have to carry or replace. Babylon Native publishes no
packaging, so there is no number to put in the installer column and the cell
says so.

### How to read the columns

- **Installer** is what a player downloads. For Tauri both numbers are given
  because they are genuinely different products: the `.deb` links the system
  WebKitGTK and the AppImage carries it.
- **`dom_ready`** is spawn → the page exists. **`scene_ready`** is spawn →
  Babylon has created its engine, queried the GPU, loaded the 3.6 MB character
  and built the scene. The second is what a player experiences as "it started".
- **Draw calls** are deterministic; the runner refuses a route whose draw-call
  count differs between passes, because that would mean the scene changed and
  the timings are not comparable.
- **`draw` vs `frame`.** `draw` is CPU time inside `scene.render()`; `frame` is
  one loop iteration including a `gl.finish()`. Only `frame` can contain
  pacing — see [the pacing control](#the-pacing-control).
- **GPU evidence** is the string the engine reported *when it can be trusted*,
  and otherwise the drivers the process mapped plus the GPU device nodes it held
  open. See [the Apple GPU problem](#the-apple-gpu-problem).

### The pacing control

`frame_p50`/`frame_p95` bracket a `gl.finish()` and the gap between ticks, which
is exactly where vsync and compositor pacing leak in. So an asymmetry there
would corrupt the one comparison this document is quoted on — and the first
version of this work had one.

`bench/native_shell/electron/main.js` set Chromium's `--disable-frame-rate-limit`
and `--disable-gpu-vsync`, and justified them in a comment as matching
`scripts/babylon_bench.py`. **That precedent did not exist** — `babylon_bench.py`
sets no such flags — and WebKitGTK exposes no equivalent, so the Tauri shell
could not have been given the same treatment. One side of a two-sided comparison
was running free and the other was not.

It is fixed by controlling pacing from **outside** both shells, identically:
`scripts/native_shell_bench.py`'s `launch_shell()` sets `__GL_SYNC_TO_VBLANK=0`
(NVIDIA) and `vblank_mode=0` (Mesa) for every route, and the Electron-only
switches are gone. Whatever those variables do, they do to both processes.

**Re-measured after the fix, the gap widened rather than closed:**

| | before (asymmetric, load 5.8–6.6) | after (symmetric, load 0.8–1.1) |
| --- | ---: | ---: |
| Electron draw p50 | 1.89 ms | **1.23 ms** |
| Tauri draw p50 | 6.86 ms | **5.42 ms** |
| **ratio** | 3.63x | **4.41x** |
| Electron frame p50 | 4.20 ms | **2.68 ms** |
| Tauri frame p50 | 21.56 ms | **17.62 ms** |
| **ratio** | 5.13x | **6.59x** |

Both routes got faster, because the second run was on an idle machine; the
*ratio* moved against Tauri. So the confound was real, it was worth removing,
and removing it did not rescue Tauri.

**What is still not fully controlled, stated plainly.** Tauri's frame p50 of
17.62 ms sits close to a 60 Hz interval, so it is possible that WebKitGTK still
paces to the compositor through a path those two environment variables do not
reach. That possibility is precisely why the argument in §4 rests on the `draw`
columns, which are measured inside `scene.render()` and cannot contain pacing at
all. On `draw`, Tauri costs 4.41x — with no vsync interpretation available.

### The load caveat

This is not the usual "the machine was busy" footnote. It changed the answer,
and it is the reason the runner works the way it does.

The first version of this measurement ran **one route to completion and then the
next**. Under that scheme, on the same binaries and the same scene, three
sessions produced Electron/Tauri draw-p50 ratios of **5.8x, 1.4x and 3.7x** —
because a C++ build started partway through one of them, and whichever route was
being measured at the time absorbed all of it. That is exactly the trap #341
documented and fixed for the same reason, in the same words: *"session and
ordering matter more than expected."*

The published run interleaves — one pass of Electron, one pass of Tauri, five
times — so a drifting load lands on both routes. It was taken on an effectively
idle box: load 0.56–0.80 across the cold-start passes and 0.80–1.06 across the
frame-cost passes, recorded per pass in `report.json`. (An earlier draft of this
sentence said "5.77–6.59 for every pass" of the previous run; that range held
for its frame-cost passes only, and its cold starts dipped to 5.51. Per-pass
figures are in the report precisely so a reader never has to trust a prose
range.) The spreads are what a reader should judge it by: Tauri's five draw-p50
passes span **2%** of their median (5.38–5.48 ms) and Electron's span **6%**
(1.17–1.24 ms). The gap under test is 4.4x — two orders of magnitude outside
either spread.

Absolute numbers here are still one machine on one afternoon. **The ratios are
what this document argues from**, and they are what interleaving defends.

### The Apple GPU problem

WebKit answers `WEBGL_debug_renderer_info` with renderer **"Apple GPU"** and
vendor **"Apple Inc."** on every platform, including Linux on an NVIDIA card. It
is fingerprinting resistance, and it is a lie that looks like a fact:
`scripts/babylon_bench.py` refuses masked strings like `"WebKit WebGL"`, but
`"Apple GPU"` is a *positive identification*, so it sails straight through as
hardware evidence — and a fabricated GPU name would then have been printed in
the table above as proof.

`scripts/native_shell_bench.py` demotes it, and then proves hardware a different
way: it walks the shell's process tree and reads `/proc/<pid>/maps`. A process
with `libEGL_nvidia` and `libnvidia-eglcore` mapped is on the GPU whatever it is
willing to say about itself; a process with only `swrast_dri` mapped is not,
whatever it says. Both shells were checked this way, and the Tauri row's
evidence is those mapped drivers, not a string.

This matters for the verdict: **WebKitGTK's frame cost is not a
software-rendering artefact.** WebKitGTK had the NVIDIA driver mapped and still
cost 4.4x Electron's draw time for the same 87 draw calls. (What that does *not*
establish is anything about Tauri's other two webviews — see
[the recommendation](#4-the-recommendation).)

**How far that evidence actually reaches.** A mapped `.so` proves the loader
touched a driver, not that the GL context which drew the measured frame used it.
So the runner also records the **GPU device nodes each shell's process tree has
open**, which is one step closer to the hardware: both routes held
`/dev/dri/renderD128`, `/dev/nvidia0`, `/dev/nvidiactl` and `/dev/nvidia-modeset`
open during the measured passes. That is still not proof that *this particular
frame* went through the device, and no cheap check available here would be. The
residual risk is one-directional and against the conclusion, not for it: if
Tauri were secretly on a software path despite all of that, its real hardware
numbers would be *better* than published, not worse.

### Against the browser and native baselines

| | draw calls | draw p50 | draw p95 | source |
| --- | ---: | ---: | ---: | --- |
| LÖVE procedural (native) | 14 | — | 0.95–1.01 ms | #328 |
| LÖVE rigged (native) | 331.6 | — | 2.74–2.80 ms | #328 |
| **LÖVE optimised (native)** | **33.4** | — | — | #337 slice 2 |
| Babylon merged, Chrome 150 | 87 | 1.35 ms | 2.29 ms | #341 |
| Babylon merged, Firefox 153 | 87 | 2.80 ms | 4.08 ms | #341 |
| **Babylon merged, Electron** | **87** | **1.23 ms** | **1.96 ms** | this document |
| **Babylon merged, Tauri (WebKitGTK)** | **87** | **5.42 ms** | **6.22 ms** | this document |

Electron's Chromium reports itself as `Chrome/150.0` in its user agent — the
same major version #341 measured — which is why its draw times land next to the
#341 Chrome row rather than somewhere unrelated. Electron measures slightly
*faster* than #341's Chrome (1.23 ms against 1.35 ms draw p50), which is what an
idle box against a loaded one should look like. Treat that as a sanity check on
the harness, not as a second measurement of Chrome.

### What could not be measured, and why

| Route / target | Column | Why not |
| --- | --- | --- |
| **WebView2** (Windows) | installer, cold start, frame cost | Windows-only component; no Windows hardware. #342 reached the same wall for the determinism run and documented it there. **This is the most consequential gap in the table**: WebView2 is Chromium and V8, the same engine Electron ships, so the Linux frame-cost gap that this document's §4 point 1 rests on is the one result least likely to reproduce on Windows. Tauri's case is strongest exactly where nothing was measured. |
| **WKWebView** (macOS) | installer, cold start, frame cost | macOS-only framework; no macOS hardware. JavaScriptCore, the same engine family as the WebKitGTK that *was* measured — but a different embedding on a different graphics stack (Metal, not GL), so the Linux frame cost is a hint about it and not evidence. |
| **Babylon Native** | installer size | **There is nothing to measure.** It publishes no installer, no package and no release; §1.1. The stripped `Playground` binary size in §3 is an application binary, not a distributable, and it excludes the JavaScriptCore, GL and X11 libraries it links against. |
| **Babylon Native** | frame cost | Out of scope here and deliberately so. A comparable number needs the #341 scene *and* its captured payload driving the same ten skeletons inside the Playground host; anything less would produce a number that looks like the others and is not. Given §1.5, the route is not a candidate, so this is not the missing evidence. |
| Electron, Tauri | Windows / macOS installer size | Both cross-compile, neither was built. Linux figures do not transfer: an Electron Windows installer carries the same Chromium, but Tauri's Windows size depends on WebView2 already being present on the machine. |

---

## 4. The recommendation

**Ship Electron when a desktop build is scheduled, on Linux — the currently
supported platform** (`docs/online/platform_decision.md` records Linux as the
only one, and every number here is Linux-only). In order of weight:

1. **Draw cost — on Linux, against WebKitGTK, and only there.** 4.41x on
   `draw p50` is not a tuning gap, and it is measured inside `scene.render()`
   where [pacing cannot reach it](#the-pacing-control). #330 is already deciding
   whether Babylon's frame cost justifies a rewrite against an optimised LÖVE
   renderer that now draws ten players in 33.4 calls
   ([PR #350](https://github.com/osobytes/goliseo/pull/350)); a native shell
   that multiplies Babylon's draw time by 4.4 removes that argument on Linux. **This reason does not transfer to Windows**, where Tauri's webview is
   WebView2 — Chromium and V8, the same engine as Electron — and was not
   measured; see [what could not be measured](#what-could-not-be-measured-and-why).
   Reasons 2 to 4 do not depend on it, which is why the recommendation survives
   this caveat instead of resting on it.
2. **Engine uniformity, which is worth more here than it looks.** The same V8
   and the same Chromium in the desktop build and in the browser build means one
   rendering-quirk surface, one profile, one set of shader-compilation
   behaviours, and #341's browser numbers transfer to the desktop product
   instead of needing to be retaken. This project already maintains a browser
   path (`docs/online/platform_decision.md`) and is not going to stop.
3. **Cold start.** Electron reaches a rendered scene in 988 ms; Tauri takes
   1829 ms.
4. **It is boring.** It has releases, a packaging tool that produced an AppImage
   on the first attempt, and no unowned C++ on the critical path.

**Accept the download size.** 94.2 MiB against Tauri's 1.3 MiB `.deb` is the
real cost and it should not be minimised — but a 3.6 MB character asset and a
8.2 MB Babylon bundle are already in the product, the comparison is a one-time
download against a per-frame cost, and Tauri's small package is small precisely
because it borrows the platform's renderer — which on Linux is WebKitGTK and
then costs 4.4x the draw time. On Windows it would borrow Chromium instead, and that
trade would look very different; it was not measured.

**Reconsider if any of these change:**

- WebKitGTK's WebGL performance improves materially — this is one measurement of
  one version (2.52.3, `Version/60.5 Safari/605.1.15`) on one driver.
- Babylon Native publishes a release, and audio and particles land. Then the
  architectural case is worth re-running this spike against.
- The product stops needing a browser build. Engine uniformity is the second
  reason for Electron and it evaporates without a web target.
- **A Windows or macOS support target is added.** Both are unmeasured here, and
  Windows changes *two* things at once, not one. The obvious one is installer
  economics: WebView2 ships with the OS, so Tauri's package stays tiny without
  an AppImage carrying a renderer. The load-bearing one is that **WebView2 is
  Chromium and V8 — the same engine Electron ships — so the 4.4x draw-cost
  finding, which is a WebKitGTK finding, would very likely not reproduce there
  at all.** A Windows decision must re-measure rather than inherit this one; on
  Windows the case for Electron narrows to engine uniformity with the browser
  build and to maturity, and the frame-cost argument may simply not exist.

**The fallback stays sound.** #329 notes that two different *renderers* on web
and native cannot desync, because rendering is presentation-only and the
simulation is shared — #342 is the evidence that the simulation half of that
holds across engines. So "Babylon on web, something else native" remains
structurally safe. It is not the plan and this document does not recommend it.

---

## 5. What this does to #330

#330's recorded 2026-08-04 decision to drop LÖVE rested on two assumptions.
Both have now been tested and neither survived:

| Assumption | Status | Evidence |
| --- | --- | --- |
| "A real animation system makes character count stop being the binding constraint" | **Falsified** | #341: Babylon's marginal cost per character is flat to slightly rising at 10 → 20 → 40 in every configuration and both browsers. Lower constant, same curve. |
| "Babylon Native works well enough to carry the native build" | **Falsified for now** | §1.5 and §1.6: 28% of its own validation suite runs before a segfault here; no releases; no audio; no particles; no packaging. |

And the third leg has moved underneath both of them: **#337 slice 2 draws ten
players in 33.4 calls**, down from 331.6, taking the rigged renderer's added
draw p95 from 91.2% to 56.6% of its 2 ms budget
([PR #350](https://github.com/osobytes/goliseo/pull/350)) — *fewer draw calls
than Babylon merged in Chrome (87), and fewer than either native shell measured
here (87)*.

So the honest summary for whoever decides #330:

- Migrating to Babylon does **not** buy better skeleton scaling (#341).
- Migrating to Babylon does **not** now buy fewer draw calls than optimised
  LÖVE (#337 slice 2 vs the 87 in §3).
- The native story for Babylon on **Linux** is **Electron** today, because
  Babylon Native is not shippable (§1.5, §1.6) and Tauri-on-WebKitGTK costs
  4.4x the draw time with no frame-budget headroom at ten characters. **The Tauri half of
  that is Linux-only**: on Windows, Tauri's webview is WebView2 — Chromium and
  V8, the same engine as Electron — and was not measured, so "Electron or
  nothing" is a statement about the supported platform, not about Tauri in
  general. Electron's other advantages (engine uniformity with the browser
  build, cold start, maturity) do not depend on the Tauri frame number.
- What migrating still buys is the thing it always bought: a bought-not-built
  animation pipeline, glTF assets, and `BoneIKController` — which, to be fair to
  it, **works everywhere it was tested, including natively** (§1.4).

That is a narrower case than #330 started with. It is not an empty one, and this
document does not decide #330 — it removes one of the two assumptions #330 was
resting on and prices the native half of the other.

---

## 6. Residual risks

- **One machine, one afternoon.** Every measured number here comes from a single
  Linux box with an RTX 2070 SUPER under active load from other agents. The
  4.4x Electron-versus-Tauri draw gap is two orders of magnitude larger than
  the 2–6% within-run spread the five interleaved passes measured, and
  interleaving plus the identical draw-call count across passes is the evidence
  it is not noise — but a second machine has not confirmed it, and a sequential
  version of this same runner produced ratios between 1.4x and 5.8x before
  interleaving fixed it.
- **The Babylon Native crash is not root-caused.** It is reproducible here and
  green upstream. It could be a Mesa 25 / NVIDIA 595 interaction, a genuine
  upstream bug that GitHub's runners do not reach, or something about this box.
  What is *not* in doubt is the operational conclusion: it crashed, repeatedly,
  in its own suite, on the machine this game is built on.
- **No Windows and no macOS evidence at all.** Three of the six shell-and-target
  combinations in §3 are `NOT MEASURED`, and one of them (Tauri on Windows via
  WebView2) is where Tauri's case is strongest. A decision to ship Windows
  should re-open this.
- **The shells are minimal.** `bench/native_shell/electron` and
  `.../tauri` are one window and one URL each. A real client adds a menu, IPC, an
  updater and a splash screen, all of which move cold start and none of which
  move frame cost.
- **Babylon Native's frame cost is unknown**, so the possibility that it is
  *faster* than Electron is untested. It does not change the recommendation —
  a route with no audio, no particles and no releases is not blocked on being
  fast — but it is an open number.
- **Tauri's `frame` numbers may still contain pacing.** `__GL_SYNC_TO_VBLANK=0`
  and `vblank_mode=0` are applied to both shells, but WebKitGTK's 17.62 ms frame
  p50 sits close to a 60 Hz interval and a compositor path those variables do
  not reach cannot be excluded. The `draw` columns, which the recommendation
  rests on, are measured inside `scene.render()` and cannot contain pacing.
- **The GPU evidence proves a device is open, not that the frame used it.**
  Mapped drivers plus open `/dev/dri/renderD128` and `/dev/nvidia0` are strong
  circumstantial evidence and not a proof that the measured frame went through
  the GPU. The risk is one-directional: a hidden software path would make
  Tauri's real numbers better than published.
- **`BoneIKController` was proved to run, not proved to be good.** The spike
  shows the effector follows a moving target. It does not measure convergence
  quality, cost per character, or behaviour under the pose blending #341
  exercises. #318 owns that.

---

## 7. Running it

None of this is a CI gate, for the same reason `verify_webview.py` is not: it
needs a GPU, a display, a network and, for one route, a from-source C++ build of
someone else's project. What *is* in `scripts/check.sh` and in CI is the
controller self-test, which starts no shell:

```bash
python3 -B scripts/native_shell_bench.py --self-test    # refusal + parsing logic only
```

Read that name literally. It proves the runner rejects a software rasteriser, a
spoofed "Apple GPU", a page that errored, a run with no environment marker, a
route with no completed pass, passes whose draw-call counts disagree, and a
partial matrix; and it proves the `/proc` readers survive a process exiting
underneath them and find an open GPU device node when there is one. **It does
not start Electron, Tauri or a browser, and a green CI run is not shell
evidence.** The demonstrations that it goes red are in the PR for #329, run
against a hermetic `mktemp -d` copy.

The measurements:

```bash
# Build both shells, then measure. The build is pinned end to end: `npm ci`
# (not `npm install`, which silently reconciles a drifted lockfile into a
# different Electron), `@tauri-apps/cli@2.11.4` exactly, and `-- --locked` so
# `Cargo.toml`'s `tauri = "2"` cannot resolve past what `Cargo.lock` pins.
DISPLAY=:1 python3 -B scripts/native_shell_bench.py --build --routes electron,tauri

# Measure only, against shells already built. This reproduces the published
# table with no extra flags: the defaults ARE 5 cold starts and 5 interleaved
# frame passes.
DISPLAY=:1 python3 -B scripts/native_shell_bench.py --routes electron,tauri
```

A complete run writes `.bench/native_shell/report.json`; a run with any failed
route writes `report_incomplete.json` and deletes any stale `report.json`, so a
reader who forgets the exit code cannot pick up half a table. Same rule, and the
same reason, as `scripts/babylon_bench.py`.

The Babylon Native spike needs a Babylon Native build, which this repository
does not vendor. **This needs `libcurl4-openssl-dev` on the system include
path** — see [the stale `BUILDING.md`](#13-building-it--measured) for why
pointing CMake elsewhere does not work:

```bash
# Pinned to the commit every measurement in this document was taken against.
# An unpinned clone builds a different upstream and reproduces a different
# result -- see section 1.3.
git init BabylonNative && cd BabylonNative
git remote add origin https://github.com/BabylonJS/BabylonNative.git
git fetch --depth 1 origin aa244ec98c00660ee1832f68d4fdaa7f2620128e
git checkout FETCH_HEAD
git submodule update --init --recursive

cmake -G Ninja -B build/Linux \
  -D JAVASCRIPTCORE_LIBRARY=/usr/lib/x86_64-linux-gnu/libjavascriptcoregtk-4.1.so \
  -D NAPI_JAVASCRIPT_ENGINE=JavaScriptCore -D CMAKE_BUILD_TYPE=RelWithDebInfo \
  -D OpenGL_GL_PREFERENCE=GLVND . && ninja -C build/Linux

# The spike needs the same pinned character the #341 bench uses. Either bench
# runner stages it; take it from whichever has been run.
cp bench/babylon_native/spike.js  <BabylonNative>/build/Linux/Apps/Playground/Scripts/
cp .bench/native_shell/site/vendor/character.glb \
   <BabylonNative>/build/Linux/Apps/Playground/Scripts/character.glb
cp bench/babylon_native/prove_flat_fill.js \
   <BabylonNative>/build/Linux/Apps/Playground/Scripts/
cd <BabylonNative>/build/Linux/Apps/Playground

DISPLAY=:1 ./Playground app:///Scripts/spike.js                   # expect exit 0
DISPLAY=:1 ./Playground app:///Scripts/prove_flat_fill.js \
                       app:///Scripts/spike.js                    # expect exit 1
```

The second command is the section 9 demonstration that the spike's render check
can go red, run against a real renderer rather than a fixture. If it ever exits
0, the check has stopped discriminating and §1.4's "it drew" is void again.

## Assets

The Babylon bundles and the character are fetched on demand and verified against
pinned SHA-256 hashes, never committed — see `scripts/babylon_bench.py` and
THIRD_PARTY.md. The three bundler icons under `bench/native_shell/tauri/icons/`
are **generated** by `native_shell_bench.py` and gitignored — solid squares,
present only because both bundlers refuse to package without an icon, and
nothing ever displays them. Generating rather than committing them keeps
THIRD_PARTY.md's "no tracked image asset" audit true.
