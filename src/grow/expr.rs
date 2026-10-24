//! Arithmetic for parametric L-systems (ABOP ch. 1.10): the arguments of
//! modules like `F(l*0.9)` and the conditions of rules like `A(n) : n > 0`.
//!
//! Expressions are compiled once per derivation into a small stack program
//! over numbered variables (a rule's parameters, then the species'
//! `defines`), so evaluating one is a short loop with no lookups.
//!
//! | Syntax | Meaning |
//! |---|---|
//! | `1.5` `n` `pi` | number, variable, constant |
//! | `+ - * /` | arithmetic (unary `-` too) |
//! | `< > <= >= == !=` | comparison: 1 or 0 |
//! | `&&` `\|\|` `!` | logic: 0 is false, anything else true |
//! | `min max pow clamp` | `min(a, b)`, `pow(a, b)`, `clamp(x, lo, hi)` |
//! | `sqrt abs floor sin cos` | one argument; angles in radians |
//! | `rand(lo, hi)` | uniform in `lo..hi`, from the plant's seeded RNG |

use crate::random::PlantRng;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Func {
    Min,
    Max,
    Pow,
    Clamp,
    Sqrt,
    Abs,
    Floor,
    Sin,
    Cos,
    Rand,
}

impl Func {
    fn named(name: &str) -> Option<Self> {
        Some(match name {
            "min" => Self::Min,
            "max" => Self::Max,
            "pow" => Self::Pow,
            "clamp" => Self::Clamp,
            "sqrt" => Self::Sqrt,
            "abs" => Self::Abs,
            "floor" => Self::Floor,
            "sin" => Self::Sin,
            "cos" => Self::Cos,
            "rand" => Self::Rand,
            _ => return None,
        })
    }

    const fn arity(self) -> usize {
        match self {
            Self::Sqrt | Self::Abs | Self::Floor | Self::Sin | Self::Cos => 1,
            Self::Min | Self::Max | Self::Pow | Self::Rand => 2,
            Self::Clamp => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Num(f32),
    Var(usize),
    Neg,
    Not,
    Add,
    Sub,
    Mul,
    Div,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    And,
    Or,
    Call(Func),
}

/// A compiled expression.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Expr(Vec<Op>);

impl Expr {
    /// Compiles `src`; a variable's value is `env[i]` where `names[i]` is
    /// its name.
    ///
    /// # Errors
    ///
    /// A message naming the problem: an unknown name, a wrong argument
    /// count, or a syntax error.
    pub fn parse(src: &str, names: &[&str]) -> Result<Self, String> {
        let mut p = Parser { chars: src.chars().collect(), pos: 0, names, out: Vec::new() };
        p.or()?;
        p.skip_ws();
        if let Some(c) = p.peek() {
            return Err(format!("unexpected '{c}' in \"{src}\""));
        }
        Ok(Self(p.out))
    }

    /// The value under `env`; `stack` is scratch space kept between calls.
    /// Never NaN or infinite: those become 0.
    pub fn eval(&self, env: &[f32], stack: &mut Vec<f32>, rng: &mut PlantRng) -> f32 {
        stack.clear();
        for &op in &self.0 {
            let v = match op {
                Op::Num(v) => v,
                Op::Var(i) => env.get(i).copied().unwrap_or(0.0),
                Op::Neg => -stack.pop().unwrap_or(0.0),
                Op::Not => truth(stack.pop().unwrap_or(0.0) == 0.0),
                Op::Call(f) => call(f, stack, rng),
                _ => {
                    let b = stack.pop().unwrap_or(0.0);
                    let a = stack.pop().unwrap_or(0.0);
                    binary(op, a, b)
                }
            };
            stack.push(v);
        }
        let v = stack.pop().unwrap_or(0.0);
        if v.is_finite() { v } else { 0.0 }
    }
}

const fn truth(b: bool) -> f32 {
    if b { 1.0 } else { 0.0 }
}

fn binary(op: Op, a: f32, b: f32) -> f32 {
    match op {
        Op::Add => a + b,
        Op::Sub => a - b,
        Op::Mul => a * b,
        Op::Div => a / b,
        Op::Lt => truth(a < b),
        Op::Gt => truth(a > b),
        Op::Le => truth(a <= b),
        Op::Ge => truth(a >= b),
        Op::Eq => truth((a - b).abs() <= f32::EPSILON * a.abs().max(1.0)),
        Op::Ne => truth((a - b).abs() > f32::EPSILON * a.abs().max(1.0)),
        Op::And => truth(a != 0.0 && b != 0.0),
        Op::Or => truth(a != 0.0 || b != 0.0),
        _ => 0.0,
    }
}

fn call(f: Func, stack: &mut Vec<f32>, rng: &mut PlantRng) -> f32 {
    let mut arg = [0.0f32; 3];
    for slot in arg.iter_mut().take(f.arity()).rev() {
        *slot = stack.pop().unwrap_or(0.0);
    }
    let [a, b, c] = arg;
    match f {
        Func::Min => a.min(b),
        Func::Max => a.max(b),
        Func::Pow => a.powf(b),
        Func::Clamp => a.clamp(b.min(c), c.max(b)),
        Func::Sqrt => a.max(0.0).sqrt(),
        Func::Abs => a.abs(),
        Func::Floor => a.floor(),
        Func::Sin => a.sin(),
        Func::Cos => a.cos(),
        Func::Rand => rng.range(a, b),
    }
}

/// Recursive descent, lowest precedence first; writes postfix ops to `out`.
struct Parser<'a> {
    chars: Vec<char>,
    pos: usize,
    names: &'a [&'a str],
    out: Vec<Op>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    const fn bump(&mut self) {
        self.pos = self.pos.saturating_add(1);
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    /// Consumes `token` if it comes next (after whitespace).
    fn eat(&mut self, token: &str) -> bool {
        self.skip_ws();
        let n = token.chars().count();
        let ahead = self.chars.iter().skip(self.pos).take(n);
        if ahead.copied().eq(token.chars()) {
            self.pos = self.pos.saturating_add(n);
            true
        } else {
            false
        }
    }

    fn or(&mut self) -> Result<(), String> {
        self.and()?;
        while self.eat("||") {
            self.and()?;
            self.out.push(Op::Or);
        }
        Ok(())
    }

    fn and(&mut self) -> Result<(), String> {
        self.cmp()?;
        while self.eat("&&") {
            self.cmp()?;
            self.out.push(Op::And);
        }
        Ok(())
    }

    fn cmp(&mut self) -> Result<(), String> {
        self.add()?;
        // Two-character operators first, so `<=` isn't read as `<`.
        for (token, op) in [("<=", Op::Le), (">=", Op::Ge), ("==", Op::Eq), ("!=", Op::Ne), ("<", Op::Lt), (">", Op::Gt)] {
            if self.eat(token) {
                self.add()?;
                self.out.push(op);
                break;
            }
        }
        Ok(())
    }

    fn add(&mut self) -> Result<(), String> {
        self.mul()?;
        loop {
            let op = if self.eat("+") {
                Op::Add
            } else if self.eat("-") {
                Op::Sub
            } else {
                return Ok(());
            };
            self.mul()?;
            self.out.push(op);
        }
    }

    fn mul(&mut self) -> Result<(), String> {
        self.unary()?;
        loop {
            let op = if self.eat("*") {
                Op::Mul
            } else if self.eat("/") {
                Op::Div
            } else {
                return Ok(());
            };
            self.unary()?;
            self.out.push(op);
        }
    }

    fn unary(&mut self) -> Result<(), String> {
        if self.eat("-") {
            self.unary()?;
            self.out.push(Op::Neg);
        } else if self.eat("!") {
            self.unary()?;
            self.out.push(Op::Not);
        } else {
            self.primary()?;
        }
        Ok(())
    }

    fn primary(&mut self) -> Result<(), String> {
        self.skip_ws();
        let Some(c) = self.peek() else {
            return Err("expression ends early".into());
        };
        if self.eat("(") {
            self.or()?;
            return if self.eat(")") { Ok(()) } else { Err("missing ')'".into()) };
        }
        if c.is_ascii_digit() || c == '.' {
            let start = self.pos;
            while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.') {
                self.bump();
            }
            let text: String = self.chars.iter().skip(start).take(self.pos.saturating_sub(start)).collect();
            let v = text.parse::<f32>().map_err(|_| format!("bad number \"{text}\""))?;
            self.out.push(Op::Num(v));
            return Ok(());
        }
        if c.is_alphabetic() || c == '_' {
            let start = self.pos;
            while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                self.bump();
            }
            let name: String = self.chars.iter().skip(start).take(self.pos.saturating_sub(start)).collect();
            return self.name(&name);
        }
        Err(format!("unexpected '{c}'"))
    }

    /// A variable, constant or function call.
    fn name(&mut self, name: &str) -> Result<(), String> {
        if let Some(i) = self.names.iter().position(|n| *n == name) {
            self.out.push(Op::Var(i));
            return Ok(());
        }
        if name == "pi" {
            self.out.push(Op::Num(std::f32::consts::PI));
            return Ok(());
        }
        let Some(f) = Func::named(name) else {
            return Err(format!("unknown name \"{name}\""));
        };
        if !self.eat("(") {
            return Err(format!("{name} needs arguments"));
        }
        let mut count = 0usize;
        if !self.eat(")") {
            loop {
                self.or()?;
                count = count.saturating_add(1);
                if self.eat(")") {
                    break;
                }
                if !self.eat(",") {
                    return Err(format!("expected ',' or ')' in {name}(...)"));
                }
            }
        }
        if count != f.arity() {
            return Err(format!("{name} takes {} arguments, got {count}", f.arity()));
        }
        self.out.push(Op::Call(f));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(src: &str, names: &[&str], env: &[f32]) -> f32 {
        Expr::parse(src, names).unwrap().eval(env, &mut Vec::new(), &mut PlantRng::new(0))
    }

    #[test]
    fn precedence_and_variables() {
        assert!((eval("1 + 2 * 3", &[], &[]) - 7.0).abs() < 1e-6);
        assert!((eval("(1 + 2) * 3", &[], &[]) - 9.0).abs() < 1e-6);
        assert!((eval("-n / 2", &["n"], &[4.0]) + 2.0).abs() < 1e-6);
        assert!((eval("n > 2 && m <= 1", &["n", "m"], &[3.0, 1.0]) - 1.0).abs() < 1e-6);
        assert!((eval("min(n, 2) + clamp(5, 0, 1)", &["n"], &[9.0]) - 3.0).abs() < 1e-6);
        assert!(eval("1 / 0", &[], &[]).abs() < 1e-6, "infinities become 0");
    }

    #[test]
    fn rand_stays_in_range() {
        let e = Expr::parse("rand(2, 3)", &[]).unwrap();
        let mut rng = PlantRng::new(5);
        for _ in 0..100 {
            assert!((2.0..3.0).contains(&e.eval(&[], &mut Vec::new(), &mut rng)));
        }
    }

    #[test]
    fn errors_name_the_problem() {
        assert!(Expr::parse("x + 1", &[]).unwrap_err().contains("\"x\""));
        assert!(Expr::parse("min(1)", &[]).is_err());
        assert!(Expr::parse("(1", &[]).is_err());
        assert!(Expr::parse("1 2", &[]).is_err());
    }
}
