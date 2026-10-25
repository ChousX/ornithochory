//! Every bundled species on a flat plane: a row of each stemmed species and
//! patches of grass and moss scattered around them. A ball rolls through
//! the shrubs to show interaction.
//!
//! `cargo run --release --example grove`. Arrow keys orbit, W/S zoom.
//!
//! Screenshot mode, for judging the look without a window:
//! `GROVE_SHOTS=<dir> cargo run --release --example grove` runs headless,
//! waits until every plant has generated, shoots a fixed set of views (see
//! `VIEWS`) into `<dir>/<view>.png`, writes `<dir>/stats.txt` (time to generate
//! everything, mean frame time without vsync) and exits. It also plants a
//! row of each species forced to every tier, for comparing LOD.
//!
//! Plants are ordinary Bevy PBR meshes, so every renderer feature applies to
//! them. `GROVE_STACK` turns some on, comma-separated: `deferred` (deferred
//! shading), `ssao`, `fog`, `bloom`; e.g. `GROVE_STACK=deferred,ssao,fog`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::app::ScheduleRunnerPlugin;
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::core_pipeline::prepass::{DeferredPrepass, DepthPrepass};
use bevy::pbr::{DefaultOpaqueRendererMethod, DistanceFog, FogFalloff, ScreenSpaceAmbientOcclusion};
use bevy::post_process::bloom::Bloom;
use bevy::render::view::Msaa;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use ornithochory::num::f32_from_u32;
use ornithochory::plant::CoverBuilt;
use ornithochory::prelude::*;

const SPECIES: [&str; 4] = ["oak", "spruce", "bush", "fern"];

/// Shot name, orbit target, yaw, pitch, distance.
const VIEWS: [(&str, Vec3, f32, f32, f32); 9] = [
    ("overview", Vec3::new(0.0, 2.0, 0.0), 0.6, 0.35, 28.0),
    ("oak", Vec3::new(0.0, 4.5, 9.0), 0.3, 0.15, 13.0),
    ("spruce", Vec3::new(0.0, 4.5, 3.0), 2.75, 0.12, 13.0),
    ("bush", Vec3::new(0.0, 0.7, -3.0), 0.5, 0.35, 4.5),
    ("fern", Vec3::new(0.0, 0.3, -9.0), 0.2, 0.55, 2.6),
    ("ground", Vec3::new(4.0, 0.1, -14.0), 0.9, 0.18, 2.2),
    // From -Z, so Hero (largest x) is on the left.
    ("tiers_trees", Vec3::new(0.0, 4.0, -20.0), std::f32::consts::PI, 0.1, 30.0),
    ("tiers_small", Vec3::new(0.0, 0.6, -28.0), std::f32::consts::PI, 0.25, 9.0),
    ("far", Vec3::new(0.0, 2.0, 0.0), 0.6, 0.2, 75.0),
];

fn main() {
    let shots = std::env::var("GROVE_SHOTS").ok();
    let mut app = App::new();
    if shots.is_some() {
        // No window: the camera renders into an image (`shot_target`), and
        // the loop runs uncapped so the frame time means something.
        app.add_plugins((
            DefaultPlugins
                .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
                .disable::<WinitPlugin>(),
            // With a window plugin, DefaultPlugins leaves the runner to winit.
            ScheduleRunnerPlugin::run_loop(Duration::ZERO),
        ));
    } else {
        app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: "grove".into(), ..default() }),
            ..default()
        }));
    }
    let stack = Stack::from_env();
    if stack.has("deferred") {
        app.insert_resource(DefaultOpaqueRendererMethod::deferred());
    }
    app.add_plugins(OrnithochoryPlugin)
        .insert_resource(ClearColor(SKY))
        // Sky fill: without it, everything the sun misses is near black.
        .insert_resource(GlobalAmbientLight { color: SKY, brightness: 900.0, ..default() })
        .insert_resource(stack)
        .add_systems(Startup, (setup, render_stack.after(setup)))
        .add_systems(Update, (orbit, roll_ball, scatter_cover));
    if let Some(dir) = shots {
        app.insert_resource(Shots {
            dir,
            started: Instant::now(),
            stage: Stage::Generating,
            frames: 0,
            frame_secs: 0.0,
            target: Handle::default(),
        })
        .add_systems(Startup, (tier_rows, shot_target.after(setup)))
            .add_systems(Update, shoot.after(orbit));
    }
    app.run();
}

const SKY: Color = Color::srgb(0.62, 0.74, 0.86);

/// Bevy renderer features to show the plants under (`GROVE_STACK`).
#[derive(Resource, Clone, Default)]
struct Stack(String);

impl Stack {
    fn from_env() -> Self {
        Self(std::env::var("GROVE_STACK").unwrap_or_default())
    }

    fn has(&self, feature: &str) -> bool {
        self.0.split(',').any(|s| s.trim() == feature)
    }
}

/// Puts the `GROVE_STACK` features on the camera.
fn render_stack(mut commands: Commands, stack: Res<Stack>, cams: Query<Entity, With<Orbit>>) {
    for cam in &cams {
        let mut e = commands.entity(cam);
        if stack.has("deferred") || stack.has("ssao") {
            // Both need the prepass textures, which don't support MSAA.
            e.insert((Msaa::Off, DepthPrepass));
        }
        if stack.has("deferred") {
            e.insert(DeferredPrepass);
        }
        if stack.has("ssao") {
            e.insert(ScreenSpaceAmbientOcclusion::default());
        }
        if stack.has("fog") {
            e.insert(DistanceFog { color: SKY, falloff: FogFalloff::Linear { start: 25.0, end: 140.0 }, ..default() });
        }
        if stack.has("bloom") {
            e.insert(Bloom::NATURAL);
        }
    }
}

#[derive(Component)]
struct Orbit {
    target: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
}

#[derive(Component)]
struct Ball;

/// Cover species to scatter once they load.
#[derive(Resource)]
struct CoverSpecies(Vec<Handle<Species>>);

fn setup(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        // No `PlantCamera`: plants pick their tiers for any 3D camera.
        Camera3d::default(),
        Orbit { target: Vec3::Y * 2.0, yaw: 0.6, pitch: 0.35, distance: 28.0 },
        Transform::default(),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 12_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, 0.8, -0.9, 0.0)),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(80.0, 80.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.30, 0.27, 0.18))),
    ));

    // A row per species, several seeds each, so the variants show.
    for (row, name) in SPECIES.iter().enumerate() {
        let species: Handle<Species> = assets.load(format!("species/{name}.plant.ron"));
        let spacing = if row < 2 { 7.0 } else { 2.5 };
        let z = f32::from(u8::try_from(row).unwrap_or(0)).mul_add(-6.0, 9.0);
        for i in 0u8..5 {
            let x = (f32::from(i) - 2.0) * spacing;
            commands.spawn((
                Plant { species: species.clone(), seed: u64::from(i) },
                Transform::from_xyz(x, 0.0, z).with_rotation(Quat::from_rotation_y(f32::from(i))),
            ));
        }
    }

    commands.insert_resource(CoverSpecies(["grass", "moss"].map(|n| assets.load(format!("species/{n}.plant.ron"))).to_vec()));

    commands.spawn((
        Ball,
        FoliageBender { radius: 0.6 },
        Mesh3d(meshes.add(Sphere::new(0.5))),
        MeshMaterial3d(materials.add(Color::srgb(0.8, 0.2, 0.1))),
        Transform::from_xyz(0.0, 0.5, 0.0),
    ));
}

/// Grass and moss by their own placement rules, one patch per 10 m square
/// as a terrain chunk would have, once their assets have loaded.
fn scatter_cover(mut commands: Commands, cover: Option<Res<CoverSpecies>>, species: Res<Assets<Species>>) {
    let Some(cover) = cover else { return };
    let flat = |p: Vec2| Some(GroundHit { pos: Vec3::new(p.x, 0.0, p.y), normal: Vec3::Y, material: None });
    let Some(loaded) = cover.0.iter().map(|h| species.get(h).map(|s| (h, s))).collect::<Option<Vec<_>>>() else {
        return;
    };
    for (handle, s) in loaded {
        for gx in -3i8..3 {
            for gz in -3i8..3 {
                let min = Vec2::new(f32::from(gx), f32::from(gz)) * 10.0;
                let tufts: Arc<[Placed]> = scatter(&s.placement, species_seed(7, s), min, min + 10.0, flat).into();
                commands.spawn(CoverPatch { species: handle.clone(), tufts, lod: PlantLod::Hero });
            }
        }
    }
    commands.remove_resource::<CoverSpecies>();
}

fn orbit(time: Res<Time>, keys: Res<ButtonInput<KeyCode>>, mut cams: Query<(&mut Orbit, &mut Transform)>) {
    let dt = time.delta_secs();
    for (mut o, mut t) in &mut cams {
        let axis = |neg: KeyCode, pos: KeyCode| f32::from(u8::from(keys.pressed(pos))) - f32::from(u8::from(keys.pressed(neg)));
        o.yaw = axis(KeyCode::ArrowLeft, KeyCode::ArrowRight).mul_add(dt, o.yaw);
        o.pitch = axis(KeyCode::ArrowDown, KeyCode::ArrowUp).mul_add(dt, o.pitch).clamp(0.05, 1.4);
        o.distance = (axis(KeyCode::KeyW, KeyCode::KeyS) * 15.0).mul_add(dt, o.distance).clamp(2.0, 120.0);
        let dir = Quat::from_euler(EulerRot::YXZ, o.yaw, -o.pitch, 0.0) * Vec3::Z;
        *t = Transform::from_translation(dir * o.distance + o.target).looking_at(o.target, Vec3::Y);
    }
}

/// Rolls the ball back and forth along the bush row.
fn roll_ball(time: Res<Time>, mut balls: Query<&mut Transform, With<Ball>>) {
    for mut t in &mut balls {
        t.translation = Vec3::new((time.elapsed_secs() * 0.4).sin() * 6.0, 0.5, -3.0);
    }
}

// ============================================================================
// Screenshot mode
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Waiting for every plant and patch to generate.
    Generating,
    /// Measuring frame time at the overview.
    Timing,
    /// Showing view `n`; shoots once it has settled.
    View(usize),
    /// Waiting for the last screenshot to land.
    Saving,
}

#[derive(Resource)]
struct Shots {
    dir: String,
    started: Instant,
    stage: Stage,
    /// Frames spent in the current stage.
    frames: u32,
    frame_secs: f32,
    /// What the camera renders into: a fixed size whatever the window
    /// manager does to the window.
    target: Handle<Image>,
}

const SHOT_SIZE: (u32, u32) = (1280, 720);

/// Points the camera at a fixed-size image, so every shot frames the same.
fn shot_target(
    mut commands: Commands,
    mut shots: ResMut<Shots>,
    mut images: ResMut<Assets<Image>>,
    cams: Query<Entity, With<Orbit>>,
) {
    let mut image = Image::new_target_texture(SHOT_SIZE.0, SHOT_SIZE.1, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    shots.target = images.add(image);
    for cam in &cams {
        commands
            .entity(cam)
            .insert(RenderTarget::Image(ImageRenderTarget { handle: shots.target.clone(), scale_factor: 1.0 }));
    }
}

/// Frames a view is shown before it is shot: tiers settle and new meshes upload.
const SETTLE_FRAMES: u32 = 20;
/// Frames the overview's frame time is averaged over.
const TIMING_FRAMES: u32 = 240;
/// Frames to wait at least before trusting that nothing is left to
/// generate: plants and patches spawn in the first frames.
const FIRST_FRAMES: u32 = 10;
/// Frames to wait for the last screenshot to be written before exiting
/// anyway.
const SAVE_TIMEOUT_FRAMES: u32 = 600;

/// Every stemmed species forced to each tier, Hero to Far, left to right.
fn tier_rows(mut commands: Commands, assets: Res<AssetServer>) {
    // Seen from -Z (see `VIEWS`): each row runs Hero to Far from +x to -x.
    let rows: [(&str, f32, f32, f32); 4] =
        [("oak", 20.0, -20.0, 8.0), ("spruce", -4.0, -20.0, 8.0), ("bush", 7.0, -28.0, 2.5), ("fern", -1.5, -28.0, 2.0)];
    for (name, x0, z, step) in rows {
        let species: Handle<Species> = assets.load(format!("species/{name}.plant.ron"));
        for (k, tier) in PlantLod::ALL.into_iter().enumerate() {
            let x = f32::from(u8::try_from(k).unwrap_or(0)).mul_add(-step, x0);
            commands.spawn((
                Plant { species: species.clone(), seed: 3 },
                ForcePlantTier(tier),
                Transform::from_xyz(x, 0.0, z),
            ));
        }
    }
}

fn shoot(
    mut commands: Commands,
    time: Res<Time>,
    mut shots: ResMut<Shots>,
    cover: Option<Res<CoverSpecies>>,
    waiting_plants: Query<(), (With<Plant>, Without<PlantParts>)>,
    waiting_patches: Query<(), (With<CoverPatch>, Without<CoverBuilt>)>,
    mut cams: Query<&mut Orbit>,
    mut exit: MessageWriter<AppExit>,
) {
    shots.frames = shots.frames.saturating_add(1);
    let show = |cams: &mut Query<&mut Orbit>, i: usize| {
        if let (Some(&(_, target, yaw, pitch, distance)), Ok(mut o)) = (VIEWS.get(i), cams.single_mut()) {
            *o = Orbit { target, yaw, pitch, distance };
        }
    };
    match shots.stage {
        Stage::Generating => {
            if cover.is_none() && waiting_plants.is_empty() && waiting_patches.is_empty() && shots.frames > FIRST_FRAMES {
                let secs = shots.started.elapsed().as_secs_f32();
                info!("grove: everything generated in {secs:.2} s");
                let _ = std::fs::create_dir_all(&shots.dir);
                let _ = std::fs::write(format!("{}/stats.txt", shots.dir), format!("generated_secs {secs:.3}\n"));
                show(&mut cams, 0);
                shots.stage = Stage::Timing;
                shots.frames = 0;
                shots.frame_secs = 0.0;
            }
        }
        Stage::Timing => {
            // Skip the first frames after the switch: tier changes upload meshes.
            if shots.frames > SETTLE_FRAMES {
                shots.frame_secs += time.delta_secs();
            }
            if shots.frames >= TIMING_FRAMES.saturating_add(SETTLE_FRAMES) {
                let ms = shots.frame_secs / f32_from_u32(TIMING_FRAMES) * 1000.0;
                info!("grove: overview mean frame {ms:.2} ms");
                let path = format!("{}/stats.txt", shots.dir);
                let old = std::fs::read_to_string(&path).unwrap_or_default();
                let _ = std::fs::write(&path, format!("{old}overview_frame_ms {ms:.3}\n"));
                shots.stage = Stage::View(0);
                shots.frames = 0;
            }
        }
        Stage::View(i) => {
            if shots.frames == 1 {
                show(&mut cams, i);
            }
            if shots.frames >= SETTLE_FRAMES {
                if let Some((name, ..)) = VIEWS.get(i) {
                    let path = format!("{}/{name}.png", shots.dir);
                    info!("grove: shot {path}");
                    commands.spawn(Screenshot::image(shots.target.clone())).observe(save_to_disk(path));
                }
                shots.stage = if i.saturating_add(1) < VIEWS.len() { Stage::View(i.saturating_add(1)) } else { Stage::Saving };
                shots.frames = 0;
            }
        }
        Stage::Saving => {
            let last = VIEWS.last().map(|v| format!("{}/{}.png", shots.dir, v.0));
            if last.is_some_and(|p| std::path::Path::new(&p).exists()) || shots.frames > SAVE_TIMEOUT_FRAMES {
                exit.write(AppExit::Success);
            }
        }
    }
}
