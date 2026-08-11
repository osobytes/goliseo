// Scripted network impairment over the transport contract: delay, jitter,
// loss, bursts, duplication and the reordering they produce.
//
// WHY THIS EXISTS (#472). The native rollback matrix drives every scenario
// through `gc_sim::network_conditions` under the authored profiles in
// `gc-data`. Browser evidence had no equivalent at all: two Chrome peers
// exchanged inputs over a loopback link that never delayed, dropped or
// reordered anything, so "two real browsers agreed" meant "two real browsers
// agreed on a perfect network" -- which is not the property a player's
// connection tests.
//
// This module wraps ANY `TransportAdapter` or `StarTransportAdapter` and
// impairs what the wrapped endpoint sends, so the same harness can run a
// clean link or a stressed one by swapping a decorator.
//
// ## What is mirrored from the native module, exactly
//
// `rust/crates/gc-sim/src/network_conditions.rs` is the reference. Divergence
// here is the defect that makes two suites disagree while both look green, so
// each of these is deliberate, not incidental:
//
//   * TIME IS TICKS, NOT MILLISECONDS. `base_delay_ticks`, the jitter bounds
//     and `burst_length_ticks` are all transport ticks, a clock the CALLER
//     owns and advances (`advanceTo`). It is deliberately separate from the
//     simulation tick a message carries.
//   * FOUR ROLLS PER SEND, ALWAYS, IN ORDER: jitter, independent loss,
//     duplication, burst start. A dropped packet consumes all four. Consuming
//     fewer -- the obvious "don't roll jitter if we're going to drop it"
//     optimisation -- desynchronises the generator from the native run
//     immediately.
//   * BURST BEATS INDEPENDENT LOSS. A packet inside an active burst is a
//     burst loss and is never also counted as an independent loss, and the
//     packet that STARTS a burst is itself dropped.
//   * THE BURST WINDOW IS PER SOURCE, THE GENERATOR AND THE SEQUENCE ARE
//     SHARED. A star with three guests draws from one generator in send
//     order; only `burst_until` is per peer.
//   * LOSS IS PER PACKET, NOT PER TICK. `independent_loss_rate` is rolled on
//     each send.
//   * ARRIVAL IS NEVER BEFORE SEND: `max(send + base + jitter, send)`, so a
//     negative jitter cannot make a packet arrive before it left.
//   * DELIVERY ORDER IS `(arrival_tick, sequence, duplicate_ordinal)`.
//     Reordering is not simulated separately -- it EMERGES from jitter, which
//     is why `playable` and `stress` reorder and `omp0_parity` never does.
//
// ## What is deliberately NOT mirrored -- AND WHAT THAT COSTS
//
// The native module also carries redundant input history on every packet,
// keeps an authoritative-record ledger per slot, and offers `resend`/`drain`
// to guarantee recovery of a named input tick. All of that is rollback
// protocol, one layer above this one: here a message is opaque bytes the
// transport contract already validated. `history_recovered` therefore has no
// counterpart in `ImpairmentCounters`.
//
// The exclusion is sound for what the differential compares --
// `rollback_lab`'s per-tick loop calls only `send`/`poll`, and `drain` runs
// once in `finish_campaign` after that loop, as one-time recovery rather than
// ongoing network behaviour. But it has a consequence worth stating plainly,
// because it is invisible to whoever wires this up next:
//
//   **THIS MODULE HAS NO EQUIVALENT OF GUARANTEED EVENTUAL DELIVERY.** A loss
//   the native protocol resends its way past is a PERMANENT loss here. If the
//   harness that drives this does not build an equivalent recovery layer --
//   redundant history on the packet, or a resend path above the transport --
//   the browser and native suites diverge again, one layer up from where this
//   PR just pinned them: the native run recovers the input and continues, the
//   browser run never sees it and desyncs, and the difference is the harness,
//   not the netcode. Decide that deliberately; do not inherit it by accident.
//
// The two implementations are pinned against each other by a shared
// transcript -- see `impairment_parity.spec.ts` and
// `rust/crates/gc-sim/tests/browser_impairment_parity.rs`.
//
// ## Nothing in CI drives this yet
//
// This module is the mechanism only. No harness constructs it today, so no
// browser evidence is impaired yet and #472 stays open. Wiring it up means:
//
//   1. choosing the profile per run in `tools/browser_online_match`'s peer
//      page, and naming it in the artifact so a browser result can be read
//      beside the native matrix row it corresponds to;
//   2. advancing the transport clock from the peer's own tick loop -- one
//      `setTransportTick(t)` before that tick's sends, one `advanceTo(t)`
//      after them -- and passing `strict_clock: true`, so a loop that skips
//      step one fails on its first tick instead of silently attributing every
//      send to the previous tick for an hour (see `unclocked_sends`);
//   3. reporting `impairmentCounters()` in the run's evidence, so a run that
//      impaired nothing is visibly not a clean run;
//   4. deciding what happens to a permanently lost input -- see the cost
//      stated above. This is the one that is easy to skip and expensive to
//      discover.

import { ok, err } from "@gc/core";
import * as contract from "./contract.ts";
import type {
  StarTransportAdapter,
  TransportAdapter,
  TransportChannel,
  TransportDiagnostics,
  TransportEvent,
  TransportMessage,
  TransportPeerEvent,
  TransportPeerMessage,
  TransportPeerState,
  TransportResult,
  TransportRole,
  TransportStarDiagnostics,
  TransportState,
} from "./contract.ts";
import { rngRoll, rngSeed } from "./impairment_rng.ts";
import type { NetworkProfile } from "./network_profiles.ts";

/** Why a scheduled packet was dropped. */
export type ImpairmentDropReason = "independent_loss" | "burst_loss";

/** The outcome of one impaired send. */
export interface ImpairmentReceipt {
  /** Monotonic per-link send sequence, starting at one. */
  readonly sequence: number;
  /** Which source the packet came from; always 1 on a point-to-point link. */
  readonly source_slot: number;
  /** Transport tick the packet was sent on. */
  readonly send_tick: number;
  readonly dropped: boolean;
  readonly drop_reason: ImpairmentDropReason | null;
  /** Transport tick the packet arrives on, or null when it was dropped. */
  readonly arrival_tick: number | null;
  /** Whether an impairment-created duplicate was also scheduled. */
  readonly duplicated: boolean;
}

/** Running impairment counters since the decorator was created. */
export interface ImpairmentCounters {
  /** Packets offered to the link, excluding impairment-created duplicates. */
  readonly sent: number;
  /** Envelopes released to the wrapped adapter, including duplicates. */
  readonly delivered: number;
  readonly independent_lost: number;
  readonly burst_lost: number;
  readonly duplicated: number;
  /** Non-duplicate envelopes released after a higher sequence already was. */
  readonly reordered: number;
  /**
   * Sends attributed to a transport tick the caller never opened with
   * `setTransportTick`.
   *
   * WHY THIS IS A COUNTER AND NOT A COMMENT. `TransportAdapter.send` carries
   * no tick, so a decorated send is attributed to whatever tick the link was
   * last told about -- and `advanceTo` moves that clock too. A caller whose
   * loop forgets `setTransportTick(t)` therefore has every send that tick
   * attributed to the PREVIOUS tick: no exception, no failure, just an
   * off-by-one between the impairment clock and the real tick loop, running
   * silently through a whole soak and producing delays that look plausible
   * and are wrong. A number in the run's evidence is what catches that at
   * 3am; a doc comment is not.
   *
   * Nonzero does not always mean a bug -- a caller with no tick loop at all
   * reports every send here, and that is the honest reading of a run whose
   * delays are not tick-attributed to anything. Evidence gathered for
   * comparison against the native matrix should expect zero, and
   * `strict_clock` turns the same detection into an immediate throw.
   */
  readonly unclocked_sends: number;
}

export interface ImpairmentOptions {
  /** The authored profile to impair with. See `network_profiles.ts`. */
  readonly profile: NetworkProfile;
  /** Impairment seed. The same seed and profile replay exactly. */
  readonly seed: number;
  /**
   * Throw instead of counting when a send is attributed to a tick the caller
   * never opened with `setTransportTick`. Off by default, because a caller
   * with no tick loop is a legitimate (if unmeasured) use; a harness
   * gathering evidence for comparison against the native matrix should turn
   * it on, so a mis-clocked loop fails on its first tick instead of
   * producing a plausible-looking hour of wrong delays.
   */
  readonly strict_clock?: boolean;
}

/** One envelope released into the wrapped adapter, in delivery order. */
export interface ImpairmentRelease {
  /** The send this envelope belongs to. */
  readonly sequence: number;
  /** Zero for the original, one for the impairment-created duplicate. */
  readonly duplicate_ordinal: number;
  readonly arrival_tick: number;
}

/** A released envelope on a star link, which also names its peer. */
export interface ImpairedStarRelease extends ImpairmentRelease {
  readonly peer_id: string;
  readonly channel: TransportChannel;
}

const ZERO_COUNTERS: ImpairmentCounters = {
  sent: 0,
  delivered: 0,
  independent_lost: 0,
  burst_lost: 0,
  duplicated: 0,
  reordered: 0,
  unclocked_sends: 0,
};

// Authoring invariants, not runtime conditions: a profile that violates one
// of these is a bug in whoever built it, so it fails loud rather than
// impairing a run in a way nobody can interpret (AGENTS.md §7).
function assertProfile(profile: NetworkProfile): void {
  const finite = (value: number): boolean => Number.isFinite(value);
  const integral = (value: number): boolean => finite(value) && value === Math.floor(value);
  if (!integral(profile.base_delay_ticks) || profile.base_delay_ticks < 0) {
    throw new Error("impairment base delay must be a non-negative integer");
  }
  if (!integral(profile.jitter_min_ticks) || !integral(profile.jitter_max_ticks)) {
    throw new Error("impairment jitter bounds must be integers");
  }
  if (profile.jitter_min_ticks > profile.jitter_max_ticks) {
    throw new Error("impairment jitter bounds are reversed");
  }
  for (const [label, rate] of [
    ["loss", profile.independent_loss_rate],
    ["duplication", profile.duplication_rate],
    ["burst", profile.burst_start_rate],
  ] as const) {
    if (!finite(rate) || rate < 0 || rate > 1) {
      throw new Error(`impairment ${label} rate must be in [0, 1]`);
    }
  }
  if (!integral(profile.burst_length_ticks) || profile.burst_length_ticks < 0) {
    throw new Error("impairment burst length must be a non-negative integer");
  }
  const burstOff = profile.burst_start_rate === 0 && profile.burst_length_ticks === 0;
  const burstOn = profile.burst_start_rate > 0 && profile.burst_length_ticks > 0;
  if (!burstOff && !burstOn) {
    throw new Error("impairment burst rate and length must both be disabled or enabled");
  }
}

function assertTick(tick: number, what: string): void {
  if (!Number.isFinite(tick) || tick !== Math.floor(tick) || tick < 0) {
    throw new Error(`impairment ${what} must be a non-negative integer tick`);
  }
}

/** One packet in flight, carrying whatever the decorator needs to release it. */
interface PendingEnvelope<T> {
  readonly sequence: number;
  readonly duplicate_ordinal: number;
  readonly arrival_tick: number;
  readonly payload: T;
}

/**
 * The impairment mechanism itself: a seeded schedule of arrivals, drops and
 * duplicates over an opaque payload. Both decorators below are thin wrappers
 * that decide what the payload is and where a released envelope goes.
 *
 * This is exported so a harness can script impairment over something that is
 * not a transport adapter at all (a wasm bridge, a recorded tape) without
 * re-deriving the semantics.
 */
export class ImpairmentLink<T> {
  private readonly _profile: NetworkProfile;
  private readonly _seed: number;
  private readonly _strictClock: boolean;
  private _rngState: number;
  private _sequence = 0;
  private _clockTick = -1;
  /** The last tick the CALLER named, as opposed to one a delivery moved us to. */
  private _openedTick: number | null = null;
  private _pending: PendingEnvelope<T>[] = [];
  /** Tick each source's active burst runs until; absent when none. */
  private _burstUntil = new Map<number, number>();
  private _maxDeliveredSequence = 0;
  private _counters: ImpairmentCounters = ZERO_COUNTERS;

  constructor(options: ImpairmentOptions) {
    assertProfile(options.profile);
    this._profile = options.profile;
    this._seed = options.seed;
    this._strictClock = options.strict_clock ?? false;
    this._rngState = rngSeed(options.seed);
  }

  profile(): NetworkProfile {
    return this._profile;
  }

  seed(): number {
    return this._seed;
  }

  /** The transport tick this link has been advanced to; -1 before any use. */
  transportTick(): number {
    return this._clockTick;
  }

  counters(): ImpairmentCounters {
    return this._counters;
  }

  /** Envelopes in flight, duplicates counted separately. */
  pendingCount(): number {
    return this._pending.length;
  }

  // Four rolls, always, in this order -- see the header. Returning them as a
  // tuple rather than rolling lazily is what keeps that guarantee readable.
  private _rolls(): readonly [number, number, number, number] {
    const jitter = rngRoll(this._rngState);
    const loss = rngRoll(jitter.state);
    const duplicate = rngRoll(loss.state);
    const burst = rngRoll(duplicate.state);
    this._rngState = burst.state;
    return [jitter.sample, loss.sample, duplicate.sample, burst.sample];
  }

  private _jitterTicks(roll: number): number {
    const width = this._profile.jitter_max_ticks - this._profile.jitter_min_ticks + 1;
    return this._profile.jitter_min_ticks + Math.floor(roll * width);
  }

  /**
   * Offer one packet to the link at the current transport tick. The receipt
   * says what the link did with it; the payload is released later, by
   * `due`, unless it was dropped.
   *
   * Throws if `sendTick` moves backwards: the transport clock is the
   * caller's to advance monotonically, and a regression is a bug in the
   * harness rather than a network condition.
   */
  schedule(sourceSlot: number, sendTick: number, payload: T): ImpairmentReceipt {
    assertTick(sendTick, "send tick");
    if (sendTick < this._clockTick) {
      throw new Error("impairment send tick must be monotonic");
    }
    // Naming the tick IS opening it: a caller that passes an explicit tick
    // cannot be off by one about which tick it meant.
    this._openedTick = sendTick;
    return this._schedule(sourceSlot, sendTick, payload);
  }

  /**
   * Schedule at whatever tick the link is currently on -- the entry point the
   * decorators use, because `TransportAdapter.send` carries no tick.
   *
   * This is the only path on which the tick can be WRONG rather than merely
   * old, so it is the only path that checks: if the current tick is not one
   * the caller opened with `setTick`, the send is counted in
   * `unclocked_sends` (or thrown on, under `strict_clock`).
   */
  scheduleAtCurrentTick(sourceSlot: number, payload: T): ImpairmentReceipt {
    const sendTick = Math.max(this._clockTick, 0);
    if (this._openedTick !== sendTick) {
      if (this._strictClock) {
        throw new Error(
          `impairment send at tick ${sendTick}, which the caller never opened with setTransportTick -- ` +
            "the impairment clock is behind the caller's tick loop and every delay this run would be wrong",
        );
      }
      this._counters = {
        ...this._counters,
        unclocked_sends: this._counters.unclocked_sends + 1,
      };
    }
    return this._schedule(sourceSlot, sendTick, payload);
  }

  private _schedule(sourceSlot: number, sendTick: number, payload: T): ImpairmentReceipt {
    this._clockTick = sendTick;
    this._sequence += 1;
    const sequence = this._sequence;
    this._counters = { ...this._counters, sent: this._counters.sent + 1 };

    const [jitterRoll, lossRoll, duplicateRoll, burstRoll] = this._rolls();

    const burstUntil = this._burstUntil.get(sourceSlot) ?? -1;
    const activeBurst = sendTick <= burstUntil;
    let startedBurst = false;
    if (!activeBurst && burstRoll < this._profile.burst_start_rate) {
      startedBurst = true;
      this._burstUntil.set(sourceSlot, sendTick + this._profile.burst_length_ticks - 1);
    }

    if (activeBurst || startedBurst) {
      this._counters = { ...this._counters, burst_lost: this._counters.burst_lost + 1 };
      return {
        sequence,
        source_slot: sourceSlot,
        send_tick: sendTick,
        dropped: true,
        drop_reason: "burst_loss",
        arrival_tick: null,
        duplicated: false,
      };
    }
    if (lossRoll < this._profile.independent_loss_rate) {
      this._counters = {
        ...this._counters,
        independent_lost: this._counters.independent_lost + 1,
      };
      return {
        sequence,
        source_slot: sourceSlot,
        send_tick: sendTick,
        dropped: true,
        drop_reason: "independent_loss",
        arrival_tick: null,
        duplicated: false,
      };
    }

    const jitter = this._jitterTicks(jitterRoll);
    const arrivalTick = Math.max(sendTick + this._profile.base_delay_ticks + jitter, sendTick);
    this._pending.push({ sequence, duplicate_ordinal: 0, arrival_tick: arrivalTick, payload });

    const duplicated = duplicateRoll < this._profile.duplication_rate;
    if (duplicated) {
      this._pending.push({ sequence, duplicate_ordinal: 1, arrival_tick: arrivalTick, payload });
      this._counters = { ...this._counters, duplicated: this._counters.duplicated + 1 };
    }

    return {
      sequence,
      source_slot: sourceSlot,
      send_tick: sendTick,
      dropped: false,
      drop_reason: null,
      arrival_tick: arrivalTick,
      duplicated,
    };
  }

  /**
   * Advance the transport clock and take every envelope due at or before it,
   * in `(arrival_tick, sequence, duplicate_ordinal)` order.
   *
   * Throws on a non-monotonic tick, for the same reason `schedule` does.
   */
  due(deliveryTick: number): readonly PendingEnvelope<T>[] {
    assertTick(deliveryTick, "delivery tick");
    if (deliveryTick < this._clockTick) {
      throw new Error("impairment delivery tick must be monotonic");
    }
    this._clockTick = deliveryTick;

    const due: PendingEnvelope<T>[] = [];
    const stillPending: PendingEnvelope<T>[] = [];
    for (const envelope of this._pending) {
      if (envelope.arrival_tick <= deliveryTick) {
        due.push(envelope);
      } else {
        stillPending.push(envelope);
      }
    }
    this._pending = stillPending;
    due.sort(
      (left, right) =>
        left.arrival_tick - right.arrival_tick ||
        left.sequence - right.sequence ||
        left.duplicate_ordinal - right.duplicate_ordinal,
    );

    let delivered = this._counters.delivered;
    let reordered = this._counters.reordered;
    for (const envelope of due) {
      delivered += 1;
      if (envelope.duplicate_ordinal === 0) {
        if (envelope.sequence < this._maxDeliveredSequence) {
          reordered += 1;
        }
        this._maxDeliveredSequence = Math.max(this._maxDeliveredSequence, envelope.sequence);
      }
    }
    this._counters = { ...this._counters, delivered, reordered };
    return due;
  }

  /**
   * Move the transport clock without releasing anything -- what the native
   * module's `send` does to its own clock before it schedules. A harness
   * drives one tick as `setTick(t)`, then its sends, then `due(t)`.
   *
   * Throws on a non-monotonic tick, for the same reason `schedule` does.
   */
  setTick(tick: number): void {
    assertTick(tick, "transport tick");
    if (tick < this._clockTick) {
      throw new Error("impairment transport tick must be monotonic");
    }
    this._clockTick = tick;
    this._openedTick = tick;
  }

  /**
   * Discard everything in flight matching `matches`. The per-source burst
   * window is deliberately NOT cleared: it belongs to the link's source
   * slot, not to whichever peer currently occupies it.
   */
  discardWhere(matches: (payload: T) => boolean): void {
    this._pending = this._pending.filter((envelope) => !matches(envelope.payload));
  }

  /** Discard everything in flight. Counters and the generator are untouched. */
  discardAll(): void {
    this._pending = [];
  }
}

/**
 * A `TransportAdapter` whose outbound traffic is impaired.
 *
 * The wrapped adapter is the link. `send`/`enqueue` hand a message to the
 * impairment schedule instead of the adapter, and `advanceTo` releases what
 * has arrived into the adapter -- so from the far end's point of view the
 * message simply took longer, went missing, arrived twice, or arrived out of
 * order.
 *
 * A dropped message is still an `ok` send: the far end never hearing it IS
 * the loss, and a caller that treated it as a local failure would retry and
 * defeat the impairment.
 */
export class ImpairedTransport implements TransportAdapter {
  private readonly _inner: TransportAdapter;
  private readonly _link: ImpairmentLink<TransportMessage>;
  private _lastReceipt: ImpairmentReceipt | null = null;

  constructor(inner: TransportAdapter, options: ImpairmentOptions) {
    this._inner = inner;
    this._link = new ImpairmentLink<TransportMessage>(options);
  }

  /** The impairment schedule itself, for a harness that wants the detail. */
  impairment(): ImpairmentLink<TransportMessage> {
    return this._link;
  }

  impairmentCounters(): ImpairmentCounters {
    return this._link.counters();
  }

  /** What the link did with the most recent send. Null before any send. */
  lastReceipt(): ImpairmentReceipt | null {
    return this._lastReceipt;
  }

  transportTick(): number {
    return this._link.transportTick();
  }

  /**
   * Open a transport tick: move the clock without releasing anything. Call it
   * before that tick's sends, and `advanceTo(tick)` after them.
   *
   * Skipping it does not fail -- it attributes those sends to the previous
   * tick and counts them in `unclocked_sends`. See that counter, and
   * `strict_clock`.
   */
  setTransportTick(tick: number): void {
    this._link.setTick(tick);
  }

  pendingCount(): number {
    return this._link.pendingCount();
  }

  /**
   * Advance the transport clock and release everything that has arrived into
   * the wrapped adapter, in arrival order. Returns the released envelopes in
   * that order -- what a harness logs as "what landed on this tick". A
   * wrapped adapter that rejects one (a full queue, a closed link) fails the
   * call with its own error rather than silently losing it.
   */
  advanceTo(tick: number): TransportResult<readonly ImpairmentRelease[]> {
    const due = this._link.due(tick);
    const released: ImpairmentRelease[] = [];
    for (const envelope of due) {
      const sent = this._inner.send(envelope.payload);
      if (!sent.ok) {
        return err(sent.error);
      }
      released.push({
        sequence: envelope.sequence,
        duplicate_ordinal: envelope.duplicate_ordinal,
        arrival_tick: envelope.arrival_tick,
      });
    }
    return ok(released);
  }

  initialize(): TransportResult<true> {
    return this._inner.initialize();
  }

  shutdown(): TransportResult<true> {
    this._link.discardAll();
    return this._inner.shutdown();
  }

  private _scheduled(message: TransportMessage): TransportResult<true> {
    const validated = contract.validate(message);
    if (!validated.ok) {
      return err(validated.error);
    }
    const state = this._inner.state();
    if (state === "new") {
      return err({ message: "transport is not initialized", code: "not_initialized" });
    }
    if (state === "closed") {
      return err({ message: "transport is closed", code: "closed" });
    }
    if (state !== "connected") {
      return err({ message: "transport is not connected", code: "not_connected" });
    }
    this._lastReceipt = this._link.scheduleAtCurrentTick(1, contract.copy(message));
    return ok(true);
  }

  enqueue(message: TransportMessage): TransportResult<true> {
    return this._scheduled(message);
  }

  send(message: TransportMessage): TransportResult<true> {
    return this._scheduled(message);
  }

  poll(): TransportResult<TransportMessage | null> {
    return this._inner.poll();
  }

  pollEvent(): TransportEvent | null {
    return this._inner.pollEvent();
  }

  state(): TransportState {
    return this._inner.state();
  }

  diagnostics(): TransportDiagnostics {
    return this._inner.diagnostics();
  }
}

interface StarPayload {
  readonly peer_id: string;
  readonly channel: TransportChannel;
  readonly message: TransportMessage;
}

/**
 * A `StarTransportAdapter` whose outbound traffic is impaired, per peer.
 *
 * One generator and one send sequence are shared across every peer -- that is
 * the native module's shape, and a per-peer generator would diverge from it.
 * The loss-burst window is the only per-peer state, keyed by the star's own
 * slot so a reopened peer id reuses its slot rather than inventing one.
 */
export class ImpairedStarTransport implements StarTransportAdapter {
  private readonly _inner: StarTransportAdapter;
  private readonly _link: ImpairmentLink<StarPayload>;
  private readonly _slots = new Map<string, number>();
  private _lastReceipt: ImpairmentReceipt | null = null;

  constructor(inner: StarTransportAdapter, options: ImpairmentOptions) {
    this._inner = inner;
    this._link = new ImpairmentLink<StarPayload>(options);
  }

  /** The impairment schedule itself, for a harness that wants the detail. */
  impairment(): ImpairmentLink<StarPayload> {
    return this._link;
  }

  impairmentCounters(): ImpairmentCounters {
    return this._link.counters();
  }

  lastReceipt(): ImpairmentReceipt | null {
    return this._lastReceipt;
  }

  transportTick(): number {
    return this._link.transportTick();
  }

  /**
   * Open a transport tick: move the clock without releasing anything. Call it
   * before that tick's sends, and `advanceTo(tick)` after them.
   *
   * Skipping it does not fail -- it attributes those sends to the previous
   * tick and counts them in `unclocked_sends`. See that counter, and
   * `strict_clock`.
   */
  setTransportTick(tick: number): void {
    this._link.setTick(tick);
  }

  pendingCount(): number {
    return this._link.pendingCount();
  }

  /** See `ImpairedTransport.advanceTo`. */
  advanceTo(tick: number): TransportResult<readonly ImpairedStarRelease[]> {
    const due = this._link.due(tick);
    const released: ImpairedStarRelease[] = [];
    for (const envelope of due) {
      const payload = envelope.payload;
      const sent = this._inner.send(payload.peer_id, payload.channel, payload.message);
      if (!sent.ok) {
        return err(sent.error);
      }
      released.push({
        sequence: envelope.sequence,
        duplicate_ordinal: envelope.duplicate_ordinal,
        arrival_tick: envelope.arrival_tick,
        peer_id: payload.peer_id,
        channel: payload.channel,
      });
    }
    return ok(released);
  }

  // The star owns slot identity; this only remembers what it assigned, and
  // falls back to its diagnostics for a peer opened before the decorator saw
  // it (a guest, whose single link is always slot 1).
  private _slotFor(peerId: string): number {
    const known = this._slots.get(peerId);
    if (known !== undefined) {
      return known;
    }
    for (const peer of this._inner.diagnostics().peers) {
      if (peer.peer_id === peerId) {
        this._slots.set(peerId, peer.slot);
        return peer.slot;
      }
    }
    this._slots.set(peerId, 1);
    return 1;
  }

  initialize(): TransportResult<true> {
    return this._inner.initialize();
  }

  shutdown(): TransportResult<true> {
    this._link.discardAll();
    return this._inner.shutdown();
  }

  role(): TransportRole {
    return this._inner.role();
  }

  capacity(): number {
    return this._inner.capacity();
  }

  openPeer(peerId: string): TransportResult<number> {
    const opened = this._inner.openPeer(peerId);
    if (opened.ok) {
      this._slots.set(peerId, opened.value);
    }
    return opened;
  }

  closePeer(peerId: string, reason?: string): TransportResult<true> {
    // Everything still in flight for that peer is gone with the link -- a
    // packet cannot arrive at a peer that has been closed.
    this._link.discardWhere((payload) => payload.peer_id === peerId);
    this._slots.delete(peerId);
    return reason === undefined
      ? this._inner.closePeer(peerId)
      : this._inner.closePeer(peerId, reason);
  }

  peerIds(): readonly string[] {
    return this._inner.peerIds();
  }

  peerState(peerId: string): TransportPeerState | null {
    return this._inner.peerState(peerId);
  }

  requestOffer(peerId: string): TransportResult<true> {
    return this._inner.requestOffer(peerId);
  }

  acceptOffer(signal: string): TransportResult<true> {
    return this._inner.acceptOffer(signal);
  }

  acceptAnswer(peerId: string, signal: string): TransportResult<true> {
    return this._inner.acceptAnswer(peerId, signal);
  }

  takeSignal(peerId: string): TransportResult<string | null> {
    return this._inner.takeSignal(peerId);
  }

  send(
    peerId: string,
    channel: TransportChannel,
    message: TransportMessage,
  ): TransportResult<true> {
    const validated = contract.validateChannelMessage(channel, message);
    if (!validated.ok) {
      return err(validated.error);
    }
    const peerIdValid = contract.validatePeerId(peerId);
    if (!peerIdValid.ok) {
      return err(peerIdValid.error);
    }
    if (this._inner.peerState(peerId) === null) {
      return err({ message: "transport peer is not open", code: "unknown_peer" });
    }
    this._lastReceipt = this._link.scheduleAtCurrentTick(this._slotFor(peerId), {
      peer_id: peerId,
      channel,
      message: contract.copy(message),
    });
    return ok(true);
  }

  /**
   * One independently impaired packet per open peer, in slot order -- the
   * same shape a host star has on the wire, where a broadcast is N sends and
   * one of them can be the one that goes missing.
   */
  broadcast(channel: TransportChannel, message: TransportMessage): TransportResult<number> {
    const validated = contract.validateChannelMessage(channel, message);
    if (!validated.ok) {
      return err(validated.error);
    }
    let scheduled = 0;
    for (const peerId of this._inner.peerIds()) {
      const sent = this.send(peerId, channel, message);
      if (!sent.ok) {
        return err(sent.error);
      }
      scheduled += 1;
    }
    return ok(scheduled);
  }

  poll(): TransportResult<TransportPeerMessage | null> {
    return this._inner.poll();
  }

  pollBatch(limit?: number): readonly TransportPeerMessage[] {
    return limit === undefined ? this._inner.pollBatch() : this._inner.pollBatch(limit);
  }

  pollEvent(): TransportPeerEvent | null {
    return this._inner.pollEvent();
  }

  state(): TransportState {
    return this._inner.state();
  }

  diagnostics(): TransportStarDiagnostics {
    return this._inner.diagnostics();
  }
}

/** Wrap a point-to-point adapter so its outbound traffic is impaired. */
export function impaired(inner: TransportAdapter, options: ImpairmentOptions): ImpairedTransport {
  return new ImpairedTransport(inner, options);
}

/** Wrap a host-star adapter so its outbound traffic is impaired, per peer. */
export function impairedStar(
  inner: StarTransportAdapter,
  options: ImpairmentOptions,
): ImpairedStarTransport {
  return new ImpairedStarTransport(inner, options);
}
