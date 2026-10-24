# Procedural plants: a survey for Ornithochory

Written 2026-10-24. What is well documented for generating trees, shrubs,
herbs, grass, moss and lichen, and for drawing a lot of them cheaply. Each
section ends with **Cost notes**: what the technique costs and how to keep that
down on the hardware these crates aim at: a Steam Deck (tarasaur's target), an
Adreno-class tiled integrated GPU (tir, where ornisculpt is benchmarked) and
WebGPU in a browser.

Where a claim comes from a source I could only read the abstract or a summary
of, it says so.

## 1. Two families, one intermediate form

Almost every documented method falls into one of two families:

1. **Structure**: a plant is a branching graph of axes (trunk, branches,
   stems, petioles) with organs (leaves, needles, flowers, fruit) on the ends.
   Trees, shrubs, ferns, herbs, vines, roots, and even a single grass blade (one
   axis, no branches).
2. **Cover**: a plant is a density over a host surface, and the individual is
   too small or too numerous to matter: grass fields, moss, lichen, algae,
   leaf litter.

Generators in family 1 differ in how they decide the graph, but they all end
in the same thing: nodes with a position, radius, parent and branch order.
That shared form is what makes plant generation generic. Any grower writes it,
and one mesher, one LOD scheme and one wind model read it (section 7).

## 2. Structure generators

### 2.1 L-systems (Lindenmayer; Prusinkiewicz & Lindenmayer, *The Algorithmic Beauty of Plants*, 1990)

A string-rewriting grammar: an axiom and productions (`A -> F[+A][-A]FA`)
applied in parallel for `n` generations, then read by a 3D turtle (`F` forward,
`+ - & ^ \ /` rotate, `[ ]` push and pop). The book (free at
algorithmicbotany.org) covers the extensions that make it practical:

- **Bracketed**: `[ ]` branches.
- **Stochastic**: several right-hand sides with probabilities; seeded, it is
  deterministic.
- **Parametric**: modules carry numbers (`F(l, w)`) and productions have
  conditions (`A(t) : t > 2 -> ...`), so length and width taper by age.
- **Context-sensitive**: a production looks at neighbours; this is how
  signals (hormones, water) travel along the plant.
- **Open L-systems** (Měch & Prusinkiewicz 1996): the plant queries its
  environment (light, collision, terrain) between derivation steps.

Good at: ferns, herbs, flowers, inflorescences, phyllotaxis, palms, stylised
trees, anything with a botanical rule. Weak at: realistic large crowns, which
look regular unless the environment pushes back.

**Cost notes.** String growth is exponential in generations. Cap the module
count, not only the depth, and stop expanding a branch once its turtle step is
under a pixel at the hero LOD. Parse the grammar once per species and keep the
symbols as `u8` ids, not strings. Derivation is CPU-cheap (microseconds to a
few milliseconds for a tree) next to meshing, so cache the *skeleton* per
`(species, seed)` and mesh per LOD.

### 2.2 Recursive parametric trees (Weber & Penn, "Creation and Rendering of Realistic Trees", SIGGRAPH 1995)

No grammar: a tree is up to four recursion levels, each with about a dozen
numbers (length, taper, curve, split count, down angle, rotation, branch
count, leaf count). Shape envelopes (conical, spherical, flame, tend-flame,
...) set branch length along the trunk. This is the model behind Blender's
Sapling add-on and the parameter style SpeedTree and most game tree tools use.

Good at: recognisable species from a small, artist-friendly table. Weak at:
reacting to neighbours or obstacles.

**Cost notes.** Linear in node count and fully predictable, so it is the
easiest to budget: the parameters directly say how many branches and leaves
come out. Its level structure maps one-to-one onto LOD: drop level 3 at
`Near`, levels 2 and 3 at `Far`.

### 2.3 Space colonization (Runions, Lane & Prusinkiewicz, "Modeling Trees with a Space Colonization Algorithm", 2007)

Scatter attraction points in the crown volume. Each iteration, every point
pulls on the nearest tree node within an influence radius; each pulled node
grows a new node towards the mean direction of its points; points within a
kill distance of a node are removed. Repeat until no points remain or nothing
grows. Then radii come from the pipe model (section 2.6).

The crown shape is just the point volume, so any SDF or mesh works as an
envelope: a sphere for an oak, a cone for a spruce, a hedge, a topiary, a
tree pressed against a cliff (remove points inside the terrain). Several
trees sharing one point cloud compete for space, which gives forest-edge
asymmetry for free. The same algorithm grows roots (points below ground) and
leaf venation (2D, the paper's origin).

Good at: natural, irregular crowns with real competition; roots; vines over a
surface. Weak at: species-specific branching rules (all crowns look "ash-like"
unless steered).

**Cost notes.** The naive version is O(points x nodes) per iteration. Use a
uniform grid over nodes (cell = influence radius) so each point checks 27
cells; with that, a 2,000-point crown grows in milliseconds. Grow coarse
(large step, fewer points) and subdivide the skeleton afterwards for hero
meshes rather than growing at hero resolution. It is iterative and data
parallel, so it can move to a compute shader later.

### 2.4 Self-organizing trees (Palubicki et al., "Self-organizing tree models for image synthesis", SIGGRAPH 2009)

Space colonization or shadow propagation decides how much light/space each
bud gets; a resource is passed basipetally (tip to base) and allocated back
acropetally with an apical-dominance bias (the Borchert-Honda model); buds
with too little resource stay dormant or are shed. Interactive steering:
brush, sketch, prune, bend.

Good at: the most convincing temperate trees in the literature for the
effort; responds to neighbours and pruning. Builds on 2.3, so the same grid
and point cloud are reused.

**Cost notes.** A few times 2.3's cost. Shadow propagation uses a coarse voxel
grid of light (the paper's alternative to points); keep it at a few cm to dm
per cell. Fine for generation-time; not for every frame.

### 2.5 Simulation-scale and data-driven

- **Synthetic silviculture** (Makowski et al., SIGGRAPH 2019): many trees
  growing in an ecosystem simulation, with species-level traits. Useful as a
  reference for forest composition, not needed per tree.
- **Inverse procedural modelling** (Stava et al., 2014): fit a parametric tree
  model to an example. Interesting for authoring tools later.
- **Climbing plants** (Hädrich et al., SIGGRAPH 2017): vines as connected
  particles that attach to and grow over geometry. Space colonization with
  points sampled on a surface gets most of the look for less.
- **Real-time GPU tree generation** (Kuth et al., HPG 2025): trees generated
  and drawn every frame entirely on the GPU with work graphs and mesh nodes.
  Their scene has 1,200 trees of 20 types from about 51 KiB of generation
  data, where the equivalent static meshes would be about 34.8 GiB, at a
  7.74 ms median frame on a Radeon RX 7900 XTX. It is the clearest evidence
  that "generate, don't store" pays, but work graphs and mesh shaders are not
  available through wgpu on our targets, so treat it as a later direction.
  (From the abstract and publisher page.)

### 2.6 Radii: the pipe model

Leonardo's rule and the pipe model (Shinozaki 1964): a parent's cross-section
is the sum of its children's, `r_parent^e = sum r_child^e` with `e` about 2
(2 to 3 in practice). Compute it in one pass from the tips down after any
grower. It gives every grower plausible tapering for free and is what lets a
generic mesher work.

### 2.7 Organs

- **Leaves**: phyllotaxis by the golden angle (137.5 degrees) around the axis;
  a leaf is a card (two triangles, alpha-tested texture) or a small bent mesh
  (6 to 20 triangles, no alpha).
- **Leaf clusters**: at a distance, replace leaves on a twig with one card
  showing a pre-rendered twig. This is the standard SpeedTree-style LOD step.
- **Needles**: never individually. Cards along twigs (bottle-brush), or a
  shell/fur look on the twig.
- **Flowers and fruit**: instanced small meshes placed like leaves, chosen by
  a season parameter.

## 3. Cover generators

### 3.1 Grass

- **Geometry blades**: one blade = 3 to 15 vertices, curved (a quadratic
  Bezier in Jahrmann & Wimmer, "Responsive Real-Time Grass Rendering for
  General 3D Scenes", I3D 2017), generated per frame or per tile on the GPU.
  They work on arbitrary 3D surfaces, not only heightmaps, and cull blades
  by orientation, view frustum, distance and occlusion before drawing. A
  poster from the same group reports culling up to 75% of blades without a
  visible density change (poster only).
- **Ghost of Tsushima** (Wohllaib, GDC 2021, "Procedural Grass in Ghost of
  Tsushima"): every blade is generated on the GPU from a hash of its tile
  position, which gives its position, height, colour and sway phase, so
  nothing is stored per blade. Blades seen edge-on are widened in view space
  so a field does not thin out at grazing angles. (From the session
  description and secondary write-ups; the talk is GDC Vault members-only.)
- **Cards**: a few crossed, alpha-tested quads per clump. Cheap in vertices,
  expensive in overdraw (below).

### 3.2 Moss

There is little academic work on real-time moss. What production does:

- **As a material**: a moss layer blended into the host surface's shader by
  "up-facing-ness" (normal dot up), ambient occlusion/cavity, wetness and
  noise. Nearly every game moss is this. For tarasaur that means moss is a
  `MaterialField` id or a palette blend, not geometry.
- **Shells** near the camera: a few offset copies of the host surface,
  alpha-tested by a noise of stalk positions (the same technique as
  ornisculpt's fur coat). Reads well up close, cheap far away because shells
  stop being drawn.
- **Tufts**: small instanced domes or clumps scattered by a density rule,
  for cushion moss on rocks and logs.
- **Pixar's "Moss"** (SIGGRAPH talks since *Brave*) is a production system
  for procedural ground cover (grass, debris), not moss rendering. It is a
  good reference for rule-driven cover at scale.
- Offline methods (an L-system-grown Houdini moss; a RenderMan Spanish-moss
  shader in a Clemson thesis) are about look, not runtime.

So moss in Ornithochory is a *cover* species: a density rule (slope, light,
wetness, host material) that drives up to three representations by distance:
tufts and shells near, material blend everywhere.

### 3.3 Lichen

Desbenoit, Galin & Akkouche, "Simulating and modeling lichen growth" (CGF
2004): seeding, then an open diffusion-limited aggregation (DLA) that spreads
over the substrate guided by wetness, light and ambient exposure, then
instantiating lichen shapes from an atlas. The DLA is expensive and
offline-flavoured; the useful part for games is the **exposure-driven density
field**, which is the same field moss uses with different weights. (From the
abstract and lecture slides.)

### 3.4 Ground scatter (placement)

- **Horizon Zero Dawn** (van Muijden, GDC 2017, "GPU-Based Run-Time Procedural
  Placement"): placement is data-driven density maps combined per asset type,
  evaluated on the GPU around the player, deterministic and locally stable.
  The slides cite over 500 asset types, over 100,000 objects in a scene and
  about 250 microseconds of average GPU time.
- **Deterministic jittered grids / hashed Poisson disks**: a point per cell,
  jittered by a hash of `(world seed, cell)`. Placement is a pure function of
  position, so a chunk can be re-scattered at any time and gets the same
  plants, nothing is saved, and neighbouring chunks agree at their borders.
- **Wang tiles / precomputed Poisson tiles**: blue-noise distributions without
  per-point rejection.

## 4. Wind and interaction

- **Hierarchical vertex wind** (Sousa, "Vegetation Procedural Animation and
  Shading in Crysis", GPU Gems 3 ch. 16, 2007): main bending of the whole
  plant, then branch sway and leaf flutter, each driven by per-vertex weights
  baked at generation (branch level, distance along branch, a phase). All in
  the vertex shader, no CPU work per plant.
- **Pivot painting** (Unreal): bake each branch's pivot and axis into the
  mesh (UVs/colours) so the shader rotates branches about real pivots.
- **Skinned wind** (Unreal 5.7 Nanite Foliage): trees are skeletal meshes and
  a procedural wind plugin drives the bones (experimental in 5.7). Better
  silhouettes, costs skinning.
- **Interaction**: a small top-down "displacement" texture or buffer of
  benders (players, mobs) around the camera; grass blades read it and bend
  away. Cost is one tiny texture per frame, independent of plant count.

**Cost notes.** Bake the wind weights into vertex attributes at generation;
then wind costs a few ALU per vertex and nothing on the CPU. Skinning is only
worth it for a handful of hero trees.

## 5. Rendering at scale

### 5.1 LOD ladder

What the shipped engines converge on (SpeedTree, Unreal, Decima):

| Tier | Typical distance | Representation |
|---|---|---|
| Hero | nearest few | full mesh, geometry leaves, full wind |
| Near | tens of metres | reduced rings/segments, leaf clusters, branch wind |
| Far | hundreds of metres | trunk + a few clusters, main bend only |
| Impostor | beyond | billboard / octahedral impostor |
| Cover | anywhere | material blend on the host surface |

Pick the tier by **projected screen size** with hysteresis, as ornisculpt's
`QualityProfile` already does for mobs, not by raw distance; and cross-fade
(dithered alpha) over a few frames to hide pops.

### 5.2 Impostors

An impostor is the plant pre-rendered from many directions into an atlas
(albedo, normal, depth). Octahedral impostors map the view directions over a
hemi-octahedron and blend the three nearest frames, which removes most of the
"cardboard" look of a single billboard. One quad per tree at any distance.
Generate the atlas once per species variant, offscreen, at load.

### 5.3 Unreal 5.7 Nanite Foliage

Experimental in 5.7: Nanite Voxels for distant foliage (many tiny overlapping
elements drawn as a solid mass), Nanite Assemblies (one instance of a part
reused across many trees) and Nanite Skinning for wind. Not reachable from
wgpu, but two of its ideas are: **assemblies** (instance the same twig or
branch mesh many times inside a tree, instead of a unique mesh per tree) and
**volumetric far LOD** (a distant forest as a density, not trees).

### 5.4 Overdraw and alpha test: the main cost on our GPUs

- Alpha-tested cards (`discard` in the fragment shader) disable early-Z for
  the draw on many GPUs and break hidden-surface removal on tiled mobile GPUs
  (Adreno, Mali), so every layer of leaves is shaded. A dense canopy of cards
  can be the single largest frame cost on tir and the Steam Deck.
- Mitigations, roughly in order of value:
  1. Prefer **geometry leaves and blades** (a few more vertices, no discard)
     at Hero/Near on tiled GPUs; vertices are cheap there, overdraw is not.
  2. **Depth prepass** for alpha-tested foliage so the main pass shades each
     pixel once (tarasaur already has a `VoxelDepthPrepass`).
  3. **Trim cards to the texture's silhouette** (an 8-gon instead of a quad)
     to cut transparent area by 30 to 50%.
  4. Alpha-to-coverage with MSAA instead of blending; never alpha-blend
     foliage.
  5. Fewer, larger leaf clusters with distance.

### 5.5 Instancing, culling and draw count

- One mesh per species variant (8 to 16 variants per species by seed is
  plenty) and per-instance data (transform, colour tint, wind phase) is the
  biggest lever: memory and generation scale with variants, not with plants.
- Bevy since 0.16 batches entities that share a mesh and material and
  draws them with GPU-driven multi-draw indirect, with GPU frustum culling.
  That makes "an entity per tree" acceptable into the low tens of thousands;
  the cost that remains is per-entity CPU work (transform propagation,
  extraction, visibility).
- Past that (grass, moss tufts, small rocks), stop using entities: one entity
  per *chunk patch* owning an instance buffer, drawn with a custom instanced
  pipeline, culled on the GPU. Never an entity per blade.
- Cull small things by distance aggressively: a grass blade 40 m away is
  sub-pixel, so density should fall with distance and the fade is hidden by
  the ground material colour matching the grass.

### 5.6 Generation cost

- Pure function of `(species, seed)`: cache skeletons and meshes by a content
  hash (ornisculpt already uses xxh3 keys plus a `GENERATOR_VERSION`), dedupe
  in-flight requests, and generate on `AsyncComputeTaskPool` with a per-frame
  budget, never on the main thread. Ornisculpt's own baseline shows the
  failure mode: 100 to 550 ms frames while generating.
- Generate variants lazily: the first time a species variant is within the
  Near radius, not at startup.
- Meshing budget per tier: rings around a tube 3 to 12, segments per metre by
  screen size; merge thin twigs into leaf clusters below Near.

### 5.7 Placement cost

- Scatter on the CPU per chunk when the chunk loads, off-thread: hash grid,
  density rule, then one ray (or SDF sample) per candidate to find the
  ground. Reject candidates by cheap rules (noise, slope from a coarse
  sample) *before* the ray.
- Re-scatter only the chunks whose terrain changed, and only the plants whose
  footprint overlaps the edit.
- Coarse terrain LODs get coarse plant LODs: at tarasaur's `Low`/`Lowest`
  chunks only impostors and cover material are worth placing.

## 6. Prior art in Bevy

- `warbler_grass`: instanced grass, last documented for Bevy 0.13.
- `bevy_procedural_grass`: instanced, wind, culling; Bevy 0.12.
- `bevy_feronia`: scattering plus foliage materials, early development; its
  compatibility table lists Bevy 0.18.

None targets Bevy 0.20 or generates trees, and none knows about voxel terrain,
so Ornithochory does not depend on them; their instanced-grass pipelines are
the reference for its cover renderer.

## 7. What this means for Ornithochory

1. **One intermediate form.** Every structural grower (L-system, space
   colonization, later Weber-Penn and self-organizing) emits a
   `PlantSkeleton`: nodes with position, radius, parent, order. Pipe-model
   radii, phyllotaxis, the tube mesher, leaves, wind weights and LOD all work
   on that, so a new grower is one trait impl.
2. **Cover is a species too**, with a density rule and per-distance
   representations (tufts, shells, material), so grass, moss and lichen use
   the same placement as trees.
3. **Placement is a pure function of position**, so it needs no saves and
   chunks re-scatter on load or edit, matching tarasaur's generator model.
4. **Variants, not individuals**: a species has N cached variants; plants are
   instances with a tint and phase.
5. **Wind in the vertex shader** from baked weights; interaction through a
   bender buffer that ornisculpt mobs write.
6. **Overdraw first**: geometry leaves near, trimmed cards mid, impostors far,
   with a depth prepass. Measure on tir before adding any alpha-tested layer.

## Sources

- Prusinkiewicz & Lindenmayer, *The Algorithmic Beauty of Plants* (1990), algorithmicbotany.org
- Weber & Penn, "Creation and Rendering of Realistic Trees", SIGGRAPH 1995
- [Runions, Lane & Prusinkiewicz, "Modeling Trees with a Space Colonization Algorithm" (2007)](https://www.algorithmicbotany.org/papers/colonization.egwnp2007.html)
- [Palubicki et al., "Self-organizing tree models for image synthesis" (2009)](https://algorithmicbotany.org/papers/selforg.sig2009.html)
- [Kuth et al., "Real-Time GPU Tree Generation", HPG 2025](https://diglib.eg.org/handle/10.2312/hpg20251168) ([PDF](https://gpuopen.com/download/Real-Time_GPU_Tree_Generation.pdf))
- [Jahrmann & Wimmer, "Responsive Real-Time Grass Rendering for General 3D Scenes", I3D 2017](https://www.cg.tuwien.ac.at/research/publications/2017/JAHRMANN-2017-RRTG/)
- [Wohllaib, "Procedural Grass in Ghost of Tsushima", GDC 2021](https://gdcvault.com/play/1027214/Advanced-Graphics-Summit-Procedural-Grass)
- [van Muijden, "GPU-Based Run-Time Procedural Placement in Horizon Zero Dawn", GDC 2017 (slides)](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf)
- [Desbenoit, Galin & Akkouche, "Simulating and modeling lichen growth", CGF 2004](https://diglib.eg.org/handle/10.2312/CGF.v23i3pp341-350)
- Sousa, "Vegetation Procedural Animation and Shading in Crysis", GPU Gems 3, ch. 16 (2007)
- [Pixar Moss, SIGGRAPH talk](https://history.siggraph.org/?p=77389)
- [Clemson thesis on Spanish moss](https://tigerprints.clemson.edu/all_theses/2603)
- [Unreal Engine 5.7 What's New (Nanite Foliage)](https://www.unrealengine.com/en-US/whats-new); [Tom Looman, UE 5.7 performance highlights](https://tomlooman.com/unreal-engine-5-7-performance-highlights/)
- [bevy_feronia](https://docs.rs/bevy_feronia/), [warbler_grass](https://github.com/emiongit/warbler_grass), [bevy_procedural_grass](https://github.com/Affinator/bevy_procedural_grass)
