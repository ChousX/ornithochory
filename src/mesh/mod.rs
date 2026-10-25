//! Skeleton -> triangles, per [`PlantLod`].
//!
//! - [`tube`]: stems as generalized cylinders along the skeleton's chains,
//!   with parallel-transport frames so they don't twist.
//! - [`leaves`]: geometry leaves (no alpha test: on tiled GPUs overdraw from
//!   `discard` costs more than a few extra vertices).
//! - [`cover`]: grass clumps and moss cushions, merged per patch.
//!
//! Every vertex carries [`ATTRIBUTE_WIND`] so a wind shader needs nothing
//! from the CPU per frame: `x` how far up the plant (0 at the root, 1 at the
//! farthest tip), `y` branch order (0..1), `z` a per-branch phase (0..1),
//! `w` 1 on leaves and blades (flutter), 0 on stems.

pub mod cover;
pub mod leaves;
pub mod tube;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, MeshVertexAttribute, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::VertexFormat;

use crate::num::{u32_from_usize, usize_from};

/// GPU bytes of the attributes every vertex has: position and normal (12
/// each), UV (8) and wind (16).
const BASE_VERTEX_BYTES: usize = 48;
/// GPU bytes of a colour or a tangent, on meshes that have them.
const VEC4_BYTES: usize = 16;
/// GPU bytes of an index (`u32`).
const INDEX_BYTES: usize = 4;

/// Per-vertex wind weights; see the module docs.
pub const ATTRIBUTE_WIND: MeshVertexAttribute =
    MeshVertexAttribute::new("Ornithochory_Wind", 0x0e17_4c40_9a1d_0001, VertexFormat::Float32x4);

/// How much detail a plant is built with. Chosen per plant by projected
/// size (see `crate::plant::PlantQuality`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PlantLod {
    /// Full detail, for plants close up.
    #[default]
    Hero,
    /// Fewer stem sides, twigs and leaves, for the middle distance.
    Near,
    /// The fewest, for plants small on screen.
    Far,
}

impl PlantLod {
    /// Every tier, finest first.
    pub const ALL: [Self; 3] = [Self::Hero, Self::Near, Self::Far];

    /// The tier's place in [`Self::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Hero => 0,
            Self::Near => 1,
            Self::Far => 2,
        }
    }

    /// Ring sides for a stem `radius` thick, when the thickest is `max` and
    /// gets `base` sides at the hero tier.
    #[must_use]
    pub fn sides(self, radius: f32, max: f32, base: u8) -> u32 {
        let full = f32::from(base.max(3)) * (radius / max.max(1e-6)).sqrt().clamp(0.0, 1.0);
        let scaled = match self {
            Self::Hero => full,
            Self::Near => full * 0.6,
            Self::Far => full * 0.35,
        };
        crate::num::u32_from_f32(scaled.round()).max(3)
    }

    /// Stems thinner than `tip_radius` times this are left out.
    #[must_use]
    pub const fn min_radius_factor(self) -> f32 {
        match self {
            Self::Hero => 0.0,
            Self::Near => 1.8,
            Self::Far => 4.0,
        }
    }

    /// Fraction of leaves kept. Kept leaves grow by `1 / keep` in area
    /// (more across than along) so the crown's leaf area, and so its
    /// silhouette and shade, stays the same.
    #[must_use]
    pub const fn leaf_keep(self) -> f32 {
        match self {
            Self::Hero => 1.0,
            Self::Near => 0.4,
            Self::Far => 0.12,
        }
    }

    /// Lengthwise leaf and blade segments, capped by the authored count.
    #[must_use]
    pub fn segments(self, authored: u8) -> u32 {
        match self {
            Self::Hero => u32::from(authored.max(1)),
            Self::Near | Self::Far => 1,
        }
    }
}

/// One `T` for each [`PlantLod`], by name: what a variant holds per tier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PerTier<T> {
    /// At [`PlantLod::Hero`].
    pub hero: T,
    /// At [`PlantLod::Near`].
    pub near: T,
    /// At [`PlantLod::Far`].
    pub far: T,
}

impl<T> PerTier<T> {
    /// Each tier's value from `f`, finest first.
    pub fn from_fn(mut f: impl FnMut(PlantLod) -> T) -> Self {
        Self { hero: f(PlantLod::Hero), near: f(PlantLod::Near), far: f(PlantLod::Far) }
    }

    /// The value at `lod`.
    #[must_use]
    pub const fn get(&self, lod: PlantLod) -> &T {
        match lod {
            PlantLod::Hero => &self.hero,
            PlantLod::Near => &self.near,
            PlantLod::Far => &self.far,
        }
    }

    /// The value at `lod`, to change.
    pub const fn get_mut(&mut self, lod: PlantLod) -> &mut T {
        match lod {
            PlantLod::Hero => &mut self.hero,
            PlantLod::Near => &mut self.near,
            PlantLod::Far => &mut self.far,
        }
    }

    /// Every tier's value through `f`, finest first.
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> PerTier<U> {
        PerTier { hero: f(self.hero), near: f(self.near), far: f(self.far) }
    }

    /// Each tier and its value, finest first.
    pub fn iter(&self) -> impl Iterator<Item = (PlantLod, &T)> {
        PlantLod::ALL.into_iter().map(move |lod| (lod, self.get(lod)))
    }
}

/// Triangles under construction, in plant space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshData {
    /// Plant space, metres.
    pub positions: Vec<[f32; 3]>,
    /// Unit, one per position.
    pub normals: Vec<[f32; 3]>,
    /// Texture coordinates, one per position.
    pub uvs: Vec<[f32; 2]>,
    /// Linear RGBA tint; empty when the material carries the colour.
    pub colors: Vec<[f32; 4]>,
    /// Tangents for normal maps (`xyz` along +U, `w` handedness); empty
    /// unless [`MeshData::compute_tangents`] ran. Stems get them, so bark
    /// can be normal-mapped with an ordinary `StandardMaterial`.
    pub tangents: Vec<[f32; 4]>,
    /// [`ATTRIBUTE_WIND`] weights, one per position.
    pub wind: Vec<[f32; 4]>,
    /// Three per triangle, counter-clockwise seen from its front.
    pub indices: Vec<u32>,
}

impl MeshData {
    /// No triangles: nothing to draw.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Vertices so far, saturating at `u32::MAX`.
    #[must_use]
    pub fn vertex_count(&self) -> u32 {
        u32_from_usize(self.positions.len())
    }

    /// Adds a vertex and returns its index.
    pub fn vertex(&mut self, pos: Vec3, normal: Vec3, uv: Vec2, wind: Vec4, color: Option<Vec4>) -> u32 {
        let i = self.vertex_count();
        self.positions.push(pos.to_array());
        self.normals.push(normal.normalize_or(Vec3::Y).to_array());
        self.uvs.push(uv.to_array());
        self.wind.push(wind.to_array());
        if let Some(c) = color {
            self.colors.push(c.to_array());
        }
        i
    }

    /// Adds a triangle over three vertices, counter-clockwise seen from its
    /// front.
    pub fn triangle(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend([a, b, c]);
    }

    /// Appends `other` moved by `transform`, tinted by `tint` (multiplied
    /// into its colours). How cover patches are merged.
    pub fn append_transformed(&mut self, other: &Self, transform: &Transform, tint: Vec4) {
        let base = self.vertex_count();
        let m = transform.to_matrix();
        let rot = transform.rotation;
        self.positions
            .extend(other.positions.iter().map(|&p| m.transform_point3(Vec3::from(p)).to_array()));
        self.normals.extend(other.normals.iter().map(|&n| (rot * Vec3::from(n)).to_array()));
        self.uvs.extend_from_slice(&other.uvs);
        self.wind.extend_from_slice(&other.wind);
        if other.colors.len() == other.positions.len() {
            self.colors.extend(other.colors.iter().map(|&c| (Vec4::from(c) * tint).to_array()));
        }
        if other.tangents.len() == other.positions.len() {
            self.tangents
                .extend(other.tangents.iter().map(|&t| (rot * Vec4::from(t).truncate()).extend(t[3]).to_array()));
        }
        self.indices.extend(other.indices.iter().map(|&i| i.wrapping_add(base)));
    }

    /// Fills [`MeshData::tangents`] from the UVs: per-triangle UV
    /// derivatives summed per vertex, then made perpendicular to the normal
    /// (the classic Lengyel method; enough for the smooth UVs the meshers
    /// write, and far cheaper than `MikkTSpace`).
    pub fn compute_tangents(&mut self) {
        let n = self.positions.len();
        let mut tan = vec![Vec3::ZERO; n];
        let mut bitan = vec![Vec3::ZERO; n];
        let at = |v: &[[f32; 3]], i: u32| v.get(usize_from(i)).map_or(Vec3::ZERO, |&p| Vec3::from(p));
        let uv = |i: u32| self.uvs.get(usize_from(i)).map_or(Vec2::ZERO, |&u| Vec2::from(u));
        for &[a, b, c] in self.indices.as_chunks::<3>().0 {
            let (e1, e2) = (at(&self.positions, b) - at(&self.positions, a), at(&self.positions, c) - at(&self.positions, a));
            let (d1, d2) = (uv(b) - uv(a), uv(c) - uv(a));
            let det = d1.x.mul_add(d2.y, -(d2.x * d1.y));
            if det.abs() < 1e-12 {
                continue;
            }
            let r = det.recip();
            let t = (e1 * d2.y - e2 * d1.y) * r;
            let bt = (e2 * d1.x - e1 * d2.x) * r;
            for i in [a, b, c] {
                if let Some(v) = tan.get_mut(usize_from(i)) {
                    *v += t;
                }
                if let Some(v) = bitan.get_mut(usize_from(i)) {
                    *v += bt;
                }
            }
        }
        self.tangents = tan
            .into_iter()
            .zip(bitan)
            .zip(&self.normals)
            .map(|((t, bt), &nrm)| {
                let nrm = Vec3::from(nrm);
                let t = (t - nrm * nrm.dot(t)).normalize_or(nrm.any_orthonormal_vector());
                let w = if nrm.cross(t).dot(bt) < 0.0 { -1.0 } else { 1.0 };
                t.extend(w).to_array()
            })
            .collect();
    }

    /// The Bevy mesh. Main-world copies are dropped once uploaded.
    #[must_use]
    pub fn into_mesh(self) -> Mesh {
        let n = self.positions.len();
        let has_colors = !self.colors.is_empty() && self.colors.len() == n;
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
            .with_inserted_attribute(ATTRIBUTE_WIND, self.wind)
            .with_inserted_indices(Indices::U32(self.indices));
        if has_colors {
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        }
        if !self.tangents.is_empty() && self.tangents.len() == n {
            mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, self.tangents);
        }
        mesh
    }

    /// Bytes this mesh takes on the GPU (vertices + indices), for budgets.
    #[must_use]
    pub const fn gpu_bytes(&self) -> usize {
        let colors = if self.colors.is_empty() { 0 } else { VEC4_BYTES };
        let tangents = if self.tangents.is_empty() { 0 } else { VEC4_BYTES };
        let per_vertex = BASE_VERTEX_BYTES.saturating_add(colors).saturating_add(tangents);
        self.positions
            .len()
            .saturating_mul(per_vertex)
            .saturating_add(self.indices.len().saturating_mul(INDEX_BYTES))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_tier_values_go_by_their_tier() {
        let mut t = PerTier::from_fn(PlantLod::index);
        assert_eq!(t, PerTier { hero: 0, near: 1, far: 2 });
        *t.get_mut(PlantLod::Near) += 10;
        assert_eq!(*t.get(PlantLod::Near), 11);
        assert_eq!(t.map(|v| v * 2), PerTier { hero: 0, near: 22, far: 4 });
        let order: Vec<(PlantLod, usize)> = t.iter().map(|(lod, &v)| (lod, v)).collect();
        assert_eq!(order, [(PlantLod::Hero, 0), (PlantLod::Near, 11), (PlantLod::Far, 2)]);
    }

    #[test]
    fn tangents_follow_u_and_stay_perpendicular() {
        // A quad in the XY plane facing +Z, U along +X, V along +Y.
        let mut quad = MeshData::default();
        let corners = [(Vec3::ZERO, Vec2::ZERO), (Vec3::X, Vec2::X), (Vec3::new(1.0, 1.0, 0.0), Vec2::ONE), (Vec3::Y, Vec2::Y)];
        let ids = corners.map(|(p, uv)| quad.vertex(p, Vec3::Z, uv, Vec4::ZERO, None));
        quad.triangle(ids[0], ids[1], ids[2]);
        quad.triangle(ids[0], ids[2], ids[3]);
        quad.compute_tangents();
        assert_eq!(quad.tangents.len(), 4);
        for t in &quad.tangents {
            assert!((Vec3::from_slice(t) - Vec3::X).length() < 1e-5, "{t:?}");
            assert!((t[3] - 1.0).abs() < 1e-6);
        }
    }
}
