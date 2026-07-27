/* OMP-3 browser WebRTC host-star transport. The generated player embeds this source. */
(function () {
  "use strict";

  var VERSION = 1;
  var MAX_GUESTS = 7;
  var HOST_PEER_ID = "host";
  var DEFAULT_QUEUE_LIMIT = 64;
  var MAX_QUEUE_LIMIT = 256;
  var MAX_PAYLOAD_BYTES = 1024;
  var DEFAULT_BUFFERED_AMOUNT_LIMIT = 65536;
  var MAX_BUFFERED_AMOUNT_LIMIT = 1048576;
  var MAX_PEER_ID_BYTES = 128;
  var MAX_SIGNAL_BYTES = 65536;
  var ICE_TIMEOUT_MS = 5000;
  var PEER_ID_PATTERN = /^[0-9A-Za-z][0-9A-Za-z_-]*$/;
  var CHANNEL_ORDER = ["control", "input"];
  var CHANNEL_MESSAGE_TYPES = {
    control: { event: true, state: true },
    input: { input: true }
  };
  /* Channel configuration is part of the transport contract: control is
   * reliable and ordered, input is unordered and never retransmitted. */
  var CHANNEL_CONFIG = {
    control: { ordered: true },
    input: { ordered: false, maxRetransmits: 0 }
  };

  window.GalacticCupStarTransport = window.GalacticCupStarTransport || (function () {
    var star = {
      role: "host",
      state: "new",
      queue_limit: DEFAULT_QUEUE_LIMIT,
      event_limit: Math.max(2, DEFAULT_QUEUE_LIMIT),
      max_guests: MAX_GUESTS,
      buffered_amount_limit: DEFAULT_BUFFERED_AMOUNT_LIMIT,
      peers: Object.create(null),
      order: [],
      events: [],
      cursor: 0,
      sent: 0,
      received: 0,
      dropped_outbound: 0,
      dropped_inbound: 0,
      malformed: 0,
      unsupported_version: 0,
      overflow: 0,
      backpressure: 0,
      last_error: ""
    };

    function byteLength(value) {
      if (window.TextEncoder) {
        return new window.TextEncoder().encode(value).length;
      }
      return unescape(encodeURIComponent(value)).length;
    }

    function escapeField(value) {
      return encodeURIComponent(String(value));
    }

    function unescapeField(value) {
      try {
        return decodeURIComponent(String(value));
      } catch (error) {
        return null;
      }
    }

    function pushEvent(line) {
      if (star.events.length >= star.event_limit) {
        star.events.shift();
        star.overflow += 1;
        star.last_error = "star event queue is full";
      }
      star.events.push(line);
    }

    function starState(next) {
      star.state = next;
      pushEvent("star_state|" + next);
    }

    function peerState(peer, next) {
      peer.state = next;
      CHANNEL_ORDER.forEach(function (name) {
        peer.channels[name].state = next;
      });
      pushEvent("peer_state|" + peer.id + "|" + next);
    }

    function countError(code, peer) {
      if (code === "malformed" || code === "payload_too_large" || code === "channel_mismatch") {
        star.malformed += 1;
        if (peer) {
          peer.malformed += 1;
        }
      } else if (code === "unsupported_version") {
        star.unsupported_version += 1;
      } else if (code === "overflow") {
        star.overflow += 1;
      } else if (code === "backpressure") {
        star.backpressure += 1;
        if (peer) {
          peer.backpressure += 1;
        }
      }
    }

    function starError(code, detail) {
      star.last_error = detail || code;
      countError(code, null);
      pushEvent("star_error|" + code + "|" + escapeField(star.last_error));
      return "error|" + code + "|" + escapeField(star.last_error);
    }

    function peerError(peer, channelName, code, detail) {
      star.last_error = detail || code;
      peer.last_error = star.last_error;
      countError(code, peer);
      pushEvent(
        "peer_error|" + peer.id + "|" + (channelName || "") + "|" + code + "|" +
          escapeField(star.last_error)
      );
      return "error|" + code + "|" + escapeField(star.last_error);
    }

    function newChannel() {
      return {
        state: "connecting",
        handle: null,
        outbound: [],
        inbound: [],
        backpressured: false,
        sent: 0,
        received: 0,
        dropped_outbound: 0,
        dropped_inbound: 0,
        last_seq: null
      };
    }

    function newPeer(id, slot) {
      return {
        id: id,
        slot: slot,
        state: "connecting",
        ice_state: "new",
        pc: null,
        channels: { control: newChannel(), input: newChannel() },
        signal: null,
        sequence_gaps: 0,
        backpressure: 0,
        malformed: 0,
        last_error: ""
      };
    }

    function bufferedAmount(channel) {
      return channel.handle && typeof channel.handle.bufferedAmount === "number"
        ? channel.handle.bufferedAmount
        : 0;
    }

    /* Envelope parsing mirrors game/transport/contract.lua. Payload bytes stay
     * opaque; this bridge never inspects or rewrites them. */
    function parseWire(wire) {
      if (typeof wire !== "string") {
        return { error: "malformed", detail: "wire is not a string" };
      }
      var fields = wire.split("|");
      if (fields.length !== 5) {
        return { error: "malformed", detail: "wire has invalid fields" };
      }
      if (!/^(0|[1-9][0-9]*)$/.test(fields[0])) {
        return { error: "malformed", detail: "wire version is invalid" };
      }
      if (Number(fields[0]) !== VERSION) {
        return { error: "unsupported_version", detail: "wire version is unsupported" };
      }
      var type = unescapeField(fields[1]);
      if (type !== "input" && type !== "event" && type !== "state") {
        return { error: "malformed", detail: "wire type is invalid" };
      }
      if (!/^(0|[1-9][0-9]*)$/.test(fields[2])) {
        return { error: "malformed", detail: "wire sequence is invalid" };
      }
      var tick = null;
      if (fields[3] !== "") {
        if (!/^(0|[1-9][0-9]*)$/.test(fields[3])) {
          return { error: "malformed", detail: "wire tick is invalid" };
        }
        tick = Number(fields[3]);
      }
      if (type === "input" && tick === null) {
        return { error: "malformed", detail: "input wire is missing its tick" };
      }
      var payload = unescapeField(fields[4]);
      if (payload === null) {
        return { error: "malformed", detail: "wire payload escape is invalid" };
      }
      if (byteLength(payload) > MAX_PAYLOAD_BYTES) {
        return { error: "payload_too_large", detail: "wire payload exceeds the byte limit" };
      }
      return { type: type, seq: Number(fields[2]), tick: tick };
    }

    function checkChannelMessage(channelName, parsed) {
      if (!CHANNEL_MESSAGE_TYPES[channelName]) {
        return { error: "channel_mismatch", detail: "channel must be control or input" };
      }
      if (!CHANNEL_MESSAGE_TYPES[channelName][parsed.type]) {
        return {
          error: "channel_mismatch",
          detail: "the " + channelName + " channel does not carry " + parsed.type + " messages"
        };
      }
      return null;
    }

    function validPeerId(id) {
      return typeof id === "string" &&
        id.length > 0 &&
        byteLength(id) <= MAX_PEER_ID_BYTES &&
        PEER_ID_PATTERN.test(id);
    }

    /* Sending is refused only when the bounded queue is full. A saturated send
     * buffer is backpressure: the message stays queued and drains later, so no
     * call blocks the LOVE frame loop. */
    function drain(peer, channelName) {
      var channel = peer.channels[channelName];
      var handle = channel.handle;
      if (!handle || handle.readyState !== "open") {
        return;
      }
      while (channel.outbound.length > 0) {
        var wire = channel.outbound[0];
        if (bufferedAmount(channel) + byteLength(wire) > star.buffered_amount_limit) {
          /* Latch the report so a saturated buffer costs one event, not one
           * per drain attempt. It clears as soon as the channel moves again. */
          if (!channel.backpressured) {
            channel.backpressured = true;
            peerError(peer, channelName, "backpressure", "channel send buffer is full");
          }
          return;
        }
        channel.outbound.shift();
        channel.backpressured = false;
        try {
          handle.send(wire);
        } catch (error) {
          channel.dropped_outbound += 1;
          star.dropped_outbound += 1;
          peerError(peer, channelName, "bridge_error", String(error));
          return;
        }
      }
    }

    function drainAll() {
      star.order.forEach(function (id) {
        var peer = star.peers[id];
        CHANNEL_ORDER.forEach(function (name) {
          drain(peer, name);
        });
      });
    }

    function enqueue(peer, channelName, wire) {
      var parsed = parseWire(wire);
      if (parsed.error) {
        return peerError(peer, channelName, parsed.error, parsed.detail);
      }
      var mismatch = checkChannelMessage(channelName, parsed);
      if (mismatch) {
        return peerError(peer, channelName, mismatch.error, mismatch.detail);
      }
      if (peer.state !== "connected") {
        return peerError(peer, channelName, "not_connected", "peer link is not connected");
      }
      var channel = peer.channels[channelName];
      if (channel.outbound.length >= star.queue_limit) {
        channel.dropped_outbound += 1;
        star.dropped_outbound += 1;
        return peerError(peer, channelName, "overflow", "peer outbound queue is full");
      }
      channel.outbound.push(wire);
      channel.sent += 1;
      star.sent += 1;
      drain(peer, channelName);
      return "ok";
    }

    function receive(peer, channelName, wire) {
      var channel = peer.channels[channelName];
      var parsed = parseWire(wire);
      if (parsed.error) {
        channel.dropped_inbound += 1;
        star.dropped_inbound += 1;
        peerError(peer, channelName, parsed.error, parsed.detail);
        return;
      }
      var mismatch = checkChannelMessage(channelName, parsed);
      if (mismatch) {
        channel.dropped_inbound += 1;
        star.dropped_inbound += 1;
        peerError(peer, channelName, mismatch.error, mismatch.detail);
        return;
      }
      if (channel.inbound.length >= star.queue_limit) {
        channel.dropped_inbound += 1;
        star.dropped_inbound += 1;
        peerError(peer, channelName, "overflow", "peer inbound queue is full");
        return;
      }
      if (channel.last_seq !== null && parsed.seq > channel.last_seq + 1) {
        peer.sequence_gaps += parsed.seq - channel.last_seq - 1;
      }
      if (channel.last_seq === null || parsed.seq > channel.last_seq) {
        channel.last_seq = parsed.seq;
      }
      channel.inbound.push(peer.id + "|" + channelName + "|" + wire);
    }

    function attachChannel(peer, handle) {
      var name = handle.label;
      if (name !== "control" && name !== "input") {
        handle.close();
        peerError(peer, null, "channel_mismatch", "unexpected data channel label");
        return;
      }
      var channel = peer.channels[name];
      /* Exactly two data channels per link, for the life of the link. Either
       * side of an established connection may call createDataChannel at any
       * time; a second channel on an occupied label would re-register
       * listeners without bound and silently take over the reference this
       * bridge sends through. Refuse it and close it. */
      if (channel.handle) {
        try {
          handle.close();
        } catch (error) {
          star.last_error = String(error);
        }
        peerError(peer, name, "channel_mismatch", "duplicate " + name + " data channel refused");
        return;
      }
      channel.handle = handle;
      handle.binaryType = "arraybuffer";
      /* Resume draining well before the buffer empties, so backpressure costs
       * latency rather than a stalled channel. */
      handle.bufferedAmountLowThreshold = Math.floor(star.buffered_amount_limit / 2);
      handle.onopen = function () {
        channel.state = "connected";
        if (
          peer.channels.control.state === "connected" &&
          peer.channels.input.state === "connected" &&
          peer.state !== "connected"
        ) {
          peerState(peer, "connected");
        }
        drain(peer, name);
      };
      handle.onbufferedamountlow = function () {
        drain(peer, name);
      };
      handle.onclose = function () {
        channel.state = "disconnected";
        if (peer.state === "connected") {
          peerState(peer, "disconnected");
          peerError(peer, name, "disconnected", name + " data channel closed");
        }
      };
      handle.onerror = function () {
        peerError(peer, name, "bridge_error", name + " data channel error");
      };
      handle.onmessage = function (event) {
        receive(peer, name, typeof event.data === "string" ? event.data : "");
      };
    }

    function ensurePeerConnection(peer) {
      if (peer.pc) {
        return peer.pc;
      }
      if (!window.RTCPeerConnection) {
        peerError(peer, null, "bridge_unavailable", "RTCPeerConnection is not available");
        return null;
      }
      /* OMP-3 stays on manual signaling with no configured ICE servers.
       * Production STUN/TURN selection belongs to OMP-4. */
      var pc = new window.RTCPeerConnection({ iceServers: [] });
      peer.pc = pc;
      /* These handlers close over `pc`, never `peer.pc`. RTCPeerConnection
       * .close() fires the state-change callbacks on a later task, by which
       * time closePeer has already set peer.pc to null. */
      pc.oniceconnectionstatechange = function () {
        peer.ice_state = pc.iceConnectionState;
        if (peer.ice_state === "failed") {
          peerError(peer, null, "disconnected", "ICE negotiation failed");
          peerState(peer, "error");
        }
      };
      pc.onconnectionstatechange = function () {
        var connection = pc.connectionState;
        if (connection === "failed" || connection === "closed") {
          if (peer.state !== "closed") {
            peerError(peer, null, "disconnected", "peer connection " + connection);
            peerState(peer, "disconnected");
          }
        } else if (connection === "disconnected" && peer.state === "connected") {
          peerError(peer, null, "disconnected", "peer connection disconnected");
          peerState(peer, "disconnected");
        }
      };
      pc.ondatachannel = function (event) {
        attachChannel(peer, event.channel);
      };
      return pc;
    }

    /* Detach every listener before closing, so a late callback from a closed
     * connection or channel cannot touch a torn-down peer at all. */
    function detach(target, names) {
      if (!target) {
        return;
      }
      names.forEach(function (name) {
        target[name] = null;
      });
    }

    function waitForIce(pc) {
      if (pc.iceGatheringState === "complete") {
        return Promise.resolve();
      }
      return new Promise(function (resolve) {
        var settled = false;
        var finish = function () {
          if (!settled) {
            settled = true;
            resolve();
          }
        };
        pc.onicegatheringstatechange = function () {
          if (pc.iceGatheringState === "complete") {
            finish();
          }
        };
        window.setTimeout(finish, ICE_TIMEOUT_MS);
      });
    }

    function storeSignal(peer, description) {
      var blob = JSON.stringify({ type: description.type, sdp: description.sdp });
      if (byteLength(blob) > MAX_SIGNAL_BYTES) {
        peerError(peer, null, "signal_error", "local signal exceeds the byte limit");
        return;
      }
      peer.signal = blob;
    }

    function parseSignal(peer, raw) {
      /* Bound the encoded length before decoding, so an oversized blob is
       * rejected without allocating its decoded form first. */
      if (typeof raw !== "string" || raw.length > MAX_SIGNAL_BYTES) {
        peerError(peer, null, "signal_error", "remote signal exceeds the byte limit");
        return null;
      }
      var text = unescapeField(raw);
      if (text === null || byteLength(text) > MAX_SIGNAL_BYTES) {
        peerError(peer, null, "signal_error", "remote signal is malformed");
        return null;
      }
      try {
        var value = JSON.parse(text);
        if (!value || (value.type !== "offer" && value.type !== "answer")) {
          peerError(peer, null, "signal_error", "remote signal is not an offer or answer");
          return null;
        }
        return value;
      } catch (error) {
        peerError(peer, null, "signal_error", "remote signal is not JSON");
        return null;
      }
    }

    function closePeer(peer, reason) {
      CHANNEL_ORDER.forEach(function (name) {
        var channel = peer.channels[name];
        star.dropped_outbound += channel.outbound.length;
        star.dropped_inbound += channel.inbound.length;
        channel.dropped_outbound += channel.outbound.length;
        channel.dropped_inbound += channel.inbound.length;
        channel.outbound = [];
        channel.inbound = [];
        if (channel.handle) {
          detach(channel.handle, [
            "onopen",
            "onclose",
            "onerror",
            "onmessage",
            "onbufferedamountlow"
          ]);
          try {
            channel.handle.close();
          } catch (error) {
            star.last_error = String(error);
          }
          channel.handle = null;
        }
      });
      if (peer.pc) {
        detach(peer.pc, [
          "oniceconnectionstatechange",
          "onconnectionstatechange",
          "onicegatheringstatechange",
          "ondatachannel"
        ]);
        try {
          peer.pc.close();
        } catch (error) {
          star.last_error = String(error);
        }
        peer.pc = null;
      }
      peer.ice_state = "closed";
      peer.signal = null;
      if (peer.state !== "closed") {
        peerState(peer, "closed");
        peerError(peer, null, "disconnected", reason || "peer closed");
      }
    }

    function requireConnected() {
      if (star.state === "new") {
        return starError("not_initialized", "star transport is not initialized");
      }
      if (star.state === "closed") {
        return starError("closed", "star transport is closed");
      }
      if (star.state !== "connected") {
        return starError("not_connected", "star transport is not connected");
      }
      return null;
    }

    function findPeer(id) {
      var peer = star.peers[id];
      if (!peer) {
        return null;
      }
      return peer;
    }

    function addPeer(id) {
      var peer = newPeer(id, star.order.length + 1);
      star.peers[id] = peer;
      star.order.push(id);
      pushEvent("peer_state|" + id + "|connecting");
      return peer;
    }

    function channelRecord(channel) {
      return [
        channel.state,
        channel.outbound.length,
        channel.inbound.length,
        bufferedAmount(channel),
        channel.sent,
        channel.received,
        channel.dropped_outbound,
        channel.dropped_inbound
      ];
    }

    return {
      initialize: function (role, queueLimit, maxGuests, bufferedLimit) {
        var requestedRole = unescapeField(role);
        if (requestedRole !== "host" && requestedRole !== "guest") {
          return starError("malformed", "role must be host or guest");
        }
        var limit = Number(queueLimit);
        var guests = Number(maxGuests);
        var buffered = Number(bufferedLimit);
        if (!Number.isSafeInteger(limit) || limit < 1 || limit > MAX_QUEUE_LIMIT) {
          return starError("malformed", "queue limit is invalid");
        }
        if (!Number.isSafeInteger(guests) || guests < 1 || guests > MAX_GUESTS) {
          return starError("malformed", "guest capacity is invalid");
        }
        if (
          !Number.isSafeInteger(buffered) ||
          buffered < 1 ||
          buffered > MAX_BUFFERED_AMOUNT_LIMIT
        ) {
          return starError("malformed", "bufferedAmount limit is invalid");
        }
        if (star.state === "connected") {
          return "star|connected";
        }
        star.role = requestedRole;
        star.queue_limit = limit;
        star.event_limit = Math.max(2, limit);
        star.max_guests = requestedRole === "host" ? guests : 1;
        star.buffered_amount_limit = buffered;
        star.peers = Object.create(null);
        star.order = [];
        star.cursor = 0;
        starState("connected");
        if (requestedRole === "guest") {
          addPeer(HOST_PEER_ID);
        }
        return "star|connected";
      },

      shutdown: function () {
        star.order.forEach(function (id) {
          closePeer(star.peers[id], "star transport shutdown");
        });
        star.peers = Object.create(null);
        star.order = [];
        star.cursor = 0;
        starState("closed");
        return "star|closed";
      },

      open_peer: function (peerId) {
        var blocked = requireConnected();
        if (blocked) {
          return blocked;
        }
        if (star.role !== "host") {
          return starError("role_forbidden", "only a host opens guest links");
        }
        var id = unescapeField(peerId);
        if (!validPeerId(id)) {
          return starError("malformed", "peer id is invalid");
        }
        if (id === HOST_PEER_ID) {
          return starError("duplicate_peer", "the host peer id is reserved");
        }
        if (star.peers[id]) {
          return starError("duplicate_peer", "star peer is already open");
        }
        if (star.order.length >= star.max_guests) {
          return starError("capacity", "star transport is at guest capacity");
        }
        return "slot|" + addPeer(id).slot;
      },

      close_peer: function (peerId, reason) {
        var blocked = requireConnected();
        if (blocked) {
          return blocked;
        }
        var peer = findPeer(unescapeField(peerId));
        if (!peer) {
          return starError("unknown_peer", "star transport has no peer with that id");
        }
        closePeer(peer, unescapeField(reason) || "peer closed");
        return "ok";
      },

      request_offer: function (peerId) {
        var blocked = requireConnected();
        if (blocked) {
          return blocked;
        }
        if (star.role !== "host") {
          return starError("role_forbidden", "only a host creates offers");
        }
        var peer = findPeer(unescapeField(peerId));
        if (!peer) {
          return starError("unknown_peer", "star transport has no peer with that id");
        }
        var pc = ensurePeerConnection(peer);
        if (!pc) {
          return "error|bridge_unavailable|" + escapeField("RTCPeerConnection is not available");
        }
        CHANNEL_ORDER.forEach(function (name) {
          if (!peer.channels[name].handle) {
            attachChannel(peer, pc.createDataChannel(name, CHANNEL_CONFIG[name]));
          }
        });
        Promise.resolve()
          .then(function () {
            return pc.createOffer();
          })
          .then(function (offer) {
            return pc.setLocalDescription(offer);
          })
          .then(function () {
            return waitForIce(pc);
          })
          .then(function () {
            storeSignal(peer, pc.localDescription);
          })
          .catch(function (error) {
            peerError(peer, null, "signal_error", String(error));
          });
        return "ok";
      },

      accept_offer: function (signal) {
        var blocked = requireConnected();
        if (blocked) {
          return blocked;
        }
        if (star.role !== "guest") {
          return starError("role_forbidden", "only a guest accepts an offer");
        }
        var peer = findPeer(HOST_PEER_ID) || addPeer(HOST_PEER_ID);
        var offer = parseSignal(peer, signal);
        if (!offer) {
          return "error|signal_error|" + escapeField("remote offer is invalid");
        }
        var pc = ensurePeerConnection(peer);
        if (!pc) {
          return "error|bridge_unavailable|" + escapeField("RTCPeerConnection is not available");
        }
        Promise.resolve()
          .then(function () {
            return pc.setRemoteDescription(offer);
          })
          .then(function () {
            return pc.createAnswer();
          })
          .then(function (answer) {
            return pc.setLocalDescription(answer);
          })
          .then(function () {
            return waitForIce(pc);
          })
          .then(function () {
            storeSignal(peer, pc.localDescription);
          })
          .catch(function (error) {
            peerError(peer, null, "signal_error", String(error));
          });
        return "ok";
      },

      accept_answer: function (peerId, signal) {
        var blocked = requireConnected();
        if (blocked) {
          return blocked;
        }
        if (star.role !== "host") {
          return starError("role_forbidden", "only a host accepts an answer");
        }
        var peer = findPeer(unescapeField(peerId));
        if (!peer) {
          return starError("unknown_peer", "star transport has no peer with that id");
        }
        if (!peer.pc) {
          return peerError(peer, null, "signal_error", "peer has no pending offer");
        }
        var answer = parseSignal(peer, signal);
        if (!answer) {
          return "error|signal_error|" + escapeField("remote answer is invalid");
        }
        peer.pc.setRemoteDescription(answer).catch(function (error) {
          peerError(peer, null, "signal_error", String(error));
        });
        return "ok";
      },

      take_signal: function (peerId) {
        var blocked = requireConnected();
        if (blocked) {
          return blocked;
        }
        var peer = findPeer(unescapeField(peerId));
        if (!peer) {
          return starError("unknown_peer", "star transport has no peer with that id");
        }
        if (!peer.signal) {
          return "";
        }
        var blob = peer.signal;
        peer.signal = null;
        return "signal|" + escapeField(blob);
      },

      send: function (line) {
        var blocked = requireConnected();
        if (blocked) {
          return blocked;
        }
        if (typeof line !== "string") {
          return starError("malformed", "addressed wire must be a string");
        }
        var first = line.indexOf("|");
        var second = line.indexOf("|", first + 1);
        if (first < 0 || second < 0) {
          return starError("malformed", "addressed wire has invalid fields");
        }
        var peerId = line.slice(0, first);
        var channelName = line.slice(first + 1, second);
        var wire = line.slice(second + 1);
        if (!validPeerId(peerId)) {
          return starError("malformed", "peer id is invalid");
        }
        if (star.role === "guest" && peerId !== HOST_PEER_ID) {
          return starError("role_forbidden", "a guest may only address the host");
        }
        /* Message shape decides the returned code before the peer is resolved,
         * so a call that is both badly shaped and badly addressed reports the
         * same code here as in the Lua adapters, which cannot resolve a peer
         * without asking this bridge. The peer is still looked up first, so a
         * shape fault aimed at a known link stays attributed to that link. */
        var peer = findPeer(peerId);
        var known = CHANNEL_MESSAGE_TYPES[channelName] ? channelName : null;
        var parsed = parseWire(wire);
        if (parsed.error) {
          return peer
            ? peerError(peer, known, parsed.error, parsed.detail)
            : starError(parsed.error, parsed.detail);
        }
        var mismatch = checkChannelMessage(channelName, parsed);
        if (mismatch) {
          return peer
            ? peerError(peer, known, mismatch.error, mismatch.detail)
            : starError(mismatch.error, mismatch.detail);
        }
        if (!peer) {
          return starError("unknown_peer", "star transport has no peer with that id");
        }
        return enqueue(peer, channelName, wire);
      },

      broadcast: function (line) {
        var blocked = requireConnected();
        if (blocked) {
          return blocked;
        }
        if (star.role !== "host") {
          return starError("role_forbidden", "only a host fans out canonical batches");
        }
        if (typeof line !== "string") {
          return starError("malformed", "broadcast wire must be a string");
        }
        var first = line.indexOf("|");
        if (first < 0) {
          return starError("malformed", "broadcast wire has invalid fields");
        }
        var channelName = line.slice(0, first);
        var wire = line.slice(first + 1);
        if (!CHANNEL_MESSAGE_TYPES[channelName]) {
          return starError("channel_mismatch", "channel must be control or input");
        }
        var parsed = parseWire(wire);
        if (parsed.error) {
          return starError(parsed.error, parsed.detail);
        }
        var mismatch = checkChannelMessage(channelName, parsed);
        if (mismatch) {
          return starError(mismatch.error, mismatch.detail);
        }
        var delivered = 0;
        star.order.forEach(function (id) {
          var peer = star.peers[id];
          if (peer.state === "connected" && enqueue(peer, channelName, wire) === "ok") {
            delivered += 1;
          }
        });
        return "delivered|" + delivered;
      },

      /* Deterministic drain: a persistent cursor walks (slot, channel-rank)
       * pairs and takes at most one message per pair. Browser callback arrival
       * order is deliberately not treated as simulation order. */
      poll: function () {
        if (star.state !== "connected") {
          return "";
        }
        var slots = star.order.length;
        if (slots === 0) {
          return "";
        }
        var pairs = slots * CHANNEL_ORDER.length;
        for (var attempt = 0; attempt < pairs; attempt += 1) {
          var peerIndex = Math.floor(star.cursor / CHANNEL_ORDER.length) % slots;
          var channelIndex = star.cursor % CHANNEL_ORDER.length;
          star.cursor = (star.cursor + 1) % pairs;
          var peer = star.peers[star.order[peerIndex]];
          var channel = peer.channels[CHANNEL_ORDER[channelIndex]];
          if (channel.inbound.length > 0) {
            channel.received += 1;
            star.received += 1;
            return channel.inbound.shift();
          }
        }
        return "";
      },

      poll_event: function () {
        drainAll();
        return star.events.length === 0 ? "" : star.events.shift();
      },

      diagnostics: function () {
        var lines = [
          [
            "star",
            star.role,
            star.state,
            star.max_guests,
            star.order.length,
            star.queue_limit,
            star.buffered_amount_limit,
            star.events.length,
            star.sent,
            star.received,
            star.dropped_outbound,
            star.dropped_inbound,
            star.malformed,
            star.unsupported_version,
            star.overflow,
            star.backpressure,
            escapeField(star.last_error)
          ].join("|")
        ];
        star.order.forEach(function (id) {
          var peer = star.peers[id];
          var fields = ["peer", peer.id, peer.slot, peer.state, escapeField(peer.ice_state)];
          fields = fields
            .concat(channelRecord(peer.channels.control))
            .concat(channelRecord(peer.channels.input));
          fields.push(peer.sequence_gaps);
          fields.push(peer.backpressure);
          fields.push(peer.malformed);
          fields.push(escapeField(peer.last_error));
          lines.push(fields.join("|"));
        });
        return lines.join("\n");
      }
    };
  })();
})();
