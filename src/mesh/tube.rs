//! Stems: one generalized cylinder per skeleton chain.
//!
//! Frames are carried along each chain by parallel transport (the ring's
//! reference direction is rotated by the same rotation that turns one
//! tangent into the next), so tubes never twist or pinch at bends.
//!
//! Cost: `sides + 1` vertices per node (one duplicate for the UV seam) and
//! `2 * sides` triangles per segment. `PlantLod` cuts sides on thin and far
//! stems and drops the thinnest twigs entirely.

use bevy::prelude::*;

use super::{MeshData, PlantLod};
use crate::num::{f32_from_u32, usize_from};
use crate::random::hash_unit;
use crate::skeleton::PlantSkeleton;
use crate::species::Bark;

/// The stems of `skel` at `lod`.
#[must_use]
pub fn stems(skel: &PlantSkeleton, bark: &Bark, tip_radius: f32, lod: PlantLod) -> MeshData {
    let base_sides = bark.sides;
    let mut out = MeshData::default();
    let lengths = skel.path_lengths();
    let reach = lengths.iter().copied().fold(1e-3, f32::max);
    let max_order = f32::from(skel.nodes.iter().map(|n| n.order).max().unwrap_or(0).max(1));
    let max_radius = skel.nodes.iter().map(|n| n.radius).fold(1e-6, f32::max);
    let min_radius = (tip_radius * lod.min_radius_factor()).max(bark.min_radius);

    for (ci, chain) in axes(skel).iter().enumerate() {
        // Rings: (centre, radius, path length from the root).
        let mut rings: Vec<(Vec3, f32, f32)> = Vec::with_capacity(chain.len());
        let mut order = 0u8;
        for (k, &i) in chain.iter().enumerate() {
            let Some(n) = skel.node(i) else { continue };
            // A branch's first node is its parent's: start no wider than the
            // branch itself so it doesn't bulge out of the stem it leaves.
            let r = if k == 0 && chain.len() > 1 {
                let next = chain.get(1).and_then(|&j| skel.node(j)).map_or(n.radius, |c| c.radius);
                n.radius.min(next * 1.15)
            } else {
                n.radius
            };
            if k > 1 && r < min_radius {
                break;
            }
            if k == 1 {
                order = n.order;
            }
            rings.push((n.pos, r, lengths.get(usize_from(i)).copied().unwrap_or(0.0)));
        }
        if rings.len() < 2 || rings.get(1).is_some_and(|r| r.1 < min_radius) {
            continue;
        }

        let thickest = rings.iter().map(|r| r.1).fold(0.0, f32::max);
        if lod != PlantLod::Far && thickest > (tip_radius * SMOOTH_ABOVE).max(max_radius * SMOOTH_FRACTION) {
            rings = smoothed(&rings);
        }
        let sides = lod.sides(thickest, max_radius, base_sides);
        let phase = hash_unit(u64::try_from(ci).unwrap_or(0), 0x5eed);
        let order_w = f32::from(order) / max_order;
        tube(&mut out, &rings, sides, |path| Vec4::new(path / reach, order_w, phase, 0.0));
    }
    out
}

/// Limbs thicker than `tip_radius` times this, and than this fraction of
/// the trunk, get a Catmull-Rom midpoint in every segment (Hero and Near),
/// so a coarse skeleton bends smoothly where it shows most. Twigs keep
/// their nodes.
const SMOOTH_ABOVE: f32 = 3.0;
const SMOOTH_FRACTION: f32 = 0.25;

/// The skeleton cut into axes like [`PlantSkeleton::chains`], except that
/// an axis carries on into the child with the most cross-section, weighted
/// by how straight it carries on. Counting nodes instead (as `chains`
/// does) can run a thick limb into a thin, hooked twig while a thicker
/// branch leaves it: the limb pinches, then a fatter one starts beside it.
fn axes(skel: &PlantSkeleton) -> Vec<Vec<u32>> {
    let kids = skel.children();
    let dir = |from: Vec3, to: u32| skel.node(to).map_or(Vec3::ZERO, |n| (n.pos - from).normalize_or_zero());
    let cont: Vec<Option<u32>> = skel
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let incoming = n.parent.and_then(|p| skel.node(p)).map_or(Vec3::Y, |p| (n.pos - p.pos).normalize_or(Vec3::Y));
            let score = |c: u32| {
                let r = skel.node(c).map_or(0.0, |c| c.radius);
                r * r * incoming.dot(dir(n.pos, c)).mul_add(0.5, 0.5).max(0.05)
            };
            kids.get(i)?.iter().copied().max_by(|&a, &b| score(a).total_cmp(&score(b)).then(b.cmp(&a)))
        })
        .collect();
    let follow = |start: Vec<u32>| {
        let mut chain = start;
        while let Some(next) = chain.last().and_then(|&l| cont.get(usize_from(l)).copied().flatten()) {
            chain.push(next);
        }
        chain
    };
    let mut chains = Vec::new();
    for (i, n) in skel.nodes.iter().enumerate() {
        let i = crate::num::u32_from_usize(i);
        match n.parent {
            None => chains.push(follow(vec![i])),
            Some(p) if cont.get(usize_from(p)).copied().flatten() != Some(i) => chains.push(follow(vec![p, i])),
            Some(_) => {}
        }
    }
    chains
}

/// `rings` with a Catmull-Rom midpoint in every segment.
fn smoothed(rings: &[(Vec3, f32, f32)]) -> Vec<(Vec3, f32, f32)> {
    let at = |k: usize| rings.get(k.min(rings.len().saturating_sub(1))).copied().unwrap_or((Vec3::ZERO, 0.0, 0.0));
    let mut out = Vec::with_capacity(rings.len().saturating_mul(2));
    for k in 0..rings.len() {
        let p1 = at(k);
        out.push(p1);
        if k.saturating_add(1) >= rings.len() {
            break;
        }
        let (p0, p2, p3) = (at(k.saturating_sub(1)), at(k.saturating_add(1)), at(k.saturating_add(2)));
        // Catmull-Rom at t = 0.5: (-p0 + 9 p1 + 9 p2 - p3) / 16.
        let mid = ((p1.0 + p2.0) * 9.0 - p0.0 - p3.0) / 16.0;
        out.push((mid, p1.1.midpoint(p2.1), p1.2.midpoint(p2.2)));
    }
    out
}

/// Appends a tube through `rings` (centre, radius, path length).
fn tube(out: &mut MeshData, rings: &[(Vec3, f32, f32)], sides: u32, wind: impl Fn(f32) -> Vec4) {
    let tangent = |k: usize| -> Vec3 {
        let p = |j: usize| rings.get(j).map_or(Vec3::ZERO, |r| r.0);
        let prev = p(k.saturating_sub(1));
        let next = p(k.saturating_add(1).min(rings.len().saturating_sub(1)));
        (next - prev).normalize_or(Vec3::Y)
    };

    let mut t_prev = tangent(0);
    let mut normal = t_prev.any_orthonormal_vector();
    let circumference = rings.first().map_or(0.1, |r| r.1 * std::f32::consts::TAU).max(1e-3);
    let start = out.vertex_count();
    let ring_len = sides.saturating_add(1);

    for k in 0..rings.len() {
        let Some(&(centre, radius, path)) = rings.get(k) else { continue };
        let t = tangent(k);
        normal = Quat::from_rotation_arc(t_prev, t) * normal;
        t_prev = t;
        let binormal = t.cross(normal);
        let along = (path - rings.first().map_or(0.0, |r| r.2)) / circumference;
        for s in 0..=sides {
            let a = f32_from_u32(s) / f32_from_u32(sides) * std::f32::consts::TAU;
            let dir = normal * a.cos() + binormal * a.sin();
            out.vertex(
                centre + dir * radius,
                dir,
                Vec2::new(f32_from_u32(s) / f32_from_u32(sides), along),
                wind(path),
                None,
            );
        }
    }

    for k in 0..rings.len().saturating_sub(1) {
        let Ok(k) = u32::try_from(k) else { break };
        let a0 = start.wrapping_add(k.wrapping_mul(ring_len));
        let b0 = a0.wrapping_add(ring_len);
        for s in 0..sides {
            let (a, b) = (a0.wrapping_add(s), b0.wrapping_add(s));
            let (a1, b1) = (a.wrapping_add(1), b.wrapping_add(1));
            out.triangle(a, a1, b);
            out.triangle(a1, b1, b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pole() -> PlantSkeleton {
        let mut s = PlantSkeleton::rooted_at(Vec3::ZERO);
        let a = s.push(0, Vec3::Y, 0);
        s.push(a, Vec3::Y * 2.0, 0);
        s.pipe_radii(0.1, 2.0);
        s
    }

    #[test]
    fn faces_point_outwards() {
        let m = stems(&pole(), &Bark { sides: 8, ..Bark::default() }, 0.1, PlantLod::Hero);
        assert!(!m.is_empty());
        for tri in m.indices.chunks(3) {
            let at = |i: u32| Vec3::from(m.positions[usize_from(i)]);
            let (v0, v1, v2) = (at(tri[0]), at(tri[1]), at(tri[2]));
            let n = (v1 - v0).cross(v2 - v0);
            let centre = (v0 + v1 + v2) / 3.0;
            let out = Vec3::new(centre.x, 0.0, centre.z);
            assert!(n.dot(out) > 0.0, "inward face at {centre}");
        }
    }

    #[test]
    fn far_is_cheaper() {
        let bark = Bark { sides: 12, ..Bark::default() };
        let hero = stems(&pole(), &bark, 0.1, PlantLod::Hero);
        let far = stems(&pole(), &bark, 0.1, PlantLod::Far);
        assert!(far.indices.len() < hero.indices.len());
    }

    #[test]
    fn hidden_twigs_are_left_out() {
        let bark = Bark { min_radius: 0.5, ..Bark::default() };
        assert!(stems(&pole(), &bark, 0.1, PlantLod::Hero).is_empty());
    }
}
