//! The benchmark workload: deterministic work units of a fixed size.
//!
//! One unit runs three kernels - integer hashing, a floating-point n-body
//! simulation and scattered updates of a table larger than a first-level
//! cache. Only wrapping integer arithmetic and the exactly rounded IEEE 754
//! operations (add, subtract, multiply, divide, square root) are used, so a
//! unit computes the same bits on every machine and at every optimisation level.

use std::hint::black_box;

/// Words in the scratch table: 128 KiB, more than a first-level data cache holds.
const TABLE_WORDS: usize = 1 << 14;

// Kernel sizes. A unit takes about 0.2 ms on a current desktop core, of which
// roughly 40 % is integer work, 40 % floating point and 20 % memory traffic.
const HASH_LANES: usize = 4;
const HASH_ROUNDS: u32 = 13_000;
const ORBIT_STEPS: u32 = 3_000;
const TABLE_UPDATES: u32 = 28_000;

const BODIES: usize = 4;
const MASSES: [f64; BODIES] = [1.0, 0.5, 0.25, 0.125];
const START: [[f64; 3]; BODIES] = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.5], [-1.0, 2.5, 0.0], [0.5, -3.0, -1.0]];
const TIME_STEP: f64 = 1.0 / 64.0;
/// Added to squared distances so that close encounters stay finite.
const SOFTENING: f64 = 1.0 / 16.0;

const GOLDEN_RATIO: u64 = 0x9E37_79B9_7F4A_7C15;

/// The state one worker thread carries from unit to unit.
pub struct Workload {
    table: Vec<u64>,
    state: u64,
}

impl Workload {
    pub fn new() -> Self {
        let mut state = GOLDEN_RATIO;
        let table = (0..TABLE_WORDS)
            .map(|_| {
                state = mix(state);
                state
            })
            .collect();
        Workload { table, state }
    }

    /// Runs one work unit. Its result seeds the next unit, so no unit can be skipped or reordered.
    pub fn run_unit(&mut self) {
        let seed = black_box(self.state);
        let hashed = hash(seed);
        let orbited = orbit(seed);
        let stirred = stir(&mut self.table, hashed);
        self.state = black_box(mix(hashed ^ orbited.rotate_left(21) ^ stirred.rotate_left(42)));
    }
}

/// The SplitMix64 output function: a cheap, well-distributed 64-bit mixer.
fn mix(value: u64) -> u64 {
    let mut z = value.wrapping_add(GOLDEN_RATIO);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Integer kernel: chains of multiply, shift, rotate and bit-count steps. The
/// lanes do not depend on each other, so a wide core can work on all of them at once.
fn hash(seed: u64) -> u64 {
    let mut lanes: [u64; HASH_LANES] = std::array::from_fn(|lane| mix(seed ^ lane as u64));
    let mut sum = 0u64;
    for round in 0..HASH_ROUNDS {
        for lane in &mut lanes {
            let mut hash = (*lane ^ u64::from(round)).wrapping_mul(GOLDEN_RATIO);
            hash ^= hash >> 29;
            hash = hash.rotate_left(hash.count_ones());
            sum = sum.wrapping_add(hash.swap_bytes() ^ u64::from(hash.leading_zeros()));
            *lane = hash;
        }
    }
    lanes.iter().fold(sum, |digest, lane| mix(digest ^ lane))
}

/// Floating-point kernel: four bodies under softened gravity, advanced with
/// a fixed number of leapfrog steps from start positions nudged by `seed`.
fn orbit(seed: u64) -> u64 {
    let mut position = START;
    let mut velocity = [[0.0f64; 3]; BODIES];
    for (slot, coordinate) in position.iter_mut().flatten().enumerate() {
        // One seed byte per coordinate, as an exact fraction in 0..1.
        *coordinate += f64::from(seed.to_le_bytes()[slot % 8]) / 256.0;
    }

    for _ in 0..ORBIT_STEPS {
        for first in 0..BODIES {
            for second in first + 1..BODIES {
                let delta: [f64; 3] =
                    std::array::from_fn(|axis| position[second][axis] - position[first][axis]);
                let squared = delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2] + SOFTENING;
                let pull = TIME_STEP / (squared * squared.sqrt());
                for (axis, offset) in delta.iter().enumerate() {
                    velocity[first][axis] += offset * MASSES[second] * pull;
                    velocity[second][axis] -= offset * MASSES[first] * pull;
                }
            }
        }
        for (body, speed) in position.iter_mut().zip(&velocity) {
            for (coordinate, rate) in body.iter_mut().zip(speed) {
                *coordinate += rate * TIME_STEP;
            }
        }
    }

    position.iter().chain(&velocity).flatten().fold(0, |digest, value| mix(digest ^ value.to_bits()))
}

/// Memory kernel: pseudo-random read-modify-write traffic across the table.
fn stir(table: &mut [u64], seed: u64) -> u64 {
    let mask = table.len() - 1;
    let mut random = seed | 1;
    let mut sum = 0u64;
    for _ in 0..TABLE_UPDATES {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        let (near, far) = (random as usize & mask, (random >> 32) as usize & mask);
        let value = table[near].wrapping_add(random);
        table[near] = table[far].rotate_left(11) ^ value;
        table[far] = value;
        sum = sum.wrapping_add(value);
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_are_deterministic() {
        let (mut first, mut second) = (Workload::new(), Workload::new());
        for _ in 0..3 {
            first.run_unit();
            second.run_unit();
            assert_eq!(first.state, second.state);
        }
        assert_eq!(first.table, second.table);
    }

    #[test]
    fn units_compute_the_same_bits_on_every_machine() {
        let mut workload = Workload::new();
        for _ in 0..3 {
            workload.run_unit();
        }
        // A different value here means different scores: bump `VERSION` in the parent module.
        assert_eq!(workload.state, 0x6E21_D28A_1E0B_D157);
    }

    #[test]
    fn every_unit_changes_the_state() {
        let mut workload = Workload::new();
        let mut seen = vec![workload.state];
        for _ in 0..4 {
            workload.run_unit();
            assert!(!seen.contains(&workload.state));
            seen.push(workload.state);
        }
    }

    #[test]
    fn kernels_depend_on_their_seed() {
        assert_ne!(hash(1), hash(2));
        assert_ne!(orbit(1), orbit(2));
        let mut table = vec![0u64; 64];
        assert_ne!(stir(&mut table.clone(), 1), stir(&mut table, 2));
    }

    #[test]
    fn mixer_matches_splitmix64() {
        // First outputs of SplitMix64 seeded with zero.
        assert_eq!(mix(0), 0xE220_A839_7B1D_CDAF);
        assert_eq!(mix(GOLDEN_RATIO), 0x6E78_9E6A_A1B9_65F4);
    }
}
