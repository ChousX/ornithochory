//! Plant levers: the values of a [`Species`] a design may vary, each with the
//! bounds it makes sense within.
//!
//! A lever names one or more values by their place in the species (the paths
//! a `genetics` genome uses; see [`crate::design`]), so which ones it reaches
//! depends on how the species grows: [`PlantLever::Step`] is a node spacing
//! for every grower, [`PlantLever::Height`] a trunk's length or a tuft's
//! height. Each value a lever reaches varies on its own.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::grow::{Envelope, Growth};
use crate::species::{Cover, Form, Species, Structure, Tuft};

/// One kind of value a design may vary; [`PlantLever::paths`] finds where
/// it sits in a given species.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PlantLever {
    /// A [`Recursive`](crate::grow::Recursive) trunk's length, or a tuft's
    /// height, metres.
    Height,
    /// A cushion's radius, metres.
    Radius,
    /// Node spacing (an L-system's `F`), metres.
    Step,
    /// An L-system's turn angle, degrees.
    Angle,
    /// How strongly an L-system's stems bend towards its tropism.
    Tropism,
    /// A named constant of an L-system's `defines`.
    Define(String),
    /// The width of the crown envelope (an ellipsoid's x and z radii, a
    /// cone's or vase's radius), metres.
    CrownRadius,
    /// The height of the crown envelope (an ellipsoid's y radius, a cone's
    /// or vase's height), metres.
    CrownHeight,
    /// Space colonization: how far a point reaches, metres.
    Influence,
    /// Space colonization: how close a node uses a point up, metres.
    Kill,
    /// Space colonization: how much a node keeps its parent's heading, 0..1.
    Inertia,
    /// Space colonization: the trunk's random bend.
    TrunkWander,
    /// Children on each axis of a recursive level.
    LevelCount(u8),
    /// A recursive level's angles from its parent, at both ends of its span,
    /// degrees.
    LevelAngle(u8),
    /// A recursive level's longest child, as a fraction of its parent.
    LevelLength(u8),
    /// A recursive level's bend over its first half, degrees.
    LevelCurve(u8),
    /// A stem tip's radius, metres.
    TipRadius,
    /// Pipe-model exponent: 2 slender to 3 stout.
    PipeExponent,
    /// A leaf's length, metres.
    LeafLength,
    /// A leaf's width, metres.
    LeafWidth,
    /// How far leaves bend down from their stem, radians.
    LeafDroop,
    /// Leaves in a clump.
    LeafCluster,
    /// One sRGB channel (0 red, 1 green, 2 blue) of the bark.
    BarkColor(u8),
    /// One sRGB channel of the leaves.
    LeafColor(u8),
    /// One sRGB channel of a cover.
    CoverColor(u8),
    /// The bark's perceptual roughness, 0..1.
    BarkRoughness,
    /// The fraction of a cover that is dry.
    Dry,
    /// How far blades bend over, as a fraction of their height.
    Lean,
    /// Blades in a tuft.
    BladeCount,
    /// One sRGB channel of a cover's tips, if it names their colour.
    TipColor(u8),
    /// Mounds in a cushion.
    Lumps,
    /// How lobed a cushion's outline is: 0 round, 0.4 deeply lobed.
    Lobes,
    /// How far a cushion's mounds sprawl, as a multiple of the usual.
    Sprawl,
    /// A cushion's mound profile: higher, a flatter top.
    Flatness,
    /// A cushion's mound profile: lower, a steeper drop to the rim.
    Shoulder,
    /// Bristles (single-triangle shoots) on a cushion.
    Bristles,
    /// Bristle height, as a fraction of the cushion's.
    BristleHeight,
    /// How far a cushion's rim bristles lean out, per unit of height.
    BristleSplay,
    /// Sporophytes per cushion, on average.
    Capsules,
    /// Sporophyte stalk height, as a multiple of the cushion's.
    CapsuleHeight,
    /// One sRGB channel of a cushion's sporophytes.
    CapsuleColor(u8),
    /// Grid cell size, metres: at most one plant per cell.
    Spacing,
    /// Fraction of cells that get a plant.
    Probability,
}

/// Why a lever can't turn a species.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LeverError {
    /// The species has nothing for the lever to turn.
    #[error("{lever:?} doesn't apply to `{species}`, which {why}")]
    DoesNotApply {
        /// The lever.
        lever: PlantLever,
        /// The species' name.
        species: String,
        /// What the species lacks, as the end of a sentence.
        why: &'static str,
    },
}

impl PlantLever {
    /// The values this lever may take. A design varies it within a range
    /// inside this one.
    #[must_use]
    pub const fn range(&self) -> (f32, f32) {
        use std::f32::consts::PI;
        match self {
            Self::Height => (0.01, 60.0),
            Self::Radius => (0.01, 5.0),
            Self::Step => (0.01, 2.0),
            Self::Angle | Self::LevelAngle(_) => (0.0, 180.0),
            Self::Define(_) => (-1.0e6, 1.0e6),
            Self::CrownRadius | Self::CrownHeight => (0.05, 40.0),
            Self::Influence => (0.02, 20.0),
            Self::Kill => (0.01, 10.0),
            Self::Inertia
            | Self::TrunkWander
            | Self::BarkColor(_)
            | Self::LeafColor(_)
            | Self::CoverColor(_)
            | Self::TipColor(_)
            | Self::CapsuleColor(_)
            | Self::BarkRoughness
            | Self::Dry
            | Self::Probability => (0.0, 1.0),
            Self::LevelCount(_) | Self::Bristles => (0.0, 400.0),
            Self::Tropism | Self::LevelLength(_) | Self::Lean => (0.0, 2.0),
            Self::LevelCurve(_) => (-180.0, 180.0),
            Self::TipRadius => (0.0005, 0.2),
            Self::PipeExponent => (2.0, 3.0),
            Self::LeafLength => (0.005, 2.0),
            Self::LeafWidth => (0.002, 1.0),
            Self::LeafDroop => (-PI, PI),
            Self::LeafCluster => (1.0, 32.0),
            Self::BladeCount => (1.0, 64.0),
            Self::Lumps => (0.0, 16.0),
            Self::Lobes => (0.0, 0.6),
            Self::Flatness => (0.5, 10.0),
            Self::Shoulder => (0.05, 2.0),
            Self::Sprawl | Self::BristleHeight | Self::BristleSplay => (0.0, 3.0),
            Self::Capsules => (0.0, 64.0),
            Self::CapsuleHeight => (0.0, 10.0),
            Self::Spacing => (0.01, 100.0),
        }
    }

    /// Whether the lever counts whole things (and so varies in whole steps).
    #[must_use]
    pub const fn is_count(&self) -> bool {
        matches!(self, Self::LevelCount(_) | Self::LeafCluster | Self::BladeCount | Self::Lumps | Self::Bristles | Self::Capsules)
    }

    /// Where the values this lever turns sit in `species`, as genome paths.
    ///
    /// # Errors
    ///
    /// When the species has nothing for the lever to turn: a trunk height on
    /// a space-colonized tree, leaves on a leafless one, a fourth colour
    /// channel.
    pub fn paths(&self, species: &Species) -> Result<Vec<String>, LeverError> {
        let not = |why| LeverError::DoesNotApply { lever: self.clone(), species: species.name.clone(), why };
        let one = |path: String| Ok(vec![path]);
        let structure = || match &species.form {
            Form::Structure(s) => Ok(s.as_ref()),
            Form::Cover(_) => Err(not("is a cover")),
        };
        let channel = |c: &u8, base: &str| if *c < 3 { one(format!("{base}.{c}")) } else { Err(not("has three colour channels")) };
        match self {
            Self::Height => match &species.form {
                Form::Structure(s) => match s.growth {
                    Growth::Recursive(_) => one(format!("{G}.Recursive.height")),
                    _ => Err(not("only has a trunk length if it grows recursively")),
                },
                Form::Cover(c) => one(format!("form.Cover.tuft.{}.height", tuft_name(&c.tuft))),
            },
            Self::Radius => match &species.form {
                Form::Cover(c) if matches!(c.tuft, Tuft::Cushion { .. }) => one("form.Cover.tuft.Cushion.radius".into()),
                _ => Err(not("is no cushion")),
            },
            Self::Step => one(format!("{G}.{}.step", growth_name(&structure()?.growth))),
            Self::Angle | Self::Tropism | Self::Define(_) => {
                let Growth::LSystem(l) = &structure()?.growth else { return Err(not("is no L-system")) };
                match self {
                    Self::Angle => one(format!("{G}.LSystem.angle")),
                    Self::Tropism => one(format!("{G}.LSystem.tropism_strength")),
                    Self::Define(key) if l.defines.contains_key(key) => one(format!("{G}.LSystem.defines.{key}")),
                    _ => Err(not("defines no such constant")),
                }
            }
            Self::CrownRadius | Self::CrownHeight => self.crown(structure()?, &not),
            Self::Influence | Self::Kill | Self::Inertia | Self::TrunkWander => {
                let Growth::Colonize(_) = &structure()?.growth else { return Err(not("isn't space-colonized")) };
                let field = match self {
                    Self::Influence => "influence",
                    Self::Kill => "kill",
                    Self::Inertia => "inertia",
                    _ => "trunk_wander",
                };
                one(format!("{G}.Colonize.{field}"))
            }
            Self::LevelCount(i) | Self::LevelAngle(i) | Self::LevelLength(i) | Self::LevelCurve(i) => {
                let Growth::Recursive(r) = &structure()?.growth else { return Err(not("doesn't grow recursively")) };
                if usize::from(*i) >= r.levels.len() {
                    return Err(not("has fewer levels"));
                }
                let base = format!("{G}.Recursive.levels.{i}");
                Ok(match self {
                    Self::LevelCount(_) => vec![format!("{base}.count")],
                    Self::LevelAngle(_) => vec![format!("{base}.angle.0"), format!("{base}.angle.1")],
                    Self::LevelLength(_) => vec![format!("{base}.length")],
                    _ => vec![format!("{base}.curve")],
                })
            }
            Self::TipRadius => structure().and_then(|_| one(format!("{S}.tip_radius"))),
            Self::PipeExponent => structure().and_then(|_| one(format!("{S}.pipe_exponent"))),
            Self::BarkColor(c) => structure().and_then(|_| channel(c, &format!("{S}.bark.color"))),
            Self::BarkRoughness => structure().and_then(|_| one(format!("{S}.bark.roughness"))),
            Self::LeafLength | Self::LeafWidth | Self::LeafDroop | Self::LeafCluster | Self::LeafColor(_) => {
                if structure()?.leaves.is_none() {
                    return Err(not("has no leaves"));
                }
                let base = format!("{S}.leaves");
                match self {
                    Self::LeafLength => one(format!("{base}.length")),
                    Self::LeafWidth => one(format!("{base}.width")),
                    Self::LeafDroop => one(format!("{base}.droop")),
                    Self::LeafCluster => one(format!("{base}.cluster")),
                    Self::LeafColor(c) => channel(c, &format!("{base}.color")),
                    _ => Err(not("has no such leaf value")),
                }
            }
            Self::CoverColor(_) | Self::TipColor(_) | Self::Dry | Self::Lean | Self::BladeCount => {
                let Form::Cover(cover) = &species.form else { return Err(not("isn't a cover")) };
                match (self, &cover.tuft) {
                    (Self::CoverColor(c), _) => channel(c, "form.Cover.color"),
                    (Self::TipColor(c), _) if cover.tip_color.is_some() => channel(c, "form.Cover.tip_color"),
                    (Self::TipColor(_), _) => Err(not("lightens its tips from its colour")),
                    (Self::Dry, _) => one("form.Cover.dry".into()),
                    (Self::Lean, Tuft::Blades { .. }) => one("form.Cover.tuft.Blades.lean".into()),
                    (Self::BladeCount, Tuft::Blades { .. }) => one("form.Cover.tuft.Blades.count".into()),
                    _ => Err(not("has no blades")),
                }
            }
            Self::Lumps
            | Self::Lobes
            | Self::Sprawl
            | Self::Flatness
            | Self::Shoulder
            | Self::Bristles
            | Self::BristleHeight
            | Self::BristleSplay
            | Self::Capsules
            | Self::CapsuleHeight
            | Self::CapsuleColor(_) => self.cushion(species, &not),
            Self::Spacing => one("placement.spacing".into()),
            Self::Probability => one("placement.probability".into()),
        }
    }

    /// The crown envelope's paths for [`Self::CrownRadius`] or
    /// [`Self::CrownHeight`].
    fn crown(&self, structure: &Structure, not: &dyn Fn(&'static str) -> LeverError) -> Result<Vec<String>, LeverError> {
        let (growth, envelope) = match &structure.growth {
            Growth::Colonize(c) => ("Colonize", Some(&c.envelope)),
            Growth::LSystem(l) => ("LSystem", l.envelope.as_ref()),
            Growth::Recursive(_) => ("Recursive", None),
        };
        let envelope = envelope.ok_or_else(|| not("has no envelope"))?;
        let base = format!("{G}.{growth}.envelope");
        let radius = matches!(self, Self::CrownRadius);
        Ok(match envelope {
            Envelope::Ellipsoid { .. } if radius => vec![format!("{base}.Ellipsoid.radii.0"), format!("{base}.Ellipsoid.radii.2")],
            Envelope::Ellipsoid { .. } => vec![format!("{base}.Ellipsoid.radii.1")],
            Envelope::Cone { .. } => vec![format!("{base}.Cone.{}", if radius { "radius" } else { "height" })],
            Envelope::Vase { .. } => vec![format!("{base}.Vase.{}", if radius { "radius" } else { "height" })],
        })
    }

    /// A cushion's path for one of its levers ([`Self::Lumps`] through
    /// [`Self::CapsuleColor`]).
    fn cushion(&self, species: &Species, not: &dyn Fn(&'static str) -> LeverError) -> Result<Vec<String>, LeverError> {
        let Form::Cover(Cover { tuft: Tuft::Cushion { .. }, .. }) = &species.form else { return Err(not("is no cushion")) };
        let field = match self {
            Self::CapsuleColor(c) if *c < 3 => return Ok(vec![format!("{CUSHION}.capsule_color.{c}")]),
            Self::CapsuleColor(_) => return Err(not("has three colour channels")),
            Self::Lumps => "lumps",
            Self::Lobes => "lobes",
            Self::Sprawl => "sprawl",
            Self::Flatness => "flatness",
            Self::Shoulder => "shoulder",
            Self::Bristles => "bristles",
            Self::BristleHeight => "bristle_height",
            Self::BristleSplay => "bristle_splay",
            Self::Capsules => "capsules",
            Self::CapsuleHeight => "capsule_height",
            _ => return Err(not("has no such cushion value")),
        };
        Ok(vec![format!("{CUSHION}.{field}")])
    }
}

const S: &str = "form.Structure";
const CUSHION: &str = "form.Cover.tuft.Cushion";
const G: &str = "form.Structure.growth";

const fn growth_name(growth: &Growth) -> &'static str {
    match growth {
        Growth::LSystem(_) => "LSystem",
        Growth::Colonize(_) => "Colonize",
        Growth::Recursive(_) => "Recursive",
    }
}

const fn tuft_name(tuft: &Tuft) -> &'static str {
    match tuft {
        Tuft::Blades { .. } => "Blades",
        Tuft::Cushion { .. } => "Cushion",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn species(name: &str) -> Species {
        let path = format!("{}/assets/species/{name}.plant.ron", env!("CARGO_MANIFEST_DIR"));
        ron::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn paths(lever: PlantLever, species: &Species) -> Vec<String> {
        lever.paths(species).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn crown_levers_follow_the_envelope() {
        let oak = species("oak");
        let base = "form.Structure.growth.Colonize.envelope.Ellipsoid.radii";
        assert_eq!(paths(PlantLever::CrownRadius, &oak), [format!("{base}.0"), format!("{base}.2")]);
        assert_eq!(paths(PlantLever::CrownHeight, &oak), [format!("{base}.1")]);
        let bush = species("bush");
        assert_eq!(paths(PlantLever::CrownHeight, &bush), ["form.Structure.growth.LSystem.envelope.Ellipsoid.radii.1"]);
        // The fern has no envelope, and recursive growth never does.
        assert!(PlantLever::CrownRadius.paths(&species("fern")).is_err());
        assert!(PlantLever::CrownRadius.paths(&species("spruce")).is_err());
    }

    #[test]
    fn growth_levers_need_their_grower() {
        let oak = species("oak");
        assert_eq!(paths(PlantLever::Influence, &oak), ["form.Structure.growth.Colonize.influence"]);
        assert_eq!(paths(PlantLever::Step, &oak), ["form.Structure.growth.Colonize.step"]);
        assert!(PlantLever::Height.paths(&oak).is_err());
        assert!(PlantLever::Angle.paths(&oak).is_err());

        let spruce = species("spruce");
        assert_eq!(paths(PlantLever::Height, &spruce), ["form.Structure.growth.Recursive.height"]);
        let level = "form.Structure.growth.Recursive.levels.1";
        assert_eq!(paths(PlantLever::LevelAngle(1), &spruce), [format!("{level}.angle.0"), format!("{level}.angle.1")]);
        assert!(PlantLever::LevelCount(2).paths(&spruce).is_err(), "the spruce has two levels");
        assert!(PlantLever::Kill.paths(&spruce).is_err());

        let bush = species("bush");
        assert_eq!(paths(PlantLever::Define("stems".into()), &bush), ["form.Structure.growth.LSystem.defines.stems"]);
        assert!(PlantLever::Define("nope".into()).paths(&bush).is_err());
        assert_eq!(paths(PlantLever::Angle, &bush), ["form.Structure.growth.LSystem.angle"]);
    }

    #[test]
    fn structure_and_cover_levers_stay_apart() {
        let oak = species("oak");
        assert_eq!(paths(PlantLever::LeafColor(1), &oak), ["form.Structure.leaves.color.1"]);
        assert!(PlantLever::LeafColor(3).paths(&oak).is_err(), "three colour channels");
        assert_eq!(paths(PlantLever::TipRadius, &oak), ["form.Structure.tip_radius"]);
        assert!(PlantLever::Dry.paths(&oak).is_err());

        let grass = species("grass");
        assert_eq!(paths(PlantLever::Height, &grass), ["form.Cover.tuft.Blades.height"]);
        assert_eq!(paths(PlantLever::BladeCount, &grass), ["form.Cover.tuft.Blades.count"]);
        assert_eq!(paths(PlantLever::CoverColor(0), &grass), ["form.Cover.color.0"]);
        assert!(PlantLever::Radius.paths(&grass).is_err());
        assert!(PlantLever::TipRadius.paths(&grass).is_err());

        let moss = species("moss");
        assert_eq!(paths(PlantLever::Radius, &moss), ["form.Cover.tuft.Cushion.radius"]);
        assert_eq!(paths(PlantLever::Height, &moss), ["form.Cover.tuft.Cushion.height"]);
        assert!(PlantLever::Lean.paths(&moss).is_err(), "a cushion has no blades");
        assert_eq!(paths(PlantLever::Capsules, &moss), ["form.Cover.tuft.Cushion.capsules"]);
        assert_eq!(paths(PlantLever::Flatness, &moss), ["form.Cover.tuft.Cushion.flatness"]);
        assert_eq!(paths(PlantLever::CapsuleColor(2), &moss), ["form.Cover.tuft.Cushion.capsule_color.2"]);
        assert_eq!(paths(PlantLever::TipColor(1), &moss), ["form.Cover.tip_color.1"]);
        assert!(PlantLever::Sprawl.paths(&grass).is_err(), "grass is no cushion");
        assert!(PlantLever::Bristles.paths(&oak).is_err(), "an oak is no cushion");
        assert_eq!(paths(PlantLever::TipColor(0), &grass), ["form.Cover.tip_color.0"]);
        // A cover that names no tip colour lightens its own.
        let mut plain = grass.clone();
        if let Form::Cover(c) = &mut plain.form {
            c.tip_color = None;
        }
        assert!(PlantLever::TipColor(0).paths(&plain).is_err());

        // Placement applies to every species.
        for s in [&oak, &grass, &moss] {
            assert_eq!(paths(PlantLever::Spacing, s), ["placement.spacing"]);
            assert_eq!(paths(PlantLever::Probability, s), ["placement.probability"]);
        }
    }

    #[test]
    fn ranges_are_ordered_and_counts_are_whole() {
        let all = [
            PlantLever::Height,
            PlantLever::Radius,
            PlantLever::Step,
            PlantLever::Angle,
            PlantLever::Tropism,
            PlantLever::Define("k".into()),
            PlantLever::CrownRadius,
            PlantLever::CrownHeight,
            PlantLever::Influence,
            PlantLever::Kill,
            PlantLever::Inertia,
            PlantLever::TrunkWander,
            PlantLever::LevelCount(0),
            PlantLever::LevelAngle(0),
            PlantLever::LevelLength(0),
            PlantLever::LevelCurve(0),
            PlantLever::TipRadius,
            PlantLever::PipeExponent,
            PlantLever::LeafLength,
            PlantLever::LeafWidth,
            PlantLever::LeafDroop,
            PlantLever::LeafCluster,
            PlantLever::BarkColor(0),
            PlantLever::LeafColor(0),
            PlantLever::CoverColor(0),
            PlantLever::BarkRoughness,
            PlantLever::Dry,
            PlantLever::Lean,
            PlantLever::BladeCount,
            PlantLever::TipColor(0),
            PlantLever::Lumps,
            PlantLever::Lobes,
            PlantLever::Sprawl,
            PlantLever::Flatness,
            PlantLever::Shoulder,
            PlantLever::Bristles,
            PlantLever::BristleHeight,
            PlantLever::BristleSplay,
            PlantLever::Capsules,
            PlantLever::CapsuleHeight,
            PlantLever::CapsuleColor(0),
            PlantLever::Spacing,
            PlantLever::Probability,
        ];
        for lever in &all {
            let (lo, hi) = lever.range();
            assert!(lo < hi, "{lever:?}");
            if lever.is_count() {
                assert!(lo >= 0.0 && lo.fract().abs() < f32::EPSILON && hi.fract().abs() < f32::EPSILON, "{lever:?}");
            }
        }
        assert_eq!(all.iter().filter(|l| l.is_count()).count(), 6);
    }
}
