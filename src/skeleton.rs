//! The one intermediate form every structural grower writes and every later
//! stage reads: a tree of nodes (position, radius, parent, branch order)
//! plus the organs hung on it.
//!
//! A new way of growing plants is one function that returns a
//! [`PlantSkeleton`]; pipe-model radii, branch orders, leaves, meshing, wind
//! weights and LOD all come for free.
//!
//! Invariant: a node's parent has a smaller index than the node, so one pass
//! in index order visits parents first and one pass in reverse visits
//! children first.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::num::{u32_from_usize, usize_from};

/// One point on an axis. The segment to its parent is a piece of stem.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    /// Plant space, metres; the root sits at the origin, +Y up.
    pub pos: Vec3,
    /// Stem radius here, metres. Growers may leave 0 and call
    /// [`PlantSkeleton::pipe_radii`].
    pub radius: f32,
    /// The node this one grows from; `None` only at the root.
    pub parent: Option<u32>,
    /// 0 = trunk (or main stem), 1 = its branches, ...
    pub order: u8,
}

/// What an [`Organ`] is. Only leaves are meshed so far.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OrganKind {
    /// Meshed by [`crate::mesh::leaves`].
    #[default]
    Leaf,
    /// A flower.
    Flower,
    /// A fruit or seed head.
    Fruit,
}

/// Something hung on a node: a leaf, flower or fruit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Organ {
    /// The node it hangs on.
    pub node: u32,
    /// Unit direction the organ points, from the node.
    pub dir: Vec3,
    /// Leaf, flower or fruit.
    pub kind: OrganKind,
    /// Multiplies the species' organ size.
    pub scale: f32,
}

/// A grown plant before meshing: its stems as a tree of nodes, and the
/// organs on them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlantSkeleton {
    /// The root first; every parent before its children.
    pub nodes: Vec<Node>,
    /// Leaves, flowers and fruit the grower placed itself.
    pub organs: Vec<Organ>,
}

impl PlantSkeleton {
    /// A skeleton holding only its root.
    #[must_use]
    pub fn rooted_at(pos: Vec3) -> Self {
        Self {
            nodes: vec![Node { pos, radius: 0.0, parent: None, order: 0 }],
            organs: Vec::new(),
        }
    }

    /// Node `i`, if there is one.
    #[must_use]
    pub fn node(&self, i: u32) -> Option<&Node> {
        self.nodes.get(usize_from(i))
    }

    /// Adds a node under `parent` and returns its index.
    pub fn push(&mut self, parent: u32, pos: Vec3, order: u8) -> u32 {
        let i = u32_from_usize(self.nodes.len());
        self.nodes.push(Node { pos, radius: 0.0, parent: Some(parent), order });
        i
    }

    /// Each node's children, in index order.
    #[must_use]
    pub fn children(&self) -> Vec<Vec<u32>> {
        let mut out = vec![Vec::new(); self.nodes.len()];
        for (i, n) in self.nodes.iter().enumerate() {
            if let Some(list) = n.parent.and_then(|p| out.get_mut(usize_from(p))) {
                list.push(u32_from_usize(i));
            }
        }
        out
    }

    /// How many nodes hang below each node, itself included.
    #[must_use]
    pub fn subtree_sizes(&self) -> Vec<u32> {
        let mut size = vec![1u32; self.nodes.len()];
        for i in (0..self.nodes.len()).rev() {
            let Some(&own) = size.get(i) else { continue };
            if let Some(p) = self.nodes.get(i).and_then(|n| n.parent)
                && let Some(s) = size.get_mut(usize_from(p))
            {
                *s = s.saturating_add(own);
            }
        }
        size
    }

    /// Segments from each node to its nearest tip (tips are 0). Leaves hang
    /// where this is small.
    #[must_use]
    pub fn tip_distances(&self) -> Vec<u16> {
        let mut d = vec![u16::MAX; self.nodes.len()];
        let has_child = self.children();
        for i in (0..self.nodes.len()).rev() {
            if has_child.get(i).is_some_and(Vec::is_empty)
                && let Some(v) = d.get_mut(i)
            {
                *v = 0;
            }
            let Some(&own) = d.get(i) else { continue };
            if let Some(p) = self.nodes.get(i).and_then(|n| n.parent)
                && let Some(v) = d.get_mut(usize_from(p))
            {
                *v = (*v).min(own.saturating_add(1));
            }
        }
        d
    }

    /// The child each node's axis continues into: the one carrying the most
    /// of the plant above it. `None` at tips.
    #[must_use]
    pub fn continuations(&self) -> Vec<Option<u32>> {
        let sizes = self.subtree_sizes();
        self.children()
            .into_iter()
            .map(|kids| {
                kids.into_iter()
                    .max_by_key(|&c| sizes.get(usize_from(c)).copied().unwrap_or(0))
            })
            .collect()
    }

    /// Branch orders from the shape alone: a continuation keeps its parent's
    /// order, every other child is one higher. For growers (like space
    /// colonization) that don't know which branch is which.
    pub fn assign_orders(&mut self) {
        let cont = self.continuations();
        for i in 0..self.nodes.len() {
            let Some(p) = self.nodes.get(i).and_then(|n| n.parent) else { continue };
            let parent_order = self.node(p).map_or(0, |n| n.order);
            let continues = cont.get(usize_from(p)).copied().flatten() == Some(u32_from_usize(i));
            if let Some(n) = self.nodes.get_mut(i) {
                n.order = if continues { parent_order } else { parent_order.saturating_add(1) };
            }
        }
    }

    /// Radii by the pipe model: a parent's `r^e` is the sum of its
    /// children's, tips start at `tip_radius`. `exponent` is about 2 (da
    /// Vinci) to 3 (thicker trunks). A radius the grower already set is
    /// kept as a floor for that node (an L-system's `#`), without widening
    /// the stems below it.
    pub fn pipe_radii(&mut self, tip_radius: f32, exponent: f32) {
        let e = exponent.max(1.0);
        let mut flow = vec![0.0f32; self.nodes.len()];
        for i in (0..self.nodes.len()).rev() {
            let Some(f) = flow.get_mut(i) else { continue };
            if *f == 0.0 {
                *f = tip_radius.powf(e);
            }
            let own = *f;
            if let Some(n) = self.nodes.get_mut(i) {
                n.radius = own.powf(e.recip()).max(n.radius);
            }
            if let Some(p) = self.nodes.get(i).and_then(|n| n.parent)
                && let Some(pf) = flow.get_mut(usize_from(p))
            {
                *pf += own;
            }
        }
    }

    /// The plant cut into axes for meshing: each chain is a list of nodes
    /// along one branch, starting at the node it grows from.
    #[must_use]
    pub fn chains(&self) -> Vec<Vec<u32>> {
        self.chains_along(&self.continuations())
    }

    /// The plant cut into axes like [`Self::chains`], each carrying on into
    /// the child `cont` names for its last node (`None` ends it).
    pub(crate) fn chains_along(&self, cont: &[Option<u32>]) -> Vec<Vec<u32>> {
        let follow = |start: Vec<u32>| {
            let mut chain = start;
            while let Some(next) = chain.last().and_then(|&l| cont.get(usize_from(l)).copied().flatten()) {
                chain.push(next);
            }
            chain
        };
        let mut chains = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            let i = u32_from_usize(i);
            match n.parent {
                None => chains.push(follow(vec![i])),
                Some(p) if cont.get(usize_from(p)).copied().flatten() != Some(i) => {
                    chains.push(follow(vec![p, i]));
                }
                Some(_) => {}
            }
        }
        chains
    }

    /// Axis-aligned bounds of the nodes, padded by their radii.
    #[must_use]
    pub fn bounds(&self) -> (Vec3, Vec3) {
        self.nodes.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), n| {
            (lo.min(n.pos - n.radius), hi.max(n.pos + n.radius))
        })
    }

    /// Distance from the root along the stem to each node, metres.
    #[must_use]
    pub fn path_lengths(&self) -> Vec<f32> {
        let mut len = vec![0.0f32; self.nodes.len()];
        for i in 0..self.nodes.len() {
            let Some(n) = self.nodes.get(i) else { continue };
            let Some(p) = n.parent else { continue };
            let (base, ppos) = (
                len.get(usize_from(p)).copied().unwrap_or(0.0),
                self.node(p).map_or(n.pos, |pn| pn.pos),
            );
            if let Some(l) = len.get_mut(i) {
                *l = base + n.pos.distance(ppos);
            }
        }
        len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Y: a trunk of two segments that forks into two branches.
    fn fork() -> PlantSkeleton {
        let mut s = PlantSkeleton::rooted_at(Vec3::ZERO);
        let a = s.push(0, Vec3::Y, 0);
        let b = s.push(a, Vec3::new(0.0, 2.0, 0.0), 0);
        s.push(b, Vec3::new(0.5, 3.0, 0.0), 0);
        let side = s.push(b, Vec3::new(-0.5, 2.5, 0.0), 0);
        s.push(side, Vec3::new(-0.8, 2.8, 0.0), 0);
        s
    }

    #[test]
    fn pipe_model_conserves_area() {
        let mut s = fork();
        s.pipe_radii(0.01, 2.0);
        let r = |i: usize| s.nodes[i].radius;
        // Node 2 carries two tips: r^2 = 2 * 0.01^2.
        assert!(2.0f32.mul_add(-0.01f32.powi(2), r(2).powi(2)).abs() < 1e-9);
        assert!(r(0) >= r(2) && r(2) > r(3));
    }

    #[test]
    fn orders_and_chains_follow_the_bigger_branch() {
        let mut s = fork();
        s.assign_orders();
        // The fork's two children: one continues the trunk, one branches.
        assert_eq!(s.nodes[3].order + s.nodes[4].order, 1);
        // The side branch is the heavier one, so the trunk continues into it.
        assert_eq!(s.nodes[4].order, 0);
        let chains = s.chains();
        assert_eq!(chains.len(), 2);
        // Every node except the shared fork node appears in exactly one chain.
        let total: usize = chains.iter().map(Vec::len).sum();
        assert_eq!(total, s.nodes.len() + 1);
    }

    #[test]
    fn lengths_and_bounds_measure_the_stems() {
        let mut s = fork();
        let len = s.path_lengths();
        assert!(len[0].abs() < 1e-6 && (len[2] - 2.0).abs() < 1e-6);
        // Up the trunk, then out along the side branch.
        let side = 2.0 + Vec2::new(0.5, 0.5).length() + Vec2::new(0.3, 0.3).length();
        assert!((len[5] - side).abs() < 1e-5, "{}", len[5]);
        s.pipe_radii(0.01, 2.0);
        let (lo, hi) = s.bounds();
        let r0 = s.nodes[0].radius;
        assert!((lo.y + r0).abs() < 1e-6 && (lo.x + 0.8 + 0.01).abs() < 1e-6, "{lo}");
        assert!((hi.y - 3.0 - 0.01).abs() < 1e-6, "{hi}");
        assert_eq!(s.subtree_sizes(), [6, 5, 4, 1, 2, 1]);
    }

    #[test]
    fn tips_are_zero_from_a_tip() {
        let d = fork().tip_distances();
        assert_eq!(d[3], 0);
        assert_eq!(d[5], 0);
        assert_eq!(d[2], 1);
        assert_eq!(d[0], 3);
    }
}
