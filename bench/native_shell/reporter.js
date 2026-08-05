/*
 * Marker reporter for the #329 native-shell measurement.
 *
 * `scripts/native_shell_bench.py` injects this into the #341 bench page when it
 * serves it to a native shell. It exists because a shell is not a WebDriver
 * session: there is no `execute_script` to poll `window.__GC_BENCH__` with, and
 * console plumbing differs per shell -- Electron forwards renderer console to
 * the main process, WebKitGTK inside Tauri does not, and a comparison whose two
 * halves read their results through different channels is a comparison of the
 * channels.
 *
 * So both shells report the same way: HTTP POST back to the server that served
 * the page. Same origin, so no CORS and no COEP interaction.
 *
 * `bench/babylon/bench.js` is not modified by any of this. It already publishes
 * every marker on `window.__GC_BENCH__`; this file only forwards them.
 */
(function () {
    "use strict";

    const ENDPOINT = new URL("/gc_shell_mark", window.location.origin).toString();

    function post(body) {
        const text = JSON.stringify(body);
        try {
            // `keepalive` so the last POST survives the shell tearing the page
            // down, which is exactly when the `finished` mark is sent.
            fetch(ENDPOINT, {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: text,
                keepalive: true,
            }).catch(function () {
                fallback(text);
            });
        } catch (error) {
            fallback(text);
        }
    }

    function fallback(text) {
        try {
            const request = new XMLHttpRequest();
            request.open("POST", ENDPOINT, false);
            request.setRequestHeader("Content-Type", "application/json");
            request.send(text);
        } catch (error) {
            // Nothing left to try. The runner's timeout is the backstop, and a
            // silent failure here surfaces there as a missing mark, not as a
            // pass.
        }
    }

    function mark(name, extra) {
        const body = { kind: "mark", name: name, wall_ms: Date.now() };
        if (extra) {
            Object.assign(body, extra);
        }
        post(body);
    }

    mark("reporter_ready", {
        time_origin_ms: Math.round(performance.timeOrigin),
        user_agent: navigator.userAgent,
    });

    if (document.readyState === "loading") {
        document.addEventListener("DOMContentLoaded", function () {
            mark("dom_ready");
        });
    } else {
        mark("dom_ready");
    }

    let forwarded = 0;
    let sceneReported = false;
    let finished = false;

    const timer = setInterval(function () {
        const state = window.__GC_BENCH__;
        if (!state) {
            return;
        }
        while (forwarded < state.markers.length) {
            const line = state.markers[forwarded];
            forwarded += 1;
            post({ kind: "marker", line: line, wall_ms: Date.now() });
            if (!sceneReported && line.indexOf("GC_BENCH_ENV|") === 0) {
                sceneReported = true;
                mark("scene_ready");
            }
        }
        if (!finished && (state.status === "done" || state.status === "error")) {
            finished = true;
            clearInterval(timer);
            mark("finished", { status: state.status });
        }
    }, 20);
})();
