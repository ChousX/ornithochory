//! Cover tufts: the small shape a cover species repeats over a surface.
//!
//! - Blades: a tussock of curved geometry blades (grass, sedge). Each blade
//!   is a quadratic Bezier (Jahrmann & Wimmer 2017) that keeps its width
//!   most of the way up, splays out from the clump's centre and twists a
//!   little. Short blades get fewer segments than tall ones, the shortest
//!   one: a cheap, dense understory.
//! - Cushion: overlapping low mounds with lobed outlines (cushion moss,
//!   lichen rosettes), optionally bristling with tiny shoots.
//!
//! Colour runs from a shaded root to a lighter tip (`Cover::shade`,
//! `Cover::tip_color`), with a few dry blades, which is most of what makes
//! a stand read as dense: the dark bases merge into one mass under the
//! tips, as in Ghost of Tsushima's grass. Normals are rounded across each
//! blade and bent towards the tuft's up, so a field shades like one
//! surface and a blade seen from behind is lit like its front (the cover
//! material doesn't flip back-face normals; see `PlantLibrary`).
//!
//! Cost: a blade is `2 * segments + 1` vertices and `2 * segments - 1`
//! triangles; a cushion mound `rings * sides + 1` vertices and
//! `(2 * rings - 1) * sides` triangles (`lumps` mounds, each 2 rings in a
//! cluster), plus 3 vertices and 1 triangle per bristle. Tufts are merged into one mesh per patch
//! (`MeshData::append_transformed`), so a patch is one draw however many
//! tufts it holds.

use bevy::asset::RenderAssetUsages;
use bevy::color::{Color, LinearRgba};
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::{MeshData, PlantLod};
use crate::num::{f32_from_u32, u32_from_f32, usize_from};
use crate::random::{PlantRng, hash_unit};
use crate::species::{Cover, Tuft};

/// How far blade and cushion normals bend towards the tuft's up, 0..1.
const NORMAL_UP: f32 = 0.5;
/// How far a blade's edge normals turn outwards, so it shades as if round.
const NORMAL_ROUND: f32 = 0.4;
/// How much of a dry blade's root keeps the green, 0..1.
const DRY_ROOT_GREEN: f32 = 0.5;
/// The fewest points around a cushion's rim, at any tier.
const MIN_RIM_SIDES: u32 = 5;

/// The most `height_jitter` can spread blade heights, so none reaches 0.
const MAX_HEIGHT_JITTER: f32 = 0.95;
/// Blade heights are drawn as `u^TALL_SKEW` of their spread: above 1, short
/// blades are the common ones.
const TALL_SKEW: f32 = 1.5;
/// Blades at the clump's centre stand this many times `height`, before
/// jitter...
const CENTRE_HEIGHT: f32 = 1.08;
/// ...and those at its rim this much less.
const RIM_SHORTFALL: f32 = 0.2;
/// How far a blade's bend turns from straight out of the clump, radians
/// either way: mostly outwards, some across, a few back over the centre.
const LEAN_YAW_SPREAD: f32 = 1.1;
/// Dry blades bend over this many times as far as green ones.
const DRY_DROOP: f32 = 1.3;
/// Each blade bends over by `lean` times a draw from this range.
const LEAN_JITTER: (f32, f32) = (0.5, 1.3);
/// The furthest any blade bends over, as a fraction of its height.
const MAX_LEAN: f32 = 0.75;
/// Blades whose height draw is under this are single triangles...
const SINGLE_TRIANGLE_BELOW: f32 = 0.3;
/// ...and under this one segment short of the tallest.
const FEWER_SEGMENTS_BELOW: f32 = 0.6;
/// How brown a green blade can be, 0..1, drawn as `u^3` so most are green.
const MAX_BROWNING: f32 = 0.35;
/// Each blade's width is `width` times a draw from this range.
const WIDTH_JITTER: (f32, f32) = (0.8, 1.2);
/// The most a blade twists from root to tip, radians either way.
const MAX_TWIST: f32 = 0.8;
/// A bent blade's tip drops by this many times `lean` heights, so the
/// blade keeps about its length.
const TIP_DROP: f32 = 0.45;

/// The waves around a mound's rim: (amplitude, as a fraction of `lobes`;
/// waves around).
const RIM_WAVES: [(f32, f32); 3] = [(0.5, 2.0), (0.35, 3.0), (0.25, 5.0)];
/// Each wave's amplitude is its own times a draw from this range.
const WAVE_JITTER: (f32, f32) = (0.6, 1.0);
/// A rim never comes in nearer than this fraction of the mound's radius.
const MIN_RIM: f32 = 0.4;
/// The rim sits this many mound heights below the ground, so a cushion on
/// a slope shows no gap.
const RIM_SINK: f32 = 0.15;
/// A mound's profile: `(1 - rho^FLAT_TOP)^SHOULDER` of its height at `rho`
/// from the centre. Higher `FLAT_TOP`, a flatter top...
const FLAT_TOP: f32 = 2.5;
/// ...and lower `SHOULDER`, a steeper drop to the rim.
const SHOULDER: f32 = 0.6;
/// A cluster's outer mounds sit this far out, as a fraction of the
/// cushion's radius.
const LUMP_DISTANCE: (f32, f32) = (0.3, 0.6);
/// Each mound's radius is the cushion's times a draw from this range...
const LUMP_RADIUS: (f32, f32) = (0.45, 0.62);
/// ...times `1 - LUMP_TAPER * distance`: outer mounds are smaller.
const LUMP_TAPER: f32 = 0.35;
/// Each mound's height is the cushion's times a draw from this range...
const LUMP_HEIGHT: (f32, f32) = (0.65, 1.1);
/// ...times `LUMP_PEAK - LUMP_SLOPE * distance`: the middle stands tallest.
const LUMP_PEAK: f32 = 1.1;
/// See [`LUMP_PEAK`].
const LUMP_SLOPE: f32 = 0.5;
/// A cluster's mounds have this share of the cushion's `lobes`.
const LUMP_LOBES: f32 = 0.6;
/// One mound standing in for a cluster reaches this share of each mound's
/// radius past its centre...
const STAND_IN_REACH: f32 = 0.8;
/// ...and is this share as tall as the tallest.
const STAND_IN_HEIGHT: f32 = 0.85;

/// A cushion's colour runs from root to tip by `(height / top)^this`:
/// above 1, the light only near the tops.
const CUSHION_SHADE_SKEW: f32 = 1.5;
/// A cushion's brightness jitters by this share of the cover's
/// `color_jitter`.
const CUSHION_JITTER: f32 = 0.6;
/// The wind weight up the plant at a cushion's top (it barely sways).
const CUSHION_SWAY: f32 = 0.2;

/// Bristles stand within this fraction of the way out to the rim...
const BRISTLE_SPAN: f32 = 0.92;
/// ...rooted this many mound heights under the surface.
const BRISTLE_ROOT_DEPTH: f32 = 0.12;
/// How far a bristle wanders from leaning straight out, horizontally per
/// unit of height.
const BRISTLE_WANDER: f32 = 0.3;
/// How far bristles at the rim lean out, per unit of height.
const BRISTLE_SPLAY: f32 = 1.2;
/// Each bristle's height is `bristle_height` times a draw from this range.
const BRISTLE_HEIGHT_JITTER: (f32, f32) = (0.6, 1.4);
/// A bristle's base is this many times its height across.
const BRISTLE_WIDTH: f32 = 0.09;
/// How far a bristle's normal turns to straight up, 0..1.
const BRISTLE_NORMAL_UP: f32 = 0.7;
/// A bristle's tip is this many times brighter than its base.
const BRISTLE_TIP_LIGHT: f32 = 1.1;

/// One tuft of `cover` at `lod`, centred on the origin, +Y up.
#[must_use]
pub fn tuft(cover: &Cover, lod: PlantLod, seed: u64) -> MeshData {
    let mut out = MeshData::default();
    let paint = Paint::new(cover);
    let mut rng = PlantRng::fork(seed, "tuft");
    match cover.tuft {
        Tuft::Blades { count, height, width, lean, segments, spread, height_jitter, splay } => {
            let clump = Clump { count, height, width, lean, segments, spread, height_jitter, splay };
            blades(&mut out, &clump, lod, &paint, &mut rng, seed);
        }
        Tuft::Cushion { radius, height, sides, lumps, lobes, bristles, bristle_height } => {
            let mounds = cluster(radius, height, lumps, lobes, lod, &mut rng);
            let sides = match lod {
                PlantLod::Hero => u32::from(sides).max(MIN_RIM_SIDES),
                PlantLod::Near => u32::from(sides).max(MIN_RIM_SIDES).div_ceil(2).max(MIN_RIM_SIDES),
                PlantLod::Far => MIN_RIM_SIDES,
            };
            let rings = rings(lod, mounds.len() > 1);
            let top = mounds.iter().map(|m| m.height).fold(1e-4, f32::max);
            for m in &mounds {
                cushion(&mut out, m, rings, sides, top, &paint, &mut rng);
            }
            let bristles = match lod {
                PlantLod::Hero => u32::from(bristles),
                PlantLod::Near => u32::from(bristles).div_ceil(4),
                PlantLod::Far => 0,
            };
            for b in 0..bristles {
                let pick = usize_from(b).checked_rem(mounds.len()).unwrap_or(0);
                let Some(m) = mounds.get(pick) else { break };
                bristle(&mut out, m, bristle_height * height, top, &paint, &mut rng, hash_unit(seed, u64::from(b)));
            }
        }
    }
    out
}

/// A cover's colours, linear.
struct Paint {
    root: LinearRgba,
    tip: LinearRgba,
    dry_root: LinearRgba,
    dry_tip: LinearRgba,
    jitter: f32,
    dry: f32,
    /// Planar UVs per metre on cushions, for the detail speckle.
    uv_per_m: f32,
}

impl Paint {
    fn new(cover: &Cover) -> Self {
        let srgb = |c: [f32; 3]| Color::srgb(c[0], c[1], c[2]).to_linear();
        let base = srgb(cover.color);
        let tip = cover.tip_color.map_or_else(|| scale(base, 1.0 + cover.shade), srgb);
        let dry = srgb(cover.dry_color);
        let root = scale(base, 1.0 - cover.shade);
        Self {
            root,
            tip,
            dry_root: mix(root, scale(dry, 1.0 - cover.shade), 1.0 - DRY_ROOT_GREEN),
            dry_tip: dry,
            jitter: cover.color_jitter,
            dry: cover.dry,
            uv_per_m: (cover.detail_size.max(1e-4) * f32_from_u32(DETAIL_CELLS)).recip(),
        }
    }

    /// The colour `t` of the way from root to tip, `dry` of the way to the
    /// dry colours, times `k`.
    fn at(&self, t: f32, dry: f32, k: f32) -> Vec4 {
        let t = t.clamp(0.0, 1.0);
        let green = mix(self.root, self.tip, t);
        let straw = mix(self.dry_root, self.dry_tip, t);
        let c = scale(mix(green, straw, dry.clamp(0.0, 1.0)), k.max(0.0));
        Vec4::new(c.red, c.green, c.blue, 1.0)
    }
}

fn scale(c: LinearRgba, k: f32) -> LinearRgba {
    LinearRgba::rgb(c.red * k, c.green * k, c.blue * k)
}

fn mix(a: LinearRgba, b: LinearRgba, t: f32) -> LinearRgba {
    LinearRgba::rgb(
        (b.red - a.red).mul_add(t, a.red),
        (b.green - a.green).mul_add(t, a.green),
        (b.blue - a.blue).mul_add(t, a.blue),
    )
}

/// `Tuft::Blades`, unpacked.
struct Clump {
    count: u8,
    height: f32,
    width: f32,
    lean: f32,
    segments: u8,
    spread: f32,
    height_jitter: f32,
    splay: f32,
}

fn blades(out: &mut MeshData, c: &Clump, lod: PlantLod, paint: &Paint, rng: &mut PlantRng, seed: u64) {
    let count = u32::from(c.count.max(1));
    let keep = match lod {
        PlantLod::Hero => count,
        PlantLod::Near => count.div_ceil(2),
        PlantLod::Far => count.div_ceil(4),
    };
    // Fewer, wider blades far away keep the clump's coverage.
    let widen = (f32_from_u32(count) / f32_from_u32(keep.max(1))).sqrt();
    let segments = lod.segments(c.segments);
    let radius = c.height * c.spread;
    let jitter = c.height_jitter.clamp(0.0, MAX_HEIGHT_JITTER);
    for b in 0..keep {
        // Uniform over the clump's disc; blades lean out from its centre.
        let d = rng.unit().sqrt();
        let around = rng.range(0.0, std::f32::consts::TAU);
        let out_dir = Vec3::new(around.cos(), 0.0, around.sin());
        let at = out_dir * (radius * d);
        // Short blades are the common ones; the centre stands tallest.
        let tall = rng.unit().powf(TALL_SKEW);
        let h = c.height * (1.5 * jitter).mul_add(tall, 1.0 - jitter) * d.mul_add(-RIM_SHORTFALL, CENTRE_HEIGHT);
        let dry = rng.chance(paint.dry);
        // Mostly outwards, some across, a few back over the centre.
        let yaw = rng.signed().mul_add(LEAN_YAW_SPREAD, around);
        let lean_dir = Vec3::new(yaw.cos(), 0.0, yaw.sin());
        let droop = if dry { DRY_DROOP } else { 1.0 };
        let lean = c.lean * rng.range(LEAN_JITTER.0, LEAN_JITTER.1) * (h / c.height.max(1e-4)) * droop;
        let tilt = Quat::from_axis_angle(out_dir.cross(Vec3::Y).normalize_or(Vec3::X), -c.splay * d);
        // Short blades need fewer segments to look curved; the shortest are
        // single triangles, a cheap dense understory under the tall ones.
        let blade_segs = if tall < SINGLE_TRIANGLE_BELOW {
            1
        } else if tall < FEWER_SEGMENTS_BELOW {
            segments.saturating_sub(1).max(1)
        } else {
            segments
        };
        let k = rng.signed().mul_add(paint.jitter, 1.0);
        // Dry blades are straw; some green ones are starting to turn.
        let browning = if dry { 1.0 } else { rng.unit().powi(3) * MAX_BROWNING };
        let width = c.width * widen * rng.range(WIDTH_JITTER.0, WIDTH_JITTER.1);
        let twist = rng.signed() * MAX_TWIST;
        let blade_spec = Blade {
            at,
            axis: tilt * Vec3::Y,
            lean_dir,
            height: h,
            width,
            lean: lean.min(MAX_LEAN),
            twist,
            segments: blade_segs,
            phase: hash_unit(seed, u64::from(b)),
            k,
            dry: browning,
        };
        blade(out, &blade_spec, paint);
    }
}

struct Blade {
    at: Vec3,
    /// Unit; the way the blade's root points.
    axis: Vec3,
    /// Unit, horizontal; the way it bends over.
    lean_dir: Vec3,
    height: f32,
    width: f32,
    /// 0 straight up, towards 1 bent over flat.
    lean: f32,
    /// Radians from root to tip about the blade's axis.
    twist: f32,
    segments: u32,
    phase: f32,
    k: f32,
    dry: f32,
}

fn blade(out: &mut MeshData, b: &Blade, paint: &Paint) {
    // Quadratic Bezier: the control point straight up the axis, the tip
    // pulled over by `lean` and down to keep the length about the same.
    let p0 = b.at;
    let p1 = b.at + b.axis * b.height;
    let p2 = b.at + b.axis * (b.height * b.lean.mul_add(-TIP_DROP, 1.0)) + b.lean_dir * (b.height * b.lean);
    let first = out.vertex_count();
    let segs = b.segments.max(1);
    let lit = |n: Vec3| n.normalize_or(Vec3::Y).lerp(Vec3::Y, NORMAL_UP);
    for s in 0..=segs {
        let t = f32_from_u32(s) / f32_from_u32(segs);
        let u = 1.0 - t;
        let centre = p0 * (u * u) + p1 * (2.0 * u * t) + p2 * (t * t);
        let tangent = ((p1 - p0) * u + (p2 - p1) * t).normalize_or(Vec3::Y);
        let across = (Quat::from_axis_angle(Vec3::Y, b.twist * t) * b.lean_dir.cross(Vec3::Y)).normalize_or(Vec3::X);
        // Facing back against the bend, so a field shades as one surface.
        let face = across.cross(tangent).normalize_or(Vec3::Y);
        let color = paint.at(t, b.dry, b.k);
        let wind = Vec4::new(t, 1.0, b.phase, 1.0);
        if s == segs {
            out.vertex(centre, lit(face), Vec2::new(0.5, 1.0), wind, Some(color));
            break;
        }
        // Strap-shaped: full width most of the way, then a quick taper.
        let half = b.width * 0.5 * t.mul_add(-t, 1.0);
        out.vertex(centre - across * half, lit(face - across * NORMAL_ROUND), Vec2::new(0.0, t), wind, Some(color));
        out.vertex(centre + across * half, lit(face + across * NORMAL_ROUND), Vec2::new(1.0, t), wind, Some(color));
    }
    for s in 0..segs {
        let row = first.wrapping_add(s.wrapping_mul(2));
        let (l, r, nl) = (row, row.wrapping_add(1), row.wrapping_add(2));
        out.triangle(l, r, nl);
        if s.wrapping_add(1) != segs {
            out.triangle(r, row.wrapping_add(3), nl);
        }
    }
}

/// One mound of a cushion: a flat-topped dome with a lobed rim.
struct Mound {
    centre: Vec2,
    radius: f32,
    height: f32,
    /// `(amplitude, frequency, phase)` of the rim's lobes.
    lobes: [(f32, f32, f32); 3],
}

impl Mound {
    fn new(centre: Vec2, radius: f32, height: f32, lobes: f32, rng: &mut PlantRng) -> Self {
        let lobes = RIM_WAVES.map(|(amp, freq)| {
            (amp * lobes * rng.range(WAVE_JITTER.0, WAVE_JITTER.1), freq, rng.range(0.0, std::f32::consts::TAU))
        });
        Self { centre, radius, height, lobes }
    }

    /// Rim radius at angle `a`.
    fn rim(&self, a: f32) -> f32 {
        let wobble: f32 = self.lobes.iter().map(|&(amp, f, ph)| amp * f.mul_add(a, ph).sin()).sum();
        self.radius * (1.0 + wobble).max(MIN_RIM)
    }

    /// The surface point at `rho` (0 centre, 1 rim) and angle `a`. The rim
    /// sits a little below the ground so a cushion never shows a gap on a
    /// slope.
    fn point(&self, rho: f32, a: f32) -> Vec3 {
        let flat = self.centre + Vec2::new(a.cos(), a.sin()) * (self.rim(a) * rho);
        // Flat-topped, then rounding steeply down to the rim.
        let y = if rho >= 1.0 {
            -RIM_SINK * self.height
        } else {
            (1.0 - rho.clamp(0.0, 1.0).powf(FLAT_TOP)).max(0.0).powf(SHOULDER) * self.height
        };
        Vec3::new(flat.x, y, flat.y)
    }
}

/// The mounds a cushion is built from: `lumps` overlapping shoot clumps,
/// the tallest near the middle, or one mound (`lumps` 0 or 1, and always
/// at the far tier). Where they meet they crease, and their dark rims
/// show as the hollows between clumps.
fn cluster(radius: f32, height: f32, lumps: u8, lobes: f32, lod: PlantLod, rng: &mut PlantRng) -> Vec<Mound> {
    let kept = match lod {
        PlantLod::Hero => lumps,
        PlantLod::Near => lumps.div_ceil(2),
        PlantLod::Far => 1,
    };
    // Every tier draws the same stream, so the mounds it keeps match.
    let all: Vec<Mound> = (0..lumps.max(1))
        .map(|i| {
            if lumps <= 1 {
                return Mound::new(Vec2::ZERO, radius, height, lobes, rng);
            }
            let a = rng.range(0.0, std::f32::consts::TAU);
            let d = if i == 0 { 0.0 } else { rng.range(LUMP_DISTANCE.0, LUMP_DISTANCE.1) };
            let r = radius * rng.range(LUMP_RADIUS.0, LUMP_RADIUS.1) * d.mul_add(-LUMP_TAPER, 1.0);
            let h = height * rng.range(LUMP_HEIGHT.0, LUMP_HEIGHT.1) * d.mul_add(-LUMP_SLOPE, LUMP_PEAK);
            Mound::new(Vec2::new(a.cos(), a.sin()) * (radius * d), r, h, lobes * LUMP_LOBES, rng)
        })
        .collect();
    if kept <= 1 && lumps > 1 {
        // One mound standing in for the cluster: its footprint, its height.
        let foot = all.iter().map(|m| m.radius.mul_add(STAND_IN_REACH, m.centre.length())).fold(0.0, f32::max);
        let h = all.iter().map(|m| m.height).fold(0.0, f32::max) * STAND_IN_HEIGHT;
        return vec![Mound::new(Vec2::ZERO, foot.min(radius), h, lobes, rng)];
    }
    all.into_iter().take(usize::from(kept.max(1))).collect()
}

/// Ring radii (fractions of the rim) from the rim in, per tier; the small
/// mounds of a cluster get fewer.
const fn rings(lod: PlantLod, small: bool) -> &'static [f32] {
    match (lod, small) {
        (PlantLod::Hero, false) => &[1.0, 0.82, 0.55, 0.25],
        (PlantLod::Near, false) => &[1.0, 0.65, 0.3],
        (PlantLod::Hero | PlantLod::Near, true) | (PlantLod::Far, _) => &[1.0, 0.5],
    }
}

/// One mound, its colour scaled so `top` is the cushion's highest point.
fn cushion(out: &mut MeshData, m: &Mound, rings: &[f32], sides: u32, top: f32, paint: &Paint, rng: &mut PlantRng) {
    let first = out.vertex_count();
    let add = |out: &mut MeshData, p: Vec3, rng: &mut PlantRng| {
        // Mound tops are lightest, hollows and the rim darkest: the shade a
        // mat of shoots casts on itself, which is most of moss's velvet.
        let t = (p.y / top).clamp(0.0, 1.0).powf(CUSHION_SHADE_SKEW);
        let k = rng.signed().mul_add(paint.jitter * CUSHION_JITTER, 1.0);
        let uv = p.xz() * paint.uv_per_m;
        out.vertex(p, Vec3::Y, uv, Vec4::new(t * CUSHION_SWAY, 1.0, 0.0, 0.0), Some(paint.at(t, 0.0, k)))
    };
    let ring_count = u32::try_from(rings.len()).unwrap_or(0);
    for &rho in rings {
        for s in 0..sides {
            let a = f32_from_u32(s) / f32_from_u32(sides) * std::f32::consts::TAU;
            add(out, m.point(rho, a), rng);
        }
    }
    let crown = add(out, m.point(0.0, 0.0), rng);
    for ring in 0..ring_count.saturating_sub(1) {
        let a0 = first.wrapping_add(ring.wrapping_mul(sides));
        let b0 = a0.wrapping_add(sides);
        for s in 0..sides {
            let s1 = s.wrapping_add(1).checked_rem(sides).unwrap_or(0);
            let (a, a1, b, b1) = (a0.wrapping_add(s), a0.wrapping_add(s1), b0.wrapping_add(s), b0.wrapping_add(s1));
            out.triangle(a, b, a1);
            out.triangle(a1, b, b1);
        }
    }
    let inner = first.wrapping_add(sides.wrapping_mul(ring_count.saturating_sub(1)));
    for s in 0..sides {
        let s1 = s.wrapping_add(1).checked_rem(sides).unwrap_or(0);
        out.triangle(inner.wrapping_add(s), crown, inner.wrapping_add(s1));
    }
    smooth_normals(out, first);
}

/// Area-weighted vertex normals for the vertices from `first` on, bent
/// towards +Y (see [`NORMAL_UP`]).
fn smooth_normals(out: &mut MeshData, first: u32) {
    let start = usize_from(first);
    let mut sum = vec![Vec3::ZERO; out.positions.len().saturating_sub(start)];
    let at = |i: u32| out.positions.get(usize_from(i)).copied().map_or(Vec3::ZERO, Vec3::from);
    for &[a, b, c] in out.indices.as_chunks::<3>().0 {
        if a < first || b < first || c < first {
            continue;
        }
        let face = (at(b) - at(a)).cross(at(c) - at(a));
        for i in [a, b, c] {
            if let Some(s) = sum.get_mut(usize_from(i.wrapping_sub(first))) {
                *s += face;
            }
        }
    }
    for (n, s) in out.normals.iter_mut().skip(start).zip(sum) {
        *n = s.normalize_or(Vec3::Y).lerp(Vec3::Y, NORMAL_UP).normalize_or(Vec3::Y).to_array();
    }
}

/// One shoot standing out of the cushion: a single thin triangle, leaning
/// out with the surface.
fn bristle(out: &mut MeshData, m: &Mound, height: f32, top: f32, paint: &Paint, rng: &mut PlantRng, phase: f32) {
    let rho = rng.unit().sqrt() * BRISTLE_SPAN;
    let a = rng.range(0.0, std::f32::consts::TAU);
    let root = m.point(rho, a) - Vec3::Y * (m.height * BRISTLE_ROOT_DEPTH);
    let out_dir = Vec3::new(a.cos(), 0.0, a.sin());
    let wander = Vec3::new(rng.signed(), 0.0, rng.signed()) * BRISTLE_WANDER;
    let up = (Vec3::Y + out_dir * (rho * rho * BRISTLE_SPLAY) + wander).normalize_or(Vec3::Y);
    let h = height * rng.range(BRISTLE_HEIGHT_JITTER.0, BRISTLE_HEIGHT_JITTER.1);
    let side = Vec3::new(rng.signed(), 0.0, rng.signed());
    let across = up.cross(side).normalize_or(Vec3::X) * (h * BRISTLE_WIDTH);
    let dry = if rng.chance(paint.dry) { 1.0 } else { 0.0 };
    let k = rng.signed().mul_add(paint.jitter, 1.0);
    let t0 = (root.y / top.max(1e-4)).clamp(0.0, 1.0);
    let normal = up.cross(across).normalize_or(Vec3::Y).lerp(Vec3::Y, BRISTLE_NORMAL_UP);
    let wind = Vec4::new(CUSHION_SWAY, 1.0, phase, 1.0);
    let uv = root.xz() * paint.uv_per_m;
    let a0 = out.vertex(root - across, normal, uv, wind, Some(paint.at(t0, dry, k)));
    let a1 = out.vertex(root + across, normal, uv, wind, Some(paint.at(t0, dry, k)));
    let tip = out.vertex(root + up * h, normal, uv, wind, Some(paint.at(1.0, dry, k * BRISTLE_TIP_LIGHT)));
    out.triangle(a0, a1, tip);
}

/// Texels across the detail texture.
const DETAIL_TEXELS: u32 = 128;
/// Speckles (shoot tips) across it.
const DETAIL_CELLS: u32 = 16;
/// A shoot tip's spot fades to the floor this far from its centre, in
/// cells...
const SPOT_RADIUS: f32 = 0.8;
/// ...as `(1 - d / SPOT_RADIUS)^SPOT_FALLOFF`: under 1, a soft round spot
/// with a firm edge.
const SPOT_FALLOFF: f32 = 0.8;
/// The dimmest shoot tip's brightness; the brightest is 1.
const MIN_TIP_BRIGHTNESS: f32 = 0.5;
/// Mixed into the seed (by xor) for the hash of each tip's second jitter
/// axis, apart from its first.
const JITTER_Y_SALT: u64 = 0x5eed;
/// Mixed into the seed (by xor) for the hash of each tip's brightness.
const BRIGHTNESS_SALT: u64 = 0xb417;
/// Anisotropic filtering samples, so the speckle stays sharp at grazing
/// angles.
const DETAIL_ANISOTROPY: u16 = 4;

/// A tiling speckle of shoot tips for a cover's material, with `strength`
/// contrast (0..1), and its mean brightness.
///
/// Each tip is a soft round spot of its own brightness on a dark floor
/// (Worley noise), like the packed leafy tips of a moss cushion seen from
/// above. Mipmapped by box filter, so it fades to the mean, not to
/// shimmer, with distance.
#[must_use]
pub fn detail_texture(strength: f32, seed: u64) -> (Image, f32) {
    let levels = mip_chain(speckle(strength, seed));
    let mean = levels.last().and_then(|l| l.first()).copied().unwrap_or(1.0);
    let bytes: Vec<u8> = levels
        .iter()
        .flatten()
        .flat_map(|&v| {
            let g = u8::try_from(u32_from_f32((v.clamp(0.0, 1.0) * 255.0).round())).unwrap_or(u8::MAX);
            [g, g, g, u8::MAX]
        })
        .collect();
    let size = Extent3d { width: DETAIL_TEXELS, height: DETAIL_TEXELS, depth_or_array_layers: 1 };
    let mut image = Image::new_uninit(size, TextureDimension::D2, TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
    image.texture_descriptor.mip_level_count = u32::try_from(levels.len()).unwrap_or(1);
    image.data = Some(bytes);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        anisotropy_clamp: DETAIL_ANISOTROPY,
        ..ImageSamplerDescriptor::linear()
    });
    (image, mean)
}

/// The speckle at full size: [`DETAIL_TEXELS`] rows of as many
/// brightnesses, `1 - strength` between the tips.
fn speckle(strength: f32, seed: u64) -> Vec<f32> {
    let floor = 1.0 - strength.clamp(0.0, 1.0);
    let cells = DETAIL_CELLS;
    let texel = f32_from_u32(cells) / f32_from_u32(DETAIL_TEXELS);
    let point = |cx: u32, cy: u32| {
        let i = u64::from(cy.wrapping_mul(cells).wrapping_add(cx));
        let jitter = Vec2::new(hash_unit(seed, i), hash_unit(seed ^ JITTER_Y_SALT, i));
        (jitter, hash_unit(seed ^ BRIGHTNESS_SALT, i).mul_add(1.0 - MIN_TIP_BRIGHTNESS, MIN_TIP_BRIGHTNESS))
    };
    let mut level: Vec<f32> = Vec::with_capacity(usize_from(DETAIL_TEXELS.wrapping_mul(DETAIL_TEXELS)));
    for y in 0..DETAIL_TEXELS {
        for x in 0..DETAIL_TEXELS {
            let p = Vec2::new(f32_from_u32(x) + 0.5, f32_from_u32(y) + 0.5) * texel;
            let (cx, cy) = (u32_from_f32(p.x), u32_from_f32(p.y));
            let mut best = (f32::MAX, 0.0);
            for dy in 0..3u32 {
                for dx in 0..3u32 {
                    // Neighbours wrap, so the texture tiles.
                    let nx = cx.wrapping_add(cells).wrapping_add(dx).wrapping_sub(1);
                    let ny = cy.wrapping_add(cells).wrapping_add(dy).wrapping_sub(1);
                    let (jitter, bright) = point(nx.checked_rem(cells).unwrap_or(0), ny.checked_rem(cells).unwrap_or(0));
                    let at = Vec2::new(f32_from_u32(nx), f32_from_u32(ny)) - f32_from_u32(cells) + jitter;
                    let d = p.distance(at);
                    if d < best.0 {
                        best = (d, bright);
                    }
                }
            }
            let spot = (1.0 - best.0 / SPOT_RADIUS).max(0.0).powf(SPOT_FALLOFF) * best.1;
            level.push((1.0 - floor).mul_add(spot, floor));
        }
    }
    level
}

/// `level` ([`DETAIL_TEXELS`] across) and every mip under it, each the
/// mean of four texels of the one above, down to a single texel.
fn mip_chain(level: Vec<f32>) -> Vec<Vec<f32>> {
    let mut levels = vec![level];
    let mut size = DETAIL_TEXELS;
    while size > 1 {
        let half = size / 2;
        let Some(prev) = levels.last() else { break };
        let at = |x: u32, y: u32| prev.get(usize_from(y.wrapping_mul(size).wrapping_add(x))).copied().unwrap_or(0.0);
        let next = (0..half.wrapping_mul(half))
            .map(|i| {
                let (x, y) = (i.checked_rem(half).unwrap_or(0).wrapping_mul(2), i.checked_div(half).unwrap_or(0).wrapping_mul(2));
                (at(x, y) + at(x.wrapping_add(1), y) + at(x, y.wrapping_add(1)) + at(x.wrapping_add(1), y.wrapping_add(1))) * 0.25
            })
            .collect();
        levels.push(next);
        size = half;
    }
    levels
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::num::f32_from_usize;

    fn faces_up(m: &MeshData) -> bool {
        m.indices.chunks(3).all(|t| {
            let p = |i: u32| Vec3::from(m.positions[usize_from(i)]);
            let n = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
            let centre = (p(t[0]) + p(t[1]) + p(t[2])) / 3.0;
            n.dot(centre.with_y(centre.y + 0.01)) > -1e-6
        })
    }

    fn cover(tuft: Tuft) -> Cover {
        Cover {
            tuft,
            color: [0.2, 0.4, 0.1],
            shade: 0.3,
            color_jitter: 0.1,
            tip_color: None,
            dry: 0.2,
            dry_color: [0.6, 0.5, 0.3],
            detail: 0.5,
            detail_size: 0.005,
        }
    }

    fn cushion(lumps: u8, bristles: u16) -> Cover {
        cover(Tuft::Cushion { radius: 0.1, height: 0.04, sides: 8, lumps, lobes: 0.0, bristles, bristle_height: 0.4 })
    }

    fn grass(count: u8, segments: u8, height_jitter: f32) -> Cover {
        cover(Tuft::Blades { count, height: 0.3, width: 0.02, lean: 0.3, segments, spread: 0.25, height_jitter, splay: 0.2 })
    }

    #[test]
    fn cushion_is_closed_and_outward() {
        let m = tuft(&cushion(0, 0), PlantLod::Hero, 1);
        assert!(m.indices.iter().all(|&i| i < m.vertex_count()));
        assert!(faces_up(&m));
        assert!(m.normals.iter().all(|n| n[1] > 0.0), "cushion normals point up and out");
    }

    #[test]
    fn lumpy_cushions_stay_in_bounds() {
        let c = cover(Tuft::Cushion { radius: 0.1, height: 0.04, sides: 10, lumps: 6, lobes: 0.4, bristles: 20, bristle_height: 0.5 });
        for seed in 0..20 {
            let m = tuft(&c, PlantLod::Hero, seed);
            assert!(m.indices.iter().all(|&i| i < m.vertex_count()));
            for p in &m.positions {
                assert!(p.iter().all(|v| v.is_finite()));
                assert!(Vec2::new(p[0], p[2]).length() < 0.2 && p[1] < 0.2 && p[1] > -0.02, "{p:?}");
            }
        }
    }

    #[test]
    fn far_cover_is_cheaper() {
        let c = grass(12, 4, 0.3);
        let hero = tuft(&c, PlantLod::Hero, 1);
        let far = tuft(&c, PlantLod::Far, 1);
        assert!(far.indices.len() * 4 <= hero.indices.len());
        assert!(hero.indices.iter().all(|&i| i < hero.vertex_count()));
        let moss = cushion(5, 24);
        let [h, n, f] = PlantLod::ALL.map(|lod| tuft(&moss, lod, 1).indices.len());
        assert!(f < n && n < h, "{h} {n} {f}");
    }

    #[test]
    fn detail_texture_is_a_full_mip_chain() {
        let (image, mean) = detail_texture(0.6, 1);
        let texels: u32 = (0..image.texture_descriptor.mip_level_count).map(|l| (DETAIL_TEXELS >> l).pow(2)).sum();
        assert_eq!(image.data.as_ref().map(Vec::len), Some(usize_from(texels) * 4));
        assert!(mean > 0.4 && mean < 1.0, "{mean}");
    }

    #[test]
    fn mips_average_down_to_one_texel() {
        // A checkerboard: every mip under the top is mid-grey.
        let n = DETAIL_TEXELS;
        let level = (0..n * n).map(|i| if (i % n + i / n) % 2 == 0 { 1.0 } else { 0.0 }).collect();
        let levels = mip_chain(level);
        assert_eq!(levels.len(), 8);
        assert_eq!(levels.last().map(Vec::len), Some(1));
        for mip in levels.iter().skip(1) {
            assert!(mip.iter().all(|v| (v - 0.5).abs() < 1e-6));
        }
    }

    #[test]
    fn grass_darkens_at_the_root() {
        let m = tuft(&grass(6, 3, 0.0), PlantLod::Hero, 3);
        let luma = |c: &[f32; 4]| c[0] + c[1] + c[2];
        let mean = |keep: &dyn Fn(f32) -> bool| {
            let picked: Vec<f32> = m.positions.iter().zip(&m.colors).filter(|(p, _)| keep(p[1])).map(|(_, c)| luma(c)).collect();
            picked.iter().sum::<f32>() / f32_from_usize(picked.len().max(1))
        };
        assert!(mean(&|y| y < 0.03) < mean(&|y| y > 0.15));
    }
}
