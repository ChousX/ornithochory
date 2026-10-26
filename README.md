# Ornithochory

Generic procedural plants for [Bevy](https://bevyengine.org) 0.20: trees,
shrubs, ferns, grass and moss from one pipeline, placed as a pure function of
the world seed so nothing needs saving. Built to sit beside
[tarasaur](https://github.com/ChousX/tarasaur) terrain and
[ornisculpt](https://github.com/ChousX/ornisculpt) creatures, and to run on
low-end hardware.

(*Ornithochory*: seed dispersal by birds.)

## How it works

```text
*.plant.ron ──> grower ──> PlantSkeleton ──> meshes per tier ──> Plant / CoverPatch
  Species       LSystem      nodes: pos,       stems (tubes)       variants cached,
                Colonize     radius, parent,   leaves (geometry)   tier by screen size
                Recursive    order + organs    tufts (cover)
                (yours)
```

- **Generic.** Every grower writes the same `PlantSkeleton`. Pipe-model
  radii, branch orders, leaves, meshing, wind weights and LOD read only
  that, so a new way of growing (self-organizing trees, a hand-made
  skeleton) is one function.
- **Two forms.** `Structure` for anything with stems, `Cover` for grass,
  moss and lichen, which only matter as a density on a surface. Both use
  the same `Placement` rules.
- **Variants, not individuals.** A species generates a handful of variants
  (off-thread, a few at a time); each plant shows one, with its own scale,
  rotation and tint. Plants sharing a variant share a mesh and material, so
  Bevy batches them into one GPU-driven draw.
- **Three tiers** (`Hero`, `Near`, `Far`) picked by on-screen size with
  hysteresis. Far tiers cut stem sides, drop thin twigs, and keep fewer,
  larger leaves so the crown keeps its area.
- **No alpha test.** Leaves and blades are geometry: on tiled GPUs
  (Steam Deck, Adreno) overdraw from `discard` costs more than vertices.
- **Plain Bevy PBR.** Plants are ordinary `Mesh3d` + `StandardMaterial`
  entities, so Bevy's lights, shadows, ambient and environment light, fog,
  SSAO, bloom, forward or deferred shading and frustum culling all apply
  with no extra setup.
- **Wind-ready.** Every vertex carries `ATTRIBUTE_WIND` (height up the
  plant, branch order, phase, flutter) for a vertex-shader wind with no CPU
  cost per plant.

## Species

```ron
(
    name: "oak",
    variants: 6,
    form: Structure((
        growth: Colonize((
            envelope: Ellipsoid(center: (0.0, 6.0, 0.0), radii: (3.6, 2.8, 3.6)),
            points: 1200, influence: 1.8, kill: 0.45, step: 0.3,
        )),
        leaves: Some((length: 0.16, width: 0.09, tip_depth: 2, per_node: 3)),
    )),
    placement: (spacing: 7.0, probability: 0.45, slope: (0.0, 25.0)),
)
```

Bundled in `assets/species/`: `oak` (space colonization), `spruce`
(Weber-Penn style recursion: a leader with whorls of branches), `fern` and
`bush` (parametric L-systems, see `src/grow/lsystem.rs` for the symbols),
`grass` and `moss` (cover).

## Use

```rust
use bevy::prelude::*;
use ornithochory::prelude::*;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, OrnithochoryPlugin))
        .add_systems(Startup, |mut commands: Commands, assets: Res<AssetServer>| {
            commands.spawn((Camera3d::default(), Transform::from_xyz(0.0, 4.0, 14.0)));
            commands.spawn((
                Plant { species: assets.load("species/oak.plant.ron"), seed: 1 },
                Transform::default(),
            ));
        })
        .run();
}
```

Tiers are picked for every active 3D camera. Mark cameras with
`PlantCamera` to pick only for those (say, the player's view and not a
minimap).

### Materials

Each species builds two `StandardMaterial`s: bark from `bark.color`,
`bark.roughness` and, optionally, `bark.texture`, `bark.normal_map` and
`bark.uv_scale` (asset paths; stems carry tangents, U runs once around a
stem); and foliage from vertex colours, with diffuse transmission so leaves
glow when lit from behind. To use your own, put `PlantMaterials` on a
`Plant` or `CoverPatch`:

```rust
commands.spawn((
    Plant { species: assets.load("species/oak.plant.ron"), seed: 1 },
    PlantMaterials { stems: Some(my_bark), foliage: None },
    Transform::default(),
));
```

Changing or removing it later swaps the materials on the plant as shown.
Foliage colour comes from vertex colours, which `StandardMaterial`
multiplies in, so a white `base_color` keeps the species' colours. Foliage
meshes have no tangents, so only bark takes a normal map.

### With tarasaur (`--features tarasaur`)

```rust
app.add_plugins((TarasaurPlugin::default(), OrnithochoryPlugin, TerrainPlantsPlugin::<MyMaterial>::default()))
    .insert_resource(Flora {
        world_seed: 42,
        entries: vec![
            FloraEntry { species: assets.load("species/oak.plant.ron"), min_lod: LOD::Medium },
            FloraEntry { species: assets.load("species/grass.plant.ron"), min_lod: LOD::High },
        ],
        ..default()
    });
```

Chunks are scattered as their fields load, re-scattered when edited (after
the SDF settles) or when their LOD changes, and their plants unload with
them. `placement.materials` lists the voxel material ids a species grows on.
Plants grow only on terrain that is shown (`Flora::clipped`, on by
default): ground hidden by a `VisibilityField` stays bare, and a chunk
re-scatters when what it shows changes.

### With ornisculpt (`--features ornisculpt`)

Add `OrnisculptPlantsPlugin`. Every mob becomes a `FoliageBender` as wide
as its body (small plants lean out of its way), and a camera with
ornisculpt's `TierCamera` also picks plant tiers. Any other entity can push
plants too: give it a `FoliageBender`.

### Plant designs (`--features genetics`)

A whole species can be written as DNA with the
[genetics](https://github.com/ChousX/genetics) crate. A
`*.plant-design.ron` names a species and the traits that may vary, each a
`PlantLever` and the range it varies within:

```ron
(
    species: "species/grass.plant.ron",
    traits: [
        Lever(lever: Height, range: (0.18, 0.45)),
        Lever(lever: BladeCount, range: (10.0, 22.0)),
    ],
)
```

`OrnithochoryPlugin` loads these as `PlantDesign` assets. `founder()` is
the design's first genome, the species as designed; mutate it with
`genetics`, and `design::express` gives back the species a genome grows.
Everything else in the species is a fixed gene that mutation never
touches. A genome is a strain, not a single plant: every plant grown from
it shares its cached variants. `Complexity::of` counts what one variant
costs (stem nodes and organs), for budgets.

## Examples

```sh
cargo run --release --example grove                       # every species on a plane
cargo run --release --example terrain --features tarasaur # on streamed, editable terrain
```

The grove also runs headless for judging changes:
`GROVE_SHOTS=<dir> cargo run --release --example grove` saves a fixed set of
1280x720 views (each species close up, every tier side by side, far away)
and `stats.txt` (generation time, uncapped frame time), then exits.
`GROVE_STACK=deferred,ssao,fog,bloom` (any subset) shows the plants under
those Bevy renderer features.

### Editing while it runs

Species reload by themselves: save a `*.plant.ron` while an example runs
and its plants regrow (the examples build Bevy with `file_watcher`).

Code can too, through Bevy's `hotpatching` (Dioxus' subsecond, driven by
`dx`, which the devenv shell provides):

```sh
grove-live   # dx serve --hot-patch on live/, assets from the repository root
```

Save a file under `src/` or `examples/grove.rs` and dx patches it into the
running grove. With the `hotpatching` feature, ornithochory then drops its
cached variants and regrows every plant and patch, so an edit to a grower,
a mesher or a material shows without a restart. `live/` is the grove
example as a crate of its own: dx patches the crates the running crate
depends on, but not a library beside an example in the same package.

A patch can't change a struct's fields (a species, a component, a
resource), a system's parameters, which systems run, or what `Startup` did:
for those, rebuild with `r` in dx. Your own app gets the same with
`ornithochory/hotpatching` and its own `dx serve --hot-patch`.

## Status

Early. Working: species assets with hot reload, L-system, space
colonization and recursive (Weber-Penn) growers, stems, leaves with
clumps and crown-shaped normals, grass and moss tufts, three tiers,
cached variants generated off-thread, cover patches, deterministic
placement, tarasaur scattering, ornisculpt benders, plant designs as DNA.

Next, in order of what they buy (see `docs/design/roadmap.md`):

1. Wind and bend vertex shader reading `ATTRIBUTE_WIND` and `Benders`.
2. Impostors for a fourth tier past `Far`.
3. GPU-instanced cover instead of merged patches.
4. A self-organizing grower.

The research behind these choices, with sources:
[`docs/research/procedural-plants.md`](docs/research/procedural-plants.md).

## License

MIT or Apache-2.0, at your option.
