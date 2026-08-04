# wasm determinism parity across desktop webviews

Experiment record for [issue #342](https://github.com/osobytes/goliseo/issues/342).
Feeds the native-desktop-route decision in
[#329](https://github.com/osobytes/goliseo/issues/329).

## The question

A native shell built on a system webview — Tauri, and anything shaped like it —
ships **a different JavaScript engine per platform**: V8 inside WebView2 on
Windows, JavaScriptCore inside WKWebView on macOS, JavaScriptCore inside
WebKitGTK on Linux. The obvious objection follows: three engines, three chances
for two players to compute different simulation state and desync.

The counter-argument is that the simulation does not run on the JavaScript
engine at all. It runs in WebAssembly, whose arithmetic is pinned by the
specification across conformant runtimes — IEEE-754 with defined rounding, no
extended precision, no reassociation — with NaN bit patterns the documented
exception. The JavaScript engine only drives rendering, which is
presentation-only and cannot desync.

That is reasoning. This is the evidence.

## Verdict

**Determinism parity holds across every runtime that was exercised.** The same
`simhost.wasm` produced byte-identical hashes under three independent JavaScript
engines — V8, SpiderMonkey and JavaScriptCore — plus node. Nothing observed here
argues against the portable-simulation-core architecture that Phase 0, #327,
#332 and the rest of the milestone rest on.

The finding is bounded by what was actually run, and one of the three webviews
in the issue title is the one that *was* run. See
[what this does and does not settle](#what-this-does-and-does-not-settle).

## Result

Fixture: the frozen 7201-tick determinism contract, run inside a Web Worker,
loaded over HTTP from a loopback server. Module: `wasm/sim-host` built from
`999f0a9` (see [the build caveat](#the-build-caveat)) — **the same binary in
every row**, so a difference could only have come from the runtime.

Expected: final hash `bfbb106aea5480f8`, sequence digest `a190b60058a64e63`.

| Runtime | JS engine | Version | Final hash | Sequence digest | Verdict | Cold start (runtime / sim ready) | Per tick |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **WebKitGTK** | JavaScriptCore | 2.52.3 (WebKit2-4.1) | `bfbb106aea5480f8` | `a190b60058a64e63` | **MATCH** | 38.0 ms / 86.0 ms | 0.3683 ms |
| Chrome | V8 | 150.0.7871.181 | `bfbb106aea5480f8` | `a190b60058a64e63` | MATCH | 19.1 ms / 45.4 ms | 0.4762 ms |
| Firefox | SpiderMonkey | 153.0.1 | `bfbb106aea5480f8` | `a190b60058a64e63` | MATCH | 58.0 ms / 78.0 ms | 0.4317 ms |
| node (reference, not a webview) | V8 | 22.22.0 | `bfbb106aea5480f8` | `a190b60058a64e63` | MATCH | — (reads from disk) | 0.4566 ms |
| **WebView2** | V8 | — | — | — | **NOT RUN** | — | — |
| **WKWebView** | JavaScriptCore | — | — | — | **NOT RUN** | — | — |

Host: Linux 7.0.0-28-generic x86_64, `DISPLAY=:1`. Measured 2026-08-04.

A fourth, unpinned agreement is worth recording because nobody chose it as a
contract: the performance run that follows the fixture prints
`state hash: ab55d5912c3b6009` after 600 ticks, and every runtime above printed
that same value. It is a second, independent 64-bit agreement over a different
tick count.

Cold start and per-tick cost differ between runtimes by up to a third, and vary
by tens of percent between runs of the same runtime on an otherwise busy
machine. That is expected and is not a determinism signal: it is JIT tiering and
wasm compilation strategy, which change *how fast* the same arithmetic is
performed and not *what it computes*. The hashes did not move at all. Firefox's
and WebKitGTK's marks land on whole milliseconds because worker
`performance.now()` is clamped there.

### Why WebView2 and WKWebView were not run

Both are a hardware constraint, not a missing feature:

- **WebView2** is a Windows-only component. There is no Linux or macOS build to
  point a driver at. The runner implements it — `--webviews webview2` drives a
  WebView2 host application through msedgedriver's `use_webview2` mode, which
  attaches to an application embedding the control rather than launching Edge,
  so the engine under test is the one a native shell would actually ship.
  **It has never been executed.** Anyone with a Windows box, the WebView2
  runtime, msedgedriver and a host application can run it; treat its first run
  as debugging unproven code, not as a regression.
  `scripts/windows_browser_campaign.ps1` establishes that a Windows evidence
  path exists in this repository, but it schedules Chrome and Firefox only —
  there is no existing WebView2 route to reuse.
- **WKWebView** is a macOS/iOS framework with no WebDriver that attaches to it.
  `safaridriver` drives Safari, a different application, so it is not a
  substitute even though it shares JavaScriptCore. Measuring WKWebView itself
  needs a macOS machine and a small native host application that loads the page
  — which is the shell a native-route decision would be building anyway, so it
  belongs to that work rather than to this harness. The runner reports it as
  NOT RUN with that reason and never as a pass.

## What this does and does not settle

**Settled.** Three independent JavaScript engines, including both engines a
webview shell would use on Linux and macOS, run the identical wasm module to
identical 64-bit hashes over 7201 ticks. WebKitGTK — the JavaScriptCore path,
and the engine most different from the V8 the project has been developed against
— is directly measured, not inferred.

**Strongly supported but not directly measured.** WKWebView is JavaScriptCore,
the same engine family as WebKitGTK, so the macOS webview's engine has been
exercised even though the macOS *embedding* has not. WebView2 is V8, exercised
here through Chrome and node. In both cases the untested part is the embedding,
not the engine — and the embedding does not implement WebAssembly arithmetic.

**Not settled.** Nothing here is evidence about NaN bit patterns, which the
WebAssembly specification explicitly leaves non-deterministic. This fixture
never produces one; a simulation that did could still diverge and this result
would not have caught it. That is a property of the simulation to keep, not a
property of the runtimes to trust.

**For #329.** The "Tauri ships three JavaScript engines" objection is not a
determinism objection. It remains a *rendering* and *packaging* objection —
three engines still mean three sets of rendering quirks — but the simulation
core is not at risk from it on this evidence.

## The build caveat

The module was built from commit `999f0a9`, not from `main` at `e091234`, because
**the module does not currently run when built from `main`**. #336 added
`render.identity`, `render.player_pose` and `render.frame` to the portable-Lua
probe, but `wasm/sim-host/build.rs` embeds only `core`, `data`, `scripts` and
`sim` — the `ROOTS` caveat that file documents in its own header. The probe
therefore aborts before it reaches the fixture:

```
ok    sim.headless
FAIL  render.identity
      module 'render.identity' not found: ... no embedded module 'render.identity'
```

That is a regression on `main`, tracked separately; it is not a determinism
finding and it does not affect this result, because every row of the table above
ran the same binary. `999f0a9` is the module #334/#335 delivered and the one
`verify_browser.py` was already passing with, which also makes these numbers
directly comparable to the Chrome and Firefox evidence recorded there.

## Running it

The measurement is run by hand. It needs a built module, a display and a real
webview, so it is **not** a CI gate — the same standing as
`scripts/phase0_sim_host.lua`. `scripts/check.sh` and CI run only
`verify_webview.py --self-test`, which proves the verdict rule rejects bad runs
and which **starts no webview**. Do not read a green CI run as webview evidence.

```bash
wasm/sim-host/build.sh                       # Docker supplies emcc

# Linux: the runtime that can actually be measured here.
DISPLAY=:1 /usr/bin/python3 wasm/sim-host/verify_webview.py --webviews webkitgtk

# Every runtime, including the ones this host cannot run. Exits non-zero and
# names each one as NOT RUN, which is the point.
DISPLAY=:1 /usr/bin/python3 wasm/sim-host/verify_webview.py --webviews all

# Windows, unexercised:
#   python wasm\sim-host\verify_webview.py --webviews webview2 ^
#       --webview2-host-app "C:\path\to\host.exe"

# The Chrome/Firefox baseline, same page, same worker, same verdict rule.
python3 wasm/sim-host/verify_browser.py
```

**Use `/usr/bin/python3` for the webview runner.** PyGObject is a system
package; a pyenv or virtualenv interpreter cannot import `gi`, and WebKitGTK is
driven through PyGObject rather than WebDriver because `WebKitWebDriver` is not
packaged on the development box and installing it needs root. The runner says
this rather than dying on an `ImportError`.

`--json <path>` writes a machine-readable record of every runtime, including the
ones that did not run and why.
