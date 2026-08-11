//! Port of `spec/sim/network_conditions_spec.lua`.
//!
//! On the determinism path (uses `core.rng` and feeds rollback resim): see
//! `differential.rs` for the required bit-for-bit comparison against the
//! reference Lua implementation.

use gc_core::rng;
use gc_sim::input_frame::{self, InputSample, InputSampleOptions};
use gc_sim::network_conditions::{
    self, NetworkConditionErrorCode, NetworkConditions, NetworkDelivery, NetworkProfile,
    NetworkResendRequest,
};

fn sample(move_x: Option<i64>, edges: Option<i64>) -> InputSample {
    input_frame::new_sample(InputSampleOptions {
        move_x,
        edges,
        ..Default::default()
    })
    .unwrap()
}

#[derive(Default)]
struct ProfileOptions {
    base_delay_ticks: i64,
    jitter_min_ticks: i64,
    jitter_max_ticks: i64,
    independent_loss_rate: f64,
    duplication_rate: f64,
    burst_start_rate: f64,
    burst_length_ticks: i64,
}

fn profile(o: ProfileOptions) -> NetworkProfile {
    NetworkProfile {
        base_delay_ticks: o.base_delay_ticks,
        jitter_min_ticks: o.jitter_min_ticks,
        jitter_max_ticks: o.jitter_max_ticks,
        independent_loss_rate: o.independent_loss_rate,
        duplication_rate: o.duplication_rate,
        burst_start_rate: o.burst_start_rate,
        burst_length_ticks: o.burst_length_ticks,
    }
}

fn delivery_schedule(deliveries: &[NetworkDelivery]) -> String {
    deliveries
        .iter()
        .map(|d| format!("{}:{}@{}", d.sequence, d.duplicate_ordinal, d.arrival_tick))
        .collect::<Vec<_>>()
        .join(",")
}

fn outcome(conditions: &mut NetworkConditions) -> String {
    let deliveries = network_conditions::poll(conditions, 1000);
    let counters = network_conditions::counters(conditions);
    let delivery_parts: Vec<String> = deliveries
        .iter()
        .map(|delivery| {
            let record_parts: Vec<String> = network_conditions::records(delivery)
                .iter()
                .map(|record| {
                    format!(
                        "{}/{}/{}/{}/{}",
                        record.tick,
                        record.sample.move_x,
                        record.sample.move_y,
                        record.sample.held,
                        record.sample.edges
                    )
                })
                .collect();
            format!(
                "{}:{}:{}:{}:{}:{}",
                delivery.source_slot,
                delivery.send_tick,
                delivery.sequence,
                delivery.duplicate_ordinal,
                delivery.arrival_tick,
                record_parts.join(",")
            )
        })
        .collect();
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        delivery_parts.join(";"),
        counters.sent,
        counters.delivered,
        counters.independent_lost,
        counters.burst_lost,
        counters.duplicated,
        counters.reordered,
        counters.history_recovered
    )
}

fn populated_playable(seed: f64) -> NetworkConditions {
    let playable: NetworkProfile =
        gc_data::network_profiles::get(gc_data::network_profiles::NetworkProfileName::Playable)
            .into();
    let mut conditions = network_conditions::new(&playable, seed);
    for tick in 0..40_i64 {
        network_conditions::send(
            &mut conditions,
            tick,
            (tick % 8) + 1,
            tick,
            &sample(Some(tick % 128), None),
        )
        .unwrap();
    }
    conditions
}

#[test]
fn omp2_deterministic_network_conditions_pins_the_named_laboratory_profiles() {
    let clean: NetworkProfile =
        gc_data::network_profiles::get(gc_data::network_profiles::NetworkProfileName::Clean).into();
    assert_eq!(clean.base_delay_ticks, 0);
    assert_eq!(clean.jitter_min_ticks, 0);
    assert_eq!(clean.jitter_max_ticks, 0);
    assert_eq!(clean.independent_loss_rate, 0.0);
    assert_eq!(clean.duplication_rate, 0.0);
    assert_eq!(clean.burst_start_rate, 0.0);
    assert_eq!(clean.burst_length_ticks, 0);

    let parity: NetworkProfile =
        gc_data::network_profiles::get(gc_data::network_profiles::NetworkProfileName::Omp0Parity)
            .into();
    assert_eq!(parity.base_delay_ticks, 3);
    assert_eq!(parity.jitter_min_ticks, 0);
    assert_eq!(parity.jitter_max_ticks, 0);
    assert_eq!(parity.independent_loss_rate, 0.01);
    assert_eq!(parity.duplication_rate, 0.0);
    assert_eq!(parity.burst_start_rate, 0.0);
    assert_eq!(parity.burst_length_ticks, 0);

    let playable: NetworkProfile =
        gc_data::network_profiles::get(gc_data::network_profiles::NetworkProfileName::Playable)
            .into();
    assert_eq!(playable.base_delay_ticks, 3);
    assert_eq!(playable.jitter_min_ticks, -2);
    assert_eq!(playable.jitter_max_ticks, 2);
    assert_eq!(playable.independent_loss_rate, 0.01);
    assert_eq!(playable.duplication_rate, 0.0025);
    assert_eq!(playable.burst_start_rate, 0.0025);
    assert_eq!(playable.burst_length_ticks, 3);

    let stress: NetworkProfile =
        gc_data::network_profiles::get(gc_data::network_profiles::NetworkProfileName::Stress)
            .into();
    assert_eq!(stress.base_delay_ticks, 6);
    assert_eq!(stress.jitter_min_ticks, -3);
    assert_eq!(stress.jitter_max_ticks, 3);
    assert_eq!(stress.independent_loss_rate, 0.03);
    assert_eq!(stress.duplication_rate, 0.01);
    assert_eq!(stress.burst_start_rate, 0.01);
    assert_eq!(stress.burst_length_ticks, 3);
}

#[test]
fn omp2_deterministic_network_conditions_preserves_exact_samples_and_send_order_under_the_clean_profile()
 {
    let clean: NetworkProfile =
        gc_data::network_profiles::get(gc_data::network_profiles::NetworkProfileName::Clean).into();
    let mut conditions = network_conditions::new(&clean, 91.0);
    let supplied = sample(Some(14), Some(input_frame::EdgeAction::Dash.bit()));
    network_conditions::send(&mut conditions, 0, 1, 0, &supplied).unwrap();
    // The Lua original mutates its local `supplied.move_x` here and later
    // asserts the retained record still reads 14, proving `send` copies
    // rather than aliases the caller's sample. `InputSample` is `Copy` in
    // this port, so `send`'s `&InputSample` parameter can never alias
    // anything the caller later mutates — the property is structurally
    // guaranteed rather than merely tested. The `move_x == 14` assertion
    // below is kept as direct evidence of the same fact.
    network_conditions::send(&mut conditions, 1, 1, 1, &sample(Some(22), None)).unwrap();

    let mut deliveries = network_conditions::poll(&mut conditions, 1);
    assert_eq!(delivery_schedule(&deliveries), "1:0@0,2:0@1");
    assert_eq!(deliveries[0].current.sample.move_x, 14);
    assert_eq!(
        deliveries[0].current.sample.edges,
        input_frame::EdgeAction::Dash.bit()
    );
    assert_eq!(deliveries[0].history.len(), 0);
    assert_eq!(deliveries[1].history.len(), 1);
    deliveries[1].history[0].sample.move_x = -1;
    let records = network_conditions::records(&deliveries[1]);
    assert_eq!(records[0].tick, 0);
    assert_eq!(
        records[0].sample.move_x, -1,
        "records copies the caller-owned delivery"
    );
}

#[test]
fn omp2_deterministic_network_conditions_uses_literal_fixed_latency_and_clamps_negative_delivery_time()
 {
    let delayed_profile = profile(ProfileOptions {
        base_delay_ticks: 3,
        ..Default::default()
    });
    let mut delayed = network_conditions::new(&delayed_profile, 7.0);
    let receipt =
        network_conditions::send(&mut delayed, 7, 1, 20, &sample(Some(20), None)).unwrap();
    assert_eq!(receipt.arrival_tick, Some(10));
    assert_eq!(network_conditions::poll(&mut delayed, 9).len(), 0);
    assert_eq!(
        delivery_schedule(&network_conditions::poll(&mut delayed, 10)),
        "1:0@10"
    );

    let clamped_profile = profile(ProfileOptions {
        jitter_min_ticks: -3,
        jitter_max_ticks: -3,
        ..Default::default()
    });
    let mut clamped = network_conditions::new(&clamped_profile, 7.0);
    let clamped_receipt =
        network_conditions::send(&mut clamped, 12, 1, 20, &sample(Some(20), None)).unwrap();
    assert_eq!(clamped_receipt.arrival_tick, Some(12));
}

#[test]
fn omp2_deterministic_network_conditions_bounds_transport_arrival_before_mutating_send_or_drain_state()
 {
    let maximum = network_conditions::MAX_TRANSPORT_TICK;
    assert_eq!(maximum, 2_147_483_647);

    let rejected_profile = profile(ProfileOptions {
        base_delay_ticks: 3,
        ..Default::default()
    });
    let mut rejected = network_conditions::new(&rejected_profile, 7.0);
    let err = network_conditions::send(&mut rejected, maximum - 2, 1, 0, &sample(Some(1), None))
        .unwrap_err();
    assert_eq!(err.code, NetworkConditionErrorCode::Malformed);
    assert!(err.message.contains("transport tick limit"));
    assert_eq!(network_conditions::counters(&rejected).sent, 0);
    assert_eq!(
        network_conditions::diagnostics(&rejected).retained_authoritative_records,
        0
    );

    let boundary =
        network_conditions::send(&mut rejected, maximum - 3, 1, 0, &sample(Some(1), None)).unwrap();
    assert_eq!(
        boundary.sequence, 1,
        "rejected send consumes neither sequence nor RNG state"
    );
    assert_eq!(boundary.arrival_tick, Some(maximum));
    assert_eq!(
        delivery_schedule(&network_conditions::poll(&mut rejected, maximum)),
        "1:0@2147483647"
    );

    let draining_profile = profile(ProfileOptions {
        base_delay_ticks: 3,
        ..Default::default()
    });
    let mut draining = network_conditions::new(&draining_profile, 9.0);
    network_conditions::send(&mut draining, maximum - 6, 1, 4, &sample(Some(4), None)).unwrap();
    network_conditions::poll(&mut draining, maximum - 3);
    let before = network_conditions::counters(&draining);
    let err = network_conditions::drain(
        &mut draining,
        maximum - 2,
        1,
        &[NetworkResendRequest {
            source_slot: 1,
            input_tick: 4,
        }],
    )
    .unwrap_err();
    assert_eq!(err.code, NetworkConditionErrorCode::Malformed);
    assert!(err.message.contains("transport tick limit"));
    assert_eq!(network_conditions::counters(&draining).sent, before.sent);
    assert_eq!(network_conditions::pending(&draining), 0);
}

#[test]
fn omp2_deterministic_network_conditions_packs_sample_extrema_into_distinct_collision_free_diagnostic_keys()
 {
    let minimum = input_frame::new_sample(InputSampleOptions {
        move_x: Some(-127),
        move_y: Some(-127),
        held: Some(0),
        edges: Some(0),
    })
    .unwrap();
    let before_rollover = input_frame::new_sample(InputSampleOptions {
        move_x: Some(-127),
        move_y: Some(127),
        held: Some(127),
        edges: Some(127),
    })
    .unwrap();
    let after_rollover = input_frame::new_sample(InputSampleOptions {
        move_x: Some(-126),
        move_y: Some(-127),
        held: Some(0),
        edges: Some(0),
    })
    .unwrap();
    let maximum = input_frame::new_sample(InputSampleOptions {
        move_x: Some(127),
        move_y: Some(127),
        held: Some(127),
        edges: Some(127),
    })
    .unwrap();
    assert_eq!(network_conditions::sample_key(&minimum).unwrap(), 0);
    assert_eq!(
        network_conditions::sample_key(&before_rollover).unwrap(),
        (254 * 256 + 127) * 128 + 127
    );
    assert_eq!(
        network_conditions::sample_key(&after_rollover).unwrap(),
        255 * 256 * 128
    );
    assert_eq!(
        network_conditions::sample_key(&maximum).unwrap(),
        ((254 * 255 + 254) * 256 + 127) * 128 + 127
    );
}

#[test]
fn omp2_deterministic_network_conditions_hits_both_jitter_bounds_and_reorders_only_by_natural_arrival()
 {
    let p = profile(ProfileOptions {
        base_delay_ticks: 2,
        jitter_min_ticks: -2,
        jitter_max_ticks: 2,
        ..Default::default()
    });
    let mut conditions = network_conditions::new(&p, 102223.0);
    let upper = network_conditions::send(&mut conditions, 0, 1, 0, &sample(Some(1), None)).unwrap();
    let lower = network_conditions::send(&mut conditions, 1, 1, 1, &sample(Some(2), None)).unwrap();
    assert_eq!(upper.arrival_tick, Some(4), "first jitter roll selects +2");
    assert_eq!(lower.arrival_tick, Some(1), "second jitter roll selects -2");

    assert_eq!(
        delivery_schedule(&network_conditions::poll(&mut conditions, 1)),
        "2:0@1"
    );
    assert_eq!(
        delivery_schedule(&network_conditions::poll(&mut conditions, 4)),
        "1:0@4"
    );
    assert_eq!(network_conditions::counters(&conditions).reordered, 1);
    assert_eq!(
        network_conditions::counters(&conditions).history_recovered,
        1,
        "the reordered original does not recount history recovered by the later sequence"
    );
}

#[test]
fn omp2_deterministic_network_conditions_follows_the_literal_independent_loss_schedule_for_seed_85()
{
    let p = profile(ProfileOptions {
        independent_loss_rate: 0.5,
        ..Default::default()
    });
    let mut conditions = network_conditions::new(&p, 85.0);
    let mut dropped = String::new();
    for tick in 0..6_i64 {
        let receipt =
            network_conditions::send(&mut conditions, tick, 1, tick, &sample(Some(tick), None))
                .unwrap();
        dropped.push(if receipt.dropped { '1' } else { '0' });
    }
    assert_eq!(dropped, "101001");
    assert_eq!(
        delivery_schedule(&network_conditions::poll(&mut conditions, 5)),
        "2:0@1,4:0@3,5:0@4"
    );
    let counters = network_conditions::counters(&conditions);
    assert_eq!(counters.independent_lost, 3);
    assert_eq!(counters.burst_lost, 0);
}

#[test]
fn omp2_deterministic_network_conditions_duplicates_envelopes_with_stable_identity_and_equal_arrival_ordering()
 {
    let p = profile(ProfileOptions {
        duplication_rate: 0.5,
        ..Default::default()
    });
    let mut conditions = network_conditions::new(&p, 592.0);
    for tick in 0..5_i64 {
        network_conditions::send(&mut conditions, 0, 1, tick, &sample(Some(tick), None)).unwrap();
    }
    let deliveries = network_conditions::poll(&mut conditions, 0);
    assert_eq!(
        delivery_schedule(&deliveries),
        "1:0@0,1:1@0,2:0@0,3:0@0,3:1@0,4:0@0,5:0@0"
    );
    assert_eq!(deliveries[0].sequence, deliveries[1].sequence);
    assert_eq!(deliveries[0].current.tick, deliveries[1].current.tick);
    assert_eq!(network_conditions::counters(&conditions).duplicated, 2);
    assert_eq!(
        network_conditions::counters(&conditions).history_recovered,
        0
    );
}

#[test]
fn omp2_deterministic_network_conditions_applies_a_literal_three_tick_burst_per_source_slot() {
    let p = profile(ProfileOptions {
        burst_start_rate: 0.5,
        burst_length_ticks: 3,
        ..Default::default()
    });
    let mut conditions = network_conditions::new(&p, 58.0);
    let r1 = network_conditions::send(&mut conditions, 0, 1, 0, &sample(Some(0), None)).unwrap();
    let r2 = network_conditions::send(&mut conditions, 1, 1, 1, &sample(Some(1), None)).unwrap();
    let r3 = network_conditions::send(&mut conditions, 2, 1, 2, &sample(Some(2), None)).unwrap();
    let other_slot =
        network_conditions::send(&mut conditions, 2, 2, 0, &sample(Some(20), None)).unwrap();
    let r4 = network_conditions::send(&mut conditions, 3, 1, 3, &sample(Some(3), None)).unwrap();
    let r5 = network_conditions::send(&mut conditions, 4, 1, 4, &sample(Some(4), None)).unwrap();

    assert_eq!(r1.drop_reason, None);
    assert_eq!(
        r2.drop_reason,
        Some(network_conditions::NetworkDropReason::BurstLoss)
    );
    assert_eq!(
        r3.drop_reason,
        Some(network_conditions::NetworkDropReason::BurstLoss)
    );
    assert_eq!(
        r4.drop_reason,
        Some(network_conditions::NetworkDropReason::BurstLoss)
    );
    assert_eq!(r5.drop_reason, None);
    assert_eq!(
        other_slot.drop_reason, None,
        "slot two is not inside slot one's burst"
    );
    assert_eq!(
        delivery_schedule(&network_conditions::poll(&mut conditions, 4)),
        "1:0@0,4:0@2,6:0@4"
    );
    assert_eq!(network_conditions::counters(&conditions).burst_lost, 3);
}

#[test]
fn omp2_deterministic_network_conditions_retains_exactly_six_earlier_unique_rows_and_recovers_loss_from_history()
 {
    let clean: NetworkProfile =
        gc_data::network_profiles::get(gc_data::network_profiles::NetworkProfileName::Clean).into();
    let mut retained = network_conditions::new(&clean, 3.0);
    for tick in 0..8_i64 {
        network_conditions::send(&mut retained, tick, 1, tick, &sample(Some(tick), None)).unwrap();
    }
    let deliveries = network_conditions::poll(&mut retained, 7);
    let last = deliveries.last().unwrap();
    assert_eq!(last.history.len(), 6);
    for (index, record) in last.history.iter().enumerate() {
        assert_eq!(record.tick, index as i64 + 1);
    }
    assert_eq!(last.current.tick, 7);

    let recovered_profile = profile(ProfileOptions {
        independent_loss_rate: 0.5,
        ..Default::default()
    });
    let mut recovered = network_conditions::new(&recovered_profile, 85.0);
    assert!(
        network_conditions::send(&mut recovered, 0, 1, 0, &sample(Some(40), None))
            .unwrap()
            .dropped
    );
    assert!(
        !network_conditions::send(&mut recovered, 1, 1, 1, &sample(Some(41), None))
            .unwrap()
            .dropped
    );
    let recovered_delivery = network_conditions::poll(&mut recovered, 1);
    let records = network_conditions::records(&recovered_delivery[0]);
    assert_eq!(records[0].tick, 0);
    assert_eq!(records[1].tick, 1);
    assert_eq!(
        network_conditions::counters(&recovered).history_recovered,
        1
    );

    let oldest_profile = profile(ProfileOptions {
        independent_loss_rate: 0.5,
        ..Default::default()
    });
    let mut oldest = network_conditions::new(&oldest_profile, 290.0);
    for tick in 0..7_i64 {
        network_conditions::send(&mut oldest, tick, 1, tick, &sample(Some(60 + tick), None))
            .unwrap();
    }
    let oldest_delivery = network_conditions::poll(&mut oldest, 6);
    assert_eq!(oldest_delivery.len(), 1);
    let oldest_records = network_conditions::records(&oldest_delivery[0]);
    assert_eq!(oldest_records.len(), 7);
    assert_eq!(
        oldest_records[0].tick, 0,
        "the oldest of six redundant rows is recovered"
    );
    assert_eq!(oldest_records[6].tick, 6);
    assert_eq!(network_conditions::counters(&oldest).history_recovered, 6);
}

#[test]
fn omp2_deterministic_network_conditions_rejects_conflicting_history_and_resends_without_adding_an_input_row()
 {
    let clean: NetworkProfile =
        gc_data::network_profiles::get(gc_data::network_profiles::NetworkProfileName::Clean).into();
    let mut conditions = network_conditions::new(&clean, 11.0);
    network_conditions::send(&mut conditions, 0, 1, 0, &sample(Some(10), None)).unwrap();
    network_conditions::send(&mut conditions, 1, 1, 1, &sample(Some(11), None)).unwrap();

    let err =
        network_conditions::send(&mut conditions, 1, 1, 1, &sample(Some(12), None)).unwrap_err();
    assert_eq!(
        err.code,
        NetworkConditionErrorCode::ConflictingAuthoritative
    );
    assert!(err.message.contains("tick 1 slot 1"));
    assert_eq!(network_conditions::counters(&conditions).sent, 2);

    let receipt = network_conditions::resend(&mut conditions, 2, 1, 1).unwrap();
    assert!(receipt.authoritative_duplicate);
    let deliveries = network_conditions::poll(&mut conditions, 2);
    let resend = deliveries.last().unwrap();
    assert_eq!(resend.history.len(), 1);
    assert_eq!(resend.history[0].tick, 0);
    assert_eq!(resend.current.tick, 1);
}

#[test]
fn omp2_deterministic_network_conditions_drains_a_lost_final_sample_through_resends_without_a_match_tick()
 {
    let p = profile(ProfileOptions {
        independent_loss_rate: 0.5,
        ..Default::default()
    });
    let mut conditions = network_conditions::new(&p, 85.0);
    let original =
        network_conditions::send(&mut conditions, 0, 1, 99, &sample(Some(99), None)).unwrap();
    assert_eq!(
        original.drop_reason,
        Some(network_conditions::NetworkDropReason::IndependentLoss)
    );

    let result = network_conditions::drain(
        &mut conditions,
        1,
        5,
        &[NetworkResendRequest {
            source_slot: 1,
            input_tick: 99,
        }],
    )
    .unwrap();
    assert!(result.complete);
    assert_eq!(result.final_tick, 1);
    assert_eq!(result.recovered, 1);
    assert_eq!(result.pending, 0);
    assert_eq!(delivery_schedule(&result.deliveries), "2:0@1");
    assert_eq!(
        result.deliveries[0].current.tick, 99,
        "transport advanced, input did not"
    );
}

#[test]
fn omp2_deterministic_network_conditions_is_byte_equivalent_for_one_seed_and_isolated_from_match_rng()
 {
    let mut match_state = rng::seed(999.0);
    let mut control_state = rng::seed(999.0);
    let first = outcome(&mut populated_playable(431.0));
    let second = outcome(&mut populated_playable(431.0));
    let different = outcome(&mut populated_playable(432.0));
    assert_eq!(first, second);
    assert_ne!(first, different);

    let (next_match_state, match_roll) = rng::roll(match_state);
    let (next_control_state, control_roll) = rng::roll(control_state);
    match_state = next_match_state;
    control_state = next_control_state;
    assert_eq!(match_state, control_state);
    assert_eq!(match_roll, control_roll);
}

#[test]
fn omp2_deterministic_network_conditions_stays_within_broad_deterministic_playable_profile_bounds()
{
    let playable: NetworkProfile =
        gc_data::network_profiles::get(gc_data::network_profiles::NetworkProfileName::Playable)
            .into();
    let mut conditions = network_conditions::new(&playable, 4242.0);
    let count = 10_000_i64;
    for tick in 0..count {
        network_conditions::send(
            &mut conditions,
            tick,
            (tick % input_frame::SLOT_COUNT) + 1,
            tick,
            &sample(Some(tick % 128), None),
        )
        .unwrap();
    }
    network_conditions::poll(&mut conditions, count + 10);
    let counters = network_conditions::counters(&conditions);
    assert_eq!(counters.sent, count);
    assert_eq!(network_conditions::pending(&conditions), 0);
    assert!(counters.independent_lost >= 50 && counters.independent_lost <= 160);
    assert!(counters.burst_lost >= 20 && counters.burst_lost <= 180);
    assert!(counters.duplicated >= 5 && counters.duplicated <= 60);
    assert!(counters.reordered > 0);
    assert!(counters.history_recovered > 0);
}

#[test]
fn omp2_deterministic_network_conditions_bounds_retained_diagnostics_across_a_full_seven_remote_fixture()
 {
    let clean: NetworkProfile =
        gc_data::network_profiles::get(gc_data::network_profiles::NetworkProfileName::Clean).into();
    let mut conditions = network_conditions::new(&clean, 1201.0);
    let last_tick = 7200_i64;
    let remote_slots = 7_i64;
    for tick in 0..=last_tick {
        for source_slot in 1..=remote_slots {
            network_conditions::send(
                &mut conditions,
                tick,
                source_slot,
                tick,
                &sample(Some((tick + source_slot) % 128), None),
            )
            .unwrap();
        }
        network_conditions::poll(&mut conditions, tick);
        if tick % 600 == 0 {
            let diagnostics = network_conditions::diagnostics(&conditions);
            assert!(
                diagnostics.retained_authoritative_records
                    <= remote_slots * network_conditions::RETAINED_RECORDS as i64
            );
            assert!(
                diagnostics.delivered_ledger_entries
                    <= diagnostics.retained_authoritative_records
                        + diagnostics.pending_record_references
            );
        }
    }

    let diagnostics = network_conditions::diagnostics(&conditions);
    assert_eq!(diagnostics.retained_authoritative_records, 49);
    assert_eq!(diagnostics.delivered_ledger_entries, 49);
    assert_eq!(diagnostics.pending_envelopes, 0);
    assert_eq!(diagnostics.pending_record_references, 0);
    assert_eq!(diagnostics.peak_retained_authoritative_records, 49);
    assert_eq!(diagnostics.peak_delivered_ledger_entries, 49);
    assert_eq!(diagnostics.peak_pending_envelopes, remote_slots);
    assert!(diagnostics.peak_pending_record_references >= remote_slots);
    assert_eq!(network_conditions::counters(&conditions).sent, 50407);
}
