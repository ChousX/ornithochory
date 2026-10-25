//! Ornisculpt mobs in the undergrowth.
//!
//! - Every `Mob` is a [`FoliageBender`] as wide as its generated body, so
//!   it pushes small plants aside (and, once the wind shader lands, bends
//!   grass on the GPU).
//! - A camera with ornisculpt's `TierCamera` also picks plant tiers, so one
//!   marker drives both crates' LOD.

use bevy::prelude::*;
use ornisculpt::mob::{GeneratedBody, Mob, MobBody, MobSet, TierCamera};

use crate::bend::FoliageBender;
use crate::plant::{PlantCamera, PlantSet};

/// How much of a body's bounding radius pushes plants. Bounding spheres
/// include the head and tail, which mostly pass over the undergrowth.
const BODY_PUSH: f32 = 0.6;

/// Makes ornisculpt mobs bend plants and its `TierCamera`s pick plant
/// tiers. Add it beside `OrnithochoryPlugin` and ornisculpt's own plugin.
#[derive(Default)]
pub struct OrnisculptPlantsPlugin;

impl Plugin for OrnisculptPlantsPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(mob_bends)
            .add_observer(tier_camera_picks_plants)
            .add_systems(Update, size_benders.after(MobSet::Generation).before(PlantSet::Tier));
    }
}

fn mob_bends(trigger: On<Add<Mob>>, mut commands: Commands) {
    commands.entity(trigger.entity).try_insert(FoliageBender::default());
}

fn tier_camera_picks_plants(trigger: On<Add<TierCamera>>, mut commands: Commands) {
    commands.entity(trigger.entity).try_insert(PlantCamera);
}

/// A bender as wide as its mob, once the mob's body exists.
fn size_benders(
    bodies: Res<Assets<GeneratedBody>>,
    mut mobs: Query<(&MobBody, &mut FoliageBender), Changed<MobBody>>,
) {
    for (body, mut bender) in &mut mobs {
        if let Some(b) = bodies.get(&body.0) {
            bender.radius = b.radius * BODY_PUSH;
        }
    }
}
