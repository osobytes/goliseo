# OMP-3 manual-connect lobby

The lobby is the developer-facing route that turns the
[session protocol](session_protocol.md), the
[coordinator](session_coordinator.md), and the
[star transport](transport_bridge.md) into something a person can operate: pick
a role, exchange an offer and an answer by hand, choose a match mode, see who
owns which of the eight canonical outfield slots, ready up, and reach the
synchronized start boundary — with no browser console.

It presents coordinator truth. It owns no admission rule, no manifest, no
ownership invariant, no readiness barrier, and no countdown.

## Modules

| Module | Purity | Responsibility |
| --- | --- | --- |
| `game.screens.lobby_model` | pure | Session state, commands, effects, and the derived view. |
| `game.screens.lobby` | pure | `layout` / `update` over the model, per AGENTS.md §9. |
| `game.screens.online_lobby` | impure | Owns the star, the clipboard, the tick clock, and drawing. |
| `game.online.lobby_link` | impure | Control-wire framing and the manual signaling handshake. |

`lobby_model.command(model, command)` returns a fresh model and an ordered list
of effects. Effects are the only way the lobby touches the world: `open_star`,
`open_peer`, `request_offer`, `accept_offer`, `accept_answer`, `send`, `close`,
`clipboard`, `paste_request`, `start_match`, `shutdown`, and `leave`. Because
the transition is pure, the entire lobby — including the complete host/guest
handshake against `fake_star` — runs headlessly.

`lobby.update` puts the effects it produced on `state.effects`; the owning
screen drains them after each update. That is the one deliberate deviation from
the simplest screens, and it exists so the transition itself stays pure.

## Match modes

The host picks `1v1`, `2v2`, or `4v4` before the manifest is proposed and the
required human count follows from the mode: 2, 4, or 8. `3v3` does not exist —
the closed table in `protocol.MATCH_MODES` is the only source, so the lobby
cannot offer a mode the protocol would refuse.

**The mode locks at manifest proposal, not at countdown.** The mode is a
manifest field, the manifest is immutable after proposal, and admission closes
at the same moment; there is no legal way to change it later without a new
session. Changing it before proposal discards the seating order that depends on
it. Readiness cannot exist before a manifest is accepted, so "readiness clears
on a mode change" holds vacuously by construction rather than by a rule the
lobby has to remember.

## Ownership, seating, and pair selection

The host keeps an ordered seating list of admitted humans and hands it to
`coordinator.plan_assignments`, which seats the Nth human in the Nth contiguous
block of `slots_per_human` slots. Repartitioning is therefore a *permutation of
the seating list*, not a second ownership algorithm: the `SWAP` control on seat
N exchanges seats N and N+1 and republishes. In `2v2` that exchanges the two
pairs on a team (or moves a human across the halfway line); in `1v1` and `4v4`
it does the same thing to whole lines and single slots. Every republication is a
new ownership generation, so the coordinator clears readiness exactly as it does
for any other pre-freeze configuration change.

**Every player chooses the players it controls.** The `TAKE` control beside a
roster slot asks the host for it. The request is that slot plus the slots the
peer already owns, minus the last one it does not open the match on: a
preference refines the pair you already control, which is exactly the rule the
host enforces, and is why a peer keeps its opening live slot across one. The
control is offered only where it could do something, so an owned set with
nothing to trade — a single slot in `4v4`, a whole outfield line in `1v1` —
offers none at all. No mode is named to make that true.

The host stays authoritative: the lobby sends `prefer_pair` and presents the
answer beside the roster as `PAIR <slots> <what happened>`. While the request is
in flight that reads as waiting; afterwards it is the grant, "you already
control that pair", or the plain-language equivalent of the typed reason the
host refused it with. Every one of those strings comes from
`lobby_model.PREFERENCE_TEXT`; the wire vocabulary stays closed and these
strings never cross a link.

The `SWAP` control is unchanged and still belongs to the host, but it now
deliberately outranks guest choices: it reasserts the host's seating order, so
it republishes over every granted pair and the coordinator drops the claims with
it. Everything else leaves granted ownership alone — the lobby only publishes a
seating plan when ownership does not already seat the whole roster, so ordinary
control traffic cannot quietly undo a pair a guest was given.

A roster change is the one case that has to publish a plan over granted pairs,
and it is the one case that does not simply overrule them. The lobby asks the
coordinator to seat the new roster *around* the claims it can still hold, so a
pair the new roster still has room for comes through untouched.

Either way, a player who loses a pair is told rather than reseated in silence:
the status line reads the plain-language `reseated` text, in the same place a
grant and every refusal are read. That text names no cause, because the two
causes are indistinguishable from where the player is standing — a `SWAP` and a
roster change both arrive as ownership that no longer seats the pair they were
given.

The roster shows all eight canonical slots with their producer and driver:

- `LIVE` — the human's opening live slot, taken from
  `coordinator.preview_live`, which is the same rule and the same code the
  freeze records as `CoordinatorFreeze.live`; the lobby does not restate it;
- `AI (OWNED)` — a slot inside a human's owned set that the deterministic AI
  drives, which only exists in `1v1` and `2v2`;
- `AI FILL` — a declared bot producer.

Both keepers are listed separately as protected AI in every mode, and the specs
assert that no keeper player id ever appears on a canonical slot.

Empty seats are only bot-filled when the host explicitly approves it; otherwise
the lobby refuses to propose a manifest until the mode's full human count has
connected.

## Manual signaling and blob hygiene

The host invites a peer, which opens star link `guest_N` and requests an offer.
Blobs move through the clipboard, never through the layout: the model holds the
outgoing blob only until the `COPY SIGNAL` effect hands it over, then drops it,
and an imported blob is passed straight to the transport and never retained at
all. What the screen renders is a record — direction, peer, byte count, and an
eight-character FNV-1a digest. A spec asserts that no widget text ever contains
the blob.

A guest's coordinator identity and its transport link identity are the same
string, because the host's Nth invitation opens link `guest_N` and the star
binds the endpoints by that name. The role screen therefore lets a guest choose
which invitation it is answering before the session starts.

## Control framing

A session control wire is bounded at 8,192 bytes; a transport payload is bounded
at 1,024. A manifest proposal is about 2.5 kB, so it cannot travel as one
message. `lobby_link` adds the bounded framing the transport bridge document
leaves to its caller:

```text
GCLF;1;<index>;<count>;<chunk>
```

with chunks of at most 960 bytes and at most 9 frames, which covers the full
protocol bound. The control channel is reliable and ordered, so reassembly needs
no windows or retransmission: a frame that starts mid-wire, arrives out of
order, disagrees about the count, or would overflow the protocol bound is a
malformed stream, and the buffer is discarded rather than resynchronised.

This framing is a lobby-layer decision. If the transport ever exposes the
control channel with a larger envelope, the framing should move or disappear.

## Closing a link that was just announced on

A coordinator termination emits an `abort` on a link and a `close` for it in the
same action list. An adapter is only required to *queue* the announcement, so
closing immediately can discard it. `lobby_link` therefore defers every close by
one pump. Even when the announcement is still lost — the fake star models a hard
close that drops the remote's buffer too — the peer learns from the transport
state change, and both paths end the session with a stable reason.

## Failure presentation

Every `CoordinatorTerminalReason` has a plain-language equivalent in
`lobby_model.TERMINAL_TEXT`, shown together with the coordinator's own
non-localised detail (for example `local identity differs at
manifest.content_id`). Rejected local commands surface as a one-line reason that
clears on the next command. Wire codes stay inside the protocol's closed
vocabulary; these strings never cross a link.

## What the lobby still does not do

Public room codes, automatic signaling, STUN/TURN UX, matchmaking, accounts,
chat, reconnect, host migration, join-in-progress, and loadout editing are all
out of scope. The lobby emits a `start_match` action carrying the coordinator's
freeze; dispatching that into a running online match belongs to the online match
flow, not here.

The manifest it proposes is the pinned protocol fixture, injected rather than
hard-wired, until a content-derived manifest builder exists.
