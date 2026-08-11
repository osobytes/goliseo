// The whole harness page. Loaded twice, once per real browser tab, as
// `index.html?role=host` and `index.html?role=guest` -- see this
// directory's sibling `drive_two_peers.py` (invoked from
// `scripts/browser_online_peers.py`) for how the two tabs are driven.
//
// This is deliberately NOT `@gc/app`/`@gc/screens`/`@gc/online` wired up as
// a real client -- this task's file ownership excludes editing any of
// those, and more importantly a minimal page keeps the thing under test
// narrow: real `RTCPeerConnection`/`RTCDataChannel` timing feeding
// `MatchDriverBridge` through `@gc/transport`'s `BrowserStarTransport`,
// nothing else. See tools/browser_online_match's header (this file's
// sibling) for the full brief.
//
// What this page proves, concretely: after a real WebRTC handshake
// (manual signaling relayed by the Python driver -- see below), both
// peers independently advance a `MatchDriverBridge` for the same number of
// ticks, driven only by whatever their own `RTCDataChannel.onmessage`
// callbacks happened to deliver and when. Each peer accumulates its own
// `MatchDriverCheckpoint` (tick, boundary hash) sequence. The Python driver
// reads both sequences back independently and diffs them -- this page
// never compares itself to anything; it only reports what IT computed.

import init, {
  Session,
  MatchDriverBridge,
  matchDriverFixtureFreezeJson,
  matchDriverFixtureManifestJson,
  matchDriverFixtureGuestPeerId,
} from "../../../ts/packages/wasm/dist/pkg-web/gc_wasm.js";

import {
  BrowserStarTransport,
  installGoliseoStarTransport,
  browserStarEval,
  newMessage,
  type TransportPeerEvent,
  type TransportMessage,
} from "@gc/transport";

// --- binary-string <-> bytes, mirroring @gc/wasm's binary_string.ts ---
// (reimplemented locally rather than imported: it is a five-line pure
// helper and this page must not depend on anything under
// ts/packages/wasm/src -- only the already-built pkg-web artifact,
// which this task may read but not edit).
function byteStringFromBytes(bytes: Uint8Array): string {
  let out = "";
  const chunkSize = 4096;
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    const chunk = bytes.subarray(offset, Math.min(offset + chunkSize, bytes.length));
    out += String.fromCharCode(...chunk);
  }
  return out;
}

function bytesFromByteString(value: string): Uint8Array {
  const bytes = new Uint8Array(value.length);
  for (let index = 0; index < value.length; index += 1) {
    bytes[index] = value.charCodeAt(index) & 0xff;
  }
  return bytes;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

type Phase =
  | "booting"
  | "signaling"
  | "awaiting_counterpart_signal"
  | "connecting"
  | "connected"
  | "ready_to_run"
  | "running"
  | "done"
  | "error";

interface Checkpoint {
  readonly tick: number;
  readonly hash: string;
}

interface Report {
  readonly role: "host" | "guest";
  readonly selfPeerId: string;
  readonly counterpartPeerId: string;
  readonly tickCount: number;
  readonly checkpoints: readonly Checkpoint[];
  readonly errors: readonly string[];
  readonly finalStatus: unknown;
  readonly terminal: unknown;
  readonly transportDiagnostics: unknown;
}

interface PeerControlState {
  status: Phase;
  role: "host" | "guest";
  error: string | null;
  // Populated by this page once available; read by the driver.
  hostOffer: string | null;
  guestAnswer: string | null;
  // Written INTO by the driver (out-of-band signaling relay -- see this
  // file's header). Never read or written by anything but the driver and
  // this page's own signaling phase.
  hostOfferForGuest: string | null;
  guestAnswerForHost: string | null;
  // The tick-loop start barrier. Both pages build their `MatchDriverBridge`
  // and report `ready_to_run` independently, then BLOCK here -- there is no
  // shared `first_input_tick` handshake in this minimal harness (that is
  // real `Coordinator` machinery this harness deliberately does not stand
  // up). Without a barrier, whichever page's wasm/JS setup happened to
  // finish first would tick alone for a while, and `match_driver`'s
  // resend-until-confirmed retransmission would grow that peer's
  // unconfirmed backlog past the other side's transport inbound queue
  // before it ever got a chance to consume anything -- a real failure this
  // harness hit before this barrier existed, but a test-pacing artifact,
  // not evidence about net_inbox. The driver pokes this `true` on both
  // pages back-to-back once both report `ready_to_run`.
  startSignal: boolean;
  report: Report | null;
}

declare global {
  interface Window {
    __gcOnlinePeer?: PeerControlState;
  }
}

function marker(kind: string, fields: Record<string, string | number>): string {
  const parts = [`GC_ONLINE_PEER`, kind];
  for (const [key, value] of Object.entries(fields)) {
    parts.push(`${key}=${value}`);
  }
  return parts.join("|");
}

async function waitFor<T>(fn: () => T | null, timeoutMs: number, description: string): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = fn();
    if (value !== null && value !== undefined) {
      return value;
    }
    if (Date.now() > deadline) {
      throw new Error(`timed out waiting for ${description}`);
    }
    await sleep(20);
  }
}

async function waitPeerConnected(
  transport: BrowserStarTransport,
  peerId: string,
  timeoutMs: number,
  errors: string[],
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const event: TransportPeerEvent | null = transport.pollEvent();
    if (event) {
      if (event.kind === "peer_state" && event.peer_id === peerId && event.state === "connected") {
        return;
      }
      if (event.kind === "star_error" || event.kind === "peer_error") {
        errors.push(`${event.kind}: ${event.code ?? ""} ${event.message ?? ""}`);
      }
      continue;
    }
    if (Date.now() > deadline) {
      throw new Error(`timed out waiting for peer ${peerId} to connect`);
    }
    await sleep(20);
  }
}

async function main(): Promise<void> {
  const params = new URLSearchParams(location.search);
  const role = params.get("role") === "guest" ? "guest" : "host";
  const tickCount = Number(params.get("ticks") ?? "150");
  const tickSleepMs = Number(params.get("tick_sleep_ms") ?? "50");

  const state: PeerControlState = {
    status: "booting",
    role,
    error: null,
    hostOffer: null,
    guestAnswer: null,
    hostOfferForGuest: null,
    guestAnswerForHost: null,
    startSignal: false,
    report: null,
  };
  window.__gcOnlinePeer = state;
  console.log(marker("booted", { role }));

  const errors: string[] = [];

  try {
    await init();
    installGoliseoStarTransport();

    const transport = new BrowserStarTransport({ role, eval: browserStarEval });
    const initResult = transport.initialize();
    if (!initResult.ok) {
      throw new Error(`transport.initialize: ${initResult.error.message}`);
    }

    const HOST_PEER_ID = "host";
    const GUEST_PEER_ID = matchDriverFixtureGuestPeerId(1);
    const selfPeerId = role === "host" ? HOST_PEER_ID : GUEST_PEER_ID;
    const counterpartPeerId = role === "host" ? GUEST_PEER_ID : HOST_PEER_ID;

    state.status = "signaling";
    if (role === "host") {
      const openResult = transport.openPeer(GUEST_PEER_ID);
      if (!openResult.ok) {
        throw new Error(`transport.openPeer: ${openResult.error.message}`);
      }
      const offerRequest = transport.requestOffer(GUEST_PEER_ID);
      if (!offerRequest.ok) {
        throw new Error(`transport.requestOffer: ${offerRequest.error.message}`);
      }
      const offer = await waitFor(
        () => {
          const result = transport.takeSignal(GUEST_PEER_ID);
          if (!result.ok) {
            throw new Error(`transport.takeSignal: ${result.error.message}`);
          }
          return result.value;
        },
        15000,
        "local offer signal",
      );
      state.hostOffer = offer;
      console.log(marker("offer_ready", { role, bytes: offer.length }));

      state.status = "awaiting_counterpart_signal";
      const answer = await waitFor(() => state.guestAnswerForHost, 30000, "guest answer relayed by driver");
      const acceptResult = transport.acceptAnswer(GUEST_PEER_ID, answer);
      if (!acceptResult.ok) {
        throw new Error(`transport.acceptAnswer: ${acceptResult.error.message}`);
      }
    } else {
      state.status = "awaiting_counterpart_signal";
      const offer = await waitFor(() => state.hostOfferForGuest, 30000, "host offer relayed by driver");
      const acceptResult = transport.acceptOffer(offer);
      if (!acceptResult.ok) {
        throw new Error(`transport.acceptOffer: ${acceptResult.error.message}`);
      }
      state.status = "signaling";
      const answer = await waitFor(
        () => {
          const result = transport.takeSignal(HOST_PEER_ID);
          if (!result.ok) {
            throw new Error(`transport.takeSignal: ${result.error.message}`);
          }
          return result.value;
        },
        15000,
        "local answer signal",
      );
      state.guestAnswer = answer;
      console.log(marker("answer_ready", { role, bytes: answer.length }));
    }

    state.status = "connecting";
    await waitPeerConnected(transport, counterpartPeerId, 20000, errors);
    state.status = "connected";
    console.log(marker("connected", { role, peer: counterpartPeerId }));

    // A real match session, deterministic and identical on both peers --
    // neither side transmits freeze/manifest/session parameters, both
    // derive them from the same pure fixture functions, exactly the
    // property `MatchDriverBridge`'s constructor doc requires ("every peer
    // sharing one session must be built from the SAME boundary zero").
    const session = new Session("nebula", "orion", 7, 20, 3);
    const freezeJson = matchDriverFixtureFreezeJson("1v1");
    const manifestJson = matchDriverFixtureManifestJson("1v1");
    // `queueLimit` (the driver's own internal `WasmStarTransport` inbound
    // queue -- separate from `@gc/transport`'s star, which has its own,
    // already-generous 64-message-per-channel bound) is raised well past
    // the 64-message default: this Selenium-driven harness is not
    // vsync-locked to a shared 60Hz clock the way two real players'
    // browsers would be, so a batch resimulation catch-up after a burst of
    // confirmed input can legitimately need to hold open a wider gap than
    // a smoothly-paced real session ever would. See
    // `crates/gc-wasm/src/match_driver_bridge.rs`'s `MatchDriverBridge::new`
    // doc for this parameter's own justification for exactly this case.
    const bridge = new MatchDriverBridge(
      session,
      role,
      selfPeerId,
      freezeJson,
      manifestJson,
      undefined,
      2048,
      undefined,
    );
    bridge.initializeTransport();
    bridge.openPeer(counterpartPeerId);
    bridge.setPeerConnected(counterpartPeerId);

    // Start-of-tick-loop barrier -- see `PeerControlState.startSignal`'s
    // doc for why this exists. Blocks until the driver pokes both pages
    // almost simultaneously, once both report `ready_to_run`.
    state.status = "ready_to_run";
    console.log(marker("ready_to_run", { role }));
    await waitFor(() => (state.startSignal ? true : null), 30000, "start signal from driver");

    state.status = "running";
    console.log(marker("running", { role, ticks: tickCount }));

    const checkpoints: Checkpoint[] = [];
    let stoppedEarly = false;
    for (let tick = 0; tick < tickCount; tick += 1) {
      // Drain whatever the REAL data channel's onmessage callback has
      // already delivered by this point in this event-loop turn -- see
      // this file's header: this is the exact seam
      // `crates/gc-wasm/src/net_inbox.rs` says must never be a callback
      // reaching the driver directly.
      for (const inbound of transport.pollBatch(64)) {
        try {
          bridge.enqueueInbound(
            inbound.peer_id,
            inbound.channel,
            inbound.message.type,
            inbound.message.seq,
            inbound.message.tick,
            bytesFromByteString(inbound.message.payload),
          );
        } catch (error) {
          errors.push(`enqueueInbound: ${String(error)}`);
        }
      }

      let batch: { checkpoints?: readonly Checkpoint[] };
      try {
        batch = JSON.parse(bridge.advance(undefined)) as { checkpoints?: readonly Checkpoint[] };
      } catch (error) {
        errors.push(`advance: ${String(error)}`);
        stoppedEarly = true;
        break;
      }
      if (Array.isArray(batch.checkpoints)) {
        for (const checkpoint of batch.checkpoints) {
          checkpoints.push({ tick: checkpoint.tick, hash: checkpoint.hash });
        }
      }

      let outbound: Array<{
        peer_id: string;
        channel: "control" | "input";
        message: { version: number; kind: TransportMessage["type"]; seq: number; tick: number | null; payload_bytes: number[] };
      }>;
      try {
        outbound = JSON.parse(bridge.drainOutboundJson());
      } catch (error) {
        errors.push(`drainOutboundJson parse: ${String(error)}`);
        outbound = [];
      }
      for (const envelope of outbound) {
        const bytes = new Uint8Array(envelope.message.payload_bytes);
        const built = newMessage({
          version: envelope.message.version,
          type: envelope.message.kind,
          seq: envelope.message.seq,
          ...(envelope.message.tick !== null && envelope.message.tick !== undefined
            ? { tick: envelope.message.tick }
            : {}),
          payload: byteStringFromBytes(bytes),
        });
        if (!built.ok) {
          errors.push(`outbound message rejected locally: ${built.error.message}`);
          continue;
        }
        const sendResult = transport.send(envelope.peer_id, envelope.channel, built.value);
        if (!sendResult.ok) {
          errors.push(`transport.send: ${sendResult.error.message}`);
        }
      }

      for (;;) {
        const event = transport.pollEvent();
        if (!event) {
          break;
        }
        if (event.kind === "star_error" || event.kind === "peer_error") {
          errors.push(`${event.kind}: ${event.code ?? ""} ${event.message ?? ""}`);
        }
      }

      const status = JSON.parse(bridge.statusJson()) as string;
      if (status !== "active") {
        stoppedEarly = status !== "completed";
        break;
      }

      if (tickSleepMs > 0) {
        await sleep(tickSleepMs);
      }
    }

    // A short drain window after the tick loop: the last few ticks' worth
    // of outbound traffic is still in flight over the real channel, and
    // the counterpart's own last few checkpoints may not have been
    // observable locally yet (checkpoints are a property of the LOCAL
    // driver's own boundary hashing, not of anything received -- but
    // draining here still lets any trailing inbound settle before this
    // page reports "done", so the driver never reads a report mid-flight).
    for (let drain = 0; drain < 20; drain += 1) {
      for (const inbound of transport.pollBatch(64)) {
        try {
          bridge.enqueueInbound(
            inbound.peer_id,
            inbound.channel,
            inbound.message.type,
            inbound.message.seq,
            inbound.message.tick,
            bytesFromByteString(inbound.message.payload),
          );
        } catch (error) {
          errors.push(`enqueueInbound (drain): ${String(error)}`);
        }
      }
      await sleep(20);
    }

    state.report = {
      role,
      selfPeerId,
      counterpartPeerId,
      tickCount: stoppedEarly ? checkpoints.length : tickCount,
      checkpoints,
      errors,
      finalStatus: JSON.parse(bridge.statusJson()) as unknown,
      terminal: JSON.parse(bridge.terminalJson()) as unknown,
      transportDiagnostics: transport.diagnostics(),
    };
    state.status = "done";
    console.log(
      marker("done", {
        role,
        checkpoints: checkpoints.length,
        errors: errors.length,
        status: JSON.parse(bridge.statusJson()) as string,
      }),
    );
  } catch (error) {
    state.status = "error";
    state.error = String(error);
    console.error(marker("error", { role, message: JSON.stringify(String(error)) }));
  }
}

void main();
