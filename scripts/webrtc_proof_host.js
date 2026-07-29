/* OMP-0 manual WebRTC proof host. The generated player embeds this source. */
(function () {
  "use strict";

  var VERSION = 1;
  var MAX_HISTORY = 6;
  var MAX_MESSAGE_BYTES = 512;
  var MAX_INPUT_BYTES = 128;
  var MAX_QUEUE_DEPTH = 64;
  var INPUT_SIZE_ESTIMATE = 160;

  function now() {
    return window.performance ? window.performance.now() : Date.now();
  }

  function encode(value) {
    return encodeURIComponent(String(value));
  }

  function decode(value) {
    try {
      return decodeURIComponent(value);
    } catch (error) {
      return null;
    }
  }

  function byteLength(value) {
    if (window.TextEncoder) {
      return new window.TextEncoder().encode(value).length;
    }
    return unescape(encodeURIComponent(value)).length;
  }

  function wire(type, seq, tick, payload) {
    return [
      VERSION,
      encode(type),
      seq,
      tick === null || tick === undefined ? "" : tick,
      encode(payload)
    ].join("|");
  }

  function parseWire(value) {
    if (typeof value !== "string") {
      return { code: "malformed", detail: "message is not a string" };
    }
    var fields = value.split("|");
    if (fields.length !== 5 || Number(fields[0]) !== VERSION) {
      return { code: "unsupported_version", detail: "message protocol version is unsupported" };
    }
    var type = decode(fields[1]);
    var payload = decode(fields[4]);
    if (!type || payload === null || (type !== "event" && type !== "input")) {
      return { code: "malformed", detail: "message envelope is malformed" };
    }
    var seq = Number(fields[2]);
    var tick = fields[3] === "" ? null : Number(fields[3]);
    if (!Number.isSafeInteger(seq) || seq < 0 || (tick !== null && (!Number.isSafeInteger(tick) || tick < 0))) {
      return { code: "malformed", detail: "message sequence or tick is invalid" };
    }
    if (type === "input" && tick === null) {
      return { code: "malformed", detail: "input message is missing its tick" };
    }
    return { type: type, seq: seq, tick: tick, payload: payload };
  }

  function buildId() {
    return window.__GOLISEO__ && window.__GOLISEO__.build_id
      ? window.__GOLISEO__.build_id
      : "unknown";
  }

  function percentile(values, fraction) {
    if (values.length === 0) {
      return null;
    }
    var sorted = values.slice().sort(function (a, b) { return a - b; });
    return sorted[Math.max(0, Math.ceil(sorted.length * fraction) - 1)];
  }

  function maximum(values) {
    return values.length === 0 ? null : Math.max.apply(null, values);
  }

  function seededRandom(seed) {
    var state = Number(seed) || 1;
    return function () {
      state = (state * 1664525 + 1013904223) % 4294967296;
      return state / 4294967296;
    };
  }

  function Proof(options) {
    options = options || {};
    this.role = options.role || "guest";
    this.protocol_version = options.protocol_version || VERSION;
    this.build_id = options.build_id || buildId();
    this.queue_limit = options.queue_limit || MAX_QUEUE_DEPTH;
    this.network_profile = options.network_profile || "baseline";
    this.one_way_delay_ms = Math.max(0, Number(options.one_way_delay_ms) || 0);
    this.input_loss_percent = Math.max(0, Math.min(100, Number(options.input_loss_percent) || 0));
    this.state = "new";
    this.peer_role = null;
    this.handshake_complete = false;
    this.disconnect_reason = null;
    this.pc = null;
    this.control = null;
    this.input = null;
    this.control_seq = 0;
    this.input_seq = 0;
    this.tick = 0;
    this.history = [];
    this.input_timer = null;
    this.input_started_at = null;
    this.input_stopped_at = null;
    this.ping_timer = null;
    this.started_at = now();
    this.events = [];
    this.sent = 0;
    this.received = 0;
    this.unique_ticks = 0;
    this.history_retransmits = 0;
    this.sequence_gaps = 0;
    this.out_of_order = 0;
    this.dropped = 0;
    this.shaper_dropped = 0;
    this.shaper_pending = 0;
    this.max_shaper_pending = 0;
    this.last_received_seq = null;
    this.last_received_tick = null;
    this.seen_ticks = Object.create(null);
    this.rtt_samples = [];
    this.jitter_samples = [];
    this.pending_pings = Object.create(null);
    this.queue_depth_samples = [];
    this.max_queue_depth = 0;
    this.last_error = null;
    this._handshake_sent = false;
    this._last_rtt = null;
    this._random = seededRandom(options.loss_seed);
  }

  Proof.prototype.record = function (kind, fields) {
    var event = { kind: kind, at_ms: now() - this.started_at };
    fields = fields || {};
    Object.keys(fields).forEach(function (key) { event[key] = fields[key]; });
    if (this.events.length >= this.queue_limit) {
      this.events.shift();
    }
    this.events.push(event);
    var parts = ["GC_WEBRTC", kind];
    Object.keys(fields).sort().forEach(function (key) {
      parts.push(key + "=" + encode(fields[key]));
    });
    if (window.console && window.console.info) {
      window.console.info(parts.join("|"));
    }
  };

  Proof.prototype.fail = function (code, detail) {
    if (!this.last_error) {
      this.last_error = { code: code, detail: detail };
    }
    this.state = "error";
    this.record("error", { code: code, detail: detail });
    if (this.pc && this.pc.connectionState !== "closed") {
      this.pc.close();
    }
    return { ok: false, code: code, detail: detail };
  };

  Proof.prototype.ensurePeer = function () {
    if (!window.RTCPeerConnection) {
      this.fail("unsupported", "RTCPeerConnection is not available in this browser");
      throw new Error("RTCPeerConnection is not available");
    }
    if (this.pc) {
      return this.pc;
    }
    this.pc = new window.RTCPeerConnection({ iceServers: [] });
    this.state = "connecting";
    this.pc.oniceconnectionstatechange = function () {
      this.record("ice_state", { state: this.pc.iceConnectionState });
      if (this.pc.iceConnectionState === "failed") {
        this.fail("ice_failed", "ICE negotiation failed");
      }
    }.bind(this);
    this.pc.onconnectionstatechange = function () {
      this.record("connection_state", { state: this.pc.connectionState });
      if (this.pc.connectionState === "connected") {
        this.state = "connected";
      } else if (this.pc.connectionState === "disconnected") {
        this.disconnect_reason = "peer_disconnected";
        this.state = "disconnected";
      } else if (this.pc.connectionState === "failed") {
        this.fail("connection_failed", "peer connection failed");
      }
    }.bind(this);
    this.pc.ondatachannel = function (event) {
      this.attachChannel(event.channel);
    }.bind(this);
    return this.pc;
  };

  Proof.prototype.attachChannel = function (channel) {
    if (channel.label === "control") {
      this.control = channel;
    } else if (channel.label === "input") {
      this.input = channel;
    } else {
      channel.close();
      return;
    }
    channel.onopen = function () {
      this.record("channel_open", { channel: channel.label });
      if (channel.label === "control") {
        this.sendHandshake();
        this.startPings();
      }
      if (this.control && this.input && this.control.readyState === "open" && this.input.readyState === "open") {
        this.state = "connected";
      }
    }.bind(this);
    channel.onclose = function () {
      this.disconnect_reason = channel.label + "_channel_closed";
      this.state = "disconnected";
      this.record("channel_close", { channel: channel.label });
    }.bind(this);
    channel.onerror = function () {
      this.fail("channel_error", channel.label + " data channel error");
    }.bind(this);
    channel.onmessage = function (event) {
      if (channel.label === "control") {
        this.receiveControl(event.data);
      } else {
        this.receiveInput(event.data);
      }
    }.bind(this);
  };

  Proof.prototype.waitForIce = function () {
    if (this.pc.iceGatheringState === "complete") {
      return Promise.resolve(this.pc.localDescription);
    }
    return new Promise(function (resolve) {
      var done = false;
      var finish = function () {
        if (!done) {
          done = true;
          resolve(this.pc.localDescription);
        }
      }.bind(this);
      this.pc.onicegatheringstatechange = function () {
        if (this.pc.iceGatheringState === "complete") {
          finish();
        }
      }.bind(this);
      window.setTimeout(finish, 5000);
    }.bind(this));
  };

  Proof.prototype.createOffer = async function () {
    if (this.role !== "host") {
      return this.fail("role_mismatch", "only the host creates the offer");
    }
    var pc = this.ensurePeer();
    this.attachChannel(pc.createDataChannel("control", { ordered: true }));
    this.attachChannel(pc.createDataChannel("input", { ordered: false, maxRetransmits: 0 }));
    var offer = await pc.createOffer();
    await pc.setLocalDescription(offer);
    await this.waitForIce();
    this.record("offer_ready", { role: this.role });
    return JSON.parse(JSON.stringify(pc.localDescription));
  };

  Proof.prototype.acceptOffer = async function (offer) {
    if (this.role !== "guest") {
      return this.fail("role_mismatch", "only the guest accepts the offer");
    }
    var pc = this.ensurePeer();
    try {
      await pc.setRemoteDescription(offer);
      var answer = await pc.createAnswer();
      await pc.setLocalDescription(answer);
      await this.waitForIce();
      this.record("answer_ready", { role: this.role });
      return JSON.parse(JSON.stringify(pc.localDescription));
    } catch (error) {
      return this.fail("signaling_error", String(error));
    }
  };

  Proof.prototype.acceptAnswer = async function (answer) {
    if (this.role !== "host" || !this.pc) {
      return this.fail("role_mismatch", "only the host accepts the answer after creating an offer");
    }
    try {
      await this.pc.setRemoteDescription(answer);
      this.record("answer_accepted", { role: this.role });
      return { ok: true };
    } catch (error) {
      return this.fail("signaling_error", String(error));
    }
  };

  Proof.prototype.sendControl = function (message) {
    if (!this.control || this.control.readyState !== "open") {
      return false;
    }
    if (this.control.bufferedAmount > this.queue_limit * 256 || this.shaper_pending >= this.queue_limit) {
      this.dropped += 1;
      this.record("queue_overflow", { channel: "control" });
      return false;
    }
    var value = wire("event", this.control_seq++, null, JSON.stringify(message));
    if (byteLength(value) > MAX_MESSAGE_BYTES) {
      this.dropped += 1;
      this.record("control_rejected", { reason: "message_too_large" });
      return false;
    }
    this.sendShaped(this.control, value, false);
    return true;
  };

  Proof.prototype.sendShaped = function (channel, value, lossy) {
    if (lossy && this.input_loss_percent > 0 && this._random() * 100 < this.input_loss_percent) {
      this.shaper_dropped += 1;
      return false;
    }
    if (this.one_way_delay_ms <= 0) {
      channel.send(value);
      return true;
    }
    this.shaper_pending += 1;
    this.max_shaper_pending = Math.max(this.max_shaper_pending, this.shaper_pending);
    window.setTimeout(function () {
      this.shaper_pending = Math.max(0, this.shaper_pending - 1);
      if (channel.readyState === "open") {
        channel.send(value);
      } else {
        this.dropped += 1;
      }
    }.bind(this), this.one_way_delay_ms);
    return true;
  };

  Proof.prototype.sendHandshake = function () {
    if (this._handshake_sent) {
      return;
    }
    this._handshake_sent = this.sendControl({
      kind: "handshake",
      version: this.protocol_version,
      build_id: this.build_id,
      role: this.role
    });
  };

  Proof.prototype.reject = function (code, detail) {
    this.sendControl({ kind: "reject", code: code, detail: detail });
    return this.fail(code, detail);
  };

  Proof.prototype.receiveControl = function (value) {
    if (typeof value !== "string" || byteLength(value) > MAX_MESSAGE_BYTES) {
      this.reject("message_too_large", "control message exceeds 512 bytes");
      return;
    }
    var message = parseWire(value);
    if (message.code) {
      this.reject(message.code, message.detail);
      return;
    }
    var payload;
    try {
      payload = JSON.parse(message.payload);
    } catch (error) {
      this.reject("malformed", "control payload is not JSON");
      return;
    }
    if (payload.kind === "handshake") {
      if (payload.version !== this.protocol_version) {
        this.reject("unsupported_version", "peer protocol version does not match");
        return;
      }
      if (payload.build_id !== this.build_id) {
        this.reject("build_mismatch", "peer build identity does not match: " + payload.build_id);
        return;
      }
      if ((this.role === "host" && payload.role !== "guest") || (this.role === "guest" && payload.role !== "host")) {
        this.reject("role_mismatch", "peer role does not complement local role");
        return;
      }
      this.peer_role = payload.role;
      this.handshake_complete = true;
      this.state = "connected";
      this.sendControl({ kind: "handshake_ack", version: this.protocol_version, build_id: this.build_id });
      this.record("handshake_ok", { peer_role: payload.role });
    } else if (payload.kind === "handshake_ack") {
      if (payload.version !== this.protocol_version || payload.build_id !== this.build_id) {
        this.reject("build_mismatch", "peer handshake acknowledgement does not match");
        return;
      }
      this.handshake_complete = true;
      this.state = "connected";
      this.record("handshake_ack", {});
    } else if (payload.kind === "ping") {
      this.sendControl({ kind: "pong", id: payload.id, sent_at: payload.sent_at });
    } else if (payload.kind === "pong" && this.pending_pings[payload.id]) {
      var rtt = now() - this.pending_pings[payload.id];
      delete this.pending_pings[payload.id];
      this.rtt_samples.push(rtt);
      if (this._last_rtt !== null) {
        this.jitter_samples.push(Math.abs(rtt - this._last_rtt));
      }
      this._last_rtt = rtt;
      this.record("rtt", { rtt_ms: rtt.toFixed(3) });
    } else if (payload.kind === "reject") {
      this.fail(payload.code || "rejected", payload.detail || "peer rejected the handshake");
    } else {
      this.reject("malformed", "control message kind is unsupported");
    }
  };

  Proof.prototype.receiveInput = function (value) {
    if (!this.handshake_complete) {
      this.record("input_rejected", { reason: "handshake_incomplete" });
      return;
    }
    if (typeof value !== "string" || byteLength(value) > MAX_MESSAGE_BYTES) {
      this.record("input_rejected", { reason: "message_too_large" });
      return;
    }
    var message = parseWire(value);
    if (message.code || message.type !== "input" || byteLength(message.payload) > MAX_INPUT_BYTES) {
      this.record("input_rejected", { reason: message.code || "input_too_large" });
      return;
    }
    var payload;
    try {
      payload = JSON.parse(message.payload);
    } catch (error) {
      this.record("input_rejected", { reason: "malformed" });
      return;
    }
    var history = Array.isArray(payload.history) ? payload.history : [];
    if (history.length > MAX_HISTORY || message.tick === null || history.some(function (history_tick) {
      return !Number.isSafeInteger(history_tick) || history_tick < 0 || history_tick >= message.tick;
    })) {
      this.record("input_rejected", { reason: "malformed" });
      return;
    }
    this.received += 1;
    if (this.last_received_seq !== null) {
      if (message.seq > this.last_received_seq + 1) {
        this.sequence_gaps += message.seq - this.last_received_seq - 1;
      } else if (message.seq <= this.last_received_seq) {
        this.out_of_order += 1;
      }
    }
    this.last_received_seq = Math.max(this.last_received_seq === null ? message.seq : this.last_received_seq, message.seq);
    history.forEach(function (history_tick) {
      if (Number.isSafeInteger(history_tick) && history_tick >= 0 && !this.seen_ticks[history_tick]) {
        this.seen_ticks[history_tick] = true;
        this.unique_ticks += 1;
        this.history_retransmits += 1;
      }
    }.bind(this));
    if (!this.seen_ticks[message.tick]) {
      this.seen_ticks[message.tick] = true;
      this.unique_ticks += 1;
    }
    this.last_received_tick = this.last_received_tick === null
      ? message.tick
      : Math.max(this.last_received_tick, message.tick);
  };

  Proof.prototype.startPings = function () {
    if (this.ping_timer) {
      return;
    }
    this.ping_timer = window.setInterval(function () {
      if (!this.handshake_complete) {
        return;
      }
      var id = String(this.control_seq++);
      this.pending_pings[id] = now();
      this.sendControl({ kind: "ping", id: id, sent_at: this.pending_pings[id] });
    }.bind(this), 1000);
  };

  Proof.prototype.startInput = function (options) {
    options = options || {};
    var hz = options.hz || 60;
    var duration = options.duration_ms || 0;
    if (hz <= 0 || hz > 60) {
      return this.fail("malformed", "input cadence must be between 1 and 60 Hz");
    }
    if (!this.handshake_complete || !this.input || this.input.readyState !== "open") {
      return this.fail("not_connected", "input traffic requires an open handshaken channel");
    }
    this.stopInput();
    this.input_started_at = now();
    this.input_stopped_at = null;
    this.queue_depth_samples = [];
    var period = 1000 / hz;
    var next_tick_at = now() + period;
    var schedule_next = function () {
      next_tick_at += period;
      this.input_timer = window.setTimeout(
        send_tick,
        Math.max(0, next_tick_at - now())
      );
    }.bind(this);
    var send_tick = function () {
      if (!this.input || this.input.readyState !== "open") {
        this.input_timer = null;
        this.input_stopped_at = now();
        return;
      }
      var queue_depth = Math.ceil(this.input.bufferedAmount / INPUT_SIZE_ESTIMATE) + this.shaper_pending;
      this.queue_depth_samples.push(queue_depth);
      this.max_queue_depth = Math.max(this.max_queue_depth, queue_depth);
      if (queue_depth >= this.queue_limit || queue_depth >= MAX_QUEUE_DEPTH) {
        this.dropped += 1;
        this.record("queue_overflow", { channel: "input", depth: queue_depth });
        schedule_next();
        return;
      }
      this.tick += 1;
      var history = this.history.slice(-MAX_HISTORY);
      var payload = JSON.stringify({ kind: "input", history: history, state: 0 });
      if (byteLength(payload) > MAX_INPUT_BYTES) {
        this.dropped += 1;
        this.record("input_rejected", { reason: "input_too_large" });
        schedule_next();
        return;
      }
      this.sendShaped(
        this.input,
        wire("input", this.input_seq++, this.tick, payload),
        true
      );
      this.sent += 1;
      this.history.push(this.tick);
      if (this.history.length > MAX_HISTORY) {
        this.history.shift();
      }
      schedule_next();
    }.bind(this);
    this.input_timer = window.setTimeout(send_tick, period);
    this.record("input_started", { hz: hz, duration_ms: duration });
    if (duration > 0) {
      window.setTimeout(function () { this.stopInput(); }.bind(this), duration);
    }
    return { ok: true, hz: hz, duration_ms: duration };
  };

  Proof.prototype.stopInput = function () {
    if (this.input_timer) {
      window.clearTimeout(this.input_timer);
      this.input_timer = null;
      this.input_stopped_at = now();
      this.record("input_stopped", {});
    }
  };

  Proof.prototype.waitForHandshake = function (timeout_ms) {
    timeout_ms = timeout_ms || 10000;
    return new Promise(function (resolve, reject) {
      var started = now();
      var check = window.setInterval(function () {
        if (this.handshake_complete) {
          window.clearInterval(check);
          resolve(this.diagnostics());
        } else if (this.last_error) {
          window.clearInterval(check);
          reject(this.last_error);
        } else if (now() - started > timeout_ms) {
          window.clearInterval(check);
          reject({ code: "timeout", detail: "WebRTC proof handshake timed out" });
        }
      }.bind(this), 50);
    }.bind(this));
  };

  Proof.prototype.diagnostics = function () {
    var elapsed = Math.max((now() - this.started_at) / 1000, 0.001);
    var input_elapsed = this.input_started_at === null
      ? 0
      : Math.max(((this.input_stopped_at || now()) - this.input_started_at) / 1000, 0.001);
    var queue_depth = this.input
      ? Math.ceil(this.input.bufferedAmount / INPUT_SIZE_ESTIMATE) + this.shaper_pending
      : this.shaper_pending;
    return {
      state: this.state,
      role: this.role,
      peer_role: this.peer_role,
      protocol_version: this.protocol_version,
      build_id: this.build_id,
      network_profile: this.network_profile,
      one_way_delay_ms: this.one_way_delay_ms,
      input_loss_percent: this.input_loss_percent,
      handshake_complete: this.handshake_complete,
      duration_s: elapsed,
      input_duration_s: input_elapsed,
      send_rate: input_elapsed > 0 ? this.sent / input_elapsed : 0,
      receive_rate: input_elapsed > 0 ? this.received / input_elapsed : 0,
      sent: this.sent,
      received: this.received,
      unique_ticks: this.unique_ticks,
      history_retransmits: this.history_retransmits,
      sequence_gaps: this.sequence_gaps,
      out_of_order: this.out_of_order,
      dropped: this.dropped,
      shaper_dropped: this.shaper_dropped,
      shaper_pending: this.shaper_pending,
      max_shaper_pending: this.max_shaper_pending,
      queue_depth: queue_depth,
      queue_p50: percentile(this.queue_depth_samples, 0.50),
      queue_p95: percentile(this.queue_depth_samples, 0.95),
      max_queue_depth: Math.max(this.max_queue_depth, queue_depth),
      rtt_p50_ms: percentile(this.rtt_samples, 0.50),
      rtt_p95_ms: percentile(this.rtt_samples, 0.95),
      rtt_max_ms: maximum(this.rtt_samples),
      jitter_p50_ms: percentile(this.jitter_samples, 0.50),
      jitter_p95_ms: percentile(this.jitter_samples, 0.95),
      jitter_max_ms: maximum(this.jitter_samples),
      disconnect_reason: this.disconnect_reason,
      last_error: this.last_error,
      events: this.events.slice()
    };
  };

  Proof.prototype.disconnect = function (reason) {
    this.stopInput();
    if (this.ping_timer) {
      window.clearInterval(this.ping_timer);
      this.ping_timer = null;
    }
    this.disconnect_reason = reason || "manual_disconnect";
    if (this.pc) {
      this.pc.close();
    }
    this.state = "disconnected";
    this.record("disconnect", { reason: this.disconnect_reason });
    return this.diagnostics();
  };

  Proof.prototype.create_offer = Proof.prototype.createOffer;
  Proof.prototype.accept_offer = Proof.prototype.acceptOffer;
  Proof.prototype.accept_answer = Proof.prototype.acceptAnswer;
  Proof.prototype.start_input = Proof.prototype.startInput;
  Proof.prototype.stop_input = Proof.prototype.stopInput;
  Proof.prototype.wait_for_handshake = Proof.prototype.waitForHandshake;

  window.GoliseoWebRTCProof = window.GoliseoWebRTCProof || {
    VERSION: VERSION,
    create: function (options) { return new Proof(options); },
    encode: wire,
    decode: parseWire
  };
})();
