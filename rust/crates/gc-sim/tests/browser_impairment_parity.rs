//! The native half of the browser-impairment differential (#472).
//!
//! `ts/packages/transport/src/impairment.ts` re-implements the impairment
//! half of [`gc_sim::network_conditions`] over the browser transport
//! contract, because browser evidence has to impair traffic the way the
//! native rollback matrix does or the two suites measure different things
//! while both look green.
//!
//! Nothing but convention keeps the two implementations in step, so this
//! test and `ts/packages/transport/src/impairment_parity.spec.ts` each run
//! the SAME scripted scenarios and each assert the SAME transcript literal.
//! `scripts/check_network_profile_parity.mjs` (gate 0c) additionally asserts
//! that the two literals are byte-identical, so a drift on either side is
//! caught even if only one language's tests are run.
//!
//! WHAT THE TRANSCRIPT ENCODES, and why it is compact rather than verbose:
//! the shared subset of the two implementations is the impairment DECISION
//! SEQUENCE -- which packet was dropped and why, which was duplicated, when
//! each arrives, and in what order they come out. The native module's
//! redundant input history, its authoritative-record ledger, and `drain`
//! have no browser counterpart and are deliberately absent here.
//!
//!   scenario|name=..|profile=..|seed=..|sends=..|slots=..
//!   sends|<one entry per send, in send order>
//!   deliveries|<one entry per delivered envelope, in delivery order>
//!   counters|sent=..,delivered=..,independent_lost=..,burst_lost=..,duplicated=..,reordered=..
//!
//! A `sends` entry is the packet's arrival tick, suffixed `+` when the
//! profile also scheduled a duplicate of it; `x` for an independent loss and
//! `b` for a burst loss. Its index is the packet's sequence minus one and
//! its send tick, and for a two-slot scenario the source slot alternates
//! `(index % 2) + 1` -- all three are implied, so they are not restated.
//! A `deliveries` entry is the delivered packet's sequence, suffixed `d`
//! when it is the impairment-created duplicate copy.
//!
//! ## What this differential is not
//!
//! The two implementations are checked against ONE SHARED GOLDEN LITERAL that
//! is duplicated in both files -- not against each other's live output. Both
//! sides really do the work to reproduce it, and gate 0c keeps the two copies
//! byte-identical, so drift after the golden was captured is caught. But a bug
//! present in BOTH implementations at the moment the golden was captured would
//! be baked into the golden and invisible here forever. That residual risk is
//! covered by reading this module against `network_conditions.rs` directly,
//! not by this test: if the two implementations are ever changed together,
//! re-derive the golden from this side and re-read the source while doing it.
//!
//! Run `cargo test -p gc-sim --test browser_impairment_parity -- --nocapture`
//! to print the transcript this scenario set produces.

use gc_data::network_profiles::{self, NetworkProfileName};
use gc_sim::input_frame::{self, InputSampleOptions};
use gc_sim::network_conditions::{self, NetworkConditions, NetworkProfile};

/// Every scenario in the shared transcript, in order.
struct Scenario {
    name: &'static str,
    profile: NetworkProfileName,
    seed: f64,
    sends: i64,
    slots: i64,
}

const SCENARIOS: &[Scenario] = &[
    // No impairment at all: every arrival tick equals its send tick and no
    // counter moves. This is the scenario that goes red if either
    // implementation ever invents delay the authored profile does not ask
    // for.
    Scenario {
        name: "clean_single_slot",
        profile: NetworkProfileName::Clean,
        seed: 20_260_811.0,
        sends: 24,
        slots: 1,
    },
    // Fixed delay, no jitter, a small independent loss rate: the profile
    // whose only impairment is a constant three-tick offset plus the
    // occasional drop.
    Scenario {
        name: "omp0_parity_single_slot",
        profile: NetworkProfileName::Omp0Parity,
        seed: 20_260_811.0,
        sends: 96,
        slots: 1,
    },
    // Jitter wide enough to reorder, plus loss, duplication and bursts.
    Scenario {
        name: "playable_single_slot",
        profile: NetworkProfileName::Playable,
        seed: 4_713.0,
        sends: 160,
        slots: 1,
    },
    Scenario {
        name: "stress_single_slot",
        profile: NetworkProfileName::Stress,
        seed: 20_260_811.0,
        sends: 160,
        slots: 1,
    },
    // Two sources over one link. The RNG and the send sequence are SHARED
    // across sources; only the loss-burst window is per source. A browser
    // star that gave each peer its own generator would diverge here and
    // nowhere else.
    Scenario {
        name: "stress_two_slots",
        profile: NetworkProfileName::Stress,
        seed: 991.0,
        sends: 160,
        slots: 2,
    },
];

// Kept in sync, by assertion, with EXPECTED_TRANSCRIPT in
// ts/packages/transport/src/impairment_parity.spec.ts. See gate 0c.
const EXPECTED_TRANSCRIPT: &str = r"scenario|name=clean_single_slot|profile=clean|seed=20260811|sends=24|slots=1
sends|0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23
deliveries|1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24
counters|sent=24,delivered=24,independent_lost=0,burst_lost=0,duplicated=0,reordered=0
scenario|name=omp0_parity_single_slot|profile=omp0_parity|seed=20260811|sends=96|slots=1
sends|3,4,5,6,7,8,9,10,11,12,13,14,x,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,69,70,x,72,73,74,75,76,77,78,79,80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,96,97,98
deliveries|1,2,3,4,5,6,7,8,9,10,11,12,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,70,71,72,73,74,75,76,77,78,79,80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,96
counters|sent=96,delivered=94,independent_lost=2,burst_lost=0,duplicated=0,reordered=0
scenario|name=playable_single_slot|profile=playable|seed=4713|sends=160|slots=1
sends|1,6,5,5,5,9,11,11,13,13,12,15,14,17,19,19,17,22,23,24,23,22,25,25,25,27,29,32,32,34,35,35,35,36,36,36,38,39,43,40,41,45,45,46,48+,47,48,52,52,53,51,52,55,58,58,60,58,60,62,64,63,66,65,68,67,66,70,68,70,71,71,73,75,74,75,77,77,79,81,80,85,86,86,85,86,86,91,90,93,90,92,96,97,98,99,96,100,101,103,100,101,x,105,105,109,108,108,110,113,111,113,115,116,117,118,120,121,119,122,124,124,123,123,127,128,126,130,130,133,131,133,136,136,137,137,140,140,138,142,143,143,145,143,147,147,147,151,151,151,153,154,156,153,154,158,159,159,159,161,162
deliveries|1,3,4,5,2,6,7,8,11,9,10,13,12,14,17,15,16,18,22,19,21,20,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,40,41,39,42,43,44,46,45,45d,47,51,48,49,52,50,53,54,55,57,56,58,59,61,60,63,62,66,65,64,68,67,69,70,71,72,74,73,75,76,77,78,80,79,81,84,82,83,85,86,88,90,87,91,89,92,96,93,94,95,97,100,98,101,99,103,104,106,107,105,108,110,109,111,112,113,114,115,118,116,117,119,122,123,120,121,126,124,125,127,128,130,129,131,132,133,134,135,138,136,137,139,140,141,143,142,144,145,146,147,148,149,150,153,151,154,152,155,156,157,158,159,160
counters|sent=160,delivered=160,independent_lost=1,burst_lost=0,duplicated=1,reordered=45
scenario|name=stress_single_slot|profile=stress|seed=20260811|sends=160|slots=1
sends|6,6,5,11,12,10,12,12,14,15,14,18,x,22,22,x,21,22,27,28,29,24,26,30,x,30,31,31,34,36,33,37,40,37,41,38,44,b,b,b,43,45,49,49,49,52,52,52,51,52,53,60,56,56,58,58,64,60,66,b,b,b,69,66,71,71,75,73,x,76,75,80,76,81,81,78,83,82,81,88,89,87,89,88,91,93,93,91,94,92,98,97,99,100,103,100,105,103,102,105,107,108,108,109,112,112,115,116,115,117,116,x,119,118,120,118,123,126,127,124,126,130,126,127,129,130,132,135,136,134,138,136,140,140,139,138,144,144,146,143,149,150,150,151,153,154,153,155,156,156,157,159,159,159,163,164,160,164,162,164
deliveries|3,1,2,6,4,5,7,8,9,11,10,12,17,14,15,18,22,23,19,20,21,24,26,27,28,31,29,30,32,34,36,33,35,41,37,42,43,44,45,49,46,47,48,50,51,53,54,55,56,52,58,57,59,64,63,65,66,68,67,71,70,73,76,72,74,75,79,78,77,82,80,84,81,83,85,88,90,86,87,89,92,91,93,94,96,99,95,98,97,100,101,102,103,104,105,106,107,109,108,111,110,114,116,113,115,117,120,118,121,123,119,124,125,122,126,127,130,128,129,132,131,136,135,133,134,140,137,138,139,141,142,143,144,145,147,146,148,149,150,151,152,153,154,157,159,155,156,158,160
counters|sent=160,delivered=149,independent_lost=5,burst_lost=6,duplicated=0,reordered=58
scenario|name=stress_two_slots|profile=stress|seed=991|sends=160|slots=2
sends|3,10,6,9,7,13,12,11,11,18,18,18,16,b,17,b,24,21,25,25,25,27,28,31,29,31,31,34,35,34,34,38,36+,42,38,43,42,46,47,46,47,44,51+,48,52,50,54,56,57,57,58,58,58,56,60,58,59,b,61,b,66,70,70,68,69,b,69,b,73,78,74,80,79,76,x,84,85,83,87,87,89,84,87,89,92,93,90,94,92,92,98,95,98,101,100,102,103,104,104,103,105,109,111,107,113,109,112,112,x,115,119,115,116,122,120,123,123,123,121,127,126,129,127,131,130,128,129,131,136,133,137,139,141,137,141,138+,139,140,144,146,145,144,150,148,152,153,154,154,156,158,159,155,160,157,162,161,159,163,166,168
deliveries|1,3,5,4,2,8,9,7,6,13,15,10,11,12,18,17,19,20,21,22,23,25,24,26,27,28,30,31,29,33,33d,32,35,34,37,36,42,38,40,39,41,44,46,43,43d,45,47,48,54,49,50,51,52,53,56,57,55,59,61,64,65,67,62,63,69,71,74,70,73,72,78,76,82,77,79,80,83,81,84,87,85,89,90,86,88,92,91,93,95,94,96,97,100,98,99,101,104,102,106,103,107,108,105,110,112,113,111,115,119,114,116,117,118,121,120,123,126,122,127,125,124,128,130,129,131,134,136,136d,132,137,138,133,135,139,142,141,140,144,143,145,146,147,148,152,149,154,150,151,157,153,156,155,158,159,160
counters|sent=160,delivered=155,independent_lost=2,burst_lost=6,duplicated=3,reordered=67";

fn conditions_for(scenario: &Scenario) -> NetworkConditions {
    let authored = network_profiles::get(scenario.profile);
    let profile = NetworkProfile::from(authored);
    network_conditions::new(&profile, scenario.seed)
}

/// The transport tick the last packet can still be in flight at: one past
/// the worst case arrival of the final send.
fn drain_tick(scenario: &Scenario) -> i64 {
    let authored = network_profiles::get(scenario.profile);
    scenario.sends + authored.base_delay_ticks + authored.jitter_max_ticks + 1
}

fn run_scenario(scenario: &Scenario) -> String {
    let mut conditions = conditions_for(scenario);
    let mut sends: Vec<String> = Vec::with_capacity(scenario.sends as usize);
    let mut deliveries: Vec<String> = Vec::new();

    let mut record_deliveries = |conditions: &mut NetworkConditions, tick: i64| {
        for delivery in network_conditions::poll(conditions, tick) {
            let suffix = if delivery.duplicate_ordinal == 0 {
                ""
            } else {
                "d"
            };
            deliveries.push(format!("{}{}", delivery.sequence, suffix));
        }
    };

    for index in 0..scenario.sends {
        let source_slot = index % scenario.slots + 1;
        // A distinct sample per send, so a scenario can never accidentally
        // pass by every packet carrying the same bytes.
        let sample = input_frame::new_sample(InputSampleOptions {
            move_x: Some(index % 127),
            move_y: Some(-(index % 63)),
            held: Some(index % 8),
            edges: Some(index % 4),
        })
        .expect("scripted impairment sample is valid");
        // Per slot, the input tick advances once per full slot rotation, so
        // it never precedes that slot's retained authority.
        let input_tick = index / scenario.slots;
        let receipt =
            network_conditions::send(&mut conditions, index, source_slot, input_tick, &sample)
                .expect("scripted impairment send is valid");
        sends.push(match (receipt.dropped, receipt.drop_reason) {
            (true, Some(network_conditions::NetworkDropReason::BurstLoss)) => "b".to_string(),
            (true, Some(network_conditions::NetworkDropReason::IndependentLoss)) => "x".to_string(),
            (true, None) => panic!("a dropped packet must carry a drop reason"),
            (false, _) => {
                let arrival = receipt
                    .arrival_tick
                    .expect("a delivered packet must carry an arrival tick");
                format!("{}{}", arrival, if receipt.duplicated { "+" } else { "" })
            }
        });
        record_deliveries(&mut conditions, index);
    }
    record_deliveries(&mut conditions, drain_tick(scenario));

    let counters = network_conditions::counters(&conditions);
    assert_eq!(
        network_conditions::pending(&conditions),
        0,
        "{}: the drain tick left packets in flight, so the transcript is incomplete",
        scenario.name
    );

    let authored_name = match scenario.profile {
        NetworkProfileName::Clean => "clean",
        NetworkProfileName::Omp0Parity => "omp0_parity",
        NetworkProfileName::Playable => "playable",
        NetworkProfileName::Stress => "stress",
    };
    format!(
        "scenario|name={}|profile={}|seed={}|sends={}|slots={}\n\
         sends|{}\n\
         deliveries|{}\n\
         counters|sent={},delivered={},independent_lost={},burst_lost={},duplicated={},reordered={}",
        scenario.name,
        authored_name,
        scenario.seed as i64,
        scenario.sends,
        scenario.slots,
        sends.join(","),
        deliveries.join(","),
        counters.sent,
        counters.delivered,
        counters.independent_lost,
        counters.burst_lost,
        counters.duplicated,
        counters.reordered,
    )
}

fn transcript() -> String {
    SCENARIOS
        .iter()
        .map(run_scenario)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn native_impairment_transcript_matches_the_shared_fixture() {
    let actual = transcript();
    if actual != EXPECTED_TRANSCRIPT {
        println!("----- ACTUAL TRANSCRIPT -----\n{actual}\n----- END -----");
    }
    assert_eq!(
        actual, EXPECTED_TRANSCRIPT,
        "the native impairment transcript drifted; the browser mirror asserts the same literal"
    );
}
