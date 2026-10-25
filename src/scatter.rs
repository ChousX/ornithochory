//! Placement: where a species grows inside a region, as a pure function of
//! the world seed and position.
//!
//! The world is cut into a global grid of `spacing`-sized cells, each with
//! at most one candidate, jittered by the cell's hash. A candidate belongs
//! to whichever region contains it, so neighbouring regions (terrain chunks)
//! never double up or leave seams, and scattering a region again gives the
//! same plants: nothing is saved.
//!
//! Cost: the cheap rules (probability, noise) run before the ground query,
//! which is the only expensive step (a ray or SDF sample).

use bevy::prelude::*;
use fastnoise_lite::{FastNoiseLite, FractalType, NoiseType};

use crate::num::{f32_from_i32, i32_from_f32};
use crate::random::{cell_hash, hash_unit, stable_hash};
use crate::species::{Placement, Species};

/// The finest grid, metres: a spacing of 0 would ask for endless cells.
const MIN_SPACING: f32 = 0.01;

/// Whether a cell gets a candidate, against `Placement::probability`: one
/// lane of the cell's hash (see [`hash_unit`]). Each decision draws its own
/// lane, so none depends on another.
const KEEP_LANE: u64 = 0;
/// The candidate's offset across its cell, x then z; see [`KEEP_LANE`].
const JITTER_LANES: (u64, u64) = (1, 2);
/// The plant's turn about its up axis; see [`KEEP_LANE`].
const YAW_LANE: u64 = 3;
/// The plant's scale within `Placement::scale`; see [`KEEP_LANE`].
const SCALE_LANE: u64 = 4;

/// What the ground is like at a candidate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundHit {
    /// Where the ground is, world space.
    pub pos: Vec3,
    /// Unit, out of the ground.
    pub normal: Vec3,
    /// The host's material id, if it has materials.
    pub material: Option<u8>,
}

/// One plant to spawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// The plant's root, rotation and scale.
    pub transform: Transform,
    /// Picks the variant and the per-plant tint.
    pub seed: u64,
}

/// The seed a species scatters with in a world.
#[must_use]
pub fn species_seed(world_seed: u64, species: &Species) -> u64 {
    stable_hash(world_seed, &species.name)
}

/// The cells overlapping `min..max` (world XZ), each with its candidate
/// point if one lies inside the region and passes the cheap rules.
pub fn candidates(placement: &Placement, seed: u64, min: Vec2, max: Vec2) -> impl Iterator<Item = (Vec2, u64)> {
    let spacing = placement.spacing.max(MIN_SPACING);
    let lo = (min / spacing).floor();
    let hi = (max / spacing).ceil();
    let (x0, z0, x1, z1) = (i32_from_f32(lo.x), i32_from_f32(lo.y), i32_from_f32(hi.x), i32_from_f32(hi.y));
    let noise = placement.noise.map(|n| {
        let [b0, b1, b2, b3, ..] = seed.to_le_bytes();
        let mut f = FastNoiseLite::with_seed(i32::from_le_bytes([b0, b1, b2, b3]));
        f.set_noise_type(Some(NoiseType::OpenSimplex2));
        f.set_frequency(Some(n.frequency));
        f.set_fractal_type(Some(FractalType::FBm));
        f.set_fractal_octaves(Some(i32::from(n.octaves.max(1))));
        (f, n.threshold)
    });
    let probability = placement.probability;
    (z0..z1).flat_map(move |z| (x0..x1).map(move |x| IVec2::new(x, z))).filter_map(move |cell| {
        let h = cell_hash(seed, cell);
        if hash_unit(h, KEEP_LANE) >= probability {
            return None;
        }
        let jitter = Vec2::new(hash_unit(h, JITTER_LANES.0), hash_unit(h, JITTER_LANES.1));
        let p = (Vec2::new(f32_from_i32(cell.x), f32_from_i32(cell.y)) + jitter) * spacing;
        if p.x < min.x || p.y < min.y || p.x >= max.x || p.y >= max.y {
            return None;
        }
        if let Some((f, threshold)) = &noise
            && f.get_noise_2d(p.x, p.y) < *threshold
        {
            return None;
        }
        Some((p, h))
    })
}

/// Places `placement` over `min..max` (world XZ). `ground` answers "what
/// is under this point", or `None` for no ground (a hole, an unloaded
/// chunk, a cliff overhang above the region).
pub fn scatter(
    placement: &Placement,
    seed: u64,
    min: Vec2,
    max: Vec2,
    mut ground: impl FnMut(Vec2) -> Option<GroundHit>,
) -> Vec<Placed> {
    let cos_lo = placement.slope.1.to_radians().cos();
    let cos_hi = placement.slope.0.to_radians().cos();
    candidates(placement, seed, min, max)
        .filter_map(|(p, h)| {
            let hit = ground(p)?;
            let up = hit.normal.y;
            // A little slack, so ground exactly at a limit isn't lost to
            // rounding.
            if up < cos_lo - 1e-4 || up > cos_hi + 1e-4 {
                return None;
            }
            if hit.pos.y < placement.altitude.0 || hit.pos.y > placement.altitude.1 {
                return None;
            }
            if !placement.materials.is_empty() && hit.material.is_none_or(|m| !placement.materials.contains(&m)) {
                return None;
            }
            let yaw = Quat::from_rotation_y(hash_unit(h, YAW_LANE) * std::f32::consts::TAU);
            let tilt_to = Vec3::Y.lerp(hit.normal, placement.align.clamp(0.0, 1.0)).normalize_or(Vec3::Y);
            let rotation = Quat::from_rotation_arc(Vec3::Y, tilt_to) * yaw;
            let (s0, s1) = placement.scale;
            let scale = (s1 - s0).mul_add(hash_unit(h, SCALE_LANE), s0);
            let translation = hit.pos - tilt_to * (placement.sink * scale);
            Some(Placed { transform: Transform { translation, rotation, scale: Vec3::splat(scale) }, seed: h })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Level ground of material 1 everywhere.
    fn flat() -> impl Fn(Vec2) -> Option<GroundHit> {
        |p| Some(GroundHit { pos: Vec3::new(p.x, 0.0, p.y), normal: Vec3::Y, material: Some(1) })
    }

    #[test]
    fn regions_tile_without_seams() {
        let pl = Placement { spacing: 1.3, probability: 0.8, ..Placement::default() };
        let whole = scatter(&pl, 5, Vec2::ZERO, Vec2::splat(20.0), flat());
        let mut parts = Vec::new();
        for (x, z) in [(0.0, 0.0), (10.0, 0.0), (0.0, 10.0), (10.0, 10.0)] {
            let min = Vec2::new(x, z);
            parts.extend(scatter(&pl, 5, min, min + 10.0, flat()));
        }
        let key = |p: &Placed| p.seed;
        let mut a: Vec<_> = whole.iter().map(key).collect();
        let mut b: Vec<_> = parts.iter().map(key).collect();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b);
        assert!(a.len() > 100);
    }

    #[test]
    fn rules_filter() {
        let pl = Placement { spacing: 1.0, probability: 1.0, materials: vec![2], ..Placement::default() };
        assert_eq!(scatter(&pl, 1, Vec2::ZERO, Vec2::splat(5.0), flat()).len(), 0);
        let steep = |p: Vec2| Some(GroundHit { pos: Vec3::new(p.x, 0.0, p.y), normal: Vec3::new(1.0, 1.0, 0.0).normalize(), material: None });
        let pl = Placement { spacing: 1.0, probability: 1.0, slope: (0.0, 30.0), ..Placement::default() };
        assert_eq!(scatter(&pl, 1, Vec2::ZERO, Vec2::splat(5.0), steep).len(), 0);
        let pl = Placement { slope: (0.0, 50.0), ..pl };
        assert_ne!(scatter(&pl, 1, Vec2::ZERO, Vec2::splat(5.0), steep).len(), 0);
    }
}
