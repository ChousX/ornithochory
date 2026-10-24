# Roadmap

Ordered by what each step buys per unit of work, from the cost notes in
`docs/research/procedural-plants.md`. Every step lands with a before/after
frame time on tir (Adreno, the slowest GPU we target) once a bench exists;
step 0 is that bench.

## 0. Bench

A fixed scene (a 100 m square of the bundled species at the default
`Placement`, a fixed camera path) printing: variant generation time per
species and tier, triangles and draws per tier, frame time. Same rules as
ornisculpt's `bench/`: versioned constants, compare only within a version.

## 1. Wind and bend shader

An `ExtendedMaterial<StandardMaterial, PlantWind>` whose vertex shader reads
`ATTRIBUTE_WIND`:

- `x` (height up the plant) scales the main bend,
- `y` (order) and `z` (phase) drive branch sway,
- `w` (flutter) adds leaf and blade flutter,
- `Benders` (a 64-entry uniform) pushes blades and leaves away.

Replaces the CPU lean in `bend.rs`. No per-plant CPU cost.

## 2. Impostor tier

Past `Far`, a camera-facing octahedral impostor per variant: render the
variant from 8x8 hemi-octahedral directions into an albedo+normal atlas at
load, draw one quad per plant, blend the three nearest frames. Needs a
`PlantLod::Impostor` and a fourth `ReadyVariant` tier.

## 3. Instanced cover

Merged patches cost vertex memory proportional to tuft count. Replace them
with one instance buffer per patch (position, rotation, scale, tint, 16 to
32 bytes per tuft) drawn with a custom instanced pipeline, as Bevy's
`custom_shader_instancing` example does. Density falls with distance in the
shader; the far edge fades into the ground material.

## 4. More growers

(Weber-Penn style recursion landed as `grow::Recursive`; the spruce uses it.)

- Self-organizing trees (Palubicki 2009) on the colonization grid: bud fate
  and resource allocation, so neighbours and obstacles shape the crown.
- Colonization against obstacles: drop attraction points inside the
  terrain SDF so trees by a cliff grow away from it.

## 5. Terrain-aware cover

- Moss as a tarasaur material blend where tufts would be too many: paint a
  moss id into `MaterialField` by the same density rule.
- Cover on walls and overhangs: sample the chunk surface instead of
  casting down.

## 6. Seeds (the name)

Fruit as ornisculpt equip items; a mob that eats one drops `SeedDropped`
some time later, which spawns a young `Plant` there. Growth stages as a
`Plant::age` that scales and re-tiers the plant.
