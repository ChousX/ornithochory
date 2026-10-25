//! Plants in the ECS.
//!
//! - [`Plant`]: one individual (a tree, a fern). It shows one of its
//!   species' cached variants, picked by its seed, at the tier its size on
//!   screen calls for. Variants generate on `AsyncComputeTaskPool`, a few at
//!   a time, never on the main thread.
//! - [`CoverPatch`]: many small tufts (grass, moss) merged into one mesh, so
//!   a patch is one entity and one draw however many tufts it holds.
//!
//! Plants sharing a variant share its mesh and material, so Bevy batches
//! them into one GPU-driven draw. Anything denser than about one entity per
//! square metre belongs in a `CoverPatch`.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::camera::Projection;
use bevy::color::{Color, LinearRgba};
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};

use crate::generate::{GeneratedVariant, generate, variant_seed};
use crate::mesh::{MeshData, PlantLod, cover};
use crate::random::{PlantRng, hash_unit};
use crate::scatter::Placed;
use crate::species::{Form, Species};

/// Generates, attaches and tiers [`Plant`]s and [`CoverPatch`]es. Added by
/// [`crate::OrnithochoryPlugin`].
#[derive(Default)]
pub struct PlantPlugin;

impl Plugin for PlantPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlantLibrary>()
            .init_resource::<PlantQuality>()
            .configure_sets(Update, (PlantSet::Generate, PlantSet::Tier).chain())
            .add_systems(
                Update,
                (
                    (
                        reload_species,
                        request_variants,
                        poll_variants,
                        attach_plants,
                        apply_plant_materials,
                        build_cover,
                        finish_cover,
                    )
                        .chain()
                        .in_set(PlantSet::Generate),
                    choose_tiers.in_set(PlantSet::Tier),
                ),
            );
    }
}

/// One plant. Its `Transform` places the root.
#[derive(Component, Clone, Debug)]
#[require(Transform, Visibility)]
pub struct Plant {
    pub species: Handle<Species>,
    /// Picks the variant (`seed % variants`).
    pub seed: u64,
}

/// Cameras that decide plant tiers (the largest on-screen size wins).
/// Optional: without any, every active 3D camera does.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct PlantCamera;

/// Your own materials for a plant or cover patch, in place of the ones
/// built from its species.
///
/// Any `StandardMaterial` works: textures, normal
/// maps (stems carry tangents), transmission, alpha modes. Leave a field
/// `None` to keep the species' material for that part. Foliage meshes
/// carry vertex colours, which `StandardMaterial` multiplies in; use a
/// white `base_color` to keep them, or set your own colours there.
#[derive(Component, Clone, Debug, Default)]
pub struct PlantMaterials {
    pub stems: Option<Handle<StandardMaterial>>,
    pub foliage: Option<Handle<StandardMaterial>>,
}

/// The cameras plant tiers are chosen for: those marked [`PlantCamera`],
/// or every active 3D camera when none is marked, so plants work in a
/// plain Bevy scene with no setup.
pub(crate) fn tier_cameras<'a>(
    cameras: impl Iterator<Item = (&'a GlobalTransform, &'a Projection, &'a Camera, bool, bool)>,
) -> Vec<(GlobalTransform, Projection, f32)> {
    let all: Vec<_> = cameras.filter(|(_, _, c, _, is_3d)| c.is_active && *is_3d).collect();
    let marked = all.iter().any(|(.., m, _)| *m);
    all.into_iter()
        .filter(|(.., m, _)| !marked || *m)
        .filter_map(|(t, p, c, ..)| c.logical_viewport_size().map(|s| (*t, p.clone(), s.y)))
        .collect()
}

/// The tier a plant is shown at. Added once its variant is ready.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlantTier(pub PlantLod);

/// Pins a plant to a tier.
#[derive(Component, Clone, Copy, Debug)]
pub struct ForcePlantTier(pub PlantLod);

/// The child entities drawing a plant.
#[derive(Component, Clone, Copy, Debug)]
pub struct PlantParts {
    pub stems: Entity,
    pub foliage: Entity,
    pub radius: f32,
}

/// Marks a plant's child mesh entities.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct PlantPart;

/// Quality knobs, low-end to high-end. Same shape as ornisculpt's
/// `QualityProfile`.
#[derive(Resource, Clone, Debug)]
pub struct PlantQuality {
    /// On-screen height in pixels at or above which a plant is `Hero` /
    /// `Near`; below is `Far`.
    pub hero_px: f32,
    pub near_px: f32,
    /// Fractional band a size must cross to change tier.
    pub hysteresis: f32,
    /// Variant generation tasks running at once.
    pub max_in_flight: usize,
    /// Cover patch mesh tasks running at once.
    pub max_cover_in_flight: usize,
}

impl Default for PlantQuality {
    fn default() -> Self {
        Self::MEDIUM
    }
}

impl PlantQuality {
    pub const LOW: Self = Self { hero_px: 700.0, near_px: 160.0, hysteresis: 0.15, max_in_flight: 2, max_cover_in_flight: 2 };
    pub const MEDIUM: Self = Self { hero_px: 450.0, near_px: 100.0, hysteresis: 0.15, max_in_flight: 4, max_cover_in_flight: 4 };
    pub const HIGH: Self = Self { hero_px: 300.0, near_px: 60.0, hysteresis: 0.15, max_in_flight: 8, max_cover_in_flight: 8 };
}

/// Ordering of the plant systems in `Update`.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum PlantSet {
    /// Start and finish generation tasks, attach finished meshes.
    Generate,
    /// Pick each plant's tier.
    Tier,
}

type VariantKey = (AssetId<Species>, u8);

/// One variant's uploaded meshes: `(stems, foliage)` per tier.
#[derive(Clone, Debug)]
pub struct ReadyVariant {
    pub tiers: [(Option<Handle<Mesh>>, Option<Handle<Mesh>>); 3],
    pub radius: f32,
}

enum VariantSlot {
    Pending(Task<GeneratedVariant>),
    Ready(ReadyVariant),
}

#[derive(Clone, Debug)]
struct SpeciesMaterials {
    bark: Handle<StandardMaterial>,
    foliage: Handle<StandardMaterial>,
}

impl SpeciesMaterials {
    /// These materials with any a [`PlantMaterials`] replaces.
    fn overridden(self, own: Option<&PlantMaterials>) -> Self {
        let Some(own) = own else { return self };
        Self {
            bark: own.stems.clone().unwrap_or(self.bark),
            foliage: own.foliage.clone().unwrap_or(self.foliage),
        }
    }
}

/// Applies a changed [`PlantMaterials`] to plants and patches already shown.
fn apply_plant_materials(
    mut commands: Commands,
    species: Res<Assets<Species>>,
    mut library: ResMut<PlantLibrary>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    assets: Res<AssetServer>,
    plants: Query<(&Plant, &PlantParts, Ref<PlantMaterials>)>,
    patches: Query<(Entity, &CoverPatch, Ref<PlantMaterials>), With<Mesh3d>>,
    mut removed: RemovedComponents<PlantMaterials>,
    all_plants: Query<(&Plant, &PlantParts)>,
    all_patches: Query<(Entity, &CoverPatch), With<Mesh3d>>,
) {
    let mut set = |commands: &mut Commands, id: AssetId<Species>, s: &Species, own: Option<&PlantMaterials>, parts: (Option<Entity>, Entity)| {
        let mats = library.materials_for(id, s, &mut materials, &mut images, &assets).overridden(own);
        if let Some(stems) = parts.0 {
            commands.entity(stems).insert(MeshMaterial3d(mats.bark));
        }
        commands.entity(parts.1).insert(MeshMaterial3d(mats.foliage));
    };
    for (plant, parts, own) in &plants {
        if own.is_changed()
            && let Some(s) = species.get(&plant.species)
        {
            set(&mut commands, plant.species.id(), s, Some(&own), (Some(parts.stems), parts.foliage));
        }
    }
    for (entity, patch, own) in &patches {
        if own.is_changed()
            && let Some(s) = species.get(&patch.species)
        {
            set(&mut commands, patch.species.id(), s, Some(&own), (None, entity));
        }
    }
    for entity in removed.read() {
        if let Ok((plant, parts)) = all_plants.get(entity)
            && let Some(s) = species.get(&plant.species)
        {
            set(&mut commands, plant.species.id(), s, None, (Some(parts.stems), parts.foliage));
        } else if let Ok((entity, patch)) = all_patches.get(entity)
            && let Some(s) = species.get(&patch.species)
        {
            set(&mut commands, patch.species.id(), s, None, (None, entity));
        }
    }
}

/// Generated variants and materials, shared by every plant of a species.
#[derive(Resource, Default)]
pub struct PlantLibrary {
    variants: HashMap<VariantKey, VariantSlot>,
    materials: HashMap<AssetId<Species>, SpeciesMaterials>,
}

impl PlantLibrary {
    /// The variant's meshes, once generated.
    #[must_use]
    pub fn ready(&self, species: AssetId<Species>, variant: u8) -> Option<&ReadyVariant> {
        match self.variants.get(&(species, variant)) {
            Some(VariantSlot::Ready(r)) => Some(r),
            _ => None,
        }
    }

    fn in_flight(&self) -> usize {
        self.variants.values().filter(|v| matches!(v, VariantSlot::Pending(_))).count()
    }

    fn materials_for(
        &mut self,
        id: AssetId<Species>,
        species: &Species,
        materials: &mut Assets<StandardMaterial>,
        images: &mut Assets<Image>,
        assets: &AssetServer,
    ) -> SpeciesMaterials {
        self.materials
            .entry(id)
            .or_insert_with(|| {
                let (bark, roughness) = match &species.form {
                    Form::Structure(s) => (s.bark.color, s.bark.roughness),
                    Form::Cover(_) => ([1.0; 3], 0.8),
                };
                // Colour comes from the vertices; both faces are drawn.
                let mut foliage = StandardMaterial {
                    perceptual_roughness: 0.7,
                    reflectance: 0.35,
                    cull_mode: None,
                    ..default()
                };
                if let Form::Structure(_) = &species.form {
                    // Leaves are not `double_sided`: that flips the normal
                    // on back faces, which turns every leaf seen from
                    // behind black. The leaf mesher turns each normal to
                    // the lit side (and can bend it to the crown's), so
                    // both faces share it, like a thin leaf lit through.
                    // A little diffuse transmission lights the side turned
                    // from the sun too; without `TransmittedShadowReceiver`
                    // it costs one more Lambert term per light and no
                    // shadow lookup. Matte: at 0.7, sprays seen at a
                    // grazing angle sheen grey.
                    foliage.perceptual_roughness = 0.85;
                    foliage.diffuse_transmission = 0.3;
                }
                if let Form::Cover(c) = &species.form {
                    // Cover normals are bent towards the ground's up (see
                    // `mesh::cover`), so a field shades like the surface
                    // it covers, and they are not flipped on back faces
                    // (`double_sided` stays off): a blade seen from behind
                    // is lit like its front. Rougher, and a fine speckle
                    // of shoot tips if it asks for one, scaled so it
                    // darkens nothing on average (what its mips fade to).
                    foliage.perceptual_roughness = 0.85;
                    foliage.reflectance = 0.25;
                    if c.detail > 0.0 {
                        let (image, mean) = cover::detail_texture(c.detail, crate::random::stable_hash(0, &species.name));
                        foliage.base_color = Color::LinearRgba(LinearRgba::gray(mean.max(0.05).recip()));
                        foliage.base_color_texture = Some(images.add(image));
                    }
                }
                let mut bark_material = StandardMaterial {
                    base_color: Color::srgb(bark[0], bark[1], bark[2]),
                    perceptual_roughness: roughness,
                    ..default()
                };
                if let Form::Structure(s) = &species.form {
                    let b = &s.bark;
                    bark_material.base_color_texture = b.texture.as_ref().map(|p| assets.load(p.clone()));
                    bark_material.normal_map_texture = b.normal_map.as_ref().map(|p| assets.load(p.clone()));
                    bark_material.uv_transform = Affine2::from_scale(Vec2::from(b.uv_scale));
                }
                SpeciesMaterials {
                    bark: materials.add(bark_material),
                    foliage: materials.add(foliage),
                }
            })
            .clone()
    }
}

fn variant_of(species: &Species, seed: u64) -> u8 {
    let n = u64::from(species.variants.max(1));
    seed.checked_rem(n).and_then(|v| u8::try_from(v).ok()).unwrap_or(0)
}

fn request_variants(
    plants: Query<&Plant, Without<PlantParts>>,
    species: Res<Assets<Species>>,
    quality: Res<PlantQuality>,
    mut library: ResMut<PlantLibrary>,
) {
    let mut free = quality.max_in_flight.saturating_sub(library.in_flight());
    for plant in &plants {
        if free == 0 {
            return;
        }
        let Some(s) = species.get(&plant.species) else { continue };
        let key = (plant.species.id(), variant_of(s, plant.seed));
        if library.variants.contains_key(&key) {
            continue;
        }
        let (s, variant) = (s.clone(), key.1);
        let task = AsyncComputeTaskPool::get().spawn(async move { generate(&s, variant) });
        library.variants.insert(key, VariantSlot::Pending(task));
        free = free.saturating_sub(1);
    }
}

fn poll_variants(mut library: ResMut<PlantLibrary>, mut meshes: ResMut<Assets<Mesh>>) {
    for slot in library.variants.values_mut() {
        let VariantSlot::Pending(task) = slot else { continue };
        let Some(done) = check_ready(task) else { continue };
        let mut upload = |m: MeshData| (!m.is_empty()).then(|| meshes.add(m.into_mesh()));
        let [hero, near, far] = done.tiers;
        let mut tier = |t: crate::generate::TierMeshes| (upload(t.stems), upload(t.foliage));
        let tiers = [tier(hero), tier(near), tier(far)];
        *slot = VariantSlot::Ready(ReadyVariant { tiers, radius: done.radius });
    }
}

/// Shows `handle` on a part, or hides the part when the tier has nothing.
fn show(commands: &mut Commands, part: Entity, handle: Option<&Handle<Mesh>>) {
    let mut e = commands.entity(part);
    match handle {
        Some(h) => {
            e.insert((Mesh3d(h.clone()), Visibility::Inherited));
        }
        None => {
            e.insert(Visibility::Hidden);
        }
    }
}

fn attach_plants(
    mut commands: Commands,
    plants: Query<(Entity, &Plant, Option<&ForcePlantTier>, Option<&PlantMaterials>), Without<PlantParts>>,
    species: Res<Assets<Species>>,
    mut library: ResMut<PlantLibrary>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    assets: Res<AssetServer>,
) {
    for (entity, plant, force, own) in &plants {
        let Some(s) = species.get(&plant.species) else { continue };
        let id = plant.species.id();
        let Some(ready) = library.ready(id, variant_of(s, plant.seed)).cloned() else { continue };
        let mats = library.materials_for(id, s, &mut materials, &mut images, &assets).overridden(own);
        // Start coarse; `choose_tiers` promotes it within a frame.
        let tier = force.map_or(PlantLod::Far, |f| f.0);
        let stems = commands.spawn((PlantPart, MeshMaterial3d(mats.bark), ChildOf(entity))).id();
        let foliage = commands.spawn((PlantPart, MeshMaterial3d(mats.foliage), ChildOf(entity))).id();
        if let Some((st, fo)) = ready.tiers.get(tier.index()) {
            show(&mut commands, stems, st.as_ref());
            show(&mut commands, foliage, fo.as_ref());
        }
        commands
            .entity(entity)
            .insert((PlantParts { stems, foliage, radius: ready.radius }, PlantTier(tier)));
    }
}

/// Approximate on-screen height (pixels) of a sphere of `radius` at `pos`.
fn screen_px(cam: &GlobalTransform, proj: &Projection, viewport_h: f32, pos: Vec3, radius: f32) -> f32 {
    match proj {
        Projection::Perspective(p) => {
            let depth = (pos - cam.translation()).dot(cam.forward().as_vec3()).max(0.01);
            radius * 2.0 / (depth * (p.fov * 0.5).tan() * 2.0) * viewport_h
        }
        Projection::Orthographic(o) => radius * 2.0 / o.area.height().max(1e-3) * viewport_h,
        Projection::Custom(_) => viewport_h,
    }
}

/// The tier for `px` on-screen pixels, staying at `current` inside the
/// hysteresis band.
fn tier_for(px: f32, current: PlantLod, q: &PlantQuality) -> PlantLod {
    let wanted = if px >= q.hero_px {
        PlantLod::Hero
    } else if px >= q.near_px {
        PlantLod::Near
    } else {
        PlantLod::Far
    };
    let band = |threshold: f32| (px - threshold).abs() < threshold * q.hysteresis;
    match (current, wanted) {
        (PlantLod::Hero, PlantLod::Near) | (PlantLod::Near, PlantLod::Hero) if band(q.hero_px) => current,
        (PlantLod::Near, PlantLod::Far) | (PlantLod::Far, PlantLod::Near) if band(q.near_px) => current,
        _ => wanted,
    }
}

pub(crate) fn choose_tiers(
    mut commands: Commands,
    quality: Res<PlantQuality>,
    species: Res<Assets<Species>>,
    library: Res<PlantLibrary>,
    cameras: Query<(&GlobalTransform, &Projection, &Camera, Has<PlantCamera>, Has<Camera3d>)>,
    mut plants: Query<(&Plant, &GlobalTransform, &PlantParts, &mut PlantTier, Option<&ForcePlantTier>)>,
) {
    let cams = tier_cameras(cameras.iter());
    if cams.is_empty() {
        return;
    }
    for (plant, transform, parts, mut tier, force) in &mut plants {
        let next = force.map_or_else(
            || {
                let radius = parts.radius * transform.scale().max_element();
                let px = cams
                    .iter()
                    .map(|(t, p, h)| screen_px(t, p, *h, transform.translation(), radius))
                    .fold(0.0, f32::max);
                tier_for(px, tier.0, &quality)
            },
            |f| f.0,
        );
        if next == tier.0 {
            continue;
        }
        let Some(s) = species.get(&plant.species) else { continue };
        let Some(ready) = library.ready(plant.species.id(), variant_of(s, plant.seed)) else { continue };
        let Some((st, fo)) = ready.tiers.get(next.index()) else { continue };
        show(&mut commands, parts.stems, st.as_ref());
        show(&mut commands, parts.foliage, fo.as_ref());
        tier.0 = next;
    }
}

/// Edited species regenerate: their variants and materials are dropped and
/// their plants re-attached.
fn reload_species(
    mut commands: Commands,
    mut events: MessageReader<AssetEvent<Species>>,
    mut library: ResMut<PlantLibrary>,
    plants: Query<(Entity, &Plant, Option<&PlantParts>)>,
    patches: Query<(Entity, &CoverPatch)>,
) {
    let mut changed = Vec::new();
    for ev in events.read() {
        if let AssetEvent::Modified { id } | AssetEvent::Removed { id } = ev {
            changed.push(*id);
        }
    }
    if changed.is_empty() {
        return;
    }
    library.variants.retain(|(id, _), _| !changed.contains(id));
    library.materials.retain(|id, _| !changed.contains(id));
    for (entity, plant, parts) in &plants {
        if !changed.contains(&plant.species.id()) {
            continue;
        }
        if let Some(parts) = parts {
            commands.entity(parts.stems).despawn();
            commands.entity(parts.foliage).despawn();
        }
        commands.entity(entity).remove::<(PlantParts, PlantTier)>();
    }
    for (entity, patch) in &patches {
        if changed.contains(&patch.species.id()) {
            commands.entity(entity).remove::<(CoverBuilt, CoverTask, Mesh3d)>();
        }
    }
}

// ============================================================================
// Cover
// ============================================================================

/// A patch of cover: every tuft in `tufts` (relative to this entity),
/// merged into one mesh at `lod`. Changing it rebuilds the mesh.
#[derive(Component, Clone, Debug)]
#[require(Transform, Visibility)]
pub struct CoverPatch {
    pub species: Handle<Species>,
    pub tufts: Arc<[Placed]>,
    pub lod: PlantLod,
}

/// The patch's mesh is current.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct CoverBuilt;

#[derive(Component)]
struct CoverTask(Task<MeshData>);

/// Merges a patch's tufts: one tuft mesh per variant, copied to every
/// placement with a per-tuft brightness and warmth tint.
#[must_use]
pub fn merge_cover(species: &Species, tufts: &[Placed], lod: PlantLod) -> MeshData {
    let Form::Cover(c) = &species.form else { return MeshData::default() };
    let shapes: Vec<MeshData> = (0..species.variants.max(1))
        .map(|v| cover::tuft(c, lod, variant_seed(species, v)))
        .collect();
    let mut out = MeshData::default();
    for t in tufts {
        let Some(shape) = shapes.get(usize::from(variant_of(species, t.seed))) else { continue };
        let k = PlantRng::new(t.seed).signed().mul_add(c.color_jitter, 1.0) * hash_unit(t.seed, 9).mul_add(0.1, 0.95);
        // Some tufts a little yellower, some bluer, so a stand isn't one green.
        let warm = hash_unit(t.seed, 10).mul_add(2.0, -1.0) * c.color_jitter;
        out.append_transformed(shape, &t.transform, Vec4::new(k * (1.0 + warm), k, k * (1.0 - warm), 1.0));
    }
    out
}

fn build_cover(
    mut commands: Commands,
    quality: Res<PlantQuality>,
    species: Res<Assets<Species>>,
    running: Query<(), With<CoverTask>>,
    changed: Query<Entity, (Changed<CoverPatch>, With<CoverBuilt>)>,
    waiting: Query<(Entity, &CoverPatch), (Without<CoverBuilt>, Without<CoverTask>)>,
) {
    for entity in &changed {
        commands.entity(entity).remove::<CoverBuilt>();
    }
    let mut free = quality.max_cover_in_flight.saturating_sub(running.iter().count());
    for (entity, patch) in &waiting {
        if free == 0 {
            return;
        }
        let Some(s) = species.get(&patch.species) else { continue };
        let (s, tufts, lod) = (s.clone(), patch.tufts.clone(), patch.lod);
        let task = AsyncComputeTaskPool::get().spawn(async move { merge_cover(&s, &tufts, lod) });
        commands.entity(entity).insert(CoverTask(task));
        free = free.saturating_sub(1);
    }
}

fn finish_cover(
    mut commands: Commands,
    species: Res<Assets<Species>>,
    mut library: ResMut<PlantLibrary>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    assets: Res<AssetServer>,
    mut tasks: Query<(Entity, &CoverPatch, &mut CoverTask, Option<&PlantMaterials>)>,
) {
    for (entity, patch, mut task, own) in &mut tasks {
        let Some(data) = check_ready(&mut task.0) else { continue };
        let mut e = commands.entity(entity);
        e.remove::<CoverTask>().insert(CoverBuilt);
        let Some(s) = species.get(&patch.species) else { continue };
        if data.is_empty() {
            e.remove::<Mesh3d>();
            continue;
        }
        let mats = library.materials_for(patch.species.id(), s, &mut materials, &mut images, &assets).overridden(own);
        e.insert((Mesh3d(meshes.add(data.into_mesh())), MeshMaterial3d(mats.foliage)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hysteresis_holds_near_a_threshold() {
        let q = PlantQuality::MEDIUM;
        assert_eq!(tier_for(q.hero_px * 0.95, PlantLod::Hero, &q), PlantLod::Hero);
        assert_eq!(tier_for(q.hero_px * 0.95, PlantLod::Near, &q), PlantLod::Near);
        assert_eq!(tier_for(q.hero_px * 0.5, PlantLod::Hero, &q), PlantLod::Near);
        assert_eq!(tier_for(1.0, PlantLod::Hero, &q), PlantLod::Far);
    }

    /// The bundled cover over the grove example's 60 m square (36 patches
    /// of 10 m, world seed 7), per tier: what a field of cover costs.
    /// `-- --nocapture` prints the counts.
    #[test]
    fn grove_cover_budget() {
        use crate::scatter::{GroundHit, scatter, species_seed};
        let flat = |p: Vec2| Some(GroundHit { pos: Vec3::new(p.x, 0.0, p.y), normal: Vec3::Y, material: None });
        let mut total = [0usize; 3];
        for name in ["grass", "moss"] {
            let path = format!("{}/assets/species/{name}.plant.ron", env!("CARGO_MANIFEST_DIR"));
            let s: Species = ron::de::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
            let mut tris = [0usize; 3];
            let mut verts = 0usize;
            let mut tufts = 0usize;
            let t = std::time::Instant::now();
            for gx in -3i8..3 {
                for gz in -3i8..3 {
                    let min = Vec2::new(f32::from(gx), f32::from(gz)) * 10.0;
                    let placed = scatter(&s.placement, species_seed(7, &s), min, min + 10.0, flat);
                    tufts += placed.len();
                    for lod in PlantLod::ALL {
                        let m = merge_cover(&s, &placed, lod);
                        assert!(m.indices.iter().all(|&i| i < m.vertex_count()), "{name}: index out of range");
                        tris[lod.index()] += m.indices.len() / 3;
                        if lod == PlantLod::Hero {
                            verts += m.positions.len();
                        }
                    }
                }
            }
            eprintln!(
                "{name}: {tufts} tufts, hero {} tris ({verts} verts), near {}, far {} ({:?} for all tiers)",
                tris[0], tris[1], tris[2], t.elapsed()
            );
            assert!(tris[2] <= tris[1] && tris[1] <= tris[0], "{name}: coarser tiers cost more");
            for (a, b) in total.iter_mut().zip(tris) {
                *a += b;
            }
        }
        eprintln!("cover total: hero {} tris, near {}, far {}", total[0], total[1], total[2]);
    }
}
