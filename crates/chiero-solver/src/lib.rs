//! The solver. See `docs/specs/022-solver.md`.
//!
//! **Knows nothing about C and nothing about CIR** (001 §2). Its vocabulary is sorts and
//! terms, which keeps its test suite pure constraint solving and stops C semantics
//! leaking into a layer that must be trustworthy.

use indexmap::IndexMap;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sort {
    Bool,
    BitVec(u32),
}

impl Sort {
    pub fn width(self) -> u32 {
        match self {
            Sort::Bool => 1,
            Sort::BitVec(w) => w,
        }
    }
}

/// A bitvector constant, stored in the low `width` bits with the rest zero.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BvConst {
    width: u32,
    bits: u128,
}

impl BvConst {
    /// Truncates to `width`, so a `BvConst` is always canonical and two constants
    /// denoting the same value are `Eq` — which hash-consing relies on.
    pub fn new(width: u32, bits: u128) -> Self {
        assert!(width > 0 && width <= 128, "width {width} out of range");
        BvConst {
            width,
            bits: bits & mask(width),
        }
    }

    pub fn width(self) -> u32 {
        self.width
    }

    pub fn bits(self) -> u128 {
        self.bits
    }

    /// Interpreted as two's complement.
    pub fn signed(self) -> i128 {
        let m = 1u128 << (self.width - 1);
        if self.bits & m != 0 {
            (self.bits | !mask(self.width)) as i128
        } else {
            self.bits as i128
        }
    }

    pub fn is_negative(self) -> bool {
        self.bits & (1u128 << (self.width - 1)) != 0
    }

    pub fn all_ones(width: u32) -> Self {
        BvConst {
            width,
            bits: mask(width),
        }
    }

    pub fn zero(width: u32) -> Self {
        BvConst { width, bits: 0 }
    }
}

fn mask(w: u32) -> u128 {
    if w >= 128 {
        u128::MAX
    } else {
        (1u128 << w) - 1
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Term(pub u32);

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VarId(pub u32);

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BinKind {
    Add,
    Mul,
    And,
    Or,
    Xor,
    UDiv,
    SDiv,
    URem,
    SRem,
    Shl,
    LShr,
    AShr,
    Ult,
    Slt,
    Eq,
}

impl BinKind {
    /// Commutative operators have their operands normalized, so `x+1` and `1+x` are one
    /// term (022 §3).
    fn is_commutative(self) -> bool {
        matches!(
            self,
            BinKind::Add | BinKind::Mul | BinKind::And | BinKind::Or | BinKind::Xor | BinKind::Eq
        )
    }

    /// Comparisons yield `Bool`; everything else yields its operands' width.
    fn is_predicate(self) -> bool {
        matches!(self, BinKind::Ult | BinKind::Slt | BinKind::Eq)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Node {
    Const(BvConst),
    Var(VarId, Sort),
    Bin(BinKind, Term, Term),
    Not(Term),
    Extend { a: Term, to: u32, signed: bool },
    Extract { a: Term, hi: u32, lo: u32 },
}

/// A complete assignment. Every declared variable has a value (022 §2), which is what
/// makes evaluation total and a validated model a real witness.
#[derive(Clone, Debug, Default)]
pub struct Model {
    values: IndexMap<VarId, BvConst>,
}

impl Model {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&mut self, v: VarId, c: BvConst) {
        self.values.insert(v, c);
    }
    pub fn get(&self, v: VarId) -> Option<BvConst> {
        self.values.get(&v).copied()
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalError(pub String);

/// Hash-consed term storage. Structural equality is `Term` equality, which is what makes
/// the caches in 022 §6 structural rather than textual.
#[derive(Debug, Default)]
pub struct TermArena {
    nodes: Vec<Node>,
    interned: IndexMap<Node, Term>,
    vars: Vec<(String, Sort)>,
}

impl TermArena {
    pub fn new() -> Self {
        Self::default()
    }

    fn intern(&mut self, n: Node) -> Term {
        if let Some(&t) = self.interned.get(&n) {
            return t;
        }
        let t = Term(self.nodes.len() as u32);
        self.nodes.push(n.clone());
        self.interned.insert(n, t);
        t
    }

    pub fn bv(&mut self, w: u32, v: u128) -> Term {
        self.intern(Node::Const(BvConst::new(w, v)))
    }

    pub fn var(&mut self, s: Sort, name: &str) -> Term {
        let id = VarId(self.vars.len() as u32);
        self.vars.push((name.to_string(), s));
        self.intern(Node::Var(id, s))
    }

    pub fn var_id(&self, t: Term) -> Option<VarId> {
        match self.nodes[t.0 as usize] {
            Node::Var(v, _) => Some(v),
            _ => None,
        }
    }

    pub fn as_const(&self, t: Term) -> Option<BvConst> {
        match self.nodes[t.0 as usize] {
            Node::Const(c) => Some(c),
            _ => None,
        }
    }

    /// Width of a term's sort.
    pub fn width(&self, t: Term) -> u32 {
        match &self.nodes[t.0 as usize] {
            Node::Const(c) => c.width(),
            Node::Var(_, s) => s.width(),
            Node::Bin(k, a, _) => {
                if k.is_predicate() {
                    1
                } else {
                    self.width(*a)
                }
            }
            Node::Not(a) => self.width(*a),
            Node::Extend { to, .. } => *to,
            Node::Extract { hi, lo, .. } => hi - lo + 1,
        }
    }

    /// Build a binary node, folding constants and normalizing commutative operands.
    ///
    /// Folding at construction makes it an **invariant** rather than a pass: no term in
    /// the arena is a constant operation over constants (022 §3).
    fn bin(&mut self, k: BinKind, a: Term, b: Term) -> Term {
        assert_eq!(
            self.width(a),
            self.width(b),
            "operand widths must match for {k:?}"
        );
        if let (Some(x), Some(y)) = (self.as_const(a), self.as_const(b)) {
            let r = fold(k, x, y);
            return self.intern(Node::Const(r));
        }
        // Identity and annihilator laws.
        if let Some(c) = self.as_const(b) {
            match k {
                BinKind::Add | BinKind::Or | BinKind::Xor if c.bits() == 0 => return a,
                BinKind::Mul if c.bits() == 1 => return a,
                BinKind::Mul | BinKind::And if c.bits() == 0 => return b,
                BinKind::And if c.bits() == mask(c.width()) => return a,
                _ => {}
            }
        }
        let (a, b) = if k.is_commutative() && b < a {
            (b, a)
        } else {
            (a, b)
        };
        self.intern(Node::Bin(k, a, b))
    }

    pub fn add(&mut self, a: Term, b: Term) -> Term {
        self.bin(BinKind::Add, a, b)
    }
    pub fn mul(&mut self, a: Term, b: Term) -> Term {
        self.bin(BinKind::Mul, a, b)
    }
    pub fn and(&mut self, a: Term, b: Term) -> Term {
        self.bin(BinKind::And, a, b)
    }
    pub fn or(&mut self, a: Term, b: Term) -> Term {
        self.bin(BinKind::Or, a, b)
    }
    pub fn xor(&mut self, a: Term, b: Term) -> Term {
        self.bin(BinKind::Xor, a, b)
    }
    pub fn udiv(&mut self, a: Term, b: Term) -> Term {
        self.bin(BinKind::UDiv, a, b)
    }
    pub fn sdiv(&mut self, a: Term, b: Term) -> Term {
        self.bin(BinKind::SDiv, a, b)
    }
    pub fn urem(&mut self, a: Term, b: Term) -> Term {
        self.bin(BinKind::URem, a, b)
    }
    pub fn srem(&mut self, a: Term, b: Term) -> Term {
        self.bin(BinKind::SRem, a, b)
    }
    pub fn shl(&mut self, a: Term, b: Term) -> Term {
        self.bin(BinKind::Shl, a, b)
    }
    pub fn lshr(&mut self, a: Term, b: Term) -> Term {
        self.bin(BinKind::LShr, a, b)
    }
    pub fn ashr(&mut self, a: Term, b: Term) -> Term {
        self.bin(BinKind::AShr, a, b)
    }
    pub fn ult(&mut self, a: Term, b: Term) -> Term {
        self.bin(BinKind::Ult, a, b)
    }
    pub fn slt(&mut self, a: Term, b: Term) -> Term {
        self.bin(BinKind::Slt, a, b)
    }
    pub fn eq(&mut self, a: Term, b: Term) -> Term {
        self.bin(BinKind::Eq, a, b)
    }

    /// Bitwise complement.
    pub fn not(&mut self, a: Term) -> Term {
        if let Some(c) = self.as_const(a) {
            return self.intern(Node::Const(BvConst::new(c.width(), !c.bits())));
        }
        self.intern(Node::Not(a))
    }

    pub fn sext(&mut self, a: Term, to: u32) -> Term {
        assert!(to >= self.width(a), "sext must widen");
        self.intern(Node::Extend {
            a,
            to,
            signed: true,
        })
    }

    pub fn zext(&mut self, a: Term, to: u32) -> Term {
        assert!(to >= self.width(a), "zext must widen");
        self.intern(Node::Extend {
            a,
            to,
            signed: false,
        })
    }

    pub fn extract(&mut self, a: Term, hi: u32, lo: u32) -> Term {
        assert!(hi >= lo && hi < self.width(a), "extract out of range");
        self.intern(Node::Extract { a, hi, lo })
    }

    /// Evaluate against a **complete** model (022 §3.1).
    ///
    /// Written from the SMT-LIB standard. A missing variable is an error rather than a
    /// default: a default would let a wrong model validate, which is the one thing this
    /// function exists to prevent.
    pub fn eval(&self, m: &Model, t: Term) -> Result<BvConst, EvalError> {
        Ok(match &self.nodes[t.0 as usize] {
            Node::Const(c) => *c,
            Node::Var(v, s) => m
                .get(*v)
                .ok_or_else(|| EvalError(format!("{v:?} has no value in the model")))
                .and_then(|c| {
                    if c.width() == s.width() {
                        Ok(c)
                    } else {
                        Err(EvalError(format!(
                            "{v:?} width {} != {}",
                            c.width(),
                            s.width()
                        )))
                    }
                })?,
            Node::Bin(k, a, b) => fold(*k, self.eval(m, *a)?, self.eval(m, *b)?),
            Node::Not(a) => {
                let v = self.eval(m, *a)?;
                BvConst::new(v.width(), !v.bits())
            }
            Node::Extend { a, to, signed } => {
                let v = self.eval(m, *a)?;
                if *signed {
                    BvConst::new(*to, v.signed() as u128)
                } else {
                    BvConst::new(*to, v.bits())
                }
            }
            Node::Extract { a, hi, lo } => {
                let v = self.eval(m, *a)?;
                BvConst::new(hi - lo + 1, v.bits() >> lo)
            }
        })
    }

    pub fn eval_ground(&self, t: Term) -> Result<BvConst, EvalError> {
        self.eval(&Model::new(), t)
    }

    pub fn eval_ground_bool(&self, t: Term) -> Result<bool, EvalError> {
        Ok(self.eval_ground(t)?.bits() != 0)
    }
}

/// SMT-LIB semantics for a binary operator.
///
/// **The zero cases are not uniform**, verified against z3 4.8.12: `bvudiv x 0` and
/// `bvsdiv x 0` for non-negative `x` give all ones, `bvsdiv x 0` for negative `x` gives
/// 1, and `bvurem`/`bvsrem` by zero give back the dividend. A uniform "all ones" rule —
/// which the spec itself had until z3 was asked — is wrong for three of the four.
fn fold(k: BinKind, x: BvConst, y: BvConst) -> BvConst {
    let w = x.width();
    let b = |v: bool| BvConst::new(1, v as u128);
    match k {
        BinKind::Add => BvConst::new(w, x.bits().wrapping_add(y.bits())),
        BinKind::Mul => BvConst::new(w, x.bits().wrapping_mul(y.bits())),
        BinKind::And => BvConst::new(w, x.bits() & y.bits()),
        BinKind::Or => BvConst::new(w, x.bits() | y.bits()),
        BinKind::Xor => BvConst::new(w, x.bits() ^ y.bits()),
        BinKind::UDiv => {
            if y.bits() == 0 {
                BvConst::all_ones(w)
            } else {
                BvConst::new(w, x.bits() / y.bits())
            }
        }
        BinKind::SDiv => {
            if y.bits() == 0 {
                // -1 for a non-negative dividend, 1 for a negative one.
                if x.is_negative() {
                    BvConst::new(w, 1)
                } else {
                    BvConst::all_ones(w)
                }
            } else {
                BvConst::new(w, x.signed().wrapping_div(y.signed()) as u128)
            }
        }
        BinKind::URem => {
            if y.bits() == 0 {
                x
            } else {
                BvConst::new(w, x.bits() % y.bits())
            }
        }
        BinKind::SRem => {
            if y.bits() == 0 {
                x
            } else {
                BvConst::new(w, x.signed().wrapping_rem(y.signed()) as u128)
            }
        }
        // Shifts by >= width yield 0 (or all ones for an arithmetic shift of a negative
        // value). x86 masks the count instead; 070 §1.1 routes around the divergence.
        BinKind::Shl => {
            if y.bits() >= w as u128 {
                BvConst::zero(w)
            } else {
                BvConst::new(w, x.bits() << y.bits())
            }
        }
        BinKind::LShr => {
            if y.bits() >= w as u128 {
                BvConst::zero(w)
            } else {
                BvConst::new(w, x.bits() >> y.bits())
            }
        }
        BinKind::AShr => {
            if y.bits() >= w as u128 {
                if x.is_negative() {
                    BvConst::all_ones(w)
                } else {
                    BvConst::zero(w)
                }
            } else {
                BvConst::new(w, (x.signed() >> y.bits()) as u128)
            }
        }
        BinKind::Ult => b(x.bits() < y.bits()),
        BinKind::Slt => b(x.signed() < y.signed()),
        BinKind::Eq => b(x.bits() == y.bits()),
    }
}
