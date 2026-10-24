//! Recursive parametric trees after Weber & Penn ("Creation and Rendering
//! of Realistic Trees", SIGGRAPH 1995), the model behind Blender's Sapling
//! and most game tree tools.
//!
//! A straight trunk carries `levels[0]` children, each child carries
//! `levels[1]` children, and so on. Every level is a dozen numbers: how
//! many children and where, at what angle, how long (a length profile
//! along the parent: conical for conifers, spherical for round crowns), how
//! they bend. Children can share a node in whorls, so a spruce's tiers of
//! near-horizontal branches and its flat, drooping sprays come from the
//! table rather than from competition.
//!
//! Cost: linear in the nodes it makes, and the table says how many that is
//! before growing; `max_nodes` caps it.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::num::{f32_from_u32, f32_from_usize, u32_from_f32, usize_from};
use crate::random::PlantRng;
use crate::skeleton::PlantSkeleton;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Recursive {
    /// Trunk length, metres. The trunk is the leader: it runs straight to
    /// the top and every branch is shorter than it (apical dominance).
    pub height: f32,
    /// Trunk node spacing, metres.
    pub step: f32,
    /// Random bend of the trunk per step, radians.
    pub wander: f32,
    /// Children of the trunk, then their children, and so on.
    pub levels: Vec<Level>,
    /// Growth stops adding axes past this many nodes.
    pub max_nodes: u32,
}

impl Default for Recursive {
    fn default() -> Self {
        Self {
            height: 8.0,
            step: 0.3,
            wander: 0.02,
            levels: vec![
                Level { count: 40, whorl: 4, ..Level::default() },
                Level { count: 6, angle: (50.0, 40.0), length: 0.4, shape: Shape::Tapered, ..Level::default() },
            ],
            max_nodes: 8000,
        }
    }
}

/// One level of branching: how the children of every axis one level up
/// are placed and shaped.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Level {
    /// Children on each parent axis.
    pub count: u16,
    /// Children sharing a node: 1 alternates up the parent, 3 to 7 make
    /// whorls (pines, spruces, firs).
    pub whorl: u8,
    /// Where children grow along the parent, 0 (base) to 1 (tip).
    pub span: (f32, f32),
    /// Angle from the parent axis, degrees, at the start and the end of the
    /// span. Over 90 points the lowest branches down.
    pub angle: (f32, f32),
    /// The longest child, as a fraction of its parent's length.
    pub length: f32,
    /// Child length along the span.
    pub shape: Shape,
    /// Node spacing, metres.
    pub step: f32,
    /// Bend over the first and the second half of each child, degrees;
    /// positive bends up. A sagging branch with an upturned tip is
    /// `curve: -30, curve_back: 40`.
    pub curve: f32,
    pub curve_back: f32,
    /// Random bend per step, radians.
    pub wander: f32,
    /// Random spread of angles and lengths, 0..1.
    pub jitter: f32,
    /// Turn around the parent between one child (or whorl) and the next,
    /// degrees. 137.5 spirals; 180 alternates left and right.
    pub roll: f32,
    /// 0 spreads children all around the parent, 1 keeps them to its two
    /// flanks: the flat sprays of conifers and of many shrubs.
    pub planar: f32,
}

impl Default for Level {
    fn default() -> Self {
        Self {
            count: 20,
            whorl: 1,
            span: (0.15, 1.0),
            angle: (80.0, 50.0),
            length: 0.3,
            shape: Shape::Conical,
            step: 0.25,
            curve: 0.0,
            curve_back: 0.0,
            wander: 0.04,
            jitter: 0.2,
            roll: 137.5,
            planar: 0.0,
        }
    }
}

/// Child length along the parent's span (`u` 0 at the span's start, 1 at
/// its end); Weber & Penn's shapes.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    /// Longest at the base: conifers.
    #[default]
    Conical,
    /// Longest in the middle: round broadleaf crowns.
    Spherical,
    /// Longest low, rounding off to the top.
    Hemispherical,
    /// Longest at 70%: a flame.
    Flame,
    /// From full at the base to half at the end.
    Tapered,
    Constant,
}

impl Shape {
    fn ratio(self, u: f32) -> f32 {
        let u = u.clamp(0.0, 1.0);
        match self {
            Self::Conical => (1.0 - u).mul_add(0.9, 0.1),
            Self::Spherical => (std::f32::consts::PI * u).sin().mul_add(0.8, 0.2),
            Self::Hemispherical => (std::f32::consts::FRAC_PI_2 * u).cos().mul_add(0.8, 0.2),
            Self::Flame => {
                if u < 0.7 {
                    u / 0.7
                } else {
                    (1.0 - u) / 0.3
                }
            }
            Self::Tapered => u.mul_add(-0.5, 1.0),
            Self::Constant => 1.0,
        }
    }
}

/// An axis waiting for its children: its nodes (the first is where it
/// leaves its parent), its length, and its level.
struct Axis {
    nodes: Vec<u32>,
    length: f32,
    level: usize,
}

/// How one axis grows from its first node.
struct Shoot {
    dir: Vec3,
    length: f32,
    step: f32,
    curve: f32,
    curve_back: f32,
    wander: f32,
    order: u8,
}

impl Recursive {
    #[must_use]
    pub fn grow(&self, rng: &mut PlantRng) -> PlantSkeleton {
        let mut skel = PlantSkeleton::rooted_at(Vec3::ZERO);
        let trunk = Shoot {
            dir: Vec3::Y,
            length: self.height.max(0.1),
            step: self.step,
            curve: 0.0,
            curve_back: 0.0,
            wander: self.wander,
            order: 0,
        };
        let nodes = grow_axis(&mut skel, 0, &trunk, rng);
        // Breadth first, so a node cap trims the finest level evenly
        // rather than leaving one side of the tree bare.
        let mut queue = std::collections::VecDeque::from([Axis { nodes, length: trunk.length, level: 0 }]);
        while let Some(axis) = queue.pop_front() {
            let Some(level) = self.levels.get(axis.level) else { continue };
            for child in children(&skel, &axis, level, rng) {
                if u32::try_from(skel.nodes.len()).unwrap_or(u32::MAX) >= self.max_nodes {
                    return skel;
                }
                let (from, shoot) = child;
                let nodes = grow_axis(&mut skel, from, &shoot, rng);
                queue.push_back(Axis { nodes, length: shoot.length, level: axis.level.saturating_add(1) });
            }
        }
        skel
    }
}

/// Where `axis`'s children start and how each grows.
fn children(skel: &PlantSkeleton, axis: &Axis, level: &Level, rng: &mut PlantRng) -> Vec<(u32, Shoot)> {
    let mut out = Vec::new();
    let last = axis.nodes.len().saturating_sub(1);
    if last == 0 || level.count == 0 {
        return out;
    }
    let whorl = u32::from(level.whorl.max(1));
    let groups = u32::from(level.count).div_ceil(whorl);
    let mut roll = rng.range(0.0, std::f32::consts::TAU);
    let order = u8::try_from(axis.level.saturating_add(1)).unwrap_or(u8::MAX);
    let flatten = 1.0 - level.planar.clamp(0.0, 1.0);
    for group in 0..groups {
        // How far through the span this group sits, then along the axis.
        let frac = (f32_from_u32(group) + 0.5) / f32_from_u32(groups);
        let along_axis = (level.span.1 - level.span.0).mul_add(frac, level.span.0);
        // Nodes 1..=last are the axis's own (node 0 is its parent's).
        let slot = usize_from(u32_from_f32((along_axis * f32_from_usize(last)).round())).clamp(1, last);
        let (Some(&at), Some(&before)) = (axis.nodes.get(slot), axis.nodes.get(slot.saturating_sub(1))) else { continue };
        let (Some(here), Some(prev)) = (skel.node(at), skel.node(before)) else { continue };
        let heading = (here.pos - prev.pos).normalize_or(Vec3::Y);
        let side = heading.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
        let up = side.cross(heading);
        for member in 0..whorl {
            let az = std::f32::consts::TAU.mul_add(f32_from_u32(member) / f32_from_u32(whorl), roll);
            // Planar: pull the azimuth towards the nearer flank (0 or pi).
            let flank = if az.cos() >= 0.0 { 0.0 } else { std::f32::consts::PI };
            let wrapped = (az - flank + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            let az = wrapped.mul_add(flatten, flank);
            let spread = level.jitter * rng.signed();
            let angle = (level.angle.1 - level.angle.0).mul_add(frac, level.angle.0) * spread.mul_add(0.25, 1.0);
            let perp = side * az.cos() + up * az.sin();
            let (sin, cos) = angle.to_radians().sin_cos();
            let dir = (heading * cos + perp * sin).normalize_or(perp);
            let length = axis.length * level.length * level.shape.ratio(frac) * level.jitter.mul_add(rng.signed() * 0.5, 1.0);
            if length < level.step * 0.5 {
                continue;
            }
            out.push((
                at,
                Shoot {
                    dir,
                    length,
                    step: level.step,
                    curve: level.curve,
                    curve_back: level.curve_back,
                    wander: level.wander,
                    order,
                },
            ));
        }
        roll += level.roll.to_radians();
    }
    out
}

/// Grows `shoot` from node `from`; returns its nodes, `from` first.
fn grow_axis(skel: &mut PlantSkeleton, from: u32, shoot: &Shoot, rng: &mut PlantRng) -> Vec<u32> {
    let step = shoot.step.max(0.02);
    let segments = u32_from_f32((shoot.length / step).ceil()).max(1);
    let seg = shoot.length / f32_from_u32(segments);
    let half = segments.div_ceil(2);
    let (front, back) = (
        shoot.curve.to_radians() / f32_from_u32(half),
        shoot.curve_back.to_radians() / f32_from_u32(segments.saturating_sub(half).max(1)),
    );
    let mut nodes = vec![from];
    let (mut at, mut pos, mut dir) = (from, skel.node(from).map_or(Vec3::ZERO, |n| n.pos), shoot.dir);
    for k in 0..segments {
        // Bend in the vertical plane through the axis: about
        // `dir x Y`, which turns `dir` towards +Y for positive angles.
        if let Some(axis) = dir.cross(Vec3::Y).try_normalize() {
            dir = Quat::from_axis_angle(axis, if k < half { front } else { back }) * dir;
        }
        let w = Vec3::new(rng.signed(), rng.signed(), rng.signed()) * shoot.wander;
        dir = (dir + w - dir * w.dot(dir)).normalize_or(dir);
        pos += dir * seg;
        at = skel.push(at, pos, shoot.order);
        nodes.push(at);
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_whorls_deterministically() {
        let r = Recursive::default();
        let a = r.grow(&mut PlantRng::new(3));
        assert_eq!(a, r.grow(&mut PlantRng::new(3)));
        assert!(a.nodes.len() > 100, "{} nodes", a.nodes.len());
        // Every parent comes before its child.
        assert!(a.nodes.iter().enumerate().all(|(i, n)| n.parent.is_none_or(|p| usize_from(p) < i)));
        // The trunk is the tallest axis and stays near the middle.
        let top = a.nodes.iter().filter(|n| n.order == 0).map(|n| n.pos.y).fold(0.0, f32::max);
        assert!(a.nodes.iter().all(|n| n.pos.y <= top + 0.5));
        let firsts = a.nodes.iter().filter(|n| n.order == 1 && n.parent.and_then(|p| a.node(p)).is_some_and(|q| q.order == 0)).count();
        assert!(firsts >= 4, "{firsts} branches");
    }

    #[test]
    fn caps_its_nodes() {
        let r = Recursive { max_nodes: 300, ..Recursive::default() };
        assert!(r.grow(&mut PlantRng::new(1)).nodes.len() <= 330);
    }

    #[test]
    fn conical_crowns_narrow_upwards() {
        let r = Recursive { height: 10.0, ..Recursive::default() };
        let s = r.grow(&mut PlantRng::new(5));
        let width = |lo: f32, hi: f32| {
            s.nodes.iter().filter(|n| (lo..hi).contains(&n.pos.y)).map(|n| n.pos.xz().length()).fold(0.0, f32::max)
        };
        assert!(width(2.0, 4.0) > width(8.0, 10.0));
    }
}
