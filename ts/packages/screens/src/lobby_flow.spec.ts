// This suite's `Driver` class (defined below) drives `lobby_model.ts`'s
// pure model over a link layer, completing a real manual offer/answer
// handshake against the coordinator (Rust-owned, `crates/gc-netcode`, no
// wasm bridge this milestone) -- the boundary `ARCHITECTURE.md` §1.1 draws
// deliberately.
//
// # Why this file cannot import the real `lobby_link`
//
// `@gc/screens`' own `package.json` depends on `@gc/core`, `@gc/ui`, and
// `@gc/presentation` only -- not `@gc/online` or `@gc/transport`, and this
// task may not edit `package.json`. pnpm's workspace linking is strict
// about that: `packages/screens/node_modules/@gc` only symlinks the three
// declared dependencies, so `import ... from "@gc/online"` cannot resolve
// here, at either `tsc --build` or `vitest run` time. (This is also the
// correct direction per `ARCHITECTURE.md`'s file-mapping table: `@gc/online`
// depends on nothing in `@gc/screens`, and `@gc/app` -- which *does* depend
// on both -- is the layer meant to wire them together, not this one.)
//
// So `FakeTransport` below is a small, self-contained stand-in for the
// combination of a star transport and a `LobbyLink`, hand-written for this
// file. It is legitimate for the same reason `match_presentation.spec.ts`'s
// hand-written `fakeRollbackEvents` is: `lobby_link`'s own framing and
// reassembly invariants (chunking, delimiter-safety, out-of-order refusal)
// are pinned for real, against the genuine module, in this package's
// sibling `lobby_link.spec.ts` (`@gc/online`). What `FakeTransport` needs
// to reproduce here is only the *shape* of the event stream a real link
// produces (`signal`, `peer_connected`, `control`) so that `lobby_model`'s
// effects can be driven end to end -- not the byte-level chunking, which
// this file's messages (small JSON envelopes) never approach the bound of
// anyway.
//
// # The `CoordinatorPort` fake, and what it does and does not prove
//
// The real coordinator is a ~3,300-line replicated state machine (manifest
// proposal, slot assignment, the pair-preference protocol, readiness, the
// countdown/freeze barrier, build-skew detection, guest drop/departure).
// Reimplementing it here would create a second source of truth for exactly
// the kind of state two peers must agree on bit for bit -- precisely what
// `ARCHITECTURE.md` §1.1 exists to prevent, and precisely why it stays
// Rust-only, with no TypeScript implementation of its own.
//
// The fake below is not that. It is a compact, general (not per-test
// scripted) implementation of the same *shape* of protocol -- one JSON
// message per coordinator wire kind, exchanged over `FakeTransport` --
// built by reading the real coordinator's rules (`crates/gc-netcode`) and
// carrying them over (contiguous-block seat planning, the pair-preference
// verdict order: frozen -> seated -> shape -> team -> continuity ->
// claimed, claim survival on a roster change, the client-side preference
// timeout, the two-ack countdown/start barrier, build- vs
// manifest-mismatch classification) rather than hand-tuning responses per
// assertion. Every case below exercises this fake through the *same*
// `lobby_model.command` entry point the real coordinator would be driven
// through; nothing in `lobby_model.ts` itself is touched or duplicated.
//
// What that buys, honestly: every assertion in the driver-based cases below
// is about `lobby_model`'s own projection and control flow (the `view()`
// shape, the text tables, effect ordering, phase transitions) reacting to a
// *plausible* coordinator, not about whether the real coordinator's
// admission/assignment/preference logic is correct -- that is `crates/
// gc-netcode`'s `tests/coordinator.rs` and `tests/coordinator_driver.rs`
// job, including, historically, a differential against an earlier
// reference implementation. No case here was written by picking the
// assertion first and hand-tuning the fake's response to match; every
// verdict the fake produces falls out of the general rules above applied
// to that case's actual roster/assignment state.

import { describe, expect, it } from "vitest";
import {
  view as lobbyModelView,
  command,
  newLobbyModel,
  COUNTDOWN_TICKS,
  DEPARTURE_TEXT,
  PREFERENCE_TEXT,
  TERMINAL_TEXT,
} from "./lobby_model.ts";
import type {
  CoordinatorAction,
  CoordinatorDeparture,
  CoordinatorManifestExpectation,
  CoordinatorNewGuestOptions,
  CoordinatorNewHostOptions,
  CoordinatorOutcome,
  CoordinatorPort,
  CoordinatorState,
  CoordinatorTerminalReason,
  Fnv1a64Port,
  InputFramePort,
  InputSlotId,
  InputTeam,
  LobbyCommand,
  LobbyEffect,
  LobbyModel,
  LobbyModelPorts,
  LobbyRole,
  ProtocolFixturePort,
  ProtocolPort,
  SessionManifest,
  SessionMatchMode,
  SessionMatchModeShape,
  SessionPreferenceRejection,
  SessionPreferenceStatus,
  SessionSlotProducer,
  TransportContractPort,
} from "./lobby_model.ts";

// ---------------------------------------------------------------------------
// A checked-access helper standing in for `assert()` (AGENTS.md §7): a
// missing value here is a coding error in the fake, not an expected
// failure, so it throws rather than being papered over with `!`.
// ---------------------------------------------------------------------------

function must<T>(value: T | undefined, message: string): T {
  if (value === undefined) {
    throw new Error(message);
  }
  return value;
}

// ---------------------------------------------------------------------------
// Shared fixtures: canonical slots, match mode shapes, a manifest.
// ---------------------------------------------------------------------------

const SLOT_ORDER: readonly { readonly id: InputSlotId; readonly team: InputTeam }[] = [
  { id: "home_1", team: "home" },
  { id: "home_2", team: "home" },
  { id: "home_3", team: "home" },
  { id: "home_4", team: "home" },
  { id: "away_1", team: "away" },
  { id: "away_2", team: "away" },
  { id: "away_3", team: "away" },
  { id: "away_4", team: "away" },
];

const SLOT_INDEX: Readonly<Record<string, number>> = Object.fromEntries(
  SLOT_ORDER.map((entry, index) => [entry.id, index + 1]),
);

function teamOfSlot(slot: InputSlotId): InputTeam {
  return must(
    SLOT_ORDER.find((entry) => entry.id === slot),
    `unknown canonical slot ${slot}`,
  ).team;
}

const MATCH_MODES: Readonly<Record<SessionMatchMode, SessionMatchModeShape>> = {
  "1v1": { humans: 2, slots_per_human: 4, team_humans: 1 },
  "2v2": { humans: 4, slots_per_human: 2, team_humans: 2 },
  "4v4": { humans: 8, slots_per_human: 1, team_humans: 4 },
};

function matchModeShape(mode: SessionMatchMode): SessionMatchModeShape {
  return must(MATCH_MODES[mode], `unknown match mode ${mode}`);
}

function baseManifest(mode: SessionMatchMode): SessionManifest {
  const rosterEntry = (playerId: string, position: string) => ({ position, player_id: playerId });
  return {
    session_id: "session_alpha",
    match_mode: mode,
    build_id: "build.fixture",
    source_id: "source.fixture",
    content_id: "content.fixture",
    tuning_id: "tuning.fixture",
    match_config_id: "match_config.fixture",
    fixture_id: "fixture.default_mixed.v1",
    arena_id: "arena.goliseo",
    combat_rules_id: "combat_interaction.fixture",
    gameplay_ai_policy_id: "gameplay_ai.fixture",
    combat_status: "provisional_fixture",
    slots: [
      { player_id: "zyro_vex" },
      { player_id: "mika_olu" },
      { player_id: "rok_tann" },
      { player_id: "sela_dwin" },
      { player_id: "drell" },
      { player_id: "morv" },
      { player_id: "krag" },
      { player_id: "tox_vren" },
    ],
    teams: [
      {
        team: "home",
        roster: [
          rosterEntry("ozzo", "keeper"),
          rosterEntry("zyro_vex", "forward"),
          rosterEntry("mika_olu", "defender"),
          rosterEntry("rok_tann", "midfielder"),
          rosterEntry("sela_dwin", "forward"),
        ],
      },
      {
        team: "away",
        roster: [
          rosterEntry("gax_oru", "keeper"),
          rosterEntry("drell", "defender"),
          rosterEntry("morv", "defender"),
          rosterEntry("krag", "midfielder"),
          rosterEntry("tox_vren", "forward"),
        ],
      },
    ],
  };
}

// The content-derived template the shipped app injects, per `lobby_model`'s
// `LobbyModelOptions.template`. `build_id` is the identity under test in
// the "lobby build skew" cases below, so each of those constructs its own
// variant rather than using the fixture default.
function fakeManifest(mode: SessionMatchMode): SessionManifest {
  return baseManifest(mode);
}

// `foreignManifest` produces the manifest a peer would propose from a
// build whose control vocabulary differs, with everything else -- name,
// version, channel -- identical. That effect comes from monkey-patching
// `protocol.vocabulary_id` (Rust-owned) elsewhere; here it is produced
// directly on the fixture instead.
function foreignManifest(mode: SessionMatchMode): SessionManifest {
  return { ...baseManifest(mode), build_id: `${baseManifest(mode).build_id}0` };
}

function manifestWithContentId(mode: SessionMatchMode, contentId: string): SessionManifest {
  return { ...baseManifest(mode), content_id: contentId };
}

// ---------------------------------------------------------------------------
// LobbyModelPorts fakes (protocol / protocol_fixture / transport_contract /
// fnv1a64 / input_frame) -- the ones with no coordination logic of their
// own, following the same shape as `lobby.spec.ts`'s fakes in this package.
// ---------------------------------------------------------------------------

function fakeInputFrame(): InputFramePort {
  return {
    slotCount: SLOT_ORDER.length,
    slot(index) {
      return SLOT_ORDER[index - 1];
    },
  };
}

function fakeProtocol(): ProtocolPort {
  return {
    matchModes: MATCH_MODES,
    encode(message) {
      return JSON.stringify(message);
    },
    slotIndex(slot) {
      return SLOT_INDEX[slot];
    },
  };
}

function fakeProtocolFixture(): ProtocolFixturePort {
  return {
    manifest: fakeManifest,
    runtime: () => ({ runtime_id: "fixture" }),
  };
}

function fakeTransportContract(): TransportContractPort {
  return { hostPeerId: "host", maxGuests: 7 };
}

function simpleHash(text: string): string {
  let h = 0;
  for (let i = 0; i < text.length; i += 1) {
    h = (h * 31 + text.charCodeAt(i)) | 0;
  }
  return Math.abs(h).toString(16).padStart(8, "0");
}

function fakeFnv1a64(): Fnv1a64Port {
  return { hash: simpleHash };
}

// ---------------------------------------------------------------------------
// The CoordinatorPort fake. See this file's header for what it is and is
// not. `FakeCoordState`/`FakePeer` carry bookkeeping `CoordinatorState`
// does not expose (assignment epoch, per-peer link id and declared build,
// the freeze record, the pending-preference deadline); the boundary
// between the two is a single, explicit, well-contained cast, commented at
// each crossing.
// ---------------------------------------------------------------------------

// How long a guest waits for the host's verdict on a pair request before it
// gives up. Mirrors the real coordinator's `PREFERENCE_TIMEOUT_TICKS`
// constant (Rust-owned, no bridge -- see this file's header).
const PREFERENCE_TIMEOUT_TICKS = 300;

interface FakePreference {
  readonly slots: readonly InputSlotId[];
  readonly assignment_id?: string;
  // `lobby_model.ts`'s `SessionPreferenceStatus` now includes "pending"
  // (it used to be "granted" | "unchanged" | "rejected" only, and this
  // field had to be locally widened to represent the real coordinator's
  // between-request-and-verdict state -- see that type's own doc comment
  // for the fix).
  readonly status: SessionPreferenceStatus;
  readonly reason?: SessionPreferenceRejection;
  readonly deadline?: number;
}

interface FakePeer {
  readonly peer_id: string;
  readonly link_id: string;
  readonly role: LobbyRole;
  readonly build_id: string;
  readonly ready: boolean;
  readonly started: boolean;
  readonly accepted_manifest_id?: string;
  readonly pair_choice?: readonly InputSlotId[];
}

interface FakeFreeze {
  readonly manifest_id: string;
  readonly assignment_id: string;
  readonly countdown_id: string;
  readonly first_input_tick: number;
  readonly match_mode: SessionMatchMode;
  readonly assignments: readonly SessionSlotProducer[];
  readonly owned: Readonly<Record<string, readonly InputSlotId[]>>;
  readonly live: Readonly<Record<string, InputSlotId>>;
}

// `CoordinatorState`'s own shape, widened with the extra bookkeeping this
// fake needs and `lobby_model.ts` never reads. `role`/`peer_id`/`phase`/
// `manifest`/`manifest_id`/`assignments`/`countdown_remaining`/`terminal`/
// `departure` all keep their public meaning; `peers[0]` is always this
// coordinator's own entry (mirrors the real coordinator's own local-peer
// entry), matching every other peer.
interface FakeCoordState {
  readonly role: LobbyRole;
  readonly peer_id: string;
  readonly phase:
    | "new"
    | "handshake"
    | "manifest"
    | "assigned"
    | "ready"
    | "countdown"
    | "running"
    | "result"
    | "terminal";
  readonly peers: readonly FakePeer[];
  readonly hostPeerId: string;
  readonly buildId: string;
  readonly expectation?: CoordinatorManifestExpectation;
  readonly clock: number;
  readonly assignmentEpoch: number;
  readonly manifest?: SessionManifest;
  readonly manifest_id?: string;
  readonly assignments?: readonly SessionSlotProducer[];
  readonly assignment_id?: string;
  readonly preference?: FakePreference;
  readonly freeze?: FakeFreeze;
  readonly countdown_remaining?: number;
  readonly terminal?: { readonly reason: CoordinatorTerminalReason; readonly detail?: string };
  readonly departure?: CoordinatorDeparture;
}

// The one crossing between this fake's own bookkeeping and the interface
// `lobby_model.ts` declares. `CoordinatorState` does not (and should not)
// know about `FakeCoordState`'s extra fields, so this is a boundary cast,
// not a structural one -- `unknown` in the middle, never `any`.
function toPublic(state: FakeCoordState): CoordinatorState {
  return state as unknown as CoordinatorState;
}
function toFake(state: CoordinatorState): FakeCoordState {
  return state as unknown as FakeCoordState;
}

const TERMINAL_CODES: Readonly<Record<CoordinatorTerminalReason, string>> = {
  completed: "completed",
  local_abort: "host_abort",
  peer_abort: "host_abort",
  guest_left: "peer_disconnect",
  host_left: "peer_disconnect",
  removed: "peer_disconnect",
  transport_lost: "peer_disconnect",
  protocol_violation: "malformed_message",
  manifest_mismatch: "manifest_mismatch",
  build_mismatch: "manifest_mismatch",
  invalid_assignment: "invalid_assignment",
  start_ack_timeout: "peer_disconnect",
  input_channel_failure: "peer_disconnect",
  late_input: "desync",
  hash_mismatch: "desync",
};

// Mirrors the real coordinator's `DISCONNECT_REASONS` table (Rust-owned;
// no bridge). Also used directly by the "has host-side language for every
// reason a drop can carry" completeness case below, since that table has
// no TypeScript counterpart to read it from.
const DISCONNECT_REASONS: Readonly<Record<string, CoordinatorTerminalReason>> = {
  peer_left: "guest_left",
  transport_lost: "transport_lost",
  host_left: "host_left",
  protocol_error: "protocol_violation",
};

const EXPECTATION_FIELDS: readonly (keyof CoordinatorManifestExpectation)[] = [
  "build_id",
  "source_id",
  "content_id",
  "tuning_id",
  "match_config_id",
  "fixture_id",
  "arena_id",
  "combat_rules_id",
  "gameplay_ai_policy_id",
  "combat_status",
];

const IDENTITY_REASONS: Readonly<Record<string, CoordinatorTerminalReason>> = {
  "manifest.build_id": "build_mismatch",
  "manifest.source_id": "build_mismatch",
};

function expectationDifference(
  expectation: CoordinatorManifestExpectation | undefined,
  manifest: SessionManifest,
): { readonly path: string } | undefined {
  if (!expectation) {
    return undefined;
  }
  for (const field of EXPECTATION_FIELDS) {
    const expected = expectation[field];
    const actual = (manifest as unknown as Record<string, unknown>)[field];
    if (expected !== undefined && expected !== actual) {
      return { path: `manifest.${field}` };
    }
  }
  return undefined;
}

function linkTargets(state: FakeCoordState): readonly string[] {
  return state.peers.slice(1).map((peer) => peer.link_id);
}

function ownedSlotsOf(
  assignments: readonly SessionSlotProducer[],
  producerId: string,
): InputSlotId[] {
  return assignments
    .filter((p) => p.producer_kind === "peer" && p.producer_id === producerId)
    .map((p) => p.slot);
}

function ownsSlot(state: FakeCoordState, peerId: string): boolean {
  return (state.assignments ?? []).some(
    (p) => p.producer_kind === "peer" && p.producer_id === peerId,
  );
}

function slotListsEqual(a: readonly InputSlotId[], b: readonly InputSlotId[]): boolean {
  return a.length === b.length && a.every((slot, index) => slot === b[index]);
}

function assignmentsEqual(
  a: readonly SessionSlotProducer[] | undefined,
  b: readonly SessionSlotProducer[],
): boolean {
  if (!a || a.length !== b.length) {
    return false;
  }
  return a.every((p, index) => {
    const q = b[index];
    return (
      q !== undefined &&
      p.slot === q.slot &&
      p.team === q.team &&
      p.producer_kind === q.producer_kind &&
      p.producer_id === q.producer_id
    );
  });
}

function previewLiveOf(
  assignments: readonly SessionSlotProducer[] | undefined,
): Readonly<Record<string, InputSlotId>> {
  const live: Record<string, InputSlotId> = {};
  for (const producer of assignments ?? []) {
    if (producer.producer_kind === "peer" && live[producer.producer_id] === undefined) {
      live[producer.producer_id] = producer.slot;
    }
  }
  return live;
}

// Contiguous-block seating: mirrors `coordinator.plan_assignments`. The Nth
// human in `seating` owns canonical slots `(N-1)*k+1..N*k`; every slot the
// seating does not reach becomes a bot producer.
function planAssignments(
  manifest: SessionManifest,
  seating: readonly string[],
): readonly SessionSlotProducer[] | undefined {
  const shape = matchModeShape(manifest.match_mode);
  if (seating.length > shape.humans) {
    return undefined;
  }
  const peerBySlot = new Map<number, string>();
  seating.forEach((peerId, order) => {
    for (let offset = 0; offset < shape.slots_per_human; offset += 1) {
      peerBySlot.set(order * shape.slots_per_human + offset, peerId);
    }
  });
  return SLOT_ORDER.map((entry, index) => {
    const peerId = peerBySlot.get(index);
    if (peerId !== undefined) {
      return {
        slot: entry.id,
        team: entry.team,
        producer_kind: "peer",
        producer_id: peerId,
      } satisfies SessionSlotProducer;
    }
    return {
      slot: entry.id,
      team: entry.team,
      producer_kind: "bot",
      producer_id: `bot.${entry.id}`,
    } satisfies SessionSlotProducer;
  });
}

// Mirrors `coordinator.reseat_claims`: a roster-derived plan knows nothing
// of granted pairs, so this reconciles the two. A claim survives when the
// plan still seats a human on every one of its slots, on one team, and none
// of those slots is already kept for another peer (peers are walked in
// their own array order, so an earlier claim wins a conflict). What is left
// over is filled from the plan's own order, `slots_per_human` at a time.
function reseatClaims(
  shape: SessionMatchModeShape,
  plan: readonly SessionSlotProducer[],
  peers: readonly FakePeer[],
): {
  readonly assignments: SessionSlotProducer[];
  readonly retained: Readonly<Record<string, readonly InputSlotId[]>>;
} {
  const humanIndex = new Map<InputSlotId, number>();
  const humanOrder: number[] = [];
  plan.forEach((producer, index) => {
    if (producer.producer_kind === "peer") {
      humanIndex.set(producer.slot, index);
      humanOrder.push(index);
    }
  });
  const kept = new Set<InputSlotId>();
  const survives = (claim: readonly InputSlotId[]): boolean => {
    if (claim.length !== shape.slots_per_human) {
      return false;
    }
    let team: InputTeam | undefined;
    for (const slot of claim) {
      const index = humanIndex.get(slot);
      if (index === undefined || kept.has(slot)) {
        return false;
      }
      const slotTeam = must(plan[index], "reseat: plan index out of range").team;
      team = team ?? slotTeam;
      if (slotTeam !== team) {
        return false;
      }
    }
    return true;
  };
  const retained: Record<string, readonly InputSlotId[]> = {};
  for (const peer of peers) {
    const claim = peer.pair_choice;
    if (claim && survives(claim)) {
      retained[peer.peer_id] = claim;
      for (const slot of claim) {
        kept.add(slot);
      }
    }
  }
  const free: number[] = [];
  const unseated: string[] = [];
  const seen = new Set<string>();
  for (const index of humanOrder) {
    const producer = must(plan[index], "reseat: plan index out of range");
    if (!kept.has(producer.slot)) {
      free.push(index);
    }
    if (!seen.has(producer.producer_id) && retained[producer.producer_id] === undefined) {
      seen.add(producer.producer_id);
      unseated.push(producer.producer_id);
    }
  }
  const reseated = plan.map((producer) => ({ ...producer }));
  for (const peer of peers) {
    const claim = retained[peer.peer_id];
    if (!claim) {
      continue;
    }
    for (const slot of claim) {
      const index = humanIndex.get(slot);
      if (index !== undefined) {
        reseated[index] = {
          ...must(reseated[index], "reseat: rebuilt index out of range"),
          producer_id: peer.peer_id,
        };
      }
    }
  }
  let cursor = 0;
  for (const producerId of unseated) {
    for (let i = 0; i < shape.slots_per_human; i += 1) {
      const slotIndex = free[cursor];
      cursor += 1;
      if (slotIndex !== undefined) {
        reseated[slotIndex] = {
          ...must(reseated[slotIndex], "reseat: free index out of range"),
          producer_id: producerId,
        };
      }
    }
  }
  return { assignments: reseated, retained };
}

// Mirrors `exchange_assignments`: the Nth slot the requester gains returns
// the Nth slot it vacates to whoever held the gained slot before.
function exchangeAssignments(
  assignments: readonly SessionSlotProducer[],
  peerId: string,
  requested: readonly InputSlotId[],
): SessionSlotProducer[] {
  const current = ownedSlotsOf(assignments, peerId);
  const requestedSet = new Set(requested);
  const heldSet = new Set(current);
  const gained = requested.filter((slot) => !heldSet.has(slot));
  const vacated = current.filter((slot) => !requestedSet.has(slot));
  const returnedTo: Record<string, string> = {};
  gained.forEach((slot, index) => {
    const vacatedSlot = vacated[index];
    if (vacatedSlot === undefined) {
      return;
    }
    const gainedIndex = must(SLOT_INDEX[slot], `unknown slot ${slot}`) - 1;
    const gainedProducer = must(assignments[gainedIndex], "exchange: gained index out of range");
    returnedTo[vacatedSlot] = gainedProducer.producer_id;
  });
  return SLOT_ORDER.map((entry, index) => {
    const existing = must(assignments[index], "exchange: assignment index out of range");
    if (requestedSet.has(entry.id)) {
      return {
        ...existing,
        producer_kind: "peer",
        producer_id: peerId,
      } satisfies SessionSlotProducer;
    }
    const returned = returnedTo[entry.id];
    if (returned !== undefined) {
      return {
        ...existing,
        producer_kind: "peer",
        producer_id: returned,
      } satisfies SessionSlotProducer;
    }
    return existing;
  });
}

interface PreferenceVerdict {
  readonly status: SessionPreferenceStatus;
  readonly reason?: SessionPreferenceRejection;
  readonly assignments?: readonly SessionSlotProducer[];
}

// Mirrors `coordinator.evaluate_preference`'s fixed reason order: frozen ->
// seated -> shape -> team -> continuity -> claimed.
function evaluatePreference(
  state: FakeCoordState,
  peerId: string,
  requested: readonly InputSlotId[],
): PreferenceVerdict {
  if (state.freeze) {
    return { status: "rejected", reason: "after_freeze" };
  }
  const manifest = state.manifest;
  if (!manifest || !state.assignments) {
    return { status: "rejected", reason: "not_seated" };
  }
  const current = ownedSlotsOf(state.assignments, peerId);
  if (current.length === 0) {
    return { status: "rejected", reason: "not_seated" };
  }
  const shape = matchModeShape(manifest.match_mode);
  const seen = new Set<string>();
  for (const slot of requested) {
    if (SLOT_INDEX[slot] === undefined || seen.has(slot)) {
      return { status: "rejected", reason: "invalid_slot" };
    }
    seen.add(slot);
  }
  if (requested.length !== shape.slots_per_human) {
    return { status: "rejected", reason: "invalid_slot" };
  }
  const team = teamOfSlot(must(current[0], "evaluate_preference: peer owns no slots"));
  for (const slot of requested) {
    if (teamOfSlot(slot) !== team) {
      return { status: "rejected", reason: "wrong_team" };
    }
  }
  const held = new Set(current);
  let keeps = 0;
  for (const slot of requested) {
    if (held.has(slot)) {
      keeps += 1;
    }
  }
  if (keeps === 0) {
    return { status: "rejected", reason: "detached" };
  }
  const claimed = new Set<InputSlotId>();
  for (const peer of state.peers) {
    if (peer.peer_id !== peerId) {
      for (const slot of peer.pair_choice ?? []) {
        claimed.add(slot);
      }
    }
  }
  for (const slot of requested) {
    if (!held.has(slot) && claimed.has(slot)) {
      return { status: "rejected", reason: "already_taken" };
    }
  }
  if (keeps === current.length) {
    return { status: "unchanged" };
  }
  const assignments = exchangeAssignments(state.assignments, peerId, requested);
  return { status: "granted", assignments };
}

function claimsAfter(
  state: FakeCoordState,
  peerId: string,
  slots: readonly InputSlotId[],
): Readonly<Record<string, readonly InputSlotId[]>> {
  const claims: Record<string, readonly InputSlotId[]> = {};
  for (const peer of state.peers) {
    if (peer.pair_choice) {
      claims[peer.peer_id] = peer.pair_choice;
    }
  }
  claims[peerId] = slots;
  return claims;
}

// Every peer -- host publishing locally, or a guest that just received a
// `slot_assignment` wire -- runs its own local preference against the fresh
// ownership through this, so a claim that no longer fits reads `reseated`
// on whichever side is asking, exactly like `settle_preference`.
function settlePreference(state: FakeCoordState): FakeCoordState {
  const preference = state.preference;
  if (!preference || (preference.status !== "granted" && preference.status !== "unchanged")) {
    return state;
  }
  const owned = ownedSlotsOf(state.assignments ?? [], state.peer_id);
  if (slotListsEqual(owned, preference.slots)) {
    return state;
  }
  return {
    ...state,
    preference: { slots: preference.slots, status: "rejected", reason: "reseated" },
  };
}

function manifestIdOf(manifest: SessionManifest): string {
  return `manifest-${simpleHash(JSON.stringify(manifest))}`;
}

function assignmentIdOf(assignments: readonly SessionSlotProducer[], epoch: number): string {
  return `assignment-${epoch}-${simpleHash(JSON.stringify(assignments))}`;
}

function publishOwnership(
  state: FakeCoordState,
  assignments: readonly SessionSlotProducer[],
  retained: Readonly<Record<string, readonly InputSlotId[]>> | undefined,
  actions: CoordinatorAction[],
): FakeCoordState {
  const epoch = state.assignmentEpoch + 1;
  const assignmentId = assignmentIdOf(assignments, epoch);
  const peers = state.peers.map((peer) => {
    const claim = retained?.[peer.peer_id];
    const { pair_choice: _dropped, ...rest } = peer;
    return claim !== undefined
      ? { ...rest, ready: false, pair_choice: claim }
      : { ...rest, ready: false };
  });
  let next: FakeCoordState = {
    ...state,
    assignmentEpoch: epoch,
    assignment_id: assignmentId,
    assignments,
    phase: "assigned",
    peers,
  };
  next = settlePreference(next);
  actions.push({
    kind: "send",
    message: {
      kind: "slot_assignment",
      manifest_id: state.manifest_id,
      assignment_id: assignmentId,
      assignments,
    },
    targets: linkTargets(next),
  });
  return next;
}

function refreshReadyPhase(state: FakeCoordState): FakeCoordState {
  if (state.phase !== "assigned" && state.phase !== "ready") {
    return state;
  }
  const self = must(state.peers[0], "coordinator state has no self peer");
  const allReady = state.role === "guest" ? self.ready : state.peers.every((peer) => peer.ready);
  return { ...state, phase: allReady ? "ready" : "assigned" };
}

function skewReason(state: FakeCoordState, peer: FakePeer): CoordinatorTerminalReason | undefined {
  return peer.build_id !== state.buildId ? "build_mismatch" : undefined;
}

function terminateFrom(
  state: FakeCoordState,
  options: {
    readonly reason: CoordinatorTerminalReason;
    readonly detail?: string;
    readonly announce?: boolean;
    readonly excludeLink?: string;
  },
): readonly [FakeCoordState, CoordinatorOutcome] {
  const actions: CoordinatorAction[] = [];
  if (options.announce) {
    const targets = linkTargets(state).filter((target) => target !== options.excludeLink);
    if (targets.length > 0) {
      actions.push({
        kind: "send",
        message: { kind: "abort", code: TERMINAL_CODES[options.reason] },
        targets,
      });
    }
  }
  const next: FakeCoordState = {
    ...state,
    phase: "terminal",
    terminal: {
      reason: options.reason,
      ...(options.detail !== undefined ? { detail: options.detail } : {}),
    },
  };
  return [next, { accepted: true, actions }];
}

function dropGuest(
  state: FakeCoordState,
  peer: FakePeer,
  code: string,
  detail: string,
  reason?: CoordinatorTerminalReason,
): readonly [FakeCoordState, CoordinatorOutcome] {
  const targets = linkTargets(state);
  const departure: CoordinatorDeparture = {
    peer_id: peer.peer_id,
    reason: reason ?? DISCONNECT_REASONS[code] ?? "protocol_violation",
    code,
    detail,
  };
  const actions: CoordinatorAction[] = [
    { kind: "send", message: { kind: "disconnect", target_peer_id: peer.peer_id, code }, targets },
  ];
  const peers = state.peers
    .filter((p) => p.peer_id !== peer.peer_id)
    .map((p) => ({ ...p, ready: false }));
  const phase = state.phase === "ready" ? "assigned" : state.phase;
  const wasOwning = ownsSlot(state, peer.peer_id);
  const base: FakeCoordState = { ...state, peers, phase, departure };
  if (!wasOwning) {
    return [base, { accepted: true, actions }];
  }
  const { assignments: _a, assignment_id: _b, ...withoutAssignments } = base;
  return [withoutAssignments, { accepted: true, actions }];
}

// ---- Local command handlers ----

function handleConnect(state: FakeCoordState): readonly [FakeCoordState, CoordinatorOutcome] {
  if (state.role !== "guest" || state.phase !== "new") {
    return [state, { accepted: true, actions: [] }];
  }
  const next: FakeCoordState = { ...state, phase: "handshake" };
  const actions: CoordinatorAction[] = [
    {
      kind: "send",
      message: { kind: "handshake", role: "guest", build_id: state.buildId },
      targets: linkTargets(state),
    },
  ];
  return [next, { accepted: true, actions }];
}

function handleProposeManifest(
  state: FakeCoordState,
  event: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  if (state.role !== "host") {
    return [state, { accepted: false, reason: "not_permitted", actions: [] }];
  }
  const manifest = event["manifest"] as SessionManifest;
  const manifestId = manifestIdOf(manifest);
  if (state.manifest_id !== undefined) {
    if (state.manifest_id === manifestId) {
      return [state, { accepted: true, actions: [] }];
    }
    return [state, { accepted: false, reason: "identity_mismatch", actions: [] }];
  }
  const self = must(state.peers[0], "host has no self peer");
  const peers = [{ ...self, accepted_manifest_id: manifestId }, ...state.peers.slice(1)];
  const next: FakeCoordState = {
    ...state,
    manifest,
    manifest_id: manifestId,
    phase: "manifest",
    peers,
  };
  const actions: CoordinatorAction[] = [];
  const targets = linkTargets(next);
  if (targets.length > 0) {
    actions.push({
      kind: "send",
      message: { kind: "manifest_proposal", manifest_id: manifestId, manifest },
      targets,
    });
  }
  return [next, { accepted: true, actions }];
}

function handleAssignSlots(
  state: FakeCoordState,
  event: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  if (state.role !== "host") {
    return [state, { accepted: false, reason: "not_permitted", actions: [] }];
  }
  if (state.phase !== "manifest" && state.phase !== "assigned" && state.phase !== "ready") {
    return [state, { accepted: false, reason: "invalid_phase", actions: [] }];
  }
  for (const peer of state.peers) {
    if (peer.accepted_manifest_id !== state.manifest_id) {
      return [state, { accepted: false, reason: "invalid_phase", actions: [] }];
    }
  }
  let assignments = event["assignments"] as readonly SessionSlotProducer[];
  let retained: Readonly<Record<string, readonly InputSlotId[]>> | undefined;
  if (event["preserve_claims"] === true) {
    const shape = matchModeShape(
      must(state.manifest, "assign_slots requires a manifest").match_mode,
    );
    const reseat = reseatClaims(shape, assignments, state.peers);
    assignments = reseat.assignments;
    retained = reseat.retained;
  }
  if (assignmentsEqual(state.assignments, assignments)) {
    return [state, { accepted: true, actions: [] }];
  }
  const actions: CoordinatorAction[] = [];
  const next = publishOwnership(state, assignments, retained, actions);
  return [next, { accepted: true, actions }];
}

function handlePreferPair(
  state: FakeCoordState,
  event: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  if (state.phase !== "assigned" && state.phase !== "ready") {
    return [state, { accepted: false, reason: "invalid_phase", actions: [] }];
  }
  if (state.manifest_id === undefined || state.assignment_id === undefined) {
    return [state, { accepted: false, reason: "invalid_phase", actions: [] }];
  }
  const slots = event["slots"] as readonly InputSlotId[];
  if (state.role === "guest") {
    const preference: FakePreference = {
      slots,
      assignment_id: state.assignment_id,
      status: "pending",
      deadline: state.clock + PREFERENCE_TIMEOUT_TICKS,
    };
    const next: FakeCoordState = { ...state, preference };
    const actions: CoordinatorAction[] = [
      {
        kind: "send",
        message: {
          kind: "pair_preference",
          manifest_id: state.manifest_id,
          assignment_id: state.assignment_id,
          slots,
        },
        targets: linkTargets(state),
      },
    ];
    return [next, { accepted: true, actions }];
  }
  const verdict = evaluatePreference(state, state.peer_id, slots);
  const actions: CoordinatorAction[] = [];
  let next: FakeCoordState = {
    ...state,
    preference: {
      slots,
      status: verdict.status,
      ...(verdict.reason !== undefined ? { reason: verdict.reason } : {}),
    },
  };
  if (verdict.status === "granted" && verdict.assignments) {
    next = publishOwnership(
      next,
      verdict.assignments,
      claimsAfter(state, state.peer_id, slots),
      actions,
    );
  } else if (verdict.status === "unchanged") {
    const self = must(next.peers[0], "host has no self peer");
    next = { ...next, peers: [{ ...self, pair_choice: slots }, ...next.peers.slice(1)] };
  }
  return [next, { accepted: true, actions }];
}

function handleSetReady(
  state: FakeCoordState,
  event: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  const ready = event["ready"];
  if (typeof ready !== "boolean") {
    return [state, { accepted: false, reason: "malformed", actions: [] }];
  }
  if (state.phase !== "assigned" && state.phase !== "ready") {
    return [state, { accepted: false, reason: "invalid_phase", actions: [] }];
  }
  const self = must(state.peers[0], "coordinator state has no self peer");
  if (ready) {
    if (self.accepted_manifest_id !== state.manifest_id) {
      return [state, { accepted: false, reason: "identity_mismatch", actions: [] }];
    }
    if (!ownsSlot(state, self.peer_id)) {
      return [state, { accepted: false, reason: "invalid_assignment", actions: [] }];
    }
  }
  if (self.ready === ready) {
    return [state, { accepted: true, actions: [] }];
  }
  const actions: CoordinatorAction[] = [];
  if (state.role === "guest") {
    actions.push({
      kind: "send",
      message: {
        kind: "ready",
        manifest_id: state.manifest_id,
        assignment_id: state.assignment_id,
        ready,
      },
      targets: linkTargets(state),
    });
  }
  const peers = [{ ...self, ready }, ...state.peers.slice(1)];
  const next = refreshReadyPhase({ ...state, peers });
  return [next, { accepted: true, actions }];
}

function freezeSession(
  state: FakeCoordState,
  manifest: SessionManifest,
  countdownId: string,
  firstInputTick: number,
): FakeFreeze {
  const assignments = must(state.assignments, "freeze requires published assignments");
  const owned: Record<string, readonly InputSlotId[]> = {};
  for (const producer of assignments) {
    if (producer.producer_kind === "peer" && owned[producer.producer_id] === undefined) {
      owned[producer.producer_id] = ownedSlotsOf(assignments, producer.producer_id);
    }
  }
  return {
    manifest_id: must(state.manifest_id, "freeze requires a manifest id"),
    assignment_id: must(state.assignment_id, "freeze requires an assignment id"),
    countdown_id: countdownId,
    first_input_tick: firstInputTick,
    match_mode: manifest.match_mode,
    assignments,
    owned,
    live: previewLiveOf(assignments),
  };
}

function emitStart(state: FakeCoordState, actions: CoordinatorAction[]): FakeCoordState {
  const freeze = must(state.freeze, "emit_start requires a freeze");
  actions.push({
    kind: "send",
    message: {
      kind: "start",
      manifest_id: freeze.manifest_id,
      countdown_id: freeze.countdown_id,
      first_input_tick: freeze.first_input_tick,
    },
    targets: linkTargets(state),
  });
  let next: FakeCoordState = { ...state, countdown_remaining: 0 };
  const pending = state.peers.slice(1).some((peer) => !peer.started);
  if (!pending) {
    next = { ...next, phase: "running" };
    actions.push({ kind: "start_match", freeze });
  }
  return next;
}

function handleBeginCountdown(
  state: FakeCoordState,
  event: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  if (state.role !== "host") {
    return [state, { accepted: false, reason: "not_permitted", actions: [] }];
  }
  if (state.phase !== "ready") {
    return [state, { accepted: false, reason: "invalid_phase", actions: [] }];
  }
  const manifest = must(state.manifest, "begin_countdown requires a manifest");
  const countdownId = event["countdown_id"] as string;
  const remainingTicks = event["remaining_ticks"] as number;
  const firstInputTick = event["first_input_tick"] as number;
  const freeze = freezeSession(state, manifest, countdownId, firstInputTick);
  let next: FakeCoordState = {
    ...state,
    freeze,
    countdown_remaining: remainingTicks,
    phase: "countdown",
  };
  const actions: CoordinatorAction[] = [
    {
      kind: "send",
      message: {
        kind: "countdown",
        manifest_id: freeze.manifest_id,
        countdown_id: freeze.countdown_id,
        remaining_ticks: remainingTicks,
        first_input_tick: freeze.first_input_tick,
      },
      targets: linkTargets(next),
    },
  ];
  if (remainingTicks === 0) {
    next = emitStart(next, actions);
  }
  return [next, { accepted: true, actions }];
}

function expirePreference(state: FakeCoordState): FakeCoordState {
  const pending = state.preference;
  if (!pending || pending.status !== "pending") {
    return state;
  }
  const deadline = pending.deadline;
  if (deadline === undefined || state.clock <= deadline) {
    return state;
  }
  return {
    ...state,
    preference: { slots: pending.slots, status: "rejected", reason: "no_response" },
  };
}

function handleTick(state: FakeCoordState): readonly [FakeCoordState, CoordinatorOutcome] {
  let next: FakeCoordState = { ...state, clock: state.clock + 1 };
  if (state.phase === "terminal") {
    return [next, { accepted: true, actions: [] }];
  }
  next = expirePreference(next);
  const actions: CoordinatorAction[] = [];
  if (state.phase === "countdown") {
    const remaining = next.countdown_remaining ?? 0;
    if (remaining > 0) {
      next = { ...next, countdown_remaining: remaining - 1 };
      if (next.countdown_remaining === 0 && next.role === "host") {
        next = emitStart(next, actions);
      }
    }
  }
  return [next, { accepted: true, actions }];
}

function handleLeave(state: FakeCoordState): readonly [FakeCoordState, CoordinatorOutcome] {
  const actions: CoordinatorAction[] = [];
  if (state.phase !== "new") {
    actions.push({
      kind: "send",
      message: { kind: "disconnect", target_peer_id: state.peer_id, code: "peer_left" },
      targets: linkTargets(state),
    });
  }
  const next: FakeCoordState = { ...state, phase: "terminal", terminal: { reason: "guest_left" } };
  return [next, { accepted: true, actions }];
}

function handleAbort(
  state: FakeCoordState,
  event: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  const detail = event["detail"];
  return terminateFrom(state, {
    reason: "local_abort",
    ...(typeof detail === "string" ? { detail } : {}),
    announce: true,
  });
}

// ---- Control (wire) message handlers ----

function admitGuest(
  state: FakeCoordState,
  linkId: string,
  message: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  if (state.role !== "host") {
    return [state, { accepted: true, actions: [] }];
  }
  if (state.peers.some((peer) => peer.peer_id === linkId)) {
    return [state, { accepted: true, actions: [] }];
  }
  const buildId = message["build_id"] as string;
  const newPeer: FakePeer = {
    peer_id: linkId,
    link_id: linkId,
    role: "guest",
    build_id: buildId,
    ready: false,
    started: false,
  };
  const { departure: _oldDeparture, ...rest } = state;
  const next: FakeCoordState = {
    ...(rest as FakeCoordState),
    peers: [...state.peers, newPeer],
    phase: "handshake",
  };
  return [next, { accepted: true, actions: [] }];
}

function applyManifestProposal(
  state: FakeCoordState,
  peer: FakePeer,
  message: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  const manifest = message["manifest"] as SessionManifest;
  const manifestId = message["manifest_id"] as string;
  if (state.manifest_id !== undefined) {
    if (state.manifest_id === manifestId) {
      return [state, { accepted: true, actions: [] }];
    }
    return terminateFrom(state, {
      reason: "manifest_mismatch",
      detail: "the manifest is immutable after proposal",
      announce: true,
    });
  }
  const difference = expectationDifference(state.expectation, manifest);
  if (difference) {
    const reason = IDENTITY_REASONS[difference.path] ?? "manifest_mismatch";
    return terminateFrom(state, {
      reason,
      detail: `local identity differs at ${difference.path}`,
      announce: true,
    });
  }
  const self = must(state.peers[0], "guest has no self peer");
  const next: FakeCoordState = {
    ...state,
    manifest,
    manifest_id: manifestId,
    phase: "manifest",
    peers: [{ ...self, accepted_manifest_id: manifestId }, ...state.peers.slice(1)],
  };
  const actions: CoordinatorAction[] = [
    {
      kind: "send",
      message: { kind: "manifest_accept", manifest_id: manifestId },
      targets: linkTargets(next),
    },
  ];
  return [next, { accepted: true, actions }];
}

function applyManifestAccept(
  state: FakeCoordState,
  peer: FakePeer,
  message: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  const manifestId = message["manifest_id"] as string;
  if (manifestId !== state.manifest_id) {
    return dropGuest(
      state,
      peer,
      "protocol_error",
      "a guest accepted a manifest this session never proposed",
      skewReason(state, peer),
    );
  }
  if (peer.accepted_manifest_id === manifestId) {
    return [state, { accepted: true, actions: [] }];
  }
  const peers = state.peers.map((p) =>
    p.peer_id === peer.peer_id ? { ...p, accepted_manifest_id: manifestId } : p,
  );
  return [
    { ...state, peers },
    { accepted: true, actions: [] },
  ];
}

function applySlotAssignment(
  state: FakeCoordState,
  peer: FakePeer,
  message: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  const assignments = message["assignments"] as readonly SessionSlotProducer[];
  const assignmentId = message["assignment_id"] as string;
  const peers = state.peers.map((p) => ({ ...p, ready: false }));
  let next: FakeCoordState = {
    ...state,
    assignments,
    assignment_id: assignmentId,
    phase: "assigned",
    peers,
  };
  next = settlePreference(next);
  return [next, { accepted: true, actions: [] }];
}

function applyReady(
  state: FakeCoordState,
  peer: FakePeer,
  message: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  const readyValue = message["ready"] as boolean;
  const assignmentId = message["assignment_id"];
  if (assignmentId !== state.assignment_id) {
    return [state, { accepted: false, reason: "invalid_assignment", actions: [] }];
  }
  if (readyValue && !ownsSlot(state, peer.peer_id)) {
    return [state, { accepted: false, reason: "invalid_assignment", actions: [] }];
  }
  if (peer.ready === readyValue) {
    return [state, { accepted: true, actions: [] }];
  }
  const peers = state.peers.map((p) =>
    p.peer_id === peer.peer_id ? { ...p, ready: readyValue } : p,
  );
  const next = refreshReadyPhase({ ...state, peers });
  return [next, { accepted: true, actions: [] }];
}

function applyPairPreference(
  state: FakeCoordState,
  peer: FakePeer,
  message: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  const slots = message["slots"] as readonly InputSlotId[];
  const bodyAssignmentId = message["assignment_id"] as string;
  let verdict: PreferenceVerdict;
  if (state.freeze) {
    verdict = { status: "rejected", reason: "after_freeze" };
  } else if (bodyAssignmentId !== state.assignment_id) {
    verdict = { status: "rejected", reason: "superseded" };
  } else {
    verdict = evaluatePreference(state, peer.peer_id, slots);
  }
  const actions: CoordinatorAction[] = [];
  let next = state;
  if (verdict.status === "granted" && verdict.assignments) {
    next = publishOwnership(
      state,
      verdict.assignments,
      claimsAfter(state, peer.peer_id, slots),
      actions,
    );
  } else if (verdict.status === "unchanged") {
    const peers = state.peers.map((p) =>
      p.peer_id === peer.peer_id ? { ...p, pair_choice: slots } : p,
    );
    next = { ...state, peers };
  }
  actions.push({
    kind: "send",
    message: {
      kind: "pair_preference_result",
      manifest_id: state.manifest_id,
      assignment_id: bodyAssignmentId,
      slots,
      status: verdict.status,
      ...(verdict.reason !== undefined ? { reason: verdict.reason } : {}),
    },
    targets: [peer.link_id],
  });
  return [next, { accepted: true, actions }];
}

function applyPairPreferenceResult(
  state: FakeCoordState,
  peer: FakePeer,
  message: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  const pending = state.preference;
  const bodySlots = message["slots"] as readonly InputSlotId[];
  const bodyAssignmentId = message["assignment_id"] as string;
  if (
    !pending ||
    pending.status !== "pending" ||
    pending.assignment_id !== bodyAssignmentId ||
    !slotListsEqual(pending.slots, bodySlots)
  ) {
    return [state, { accepted: true, actions: [] }];
  }
  const status = message["status"] as SessionPreferenceStatus;
  const reason = message["reason"] as SessionPreferenceRejection | undefined;
  const next: FakeCoordState = {
    ...state,
    preference: {
      slots: pending.slots,
      assignment_id: pending.assignment_id,
      status,
      ...(reason !== undefined ? { reason } : {}),
    },
  };
  return [next, { accepted: true, actions: [] }];
}

function applyCountdown(
  state: FakeCoordState,
  peer: FakePeer,
  message: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  const countdownId = message["countdown_id"] as string;
  const firstInputTick = message["first_input_tick"] as number;
  const remainingTicks = message["remaining_ticks"] as number;
  const freeze = freezeSession(
    state,
    must(state.manifest, "countdown requires a manifest"),
    countdownId,
    firstInputTick,
  );
  const next: FakeCoordState = {
    ...state,
    freeze,
    countdown_remaining: remainingTicks,
    phase: "countdown",
  };
  return [next, { accepted: true, actions: [] }];
}

function applyStart(
  state: FakeCoordState,
  peer: FakePeer,
  _message: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  const freeze = must(state.freeze, "start requires a freeze");
  if (state.role === "guest") {
    const self = must(state.peers[0], "guest has no self peer");
    if (self.started) {
      return [state, { accepted: true, actions: [] }];
    }
    const actions: CoordinatorAction[] = [
      {
        kind: "send",
        message: {
          kind: "start",
          manifest_id: freeze.manifest_id,
          countdown_id: freeze.countdown_id,
          first_input_tick: freeze.first_input_tick,
        },
        targets: linkTargets(state),
      },
    ];
    const next: FakeCoordState = {
      ...state,
      peers: [{ ...self, started: true }, ...state.peers.slice(1)],
      countdown_remaining: 0,
      phase: "running",
    };
    actions.push({ kind: "start_match", freeze });
    return [next, { accepted: true, actions }];
  }
  if (peer.started) {
    return [state, { accepted: true, actions: [] }];
  }
  const peers = state.peers.map((p) => (p.peer_id === peer.peer_id ? { ...p, started: true } : p));
  let next: FakeCoordState = { ...state, peers };
  const pending = peers.slice(1).some((p) => !p.started);
  const actions: CoordinatorAction[] = [];
  if (!pending && next.countdown_remaining === 0) {
    next = { ...next, phase: "running" };
    actions.push({ kind: "start_match", freeze });
  }
  return [next, { accepted: true, actions }];
}

function applyAbort(
  state: FakeCoordState,
  peer: FakePeer,
  message: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  const code = message["code"] as string;
  if (state.role === "host" && !state.freeze) {
    const reason = code === "manifest_mismatch" ? skewReason(state, peer) : undefined;
    return dropGuest(state, peer, "protocol_error", `a guest aborted with ${code}`, reason);
  }
  return terminateFrom(state, { reason: "peer_abort", announce: true, excludeLink: peer.link_id });
}

function applyDisconnect(
  state: FakeCoordState,
  peer: FakePeer,
  message: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  const code = message["code"] as string;
  const targetPeerId = message["target_peer_id"] as string;
  const reason = DISCONNECT_REASONS[code] ?? "protocol_violation";
  if (state.role === "guest") {
    if (targetPeerId !== state.peer_id && code !== "host_left") {
      const self = must(state.peers[0], "guest has no self peer");
      if (!self.ready) {
        return [state, { accepted: true, actions: [] }];
      }
      const peers = state.peers.map((p) => ({ ...p, ready: false }));
      const phase = state.phase === "ready" ? "assigned" : state.phase;
      return [
        { ...state, peers, phase },
        { accepted: true, actions: [] },
      ];
    }
    return terminateFrom(state, { reason: code === "host_left" ? "host_left" : "removed" });
  }
  if (targetPeerId !== peer.peer_id) {
    return terminateFrom(state, {
      reason: "protocol_violation",
      detail: "a guest cannot disconnect another peer",
      announce: true,
    });
  }
  if (state.freeze) {
    return terminateFrom(state, { reason, announce: true, excludeLink: peer.link_id });
  }
  return dropGuest(state, peer, code, `a guest announced its own disconnect as ${code}`);
}

function handleControl(
  state: FakeCoordState,
  event: Readonly<Record<string, unknown>>,
): readonly [FakeCoordState, CoordinatorOutcome] {
  const linkId = event["link_id"] as string;
  const wire = event["wire"] as string;
  const message = JSON.parse(wire) as Readonly<Record<string, unknown>> & { readonly kind: string };
  if (message.kind === "handshake") {
    return admitGuest(state, linkId, message);
  }
  const peer = state.peers.find((p) => p.peer_id === linkId);
  if (!peer) {
    return [state, { accepted: true, actions: [] }];
  }
  switch (message.kind) {
    case "manifest_proposal":
      return applyManifestProposal(state, peer, message);
    case "manifest_accept":
      return applyManifestAccept(state, peer, message);
    case "slot_assignment":
      return applySlotAssignment(state, peer, message);
    case "ready":
      return applyReady(state, peer, message);
    case "pair_preference":
      return applyPairPreference(state, peer, message);
    case "pair_preference_result":
      return applyPairPreferenceResult(state, peer, message);
    case "countdown":
      return applyCountdown(state, peer, message);
    case "start":
      return applyStart(state, peer, message);
    case "abort":
      return applyAbort(state, peer, message);
    case "disconnect":
      return applyDisconnect(state, peer, message);
    default:
      return [state, { accepted: true, actions: [] }];
  }
}

function stepImpl(
  rawState: CoordinatorState,
  event: { readonly kind: string; readonly [key: string]: unknown },
): readonly [CoordinatorState, CoordinatorOutcome] {
  const state = toFake(rawState);
  let result: readonly [FakeCoordState, CoordinatorOutcome];
  switch (event.kind) {
    case "connect":
      result = handleConnect(state);
      break;
    case "propose_manifest":
      result = handleProposeManifest(state, event);
      break;
    case "assign_slots":
      result = handleAssignSlots(state, event);
      break;
    case "prefer_pair":
      result = handlePreferPair(state, event);
      break;
    case "set_ready":
      result = handleSetReady(state, event);
      break;
    case "begin_countdown":
      result = handleBeginCountdown(state, event);
      break;
    case "tick":
      result = handleTick(state);
      break;
    case "abort":
      result = handleAbort(state, event);
      break;
    case "leave":
      result = handleLeave(state);
      break;
    case "control":
      result = handleControl(state, event);
      break;
    default:
      result = [state, { accepted: true, actions: [] }];
  }
  return [toPublic(result[0]), result[1]];
}

function createCoordinator(
  options: CoordinatorNewHostOptions | CoordinatorNewGuestOptions,
): CoordinatorState {
  if (options.role === "host") {
    const self: FakePeer = {
      peer_id: options.peer_id,
      link_id: options.peer_id,
      role: "host",
      build_id: options.build_id,
      ready: false,
      started: false,
    };
    const state: FakeCoordState = {
      role: "host",
      peer_id: options.peer_id,
      phase: "handshake",
      peers: [self],
      hostPeerId: options.peer_id,
      buildId: options.build_id,
      clock: 0,
      assignmentEpoch: 0,
    };
    return toPublic(state);
  }
  const self: FakePeer = {
    peer_id: options.peer_id,
    link_id: options.host_link_id,
    role: "guest",
    build_id: options.build_id,
    ready: false,
    started: false,
  };
  const hostPlaceholder: FakePeer = {
    peer_id: options.host_peer_id,
    link_id: options.host_link_id,
    role: "host",
    build_id: "",
    ready: false,
    started: false,
  };
  const state: FakeCoordState = {
    role: "guest",
    peer_id: options.peer_id,
    phase: "new",
    peers: [self, hostPlaceholder],
    hostPeerId: options.host_peer_id,
    buildId: options.build_id,
    expectation: options.expectation,
    clock: 0,
    assignmentEpoch: 0,
  };
  return toPublic(state);
}

function ownershipSeatsRoster(rawState: CoordinatorState): boolean {
  const state = toFake(rawState);
  if (!state.assignments || !state.manifest) {
    return false;
  }
  const shape = matchModeShape(state.manifest.match_mode);
  const counts: Record<string, number> = {};
  for (const producer of state.assignments) {
    if (producer.producer_kind === "peer") {
      counts[producer.producer_id] = (counts[producer.producer_id] ?? 0) + 1;
    }
  }
  return state.peers.every((peer) => (counts[peer.peer_id] ?? 0) === shape.slots_per_human);
}

function fakeCoordinatorPort(): CoordinatorPort {
  return {
    create: createCoordinator,
    step: stepImpl,
    planAssignments,
    ownedSlots: (state, peerId) => ownedSlotsOf(toFake(state).assignments ?? [], peerId),
    previewLive: previewLiveOf,
    ownershipSeatsRoster,
  };
}

function ports(): LobbyModelPorts {
  return {
    coordinator: fakeCoordinatorPort(),
    protocol: fakeProtocol(),
    protocolFixture: fakeProtocolFixture(),
    transportContract: fakeTransportContract(),
    fnv1a64: fakeFnv1a64(),
    inputFrame: fakeInputFrame(),
  };
}

// ---------------------------------------------------------------------------
// FakeTransport -- see this file's header for why this exists instead of
// `@gc/online`'s real `lobby_link.ts` / a real star transport.
// ---------------------------------------------------------------------------

type PendingStage =
  "requested" | "offer_sent" | "offer_accepted" | "answer_sent" | "connecting" | "connected";

interface PendingLink {
  stage: PendingStage;
  readonly hostPeerId: string;
  readonly guestPeerId: string;
}

class FakeTransport {
  private readonly mailboxes = new Map<string, LobbyCommand[]>();
  private readonly pending = new Map<string, PendingLink>();

  private mailbox(peerId: string): LobbyCommand[] {
    let box = this.mailboxes.get(peerId);
    if (!box) {
      box = [];
      this.mailboxes.set(peerId, box);
    }
    return box;
  }

  applyEffect(selfPeerId: string, effect: LobbyEffect): void {
    switch (effect.kind) {
      case "open_peer":
        this.pending.set(effect.peer_id, {
          stage: "requested",
          hostPeerId: selfPeerId,
          guestPeerId: effect.peer_id,
        });
        break;
      case "accept_offer": {
        const link = this.pending.get(selfPeerId);
        if (link) {
          link.stage = "offer_accepted";
        }
        break;
      }
      case "accept_answer": {
        const link = this.pending.get(effect.peer_id);
        if (link) {
          link.stage = "connecting";
        }
        break;
      }
      case "send":
        this.mailbox(effect.link_id).push({
          kind: "control",
          link_id: selfPeerId,
          wire: effect.wire,
        });
        break;
      default:
        break;
    }
  }

  pumpOnce(): void {
    for (const link of this.pending.values()) {
      if (link.stage === "requested") {
        this.mailbox(link.hostPeerId).push({
          kind: "signal",
          peer_id: link.guestPeerId,
          signal: `offer:${link.guestPeerId}`,
        });
        link.stage = "offer_sent";
      } else if (link.stage === "offer_accepted") {
        this.mailbox(link.guestPeerId).push({
          kind: "signal",
          peer_id: link.hostPeerId,
          signal: `answer:${link.guestPeerId}`,
        });
        link.stage = "answer_sent";
      } else if (link.stage === "connecting") {
        this.mailbox(link.hostPeerId).push({ kind: "peer_connected", peer_id: link.guestPeerId });
        this.mailbox(link.guestPeerId).push({ kind: "peer_connected", peer_id: link.hostPeerId });
        link.stage = "connected";
      }
    }
  }

  drain(peerId: string): LobbyCommand[] {
    const box = this.mailbox(peerId);
    this.mailboxes.set(peerId, []);
    return box;
  }
}

// ---------------------------------------------------------------------------
// The bare-model test driver: wires a fake transport and a roster of lobby
// models together so a scenario can be scripted peer by peer.
// ---------------------------------------------------------------------------

interface TestPeer {
  readonly id: string;
  model: LobbyModel;
  clipboard?: string;
  freeze?: unknown;
  left: boolean;
  readonly sent: string[];
}

interface TestFreeze {
  readonly match_mode: SessionMatchMode;
  readonly manifest_id: string;
  readonly owned: Readonly<Record<string, readonly InputSlotId[]>>;
  readonly live: Readonly<Record<string, InputSlotId>>;
}

class Driver {
  readonly network = new FakeTransport();
  readonly peers: TestPeer[] = [];
  private readonly modelPorts: LobbyModelPorts;

  constructor(modelPorts: LobbyModelPorts) {
    this.modelPorts = modelPorts;
  }

  add(
    role: LobbyRole,
    peerId: string,
    template?: (mode: SessionMatchMode) => SessionManifest,
  ): TestPeer {
    const peer: TestPeer = {
      id: peerId,
      model: newLobbyModel(this.modelPorts, { peer_id: peerId, ...(template ? { template } : {}) }),
      left: false,
      sent: [],
    };
    this.peers.push(peer);
    this.send(peer, { kind: "role", role });
    return peer;
  }

  send(peer: TestPeer, cmd: LobbyCommand): void {
    const [model, effects] = command(peer.model, this.modelPorts, cmd);
    peer.model = model;
    for (const effect of effects) {
      this.runEffect(peer, effect);
    }
  }

  private runEffect(peer: TestPeer, effect: LobbyEffect): void {
    if (effect.kind === "clipboard") {
      peer.clipboard = effect.text;
    } else if (effect.kind === "start_match") {
      peer.freeze = effect.freeze;
    } else if (effect.kind === "leave") {
      peer.left = true;
    } else if (effect.kind === "paste_request") {
      // Only meaningful for the mounted-screen shell, not the bare driver.
    } else if (effect.kind === "send") {
      peer.sent.push(effect.wire);
      this.network.applyEffect(peer.id, effect);
    } else {
      this.network.applyEffect(peer.id, effect);
    }
  }

  pump(rounds = 6): void {
    for (let i = 0; i < rounds; i += 1) {
      this.network.pumpOnce();
      for (const peer of this.peers) {
        for (const event of this.network.drain(peer.id)) {
          this.send(peer, event);
        }
      }
    }
  }

  tick(count: number): void {
    for (let i = 0; i < count; i += 1) {
      for (const peer of this.peers) {
        this.send(peer, { kind: "tick" });
      }
      this.pump(2);
    }
  }

  // One complete manual handshake: the host invites, both sides copy and
  // paste, and the guest's coordinator handshake reaches the host.
  connect(host: TestPeer, guest: TestPeer): void {
    this.send(host, { kind: "invite" });
    this.pump(2);
    this.send(host, { kind: "copy" });
    const offer = host.clipboard;
    if (offer === undefined) {
      throw new Error("the host produced no offer");
    }
    this.send(guest, { kind: "paste", text: offer });
    this.pump(2);
    this.send(guest, { kind: "copy" });
    const answer = guest.clipboard;
    if (answer === undefined) {
      throw new Error("the guest produced no answer");
    }
    this.send(host, { kind: "paste", text: answer });
    this.pump(4);
  }
}

function seatedLobby(
  modelPorts: LobbyModelPorts,
  mode: SessionMatchMode,
  guestCount: number,
): { readonly driver: Driver; readonly host: TestPeer; readonly guests: TestPeer[] } {
  const driver = new Driver(modelPorts);
  const host = driver.add("host", "host");
  driver.send(host, { kind: "mode", mode });
  const guests: TestPeer[] = [];
  for (let i = 1; i <= guestCount; i += 1) {
    const guest = driver.add("guest", `guest_${i}`);
    driver.connect(host, guest);
    guests.push(guest);
  }
  return { driver, host, guests };
}

function view(modelPorts: LobbyModelPorts, peer: TestPeer) {
  return lobbyModelView(modelPorts, peer.model);
}

function owned(modelPorts: LobbyModelPorts, peer: TestPeer, producerId: string): string[] {
  const slots: string[] = [];
  for (const slot of view(modelPorts, peer).slots) {
    if (slot.owner === producerId) {
      slots.push(slot.slot);
    }
  }
  return slots;
}

// ---------------------------------------------------------------------------
// "manual lobby handshake"
// ---------------------------------------------------------------------------

describe("manual lobby handshake", () => {
  it("completes offer and answer exchange without console commands", () => {
    const modelPorts = ports();
    const { driver, host, guests } = seatedLobby(modelPorts, "1v1", 1);
    expect(view(modelPorts, host).connected).toBe(2);
    expect(view(modelPorts, must(guests[0], "no guest"))).toBeTruthy();
    expect(view(modelPorts, must(guests[0], "no guest")).connected).toBe(2);
    expect(host.model.pending_link).toBeUndefined();
    driver.pump(1);
  });

  it("never retains the pasted blob after it is used", () => {
    const modelPorts = ports();
    const { host } = seatedLobby(modelPorts, "1v1", 1);
    expect(host.model.outgoing).toBeUndefined();
    const record = must(view(modelPorts, host).imported, "no imported signal recorded");
    expect(record.direction).toBe("answer");
    expect(record.bytes > 0).toBe(true);
    expect(record.fingerprint.length).toBe(8);
  });

  it("reports a malformed paste without ending the session", () => {
    const modelPorts = ports();
    const driver = new Driver(modelPorts);
    const host = driver.add("host", "host");
    driver.send(host, { kind: "paste", text: "" });
    expect(view(modelPorts, host).error).toBeTruthy();
    expect(view(modelPorts, host).phase).toBe("handshake");
    driver.send(host, { kind: "invite" });
    expect(view(modelPorts, host).error).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// "lobby match modes"
// ---------------------------------------------------------------------------

describe("lobby match modes", () => {
  it("seats a 1v1 human on a whole outfield line", () => {
    const modelPorts = ports();
    const { driver, host, guests } = seatedLobby(modelPorts, "1v1", 1);
    driver.send(host, { kind: "lock" });
    driver.pump(6);

    const hostSlots = owned(modelPorts, host, "host");
    expect(hostSlots.length).toBe(4);
    expect(hostSlots[0]).toBe("home_1");
    expect(hostSlots[3]).toBe("home_4");
    expect(owned(modelPorts, host, "guest_1").length).toBe(4);
    expect(owned(modelPorts, must(guests[0], "no guest"), "guest_1").length).toBe(4);
  });

  it("shows AI-driven slots inside a human's owned set", () => {
    const modelPorts = ports();
    const { driver, host } = seatedLobby(modelPorts, "1v1", 1);
    driver.send(host, { kind: "lock" });
    driver.pump(6);

    let human = 0;
    let ai = 0;
    for (const slot of view(modelPorts, host).slots) {
      if (slot.owner === "host") {
        if (slot.driver === "human") {
          human += 1;
        } else {
          ai += 1;
          expect(slot.owner_kind).toBe("peer");
        }
      }
    }
    expect(human).toBe(1);
    expect(ai).toBe(3);
  });

  it("seats a 2v2 human on a chosen pair and repartitions on a swap", () => {
    const modelPorts = ports();
    const { driver, host, guests } = seatedLobby(modelPorts, "2v2", 3);
    driver.send(host, { kind: "lock" });
    driver.pump(8);

    expect(owned(modelPorts, host, "host").length).toBe(2);
    expect(owned(modelPorts, host, "host").join(",")).toBe("home_1,home_2");
    expect(owned(modelPorts, host, "guest_1").join(",")).toBe("home_3,home_4");

    driver.send(host, { kind: "ready", ready: true });
    driver.pump(2);
    expect(view(modelPorts, host).ready).toBe(true);

    driver.send(host, { kind: "swap", index: 1 });
    driver.pump(4);
    expect(owned(modelPorts, host, "host").join(",")).toBe("home_3,home_4");
    expect(owned(modelPorts, host, "guest_1").join(",")).toBe("home_1,home_2");
    expect(view(modelPorts, host).ready).toBe(false);
    expect(owned(modelPorts, must(guests[0], "no guest"), "guest_1").length).toBe(2);
  });

  it("gates the required peer count on the mode", () => {
    const modelPorts = ports();
    const driver = new Driver(modelPorts);
    const host = driver.add("host", "host");
    driver.send(host, { kind: "mode", mode: "2v2" });
    expect(view(modelPorts, host).required).toBe(4);
    driver.send(host, { kind: "lock" });
    expect(view(modelPorts, host).error).toBeTruthy();
    expect(view(modelPorts, host).phase).toBe("handshake");

    driver.send(host, { kind: "mode", mode: "1v1" });
    expect(view(modelPorts, host).required).toBe(2);
  });

  it("fills empty seats with AI only when the host approves it", () => {
    const modelPorts = ports();
    const driver = new Driver(modelPorts);
    const host = driver.add("host", "host");
    driver.send(host, { kind: "mode", mode: "4v4" });
    driver.send(host, { kind: "bot_fill" });
    driver.send(host, { kind: "lock" });
    driver.pump(4);

    expect(owned(modelPorts, host, "host").length).toBe(1);
    let bots = 0;
    for (const slot of view(modelPorts, host).slots) {
      if (slot.owner_kind === "bot") {
        bots += 1;
        expect(slot.driver).toBe("ai");
      }
    }
    expect(bots).toBe(7);
  });

  it("keeps both keepers protected and slotless in every mode", () => {
    for (const mode of ["1v1", "2v2", "4v4"] as const) {
      const modelPorts = ports();
      const driver = new Driver(modelPorts);
      const host = driver.add("host", "host");
      driver.send(host, { kind: "mode", mode });
      const keepers = view(modelPorts, host).keepers;
      expect(keepers.length).toBe(2);
      for (const slot of view(modelPorts, host).slots) {
        for (const keeper of keepers) {
          expect(slot.player_id).not.toBe(keeper.player_id);
        }
      }
    }
  });

  it("locks the mode once the manifest is proposed", () => {
    const modelPorts = ports();
    const { driver, host } = seatedLobby(modelPorts, "1v1", 1);
    driver.send(host, { kind: "lock" });
    driver.pump(4);
    expect(view(modelPorts, host).mode_locked).toBe(true);
    driver.send(host, { kind: "mode", mode: "4v4" });
    expect(view(modelPorts, host).error).toBeTruthy();
    expect(view(modelPorts, host).mode).toBe("1v1");
  });
});

// ---------------------------------------------------------------------------
// "lobby readiness and countdown"
// ---------------------------------------------------------------------------

describe("lobby readiness and countdown", () => {
  it("reaches a synchronized start only after every peer is ready", () => {
    const modelPorts = ports();
    const { driver, host, guests } = seatedLobby(modelPorts, "1v1", 1);
    const guest = must(guests[0], "no guest");
    driver.send(host, { kind: "lock" });
    driver.pump(6);

    driver.send(host, { kind: "start" });
    expect(view(modelPorts, host).error).toBeTruthy();

    driver.send(host, { kind: "ready", ready: true });
    driver.send(guest, { kind: "ready", ready: true });
    driver.pump(4);
    expect(view(modelPorts, host).phase).toBe("ready");
    expect(view(modelPorts, host).can_start).toBe(true);

    driver.send(host, { kind: "start" });
    driver.pump(2);
    expect(view(modelPorts, host).phase).toBe("countdown");
    expect(view(modelPorts, guest).countdown).not.toBeUndefined();

    driver.tick(180 + 4);
    const freeze = must(host.freeze, "the host never reached the start boundary") as TestFreeze;
    expect(freeze.match_mode).toBe("1v1");
    const guestFreeze = must(
      guest.freeze,
      "the guest never reached the start boundary",
    ) as TestFreeze;
    expect(guestFreeze.manifest_id).toBe(freeze.manifest_id);
    expect(must(freeze.owned["host"], "no owned slots for host").length).toBe(4);
    expect(freeze.live["host"]).toBe("home_1");
  });

  it("clears readiness when ownership is republished", () => {
    const modelPorts = ports();
    const { driver, host, guests } = seatedLobby(modelPorts, "2v2", 3);
    driver.send(host, { kind: "lock" });
    driver.pump(8);
    for (const peer of [host, ...guests]) {
      driver.send(peer, { kind: "ready", ready: true });
    }
    driver.pump(4);
    expect(view(modelPorts, host).phase).toBe("ready");

    driver.send(host, { kind: "swap", index: 2 });
    driver.pump(4);
    expect(view(modelPorts, host).phase).toBe("assigned");
    expect(view(modelPorts, host).ready_count).toBe(0);
  });
});

// ---------------------------------------------------------------------------
// "lobby pair selection"
// ---------------------------------------------------------------------------

describe("lobby pair selection", () => {
  function offersPair(modelPorts: LobbyModelPorts, peer: TestPeer, slot: InputSlotId): boolean {
    for (const row of view(modelPorts, peer).slots) {
      if (row.slot === slot) {
        return row.can_prefer;
      }
    }
    return false;
  }

  function lockedLobby(modelPorts: LobbyModelPorts, mode: SessionMatchMode, guestCount: number) {
    const result = seatedLobby(modelPorts, mode, guestCount);
    result.driver.send(result.host, { kind: "lock" });
    result.driver.pump(8);
    return result;
  }

  it("lets a guest choose its pair and shows the request through to the grant", () => {
    const modelPorts = ports();
    const { driver, host, guests } = lockedLobby(modelPorts, "2v2", 3);
    const chooser = must(guests[1], "no second guest");
    expect(owned(modelPorts, chooser, "guest_2").join(",")).toBe("away_1,away_2");
    expect(offersPair(modelPorts, chooser, "away_3")).toBe(true);

    driver.send(chooser, { kind: "pair", slot: "away_3" });
    const pending = must(
      view(modelPorts, chooser).preference,
      "the request must be visible at once",
    );
    expect(pending.status).toBe("pending");
    expect(pending.slots.join(",")).toBe("away_1,away_3");

    driver.pump(8);
    const granted = must(view(modelPorts, chooser).preference, "no preference after grant");
    expect(granted.status).toBe("granted");
    expect(owned(modelPorts, chooser, "guest_2").join(",")).toBe("away_1,away_3");
    expect(owned(modelPorts, host, "guest_2").join(",")).toBe("away_1,away_3");
    expect(owned(modelPorts, host, "guest_3").join(",")).toBe("away_2,away_4");
    expect(view(modelPorts, host).ready).toBe(false);
  });

  it("keeps a granted pair through the traffic that follows it", () => {
    const modelPorts = ports();
    const { driver, host, guests } = lockedLobby(modelPorts, "2v2", 3);
    driver.send(must(guests[1], "no second guest"), { kind: "pair", slot: "away_3" });
    driver.pump(8);
    expect(owned(modelPorts, host, "guest_2").join(",")).toBe("away_1,away_3");

    for (const peer of [host, ...guests]) {
      driver.send(peer, { kind: "ready", ready: true });
    }
    driver.pump(8);
    expect(view(modelPorts, host).phase).toBe("ready");
    expect(owned(modelPorts, host, "guest_2").join(",")).toBe("away_1,away_3");
    expect(owned(modelPorts, must(guests[1], "no second guest"), "guest_2").join(",")).toBe(
      "away_1,away_3",
    );
  });

  it("shows the typed reason when the host refuses", () => {
    const modelPorts = ports();
    const { driver, host, guests } = lockedLobby(modelPorts, "2v2", 3);
    driver.send(must(guests[1], "no second guest"), { kind: "pair", slot: "away_3" });
    driver.pump(8);

    driver.send(must(guests[2], "no third guest"), { kind: "pair", slot: "away_3" });
    driver.pump(8);
    const refused = must(
      view(modelPorts, must(guests[2], "no third guest")).preference,
      "no refusal recorded",
    );
    expect(refused.status).toBe("rejected");
    expect(refused.reason).toBe("already_taken");
    expect(owned(modelPorts, must(guests[2], "no third guest"), "guest_3").join(",")).toBe(
      "away_2,away_4",
    );
    expect(owned(modelPorts, host, "guest_2").join(",")).toBe("away_1,away_3");
  });

  function assertPartition(
    modelPorts: LobbyModelPorts,
    peer: TestPeer,
    mode: SessionMatchMode,
  ): void {
    const shape = matchModeShape(mode);
    const counts: Record<string, number> = {};
    const rows = view(modelPorts, peer).slots;
    expect(rows.length).toBe(8);
    for (const row of rows) {
      const owner = must(row.owner, `${row.slot} has no producer`);
      counts[owner] = (counts[owner] ?? 0) + 1;
    }
    for (const seat of view(modelPorts, peer).seats) {
      expect(counts[seat.peer_id] ?? 0).toBe(shape.slots_per_human);
    }
  }

  it("keeps the pair a roster change still fits and says why the other went", () => {
    const modelPorts = ports();
    const { driver, host, guests } = lockedLobby(modelPorts, "2v2", 3);
    const guest1 = must(guests[0], "no first guest");
    const guest2 = must(guests[1], "no second guest");
    const guest3 = must(guests[2], "no third guest");
    driver.send(guest1, { kind: "pair", slot: "home_2" });
    driver.pump(8);
    driver.send(guest2, { kind: "pair", slot: "away_3" });
    driver.pump(8);
    expect(owned(modelPorts, host, "guest_1").join(",")).toBe("home_2,home_3");
    expect(owned(modelPorts, host, "guest_2").join(",")).toBe("away_1,away_3");

    driver.send(guest3, { kind: "leave" });
    driver.pump(8);

    expect(owned(modelPorts, host, "guest_1").join(",")).toBe("home_2,home_3");
    expect(owned(modelPorts, guest1, "guest_1").join(",")).toBe("home_2,home_3");
    expect(must(view(modelPorts, guest1).preference, "no preference for guest_1").status).toBe(
      "granted",
    );

    const dropped = must(view(modelPorts, guest2).preference, "a dropped pair must still be shown");
    expect(dropped.status).toBe("rejected");
    expect(dropped.reason).toBe("reseated");
    expect(owned(modelPorts, guest2, "guest_2").join(",")).toBe("away_1,away_2");
    expect(owned(modelPorts, host, "guest_2").join(",")).toBe("away_1,away_2");
    assertPartition(modelPorts, host, "2v2");
    assertPartition(modelPorts, guest1, "2v2");
    assertPartition(modelPorts, guest2, "2v2");
    expect(view(modelPorts, host).ready).toBe(false);
  });

  it("tells a guest when the host's swap took its pair back", () => {
    const modelPorts = ports();
    const { driver, host, guests } = lockedLobby(modelPorts, "2v2", 3);
    const guest2 = must(guests[1], "no second guest");
    driver.send(guest2, { kind: "pair", slot: "away_3" });
    driver.pump(8);
    expect(must(view(modelPorts, guest2).preference, "no preference").status).toBe("granted");
    expect(owned(modelPorts, guest2, "guest_2").join(",")).toBe("away_1,away_3");

    driver.send(host, { kind: "swap", index: 3 });
    driver.pump(8);

    const taken = must(
      view(modelPorts, guest2).preference,
      "a swapped-away pair must still be shown",
    );
    expect(taken.status).toBe("rejected");
    expect(taken.reason).toBe("reseated");
    expect(owned(modelPorts, guest2, "guest_2").join(",")).toBe("away_3,away_4");
    expect(owned(modelPorts, host, "guest_2").join(",")).toBe("away_3,away_4");
    assertPartition(modelPorts, host, "2v2");
    assertPartition(modelPorts, guest2, "2v2");
  });

  it("offers nothing to choose in 1v1 or 4v4", () => {
    for (const testCase of [
      { mode: "1v1" as const, guests: 1 },
      { mode: "4v4" as const, guests: 7 },
    ]) {
      const modelPorts = ports();
      const { driver, host } = lockedLobby(modelPorts, testCase.mode, testCase.guests);
      for (const row of view(modelPorts, host).slots) {
        expect(row.can_prefer).toBe(false);
      }
      driver.send(host, { kind: "pair", slot: "home_4" });
      expect(view(modelPorts, host).error).toBeTruthy();
      expect(view(modelPorts, host).preference).toBeUndefined();
    }
  });

  it("stops waiting in plain language when the host never answers", () => {
    const modelPorts = ports();
    const { driver, host, guests } = lockedLobby(modelPorts, "2v2", 3);
    const chooser = must(guests[1], "no second guest");
    const before = owned(modelPorts, chooser, "guest_2").join(",");
    const hostIndex = driver.peers.indexOf(host);
    if (hostIndex >= 0) {
      driver.peers.splice(hostIndex, 1);
    }

    driver.send(chooser, { kind: "pair", slot: "away_3" });
    expect(must(view(modelPorts, chooser).preference, "no pending preference").status).toBe(
      "pending",
    );
    driver.tick(PREFERENCE_TIMEOUT_TICKS + 1);

    const givenUp = must(view(modelPorts, chooser).preference, "no preference after timeout");
    expect(givenUp.status).toBe("rejected");
    expect(givenUp.reason).toBe("no_response");
    expect(givenUp.slots.join(",")).toBe("away_1,away_3");
    expect(owned(modelPorts, chooser, "guest_2").join(",")).toBe(before);
    expect(view(modelPorts, chooser).terminal).toBeUndefined();
    expect(view(modelPorts, chooser).phase).toBe("assigned");
  });

  // Not driven through `Driver` at all: a pure completeness check that
  // `lobby_model.PREFERENCE_TEXT` has a non-empty string for every
  // status/reason the closed vocabulary can produce, and nothing extra.
  // `SessionPreferenceStatus`/`SessionPreferenceRejection` are that
  // vocabulary, already declared in `lobby_model.ts` and sourced from the
  // real protocol's own vocabulary (Rust-owned, no bridge -- see this
  // file's header).
  it("has plain language for every outcome a request can end on", () => {
    const statuses: readonly SessionPreferenceStatus[] = ["granted", "unchanged", "rejected"];
    const reasons: readonly SessionPreferenceRejection[] = [
      "already_taken",
      "wrong_team",
      "invalid_slot",
      "detached",
      "not_seated",
      "superseded",
      "after_freeze",
      "no_response",
      "reseated",
    ];
    const reachable: Record<string, boolean> = { pending: true };
    for (const status of statuses) {
      if (status !== "rejected") {
        reachable[status] = true;
      }
    }
    for (const reason of reasons) {
      reachable[reason] = true;
    }
    for (const key of Object.keys(reachable)) {
      const text = PREFERENCE_TEXT[key];
      expect(typeof text === "string" && text.length > 0).toBe(true);
    }
    for (const key of Object.keys(PREFERENCE_TEXT)) {
      expect(reachable[key]).toBe(true);
    }
  });
});

// ---------------------------------------------------------------------------
// "lobby build skew"
// ---------------------------------------------------------------------------

describe("lobby build skew", () => {
  function buildRow(modelPorts: LobbyModelPorts, peer: TestPeer): string {
    for (const row of view(modelPorts, peer).identity) {
      if (row.label === "BUILD") {
        return row.value;
      }
    }
    throw new Error("the lobby stopped showing a build identity");
  }

  it("ends a same-version, different-vocabulary session at the manifest check", () => {
    const modelPorts = ports();
    const driver = new Driver(modelPorts);
    const host = driver.add("host", "host", foreignManifest);
    driver.send(host, { kind: "mode", mode: "1v1" });
    const guest = driver.add("guest", "guest_1", fakeManifest);
    expect(buildRow(modelPorts, host)).not.toBe(buildRow(modelPorts, guest));
    driver.connect(host, guest);
    driver.send(host, { kind: "lock" });
    driver.pump(6);

    const state = must(guest.model.coordinator, "guest has no coordinator");
    expect(state.phase).toBe("terminal");
    const terminal = must(state.terminal, "guest has no terminal");
    expect(terminal.reason).toBe("build_mismatch");
    expect(terminal.detail).toBe("local identity differs at manifest.build_id");
    expect(view(modelPorts, guest).terminal_text).toBe(
      "The peers are running different builds. Install the same build on both.",
    );

    // `assignment_id` is internal bookkeeping this fake keeps but
    // `CoordinatorState`'s public type does not expose; `assignments`
    // tracks it 1:1 and is the field the interface actually declares.
    expect(state.assignments).toBeUndefined();
    expect(guest.freeze).toBeUndefined();

    driver.pump(8);
    const hostState = must(host.model.coordinator, "host has no coordinator");
    expect(hostState.terminal).toBeUndefined();
    expect(hostState.peers.length).toBe(1);
    expect(host.freeze).toBeUndefined();

    const departure = must(hostState.departure, "the host was told nothing");
    expect(departure.reason).toBe("build_mismatch");
    expect(departure.peer_id).toBe("guest_1");
    expect(departure.code).toBe("protocol_error");
    expect(view(modelPorts, host).departure_text).toBe(
      "A guest was dropped: it disagreed about this session's identity, and it " +
        "declared a different build. Install the same build on both to rule that out.",
    );
    expect(view(modelPorts, host).terminal_text).toBeUndefined();

    let announced = 0;
    for (const wire of host.sent) {
      const decoded = JSON.parse(wire) as {
        readonly kind: string;
        readonly code?: string;
        readonly target_peer_id?: string;
      };
      if (decoded.kind === "disconnect") {
        announced += 1;
        expect(decoded.code).toBe("protocol_error");
        expect(decoded.target_peer_id).toBe("guest_1");
      }
      expect(wire.includes("build_mismatch")).toBe(false);
    }
    expect(announced).toBe(1);
  });

  it("says only that a guest went when the two builds agree", () => {
    const modelPorts = ports();
    const driver = new Driver(modelPorts);
    const host = driver.add("host", "host", fakeManifest);
    driver.send(host, { kind: "mode", mode: "1v1" });
    const guest = driver.add("guest", "guest_1", fakeManifest);
    expect(buildRow(modelPorts, host)).toBe(buildRow(modelPorts, guest));
    driver.connect(host, guest);
    driver.send(host, { kind: "lock" });
    driver.pump(6);
    expect(view(modelPorts, host).departure).toBeUndefined();

    driver.send(guest, { kind: "leave" });
    driver.pump(6);
    const hostState = must(host.model.coordinator, "host has no coordinator");
    const departure = must(hostState.departure, "the host lost a guest and said nothing");
    expect(departure.reason).toBe("guest_left");
    expect(view(modelPorts, host).departure_text).toBe("A guest left the lobby.");
    expect(hostState.terminal).toBeUndefined();
  });

  // Not driven through `Driver`: a pure completeness check that
  // `lobby_model.DEPARTURE_TEXT`/`TERMINAL_TEXT` have a sentence for every
  // coordinator disconnect code and every departure reason. The real
  // coordinator's `DISCONNECT_REASONS` table is Rust-owned with no
  // TypeScript counterpart (see this file's `DISCONNECT_REASONS` constant
  // above, which mirrors it directly for this one purpose).
  it("has host-side language for every reason a drop can carry", () => {
    for (const [reason, text] of Object.entries(DEPARTURE_TEXT)) {
      expect(typeof text === "string" && text.length > 0).toBe(true);
      expect(TERMINAL_TEXT[reason as CoordinatorTerminalReason]).not.toBeUndefined();
    }
    for (const reason of Object.values(DISCONNECT_REASONS)) {
      expect(DEPARTURE_TEXT[reason]).not.toBeUndefined();
    }
    expect(DEPARTURE_TEXT.build_mismatch).not.toBeUndefined();
  });

  it("leaves peers on the same vocabulary exactly as compatible as before", () => {
    const modelPorts = ports();
    const driver = new Driver(modelPorts);
    const host = driver.add("host", "host", fakeManifest);
    driver.send(host, { kind: "mode", mode: "1v1" });
    const guest = driver.add("guest", "guest_1", fakeManifest);
    expect(buildRow(modelPorts, host)).toBe(buildRow(modelPorts, guest));
    driver.connect(host, guest);
    driver.send(host, { kind: "lock" });
    driver.pump(6);

    expect(view(modelPorts, guest).phase).toBe("assigned");
    driver.send(host, { kind: "ready", ready: true });
    driver.send(guest, { kind: "ready", ready: true });
    driver.pump(4);
    driver.send(host, { kind: "start" });
    driver.pump(2);
    driver.tick(COUNTDOWN_TICKS + 4);

    const freeze = must(host.freeze, "the host never reached the start boundary") as TestFreeze;
    const guestFreeze = must(
      guest.freeze,
      "the guest never reached the start boundary",
    ) as TestFreeze;
    expect(guestFreeze.manifest_id).toBe(freeze.manifest_id);
    expect(must(host.model.coordinator, "no host coordinator").terminal).toBeUndefined();
    expect(must(guest.model.coordinator, "no guest coordinator").terminal).toBeUndefined();
    expect(view(modelPorts, host).departure).toBeUndefined();
    expect(view(modelPorts, host).departure_text).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// "online lobby screen shell"
// ---------------------------------------------------------------------------

describe("online lobby screen shell", () => {
  it("carries a full 1v1 session through two mounted screens", async () => {
    const { OnlineLobby } = await import("./online_lobby.ts");
    const network = new FakeTransport();
    const clipboards: Record<string, string | undefined> = {};

    interface FakeStar {
      readonly peerId: string;
    }

    function newFakeLink(star: FakeStar) {
      return {
        star,
        send(linkId: string, wire: string): void {
          network.applyEffect(star.peerId, { kind: "send", link_id: linkId, wire });
        },
        apply(effect: LobbyEffect): readonly [boolean, string | undefined] {
          network.applyEffect(star.peerId, effect);
          return [true, undefined] as const;
        },
        poll(): readonly LobbyCommand[] {
          return network.drain(star.peerId);
        },
      };
    }

    function mount(name: string) {
      return new OnlineLobby({ w: 960, h: 540 }, undefined, {
        starFactory: (_role, peerId) => ({ peerId }),
        newLink: newFakeLink,
        clipboard: {
          read: () => clipboards["shared"],
          write: (text: string) => {
            clipboards["shared"] = text;
          },
        },
        modelPorts: ports(),
        modelOptions: { peer_id: name },
      });
    }

    const host = mount("host");
    const guest = mount("guest_1");
    function pump(rounds = 4): void {
      for (let i = 0; i < rounds; i += 1) {
        network.pumpOnce();
        host.update(0);
        guest.update(0);
      }
    }

    host.dispatch({ kind: "role", role: "host" });
    host.dispatch({ kind: "mode", mode: "1v1" });
    guest.dispatch({ kind: "role", role: "guest" });
    host.dispatch({ kind: "invite" });
    pump(2);
    host.dispatch({ kind: "copy" });
    guest.dispatch({ kind: "paste_request" });
    pump(2);
    guest.dispatch({ kind: "copy" });
    host.dispatch({ kind: "paste_request" });
    pump(4);

    expect(lobbyModelView(host.state.ports, host.state.model).connected).toBe(2);
    host.dispatch({ kind: "lock" });
    pump(4);
    host.dispatch({ kind: "ready", ready: true });
    guest.dispatch({ kind: "ready", ready: true });
    pump(2);
    host.dispatch({ kind: "start" });
    for (let i = 0; i < COUNTDOWN_TICKS + 10; i += 1) {
      network.pumpOnce();
      host.update(1 / 60);
      guest.update(1 / 60);
    }
    expect(lobbyModelView(host.state.ports, host.state.model).started).toBe(true);
    expect(lobbyModelView(guest.state.ports, guest.state.model).started).toBe(true);
    host.teardown();
    guest.teardown();
  });

  // "is reachable from the title and returns to it" moved to
  // `packages/app/src/lobby_flow.spec.ts`: the case is about `game.app`'s
  // routing, and `@gc/screens` cannot depend on `@gc/app` (the dependency
  // runs the other way).
});

// ---------------------------------------------------------------------------
// "lobby failure paths"
// ---------------------------------------------------------------------------

describe("lobby failure paths", () => {
  it("ends a guest session with a stable reason when the host aborts", () => {
    const modelPorts = ports();
    const { driver, host, guests } = seatedLobby(modelPorts, "1v1", 1);
    driver.send(host, { kind: "leave" });
    driver.pump(4);
    expect(host.left).toBe(true);
    const guestView = view(modelPorts, must(guests[0], "no guest"));
    expect(guestView.phase).toBe("terminal");
    expect(guestView.terminal_text).not.toBeUndefined();
  });

  it("drops a departed guest and voids the ownership that named it", () => {
    const modelPorts = ports();
    const { driver, host, guests } = seatedLobby(modelPorts, "2v2", 3);
    driver.send(host, { kind: "lock" });
    driver.pump(8);
    expect(view(modelPorts, host).connected).toBe(4);

    driver.send(must(guests[2], "no third guest"), { kind: "leave" });
    driver.pump(4);
    expect(view(modelPorts, host).connected).toBe(3);
    expect(view(modelPorts, host).phase).toBe("assigned");
  });

  it("terminates a guest whose local identity differs from the manifest", () => {
    const modelPorts = ports();
    const driver = new Driver(modelPorts);
    const host = driver.add("host", "host");
    driver.send(host, { kind: "mode", mode: "1v1" });
    const guest: TestPeer = {
      id: "guest_1",
      model: newLobbyModel(modelPorts, {
        peer_id: "guest_1",
        template: (mode) => manifestWithContentId(mode, "content.other.v1"),
      }),
      left: false,
      sent: [],
    };
    driver.peers.push(guest);
    driver.send(guest, { kind: "role", role: "guest" });
    driver.connect(host, guest);
    driver.send(host, { kind: "lock" });
    driver.pump(6);

    const guestView = view(modelPorts, guest);
    expect(guestView.phase).toBe("terminal");
    expect(must(guestView.terminal, "no terminal").reason).toBe("manifest_mismatch");
    expect(must(guestView.terminal, "no terminal").detail?.includes("content_id")).toBe(true);
  });
});
