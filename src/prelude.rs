//! Common imports: `use ornithochory::prelude::*;`.

pub use crate::OrnithochoryPlugin;
pub use crate::bend::{Benders, FoliageBender};
pub use crate::mesh::PlantLod;
pub use crate::plant::{
    CoverPatch, ForcePlantTier, Plant, PlantCamera, PlantLibrary, PlantMaterials, PlantParts, PlantQuality, PlantSet, PlantTier,
};
pub use crate::scatter::{GroundHit, Placed, scatter, species_seed};
pub use crate::lever::PlantLever;
pub use crate::species::{Form, Placement, Species};

#[cfg(feature = "genetics")]
pub use crate::design::{PlantDesign, PlantTrait};

#[cfg(feature = "ornisculpt")]
pub use crate::integrate::ornisculpt::OrnisculptPlantsPlugin;
#[cfg(feature = "tarasaur")]
pub use crate::integrate::tarasaur::{Flora, FloraEntry, TerrainPlantsPlugin};
