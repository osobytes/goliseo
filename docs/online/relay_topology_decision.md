# Online topology decision: dedicated relay

Status: **proposed** on 2026-07-27. Supersedes the inherited host-star for OMP-4 onward.
OMP-3 ships as built.

## Decision

Online matches will route input through a **dedicated relay server** rather than through a
player-host, starting with OMP-4.

- The relay **forwards opaque payloads and never parses a game packet**.
- Transport stays **WebRTC data channels**, terminated server-side in ICE-lite mode.
- The relay runs on a **small VM with a public IP**, not a managed edge platform.
- The **session coordinator stays client-side**. Only input distribution moves.
- OMP-3's manual-connect host-star is retained as a LAN and no-infrastructure path.

## Why this is being decided at all

There is no prior topology decision to supersede. `session_protocol.md` records that each link
"keeps the OMP-0 topology", and OMP-0's topology was a **two-peer** WebRTC proof. The star was
generalised from 1+1 to 1+7 in #164 without the shape ever being weighed against alternatives.

That is acceptable as history and a poor foundation for OMP-4, which introduces server
infrastructure anyway. This document exists so the next shape is chosen rather than inherited.

## What the host-star actually costs

Measured from the codebase, not estimated. `advance` runs once per fixed tick, so packets go at
60 Hz; a match is 120 seconds, or 7,200 ticks. `input_protocol_conformance.lua` pins a maximal host
batch at 755 bytes.

| | host-star | mesh | relay |
| --- | ---: | ---: | ---: |
| worst-node upload per tick | **5,285 B** (host) | 1,190 B | 1,190 B (client) |
| total traffic per tick | ~6,475 B | ~9,520 B | ~6,475 B |
| connections | 7 | **28** | 8 |
| input path | 2 legs | **1 leg** | 2 legs |
| NAT failure surface | 7 pairs | 28 pairs | **no traversal** |

The star concentrates the whole match on one player's uplink: roughly **2.5 Mbps sustained
upload**, on residential broadband, from someone who is also playing. #243 is the first symptom of
that concentration rather than an isolated bug.

Mesh removes the concentration and halves the input path, which directly shrinks rollback depth.
It was rejected: 28 connections behind residential NAT means **any single failed pair breaks the
match**, and under OMP-3's manual signaling it is not even expressible.

## Why the relay must not understand the game

The obvious design has the relay do what the host does today — collect bundles, canonicalise,
fan out. That is rejected.

`input_protocol.canonical_host_batch` is determinism-critical logic with pinned conformance
goldens. Reimplementing it server-side means **two implementations of a desync-critical function in
two languages**, kept in sync by hand. This milestone has already spent four review rounds on
determinism bugs that were subtler than that.

Instead the relay concatenates whatever opaque payloads it received this tick and frames them to
the other clients. Canonicalisation stays in the Lua that is already written, already tested, and
already pinned.

This is also *cheaper*. The canonical batch carries all eight slots including the recipient's own
rows; a forwarding relay never echoes your own rows back to you.

| relay style | server to client per tick | server-side game logic |
| --- | ---: | --- |
| canonicalising | 755 B | full protocol, ported |
| **framing (opaque)** | **~650 B** | none |

And **#243 disappears structurally**: there is no eight-slot aggregation into one bounded packet,
so there is nothing to saturate.

## Why WebRTC rather than WebTransport or WebSocket

**WebSocket is disqualified**, and for a reason specific to this design rather than a general
preference. TCP does not merely add latency to the seven-tick redundancy window — it renders it
inert. The kernel will not deliver tick N+1 until tick N is retransmitted, so the redundant copies
sit in the receive buffer unreadable. The bytes are paid and nothing is bought.

Two corrections worth recording, because the popular sources are wrong in both directions: the
often-quoted multi-second TCP stall figures predate Linux RACK-TLP defaults and do not apply to a
continuous 60 Hz stream (realistic isolated-loss stall is 1 to 1.5 RTT). And no shipped browser
rollback game on WebSocket has ever been documented — the browser-rollback projects that exist
chose WebTransport or WebRTC.

**WebTransport was seriously considered and rejected on fit, not maturity.** It is supported on
both target browsers (Chrome 97+, Firefox 114+; this project tests Chrome 150/151 and Firefox 152)
and reached Baseline in March 2026. The problems are local to this codebase:

- Datagrams cap near 1200 bytes with **no fragmentation**. The 755-byte payload fits, but the
  declared `MAX_PAYLOAD_BYTES = 1024` ceiling lands near 1105 bytes after percent-escaping, and
  below 1200 on reduced-MTU paths. Today's traffic fits; the declared headroom does not.
- There is **no `bufferedAmount` equivalent**. The per-channel backpressure model built and
  reviewed in #164 would have to be replaced with writer-ready pacing.
- Firefox datagrams **do not take priority over stream data**
  ([bug 1900875](https://bugzilla.mozilla.org/show_bug.cgi?id=1900875), open since 2024),
  reintroducing the cross-channel interference the two-channel split exists to avoid.
- **No Lua or LuaJIT WebTransport client exists**, which forecloses the native client in #31.

**WebRTC keeps what is already built.** The browser bridge, the two-channel model,
`maxRetransmits: 0`, the `bufferedAmount` machinery, and the `StarTransportAdapter` shape all
survive; only the endpoint changes from a player to a server. `request_offer`, `accept_offer`,
`accept_answer`, `take_signal` and `ice_state` disappear, which is a net simplification.

Server-side termination is less burdensome than its reputation. With a public IP the relay runs
**ICE-lite** — one host candidate, browsers connect outbound, everything muxed onto a single UDP
port. Mature implementations exist in Go (pion), Rust (str0m) and C++ (libdatachannel).

**TURN stops being a prerequisite.** `platform_decision.md` currently lists production STUN/TURN as
unproven; a public-IP relay deletes that item for players on home connections rather than deferring
it again.

### Known gotcha to design around

Chrome loses roughly **5% of small unreliable packets in the first moments after connect**, because
small packets do not advance PMTU discovery in dcSCTP. Small packets at high rate from tick zero is
exactly this profile. The documented mitigation is to send approximately 1200-byte padding packets
immediately after the peer connection is created, before the match starts.

## Why a VM rather than a managed platform

Cloudflare's pricing is extraordinary for this shape — no egress charge, no charge for outbound
WebSocket messages, and a 20:1 inbound ratio, so the entire fan-out amplification is free. That is
roughly $1,348/month at 100 sustained matches against $257,295 for AWS API Gateway WebSockets.

It is nonetheless unavailable here. That pricing applies to **WebSocket** traffic, which is
disqualified above, and Cloudflare cannot terminate the alternative: Realtime DataChannels are
**one-way only** with unreliability and message size undocumented, and `workerd` has no QUIC or
HTTP/3
([workerd#6451](https://github.com/cloudflare/workerd/issues/6451) is unanswered). The best-priced
platform is not available for the only viable transport.

At the scale this project actually operates, cost is not a decision input:

| scenario | egress per month | flat-rate VM |
| --- | ---: | ---: |
| 10 matches, a few hours a day | 1.17 TB | **$5 to $7** |
| 100 sustained | 93.9 TB | $8.50 to $30 |

So the choice is made on latency and operational burden, and a self-placed VM wins both: the region
is chosen deliberately, and a forwarding relay needs no capacity planning.

Note that a managed edge platform would not have bought edge placement anyway. Cloudflare Durable
Objects are placed by **11 coarse region hints**, the hints are best-effort, and **an object cannot
move after creation** — so the relay hop would be regional, not edge, exactly as a VM is.

### Traps recorded so they are not rediscovered

- **Never place the relay behind a NAT Gateway.** All three hyperscalers charge $0.045/GB in *both*
  directions, which makes free ingress billable and roughly doubles the bill. Put a public IP
  directly on the instance.
- **Anything billing per message is disqualified by arithmetic.** At 60 Hz across eight clients,
  each 170-byte packet bills as a full message against a 32 KB unit.
- **Prefer providers that bill overage over providers that throttle.** netcup drops to 200-300
  Mbit/s and OVH APAC to 10 Mbps. Injecting queueing delay is the precise failure mode rollback
  exists to avoid.
- **Per-instance packet-rate limits are undocumented industry-wide.** At 60 Hz small packets, pps
  is likelier to bind than bandwidth. Benchmark before scaling.

## What this does not solve

**Host departure still ends the session.** The relay survives any client leaving, but the session
coordinator — admission, manifest, readiness, countdown, result acknowledgement — remains
client-side on the room creator. Moving it server-side would reintroduce exactly the server-side
game logic rejected above. Host migration remains OMP-5 scope, and this decision does not change
that.

**The input path is still two legs.** Mesh is better on latency; a well-connected relay is usually
faster than a residential host, but this is a real trade rather than a free win.

**Send rate is untouched and orthogonal.** Packets go every tick at 60 Hz. Many rollback titles
send less often with deeper redundancy per packet, which would scale every number here down
proportionally. Worth evaluating independently of topology.

## Migration

`StarTransportAdapter` is already the seam. A relay adapter becomes a third implementation
alongside `fake_star` and `browser_star`, and the driver above it does not change. The manual
offer/answer path is retained for LAN and no-infrastructure play.

## Alternatives rejected

| Option | Why not |
| --- | --- |
| Keep host-star | Concentrates the match on one residential uplink; #243 is its first symptom, not its last |
| Full mesh | 28 NAT-traversal pairs, any one failure breaks the match; impossible under manual signaling |
| Authoritative server | Discards the OMP-1/OMP-2 determinism investment, this project's most valuable technical asset |
| Canonicalising relay | Two implementations of a desync-critical function in two languages |
| Move the coordinator server-side | Reintroduces server-side game logic to solve a problem host migration solves better |
| WebSocket transport | TCP in-order delivery makes the redundancy window inert |
| WebTransport | Datagram ceiling near the declared payload bound, no backpressure equivalent, Firefox datagram priority gap, no native Lua client |
| Managed edge platform | Cannot terminate WebRTC data channels; would not deliver edge placement regardless |

## Stepping stone available before committing

Force-relay TURN (`iceTransportPolicy: "relay"`) with coturn or a hosted TURN service leaves the
browser code **completely unchanged** and closes the NAT-traversal risk immediately. It does not
provide a dedicated relay — the host still holds authority and carries the fan-out — but it
de-risks traversal and can be evaluated before any server work begins.
