//! Space colonization (Runions, Lane & Prusinkiewicz, "Modeling Trees with a
//! Space Colonization Algorithm", 2007).
//!
//! Attraction points fill the crown's envelope. Each iteration every point
//! pulls on its nearest node within `influence`; each pulled node grows one
//! `step` towards the mean of its points; points within `kill` of a node are
//! used up. The crown is just the envelope, so any shape works, and
//! branches compete for space without any branching rule.
//!
//! Cost: nodes live in a uniform grid of `influence`-sized cells, so a point
//! checks 27 cells instead of every node. Grow coarse (a large `step`) and
//! let the mesher smooth it; a few thousand points grow in milliseconds.

use std::collections::HashMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::num::{i32_from_f32, u32_from_usize, usize_from};
use crate::random::PlantRng;
use crate::skeleton::PlantSkeleton;

/// How a space-colonized plant grows: its crown's shape and how hard its
/// branches compete for the space in it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Colonize {
    /// The crown's shape: attraction points fill it.
    pub envelope: Envelope,
    /// Attraction points scattered in the envelope.
    pub points: u32,
    /// How far a point reaches for a node, metres.
    pub influence: f32,
    /// How close a node must come to use a point up, metres.
    pub kill: f32,
    /// Node spacing, metres.
    pub step: f32,
    /// Growth stops after this many rounds, or once no node grew.
    pub max_iterations: u32,
    /// Added to every growth direction (+Y reaches up, -Y droops).
    pub tropism: Vec3,
    /// Random bend added to the trunk while it climbs to the crown.
    pub trunk_wander: f32,
    /// How much a new node keeps its parent's heading, against the pull
    /// of its points. Plain space colonization (0) turns freely, so limbs
    /// zigzag and twigs hook back on themselves; real shoots carry on
    /// roughly straight.
    pub inertia: f32,
    /// Passes of smoothing over the grown skeleton: each moves every inner
    /// node halfway towards the midpoint of its neighbours along its axis,
    /// taking out the step-by-step kinks.
    pub smooth: u8,
}

impl Default for Colonize {
    fn default() -> Self {
        Self {
            envelope: Envelope::Ellipsoid { center: Vec3::new(0.0, 5.0, 0.0), radii: Vec3::new(3.0, 2.5, 3.0) },
            points: 1500,
            influence: 2.0,
            kill: 0.5,
            step: 0.25,
            max_iterations: 200,
            tropism: Vec3::new(0.0, 0.1, 0.0),
            trunk_wander: 0.08,
            inertia: 0.0,
            smooth: 0,
        }
    }
}

/// The volume a crown fills.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Envelope {
    /// A round crown: broadleaf trees and bushes.
    Ellipsoid {
        /// Plant space, metres.
        center: Vec3,
        /// Half-widths along x, y and z, metres.
        radii: Vec3,
    },
    /// Apex up, base at `base` metres high: conifers.
    Cone {
        /// Height of the widest point, metres.
        base: f32,
        /// From the base up to the apex, metres.
        height: f32,
        /// At the base, metres.
        radius: f32,
    },
    /// Wide at the top: vase-shaped shrubs, elms.
    Vase {
        /// Height of the narrow point, metres.
        base: f32,
        /// From the base up to the rim, metres.
        height: f32,
        /// At the rim, metres.
        radius: f32,
    },
}

impl Envelope {
    fn bounds(self) -> (Vec3, Vec3) {
        match self {
            Self::Ellipsoid { center, radii } => (center - radii, center + radii),
            Self::Cone { base, height, radius } | Self::Vase { base, height, radius } => {
                (Vec3::new(-radius, base, -radius), Vec3::new(radius, base + height, radius))
            }
        }
    }

    /// Whether `p` (plant space, metres) is inside.
    #[must_use]
    pub fn contains(self, p: Vec3) -> bool {
        match self {
            Self::Ellipsoid { center, radii } => ((p - center) / radii.max(Vec3::splat(1e-4))).length_squared() <= 1.0,
            Self::Cone { base, height, radius } => {
                let t = (p.y - base) / height.max(1e-4);
                (0.0..=1.0).contains(&t) && p.xz().length() <= radius * (1.0 - t)
            }
            Self::Vase { base, height, radius } => {
                let t = (p.y - base) / height.max(1e-4);
                (0.0..=1.0).contains(&t) && p.xz().length() <= radius * t.sqrt()
            }
        }
    }

    /// The lowest point of the envelope on its axis, where the trunk aims.
    fn foot(self) -> Vec3 {
        let (lo, hi) = self.bounds();
        Vec3::new(lo.x.midpoint(hi.x), lo.y, lo.z.midpoint(hi.z))
    }

    fn sample(self, rng: &mut PlantRng) -> Option<Vec3> {
        let (lo, hi) = self.bounds();
        (0..SAMPLE_TRIES).find_map(|_| {
            let p = Vec3::new(rng.range(lo.x, hi.x), rng.range(lo.y, hi.y), rng.range(lo.z, hi.z));
            self.contains(p).then_some(p)
        })
    }
}

/// Nodes bucketed by cell for nearest-node queries.
struct Grid {
    cell: f32,
    cells: HashMap<IVec3, Vec<u32>>,
}

impl Grid {
    fn key(&self, p: Vec3) -> IVec3 {
        let c = p / self.cell;
        IVec3::new(i32_from_f32(c.x.floor()), i32_from_f32(c.y.floor()), i32_from_f32(c.z.floor()))
    }

    fn insert(&mut self, i: u32, p: Vec3) {
        self.cells.entry(self.key(p)).or_default().push(i);
    }

    /// The nearest node within `self.cell` of `p`: (index, distance).
    fn nearest(&self, p: Vec3, nodes: &PlantSkeleton) -> Option<(u32, f32)> {
        let k = self.key(p);
        let mut best: Option<(u32, f32)> = None;
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let Some(list) = self.cells.get(&k.saturating_add(IVec3::new(dx, dy, dz))) else { continue };
                    for &i in list {
                        let Some(n) = nodes.node(i) else { continue };
                        let d = n.pos.distance(p);
                        let better = best.is_none_or(|(bi, bd)| d.total_cmp(&bd).then(i.cmp(&bi)).is_lt());
                        if d <= self.cell && better {
                            best = Some((i, d));
                        }
                    }
                }
            }
        }
        best
    }
}

/// The finest node spacing `step` can ask for, metres.
const MIN_STEP: f32 = 0.01;
/// A point reaches at least this many steps, so a node always has room to
/// be pulled.
const MIN_INFLUENCE_STEPS: f32 = 2.0;
/// The trunk climbs at most this many times the steps up to the crown's
/// foot...
const CLIMB_SLACK: f32 = 2.0;
/// ...plus this many, before it stops waiting for the crown to pull it.
const CLIMB_EXTRA: f32 = 4.0;
/// A node isn't grown closer than this many steps to an existing node.
const MIN_NODE_GAP: f32 = 0.5;
/// Rejection tries to find a point inside the envelope before that point
/// is given up.
const SAMPLE_TRIES: usize = 64;

/// The distances growth works at, metres, made consistent: `step` is at
/// least [`MIN_STEP`], `influence` at least [`MIN_INFLUENCE_STEPS`] steps,
/// `kill` between the two.
struct Reach {
    step: f32,
    influence: f32,
    kill: f32,
}

impl Colonize {
    /// The skeleton for one variant, branch orders assigned and radii left
    /// at 0.
    #[must_use]
    pub fn grow(&self, rng: &mut PlantRng) -> PlantSkeleton {
        let step = self.step.max(MIN_STEP);
        let influence = self.influence.max(step * MIN_INFLUENCE_STEPS);
        let reach = Reach { step, influence, kill: self.kill.clamp(step, influence) };
        let mut points: Vec<Vec3> = (0..self.points).filter_map(|_| self.envelope.sample(rng)).collect();

        let mut skel = PlantSkeleton::rooted_at(Vec3::ZERO);
        let mut grid = Grid { cell: influence, cells: HashMap::new() };
        grid.insert(0, Vec3::ZERO);
        self.climb(&mut skel, &mut grid, &points, &reach, rng);
        for _ in 0..self.max_iterations {
            if points.is_empty() || !self.grow_once(&mut skel, &mut grid, &mut points, &reach) {
                break;
            }
        }
        smooth(&mut skel, self.smooth);
        skel.assign_orders();
        skel
    }

    /// The trunk climbs towards the envelope until the crown can pull it.
    fn climb(&self, skel: &mut PlantSkeleton, grid: &mut Grid, points: &[Vec3], reach: &Reach, rng: &mut PlantRng) {
        let step = reach.step;
        let foot = self.envelope.foot();
        let (mut tip, mut tip_pos) = (0u32, Vec3::ZERO);
        let climb_limit = (foot.y / step).ceil().max(0.0).mul_add(CLIMB_SLACK, CLIMB_EXTRA);
        let mut climbed = 0.0;
        while climbed < climb_limit && !points.iter().any(|p| p.distance(tip_pos) <= reach.influence) {
            let aim = (foot + Vec3::Y * step - tip_pos).normalize_or(Vec3::Y);
            let wander = Vec3::new(rng.signed(), 0.0, rng.signed()) * self.trunk_wander;
            tip_pos += (aim + wander).normalize_or(Vec3::Y) * step;
            tip = skel.push(tip, tip_pos, 0);
            grid.insert(tip, tip_pos);
            climbed += 1.0;
        }
    }

    /// One round of growth: every point pulls its nearest node, points
    /// within `kill` of a node are used up, and every pulled node grows a
    /// step towards its points. Whether any node grew.
    fn grow_once(&self, skel: &mut PlantSkeleton, grid: &mut Grid, points: &mut Vec<Vec3>, reach: &Reach) -> bool {
        let mut pull = vec![Vec3::ZERO; skel.nodes.len()];
        points.retain(|&p| match grid.nearest(p, skel) {
            Some((_, d)) if d <= reach.kill => false,
            Some((i, _)) => {
                if let (Some(acc), Some(n)) = (pull.get_mut(usize_from(i)), skel.node(i)) {
                    *acc += (p - n.pos).normalize_or_zero();
                }
                true
            }
            None => true,
        });

        let mut grew = false;
        for (i, acc) in pull.into_iter().enumerate() {
            if acc.length_squared() < 1e-8 {
                continue;
            }
            let i = u32_from_usize(i);
            let Some(node) = skel.node(i) else { continue };
            let from = node.pos;
            let heading = node.parent.and_then(|p| skel.node(p)).map_or(Vec3::ZERO, |p| (from - p.pos).normalize_or_zero());
            let dir = (acc.normalize() + heading * self.inertia + self.tropism).normalize_or_zero();
            if dir == Vec3::ZERO {
                continue;
            }
            let pos = from + dir * reach.step;
            // A node pulled from both sides by points it can't reach
            // would step back and forth into a knot of tiny twigs (and
            // the pipe model would then thicken the whole limb under
            // it). Don't grow onto an existing node.
            if grid.nearest(pos, skel).is_some_and(|(_, d)| d < reach.step * MIN_NODE_GAP) {
                continue;
            }
            let new = skel.push(i, pos, 0);
            grid.insert(new, pos);
            grew = true;
        }
        grew
    }
}

/// `passes` of Laplacian smoothing along each axis; the root stays put.
fn smooth(skel: &mut PlantSkeleton, passes: u8) {
    if passes == 0 {
        return;
    }
    let cont = skel.continuations();
    for _ in 0..passes {
        let moved: Vec<Vec3> = skel
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let parent = n.parent.and_then(|p| skel.node(p));
                let next = cont.get(i).copied().flatten().and_then(|c| skel.node(c));
                match (parent, next) {
                    (Some(p), Some(c)) => n.pos.lerp(p.pos.midpoint(c.pos), 0.5),
                    _ => n.pos,
                }
            })
            .collect();
        for (n, p) in skel.nodes.iter_mut().zip(moved) {
            n.pos = p;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_into_the_crown_deterministically() {
        let c = Colonize { points: 400, ..Colonize::default() };
        let a = c.grow(&mut PlantRng::new(9));
        assert_eq!(a, c.grow(&mut PlantRng::new(9)));
        let (lo, hi) = a.bounds();
        assert!(a.nodes.len() > 50, "{} nodes", a.nodes.len());
        assert!(hi.y > 5.0 && lo.y >= -0.01, "{lo} {hi}");
        // More than one branch: the crown split the trunk.
        assert!(a.chains().len() > 5);
    }

    #[test]
    fn inertia_and_smoothing_straighten_limbs() {
        // Mean turn between a segment and the next along each axis.
        let kink = |s: &PlantSkeleton| {
            let cont = s.continuations();
            let turns: Vec<f32> = s
                .nodes
                .iter()
                .enumerate()
                .filter_map(|(i, n)| {
                    let p = s.node(n.parent?)?;
                    let c = s.node(cont.get(i).copied().flatten()?)?;
                    Some((n.pos - p.pos).angle_between(c.pos - n.pos))
                })
                .collect();
            turns.iter().sum::<f32>() / crate::num::f32_from_usize(turns.len().max(1))
        };
        let plain = Colonize { points: 600, ..Colonize::default() };
        let a = plain.grow(&mut PlantRng::new(4));
        let b = Colonize { inertia: 0.5, smooth: 2, ..plain }.grow(&mut PlantRng::new(4));
        assert!(kink(&b) < kink(&a), "{} vs {}", kink(&b), kink(&a));
    }

    #[test]
    fn a_cone_stays_narrow_at_the_top() {
        let c = Colonize {
            envelope: Envelope::Cone { base: 1.0, height: 8.0, radius: 2.0 },
            points: 600,
            ..Colonize::default()
        };
        let s = c.grow(&mut PlantRng::new(2));
        let top: Vec<_> = s.nodes.iter().filter(|n| n.pos.y > 7.5).collect();
        assert!(top.iter().all(|n| n.pos.xz().length() < 1.2));
    }
}
