//! Deterministic randomness: every plant and every placement is a pure
//! function of a seed, so nothing generated needs saving.
//!
//! - Hashing: xxh3, a specified algorithm, so a label or a cell hashes the
//!   same on every platform and version (unlike `std`'s `DefaultHasher`).
//!   Ornisculpt forks its streams the same way.
//! - RNG: `SplitMix64`. Tiny, fast, and good enough for shapes; nothing here
//!   is cryptographic.

use bevy::prelude::*;
use xxhash_rust::xxh3::{xxh3_64_with_seed, xxh3_128};

use crate::num::{u64_from_i64_bits, unit_f32_from_u64};

/// `SplitMix64`'s step between states: the golden ratio as 64 fixed-point
/// bits.
const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
/// `SplitMix64`'s two output multipliers (Stafford's Mix13).
const MIX: (u64, u64) = (0xBF58_476D_1CE4_E5B9, 0x94D0_49BB_1331_11EB);
/// Spreads [`hash_unit`]'s lanes over the hash's bits, so neighbouring
/// lanes draw unrelated values.
const LANE_SPREAD: u64 = 0xD6E8_FEB8_6659_FD93;
/// Rejection tries for a point in the unit ball before giving up on 0.
/// Each succeeds about half the time, so 32 failing is a 1-in-10^10 event.
const BALL_TRIES: usize = 32;

/// Stable 64-bit hash of `label` under `seed`.
#[must_use]
pub fn stable_hash(seed: u64, label: &str) -> u64 {
    xxh3_64_with_seed(label.as_bytes(), seed)
}

/// Stable hash of a grid cell under `seed`: placement uses it so a cell gets
/// the same plant whichever chunk asks.
#[must_use]
pub fn cell_hash(seed: u64, cell: IVec2) -> u64 {
    let x = u64_from_i64_bits(i64::from(cell.x));
    let y = u64_from_i64_bits(i64::from(cell.y));
    let mut bytes = [0u8; 16];
    let (a, b) = bytes.split_at_mut(8);
    a.copy_from_slice(&x.to_le_bytes());
    b.copy_from_slice(&y.to_le_bytes());
    xxh3_64_with_seed(&bytes, seed)
}

/// 128-bit content hash: cache keys for generated meshes.
#[must_use]
pub fn content_hash(bytes: &[u8]) -> u128 {
    xxh3_128(bytes)
}

/// The RNG every grower draws from.
#[derive(Clone, Debug)]
pub struct PlantRng(u64);

impl PlantRng {
    /// A stream starting from `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// An independent stream for `label` under `seed`: changing how one
    /// stage draws never reshuffles another.
    #[must_use]
    pub fn fork(seed: u64, label: &str) -> Self {
        Self(stable_hash(seed, label))
    }

    /// The next 64 random bits.
    pub const fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(GOLDEN_GAMMA);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(MIX.0);
        z = (z ^ (z >> 27)).wrapping_mul(MIX.1);
        z ^ (z >> 31)
    }

    /// Uniform in `0..1`.
    pub const fn unit(&mut self) -> f32 {
        unit_f32_from_u64(self.next_u64())
    }

    /// Uniform in `-1..1`.
    pub const fn signed(&mut self) -> f32 {
        self.unit().mul_add(2.0, -1.0)
    }

    /// Uniform in `lo..hi`.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        (hi - lo).mul_add(self.unit(), lo)
    }

    /// `true` with probability `p`.
    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    /// Uniform inside the unit ball (rejection; 52% acceptance).
    pub fn in_unit_ball(&mut self) -> Vec3 {
        for _ in 0..BALL_TRIES {
            let v = Vec3::new(self.signed(), self.signed(), self.signed());
            if v.length_squared() <= 1.0 {
                return v;
            }
        }
        Vec3::ZERO
    }
}

/// A cell's hash as a value in `0..1`, for a second, third... draw from one
/// hash without an RNG: `hash_unit(h, 0)`, `hash_unit(h, 1)`.
#[must_use]
pub const fn hash_unit(hash: u64, lane: u64) -> f32 {
    PlantRng::new(hash ^ lane.wrapping_mul(LANE_SPREAD)).unit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forks_are_stable_and_independent() {
        let a = PlantRng::fork(7, "trunk").next_u64();
        assert_eq!(a, PlantRng::fork(7, "trunk").next_u64());
        assert_ne!(a, PlantRng::fork(7, "leaves").next_u64());
        assert_ne!(a, PlantRng::fork(8, "trunk").next_u64());
    }

    #[test]
    fn unit_stays_in_range() {
        let mut rng = PlantRng::new(1);
        for _ in 0..10_000 {
            let u = rng.unit();
            assert!((0.0..1.0).contains(&u));
        }
    }

    #[test]
    fn lanes_are_stable_and_apart() {
        let h = cell_hash(3, IVec2::new(4, -2));
        assert!((hash_unit(h, 0) - hash_unit(h, 0)).abs() < f32::EPSILON);
        assert!((hash_unit(h, 0) - hash_unit(h, 1)).abs() > f32::EPSILON);
        assert!((0.0..1.0).contains(&hash_unit(h, 7)));
    }

    #[test]
    fn ball_points_stay_inside() {
        let mut rng = PlantRng::new(2);
        for _ in 0..1000 {
            assert!(rng.in_unit_ball().length_squared() <= 1.0);
        }
        let r = rng.range(2.0, 3.0);
        assert!((2.0..3.0).contains(&r));
    }

    #[test]
    fn cells_hash_apart() {
        assert_ne!(cell_hash(1, IVec2::new(0, 1)), cell_hash(1, IVec2::new(1, 0)));
        assert_eq!(cell_hash(1, IVec2::new(-3, 9)), cell_hash(1, IVec2::new(-3, 9)));
    }
}
