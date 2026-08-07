//! Match session lifecycle: the wasm-bindgen control surface a JS lobby/app
//! layer uses to create a match and advance it one tick at a time.
//!
//! ## Why input crosses as wire text, not fields
//!
//! [`Session::step`] takes a canonical [`gc_sim::input_frame`] wire string,
//! the exact same encoding the rollback netcode already puts on the network
//! and in the OMP-1 fixture. That format is already `String`-in/`String`-out
//! (`input_frame::decode`/`encode`), which `wasm-bindgen` binds natively
//! with no bespoke marshalling code — and it means a session driven from JS
//! consumes bit-identical input to a session driven from a recorded fixture,
//! which is the whole property this wasm build exists to prove.
//!
//! ## Scope
//!
//! An ordinary [`Session`] (see "An ordinary session is LEGACY mode" below)
//! steps the legacy single-player `MatchInput` path — [`Session::new`] no
//! longer builds an [`gc_sim::input_frame::InputOwnership`] for its own
//! live, steppable state at all. Slot mode still exists in this module —
//! [`Session::capture_snapshot`]/[`Session::snapshot_handle`] still hand
//! back a slot-mode boundary-zero snapshot, because that is what online and
//! rollback callers need — but nothing in this module steps a slot-mode
//! `MatchState` itself; the rollback netcode and the determinism fixture do
//! that with their own producers, downstream of the snapshot this module
//! hands them.
//!
//! ## Combat is opt-in, mirroring the Lua exactly
//!
//! [`Session::new`]'s `combat_enabled` parameter is this surface's version
//! of `game/screens/match.lua`'s own `_opts.combat_enabled`: `false`/absent
//! (the default) never builds a combat companion, exactly like an ordinary
//! Lua match; `true` builds one once, at construction, via
//! `gc_sim::combat::new_state` (`combat_sim.new_state(initial)` in the
//! Lua — see `Match:restart`), and [`Session::step`] threads it through
//! every subsequent `gc_sim::r#match::step` call unchanged in shape
//! (`Some`/`None` never flips after construction) — this reproduces the
//! Lua's *behavior*, not a new policy: `Match:restart` builds
//! `self._combat_state` exactly once and never rebuilds or clears it
//! mid-match either.
//!
//! ## An ordinary session is LEGACY mode, mirroring `Match:restart` exactly
//!
//! `game/screens/match.lua`'s `Match:restart` only builds an
//! `InputOwnership` (entering slot mode) when `rollback_options` (a
//! development-only lab) is set. An ordinary match — the only kind the
//! product build ever plays — stays on the legacy path: `sim_match.new` is
//! called with no `input_ownership`, so `sim.match.step` takes the "Legacy"
//! branch, applying the ONE local player's `MatchInput` to whichever player
//! index `MatchState.controlled` currently names, while every other player
//! runs the inline AI branch built into `sim::match::step` itself. This is
//! also the exact path `crates/gc-sim/tests/match_differential.rs` proves
//! bit-exact against real Lua for 7,201 ticks and all ten players — see
//! that test's own module doc.
//!
//! [`Session::new`] now builds its LIVE, steppable `MatchState`
//! (`crate::registry::Entry::state`) the same way: `input_ownership: None`.
//! [`Session::step`] decodes the wire it receives (still the full
//! eight-slot canonical `input_frame` format — see this module's "Why input
//! crosses as wire text" section above, unchanged), reads ONLY the single
//! canonical `home_1` slot (`gc_sim::input_frame::slot`'s index 1 —
//! [`LOCAL_SLOT_ZERO_INDEX`], the same "this session's browser player's
//! slot" convention this module already used before this change), and
//! dequantizes that one sample into a `MatchInput` via the existing
//! [`gc_sim::slot_input::to_match_input`] dequantizer — the very same
//! function `gc_sim::slot_input::materialize`'s `Frame` sources already use
//! internally — before stepping via `StepInput::Legacy`. Every other
//! canonical slot in the wire is simply never read: there is no bot fill to
//! materialize, because legacy mode's inline AI is not a producer-level
//! concept the way slot mode's declared bot sources are — it lives inside
//! `sim::match::step` itself, unconditionally, for every player that is not
//! `MatchState.controlled`.
//!
//! This REPLACES this module's prior design (every session forced into
//! slot mode, with seven canonical slots filled by a declared
//! `gc_sim::slot_input` bot producer seeded `seed + 1000.0 + slot_index`).
//! That design was deterministic, and it did fix the wave-prior bug where
//! an unfilled slot mode session played out scoreless with zero events (see
//! `slot_wiring_tests` below) — but it was a different computational path
//! from what an ordinary Lua match runs: different RNG streams, different
//! seeding, materialized through a producer at all, none of which
//! `game/screens/match.lua`'s own `Match:restart` does outside the
//! development-only rollback lab. A path can be fully deterministic and
//! still not be bit-identical to Lua; only [`Session::capture_snapshot`]'s
//! `rollback_boundary` (see [`crate::registry::Entry::rollback_boundary`]'s
//! doc) still needs slot mode now, because it seeds online/rollback play,
//! which genuinely is slot mode in both Lua and here.
//!
//! Online/rollback callers (`crate::match_driver_bridge`,
//! `crate::rollback_playable_lab_bridge`, `crate::coordinator_bridge`) are
//! unaffected: they never call [`Session::step`] to drive their own match
//! at all (confirmed by reading every `Session::new` call site in this
//! crate: each either calls `capture_snapshot`/`snapshot_handle` once,
//! immediately, before any `step`, or is a peer's own `MatchDriverBridge`
//! constructor argument that only reads `capture_snapshot()`, never
//! `step()`). What they need from a `Session` is a valid slot-mode
//! boundary-zero `MatchSnapshot` — [`crate::registry::Entry::rollback_boundary`] — which
//! [`Session::new`] still builds, at construction, exactly the way this
//! module used to build `Entry::state` itself. They supply their own
//! producer (`gc_netcode::match_driver::MatchDriver`'s) for the peers/bots
//! their own protocol assigns; `Session` was never on that path for
//! stepping, only for boundary-zero seeding, and that has not changed.
//!
//! ## Where the `MatchState` actually lives
//!
//! `Session` itself is just a `u32` handle into [`crate::registry`]'s slab.
//! [`crate::render_export`]'s raw, non-`wasm-bindgen` per-frame exports look
//! the same entry up by that handle — see the registry module's doc for why
//! the state cannot simply live inside this struct.

use gc_data::players;
use gc_data::tactics::{self, TacticData};
use gc_data::teams::{self, TeamData};
use gc_render::frame::{self as render_frame, RenderFrameOptions};
use gc_render::frame_buffer;
use gc_sim::combat;
use gc_sim::fixed_clock;
use gc_sim::input_frame::{self, InputFixtureRosters, InputOwnership, InputSlotAssignment};
use gc_sim::r#match as sim_match;
use gc_sim::match_snapshot::{self, PitchSize};
use gc_sim::slot_input;
use gc_sim::tuning::Tuning;
use wasm_bindgen::prelude::*;

use crate::registry::{self, Entry};

const FIELD_W: f64 = 960.0;
const FIELD_H: f64 = 540.0;

/// Build the eight canonical slot assignments from two five-player rosters
/// (`[keeper, outfield_1, outfield_2, outfield_3, outfield_4]`), matching
/// the convention `gc_data::teams::TeamData::roster` and the OMP-1 fixture
/// both use: the roster's first id is the keeper and sits outside the
/// four controlled slots per side.
/// `pub(crate)`, not private: [`crate::rollback_events_bridge`]'s own tests
/// need a slot-mode boundary-zero `MatchState` (input_tick advances only in
/// slot mode — see `gc_sim::r#match::step`'s doc) with direct access to
/// `MatchState.events`, which [`Session`]'s own narrow getter surface does
/// not expose. Reusing this rather than a second copy keeps exactly one
/// canonical slot assignment for the fixture teams.
pub(crate) fn ownership(home_roster: &[&str; 5], away_roster: &[&str; 5]) -> InputOwnership {
    let mut slots = Vec::with_capacity(8);
    for index in 0..input_frame::SLOT_COUNT {
        let canonical = input_frame::slot(index + 1).expect("canonical slot index in range");
        let source = match canonical.team {
            input_frame::Team::Home => home_roster,
            input_frame::Team::Away => away_roster,
        };
        // Slots 0..4 are home outfielders, 4..8 are away outfielders (see
        // input_frame::SLOT_ORDER); within one side the outfield roster
        // positions (1..5) map onto the four slots in order.
        let side_index = if matches!(canonical.team, input_frame::Team::Home) {
            index
        } else {
            index - input_frame::HOME_SLOT_COUNT
        };
        slots.push(InputSlotAssignment {
            slot: canonical.id,
            team: canonical.team,
            player_id: source[(side_index + 1) as usize].to_string(),
        });
    }
    InputOwnership {
        version: input_frame::VERSION,
        rosters: InputFixtureRosters {
            home: home_roster.iter().map(|s| (*s).to_string()).collect(),
            away: away_roster.iter().map(|s| (*s).to_string()).collect(),
        },
        slots: slots.try_into().expect("exactly eight slots built"),
    }
}

/// The single canonical slot (`gc_sim::input_frame::slot`'s index 1,
/// `"home_1"`) this session's own browser player drives — the same
/// `"home_1"`-is-the-local-slot convention
/// `crate::online_combat_phases_bridge`'s own driver fixture documents
/// ("Driver index 1 is the host, whose opening live slot is `home_1`").
///
/// [`Session::step`] reads only this one sample out of the eight-slot wire
/// it receives and dequantizes it into the ONE `MatchInput` a legacy-mode
/// step takes — see this module's doc, "An ordinary session is LEGACY
/// mode". Every other canonical slot in the wire goes unread: there is no
/// bot fill to materialize for it anymore, because legacy mode's AI for
/// every non-controlled player lives inside `sim::match::step` itself, not
/// in a per-slot producer.
const LOCAL_SLOT_ZERO_INDEX: usize = 0;

/// Resolves an optional caller-supplied tactic id (`gc_data::tactics::ALL`'s
/// ids, e.g. `"press_high"`) into the `&'static TacticData`
/// `sim_match::NewMatchOptions.tactic`/`.away_tactic` want. `None` passes
/// straight through as `None` — `sim_match::new` already defaults an absent
/// tactic to `tactics::get("balanced")` itself (that struct's own field
/// doc), so this never re-derives that default, matching `home_formation`'s
/// own "not validated/re-derived here" precedent for everything except the
/// one check that would otherwise become a downstream panic: an id that
/// names no authored tactic. `gc_data::tactics::get` returning `None` is the
/// only failure mode here, so unlike [`resolve_starter_roster`] this has no
/// shape invariant of its own to enforce.
///
/// Returns a plain `Result<_, String>`, not `Result<_, JsValue>` — see
/// [`resolve_starter_roster`]'s doc for why every helper in this module
/// stays off `JsValue` until [`Session::new`] itself wraps the error at the
/// `#[wasm_bindgen]` boundary.
///
/// # Errors
///
/// Returns `Err` (naming `field` and the offending id) if `id` is `Some` and
/// not authored content.
fn resolve_tactic(id: Option<&str>, field: &str) -> Result<Option<&'static TacticData>, String> {
    match id {
        Some(id) => tactics::get(id)
            .map(Some)
            .ok_or_else(|| format!("unknown {field} id: {id}")),
        None => Ok(None),
    }
}

/// Resolves five caller-supplied player ids into the canonical `&'static
/// str` identity `gc_data::players::ALL` already owns for each — never
/// leaking JS-owned string data, only reusing content that was already
/// `'static` (see [`leak_roster`], which this feeds) — and validates the
/// exact shape this module's own [`ownership`] helper and `sim_match::new`'s
/// downstream `build_team`/`input_frame::validate_ownership` both assume:
/// exactly five ids, no duplicate, the keeper at index 0 and nowhere else
/// (`gc_data::teams::TeamData::roster`'s own doc: "5 player ids... the
/// remaining 4 listed in formation line order" — the same
/// keeper-then-outfield convention [`ownership`]'s own doc already
/// documents for every authored roster), and no id shared with
/// `away_roster` (`input_frame::validate_ownership`'s own rule, "one roster
/// player cannot belong to both fixture sides" — violating it would reach
/// `sim_match::new`'s `.expect("valid input ownership")` and panic).
/// `sim_match::new` and [`ownership`] panic on a malformed roster rather
/// than returning a `Result` (README rule 5.5 is specifically about
/// JavaScript-originated input) — this validates the exact shape they
/// assume BEFORE either ever runs, so a bad caller-supplied starting XI
/// surfaces as a typed error here instead of a wasm panic downstream.
///
/// Returns a plain `Result<_, String>`, not `Result<_, JsValue>` —
/// `wasm_bindgen::JsValue::from_str` traps ("function not implemented on
/// non-wasm32 targets") off the `wasm32` target, aborting the whole native
/// test process, exactly the constraint `rollback_playable_lab_bridge.rs`'s
/// own `mod tests` documents and works around the same way: keep every
/// validation helper on `String` so it is exercisable from an ordinary
/// native `#[test]`, and let the thin `#[wasm_bindgen]` boundary
/// ([`Session::new`]) be the only place a `JsValue` gets constructed at all.
///
/// # Errors
///
/// Returns `Err` if `ids` is not exactly five entries, contains a
/// duplicate, names an unknown player id, does not have exactly one keeper
/// at index 0, or shares an id with `away_roster`.
fn resolve_starter_roster(
    ids: &[String],
    away_roster: &[&str; 5],
) -> Result<[&'static str; 5], String> {
    if ids.len() != 5 {
        return Err(
            "starting XI must be exactly five player ids (one keeper, four outfield)".to_string(),
        );
    }
    let mut resolved: [&'static str; 5] = [""; 5];
    for (index, id) in ids.iter().enumerate() {
        let player =
            players::get(id).ok_or_else(|| format!("unknown starting XI player id: {id}"))?;
        let is_keeper = player.position == players::Position::Keeper;
        if index == 0 {
            if !is_keeper {
                return Err("starting XI's first id must be the keeper".to_string());
            }
        } else if is_keeper {
            return Err("starting XI may only name one keeper, at index 0".to_string());
        }
        if away_roster.contains(&player.id) {
            return Err(format!(
                "starting XI id also belongs to the away fixture roster: {}",
                player.id
            ));
        }
        resolved[index] = player.id;
    }
    for i in 0..resolved.len() {
        for j in (i + 1)..resolved.len() {
            if resolved[i] == resolved[j] {
                return Err("starting XI ids must be unique".to_string());
            }
        }
    }
    Ok(resolved)
}

/// Leaks a small, fixed five-pointer array so a caller-supplied starting XI
/// can satisfy `gc_data::teams::TeamData::roster`'s `&'static [&'static
/// str]` field type — [`Session::new`] copies the authored home team and
/// overwrites just this field, since `NewMatchOptions` has no separate
/// "starter ids" parameter of its own (see [`Session::new`]'s doc). Every
/// element `roster` holds is already one of `gc_data::players::ALL`'s own
/// `&'static str` ids (built by [`resolve_starter_roster`]), so this never
/// leaks JS-owned string data, only a fresh container for five pointers into
/// content that was already `'static`. Bounded: one five-pointer allocation
/// per [`Session::new`] call that supplies `home_starter_ids`, for the
/// session's lifetime — the same leak-a-small-`'static`-container pattern
/// `gc_netcode::coordinator` and `gc_sim::rollback_validation` already use
/// elsewhere in this workspace, for the identical reason (turning validated,
/// dynamically-sized input into a `'static` reference a `#[derive(Clone,
/// Copy)]` content struct can hold).
fn leak_roster(roster: [&'static str; 5]) -> &'static [&'static str] {
    Box::leak(roster.to_vec().into_boxed_slice())
}

/// One live match. Construct with [`Session::new`], advance with
/// [`Session::step`], read state with the getters below. Dropping the JS
/// wrapper (or calling the generated `free()`) releases the underlying
/// [`crate::registry`] slot.
#[wasm_bindgen]
pub struct Session {
    handle: u32,
}

impl Drop for Session {
    fn drop(&mut self) {
        registry::free(self.handle);
    }
}

#[wasm_bindgen]
impl Session {
    /// Start a new match between two authored teams
    /// (`gc_data::teams::ALL`'s ids, e.g. `"nebula"` / `"orion"`).
    /// `home_formation` overrides the home team's authored default formation
    /// (`gc_data::formations::ALL`'s ids, e.g. `"2-1-1"`) — `None`/omitted
    /// keeps `home.formation`, exactly `sim_match::new`'s own default. This
    /// binding does not validate `home_formation` against
    /// `gc_data::formations::get` itself: `sim_match::new`'s own
    /// `NewMatchOptions.home_formation` is a plain `Option<&str>` stored
    /// as-is (`match.rs`'s `formation` field), with no such check on the
    /// Rust side either — the same content-table membership a screen like
    /// `packages/screens/src/formation.ts` is responsible for enforcing
    /// before ever offering an id here, not something this constructor
    /// silently re-derives.
    ///
    /// `combat_enabled` (`None`/omitted defaults to `false`) mirrors
    /// `game/screens/match.lua`'s own `_opts.combat_enabled` — see this
    /// module's doc's "Combat is opt-in, mirroring the Lua exactly" section.
    /// `false` (the default) reproduces the pre-existing behavior exactly:
    /// no combat companion is ever built, and every [`Session::step`] call
    /// passes `combat_state: None`, byte for byte what this constructor did
    /// before this parameter existed.
    ///
    /// `tactic`/`away_tactic` (`None`/omitted for either) name an authored
    /// `gc_data::tactics::ALL` id (e.g. `"press_high"`) for the home/away
    /// side respectively, reaching `sim_match::NewMatchOptions.tactic`/
    /// `.away_tactic` directly (resolved via [`resolve_tactic`]) — before
    /// this parameter existed every session simulated `tactics::get("balanced")`
    /// unconditionally on both sides, with no way for a caller to select
    /// anything else, so a request's `press_high` choice could never reach
    /// `MatchState.press` here. An id naming no authored tactic is a typed
    /// error, not the silent "not validated here" treatment `home_formation`
    /// gets — see [`resolve_tactic`]'s own doc for why the two parameters
    /// differ.
    ///
    /// `home_starter_ids` (`None`/omitted keeps `home.roster`, the authored
    /// default — exactly the prior, only behavior) overrides the home team's
    /// starting XI: five player ids, the keeper first and the four
    /// outfielders in formation line order, the same shape
    /// `gc_data::teams::TeamData::roster` itself uses. Every id must be
    /// authored content, distinct, shaped keeper-then-four-outfield, and not
    /// already on the away roster — see [`resolve_starter_roster`]'s own doc
    /// for exactly what is (and, notably, is NOT — squad eligibility for a
    /// particular team is a product policy a calling screen enforces, not a
    /// simulation invariant this constructor re-derives, matching
    /// `home_formation`'s own precedent) validated and why. There is no
    /// `away_starter_ids`: the away side keeps its authored `away.roster`
    /// unconditionally, the same scope `home_formation` already draws (no
    /// `away_formation` parameter exists here either).
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if either team id is not authored
    /// content, does not carry a five-player roster, `tactic`/`away_tactic`
    /// names no authored tactic, or `home_starter_ids` is present and
    /// malformed (see [`resolve_starter_roster`]'s doc for the exact
    /// conditions).
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(constructor)]
    pub fn new(
        home_team_id: &str,
        away_team_id: &str,
        seed: f64,
        duration_seconds: f64,
        max_goals: i32,
        home_formation: Option<String>,
        combat_enabled: Option<bool>,
        tactic: Option<String>,
        away_tactic: Option<String>,
        home_starter_ids: Option<Vec<String>>,
    ) -> Result<Session, JsValue> {
        let home =
            teams::get(home_team_id).ok_or_else(|| JsValue::from_str("unknown home team id"))?;
        let away =
            teams::get(away_team_id).ok_or_else(|| JsValue::from_str("unknown away team id"))?;
        if home.roster.len() != 5 || away.roster.len() != 5 {
            return Err(JsValue::from_str(
                "session play requires a five-player roster (one keeper, four outfield)",
            ));
        }
        let away_roster: [&str; 5] = away.roster.try_into().expect("checked len == 5");

        // A caller-supplied starting XI overrides `home.roster` -- see
        // `Session::new`'s doc and `resolve_starter_roster`'s/
        // `leak_roster`'s own docs for why a synthetic `TeamData` (every
        // field copied from the authored team except `roster`) is what
        // actually threads it through, since `NewMatchOptions` has no
        // separate "starter ids" field of its own.
        let (home_effective, home_roster): (TeamData, [&str; 5]) = match home_starter_ids {
            Some(ids) => {
                let resolved = resolve_starter_roster(&ids, &away_roster)
                    .map_err(|err| JsValue::from_str(&err))?;
                let mut synthetic = *home;
                synthetic.roster = leak_roster(resolved);
                (synthetic, resolved)
            }
            None => (*home, home.roster.try_into().expect("checked len == 5")),
        };

        let home_tactic =
            resolve_tactic(tactic.as_deref(), "tactic").map_err(|err| JsValue::from_str(&err))?;
        let away_tactic_data = resolve_tactic(away_tactic.as_deref(), "away tactic")
            .map_err(|err| JsValue::from_str(&err))?;

        let tune = Tuning::new();
        // The LIVE, steppable state -- LEGACY mode (`input_ownership:
        // None`), mirroring `game/screens/match.lua`'s `Match:restart` for
        // an ordinary (non-rollback-lab) match exactly. See this module's
        // doc, "An ordinary session is LEGACY mode".
        let mut state = sim_match::new(sim_match::NewMatchOptions {
            home: &home_effective,
            away,
            field: PitchSize {
                w: FIELD_W,
                h: FIELD_H,
            },
            home_formation: home_formation.as_deref(),
            tactic: home_tactic,
            away_tactic: away_tactic_data,
            duration: Some(duration_seconds),
            max_goals: Some(i64::from(max_goals)),
            seed: Some(seed),
            players_by_id: None,
            species_by_id: None,
            showcase_players_by_id: None,
            // `None` matches `Match:restart`'s own ordinary-match call,
            // which never passes `human_controlled` either -- `sim_match::new`
            // defaults an absent value to `true` (one controlled player),
            // exactly what an ordinary product match wants.
            human_controlled: None,
            input_ownership: None,
        });
        // Built once, at construction, exactly like `Match:restart`'s own
        // `initial_combat = combat_sim.new_state(initial)` -- never rebuilt
        // or cleared for the rest of this session's life. `None` (the same
        // roster override `match_driver_fixture::initial_snapshot` and
        // `match_driver_fixture::DriverRules` both pass) matches every
        // other already-tested `combat::new_state` call site in this
        // workspace rather than guessing at an untested override.
        let combat_state = if combat_enabled.unwrap_or(false) {
            Some(combat::new_state(&mut state, None))
        } else {
            None
        };
        let roster = render_frame::roster(&state);

        // The SLOT-MODE boundary-zero snapshot -- built independently, from
        // the identical match parameters above except `input_ownership`,
        // for `Session::capture_snapshot`/`snapshot_handle` (online/rollback
        // callers; see `crate::registry::Entry::rollback_boundary`'s doc).
        // `sim_match::new` is a pure function of these parameters, so this
        // reproduces exactly what this constructor used to hand back as its
        // OWN `state` before this change (every field it prints/hashes is
        // determined the same way -- see `sim_match::new`'s body -- except
        // the ownership-derived fields the two calls deliberately differ
        // on: `slot_mode`, `input_ownership`, `slot_players`).
        let mut rollback_state = sim_match::new(sim_match::NewMatchOptions {
            home: &home_effective,
            away,
            field: PitchSize {
                w: FIELD_W,
                h: FIELD_H,
            },
            home_formation: home_formation.as_deref(),
            tactic: home_tactic,
            away_tactic: away_tactic_data,
            duration: Some(duration_seconds),
            max_goals: Some(i64::from(max_goals)),
            seed: Some(seed),
            players_by_id: None,
            species_by_id: None,
            showcase_players_by_id: None,
            // `None` matches the OMP-1 fixture's own construction
            // (`determinism_evidence::new_state`) rather than guessing at an
            // untested combination.
            human_controlled: None,
            input_ownership: Some(ownership(&home_roster, &away_roster)),
        });
        let rollback_combat_state = if combat_enabled.unwrap_or(false) {
            Some(combat::new_state(&mut rollback_state, None))
        } else {
            None
        };
        let rollback_boundary =
            match_snapshot::capture(&rollback_state, rollback_combat_state.as_ref());

        let handle = registry::insert(Entry {
            state,
            tune,
            roster,
            combat: combat_state,
            tick: 0,
            rollback_boundary,
        });
        Ok(Session { handle })
    }

    fn with_entry<R>(&self, f: impl FnOnce(&mut Entry) -> R) -> R {
        registry::with_entry(self.handle, f)
            .expect("a live Session's registry entry is never missing")
    }

    /// Advance the match by exactly one fixed tick, consuming one canonical
    /// `input_frame` wire (`gc_sim::input_frame::encode`'s format). Only the
    /// `home_1` slot ([`LOCAL_SLOT_ZERO_INDEX`],
    /// `gc_sim::input_frame::slot`'s index 1) is read from `wire`; every
    /// other canonical slot goes unread. This session's `MatchState`
    /// (`crate::registry::Entry::state`) runs in LEGACY mode (see this
    /// module's doc, "An ordinary session is LEGACY mode"), so the ONE
    /// dequantized `home_1` sample IS the tick's whole `MatchInput` —
    /// dequantized via the existing [`gc_sim::slot_input::to_match_input`],
    /// the same function slot mode's own `Frame` sources already use, per
    /// the task brief this module was written against: reuse the
    /// dequantizer rather than invent a second conversion. Every other
    /// player is driven by `sim::match::step`'s own inline AI branch,
    /// unconditionally, for whichever player index is not
    /// `MatchState.controlled` — not by a bot fill this method materializes.
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if `wire` fails to decode as a
    /// canonical input frame, or if its `home_1` sample is not a validly
    /// quantized one.
    pub fn step(&mut self, wire: &str) -> Result<(), JsValue> {
        let frame = input_frame::decode(wire).map_err(|err| JsValue::from_str(&err.to_string()))?;
        input_frame::validate(&frame).map_err(|err| JsValue::from_str(&err.to_string()))?;
        let match_input = slot_input::to_match_input(&frame.slots[LOCAL_SLOT_ZERO_INDEX]);
        self.with_entry(|entry| {
            sim_match::step(
                &mut entry.state,
                gc_sim::fixed_clock::TICK_SECONDS,
                sim_match::StepInput::Legacy(match_input),
                entry.combat.as_mut(),
                &entry.tune,
            );
            // `entry.state.input_tick` never advances in legacy mode (only
            // `sim::match::step`'s slot-mode branch touches it) -- this is
            // the session's own counter every existing caller's `inputTick`
            // contract depends on. See `crate::registry::Entry::tick`'s doc.
            entry.tick += 1;
        });
        Ok(())
    }

    /// Home score.
    #[wasm_bindgen(getter, js_name = scoreHome)]
    pub fn score_home(&self) -> f64 {
        self.with_entry(|entry| entry.state.score.home as f64)
    }

    /// Away score.
    #[wasm_bindgen(getter, js_name = scoreAway)]
    pub fn score_away(&self) -> f64 {
        self.with_entry(|entry| entry.state.score.away as f64)
    }

    /// Seconds remaining.
    #[wasm_bindgen(getter, js_name = timeLeft)]
    pub fn time_left(&self) -> f64 {
        self.with_entry(|entry| entry.state.time_left)
    }

    /// Whether the match has finished.
    #[wasm_bindgen(getter)]
    pub fn finished(&self) -> bool {
        self.with_entry(|entry| entry.state.finished)
    }

    /// Ticks [`Session::step`] has run so far -- `crate::registry::Entry::tick`,
    /// not `MatchState.input_tick` (which stays `0` for this session's whole
    /// life; see that field's doc for why).
    #[wasm_bindgen(getter, js_name = inputTick)]
    pub fn input_tick(&self) -> f64 {
        self.with_entry(|entry| entry.tick as f64)
    }

    /// The current canonical snapshot hash (`gc_sim::match_snapshot::hash`),
    /// for a JS caller that wants to compare against a peer without
    /// decoding the render block. Includes the combat companion in the
    /// hashed snapshot when this session was constructed with
    /// `combat_enabled = true` (`entry.combat`), matching
    /// `match_snapshot::capture`'s own contract: a combat-bearing session
    /// whose hash silently dropped combat state would agree with a
    /// non-combat peer it must not agree with.
    #[wasm_bindgen(js_name = snapshotHash)]
    pub fn snapshot_hash(&self) -> String {
        self.with_entry(|entry| {
            match_snapshot::hash(&match_snapshot::capture(
                &entry.state,
                entry.combat.as_ref(),
            ))
        })
    }

    /// This match's registry handle, for [`crate::render_export`]'s raw
    /// per-frame exports (`renderFrameBuild(handle)`,
    /// `renderFramePtr()`/`renderFrameLen()`).
    #[wasm_bindgen(getter)]
    pub fn handle(&self) -> u32 {
        self.handle
    }

    /// Match-constant per-player roster fields, as a flat `Float64Array`
    /// (`gc_render::frame_buffer::encode_roster`'s numeric half). Crosses
    /// once per match, not per frame, so it uses the ordinary
    /// `wasm-bindgen` path rather than [`crate::render_export`]'s raw one.
    #[wasm_bindgen(js_name = rosterNumeric)]
    pub fn roster_numeric(&self) -> Vec<f64> {
        self.with_entry(|entry| frame_buffer::encode_roster(&entry.roster).0)
    }

    /// Match-constant roster ids and display names
    /// (`gc_render::frame_buffer::encode_roster`'s string half, a
    /// newline-joined blob per that function's documented format).
    #[wasm_bindgen(js_name = rosterIdsAndNames)]
    pub fn roster_ids_and_names(&self) -> String {
        self.with_entry(|entry| frame_buffer::encode_roster(&entry.roster).1)
    }

    /// This session's current boundary snapshot
    /// (`gc_sim::match_snapshot::capture`), as an opaque handle — for
    /// building a `crate::rollback_events_bridge::RollbackEventsTimeline`
    /// via its `create` (satisfying
    /// `packages/online/src/match_presentation.ts`'s
    /// `newOnlineMatchPresentation`) without a `MatchDriverBridge`. See
    /// [`crate::rollback_events_bridge::WasmMatchSnapshot`]'s doc for why
    /// this crosses as an opaque handle rather than JSON.
    #[wasm_bindgen(js_name = snapshotHandle)]
    pub fn snapshot_handle(&self) -> crate::rollback_events_bridge::WasmMatchSnapshot {
        crate::rollback_events_bridge::WasmMatchSnapshot::new(self.capture_snapshot())
    }

    /// This session's raw simulation state, as JSON —
    /// `crate::match_state_bridge::match_state_to_json`'s shape, matching
    /// `packages/render/src/replay.ts`'s `MatchState` interface
    /// field-for-field. Unlike [`Session::snapshot_handle`] (an opaque
    /// handle, never inspected on the JS side) this crosses fully decoded:
    /// `replay.ts`'s `captureFrame` needs to actually read
    /// `outfield_press`/`transition`/per-player timers, not merely retain
    /// them for a later hash comparison. See `crate::match_state_bridge`'s
    /// module doc for why this is a narrow, hand-picked slice of
    /// `MatchState` rather than the whole (70+ field) struct, and why it
    /// crosses as JSON rather than a raw block like
    /// [`crate::render_export`]'s per-frame `RenderFrame`.
    #[wasm_bindgen(js_name = matchStateJson)]
    #[must_use]
    pub fn match_state_json(&self) -> String {
        self.with_entry(|entry| crate::match_state_bridge::match_state_to_json(&entry.state))
            .to_json_string()
    }

    /// This session's most recently stepped tick's combat events
    /// (`gc_sim::combat_snapshot::CombatEvent`), as a JSON array in
    /// `crate::rollback_events_bridge::raw_combat_event_to_json`'s shape —
    /// the exact wire shape `MatchDriverBridge`'s own `advance()` batch and
    /// `OnlineCombatPhasesBridge.onlineCombatPhaseObserved`'s
    /// `combat_events_json` parameter already use for `combat_events`, so a
    /// caller decoding this needs no second parser. This is the BASE
    /// (non-rollback) counterpart those two already have: before this
    /// getter existed, a [`Session`] built with `combat_enabled = true`
    /// ([`Session::new`]) drove its combat companion every tick with no way
    /// for a caller to observe what it did — confirmed absent from this
    /// bridge's surface and `types.ts` before this method.
    ///
    /// `"[]"` when this session was built without `combat_enabled` (no
    /// combat companion at all — see [`Session::new`]'s doc), and also
    /// before the first [`Session::step`] call. Reflects exactly the tick
    /// [`Session::step`] most recently ran: like `gc_sim::combat::step`'s
    /// own per-tick `events` field, this is REPLACED, not accumulated, by
    /// every step (`combat.rs`'s own `events.clear()` at the top of each
    /// step) — read it before the next `step` call if the caller needs
    /// every tick's events.
    #[wasm_bindgen(js_name = combatEventsJson)]
    #[must_use]
    pub fn combat_events_json(&self) -> String {
        self.with_entry(|entry| {
            let events: &[gc_sim::combat_snapshot::CombatEvent] =
                entry.combat.as_ref().map_or(&[], |c| c.events.as_slice());
            crate::json::Json::Array(
                events
                    .iter()
                    .map(crate::rollback_events_bridge::raw_combat_event_to_json)
                    .collect(),
            )
            .to_json_string()
        })
    }
}

impl Session {
    /// This session's slot-mode boundary-zero snapshot
    /// (`crate::registry::Entry::rollback_boundary`), for
    /// `crate::match_driver_bridge`'s online match construction: a
    /// [`gc_netcode::match_driver::MatchDriver`] is built from a boundary-zero
    /// snapshot the same way `sim_match::new` builds one here, and reusing
    /// this crate's own already-tested team/ownership resolution (rather
    /// than re-deriving it from a manifest's content id — see that module's
    /// doc for why online content resolution beyond team roster selection is
    /// out of this wave's scope) is cheaper and no less correct than
    /// building a second path to the same state.
    ///
    /// NOT derived from `entry.state` (unlike before this module's ordinary
    /// session moved to legacy mode): `entry.state.slot_mode` is `false`
    /// now, and `gc_sim::rollback_session::new` asserts the snapshot it is
    /// handed is slot-mode. `entry.rollback_boundary` was built once, at
    /// construction, from the identical match parameters in slot mode —
    /// see [`Session::new`]'s doc — specifically so this method keeps
    /// returning a valid online boundary zero regardless of what mode the
    /// session's own live `state` runs in.
    ///
    /// Carries its own combat companion, same as [`Session::snapshot_hash`]
    /// carries `entry.combat` — a combat-enabled `Session` handed to
    /// `crate::match_driver_bridge::MatchDriverBridge::new` seeds an online
    /// match whose boundary zero silently lacked the combat companion
    /// otherwise, contradicting the `Session` it was captured from.
    pub(crate) fn capture_snapshot(&self) -> match_snapshot::MatchSnapshot {
        self.with_entry(|entry| entry.rollback_boundary.clone())
    }
}

/// Build a [`RenderFrameOptions`] that reuses `entry`'s cached roster
/// instead of rebuilding it every frame. Shared with
/// [`crate::render_export`].
pub(crate) fn frame_options(entry: &Entry) -> RenderFrameOptions {
    RenderFrameOptions {
        roster: Some(entry.roster.clone()),
        ..Default::default()
    }
}

/// A wasm-bound `gc_sim::fixed_clock::FixedClockState`: the render-driven
/// tick-count decision for one match, planned once per render update from a
/// `dt`. See v2/README.md §2.1 -- tick COUNT changes what the simulation
/// computes (it decides how many [`Session::step`] calls run), so the policy
/// that turns a render `dt` into a tick count must be computed here, not
/// re-derived in TypeScript. That is exactly what happened before this type
/// existed: `packages/screens/src/match.ts` hand-copied
/// `gc_sim::fixed_clock::advance`'s accumulator/catch-up/drop algorithm,
/// undetectably drifting from this crate the moment either side's constants
/// changed. `FixedClock` below removes the second implementation instead of
/// pinning it: TypeScript calls [`FixedClock::advance`] and gets back a tick
/// count, nothing more.
///
/// `gc_sim::fixed_clock::advance` is generic over a per-tick input type and
/// step callback so a native, single-process caller can supply both inline;
/// across the wasm boundary that shape does not fit (a JS caller cannot hand
/// wasm a closure to invoke once per simulated tick without a much larger
/// marshalling layer), and it does not need to -- `MatchScreen`
/// (`packages/screens/src/match.ts`) already advances one input sample per
/// [`Session::step`] call itself, outside this clock. So [`FixedClock::advance`]
/// below calls `fixed_clock::advance` with a trivial `()` input and a
/// `step_fn` that always continues, and reports only the resulting tick
/// COUNT -- an upper bound the caller must run via [`Session::step`],
/// breaking out of that loop and calling [`FixedClock::stop_early`] instead
/// of running the rest if it stops before running all of them (e.g. the
/// match finishes mid-batch) -- exactly mirroring what a `false` return from
/// `advance`'s own `step_fn` would have done to the accumulator.
#[wasm_bindgen]
pub struct FixedClock {
    state: fixed_clock::FixedClockState,
}

#[wasm_bindgen]
impl FixedClock {
    /// A fresh clock at tick zero.
    #[wasm_bindgen(constructor)]
    pub fn new() -> FixedClock {
        FixedClock {
            state: fixed_clock::new(),
        }
    }

    /// Ticks this clock has authorized so far -- its own bookkeeping,
    /// independent of (and expected to track) [`Session::input_tick`].
    #[wasm_bindgen(getter)]
    pub fn tick(&self) -> f64 {
        self.state.tick as f64
    }

    /// Accumulate `render_dt` seconds and report how many ticks the caller
    /// should run this render update -- `gc_sim::fixed_clock::advance`'s
    /// exact catch-up/drop policy (`TICK_SECONDS`, `MAX_TICKS_PER_UPDATE`,
    /// its epsilon), minus the `step_fn` early-stop hook (see this struct's
    /// doc). The caller must run at most this many [`Session::step`] calls,
    /// in order; if it runs fewer, it must call [`FixedClock::stop_early`]
    /// afterward.
    ///
    /// # Errors
    ///
    /// Returns a `JsValue` (a `String`) if `render_dt` is not finite and
    /// non-negative -- mirrors `advance`'s own assertion.
    pub fn advance(&mut self, render_dt: f64) -> Result<u32, JsValue> {
        if !(render_dt.is_finite() && render_dt >= 0.0) {
            return Err(JsValue::from_str(
                "render dt must be a finite non-negative number",
            ));
        }
        let outcome =
            fixed_clock::advance(&mut self.state, render_dt, |_tick| (), |_tick, ()| true);
        Ok(outcome.ticks)
    }

    /// The caller ran fewer ticks than the last [`FixedClock::advance`] call
    /// authorized (its `step_fn` would have returned `false`) -- zeroes the
    /// carried-over accumulator, exactly as `advance`'s own early-stop
    /// branch does.
    #[wasm_bindgen(js_name = stopEarly)]
    pub fn stop_early(&mut self) {
        self.state.accumulator = 0.0;
    }
}

impl Default for FixedClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod fixed_clock_tests {
    use super::*;

    #[test]
    fn a_fresh_clock_plans_zero_ticks_for_a_sub_tick_dt() {
        let mut clock = FixedClock::new();
        assert_eq!(
            clock.advance(1.0 / 120.0).expect("finite non-negative dt"),
            0
        );
    }

    #[test]
    fn ticks_accumulate_across_calls_exactly_like_gc_sim_fixed_clock() {
        let mut clock = FixedClock::new();
        assert_eq!(clock.advance(1.0 / 120.0).unwrap(), 0);
        assert_eq!(clock.advance(1.0 / 120.0).unwrap(), 1);
        assert_eq!(clock.advance(1.0 / 30.0).unwrap(), 2);
    }

    #[test]
    fn a_long_frame_is_capped_at_max_ticks_per_update() {
        let mut clock = FixedClock::new();
        // Ten seconds of debt in one render update -- far more than
        // MAX_TICKS_PER_UPDATE (8) whole ticks.
        let ticks = clock.advance(10.0).unwrap();
        assert_eq!(ticks, fixed_clock::MAX_TICKS_PER_UPDATE);
    }

    // `advance`'s error path returns a `JsValue`, which
    // `wasm_bindgen::JsValue::from_str` cannot construct off the wasm32
    // target ("function not implemented on non-wasm32 targets", aborting the
    // whole native test process) -- see `coordinator_bridge.rs`'s identical
    // note. `sim_host.spec.ts` (`@gc/app`, which exercises the real compiled
    // artifact) covers `planTicks` rejecting a non-finite/negative `dt`.

    #[test]
    fn stop_early_zeroes_the_accumulator_so_the_next_advance_starts_fresh() {
        let mut clock = FixedClock::new();
        // Bank a fraction of a tick, then discard it via stop_early --
        // mirrors what MatchScreen does when the match finishes mid-batch.
        clock.advance(1.0 / 120.0).unwrap();
        clock.stop_early();
        // A dt just under one tick after a reset plans zero ticks -- if
        // stop_early had not zeroed the accumulator, the banked 1/120s
        // would combine with this call and plan one.
        assert_eq!(clock.advance(1.0 / 120.0).unwrap(), 0);
    }
}

#[cfg(test)]
mod slot_wiring_tests {
    use super::*;

    /// Regression test for an earlier wave's bug: every match played in the
    /// browser finished 0-0 with zero events because seven of the eight
    /// canonical slots received a permanent all-neutral row forever, tick
    /// after tick (`Session` used to force every session into slot mode
    /// with no bot fill for the other seven slots). This test drives a real
    /// [`Session`] end to end with the local `home_1` slot itself ALSO
    /// idle the whole match (an all-neutral wire every tick, exactly as an
    /// AFK browser player would produce), across the same seeds the
    /// original diagnosis used.
    ///
    /// `Session` has since moved to LEGACY mode for an ordinary match (see
    /// this module's doc, "An ordinary session is LEGACY mode") — a
    /// different fix for the same class of symptom. This test still holds:
    /// with the local player idle, `sim::match::step`'s inline AI branch
    /// drives every one of the other nine players unconditionally (proven
    /// bit-exact against Lua by `crates/gc-sim/tests/match_differential.rs`,
    /// which is exactly this scenario at the `sim::match::step` layer), so
    /// the match must still be live regardless of what the local player
    /// does — just via legacy mode's inline AI now, not a slot-mode bot
    /// producer.
    #[test]
    fn a_full_match_on_an_idle_local_wire_is_not_a_permanent_scoreless_zero_event_stalemate() {
        for seed in [1.0, 17.0, 42.0, 120.0] {
            // `gc_sim::r#match::NO_GOAL_LIMIT` -- an explicit cap large
            // enough that a two-minute match never hits it, so `finished`
            // is driven by the clock exactly like an ordinary browser
            // match, not by a test-only goal cap.
            let mut session = Session::new(
                "nebula", "orion", seed, 120.0, 99, None, None, None, None, None,
            )
            .expect("authored teams with five-player rosters");
            let mut total_events: usize = 0;
            loop {
                let finished = session.with_entry(|entry| entry.state.finished);
                if finished {
                    break;
                }
                let tick = session.input_tick() as i64;
                let neutral =
                    input_frame::new(tick, None).expect("an all-neutral frame is always valid");
                let wire = input_frame::encode(&neutral).expect("a valid frame always encodes");
                session
                    .step(&wire)
                    .expect("a canonical neutral wire always decodes");
                total_events += session.with_entry(|entry| entry.state.events.len());
            }
            let score_home = session.score_home();
            let score_away = session.score_away();
            assert!(
                total_events > 0 || score_home > 0.0 || score_away > 0.0,
                "seed {seed}: a full match produced zero events and a 0-0 score \
                 -- every non-local slot is still receiving permanent neutral input"
            );
        }
    }
}

#[cfg(test)]
mod combat_opt_in_tests {
    use super::*;

    fn neutral_wire(tick: i64) -> String {
        let neutral = input_frame::new(tick, None).expect("an all-neutral frame is always valid");
        input_frame::encode(&neutral).expect("a valid frame always encodes")
    }

    /// Regression test for this wave's bug: before this fix,
    /// `Session::step` hard-coded `combat_state: None` unconditionally --
    /// there was no way to turn combat on at all, for any caller. `None`/
    /// omitted must keep reproducing that exact prior behavior (no combat
    /// companion, ever).
    #[test]
    fn combat_enabled_omitted_or_false_never_builds_a_combat_companion() {
        for combat_enabled in [None, Some(false)] {
            let mut session = Session::new(
                "nebula",
                "orion",
                7.0,
                20.0,
                3,
                None,
                combat_enabled,
                None,
                None,
                None,
            )
            .expect("authored teams with five-player rosters");
            assert!(
                session.with_entry(|entry| entry.combat.is_none()),
                "combat_enabled={combat_enabled:?} must not build a combat companion"
            );
            // Stepping must still work exactly as before -- no combat state
            // to pass means `sim_match::step` keeps receiving `None`.
            let tick = session.input_tick() as i64;
            session
                .step(&neutral_wire(tick))
                .expect("a canonical neutral wire always decodes");
        }
    }

    /// The opt-in itself: `combat_enabled = true` builds the combat
    /// companion once, at construction (mirroring `Match:restart`'s
    /// `combat_sim.new_state(initial)`), and every `step` call afterward
    /// keeps driving it without panicking -- proving the wiring reaches
    /// `sim_match::step`'s `combat_state` parameter for real, not just that
    /// `Entry.combat` is non-`None` at construction.
    #[test]
    fn combat_enabled_true_builds_and_drives_a_combat_companion_every_tick() {
        let mut session = Session::new(
            "nebula",
            "orion",
            7.0,
            20.0,
            3,
            None,
            Some(true),
            None,
            None,
            None,
        )
        .expect("authored teams with five-player rosters");
        assert!(
            session.with_entry(|entry| entry.combat.is_some()),
            "combat_enabled=true must build a combat companion at construction"
        );
        for _ in 0..30 {
            let tick = session.input_tick() as i64;
            session
                .step(&neutral_wire(tick))
                .expect("a canonical neutral wire always decodes");
            assert!(
                session.with_entry(|entry| entry.combat.is_some()),
                "the combat companion must never disappear mid-match"
            );
        }
    }

    /// Regression test for this wave's gap: before [`Session::combat_events_json`]
    /// existed, nothing on this base (non-rollback) path could report what
    /// combat did on a given tick at all -- a combat-enabled session drove
    /// its companion every step with no observable output. Without a
    /// companion (`combat_enabled` omitted/`false`) this must always be an
    /// empty JSON array, both before the first `step` and after several.
    #[test]
    fn combat_events_json_is_always_an_empty_array_without_a_combat_companion() {
        let mut session = Session::new(
            "nebula",
            "orion",
            7.0,
            20.0,
            3,
            None,
            Some(false),
            None,
            None,
            None,
        )
        .expect("authored teams with five-player rosters");
        assert_eq!(session.combat_events_json(), "[]");
        for _ in 0..5 {
            let tick = session.input_tick() as i64;
            session
                .step(&neutral_wire(tick))
                .expect("a canonical neutral wire always decodes");
            assert_eq!(session.combat_events_json(), "[]");
        }
    }

    /// The wiring itself: [`Session::combat_events_json`] must reflect the
    /// REAL `gc_sim::combat_snapshot::CombatMatchState.events` this
    /// session's own combat companion carries after each `step` -- not a
    /// hard-coded `"[]"`, and not an accumulation across ticks (`combat.rs`'s
    /// `events.clear()` at the top of every `combat::step` call means each
    /// tick's count can rise, fall, or repeat). Comparing the decoded JSON
    /// array's length against `entry.combat.events.len()` directly (rather
    /// than asserting a specific count, which would depend on
    /// `sim.combat`'s own request-generation behavior) proves the getter
    /// reads the live companion every call instead of a stale or default
    /// value.
    #[test]
    fn combat_events_json_mirrors_the_combat_companion_s_own_per_tick_event_count() {
        let mut session = Session::new(
            "nebula",
            "orion",
            7.0,
            20.0,
            3,
            None,
            Some(true),
            None,
            None,
            None,
        )
        .expect("authored teams with five-player rosters");
        for _ in 0..30 {
            let tick = session.input_tick() as i64;
            session
                .step(&neutral_wire(tick))
                .expect("a canonical neutral wire always decodes");
            let expected = session.with_entry(|entry| {
                entry
                    .combat
                    .as_ref()
                    .expect("combat_enabled=true always carries a companion")
                    .events
                    .len()
            });
            let json = session.combat_events_json();
            let parsed =
                crate::json::Json::parse(&json).expect("combat_events_json is always valid JSON");
            let items = parsed
                .as_array()
                .expect("combat_events_json is always a JSON array");
            assert_eq!(
                items.len(),
                expected,
                "combat_events_json must mirror this tick's real combat.events count"
            );
        }
    }

    /// [`Session::snapshot_hash`]/[`Session::capture_snapshot`] must carry
    /// the combat companion through, not silently drop it the way both did
    /// before this fix (`match_snapshot::capture(&entry.state, None)`
    /// unconditionally). Two sessions built from the same seed/duration but
    /// differing only in `combat_enabled` must disagree on the captured
    /// snapshot's hash -- if they agreed, the combat companion would not
    /// actually be part of what gets captured/hashed.
    #[test]
    fn snapshot_hash_and_capture_snapshot_carry_the_combat_companion() {
        let without_combat = Session::new(
            "nebula",
            "orion",
            7.0,
            20.0,
            3,
            None,
            Some(false),
            None,
            None,
            None,
        )
        .expect("authored teams with five-player rosters");
        let with_combat = Session::new(
            "nebula",
            "orion",
            7.0,
            20.0,
            3,
            None,
            Some(true),
            None,
            None,
            None,
        )
        .expect("authored teams with five-player rosters");

        assert_ne!(
            without_combat.snapshot_hash(),
            with_combat.snapshot_hash(),
            "a combat-enabled session's hash must differ from an otherwise-identical \
             non-combat session -- otherwise the combat companion is not really being hashed"
        );

        let without_combat_snapshot = without_combat.capture_snapshot();
        let with_combat_snapshot = with_combat.capture_snapshot();
        assert_ne!(
            match_snapshot::hash(&without_combat_snapshot),
            match_snapshot::hash(&with_combat_snapshot),
            "capture_snapshot must also carry the combat companion through"
        );
    }
}

#[cfg(test)]
mod tactic_and_starter_xi_tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn new_session(
        home_formation: Option<String>,
        tactic: Option<String>,
        away_tactic: Option<String>,
        home_starter_ids: Option<Vec<String>>,
    ) -> Result<Session, JsValue> {
        Session::new(
            "nebula",
            "orion",
            7.0,
            20.0,
            3,
            home_formation,
            None,
            tactic,
            away_tactic,
            home_starter_ids,
        )
    }

    /// Regression test for this wave's bug: before this fix `Session::new`
    /// had no `tactic`/`away_tactic` parameter at all and always built
    /// `tactics::get("balanced")` on both sides -- `press.home` (directly
    /// `home_tactic.press`, `sim_match::new`'s own construction, no `step`
    /// needed to observe it) could never be anything but `balanced`'s `1`.
    /// `"press_high"`'s own authored `press` value is `2`
    /// (`gc_data::tactics::ALL`) -- this is the same `press.home == 2`
    /// assertion `packages/app/src/flow.spec.ts`'s blocked "ten-player
    /// press-high" case needs.
    #[test]
    fn a_supplied_home_tactic_id_reaches_the_match_s_press_state() {
        let balanced =
            new_session(None, None, None, None).expect("no tactic keeps the balanced default");
        assert_eq!(balanced.with_entry(|entry| entry.state.press.home), 1);

        let press_high = new_session(None, Some("press_high".to_string()), None, None)
            .expect("press_high is authored content");
        assert_eq!(press_high.with_entry(|entry| entry.state.press.home), 2);
    }

    /// The away side gets the identical treatment, independently of the
    /// home side -- `NewMatchOptions.away_tactic` was already reachable from
    /// `gc_sim::r#match::new` itself; only this wasm binding had no
    /// parameter for it.
    #[test]
    fn a_supplied_away_tactic_id_reaches_the_match_s_press_state_independently() {
        let session = new_session(None, None, Some("press_high".to_string()), None)
            .expect("press_high is authored content");
        assert_eq!(session.with_entry(|entry| entry.state.press.away), 2);
        // The home side is untouched -- still balanced's `1`.
        assert_eq!(session.with_entry(|entry| entry.state.press.home), 1);
    }

    /// An id naming no authored tactic is a typed `Err`, never a panic --
    /// `gc_data::tactics::get` returning `None` must not reach
    /// `sim_match::new` at all. Exercises [`resolve_tactic`] directly rather
    /// than `Session::new` itself: `Session::new`'s own error path
    /// constructs a `wasm_bindgen::JsValue`, which aborts the whole native
    /// test process off the `wasm32` target (see [`resolve_starter_roster`]'s
    /// doc) -- `packages/wasm/src/session.spec.ts` is where that `JsValue`
    /// error path is exercised end to end, against the real compiled
    /// artifact.
    #[test]
    fn an_unknown_tactic_id_is_a_typed_error_not_a_panic() {
        let err = resolve_tactic(Some("not_a_real_tactic"), "tactic")
            .expect_err("an unauthored tactic id must be rejected");
        assert!(err.contains("not_a_real_tactic"));
    }

    /// The opt-in itself: a caller-supplied starting XI (here, Nebula's
    /// squad-only forward `mika_olu` swapped in for the authored roster's
    /// `zyro_vex`) actually reaches the simulated match's own player list --
    /// not merely accepted and discarded.
    #[test]
    fn a_supplied_starting_xi_reaches_the_match_s_player_roster() {
        let starters = vec![
            "ozzo".to_string(),
            "brakka".to_string(),
            "veil_nyx".to_string(),
            "rok_tann".to_string(),
            "mika_olu".to_string(),
        ];
        let session = new_session(None, None, None, Some(starters))
            .expect("a valid five-a-side starting XI with the keeper first");
        let ids: Vec<String> =
            session.with_entry(|entry| entry.state.players.iter().map(|p| p.id.clone()).collect());
        assert!(ids.contains(&"mika_olu".to_string()));
        assert!(!ids.contains(&"zyro_vex".to_string()));
        // The keeper is still present -- only the outfield swap happened.
        assert!(ids.contains(&"ozzo".to_string()));
    }

    /// `None`/omitted keeps the authored roster exactly -- the prior, only
    /// behavior, unchanged by this parameter's addition.
    #[test]
    fn an_omitted_starting_xi_keeps_the_authored_roster() {
        let session = new_session(None, None, None, None).expect("authored roster always valid");
        let ids: Vec<String> =
            session.with_entry(|entry| entry.state.players.iter().map(|p| p.id.clone()).collect());
        assert!(ids.contains(&"zyro_vex".to_string()));
    }

    /// Every one of `resolve_starter_roster`'s shape checks must reject
    /// before ever reaching `sim_match::new` -- a wasm panic downstream
    /// (from `sim_match::new`'s or `input_frame::validate_ownership`'s own
    /// `assert!`/`.expect(...)`) is exactly what this batch of checks
    /// exists to prevent, per this crate's own "typed error, never panic"
    /// rule for JS-originated input. Exercises [`resolve_starter_roster`]
    /// directly rather than `Session::new` itself: `Session::new`'s own
    /// error path constructs a `wasm_bindgen::JsValue`, which aborts the
    /// whole native test process off the `wasm32` target (see that
    /// function's own doc).
    #[test]
    fn a_malformed_starting_xi_is_a_typed_error_not_a_panic() {
        let base = ["ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex"];
        let away_roster: [&str; 5] = ["gax_oru", "drell", "morv", "krag", "tox_vren"];

        // Wrong length.
        resolve_starter_roster(&["ozzo".to_string(), "brakka".to_string()], &away_roster)
            .expect_err("four short of a starting XI must be rejected");

        // Unknown player id.
        let mut unknown = base.map(str::to_string).to_vec();
        unknown[4] = "not_a_real_player".to_string();
        resolve_starter_roster(&unknown, &away_roster)
            .expect_err("an unauthored player id must be rejected");

        // Keeper not at index 0.
        let mut swapped = base.map(str::to_string).to_vec();
        swapped.swap(0, 1);
        resolve_starter_roster(&swapped, &away_roster)
            .expect_err("a non-keeper first id must be rejected");

        // A second keeper among the outfielders.
        let mut two_keepers = base.map(str::to_string).to_vec();
        two_keepers[1] = "ozzo".to_string();
        resolve_starter_roster(&two_keepers, &away_roster)
            .expect_err("a duplicate id (and a second keeper) must be rejected");

        // Duplicate, non-keeper id.
        let mut duplicate = base.map(str::to_string).to_vec();
        duplicate[4] = duplicate[3].clone();
        resolve_starter_roster(&duplicate, &away_roster)
            .expect_err("a duplicate outfield id must be rejected");

        // Shares an id with the (fixed) away roster.
        let mut clashes_with_away = base.map(str::to_string).to_vec();
        clashes_with_away[4] = "gax_oru".to_string(); // orion's keeper
        resolve_starter_roster(&clashes_with_away, &away_roster)
            .expect_err("an id already on the away roster must be rejected");
    }
}
