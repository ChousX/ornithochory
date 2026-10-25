//! Plant designs and genomes (`genetics` feature): a whole [`Species`] as DNA.
//!
//! Every value of the species is in the genome. The ones that must stay as
//! designed (how it grows, its L-system rules, which leaves and tufts it has,
//! the ground it grows on) are fixed genes, which point mutation never
//! touches; the ones a [`PlantDesign`] lists as traits vary within their
//! ranges.
//!
//! A genome is a strain, not a single plant: [`express`] gives one
//! [`Species`] for every plant grown from it, and the species' variants are
//! what tells those plants apart. A genome per plant would give each its own
//! meshes and break batching.

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::prelude::*;
use genetics::{Gene, Genome, Trait, Variation};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::generate::{build_structure, variant_seed};
use crate::lever::{LeverError, PlantLever};
use crate::num::i32_from_f32;
use crate::species::{Form, Species, Tuft};

/// One value of a design that may vary.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum PlantTrait {
    /// Every value `lever` reaches varies within `range`, which must lie
    /// inside [`PlantLever::range`] and hold the value as designed.
    Lever {
        /// The values that vary.
        lever: PlantLever,
        /// Lowest and highest value they may take, in the lever's units.
        range: (f32, f32),
    },
    /// Any value of the species by its genome path.
    Field(Trait),
}

/// A plant as someone designed it: the species, and what may vary.
#[derive(Asset, TypePath, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlantDesign {
    /// The species as designed: every strain's founder.
    pub species: Species,
    /// The values of `species` that may vary from strain to strain.
    pub traits: Vec<PlantTrait>,
}

/// Why a design can't become a genome.
#[derive(Debug, Error)]
pub enum DesignError {
    /// A lever doesn't apply to the species.
    #[error(transparent)]
    Lever(#[from] LeverError),
    /// A lever's range lies outside the values the lever may take.
    #[error("{lever:?}: range {lo}..={hi} is not inside {min}..={max}")]
    Range {
        /// The lever.
        lever: PlantLever,
        /// The design's lowest value.
        lo: f32,
        /// The design's highest value.
        hi: f32,
        /// The lowest value the lever may take ([`PlantLever::range`]).
        min: f32,
        /// The highest value the lever may take.
        max: f32,
    },
    /// The genome couldn't encode or decode the species: a trait's range
    /// doesn't hold the designed value, or a mutant is unviable.
    #[error(transparent)]
    Genetics(#[from] genetics::Error),
}

/// How elaborate a plant is, for budgets: the stem nodes and organs (leaves,
/// flowers, fruit) of one grown variant, or a cover's blades or a cushion's
/// mounds and bristles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Complexity {
    /// Stem nodes, or a cushion's mounds.
    pub nodes: usize,
    /// Leaves, flowers and fruit, or a cover's blades or bristles.
    pub organs: usize,
}

impl Complexity {
    /// Grows variant 0 to count it; a pure function, but not a free one for
    /// a large tree.
    #[must_use]
    pub fn of(species: &Species) -> Self {
        match &species.form {
            Form::Structure(s) => {
                let skeleton = build_structure(s, variant_seed(species, 0));
                Self { nodes: skeleton.nodes.len(), organs: skeleton.organs.len() }
            }
            Form::Cover(c) => match c.tuft {
                Tuft::Blades { count, .. } => Self { nodes: 0, organs: usize::from(count) },
                Tuft::Cushion { lumps, bristles, .. } => Self { nodes: usize::from(lumps), organs: usize::from(bristles) },
            },
        }
    }
}

impl PlantDesign {
    /// Which values of the species may vary.
    ///
    /// # Errors
    ///
    /// When a lever doesn't apply to the species, or its range lies outside
    /// the lever's.
    pub fn variation(&self) -> Result<Variation, DesignError> {
        let mut variation = Variation::new();
        for t in &self.traits {
            match t {
                PlantTrait::Lever { lever, range: (lo, hi) } => {
                    let (min, max) = lever.range();
                    if !(min <= *lo && lo < hi && *hi <= max) {
                        return Err(DesignError::Range { lever: lever.clone(), lo: *lo, hi: *hi, min, max });
                    }
                    let gene = if lever.is_count() {
                        Gene::Count { min: whole(lo.ceil()), max: whole(hi.floor()) }
                    } else {
                        Gene::range((*lo).into(), (*hi).into())
                    };
                    for path in lever.paths(&self.species)? {
                        variation = variation.with(path, gene);
                    }
                }
                PlantTrait::Field(t) => variation.0.push(t.clone()),
            }
        }
        Ok(variation)
    }

    /// The first genome of the design's lineage: the species as designed.
    ///
    /// # Errors
    ///
    /// When a trait doesn't apply, is out of range, or doesn't hold the
    /// designed value.
    pub fn founder(&self) -> Result<Genome, DesignError> {
        Ok(Genome::encode(&self.species, &self.variation()?)?)
    }
}

/// The species a genome grows.
///
/// # Errors
///
/// When the genome doesn't decode as a species (a mutation chose a variant
/// that carries data): such a strain is unviable.
pub fn express(genome: &Genome) -> Result<Species, DesignError> {
    Ok(genome.decode()?)
}

/// `v`, already whole, as an `i64`: lever ranges stay far inside `i32`.
fn whole(v: f32) -> i64 {
    i64::from(i32_from_f32(v))
}

/// A `*.plant-design.ron` file: a design whose species is another asset.
#[derive(Deserialize)]
struct DesignFile {
    /// A `*.plant.ron` path.
    species: String,
    #[serde(default)]
    traits: Vec<PlantTrait>,
}

/// Loads `*.plant-design.ron` files into [`PlantDesign`]s, reading the
/// species they name.
#[derive(Default, TypePath)]
pub struct PlantDesignLoader;

/// Why a `*.plant-design.ron` file didn't load.
#[derive(Debug, Error)]
pub enum PlantDesignLoaderError {
    /// The file couldn't be read.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The file isn't a valid design.
    #[error(transparent)]
    Ron(#[from] ron::error::SpannedError),
    /// The species it names didn't load.
    #[error("loading `{path}`: {reason}")]
    Species {
        /// The species' asset path, as the design names it.
        path: String,
        /// Why it didn't load.
        reason: String,
    },
}

impl AssetLoader for PlantDesignLoader {
    type Asset = PlantDesign;
    type Settings = ();
    type Error = PlantDesignLoaderError;

    async fn load(&self, reader: &mut dyn Reader, (): &(), cx: &mut LoadContext<'_>) -> Result<PlantDesign, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let file: DesignFile = ron::de::from_bytes(&bytes)?;
        let species = cx
            .load_builder()
            .load_value::<Species>(&file.species)
            .await
            .map_err(|e| PlantDesignLoaderError::Species { path: file.species.clone(), reason: e.to_string() })?
            .take();
        Ok(PlantDesign { species, traits: file.traits })
    }

    fn extensions(&self) -> &[&str] {
        &["plant-design.ron"]
    }
}

/// Registers [`PlantDesign`] assets and their loader. Added by
/// [`crate::OrnithochoryPlugin`].
#[derive(Default)]
pub struct PlantDesignPlugin;

impl Plugin for PlantDesignPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<PlantDesign>().init_asset_loader::<PlantDesignLoader>();
    }
}

#[cfg(test)]
mod tests {
    use genetics::Genome;
    use rand::SeedableRng;
    use rand_pcg::Pcg64Mcg;

    use super::*;
    use crate::grow::Growth;

    fn species(name: &str) -> Species {
        let path = format!("{}/assets/species/{name}.plant.ron", env!("CARGO_MANIFEST_DIR"));
        ron::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    const ALL: [&str; 6] = ["oak", "spruce", "bush", "fern", "grass", "moss"];

    #[test]
    fn every_species_round_trips_through_dna() {
        for name in ALL {
            let s = species(name);
            let genome = Genome::encode(&s, &Variation::new()).unwrap();
            assert_eq!(express(&genome).unwrap(), s, "{name}");
        }
    }

    fn oak() -> PlantDesign {
        let lever = |lever, range| PlantTrait::Lever { lever, range };
        PlantDesign {
            species: species("oak"),
            traits: vec![
                lever(PlantLever::CrownRadius, (3.5, 5.5)),
                lever(PlantLever::CrownHeight, (2.4, 3.8)),
                lever(PlantLever::Influence, (1.4, 2.2)),
                lever(PlantLever::PipeExponent, (2.1, 2.5)),
                lever(PlantLever::LeafLength, (0.18, 0.3)),
                lever(PlantLever::LeafCluster, (4.0, 8.0)),
                lever(PlantLever::LeafColor(1), (0.3, 0.5)),
            ],
        }
    }

    #[test]
    fn oak_mutants_stay_in_range_and_grow() {
        let founder = oak().founder().unwrap();
        assert_eq!(founder.schema.variable().count(), 8);
        let mut rng = Pcg64Mcg::seed_from_u64(5);
        let caps = Complexity::of(&species("oak"));
        for _ in 0..50 {
            let mutant = founder.mutant(0.2, &mut rng);
            assert_eq!(mutant.lineage(), founder.lineage());
            let s = express(&mutant).unwrap();
            let Form::Structure(st) = &s.form else { panic!() };
            let Growth::Colonize(c) = &st.growth else { panic!() };
            assert!((1.4..=2.2).contains(&c.influence));
            assert!((2.1..=2.5).contains(&st.pipe_exponent));
            let leaves = st.leaves.as_ref().unwrap();
            assert!((4..=8).contains(&leaves.cluster));
            assert!((0.3..=0.5).contains(&leaves.color[1]));
            let size = Complexity::of(&s);
            assert!(size.nodes > 0 && size.nodes < caps.nodes * 4, "{size:?} against {caps:?}");
        }
    }

    #[test]
    fn levers_must_apply_and_fit() {
        let mut d = oak();
        d.traits.push(PlantTrait::Lever { lever: PlantLever::Height, range: (1.0, 2.0) });
        assert!(matches!(d.founder(), Err(DesignError::Lever(_))));
        let mut d = oak();
        d.traits.push(PlantTrait::Lever { lever: PlantLever::PipeExponent, range: (1.0, 2.5) });
        assert!(matches!(d.founder(), Err(DesignError::Range { .. })));
        let mut d = oak();
        // The oak's leaves are 0.24 long.
        d.traits.push(PlantTrait::Lever { lever: PlantLever::LeafLength, range: (0.5, 0.9) });
        assert!(matches!(d.founder(), Err(DesignError::Genetics(_))));
        let grass = PlantDesign {
            species: species("grass"),
            traits: vec![
                PlantTrait::Lever { lever: PlantLever::Height, range: (0.15, 0.5) },
                PlantTrait::Lever { lever: PlantLever::BladeCount, range: (8.0, 24.0) },
                PlantTrait::Lever { lever: PlantLever::CoverColor(1), range: (0.2, 0.5) },
            ],
        };
        assert_eq!(grass.founder().unwrap().schema.variable().count(), 3);
    }
}
