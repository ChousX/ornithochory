//! Ornithochory: generic procedural plants for Bevy.
//!
//! Data flows one way:
//!
//! 1. [`species`]: a `*.plant.ron` asset says how a species grows
//!    ([`grow::Growth`] for stemmed plants, [`species::Cover`] for grass
//!    and moss) and where ([`species::Placement`]).
//! 2. [`grow`]: a grower turns it and a seed into a [`skeleton::PlantSkeleton`],
//!    the one intermediate form every later stage reads.
//! 3. [`mesh`]: stems, leaves and tufts per [`mesh::PlantLod`], with wind
//!    weights baked into every vertex.
//! 4. [`plant`]: [`plant::Plant`]s show cached variants, generated off-thread
//!    and picked by screen size; [`plant::CoverPatch`]es merge tufts.
//! 5. [`scatter`]: placement as a pure function of world seed and position,
//!    so a region re-scatters to the same plants and nothing is saved.
//!
//! Integrations, each behind its feature:
//! - `tarasaur`: species scatter over terrain chunks as they stream in and
//!   re-scatter where the terrain is edited ([`integrate::tarasaur`]).
//! - `ornisculpt`: mobs push plants aside and ornisculpt's `TierCamera`
//!   also picks plant tiers ([`integrate::ornisculpt`]).
//!
//! Research behind the design: `docs/research/procedural-plants.md`.

pub mod bend;
pub mod generate;
pub mod grow;
pub mod integrate;
pub mod mesh;
pub mod num;
pub mod plant;
pub mod prelude;
pub mod random;
pub mod scatter;
pub mod skeleton;
pub mod species;

use bevy::prelude::*;

use crate::bend::Benders;
use crate::plant::{PlantLibrary, PlantQuality, PlantSet};
use crate::species::{Species, SpeciesLoader};

/// Adds plant assets, generation, tiers and interaction.
#[derive(Default)]
pub struct OrnithochoryPlugin;

impl Plugin for OrnithochoryPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<Species>()
            .init_asset_loader::<SpeciesLoader>()
            .init_resource::<PlantLibrary>()
            .init_resource::<PlantQuality>()
            .init_resource::<Benders>()
            .configure_sets(Update, (PlantSet::Generate, PlantSet::Tier).chain())
            .add_systems(
                Update,
                (
                    (
                        plant::reload_species,
                        plant::request_variants,
                        plant::poll_variants,
                        plant::attach_plants,
                        plant::apply_plant_materials,
                        plant::build_cover,
                        plant::finish_cover,
                    )
                        .chain()
                        .in_set(PlantSet::Generate),
                    (plant::choose_tiers, bend::gather_benders, bend::lean_plants)
                        .chain()
                        .in_set(PlantSet::Tier),
                ),
            );
    }
}
