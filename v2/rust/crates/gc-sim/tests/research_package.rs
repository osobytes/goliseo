//! Port of `spec/sim/research_package_spec.lua`.
//!
//! Every case in this spec builds its fixtures through
//! `example_package.gameplay()` (a real rollback timeline and trace
//! manifest over the checked-in short match tape) and calls into
//! `sim.research_dataset` and/or `sim.research_trace` directly. Both
//! `sim::research_dataset` (`sim/research_dataset.lua`) and
//! `sim::research_trace` (`sim/research_trace.lua`) are explicitly out of
//! scope for this port (`research_dataset` needs `research_session` plus
//! both of those, `research_trace` needs `input_tape` which needs
//! `sim::r#match` — see `v2/README.md`'s porting notes for this crate), so
//! every case here is stubbed, accurately named, and `#[ignore]`d — the
//! assertion set is tracked rather than silently dropped. None of it
//! overlaps `research_schema`/`research_features`/`research_response`/
//! `research_session`/`research_timeline`'s own specs, which are fully
//! ported in their own files.

macro_rules! blocked {
    ($name:ident) => {
        #[test]
        #[ignore = "needs sim::research_trace (sim/research_trace.lua) and sim::research_dataset (sim/research_dataset.lua), not yet ported"]
        fn $name() {
            unimplemented!("needs sim::research_trace and sim::research_dataset");
        }
    };
}

blocked!(research_example_package_assembles_a_completed_session_across_every_contract);
blocked!(research_example_package_keeps_corrected_away_rollback_events_out_of_the_confirmed_export);
blocked!(
    research_example_package_rejects_a_session_or_response_set_that_names_a_trace_nobody_holds
);
blocked!(
    research_example_package_refuses_a_response_set_for_an_instrument_the_session_says_was_skipped
);
blocked!(research_example_package_keeps_model_use_tied_to_the_accepted_agreement_version);
blocked!(research_example_package_joins_two_independent_annotation_sets_to_one_gameplay_run);
blocked!(research_example_package_records_an_interrupted_session_with_structured_missingness);
blocked!(research_example_package_scores_nothing_when_a_survey_item_is_missing);
blocked!(research_example_package_pairs_a_withdrawal_tombstone_with_its_emptied_session_envelope);
blocked!(research_example_package_refuses_a_dataset_that_retains_a_withdrawn_participant);
