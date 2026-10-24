//! Plants on tarasaur terrain.
//!
//! List the species to grow in [`Flora`]. Each chunk is scattered when its
//! fields land at a LOD an entry allows, and again when the chunk is edited
//! (once the SDF's jump flood has settled) or changes LOD. Plants are
//! children of their chunk, so they unload with it. Placement is a pure
//! function of the world seed and position, so nothing is saved: a chunk
//! that streams back in grows the same plants. By default plants grow only
//! on terrain that is shown (`Flora::clipped`), so revealing ground grows
//! its plants and hiding it clears them.
//!
//! Stemmed species spawn one [`Plant`] each; cover species one
//! [`CoverPatch`] per chunk, at a tier matched to the chunk's LOD.
//!
//! Cost: one downward ray per candidate that passes the cheap rules,
//! budgeted by [`Flora::casts_per_frame`]; patches mesh off-thread.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use bevy::prelude::*;
use tarasaur::chunk::{CHUNK_SIZE, Chunk, ChunkManager, ChunkPosition, ChunkSet, world_pos_to_chunk_pos};
use tarasaur::field::{DirtyField, Field, LOD, MaterialField, SDF, VisibilityField, VoxelField, VoxelMaterial};
use tarasaur::raycast::collider::cast;
use tarasaur::raycast::terrain_ray::TerrainRay;

use crate::mesh::PlantLod;
use crate::num::{f32_from_u32, u32_from_f32};
use crate::plant::{CoverPatch, Plant, PlantSet};
use crate::scatter::{GroundHit, scatter, species_seed};
use crate::species::{Form, Species};

/// What grows on the terrain.
#[derive(Resource, Clone, Debug)]
pub struct Flora {
    pub world_seed: u64,
    pub entries: Vec<FloraEntry>,
    /// Ground rays per frame, across all chunks. A chunk is never split
    /// across frames, so one chunk may overshoot this.
    pub casts_per_frame: usize,
    /// Seconds after an edit before its chunk re-scatters, so a brush
    /// stroke re-scatters once, not every frame.
    pub settle_secs: f32,
    /// Grow only on terrain as drawn: the `SDF` clipped by its
    /// `VisibilityField`, so hidden ground stays bare and a chunk
    /// re-scatters when what is shown changes. `false` grows on the ground
    /// under hidden regions too. Without a `VisibilityField` the two agree.
    pub clipped: bool,
}

impl Default for Flora {
    fn default() -> Self {
        Self { world_seed: 0, entries: Vec::new(), casts_per_frame: 4000, settle_secs: 0.3, clipped: true }
    }
}

/// One species on the terrain.
#[derive(Clone, Debug)]
pub struct FloraEntry {
    pub species: Handle<Species>,
    /// The coarsest chunk LOD this species is placed on. Grass only on
    /// `High` chunks; trees down to `Medium`.
    pub min_lod: LOD,
}

/// Marks the plants and patches Ornithochory scattered on a chunk.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct FloraPlant;

/// Chunks waiting to be scattered, with the time they become due.
#[derive(Resource, Default)]
struct FloraQueue(HashMap<Entity, f32>);

/// Scatters [`Flora`] over the terrain. `M` is the app's voxel material,
/// whose ids `Placement::materials` lists.
pub struct TerrainPlantsPlugin<M: VoxelMaterial>(PhantomData<M>);

impl<M: VoxelMaterial> Default for TerrainPlantsPlugin<M> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<M: VoxelMaterial> Plugin for TerrainPlantsPlugin<M> {
    fn build(&self, app: &mut App) {
        app.init_resource::<Flora>().init_resource::<FloraQueue>().add_systems(
            Update,
            (queue_chunks, scatter_chunks::<M>)
                .chain()
                .after(ChunkSet::Plan)
                .before(PlantSet::Generate),
        );
    }
}

/// The plant tier a chunk's LOD calls for: cover patches have no per-plant
/// tier, so they follow the terrain's own detail.
const fn cover_tier(lod: LOD) -> PlantLod {
    match lod {
        LOD::High => PlantLod::Hero,
        LOD::Medium => PlantLod::Near,
        LOD::Low | LOD::Lowest => PlantLod::Far,
    }
}

fn queue_chunks(
    time: Res<Time>,
    flora: Res<Flora>,
    mut queue: ResMut<FloraQueue>,
    lod_changed: Query<Entity, (With<Chunk>, Changed<LOD>)>,
    edited: Query<Entity, (With<Chunk>, Changed<SDF>)>,
    revealed: Query<Entity, (With<Chunk>, Changed<VisibilityField>)>,
) {
    let now = time.elapsed_secs();
    for e in &lod_changed {
        queue.0.insert(e, now);
    }
    let shown_changed = revealed.iter().filter(|_| flora.clipped);
    for e in edited.iter().chain(shown_changed) {
        let due = queue.0.entry(e).or_insert(now);
        *due = due.max(now + flora.settle_secs);
    }
}

/// The voxel material at `p`, read just inside the surface.
fn material_at<M: VoxelMaterial>(
    chunks: &ChunkManager,
    materials: &Query<&MaterialField<M>>,
    p: Vec3,
    normal: Vec3,
) -> Option<u8> {
    let cp = world_pos_to_chunk_pos(&p);
    let field = materials.get(chunks.get_chunk(&cp)?).ok()?;
    let size = field.lod().size();
    let voxel = CHUNK_SIZE / f32_from_u32(size);
    let inside = p - normal * (voxel * 0.5);
    let local = ((inside - cp.as_vec3() * CHUNK_SIZE) / voxel).round();
    let edge = size.saturating_sub(1);
    let c = |v: f32| u32_from_f32(v).min(edge);
    Some(field.get(c(local.x), c(local.y), c(local.z)).to_id())
}

fn scatter_chunks<M: VoxelMaterial>(
    mut commands: Commands,
    time: Res<Time>,
    flora: Res<Flora>,
    species: Res<Assets<Species>>,
    mut queue: ResMut<FloraQueue>,
    terrain: TerrainRay,
    chunk_manager: Res<ChunkManager>,
    chunks: Query<(&ChunkPosition, &LOD, Option<&Children>), (With<Chunk>, Without<DirtyField<SDF, f32>>)>,
    materials: Query<&MaterialField<M>>,
    flora_children: Query<(), With<FloraPlant>>,
) {
    if flora.entries.is_empty() || queue.0.is_empty() {
        return;
    }
    let now = time.elapsed_secs();
    // Oldest due first, so a busy edit can't starve the rest.
    let mut due: Vec<(Entity, f32)> = queue.0.iter().filter(|(_, t)| **t <= now).map(|(e, t)| (*e, *t)).collect();
    due.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));

    let mut casts = 0usize;
    for (chunk, _) in due {
        if casts >= flora.casts_per_frame {
            break;
        }
        let Ok((&ChunkPosition(cp), &lod, children)) = chunks.get(chunk) else {
            // Gone, or its SDF is still settling: drop it if gone, else retry.
            if commands.get_entity(chunk).is_err() {
                queue.0.remove(&chunk);
            }
            continue;
        };
        // Wait until every species it needs has loaded.
        let wanted: Vec<(&Handle<Species>, &Species)> = flora
            .entries
            .iter()
            .filter(|e| lod >= e.min_lod)
            .filter_map(|e| species.get(&e.species).map(|s| (&e.species, s)))
            .collect();
        if wanted.len() < flora.entries.iter().filter(|e| lod >= e.min_lod).count() {
            continue;
        }
        queue.0.remove(&chunk);

        for &child in children.into_iter().flatten() {
            if flora_children.contains(child) {
                commands.entity(child).despawn();
            }
        }

        let origin = cp.as_vec3() * CHUNK_SIZE;
        let (min, max) = (origin.xz(), origin.xz() + Vec2::splat(CHUNK_SIZE));
        let (y0, y1) = (origin.y, origin.y + CHUNK_SIZE);
        let view = if flora.clipped { terrain.collider() } else { terrain.unclipped() };
        let mut ground = |p: Vec2| {
            casts = casts.saturating_add(1);
            let hit = cast(&view, Vec3::new(p.x, y1, p.y), Vec3::NEG_Y, CHUNK_SIZE)?;
            (hit.pos.y >= y0 && hit.pos.y < y1).then(|| GroundHit {
                pos: hit.pos,
                normal: hit.normal,
                material: material_at(&chunk_manager, &materials, hit.pos, hit.normal),
            })
        };

        for (handle, s) in wanted {
            let placed = scatter(&s.placement, species_seed(flora.world_seed, s), min, max, &mut ground);
            if placed.is_empty() {
                continue;
            }
            match &s.form {
                Form::Structure(_) => {
                    for p in placed {
                        let mut t = p.transform;
                        t.translation -= origin;
                        commands.spawn((Plant { species: handle.clone(), seed: p.seed }, t, FloraPlant, ChildOf(chunk)));
                    }
                }
                Form::Cover(_) => {
                    let tufts: Arc<[_]> = placed
                        .into_iter()
                        .map(|mut p| {
                            p.transform.translation -= origin;
                            p
                        })
                        .collect();
                    commands.spawn((
                        CoverPatch { species: handle.clone(), tufts, lod: cover_tier(lod) },
                        FloraPlant,
                        ChildOf(chunk),
                    ));
                }
            }
        }
    }
}
