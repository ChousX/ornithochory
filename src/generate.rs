//! Species + variant -> meshes for every tier. Pure functions, no ECS: safe
//! to run on `AsyncComputeTaskPool`, and what the plugin caches per variant.

use crate::mesh::{MeshData, PlantLod, cover, leaves, tube};
use crate::random::{PlantRng, stable_hash};
use crate::skeleton::PlantSkeleton;
use crate::species::{Form, Species, Structure};

/// Bump when any generator's output changes for the same input, so caches
/// keyed on content (on disk, later) are invalidated.
pub const GENERATOR_VERSION: u32 = 2;

/// One tier's meshes for one variant.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TierMeshes {
    /// Stems (bark material). Empty for cover.
    pub stems: MeshData,
    /// Leaves, blades or cushions (vertex-coloured material).
    pub foliage: MeshData,
}

/// Every tier of one variant, plus its bounding radius for tier selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GeneratedVariant {
    /// Meshes per tier, in [`PlantLod::index`] order.
    pub tiers: [TierMeshes; 3],
    /// Distance from the plant's origin to its farthest point, metres.
    pub radius: f32,
}

/// The seed of variant `variant` of `species`.
#[must_use]
pub fn variant_seed(species: &Species, variant: u8) -> u64 {
    stable_hash(u64::from(variant), &species.name)
}

/// A structure's skeleton for `seed`, radii included: the input to
/// [`mesh_structure`]. Custom growers can skip this and build their own
/// `PlantSkeleton`.
#[must_use]
pub fn build_structure(s: &Structure, seed: u64) -> PlantSkeleton {
    let mut rng = PlantRng::fork(seed, "growth");
    let mut skel = s.growth.grow(&mut rng);
    skel.pipe_radii(s.tip_radius, s.pipe_exponent);
    skel
}

/// A skeleton's meshes at `lod`.
#[must_use]
pub fn mesh_structure(skel: &PlantSkeleton, s: &Structure, seed: u64, lod: PlantLod) -> TierMeshes {
    let mut stems = tube::stems(skel, &s.bark, s.tip_radius, lod);
    // Bark is what gets normal-mapped; leaves and cover skip the 16 bytes
    // a vertex.
    stems.compute_tangents();
    TierMeshes {
        stems,
        foliage: s.leaves.as_ref().map(|l| leaves::leaves(skel, l, lod, seed)).unwrap_or_default(),
    }
}

/// How far leaves reach past the stems, in leaf lengths, for the bounding
/// radius: a leaf at its longest, with room to spare.
const LEAF_REACH: f32 = 1.5;

/// Every tier of one variant of `species`.
#[must_use]
pub fn generate(species: &Species, variant: u8) -> GeneratedVariant {
    let seed = variant_seed(species, variant);
    match &species.form {
        Form::Structure(s) => {
            let skel = build_structure(s, seed);
            let (lo, hi) = skel.bounds();
            let leaf = s.leaves.as_ref().map_or(0.0, |l| l.length * LEAF_REACH);
            GeneratedVariant {
                tiers: PlantLod::ALL.map(|lod| mesh_structure(&skel, s, seed, lod)),
                radius: lo.abs().max(hi.abs()).length() + leaf,
            }
        }
        Form::Cover(c) => {
            let tiers = PlantLod::ALL.map(|lod| TierMeshes { stems: MeshData::default(), foliage: cover::tuft(c, lod, seed) });
            // Sized by the hero tuft, the most detailed.
            let [hero, ..] = &tiers;
            let radius = hero.foliage.positions.iter().map(|&p| bevy::math::Vec3::from(p).length()).fold(0.0, f32::max);
            GeneratedVariant { tiers, radius }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(name: &str) -> Species {
        let path = format!("{}/assets/species/{name}.plant.ron", env!("CARGO_MANIFEST_DIR"));
        ron::de::from_bytes(&std::fs::read(&path).unwrap()).unwrap()
    }

    #[test]
    fn every_bundled_species_generates() {
        let dir = format!("{}/assets/species", env!("CARGO_MANIFEST_DIR"));
        let mut n = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            let Some(stem) = name.strip_suffix(".plant.ron") else { continue };
            let species = load(stem);
            let t = std::time::Instant::now();
            let v = generate(&species, 0);
            let hero = &v.tiers[0];
            let tris = (hero.stems.indices.len() + hero.foliage.indices.len()) / 3;
            eprintln!(
                "{stem}: {tris} hero tris ({} stems, {} foliage; near {}, far {}), radius {:.2} m, {:?}",
                hero.stems.indices.len() / 3,
                hero.foliage.indices.len() / 3,
                (v.tiers[1].stems.indices.len() + v.tiers[1].foliage.indices.len()) / 3,
                (v.tiers[2].stems.indices.len() + v.tiers[2].foliage.indices.len()) / 3,
                v.radius,
                t.elapsed()
            );
            assert!(tris > 0, "{stem} made nothing");
            assert!(v.radius > 0.0);
            for tier in &v.tiers {
                for m in [&tier.stems, &tier.foliage] {
                    assert!(m.indices.iter().all(|&i| i < m.vertex_count()), "{stem}: index out of range");
                    assert!(m.positions.iter().flatten().all(|c| c.is_finite()), "{stem}: non-finite vertex");
                }
            }
            // Coarser tiers never cost more.
            let cost = |t: &TierMeshes| t.stems.indices.len() + t.foliage.indices.len();
            assert!(cost(&v.tiers[2]) <= cost(&v.tiers[1]) && cost(&v.tiers[1]) <= cost(&v.tiers[0]), "{stem}");
            n += 1;
        }
        assert!(n >= 4, "only {n} species found");
    }

    #[test]
    fn variants_differ_and_repeat() {
        let oak = load("oak");
        assert_eq!(generate(&oak, 1), generate(&oak, 1));
        assert_ne!(generate(&oak, 1), generate(&oak, 2));
    }
}
