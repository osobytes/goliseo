//! Tier 2 of the tunable registry: presentation-only constants.
//!
//! ## Why tier 2 is authored here and not in `gc-data`
//!
//! Tier 2's promise is that changing any value in it cannot change the state
//! hash of a replayed match. `gc-sim` depends on `gc-data`, so a tier-2 table
//! authored there would be *reachable* from the simulation and the promise
//! would rest on a comment nobody can enforce. `gc-render` depends on `gc-sim`
//! — so a `gc-sim` module that tried to read this one would be a dependency
//! cycle, and `cargo` rejects it before any test runs. That is the structural
//! enforcement the issue asked for; `tests/presentation_tunables.rs` measures
//! the consequence (a replay's hash sequence is byte-identical with every
//! tier-2 value perturbed) rather than the promise.
//!
//! These are also the values that make tier 2 worth having: pose eases and the
//! landing reticle's window are exactly the things a designer wants to nudge
//! mid-session, and tier-2 isolation is what lets that happen without touching
//! rollback — no snapshot contains one.
//!
//! ## Not a second registry type
//!
//! The container is `gc_sim::tunable_registry::Registry`, the same one tier 1
//! and tier 3 use. Only the *values* are separate, and they are separate
//! because they live behind a dependency edge the simulation cannot cross.

use gc_data::tunables::{Tier, TunableDef};
use gc_sim::tunable_registry::{Registry, RegistryBuilder};
use std::sync::LazyLock;

/// Every authored tier-2 presentation constant.
///
/// Ids are prefixed `presentation.` so a stray tier-2 id in a sim blob is
/// visibly foreign rather than plausible.
pub static PRESENTATION_TUNABLES: &[TunableDef] = &[
    TunableDef {
        id: "presentation.dive_ease",
        tier: Tier::Presentation,
        label: "Dive relax s",
        cat: "Pose",
        default: 0.3,
        unit: "s",
        min: 0.05,
        max: 1.0,
        step: 0.05,
        desc: "How fast a keeper's dive pose relaxes back to neutral.",
    },
    TunableDef {
        id: "presentation.grab_ease",
        tier: Tier::Presentation,
        label: "Grab relax s",
        cat: "Pose",
        default: 0.25,
        unit: "s",
        min: 0.05,
        max: 1.0,
        step: 0.05,
        desc: "How fast a keeper's grab pose relaxes back to neutral.",
    },
    TunableDef {
        id: "presentation.throw_ease",
        tier: Tier::Presentation,
        label: "Throw relax s",
        cat: "Pose",
        default: 0.25,
        unit: "s",
        min: 0.05,
        max: 1.0,
        step: 0.05,
        desc: "How fast a keeper's throw pose relaxes back to neutral.",
    },
    TunableDef {
        id: "presentation.windup_ease",
        tier: Tier::Presentation,
        label: "Wind-up relax s",
        cat: "Pose",
        default: 0.15,
        unit: "s",
        min: 0.05,
        max: 1.0,
        step: 0.05,
        desc: "Normaliser for the strike wind-up pose's progress.",
    },
    TunableDef {
        id: "presentation.aerial_ease",
        tier: Tier::Presentation,
        label: "Aerial relax s",
        cat: "Pose",
        default: 0.22,
        unit: "s",
        min: 0.05,
        max: 1.0,
        step: 0.05,
        desc: "Default aerial-strike pose relax time.",
    },
    TunableDef {
        id: "presentation.aerial_ease_bicycle",
        tier: Tier::Presentation,
        label: "Bicycle relax s",
        cat: "Pose",
        default: 0.6,
        unit: "s",
        min: 0.05,
        max: 1.5,
        step: 0.05,
        desc: "Bicycle-kick pose relax time.",
    },
    TunableDef {
        id: "presentation.aerial_ease_jump",
        tier: Tier::Presentation,
        label: "Jump relax s",
        cat: "Pose",
        default: 0.35,
        unit: "s",
        min: 0.05,
        max: 1.5,
        step: 0.05,
        desc: "Jump-header pose relax time.",
    },
    TunableDef {
        id: "presentation.aerial_ease_control",
        tier: Tier::Presentation,
        label: "Aerial control relax s",
        cat: "Pose",
        default: 0.18,
        unit: "s",
        min: 0.05,
        max: 1.0,
        step: 0.05,
        desc: "Aerial-control pose relax time.",
    },
    TunableDef {
        id: "presentation.reticle_min_height",
        tier: Tier::Presentation,
        label: "Reticle min height",
        cat: "Reticle",
        default: 20.0,
        unit: "px",
        min: 0.0,
        max: 120.0,
        step: 5.0,
        desc: "Ball height below which no landing reticle is drawn.",
    },
    TunableDef {
        id: "presentation.reticle_min_time",
        tier: Tier::Presentation,
        label: "Reticle min time",
        cat: "Reticle",
        default: 0.05,
        unit: "s",
        min: 0.0,
        max: 1.0,
        step: 0.05,
        desc: "Shortest predicted fall a landing reticle is drawn for.",
    },
    TunableDef {
        id: "presentation.reticle_max_time",
        tier: Tier::Presentation,
        label: "Reticle max time",
        cat: "Reticle",
        default: 3.0,
        unit: "s",
        min: 0.5,
        max: 10.0,
        step: 0.5,
        desc: "Longest predicted fall a landing reticle is drawn for.",
    },
];

/// The shipped tier-2 registry, every value at its default.
///
/// Read-only `static` content, the same category as `gc_sim`'s
/// `tunable_registry::shipped` — see that module's doc on globals. A caller
/// that wants to vary a value clones this.
#[must_use]
pub fn shipped() -> &'static Registry {
    static SHIPPED: LazyLock<Registry> = LazyLock::new(assemble);
    &SHIPPED
}

/// Assemble a fresh tier-2 registry from this crate's authored content.
#[must_use]
pub fn assemble() -> Registry {
    let mut b = RegistryBuilder::new();
    b.register_tunables("gc-render", PRESENTATION_TUNABLES);
    b.build()
}
