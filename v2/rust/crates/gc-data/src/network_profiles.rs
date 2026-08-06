//! Simulated network conditions for online testing.

/// A named network condition profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkProfileName {
    /// No delay, jitter, or loss.
    Clean,
    /// OMP0-parity: fixed delay plus a small independent loss rate.
    Omp0Parity,
    /// Realistic playable conditions: delay, jitter, loss, duplication, bursts.
    Playable,
    /// Stress conditions: heavier delay, jitter, loss, duplication, bursts.
    Stress,
}

/// A simulated network condition profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NetworkProfile {
    /// Which named profile this is; also the lookup key.
    pub name: NetworkProfileName,
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

/// Every authored network profile.
pub static ALL: &[NetworkProfile] = &[
    NetworkProfile {
        name: NetworkProfileName::Clean,
        base_delay_ticks: 0,
        jitter_min_ticks: 0,
        jitter_max_ticks: 0,
        independent_loss_rate: 0.0,
        duplication_rate: 0.0,
        burst_start_rate: 0.0,
        burst_length_ticks: 0,
    },
    NetworkProfile {
        name: NetworkProfileName::Omp0Parity,
        base_delay_ticks: 3,
        jitter_min_ticks: 0,
        jitter_max_ticks: 0,
        independent_loss_rate: 0.01,
        duplication_rate: 0.0,
        burst_start_rate: 0.0,
        burst_length_ticks: 0,
    },
    NetworkProfile {
        name: NetworkProfileName::Playable,
        base_delay_ticks: 3,
        jitter_min_ticks: -2,
        jitter_max_ticks: 2,
        independent_loss_rate: 0.01,
        duplication_rate: 0.0025,
        burst_start_rate: 0.0025,
        burst_length_ticks: 3,
    },
    NetworkProfile {
        name: NetworkProfileName::Stress,
        base_delay_ticks: 6,
        jitter_min_ticks: -3,
        jitter_max_ticks: 3,
        independent_loss_rate: 0.03,
        duplication_rate: 0.01,
        burst_start_rate: 0.01,
        burst_length_ticks: 3,
    },
];

/// Look up a network profile by name.
pub fn get(name: NetworkProfileName) -> &'static NetworkProfile {
    ALL.iter()
        .find(|profile| profile.name == name)
        .expect("every NetworkProfileName has a record")
}
