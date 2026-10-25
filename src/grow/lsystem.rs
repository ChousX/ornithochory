//! Stochastic, parametric, bracketed L-systems (Prusinkiewicz & Lindenmayer,
//! *The Algorithmic Beauty of Plants*), read by a 3D turtle.
//!
//! A module is a symbol with up to four numeric arguments: `F`, `F(0.5)`,
//! `A(n-1, l*0.9)`. Arguments are expressions (see [`super::expr`]) over the
//! rule's parameters and the system's `defines`. A rule `A(n, l) : n > 0 ->
//! ...` is written `(symbol: 'A', params: ["n", "l"], when: "n > 0", to:
//! "...")`; it matches a module with at least that many arguments whose
//! condition holds, and among several matches one is picked by `weight`.
//! Spaces in strings are ignored.
//!
//! Turtle symbols (heading is the turtle's +Y; an argument replaces the
//! default amount):
//!
//! | Symbol | Meaning |
//! |---|---|
//! | `F(k)` | grow `k` steps (default 1), making a node |
//! | `f(k)` | move `k` steps without a node |
//! | `+(a)` `-(a)` | turn left / right `a` degrees (about the turtle's Z) |
//! | `&(a)` `^(a)` | pitch down / up (about its X) |
//! | `\(a)` `/(a)` | roll left / right (about its heading) |
//! | `\|` | turn around |
//! | `$` | roll until the turtle's X is level and its Z faces up (ABOP 1.5): flat leaves and fronds |
//! | `[` `]` | start / end a branch (one order higher, steps shrink) |
//! | `L(s)` | hang a leaf here, `s` times the species' leaf size |
//! | `!(k)` | multiply the step by `k` (default `step_scale`) |
//! | `#(r)` | stems made from here on are at least `r` metres thick |
//! | `~(k)` | tropism strength from here on in this branch |
//!
//! Every other symbol is a variable: rewritten by rules, ignored by the
//! turtle.
//!
//! An optional `envelope` prunes like a gardener (Prusinkiewicz, James &
//! Měch, "Synthetic Topiary", 1994): a step that would leave it ends its
//! branch there, so the plant fills the shape and its tips (and their
//! leaves) line its surface.
//!
//! Cost: the string grows exponentially with `generations`, so derivation
//! stops expanding once it holds `max_modules` modules. Cache the result
//! per variant; never derive per frame.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::Envelope;
use super::expr::Expr;
use crate::random::PlantRng;
use crate::skeleton::{Organ, OrganKind, PlantSkeleton};

/// Most arguments a module carries.
pub const MAX_ARGS: usize = 4;

/// A plant as an L-system: a start string, rules that rewrite it, and how
/// the turtle reads the result (see the module docs for the symbols).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct LSystem {
    /// The string before the first rewrite.
    pub axiom: String,
    /// The rewriting rules; a module that matches none is copied.
    pub rules: Vec<Rule>,
    /// How many times the string is rewritten.
    pub generations: u8,
    /// Derivation stops expanding past this many modules.
    pub max_modules: u32,
    /// Length of one `F`, metres.
    pub step: f32,
    /// Each `[` multiplies the step by this; so does `!`.
    pub step_scale: f32,
    /// Turn angle for `+ - & ^ \ /` without an argument, degrees.
    pub angle: f32,
    /// Random spread on every turn, degrees.
    pub angle_jitter: f32,
    /// Direction stems bend towards after each step (gravity: -Y,
    /// phototropism: +Y), and how strongly.
    pub tropism: Vec3,
    /// Radians a step turns towards `tropism`, when the heading points
    /// straight across it.
    pub tropism_strength: f32,
    /// Named constants every expression can use, so a species file can be
    /// tuned in one place.
    pub defines: BTreeMap<String, f32>,
    /// Steps that would leave this volume end their branch.
    pub envelope: Option<Envelope>,
}

impl Default for LSystem {
    fn default() -> Self {
        Self {
            axiom: "A".into(),
            rules: vec![Rule { symbol: 'A', to: "F[+A][-A]FA".into(), ..Rule::default() }],
            generations: 4,
            max_modules: 20_000,
            step: 0.3,
            step_scale: 0.8,
            angle: 25.0,
            angle_jitter: 6.0,
            tropism: Vec3::NEG_Y,
            tropism_strength: 0.05,
            defines: BTreeMap::new(),
            envelope: None,
        }
    }
}

/// `symbol(params) : when -> to`, chosen among the matching rules for the
/// same symbol by `weight`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Rule {
    /// The module this rule rewrites.
    pub symbol: char,
    /// Names for the module's first arguments, usable in `when` and `to`.
    #[serde(default)]
    pub params: Vec<String>,
    /// Condition; empty is always true.
    #[serde(default)]
    pub when: String,
    /// What the module becomes: modules whose arguments may use `params`.
    pub to: String,
    /// How likely this rule is, against the other rules that match.
    #[serde(default = "default_weight")]
    pub weight: f32,
}

impl Default for Rule {
    fn default() -> Self {
        Self { symbol: 'A', params: Vec::new(), when: String::new(), to: String::new(), weight: 1.0 }
    }
}

const fn default_weight() -> f32 {
    1.0
}

/// One symbol of a derived string, with its arguments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Module {
    /// What the module is: a turtle command or a variable.
    pub symbol: char,
    args: [f32; MAX_ARGS],
    count: u8,
}

impl Module {
    /// The arguments, in order.
    #[must_use]
    pub fn args(&self) -> &[f32] {
        self.args.get(..usize::from(self.count)).unwrap_or(&[])
    }

    /// The first argument, if there is one.
    #[must_use]
    pub fn arg(&self) -> Option<f32> {
        self.args().first().copied()
    }
}

/// A module in a rule or axiom, before its arguments are evaluated.
#[derive(Clone, Debug)]
struct Template {
    symbol: char,
    args: Vec<Expr>,
}

#[derive(Clone, Debug)]
struct CompiledRule {
    symbol: char,
    arity: usize,
    when: Option<Expr>,
    to: Vec<Template>,
    weight: f32,
}

/// Everything parsed once per derivation.
struct Compiled {
    axiom: Vec<Template>,
    rules: Vec<CompiledRule>,
    defines: Vec<f32>,
}

/// Splits `src` into modules, compiling each argument against `names`.
fn templates(src: &str, names: &[&str]) -> Result<Vec<Template>, String> {
    let chars: Vec<char> = src.chars().filter(|c| !c.is_whitespace()).collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(&symbol) = chars.get(i) {
        i = i.saturating_add(1);
        let mut args = Vec::new();
        if chars.get(i) == Some(&'(') {
            // Up to the matching ')', split at top-level commas.
            let mut depth = 0u32;
            let mut arg = String::new();
            i = i.saturating_add(1);
            loop {
                let Some(&c) = chars.get(i) else {
                    return Err(format!("unclosed '(' after '{symbol}' in \"{src}\""));
                };
                i = i.saturating_add(1);
                match c {
                    '(' => depth = depth.saturating_add(1),
                    ')' if depth == 0 => break,
                    ')' => depth = depth.saturating_sub(1),
                    ',' if depth == 0 => {
                        args.push(Expr::parse(&arg, names)?);
                        arg.clear();
                        continue;
                    }
                    _ => {}
                }
                arg.push(c);
            }
            args.push(Expr::parse(&arg, names)?);
            if args.len() > MAX_ARGS {
                return Err(format!("'{symbol}' has more than {MAX_ARGS} arguments in \"{src}\""));
            }
        }
        out.push(Template { symbol, args });
    }
    Ok(out)
}

impl LSystem {
    fn compile(&self) -> Result<Compiled, String> {
        let define_names: Vec<&str> = self.defines.keys().map(String::as_str).collect();
        let rules = self
            .rules
            .iter()
            .map(|r| {
                let names: Vec<&str> = r.params.iter().map(String::as_str).chain(define_names.iter().copied()).collect();
                let when = if r.when.trim().is_empty() { None } else { Some(Expr::parse(&r.when, &names)?) };
                Ok(CompiledRule {
                    symbol: r.symbol,
                    arity: r.params.len(),
                    when,
                    to: templates(&r.to, &names)?,
                    weight: r.weight.max(0.0),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Compiled { axiom: templates(&self.axiom, &define_names)?, rules, defines: self.defines.values().copied().collect() })
    }

    /// Checks that the axiom, every rule and every expression parse.
    ///
    /// # Errors
    ///
    /// The first problem found, as a message.
    pub fn validate(&self) -> Result<(), String> {
        self.compile().map(|_| ())
    }

    /// The derived string after `generations` rewrites.
    ///
    /// # Errors
    ///
    /// When the axiom or a rule doesn't parse (see [`Self::validate`]).
    pub fn derive(&self, rng: &mut PlantRng) -> Result<Vec<Module>, String> {
        let c = self.compile()?;
        let mut env: Vec<f32> = Vec::new();
        let mut stack: Vec<f32> = Vec::new();
        let mut s: Vec<Module> = Vec::new();
        for t in &c.axiom {
            s.push(instantiate(t, &c.defines, &mut stack, rng));
        }
        let cap = usize::try_from(self.max_modules).unwrap_or(usize::MAX);
        let mut matches: Vec<usize> = Vec::new();
        for _ in 0..self.generations {
            // A full string stops the derivation: later passes would only
            // copy it while expanding its first modules.
            if s.len() >= cap {
                break;
            }
            let mut next = Vec::with_capacity(s.len().saturating_mul(2));
            for m in &s {
                if next.len() >= cap {
                    next.push(*m);
                    continue;
                }
                match pick(&c, m, &mut matches, &mut env, &mut stack, rng) {
                    Some(rule) => {
                        bind(&mut env, m, rule.arity, &c.defines);
                        for t in &rule.to {
                            next.push(instantiate(t, &env, &mut stack, rng));
                        }
                    }
                    None => next.push(*m),
                }
            }
            s = next;
        }
        Ok(s)
    }

    /// Derives the string and walks the turtle over it. A system that
    /// doesn't parse logs why and grows only a root.
    #[must_use]
    pub fn grow(&self, rng: &mut PlantRng) -> PlantSkeleton {
        match self.derive(rng) {
            Ok(modules) => self.interpret(&modules, rng),
            Err(e) => {
                warn!("L-system: {e}");
                PlantSkeleton::rooted_at(Vec3::ZERO)
            }
        }
    }

    fn interpret(&self, modules: &[Module], rng: &mut PlantRng) -> PlantSkeleton {
        #[derive(Clone, Copy)]
        struct Turtle {
            pos: Vec3,
            rot: Quat,
            node: u32,
            order: u8,
            step: f32,
            /// Floor for the radius of new nodes (`#`).
            width: f32,
            tropism: f32,
            /// Left the envelope: nothing more grows on this branch.
            cut: bool,
        }

        let mut skel = PlantSkeleton::rooted_at(Vec3::ZERO);
        let mut t = Turtle {
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            node: 0,
            order: 0,
            step: self.step,
            width: 0.0,
            tropism: self.tropism_strength,
            cut: false,
        };
        let mut stack: Vec<Turtle> = Vec::new();
        let jitter = self.angle_jitter.to_radians();
        let tropism = self.tropism.normalize_or_zero();

        for m in modules {
            let a = m.arg();
            let turn = |axis: Vec3, sign: f32, rng: &mut PlantRng| {
                let angle = a.unwrap_or(self.angle).to_radians();
                Quat::from_axis_angle(axis, sign.mul_add(angle, rng.signed() * jitter))
            };
            match m.symbol {
                'F' | 'f' => {
                    if t.cut {
                        continue;
                    }
                    let to = t.pos + t.rot * Vec3::Y * (t.step * a.unwrap_or(1.0));
                    if self.envelope.is_some_and(|e| !e.contains(to)) {
                        t.cut = true;
                        continue;
                    }
                    t.pos = to;
                    if m.symbol == 'F' {
                        t.node = skel.push(t.node, t.pos, t.order);
                        if let Some(n) = skel.nodes.last_mut() {
                            n.radius = t.width;
                        }
                        // Bend the heading towards the tropism vector, in
                        // proportion to how far it points away (ABOP 2.4).
                        let h = t.rot * Vec3::Y;
                        let axis = h.cross(tropism);
                        let amount = axis.length() * t.tropism;
                        if amount.abs() > 1e-6 {
                            t.rot = Quat::from_axis_angle(axis.normalize(), amount) * t.rot;
                        }
                    }
                }
                '+' => t.rot *= turn(Vec3::Z, 1.0, rng),
                '-' => t.rot *= turn(Vec3::Z, -1.0, rng),
                '&' => t.rot *= turn(Vec3::X, 1.0, rng),
                '^' => t.rot *= turn(Vec3::X, -1.0, rng),
                '\\' => t.rot *= turn(Vec3::Y, 1.0, rng),
                '/' => t.rot *= turn(Vec3::Y, -1.0, rng),
                '|' => t.rot *= Quat::from_rotation_z(std::f32::consts::PI),
                '$' => t.rot = level(t.rot),
                '!' => t.step *= a.unwrap_or(self.step_scale),
                '#' => t.width = a.unwrap_or(0.0).max(0.0),
                '~' => t.tropism = a.unwrap_or(self.tropism_strength),
                '[' => {
                    stack.push(t);
                    t.order = t.order.saturating_add(1);
                    t.step *= self.step_scale;
                }
                ']' => {
                    if let Some(prev) = stack.pop() {
                        t = prev;
                    }
                }
                'L' if !t.cut => skel.organs.push(Organ {
                    node: t.node,
                    dir: (t.rot * Vec3::Y).normalize_or(Vec3::Y),
                    kind: OrganKind::Leaf,
                    scale: a.unwrap_or(1.0).max(0.0),
                }),
                _ => {}
            }
        }
        skel
    }
}

/// The matching rule for `m`, if any: its symbol, enough arguments, a true
/// condition; several are picked between by weight.
fn pick<'c>(
    c: &'c Compiled,
    m: &Module,
    matches: &mut Vec<usize>,
    env: &mut Vec<f32>,
    stack: &mut Vec<f32>,
    rng: &mut PlantRng,
) -> Option<&'c CompiledRule> {
    matches.clear();
    let mut total = 0.0f32;
    for (i, r) in c.rules.iter().enumerate() {
        if r.symbol != m.symbol || r.arity > usize::from(m.count) || r.weight <= 0.0 {
            continue;
        }
        if let Some(when) = &r.when {
            bind(env, m, r.arity, &c.defines);
            if when.eval(env, stack, rng) == 0.0 {
                continue;
            }
        }
        matches.push(i);
        total += r.weight;
    }
    if matches.len() > 1 {
        let mut x = rng.unit() * total;
        for &i in matches.iter() {
            let r = c.rules.get(i)?;
            x -= r.weight;
            if x < 0.0 {
                return Some(r);
            }
        }
    }
    matches.last().and_then(|&i| c.rules.get(i))
}

/// The variables a rule sees: its parameters, then the defines.
fn bind(env: &mut Vec<f32>, m: &Module, arity: usize, defines: &[f32]) {
    env.clear();
    env.extend(m.args().iter().take(arity));
    env.extend_from_slice(defines);
}

fn instantiate(t: &Template, env: &[f32], stack: &mut Vec<f32>, rng: &mut PlantRng) -> Module {
    let mut args = [0.0f32; MAX_ARGS];
    for (slot, e) in args.iter_mut().zip(&t.args) {
        *slot = e.eval(env, stack, rng);
    }
    let count = u8::try_from(t.args.len().min(MAX_ARGS)).unwrap_or(0);
    Module { symbol: t.symbol, args, count }
}

/// `rot` rolled about its heading so its X axis is horizontal and its Z
/// points up. Unchanged when heading straight up or down.
fn level(rot: Quat) -> Quat {
    let h = rot * Vec3::Y;
    let x = h.cross(Vec3::Y);
    if x.length_squared() < 1e-6 {
        return rot;
    }
    let x = x.normalize();
    let z = x.cross(h);
    Quat::from_mat3(&Mat3::from_cols(x, h, z)).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbols(m: &[Module]) -> String {
        m.iter().map(|m| m.symbol).collect()
    }

    #[test]
    fn same_seed_same_plant() {
        let l = LSystem::default();
        let a = l.grow(&mut PlantRng::new(3));
        let b = l.grow(&mut PlantRng::new(3));
        assert_eq!(a, b);
        assert!(a.nodes.len() > 20);
    }

    #[test]
    fn module_cap_bounds_growth() {
        let l = LSystem { generations: 12, max_modules: 500, ..LSystem::default() };
        let s = l.derive(&mut PlantRng::new(1)).unwrap();
        // The last pass starts under the cap and copies what it can't expand.
        assert!(s.len() < 500 * 2 + 64, "{}", s.len());
    }

    #[test]
    fn brackets_raise_the_order() {
        let l = LSystem {
            axiom: "F[+F]F".into(),
            rules: Vec::new(),
            generations: 0,
            angle_jitter: 0.0,
            ..LSystem::default()
        };
        let s = l.grow(&mut PlantRng::new(0));
        let orders: Vec<u8> = s.nodes.iter().map(|n| n.order).collect();
        assert_eq!(orders, vec![0, 0, 1, 0]);
    }

    #[test]
    fn parametric_rules_count_down() {
        let l = LSystem {
            axiom: "A(3, 1)".into(),
            rules: vec![Rule {
                symbol: 'A',
                params: vec!["n".into(), "s".into()],
                when: "n > 0".into(),
                to: "F(s) L(s * k) A(n - 1, s * 0.5)".into(),
                ..Rule::default()
            }],
            generations: 10,
            defines: BTreeMap::from([("k".into(), 2.0)]),
            ..LSystem::default()
        };
        let m = l.derive(&mut PlantRng::new(0)).unwrap();
        assert_eq!(symbols(&m), "FLFLFLA");
        let leaf: Vec<f32> = m.iter().filter(|m| m.symbol == 'L').filter_map(Module::arg).collect();
        assert_eq!(leaf, vec![2.0, 1.0, 0.5]);
        let s = l.grow(&mut PlantRng::new(0));
        let top = s.nodes.last().unwrap().pos.length();
        assert!((top - 0.525).abs() < 1e-4, "{top}");
        assert_eq!(s.organs.iter().map(|o| o.scale).collect::<Vec<_>>(), leaf);
    }

    #[test]
    fn bad_rules_are_reported() {
        let l = LSystem { axiom: "A(x)".into(), ..LSystem::default() };
        assert!(l.validate().unwrap_err().contains("\"x\""));
        assert_eq!(l.grow(&mut PlantRng::new(0)).nodes.len(), 1);
    }

    #[test]
    fn envelope_prunes() {
        let l = LSystem {
            axiom: "FFFFFFFFFF".into(),
            rules: Vec::new(),
            envelope: Some(Envelope::Ellipsoid { center: Vec3::ZERO, radii: Vec3::splat(1.0) }),
            ..LSystem::default()
        };
        let s = l.grow(&mut PlantRng::new(0));
        assert!(s.nodes.iter().all(|n| n.pos.length() <= 1.0));
        assert_eq!(s.nodes.len(), 4);
    }

    #[test]
    fn width_and_level() {
        let l = LSystem { axiom: "#(0.05)&(60)$F".into(), rules: Vec::new(), angle_jitter: 0.0, ..LSystem::default() };
        let s = l.grow(&mut PlantRng::new(0));
        assert!((s.nodes[1].radius - 0.05).abs() < 1e-6);
        let r = level(Quat::from_rotation_y(0.7) * Quat::from_rotation_x(1.0) * Quat::from_rotation_y(0.4));
        assert!((r * Vec3::X).y.abs() < 1e-5);
        assert!((r * Vec3::Z).y > 0.0);
    }
}
