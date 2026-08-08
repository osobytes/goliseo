//! Balance presets, pickable from the F1 tuning panel (F4 cycles through
//! them). Blobs use the `sim::tuning` serialize format; an empty blob means
//! pure defaults. Candidates come from the simulation-based balance search —
//! provenance and held-out evidence in docs/design/fun_metrics.md.

/// A named tuning preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TuningPreset {
    /// Persistent identity.
    pub id: &'static str,
    /// Shown in the panel status line.
    pub name: &'static str,
    /// Serialized tuning overrides, in `sim::tuning` format.
    pub blob: &'static str,
}

/// Every authored tuning preset, in panel cycle order.
pub static ALL: &[TuningPreset] = &[
    TuningPreset {
        id: "defaults",
        name: "Defaults",
        blob: "",
    },
    TuningPreset {
        id: "candidate_a",
        name: "Candidate A - direct play",
        // Candidate A: +0.47 fun vs defaults on held-out seeds. Direct play —
        // AI shoots/heads from range, presses less, keepers a touch softer.
        // Matches run hot (~4 goals). They used to hit the 3-goal cap early;
        // since #268 there is no cap and a hot match runs its full 120 seconds.
        //
        // The Lua source builds this blob via `table.concat(..., "\n")`; that
        // is the one place data/tuning_presets.lua computes rather than
        // authors a value (see AGENTS.md §8). The joined literal below is
        // byte-identical to what that call produces.
        blob: "AI_SHOOT_RANGE=340\nAI_HEADER_RANGE=300\nAI_PASS_PRESSURE=75\nSAVE_SPEED_REF=700\nAI_STEAL_CD=1.5\nCARRIER_SETTLE=0.6",
    },
    TuningPreset {
        id: "candidate_b",
        // Candidate B: the one-knob sweet spot (+0.39 fun held-out) — ~80% of
        // A's gain with minimal behavior change. The low-risk option.
        name: "Candidate B - sharper AI shots",
        blob: "AI_SHOOT_RANGE=300",
    },
];

/// Look up a tuning preset by id.
pub fn get(id: &str) -> Option<&'static TuningPreset> {
    ALL.iter().find(|preset| preset.id == id)
}
