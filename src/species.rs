//! Authored plant descriptions, loaded from `*.plant.ron`.
//!
//! A species is generic over how it grows: [`Form::Structure`] for anything
//! with stems (trees, shrubs, ferns, herbs; grown by any [`Growth`]) and
//! [`Form::Cover`] for things that only matter as a density on a surface
//! (grass, moss, lichen). Both are placed by the same [`Placement`] rule.

use std::marker::PhantomData;

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::grow::Growth;

/// Registers [`Species`] assets and their loader. Added by
/// [`crate::OrnithochoryPlugin`].
#[derive(Default)]
pub struct SpeciesPlugin;

impl Plugin for SpeciesPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<Species>().init_asset_loader::<SpeciesLoader>();
    }
}

/// One kind of plant: what it looks like, and where it grows.
#[derive(Asset, TypePath, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Species {
    /// Seeds its variants and its placement, so renaming a species
    /// regrows and moves its plants.
    pub name: String,
    /// What it is made of.
    pub form: Form,
    /// How many distinct shapes are generated and cached. Each plant shows
    /// one, picked by its seed, and differs from its siblings by scale,
    /// rotation and tint. Memory and generation time scale with this, not
    /// with the number of plants.
    #[serde(default = "default_variants")]
    pub variants: u8,
    /// Where it grows.
    #[serde(default)]
    pub placement: Placement,
}

const fn default_variants() -> u8 {
    8
}

/// A species' shape: stems, or a cover over the ground.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Form {
    /// Anything with stems. Boxed: a structure's description is far larger
    /// than a cover's.
    Structure(Box<Structure>),
    /// Grass, moss, lichen: only a density on a surface.
    Cover(Cover),
}

/// A plant with stems.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Structure {
    /// How the stems grow.
    pub growth: Growth,
    /// How the stems look.
    #[serde(default)]
    pub bark: Bark,
    /// The leaves, if it has any.
    #[serde(default)]
    pub leaves: Option<Leaves>,
    /// Radius of a stem tip, metres; the pipe model widens it downwards.
    #[serde(default = "default_tip_radius")]
    pub tip_radius: f32,
    /// Pipe-model exponent: 2 (slender) to 3 (stout trunks).
    #[serde(default = "default_pipe_exponent")]
    pub pipe_exponent: f32,
}

const fn default_tip_radius() -> f32 {
    0.008
}

const fn default_pipe_exponent() -> f32 {
    2.3
}

/// How a structure's stems are meshed and shaded.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Bark {
    /// sRGB, 0..1.
    pub color: [f32; 3],
    /// Perceptual roughness, 0..1.
    pub roughness: f32,
    /// Sides of the thickest stem at the hero tier; thinner and farther
    /// stems get fewer (see `PlantLod::sides`).
    pub sides: u8,
    /// Stems thinner than this, metres, are never meshed: twigs that the
    /// foliage hides anyway (a conifer's needle sprays) cost no triangles.
    /// Their leaves still hang where they are. 0 = mesh every stem.
    pub min_radius: f32,
    /// Asset path of a bark albedo texture, multiplied by `color`. U runs
    /// once around a stem, V one stem circumference per unit.
    pub texture: Option<String>,
    /// Asset path of a tangent-space bark normal map (OpenGL convention,
    /// like Bevy's `StandardMaterial`); stems carry tangents for it.
    pub normal_map: Option<String>,
    /// Texture repeats around (U) and along (V) a stem.
    pub uv_scale: [f32; 2],
}

impl Default for Bark {
    fn default() -> Self {
        Self {
            color: [0.32, 0.24, 0.17],
            roughness: 0.9,
            sides: 8,
            min_radius: 0.0,
            texture: None,
            normal_map: None,
            uv_scale: [1.0, 1.0],
        }
    }
}

/// A structure's leaves: where they hang, their shape and colour.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Leaves {
    /// sRGB, 0..1.
    pub color: [f32; 3],
    /// Per-leaf brightness jitter, 0..1.
    pub color_jitter: f32,
    /// Stalk to tip, metres, before per-leaf jitter.
    pub length: f32,
    /// Across at the widest, metres.
    pub width: f32,
    /// Leaves at every node within this many segments of a tip, besides any
    /// the grower placed itself (an L-system's `L`). 0 = only those.
    pub tip_depth: u16,
    /// Leaves per node when placed by `tip_depth`, spiralled by the golden
    /// angle.
    pub per_node: u8,
    /// How far leaves bend down from their stem, radians.
    pub droop: f32,
    /// Lengthwise segments: 1 is a flat diamond, more bend it along the
    /// midrib. Geometry leaves, no alpha test (see the research notes on
    /// overdraw).
    pub segments: u8,
    /// How far leaves placed by `tip_depth` lean forward along their stem
    /// (0 = straight out sideways).
    pub along: f32,
    /// 0 spirals leaves placed by `tip_depth` around the stem; 1 keeps them
    /// to its two horizontal flanks: flat sprays, like a conifer's
    /// needles or a beech twig.
    pub planar: f32,
    /// Leaves in each clump: many broadleaves (oaks among them) crowd their
    /// leaves at the shoot tips, and clumps are what make a crown read as
    /// a mass. 1 = single leaves.
    pub cluster: u8,
    /// How far a clump's leaves fan out from its centre, radians.
    pub cluster_spread: f32,
    /// 0..1: bends leaf normals from the leaf's own face towards the
    /// normal of an ellipsoid around the whole crown, so the crown shades
    /// as one soft volume instead of a confetti of flat cards (the
    /// foliage-normals trick of game tree art). Both faces share the
    /// normal, so leaves seen from behind are not black.
    pub crown_normals: f32,
    /// 0..1: darkens leaves deep inside the crown, a cheap baked occlusion.
    pub inner_shade: f32,
    /// 0..1: the share of leaves left out where a smooth noise over the
    /// crown is low, so broadleaf foliage gathers into masses with sky
    /// between them instead of an even blanket.
    pub clumping: f32,
    /// Size of those masses, metres.
    pub clump_size: f32,
    /// Teeth a side for a pinnatifid leaf (fern pinnae, lobed leaves): one
    /// triangle each, cut halfway to the midrib, `2 * lobes` triangles a
    /// leaf at the hero tier; coarser tiers draw a plain one-segment blade. 0 = a plain
    /// blade of `segments`.
    pub lobes: u8,
    /// Fraction of leaves kept at `Near` and `Far`, in place of the
    /// defaults (`PlantLod::leaf_keep`). Kept leaves grow to hold the leaf
    /// area, so a plant whose leaves are already large keeps more, smaller
    /// ones and its outline. `None` = the defaults.
    pub lod_keep: Option<(f32, f32)>,
}

impl Default for Leaves {
    fn default() -> Self {
        Self {
            color: [0.22, 0.42, 0.16],
            color_jitter: 0.15,
            length: 0.12,
            width: 0.06,
            tip_depth: 3,
            per_node: 2,
            droop: 0.6,
            segments: 2,
            along: 0.6,
            planar: 0.0,
            cluster: 1,
            cluster_spread: 0.5,
            crown_normals: 0.0,
            inner_shade: 0.0,
            clumping: 0.0,
            clump_size: 1.5,
            lobes: 0,
            lod_keep: None,
        }
    }
}

/// A plant that is a density on a surface.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Cover {
    /// The shape repeated over the surface.
    pub tuft: Tuft,
    /// sRGB, 0..1.
    pub color: [f32; 3],
    /// Darkened at the base (the shade a dense stand casts on itself) and
    /// lightened at the tips by this much.
    #[serde(default = "default_shade")]
    pub shade: f32,
    /// Per-tuft brightness jitter, 0..1.
    #[serde(default)]
    pub color_jitter: f32,
    /// sRGB colour the tips (blade tips, cushion tops) fade to. `None`:
    /// `color` lightened by `shade`.
    #[serde(default)]
    pub tip_color: Option<[f32; 3]>,
    /// Fraction of blades (or of a cushion's bristles) that are dry, 0..1.
    #[serde(default)]
    pub dry: f32,
    /// sRGB colour of a dry blade.
    #[serde(default = "default_dry_color")]
    pub dry_color: [f32; 3],
    /// Contrast of a fine speckle of shoot tips in the material, 0..1: the
    /// texture far finer than any affordable geometry that makes moss look
    /// velvety. Mipmapped, so it fades to its mean with distance. 0 = none.
    #[serde(default)]
    pub detail: f32,
    /// Size of one speckle (a shoot tip), metres.
    #[serde(default = "default_detail_size")]
    pub detail_size: f32,
}

const fn default_shade() -> f32 {
    0.35
}

const fn default_detail_size() -> f32 {
    0.005
}

const fn default_dry_color() -> [f32; 3] {
    [0.62, 0.55, 0.33]
}

/// The shape repeated over a covered surface.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Tuft {
    /// A clump of geometry blades: grass, sedge, young shoots.
    Blades {
        /// Blades in a tuft at the hero tier.
        count: u8,
        /// Mean blade height, metres.
        height: f32,
        /// Blade width at the base, metres.
        width: f32,
        /// How far a blade bends over, as a fraction of its height. Each
        /// blade draws its own from about half to one and a half times this.
        lean: f32,
        /// Lengthwise segments of the tallest blades at the hero tier.
        segments: u8,
        /// Clump radius as a fraction of `height`.
        #[serde(default = "default_spread")]
        spread: f32,
        /// Heights spread from `1 - height_jitter` to `1 + height_jitter / 2`
        /// times `height`, short blades the most common.
        #[serde(default = "default_height_jitter")]
        height_jitter: f32,
        /// How far blades tilt out from the clump's centre, radians at the
        /// clump's rim.
        #[serde(default)]
        splay: f32,
    },
    /// A low cushion of overlapping mounds with lobed outlines: cushion
    /// moss, lichen rosettes.
    Cushion {
        /// Rim radius, metres.
        radius: f32,
        /// Height at the top, metres.
        height: f32,
        /// Points around the rim at the hero tier.
        sides: u8,
        /// Overlapping mounds (shoot clumps) the cushion is built from, half
        /// as many at the near tier; 0 or 1 = one smooth mound.
        #[serde(default)]
        lumps: u8,
        /// How irregular the outline is: 0 round, 0.4 deeply lobed.
        #[serde(default = "default_lobes")]
        lobes: f32,
        /// Tiny single-triangle shoots standing out of the surface at the
        /// hero tier (a quarter of them at near), for a fine, fuzzy edge.
        #[serde(default)]
        bristles: u16,
        /// Bristle height as a fraction of `height`.
        #[serde(default = "default_bristle_height")]
        bristle_height: f32,
    },
}

const fn default_spread() -> f32 {
    0.25
}

const fn default_height_jitter() -> f32 {
    0.3
}

const fn default_lobes() -> f32 {
    0.15
}

const fn default_bristle_height() -> f32 {
    0.4
}

/// Where a species grows.
///
/// Placement is a pure function of world position:
/// a jittered grid hashed from the world seed, so the same plants come back
/// whenever a region is scattered again and nothing needs saving.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Placement {
    /// Grid cell size, metres: at most one plant per cell.
    pub spacing: f32,
    /// Fraction of cells that get a plant, before the rules below.
    pub probability: f32,
    /// Allowed ground slope, degrees from flat.
    pub slope: Range,
    /// Allowed ground height, metres.
    pub altitude: Range,
    /// Host material ids allowed (the terrain's own ids). Empty = any.
    pub materials: Vec<u8>,
    /// Patchiness: cells where this noise is under `threshold` stay bare.
    pub noise: Option<DensityNoise>,
    /// 0 = stand upright (trees), 1 = follow the ground normal (moss).
    pub align: f32,
    /// Uniform scale: each plant's is drawn evenly from this range.
    pub scale: Range,
    /// Sink the base this far below the ground, metres, so it never floats
    /// on a slope.
    pub sink: f32,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            spacing: 4.0,
            probability: 0.5,
            slope: Range { min: 0.0, max: 35.0 },
            altitude: Range { min: f32::MIN, max: f32::MAX },
            materials: Vec::new(),
            noise: None,
            align: 0.0,
            scale: Range { min: 0.85, max: 1.15 },
            sink: 0.05,
        }
    }
}

/// The lowest and highest value a [`Placement`] rule allows, both included,
/// in that rule's units: `(min: 0.0, max: 35.0)` in a species file.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Range {
    /// The lowest value allowed.
    pub min: f32,
    /// The highest value allowed.
    pub max: f32,
}

/// Smooth noise that leaves bare patches between stands of a species.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct DensityNoise {
    /// Patches per metre.
    pub frequency: f32,
    /// -1..1; higher = barer.
    pub threshold: f32,
    /// Layers of finer noise over the patches; more give raggeder edges.
    pub octaves: u8,
}

impl Default for DensityNoise {
    fn default() -> Self {
        Self { frequency: 0.05, threshold: 0.0, octaves: 3 }
    }
}

/// Loads `*.plant.ron` into a [`Species`].
#[derive(TypePath)]
pub struct SpeciesLoader(PhantomData<()>);

impl Default for SpeciesLoader {
    fn default() -> Self {
        Self(PhantomData)
    }
}

/// Why a `*.plant.ron` file didn't load.
#[derive(Debug, Error)]
pub enum SpeciesLoaderError {
    /// The file couldn't be read.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The file isn't a valid species.
    #[error(transparent)]
    Ron(#[from] ron::error::SpannedError),
}

impl AssetLoader for SpeciesLoader {
    type Asset = Species;
    type Settings = ();
    type Error = SpeciesLoaderError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        (): &(),
        _: &mut LoadContext<'_>,
    ) -> Result<Species, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(ron::de::from_bytes(&bytes)?)
    }

    fn extensions(&self) -> &[&str] {
        &["plant.ron"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The species the README shows, as it is written there: what it leaves
    /// out takes the defaults.
    #[test]
    fn the_readme_species_parses() {
        let readme = include_str!("../README.md");
        let block = readme.split("```ron").nth(1).and_then(|s| s.split("```").next()).unwrap();
        let s: Species = ron::from_str(block).unwrap();
        assert_eq!((s.name.as_str(), s.variants), ("oak", 6));
        let Form::Structure(st) = &s.form else { panic!("the oak is a structure") };
        assert!(matches!(st.growth, Growth::Colonize(_)));
        assert_eq!(st.bark, Bark::default());
        assert!((st.pipe_exponent - default_pipe_exponent()).abs() < f32::EPSILON);
        assert!((s.placement.spacing - 7.0).abs() < f32::EPSILON);
        assert_eq!(s.placement.slope, Range { min: 0.0, max: 25.0 });
        assert_eq!(s.placement.altitude, Placement::default().altitude);
    }

    /// A range names its ends; a bare `(lo, hi)` pair, the format before
    /// ranges were named, doesn't load.
    #[test]
    fn placement_ranges_name_their_ends() {
        let p: Placement = ron::from_str("(slope: (min: 5.0, max: 40.0), scale: (min: 0.5, max: 2.0))").unwrap();
        assert_eq!(p.slope, Range { min: 5.0, max: 40.0 });
        assert_eq!(p.scale, Range { min: 0.5, max: 2.0 });
        assert_eq!(p.altitude, Placement::default().altitude);
        assert!(ron::from_str::<Placement>("(slope: (5.0, 40.0))").is_err());
    }

    #[test]
    fn a_bare_cover_takes_the_defaults() {
        let s: Species = ron::from_str(
            "(name: \"g\", form: Cover((tuft: Blades(count: 4, height: 0.2, width: 0.01, lean: 0.2, segments: 2), color: (0.2, 0.4, 0.1))))",
        )
        .unwrap();
        assert_eq!(s.variants, default_variants());
        assert_eq!(s.placement, Placement::default());
        let Form::Cover(c) = &s.form else { panic!("a cover") };
        assert!((c.shade - default_shade()).abs() < f32::EPSILON);
        assert_eq!(c.dry_color, default_dry_color());
        let Tuft::Blades { spread, height_jitter, .. } = c.tuft else { panic!("blades") };
        assert!((spread - default_spread()).abs() < f32::EPSILON);
        assert!((height_jitter - default_height_jitter()).abs() < f32::EPSILON);
    }
}
