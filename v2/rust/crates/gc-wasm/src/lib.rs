//! wasm32-unknown-unknown bindings for the Galactic Cup simulation.
//!
//! This crate is glue, not simulation: it owns zero gameplay math. Its only
//! job is to get bits across the wasm boundary faithfully, in both
//! directions, at the right granularity for each direction's call
//! frequency.
//!
//! ## Two binding strategies, deliberately different
//!
//! - [`session`], [`determinism`] and [`protocol_bridge`] are the CONTROL
//!   SURFACE: session lifecycle, the determinism acceptance check, and lobby
//!   wire helpers. These are called rarely (session setup, once per lobby
//!   message, once at startup) so they use `wasm-bindgen`, trading a little
//!   per-call overhead for ergonomic, type-checked JS bindings generated
//!   automatically.
//!
//! - [`render_export`] is the PER-FRAME HOT PATH: up to eight resimulated
//!   ticks folded into one rendered frame, every rendered frame, for the
//!   lifetime of a match. `wasm-bindgen` glue allocates and marshals on
//!   every call, which is the wrong cost to pay here (`gc-render`'s own
//!   `frame_buffer` module already exists specifically to cross this
//!   boundary ONCE per frame as a flat block, and wasm-bindgen glue would
//!   undo that). So this module exports raw `extern "C"` functions instead:
//!   no `wasm-bindgen` macro, no per-call JS shim. JS reads the result
//!   directly out of linear memory as a `Float64Array` view, using the
//!   pointer and length the export hands back. See that module's doc for
//!   the exact contract.
//!
//! ## Why gc-netcode is a dependency, and what of it is actually bound
//!
//! `gc-netcode`'s deterministic reducer types (`coordinator::CoordinatorState`,
//! `coordinator::Event`, `protocol::Value`) carry no `serde` impls — and
//! this crate does not add any, because `crates/gc-netcode/**` is not a file
//! this task owns (see the worktree's per-wave file-ownership split).
//! Hand-writing a field-by-field `JsValue` bridge for every variant of that
//! reducer is a substantial task in its own right, not a two-line binding,
//! and it is not what this wave's acceptance test needs. So the lobby/
//! coordinator surface bound here ([`protocol_bridge`]) is deliberately
//! narrow: the pieces of `gc_netcode::protocol` that are ALREADY textual
//! (wire encode/decode, the vocabulary digest) and therefore bind cleanly
//! with no new serialization code. Wrapping the full coordinator reducer is
//! flagged as follow-up in this crate's own report, not silently skipped.
//!
//! ## Wave 2: the netcode surface (coordinator, match driver, rollback events)
//!
//! Wave 1 (above) deliberately left `gc_netcode::coordinator`'s reducer
//! unbound: its types carry no `serde` impls, and bridging them was flagged
//! as follow-up. This wave does that follow-up, plus `gc_netcode::match_driver`
//! and `gc_sim::rollback_events`, and it settles the question wave 1 raised:
//! **hand-written, not `serde`.**
//!
//! [`json`] is this wave's whole answer: a small, dependency-free JSON
//! reader/writer, scoped to exactly the shapes these three reducers need.
//! `protocol::Value`'s public constructors (`record`/`array`/`str`/`int`/
//! `bool`/`get`/`set`) and `CoordinatorState`/`Event`/every coordinator
//! struct's already-`pub` fields (README rule: "enum variant fields are
//! always as visible as the enum itself" — nothing needed a visibility
//! change) turned out to be sufficient to build every value this wave binds,
//! entirely from *this* crate, without touching `gc-netcode/src/**` at all.
//! Adding `serde` derives to `CoordinatorState`/`Event` would have meant
//! either deriving on a type deliberately kept `HashMap`-free/wire-shaped
//! for reasons `protocol.rs`'s own doc explains at length, or hand-rolling
//! the same field mapping this crate's `json.rs` already does — with the
//! added risk of a `Deserialize` impl being reachable from *inside*
//! `gc-netcode`, where the determinism-path discipline lives, rather than
//! confined to this crate's glue. See [`json`]'s doc for the full argument.
//!
//! [`net_inbox`] is the queue/drain seam every network-facing module here is
//! built on — read that module's doc first; [`coordinator_bridge`] and
//! [`wasm_transport`] (used by [`match_driver_bridge`]) are both instances
//! of the same discipline, not two independent designs.
//!
//! [`match_driver_bridge`] reuses `gc_netcode::match_driver_fixture::DriverRules`
//! as its [`gc_netcode::match_driver::MatchDriverRules`] implementation
//! rather than writing a new one — that module's own doc says plainly it is
//! "the real `MatchDriverRules` implementation," delegating to
//! `live_slot`/`coordinator` and carrying a genuine port of
//! `input_protocol.canonical_host_batch`, despite living in a file named
//! `_fixture`. Reimplementing it here would be exactly the duplication
//! README rule 5.1 warns against.
//!
//! [`match_driver_bridge`] feeds a match driver's own tick outputs into an
//! internally-owned `gc_sim::rollback_events::RollbackEventTimeline`
//! automatically — see that module's doc for the append/correction split
//! this mirrors from `packages/online/src/match_presentation.ts`'s
//! `consume`.
//!
//! ## Wave W3-G: the granular `RollbackEventsTimeline`/`MatchSnapshot` surface
//!
//! `match_presentation.ts`'s `RollbackEventsPort`/`MatchDriverPort` need
//! `create`/`apply`/`confirm`/`diagnostics`/`snapshot` as separate
//! callables against a caller-owned timeline, not the aggregate `advance`
//! step [`match_driver_bridge`] already had. [`rollback_events_bridge`] now
//! also exports [`rollback_events_bridge::RollbackEventsTimeline`] (those
//! four callables) and [`rollback_events_bridge::WasmMatchSnapshot`] (an
//! opaque `MatchSnapshot` handle — never serialized to JSON, exactly the
//! `TSnapshot` type parameter those TypeScript interfaces declare);
//! [`match_driver_bridge::MatchDriverBridge::snapshot_lookup`] is the
//! matching `MatchDriverPort.snapshot`. See
//! [`rollback_events_bridge`]'s own doc for the full design, including why
//! `apply` needs a step's real events (not a count) and how that JSON is
//! shared with `advance`'s own batch.
//!
//! [`tuning_bridge`] binds `gc_sim::tuning` (the live knob registry) and
//! `gc_data::tuning_presets` (the authored preset list) —
//! `packages/ui/src/tuning_panel.ts`'s own header comment names this
//! specific gap in `SimHost`. See that module's doc.
//!
//! ## Wave W4-E: the last three `NetDiagnosticsFixtureEnv` ports
//!
//! `packages/online/src/net_diagnostics_fixture.ts`'s `NetDiagnosticsFixtureEnv`
//! declares five ports; a previous wave left three unbuilt rather than rush
//! them (`matchDriverFixture`, `inputProtocol`, `inputFrame` — see that
//! file's own header comment). This wave is those three:
//! [`input_frame_bridge`] (`gc_sim::input_frame`'s per-slot sample codec),
//! [`input_protocol_bridge`] (`gc_netcode::input_protocol`'s wire packets,
//! including the forged single-slot bundles
//! `net_diagnostics.spec.ts`'s fault-detection cases need), and
//! [`match_driver_fixture_bridge`] (`gc_netcode::match_driver_fixture`'s
//! session/freeze/manifest/boundary-zero construction, plus a
//! `wasm-bindgen` wrapper over `gc_netcode::fake_star::FakeStarTransport`
//! for the fixture's own in-process star). See each module's doc for its
//! own design notes — in particular [`match_driver_fixture_bridge`]'s for
//! why its `WasmFakeStarTransport` reproduces `StarTransportAdapter`'s
//! *behaviour* without literally implementing that TypeScript interface,
//! and for the `freezeJson`/`manifestJson` pair this wave adds that nothing
//! in `@gc/wasm` could previously produce at all.
//!
//! All three follow [`json`]'s established rule (hand-written JSON, no
//! `serde`) and [`crate::net_inbox`]'s queue/drain discipline wherever they
//! touch anything network-facing ([`match_driver_fixture_bridge`]'s
//! `WasmFakeStarTransport::poll`/`poll_batch`). Wire payloads that can
//! genuinely carry arbitrary bytes (`inputProtocolEncode`/`Decode`,
//! `WasmFakeStarTransport::send`/`broadcast`/`poll*`) cross as
//! `Vec<u8>`/`&[u8]` (`Uint8Array` in JS) exactly the way
//! [`match_driver_bridge`]'s `enqueueInbound` already does — never `String`,
//! which cannot round-trip an arbitrary byte. `packages/wasm/src/binary_string.ts`
//! is the one place `@gc/wasm`'s TypeScript side converts that `Uint8Array`
//! to `@gc/transport`'s established "binary string" `TransportMessage.payload`
//! convention.
//!
//! ## Wave W6-A: defect fixes, plus the combat-phase fixture surface
//!
//! Three defects found driving the real compiled artifact, fixed in place:
//! [`json::Json::obj_omit_null`] (an absent optional field must be an
//! omitted JSON key, not a present `null` — [`match_driver_bridge`]'s
//! `diagnosticsJson`/terminal JSON now use it); [`match_driver_bridge::MatchDriverBridge::new`]'s
//! new `queue_limit` parameter (its wrapped transport's inbound queue was
//! previously fixed at 64 with nothing on the public surface able to raise
//! it); and [`input_protocol_bridge`]'s module doc, which now documents and
//! pins (via a dedicated test) the one-based-wire/zero-based-`SlotId`
//! `slot_index` convention that file's `row_to_json`/`row_from_json` already
//! relied on without saying so.
//!
//! [`online_combat_phases_bridge`] is new: a `#[wasm_bindgen]` surface over
//! the seven pinned combat-phase boundary zeroes
//! `crates/gc-netcode/tests/support/online_combat_phases.rs` already proved
//! out (itself a port of `spec/support/online_combat_phases.lua`) — see that
//! module's own doc for why this is a second copy rather than a shared one.
//! [`session::Session::new`] also gained a `home_formation` parameter (it
//! previously hard-coded `None`, with no way to select a formation at all).
//!
//! ## v2 glue: the playable rollback laboratory, and raw `MatchState`
//!
//! Two gaps the v2 glue milestone traced precisely (no TypeScript-side
//! wiring could close either — see each module's own doc for the full
//! account):
//!
//! [`rollback_playable_lab_bridge`] binds `gc_sim::rollback_playable_lab` —
//! the single-process, deterministic, network-simulated local rollback
//! harness `packages/screens/src/match_rollback_lab.spec.ts`'s
//! `RollbackHostPort` needs a real implementation of. Before this module,
//! nothing in `@gc/wasm` could produce a `"converged"`/`"settling"` status;
//! [`match_driver_bridge`] binds a structurally different thing (an OMP-3
//! TWO-PEER ONLINE driver).
//!
//! [`match_state_bridge`] encodes the RAW `gc_sim::match_snapshot::MatchState`
//! slice `packages/render/src/replay.ts`'s `captureFrame` needs
//! (`outfield_press`/`transition`/per-player timers/`outfield_decision`) —
//! distinct from, and never producible from, [`render_export`]'s
//! presentation-derived `RenderFrame`. [`session::Session::match_state_json`]
//! exposes it for the base (non-rollback) game loop;
//! [`rollback_playable_lab_bridge::RollbackPlayableLab::current_match_state_json`]/
//! `reference_match_state_json`/`match_state_json_at` expose the same thing
//! for a rollback-mode goal replay.
//!
//! ## v2 glue, wave W10-B: the last four skipped-test surfaces
//!
//! Four gaps traced precisely enough that each blocked `it.skip` names the
//! exact wasm entry point missing, rather than a whole feature:
//!
//! [`match_driver_bridge::MatchDriverBridge::render_frame_build`] is the
//! `MatchDriverBridge` half of [`render_export`]'s raw per-frame path.
//! Unlike [`session::Session`], a `MatchDriverBridge` is not
//! `crate::registry`-backed — see that method's own doc and
//! [`render_export`]'s module doc ("The `MatchDriverBridge` half") for why
//! it is therefore an ordinary bound method rather than a
//! `crate::registry`-handle-keyed free function, while the actual per-frame
//! block still crosses raw memory exactly like `Session`'s path.
//!
//! [`match_snapshot_bridge`] is new: [`match_snapshot_bridge::match_snapshot_build`]
//! is the general-purpose "build a `WasmMatchSnapshot` from caller-specified
//! fields" entry point `packages/app/src/rollback_validation.spec.ts`'s and
//! `packages/app/src/match_rollback_lab.spec.ts`'s remaining skips both name
//! — `online_combat_phases_bridge::online_combat_phase_boundary_zero` was
//! the first step in that direction but only ever built one of seven pinned
//! fixtures; see that module's own doc for the full design (a fresh
//! `gc_sim::r#match::new` construction plus a small, explicit JSON override
//! set, never a `MatchSnapshot`-as-JSON round trip).
//!
//! [`player_pose_bridge`] is new: [`player_pose_bridge::player_pose_select`]
//! is a standalone `gc_render::player_pose::select` binding over any
//! caller-supplied `MatchPlayer`-shaped JSON, not only a live session's
//! current tick — `packages/render/src/replay.spec.ts`'s remaining skip
//! names this exact gap (a goal-replay buffer records raw `MatchState`/
//! `MatchPlayer` snapshots, never a decoded `RenderFrame`, so nothing had
//! run a buffered frame through pose selection at all). See that module's
//! doc for how a full `MatchPlayer` is reconstructed from the same narrow
//! slice [`match_state_bridge::match_state_to_json`] already emits.
//!
//! [`match_state_bridge::match_state_to_json`] gained one field, `press`
//! (`ByTeam<i64>`, tactic-derived chaser count) — `packages/app/src/bootstrap.spec.ts`'s
//! and `packages/app/src/flow.spec.ts`'s remaining skips both name it as the
//! one field reachable from neither `SimSession` nor
//! `gc_render::frame::RenderFrame` anywhere in `@gc/wasm`.
//!
//! ## v2 glue, wave W12-B: the last four skipped-test surfaces
//!
//! Four more gaps, each traced to a specific missing entry point on this
//! crate rather than a whole feature:
//!
//! [`session::Session::new`] gained three more parameters: `tactic`/
//! `away_tactic` (an authored `gc_data::tactics::ALL` id per side, resolved
//! via the new [`session`]-local `resolve_tactic`) and `home_starter_ids` (a
//! caller-supplied five-player starting XI, resolved and validated via the
//! new [`session`]-local `resolve_starter_roster`/`leak_roster`). Before
//! this, every session simulated `tactics::get("balanced")` unconditionally
//! on both sides and each team's fixed authored roster unconditionally —
//! `packages/app/src/bootstrap.spec.ts`'s "applies request roster,
//! formation, tactic, and seed" and `packages/app/src/flow.spec.ts`'s
//! ten-player press-high case both name this precisely: a request's
//! `tactic_id`/`home_starter_ids` had no parameter to reach
//! `MatchState.press` (or the player list) through at all. See
//! [`session::Session::new`]'s own doc for the exact validation each
//! parameter gets and — for `home_starter_ids` specifically — why a
//! synthetic `TeamData` with a leaked, five-pointer `'static` roster slice
//! is what actually threads a caller-supplied starting XI through
//! `sim_match::new`, which has no "starter ids" parameter of its own.
//!
//! [`gc_render::frame_buffer`]'s `RenderFrame::combat` gap
//! (`packages/screens/src/online_match_flow.spec.ts`'s "online combat
//! families" skip) was investigated, not built: reading `render/frame_buffer.lua`
//! directly confirms the Lua original never carries combat on the wire
//! either — only its own `combat_present` boolean crosses, exactly what
//! `crates/gc-render/src/frame_buffer.rs` already does. README rule "a port
//! reproduces behaviour, not intent" therefore forbids adding it here: doing
//! so would not be porting anything, it would be inventing a feature the Lua
//! game never had. No wire format, layout version, or differential fixture
//! changed. That test's real blocker (confirmed by that spec file's own
//! header comment, already reasoning independently to the same conclusion)
//! is `sim.combat`'s real readiness/telegraph timing having no wasm-bound
//! per-tick surface at all — which [`session::Session::combat_events_json`]
//! (below) is the BASE-path half of, but the online/rollback half this
//! specific skipped case needs is a genuinely separate, larger feature, not
//! attempted here.
//!
//! [`session::Session::combat_events_json`] is new: the BASE (non-rollback)
//! per-tick combat event getter `packages/app/src/app.spec.ts`'s header
//! names as the first of two remaining gaps ("neither `@gc/wasm`'s
//! `SimSession` nor `gc_render::frame::RenderFrame` exposes ANY per-tick
//! combat event/state getter for the BASE... session"). Reuses
//! [`rollback_events_bridge::raw_combat_event_to_json`] (now `pub(crate)`)
//! rather than inventing a second combat-event wire shape — the identical
//! shape `MatchDriverBridge`'s own tick output already crosses under
//! `combat_events`. The second gap that same header names (`match.ts`'s own
//! BASE-mode `update()` has no code path consuming combat events) is
//! `@gc/screens`, out of this crate's file ownership, and is not attempted
//! here.
//!
//! [`match_driver_bridge::MatchDriverBridge::roster_numeric`]/
//! [`match_driver_bridge::MatchDriverBridge::roster_ids_and_names`] are new:
//! `MatchDriverBridge` had no roster export of its own at all — unlike
//! [`session::Session`]'s `rosterNumeric`/`rosterIdsAndNames` — confirmed by
//! `packages/screens/src/online_match_flow.spec.ts`'s own header comment
//! ("`MatchDriverBridge` still has no roster export of its own... so this
//! case does not feed the captured frame into `@gc/render`'s
//! `pitchDrawCommands`"). Both simply re-encode this bridge's own
//! already-built `roster` field via `gc_render::frame_buffer::encode_roster`
//! — the identical function [`session::Session`]'s own getters already call
//! — rather than a second encoder.
#![deny(missing_docs)]

pub mod ai_driven;
pub mod coordinator_bridge;
pub mod determinism;
pub mod input_frame_bridge;
pub mod input_protocol_bridge;
pub mod json;
pub mod match_driver_bridge;
pub mod match_driver_fixture_bridge;
pub mod match_snapshot_bridge;
pub mod match_state_bridge;
pub mod net_inbox;
pub mod online_combat_phases_bridge;
pub mod player_pose_bridge;
pub mod protocol_bridge;
pub mod registry;
pub mod render_export;
pub mod rollback_events_bridge;
pub mod rollback_playable_lab_bridge;
pub mod session;
pub mod tuning_bridge;
pub mod wasm_transport;
