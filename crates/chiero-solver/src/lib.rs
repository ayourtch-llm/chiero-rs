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
    Sub,
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
///
/// `PartialEq` is **map** equality, not insertion-order equality: two models assigning the
/// same values are the same model however they were built. 022 contract 8 says "returns
/// byte-identical models", and a test that wanted the stronger reading — the same values
/// in the same order — compares the rendered form as well. There was no equality at all
/// until contract 8 was written, which is its own small lesson about contracts nobody has
/// tried to check.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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

    /// Fold another model's assignments in, keeping this one's where both assign a
    /// variable.
    ///
    /// Used to reassemble a model from independent slices (§6.2). The components are
    /// **variable-disjoint** by construction, so in the intended use there is nothing to
    /// keep or discard — the `or_insert` is there so a caller that violates that gets a
    /// well-defined result rather than whichever iteration order won.
    pub fn merge_from(&mut self, other: &Model) {
        for (v, c) in &other.values {
            self.values.entry(*v).or_insert(*c);
        }
    }
}

/// A path condition, and whether anything in it was added without being checked feasible
/// (022 §6.1).
///
/// Independence slicing is equisatisfiable **only if every other component is already
/// known satisfiable**, which KLEE gets for free by checking every constraint before it
/// enters the path condition. chiero deliberately breaks that in three places — 023 §3
/// takes a branch anyway on solver `Unknown`, 021 §5 continues past an out-of-bounds
/// access on the in-bounds branch, 024 §4's `strlen` cap constrains a terminator to exist
/// — so the flag travels with the terms.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathCondition {
    terms: Vec<Term>,
    possibly_infeasible: bool,
}

impl PathCondition {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a constraint whose feasibility, in conjunction with everything already here,
    /// has been established.
    pub fn push_checked(&mut self, t: Term) {
        self.terms.push(t);
    }

    /// Add a constraint without that check, setting the flag. The three call sites in
    /// §6.1 are the reason this exists; a fourth should be added to that list rather than
    /// quietly using this.
    pub fn push_unchecked(&mut self, t: Term) {
        self.terms.push(t);
        self.possibly_infeasible = true;
    }

    /// Rebuild a path condition from a caller that stores the two halves separately.
    ///
    /// The engine keeps its constraints in `State::path` and the flag alongside them, and
    /// hands both over per query. Going through `push_checked`/`push_unchecked` in a loop
    /// would work but reads as though the *last* push decided the flag, which is exactly
    /// the confusion §6.1 is about.
    pub fn from_parts(terms: Vec<Term>, possibly_infeasible: bool) -> Self {
        PathCondition {
            terms,
            possibly_infeasible,
        }
    }

    pub fn terms(&self) -> &[Term] {
        &self.terms
    }

    pub fn possibly_infeasible(&self) -> bool {
        self.possibly_infeasible
    }

    pub fn len(&self) -> usize {
        self.terms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// §6.1: "A single full check that returns `Sat` clears it." Called by
    /// [`TieredSolver::check_path`]; a flag that is never cleared leaves every state
    /// downstream of one `Unknown` branch permanently unsliced, which is the
    /// slow-but-correct failure and so the one nothing would notice.
    pub fn mark_satisfiable(&mut self) {
        self.possibly_infeasible = false;
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
    /// See [`TermArena::id`]. Assigned on first use, never reused within the process.
    id: std::cell::Cell<u64>,
}

impl TermArena {
    pub fn new() -> Self {
        Self::default()
    }

    /// A process-unique identity for this arena.
    ///
    /// §6.2 says caches are per-`TermArena`, and a `Term` is a bare index into one — so a
    /// cache holding ids from arena A must be able to notice arena B. Nothing else needs
    /// this; it exists so the solver can refuse rather than answer confidently about
    /// terms it has never seen. Assigned lazily so `Default` stays derivable and an arena
    /// nobody asks about costs nothing.
    pub fn id(&self) -> u64 {
        if self.id.get() == 0 {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            self.id
                .set(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        }
        self.id.get()
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
        // Adjacent slices of the same value are one slice. This is what makes a
        // byte-wise store and load round-trip to the original term rather than to
        // something merely equivalent — equivalent is not enough, because the caller
        // compares terms, not models.
        if let (
            Node::Extract {
                a: x,
                hi: xh,
                lo: xl,
            },
            Node::Extract {
                a: y,
                hi: yh,
                lo: yl,
            },
        ) = (&self.nodes[hi.0 as usize], &self.nodes[lo.0 as usize])
            && x == y
            && *xl == *yh + 1
        {
            let (x, xh, yl) = (*x, *xh, *yl);
            return self.extract(x, xh, yl);
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

    pub fn sub(&mut self, a: Term, b: Term) -> Term {
        self.bin(BinKind::Sub, a, b)
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
        // A whole-width extract is the value itself. Without this a byte-wise store
        // followed by a load rebuilds `x` as a `Concat` of `Extract`s that no longer
        // *is* `x`, so `*p = x; y = *p;` loses the identity between `y` and `x` — and
        // every constraint the caller derives from it.
        if lo == 0 && hi == self.width(a) - 1 {
            return a;
        }
        if let Some(c) = self.as_const(a) {
            let w = hi - lo + 1;
            let mask = if w >= 128 {
                u128::MAX
            } else {
                (1u128 << w) - 1
            };
            return self.bv(w, (c.bits() >> lo) & mask);
        }
        // Extracting out of an extract is one extract, or a byte-wise round trip nests
        // them one level deeper per operation.
        if let Node::Extract {
            a: inner, lo: l2, ..
        } = &self.nodes[a.0 as usize]
        {
            let (inner, l2) = (*inner, *l2);
            return self.extract(inner, hi + l2, lo + l2);
        }
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

    fn render_named(&self, t: Term, names: &IndexMap<Term, String>) -> String {
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
                    BinKind::Sub => "bvsub",
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
        self.render_named(t, names)
    }

    /// Render `t` where a `Bool` is required, coercing a one-bit vector if need be.
    fn smt_bool_named(&self, names: &IndexMap<Term, String>, t: Term) -> String {
        if self.smt_is_bool(t) {
            self.render_named(t, names)
        } else {
            format!("(= {} #b1)", self.render_named(t, names))
        }
    }

    /// Render `t` where a bit-vector is required, coercing a `Bool` if need be.
    fn smt_bv_named(&self, names: &IndexMap<Term, String>, t: Term) -> String {
        if self.smt_is_bool(t) {
            format!("(ite {} #b1 #b0)", self.render_named(t, names))
        } else {
            self.render_named(t, names)
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
    /// This term's immediate operands, whatever its shape.
    ///
    /// A generic walk, because callers that need to ask "does this expression mention
    /// that one" should not have to enumerate `Node`'s variants — and a caller that
    /// enumerates them today silently stops seeing new ones. 021 §7.2's pointer-bit check
    /// is the first such caller.
    pub fn subterms(&self, t: Term) -> Vec<Term> {
        match &self.nodes[t.0 as usize] {
            Node::Const(_) | Node::Var(..) => Vec::new(),
            Node::Bin(_, a, b) => vec![*a, *b],
            Node::Not(a) => vec![*a],
            Node::Extend { a, .. } | Node::Extract { a, .. } => vec![*a],
            Node::Concat { hi, lo } => vec![*hi, *lo],
            Node::Ite { c, t, f } => vec![*c, *t, *f],
            Node::ArrayConst { val, .. } => vec![*val],
            Node::Store { a, i, v } => vec![*a, *i, *v],
            Node::Select { a, i } => vec![*a, *i],
        }
    }

    /// A binary term's operator and operands, when it is one.
    pub fn as_bin(&self, t: Term) -> Option<(BinKind, Term, Term)> {
        match &self.nodes[t.0 as usize] {
            Node::Bin(k, a, b) => Some((*k, *a, *b)),
            _ => None,
        }
    }

    /// Recognize an atom, **including a negated one and the shape CIR actually emits**.
    ///
    /// A conditional contributes its condition to one path and the negation of that
    /// condition to the other, so a reader that matched only a bare predicate could decide
    /// exactly one side of every branch. But peeling `Not` alone is not enough, because
    /// nothing in a lowered C program asserts a bare predicate. C has no boolean type at
    /// the machine level: a comparison is **materialized into an integer** and the branch
    /// tests that integer against zero, so `if (x > 10)` reaches the solver as
    ///
    /// ```text
    ///   (not (= ((_ zero_extend 31) (ite (bvslt 10 x) #b1 #b0)) (_ bv0 32)))
    /// ```
    ///
    /// The atom is in there, under three wrappers that each mean "the same truth value".
    /// This peels all of them, tracking polarity:
    ///
    /// - `not p` flips it;
    /// - `zext p` / `sext p` change no bit's zero-ness, so a truth test passes straight
    ///   through;
    /// - `ite p 1 0` **is** `p`, and `ite p 0 1` is its negation;
    /// - `p == 0` in a truth position is `!p`, in either operand order.
    ///
    /// The `== 0` case is tried and **falls back**: `x == 0` on a variable is a perfectly
    /// good atom, and peeling it would yield `x`, which is not a predicate. So the peel is
    /// attempted, and if it does not reach an atom the equality is returned as itself.
    pub fn as_atom(&self, t: Term) -> Option<Atom> {
        self.atom_at(t, false, 0)
    }

    /// `as_atom`'s worker: `t` in a **truth position**, asserted to be nonzero unless
    /// `negated`.
    ///
    /// The depth bound is a guard against a pathological term, not an expected case; eight
    /// is well past the four wrappers a lowered comparison carries.
    fn atom_at(&self, t: Term, negated: bool, depth: u32) -> Option<Atom> {
        if depth > 8 {
            return None;
        }
        match &self.nodes[t.0 as usize] {
            Node::Not(a) => self.atom_at(*a, !negated, depth + 1),
            // Widening cannot turn a zero into a nonzero or back, so a truth test is
            // unchanged by it. (This is sound only in a truth position — `zext(x) == 5` is
            // *not* `x == 5` in general — which is exactly the position this function is
            // about.)
            Node::Extend { a, .. } => self.atom_at(*a, negated, depth + 1),
            Node::Ite { c, t: tt, f } => match (
                self.as_const(*tt).map(|k| k.bits()),
                self.as_const(*f).map(|k| k.bits()),
            ) {
                (Some(1), Some(0)) => self.atom_at(*c, negated, depth + 1),
                (Some(0), Some(1)) => self.atom_at(*c, !negated, depth + 1),
                _ => None,
            },
            Node::Bin(BinKind::Eq, x, y) => {
                // `b == 0` asserts `b` is false, so the polarity flips. Tried in both
                // operand orders: the engine writes the constant on either side.
                if self.as_const(*y).is_some_and(|k| k.bits() == 0)
                    && let Some(at) = self.atom_at(*x, !negated, depth + 1)
                {
                    return Some(at);
                }
                if self.as_const(*x).is_some_and(|k| k.bits() == 0)
                    && let Some(at) = self.atom_at(*y, !negated, depth + 1)
                {
                    return Some(at);
                }
                Some(Atom {
                    kind: BinKind::Eq,
                    lhs: *x,
                    rhs: *y,
                    negated,
                })
            }
            Node::Bin(k, x, y) if k.is_predicate() => Some(Atom {
                kind: *k,
                lhs: *x,
                rhs: *y,
                negated,
            }),
            // **`a <= b` is `!(b < a)`**, and that is how it becomes an atom.
            //
            // There is no `Sle`/`Ule` kind, so lowering builds `<=` as the disjunction
            // `a < b || a == b` — and a disjunction is outside the fragment entirely. But
            // *this* disjunction is not a general one: it is a total order's non-strict
            // form, which is exactly the negation of the strict comparison with the
            // operands swapped. Rewriting it that way needs no new kind and no new term,
            // only a polarity flip, and it works in **both** polarities rather than the one
            // an ad-hoc reading would have given.
            //
            // The operands are compared as a set because `Eq` is commutative and `bin`
            // sorts its arguments by term id, so the equality half may have them either way
            // round while the strict half — `Slt`/`Ult` are not commutative — never does.
            Node::Bin(BinKind::Or, p, q) => {
                let (lt, eq) = (self.as_ordering(*p), self.as_equality(*q));
                let (lt, eq) = match (lt, eq) {
                    (Some(l), Some(e)) => (Some(l), Some(e)),
                    _ => (self.as_ordering(*q), self.as_equality(*p)),
                };
                let (kind, a1, b1) = lt?;
                let (a2, b2) = eq?;
                if (a1 == a2 && b1 == b2) || (a1 == b2 && b1 == a2) {
                    Some(Atom {
                        kind,
                        lhs: b1,
                        rhs: a1,
                        negated: !negated,
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// `a <u b` or `a <s b`, as its kind and operands.
    fn as_ordering(&self, t: Term) -> Option<(BinKind, Term, Term)> {
        match &self.nodes[t.0 as usize] {
            Node::Bin(k @ (BinKind::Ult | BinKind::Slt), a, b) => Some((*k, *a, *b)),
            _ => None,
        }
    }

    /// `a == b`, as its operands.
    fn as_equality(&self, t: Term) -> Option<(Term, Term)> {
        match &self.nodes[t.0 as usize] {
            Node::Bin(BinKind::Eq, a, b) => Some((*a, *b)),
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
            // **`independent_bin`, not `fold`** (022 contract 7d). §3's first hard rule
            // is that a tier-1 `Sat` is validated "by an independent evaluator", and an
            // evaluator that calls the folder is a spell-checker consulting the same
            // misspelling: a model built on a wrong rule is confirmed by that rule.
            // §2 names the consequence for the division cases, and it has already
            // happened here once.
            Node::Bin(k, a, b) => independent_bin(
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
/// The **independent** evaluation of a binary operation — 022 contract 7d.
///
/// A deliberate second implementation of `fold`'s semantics, written from SMT-LIB's
/// definitions rather than from `fold`. It must not be refactored to share code with it:
/// the whole value of having two is that a wrong rule in one is caught by disagreement
/// with the other, and §3 makes model validation depend on that independence. A test
/// reads this file to check the call does not come back.
///
/// Where `fold` reaches for Rust's operators, this reaches for the *definitions*:
/// unsigned division is repeated subtraction's answer (`checked_div`, with SMT-LIB's
/// all-ones for a zero divisor), signed division is defined by sign and magnitude, and the
/// shifts are defined by what SMT-LIB says a count at or past the width means rather than
/// by what Rust's `<<` would do with it. Two spellings of one truth, and when they differ,
/// one of them is wrong.
fn independent_bin(k: BinKind, x: BvConst, y: BvConst) -> BvConst {
    let w = x.width();
    let mask = |v: u128| BvConst::new(w, v);
    let pred = |v: bool| BvConst::new(1, u128::from(v));
    let (xu, yu) = (x.bits(), y.bits());
    let (xs, ys) = (x.signed(), y.signed());
    let ones = BvConst::all_ones(w).bits();
    match k {
        BinKind::Add => mask(xu.wrapping_add(yu)),
        BinKind::Sub => mask(xu.wrapping_add(ones.wrapping_sub(yu)).wrapping_add(1)),
        BinKind::Mul => mask(xu.wrapping_mul(yu)),
        BinKind::And => mask(xu & yu),
        BinKind::Or => mask(xu | yu),
        BinKind::Xor => mask((xu | yu) & !(xu & yu)),
        // SMT-LIB: `bvudiv x 0` is all ones — the largest representable value, which is
        // what "divide by nothing" saturates to.
        BinKind::UDiv => match xu.checked_div(yu) {
            Some(q) => mask(q),
            None => mask(ones),
        },
        // SMT-LIB defines `bvsdiv` by the signs: magnitude divided, sign applied, and for
        // a zero divisor `-1` when the dividend is non-negative and `1` when it is
        // negative. Verified against z3 4.8.12 (022 §2's table).
        BinKind::SDiv => {
            if yu == 0 {
                if xs < 0 { mask(1) } else { mask(ones) }
            } else {
                mask(xs.wrapping_div(ys) as u128)
            }
        }
        // `bvurem`/`bvsrem` by zero give back the **dividend**, not all ones.
        BinKind::URem => match xu.checked_rem(yu) {
            Some(r) => mask(r),
            None => x,
        },
        BinKind::SRem => {
            if yu == 0 {
                x
            } else {
                mask(xs.wrapping_rem(ys) as u128)
            }
        }
        // A count at or past the width shifts every bit out. Rust's `<<` is undefined
        // there, so the count is tested first rather than relied upon.
        //
        // ⚠️ An earlier comment here called `>=` versus `>` an **equivalent** mutation,
        // reasoning that "the value is masked to `w` afterwards". That is true at 8 and 32
        // bits and **false at 128**: `u128::wrapping_shl(128)` masks the *count* to zero
        // and shifts nothing, so the mutant returns the operand where SMT-LIB says zero.
        // `MAX_BV_BITS` is 128 and 020 declares `__int128`, so it is reachable. The
        // differential test covers 128 now. Found by review — a surviving mutant declared
        // equivalent on an argument nobody checked at every width.
        BinKind::Shl => {
            if yu >= u128::from(w) {
                BvConst::zero(w)
            } else {
                mask(xu.wrapping_shl(yu as u32))
            }
        }
        BinKind::LShr => {
            if yu >= u128::from(w) {
                BvConst::zero(w)
            } else {
                mask(xu.wrapping_shr(yu as u32))
            }
        }
        // Arithmetic shift replicates the sign bit, so a count past the width leaves the
        // sign in every position.
        BinKind::AShr => {
            if yu >= u128::from(w) {
                if xs < 0 { mask(ones) } else { BvConst::zero(w) }
            } else {
                mask(xs.wrapping_shr(yu as u32) as u128)
            }
        }
        BinKind::Ult => pred(xu < yu),
        BinKind::Slt => pred(xs < ys),
        BinKind::Eq => pred(xu == yu),
    }
}

fn fold(k: BinKind, x: BvConst, y: BvConst) -> BvConst {
    let w = x.width();
    let b = |v: bool| BvConst::new(1, v as u128);
    match k {
        BinKind::Add => BvConst::new(w, x.bits().wrapping_add(y.bits())),
        BinKind::Sub => BvConst::new(w, x.bits().wrapping_sub(y.bits())),
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
    /// **022 §4**: the wall-clock watchdog fired. A fact about the clock, not the formula
    /// — the distinction a caller needs to tell "ask again with more time" from
    /// `Incomplete`'s "this fragment is out of reach".
    Timeout,
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
/// How many candidate models `SolverLite` will try before answering `Unknown`.
///
/// Small on purpose. This is the *incomplete* solver (022 §3): its job is to answer the
/// easy majority in-process so the backend is reserved for the rest, and a long search here
/// is the cost it exists to avoid. Sixty-four covers the shapes that motivated it — a
/// `switch` default needs four, a small mask a handful — and anything needing more is
/// better escalated than ground out.
const CANDIDATE_BUDGET: u32 = 64;

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
        // **Ground assertions are decided here, before the fragment test.** A constant
        // is not an atom, so `false` — the term a folded contradiction collapses to —
        // left §3.2's fragment, came back `Unknown`, and was then handed to a backend
        // that cannot assert a bare constant either: the answer was
        // `Unknown(BackendError)` for a formula whose truth needs no solver at all.
        // Downstream that is worse than slow, because 023 §3 takes a branch the solver
        // could not refute, so a ground-refutable condition was explored as though it
        // were undecidable. Found while implementing 021 §5.2, where every concrete
        // arena index produces exactly this shape.
        let mut all: Vec<Term> = all;
        let mut ground_true = 0usize;
        for t in &all {
            if let Ok(c) = a.eval_ground(*t) {
                if c.bits() == 0 {
                    return CheckResult::Unsat;
                }
                ground_true += 1;
            }
        }
        if ground_true > 0 {
            all.retain(|t| a.eval_ground(*t).is_err());
            if all.is_empty() {
                // Everything asserted is true, so any complete assignment is a model.
                let mut m = Model::new();
                for (i, (_, sort)) in a.vars.iter().enumerate() {
                    m.set(VarId(i as u32), BvConst::zero(sort.width()));
                }
                return CheckResult::Sat(m);
            }
        }

        // **A one-bit `and` is a conjunction, and a conjunction of assertions is just more
        // assertions.** A `switch` builds its default arm as "not case 1 and not case 2",
        // which arrives as a single term; without this the whole default path falls out of
        // the fragment for being one `and` deep.
        //
        // The width guard is not decoration. `And` is *bitwise*: for a wider term,
        // `x & y != 0` does not imply `x != 0` and `y != 0`, so splitting would assert
        // something stronger than was given — and a stronger assertion set can reach an
        // empty domain, which is reported as `Unsat`, the one verdict nothing validates.
        //
        // `all` itself is left alone: it is what the candidate model is checked against,
        // and that check must be against what the caller actually asserted.
        let mut atoms = Vec::new();
        let mut work: Vec<Term> = all.clone();
        while let Some(t) = work.pop() {
            if a.width(t) == 1
                && let Some((BinKind::And, x, y)) = a.as_bin(t)
            {
                work.push(x);
                work.push(y);
                continue;
            }
            match a.as_atom(t) {
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
                // **A bounded search, not a single guess.** One candidate answered only
                // for sets whose models sit at the bottom of the domain; a `switch`
                // default arm — `x&3` none of 0, 1, 2 — has its only model three steps
                // up, and was reported `Unknown`.
                //
                // Trying more is safe for the same reason trying one was: every candidate
                // is evaluated against the *original* assertions, so a guess cannot make
                // `Sat` wrong. Exhausting the budget still yields `Unknown`, never
                // `Unsat` — a search that gives up has proved nothing, and 022 §3.1 says
                // only the syntactic fragment may refute.
                (0..CANDIDATE_BUDGET)
                    .find_map(|n| {
                        let m = dom.candidate_n(a, u128::from(n))?;
                        all.iter()
                            .all(|t| a.eval(&m, *t).map(|v| v.bits() != 0) == Ok(true))
                            .then_some(m)
                    })
                    .map_or(
                        CheckResult::Unknown(UnknownReason::Incomplete(
                            "no candidate model survived validation",
                        )),
                        CheckResult::Sat,
                    )
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
    /// Whether the assertion is the **negation** of `lhs kind rhs`.
    ///
    /// Carried rather than rewritten into another predicate because there are only three
    /// predicate kinds and none of the negations is one of them: `!(a <u b)` is `a >=u b`,
    /// `!(a <s b)` is `a >=s b`, `!(a == b)` is `a != b`. Rewriting `!(a <u b)` as
    /// `b <u a || a == b` would leave the conjunction-of-atoms fragment entirely, which is
    /// the fragment the soundness of `Unsat` rests on.
    pub negated: bool,
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
    /// The `n`th value of this domain in increasing order, **computed rather than scanned**.
    ///
    /// Candidate selection used to be `(lo..=hi).find(..)`, which is fine while the known
    /// bits are low in the word and disastrous otherwise: wave 154 taught `x & FLAG != 0` to
    /// *set* those bits, and a flag at bit 30 asked the walk to count to 2^30 — twenty-two
    /// seconds for one assertion, and nothing had hit it only because every fixture so far
    /// masked a low bit.
    ///
    /// The forced bits are not a search. Values matching a known-bits pattern are exactly
    /// `scatter(free, i) | ones` for `i = 0, 1, 2, …`, strictly increasing — so the domain
    /// is indexed by its *free* bits, and enumerating it is arithmetic on that index. A
    /// domain whose forced bits sit high costs no more to walk than one whose bits sit low.
    fn nth(&self, n: u128) -> Option<u128> {
        self.at_index(self.start_index()?.checked_add(n)?)
    }

    /// The free-bit index of the first value at or above `lo`.
    ///
    /// `lo`'s own free bits give the starting guess. Clearing a forced-zero bit or setting
    /// a forced-one bit can move the result either side of `lo`, so the index is lifted
    /// until the value clears the floor — bounded, because this is the incomplete solver
    /// and an unbounded lift is the search it exists not to perform. Giving up yields
    /// `None`, which becomes `Unknown`, which is always sound.
    fn start_index(&self) -> Option<u128> {
        // Contradictory known bits: no value has a bit both set and clear.
        if self.zeros & self.ones != 0 {
            return None;
        }
        let free = self.free();
        let mut i = gather(free, self.lo, self.width);
        for _ in 0..CANDIDATE_BUDGET {
            match self.at_index(i) {
                // `at_index` already rejects anything outside `[lo, hi]`, so a `Some`
                // *is* the answer — re-checking the floor here was dead, which a mutation
                // removing it proved by surviving every channel.
                Some(_) => return Some(i),
                // Past `hi` already: nothing at or above `lo` is in this domain.
                None if scatter(free, i, self.width).is_some_and(|v| v | self.ones > self.hi) => {
                    return None;
                }
                _ => i = i.checked_add(1)?,
            }
        }
        None
    }

    /// The value at free-bit index `i`, if it lies in `[lo, hi]`.
    fn at_index(&self, i: u128) -> Option<u128> {
        let v = scatter(self.free(), i, self.width)? | self.ones;
        (v >= self.lo && v <= self.hi).then_some(v)
    }

    /// The bits this domain has not pinned.
    fn free(&self) -> u128 {
        !(self.zeros | self.ones) & mask(self.width)
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

    /// Narrow the variable under an **exact, order-preserving widening**.
    ///
    /// `None` means "not this shape, carry on"; `Some(changed)` means the atom was handled.
    ///
    /// Two conditions make this sound, and both are refusals rather than adjustments:
    ///
    /// - **the widening must preserve the comparison's order.** `sext` preserves signed
    ///   order and `zext` unsigned; the mismatched pairs (`sext` under `<u`, `zext` under
    ///   `<s`) do not, so they are declined. Equality is preserved by both.
    /// - **the bound must be representable in the source width.** `sext(x, 64) <s 2^40` is
    ///   true for every 32-bit `x`, and truncating the bound would assert something else
    ///   entirely. Out-of-range bounds are declined rather than folded — a little
    ///   completeness given up to keep the fragment honest.
    fn narrow_widened(
        &mut self,
        a: &TermArena,
        at: &Atom,
        wide: Term,
        k: BvConst,
        flipped: bool,
    ) -> Option<bool> {
        let Node::Extend {
            a: inner, signed, ..
        } = &a.nodes[wide.0 as usize]
        else {
            return None;
        };
        let v = a.var_id(*inner)?;
        let w = a.width(*inner);
        let order_preserved = match at.kind {
            BinKind::Slt => *signed,
            BinKind::Ult => !*signed,
            BinKind::Eq => true,
            _ => false,
        };
        if !order_preserved {
            return None;
        }
        let bits = k.bits();
        let fits = if *signed {
            let trunc = bits & mask(w);
            // `bits` must be the sign-extension of its own low `w` bits, or the bound says
            // something about values the source width cannot hold.
            let reextended = if trunc >= signed_min(w) {
                trunc | (!mask(w) & mask(k.width()))
            } else {
                trunc
            };
            reextended == bits
        } else {
            bits & !mask(w) == 0
        };
        if !fits {
            return None;
        }
        let d = self.dom(v, w);
        Some(narrow(d, at.kind, bits & mask(w), flipped, at.negated))
    }

    fn apply(&mut self, a: &TermArena, at: &Atom) -> Result<bool, &'static str> {
        let lc = a.as_const(at.lhs);
        let rc = a.as_const(at.rhs);
        let lv = a.var_id(at.lhs);
        let rv = a.var_id(at.rhs);

        // **A widened variable is still that variable.** `(long)x > 5` is
        // `5 <s sext(x, 64)`, and matching only a bare `Var` left it unnarrowed. Integer
        // promotion (C11 6.3.1.1) widens every `char` and `short` before comparing it, so
        // this is the shape most comparisons on small types have, not a `long` curiosity.
        //
        // Handled before the main match and returned from, because the domain is keyed by
        // the *source* width: falling through would narrow the same variable a second time
        // at the widened width, which `dom` treats as a different domain.
        //
        // The bound is read with `eval_ground`, not `as_const`: lowering widens *both*
        // operands, so `(long)x > 5` compares `sext(x, 64)` against `sext(5, 64)` — a
        // ground term, but not a `Const` node, and `as_const` sees nothing.
        if lv.is_none()
            && let Ok(c) = a.eval_ground(at.rhs)
            && let Some(changed) = self.narrow_widened(a, at, at.lhs, c, false)
        {
            return Ok(changed);
        }
        if rv.is_none()
            && let Ok(c) = a.eval_ground(at.lhs)
            && let Some(changed) = self.narrow_widened(a, at, at.rhs, c, true)
        {
            return Ok(changed);
        }

        // `v OP const` and `const OP v` are the shapes the domain understands. An atom
        // over a non-variable, non-constant expression (an addition, a mask) is not
        // refuted here — it is simply not used to narrow, which is incompleteness
        // rather than unsoundness.
        let mut changed = false;
        match (lv, rc, lc, rv) {
            (Some(v), Some(k), _, _) => {
                let w = k.width();
                let d = self.dom(v, w);
                changed |= narrow(d, at.kind, k.bits(), false, at.negated);
            }
            (_, _, Some(k), Some(v)) => {
                let w = k.width();
                let d = self.dom(v, w);
                changed |= narrow(d, at.kind, k.bits(), true, at.negated);
            }
            _ => {
                // `masked == k` where masked is `v & m`: a known-bits fact.
                //
                // **Either operand order.** `Eq` and `And` are commutative and `bin` sorts
                // their operands by term id, so which side the constant lands on is decided
                // by interning order — not by how the caller wrote it. Reading only
                // `at.lhs` meant the same assertion was understood or not depending on
                // which term happened to be created first, and `x & 1 != 0` fell on the
                // wrong side of that.
                let masked = a
                    .as_var_and_mask(at.lhs)
                    .map(|vm| (vm, rc))
                    .or_else(|| a.as_var_and_mask(at.rhs).map(|vm| (vm, lc)));
                if at.kind == BinKind::Eq
                    && let Some(((v, m), Some(k))) = masked
                {
                    let w = k.width();
                    if at.negated {
                        // **A negated mask pins only when the mask selects one bit.**
                        //
                        // `v & m != k` says one of the masked bits differs from `k`
                        // without saying which, and a known-bits fact cannot express a
                        // disjunction — so the general case must decline. When `m` has a
                        // single bit there is no disjunction left: that bit is the one
                        // that differs, and its value is the complement of `k`'s.
                        // `if (x & FLAG)` is this case, and it is the common one.
                        if m.count_ones() == 1 {
                            let d = self.dom(v, w);
                            let (want_ones, want_zeros) =
                                if k.bits() & m == 0 { (m, 0) } else { (0, m) };
                            if d.ones | want_ones != d.ones || d.zeros | want_zeros != d.zeros {
                                d.ones |= want_ones;
                                d.zeros |= want_zeros;
                                changed = true;
                            }
                        }
                    } else {
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
        }
        Ok(changed)
    }

    /// A candidate assignment: the least value in each variable's domain. Variables that
    /// were never constrained get 0, so the model is **complete** (022 §2).
    /// The `n`th candidate model. `n == 0` is each variable's least feasible value.
    ///
    /// Successive candidates move **every** constrained variable together, which is the
    /// cheap diagonal rather than a product: a full cross-product of domains is the search
    /// this solver exists not to do, and the single-variable case — overwhelmingly the
    /// common one for a path condition — is enumerated exactly by it.
    fn candidate_n(&self, a: &TermArena, n: u128) -> Option<Model> {
        let mut m = Model::new();
        for (v, d) in &self.vars {
            m.set(*v, BvConst::new(d.width, d.nth(n)?));
        }
        // **A variable no atom narrowed still varies with `n`.**
        //
        // The model must be *complete* (022 §2), and unconstrained variables used to be
        // filled with zero — which was invisible while there was a single candidate and
        // silently defeated the search once there were many: every attempt proposed the
        // same zero. That is exactly the case the search was added for. A `switch`
        // default's `x&3 != 0 && != 1 && != 2` narrows *nothing* (each conjunct is a
        // multi-bit negated mask), so `x` never enters `vars` at all, and sixty-four
        // attempts all proposed `x = 0`.
        for (i, (_, sort)) in a.vars.iter().enumerate() {
            let v = VarId(i as u32);
            if m.get(v).is_none() {
                // An array-sorted variable has no bit width to vary; it keeps the zero
                // the completeness rule requires. `mask(0)` is not a value.
                let w = sort.width();
                m.set(
                    v,
                    if w == 0 {
                        BvConst::zero(w)
                    } else {
                        BvConst::new(w, n & mask(w))
                    },
                );
            }
        }
        Some(m)
    }
}

/// Narrow one variable's domain by `v OP k` (or `k OP v` when `flipped`).
/// The least value whose top bit is set — `2^(w-1)`, the first *negative* number at width
/// `w` read as unsigned, and therefore one past the largest signed value.
fn signed_min(w: u32) -> u128 {
    1u128 << (w - 1)
}

/// Narrow `d` by one atom, in whichever direction the atom's polarity implies.
///
/// **Every arm must be an implication, not an equivalence.** `Unsat` is produced when a
/// domain goes empty and nothing validates it afterwards (022 §3.1's asymmetry), so a
/// narrowing that removes a value the assertion actually permits turns an unsound answer
/// into the one verdict the design cannot catch. Where a relation cannot be expressed as
/// an interval the arm does nothing, which costs completeness and keeps soundness.
fn narrow(d: &mut VarDomain, kind: BinKind, k: u128, flipped: bool, negated: bool) -> bool {
    let (lo0, hi0, z0, o0) = (d.lo, d.hi, d.zeros, d.ones);
    match (kind, flipped, negated) {
        // v <u k  =>  v <= k-1
        (BinKind::Ult, false, false) => {
            d.hi = d.hi.min(k.saturating_sub(1));
            if k == 0 {
                // `v <u 0` holds for no `v`.
                d.lo = 1;
                d.hi = 0;
            }
        }
        // k <u v  =>  v >= k+1
        (BinKind::Ult, true, false) => d.lo = d.lo.max(k.saturating_add(1)),
        // !(v <u k)  =>  v >=u k. The false side of `if (v < k)`, and the reason this
        // function has a polarity at all.
        (BinKind::Ult, false, true) => d.lo = d.lo.max(k),
        // !(k <u v)  =>  v <=u k
        (BinKind::Ult, true, true) => d.hi = d.hi.min(k),
        (BinKind::Eq, _, false) => {
            d.lo = d.lo.max(k);
            d.hi = d.hi.min(k);
            d.ones |= k;
            d.zeros |= !k & mask(d.width);
        }
        // **`v != k` is a hole, and an interval has no holes.** It narrows only when the
        // excluded value sits *at* a bound, which is where it matters: the candidate model
        // is the domain's least value, so `!(v == 0)` over a full domain would otherwise
        // propose 0, fail validation, and report `Unknown` for a trivially satisfiable
        // assertion. Away from a bound the domain is left alone — incompleteness, and the
        // model validator is what still keeps the answer honest.
        (BinKind::Eq, _, true) => {
            if d.lo == k {
                d.lo = d.lo.saturating_add(1);
            }
            if d.hi == k {
                d.hi = d.hi.saturating_sub(1);
                if k == 0 {
                    // Nothing is below 0, so the domain is empty rather than wrapped.
                    d.lo = 1;
                    d.hi = 0;
                }
            }
        }
        // **Two of the four signed cases are an unsigned interval; the other two are not.**
        //
        // The domain is an unsigned range, and a signed constraint maps onto one only when
        // it confines `v` to the non-negative half. With `k` non-negative:
        //
        // - `v >=s k` is exactly `k <=u v <=u 2^(w-1)-1` — one interval;
        // - `v >s k` is the same shifted by one;
        // - `v <s k` admits every negative value, which is the *upper* unsigned half, plus
        //   `[0, k-1]` — two intervals, and no single range is an implication of it;
        // - `v <=s k` likewise.
        //
        // So the first two narrow and the last two do not. A negative `k` puts all four in
        // the second category, hence the guard.
        //
        // This is not incidental to wave 153: `x > 0` on an `int` is a *signed* comparison,
        // which is to say almost every comparison C programs write. Before this, positive
        // signed atoms appeared to work only because the candidate model is the domain's
        // least value and 0 happens to satisfy `0 <s k`; the negation had no such luck and
        // failed validation, reporting `Unknown` for a trivially satisfiable assertion.
        (BinKind::Slt, false, true) if k < signed_min(d.width) => {
            d.lo = d.lo.max(k);
            d.hi = d.hi.min(signed_min(d.width) - 1);
        }
        (BinKind::Slt, true, false) if k < signed_min(d.width) => {
            d.lo = d.lo.max(k.saturating_add(1));
            d.hi = d.hi.min(signed_min(d.width) - 1);
        }
        // **The other row of the table.** With `k` *negative* the polarities swap which one
        // is contiguous: `v <s k` becomes the run from the first negative bit pattern up to
        // `k`, one interval, while `v >=s k` becomes `[0, 2^(w-1)-1]` plus `[k_u, 2^w-1]`
        // and is declined. Wave 153 implemented one row of that table and the asymmetry hid
        // it — a branch whose *other* half answers looks like it works.
        (BinKind::Slt, false, false) if k >= signed_min(d.width) => {
            d.lo = d.lo.max(signed_min(d.width));
            d.hi = d.hi.min(k.saturating_sub(1));
        }
        // `v <s 0` is the boundary case and the one that matters most: the whole negative
        // half, `[2^(w-1), 2^w-1]`. Every `if (x < 0)` in C lands here.
        (BinKind::Slt, false, false) if k == 0 => d.lo = d.lo.max(signed_min(d.width)),
        // `!(k <s v)` is `v <=s k`; contiguous for a negative `k`, at `[2^(w-1), k_u]`.
        (BinKind::Slt, true, true) if k >= signed_min(d.width) => {
            d.lo = d.lo.max(signed_min(d.width));
            d.hi = d.hi.min(k);
        }
        // Everything else: no narrowing. Incompleteness, which the model validator turns
        // into `Unknown`; treating it as unsigned would be unsoundness, which nothing
        // downstream would catch.
        _ => {}
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
    /// What to call this backend in a result (022 §4).
    ///
    /// The file stem, not the path: 022's own examples are `z3` and `cvc5`, and a finding
    /// that named `/usr/bin/z3` would make two machines running the same solver look like
    /// they disagreed. The full path stays available for anyone diagnosing *which* z3.
    pub fn name(&self) -> &str {
        self.path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("smtlib")
    }

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

    /// A backend at a **named** path, discovery bypassed.
    ///
    /// `discover` answers "what is installed"; this answers "use this one". A test that
    /// needs a backend which misbehaves on purpose has no other way to get one, and
    /// pointing `$CHIERO_SMT_SOLVER` at it instead would make the whole process's
    /// discovery depend on one test's environment.
    pub fn at(path: impl Into<std::path::PathBuf>) -> SmtLib {
        SmtLib { path: path.into() }
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
    /// Framed answers, produced by a reader thread.
    ///
    /// **A thread, because the read has to be abandonable.** `read_form` framed the reply
    /// by reading the pipe a byte at a time, which is correct and unbounded — a process
    /// that accepts a query and says nothing parks the caller in that loop for as long as
    /// it lives. There is no portable way to put a deadline on a blocking pipe read, so
    /// the blocking happens somewhere abandonable and the owner waits on a channel.
    rx: std::sync::mpsc::Receiver<String>,
    /// How long to wait for one answer before declaring the process lost.
    timeout: std::time::Duration,
    /// Set when the watchdog fired, so the caller can tell a timeout from a death. Retrying
    /// a *death* is right — it is usually a crash on one query. Retrying a timeout just
    /// spends the budget twice to learn the same thing.
    timed_out: bool,
}

impl Session {
    fn spawn(path: &std::path::Path, timeout: std::time::Duration) -> Option<Session> {
        let child = std::process::Command::new(path)
            .args(["-in", "-smt2"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let mut child = child;
        let out = child.stdout.take()?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || read_forms(out, &tx));
        let mut s = Session {
            child,
            declared: Vec::new(),
            rx,
            timeout,
            timed_out: false,
        };
        // **`ALL`.** The logic has to admit every term the arena can build, and each
        // narrower choice has excluded something in turn: `QF_BV` rejected arrays
        // outright, and `QF_ABV` accepts `Array` but still rejects `as const` — which is
        // the *base* of every promoted object, so all of array theory was unusable.
        // Naming a narrow logic buys nothing here and has now cost two rounds.
        // **The solver's own budget, in the preamble** (022 §4). Session state, sent once:
        // re-sending it before every `(check-sat)` would spend a round trip on the hot path
        // restating what the process already knows, and keeping the process alive is worth
        // doing precisely because those round trips dominate short queries.
        //
        // **Strictly under the watchdog.** At or above it the watchdog always fires first
        // and the option is decoration — chiero would still be killing processes it had
        // asked politely to stop. `smt_timeout_ms` is where that relationship lives, so the
        // two cannot drift apart.
        s.send(&format!(
            "(set-logic ALL)\n(set-option :produce-models true)\n(set-option :timeout {})\n",
            smt_timeout_ms(timeout)
        ))?;
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

    /// Wait for one framed answer, or give up.
    ///
    /// `None` covers both "the process died" and "the process said nothing in time"; the
    /// caller separates them with [`Self::timed_out`], because only one of the two is worth
    /// retrying.
    fn read_form(&mut self) -> Option<String> {
        match self.rx.recv_timeout(self.timeout) {
            Ok(form) => Some(form),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // **Kill it here**, though `Drop for Session` would too — the caller
                // drops the session immediately after a timeout, so a mutation removing
                // this line survives, and that is worth writing down rather than
                // discovering twice. It stays because the two are different claims: `Drop`
                // tidies up whenever a session goes away, and this says the watchdog's job
                // is to *end the query*, which is 022 §4's "the process is killed". A
                // future caller that timed out and kept the session would otherwise leave
                // a process holding an unanswered query and a pipe nobody is reading.
                //
                // Restarting is the caller's half: `query` redeclares every variable on a
                // fresh session, which is where contract 14's replay correctness lives.
                self.timed_out = true;
                let _ = self.child.kill();
                None
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => None,
        }
    }
}

/// Frame the backend's output into balanced S-expressions and bare tokens.
///
/// A long-lived process means output must be framed rather than read to EOF, and
/// parenthesis balance is the framing SMT-LIB2 gives us. Runs on its own thread and stops
/// when the pipe closes — which is what killing the child does, so the watchdog needs no
/// other way to end it.
fn read_forms(mut out: std::process::ChildStdout, tx: &std::sync::mpsc::Sender<String>) {
    use std::io::Read;
    let mut buf = Vec::new();
    let mut depth = 0i32;
    let mut started = false;
    let mut byte = [0u8; 1];
    loop {
        match out.read(&mut byte) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let c = byte[0];
        buf.push(c);
        let form = match c {
            b'(' => {
                depth += 1;
                started = true;
                None
            }
            b')' => {
                depth -= 1;
                (started && depth == 0).then(|| String::from_utf8_lossy(&buf).into_owned())
            }
            b'\n' if !started && !buf.iter().all(|b| b.is_ascii_whitespace()) => {
                Some(String::from_utf8_lossy(&buf).trim().to_string())
            }
            _ => None,
        };
        if let Some(f) = form {
            if tx.send(f).is_err() {
                return;
            }
            buf.clear();
            depth = 0;
            started = false;
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
    /// Answers a backend gave that chiero could not use — unparseable output, a model
    /// that failed independent evaluation, a dead process (022 contract 15).
    ///
    /// Counted because the failure is silent otherwise: every query comes back
    /// `Unknown`, every consumer degrades honestly, and a run that decided *nothing*
    /// looks like a run over a hard program.
    pub backend_errors: u64,
    /// How many queries the wall-clock watchdog cut short (022 §4).
    ///
    /// Separate from `backend_errors` because it is a different thing to act on: errors
    /// say the solver is misbehaving, timeouts say the budget is too small or the query
    /// too hard, and a run reporting many of these should be given more time rather than a
    /// different solver.
    pub backend_timeouts: u64,
    /// How many assertions independence slicing kept out of a backend query (022 §6.2).
    ///
    /// Contract 9 needs this: "verifying only that the dumped query got smaller tests
    /// that slicing happened, not that it was correct" — but the converse is just as
    /// true, and a same-answer test alone passes for a slicer that never slices.
    pub sliced_terms_skipped: u64,
}

/// Which cached set a candidate refers to. See `TieredSolver::remember`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CacheSlot {
    sat: bool,
    idx: usize,
}

/// Tier 1, escalating to tier 2 on `Unknown`, with the caches of 022 §6.
/// How long one backend query may take before the watchdog fires, unless told otherwise.
///
/// Long enough that no query in this workspace approaches it, short enough that a wedged
/// solver is a pause rather than a hang. 022 §4 asks for the mechanism and names no number;
/// this is the number, in one place, so that changing it is one edit and not a search.
const DEFAULT_BACKEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// `$CHIERO_SMT_TIMEOUT`, in seconds.
///
/// **Zero means no watchdog**, which is the escape hatch for someone bisecting a genuinely
/// slow query who would rather wait than get `Unknown(Timeout)`. Anything unparseable is
/// the default rather than an error: a mistyped environment variable must not decide that a
/// run cannot use its solver.
fn backend_timeout() -> std::time::Duration {
    match std::env::var("CHIERO_SMT_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        Some(0) => std::time::Duration::MAX,
        Some(n) => std::time::Duration::from_secs(n),
        None => DEFAULT_BACKEND_TIMEOUT,
    }
}

/// The `:timeout` to hand the solver, in milliseconds, given the watchdog's duration.
///
/// **Nine tenths, and never zero.** The solver has to run out of time *before* the watchdog
/// does, or the process is killed while it still had time to answer and the option buys
/// nothing. A fraction rather than a fixed margin because the watchdog is configurable:
/// subtracting a constant second is wrong at 500ms and pointless at an hour.
///
/// An unbounded watchdog (`$CHIERO_SMT_TIMEOUT=0`, the bisecting escape hatch) means the
/// solver should not stop either, so it is given z3's own "no limit".
fn smt_timeout_ms(watchdog: std::time::Duration) -> u64 {
    if watchdog == std::time::Duration::MAX {
        return 0; // z3 reads 0 as unlimited
    }
    let ms = watchdog.as_millis().min(u128::from(u64::MAX)) as u64;
    (ms / 10 * 9).max(1)
}

/// Serial number for dump filenames, so concurrent solvers do not collide.
static NEXT_DUMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `$CHIERO_DUMP_QUERIES`, the directory every backend query is written to.
///
/// A newtype for the same reason [`BackendTimeout`] is one: `TieredSolver` derives
/// `Default`, and the default has to be read from the environment rather than be `None`.
#[derive(Debug, Clone)]
struct DumpDir(Option<std::path::PathBuf>);

impl Default for DumpDir {
    fn default() -> Self {
        DumpDir(
            std::env::var_os("CHIERO_DUMP_QUERIES")
                .filter(|v| !v.is_empty())
                .map(std::path::PathBuf::from),
        )
    }
}

/// The watchdog's duration, as a newtype so `TieredSolver` can keep deriving `Default`.
///
/// `Duration::default()` is **zero**, and a derived default would have made every backend
/// query time out instantly — a change that compiles, passes review, and turns the solver
/// off. The newtype puts the real default next to the constant it comes from.
#[derive(Debug, Clone, Copy)]
struct BackendTimeout(std::time::Duration);

impl Default for BackendTimeout {
    fn default() -> Self {
        BackendTimeout(backend_timeout())
    }
}

#[derive(Debug, Default)]
pub struct TieredSolver {
    /// How long one backend query may take before the watchdog fires (022 §4).
    ///
    /// A default rather than a required argument: a solver that hangs is a failure mode
    /// every caller has, and one that only the callers who remembered to configure it are
    /// protected from is not protected. `$CHIERO_SMT_TIMEOUT` (seconds) overrides it, which
    /// is how a genuinely long-running analysis asks for more without a code change.
    timeout: BackendTimeout,
    /// Where to write every backend query (022 §4), if anywhere.
    dump: DumpDir,
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
    /// 022 §6's **counterexample cache**, as three separate structures because the three
    /// rules are separate claims:
    ///
    /// - `sat_sets` — constraint sets known satisfiable, each with the model that showed
    ///   it. A **subset** of one is satisfiable (the same model does it), and a model
    ///   here that happens to satisfy an unrelated query answers that too.
    /// - `unsat_sets` — sets known unsatisfiable. A **superset** of one is unsatisfiable.
    ///   The subset direction is *not* a rule: dropping the conflicting constraint can
    ///   leave a satisfiable set, and answering `Unsat` there reports bugs that do not
    ///   exist.
    /// - `containing` — the inverted index from a term id to the cached sets holding it,
    ///   so a lookup costs time proportional to the *query* rather than to the cache.
    ///   §6 requires this: without it, the rules are correct and quadratic, and contracts
    ///   10–11 are specified at ≥1000 entries precisely to keep the scale honest.
    ///
    /// `arena` is which `TermArena` those ids belong to (§6.2) — see
    /// `counterexample_cache` for why a cache of bare indices has to know.
    arena: Option<u64>,
    sat_sets: Vec<(Vec<u32>, Model)>,
    unsat_sets: Vec<Vec<u32>>,
    containing: IndexMap<u32, Vec<CacheSlot>>,
    /// §6.2 makes slicing "required, not optional", so it is on unless switched off —
    /// which is what these two are phrased negatively for. `Default` is derived over a
    /// dozen fields and `bool::default()` is `false`; a positively-named `slicing: bool`
    /// would default a required optimisation to off, and nothing would fail.
    slicing_off: bool,
    /// Set while the path condition under test has `possibly_infeasible`. §6.1: "while it
    /// is set, slicing and the subset/superset cache rules are disabled".
    untrusted: bool,
    /// Models for individual components, keyed by the component's sorted term ids. §6.2
    /// calls the counterexample cache "per slice"; this is that, and it is what makes
    /// completing a sliced `Sat` model free after the first full check.
    slice_models: IndexMap<Vec<u32>, Model>,
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

    /// 022 §6's counterexample cache. `None` means "ask someone else".
    ///
    /// Every rule here is an implication about *sets*, and each is one line to justify:
    /// a superset of a contradiction is a contradiction; a model satisfying a superset
    /// satisfies this query. What is deliberately absent is the converse of the first —
    /// §6 calls a wrong direction here "silent and catastrophic", and the subset of an
    /// `Unsat` set is exactly where it would be wrong.
    fn counterexample_cache(&mut self, a: &mut TermArena, all: &[Term]) -> Option<CheckResult> {
        if all.is_empty() {
            return None;
        }
        // **The arena has to be the one the ids came from** (§6.2: "caches are
        // per-`TermArena`"). `check` takes an arena per call and every key is a bare
        // `Term` id, so a second arena's `Term(3)` is a different term with the same
        // name — and the subset rules turn that from an exact-collision hazard into a
        // subset one. Found by review; latent today because `Engine::run` consumes the
        // engine, and cheap to keep impossible.
        match self.arena {
            Some(id) if id == a.id() => {}
            Some(_) => {
                self.sat_sets.clear();
                self.unsat_sets.clear();
                self.containing.clear();
                self.cache.clear();
                self.arena = Some(a.id());
                return None;
            }
            None => self.arena = Some(a.id()),
        }
        let mut want: Vec<u32> = all.iter().map(|t| t.0).collect();
        want.sort_unstable();
        want.dedup();

        // **Candidates come from the inverted index**, never from the whole cache: a set
        // that subsumes or is subsumed by this query shares at least one term with it, so
        // the work is proportional to the query rather than to the cache. §6 requires
        // this, and contracts 10–11 are specified at ≥1000 entries to keep it honest —
        // a scan is correct and turns a 100 ms suite into a 90 s one.
        let mut cand: Vec<CacheSlot> = want
            .iter()
            .filter_map(|id| self.containing.get(id))
            .flatten()
            .copied()
            .collect();
        cand.sort_unstable();
        cand.dedup();

        // Rule: a **superset** of a known-`Unsat` set is `Unsat`. Capped for the same
        // reason as the sat side, and the subset test is cheap enough that the cap is
        // generous.
        for c in cand
            .iter()
            .filter(|c| !c.sat)
            .take(Self::MAX_CANDIDATES * 8)
        {
            let set = &self.unsat_sets[c.idx];
            if set.iter().all(|id| want.binary_search(id).is_ok()) {
                return Some(CheckResult::Unsat);
            }
        }

        // Rule: a **subset** of a known-`Sat` set is `Sat` with that set's model, and any
        // cached model that satisfies this query answers it. The second subsumes the
        // first, and both are settled by *evaluating* the model rather than by trusting
        // the set relation — which is what keeps a returned model honest about the query
        // it is returned for.
        //
        // **Bounded evaluation.** Sibling states share long path-condition prefixes by
        // design (023 §1), so a shared term puts *every* cached set in `containing[id]`
        // and the index that made enumeration cheap does nothing for evaluation. Measured
        // before this cap: 14.7 s for 1000 states against 0.13 s with the lookup removed,
        // roughly cubic — on an engine that budgets 10 000 live states. A miss costs one
        // backend call; an unbounded scan costs the run. Found by review.
        for c in cand.iter().filter(|c| c.sat).take(Self::MAX_CANDIDATES) {
            let m = &self.sat_sets[c.idx].1;
            if all
                .iter()
                .all(|t| a.eval(m, *t).map(|v| v.bits() != 0) == Ok(true))
            {
                return Some(CheckResult::Sat(m.clone()));
            }
        }
        None
    }

    /// How many cached sets a lookup may evaluate before giving up and asking a backend.
    const MAX_CANDIDATES: usize = 32;

    /// How many sets of each kind the counterexample cache holds. §6.2: "the
    /// counterexample cache is bounded by a documented entry count with LRU eviction — it
    /// is a known memory hog in KLEE, and 'cleared with the arena' is not a policy."
    /// A `Model` per satisfiable set is the memory here, so the number is modest.
    const MAX_SETS: usize = 4096;

    /// Record a decided set in the subsumption index.
    ///
    /// ⚠️ **This becomes coupled to §6.1 the moment slicing lands.** Today every stored
    /// set is a full assertion set, so the superset rule is monotone and correct whatever
    /// produced the contradiction — which is why `possibly_infeasible` being unimplemented
    /// is not an unsoundness yet. With slicing, `remember` would be storing *components*,
    /// and a component-level `Unsat` applied to a full query is exactly what §6.1 forbids:
    /// "while it is set, slicing and the subset/superset cache rules are disabled". The
    /// next wave will not see that coupling unless it is written here.
    fn remember(&mut self, ids: Vec<u32>, model: Option<Model>) {
        // **The slot carries which vector it indexes.** A bare `usize` made entry 3 of
        // `sat_sets` and entry 3 of `unsat_sets` the same candidate, so a query could be
        // answered from the wrong one — the silent direction, by a different route.
        let slot = CacheSlot {
            sat: model.is_some(),
            idx: match &model {
                Some(_) => self.sat_sets.len(),
                None => self.unsat_sets.len(),
            },
        };
        for id in &ids {
            self.containing.entry(*id).or_default().push(slot);
        }
        match model {
            Some(m) => self.sat_sets.push((ids, m)),
            None => self.unsat_sets.push(ids),
        }
        // **Eviction is wholesale, not least-recently-used.** `CacheSlot` is a positional
        // index into these vectors, so dropping one entry would invalidate every slot
        // after it in `containing`; a true LRU needs stable ids, which is a bigger change
        // than this bound is worth. Clearing at the bound keeps the memory claim honest
        // and loses only speed — every rule is a *shortcut*, never the only route to an
        // answer. §6.2 asks for a documented entry count with eviction; this is the
        // count, and the eviction is documented as the blunt one it is.
        if self.sat_sets.len() > Self::MAX_SETS || self.unsat_sets.len() > Self::MAX_SETS {
            self.sat_sets.clear();
            self.unsat_sets.clear();
            self.containing.clear();
        }
    }

    /// Send every tier-1 answer to tier 2 as well and assert agreement (022 §6).
    /// Too slow for production, mandatory in CI.
    /// Turn independence slicing off. §6.2 requires it, so this exists for the
    /// differential test that checks a sliced query answers what the unsliced one does.
    pub fn set_slicing(&mut self, on: bool) {
        self.slicing_off = !on;
    }

    /// Check a path condition, honouring §6.1.
    ///
    /// `assumptions` is **the query** — the constraint whose feasibility is being asked
    /// about, typically a branch condition. Slicing needs to know which variables the
    /// question is about; a bare `check` where everything sits in the assertion stack
    /// cannot slice, because every component is then relevant.
    ///
    /// A *full* check (no assumptions) that comes back `Sat` clears the path condition's
    /// flag. A check *with* assumptions proves something else satisfiable and must not:
    /// `pc ∧ c` being satisfiable does imply `pc` is, but `pc ∧ c` coming back `Sat` from
    /// a sliced query does not, since the slice may not have covered all of `pc`.
    pub fn check_path(
        &mut self,
        a: &mut TermArena,
        pc: &mut PathCondition,
        assumptions: &[Term],
    ) -> CheckResult {
        let saved = std::mem::replace(&mut self.asserted, pc.terms().to_vec());
        let saved_scopes = std::mem::take(&mut self.scopes);
        self.untrusted = pc.possibly_infeasible();
        let r = self.check(a, assumptions);
        self.untrusted = false;
        self.asserted = saved;
        self.scopes = saved_scopes;
        if assumptions.is_empty() && matches!(r, CheckResult::Sat(_)) {
            pc.mark_satisfiable();
        }
        r
    }

    /// Partition `all` into connected components by shared variables (§6.2), returning
    /// the components that mention one of `query_vars` first.
    ///
    /// Terms with **no variables** are their own component and never relevant, which is
    /// correct: a ground assertion is `true` or `false` on its own and the arena folds it
    /// long before here.
    fn components(a: &TermArena, all: &[Term], query_vars: &[VarId]) -> (Vec<Term>, Vec<Term>) {
        // Union-find over term indices, joined through a first-seen-owner per variable.
        let mut parent: Vec<usize> = (0..all.len()).collect();
        fn find(p: &mut [usize], mut i: usize) -> usize {
            while p[i] != i {
                p[i] = p[p[i]];
                i = p[i];
            }
            i
        }
        let mut owner: IndexMap<VarId, usize> = IndexMap::new();
        let mut term_vars: Vec<Vec<VarId>> = Vec::with_capacity(all.len());
        for (i, t) in all.iter().enumerate() {
            let mut vs = Vec::new();
            a.vars_of(*t, &mut vs);
            vs.sort_unstable();
            vs.dedup();
            for v in &vs {
                match owner.get(v) {
                    Some(&j) => {
                        let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                        if ri != rj {
                            parent[ri] = rj;
                        }
                    }
                    None => {
                        owner.insert(*v, i);
                    }
                }
            }
            term_vars.push(vs);
        }

        // The roots the query's variables land in.
        let mut wanted: Vec<usize> = Vec::new();
        for v in query_vars {
            if let Some(&j) = owner.get(v) {
                let r = find(&mut parent, j);
                if !wanted.contains(&r) {
                    wanted.push(r);
                }
            }
        }

        let mut relevant = Vec::new();
        let mut rest = Vec::new();
        for i in 0..all.len() {
            let r = find(&mut parent, i);
            // A variable-free term goes with the relevant side: it costs the backend
            // nothing and dropping it would change the query.
            if term_vars[i].is_empty() || wanted.contains(&r) {
                relevant.push(all[i]);
            } else {
                rest.push(all[i]);
            }
        }
        (relevant, rest)
    }

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
    /// Ask the backend, slicing the assertion set to the query's components first (§6.2).
    ///
    /// **Skipping is sound for `Unsat` unconditionally and for `Sat` only under §6.1's
    /// invariant** — a component that is quietly unsatisfiable makes a sliced `Sat` a
    /// lie. Rather than rely on the invariant alone, a sliced `Sat` is *completed*: every
    /// skipped component contributes its own model, from `slice_models` when it is
    /// already known and from the backend when it is not. Two things fall out of that,
    /// and both matter more than the saved query:
    ///
    /// - the model is complete and satisfies the **whole** path condition, which is what
    ///   023 contract 16 needs of a witness. A model covering only the query's component
    ///   keeps the verdict and produces witnesses that do not satisfy the constraints
    ///   they are offered as evidence for.
    /// - a skipped component that turns out `Unsat` is caught rather than papered over,
    ///   so the failure §6.1 describes cannot occur even when the flag is wrong.
    ///
    /// The saving is real where it pays: an infeasible branch — the case worth pruning —
    /// is refuted from its own component and the rest of the path condition is never
    /// sent. And after one full check the other components are all in `slice_models`, so
    /// completion costs no backend call.
    fn ask_backend(&mut self, a: &TermArena, all: &[Term], query: &[Term]) -> Option<CheckResult> {
        if self.slicing_off || self.untrusted || query.is_empty() {
            return self.ask_backend_raw(a, all);
        }
        let mut qvars = Vec::new();
        for t in query {
            a.vars_of(*t, &mut qvars);
        }
        qvars.sort_unstable();
        qvars.dedup();
        if qvars.is_empty() {
            return self.ask_backend_raw(a, all);
        }
        let (relevant, rest) = Self::components(a, all, &qvars);
        if rest.is_empty() {
            return self.ask_backend_raw(a, all);
        }
        self.stats.sliced_terms_skipped += rest.len() as u64;

        let mut model = match self.ask_backend_raw(a, &relevant)? {
            CheckResult::Sat(m) => m,
            // `Unsat` from a subset of the constraints refutes the whole set, whatever
            // the other components say. This is the direction the win is in.
            other => return Some(other),
        };
        self.remember_slice(a, &relevant, &model);

        // Complete the model over the components the query did not touch.
        let (mut comp_all, mut seen) = (Vec::new(), Vec::new());
        for t in &rest {
            let mut vs = Vec::new();
            a.vars_of(*t, &mut vs);
            comp_all.push((t, vs));
        }
        while let Some(i) = (0..comp_all.len()).find(|i| !seen.contains(i)) {
            // Peel one connected component off `rest` by re-partitioning around the
            // first unvisited term's variables.
            let seed = comp_all[i].1.clone();
            let remaining: Vec<Term> = (0..comp_all.len())
                .filter(|j| !seen.contains(j))
                .map(|j| *comp_all[j].0)
                .collect();
            let (comp, _) = Self::components(a, &remaining, &seed);
            for (j, (t, _)) in comp_all.iter().enumerate() {
                if !seen.contains(&j) && comp.contains(t) {
                    seen.push(j);
                }
            }
            match self.slice_model(a, &comp)? {
                Some(m) => model.merge_from(&m),
                // The invariant was wrong: some other part of the path condition is
                // dead. That refutes the whole set, and it is the answer §6.1 is about.
                None => return Some(CheckResult::Unsat),
            }
        }
        Some(CheckResult::Sat(model))
    }

    /// A component's model, from the per-slice cache or the backend. `None` means the
    /// component is unsatisfiable; the outer `Option` is a backend failure.
    fn slice_model(&mut self, a: &TermArena, comp: &[Term]) -> Option<Option<Model>> {
        let key = Self::slice_key(comp);
        if let Some(m) = self.slice_models.get(&key) {
            return Some(Some(m.clone()));
        }
        match self.ask_backend_raw(a, comp)? {
            CheckResult::Sat(m) => {
                self.slice_models.insert(key, m.clone());
                Some(Some(m))
            }
            CheckResult::Unsat => Some(None),
            CheckResult::Unknown(_) => None,
        }
    }

    fn slice_key(comp: &[Term]) -> Vec<u32> {
        let mut k: Vec<u32> = comp.iter().map(|t| t.0).collect();
        k.sort_unstable();
        k.dedup();
        k
    }

    fn remember_slice(&mut self, _a: &TermArena, comp: &[Term], m: &Model) {
        self.slice_models.insert(Self::slice_key(comp), m.clone());
    }

    fn ask_backend_raw(&mut self, a: &TermArena, all: &[Term]) -> Option<CheckResult> {
        let path = self.backend.as_ref()?.path().to_path_buf();
        let mut vars = Vec::new();
        for t in all {
            a.vars_of(*t, &mut vars);
        }

        let mut r = self.query(a, &path, all, &vars);
        if r.is_none() {
            // **A timeout is not retried.** The watchdog has already killed the process;
            // restarting and asking the same question spends the budget twice to learn the
            // same thing, and turns one slow query into two. The session is still dropped,
            // so the *next* query gets a fresh process with every variable redeclared —
            // which is 022 §4's restart-and-replay, and contract 14's correctness comes
            // from `query` rebuilding the declarations rather than from anything here.
            let watchdog_fired = self.session.as_ref().is_some_and(|s| s.timed_out);
            self.session = None;
            if watchdog_fired {
                self.stats.backend_timeouts += 1;
                return Some(CheckResult::Unknown(UnknownReason::Timeout));
            }
            // The process died. Restart and replay, then try once more; a second
            // failure is a real backend error rather than a transient one.
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
                    self.stats.backend_errors += 1;
                    Some(CheckResult::Unknown(UnknownReason::BackendError(
                        "backend model failed independent evaluation".into(),
                    )))
                }
            }
            Some((false, _)) => Some(CheckResult::Unsat),
            None => {
                // **Counted** (022 contract 15). Without it a backend that answers
                // nothing usable is invisible: every query comes back `Unknown`, every
                // consumer degrades honestly, and a run that decided nothing at all
                // reads as a run over a hard program.
                self.stats.backend_errors += 1;
                Some(CheckResult::Unknown(UnknownReason::BackendError(
                    "backend gave no usable answer".into(),
                )))
            }
        }
    }

    /// Write one standalone SMT-LIB2 script for `all` (022 §4, contract 17).
    ///
    /// Failures are ignored on purpose. A dump is a debugging aid, and a run that cannot
    /// write one should still answer the question it was asked — turning an unwritable
    /// directory into a solver error would make the diagnostic worse than the thing it
    /// diagnoses.
    fn dump_query(&mut self, a: &TermArena, dir: &std::path::Path, all: &[Term], vars: &[VarId]) {
        use std::io::Write;
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let mut script = String::from("(set-logic ALL)\n");
        for v in vars {
            let (name, sort) = &a.vars[v.0 as usize];
            script.push_str(&format!(
                "(declare-const {} {})\n",
                smt_name(v, name),
                smt_sort(*sort)
            ));
        }
        for t in all {
            script.push_str(&format!("(assert {})\n", a.to_smtlib(*t)));
        }
        script.push_str("(check-sat)\n");
        // Process id and a per-process counter, so two solvers in one run and two runs in
        // one directory do not overwrite each other's evidence.
        let n = NEXT_DUMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!("query-{}-{n:06}.smt2", std::process::id()));
        if let Ok(mut f) = std::fs::File::create(&path) {
            let _ = f.write_all(script.as_bytes());
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
            self.session = Session::spawn(path, self.timeout.0);
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

        // **A dump is a reconstruction, not a transcript.** `decls` above holds only the
        // variables *this process* has not seen, because the session is long-lived and
        // re-declaring is an error. Writing the bytes on the wire would therefore produce
        // a file that replays only if every earlier query in the session is replayed first
        // and in order — and contract 17 asks for a file that works *standalone*. So the
        // dump declares everything the query mentions, whatever the process already knows,
        // and drops the `push`/`pop` framing that only matters to a reused process.
        //
        // Written before the query is sent, deliberately: the query worth having a file for
        // is the one that never comes back.
        if let Some(dir) = self.dump.0.clone() {
            self.dump_query(a, &dir, all, vars);
        }
        let s = self.session.as_mut()?;

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
                // **The counterexample cache sits between the tiers, not above them.**
                // Above tier 1 it could answer a query tier 1 decides — including one
                // outside §3.2's fragment, where it was assigning a truth value to a
                // non-predicate term that both tiers would have refused. §6 puts the
                // caches "below escalation"; this is that, one level finer. Found by
                // review.
                // §6.1: the subset/superset rules are disabled while the path
                // condition may be infeasible. They are stated over *full* assertion
                // sets, and a set that is not known satisfiable is not one they hold for.
                if !self.untrusted
                    && let Some(r) = self.counterexample_cache(a, &all)
                {
                    return r;
                }
                match self.ask_backend(a, &all, assumptions) {
                    Some(r) => r,
                    None => tier1,
                }
            }
            _ => {
                if self.paranoid
                    && self.backend.is_some()
                    && let Some(t2) = self.ask_backend(a, &all, &[])
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
        let mut ids: Vec<u32> = all.iter().map(|t| t.0).collect();
        ids.sort_unstable();
        ids.dedup();
        match &decided {
            CheckResult::Sat(m) => {
                self.cache.insert(key, Some(m.clone()));
                self.remember(ids, Some(m.clone()));
            }
            CheckResult::Unsat => {
                self.cache.insert(key, None);
                self.remember(ids, None);
            }
            CheckResult::Unknown(_) => {}
        }
        self.stats.cache_entries = self.cache.len();
        decided
    }
}

/// Spread the bits of `n` across the set positions of `mask_bits`, low to high.
///
/// `None` when `n` needs more positions than the mask has — the index has run past the end
/// of the domain, not an error.
fn scatter(mask_bits: u128, mut n: u128, width: u32) -> Option<u128> {
    let mut v = 0u128;
    for b in 0..width {
        if mask_bits & (1u128 << b) != 0 {
            if n & 1 != 0 {
                v |= 1u128 << b;
            }
            n >>= 1;
        }
    }
    (n == 0).then_some(v)
}

/// The inverse of [`scatter`]: collect `v`'s bits at the set positions of `mask_bits` into a
/// dense index.
fn gather(mask_bits: u128, v: u128, width: u32) -> u128 {
    let mut r = 0u128;
    let mut i = 0u32;
    for b in 0..width {
        if mask_bits & (1u128 << b) != 0 {
            if v & (1u128 << b) != 0 {
                r |= 1u128 << i;
            }
            i += 1;
        }
    }
    r
}

#[cfg(test)]
mod smt_timeout_tests {
    use super::smt_timeout_ms;
    use std::time::Duration;

    /// The relationship the option exists for: the solver must run out of time **before**
    /// the watchdog kills it, at every scale.
    ///
    /// A fraction rather than a fixed margin, so this holds at 500ms and at an hour —
    /// subtracting a constant second would be wrong at the first and pointless at the
    /// second.
    #[test]
    fn the_solver_budget_expires_before_the_watchdog() {
        for secs in [1u64, 5, 10, 60, 3600] {
            let w = Duration::from_secs(secs);
            let ms = smt_timeout_ms(w);
            assert!(ms > 0, "a zero budget refuses every query at {secs}s");
            assert!(
                u128::from(ms) < w.as_millis(),
                "at {secs}s the solver would still be working when the watchdog fired"
            );
        }
    }

    /// **An unbounded watchdog means an unbounded solver.**
    ///
    /// `$CHIERO_SMT_TIMEOUT=0` is the escape hatch for someone bisecting a genuinely slow
    /// query who would rather wait than be told `Unknown(Timeout)`. Handing the solver a
    /// tiny budget there would defeat exactly that: chiero would wait forever for a process
    /// that had already given up. z3 reads `:timeout 0` as no limit.
    #[test]
    fn an_unbounded_watchdog_gives_the_solver_no_limit() {
        assert_eq!(smt_timeout_ms(Duration::MAX), 0);
    }

    /// A watchdog so short that nine tenths rounds to nothing still has to leave the solver
    /// a millisecond, or the option refuses every query outright.
    #[test]
    fn a_tiny_watchdog_still_leaves_a_positive_budget() {
        assert!(smt_timeout_ms(Duration::from_millis(1)) >= 1);
    }
}
