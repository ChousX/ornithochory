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
//! Beside the flow, [`bend`]: anything with a [`bend::FoliageBender`] pushes
//! small plants aside.
//!
//! Integrations, each behind its feature:
//! - `tarasaur`: species scatter over terrain chunks as they stream in and
//!   re-scatter where the terrain is edited (`integrate::tarasaur`).
//! - `ornisculpt`: mobs push plants aside and ornisculpt's `TierCamera`
//!   also picks plant tiers (`integrate::ornisculpt`).
//! - `genetics`: a whole species as one genome (`design`), whose
//!   `lever`s name the values a design lets vary.
//!
//! Research behind the design: `docs/research/procedural-plants.md`.

#![warn(missing_docs)]

pub mod bend;
#[cfg(feature = "genetics")]
pub mod design;
pub mod generate;
pub mod grow;
pub mod integrate;
#[cfg(feature = "genetics")]
pub mod lever;
pub mod mesh;
pub mod num;
pub mod plant;
pub mod prelude;
pub mod random;
pub mod scatter;
pub mod skeleton;
pub mod species;

use bevy::prelude::*;

/// Adds plant assets, generation, tiers and interaction: every module's
/// plugin, and `design::PlantDesignPlugin` with the `genetics` feature.
#[derive(Default)]
pub struct OrnithochoryPlugin;

impl Plugin for OrnithochoryPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((species::SpeciesPlugin, plant::PlantPlugin, bend::BendPlugin));
        #[cfg(feature = "genetics")]
        app.add_plugins(design::PlantDesignPlugin);
    }
}
