//! Leaves as geometry: a blade shaped along its midrib, curling down to the
//! tip.
//!
//! Hung where the grower put them (`Organ`s) and, if the species asks,
//! on every node near a tip, spiralled around the stem by the golden angle.
//!
//! Clumps (`cluster`) fan several leaves from one point, and leaf normals
//! can be bent towards an ellipsoid around the crown so it shades as one
//! volume.
//!
//! Cost: `2 * segments + 2` vertices and `2 * segments` triangles per leaf,
//! drawn from both sides without alpha test. A lobed leaf
//! (`Leaves::lobes`) is `3 * lobes + 1` vertices and `2 * lobes` triangles
//! at the hero tier. Far tiers keep a fraction of the
//! leaves and grow them so the crown's area stays the same.

use bevy::color::{Color, LinearRgba};
use bevy::prelude::*;
use fastnoise_lite::{FastNoiseLite, NoiseType};

use super::{MeshData, PlantLod};
use crate::num::{f32_from_u32, u32_from_usize, usize_from};
use crate::random::{PlantRng, hash_unit};
use crate::skeleton::{OrganKind, PlantSkeleton};
use crate::species::Leaves;

/// 137.5 degrees: successive leaves around a stem never line up.
const GOLDEN_ANGLE: f32 = 2.399_963;

/// The leaves of `skel` at `lod`.
#[must_use]
pub fn leaves(skel: &PlantSkeleton, spec: &Leaves, lod: PlantLod, seed: u64) -> MeshData {
    let mut out = MeshData::default();
    let lengths = skel.path_lengths();
    let reach = lengths.iter().copied().fold(1e-3, f32::max);
    let keep = match (lod, spec.lod_keep) {
        (PlantLod::Near, Some((near, _))) => near.clamp(0.01, 1.0),
        (PlantLod::Far, Some((_, far))) => far.clamp(0.01, 1.0),
        _ => lod.leaf_keep(),
    };
    // Kept leaves grow so the crown's leaf area stays the same, more in
    // width than in length: a few long spikes would fray the silhouette,
    // rounder blobs keep it.
    let (grow_long, grow_wide) = (keep.powf(-0.35), keep.powf(-0.65));
    let segments = lod.segments(spec.segments);
    let base = Color::srgb(spec.color[0], spec.color[1], spec.color[2]).to_linear();

    let mut hung = hang(skel, spec);

    // The crown proxy: an ellipsoid around every leaf's stem node.
    let (lo, hi) = hung.iter().filter_map(|h| skel.node(h.0)).fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(lo, hi), n| (lo.min(n.pos), hi.max(n.pos)),
    );
    // Clumping: a smooth 3D noise over the crown; leaves where it is low
    // are left out, so foliage gathers into masses with holes between.
    let clumps = (spec.clumping > 0.0).then(|| {
        let [b0, b1, b2, b3, ..] = seed.to_le_bytes();
        let mut f = FastNoiseLite::with_seed(i32::from_le_bytes([b0, b1, b2, b3]));
        f.set_noise_type(Some(NoiseType::OpenSimplex2));
        f.set_frequency(Some(spec.clump_size.max(0.05).recip()));
        f
    });
    // OpenSimplex2 is roughly symmetric about 0 with most values within
    // +-0.6, so this cuts about `clumping` of the leaves.
    let threshold = spec.clumping.clamp(0.0, 1.0).mul_add(1.2, -0.6);
    hung.retain(|h| {
        clumps.as_ref().is_none_or(|f| skel.node(h.0).is_some_and(|n| f.get_noise_3d(n.pos.x, n.pos.y, n.pos.z) >= threshold))
    });
    let centre = (lo + hi) * 0.5;
    let half = ((hi - lo) * 0.5).max(Vec3::splat(0.05));
    let cluster = spec.cluster.max(1);

    for (k, (node, dir, scale)) in hung.into_iter().enumerate() {
        // Thin by a hash of the leaf, not the RNG, so a leaf kept at `Far`
        // is also kept at `Near` and `Hero`: tiers don't reshuffle.
        let k = u64::try_from(k).unwrap_or(0);
        if hash_unit(seed, k) >= keep {
            continue;
        }
        // Each leaf's own stream, for the same reason.
        let mut rng = PlantRng::new(crate::random::stable_hash(seed ^ k, "leaf"));
        let Some(n) = skel.node(node) else { continue };
        let local = (n.pos - centre) / half;
        // Ellipsoid gradient: the crown's outward normal here.
        let outward = (local / half).normalize_or(Vec3::Y);
        let depth = local.length().min(1.0);
        let occlusion = spec.inner_shade.clamp(0.0, 1.0).mul_add(depth - 1.0, 1.0);
        let path = lengths.get(usize_from(node)).copied().unwrap_or(0.0);
        let wind = Vec4::new(path / reach, 1.0, hash_unit(seed ^ 0x1eaf, u64::from(node)), 1.0);
        let fan = dir.any_orthonormal_vector();
        // Only clumps draw a roll, so single leaves keep their old stream.
        let roll = if cluster > 1 { rng.range(0.0, std::f32::consts::TAU) } else { 0.0 };
        for c in 0..cluster {
            // A clump: the first leaf on `dir`, the rest fanned around it.
            let dir = if c == 0 {
                dir
            } else {
                let around = Quat::from_axis_angle(dir, GOLDEN_ANGLE.mul_add(f32_from_u32(u32::from(c)), roll)) * fan;
                let (s, co) = (spec.cluster_spread * rng.range(0.7, 1.2)).sin_cos();
                (dir * co + around * s).normalize_or(dir)
            };
            let jitter = rng.signed() * spec.color_jitter;
            let shade = (1.0 + jitter).max(0.0) * occlusion;
            let color = LinearRgba::rgb(base.red * shade, base.green * shade, base.blue * shade);
            let droop = spec.droop * rng.range(0.7, 1.3);
            let dir = dir.lerp(Vec3::NEG_Y, (droop / std::f32::consts::PI).clamp(0.0, 0.9)).normalize_or(dir);
            let size = scale * rng.range(0.8, 1.2);
            let mesh = if spec.lobes > 0 && lod == PlantLod::Hero { lobed } else { blade };
            mesh(
                &mut out,
                &Blade {
                    root: n.pos + dir * n.radius,
                    dir,
                    length: spec.length * size * grow_long,
                    width: spec.width * size * grow_wide,
                    segments,
                    color: color.to_vec4(),
                    wind,
                    outward,
                    bend: spec.crown_normals.clamp(0.0, 1.0),
                    lobes: u32::from(spec.lobes),
                },
            );
        }
    }
    out
}

/// (node, direction, scale) for every leaf before LOD thinning: the
/// grower's own, then `tip_depth`'s.
fn hang(skel: &PlantSkeleton, spec: &Leaves) -> Vec<(u32, Vec3, f32)> {
    let tips = skel.tip_distances();
    let tangent = |i: u32| -> Vec3 {
        let n = skel.node(i);
        let p = n.and_then(|n| n.parent).and_then(|p| skel.node(p));
        match (n, p) {
            (Some(n), Some(p)) => (n.pos - p.pos).normalize_or(Vec3::Y),
            _ => Vec3::Y,
        }
    };

    let mut hung: Vec<(u32, Vec3, f32)> = skel
        .organs
        .iter()
        .filter(|o| o.kind == OrganKind::Leaf)
        .map(|o| (o.node, o.dir, o.scale))
        .collect();
    if spec.tip_depth > 0 {
        let planar = spec.planar.clamp(0.0, 1.0);
        for (i, &d) in tips.iter().enumerate() {
            let i = u32_from_usize(i);
            if d > spec.tip_depth || skel.node(i).is_none_or(|n| n.parent.is_none()) {
                continue;
            }
            let t = tangent(i);
            let side = t.any_orthonormal_vector();
            let flank = t.cross(Vec3::Y).try_normalize().unwrap_or(side);
            for j in 0..spec.per_node {
                let nth = i.wrapping_mul(u32::from(spec.per_node)).wrapping_add(u32::from(j));
                let turn = GOLDEN_ANGLE * f32_from_u32(nth);
                let spiral = Quat::from_axis_angle(t, turn) * side;
                let flat = if nth.is_multiple_of(2) { flank } else { -flank };
                let out_dir = spiral.lerp(flat, planar).normalize_or(flat);
                hung.push((i, (out_dir + t * spec.along).normalize_or(t), 1.0));
            }
        }
    }

    hung
}

/// One leaf to mesh.
struct Blade {
    root: Vec3,
    dir: Vec3,
    length: f32,
    width: f32,
    segments: u32,
    color: Vec4,
    wind: Vec4,
    /// The crown's outward normal at the leaf: its face is turned to this
    /// side, and its normal bent towards it by `bend`.
    outward: Vec3,
    bend: f32,
    /// Teeth a side, for [`lobed`].
    lobes: u32,
}

impl Blade {
    /// The blade's frame: across, face, and the normal both faces are
    /// drawn with (the foliage material doesn't flip it on back faces), so
    /// it is turned to the lit, outer side and bent towards the crown's.
    fn frame(&self) -> (Vec3, Vec3, Vec3) {
        let up = if self.dir.y.abs() > 0.95 { Vec3::X } else { Vec3::Y };
        let across = self.dir.cross(up).normalize_or(Vec3::X);
        let face = across.cross(self.dir).normalize_or(Vec3::Y);
        let facing = if face.dot(self.outward + Vec3::Y * 0.5) < 0.0 { -face } else { face };
        (across, face, facing.lerp(self.outward, self.bend).normalize_or(facing))
    }
}

/// Appends one leaf from `b.root` along `b.dir`.
fn blade(out: &mut MeshData, b: &Blade) {
    let (dir, length, segments) = (b.dir, b.length, b.segments);
    let (across, face, normal) = b.frame();
    let curl = 0.25 * length;

    // A point at the stalk, `segments` rows of two across the blade, a
    // point at the tip: a diamond at 1 segment, rounder with more.
    let rows = f32_from_u32(segments.saturating_add(1));
    let at = |t: f32| b.root + dir * (length * t) - face * (curl * t * t);
    let first = out.vertex(at(0.0), normal, Vec2::new(0.5, 0.0), b.wind, Some(b.color));
    for s in 1..=segments {
        let t = f32_from_u32(s) / rows;
        // Widest about 40% of the way up.
        let half = b.width * 0.5 * (std::f32::consts::PI * t.powf(0.756)).sin();
        let mid = at(t);
        out.vertex(mid - across * half, normal, Vec2::new(0.0, t), b.wind, Some(b.color));
        out.vertex(mid + across * half, normal, Vec2::new(1.0, t), b.wind, Some(b.color));
    }
    let tip = out.vertex(at(1.0), normal, Vec2::new(0.5, 1.0), b.wind, Some(b.color));

    let row = |s: u32| first.wrapping_add(1).wrapping_add(s.wrapping_mul(2));
    out.triangle(first, row(0), row(0).wrapping_add(1));
    for s in 0..segments.saturating_sub(1) {
        let (l, r, nl, nr) = (row(s), row(s).wrapping_add(1), row(s.wrapping_add(1)), row(s.wrapping_add(1)).wrapping_add(1));
        out.triangle(l, nl, r);
        out.triangle(r, nl, nr);
    }
    let last = row(segments.saturating_sub(1));
    out.triangle(last, tip, last.wrapping_add(1));
}

/// A pinnatifid leaf from `b.root` along `b.dir`: the same curled midrib
/// and outline as [`blade`], cut into `b.lobes` teeth a side that sweep
/// towards the tip, each one triangle on the midrib.
fn lobed(out: &mut MeshData, b: &Blade) {
    let (across, face, normal) = b.frame();
    let (length, width, lobes) = (b.length, b.width, b.lobes.max(1));
    let curl = 0.25 * length;
    let mid = |t: f32| b.root + b.dir * (length * t) - face * (curl * t * t);
    let steps = f32_from_u32(lobes);

    let first = out.vertex_count();
    for j in 0..=lobes {
        let t = f32_from_u32(j) / steps;
        out.vertex(mid(t), normal, Vec2::new(0.5, t), b.wind, Some(b.color));
    }
    for j in 0..lobes {
        // Each tooth spans two midrib steps, overlapping the next one
        // (coplanar and the same colour, so the overlap doesn't show): the
        // sinuses between teeth come halfway to the midrib, as on a
        // pinnatifid fern pinna. Its tip leans towards the leaf's tip and is
        // as wide as the plain blade's outline there.
        let t = ((f32_from_u32(j) + 1.3) / steps).min(1.0 - 0.3 / steps);
        let half = width * 0.5 * (std::f32::consts::PI * t.mul_add(0.75, 0.25)).sin();
        let (a, c) = (first.wrapping_add(j), first.wrapping_add(j.saturating_add(2).min(lobes)));
        let left = out.vertex(mid(t) - across * half, normal, Vec2::new(0.0, t), b.wind, Some(b.color));
        out.triangle(a, c, left);
        let right = out.vertex(mid(t) + across * half, normal, Vec2::new(1.0, t), b.wind, Some(b.color));
        out.triangle(a, right, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grow::LSystem;

    fn shrub() -> PlantSkeleton {
        let mut s = LSystem::default().grow(&mut PlantRng::new(4));
        s.pipe_radii(0.01, 2.0);
        s
    }

    #[test]
    fn far_keeps_fewer_leaves_with_the_same_area() {
        let spec = Leaves::default();
        let hero = leaves(&shrub(), &spec, PlantLod::Hero, 1);
        let far = leaves(&shrub(), &spec, PlantLod::Far, 1);
        assert!(far.positions.len() * 4 < hero.positions.len());
        let area = |m: &MeshData| -> f32 {
            m.indices
                .chunks(3)
                .map(|t| {
                    let p = |i: u32| Vec3::from(m.positions[usize_from(i)]);
                    (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0])).length() * 0.5
                })
                .sum()
        };
        let ratio = area(&far) / area(&hero);
        assert!((0.5..2.0).contains(&ratio), "area ratio {ratio}");
    }

    #[test]
    fn clumps_and_clumping_scale_the_leaf_count() {
        let tris = |spec: &Leaves| leaves(&shrub(), spec, PlantLod::Hero, 3).indices.len();
        let single = tris(&Leaves::default());
        assert_eq!(tris(&Leaves { cluster: 3, ..Leaves::default() }), single * 3);
        let gappy = tris(&Leaves { clumping: 0.5, clump_size: 0.3, ..Leaves::default() });
        assert!(gappy < single && gappy > 0, "{gappy} of {single}");
    }

    #[test]
    fn crown_normals_point_out_of_the_crown() {
        let skel = shrub();
        let m = leaves(&skel, &Leaves { crown_normals: 1.0, ..Leaves::default() }, PlantLod::Hero, 5);
        let (lo, hi) = skel.bounds();
        let centre = (lo + hi) * 0.5;
        let outward = m
            .positions
            .iter()
            .zip(&m.normals)
            .filter(|(p, n)| (Vec3::from(**p) - centre).dot(Vec3::from(**n)) > 0.0)
            .count();
        assert!(outward * 10 > m.positions.len() * 9, "{outward} of {}", m.positions.len());
    }

    #[test]
    fn lobed_leaves_cost_two_triangles_a_lobe() {
        let plain = Leaves::default();
        let lobed = Leaves { lobes: 5, ..Leaves::default() };
        let count = |m: &MeshData| m.indices.len() / 3;
        let hero = leaves(&shrub(), &lobed, PlantLod::Hero, 3);
        // A plain blade is `2 * segments` triangles.
        let leaf_count = count(&leaves(&shrub(), &plain, PlantLod::Hero, 3)) / (2 * usize::from(plain.segments));
        assert_eq!(count(&hero), leaf_count * 10);
        assert!(hero.indices.iter().all(|&i| i < hero.vertex_count()));
        // Coarser tiers fall back to the plain blade.
        assert_eq!(leaves(&shrub(), &lobed, PlantLod::Far, 3), leaves(&shrub(), &plain, PlantLod::Far, 3));
    }

    #[test]
    fn every_index_is_in_range() {
        let m = leaves(&shrub(), &Leaves { segments: 3, cluster: 2, planar: 0.5, ..Leaves::default() }, PlantLod::Hero, 2);
        assert!(m.indices.iter().all(|&i| i < m.vertex_count()));
        assert_eq!(m.colors.len(), m.positions.len());
    }
}
