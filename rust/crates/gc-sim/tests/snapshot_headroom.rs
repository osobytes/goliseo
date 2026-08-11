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
//!    `gc_data::omp2_rollback_validation::DATA.budgets`, a crate `gc-data`
//!    with no dependencies at all. The measured side is produced by
//!    building a real `MatchState` from authored teams, running real
//!    `rollback_session::step` ticks until the retention ring is full, and
//!    reading `rollback_snapshot_history`/`rollback_input_history`'s own
//!    byte accounting — which counts the exact canonical wire bytes of each
//!    retained snapshot via `match_snapshot::encoded_size_canonical`. For
//!    the measurement to equal the budget by construction, the simulation's
//!    canonical encoder would have to start reading a `gc-data` budget
//!    table, which it does not and cannot without an obvious diff.
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

use gc_data::omp2_rollback_validation;
use gc_data::teams;
use gc_sim::combat;
use gc_sim::input_frame;
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{self, MatchState, PitchSize};
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
    total_history_bytes: i64,
}

/// Build a real session — combat companion included when `combat` is set,
/// because a combat-active match is the expensive case the budget is sized
/// for — step it past a full ring, and read the retained accounting.
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

    for tick in 0..STEPPED_TICKS {
        // Real authoritative input for every slot, so the input history is
        // genuinely populated rather than left as bare predictions.
        for slot_index in 1..=input_frame::SLOT_COUNT {
            rollback_session::add_authoritative(
                &mut session,
                tick,
                slot_index,
                input_frame::neutral_sample(),
            )
            .expect("an in-window authoritative row is accepted");
        }
        rollback_session::step(&mut session).expect("the session steps inside its duration");
    }

    let diagnostics = rollback_snapshot_history::diagnostics(&session.snapshot_history);
    let accounting = rollback_session::accounting(&mut session);
    assert_eq!(
        accounting.snapshot_bytes, diagnostics.canonical_bytes,
        "the session and the snapshot ring must agree on retained snapshot bytes"
    );
    Measurement {
        retained_boundaries: diagnostics.retained_boundary_count,
        peak_boundaries: diagnostics.peak_retained_boundary_count,
        peak_snapshot_bytes: diagnostics.peak_canonical_bytes,
        boundary_bytes,
        total_history_bytes: accounting.total_bytes,
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

    let history = snapshot_headroom::read(
        "retained_history_bytes",
        measurement.total_history_bytes,
        // History retention is snapshots plus inputs plus outputs, so it
        // can never be smaller than the snapshot floor.
        snapshot_bytes_floor(&measurement),
        budgets.history_bytes,
        WARN_MARGIN,
    );
    report(&history);
    snapshot_headroom::enforce(&history);

    assert!(
        measurement.total_history_bytes > measurement.peak_snapshot_bytes,
        "retained history must strictly exceed its snapshot component"
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
