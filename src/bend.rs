//! Interaction: things that push plants aside as they pass.
//!
//! Give any entity a [`FoliageBender`] (the ornisculpt feature gives every
//! mob one). Each frame the benders nearest the cameras are gathered into
//! [`Benders`], which is what a foliage wind shader reads to bend blades
//! and leaves on the GPU (one small buffer, whatever the plant count).
//! Until that shader lands, small hero-tier plants lean away on the CPU.

use bevy::prelude::*;

use crate::plant::{PlantCamera, PlantParts, PlantTier, tier_cameras};
use crate::mesh::PlantLod;

/// Pushes plants within `radius` metres aside.
#[derive(Component, Clone, Copy, Debug)]
pub struct FoliageBender {
    pub radius: f32,
}

impl Default for FoliageBender {
    fn default() -> Self {
        Self { radius: 0.5 }
    }
}

/// The benders nearest the cameras this frame: `xyz` position, `w` radius.
#[derive(Resource, Clone, Debug, Default)]
pub struct Benders(pub Vec<Vec4>);

impl Benders {
    /// Most benders kept; the nearest win.
    pub const MAX: usize = 64;
}

/// Plants wider than this (metres) don't lean: trees aren't pushed aside.
const MAX_LEAN_RADIUS: f32 = 1.5;
/// The most a plant leans, radians.
const MAX_LEAN: f32 = 0.45;
/// How fast a plant follows its lean, per second.
const LEAN_RATE: f32 = 8.0;

pub(crate) fn gather_benders(
    cameras: Query<(&GlobalTransform, &Projection, &Camera, Has<PlantCamera>, Has<Camera3d>)>,
    benders: Query<(&GlobalTransform, &FoliageBender)>,
    mut out: ResMut<Benders>,
) {
    let eyes: Vec<Vec3> = tier_cameras(cameras.iter()).iter().map(|(t, ..)| t.translation()).collect();
    let near = |p: Vec3| eyes.iter().map(|e| e.distance_squared(p)).fold(f32::MAX, f32::min);
    let mut all: Vec<(f32, Vec4)> = benders
        .iter()
        .map(|(t, b)| {
            let p = t.translation();
            (near(p), p.extend(b.radius * t.scale().max_element()))
        })
        .collect();
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.0.clear();
    out.0.extend(all.into_iter().take(Benders::MAX).map(|(_, v)| v));
}

pub(crate) fn lean_plants(
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
        let root = transform.translation();
        let push = benders.0.iter().fold(Vec3::ZERO, |acc, b| {
            let away = (root - b.truncate()).with_y(0.0);
            let reach = plant.radius.mul_add(0.5, b.w);
            let d = away.length();
            if d >= reach || d < 1e-4 {
                return acc;
            }
            acc + away / d * (1.0 - d / reach)
        });
        let lean = push.clamp_length_max(1.0) * MAX_LEAN;
        // Tilt about the horizontal axis perpendicular to the push.
        let target = if lean.length_squared() > 1e-8 {
            Quat::from_axis_angle(Vec3::Y.cross(lean).normalize(), lean.length())
        } else {
            Quat::IDENTITY
        };
        for part in [plant.stems, plant.foliage] {
            if let Ok(mut t) = parts.get_mut(part) {
                t.rotation = t.rotation.slerp(target, k);
            }
        }
    }
}
