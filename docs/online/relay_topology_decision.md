# Online topology decision: dedicated relay

Status: **accepted in direction, scope open**, by the repository owner on 2026-07-27. Supersedes the
inherited host-star for OMP-4 onward. OMP-3 ships as built.

Three claims in the original draft were subsequently falsified by measurement — see **Measured
corrections** immediately below. The direction survives; how far to take it is deferred until #243
closes independently.

## Decision

Online matches will route input through a **dedicated relay server** rather than through a
player-host, starting with OMP-4.

- The relay **forwards opaque payloads and never parses a game packet**.
- Transport stays **WebRTC data channels**, terminated server-side in ICE-lite mode.
- The relay runs on a **small VM with a public IP**, not a managed edge platform.
- The **session coordinator stays client-side**. Only input distribution moves.
- OMP-3's manual-connect host-star is retained as a LAN and no-infrastructure path.

## Measured corrections — read this before the rest

Everything below was written before the topology was measured. #245 (PR #252) then ran the #169
fault matrix against a relay wire and **falsified three claims in this document**. They are
corrected here rather than quietly edited out, because the reasoning that produced them is worth
seeing alongside what the measurement showed.

**1. "#243 disappears structurally" is wrong.** `4v4.stress` fails **byte-identically** under a
relay — the same eight `confirmation_stalled` statuses, the same final hashes, the same stalled
ticks — even with the host's upload on that row cut from 4,916 to 702 B/tick. #243 is a capacity
limit of the 56-row canonical batch, which belongs to the **sequencer**, not the wire. Moving the
hub does not change what gets packed.

The narrower true statement: the claim holds only of a **sequencer-less** shape, and that shape
cannot currently run (see correction 3), so it is **untested rather than disproven**. Either way
#243 needs its own fix, tracked separately.

**2. The bandwidth arithmetic was wrong in two of three places.**

| | this document predicted | measured |
| --- | ---: | ---: |
| host-star worst node | 5,285 B/tick | **5,319.9** |
| relay client upload | 1,190 | **194.4** |
| relay downlink | ~650 | **1,360.8** |
| canonical batch | 755 B | **760.0** |

The canonical batch row is not a prediction error — it moved when #243 added `confirmed_span` to
every packet. It is listed here because the original sections below still quote **755 B** (at the
`input_protocol_conformance` reference, in the relay-style table, and in the WebTransport datagram
paragraph). Those are left as written, per this document's convention of correcting above rather
than rewriting original prose — but without this row a reader has no signal they are stale.

The 1,190 was the *mesh* figure copied into the relay column — a relay client uploads one copy, not
seven — so the concentration win is roughly **27x**, not 4.4x. Better than claimed.

The downlink figure was **inverted**, and instructively so: a framing relay cannot merge rows
*precisely because it does not parse*, so it forwards seven envelopes and costs **1.79x more**
downstream than the 760-byte canonical batch. Envelope overhead multiplied by fan-out is the price
of staying ignorant of the game. "Cheaper and simpler" was wrong — it is simpler and more
expensive.

**3. "The driver above it does not change" is wrong.** `match_driver.guest_apply_authority` requires
`packet.kind == "host"`, so the first peer bundle terminates the match with `ownership_violation`.
Reaching a sequencer-less relay means **every client canonicalises locally**, which is a real driver
change. Related findings from the probe: `canonical_host_batch` validates ownership against
`arrival.transport_peer_id`, so the relay protocol must **frame per-origin** or ownership degrades
to self-declared `sender_id`; and declared bot fills have no author once no peer is the host.

**One unclaimed cost.** Relay members share a single uplink buffer, so **per-peer backpressure
isolation is something the star has and a relay does not**.

**One unclaimed simplification.** `SETTLE_RELAY_QUIET_STEPS` and the host branch of
`tail_delivered` are host-only and become unnecessary. Smaller than it looked when this was
written: #243 narrowed that wait with confirmation feedback and reduced its ordinary cost to zero,
so what the relay deletes is mostly a fallback path rather than a wait every clean match pays. The
quiet count is not yet strictly a fallback — silence can still override a report (#255).

### What still stands

NAT traversal elimination, host uplink concentration (by more than claimed), removal of the
uncompensated leg asymmetry, and input-relay survival on host departure. The **direction** survives;
the arithmetic and the migration cost did not.

### Open question

Whether to adopt the relay as a **wire replacement keeping the sequencer** (NAT solved, host stays
privileged) or **sequencer-less** (all benefits, real driver work) is **not yet decided**. It is
deferred until #243 is closed independently, so the choice is judged on what the relay actually
delivers rather than on a benefit wrongly attributed to it.

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

These are derived from **concurrency**, not from a match count, so they do not depend on how long a
match runs. That was worth stating when this was written, because match length was then inconsistent
in the codebase: the online manifest defaulted to 3,600 ticks while the simulation and the OMP-1
fixture used 7,200. [#251](https://github.com/osobytes/goliseo/issues/251) settled it at **7,200
everywhere**, which is the value the 120-second figure above already assumed, so nothing here moves.

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

## What this does about the host's advantages

`input_protocol.FAIRNESS_DELAY_TICKS = 3` exists because the host is not merely a player — it is
the **sequencer**. Its own input enters the canonical stream with zero network hops while every
guest's input has to travel, so the host confirms sooner and rolls back less. #241's tail stall is
the clearest evidence: the host reached full time and terminated first precisely *because* as
sequencer it confirms first.

**The compensation only covers half the advantage, and the half it misses is the one that matters.**
An earlier draft of this record described the constant as "a fixed guess at guest-to-host latency".
Reading the gate rather than assuming it (`input_protocol.lua:965-971`, `match_driver.lua:1755-1760`,
`:516-520`) shows something narrower.

The gate rejects a host-local arrival whose `arrival_tick - transport_tick` is under
`FAIRNESS_DELAY_TICKS`, so a host row authored at step T becomes due at step T+3 — exactly the step
that simulates the tick it is stamped for. That stops the sequencer from shortening its **own input
latency**. It is the responsiveness half, and it works.

It does nothing about **leg count**. A guest's row reaches the host in one network hop and reaches
another guest in two, so the same three-tick budget is spent once on the host's inbound path and
twice on a guest's. The host therefore predicts less and confirms sooner, and that half is entirely
uncompensated.

This is not a theoretical gap. It is why the settle phase and `SETTLE_RELAY_QUIET_STEPS` exist at
all: #241's tail stall happened because the host reached full time and terminated first, removing
the star's only relay while guests were still confirming. Had the delay equalised confirmation
depth, #237 would not have been necessary.

So the relay does not merely tidy up an approximate compensation — **it removes the half that was
never compensated.**

**The relay removes the structural advantage entirely.** With a framing relay there is no
sequencer. Every client sends one hop to the relay and receives every other client's input two hops
back. All eight occupy the identical structural position, and fairness stops being something the
protocol patches — it becomes a property of the shape.

The delay constant likely survives, but its justification changes: from "compensate a privileged
player" to "a uniform input-delay knob trading responsiveness against rollback frequency", which is
what rollback games normally tune it as. Its value deserves revisiting once it is no longer doing
double duty.

**Geography replaces structure.** A player 10 ms from the relay still has a genuine advantage over
one at 80 ms. That does not disappear. But it is a better shape of unfairness: structural advantage
is unfixable except by compensation, while geographic advantage is fixable by **placement** — put
the relay near the players' centroid, or let the room choose a region. And it is symmetric in kind,
since everyone has the same relationship to the relay and differs only in distance, rather than one
player having a categorically different relationship to it.

The unfixable asymmetry becomes a tunable one.

## Provisioning and scaling

**Many matches on one VM requires no additional software.** Room scoping is already inherent to the
relay: clients present a room id and the relay forwards only within that room. That *is* the
multi-tenancy. One VM serving many concurrent matches is the default behaviour, not a feature to
build.

**The control plane sits off the data path.** It answers "where do I play?" once at match creation,
returns a relay endpoint plus a room token, and then never sees a game packet. Consequences worth
stating plainly: it can be a small HTTP service, it can be down without killing matches already in
progress, and it scales on a completely different curve from the relay.

This is not a new component. **OMP-4's room-code service is the control plane** — resolving a room
code and selecting a relay are the same request. Later it gains a VM registry and health checks,
which is an internal change to a service already planned.

### The constraint to honour from the first line of code

**Clients must learn their relay endpoint from the room, not from configuration.**

If the endpoint is baked into the client, moving to multiple VMs later requires a client update —
and for a browser game that means every player must reload in lockstep to stay compatible. If the
endpoint arrives with the room assignment, adding VMs is a control-plane change nobody notices.

Two corollaries:

- All clients in one match must receive the **same** endpoint, so room allocation has to be atomic.
- If a relay VM dies mid-match, that match dies. This is no worse than host departure today, and it
  is recoverable later by the same host-migration work already scoped to OMP-5.

### Capacity

At friends-and-family scale — 10 concurrent matches, 80 connections, 29 Mbps — a single small VM is
enormously oversized. Even 100 sustained matches (290 Mbps, roughly 96,000 packets per second) fits
on one or two boxes on bandwidth alone.

The caveat is that **per-instance packet-rate limits are undocumented across every provider
surveyed**, and at 60 Hz with small packets, packets-per-second is likelier to bind than bandwidth.
Benchmark pps rather than trusting a bandwidth figure before scaling.

Expect multi-VM to arrive for **latency** reasons — placing relays near players in different
regions — well before capacity forces it.

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

`StarTransportAdapter` is already the seam, and a relay adapter becomes a third implementation
alongside `fake_star` and `browser_star`. `game/transport/fake_relay.lua` already exists from #245's
probe, so the wire half is done and measured.

**The driver above it does change**, contrary to this document's original claim — see correction 3.
A wire-replacement relay that keeps the sequencer needs little; a sequencer-less relay needs local
canonicalisation on every client, per-origin framing so ownership validation survives, and a new
author for declared bot fills.

The manual offer/answer path is retained for LAN and no-infrastructure play.

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
