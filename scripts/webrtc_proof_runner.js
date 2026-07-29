/* Browser-visible OMP-0 WebRTC proof controls. */
(function () {
  "use strict";

  var query = new URL(window.location.href).searchParams;
  var role = query.get("role") === "host" ? "host" : "guest";
  var profile = query.get("profile") === "shaped" ? "shaped" : "baseline";
  var duration = Math.max(1000, Number(query.get("duration_ms")) || 600000);
  var protocolVersion = Number(query.get("protocol_version")) || 1;
  var buildId = query.get("build_id") || window.__GOLISEO__.build_id;
  var profileOptions = profile === "shaped"
    ? { one_way_delay_ms: 50, input_loss_percent: 1, loss_seed: role === "host" ? 17 : 29 }
    : {};
  var proof = window.GoliseoWebRTCProof.create(Object.assign({
    role: role,
    protocol_version: protocolVersion,
    build_id: buildId,
    network_profile: profile
  }, profileOptions));
  var runStartedAt = null;
  var runCompletedAt = null;
  var runCompleted = false;

  var status = document.getElementById("status");
  var roleValue = document.getElementById("role-value");
  var profileValue = document.getElementById("profile-value");
  var durationValue = document.getElementById("duration-value");
  var signalInput = document.getElementById("signal-input");
  var signalOutput = document.getElementById("signal-output");
  var diagnostics = document.getElementById("diagnostics");
  var createOffer = document.getElementById("create-offer");
  var acceptOffer = document.getElementById("accept-offer");
  var acceptAnswer = document.getElementById("accept-answer");
  var startTraffic = document.getElementById("start-traffic");
  var stopTraffic = document.getElementById("stop-traffic");

  function setStatus(value) {
    status.textContent = value;
    status.dataset.state = value;
  }

  function parseSignal() {
    try {
      return JSON.parse(signalInput.value);
    } catch (error) {
      setStatus("invalid-signal");
      throw error;
    }
  }

  function refresh() {
    var report = proof.diagnostics();
    report.run_completed = runCompleted;
    report.requested_duration_ms = duration;
    report.runner_elapsed_ms = runStartedAt === null
      ? 0
      : (runCompletedAt || Date.now()) - runStartedAt;
    diagnostics.textContent = JSON.stringify(report, null, 2);
    startTraffic.disabled = !proof.handshake_complete || proof.input_timer !== null;
    stopTraffic.disabled = proof.input_timer === null;
    if (proof.last_error) {
      setStatus("error:" + proof.last_error.code);
    } else if (runCompleted) {
      setStatus("complete");
    } else if (proof.input_timer) {
      setStatus("running");
    } else if (proof.handshake_complete) {
      setStatus("handshake-complete");
    }
    return report;
  }

  async function createLocalOffer() {
    setStatus("creating-offer");
    var offer = await proof.create_offer();
    if (offer && offer.ok === false) {
      refresh();
      return offer;
    }
    signalOutput.value = JSON.stringify(offer);
    setStatus("offer-ready");
    return offer;
  }

  async function acceptRemoteOffer(offer) {
    setStatus("accepting-offer");
    var answer = await proof.accept_offer(offer || parseSignal());
    if (answer && answer.ok === false) {
      refresh();
      return answer;
    }
    signalOutput.value = JSON.stringify(answer);
    setStatus("answer-ready");
    return answer;
  }

  async function acceptRemoteAnswer(answer) {
    setStatus("accepting-answer");
    var result = await proof.accept_answer(answer || parseSignal());
    refresh();
    return result;
  }

  function beginTraffic() {
    runStartedAt = Date.now();
    runCompletedAt = null;
    runCompleted = false;
    var result = proof.start_input({ hz: 60, duration_ms: duration });
    window.setTimeout(function () {
      runCompletedAt = Date.now();
      runCompleted = true;
      refresh();
    }, duration + 100);
    refresh();
    return result;
  }

  function endTraffic() {
    proof.stop_input();
    runCompletedAt = Date.now();
    return refresh();
  }

  createOffer.onclick = createLocalOffer;
  acceptOffer.onclick = function () { return acceptRemoteOffer(); };
  acceptAnswer.onclick = function () { return acceptRemoteAnswer(); };
  startTraffic.onclick = beginTraffic;
  stopTraffic.onclick = endTraffic;

  window.GoliseoWebRTCProofRunner = {
    create_offer: createLocalOffer,
    accept_offer: acceptRemoteOffer,
    accept_answer: acceptRemoteAnswer,
    start_traffic: beginTraffic,
    stop_traffic: endTraffic,
    diagnostics: refresh
  };

  roleValue.textContent = role;
  profileValue.textContent = profile;
  durationValue.textContent = String(duration);
  createOffer.hidden = role !== "host";
  acceptAnswer.hidden = role !== "host";
  acceptOffer.hidden = role !== "guest";
  setStatus("ready");
  refresh();
  window.setInterval(refresh, 1000);
})();
