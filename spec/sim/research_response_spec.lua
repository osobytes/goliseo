local example_package = require("spec.fixtures.research.example_package")
local instruments = require("data.research_instruments")
local research_response = require("sim.research_response")
local research_schema = require("sim.research_schema")
local t = require("spec.support.runner")

---@return table
local function enjoyment()
    return example_package.enjoyment_responses()
end

---@param instrument_id string
---@param responses table[]
---@param overrides table?
---@return table
local function response_set(instrument_id, responses, overrides)
    overrides = overrides or {}
    local instrument = assert(instruments[instrument_id])
    local order = {}
    for index, item in ipairs(instrument.items) do
        order[index] = item.id
    end
    return {
        schema_version = 1,
        manifest_kind = "research_response_set",
        digest = research_schema.DIGEST,
        response_set_id = overrides.response_set_id or ("rs-" .. instrument_id),
        session_id = example_package.SESSION_ID,
        participant_id = example_package.PARTICIPANT_ID,
        condition_id = overrides.condition_id or "condition-b-combat-on",
        instrument_id = instrument_id,
        instrument_version = instrument.instrument_version,
        scoring_key_version = instrument.scoring_key_version,
        analysis_role = instrument.analysis_role,
        validated_instrument = instrument.validated,
        partial_administration = instrument.partial_administration,
        locale = { language_tag = "en-gb", translation_provenance = "original" },
        timing = {
            relative_to_play = overrides.relative_to_play or "natural_break",
            offset_ms = 1200,
        },
        presentation_order = order,
        randomized_presentation = false,
        responses = responses,
        scores = overrides.scores or {},
    }
end

---@return table
local function slider_set(overrides)
    return response_set("affective_slider", {
        { item_id = "as_valence", raw_response = 0.72 },
        { item_id = "as_arousal", raw_response = 0.35 },
    }, overrides)
end

---@return table
local function fallback_set(overrides)
    return response_set("custom_affect_fallback", {
        { item_id = "fallback_valence", raw_response = 5 },
        { item_id = "fallback_arousal", raw_response = 3 },
    }, overrides)
end

t.describe("research instrument register", function()
    t.it("accepts the authored register and embeds no item text", function()
        t.is_true(research_response.validate_register())
        for id, instrument in pairs(instruments) do
            t.eq(instrument.item_text_included, false, id .. " embeds item text")
            t.is_true(#instrument.constructs > 0)
            t.is_true(#instrument.items > 0)
        end
    end)

    t.it("keeps separately named constructs separate", function()
        local pxi = assert(instruments.pxi_enjoyment_addon)
        local bangs = assert(instruments.bangs_session)
        t.eq(#pxi.constructs, 1)
        t.eq(#bangs.constructs, 6)
        t.is_true(pxi.pooling_group ~= bangs.pooling_group)
        local forbids = false
        for _, forbidden in ipairs(pxi.forbidden_pooling) do
            if forbidden == "bangs_session" then
                forbids = true
            end
        end
        t.is_true(forbids, "PXI enjoyment must not be poolable with BANGS")
    end)

    t.it("rejects a register that claims a primary unvalidated instrument", function()
        local broken = research_schema.copy(instruments)
        broken.custom_diagnostics.analysis_role = "primary"
        t.is_true(not pcall(research_response.validate_register, broken))

        local text_leak = research_schema.copy(instruments)
        text_leak.pxi_enjoyment_addon.item_text_included = true
        t.is_true(not pcall(research_response.validate_register, text_leak))

        local aggregated_custom = research_schema.copy(instruments)
        aggregated_custom.custom_diagnostics.score_aggregation = "mean_all_items"
        t.is_true(not pcall(research_response.validate_register, aggregated_custom))

        local unknown_substitute = research_schema.copy(instruments)
        unknown_substitute.custom_affect_fallback.substitute_for = "no_such_instrument"
        t.is_true(not pcall(research_response.validate_register, unknown_substitute))
    end)
end)

t.describe("research response set validation", function()
    t.it("accepts the complete enjoyment administration and round-trips", function()
        local set = enjoyment()
        t.is_true(research_response.validate(set))
        local bytes = assert(research_response.encode(set))
        local decoded = assert(research_response.decode(bytes))
        t.eq(assert(research_response.encode(decoded)), bytes)
        t.eq(decoded.scores[1].construct, "enjoyment")
    end)

    t.it("rejects non-finite responses in this family", function()
        local nan = enjoyment()
        nan.responses[1].raw_response = 0 / 0
        nan.scores = {}
        t.is_true(not research_response.validate(nan))
    end)

    t.it("rejects responses off the registered scale", function()
        local out_of_range = enjoyment()
        out_of_range.responses[1].raw_response = 4
        t.is_true(not research_response.validate(out_of_range))

        local off_step = enjoyment()
        off_step.responses[1].raw_response = 1.5
        t.is_true(not research_response.validate(off_step))

        local slider_off_step = slider_set()
        slider_off_step.responses[1].raw_response = 0.725
        t.is_true(not research_response.validate(slider_off_step))
    end)

    t.it("requires exactly one row per item, answered or structurally missing", function()
        local both = enjoyment()
        both.responses[1].missing_reason = "participant_skipped"
        t.is_true(not research_response.validate(both))

        local neither = enjoyment()
        neither.responses[1].raw_response = nil
        t.is_true(not research_response.validate(neither))

        local dropped = enjoyment()
        table.remove(dropped.responses, 3)
        dropped.scores = {}
        local ok, err = research_response.validate(dropped)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("one row per", 1, true) ~= nil)

        local duplicated = enjoyment()
        duplicated.responses[3] = research_schema.copy(duplicated.responses[1])
        t.is_true(not research_response.validate(duplicated))
    end)

    t.it("requires the presentation order to cover the instrument exactly", function()
        local short_order = enjoyment()
        table.remove(short_order.presentation_order, 1)
        t.is_true(not research_response.validate(short_order))

        local duplicated_order = enjoyment()
        duplicated_order.presentation_order[1] = duplicated_order.presentation_order[2]
        t.is_true(not research_response.validate(duplicated_order))

        local foreign_order = enjoyment()
        foreign_order.presentation_order[1] = "bangs_auto_sat_1"
        t.is_true(not research_response.validate(foreign_order))
    end)

    t.it("stops on instrument, scoring-key, and role drift", function()
        local wrong_instrument_version = enjoyment()
        wrong_instrument_version.instrument_version = "pxi-2019.0-enjoyment-addon"
        local ok, err = research_response.validate(wrong_instrument_version)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("does not match registered", 1, true) ~= nil)

        local wrong_key = enjoyment()
        wrong_key.scoring_key_version = "pxi-enjoyment-sum-1"
        t.is_true(not research_response.validate(wrong_key))

        local relabelled = enjoyment()
        relabelled.analysis_role = "exploratory"
        t.is_true(not research_response.validate(relabelled))

        local unvalidated_claim = enjoyment()
        unvalidated_claim.validated_instrument = false
        t.is_true(not research_response.validate(unvalidated_claim))

        local unknown = enjoyment()
        unknown.instrument_id = "not-an-instrument"
        t.is_true(not research_response.validate(unknown))
    end)

    t.it("requires translation provenance to be internally consistent", function()
        local translated = enjoyment()
        translated.locale.translation_provenance = "validated_translation"
        t.is_true(not research_response.validate(translated))
        translated.locale.translation_id = "pxi-es-2022"
        t.is_true(research_response.validate(translated))

        local original_with_id = enjoyment()
        original_with_id.locale.translation_id = "pxi-es-2022"
        t.is_true(not research_response.validate(original_with_id))
    end)
end)

t.describe("research construct scoring", function()
    t.it("scores a mean only when every construct item is answered", function()
        local raw_scores, raw_incomplete = research_response.recompute_scores(enjoyment())
        local scores = assert(raw_scores)
        t.eq(#scores, 1)
        t.eq(#assert(raw_incomplete), 0)
        t.near(scores[1].score, 2, 1e-9)

        local partial = example_package.enjoyment_responses({ missing_item = "time_limit" })
        local partial_scores, raw_partial_incomplete = research_response.recompute_scores(partial)
        local partial_incomplete = assert(raw_partial_incomplete)
        t.eq(#assert(partial_scores), 0)
        t.eq(#partial_incomplete, 1)
        t.eq(partial_incomplete[1], "enjoyment")
        t.is_true(research_response.validate(partial))
    end)

    t.it("never aggregates an item-only instrument", function()
        local slider = slider_set()
        t.is_true(research_response.validate(slider))
        local scores, incomplete = research_response.recompute_scores(slider)
        t.eq(#assert(scores), 0)
        t.eq(#assert(incomplete), 2)

        local aggregated = slider_set({
            scores = {
                { construct = "valence", score = 0.72, rule = "item_only", item_count = 1 },
            },
        })
        local ok, err = research_response.validate(aggregated)
        t.is_true(ok == nil)
        t.is_true(type(err) == "string" and err:find("item-only", 1, true) ~= nil)
    end)

    t.it("rejects a hand-edited or partially-scored construct", function()
        local edited = enjoyment()
        edited.scores[1].score = 2.5
        t.is_true(not research_response.validate(edited))

        local wrong_count = enjoyment()
        wrong_count.scores[1].item_count = 2
        t.is_true(not research_response.validate(wrong_count))

        local scored_partial = example_package.enjoyment_responses({
            missing_item = "participant_skipped",
        })
        scored_partial.scores = {
            { construct = "enjoyment", score = 1.5, rule = "mean_all_items", item_count = 2 },
        }
        t.is_true(not research_response.validate(scored_partial))
    end)
end)

t.describe("research response administration", function()
    t.it("accepts distinct instruments at one timepoint", function()
        t.is_true(research_response.validate_administration({ enjoyment(), slider_set() }))
    end)

    t.it("refuses the accessibility fallback beside the instrument it replaces", function()
        local ok, err = research_response.validate_administration({
            slider_set({ relative_to_play = "natural_break" }),
            fallback_set({ relative_to_play = "natural_break" }),
        })
        t.is_true(ok == nil)
        t.is_true(
            type(err) == "string" and err:find("non-poolable substitute target", 1, true) ~= nil
        )

        t.is_true(research_response.validate_administration({ fallback_set() }))
    end)

    t.it("refuses a repeated administration of the same instrument and timepoint", function()
        local ok = research_response.validate_administration({
            enjoyment(),
            example_package.enjoyment_responses({ response_set_id = "rs-second" }),
        })
        t.is_true(ok == nil)

        local duplicate_ids = research_response.validate_administration({
            enjoyment(),
            enjoyment(),
        })
        t.is_true(duplicate_ids == nil)
    end)
end)
