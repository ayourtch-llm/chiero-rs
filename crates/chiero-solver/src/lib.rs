//! The solver. See `docs/specs/022-solver.md`.
//!
//! **Knows nothing about C and nothing about CIR** (001 §2). Its vocabulary is sorts and
//! terms, which keeps its test suite pure constraint solving and stops C semantics
//! leaking into a layer that must be trustworthy.

use indexmap::IndexMap;

/// The widest bit-vector a `BvConst` payload can hold.
///
/// 020 permits `Int(512)` for AVX-512, so this is a real boundary rather than a
/// theoretical one — `Const::Wide` is where wider values eventually live.
pub const MAX_BV_BITS: u32 = 128;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sort {
    Bool,
    BitVec(u32),
    /// `(Array (_ BitVec idx) (_ BitVec elem))` — the representation 021 §3 promotes a
    /// memory object to when a symbolic offset cannot be pinned to a small set.
    Array {
        idx: u32,
        elem: u32,
    },
}

impl Sort {
    /// The width of a *value* of this sort. An array has no scalar width; asking for one
    /// is a bug in the caller, so it is 0 rather than a plausible-looking number.
    pub fn width(self) -> u32 {
        match self {
            Sort::Array { .. } => 0,
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
        assert!(
            width > 0 && width <= MAX_BV_BITS,
            "width {width} out of range"
        );
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
    Extend {
        a: Term,
        to: u32,
        signed: bool,
    },
    Extract {
        a: Term,
        hi: u32,
        lo: u32,
    },
    /// SMT-LIB `concat`: the **first** argument occupies the high bits. 021 contract 5
    /// depends on this — a four-byte read with two concrete and two symbolic bytes is a
    /// `Concat`, not a promotion to array theory.
    Concat {
        hi: Term,
        lo: Term,
    },
    /// `ite` over a one-bit condition. 021 §3.1's conditional write is
    /// `ite(off == k, val, old)` per candidate byte; without this term `InitBit::Cond` is
    /// a marker with no guard behind it.
    Ite {
        c: Term,
        t: Term,
        f: Term,
    },
    /// An array whose every index holds the same value — the base a promoted object
    /// starts from (021 §3).
    ArrayConst {
        idx: u32,
        val: Term,
    },
    Select {
        a: Term,
        i: Term,
    },
    Store {
        a: Term,
        i: Term,
        v: Term,
    },
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
    /// Any assigned value, as a signed integer.
    ///
    /// For a witness the *identity* of the variable does not matter — what a reader needs
    /// is a concrete number to plug in. A model with no variables at all has none, which
    /// the caller must handle rather than fabricate.
    pub fn any_value_i64(&self) -> Option<i64> {
        self.values.values().next().map(|c| c.signed() as i64)
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
            Node::Concat { hi, lo } => self.width(*hi) + self.width(*lo),
            Node::Ite { t, .. } => self.width(*t),
            // An array is not a scalar; only `Select` yields a width.
            Node::ArrayConst { .. } | Node::Store { .. } => 0,
            Node::Select { a, .. } => self.elem_width(*a),
        }
    }

    /// The element width of an array-sorted term.
    fn elem_width(&self, t: Term) -> u32 {
        match &self.nodes[t.0 as usize] {
            Node::Var(_, Sort::Array { elem, .. }) => *elem,
            Node::ArrayConst { val, .. } => self.width(*val),
            Node::Store { a, .. } => self.elem_width(*a),
            _ => 0,
        }
    }

    /// A fresh array variable, `(Array (_ BitVec idx) (_ BitVec elem))`.
    pub fn array_var(&mut self, idx: u32, elem: u32, name: &str) -> Term {
        self.var(Sort::Array { idx, elem }, name)
    }

    /// An array whose every index holds `val`.
    pub fn array_const(&mut self, idx: u32, elem: u32, val: u128) -> Term {
        let v = self.bv(elem, val);
        self.intern(Node::ArrayConst { idx, val: v })
    }

    pub fn store(&mut self, a: Term, i: Term, v: Term) -> Term {
        self.intern(Node::Store { a, i, v })
    }

    /// `select`, folding only when the index comparison is **decidable at construction**.
    ///
    /// Folding a symbolic index to the underlying array's default would silently decide
    /// `i == j`, which is exactly the question promotion exists to hand to the solver.
    pub fn select(&mut self, arr: Term, i: Term) -> Term {
        let mut cur = arr;
        loop {
            match self.nodes[cur.0 as usize] {
                Node::Store { a, i: si, v } => {
                    // Syntactic identity is sound and free: terms are hash-consed, so
                    // `si == i` means the *same* index, symbolic or not. Without it,
                    // `v[i] = x; use v[i]` — the commonest shape there is — hands the
                    // solver a question it does not need.
                    if si == i {
                        return v;
                    }
                    match (self.as_const(si), self.as_const(i)) {
                        (Some(x), Some(y)) if x.bits() == y.bits() => return v,
                        // Both concrete and different: this store is irrelevant, read on.
                        (Some(_), Some(_)) => cur = a,
                        // Either side symbolic: the answer depends on a comparison the
                        // solver owns.
                        _ => break,
                    }
                }
                Node::ArrayConst { val, .. } => return val,
                _ => break,
            }
        }
        self.intern(Node::Select { a: cur, i })
    }

    /// SMT-LIB `concat`, folding when both sides are constant.
    /// `concat`, or `None` if the result would exceed the payload width.
    ///
    /// `BvConst` is 128 bits, so a 17-byte assembly either tripped the width assert
    /// inside the caller or built the term and deferred the panic to evaluation.
    /// Refusing is the honest answer; 020 permits `Int(512)` and the eventual home for
    /// wider values is `Const::Wide`.
    pub fn try_concat(&mut self, hi: Term, lo: Term) -> Option<Term> {
        (self.width(hi) + self.width(lo) <= MAX_BV_BITS).then(|| self.concat(hi, lo))
    }

    pub fn concat(&mut self, hi: Term, lo: Term) -> Term {
        if let (Some(x), Some(y)) = (self.as_const(hi), self.as_const(lo)) {
            let w = x.width() + y.width();
            let v = (x.bits() << y.width()) | y.bits();
            return self.bv(w, v);
        }
        self.intern(Node::Concat { hi, lo })
    }

    /// `ite`, folding a constant condition outright so a guard the solver already decided
    /// costs nothing downstream (022 §2 folds at construction for exactly this reason).
    pub fn ite(&mut self, c: Term, t: Term, f: Term) -> Term {
        assert_eq!(self.width(c), 1, "an ite condition is one bit");
        assert_eq!(
            self.width(t),
            self.width(f),
            "both ite branches must have the same width"
        );
        if let Some(k) = self.as_const(c) {
            return if k.bits() != 0 { t } else { f };
        }
        // Both branches identical: the condition cannot matter.
        if t == f {
            return t;
        }
        self.intern(Node::Ite { c, t, f })
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

    /// Render a term as SMT-LIB2.
    ///
    /// The serialization is a first-class artifact: 022 §4 requires `--dump-queries`, and
    /// a disagreement with the backend is reported by handing over the exact script.
    pub fn to_smtlib(&self, t: Term) -> String {
        // Nodes reached more than once are bound with `let` instead of written again.
        // Without this a shared DAG expands into a tree: 22 shared nodes came to 54 MB,
        // and 021 §3's init array — one `store` per bit — would be astronomically worse.
        let order = self.postorder(t);
        let mut refs: IndexMap<Term, u32> = IndexMap::new();
        for &n in &order {
            for c in self.children(n) {
                *refs.entry(c).or_insert(0) += 1;
            }
        }
        // Above a small size, **every** non-trivial node is bound. Sharing alone is not
        // enough: a long *unshared* chain — which is exactly what one `store` per bit
        // produces — has refcount 1 everywhere, so nothing would be bound and the
        // renderer would recurse its whole length. Binding flattens it to depth one.
        // Below the threshold only genuinely shared nodes are bound, so a small query
        // reads as it always did.
        const FLATTEN_ABOVE: usize = 8;
        let flatten = order.len() > FLATTEN_ABOVE;
        let mut names: IndexMap<Term, String> = IndexMap::new();
        for (i, &n) in order.iter().enumerate() {
            // Constants and variables are already their own shortest form; binding them
            // would make the text longer, not shorter.
            let trivial = matches!(self.nodes[n.0 as usize], Node::Const(_) | Node::Var(_, _));
            let shared = refs.get(&n).copied().unwrap_or(0) > 1;
            if !trivial && (shared || flatten) {
                names.insert(n, format!("s{i}"));
            }
        }
        // Nested rather than a single flat binding: SMT-LIB `let` binds in *parallel*, so
        // a later binding cannot see an earlier one in the same group.
        let mut out = String::new();
        let mut closers = 0usize;
        for &n in &order {
            if let Some(name) = names.get(&n) {
                out.push_str(&format!("(let (({name} {})) ", self.render(n, &names)));
                closers += 1;
            }
        }
        out.push_str(&self.render_root(t, &names));
        for _ in 0..closers {
            out.push(')');
        }
        out
    }

    /// Post-order over the DAG, each node once. **Iterative**: a 2000-element store chain
    /// overflowed the stack when this recursed.
    fn postorder(&self, root: Term) -> Vec<Term> {
        let mut seen: Vec<bool> = vec![false; self.nodes.len()];
        let mut out = Vec::new();
        let mut stack = vec![(root, false)];
        while let Some((n, expanded)) = stack.pop() {
            if expanded {
                out.push(n);
                continue;
            }
            if seen[n.0 as usize] {
                continue;
            }
            seen[n.0 as usize] = true;
            stack.push((n, true));
            for c in self.children(n) {
                if !seen[c.0 as usize] {
                    stack.push((c, false));
                }
            }
        }
        out
    }

    fn children(&self, t: Term) -> Vec<Term> {
        match &self.nodes[t.0 as usize] {
            Node::Const(_) | Node::Var(_, _) => vec![],
            Node::Not(a) | Node::Extend { a, .. } | Node::Extract { a, .. } => vec![*a],
            Node::Bin(_, a, b) | Node::Concat { hi: a, lo: b } | Node::Select { a, i: b } => {
                vec![*a, *b]
            }
            Node::Ite { c, t, f } | Node::Store { a: c, i: t, v: f } => vec![*c, *t, *f],
            Node::ArrayConst { val, .. } => vec![*val],
        }
    }

    /// The root, which is never itself replaced by a name.
    fn render_root(&self, t: Term, names: &IndexMap<Term, String>) -> String {
        match names.get(&t) {
            Some(n) => n.clone(),
            None => self.render(t, names),
        }
    }

    /// One node, with any *bound* child written as its name.
    fn render(&self, t: Term, names: &IndexMap<Term, String>) -> String {
        self.emit(t, names)
    }

    fn sub(&self, t: Term, names: &IndexMap<Term, String>) -> String {
        match names.get(&t) {
            Some(n) => n.clone(),
            None => self.emit(t, names),
        }
    }

    fn emit(&self, t: Term, names: &IndexMap<Term, String>) -> String {
        match &self.nodes[t.0 as usize] {
            Node::Const(c) => {
                // **Always a bitvector.** Guessing that every width-1 constant is a
                // `Bool` produced `(= v0_flag true)` for a one-bit *variable*,
                // `(concat true v0_b)` inside a concat, and `(ite … true false)` where a
                // one-bit vector was wanted — three sort errors from one guess. A one-bit
                // bitvector is what `LoadBits` of a `u32 flag:1` yields, so the case is
                // ordinary, not exotic. Coercion now happens where the *context* knows
                // which sort it needs.
                format!("(_ bv{} {})", c.bits(), c.width())
            }
            Node::Var(v, _) => smt_name(v, &self.vars[v.0 as usize].0),
            // Same distinction: `not` over a `Bool`, `bvnot` over a vector.
            Node::Not(a) if self.smt_is_bool(*a) => format!("(not {})", self.sub_named(names, *a)),
            Node::Extend { a, to, signed } if self.smt_is_bool(*a) => {
                let by = to - 1;
                let op = if *signed {
                    "sign_extend"
                } else {
                    "zero_extend"
                };
                format!("((_ {op} {by}) {})", self.smt_bv_named(names, *a))
            }
            Node::Not(a) => format!("(bvnot {})", self.sub_named(names, *a)),
            Node::Extend { a, to, signed } => {
                let by = to - self.width(*a);
                let op = if *signed {
                    "sign_extend"
                } else {
                    "zero_extend"
                };
                format!("((_ {op} {by}) {})", self.sub_named(names, *a))
            }
            Node::Extract { a, hi, lo } => {
                format!("((_ extract {hi} {lo}) {})", self.sub_named(names, *a))
            }
            Node::Concat { hi, lo } => {
                format!(
                    "(concat {} {})",
                    self.smt_bv_named(names, *hi),
                    self.smt_bv_named(names, *lo)
                )
            }
            Node::ArrayConst { idx, val } => {
                // `as const` needs the full sort annotation, since the element sort is
                // not inferable from the value alone in every SMT-LIB dialect.
                format!(
                    "((as const (Array (_ BitVec {idx}) (_ BitVec {}))) {})",
                    self.width(*val),
                    self.sub_named(names, *val)
                )
            }
            Node::Select { a, i } => {
                format!(
                    "(select {} {})",
                    self.sub_named(names, *a),
                    self.sub_named(names, *i)
                )
            }
            Node::Store { a, i, v } => format!(
                "(store {} {} {})",
                self.sub_named(names, *a),
                self.sub_named(names, *i),
                self.smt_bv_named(names, *v)
            ),
            Node::Ite { c, t, f } => {
                // A predicate is width-1 in the arena but **`Bool` in SMT-LIB**, so it is
                // already a legal `ite` condition; a genuine one-bit *vector* has to be
                // compared against `#b1`. Emitting one form for both is a sort error the
                // backend rejects outright.
                // The branches keep whatever sort they already have, but they must
                // agree with each other — a `Bool` branch beside a vector branch is the
                // same sort error one level down.
                let bool_branches = self.smt_is_bool(*t) && self.smt_is_bool(*f);
                let (tb, fb) = if bool_branches {
                    (self.sub_named(names, *t), self.sub_named(names, *f))
                } else {
                    (self.smt_bv_named(names, *t), self.smt_bv_named(names, *f))
                };
                format!("(ite {} {tb} {fb})", self.smt_bool_named(names, *c))
            }
            // `and`/`or`/`xor` over two `Bool`s are the *boolean* connectives, not the
            // bitvector ones. `(bvor (bvult …) (bvult …))` is a sort error the backend
            // rejects outright — and it was reachable from any query containing a
            // disjunction of comparisons, which is most of them.
            // A connective is boolean only when **both** sides already are. A mixed
            // pair — one predicate, one one-bit vector — was falling through to `bvand`
            // over a `Bool`, so the vector side is coerced up instead.
            Node::Bin(k, a, b) if matches!(k, BinKind::And | BinKind::Or | BinKind::Xor) => {
                let op = match k {
                    BinKind::And => "and",
                    BinKind::Or => "or",
                    _ => "xor",
                };
                if self.smt_is_bool(*a) || self.smt_is_bool(*b) {
                    format!(
                        "({op} {} {})",
                        self.smt_bool_named(names, *a),
                        self.smt_bool_named(names, *b)
                    )
                } else {
                    let bv = match k {
                        BinKind::And => "bvand",
                        BinKind::Or => "bvor",
                        _ => "bvxor",
                    };
                    format!(
                        "({bv} {} {})",
                        self.sub_named(names, *a),
                        self.sub_named(names, *b)
                    )
                }
            }
            Node::Bin(k, a, b) => {
                let (x, y) = if *k == BinKind::Eq && self.smt_is_bool(*a) && self.smt_is_bool(*b) {
                    // `=` is sort-polymorphic, so two Bools compare directly.
                    (self.sub_named(names, *a), self.sub_named(names, *b))
                } else {
                    (self.smt_bv_named(names, *a), self.smt_bv_named(names, *b))
                };
                let op = match k {
                    BinKind::Add => "bvadd",
                    BinKind::Mul => "bvmul",
                    BinKind::And => "bvand",
                    BinKind::Or => "bvor",
                    BinKind::Xor => "bvxor",
                    BinKind::UDiv => "bvudiv",
                    BinKind::SDiv => "bvsdiv",
                    BinKind::URem => "bvurem",
                    BinKind::SRem => "bvsrem",
                    BinKind::Shl => "bvshl",
                    BinKind::LShr => "bvlshr",
                    BinKind::AShr => "bvashr",
                    BinKind::Ult => "bvult",
                    BinKind::Slt => "bvslt",
                    BinKind::Eq => "=",
                };
                format!("({op} {x} {y})")
            }
        }
    }

    fn sub_named(&self, names: &IndexMap<Term, String>, t: Term) -> String {
        self.sub(t, names)
    }

    /// Render `t` where a `Bool` is required, coercing a one-bit vector if need be.
    fn smt_bool_named(&self, names: &IndexMap<Term, String>, t: Term) -> String {
        if self.smt_is_bool(t) {
            self.sub(t, names)
        } else {
            format!("(= {} #b1)", self.sub(t, names))
        }
    }

    /// Render `t` where a bit-vector is required, coercing a `Bool` if need be.
    fn smt_bv_named(&self, names: &IndexMap<Term, String>, t: Term) -> String {
        if self.smt_is_bool(t) {
            format!("(ite {} #b1 #b0)", self.sub(t, names))
        } else {
            self.sub(t, names)
        }
    }

    /// Whether this term translates to an SMT-LIB **`Bool`** rather than a bit-vector.
    ///
    /// The arena gives predicates width 1, which is convenient for evaluation and wrong
    /// for translation: `(= x y)` is a `Bool` and `#b1` is a one-bit vector, and the two
    /// are not interchangeable in `ite`, `not`, or anywhere else the backend type-checks.
    fn smt_is_bool(&self, t: Term) -> bool {
        match &self.nodes[t.0 as usize] {
            // A `Bool` variable is declared as `Bool`, so it must be *treated* as one —
            // otherwise every use is coerced to a one-bit vector and the backend rejects
            // the comparison.
            Node::Var(_, Sort::Bool) => true,
            Node::Bin(BinKind::And | BinKind::Or | BinKind::Xor, a, b) => {
                self.smt_is_bool(*a) || self.smt_is_bool(*b)
            }
            Node::Ite { t, f, .. } => self.smt_is_bool(*t) && self.smt_is_bool(*f),
            Node::Bin(k, _, _) => k.is_predicate(),
            Node::Not(a) => self.smt_is_bool(*a),
            _ => false,
        }
    }

    /// Every variable a term mentions, in declaration order.
    /// Iterative, for the same reason `postorder` is: a deep term aborts the process
    /// otherwise, and this runs on *every* term immediately before serialization — so
    /// making only the serializer iterative moved the failure rather than removing it.
    pub fn vars_of(&self, t: Term, out: &mut Vec<VarId>) {
        let mut seen: Vec<bool> = vec![false; self.nodes.len()];
        let mut stack = vec![t];
        while let Some(n) = stack.pop() {
            if seen[n.0 as usize] {
                continue;
            }
            seen[n.0 as usize] = true;
            if let Node::Var(v, _) = &self.nodes[n.0 as usize]
                && !out.contains(v)
            {
                out.push(*v);
            }
            for c in self.children(n) {
                stack.push(c);
            }
        }
    }

    #[allow(dead_code)]
    fn vars_of_recursive(&self, t: Term, out: &mut Vec<VarId>) {
        match &self.nodes[t.0 as usize] {
            Node::Var(v, _) => {
                if !out.contains(v) {
                    out.push(*v);
                }
            }
            Node::Const(_) => {}
            Node::Not(a) | Node::Extend { a, .. } | Node::Extract { a, .. } => {
                self.vars_of(*a, out)
            }
            Node::Bin(_, a, b) | Node::Concat { hi: a, lo: b } => {
                self.vars_of(*a, out);
                self.vars_of(*b, out);
            }
            Node::Ite { c, t, f } | Node::Store { a: c, i: t, v: f } => {
                self.vars_of(*c, out);
                self.vars_of(*t, out);
                self.vars_of(*f, out);
            }
            Node::ArrayConst { val, .. } => self.vars_of(*val, out),
            Node::Select { a, i } => {
                self.vars_of(*a, out);
                self.vars_of(*i, out);
            }
        }
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
    /// Iterative over the DAG, memoized. The recursive form aborted on a deep term, and
    /// this is on the **model-validation** path — so a `Sat` over a deep term would kill
    /// the process instead of validating, which is the one thing 022 §3 rests on.
    pub fn eval(&self, m: &Model, t: Term) -> Result<BvConst, EvalError> {
        let mut memo: Vec<Option<BvConst>> = vec![None; self.nodes.len()];
        for n in self.postorder(t) {
            // Array-sorted nodes have no scalar value of their own, and evaluating them
            // bottom-up would fail the whole term. `Select` walks the store chain itself,
            // reading the memo for each index and value.
            if self.is_array_sorted(n) {
                continue;
            }
            let v = self.eval_node(m, n, &memo)?;
            memo[n.0 as usize] = Some(v);
        }
        memo[t.0 as usize].ok_or_else(|| EvalError("term did not evaluate".into()))
    }

    fn is_array_sorted(&self, t: Term) -> bool {
        matches!(
            self.nodes[t.0 as usize],
            Node::ArrayConst { .. } | Node::Store { .. } | Node::Var(_, Sort::Array { .. })
        )
    }

    /// One node, with children already in `memo`.
    /// One node, with every child already in `memo` — so this is depth-1 and the pass
    /// above it is what walks the DAG.
    fn eval_node(
        &self,
        m: &Model,
        t: Term,
        memo: &[Option<BvConst>],
    ) -> Result<BvConst, EvalError> {
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
            Node::Bin(k, a, b) => fold(
                *k,
                memo[a.0 as usize].ok_or_else(|| EvalError("unevaluated child".into()))?,
                memo[b.0 as usize].ok_or_else(|| EvalError("unevaluated child".into()))?,
            ),
            Node::Not(a) => {
                let v = memo[a.0 as usize].ok_or_else(|| EvalError("unevaluated child".into()))?;
                BvConst::new(v.width(), !v.bits())
            }
            Node::Extend { a, to, signed } => {
                let v = memo[a.0 as usize].ok_or_else(|| EvalError("unevaluated child".into()))?;
                if *signed {
                    BvConst::new(*to, v.signed() as u128)
                } else {
                    BvConst::new(*to, v.bits())
                }
            }
            Node::Extract { a, hi, lo } => {
                let v = memo[a.0 as usize].ok_or_else(|| EvalError("unevaluated child".into()))?;
                BvConst::new(hi - lo + 1, v.bits() >> lo)
            }
            // The evaluator resolves a select by walking the store chain under the
            // model, which is what makes a promoted object's contents checkable without
            // a backend — the same independence 022 §3 rests the whole Sat story on.
            Node::Select { a, i } => {
                let want =
                    memo[i.0 as usize].ok_or_else(|| EvalError("unevaluated child".into()))?;
                let mut cur = *a;
                loop {
                    match &self.nodes[cur.0 as usize] {
                        Node::Store { a, i, v } => {
                            if memo[i.0 as usize]
                                .ok_or_else(|| EvalError("unevaluated child".into()))?
                                .bits()
                                == want.bits()
                            {
                                break memo[v.0 as usize]
                                    .ok_or_else(|| EvalError("unevaluated child".into()))?;
                            }
                            cur = *a;
                        }
                        Node::ArrayConst { val, .. } => {
                            break memo[val.0 as usize]
                                .ok_or_else(|| EvalError("unevaluated child".into()))?;
                        }
                        // An array *variable* has no model here; 022 §2's evaluator is
                        // total over bit-vectors, and arrays are the honest exception.
                        _ => {
                            return Err(EvalError(
                                "array variable is unassigned in this model".into(),
                            ));
                        }
                    }
                }
            }
            // Array-sorted terms have no scalar value of their own.
            Node::ArrayConst { .. } | Node::Store { .. } => {
                return Err(EvalError("array-sorted term has no scalar value".into()));
            }
            Node::Concat { hi, lo } => {
                let (x, y) = (
                    memo[hi.0 as usize].ok_or_else(|| EvalError("unevaluated child".into()))?,
                    memo[lo.0 as usize].ok_or_else(|| EvalError("unevaluated child".into()))?,
                );
                BvConst::new(x.width() + y.width(), (x.bits() << y.width()) | y.bits())
            }
            Node::Ite { c, t, f } => {
                if memo[c.0 as usize]
                    .ok_or_else(|| EvalError("unevaluated child".into()))?
                    .bits()
                    != 0
                {
                    memo[t.0 as usize].ok_or_else(|| EvalError("unevaluated child".into()))?
                } else {
                    memo[f.0 as usize].ok_or_else(|| EvalError("unevaluated child".into()))?
                }
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
#[derive(Clone, Debug)]
pub struct SmtLib {
    path: std::path::PathBuf,
}

impl SmtLib {
    /// `$CHIERO_SMT_SOLVER`, then z3, cvc5, bitwuzla on `PATH`.
    ///
    /// Discovery is a **runtime** fact. A Cargo feature cannot be conditionally enabled
    /// at runtime, so the backend is compiled in and simply finds nothing when no solver
    /// is installed — which is what lets the whole suite run without one (022 contract 2).
    pub fn discover() -> Option<SmtLib> {
        let candidates: Vec<String> = match std::env::var("CHIERO_SMT_SOLVER") {
            Ok(v) if !v.is_empty() => vec![v],
            _ => ["z3", "cvc5", "bitwuzla"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        for c in candidates {
            let path = std::path::PathBuf::from(&c);
            let found = if path.is_absolute() {
                path.exists().then_some(path)
            } else {
                std::env::var_os("PATH").and_then(|p| {
                    std::env::split_paths(&p)
                        .map(|d| d.join(&c))
                        .find(|f| f.is_file())
                })
            };
            if let Some(p) = found {
                return Some(SmtLib { path: p });
            }
        }
        None
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

/// A live backend process. Kept open across queries: 022 §4 requires real incremental
/// solving, and startup dominates short queries.
#[derive(Debug)]
struct Session {
    child: std::process::Child,
    /// Variables already declared to this process, so a restart knows what to redeclare.
    declared: Vec<VarId>,
}

impl Session {
    fn spawn(path: &std::path::Path) -> Option<Session> {
        let child = std::process::Command::new(path)
            .args(["-in", "-smt2"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let mut s = Session {
            child,
            declared: Vec::new(),
        };
        // **`ALL`.** The logic has to admit every term the arena can build, and each
        // narrower choice has excluded something in turn: `QF_BV` rejected arrays
        // outright, and `QF_ABV` accepts `Array` but still rejects `as const` — which is
        // the *base* of every promoted object, so all of array theory was unusable.
        // Naming a narrow logic buys nothing here and has now cost two rounds.
        s.send("(set-logic ALL)\n(set-option :produce-models true)\n")?;
        Some(s)
    }

    fn send(&mut self, text: &str) -> Option<()> {
        use std::io::Write;
        let stdin = self.child.stdin.as_mut()?;
        stdin.write_all(text.as_bytes()).ok()?;
        stdin.flush().ok()
    }

    /// Read one balanced S-expression or bare token from the process.
    ///
    /// A long-lived process means output must be framed rather than read to EOF, and
    /// parenthesis balance is the framing SMT-LIB2 gives us.
    fn read_answer(&mut self) -> Option<String> {
        // **An `(error …)` is not a verdict.** z3 prints it on *stdout* and then answers,
        // so framing the first parenthesized form as the result takes the error as the
        // answer, concludes the process died, and restarts it — leaving the real answer in
        // the pipe for the next query to misread. Errors are skipped and the read
        // continues, so the desync cannot happen.
        //
        // No emission currently produces an error, so no test reaches this — mutation
        // confirms it. It stays anyway: this exact desync happened when `as const` was
        // rejected under `QF_ABV`, and the next malformed emission should degrade to a
        // clean error rather than to another query's answer. Deleting a guard because the
        // bug it caught is currently fixed is how the bug comes back worse.
        let form = self.read_form()?;
        if form.trim_start().starts_with("(error") {
            // **Skipping the error and returning the next form is not a fix.** z3 prints
            // the error and then answers anyway, so accepting that answer turns a
            // *malformed script* into a confident verdict — which is worse than the
            // desync this replaced, because nothing looks wrong. The following form is
            // drained to keep the pipe aligned, and the query is reported as failed.
            let _ = self.read_form();
            return None;
        }
        Some(form)
    }

    fn read_form(&mut self) -> Option<String> {
        use std::io::Read;
        let out = self.child.stdout.as_mut()?;
        let mut buf = Vec::new();
        let mut depth = 0i32;
        let mut started = false;
        let mut byte = [0u8; 1];
        loop {
            if out.read(&mut byte).ok()? == 0 {
                return None; // the process died
            }
            let c = byte[0];
            buf.push(c);
            match c {
                b'(' => {
                    depth += 1;
                    started = true;
                }
                b')' => {
                    depth -= 1;
                    if started && depth == 0 {
                        return Some(String::from_utf8_lossy(&buf).into_owned());
                    }
                }
                b'\n' if !started && !buf.iter().all(|b| b.is_ascii_whitespace()) => {
                    return Some(String::from_utf8_lossy(&buf).trim().to_string());
                }
                _ => {}
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A name z3 will accept, unique per variable.
/// A sort in SMT-LIB syntax. An array is not a bit-vector of width 0, which is what
/// `Sort::width()` reports for one — a declaration built from it emitted `(_ BitVec 0)`
/// and the backend refused the whole script.
fn smt_sort(s: Sort) -> String {
    match s {
        Sort::Bool => "Bool".into(),
        Sort::BitVec(w) => format!("(_ BitVec {w})"),
        Sort::Array { idx, elem } => format!("(Array (_ BitVec {idx}) (_ BitVec {elem}))"),
    }
}

fn smt_name(v: &VarId, name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("v{}_{safe}", v.0)
}

/// Read `(define-fun vN_x () (_ BitVec W) #x…)` back into a `Model`.
fn parse_model(a: &TermArena, text: &str, vars: &[VarId]) -> Model {
    let mut m = Model::new();
    for v in vars {
        let (name, sort) = &a.vars[v.0 as usize];
        // Array-sorted variables have no bit-vector value to read back, and `BvConst`
        // asserts a non-zero width — so without this the model builder would panic.
        //
        // Currently unreachable: an array query cannot be validated, so it returns
        // `Unknown` before a model is ever requested. It stays as the guard for when
        // `Model` learns arrays, and mutation cannot distinguish it precisely *because*
        // the path in front of it is closed.
        if matches!(sort, Sort::Array { .. }) {
            continue;
        }
        let key = smt_name(v, name);
        let val = text
            .split(&format!("define-fun {key} "))
            .nth(1)
            .and_then(|rest| {
                let body = rest.split(')').next_back().unwrap_or("");
                let tok = rest
                    .split_whitespace()
                    .find(|t| t.starts_with("#x") || t.starts_with("#b"))
                    .or(Some(body))?;
                if let Some(h) = tok.strip_prefix("#x") {
                    u128::from_str_radix(h.trim_end_matches(')'), 16).ok()
                } else {
                    tok.strip_prefix("#b")
                        .and_then(|b| u128::from_str_radix(b.trim_end_matches(')'), 2).ok())
                }
            })
            .unwrap_or(0);
        m.set(*v, BvConst::new(sort.width(), val));
    }
    m
}

#[derive(Clone, Debug, Default)]
pub struct SolverStats {
    pub backend_calls: u64,
    /// How many times a backend **process** was started. 022 §4 wants this to stay at
    /// one for a whole run; a per-query spawn shows up here immediately.
    pub backend_spawns: u64,
    pub cache_entries: usize,
    pub tier1_unknown: u64,
}

/// Tier 1, escalating to tier 2 on `Unknown`, with the caches of 022 §6.
#[derive(Debug, Default)]
pub struct TieredSolver {
    asserted: Vec<Term>,
    scopes: Vec<usize>,
    backend: Option<SmtLib>,
    session: Option<Session>,
    paranoid: bool,
    stats: SolverStats,
    /// `Some(model)` = Sat, `None` = Unsat. The model is cached alongside the answer:
    /// caching only the verdict meant a cached `Sat` still had to re-derive a model,
    /// which sent the query straight back to the backend and defeated the cache.
    cache: IndexMap<(Vec<u32>, Vec<u32>), Option<Model>>,
}

impl TieredSolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_backend(b: SmtLib) -> Self {
        TieredSolver {
            backend: Some(b),
            ..Default::default()
        }
    }

    /// Send every tier-1 answer to tier 2 as well and assert agreement (022 §6).
    /// Too slow for production, mandatory in CI.
    pub fn set_paranoid(&mut self, on: bool) {
        self.paranoid = on;
    }

    pub fn stats(&self) -> &SolverStats {
        &self.stats
    }

    /// Ask the live backend, restarting and **replaying** if the process has died.
    ///
    /// Replay is the part that is easy to skip and impossible to notice: a restarted
    /// process still answers, just against an empty context, so the query silently
    /// becomes a different question.
    fn ask_backend(&mut self, a: &TermArena, all: &[Term]) -> Option<CheckResult> {
        let path = self.backend.as_ref()?.path().to_path_buf();
        let mut vars = Vec::new();
        for t in all {
            a.vars_of(*t, &mut vars);
        }

        let mut r = self.query(a, &path, all, &vars);
        if r.is_none() {
            // The process died. Restart and replay, then try once more; a second
            // failure is a real backend error rather than a transient one.
            self.session = None;
            r = self.query(a, &path, all, &vars);
        }
        self.stats.backend_calls += 1;
        match r {
            Some((true, m)) => {
                // Tier 2's answer is **not exempt from validation**. A backend returning
                // a wrong model would otherwise be trusted purely for being external.
                if all
                    .iter()
                    .all(|t| a.eval(&m, *t).map(|v| v.bits() != 0) == Ok(true))
                {
                    Some(CheckResult::Sat(m))
                } else {
                    Some(CheckResult::Unknown(UnknownReason::BackendError(
                        "backend model failed independent evaluation".into(),
                    )))
                }
            }
            Some((false, _)) => Some(CheckResult::Unsat),
            None => Some(CheckResult::Unknown(UnknownReason::BackendError(
                "backend gave no usable answer".into(),
            ))),
        }
    }

    /// One query against the live session, spawning it if needed. `None` means the
    /// process is unusable and the caller should restart it.
    fn query(
        &mut self,
        a: &TermArena,
        path: &std::path::Path,
        all: &[Term],
        vars: &[VarId],
    ) -> Option<(bool, Model)> {
        if self.session.is_none() {
            self.session = Session::spawn(path);
            self.stats.backend_spawns += 1;
        }
        let s = self.session.as_mut()?;

        // Declare any variable this process has not seen. Redeclaring after a restart is
        // exactly the "replay" contract 14 is about — the stack is re-sent below.
        let mut decls = String::new();
        for v in vars {
            if !s.declared.contains(v) {
                let (name, sort) = &a.vars[v.0 as usize];
                decls.push_str(&format!(
                    "(declare-const {} {})\n",
                    smt_name(v, name),
                    smt_sort(*sort)
                ));
                s.declared.push(*v);
            }
        }

        // `push`/`pop` around the assertions keeps the process reusable: the next query
        // starts from the same clean base rather than inheriting this one's constraints.
        let mut script = decls;
        script.push_str("(push 1)\n");
        for t in all {
            script.push_str(&format!("(assert {})\n", a.to_smtlib(*t)));
        }
        script.push_str("(check-sat)\n");
        s.send(&script)?;

        let verdict = s.read_answer()?;
        match verdict.trim() {
            "unsat" => {
                s.send("(pop 1)\n")?;
                Some((false, Model::new()))
            }
            "sat" => {
                s.send("(get-model)\n")?;
                let text = s.read_answer()?;
                s.send("(pop 1)\n")?;
                Some((true, parse_model(a, &text, vars)))
            }
            _ => {
                let _ = s.send("(pop 1)\n");
                None
            }
        }
    }

    /// Kill the backend **process** while keeping the session, for contract 14.
    ///
    /// Dropping the session instead would only exercise "no session yet → spawn one",
    /// which is a different and much easier path. The real failure is a process that
    /// dies mid-query: the write or read fails, and the retry must restart *and replay*.
    pub fn kill_backend_for_test(&mut self) {
        if let Some(s) = self.session.as_mut() {
            let _ = s.child.kill();
            let _ = s.child.wait();
        }
    }
}

impl Solver for TieredSolver {
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
        let all: Vec<Term> = self.asserted.iter().chain(assumptions).copied().collect();

        // The cache key is the **pair** of sorted assertion and assumption ids. Omitting
        // the assumptions makes `check([c])` and `check([¬c])` collide on one assertion
        // stack and return each other's answers — silent and catastrophic (022 §6.2).
        let mut ak: Vec<u32> = self.asserted.iter().map(|t| t.0).collect();
        let mut uk: Vec<u32> = assumptions.iter().map(|t| t.0).collect();
        ak.sort_unstable();
        uk.sort_unstable();
        let key = (ak, uk);

        if let Some(hit) = self.cache.get(&key) {
            return match hit {
                Some(m) => CheckResult::Sat(m.clone()),
                None => CheckResult::Unsat,
            };
        }

        let mut lite = SolverLite::new();
        for t in &all {
            lite.assert(*t);
        }
        let tier1 = lite.check(a, &[]);

        let decided = match &tier1 {
            CheckResult::Unknown(_) => {
                self.stats.tier1_unknown += 1;
                match self.ask_backend(a, &all) {
                    Some(r) => r,
                    None => tier1,
                }
            }
            _ => {
                if self.paranoid
                    && self.backend.is_some()
                    && let Some(t2) = self.ask_backend(a, &all)
                {
                    {
                        let agree = matches!(
                            (&tier1, &t2),
                            (CheckResult::Sat(_), CheckResult::Sat(_))
                                | (CheckResult::Unsat, CheckResult::Unsat)
                                | (_, CheckResult::Unknown(_))
                        );
                        assert!(agree, "paranoid: tier 1 said {tier1:?}, tier 2 said {t2:?}");
                    }
                }
                tier1
            }
        };

        // **`Unknown` is never cached.** A tier-1 `Unknown` cached above escalation would
        // stop tier 2 ever being consulted for any sibling state sharing that prefix, and
        // `Unknown(Timeout)` is a fact about the clock rather than the formula.
        match &decided {
            CheckResult::Sat(m) => {
                self.cache.insert(key, Some(m.clone()));
            }
            CheckResult::Unsat => {
                self.cache.insert(key, None);
            }
            CheckResult::Unknown(_) => {}
        }
        self.stats.cache_entries = self.cache.len();
        decided
    }
}
