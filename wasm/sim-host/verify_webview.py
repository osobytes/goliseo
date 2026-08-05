#!/usr/bin/env python3
"""Run the frozen determinism fixture inside desktop webviews and compare hashes.

#342. A native desktop shell built on a webview (Tauri, and anything like it)
ships a *different JavaScript engine per platform*: V8 inside WebView2 on
Windows, JavaScriptCore inside WKWebView on macOS, JavaScriptCore inside
WebKitGTK on Linux. The objection that follows is obvious -- three engines, three
chances to desync -- and the counter-argument is that the simulation does not run
on the JavaScript engine at all. It runs in WebAssembly, whose float semantics
are pinned by the specification across conformant runtimes, with NaN bit
patterns the documented exception. The engine only drives rendering, which is
presentation-only and cannot desync.

That is reasoning. This script is the evidence. It loads the same module, the
same page and the same worker `verify_browser.py` already runs in Chrome and
Firefox, points them at a desktop webview instead, and compares the hashes the
fixture prints against the frozen `bfbb106aea5480f8` / `a190b60058a64e63`.

If the hashes do not match, the finding is much larger than Tauri: it would
undermine the portable-simulation core the whole milestone rests on. So this
script is deliberately unwilling to report a pass it did not observe -- a
runtime that cannot be launched is reported as NOT RUN and fails the run, and a
runtime that printed nothing fails rather than reading as "no divergence found".

    /usr/bin/python3 wasm/sim-host/verify_webview.py            # after build.sh

WHICH INTERPRETER: the WebKitGTK runner needs PyGObject, which is a system
package. A pyenv or virtualenv `python3` will not have it. Use `/usr/bin/python3`
explicitly; the runner says so rather than dying on an ImportError. A display is
also required (GTK), e.g. `DISPLAY=:1` or `xvfb-run`.

Runtime coverage on this machine is a hardware fact, not a code one:

    webkitgtk   Linux    driven here, through PyGObject (WebKitWebDriver is not
                         packaged on this box and installing it needs root).
    webview2    Windows  driven through msedgedriver's WebView2 mode. Implemented
                         and reachable with `--webviews webview2` on a Windows
                         host with a WebView2 host application; never exercised
                         from Linux, and this script will not pretend otherwise.
    wkwebview   macOS    no automation path without a native host application to
                         embed it; reported as NOT RUN with that reason.
"""

from __future__ import annotations

import argparse
import importlib
import json
import os
import platform
import sys
from dataclasses import dataclass, field
from pathlib import Path

import verify_common
from verify_common import EXPECTED_DIGEST, EXPECTED_HASH, ROOT, check

#: Every runtime this script knows about, in report order.
RUNTIMES = ("webkitgtk", "webview2", "wkwebview")


class WebviewUnavailable(RuntimeError):
    """This runtime cannot be driven here, and the reason is worth printing.

    Distinct from a failure *of* the simulation: it says nothing about
    determinism, so it is reported as NOT RUN rather than as a mismatch -- but it
    still fails the run, because a requested measurement that did not happen is
    not a pass.
    """


@dataclass
class Result:
    """One runtime's outcome. `ran` and `ok` are deliberately separate."""

    runtime: str
    ran: bool
    ok: bool
    detail: str
    engine: str = "not determined"
    out: list[str] = field(default_factory=list)
    marks: list[dict] = field(default_factory=list)
    #: sha256 of the artifacts this runtime was actually served. Carried per
    #: result rather than once per invocation because the comparison that
    #: matters spans invocations -- this run against the Chrome/Firefox one.
    artifacts: dict[str, str] = field(default_factory=dict)

    def hashes(self) -> dict[str, str]:
        return verify_common.observed_hashes(self.out)

    def module(self) -> str:
        return verify_common.short_digest(self.artifacts)


# --------------------------------------------------------------------------
# WebKitGTK, through PyGObject
# --------------------------------------------------------------------------

#: Typelib candidates, newest API first. WebKit2 4.1 is the current GTK3 binding
#: (4.0 is the older soup2 one); WebKit 6.0 is the GTK4 binding that eventually
#: replaces both. The GTK major version differs with the binding, so it is
#: carried alongside.
WEBKIT_BINDINGS = (
    ("WebKit2", "4.1", "3.0"),
    ("WebKit2", "4.0", "3.0"),
    ("WebKit", "6.0", "4.0"),
)

GI_HINT = (
    "PyGObject with a WebKitGTK typelib is required and is a system package. "
    "Run this with /usr/bin/python3 (a pyenv or virtualenv interpreter does not "
    "see system gi), and install gir1.2-webkit2-4.1 if it is missing."
)


def _load_webkit():
    """Import gi and the best available WebKitGTK binding, or explain why not."""
    try:
        import gi
    except ImportError as error:
        raise WebviewUnavailable(f"{GI_HINT} (import gi failed: {error})") from error

    attempts: list[str] = []
    for namespace, version, gtk_version in WEBKIT_BINDINGS:
        try:
            gi.require_version(namespace, version)
            gi.require_version("Gtk", gtk_version)
        except ValueError as error:
            attempts.append(f"{namespace}-{version}: {error}")
            continue
        # Imported only after require_version, which is what pins the binding.
        from gi.repository import GLib, Gtk

        webkit = importlib.import_module(f"gi.repository.{namespace}")
        return webkit, Gtk, GLib, f"{namespace}-{version}", gtk_version
    raise WebviewUnavailable(f"{GI_HINT} (no usable typelib: {'; '.join(attempts)})")


def run_webkitgtk(url: str, timeout: int) -> tuple[list[str], list[dict], str]:
    """Load the page in a WebKitGTK WebView and poll it until the fixture ends.

    The polling loop is the same shape as the WebDriver one, expressed in GLib's
    idiom: `evaluate_javascript` is asynchronous, so a timeout source asks for a
    sample once a second and the callback decides whether to quit the loop.

    The engine's failure modes are subscribed to explicitly. A web process that
    is killed -- for running out of memory, say -- otherwise looks exactly like a
    slow run, and would be silently reported as a timeout instead of a crash.
    """
    # Both are commonly required when GTK runs against a virtual or software
    # display; without them WebKit can fail to create a rendering surface and the
    # web process never starts. Neither touches JavaScript or WebAssembly, so
    # neither can influence what this script measures.
    os.environ.setdefault("WEBKIT_DISABLE_COMPOSITING_MODE", "1")
    os.environ.setdefault("WEBKIT_DISABLE_DMABUF_RENDERER", "1")

    if not os.environ.get("DISPLAY") and not os.environ.get("WAYLAND_DISPLAY"):
        raise WebviewUnavailable(
            "no DISPLAY or WAYLAND_DISPLAY; GTK needs a display "
            "(try DISPLAY=:1 or run under xvfb-run)"
        )

    webkit, gtk, glib, binding, gtk_version = _load_webkit()

    if gtk_version == "3.0":
        ok, _ = gtk.init_check()
    else:
        ok = gtk.init_check()
    if not ok:
        raise WebviewUnavailable(f"Gtk.init_check() failed on DISPLAY={os.environ.get('DISPLAY')}")

    engine = "{} {}.{}.{} ({})".format(
        "WebKitGTK",
        webkit.get_major_version(),
        webkit.get_minor_version(),
        webkit.get_micro_version(),
        binding,
    )

    view = webkit.WebView()
    settings = view.get_settings()
    settings.set_enable_javascript(True)
    # Worker and module errors are reported through the page, but console output
    # is the only place a syntax error inside worker.js would surface.
    settings.set_enable_write_console_messages_to_stdout(True)

    window = gtk.Window()
    window.set_default_size(800, 600)
    if gtk_version == "3.0":
        window.add(view)
        window.show_all()
    else:
        window.set_child(view)
        window.present()

    state: dict = {"out": [], "marks": [], "done": False, "error": None}
    loop = glib.MainLoop()
    deadline = glib.get_monotonic_time() + timeout * 1_000_000

    def stop(reason: str | None) -> None:
        if reason and not state["error"]:
            state["error"] = reason
        if loop.is_running():
            loop.quit()

    def on_load_failed(_view, _event, failing_uri, error) -> bool:
        stop(f"load failed for {failing_uri}: {error.message}")
        return True

    def on_web_process_terminated(_view, reason) -> None:
        stop(f"web process terminated ({reason})")

    view.connect("load-failed", on_load_failed)
    # Named differently across bindings; both mean the web process died.
    for signal in ("web-process-terminated", "web-process-crashed"):
        try:
            view.connect(signal, on_web_process_terminated)
            break
        except TypeError:
            continue

    def on_sample(_view, task, _data) -> None:
        try:
            value = view.evaluate_javascript_finish(task)
        except glib.Error as error:
            # The page is still loading on the first samples; that is expected
            # and not a reason to stop.
            if "not been loaded" in str(error) or "Unsupported" in str(error):
                return
            stop(f"evaluate_javascript failed: {error}")
            return
        if value is None:
            return
        try:
            sample = json.loads(value.to_string())
        except (ValueError, TypeError) as error:
            stop(f"could not read the page sample: {error}")
            return
        state["out"] = [str(line) for line in sample.get("out", [])]
        state["marks"] = list(sample.get("marks", []))
        if verify_common.is_finished(state["out"], bool(sample.get("done"))):
            state["done"] = True
            stop(None)

    def poll() -> bool:
        if state["done"]:
            return False
        if glib.get_monotonic_time() > deadline:
            stop(f"timed out after {timeout}s")
            return False
        view.evaluate_javascript(verify_common.POLL_SCRIPT, -1, None, None, None, on_sample, None)
        return True

    view.load_uri(url)
    glib.timeout_add_seconds(1, poll)
    loop.run()

    if gtk_version == "3.0":
        window.destroy()
    else:
        window.close()

    if state["error"] and not state["out"]:
        raise WebviewUnavailable(state["error"])
    if state["error"]:
        state["out"] = [*state["out"], f"FAILED: {state['error']}"]
    return state["out"], state["marks"], engine


# --------------------------------------------------------------------------
# WebView2, through msedgedriver
# --------------------------------------------------------------------------


def run_webview2(url: str, timeout: int, host_app: str | None) -> tuple[list[str], list[dict], str]:
    """Drive a WebView2 host application through msedgedriver's WebView2 mode.

    Written so someone with a Windows box can run it, not so this machine can
    pretend to. WebView2 is a Windows component: there is no Linux build to point
    at, and no amount of harness code changes that. Selenium's Edge driver
    understands `use_webview2`, which attaches to a host application that embeds
    the control rather than launching the browser -- so the engine under test is
    the one a Tauri-style shell would actually ship.
    """
    if platform.system() != "Windows":
        raise WebviewUnavailable(
            "WebView2 is a Windows-only component; there is no Linux or macOS "
            f"build to drive (this host is {platform.system()}). Run this script "
            "with --webviews webview2 on a Windows host that has the WebView2 "
            "runtime, msedgedriver, and a host application to attach to."
        )
    if not host_app:
        raise WebviewUnavailable(
            "--webview2-host-app is required: msedgedriver's WebView2 mode attaches "
            "to an application that embeds the control, so it needs that .exe path."
        )

    try:
        from selenium import webdriver
        from selenium.webdriver.edge.options import Options
    except ImportError as error:
        raise WebviewUnavailable(f"selenium is required for WebView2 ({error})") from error

    options = Options()
    options.use_webview2 = True
    options.binary_location = host_app
    driver = webdriver.Edge(options=options)
    try:
        capabilities = driver.capabilities or {}
        engine = f"WebView2 {capabilities.get('browserVersion', 'unknown')}"
        out, marks = verify_common.poll_webdriver(driver, url, timeout)
        return out, marks, engine
    finally:
        try:
            driver.quit()
        except Exception:
            pass


# --------------------------------------------------------------------------
# WKWebView
# --------------------------------------------------------------------------


def run_wkwebview(url: str, timeout: int) -> tuple[list[str], list[dict], str]:
    """Not implemented, and the honest reason is the whole content of this function.

    WKWebView is a macOS/iOS framework. Unlike WebView2 there is no WebDriver
    that attaches to it: `safaridriver` drives Safari, which is a different
    application even though it shares JavaScriptCore. Measuring WKWebView itself
    needs a small native host app -- which is exactly the shell a native-route
    decision would be building anyway, so it belongs to that work, not to this
    harness.
    """
    raise WebviewUnavailable(
        "WKWebView is a macOS/iOS framework with no WebDriver that attaches to "
        f"it (this host is {platform.system()}). Measuring it needs a macOS "
        "machine and a small native host application that loads the page; "
        "safaridriver drives Safari, not WKWebView, so it is not a substitute."
    )


# --------------------------------------------------------------------------
# Orchestration
# --------------------------------------------------------------------------


def run_runtime(
    runtime: str,
    url: str,
    timeout: int,
    host_app: str | None,
    artifacts: dict[str, str] | None = None,
) -> Result:
    """Run one runtime and turn whatever happened into a Result."""
    artifacts = dict(artifacts or {})
    try:
        if runtime == "webkitgtk":
            out, marks, engine = run_webkitgtk(url, timeout)
        elif runtime == "webview2":
            out, marks, engine = run_webview2(url, timeout, host_app)
        elif runtime == "wkwebview":
            out, marks, engine = run_wkwebview(url, timeout)
        else:
            raise WebviewUnavailable(f"unknown runtime {runtime!r}")
    except WebviewUnavailable as error:
        return Result(runtime=runtime, ran=False, ok=False, detail=str(error), artifacts=artifacts)
    except Exception as error:  # noqa: BLE001 - reported, not swallowed
        return Result(
            runtime=runtime,
            ran=False,
            ok=False,
            detail=f"launch failed: {error}",
            artifacts=artifacts,
        )

    ok, detail = check(out)
    return Result(
        runtime=runtime,
        ran=True,
        ok=ok,
        detail=detail,
        engine=engine,
        out=out,
        marks=marks,
        artifacts=artifacts,
    )


def summarise(results: list[Result]) -> tuple[int, list[str]]:
    """Turn results into the exit code and the closing report.

    Pure, so `--self-test` can prove the exit code goes red. The rule it encodes
    is the one AGENTS.md §9 exists for: a runtime that did not run is not a pass,
    and a report that lists a failure must not exit 0.
    """
    lines: list[str] = []
    ran = [r for r in results if r.ran]
    not_run = [r for r in results if not r.ran]
    matched = [r for r in ran if r.ok]

    lines.append("")
    lines.append(f"expected  final hash {EXPECTED_HASH}  sequence digest {EXPECTED_DIGEST}")
    for result in results:
        if not result.ran:
            lines.append(f"  {result.runtime:<12} NOT RUN   {result.detail}")
            continue
        hashes = result.hashes()
        lines.append(
            f"  {result.runtime:<12} {'MATCH' if result.ok else 'MISMATCH':<9}"
            f"{hashes.get('final_hash', '(none printed)')} / "
            f"{hashes.get('sequence_digest', '(none printed)')}  "
            f"module {result.module()}  [{result.engine}]"
        )

    if not ran:
        lines.append("")
        lines.append("NO RUNTIME WAS EXERCISED. This is not a determinism result.")
        return 1, lines

    if len(matched) == len(ran) and not not_run:
        lines.append("")
        lines.append(
            f"WEBVIEW VERIFY OK: {len(matched)} runtime(s) reproduced the frozen contract."
        )
        return 0, lines

    lines.append("")
    if matched and len(matched) != len(ran):
        lines.append(
            "DETERMINISM PARITY BROKEN: runtimes disagree on the fixture. This is a "
            "finding about the portable simulation core, not about the harness."
        )
    for result in results:
        if result.ran and not result.ok:
            lines.append(f"  MISMATCH {result.runtime}: {result.detail}")
        elif not result.ran:
            lines.append(f"  NOT RUN  {result.runtime}: {result.detail}")
    return 1, lines


PASSING_OUTPUT = [
    "== frozen determinism contract",
    f"final hash       {EXPECTED_HASH}",
    f"expected         {EXPECTED_HASH}",
    f"sequence digest  {EXPECTED_DIGEST}",
    "ticks            7201",
    "verdict          MATCH",
    "PHASE0 OK: the simulation runs outside LOVE.",
]


def self_test() -> int:
    """Prove this gate can go red.

    Covers the two ways a determinism harness lies: calling a bad run good, and
    exiting 0 while printing a failure. Both are real defects this repository has
    paid for (#279), so both are asserted rather than assumed.
    """
    failures: list[str] = []

    def expect(label: str, condition: bool) -> None:
        if not condition:
            failures.append(label)

    ok, _ = check(PASSING_OUTPUT)
    expect("a real passing output must pass", ok)

    bad_cases = {
        "empty output": [],
        "no hash": [line for line in PASSING_OUTPUT if EXPECTED_HASH not in line],
        "no digest": [line for line in PASSING_OUTPUT if EXPECTED_DIGEST not in line],
        "divergence reported": [*PASSING_OUTPUT, "DIVERGED at tick 12"],
        "failure reported": [*PASSING_OUTPUT, "FAILED: render.identity"],
        "worker error only": ["WORKER_ERROR: undefined"],
        "abort only": ["ABORT: memory access out of bounds"],
        "wrong hash": [line.replace(EXPECTED_HASH, "0123456789abcdef") for line in PASSING_OUTPUT],
        # "MISMATCH" contains "MATCH". A substring test for the verdict accepts
        # the exact word that means the opposite, so the word that must never
        # pass is asserted directly rather than left to the earlier clauses.
        "a MISMATCH verdict": [
            line.replace("verdict          MATCH", "verdict          MISMATCH")
            for line in PASSING_OUTPUT
        ],
    }
    for label, out in bad_cases.items():
        ok, _ = check(out)
        expect(f"{label} must fail", not ok)

    hashes = verify_common.observed_hashes(PASSING_OUTPUT)
    expect("the printed hash is reported", hashes.get("final_hash") == EXPECTED_HASH)
    expect("the printed digest is reported", hashes.get("sequence_digest") == EXPECTED_DIGEST)
    expect("an empty run reports no hashes", verify_common.observed_hashes([]) == {})

    expect("a partial run is not finished", not verify_common.is_finished(["ok core.vec2"], False))
    expect("an exited run is finished", verify_common.is_finished([], True))
    expect("a failed run is finished", verify_common.is_finished(["FAILED: nope"], False))

    sample_module = "a1b2c3d4" * 8
    passing = Result(
        "webkitgtk",
        ran=True,
        ok=True,
        detail="ok",
        out=PASSING_OUTPUT,
        artifacts={"simhost.wasm": sample_module},
    )
    failing = Result("webkitgtk", ran=False, ok=False, detail="no display")
    mismatch = Result("webview2", ran=True, ok=False, detail="no MATCH verdict", out=["nothing"])

    expect("all-match exits 0", summarise([passing])[0] == 0)
    expect("no runtime exercised exits 1", summarise([failing])[0] == 1)
    expect("nothing exercised exits 1", summarise([])[0] == 1)
    expect("a mismatch exits 1", summarise([passing, mismatch])[0] == 1)
    expect("a not-run runtime exits 1", summarise([passing, failing])[0] == 1)
    expect(
        "a mismatch is named as a parity finding",
        any("DETERMINISM PARITY BROKEN" in line for line in summarise([passing, mismatch])[1]),
    )
    expect(
        "an unexercised runtime is not reported as a pass",
        any("NOT RUN" in line for line in summarise([failing])[1]),
    )

    expect(
        "the module identity reaches the report",
        any(f"module {sample_module[:16]}" in line for line in summarise([passing])[1]),
    )
    expect(
        "an undigested run says so instead of showing a hash",
        any("module (not digested)" in line for line in summarise([mismatch])[1]),
    )

    # The identity of the served module. "Every row ran the same binary" is the
    # load-bearing claim of a cross-runtime comparison, so the machinery that
    # makes it checkable is tested like any other verdict input.
    import hashlib
    import shutil
    import tempfile

    scratch = Path(tempfile.mkdtemp(prefix="goliseo-wasm-selftest-"))
    staged_dirs: list[Path] = []
    try:
        (scratch / "simhost.js").write_text("glue")
        (scratch / "simhost.wasm").write_bytes(b"module")
        digests = verify_common.artifact_digests(scratch)
        expect(
            "a served artifact is digested with sha256",
            digests.get("simhost.wasm") == hashlib.sha256(b"module").hexdigest(),
        )
        expect(
            "both required artifacts are digested",
            set(digests) == {"simhost.js", "simhost.wasm"},
        )
        expect(
            "the report column is the first 16 hex of the module",
            verify_common.short_digest(digests) == digests["simhost.wasm"][:16],
        )
        (scratch / "simhost.wasm").write_bytes(b"a different module")
        expect(
            "a different module is not reported as the same one",
            verify_common.artifact_digests(scratch).get("simhost.wasm")
            != digests["simhost.wasm"],
        )
        expect(
            "a missing artifact is not silently digested",
            verify_common.short_digest({}) == "(not digested)",
        )

        # stage() serves what it is given, so #332's payload harness can reuse it
        # instead of copying the staging and drifting from it.
        staged = verify_common.stage(
            scratch, page="<p>other</p>", worker="// other", required=("simhost.wasm",)
        )
        staged_dirs.append(staged)
        expect("a caller's page is staged", (staged / "index.html").read_text() == "<p>other</p>")
        expect("a caller's worker is staged", (staged / "worker.js").read_text() == "// other")

        default_staged = verify_common.stage(scratch)
        staged_dirs.append(default_staged)
        expect(
            "the default staging is still the fixture's",
            (default_staged / "index.html").read_text() == verify_common.PAGE
            and (default_staged / "worker.js").read_text() == verify_common.WORKER,
        )
        expect(
            "staging digests what was copied, not what was asked for",
            verify_common.artifact_digests(default_staged)
            == verify_common.artifact_digests(scratch),
        )

        enforced = False
        try:
            verify_common.stage(scratch, required=("nothing-here.wasm",))
        except verify_common.ArtifactsMissing:
            enforced = True
        expect("a caller's required list is enforced", enforced)
    finally:
        for directory in (scratch, *staged_dirs):
            shutil.rmtree(directory, ignore_errors=True)

    for failure in failures:
        print(f"SELF-TEST FAIL: {failure}")
    if failures:
        print(f"\n{len(failures)} self-test expectation(s) failed")
        return 1
    print("SELF-TEST OK: the webview determinism verdict rejects every bad run it was shown.")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist", type=Path, default=ROOT / "dist")
    parser.add_argument(
        "--webviews",
        default="webkitgtk",
        help=f"comma-separated subset of {', '.join(RUNTIMES)}, or 'all'",
    )
    parser.add_argument("--timeout-seconds", type=int, default=900)
    parser.add_argument(
        "--webview2-host-app",
        default=os.environ.get("WEBVIEW2_HOST_APP"),
        help="Windows only: the .exe embedding the WebView2 control to attach to",
    )
    parser.add_argument("--json", type=Path, help="write the results to this path as JSON")
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="prove the verdict logic rejects bad runs; launches no webview",
    )
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    requested = (
        list(RUNTIMES)
        if args.webviews.strip() == "all"
        else [name.strip() for name in args.webviews.split(",") if name.strip()]
    )
    unknown = [name for name in requested if name not in RUNTIMES]
    if unknown:
        print(f"unknown runtime(s): {', '.join(unknown)}; known: {', '.join(RUNTIMES)}")
        return 2

    try:
        serve_dir = verify_common.stage(args.dist)
    except verify_common.ArtifactsMissing as error:
        print(error)
        return 1

    artifacts = verify_common.artifact_digests(serve_dir)
    for name, digest in artifacts.items():
        print(f"served    {name:<14}sha256 {digest}", flush=True)

    server, url = verify_common.serve(serve_dir)
    results: list[Result] = []
    try:
        for runtime in requested:
            print(f"==> {runtime}", flush=True)
            result = run_runtime(
                runtime, url, args.timeout_seconds, args.webview2_host_app, artifacts
            )
            results.append(result)
            if not result.ran:
                print(f"    NOT RUN: {result.detail}", flush=True)
                continue
            print(f"    engine: {result.engine}", flush=True)
            verify_common.print_run(result.out, result.marks)
            print(f"    -> {'PASS' if result.ok else 'FAIL'}: {result.detail}", flush=True)
    finally:
        server.shutdown()

    code, lines = summarise(results)
    for line in lines:
        print(line, flush=True)

    if args.json:
        args.json.write_text(
            json.dumps(
                {
                    "schema": "wasm-webview-determinism-1",
                    "expected": {"final_hash": EXPECTED_HASH, "sequence_digest": EXPECTED_DIGEST},
                    "host": {"system": platform.system(), "release": platform.release()},
                    "results": [
                        {
                            "runtime": r.runtime,
                            "ran": r.ran,
                            "match": r.ok,
                            "detail": r.detail,
                            "engine": r.engine,
                            "observed": r.hashes(),
                            "artifacts_sha256": r.artifacts,
                            "cold_start_ms": {m["mark"]: m["ms"] for m in r.marks},
                        }
                        for r in results
                    ],
                },
                indent=2,
            )
            + "\n"
        )

    return code


if __name__ == "__main__":
    sys.exit(main())
