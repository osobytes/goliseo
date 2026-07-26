local instruments = require("data.research_instruments")
local research_features = require("sim.research_features")
local research_schema = require("sim.research_schema")
local t = require("spec.support.runner")

t.describe("research feature register", function()
    t.it("accepts the authored register", function()
        t.is_true(research_features.validate_registry())
        t.is_true(#research_features.ids() > 20)
    end)

    t.it("registers every instrument construct exactly once", function()
        local expected = 0
        for instrument_id, instrument in pairs(instruments) do
            for _, construct in ipairs(instrument.constructs) do
                expected = expected + 1
                local id = research_features.feature_id(instrument_id, construct)
                local feature = assert(research_features.feature(id))
                t.eq(feature.grain ~= nil, true)
                t.eq(feature.evidence_tier, "human_experience")
            end
        end
        t.eq(expected, 24, "the instrument register changed shape")
        t.eq(#research_features.ids(), expected + 7)
    end)

    t.it("returns copies so callers cannot edit the register", function()
        local feature = assert(research_features.feature("soccer_shape_proxy_score"))
        feature.human_fun_claim = true
        t.eq(assert(research_features.feature("soccer_shape_proxy_score")).human_fun_claim, false)
    end)
end)

t.describe("fun-score tagging", function()
    t.it("registers metrics.fun_score as a soccer-shape proxy, never human fun", function()
        local feature = assert(research_features.feature("soccer_shape_proxy_score"))
        t.eq(feature.evidence_tier, "soccer_shape_proxy")
        t.eq(feature.human_fun_claim, false)
        t.eq(feature.outcome_role, "diagnostic")
        t.eq(feature.leakage_risk, "outcome_derived")
        t.eq(feature.observability, "privileged_diagnostic")
        t.is_true(feature.description:find("not a measurement of human fun", 1, true) ~= nil)

        local ok, err = research_features.use_allowed("soccer_shape_proxy_score", "human_fun_claim")
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("prohibits use", 1, true) ~= nil)
        t.is_true(not research_features.use_allowed("soccer_shape_proxy_score", "primary_outcome"))
        t.is_true(research_features.use_allowed("soccer_shape_proxy_score", "model_input"))
    end)

    t.it("keeps the human-fun claim on the primary enjoyment endpoint only", function()
        local claims = {}
        for _, id in ipairs(research_features.ids()) do
            local feature = assert(research_features.feature(id))
            if feature.human_fun_claim then
                claims[#claims + 1] = id
            end
        end
        t.eq(#claims, 1)
        t.eq(claims[1], "pxi_enjoyment_addon.enjoyment")

        local enjoyment = assert(research_features.feature("pxi_enjoyment_addon.enjoyment"))
        t.eq(enjoyment.outcome_role, "primary_outcome")
        t.eq(enjoyment.leakage_risk, "none")
        t.eq(enjoyment.evidence_tier, "human_experience")
    end)

    t.it("prohibits promoting a diagnostic or exploratory construct", function()
        for _, id in ipairs({
            "pxi_partial_mechanisms.autonomy",
            "bangs_session.competence_frustration",
            "custom_diagnostics.fairness",
            "affective_slider.valence",
        }) do
            local ok, err = research_features.use_allowed(id, "primary_outcome")
            t.is_true(ok == nil, id .. " must prohibit primary_outcome use")
            t.is_true(type(err) == "string")
        end
    end)
end)

t.describe("research feature observability categories", function()
    t.it("has a worked example for every observability category", function()
        local seen = {}
        for _, id in ipairs(research_features.ids()) do
            seen[assert(research_features.feature(id)).observability] = true
        end
        for _, category in ipairs({
            "player_observable",
            "privileged_diagnostic",
            "outcome_derived",
            "protected_sensitive",
            "prohibited",
        }) do
            t.is_true(seen[category], "no registered feature exercises " .. category)
        end
    end)

    t.it("keeps prohibited and sensitive features out of every use it names", function()
        local prohibited = assert(research_features.feature("inferred_participant_skill_trait"))
        t.eq(prohibited.observability, "prohibited")
        t.eq(prohibited.outcome_role, "feature_only")
        t.is_true(
            not research_features.use_allowed("inferred_participant_skill_trait", "any_analysis")
        )
        t.is_true(
            not research_features.use_allowed("inferred_participant_skill_trait", "model_input")
        )

        local sensitive = assert(research_features.feature("declared_readability_settings"))
        t.eq(sensitive.observability, "protected_sensitive")
        t.is_true(
            not research_features.use_allowed(
                "declared_readability_settings",
                "disability_inference"
            )
        )

        local outcome = assert(research_features.feature("final_score_margin"))
        t.eq(outcome.observability, "outcome_derived")
        t.eq(outcome.leakage_risk, "outcome_derived")
        t.is_true(not research_features.use_allowed("final_score_margin", "model_input"))
    end)
end)

t.describe("research feature grains", function()
    t.it("never treats a tick, match, or encounter row as a participant", function()
        for _, id in ipairs(research_features.ids()) do
            local feature = assert(research_features.feature(id))
            if research_features.CLUSTERED_GRAINS[feature.grain] then
                t.is_true(
                    feature.pseudo_replication_guard ~= "independent_unit_participant",
                    id .. " claims participant independence at grain " .. feature.grain
                )
            end
        end
    end)

    t.it("only allows the aggregation levels it defines", function()
        t.is_true(
            research_features.aggregation_allowed(
                "pxi_enjoyment_addon.enjoyment",
                "condition_block"
            )
        )
        local ok, err =
            research_features.aggregation_allowed("pxi_enjoyment_addon.enjoyment", "tick")
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("aggregation level", 1, true) ~= nil)
        t.is_true(not research_features.aggregation_allowed("soccer_shape_proxy_score", "session"))
        t.is_true(not research_features.aggregation_allowed("no_such_feature", "match"))
    end)

    t.it("rejects a feature that violates the register invariants", function()
        local base = assert(research_features.feature("involuntary_disable_share"))
        t.is_true(research_features.validate_feature(base))

        local pseudo_replicating = research_schema.copy(base)
        pseudo_replicating.pseudo_replication_guard = "independent_unit_participant"
        local ok, err = research_features.validate_feature(pseudo_replicating)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("independent participants", 1, true) ~= nil)

        local proxy_claiming_fun = assert(research_features.feature("soccer_shape_proxy_score"))
        proxy_claiming_fun.human_fun_claim = true
        t.is_true(not research_features.validate_feature(proxy_claiming_fun))

        local leaky_primary = research_schema.copy(base)
        leaky_primary.outcome_role = "primary_outcome"
        t.is_true(not research_features.validate_feature(leaky_primary))

        local missing_window = research_schema.copy(
            assert(research_features.feature("combat_to_soccer_conversion_rate"))
        )
        missing_window.causal_window = { kind = "forward_ticks" }
        t.is_true(not research_features.validate_feature(missing_window))

        local unregistered_grain = research_schema.copy(base)
        unregistered_grain.aggregation_levels = { "session" }
        t.is_true(not research_features.validate_feature(unregistered_grain))
    end)
end)

t.describe("research feature register hash", function()
    t.it("is stable and covers every feature definition", function()
        local hash = research_features.registry_hash()
        t.eq(#hash, research_schema.HASH_LENGTH)
        t.eq(hash, research_features.registry_hash())

        local feature = assert(research_features.feature("involuntary_disable_share"))
        local before = assert(research_schema.content_hash(research_features.SHAPE, feature))
        feature.causal_window = { kind = "forward_ticks", ticks = 60 }
        local after = assert(research_schema.content_hash(research_features.SHAPE, feature))
        t.is_true(before ~= after)
    end)
end)
