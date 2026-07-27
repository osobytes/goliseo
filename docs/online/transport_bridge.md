# Transport bridge

This document is the public Lua contract for issue #5. Issue #4 supplies a
bounded, asynchronous loopback seam; it does not negotiate WebRTC, synchronize
match state, predict input, or implement rollback.

## Lua API

The entry point is `require("game.transport")`:

```lua
local transport = require("game.transport")

local link = transport.fake() -- or transport.browser()
assert(link:initialize())

local input = assert(link:enqueue({
    version = 1,
    type = "input",
    seq = 0,
    tick = 120,
    payload = "opaque input payload",
}))

local received = link:poll() -- nil when the bounded queue is empty
local event = link:poll_event() -- state/error event, or nil
local metrics = link:diagnostics()
assert(link:shutdown())
```

Both adapters expose the same operations:

| Operation | Result | Semantics |
| --- | --- | --- |
| `initialize()` | `true` or `nil, error, code` | Opens the adapter and emits a `connected` state event. |
| `shutdown()` | `true` or `nil, error, code` | Clears queued messages and emits `closed`. |
| `enqueue(message)` / `send(message)` | `true` or `nil, error, code` | Validates and queues one outbound message; it never waits for a peer. |
| `poll()` | `message` or `nil, error, code` | Removes at most one inbound message, preserving insertion order. |
| `poll_event()` | `TransportEvent?` | Removes one connection/error event without throwing. |
| `state()` | `TransportState` | Returns `new`, `connected`, `disconnected`, or `closed`. |
| `diagnostics()` | `TransportDiagnostics` | Returns queue depth, bounded-capacity, counters, and the last error. |

`disconnect(reason?)` is available on both adapters for tests and host-level
disconnect reporting. It emits a `disconnected` state event followed by a
`disconnected` error event. Both adapters discard queued inbound/outbound
messages and increment the corresponding dropped counters, so stale traffic
cannot survive a reconnect. A disconnect does not turn into a successful
reconnect implicitly; call `initialize()` again when the next transport layer
is ready.

## Envelope version 1

The Lua shape is:

```lua
---@class TransportMessage
---@field version integer -- exactly 1
---@field type "input"|"event"|"state"
---@field seq integer -- non-negative transport sequence
---@field tick integer? -- required when type == "input"
---@field payload string -- opaque UTF-8 bytes for the next protocol layer
```

The wire representation is five pipe-separated, percent-escaped fields:

```text
version | type | seq | tick-or-empty | payload
```

Only unreserved URI characters remain unescaped. Consequently, payloads may
contain pipes, newlines, percent signs, and binary-looking UTF-8 without
confusing the delimiter parser. The bridge treats payload contents as opaque;
issue #5 owns the input payload schema and any later binary encoding.

The maximum payload is 1,024 bytes. A message must have a non-negative integer
`seq`; input messages must also have a non-negative integer `tick`. Malformed
messages, unsupported versions, and oversized payloads are rejected before
queue insertion and increment diagnostics counters.

## Queueing and backpressure

The default queue limit is 64 messages per direction. The state/error event
queue uses the same bound with a minimum capacity of two. That minimum makes a
disconnect's state/error pair atomic from the observer's perspective even when
`queue_limit` is one. Adapters accept a `queue_limit` from 1 through 256 for
deterministic tests and can report the effective message limit through
`diagnostics()`.

The fake adapter loops outbound messages into its inbound queue immediately,
while preserving order. If its inbound queue is full, accepted outbound
messages remain in a bounded outbound queue until `poll()` makes room. The
browser host uses the same two queues but schedules one loopback delivery with
`queueMicrotask` (or `setTimeout` as a fallback), so browser delivery is
asynchronous and never waits on a network operation in the LÖVE update call.

When the outbound queue is full, `enqueue` returns code `overflow`; the message
is dropped and `dropped_outbound`/`overflow` are incremented. Inbound injection
in the fake adapter reports the equivalent `dropped_inbound` case. There is no
unbounded buffering and no retry loop inside the adapter. If the event queue is
full, its oldest event is dropped and the newest state/error event is retained.
Because event capacity is never below two, the `disconnected` state event and
the following `disconnected` error event are retained together. The `overflow`
counter and `last_error` make any older-event loss observable. Issue #5 must
decide whether to drop, coalesce, or stop sampling input after observing
backpressure.

`diagnostics()` includes `outbound_depth`, `inbound_depth`, `event_depth`,
`queue_limit`, `sent`, `received`, `dropped_outbound`, `dropped_inbound`,
`malformed`, `unsupported_version`, `overflow`, and `last_error`.

## Generated browser host

`scripts/web_build.py` emits `player.js` with the maintained host object
`window.GalacticCupTransportBridge`. The Lua browser adapter calls its small
method surface through the pinned runtime's existing `love.js.eval` hook:

```text
initialize(queue_limit) -> "state|connected"
shutdown()              -> "state|closed"
enqueue(wire)           -> "ok" or "error|code|detail"
poll()                  -> wire or ""
poll_event()            -> "state|...", "error|...", or ""
disconnect(reason?)     -> "state|disconnected"
diagnostics()           -> pipe-separated diagnostic fields
```

The `GalacticCupTransportBridge` host remains a bounded loopback seam. Issue #5
adds a separate `GalacticCupWebRTCProof` host for manual peer connections while
reusing this envelope shape; it does not turn the loopback adapter into a
production network client. No JavaScript module is imported by `core/`, `data/`,
or `sim/`, and no generated artifact is checked in. The browser build smoke
check verifies both hosts are present in generated `player.js`.

## Host-star transport (OMP-3)

The two-peer loopback above stays as-is. The game-facing online path uses the
**host-star** adapters instead: one host endpoint with up to seven
independently addressed guest links. Entry points are
`transport.fake_star(options)` and `transport.browser_star(options)`.

```lua
local transport = require("game.transport")

local host = transport.fake_star() -- or transport.browser_star()
assert(host:initialize())
assert(host:open_peer("guest_1"))

assert(host:send("guest_1", "control", lifecycle_message))
assert(host:broadcast("input", canonical_batch))

for _, entry in ipairs(host:poll_batch(32)) do
    -- entry.peer_id, entry.channel, entry.arrival_seq, entry.message
end

local event = host:poll_event() -- typed peer/star state or error
local metrics = host:diagnostics() -- star totals plus one record per peer
assert(host:shutdown())
```

### Topology, roles, and permissions

`contract.MAX_GUESTS` is 7 and `contract.HOST_PEER_ID` is `host`. A host
allocates one slot per guest with `open_peer(peer_id)`; the eighth call fails
with `capacity`, a repeated id with `duplicate_peer`, and the reserved `host`
id with `duplicate_peer`. A guest endpoint has exactly one link, created by
`initialize()`.

Permissions are enforced at the link level, which is the only authority a
transport legitimately has:

| Operation | Host | Guest |
| --- | --- | --- |
| `open_peer` | allowed | `role_forbidden` |
| `send(peer_id, ...)` | any open slot | only `host` |
| `broadcast(channel, ...)` | fans out to every connected link | `role_forbidden` |

A guest therefore has no way to address another guest and no way to fan out a
canonical batch. Inbound messages are attributed by the link they arrived on,
never by anything in the payload, so a guest cannot claim another peer's
identity. `TransportPeerMessage.peer_id` is exactly the
`InputPacketArrival.transport_peer_id` the input protocol expects.

Payload-level authority stays where it belongs. The star reuses the #161
session protocol (`game.online.protocol`) and #162 input packets
(`game.online.input_protocol`) verbatim as opaque payload bytes; there is no
second WebRTC message vocabulary in the game path. Deciding whether a given
control payload is legal for a given peer is the session coordinator's job.

### Channels

Every link carries exactly two data channels, and the pairing is enforced
before anything reaches the wire:

| Channel | Delivery | Carries | Rejects |
| --- | --- | --- | --- |
| `control` | reliable, ordered | `event` and `state` envelopes | `input` → `channel_mismatch` |
| `input` | unordered, `maxRetransmits: 0` | `input` envelopes | `event`/`state` → `channel_mismatch` |

`contract.CHANNEL_CONFIG` is the single source of that configuration and is
what the JavaScript bridge passes to `createDataChannel`.

### Validation order

Both adapters and the bridge judge a `send` in the same order, so the reported
code never depends on which one you asked:

1. lifecycle (`not_initialized` / `not_connected` / `closed`);
2. role permission (`role_forbidden`);
3. peer id and channel name shape (`malformed` / `channel_mismatch`);
4. message shape and channel/type pairing (`malformed`,
   `unsupported_version`, `payload_too_large`, `channel_mismatch`);
5. peer resolution (`unknown_peer`);
6. queue capacity (`overflow`).

Message shape is deliberately judged *before* peer resolution. Only the bridge
knows which links actually exist, so a Lua adapter must never answer
`unknown_peer` from its own cached peer table; shape is the one fault every
layer can judge identically. A call that is both badly shaped and badly
addressed therefore reports the shape fault everywhere.

A fault an adapter rejects locally — role misuse, a message the bridge would
never see — is recorded exactly like one the bridge reports: it sets
`last_error` and queues a typed event. The two adapters do not disagree about
whether something happened.

### Deterministic poll batching

`poll_batch(limit)` drains through a persistent cursor that walks
`(slot, channel-rank)` pairs — control before input, slot 1 before slot 7 —
taking at most one message per pair before moving on. Both adapters advance
the same cursor, so no peer can starve another and a given queue state always
produces the same batch.

This is an ordering rule at the Lua boundary and nothing more. Browser
callback arrival order is **not** simulation order: `arrival_seq` is a
per-peer ordinal stamped at poll time — the position of that message among
what the caller has drained for that peer — not a tick. It is stamped at poll
time on both adapters precisely so they agree; the browser adapter cannot
observe receive order at all. The session and rollback layers still order work
by the tick inside the payload.

### Bounds, backpressure, and teardown

Each peer/channel keeps its own bounded outbound and inbound queue at
`queue_limit` (default 64, max 256). A full queue is the only condition that
refuses a send: it returns `overflow`, drops the message, and increments
`dropped_outbound`/`dropped_inbound`.

A saturated send buffer is *backpressure*, not a rejection. When a channel's
`bufferedAmount` would exceed `buffered_amount_limit` (default 65,536 bytes,
max 1,048,576), the remaining messages stay queued and the peer reports one
latched `backpressure` event until the channel moves again. Nothing blocks,
spins, or waits on the network inside a LÖVE update.

`close_peer(peer_id, reason?)` closes exactly one link: it drops that peer's
queues, closes its channels and peer connection, emits `peer_state` and a
`disconnected` `peer_error`, and leaves every other slot untouched. It is
idempotent once the peer reports `closed`. `shutdown()` closes every link,
releases every slot so no orphan peer connection survives, and is safe to call
repeatedly.

### Peer-scoped diagnostics

`diagnostics()` returns star totals (`role`, `state`, `capacity`,
`peer_count`, `queue_limit`, `buffered_amount_limit`, `event_depth`, `sent`,
`received`, `dropped_outbound`, `dropped_inbound`, `malformed`,
`unsupported_version`, `overflow`, `backpressure`, `last_error`) plus one
`TransportPeerDiagnostics` per slot. Each peer record carries its `slot`,
`state`, `ice_state`, `sequence_gaps`, `backpressure`, `malformed`,
`last_error`, and a per-channel record with `state`, `outbound_depth`,
`inbound_depth`, `buffered_amount`, `sent`, `received`, `dropped_outbound`,
and `dropped_inbound`.

`poll_event()` returns typed events rather than strings:
`{ kind = "star_state"|"peer_state"|"star_error"|"peer_error", peer_id?,
channel?, state?, code?, message? }`.

### Generated browser bridge

`scripts/webrtc_star_host.js` is embedded into `player.js` as
`window.GalacticCupStarTransport`. It owns one `RTCPeerConnection` per guest
and the two data channels per connection; no JavaScript object crosses into
Lua. The Lua adapter exchanges bounded ASCII through `love.js.eval`:

```text
initialize(role, queue_limit, max_guests, buffered_limit) -> "star|connected"
shutdown()                    -> "star|closed"
open_peer(peer_id)            -> "slot|N" or "error|code|detail"
close_peer(peer_id, reason)   -> "ok" or "error|code|detail"
request_offer(peer_id)        -> "ok"   (host, asynchronous)
accept_offer(signal)          -> "ok"   (guest, asynchronous)
accept_answer(peer_id, signal)-> "ok"   (host)
take_signal(peer_id)          -> "signal|<escaped SDP JSON>" or ""
send("peer|channel|wire")     -> "ok" or "error|code|detail"
broadcast("channel|wire")     -> "delivered|N" or "error|code|detail"
poll()                        -> "peer|channel|wire" or ""
poll_event()                  -> typed event line or ""
diagnostics()                 -> one `star` record plus one `peer` record per slot
```

### Manual signaling (still OMP-3)

Offer and answer creation are asynchronous, so signaling never blocks a frame.
The host calls `request_offer(peer_id)` and then polls `take_signal(peer_id)`
on later frames until the SDP blob appears; the guest calls
`accept_offer(signal)` and polls `take_signal("host")` for its answer; the
host finishes with `accept_answer(peer_id, signal)`. Blobs are exchanged by
hand between two browser contexts. Automatic signaling, room codes, and
production STUN/TURN credentials remain OMP-4 work; the bridge configures no
ICE servers.

`transport.fake_star()` implements the same four calls in process, so a lobby
or a test can drive the whole handshake without a browser. The fake's blob is
an opaque rendezvous token rather than SDP.

Signaling state is scoped to one logical star, not to the process. Endpoints
see each other's tokens only when they were constructed with the same
`transport.fake_star_rendezvous()`:

```lua
local rendezvous = transport.fake_star_rendezvous()
local host = transport.fake_star({ rendezvous = rendezvous })
local guest = transport.fake_star({
    role = "guest",
    peer_id = "guest_1",
    rendezvous = rendezvous,
})
```

An endpoint built without one gets a private rendezvous and cannot complete a
handshake with anything, which makes sharing an explicit decision. That
matters for a harness running one host and seven guests in a single process:
its whole point is proving those clients converge *without* shared mutable
state, so cross-star signaling would let it pass for the wrong reason.

> **Operator note.** A real offer or answer blob contains ICE candidates,
> which include local and public IP addresses of the machine that produced it.
> Treat a pasted SDP blob as personal network data: keep it out of shared
> logs, issue trackers, and support tickets, and redact the candidate lines
> before attaching one to a bug report.

### Data channel invariants

Each link carries exactly two data channels for its whole lifetime. Either
side of an established `RTCPeerConnection` may call `createDataChannel` at any
time, so the bridge refuses a second channel arriving on an occupied
`control`/`input` label: it closes the newcomer and reports `channel_mismatch`
against that peer. Without that rule a connected peer could open channels
without bound and take over the reference the bridge sends through.

Teardown detaches every listener before closing a channel or a peer
connection, and the state-change handlers close over their own connection
rather than the peer record. `RTCPeerConnection.close()` fires
`connectionstatechange` and `iceconnectionstatechange` on a *later* task, by
which point the peer record has already been released.

The shipped bundle exposes no channel-injection seam. `scripts/web_smoke.sh`
fails the build if one appears in `player.js`, because such a seam would let
any co-resident script attach a handle to an open peer and forge traffic
attributed to that peer id, bypassing the handshake that link attribution
rests on.

## Observability contract

Expected failures are returned as `nil, message, code` and are also visible via
`poll_event()` and diagnostics. The important codes are:

- `malformed` — invalid fields or payload shape;
- `unsupported_version` — an envelope version other than 1;
- `payload_too_large` — payload over 1,024 bytes;
- `overflow` — bounded queue capacity was reached;
- `disconnected` — the host reported a peer/connection loss;
- `not_initialized`, `not_connected`, and `closed` — lifecycle misuse.

The host-star adapters add:

- `capacity` — an eighth guest was requested;
- `duplicate_peer` — the peer id is already open or is the reserved host id;
- `unknown_peer` — no link exists with that id;
- `role_forbidden` — a guest tried to fan out, open a link, or address another
  guest;
- `channel_mismatch` — the message type does not belong on that channel;
- `backpressure` — the channel's send buffer is saturated and the message
  stayed queued;
- `signal_error` — a manual offer/answer blob was malformed or rejected.

The adapter does not throw for those expected transport failures. Programmer
configuration errors such as an invalid queue limit remain assertions.
