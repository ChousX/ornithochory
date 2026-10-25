//! Interaction: things that push plants aside as they pass.
//!
//! Give any entity a [`FoliageBender`] (the ornisculpt feature gives every
//! mob one). Each frame the benders nearest the cameras are gathered into
//! [`Benders`], which is what a foliage wind shader reads to bend blades
//! and leaves on the GPU (one small buffer, whatever the plant count).
//! Until that shader lands, small hero-tier plants lean away on the CPU.

use bevy::prelude::*;

use crate::plant::{PlantCamera, PlantParts, PlantSet, PlantTier, choose_tiers, tier_cameras};
use crate::mesh::PlantLod;

/// Gathers [`Benders`] and leans small plants away from them, once tiers
/// are chosen. Added by [`crate::OrnithochoryPlugin`].
#[derive(Default)]
pub struct BendPlugin;

impl Plugin for BendPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Benders>()
            .add_systems(Update, (gather_benders, lean_plants).chain().in_set(PlantSet::Tier).after(choose_tiers));
    }
}

/// Pushes plants within `radius` metres aside.
#[derive(Component, Clone, Copy, Debug)]
pub struct FoliageBender {
    /// How far the push reaches, metres, before the entity's scale.
    pub radius: f32,
}

impl Default for FoliageBender {
    fn default() -> Self {
        Self { radius: DEFAULT_BENDER_RADIUS }
    }
}

/// The benders nearest the cameras this frame: `xyz` position, `w` radius.
#[derive(Resource, Clone, Debug, Default)]
pub struct Benders(pub Vec<Vec4>);

impl Benders {
    /// Most benders kept; the nearest win.
    pub const MAX: usize = 64;
}

/// A [`FoliageBender`]'s radius unless it says otherwise, metres: about a
/// person or a small animal.
const DEFAULT_BENDER_RADIUS: f32 = 0.5;
/// Plants wider than this (metres) don't lean: trees aren't pushed aside.
const MAX_LEAN_RADIUS: f32 = 1.5;
/// The most a plant leans, radians.
const MAX_LEAN: f32 = 0.45;
/// How fast a plant follows its lean, per second.
const LEAN_RATE: f32 = 8.0;
/// How much of a plant's own radius adds to a bender's reach: a bender
/// brushing the edge of a bush still pushes it.
const PLANT_REACH: f32 = 0.5;

fn gather_benders(
    cameras: Query<(&GlobalTransform, &Projection, &Camera, Has<PlantCamera>, Has<Camera3d>)>,
    benders: Query<(&GlobalTransform, &FoliageBender)>,
    mut out: ResMut<Benders>,
) {
    let eyes: Vec<Vec3> = tier_cameras(cameras.iter()).iter().map(|(t, ..)| t.translation()).collect();
    let all = benders.iter().map(|(t, b)| t.translation().extend(b.radius * t.scale().max_element()));
    out.0.clear();
    out.0.extend(nearest(&eyes, all));
}

/// The [`Benders::MAX`] benders (`xyz` position, `w` radius) nearest any of
/// `eyes`, nearest first.
fn nearest(eyes: &[Vec3], benders: impl Iterator<Item = Vec4>) -> impl Iterator<Item = Vec4> {
    let near = |p: Vec3| eyes.iter().map(|e| e.distance_squared(p)).fold(f32::MAX, f32::min);
    let mut all: Vec<(f32, Vec4)> = benders.map(|b| (near(b.truncate()), b)).collect();
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    all.into_iter().take(Benders::MAX).map(|(_, v)| v)
}

fn lean_plants(
    time: Res<Time>,
    benders: Res<Benders>,
    plants: Query<(&GlobalTransform, &PlantParts, &PlantTier)>,
    mut parts: Query<&mut Transform>,
) {
    let k = 1.0 - (-LEAN_RATE * time.delta_secs()).exp();
    for (transform, plant, tier) in &plants {
        if tier.0 != PlantLod::Hero || plant.radius > MAX_LEAN_RADIUS {
            continue;
        }
        let target = lean(transform.translation(), plant.radius, &benders.0);
        for part in [plant.stems, plant.foliage] {
            if let Ok(mut t) = parts.get_mut(part) {
                t.rotation = t.rotation.slerp(target, k);
            }
        }
    }
}

/// The rotation a plant rooted at `root`, `radius` metres wide, leans to
/// away from `benders`: harder the nearer they are, at most [`MAX_LEAN`].
fn lean(root: Vec3, radius: f32, benders: &[Vec4]) -> Quat {
    let push = benders.iter().fold(Vec3::ZERO, |acc, b| {
        let away = (root - b.truncate()).with_y(0.0);
        let reach = radius.mul_add(PLANT_REACH, b.w);
        let d = away.length();
        // A bender right on the root pushes no way in particular.
        if d >= reach || d < 1e-4 {
            return acc;
        }
        acc + away / d * (1.0 - d / reach)
    });
    let lean = push.clamp_length_max(1.0) * MAX_LEAN;
    // Tilt about the horizontal axis perpendicular to the push.
    if lean.length_squared() > 1e-8 {
        Quat::from_axis_angle(Vec3::Y.cross(lean).normalize(), lean.length())
    } else {
        Quat::IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plants_lean_away_and_less_from_farther() {
        let near = lean(Vec3::ZERO, 0.4, &[Vec4::new(-0.2, 0.0, 0.0, 0.5)]) * Vec3::Y;
        let far = lean(Vec3::ZERO, 0.4, &[Vec4::new(-0.5, 0.0, 0.0, 0.5)]) * Vec3::Y;
        assert!(near.x > far.x && far.x > 0.0, "{near} {far}");
        assert!(Vec3::Y.angle_between(near) <= MAX_LEAN + 1e-5);
    }

    #[test]
    fn out_of_reach_or_on_the_root_is_upright() {
        assert_eq!(lean(Vec3::ZERO, 0.4, &[Vec4::new(3.0, 0.0, 0.0, 0.5)]), Quat::IDENTITY);
        assert_eq!(lean(Vec3::ZERO, 0.4, &[Vec4::new(0.0, 1.0, 0.0, 0.5)]), Quat::IDENTITY);
        assert_eq!(lean(Vec3::ZERO, 0.4, &[]), Quat::IDENTITY);
    }

    #[test]
    fn the_nearest_benders_win() {
        let eyes = [Vec3::ZERO, Vec3::new(100.0, 0.0, 0.0)];
        let all = (0u8..100).map(|i| Vec4::new(f32::from(i), 0.0, 0.0, 1.0));
        let kept: Vec<Vec4> = nearest(&eyes, all).collect();
        assert_eq!(kept.len(), Benders::MAX);
        // Nearest first: one at each eye, then outwards from both.
        assert_eq!(kept.first().map(|b| b.x), Some(0.0));
        assert!(kept.iter().all(|b| b.x <= 32.0 || b.x >= 69.0), "{kept:?}");
    }
}
