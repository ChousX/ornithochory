//! Plants on streamed tarasaur terrain: oaks and spruces on gentle ground,
//! ferns and bushes in patches, grass and moss on the chunks nearest the
//! camera. Dig with the left mouse button (tarasaur's cursor brush) and the
//! plants on the edited chunks re-scatter.
//!
//! The terrain has grass and stone voxels; a species lists the ids it grows
//! on in `placement.materials` (the bundled ones grow on any).
//!
//! `cargo run --release --example terrain --features tarasaur`.
//! WASD to fly, Space / Shift up and down, right mouse drag to look.

use std::any::type_name;

use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use ornithochory::prelude::*;
use tarasaur::field::ops::{AccumulateExt, BlendExt};
use tarasaur::prelude::*;

fn main() {
    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins,
        TarasaurPlugin { save_dir: "saves/ornithochory_terrain".into(), ..default() },
        MaterialFieldPlugin::<Ground>::new(),
        OrnithochoryPlugin,
        TerrainPlantsPlugin::<Ground>::default(),
    ));
    {
        let mut g = app.world_mut().resource_mut::<ChunkGeneratorRegistry>();
        g.register_sdf_distance(terrain_sdf);
        g.register::<u8>(type_name::<MaterialField<Ground>>(), terrain_materials);
    }
    app.add_systems(Startup, setup).add_systems(Update, fly).run();
}

#[derive(Copy, Clone, PartialEq, Eq, Default, Debug)]
enum Ground {
    #[default]
    Grass,
    Stone,
}

impl VoxelMaterial for Ground {
    const COUNT: u8 = 2;
    fn to_id(self) -> u8 {
        match self {
            Self::Grass => 0,
            Self::Stone => 1,
        }
    }
    fn from_id(id: u8) -> Self {
        if id == 1 { Self::Stone } else { Self::Grass }
    }
}

impl AccumulateExt for Ground {
    fn accumulate(self, delta: Self) -> Self {
        delta
    }
}

impl BlendExt for Ground {
    fn blend_towards(self, target: Self, factor: f32) -> Self {
        if factor >= 0.5 { target } else { self }
    }
}

fn height(x: f32, z: f32) -> f32 {
    let mut h = 0.0;
    let (mut amp, mut freq) = (7.0, 0.025);
    for _ in 0..4 {
        h = ((x * freq).sin() * (z * freq * 1.3).cos()).mul_add(amp, h);
        amp *= 0.5;
        freq *= 2.1;
    }
    h
}

/// Every voxel of a chunk in x, y, z order, with its world position.
fn voxels(chunk: IVec3, lod: LOD) -> impl Iterator<Item = Vec3> {
    let n = lod.size();
    let origin = chunk.as_vec3() * CHUNK_SIZE;
    let v = lod.voxel_size();
    (0..n).flat_map(move |z| {
        (0..n).flat_map(move |y| {
            (0..n).map(move |x| origin + UVec3::new(x, y, z).as_vec3() * v)
        })
    })
}

fn terrain_sdf(chunk: IVec3, lod: LOD) -> Vec<f32> {
    voxels(chunk, lod).map(|p| p.y - height(p.x, p.z)).collect()
}

/// Stone on steep ground and high ridges, grass elsewhere.
fn terrain_materials(chunk: IVec3, lod: LOD) -> Vec<u8> {
    voxels(chunk, lod)
        .map(|p| {
            let e = 0.5;
            let slope = Vec2::new(height(p.x + e, p.z) - height(p.x - e, p.z), height(p.x, p.z + e) - height(p.x, p.z - e))
                .length()
                / (2.0 * e);
            let stone = slope > 0.7 || height(p.x, p.z) > 8.0;
            if stone { Ground::Stone.to_id() } else { Ground::Grass.to_id() }
        })
        .collect()
}

#[derive(Component, Default)]
struct Fly {
    yaw: f32,
    pitch: f32,
}

fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    let load = |name: &str| -> Handle<Species> { assets.load(format!("species/{name}.plant.ron")) };
    commands.insert_resource(Flora {
        world_seed: 42,
        entries: vec![
            FloraEntry { species: load("oak"), min_lod: LOD::Medium },
            FloraEntry { species: load("spruce"), min_lod: LOD::Medium },
            FloraEntry { species: load("bush"), min_lod: LOD::High },
            FloraEntry { species: load("fern"), min_lod: LOD::High },
            FloraEntry { species: load("grass"), min_lod: LOD::High },
            FloraEntry { species: load("moss"), min_lod: LOD::High },
        ],
        ..default()
    });

    commands.spawn((
        Camera3d::default(),
        PlantCamera,
        ChunkLoader::default(),
        Fly::default(),
        Transform::from_xyz(0.0, 14.0, 20.0).looking_at(Vec3::new(0.0, 4.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 12_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, 0.8, -0.9, 0.0)),
    ));
}

fn fly(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut motion: MessageReader<MouseMotion>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut cams: Query<(&mut Fly, &mut Transform)>,
) {
    let look: Vec2 = motion.read().map(|m| m.delta).sum();
    let dt = time.delta_secs();
    for (mut f, mut t) in &mut cams {
        if buttons.pressed(MouseButton::Right) {
            f.yaw = look.x.mul_add(-0.003, f.yaw);
            f.pitch = look.y.mul_add(-0.003, f.pitch).clamp(-1.5, 1.5);
            t.rotation = Quat::from_euler(EulerRot::YXZ, f.yaw, f.pitch, 0.0);
        }
        let mut v = Vec3::ZERO;
        for (key, dir) in [
            (KeyCode::KeyW, t.forward().as_vec3()),
            (KeyCode::KeyS, t.back().as_vec3()),
            (KeyCode::KeyA, t.left().as_vec3()),
            (KeyCode::KeyD, t.right().as_vec3()),
            (KeyCode::Space, Vec3::Y),
            (KeyCode::ShiftLeft, Vec3::NEG_Y),
        ] {
            if keys.pressed(key) {
                v += dir;
            }
        }
        t.translation += v.normalize_or_zero() * 12.0 * dt;
    }
}
