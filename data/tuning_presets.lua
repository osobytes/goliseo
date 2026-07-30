-- Balance presets, pickable from the F1 tuning panel (F4 cycles through
-- them). Blobs use the sim/tuning.lua serialize format; an empty blob means
-- pure defaults. Candidates come from the simulation-based balance search —
-- provenance and held-out evidence in docs/design/fun_metrics.md.

---@class TuningPreset
---@field id string
---@field name string  -- shown in the panel status line
---@field blob string

---@type TuningPreset[]
return {
    {
        id = "defaults",
        name = "Defaults",
        blob = "",
    },
    {
        -- Candidate A: +0.47 fun vs defaults on held-out seeds. Direct play —
        -- AI shoots/heads from range, presses less, keepers a touch softer.
        -- Matches run hot (~4 goals). They used to hit the 3-goal cap early;
        -- since #268 there is no cap and a hot match runs its full 120 seconds.
        id = "candidate_a",
        name = "Candidate A - direct play",
        blob = table.concat({
            "AI_SHOOT_RANGE=340",
            "AI_HEADER_RANGE=300",
            "AI_PASS_PRESSURE=75",
            "SAVE_SPEED_REF=700",
            "AI_STEAL_CD=1.5",
            "CARRIER_SETTLE=0.6",
        }, "\n"),
    },
    {
        -- Candidate B: the one-knob sweet spot (+0.39 fun held-out) — ~80% of
        -- A's gain with minimal behavior change. The low-risk option.
        id = "candidate_b",
        name = "Candidate B - sharper AI shots",
        blob = "AI_SHOOT_RANGE=300",
    },
}
