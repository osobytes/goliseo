//! Deterministic, in-process packet impairment for rollback laboratory runs.
//! Transport time is an integer tick owned by the caller and is deliberately
//! separate from the input tick carried by each authoritative sample.
//!
//! Slot identity (`source_slot`, matching [`crate::input_frame::SLOT_COUNT`]'s
//! one-based `1..=8` domain) and RNG-consumption order are protocol-facing —
//! two peers replaying the same sends must schedule byte-identical
//! deliveries — so both stay exactly as the original Lua source defined them
//! rather than converting to 0-based internal indexing (ARCHITECTURE.md §3
//! rule 3's wire exception). This module is differential-tested against reference
//! vectors captured from the Lua implementation this simulation was
//! originally validated against (see `tools/lua_reference`).
//!
//! `_records`/`_pending_references`/`_delivered_fingerprints` were Lua tables
//! keyed by `source_slot` (`1..=8`) or by `input_tick`. Per-slot state here
//! uses a fixed 8-element `Vec` indexed by `source_slot - 1` (deterministic,
//! `O(1)`, and not a hash map); per-tick state within a slot uses a small
//! linear-scan `Vec<(tick, value)>`, mirroring that Lua table's lookup
//! shape without `HashMap` (clippy denies it workspace-wide).

use crate::input_frame::{self, InputSample};
use gc_core::rng;

/// Redundant history rows retained per authoritative sample.
pub const HISTORY_RECORDS: usize = 6;
/// Authoritative rows retained per source slot (history + current).
pub const RETAINED_RECORDS: usize = HISTORY_RECORDS + 1;
/// Largest representable transport tick.
pub const MAX_TRANSPORT_TICK: i64 = 2_147_483_647;

const AXIS_CARDINALITY: i64 = input_frame::MOVE_SCALE * 2 + 1;
const HELD_CARDINALITY: i64 = 256;
const EDGE_CARDINALITY: i64 = 128;

/// Failure reasons a [`NetworkConditions`] operation can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkConditionErrorCode {
    /// The call's arguments violate a structural or range invariant.
    Malformed,
    /// A resend/send disagrees with an already-retained authoritative
    /// sample for the same slot and input tick.
    ConflictingAuthoritative,
    /// A send's input tick precedes the slot's retained authority.
    StaleAuthoritative,
    /// A resend/drain names an input tick outside retained history.
    NotRetained,
}

/// Why a scheduled packet was dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkDropReason {
    /// Dropped by the profile's independent per-packet loss rate.
    IndependentLoss,
    /// Dropped by an active loss burst.
    BurstLoss,
}

/// An expected, recoverable network-conditions failure (ARCHITECTURE.md §3 rule 5):
/// the caller is meant to handle it, not a programmer error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkConditionError {
    /// Machine-readable failure reason.
    pub code: NetworkConditionErrorCode,
    /// Human-readable detail.
    pub message: String,
}

impl NetworkConditionError {
    fn new(code: NetworkConditionErrorCode, message: impl Into<String>) -> Self {
        NetworkConditionError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for NetworkConditionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for NetworkConditionError {}

/// Result alias for fallible network-conditions operations.
pub type Result<T> = std::result::Result<T, NetworkConditionError>;

fn failure<T>(code: NetworkConditionErrorCode, message: impl Into<String>) -> Result<T> {
    Err(NetworkConditionError::new(code, message))
}

/// A simulated network condition profile. Distinct from
/// `gc_data::network_profiles::NetworkProfile`, which additionally carries a
/// `name` used only for the authored profile registry; this is the bare
/// tuning shape `sim/network_conditions.lua`'s own `---@class NetworkProfile`
/// declared (no `name` field), so ad hoc profiles (as the differential tests
/// and several spec cases build) don't need to invent one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NetworkProfile {
    /// Fixed delay, in ticks.
    pub base_delay_ticks: i64,
    /// Minimum jitter added to the delay, in ticks.
    pub jitter_min_ticks: i64,
    /// Maximum jitter added to the delay, in ticks.
    pub jitter_max_ticks: i64,
    /// Probability a packet is independently lost.
    pub independent_loss_rate: f64,
    /// Probability a packet is duplicated.
    pub duplication_rate: f64,
    /// Probability a loss burst starts on a given tick.
    pub burst_start_rate: f64,
    /// Length of a loss burst, in ticks.
    pub burst_length_ticks: i64,
}

impl From<&gc_data::network_profiles::NetworkProfile> for NetworkProfile {
    fn from(profile: &gc_data::network_profiles::NetworkProfile) -> Self {
        NetworkProfile {
            base_delay_ticks: profile.base_delay_ticks,
            jitter_min_ticks: profile.jitter_min_ticks,
            jitter_max_ticks: profile.jitter_max_ticks,
            independent_loss_rate: profile.independent_loss_rate,
            duplication_rate: profile.duplication_rate,
            burst_start_rate: profile.burst_start_rate,
            burst_length_ticks: profile.burst_length_ticks,
        }
    }
}

fn assert_profile(profile: &NetworkProfile) {
    assert!(
        profile.base_delay_ticks >= 0,
        "network base delay must be a non-negative integer"
    );
    assert!(
        profile.jitter_min_ticks <= profile.jitter_max_ticks,
        "network jitter bounds are reversed"
    );
    assert!(
        (0.0..=1.0).contains(&profile.independent_loss_rate),
        "network loss rate must be in [0, 1]"
    );
    assert!(
        (0.0..=1.0).contains(&profile.duplication_rate),
        "network duplication rate must be in [0, 1]"
    );
    assert!(
        (0.0..=1.0).contains(&profile.burst_start_rate),
        "network burst rate must be in [0, 1]"
    );
    assert!(
        profile.burst_length_ticks >= 0,
        "network burst length must be a non-negative integer"
    );
    assert!(
        (profile.burst_start_rate == 0.0 && profile.burst_length_ticks == 0)
            || (profile.burst_start_rate > 0.0 && profile.burst_length_ticks > 0),
        "network burst rate and length must both be disabled or enabled"
    );
}

/// One authoritative sample retained for a source slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NetworkInputRecord {
    /// Input tick this sample belongs to.
    pub tick: i64,
    /// The authoritative sample.
    pub sample: InputSample,
}

/// One scheduled, possibly impaired, delivery.
#[derive(Clone, Debug, PartialEq)]
pub struct NetworkDelivery {
    /// Source slot (`1..=8`).
    pub source_slot: i64,
    /// Transport tick the packet was sent on.
    pub send_tick: i64,
    /// Monotonic per-conditions send sequence number.
    pub sequence: i64,
    /// Zero for the original delivery, one for its duplicate.
    pub duplicate_ordinal: i64,
    /// Transport tick the packet arrives on.
    pub arrival_tick: i64,
    /// The current (newest) authoritative record this packet carries.
    pub current: NetworkInputRecord,
    /// Redundant earlier records this packet carries, oldest first. At most
    /// [`HISTORY_RECORDS`] entries.
    pub history: Vec<NetworkInputRecord>,
}

/// The outcome of one `send`/`resend` call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NetworkSendReceipt {
    /// This send's sequence number.
    pub sequence: i64,
    /// Whether the packet was dropped.
    pub dropped: bool,
    /// Why the packet was dropped, if it was.
    pub drop_reason: Option<NetworkDropReason>,
    /// Transport tick the packet arrives on, if it was not dropped.
    pub arrival_tick: Option<i64>,
    /// Whether an impairment-created duplicate was also scheduled.
    pub duplicated: bool,
    /// Whether this send carried an already-retained sample (a `resend`, or
    /// a `send` whose sample exactly matched an existing retained record).
    pub authoritative_duplicate: bool,
}

/// Running counters since [`new`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct NetworkConditionCounters {
    /// Source packets sent, excluding impairment-created duplicates.
    pub sent: i64,
    /// Delivered envelopes, including duplicates.
    pub delivered: i64,
    /// Packets dropped by independent loss.
    pub independent_lost: i64,
    /// Packets dropped by an active burst.
    pub burst_lost: i64,
    /// Duplicate envelopes scheduled for delivery.
    pub duplicated: i64,
    /// Unique sequence identities delivered after a later sequence.
    pub reordered: i64,
    /// First-seen samples recovered from redundant history.
    pub history_recovered: i64,
}

/// Current and peak retained-state sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct NetworkConditionDiagnostics {
    /// Currently retained authoritative records, across all slots.
    pub retained_authoritative_records: i64,
    /// Currently tracked delivered-fingerprint ledger entries.
    pub delivered_ledger_entries: i64,
    /// Currently pending (in-flight) envelopes.
    pub pending_envelopes: i64,
    /// Currently pending history/current record references.
    pub pending_record_references: i64,
    /// High-water mark of `retained_authoritative_records`.
    pub peak_retained_authoritative_records: i64,
    /// High-water mark of `delivered_ledger_entries`.
    pub peak_delivered_ledger_entries: i64,
    /// High-water mark of `pending_envelopes`.
    pub peak_pending_envelopes: i64,
    /// High-water mark of `pending_record_references`.
    pub peak_pending_record_references: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct HighWater {
    retained_authoritative_records: i64,
    delivered_ledger_entries: i64,
    pending_envelopes: i64,
    pending_record_references: i64,
}

/// A request to guarantee delivery of one already-sent input, used by
/// [`drain`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkResendRequest {
    /// Source slot (`1..=8`).
    pub source_slot: i64,
    /// Input tick to guarantee delivery of.
    pub input_tick: i64,
}

/// The outcome of a [`drain`] call.
#[derive(Clone, Debug, PartialEq)]
pub struct NetworkDrainResult {
    /// Every envelope delivered during the drain, in delivery order.
    pub deliveries: Vec<NetworkDelivery>,
    /// The last transport tick the drain advanced to.
    pub final_tick: i64,
    /// Whether every requested input was recovered and nothing remains
    /// pending.
    pub complete: bool,
    /// Pending envelope count at the end of the drain.
    pub pending: i64,
    /// Requested inputs recovered by the end of the drain.
    pub recovered: i64,
    /// Requested inputs total.
    pub requested: i64,
}

/// Per-slot record of a delivered sample's collision-free identity, used to
/// detect conflicting redelivery and to know when a requested resend has
/// landed.
type DeliveredFingerprints = Vec<(i64 /* input_tick */, i64 /* fingerprint */)>;
/// Per-slot pending-reference counts, keyed by input tick.
type PendingReferences = Vec<(i64 /* input_tick */, i64 /* refcount */)>;

/// Deterministic, in-process packet impairment for one simulated link.
#[derive(Clone, Debug, PartialEq)]
pub struct NetworkConditions {
    profile: NetworkProfile,
    rng_state: u32,
    sequence: i64,
    clock_tick: i64,
    /// Authoritative records retained per source slot (index `slot - 1`).
    records: Vec<Vec<NetworkInputRecord>>,
    pending: Vec<NetworkDelivery>,
    /// Pending record references per source slot (index `slot - 1`).
    pending_references: Vec<PendingReferences>,
    /// Tick a slot's active burst runs until (`-1` when none), per source
    /// slot (index `slot - 1`).
    burst_until: Vec<i64>,
    /// Delivered fingerprints per source slot (index `slot - 1`).
    delivered_fingerprints: Vec<DeliveredFingerprints>,
    max_delivered_sequence: i64,
    counters: NetworkConditionCounters,
    high_water: HighWater,
}

fn is_source_slot(source_slot: i64) -> bool {
    (1..=input_frame::SLOT_COUNT).contains(&source_slot)
}

fn is_input_tick(tick: i64) -> bool {
    (0..=input_frame::MAX_TICK).contains(&tick)
}

fn is_transport_tick(tick: i64) -> bool {
    (0..=MAX_TRANSPORT_TICK).contains(&tick)
}

fn slot_ix(source_slot: i64) -> usize {
    (source_slot - 1) as usize
}

fn samples_equal(left: &InputSample, right: &InputSample) -> bool {
    left.move_x == right.move_x
        && left.move_y == right.move_y
        && left.held == right.held
        && left.edges == right.edges
}

fn jitter_from_roll(profile: &NetworkProfile, roll: f64) -> i64 {
    let width = profile.jitter_max_ticks - profile.jitter_min_ticks + 1;
    profile.jitter_min_ticks + (roll * width as f64).floor() as i64
}

fn maximum_arrival_tick(profile: &NetworkProfile, send_tick: i64) -> i64 {
    (send_tick + profile.base_delay_ticks + profile.jitter_max_ticks).max(send_tick)
}

fn arrival_fits(profile: &NetworkProfile, send_tick: i64) -> bool {
    maximum_arrival_tick(profile, send_tick) <= MAX_TRANSPORT_TICK
}

// This is a collision-free mixed-radix encoding, not a hash. InputSample's
// declared bounds keep the result below 2^53 on every supported runtime.
fn sample_fingerprint(sample: &InputSample) -> i64 {
    let mut packed = sample.move_x + input_frame::MOVE_SCALE;
    packed = packed * AXIS_CARDINALITY + sample.move_y + input_frame::MOVE_SCALE;
    packed = packed * HELD_CARDINALITY + sample.held;
    packed * EDGE_CARDINALITY + sample.edges
}

fn find_record_index(records: &[NetworkInputRecord], input_tick: i64) -> Option<usize> {
    records.iter().position(|r| r.tick == input_tick)
}

impl NetworkConditions {
    fn find_record(&self, source_slot: i64, input_tick: i64) -> Option<&NetworkInputRecord> {
        let records = &self.records[slot_ix(source_slot)];
        find_record_index(records, input_tick).map(|i| &records[i])
    }

    fn diagnostic_counts(&self) -> HighWater {
        let mut retained_authoritative_records = 0_i64;
        for records in &self.records {
            retained_authoritative_records += records.len() as i64;
        }
        let mut delivered_ledger_entries = 0_i64;
        for delivered in &self.delivered_fingerprints {
            delivered_ledger_entries += delivered.len() as i64;
        }
        let mut pending_record_references = 0_i64;
        for references in &self.pending_references {
            for &(_, count) in references {
                pending_record_references += count;
            }
        }
        HighWater {
            retained_authoritative_records,
            delivered_ledger_entries,
            pending_envelopes: self.pending.len() as i64,
            pending_record_references,
        }
    }

    fn update_high_water(&mut self) {
        let current = self.diagnostic_counts();
        self.high_water.retained_authoritative_records = self
            .high_water
            .retained_authoritative_records
            .max(current.retained_authoritative_records);
        self.high_water.delivered_ledger_entries = self
            .high_water
            .delivered_ledger_entries
            .max(current.delivered_ledger_entries);
        self.high_water.pending_envelopes = self
            .high_water
            .pending_envelopes
            .max(current.pending_envelopes);
        self.high_water.pending_record_references = self
            .high_water
            .pending_record_references
            .max(current.pending_record_references);
    }

    fn adjust_pending_references(&mut self, delivery: &NetworkDelivery, delta: i64) {
        let references = &mut self.pending_references[slot_ix(delivery.source_slot)];
        let mut bump = |tick: i64| match references.iter_mut().find(|(t, _)| *t == tick) {
            Some((_, count)) => {
                *count += delta;
                assert!(*count >= 0, "network pending history reference underflow");
            }
            None => {
                assert!(delta >= 0, "network pending history reference underflow");
                references.push((tick, delta));
            }
        };
        for record in &delivery.history {
            bump(record.tick);
        }
        bump(delivery.current.tick);
        references.retain(|(_, count)| *count != 0);
    }

    fn record_is_retained(&self, source_slot: i64, input_tick: i64) -> bool {
        self.find_record(source_slot, input_tick).is_some()
    }

    // Delivered identities remain only while the corresponding authority is
    // retained or a pending envelope can still repeat it.
    fn prune_delivered_ledger(&mut self) {
        for source_slot in 1..=input_frame::SLOT_COUNT {
            let retained_now: Vec<i64> = {
                let delivered = &self.delivered_fingerprints[slot_ix(source_slot)];
                delivered.iter().map(|(tick, _)| *tick).collect()
            };
            let mut to_remove = Vec::new();
            for input_tick in retained_now {
                let still_pending = self.pending_references[slot_ix(source_slot)]
                    .iter()
                    .any(|(t, _)| *t == input_tick);
                if !self.record_is_retained(source_slot, input_tick) && !still_pending {
                    to_remove.push(input_tick);
                }
            }
            let delivered = &mut self.delivered_fingerprints[slot_ix(source_slot)];
            delivered.retain(|(tick, _)| !to_remove.contains(tick));
        }
    }

    fn packet_history(&self, source_slot: i64, input_tick: i64) -> Vec<NetworkInputRecord> {
        let records = &self.records[slot_ix(source_slot)];
        let current_index = find_record_index(records, input_tick)
            .expect("packet_history requires a retained current record");
        let first = current_index.saturating_sub(HISTORY_RECORDS);
        records[first..current_index].to_vec()
    }

    fn retain_authoritative(
        &mut self,
        source_slot: i64,
        input_tick: i64,
        sample: &InputSample,
    ) -> Result<bool> {
        let records = &mut self.records[slot_ix(source_slot)];
        if let Some(existing_index) = find_record_index(records, input_tick) {
            if !samples_equal(&records[existing_index].sample, sample) {
                return failure(
                    NetworkConditionErrorCode::ConflictingAuthoritative,
                    format!("network input conflicts at tick {input_tick} slot {source_slot}"),
                );
            }
            return Ok(true);
        }

        if let Some(latest) = records.last()
            && input_tick < latest.tick
        {
            return failure(
                NetworkConditionErrorCode::StaleAuthoritative,
                format!(
                    "network input tick {input_tick} precedes retained slot {source_slot} authority"
                ),
            );
        }

        records.push(NetworkInputRecord {
            tick: input_tick,
            sample: *sample,
        });
        if records.len() > RETAINED_RECORDS {
            records.remove(0);
        }
        Ok(false)
    }

    fn impairment_rolls(&mut self) -> (f64, f64, f64, f64) {
        let (state, jitter_roll) = rng::roll(self.rng_state);
        let (state, loss_roll) = rng::roll(state);
        let (state, duplicate_roll) = rng::roll(state);
        let (state, burst_roll) = rng::roll(state);
        self.rng_state = state;
        (jitter_roll, loss_roll, duplicate_roll, burst_roll)
    }

    fn schedule_packet(
        &mut self,
        source_slot: i64,
        send_tick: i64,
        input_tick: i64,
        authoritative_duplicate: bool,
    ) -> NetworkSendReceipt {
        self.sequence += 1;
        self.counters.sent += 1;

        let (jitter_roll, loss_roll, duplicate_roll, burst_roll) = self.impairment_rolls();

        let profile = self.profile;
        let burst_until = self.burst_until[slot_ix(source_slot)];
        let active_burst = send_tick <= burst_until;
        let mut started_burst = false;
        if !active_burst && burst_roll < profile.burst_start_rate {
            started_burst = true;
            self.burst_until[slot_ix(source_slot)] = send_tick + profile.burst_length_ticks - 1;
        }

        let sequence = self.sequence;
        if active_burst || started_burst {
            self.counters.burst_lost += 1;
            return NetworkSendReceipt {
                sequence,
                dropped: true,
                drop_reason: Some(NetworkDropReason::BurstLoss),
                arrival_tick: None,
                duplicated: false,
                authoritative_duplicate,
            };
        }
        if loss_roll < profile.independent_loss_rate {
            self.counters.independent_lost += 1;
            return NetworkSendReceipt {
                sequence,
                dropped: true,
                drop_reason: Some(NetworkDropReason::IndependentLoss),
                arrival_tick: None,
                duplicated: false,
                authoritative_duplicate,
            };
        }

        let jitter = jitter_from_roll(&profile, jitter_roll);
        let arrival_tick = (send_tick + profile.base_delay_ticks + jitter).max(send_tick);
        let current = *self
            .find_record(source_slot, input_tick)
            .expect("schedule_packet requires a retained current record");
        let delivery = NetworkDelivery {
            source_slot,
            send_tick,
            sequence,
            duplicate_ordinal: 0,
            arrival_tick,
            current,
            history: self.packet_history(source_slot, input_tick),
        };
        self.pending.push(delivery.clone());
        self.adjust_pending_references(&delivery, 1);

        let duplicated = duplicate_roll < profile.duplication_rate;
        if duplicated {
            let mut duplicate = delivery.clone();
            duplicate.duplicate_ordinal = 1;
            self.pending.push(duplicate.clone());
            self.adjust_pending_references(&duplicate, 1);
            self.counters.duplicated += 1;
        }
        self.update_high_water();

        NetworkSendReceipt {
            sequence,
            dropped: false,
            drop_reason: None,
            arrival_tick: Some(arrival_tick),
            duplicated,
            authoritative_duplicate,
        }
    }

    fn validate_send(
        &self,
        send_tick: i64,
        source_slot: i64,
        input_tick: i64,
        sample: &InputSample,
    ) -> Result<()> {
        if !is_transport_tick(send_tick) || send_tick < self.clock_tick {
            return failure(
                NetworkConditionErrorCode::Malformed,
                "network send tick must be bounded and monotonic",
            );
        }
        if !arrival_fits(&self.profile, send_tick) {
            return failure(
                NetworkConditionErrorCode::Malformed,
                "network send can arrive beyond the transport tick limit",
            );
        }
        if !is_source_slot(source_slot) {
            return failure(
                NetworkConditionErrorCode::Malformed,
                "network source slot must be between one and eight",
            );
        }
        if !is_input_tick(input_tick) {
            return failure(
                NetworkConditionErrorCode::Malformed,
                "network input tick must be a bounded non-negative integer",
            );
        }
        input_frame::validate_sample(sample).map_err(|e| {
            NetworkConditionError::new(NetworkConditionErrorCode::Malformed, e.message)
        })?;
        Ok(())
    }
}

/// Construct a fresh conditions state for one simulated link.
///
/// # Panics
///
/// Panics if `profile` violates a bound invariant, or `seed` is not finite —
/// these are authored/programmer errors (AGENTS.md §7), not recoverable
/// runtime conditions.
#[must_use]
pub fn new(profile: &NetworkProfile, seed: f64) -> NetworkConditions {
    assert_profile(profile);
    assert!(seed.is_finite(), "network seed must be finite");
    NetworkConditions {
        profile: *profile,
        rng_state: rng::seed(seed),
        sequence: 0,
        clock_tick: -1,
        records: vec![Vec::new(); input_frame::SLOT_COUNT as usize],
        pending: Vec::new(),
        pending_references: vec![Vec::new(); input_frame::SLOT_COUNT as usize],
        burst_until: vec![-1; input_frame::SLOT_COUNT as usize],
        delivered_fingerprints: vec![Vec::new(); input_frame::SLOT_COUNT as usize],
        max_delivered_sequence: 0,
        counters: NetworkConditionCounters::default(),
        high_water: HighWater::default(),
    }
}

/// Retain a new authoritative sample (or accept its identical duplicate),
/// then schedule one source packet. Every call consumes jitter,
/// independent-loss, duplication, and burst-start rolls in that order,
/// including dropped packets.
pub fn send(
    conditions: &mut NetworkConditions,
    send_tick: i64,
    source_slot: i64,
    input_tick: i64,
    sample: &InputSample,
) -> Result<NetworkSendReceipt> {
    conditions.validate_send(send_tick, source_slot, input_tick, sample)?;
    let duplicate = conditions.retain_authoritative(source_slot, input_tick, sample)?;
    conditions.update_high_water();
    conditions.clock_tick = send_tick;
    let receipt = conditions.schedule_packet(source_slot, send_tick, input_tick, duplicate);
    conditions.prune_delivered_ledger();
    Ok(receipt)
}

/// Schedule an already-retained sample without adding another input-history
/// row.
pub fn resend(
    conditions: &mut NetworkConditions,
    send_tick: i64,
    source_slot: i64,
    input_tick: i64,
) -> Result<NetworkSendReceipt> {
    if !is_transport_tick(send_tick) || send_tick < conditions.clock_tick {
        return failure(
            NetworkConditionErrorCode::Malformed,
            "network resend tick must be bounded and monotonic",
        );
    }
    if !arrival_fits(&conditions.profile, send_tick) {
        return failure(
            NetworkConditionErrorCode::Malformed,
            "network resend can arrive beyond the transport tick limit",
        );
    }
    if !is_source_slot(source_slot) || !is_input_tick(input_tick) {
        return failure(
            NetworkConditionErrorCode::Malformed,
            "network resend slot and input tick are invalid",
        );
    }
    if conditions.find_record(source_slot, input_tick).is_none() {
        return failure(
            NetworkConditionErrorCode::NotRetained,
            "network resend input is outside retained history",
        );
    }
    conditions.clock_tick = send_tick;
    let receipt = conditions.schedule_packet(source_slot, send_tick, input_tick, true);
    conditions.prune_delivered_ledger();
    Ok(receipt)
}

fn record_delivery(conditions: &mut NetworkConditions, delivery: &NetworkDelivery) {
    conditions.counters.delivered += 1;
    if delivery.duplicate_ordinal == 0 {
        if delivery.sequence < conditions.max_delivered_sequence {
            conditions.counters.reordered += 1;
        }
        conditions.max_delivered_sequence =
            conditions.max_delivered_sequence.max(delivery.sequence);
    }

    // `history_recovered` counts only first-seen REDUNDANT (history) rows,
    // not the delivery's own current row — matching the Lua source's two
    // separate loops (a `for` over `delivery.history` that increments the
    // counter, then one unconditional check against `delivery.current` that
    // does not).
    let delivered = &mut conditions.delivered_fingerprints[slot_ix(delivery.source_slot)];
    for record in &delivery.history {
        let fingerprint = sample_fingerprint(&record.sample);
        match delivered.iter().find(|(tick, _)| *tick == record.tick) {
            Some((_, existing)) => {
                assert!(
                    *existing == fingerprint,
                    "network history delivered conflicting authority"
                );
            }
            None => {
                delivered.push((record.tick, fingerprint));
                conditions.counters.history_recovered += 1;
            }
        }
    }
    let fingerprint = sample_fingerprint(&delivery.current.sample);
    let delivered = &mut conditions.delivered_fingerprints[slot_ix(delivery.source_slot)];
    match delivered
        .iter()
        .find(|(tick, _)| *tick == delivery.current.tick)
    {
        Some((_, existing)) => {
            assert!(
                *existing == fingerprint,
                "network current delivery conflicts with prior authority"
            );
        }
        None => {
            delivered.push((delivery.current.tick, fingerprint));
        }
    }
}

fn delivery_less(left: &NetworkDelivery, right: &NetworkDelivery) -> std::cmp::Ordering {
    left.arrival_tick
        .cmp(&right.arrival_tick)
        .then(left.sequence.cmp(&right.sequence))
        .then(left.duplicate_ordinal.cmp(&right.duplicate_ordinal))
}

/// Return every envelope due at or before the monotonic transport tick.
/// Equal arrivals use `(arrival_tick, sequence, duplicate_ordinal)`
/// ordering.
///
/// # Panics
///
/// Panics if `delivery_tick` is not a bounded, monotonic transport tick —
/// callers own driving transport ticks forward (AGENTS.md §7).
pub fn poll(conditions: &mut NetworkConditions, delivery_tick: i64) -> Vec<NetworkDelivery> {
    assert!(
        is_transport_tick(delivery_tick) && delivery_tick >= conditions.clock_tick,
        "network poll tick must be bounded and monotonic"
    );
    conditions.clock_tick = delivery_tick;

    let mut due = Vec::new();
    let mut still_pending = Vec::new();
    for delivery in std::mem::take(&mut conditions.pending) {
        if delivery.arrival_tick <= delivery_tick {
            due.push(delivery);
        } else {
            still_pending.push(delivery);
        }
    }
    conditions.pending = still_pending;
    for delivery in &due {
        conditions.adjust_pending_references(delivery, -1);
    }
    due.sort_by(delivery_less);

    let mut result = Vec::with_capacity(due.len());
    for delivery in &due {
        record_delivery(conditions, delivery);
        result.push(delivery.clone());
    }
    conditions.update_high_water();
    conditions.prune_delivered_ledger();
    result
}

/// Count of currently pending (in-flight) envelopes.
#[must_use]
pub fn pending(conditions: &NetworkConditions) -> i64 {
    conditions.pending.len() as i64
}

/// Running counters since [`new`].
#[must_use]
pub fn counters(conditions: &NetworkConditions) -> NetworkConditionCounters {
    conditions.counters
}

/// Current and peak retained-state sizes.
#[must_use]
pub fn diagnostics(conditions: &NetworkConditions) -> NetworkConditionDiagnostics {
    let current = conditions.diagnostic_counts();
    let peak = conditions.high_water;
    NetworkConditionDiagnostics {
        retained_authoritative_records: current.retained_authoritative_records,
        delivered_ledger_entries: current.delivered_ledger_entries,
        pending_envelopes: current.pending_envelopes,
        pending_record_references: current.pending_record_references,
        peak_retained_authoritative_records: peak.retained_authoritative_records,
        peak_delivered_ledger_entries: peak.delivered_ledger_entries,
        peak_pending_envelopes: peak.pending_envelopes,
        peak_pending_record_references: peak.pending_record_references,
    }
}

/// Return the collision-free packed diagnostic identity used by the bounded
/// delivered ledger. This is not a hash or a production wire encoding.
pub fn sample_key(sample: &InputSample) -> Result<i64> {
    input_frame::validate_sample(sample)
        .map_err(|e| NetworkConditionError::new(NetworkConditionErrorCode::Malformed, e.message))?;
    Ok(sample_fingerprint(sample))
}

/// Return a delivery's redundant rows followed by its current row. The
/// result is a defensive copy, order suitable for `rollback_input_history`'s
/// `add_authoritative`.
#[must_use]
pub fn records(delivery: &NetworkDelivery) -> Vec<NetworkInputRecord> {
    let mut records = delivery.history.clone();
    records.push(delivery.current);
    records
}

fn request_delivered(conditions: &NetworkConditions, request: NetworkResendRequest) -> bool {
    conditions.delivered_fingerprints[slot_ix(request.source_slot)]
        .iter()
        .any(|(tick, _)| *tick == request.input_tick)
}

fn requests_complete(conditions: &NetworkConditions, requests: &[NetworkResendRequest]) -> bool {
    requests.iter().all(|&r| request_delivered(conditions, r))
}

fn validated_requests(requests: &[NetworkResendRequest]) -> Result<Vec<NetworkResendRequest>> {
    let mut copied = Vec::with_capacity(requests.len());
    let mut seen: Vec<(i64, i64)> = Vec::with_capacity(requests.len());
    for request in requests {
        if !is_source_slot(request.source_slot) || !is_input_tick(request.input_tick) {
            return failure(
                NetworkConditionErrorCode::Malformed,
                "network drain request is invalid",
            );
        }
        let identity = (request.source_slot, request.input_tick);
        if seen.contains(&identity) {
            return failure(
                NetworkConditionErrorCode::Malformed,
                "network drain requests must be unique",
            );
        }
        seen.push(identity);
        copied.push(*request);
    }
    copied.sort_by(|a, b| {
        a.source_slot
            .cmp(&b.source_slot)
            .then(a.input_tick.cmp(&b.input_tick))
    });
    Ok(copied)
}

/// Advance only transport ticks. Missing requested samples are resent once
/// per transport tick until observed; after recovery, pending redundant
/// packets are polled without further sends. Match simulation never
/// advances here.
pub fn drain(
    conditions: &mut NetworkConditions,
    start_tick: i64,
    max_ticks: i64,
    requests: &[NetworkResendRequest],
) -> Result<NetworkDrainResult> {
    if !is_transport_tick(start_tick)
        || start_tick < conditions.clock_tick
        || max_ticks < 1
        || start_tick + max_ticks - 1 > MAX_TRANSPORT_TICK
    {
        return failure(
            NetworkConditionErrorCode::Malformed,
            "network drain tick range must be bounded and monotonic",
        );
    }
    if !arrival_fits(&conditions.profile, start_tick + max_ticks - 1) {
        return failure(
            NetworkConditionErrorCode::Malformed,
            "network drain can arrive beyond the transport tick limit",
        );
    }
    let sorted = validated_requests(requests)?;
    for request in &sorted {
        if conditions
            .find_record(request.source_slot, request.input_tick)
            .is_none()
        {
            return failure(
                NetworkConditionErrorCode::NotRetained,
                "network drain request is outside retained history",
            );
        }
    }

    let mut deliveries = Vec::new();
    let mut final_tick = start_tick;
    for offset in 0..max_ticks {
        final_tick = start_tick + offset;
        let before = poll(conditions, final_tick);
        deliveries.extend(before);

        if !requests_complete(conditions, &sorted) {
            for &request in &sorted {
                if !request_delivered(conditions, request) {
                    resend(
                        conditions,
                        final_tick,
                        request.source_slot,
                        request.input_tick,
                    )
                    .expect("resend of a validated, retained request cannot fail");
                }
            }
            let immediate = poll(conditions, final_tick);
            deliveries.extend(immediate);
        }

        if requests_complete(conditions, &sorted) && pending(conditions) == 0 {
            break;
        }
    }

    let recovered = sorted
        .iter()
        .filter(|&&r| request_delivered(conditions, r))
        .count() as i64;
    Ok(NetworkDrainResult {
        deliveries,
        final_tick,
        complete: recovered == sorted.len() as i64 && pending(conditions) == 0,
        pending: pending(conditions),
        recovered,
        requested: sorted.len() as i64,
    })
}
