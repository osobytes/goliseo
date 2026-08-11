//! Two-band classification of a *measured* retained-storage figure against
//! an *authored* budget from [`gc_data::omp2_rollback_validation`].
//!
//! This module deliberately holds no numbers of its own. It takes a
//! measurement someone else produced and a budget someone else authored,
//! and decides which band the pair lands in. Keeping the decision here —
//! rather than open-coded at each call site — is what lets the same
//! comparison be driven twice: once by a real measurement (which must land
//! in [`SnapshotHeadroomBand::Within`]) and once by a deliberately
//! oversized figure (which must land in [`SnapshotHeadroomBand::Exceeded`]
//! and make [`enforce`] panic). A gate with no demonstration that it can go
//! red is not a gate (AGENTS.md §9).
//!
//! ## Why there are four bands, not two
//!
//! `Within` / `Approaching` / `Exceeded` is the two-band shape #209 asked
//! for: fail on the budget, warn on approach, so the margin is reported
//! while it can still be spent deliberately instead of only at zero.
//!
//! [`SnapshotHeadroomBand::Collapsed`] is the fourth, and it exists because
//! of how this kind of check fails silently. `measured <= budget` is
//! trivially satisfied by a measurement that has quietly stopped measuring
//! — a fixture that no longer fills its retention ring, an accounting call
//! moved before the work it was meant to account for, a refactor that
//! returns an empty history. Such a check stays green forever while
//! covering nothing, which is exactly the failure mode
//! [#470](https://github.com/osobytes/goliseo/issues/470) is about. So
//! every reading carries a `floor` as well as a budget, and a measurement
//! below its floor is a failure, not a pass.
//!
//! ## Unmeasured budget fields
//!
//! [`gc_data::omp2_rollback_validation::Omp2RollbackBudgets`] has six
//! fields. This module's callers measure the three retained-storage ones —
//! `snapshot_count`, `snapshot_bytes`, `history_bytes`. The other three are
//! **not** measured here, deliberately:
//!
//! - `p95_work_ms` and `rollback_p999_ms` are wall-clock percentiles. A
//!   per-PR timing assertion on a shared CI runner measures the runner, not
//!   the code.
//! - `memory_growth_ratio` is a soak quantity: it only means anything over
//!   a long run. Deriving it from a seconds-long test would be a fabricated
//!   proxy — the same defect as an assertion that compares a constant to
//!   itself. Long-duration soak evidence is owned by
//!   [#472](https://github.com/osobytes/goliseo/issues/472).
//!
//! Those three budgets therefore remain authored-but-unmeasured today, and
//! saying so plainly is the honest outcome.

use std::fmt::Write as _;

/// Where a measurement sits relative to its budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotHeadroomBand {
    /// Below the reading's floor: the measurement is no longer measuring
    /// what it claims to, and passing would be meaningless.
    Collapsed,
    /// Inside the budget with more than `warn_margin` bytes to spare.
    Within,
    /// Inside the budget, but within `warn_margin` of it.
    Approaching,
    /// Over the budget.
    Exceeded,
}

impl SnapshotHeadroomBand {
    /// This band's stable wire word, used in diagnostic markers.
    #[must_use]
    pub fn wire_str(self) -> &'static str {
        match self {
            SnapshotHeadroomBand::Collapsed => "collapsed",
            SnapshotHeadroomBand::Within => "within",
            SnapshotHeadroomBand::Approaching => "approaching",
            SnapshotHeadroomBand::Exceeded => "exceeded",
        }
    }

    /// Whether this band must fail the gate.
    #[must_use]
    pub fn is_failure(self) -> bool {
        matches!(
            self,
            SnapshotHeadroomBand::Collapsed | SnapshotHeadroomBand::Exceeded
        )
    }
}

/// One measured quantity classified against one authored budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotHeadroomReading {
    /// What was measured, for diagnostics.
    pub label: &'static str,
    /// The measured value.
    pub measured: i64,
    /// The lowest value this measurement can legitimately produce. Below
    /// this the measurement is broken, not comfortable (see the module doc
    /// comment).
    pub floor: i64,
    /// The authored budget.
    pub budget: i64,
    /// How close to `budget` counts as approaching it.
    pub warn_margin: i64,
    /// `budget - measured`. Negative exactly when the budget is exceeded.
    pub headroom: i64,
    /// The band `measured` lands in.
    pub band: SnapshotHeadroomBand,
}

/// Classify one measurement against one budget.
///
/// # Panics
///
/// Panics if the reading is incoherent as authored — a negative
/// `warn_margin`, a non-positive `budget`, or a `floor` at or above the
/// `budget`, all of which would make the classification meaningless
/// (ARCHITECTURE.md §3 rule 5: a programmer error, not a recoverable one).
#[must_use]
pub fn read(
    label: &'static str,
    measured: i64,
    floor: i64,
    budget: i64,
    warn_margin: i64,
) -> SnapshotHeadroomReading {
    assert!(budget > 0, "{label}: budget must be positive");
    assert!(
        warn_margin >= 0,
        "{label}: warn margin must be non-negative"
    );
    assert!(
        floor >= 0 && floor < budget,
        "{label}: floor must sit below the budget"
    );
    let headroom = budget - measured;
    let band = if measured < floor {
        SnapshotHeadroomBand::Collapsed
    } else if measured > budget {
        SnapshotHeadroomBand::Exceeded
    } else if headroom <= warn_margin {
        SnapshotHeadroomBand::Approaching
    } else {
        SnapshotHeadroomBand::Within
    };
    SnapshotHeadroomReading {
        label,
        measured,
        floor,
        budget,
        warn_margin,
        headroom,
        band,
    }
}

/// This reading's stable one-line diagnostic marker.
#[must_use]
pub fn marker(reading: &SnapshotHeadroomReading) -> String {
    let mut out = String::from("GC_SNAPSHOT_HEADROOM|");
    let _ = write!(
        out,
        "{}|band={}|measured={}|floor={}|budget={}|headroom={}|warn_margin={}",
        reading.label,
        reading.band.wire_str(),
        reading.measured,
        reading.floor,
        reading.budget,
        reading.headroom,
        reading.warn_margin
    );
    out
}

/// Enforce one reading, returning its marker so a caller can record it.
///
/// The failing bands panic; `Approaching` does not, which is the whole
/// point of the warning band. A caller that swallows the returned marker
/// still gets the failure, because the failure is a panic and not a printed
/// line — AGENTS.md §9's "a harness that prints failures and exits 0 must
/// fail the gate anyway", applied at the source.
///
/// # Panics
///
/// Panics when `reading.band` is [`SnapshotHeadroomBand::Exceeded`] (the
/// budget is blown) or [`SnapshotHeadroomBand::Collapsed`] (the measurement
/// stopped measuring).
pub fn enforce(reading: &SnapshotHeadroomReading) -> String {
    let marker = marker(reading);
    match reading.band {
        SnapshotHeadroomBand::Collapsed => panic!(
            "{}: measured {} is below the {} floor -- the measurement stopped measuring, \
             so staying under the {} budget proves nothing. {marker}",
            reading.label, reading.measured, reading.floor, reading.budget
        ),
        SnapshotHeadroomBand::Exceeded => panic!(
            "{}: measured {} exceeds the authored budget of {} by {}. \
             Price the addition against the retained window rather than raising the budget. \
             {marker}",
            reading.label, reading.measured, reading.budget, -reading.headroom
        ),
        SnapshotHeadroomBand::Approaching | SnapshotHeadroomBand::Within => marker,
    }
}
