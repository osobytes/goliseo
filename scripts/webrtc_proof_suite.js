/* Run both OMP-0 WebRTC network profiles in separate browsing contexts. */
(function () {
  "use strict";

  var query = new URL(window.location.href).searchParams;
  var duration = Math.max(1000, Number(query.get("duration_ms")) || 600000);
  var runId = query.get("run_id") || "manual";
  var status = document.getElementById("suite-status");
  var diagnostics = document.getElementById("suite-diagnostics");
  var runButton = document.getElementById("run-suite");
  var startedAtMs = null;
  var startedAtIso = null;
  var suiteComplete = false;
  var suiteError = null;
  var mismatchComplete = false;

  function frame(id) {
    return document.getElementById(id);
  }

  function runner(id) {
    return frame(id).contentWindow.GoliseoWebRTCProofRunner;
  }

  function childUrl(role, profile, extra) {
    var params = new URLSearchParams({
      role: role,
      profile: profile,
      duration_ms: String(duration)
    });
    Object.keys(extra || {}).forEach(function (key) {
      params.set(key, extra[key]);
    });
    return "webrtc-proof.html?" + params.toString();
  }

  frame("baseline-host").src = childUrl("host", "baseline");
  frame("baseline-guest").src = childUrl("guest", "baseline");
  frame("shaped-host").src = childUrl("host", "shaped");
  frame("shaped-guest").src = childUrl("guest", "shaped");
  frame("mismatch-host").src = childUrl("host", "baseline");
  frame("mismatch-guest").src = childUrl("guest", "baseline", { build_id: "mismatch" });

  function waitFor(condition, timeoutMs) {
    return new Promise(function (resolve, reject) {
      var started = Date.now();
      var timer = window.setInterval(function () {
        try {
          var result = condition();
          if (result) {
            window.clearInterval(timer);
            resolve(result);
          } else if (Date.now() - started > timeoutMs) {
            window.clearInterval(timer);
            reject(new Error("proof suite timed out"));
          }
        } catch (error) {
          if (Date.now() - started > timeoutMs) {
            window.clearInterval(timer);
            reject(error);
          }
        }
      }, 50);
    });
  }

  async function connect(host, guest, expectMismatch) {
    var offer = await host.create_offer();
    var answer = await guest.accept_offer(offer);
    await host.accept_answer(answer);
    if (expectMismatch) {
      await waitFor(function () {
        var hostReport = host.diagnostics();
        var guestReport = guest.diagnostics();
        return hostReport.last_error && hostReport.last_error.code === "build_mismatch" &&
          guestReport.last_error && guestReport.last_error.code === "build_mismatch";
      }, 10000);
      return;
    }
    await waitFor(function () {
      return host.diagnostics().handshake_complete && guest.diagnostics().handshake_complete;
    }, 10000);
  }

  function collectPair(prefix) {
    return {
      host: runner(prefix + "-host").diagnostics(),
      guest: runner(prefix + "-guest").diagnostics()
    };
  }

  function report() {
    var result = {
      run_id: runId,
      requested_duration_ms: duration,
      started_at: startedAtIso,
      elapsed_ms: startedAtMs === null ? 0 : Date.now() - startedAtMs,
      browser_user_agent: window.navigator.userAgent,
      suite_complete: suiteComplete,
      suite_error: suiteError,
      mismatch_complete: mismatchComplete,
      profiles: {}
    };
    try {
      result.profiles.baseline = collectPair("baseline");
      result.profiles.shaped = collectPair("shaped");
      result.mismatch = collectPair("mismatch");
    } catch (error) {
      result.frames_ready = false;
    }
    diagnostics.textContent = JSON.stringify(result, null, 2);
    if (suiteError) {
      status.textContent = "error";
    } else if (suiteComplete) {
      status.textContent = "complete";
    } else if (startedAtMs !== null) {
      status.textContent = "running";
    } else {
      status.textContent = "ready";
    }
    status.dataset.state = status.textContent;
    return result;
  }

  runButton.onclick = async function () {
    runButton.disabled = true;
    suiteError = null;
    suiteComplete = false;
    mismatchComplete = false;
    startedAtMs = Date.now();
    startedAtIso = new Date(startedAtMs).toISOString();
    try {
      await waitFor(function () {
        return runner("baseline-host") && runner("baseline-guest") &&
          runner("shaped-host") && runner("shaped-guest") &&
          runner("mismatch-host") && runner("mismatch-guest");
      }, 10000);
      await Promise.all([
        connect(runner("baseline-host"), runner("baseline-guest"), false),
        connect(runner("shaped-host"), runner("shaped-guest"), false),
        connect(runner("mismatch-host"), runner("mismatch-guest"), true)
      ]);
      mismatchComplete = true;
      runner("baseline-host").start_traffic();
      runner("baseline-guest").start_traffic();
      runner("shaped-host").start_traffic();
      runner("shaped-guest").start_traffic();
      await waitFor(function () {
        return collectPair("baseline").host.run_completed &&
          collectPair("baseline").guest.run_completed &&
          collectPair("shaped").host.run_completed &&
          collectPair("shaped").guest.run_completed;
      }, duration + 15000);
      suiteComplete = true;
    } catch (error) {
      suiteError = String(error);
    }
    report();
  };

  window.GoliseoWebRTCProofSuite = { diagnostics: report };
  report();
  window.setInterval(report, 1000);
})();
