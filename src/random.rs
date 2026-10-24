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

    pub const fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
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
        for _ in 0..32 {
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
    PlantRng::new(hash ^ lane.wrapping_mul(0xD6E8_FEB8_6659_FD93)).unit()
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
    fn cells_hash_apart() {
        assert_ne!(cell_hash(1, IVec2::new(0, 1)), cell_hash(1, IVec2::new(1, 0)));
        assert_eq!(cell_hash(1, IVec2::new(-3, 9)), cell_hash(1, IVec2::new(-3, 9)));
    }
}
