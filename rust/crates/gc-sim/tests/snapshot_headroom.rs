//! Measures a real rollback session's retained storage against the authored
//! `Omp2RollbackBudgets`, in the two-band shape
//! [#209](https://github.com/osobytes/goliseo/issues/209) asked for.
//!
//! ## Why the measured side cannot collapse into the asserted side
//!
//! [#470](https://github.com/osobytes/goliseo/issues/470) is about a check
//! that compared authored constants to their own literals. Nothing here can
//! degenerate into that, for three independent reasons:
//!
//! 1. **The two sides share no code.** The asserted side is a literal in
//!    `gc_data::omp2_rollback_validation::DATA.budgets`; `gc-data` depends
//!    on nothing in this workspace — no `gc-sim`, no encoder (its only
//!    dependencies are `serde`/`serde_json`). The measured side is produced
//!    by building a real `MatchState` from authored teams, running real
//!    `rollback_session::step` ticks until the retention ring is full, and
//!    reading `rollback_snapshot_history`/`rollback_input_history`/
//!    `rollback_events`' own byte accounting — which counts the exact
//!    canonical wire bytes of each retained snapshot via
//!    `match_snapshot::encoded_size_canonical`. For the measurement to equal
//!    the budget by construction, the simulation's canonical encoder would
//!    have to start reading a `gc-data` budget table, which it does not and
//!    cannot without an obvious diff.
//!
//! 2. **Adding a simulation field moves the measured side and not the
//!    asserted side.** That is the whole point: this test exists to notice
//!    that drift. `assert_eq!(budget, budget)` cannot, by construction.
//!
//! 3. **A measurement that stops measuring fails.** `measured <= budget`
//!    passes trivially at zero, so every reading carries a floor as well as
//!    a budget, and `snapshot_headroom::enforce` treats a
//!    below-floor measurement as a failure. See that module's doc comment.
//!
//! ## What is measured, and what is deliberately not
//!
//! Measured against the budget: `snapshot_count`, `snapshot_bytes`,
//! `history_bytes`. Not measured: `p95_work_ms`, `rollback_p999_ms` (a
//! per-PR wall-clock percentile on a shared runner measures the runner) and
//! `memory_growth_ratio` (a soak quantity; a seconds-long proxy for it
//! would be exactly the fabricated measurement this issue is about — see
//! [#472](https://github.com/osobytes/goliseo/issues/472), which owns
//! long-duration soak evidence). Those three stay authored-but-unmeasured,
//! and that is stated rather than papered over.
//!
//! ## Relationship to the other budget users
//!
//! `tests/combat_load_fixtures.rs` compares the four pinned combat load
//! campaigns' lab peaks to the same budgets, and
//! `tests/match_snapshot.rs` prices a synthetic `bytes * 31` window. Both
//! are single-band (fail only) and both are tied to their own fixtures.
//! This file is the cheap, general, two-band reading: one real session,
//! stepped past a full ring, in well under a second.
//!
//! `history_bytes` here means what it means in `rollback_lab` and in
//! `tests/combat_load_fixtures.rs` — session accounting **plus** the
//! retained speculative event timeline. See `measure`'s doc comment for why
//! the narrower session-only figure would be the wrong thing to gate on.
//!
//! The snapshot reading sits **above** `tests/match_snapshot.rs`'s
//! synthetic `(base + delta) * 31` window (831,694 measured here against
//! 767,095 synthetic), and that ordering is expected rather than a
//! contradiction: the synthetic figure multiplies a single tick-zero
//! boundary, where most per-player state is still at its construction
//! default, while a ring filled by contested play retains 31 boundaries of
//! populated decision, press and combat state. The two are not independent
//! instruments in any case — same `nebula`/`orion` fixture, same
//! construction path, same `encoded_size_canonical` encoder — so this is a
//! statement about which state each one prices, not a cross-validation.

use gc_data::omp2_rollback_validation;
use gc_data::teams;
use gc_sim::combat;
use gc_sim::input_frame::{self, EdgeAction, HeldAction, InputSample, InputSampleOptions};
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{self, MatchState, PitchSize};
use gc_sim::rollback_events;
use gc_sim::rollback_input_history::RollbackInputSource;
use gc_sim::rollback_session;
use gc_sim::rollback_snapshot_history;
use gc_sim::snapshot_headroom::{self, SnapshotHeadroomBand};

/// How close to a budget counts as "approaching" it. 32 KiB is the margin
/// #209 settled on for the retained-snapshot window: roughly four
/// team-level field additions at the ~8.6 KiB-per-field cost priced in
/// `docs/online/omp2_rollback_validation.md`, so the warning arrives with
/// enough room left to decide what to spend it on.
const WARN_MARGIN: i64 = 32 * 1024;

/// Ticks stepped before reading the accounting: enough to fill the
/// 31-boundary ring and evict past it, so the reading is a steady-state
/// retained window rather than a partially-warmed one.
const STEPPED_TICKS: i64 = 48;

/// Scripted input for every slot on every tick.
///
/// **This must generate real events, and that is not incidental.** The
/// obvious fixture — `input_frame::neutral_sample()` on every slot — plays
/// out as ten players standing still, and measurably produces *zero*
/// `MatchEvent`s and *zero* `CombatEvent`s across the whole run. A retained
/// event window built from it is 30 steps of pure wrapper scaffolding
/// (`tick`, boundaries, `state`, and three empty arrays): a nonzero byte
/// count that prices none of the per-event encoders. A new field on
/// `MatchEvent`, `CombatEvent` or a lifecycle payload would move that
/// number by exactly nothing — a budget-compared figure measuring none of
/// the thing it names, which is the defect this whole file exists to
/// prevent.
///
/// So instead: both sides converge on the ball at a sprint, hold the
/// equipment action (the combat trigger) throughout, press a combat request
/// every sixth tick and dash on the third. The vertical split by slot
/// parity keeps the ten players from collapsing onto one point. That
/// produces real contact and real event traffic, so `wrapped_event_len`,
/// `match_event_len` and `combat_event_len` are genuinely entered and
/// priced.
fn scripted_sample(tick: i64, slot_index: i64) -> InputSample {
    let toward_opponent = if slot_index <= input_frame::HOME_SLOT_COUNT {
        1
    } else {
        -1
    };
    let held = HeldAction::Equipment.bit() | HeldAction::Sprint.bit();
    let edges = if tick % 6 == 0 {
        EdgeAction::EquipmentPressed.bit()
    } else if tick % 6 == 3 {
        EdgeAction::Dash.bit()
    } else {
        0
    };
    input_frame::new_sample(InputSampleOptions {
        move_x: Some(toward_opponent * 100),
        move_y: Some(if slot_index % 2 == 0 { 40 } else { -40 }),
        held: Some(held),
        edges: Some(edges),
    })
    .expect("the scripted sample is a valid input sample")
}

fn new_state() -> MatchState {
    let home = teams::get("nebula").expect("nebula is authored");
    let away = teams::get("orion").expect("orion is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        // Long enough that `STEPPED_TICKS` cannot reach full time: a
        // finished match stops stepping and would under-fill the ring.
        duration: Some(120.0),
        max_goals: Some(9),
        seed: Some(19.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(sim_match::ownership_for_teams(home, away, None)),
    })
}

fn sources() -> [RollbackInputSource; 8] {
    let mut sources = [RollbackInputSource::Remote; 8];
    sources[0] = RollbackInputSource::Local;
    sources
}

/// One measured retained window.
struct Measurement {
    retained_boundaries: i64,
    peak_boundaries: i64,
    peak_snapshot_bytes: i64,
    boundary_bytes: i64,
    /// Peak of `session accounting total + retained event-timeline bytes`,
    /// which is what `budgets.history_bytes` means everywhere else in this
    /// tree (see `measure`'s comment on the event timeline).
    peak_history_bytes: i64,
    /// The event timeline's own contribution at the same peak.
    peak_event_bytes: i64,
    /// Retained *events* at the same peak — not steps. A window of 30 steps
    /// holding no events at all is what the first revision of this file
    /// measured; this is the field that catches it.
    peak_event_count: i64,
    retained_event_steps: i64,
    /// Every event the simulation produced across the whole run, summed as
    /// it was produced rather than read back off the timeline.
    raw_events_produced: i64,
}

/// Translate a session tick output into the event timeline's own input row.
/// `rollback_session` and `rollback_events` each declare their own
/// `RollbackOutputStateView`, so the fields are copied across rather than
/// passed through — the same thing `rollback_lab::event_tick_output` does.
fn event_tick_output(
    output: &rollback_session::RollbackTickOutput,
) -> rollback_events::RollbackEventTickOutput {
    rollback_events::RollbackEventTickOutput {
        tick: output.tick,
        start_boundary: output.start_boundary,
        end_boundary: output.end_boundary,
        events: output.events.clone(),
        combat_events: output.combat_events.clone(),
        state: rollback_events::RollbackOutputStateView {
            score: output.state.score,
            time_left: output.state.time_left,
            finished: output.state.finished,
        },
        finished: output.finished,
    }
}

/// Build a real session — combat companion included when `combat` is set,
/// because a combat-active match is the expensive case the budget is sized
/// for — step it past a full ring, and read the retained accounting.
///
/// ## Why an event timeline is driven alongside the session
///
/// `budgets.history_bytes` bounds the *whole* retained client footprint.
/// `rollback_session::accounting().total_bytes` is only input + output +
/// snapshots; it structurally omits the retained speculative event
/// timeline. `rollback_lab` has always defined its `history_bytes` peak as
/// `accounting.total_bytes + rollback_events::accounting().total_bytes`,
/// and `tests/combat_load_fixtures.rs` compares *that* combined figure to
/// this same budget. Gating on the narrower quantity would leave retained
/// event encoding — a new `CombatEvent`/`MatchEvent` field, a wider
/// retention window — growing with nothing complaining, which is the exact
/// shape of the defect this file exists to fix. And the timeline is not a
/// lab-only artifact: `gc-wasm`'s `rollback_events_bridge` exposes it to
/// the browser, so it is real retained client memory.
///
/// The confirmation schedule below holds the speculative window at its
/// maximum (`max_unconfirmed_ticks`) rather than confirming eagerly.
/// Confirming as fast as possible would drain the timeline to near-nothing
/// and reduce its contribution to noise — an under-measurement dressed up
/// as headroom. A client whose confirmations are lagging by the full window
/// is both reachable and the case the budget has to bound.
///
/// Honest limit on what the event side covers. The scripted fixture
/// retains 41 real events across its 30-step window at the measured peak —
/// 40 `CombatEvent`s and one `MatchEvent`, from 66 produced across the run
/// — so the wrapped-event, match-event and combat-event encoders are all
/// entered and priced, and a field added to either row moves this number.
/// Two gaps remain, and neither is papered over: **lifecycle payloads are
/// not exercised** (this fixture scores no goal and reaches no full time
/// inside 48 ticks, so `lifecycle_events` stays empty; an armed-shot variant
/// was tried and produced *fewer* events, not more, so closing this gap means
/// a longer or differently-shaped fixture, which would cost the cheapness
/// that is this test's whole point), and 41 events over
/// 30 ticks is ordinary contested play, not the adversarial 51-rows-per-tick
/// case `docs/online/omp2_rollback_validation.md` prices. Worst-case event
/// load is what `tests/combat_load_fixtures.rs` exercises against the same
/// budget. This reading is the cheap early warning that per-event encoding
/// growth is visible at all.
fn measure(combat_active: bool) -> Measurement {
    let mut state = new_state();
    let combat_state = if combat_active {
        Some(combat::new_state(&mut state, None))
    } else {
        None
    };
    let boundary_zero = match_snapshot::capture_owned(&state, combat_state.as_ref());
    let boundary_bytes = match_snapshot::encoded_size_canonical(&boundary_zero) as i64;
    let mut session = rollback_session::new(&boundary_zero, sources(), None, None);
    let mut timeline = rollback_events::new(&boundary_zero, None);
    let max_unconfirmed = timeline.max_unconfirmed_ticks;

    let mut peak_history_bytes = 0;
    let mut peak_event_bytes = 0;
    let mut peak_event_count = 0;
    let mut raw_events_produced = 0;
    for tick in 0..STEPPED_TICKS {
        // Real authoritative input for every slot, so the input history is
        // genuinely populated rather than left as bare predictions — and
        // scripted rather than neutral, so the run produces real events
        // (see `scripted_sample`).
        for slot_index in 1..=input_frame::SLOT_COUNT {
            rollback_session::add_authoritative(
                &mut session,
                tick,
                slot_index,
                scripted_sample(tick, slot_index),
            )
            .expect("an in-window authoritative row is accepted");
        }
        let output =
            rollback_session::step(&mut session).expect("the session steps inside its duration");
        raw_events_produced += output.events.len() as i64
            + output
                .combat_events
                .as_ref()
                .map_or(0, |events| events.len() as i64);

        // Confirm exactly late enough to keep the speculative window full:
        // after applying tick `t`, the timeline retains `t - confirmed`
        // steps, so confirming through `t - max_unconfirmed` pins that at
        // `max_unconfirmed`.
        if tick >= max_unconfirmed {
            rollback_events::confirm(&mut timeline, tick - max_unconfirmed);
        }
        let lookup = rollback_session::snapshot(&session, output.end_boundary);
        let step_input = rollback_events::RollbackEventStepInput {
            output: event_tick_output(&output),
            snapshot: lookup
                .snapshot
                .expect("the just-produced end boundary is retained"),
        };
        rollback_events::apply(&mut timeline, tick, tick, &[step_input])
            .expect("a contiguous speculative step inside the unconfirmed window is accepted");

        let accounting = rollback_session::accounting(&mut session);
        let event_bytes = rollback_events::accounting(&timeline).total_bytes;
        let combined = accounting.total_bytes + event_bytes;
        if combined > peak_history_bytes {
            peak_history_bytes = combined;
            peak_event_bytes = event_bytes;
            // Sampled at the peak, not at end of run: the peak is the tick
            // whose number is compared to the budget, so it is the tick
            // whose event content has to be non-empty.
            peak_event_count = rollback_events::diagnostics(&timeline).retained_event_count;
        }
    }

    let diagnostics = rollback_snapshot_history::diagnostics(&session.snapshot_history);
    let accounting = rollback_session::accounting(&mut session);
    assert_eq!(
        accounting.snapshot_bytes, diagnostics.canonical_bytes,
        "the session and the snapshot ring must agree on retained snapshot bytes"
    );
    let event_diagnostics = rollback_events::diagnostics(&timeline);
    assert_eq!(
        event_diagnostics.retained_step_count, max_unconfirmed,
        "the speculative event window must end full, or its contribution is an under-measurement"
    );
    Measurement {
        retained_boundaries: diagnostics.retained_boundary_count,
        peak_boundaries: diagnostics.peak_retained_boundary_count,
        peak_snapshot_bytes: diagnostics.peak_canonical_bytes,
        boundary_bytes,
        peak_history_bytes,
        peak_event_bytes,
        peak_event_count,
        retained_event_steps: event_diagnostics.retained_step_count,
        raw_events_produced,
    }
}

/// The floor for a full ring's retained snapshot bytes: half of one real
/// boundary's exact canonical size times the retained boundary count.
///
/// Half, because the point of the floor is to catch a ring that stopped
/// retaining, not to re-assert the measurement to two significant figures —
/// individual boundaries legitimately differ in size as the match state
/// evolves, and a tight floor would turn ordinary content edits into
/// spurious failures. Half a full ring is nowhere near reachable by a
/// retaining ring and nowhere near avoidable by an empty or barely-warmed
/// one. It is derived from *this run's* measured boundary rather than a
/// pinned literal, so it tracks the encoder instead of going stale.
fn snapshot_bytes_floor(measurement: &Measurement) -> i64 {
    measurement.boundary_bytes * measurement.retained_boundaries / 2
}

fn report(reading: &gc_sim::snapshot_headroom::SnapshotHeadroomReading) {
    // Visible under `cargo test -- --nocapture`, and carried in the panic
    // message on the failing bands regardless.
    println!("{}", snapshot_headroom::marker(reading));
}

#[test]
fn a_real_combat_session_retains_a_window_inside_the_authored_snapshot_and_history_budgets() {
    let budgets = omp2_rollback_validation::DATA.budgets;
    let measurement = measure(true);

    // `snapshot_count` is the one budget that is an equality rather than a
    // ceiling, so it gets a plain assertion instead of a band: a ring that
    // retained *fewer* boundaries than the budget never filled, which would
    // make the byte readings below an under-measurement rather than a
    // comfortable result.
    assert_eq!(
        measurement.retained_boundaries, budgets.snapshot_count,
        "a session stepped past {STEPPED_TICKS} ticks must retain a full ring"
    );
    assert_eq!(
        measurement.peak_boundaries, budgets.snapshot_count,
        "the retained-boundary high-water mark must equal the authored budget"
    );

    let snapshot = snapshot_headroom::read(
        "retained_snapshot_bytes",
        measurement.peak_snapshot_bytes,
        snapshot_bytes_floor(&measurement),
        budgets.snapshot_bytes,
        WARN_MARGIN,
    );
    report(&snapshot);
    snapshot_headroom::enforce(&snapshot);

    // The same combined definition `rollback_lab` uses and
    // `tests/combat_load_fixtures.rs` compares: session accounting (input +
    // output + snapshots) plus the retained speculative event timeline.
    let history = snapshot_headroom::read(
        "retained_history_bytes",
        measurement.peak_history_bytes,
        // History retention includes the snapshot ring, so it can never be
        // smaller than the snapshot floor.
        snapshot_bytes_floor(&measurement),
        budgets.history_bytes,
        WARN_MARGIN,
    );
    report(&history);
    snapshot_headroom::enforce(&history);
    println!(
        "GC_SNAPSHOT_HEADROOM|retained_history_components|total={}|event_timeline={}\
         |event_steps={}|events_at_peak={}|events_produced={}",
        measurement.peak_history_bytes,
        measurement.peak_event_bytes,
        measurement.retained_event_steps,
        measurement.peak_event_count,
        measurement.raw_events_produced
    );

    assert!(
        measurement.peak_history_bytes > measurement.peak_snapshot_bytes,
        "retained history must strictly exceed its snapshot component"
    );
    // The event component must be present *and* non-empty. Occupancy alone
    // is not enough: 30 retained steps holding zero events is 30 wrappers
    // of metadata, a nonzero byte count that prices none of the per-event
    // encoders and would not move if `MatchEvent` or `CombatEvent` grew a
    // field. Asserting at the peak — the tick whose number is compared to
    // the budget — rather than at end of run is the point.
    assert_eq!(
        measurement.retained_event_steps, 30,
        "the speculative event window must be held full"
    );
    assert!(
        measurement.peak_event_count > 0,
        "the retained event window must hold real events at the measured peak, \
         not just {} empty step wrappers -- otherwise the history reading prices \
         none of the per-event encoders",
        measurement.retained_event_steps
    );
    assert!(
        measurement.raw_events_produced > 0,
        "the fixture must actually generate events"
    );
    assert!(
        measurement.peak_event_bytes > 0,
        "the event timeline must contribute retained bytes to the history total"
    );
}

#[test]
fn a_soccer_only_session_costs_less_than_its_combat_active_twin_and_stays_inside_budget() {
    let budgets = omp2_rollback_validation::DATA.budgets;
    let soccer = measure(false);
    let combat = measure(true);

    // The pair differs only by the combat companion, so the difference is
    // attributable rather than merely asserted.
    assert!(
        soccer.boundary_bytes < combat.boundary_bytes,
        "a combat companion must cost retained bytes: soccer {} vs combat {}",
        soccer.boundary_bytes,
        combat.boundary_bytes
    );
    assert!(
        soccer.peak_snapshot_bytes < combat.peak_snapshot_bytes,
        "the retained window must inherit that cost"
    );

    let reading = snapshot_headroom::read(
        "retained_snapshot_bytes_soccer_only",
        soccer.peak_snapshot_bytes,
        snapshot_bytes_floor(&soccer),
        budgets.snapshot_bytes,
        WARN_MARGIN,
    );
    report(&reading);
    snapshot_headroom::enforce(&reading);
}

// ---------------------------------------------------------------------
// The demonstration that this gate can go red.
//
// Each case drives the *same* `read`/`enforce` path the measurements above
// use, with a substituted figure. AGENTS.md §9: a gate with no
// demonstration that it can fail is not a gate.
// ---------------------------------------------------------------------

fn catch_panic<T>(f: impl FnOnce() -> T + std::panic::UnwindSafe) -> std::thread::Result<T> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(f);
    std::panic::set_hook(previous);
    result
}

#[test]
fn an_oversized_measurement_is_rejected_by_the_same_comparison_the_real_one_passes() {
    let budgets = omp2_rollback_validation::DATA.budgets;
    let measurement = measure(true);
    let floor = snapshot_bytes_floor(&measurement);

    // One byte over is enough: the failing band is the budget itself, not
    // some looser derived bound.
    let over = snapshot_headroom::read(
        "retained_snapshot_bytes",
        budgets.snapshot_bytes + 1,
        floor,
        budgets.snapshot_bytes,
        WARN_MARGIN,
    );
    assert_eq!(over.band, SnapshotHeadroomBand::Exceeded);
    assert_eq!(over.headroom, -1);
    let panicked = catch_panic(move || snapshot_headroom::enforce(&over));
    assert!(
        panicked.is_err(),
        "an over-budget reading must fail the gate, not merely warn"
    );

    // Exactly on the budget passes, so the failing band is not off by one.
    let exact = snapshot_headroom::read(
        "retained_snapshot_bytes",
        budgets.snapshot_bytes,
        floor,
        budgets.snapshot_bytes,
        WARN_MARGIN,
    );
    assert_eq!(exact.band, SnapshotHeadroomBand::Approaching);
    snapshot_headroom::enforce(&exact);
}

#[test]
fn a_measurement_that_stopped_measuring_is_rejected_rather_than_passing_as_comfortable() {
    let budgets = omp2_rollback_validation::DATA.budgets;
    let measurement = measure(true);
    let floor = snapshot_bytes_floor(&measurement);
    assert!(floor > 0, "the floor must be a real bound");

    // An empty ring is far under the budget. Without the floor this would
    // read as the most comfortable possible result.
    let collapsed = snapshot_headroom::read(
        "retained_snapshot_bytes",
        0,
        floor,
        budgets.snapshot_bytes,
        WARN_MARGIN,
    );
    assert_eq!(collapsed.band, SnapshotHeadroomBand::Collapsed);
    let panicked = catch_panic(move || snapshot_headroom::enforce(&collapsed));
    assert!(
        panicked.is_err(),
        "a collapsed measurement must fail the gate"
    );
}

#[test]
fn the_warning_band_fires_before_the_budget_does_and_does_not_fail_the_gate() {
    let budgets = omp2_rollback_validation::DATA.budgets;

    let approaching = snapshot_headroom::read(
        "retained_snapshot_bytes",
        budgets.snapshot_bytes - WARN_MARGIN + 1,
        1,
        budgets.snapshot_bytes,
        WARN_MARGIN,
    );
    assert_eq!(approaching.band, SnapshotHeadroomBand::Approaching);
    assert!(!approaching.band.is_failure());
    let marker = snapshot_headroom::enforce(&approaching);
    assert!(marker.contains("band=approaching"));

    // Exactly `warn_margin` of remaining headroom is the last warning, and
    // one byte further from the budget is not, so the two bands meet
    // exactly where the margin says they do.
    assert_eq!(
        snapshot_headroom::read(
            "retained_snapshot_bytes",
            budgets.snapshot_bytes - WARN_MARGIN,
            1,
            budgets.snapshot_bytes,
            WARN_MARGIN,
        )
        .band,
        SnapshotHeadroomBand::Approaching
    );
    let within = snapshot_headroom::read(
        "retained_snapshot_bytes",
        budgets.snapshot_bytes - WARN_MARGIN - 1,
        1,
        budgets.snapshot_bytes,
        WARN_MARGIN,
    );
    assert_eq!(within.band, SnapshotHeadroomBand::Within);
    assert!(snapshot_headroom::enforce(&within).contains("band=within"));
}
