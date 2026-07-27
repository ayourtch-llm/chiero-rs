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

    /// Decompose a term into a §3.2 atom, or `None` if it is outside the fragment.
    ///
    /// The fragment is a conjunction of comparison/equality atoms. A disjunction, a
    /// negation, or a bare non-predicate term is outside it — and being outside it means
    /// `Unknown`, never `Unsat`, because the domain's reasoning is only known sound
    /// within it.
    pub fn as_atom(&self, t: Term) -> Option<Atom> {
        match &self.nodes[t.0 as usize] {
            Node::Bin(k, a, b) if k.is_predicate() => Some(Atom {
                kind: *k,
                lhs: *a,
                rhs: *b,
            }),
            _ => None,
        }
    }

    /// Recognize `v & mask`, the shape a known-bits fact comes in.
    pub fn as_var_and_mask(&self, t: Term) -> Option<(VarId, u128)> {
        match &self.nodes[t.0 as usize] {
            Node::Bin(BinKind::And, a, b) => match (self.var_id(*a), self.as_const(*b)) {
                (Some(v), Some(m)) => Some((v, m.bits())),
                _ => match (self.as_const(*a), self.var_id(*b)) {
                    (Some(m), Some(v)) => Some((v, m.bits())),
                    _ => None,
                },
            },
            _ => None,
        }
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

/// Why a query could not be decided (022 §2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnknownReason {
    /// Outside the fragment `solver-lite` may answer `Unsat` over (022 §3.2).
    Incomplete(&'static str),
    ResourceLimit,
    BackendError(String),
}

/// **Three-valued, always.** Code that matches `Sat`/`Unsat` and treats the remainder as
/// one of them is a bug; the third arm is mandatory (022 §2).
#[derive(Clone, Debug)]
pub enum CheckResult {
    Sat(Model),
    Unsat,
    Unknown(UnknownReason),
}

pub trait Solver {
    fn assert(&mut self, t: Term);
    fn push(&mut self);
    fn pop(&mut self, n: u32);
    fn check(&mut self, a: &mut TermArena, assumptions: &[Term]) -> CheckResult;
}

/// Tier 1: rewriting, an interval + known-bits product domain, deliberately incomplete.
#[derive(Debug, Default)]
pub struct SolverLite {
    asserted: Vec<Term>,
    scopes: Vec<usize>,
}

impl SolverLite {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Solver for SolverLite {
    fn assert(&mut self, t: Term) {
        self.asserted.push(t);
    }

    fn push(&mut self) {
        self.scopes.push(self.asserted.len());
    }

    fn pop(&mut self, n: u32) {
        for _ in 0..n {
            if let Some(mark) = self.scopes.pop() {
                self.asserted.truncate(mark);
            }
        }
    }

    fn check(&mut self, a: &mut TermArena, assumptions: &[Term]) -> CheckResult {
        let all: Vec<Term> = self
            .asserted
            .iter()
            .chain(assumptions.iter())
            .copied()
            .collect();

        // §3.2: `Unsat` is only permitted over a conjunction of atoms. Anything else —
        // a disjunction, a nested `Ite`, a non-atomic assertion — leaves the fragment,
        // and the answer is `Unknown`. A propagator that descended into an `or` and
        // applied both sides would report a false `Unsat` for a satisfiable formula.
        let mut atoms = Vec::new();
        for t in &all {
            match a.as_atom(*t) {
                Some(at) => atoms.push(at),
                None => {
                    return CheckResult::Unknown(UnknownReason::Incomplete(
                        "assertion is outside the conjunction-of-atoms fragment",
                    ));
                }
            }
        }

        let mut dom = Domains::default();
        match dom.propagate(a, &atoms) {
            Propagation::Empty => CheckResult::Unsat,
            Propagation::Unsupported(why) => CheckResult::Unknown(UnknownReason::Incomplete(why)),
            Propagation::Fixpoint => {
                // A candidate model is only an answer once it has been evaluated
                // against every assertion (022 §3.1). Search is allowed to be
                // incomplete; it is not allowed to be wrong.
                match dom.candidate(a) {
                    Some(m)
                        if all
                            .iter()
                            .all(|t| a.eval(&m, *t).map(|v| v.bits() != 0) == Ok(true)) =>
                    {
                        CheckResult::Sat(m)
                    }
                    _ => CheckResult::Unknown(UnknownReason::Incomplete(
                        "no candidate model survived validation",
                    )),
                }
            }
        }
    }
}

/// An atom in the §3.2 fragment: a comparison or equality between a variable-rooted
/// expression and a constant, or between two such.
#[derive(Copy, Clone, Debug)]
pub struct Atom {
    pub kind: BinKind,
    pub lhs: Term,
    pub rhs: Term,
}

enum Propagation {
    Fixpoint,
    Empty,
    Unsupported(&'static str),
}

/// The interval + known-bits product domain, per variable.
#[derive(Clone, Debug)]
struct VarDomain {
    width: u32,
    /// Unsigned bounds, inclusive.
    lo: u128,
    hi: u128,
    /// Bits known to be zero, and bits known to be one.
    zeros: u128,
    ones: u128,
}

impl VarDomain {
    fn top(width: u32) -> Self {
        VarDomain {
            width,
            lo: 0,
            hi: mask(width),
            zeros: 0,
            ones: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.lo > self.hi || (self.zeros & self.ones) != 0
    }

    /// Smallest value consistent with both components, or `None` if there is none.
    fn least(&self) -> Option<u128> {
        (self.lo..=self.hi).find(|&v| v & self.zeros == 0 && v & self.ones == self.ones)
    }
}

#[derive(Default, Debug)]
struct Domains {
    vars: IndexMap<VarId, VarDomain>,
}

impl Domains {
    fn dom(&mut self, v: VarId, width: u32) -> &mut VarDomain {
        self.vars.entry(v).or_insert_with(|| VarDomain::top(width))
    }

    /// Propagate to fixpoint. Every transfer here is **wrap-safe**: a transfer that
    /// cannot represent a wrapped result widens to ⊤ rather than saturating.
    ///
    /// Saturating is the classic false-`Unsat` source. `x >u 250 ∧ y == x+10 ∧ y <u 10`
    /// is satisfiable at `x = 0xfb` (verified against z3), but a saturating transfer
    /// computes `x+10 ∈ [255,255]`, intersects with `[0,9]`, and reports empty.
    fn propagate(&mut self, a: &TermArena, atoms: &[Atom]) -> Propagation {
        for _ in 0..16 {
            let mut changed = false;
            for at in atoms {
                match self.apply(a, at) {
                    Ok(c) => changed |= c,
                    Err(why) => return Propagation::Unsupported(why),
                }
            }
            if self.vars.values().any(|d| d.is_empty()) {
                return Propagation::Empty;
            }
            if !changed {
                break;
            }
        }
        Propagation::Fixpoint
    }

    fn apply(&mut self, a: &TermArena, at: &Atom) -> Result<bool, &'static str> {
        let lc = a.as_const(at.lhs);
        let rc = a.as_const(at.rhs);
        let lv = a.var_id(at.lhs);
        let rv = a.var_id(at.rhs);

        // `v OP const` and `const OP v` are the shapes the domain understands. An atom
        // over a non-variable, non-constant expression (an addition, a mask) is not
        // refuted here — it is simply not used to narrow, which is incompleteness
        // rather than unsoundness.
        let mut changed = false;
        match (lv, rc, lc, rv) {
            (Some(v), Some(k), _, _) => {
                let w = k.width();
                let d = self.dom(v, w);
                changed |= narrow(d, at.kind, k.bits(), false);
            }
            (_, _, Some(k), Some(v)) => {
                let w = k.width();
                let d = self.dom(v, w);
                changed |= narrow(d, at.kind, k.bits(), true);
            }
            _ => {
                // `masked == k` where masked is `v & m`: a known-bits fact.
                if at.kind == BinKind::Eq
                    && let Some(k) = rc
                    && let Some((v, m)) = a.as_var_and_mask(at.lhs)
                {
                    let w = k.width();
                    let d = self.dom(v, w);
                    // Bits selected by the mask are pinned; the rest stay unknown.
                    let want_ones = k.bits() & m;
                    let want_zeros = !k.bits() & m & mask(w);
                    if d.ones | want_ones != d.ones || d.zeros | want_zeros != d.zeros {
                        d.ones |= want_ones;
                        d.zeros |= want_zeros;
                        changed = true;
                    }
                    // `k` having a bit set outside the mask is an immediate
                    // contradiction: `v & m` can never produce it.
                    if k.bits() & !m & mask(w) != 0 {
                        d.zeros = mask(w);
                        d.ones = mask(w);
                        changed = true;
                    }
                }
            }
        }
        Ok(changed)
    }

    /// A candidate assignment: the least value in each variable's domain. Variables that
    /// were never constrained get 0, so the model is **complete** (022 §2).
    fn candidate(&self, a: &TermArena) -> Option<Model> {
        let mut m = Model::new();
        for (v, d) in &self.vars {
            m.set(*v, BvConst::new(d.width, d.least()?));
        }
        for (i, (_, sort)) in a.vars.iter().enumerate() {
            let v = VarId(i as u32);
            if m.get(v).is_none() {
                m.set(v, BvConst::zero(sort.width()));
            }
        }
        Some(m)
    }
}

/// Narrow one variable's domain by `v OP k` (or `k OP v` when `flipped`).
fn narrow(d: &mut VarDomain, kind: BinKind, k: u128, flipped: bool) -> bool {
    let (lo0, hi0, z0, o0) = (d.lo, d.hi, d.zeros, d.ones);
    match (kind, flipped) {
        (BinKind::Ult, false) => d.hi = d.hi.min(k.saturating_sub(1)),
        (BinKind::Ult, true) => d.lo = d.lo.max(k.saturating_add(1)),
        (BinKind::Eq, _) => {
            d.lo = d.lo.max(k);
            d.hi = d.hi.min(k);
            d.ones |= k;
            d.zeros |= !k & mask(d.width);
        }
        // Signed comparison is not modeled by an unsigned interval; leaving it alone is
        // incompleteness, whereas treating it as unsigned would be unsound.
        _ => {}
    }
    if kind == BinKind::Ult && k == 0 && !flipped {
        // `v <u 0` is unsatisfiable.
        d.lo = 1;
        d.hi = 0;
    }
    (d.lo, d.hi, d.zeros, d.ones) != (lo0, hi0, z0, o0)
}

/// Tier 2: an SMT-LIB2 solver spoken to over a **subprocess** (022 §4).
///
/// A subprocess, not FFI, is the whole point: chiero never links a solver, builds with
/// `--no-default-features`, and runs when none is installed. Discovery is a runtime fact.
#[derive(Debug)]
pub struct SmtLib {
    path: std::path::PathBuf,
}

impl SmtLib {
    /// `$CHIERO_SMT_SOLVER`, then z3, cvc5, bitwuzla on `PATH`.
    pub fn discover() -> Option<SmtLib> {
        todo!("green")
    }
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

#[derive(Clone, Debug, Default)]
pub struct SolverStats {
    pub backend_calls: u64,
    pub cache_entries: usize,
    pub tier1_unknown: u64,
}

/// Tier 1, escalating to tier 2 on `Unknown`, with the caches of 022 §6.
#[derive(Debug, Default)]
pub struct TieredSolver {
    asserted: Vec<Term>,
    scopes: Vec<usize>,
    backend: Option<SmtLib>,
    paranoid: bool,
    stats: SolverStats,
    cache: IndexMap<(Vec<u32>, Vec<u32>), bool>,
}

impl TieredSolver {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_backend(_b: SmtLib) -> Self {
        todo!("green")
    }
    pub fn set_paranoid(&mut self, _on: bool) {
        todo!("green")
    }
    pub fn stats(&self) -> &SolverStats {
        &self.stats
    }
}

impl Solver for TieredSolver {
    fn assert(&mut self, _t: Term) {
        todo!("green")
    }
    fn push(&mut self) {
        todo!("green")
    }
    fn pop(&mut self, _n: u32) {
        todo!("green")
    }
    fn check(&mut self, _a: &mut TermArena, _assumptions: &[Term]) -> CheckResult {
        todo!("green")
    }
}
