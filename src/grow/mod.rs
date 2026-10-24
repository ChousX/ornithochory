//! Growers: each turns a description and a seed into a [`PlantSkeleton`].
//!
//! - [`LSystem`]: a stochastic, parametric, bracketed L-system read by a
//!   3D turtle.
//!   Ferns, herbs, flowers, palms, stylised trees: anything with a rule.
//! - [`Colonize`]: space colonization (Runions et al. 2007). Natural crowns
//!   filling any envelope, roots, hedges.
//! - [`Recursive`]: Weber & Penn's recursive parameter tables. Whorled
//!   conifers, palms, anything with a strong leader and regular branching.
//!
//! Anything else (self-organizing trees, a hand-made
//! skeleton) only has to return a `PlantSkeleton`; see
//! [`crate::generate::build_structure`] for what happens next.

mod colonize;
mod expr;
mod lsystem;
mod recursive;

pub use colonize::{Colonize, Envelope};
pub use lsystem::{LSystem, Module, Rule};
pub use recursive::{Level, Recursive, Shape};

use serde::{Deserialize, Serialize};

use crate::random::PlantRng;
use crate::skeleton::PlantSkeleton;

/// How a [`crate::species::Structure`] grows.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Growth {
    LSystem(LSystem),
    Colonize(Colonize),
    Recursive(Recursive),
}

impl Growth {
    /// The skeleton for one variant. Radii are left to the caller.
    #[must_use]
    pub fn grow(&self, rng: &mut PlantRng) -> PlantSkeleton {
        match self {
            Self::LSystem(l) => l.grow(rng),
            Self::Colonize(c) => c.grow(rng),
            Self::Recursive(r) => r.grow(rng),
        }
    }
}
