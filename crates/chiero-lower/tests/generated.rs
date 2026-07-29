//! **Generated differential testing.** Covers: 015 contract 5, mechanically.
//!
//! `differential.rs` is the same oracle driven by hand-written fixtures, and it works — but
//! every defect it has found was one somebody thought to spell. Six waves of the rules
//! earned in HANDOFF §9 are cross-product failures: a guard added to one arm and not its
//! sibling, a fix that reached the global path and stopped, every fixture putting the
//! pointer on the left. Those are not failures of care. They are what happens when the
//! constructs are enumerated in a thousand places instead of one.
//!
//! So this file enumerates them **once**, in a grammar, and explores the spellings ×
//! contexts × operand orders mechanically. The grammar is the thing to audit against C11
//! and 013 §4; everything else falls out of it.
//!
//! ## The verdict is five-way, not pass/fail
//!
//! A generator that cannot tell "chiero has not implemented this" from "chiero is wrong"
//! is useless, and one that treats the first as acceptable forever is worse. So:
//!
//! - `Agree` — the answers match.
//! - `Mismatch` — they differ. A defect.
//! - `Panic` — chiero panicked. A defect, and the worst kind: it takes the run with it.
//! - `SilentNoState` — **every stage was clean and the engine still produced no value.**
//!   Always a defect. `harness::lower` panics on any diagnostic, so reaching the engine at
//!   all means nothing refused; 015 §7's rule is that a gap is a diagnostic rather than a
//!   licence, and this is that rule made mechanical.
//! - `Refused { stage }` — a diagnostic was pushed. **Not** a defect: it is a gap behaving
//!   correctly. Counted, reported, and ratcheted (below), never silently tolerated.
//!
//! ## Undefined behaviour is discarded, not engineered away
//!
//! Building a provably-UB-free generator is a multi-month project that buys little here.
//! Instead the generator avoids the UB it can avoid *by construction* — one side effect per
//! statement, division only by nonzero constants, shift counts masked below the width,
//! indices in range because the generator knows every array's length — and everything else
//! is caught by compiling the fixture under `-fsanitize=undefined,address,float-cast-overflow` and discarding
//! any program that trips it. gcc at `-O0` stays the verdict, exactly as `differential.rs`
//! chose: optimisation is legal only on defined programs.

mod harness;

use std::fmt::Write as _;

// ---------------------------------------------------------------------------------------
// A PRNG, written out rather than depended on
// ---------------------------------------------------------------------------------------

/// xorshift64*, ~20 lines against a `rand` dependency that would earn its keep nowhere else
/// in the workspace. 001 §4's dependency rules are checked by `xtask check-deps`, and the
/// cheapest way to keep that gate quiet is to need nothing.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        // Any nonzero state; xorshift is degenerate at 0.
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }

    fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
}

// ---------------------------------------------------------------------------------------
// The grammar
// ---------------------------------------------------------------------------------------

/// The scalar types the generator draws from.
///
/// `_Bool` is here from the start because it is where a whole defect class lives: its CIR
/// type is one bit and its storage is a byte, and a compound assignment on one used to
/// panic the solver outright. Floats are deliberately absent — they do not execute at all
/// (HANDOFF §9), so every float program would land in the refusal ledger and drown the
/// signal that ledger exists to carry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ty {
    Bool,
    F32,
    F64,
    SChar,
    UChar,
    Short,
    UShort,
    Int,
    UInt,
    Long,
    ULong,
}

impl Ty {
    fn c(self) -> &'static str {
        match self {
            Ty::Bool => "_Bool",
            Ty::F32 => "float",
            Ty::F64 => "double",
            Ty::SChar => "signed char",
            Ty::UChar => "unsigned char",
            Ty::Short => "short",
            Ty::UShort => "unsigned short",
            Ty::Int => "int",
            Ty::UInt => "unsigned",
            Ty::Long => "long",
            Ty::ULong => "unsigned long",
        }
    }

    /// Width in bits on the one target chiero models (`TargetConfig::x86_64_linux`).
    fn bits(self) -> u32 {
        match self {
            Ty::Bool => 1,
            Ty::F32 => 32,
            Ty::F64 => 64,
            Ty::SChar | Ty::UChar => 8,
            Ty::Short | Ty::UShort => 16,
            Ty::Int | Ty::UInt => 32,
            Ty::Long | Ty::ULong => 64,
        }
    }

    fn signed(self) -> bool {
        matches!(
            self,
            Ty::SChar | Ty::Short | Ty::Int | Ty::Long | Ty::F32 | Ty::F64
        )
    }

    /// Floating types take a different constant syntax and cannot be masked into range.
    fn is_float(self) -> bool {
        matches!(self, Ty::F32 | Ty::F64)
    }

    const ALL: [Ty; 11] = [
        Ty::Bool,
        Ty::F32,
        Ty::F64,
        Ty::SChar,
        Ty::UChar,
        Ty::Short,
        Ty::UShort,
        Ty::Int,
        Ty::UInt,
        Ty::Long,
        Ty::ULong,
    ];
}

/// A variable in scope: its C name, its type, and whether it is an array element source.
#[derive(Clone)]
struct Var {
    name: String,
    ty: Ty,
}

/// The constant pool.
///
/// **Adversarial on purpose.** A pool of small random integers misses every boundary that
/// has actually mattered: the wide-enum defect needed a value above 2^32, the sign-extension
/// defects needed values that differ read signed and unsigned, and the bit-field truncation
/// needed a value one past a field's range. Boundaries are where lowering's width and
/// signedness decisions become visible.
const POOL: [i128; 21] = [
    0,
    1,
    2,
    3,
    4,
    7,
    -1,
    -2,
    -4,
    127,
    128,
    255,
    256,
    32767,
    32768,
    65535,
    2147483647,
    -2147483648,
    4294967295,
    4294967296,
    5000000000,
];

/// A generated `struct` type: its C tag and its members' types.
#[derive(Clone)]
struct Rec {
    tag: String,
    fields: Vec<Ty>,
    /// `Some(width)` makes the member a bit-field of that width.
    ///
    /// Bit-fields have their own arithmetic — the value is truncated to the field and
    /// reinterpreted at its own signedness — and wave 136 found a read-modify-write on one
    /// clobbering its neighbours. A grammar without them cannot see any of that.
    widths: Vec<Option<u32>>,
    /// A `union` rather than a `struct`. Every member overlaps, so the generator writes and
    /// reads **one** member per union: reading a member other than the one last stored is
    /// unspecified in C, and an unspecified program teaches nothing.
    is_union: bool,
}

impl Rec {
    fn field(&self, i: usize) -> String {
        format!("f{i}")
    }

    fn kw(&self) -> &'static str {
        if self.is_union { "union" } else { "struct" }
    }

    /// The members a generated program may read.
    ///
    /// For a union that is **only the first**, because every member overlaps and reading
    /// one that was not last stored is unspecified. Initialising a union with `{v}` sets
    /// the first member (C11 6.7.9p17), so the first is the one that is always defined.
    fn readable(&self) -> usize {
        if self.is_union { 1 } else { self.fields.len() }
    }
}

/// A generated helper: its name, parameter types and return type.
///
/// **`None` return means the helper returns a struct**, named by `ret_rec`. Wave 132's
/// struct-parameter defect — the callee reading its fields out of a pointer's own bytes,
/// silently, for a plausible wrong number — is invisible to any generator that cannot emit
/// one of these.
#[derive(Clone)]
struct Fun {
    name: String,
    params: Vec<Param>,
    ret: Option<Ty>,
    ret_rec: Option<usize>,
}

#[derive(Clone)]
enum Param {
    Scalar(Ty),
    Struct(usize),
}

struct Gen {
    rng: Rng,
    vars: Vec<Var>,
    /// Struct-typed locals in scope, as (name, record index).
    recs_in_scope: Vec<(String, usize)>,
    /// Arrays in scope, as (name, element type, length). The length is kept so every
    /// generated index can be **in range by construction** — an out-of-bounds read is UB
    /// and would be discarded rather than compared, which wastes the program.
    arrays: Vec<(String, Ty, usize)>,
    body: String,
    next_id: usize,
    depth: usize,
    /// The record and helper definitions this program needs, emitted as its prelude.
    records: Vec<Rec>,
    funs: Vec<Fun>,
    /// File-scope objects, as (name, type). Wave 132's pointer-global defect — reading
    /// `int *gp` as a value yielded the address *of* `gp` rather than the one it holds —
    /// lived here and is unreachable from a function-body-only grammar, which is why
    /// `agree_with` grew a prelude parameter in the first place.
    globals: Vec<Var>,
    /// A file-scope array, as (name, element type, length).
    global_arrays: Vec<(String, Ty, usize)>,
    /// A file-scope pointer and the array it is aimed at.
    global_ptrs: Vec<(String, Ty, String, usize)>,
    /// **Allow an index to leave its array.** Off for every channel that compares *values*,
    /// because an out-of-bounds access is undefined and `judge` discards the program rather
    /// than grading it. On for the memory-UB oracle, whose subject is exactly the programs
    /// the others throw away.
    ///
    /// A knob rather than a second grammar: the shapes worth indexing out of are the ones
    /// already here — local arrays, file-scope arrays, a global pointer aimed at an array,
    /// read and written through all of `access`'s spellings — and forking the grammar to
    /// get them would mean maintaining two of everything and grading the copy.
    oob: bool,
}

impl Gen {
    fn new(seed: u64) -> Gen {
        Gen {
            rng: Rng::new(seed),
            vars: Vec::new(),
            recs_in_scope: Vec::new(),
            arrays: Vec::new(),
            body: String::new(),
            next_id: 0,
            depth: 0,
            records: Vec::new(),
            funs: Vec::new(),
            globals: Vec::new(),
            global_arrays: Vec::new(),
            global_ptrs: Vec::new(),
            oob: false,
        }
    }

    /// An index into an array of `len` elements — in range, unless [`Gen::oob`] says
    /// otherwise.
    ///
    /// **Past the end rather than before the start.** A negative index is equally undefined
    /// and `access` takes a `usize`, so spelling one would mean threading a sign through
    /// every access form for a case ASan reports identically. Recorded as a limit rather
    /// than left as an accident: nothing here generates a negative index, so nothing grades
    /// chiero on one, and its own probe showed it handles them.
    ///
    /// One in three, not every time. A program whose every access is out of bounds dies on
    /// the first one, so ASan reports a single site however many the program contains, and
    /// the corpus stops covering the shapes that follow it.
    fn index(&mut self, len: usize) -> usize {
        if self.oob && self.rng.chance(3) {
            return len + self.rng.below(4);
        }
        self.rng.below(len)
    }

    fn fresh(&mut self) -> String {
        self.next_id += 1;
        format!("v{}", self.next_id)
    }

    /// Declare the records and helpers this program may use.
    ///
    /// Emitted before `probe`, which is what `agree_with`'s prelude parameter exists for
    /// and what a body-only generator cannot reach: a struct passed **by value** to a
    /// helper, and a struct **returned** from one, are two of the four shapes wave 132's
    /// missing guard broke, and neither is expressible inside one function.
    fn prelude(&mut self) -> String {
        let nrec = 1 + self.rng.below(2);
        for i in 0..nrec {
            let nf = 1 + self.rng.below(3);
            let fields: Vec<Ty> = (0..nf)
                .map(|_| {
                    // `_Bool` members are excluded: a `_Bool` field's *padding* is
                    // unspecified, so two structs with equal members can compare unequal
                    // byte-wise and the checksum would be reading something C does not
                    // define. The scalar `_Bool` cases live in `differential.rs`.
                    let t = *self.rng.pick(&Ty::ALL);
                    if t == Ty::Bool { Ty::Int } else { t }
                })
                .collect();
            let is_union = self.rng.chance(4);
            // Bit-fields only in structs, and only on `int`/`unsigned` members — those are
            // the two 013 §4 measured in VPP, and a bit-field of `char` is a gcc extension
            // whose layout is its own question.
            let widths: Vec<Option<u32>> = fields
                .iter()
                .map(|t| {
                    if is_union || !matches!(t, Ty::Int | Ty::UInt) || !self.rng.chance(2) {
                        None
                    } else {
                        Some(1 + self.rng.below(7) as u32)
                    }
                })
                .collect();
            self.records.push(Rec {
                tag: format!("S{i}"),
                fields,
                widths,
                is_union,
            });
        }
        let nfun = 1 + self.rng.below(3);
        for i in 0..nfun {
            let np = self.rng.below(3);
            let params: Vec<Param> = (0..np)
                .map(|_| {
                    if !self.records.is_empty() && self.rng.chance(2) {
                        Param::Struct(self.rng.below(self.records.len()))
                    } else {
                        Param::Scalar(*self.rng.pick(&Ty::ALL))
                    }
                })
                .collect();
            let returns_struct = !self.records.is_empty() && self.rng.chance(3);
            self.funs.push(Fun {
                name: format!("h{i}"),
                params,
                ret: if returns_struct {
                    None
                } else {
                    Some(*self.rng.pick(&Ty::ALL))
                },
                ret_rec: if returns_struct {
                    Some(self.rng.below(self.records.len()))
                } else {
                    None
                },
            });
        }
        // **File-scope objects.** A global is a different lookup path from a local at
        // every stage — sema resolves it through the global table, lowering emits
        // `AddrOfGlobal` rather than `AddrOfLocal` — and wave 132 found the two disagreeing.
        let ng = 1 + self.rng.below(3);
        for _ in 0..ng {
            let t = *self.rng.pick(&Ty::ALL);
            let t = if t == Ty::Bool { Ty::Int } else { t };
            let name = format!("g{}", self.next_id);
            self.next_id += 1;
            self.globals.push(Var { name, ty: t });
        }
        if self.rng.chance(2) {
            let t = *self.rng.pick(&Ty::ALL);
            let t = if t == Ty::Bool { Ty::Int } else { t };
            let len = 2 + self.rng.below(3);
            let name = format!("ga{}", self.next_id);
            self.next_id += 1;
            self.global_arrays.push((name.clone(), t, len));
            // A **file-scope pointer aimed at it**, initialised with an address constant —
            // which is its own path through 020 §6's `GlobalInit`.
            if self.rng.chance(2) {
                let pname = format!("gp{}", self.next_id);
                self.next_id += 1;
                self.global_ptrs.push((pname, t, name, len));
            }
        }
        self.render_prelude()
    }

    fn render_prelude(&mut self) -> String {
        let mut out = String::new();
        for v in &self.globals.clone() {
            let init = self.konst(v.ty);
            let _ = writeln!(out, "{} {} = {init};", v.ty.c(), v.name);
        }
        for (name, ty, len) in &self.global_arrays.clone() {
            let vals: Vec<String> = (0..*len).map(|_| self.konst(*ty)).collect();
            let _ = writeln!(out, "{} {name}[{len}] = {{{}}};", ty.c(), vals.join(", "));
        }
        for (pname, ty, arr, _) in &self.global_ptrs.clone() {
            let _ = writeln!(out, "{} *{pname} = {arr};", ty.c());
        }
        for r in &self.records.clone() {
            let kw = if r.is_union { "union" } else { "struct" };
            let _ = writeln!(out, "{kw} {} {{", r.tag);
            for (i, f) in r.fields.iter().enumerate() {
                match r.widths[i] {
                    Some(w) => {
                        let _ = writeln!(out, "  {} {}:{w};", f.c(), r.field(i));
                    }
                    None => {
                        let _ = writeln!(out, "  {} {};", f.c(), r.field(i));
                    }
                }
            }
            let _ = writeln!(out, "}};");
        }
        for f in &self.funs.clone() {
            let sig_ret = match (f.ret, f.ret_rec) {
                (_, Some(r)) => format!("{} {}", self.records[r].kw(), self.records[r].tag),
                (Some(t), _) => t.c().to_string(),
                _ => "int".into(),
            };
            let ps: Vec<String> = f
                .params
                .iter()
                .enumerate()
                .map(|(i, p)| match p {
                    Param::Scalar(t) => format!("{} p{i}", t.c()),
                    Param::Struct(r) => {
                        format!("{} {} p{i}", self.records[*r].kw(), self.records[*r].tag)
                    }
                })
                .collect();
            let args = if ps.is_empty() {
                "void".to_string()
            } else {
                ps.join(", ")
            };
            let _ = writeln!(out, "static {sig_ret} {}({args}) {{", f.name);
            // The body reads every parameter, so a parameter that arrives wrong is
            // observed rather than ignored — wave 132's `span_of` returned 28663 precisely
            // because the callee read its fields.
            match f.ret_rec {
                Some(r) => {
                    let tag = self.records[r].tag.clone();
                    let kw = self.records[r].kw();
                    let _ = writeln!(out, "  {kw} {tag} out;");
                    let nf = self.records[r].readable();
                    for i in 0..nf {
                        let mut e = format!("{}", 1 + i as i64);
                        for (pi, p) in f.params.iter().enumerate() {
                            match p {
                                Param::Scalar(_) => {
                                    let _ = write!(e, " + (long)p{pi}");
                                }
                                Param::Struct(pr) => {
                                    for fi in 0..self.records[*pr].readable() {
                                        let _ = write!(e, " + (long)p{pi}.f{fi}");
                                    }
                                }
                            }
                        }
                        let ft = self.records[r].fields[i];
                        // A bit-field's value is truncated to its width, which is exactly
                        // what wave 136's rule is about; the cast keeps the *source* in
                        // range so only the field's own truncation is under test.
                        let _ = writeln!(out, "  out.f{i} = ({})({e});", ft.c());
                    }
                    let _ = writeln!(out, "  return out;");
                }
                None => {
                    let t = f.ret.unwrap_or(Ty::Int);
                    let mut e = String::from("0");
                    for (pi, p) in f.params.iter().enumerate() {
                        match p {
                            Param::Scalar(_) => {
                                let _ = write!(e, " + (long)p{pi}");
                            }
                            Param::Struct(pr) => {
                                for fi in 0..self.records[*pr].readable() {
                                    let _ = write!(e, " + (long)p{pi}.f{fi}");
                                }
                            }
                        }
                    }
                    let _ = writeln!(out, "  return ({})({e});", t.c());
                }
            }
            let _ = writeln!(out, "}}");
        }
        out
    }

    /// A constant literal, suffixed so its own type is not in doubt.
    fn konst(&mut self, ty: Ty) -> String {
        // **Float constants are small integers.** The point is to exercise float *types* —
        // loads, stores, conversions, arithmetic — not float *precision*. A pool with
        // fractions would make gcc -O0, gcc -O2 and clang legitimately disagree on the last
        // bit and every such program would be discarded, testing nothing while looking
        // busy.
        if ty.is_float() {
            let v = self.rng.below(5);
            return if ty == Ty::F32 {
                format!("{v}.0f")
            } else {
                format!("{v}.0")
            };
        }
        let v = *self.rng.pick(&POOL);
        // Mask into the target type's range so the *initializer* is never itself a
        // conversion the generator did not intend. Signed overflow in a constant expression
        // is a constraint violation, not something to test here.
        let bits = ty.bits().min(64);
        let masked = if bits >= 64 {
            v
        } else {
            let m = (1i128 << bits) - 1;
            let raw = v & m;
            if ty.signed() && bits > 1 && raw >= (1i128 << (bits - 1)) {
                raw - (1i128 << bits)
            } else {
                raw
            }
        };
        if ty.bits() == 64 {
            if ty.signed() {
                format!("{masked}L")
            } else {
                format!("{}UL", masked as u128 as u64)
            }
        } else if ty.signed() {
            format!("{masked}")
        } else {
            format!("{masked}u")
        }
    }

    /// An expression of *some* type, with the type it produced.
    ///
    /// Depth-bounded rather than probability-bounded: a generator that recurses on a coin
    /// flip produces the occasional enormous program and makes shrinking the common case.
    fn expr(&mut self, want: Ty) -> String {
        if self.depth >= 3 {
            return self.leaf(want);
        }
        self.depth += 1;
        let e = match self.rng.below(10) {
            0..=2 => self.leaf(want),
            3..=5 => {
                // **Every binary operator, unparenthesised on purpose.** Precedence is what
                // wave 135 found unpinned, and volume settles it without anyone working out
                // which orderings discriminate.
                let op = *self
                    .rng
                    .pick(&["+", "-", "*", "|", "&", "^", "<", ">", "==", "!="]);
                let a = self.expr(want);
                let b = self.expr(want);
                format!("{a} {op} {b}")
            }
            6 => {
                // Division and remainder, by a **nonzero** constant only: `x / 0` is UB and
                // a generated program that hits it teaches nothing. `%` is integer-only.
                let op = if want.is_float() {
                    "/"
                } else {
                    *self.rng.pick(&["/", "%"])
                };
                let a = self.expr(want);
                let d = 1 + self.rng.below(15);
                format!("{a} {op} {d}")
            }
            7 if !want.is_float() => {
                // Shifts, with the count forced below the width — a count at or above it is
                // UB and would be discarded rather than compared. Integer-only.
                let op = *self.rng.pick(&["<<", ">>"]);
                let a = self.expr(want);
                let n = self.rng.below(want.bits().max(2) as usize - 1);
                format!("{a} {op} {n}")
            }
            8 => {
                let t = *self.rng.pick(&Ty::ALL);
                let a = self.expr(t);
                format!("({}){a}", want.c())
            }
            9 if !self.funs.is_empty() => {
                // **A call, with its arguments** — including structs passed by value, which
                // is the shape a body-only generator cannot produce at all.
                let f = self.funs[self.rng.below(self.funs.len())].clone();
                if let Some(r) = f.ret_rec {
                    // A struct-returning helper is not a scalar; use it through a member.
                    let args = self.args_for(&f);
                    let fi = self.rng.below(self.records[r].readable());
                    format!("({}){}({args}).f{fi}", want.c(), f.name)
                } else {
                    let args = self.args_for(&f);
                    format!("({}){}({args})", want.c(), f.name)
                }
            }
            _ => {
                let c = self.expr(Ty::Int);
                let a = self.expr(want);
                let b = self.expr(want);
                format!("{c} ? {a} : {b}")
            }
        };
        self.depth -= 1;
        format!("({e})")
    }

    /// **One access, spelled every way C spells it.**
    ///
    /// This is the production the defect record demands. Wave 132 found pointer arithmetic
    /// broken in `p + n`, `n + p`, `p - n`, `p += n`, `p++` and `--p` — *every* spelling
    /// except `a[i]`, which is the only one any hand-written fixture had used. A grammar
    /// that emits one spelling reproduces exactly that blind spot; this one picks among
    /// them, so a fix that reaches the subscript path and stops is caught by volume rather
    /// than by somebody remembering.
    ///
    /// `base` is an array's name and `i` an index already known to be in range.
    fn access(&mut self, base: &str, i: usize) -> String {
        match self.rng.below(6) {
            0 => format!("{base}[{i}]"),
            1 => format!("*({base} + {i})"),
            2 => format!("*({i} + {base})"),
            3 => format!("*(&{base}[{i}])"),
            // `&a[n] - k` walks back from a later element, which is where the sign of the
            // scaled offset shows: a zero-extended -1 addresses four billion elements away.
            4 => format!("*(&{base}[{i}] + 0)"),
            _ => format!("(&{base}[0])[{i}]"),
        }
    }

    fn leaf(&mut self, want: Ty) -> String {
        // **Through the file-scope pointer**, which is the shape wave 132 broke: reading
        // `gp` as a value must give the address it holds, not its own.
        if !self.global_ptrs.is_empty() && self.rng.chance(5) {
            let (pname, _, _, len) = self.global_ptrs[0].clone();
            let i = self.index(len);
            let a = self.access(&pname, i);
            return format!("({}){a}", want.c());
        }
        if !self.globals.is_empty() && self.rng.chance(4) {
            let v = self.globals[self.rng.below(self.globals.len())].clone();
            return format!("({}){}", want.c(), v.name);
        }
        if !self.global_arrays.is_empty() && self.rng.chance(4) {
            let (name, _, len) = self.global_arrays[0].clone();
            let i = self.index(len);
            let a = self.access(&name, i);
            return format!("({}){a}", want.c());
        }
        if !self.arrays.is_empty() && self.rng.chance(3) {
            let (name, _, len) = self.arrays[self.rng.below(self.arrays.len())].clone();
            let i = self.index(len);
            let a = self.access(&name, i);
            return format!("({}){a}", want.c());
        }
        let usable: Vec<Var> = self.vars.clone();
        if usable.is_empty() || self.rng.chance(3) {
            return self.konst(want);
        }
        let v = self.rng.pick(&usable).clone();
        format!("({}){}", want.c(), v.name)
    }

    /// Arguments for one call. A struct parameter is given a struct-typed local when one
    /// is in scope and a **compound literal** otherwise — which is also how the wave-138
    /// compound-literal work gets exercised by every program that needs an argument.
    fn args_for(&mut self, f: &Fun) -> String {
        let params = f.params.clone();
        let mut out: Vec<String> = Vec::new();
        for p in &params {
            match p {
                Param::Scalar(t) => {
                    let e = self.expr(*t);
                    out.push(e);
                }
                Param::Struct(r) => {
                    let candidates: Vec<(String, usize)> = self
                        .recs_in_scope
                        .iter()
                        .filter(|(_, ri)| ri == r)
                        .cloned()
                        .collect();
                    if !candidates.is_empty() && self.rng.chance(2) {
                        out.push(self.rng.pick(&candidates).0.clone());
                    } else {
                        let tag = self.records[*r].tag.clone();
                        let fields = self.records[*r].fields.clone();
                        let vals: Vec<String> = fields.iter().map(|t| self.konst(*t)).collect();
                        out.push(format!("(struct {tag}){{{}}}", vals.join(", ")));
                    }
                }
            }
        }
        out.join(", ")
    }

    /// One statement. **At most one side effect**, which removes unsequenced-modification
    /// UB and evaluation-order divergence between compilers by construction.
    fn stmt(&mut self) {
        match self.rng.below(10) {
            0 if self.arrays.len() < 3 && self.rng.chance(2) => {
                // An array with a braced initializer, which is also how wave 140's
                // element-conversion path gets exercised for every element type.
                let ty = *self.rng.pick(&Ty::ALL);
                let ty = if ty == Ty::Bool { Ty::Int } else { ty };
                let len = 2 + self.rng.below(3);
                let name = self.fresh();
                let vals: Vec<String> = (0..len).map(|_| self.konst(ty)).collect();
                let _ = writeln!(
                    self.body,
                    "  {} {name}[{len}] = {{{}}};",
                    ty.c(),
                    vals.join(", ")
                );
                self.arrays.push((name, ty, len));
            }
            3 if !self.globals.is_empty() && self.rng.chance(2) => {
                // Writing a global, which is `AddrOfGlobal` plus a store rather than the
                // local path — the two disagreed for six waves.
                let v = self.globals[self.rng.below(self.globals.len())].clone();
                let e = self.expr(v.ty);
                let _ = writeln!(self.body, "  {} = {e};", v.name);
            }
            4 if !self.global_arrays.is_empty() && self.rng.chance(2) => {
                let (name, ty, len) = self.global_arrays[0].clone();
                let i = self.index(len);
                let lhs = self.access(&name, i);
                let e = self.expr(ty);
                let _ = writeln!(self.body, "  {lhs} = {e};");
            }
            1 if !self.arrays.is_empty() => {
                // A write **through one of the spellings**, so the store path sees them all
                // and not only the subscript.
                let (name, ty, len) = self.arrays[self.rng.below(self.arrays.len())].clone();
                let i = self.index(len);
                let lhs = self.access(&name, i);
                let e = self.expr(ty);
                let _ = writeln!(self.body, "  {lhs} = {e};");
            }
            2 if !self.arrays.is_empty() && self.rng.chance(2) => {
                // **A pointer walked across the array**, which is the shape `p += n` and
                // `p++` live in. The bound keeps every dereference inside the object, so
                // the program stays defined.
                let (name, ty, len) = self.arrays[self.rng.below(self.arrays.len())].clone();
                let p = self.fresh();
                let start = self.rng.below(len);
                let _ = writeln!(self.body, "  {} *{p} = &{name}[{start}];", ty.c());
                let steps = self.rng.below(len - start);
                for _ in 0..steps {
                    let op = *self.rng.pick(&["++", "+= 1"]);
                    if op == "++" {
                        let _ = writeln!(self.body, "  {p}++;");
                    } else {
                        let _ = writeln!(self.body, "  {p} += 1;");
                    }
                }
                let e = self.expr(ty);
                let _ = writeln!(self.body, "  *{p} = {e};");
                // And read it back through a *different* spelling than it was written.
                let back = self.fresh();
                let _ = writeln!(
                    self.body,
                    "  {} {back} = {p}[0] + *({p} + 0) - {p}[0];",
                    ty.c()
                );
                self.vars.push(Var { name: back, ty });
            }
            0..=3 => {
                let ty = *self.rng.pick(&Ty::ALL);
                let name = self.fresh();
                let init = self.expr(ty);
                let _ = writeln!(self.body, "  {} {name} = {init};", ty.c());
                self.vars.push(Var { name, ty });
            }
            4..=5 => {
                if let Some(v) = self.pick_var() {
                    let e = self.expr(v.ty);
                    let _ = writeln!(self.body, "  {} = {e};", v.name);
                }
            }
            6 => {
                // Compound assignment — the shape that hid the bit-field defect and the
                // `_Bool` solver panic. `/=` and `>>=` are included deliberately: `+= 1`
                // followed by truncation absorbs a wrong sign-extension and these do not.
                if let Some(v) = self.pick_var() {
                    let op = if v.ty.is_float() {
                        *self.rng.pick(&["+=", "-=", "*="])
                    } else {
                        *self.rng.pick(&["+=", "-=", "*=", "|=", "&=", "^="])
                    };
                    let e = self.expr(v.ty);
                    let _ = writeln!(self.body, "  {} {op} {e};", v.name);
                }
            }
            7 => {
                if let Some(v) = self.pick_var() {
                    let op = *self.rng.pick(&["++", "--"]);
                    let pre = self.rng.chance(2);
                    if pre {
                        let _ = writeln!(self.body, "  {op}{};", v.name);
                    } else {
                        let _ = writeln!(self.body, "  {}{op};", v.name);
                    }
                }
            }
            8 if !self.records.is_empty() && self.rng.chance(2) => {
                // A struct local, initialized from a compound literal or from a
                // struct-returning helper — the aggregate-return path.
                let r = self.rng.below(self.records.len());
                let tag = self.records[r].tag.clone();
                let kw = self.records[r].kw();
                let name = self.fresh();
                let from_call = self
                    .funs
                    .iter()
                    .position(|f| f.ret_rec == Some(r))
                    .filter(|_| self.rng.chance(2));
                let init = match from_call {
                    Some(fi) => {
                        let f = self.funs[fi].clone();
                        let args = self.args_for(&f);
                        format!("{}({args})", f.name)
                    }
                    None => {
                        let fields = self.records[r].fields.clone();
                        let vals: Vec<String> = fields.iter().map(|t| self.konst(*t)).collect();
                        format!("(struct {tag}){{{}}}", vals.join(", "))
                    }
                };
                let _ = writeln!(self.body, "  {kw} {tag} {name} = {init};");
                self.recs_in_scope.push((name, r));
            }
            8 => {
                let c = self.expr(Ty::Int);
                let _ = writeln!(self.body, "  if ({c}) {{");
                self.nested();
                let _ = writeln!(self.body, "  }} else {{");
                self.nested();
                let _ = writeln!(self.body, "  }}");
            }
            _ => {
                // A loop with a **structurally bounded** trip count, so no generated
                // program can fail to terminate and no comparison can hang.
                if let Some(v) = self.pick_var() {
                    let n = 1 + self.rng.below(4);
                    let i = self.fresh();
                    let e = self.expr(v.ty);
                    let _ = writeln!(
                        self.body,
                        "  for (int {i} = 0; {i} < {n}; {i}++) {{ {} += {e}; }}",
                        v.name
                    );
                }
            }
        }
    }

    /// A nested block's statements. Variables declared inside do not escape, which is what
    /// makes the checksum below well defined.
    fn nested(&mut self) {
        let saved = self.vars.len();
        let saved_recs = self.recs_in_scope.len();
        let saved_arrays = self.arrays.len();
        for _ in 0..1 + self.rng.below(2) {
            self.stmt();
        }
        self.vars.truncate(saved);
        self.recs_in_scope.truncate(saved_recs);
        self.arrays.truncate(saved_arrays);
    }

    fn pick_var(&mut self) -> Option<Var> {
        if self.vars.is_empty() {
            return None;
        }
        let vars = self.vars.clone();
        Some(self.rng.pick(&vars).clone())
    }

    /// **The result is a checksum over every live variable**, not one expression's value.
    ///
    /// This is what buys neighbour-corruption detection for free: a write that lands on the
    /// wrong object changes the checksum even when the value it was supposed to produce is
    /// right. The hand-written bit-field fixtures had to spell `v.a * 100 + v.b` for the
    /// same reason, one fixture at a time.
    fn finish(mut self) -> String {
        let _ = writeln!(self.body, "  long acc = 0;");
        // File-scope objects are part of the program's state and are checksummed like any
        // other: a helper that writes one, or a store that lands on the wrong global,
        // changes the answer.
        let globals = self.globals.clone();
        for v in &globals {
            let _ = writeln!(self.body, "  acc = acc * 31 + (long)({});", v.name);
        }
        let garrays = self.global_arrays.clone();
        for (name, _, len) in &garrays {
            for i in 0..*len {
                let _ = writeln!(self.body, "  acc = acc * 31 + (long)({name}[{i}]);");
            }
        }
        let vars = self.vars.clone();
        for (i, v) in vars.iter().enumerate() {
            let _ = writeln!(
                self.body,
                "  acc = acc * 31 + (long)({}) + {};",
                v.name, i as i64
            );
        }
        // **Every field of every struct**, for the same reason as every scalar: a write
        // that lands on a neighbouring field changes the checksum even when the value it
        // was supposed to produce is right.
        let recs = self.recs_in_scope.clone();
        for (name, r) in &recs {
            for fi in 0..self.records[*r].readable() {
                let _ = writeln!(self.body, "  acc = acc * 31 + (long)({name}.f{fi});");
            }
        }
        // Every element of every array, for the same reason as every field: a write that
        // lands one element over changes the checksum even when its own value is right.
        let arrays = self.arrays.clone();
        for (name, _, len) in &arrays {
            for i in 0..*len {
                let _ = writeln!(self.body, "  acc = acc * 31 + (long)({name}[{i}]);");
            }
        }
        let _ = writeln!(self.body, "  return (int)acc;");
        self.body
    }
}

fn program(seed: u64) -> (String, String) {
    program_with(seed, false)
}

/// The same grammar with [`Gen::oob`] on, for the memory-UB oracle.
fn program_memory_ub(seed: u64) -> (String, String) {
    program_with(seed, true)
}

fn program_with(seed: u64, oob: bool) -> (String, String) {
    let mut g = Gen::new(seed);
    g.oob = oob;
    let prelude = g.prelude();
    let n = 3 + g.rng.below(7);
    for _ in 0..n {
        g.stmt();
    }
    (prelude, g.finish())
}

// ---------------------------------------------------------------------------------------
// The verdict
// ---------------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Agree,
    Mismatch {
        chiero: Option<i32>,
        gcc: i32,
    },
    Panic(String),
    SilentNoState {
        gcc: i32,
    },
    Refused {
        stage: &'static str,
        message: String,
    },
    /// **Lowering emitted CIR the verifier rejects, and refused nothing.** A defect, and a
    /// distinct one: the engine's silence is a *consequence* here, not the fault. 015 §7
    /// refuses what lowering knows it cannot represent; this is the class it does not know
    /// about, so it needs its own name or it hides inside `SilentNoState`.
    InvalidCir {
        errors: Vec<String>,
        gcc: i32,
    },
    /// **The engine declared a modelling limit.**
    ///
    /// **Currently unexercised, and that is recorded rather than hidden.** Since
    /// `refuse_floating` stops every float program at lowering, nothing in this grammar
    /// reaches the engine and degrades — a mutation deleting this arm survives. It is kept
    /// because the contract is 023 §7's and the next thing to degrade (a budget, an
    /// unmodeled extern, an engine `lowering_gap`) needs it, and because deleting it would
    /// mean rediscovering the distinction the next time a gap appears. Unlike wave 143's
    /// dead guard, this is unreachable *from today's grammar*, not in principle. 023 §7 makes `Fidelity` the contract
    /// that separates "chiero cannot model this" from "chiero is wrong": a run that hits a
    /// gap degrades below `Exact` and says so. Not a defect — but ledgered and ratcheted
    /// with the refusals, because a limit nobody looks at is indistinguishable from a bug
    /// nobody found.
    Gap {
        fidelity: String,
    },
    /// The program was undefined, or the compilers disagreed with each other about it.
    /// Not compared, and not a defect on either side.
    Discarded,
}

/// Run one generated body through both sides.
fn judge(prelude: &str, body: &str) -> Verdict {
    let Some((gcc, defined)) = gcc_answer(prelude, body) else {
        return Verdict::Discarded;
    };
    if !defined {
        return Verdict::Discarded;
    }
    match chiero_answer(prelude, body) {
        Ok(Ok((Some(v), _))) if v == gcc => Verdict::Agree,
        // **A degraded run is a declared limit, whatever it answered.** 023 §7 forbids a
        // claim of exactness once a gap has been reached, so a wrong number from an
        // `Unknown` run is chiero saying "I could not model this" — which is the honest
        // outcome, not a defect. A wrong number at `Exact` is the opposite, and stays a
        // `Mismatch`: that is the case the fidelity contract exists to make impossible.
        Ok(Ok((_, f))) if f != "Exact" => Verdict::Gap { fidelity: f },
        Ok(Ok((Some(v), _))) => Verdict::Mismatch {
            chiero: Some(v),
            gcc,
        },
        Ok(Ok((None, _))) => Verdict::SilentNoState { gcc },
        Ok(Err(errors)) => Verdict::InvalidCir { errors, gcc },
        Err(ChieroErr::Refused { stage, message }) => Verdict::Refused { stage, message },
        Err(ChieroErr::Panic(m)) => Verdict::Panic(m),
    }
}

enum ChieroErr {
    Refused {
        stage: &'static str,
        message: String,
    },
    Panic(String),
}

/// Lower and run, distinguishing a refusal from a silent nothing.
///
/// `harness::lower` panics on any diagnostic, which is right for a hand-written fixture and
/// wrong here — a refusal is information, not a test failure. So the stages are run
/// directly and each one's diagnostics are reported as themselves.
#[allow(clippy::type_complexity)]
fn chiero_answer(
    prelude: &str,
    body: &str,
) -> Result<Result<(Option<i32>, String), Vec<String>>, ChieroErr> {
    use chiero_parse::{ScopedTypedefs, parse_tu};
    use chiero_pp::{Config, preprocess_str};
    use chiero_sema::{SymbolText, TargetConfig, analyze};

    struct Names<'a>(&'a chiero_parse::ParsedTu);
    impl SymbolText for Names<'_> {
        fn text(&self, s: chiero_span::Symbol) -> Option<&str> {
            self.0.text(s)
        }
    }

    let src = format!("{prelude}\nint probe(void) {{\n{body}}}\n");
    let out = std::panic::catch_unwind(|| {
        let tu = preprocess_str("g.c", &src, Config::default());
        if let Some(d) = tu.diagnostics.first() {
            return Err(("preprocess", format!("{d:?}")));
        }
        let mut oracle = ScopedTypedefs::new();
        let parsed = parse_tu(&tu, &mut oracle);
        if let Some(d) = parsed.diagnostics.first() {
            return Err(("parse", d.message.clone()));
        }
        let names = Names(&parsed);
        let an = analyze(&parsed.ast, &TargetConfig::x86_64_linux(), &names);
        if let Some(d) = an.diagnostics.first() {
            return Err(("sema", format!("{d:?}")));
        }
        let m = chiero_lower::lower_tu(&parsed.ast, &an, &names);
        if let Some(d) = m.diagnostics.first() {
            return Err(("lower", d.message.clone()));
        }
        // **Verify what lowering emitted.** Nothing else does — 015 §7 refuses only what
        // lowering itself detected — so invalid CIR reaches the engine, which then produces
        // no state for a reason that has nothing to do with the program.
        let bad: Vec<String> = chiero_cir::verify::verify(&m.module)
            .iter()
            .filter(|e| e.is_error())
            .map(|e| format!("{e:?}"))
            .collect();
        if !bad.is_empty() {
            return Ok(Err(bad));
        }
        let mut arena = chiero_solver::TermArena::new();
        let r = chiero_exec::Engine::new(&m.module)
            .with_entry("probe")
            .run(&mut arena);
        let fidelity = format!("{:?}", r.fidelity());
        Ok(Ok((
            r.states()
                .iter()
                .find_map(|s| s.return_value_bits(&mut arena))
                .map(|b| b as u32 as i32),
            fidelity,
        )))
    });
    match out {
        Ok(Ok(v)) => Ok(v),
        Ok(Err((stage, message))) => Err(ChieroErr::Refused { stage, message }),
        Err(p) => Err(ChieroErr::Panic(panic_text(&p))),
    }
}

fn panic_text(p: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic".into()
    }
}

/// gcc's answer, and whether the program is defined.
///
/// `defined` is false when the sanitizers trip, when the binary does not run, or when
/// `-O0`, `-O2` and clang do not all agree — three compilers disagreeing about a program
/// means the program is the problem, not any of them.
fn gcc_answer(prelude: &str, body: &str) -> Option<(i32, bool)> {
    let dir = std::env::temp_dir().join(format!("chiero-gen-{}-{}", std::process::id(), next()));
    std::fs::create_dir_all(&dir).ok()?;
    let c = dir.join("g.c");
    let src = format!(
        "#include <stdio.h>\n{prelude}\nint probe(void) {{\n{body}}}\n\
         int main(void) {{ printf(\"%d\\n\", probe()); return 0; }}\n"
    );
    std::fs::write(&c, src).ok()?;

    let run = |cc: &str, extra: &[&str], tag: &str| -> Option<(i32, bool)> {
        let bin = dir.join(tag);
        let mut cmd = std::process::Command::new(cc);
        cmd.args(["-std=gnu11", "-w"]).args(extra).arg("-o");
        cmd.arg(&bin).arg(&c);
        let out = cmd.output().ok()?;
        if !out.status.success() {
            return None;
        }
        let r = std::process::Command::new(&bin).output().ok()?;
        if !r.status.success() {
            return Some((0, false));
        }
        let text = String::from_utf8_lossy(&r.stdout);
        let err = String::from_utf8_lossy(&r.stderr);
        if !err.is_empty() {
            // A sanitizer said something. Whatever it was, the program is not defined.
            return Some((0, false));
        }
        Some((text.trim().parse::<i32>().ok()?, true))
    };

    // The verdict, and the sanitizer check on the same compiler.
    let base = run("gcc", &["-O0"], "o0")?;
    let result = if !base.1 {
        Some((0, false))
    } else {
        let san = run(
            "gcc",
            &[
                "-O0",
                // **`float-cast-overflow` is named explicitly.** gcc's `undefined` group
                // does not include it, which is measurable rather than a matter of opinion:
                // `(unsigned short)(-4294905087.0)` runs clean and prints 0 under
                // `undefined,address`, and reports a runtime error the moment this is
                // added. C11 6.3.1.4 makes the conversion undefined when the integral part
                // is not representable.
                //
                // The gap was unreachable while lowering refused every float, and waves
                // 167–170 made it reachable. Seed 1832 then reported a mismatch that was
                // two implementations of undefined behaviour disagreeing — a false defect,
                // which costs this channel more than a missed one.
                "-fsanitize=undefined,address,float-cast-overflow",
                "-fno-sanitize-recover=all",
            ],
            "san",
        );
        match san {
            Some((_, false)) | None => Some((0, false)),
            Some(_) => {
                // Two more readings; disagreement means the program is undefined in a way
                // the sanitizers did not catch.
                let o2 = run("gcc", &["-O2"], "o2");
                let cl = run("clang", &["-O0"], "cl");
                let agree = o2.map(|x| x.1 && x.0 == base.0).unwrap_or(true)
                    && cl.map(|x| x.1 && x.0 == base.0).unwrap_or(true);
                if agree { Some(base) } else { Some((0, false)) }
            }
        }
    };
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn next() -> u64 {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

// ---------------------------------------------------------------------------------------
// Shrinking
// ---------------------------------------------------------------------------------------

/// Reduce a failing program while it keeps failing the same way.
///
/// **Line-based, not AST-based, and the distinction is deliberate.** A true AST reducer
/// needs the generator to hand back a tree; it builds strings, one statement per line, and
/// over *that* shape line-deletion is already a valid reduction operator. Deleting a
/// declaration breaks every later reference — and the pipeline discards the result, because
/// gcc refuses to compile it — so a broken deletion costs one compile and is rejected
/// automatically. That is the whole reason this is affordable without a tree.
///
/// The interestingness predicate is the *verdict class*, not the verdict: a mismatch that
/// shrinks from `chiero 20, gcc 12` to `chiero 2, gcc 1` is the same defect, and demanding
/// the exact numbers would stop the reduction almost immediately.
///
/// Runs to a fixpoint over single-line deletions, then over the prelude's declarations.
/// Quadratic in lines and each step is a compile, which is why the batch shrinks only what
/// it is about to report rather than everything it finds.
fn shrink(prelude: &str, body: &str, interesting: &dyn Fn(&str, &str) -> bool) -> (String, String) {
    let mut prelude = prelude.to_string();
    let mut body = body.to_string();
    if !interesting(&prelude, &body) {
        // Nothing to do, and shrinking a program whose failure we cannot reproduce would
        // "reduce" it to something unrelated.
        return (prelude, body);
    }
    let mut changed = true;
    while changed {
        changed = false;
        // Body lines, last first: a later statement is more likely to be the one that can
        // go, and removing it does not invalidate the declarations above it.
        let mut i = body.lines().count();
        while i > 0 {
            i -= 1;
            // **A `return` is never deleted.** The first version of this reducer removed
            // one, leaving a value-returning function that falls off its end — undefined
            // behaviour that the `-fsanitize=undefined,address` filter does not catch,
            // because gcc's return check is not part of it for C. The reduction still
            // "failed", so it was kept, and the report was a program whose failure meant
            // nothing. Cheaper to protect the line than to widen the UB filter.
            if body
                .lines()
                .nth(i)
                .is_some_and(|l| l.trim_start().starts_with("return"))
            {
                continue;
            }
            let kept: String = body
                .lines()
                .enumerate()
                .filter(|(n, _)| *n != i)
                .map(|(_, l)| format!("{l}\n"))
                .collect();
            if interesting(&prelude, &kept) {
                body = kept;
                changed = true;
            }
        }
        // Then the prelude, whole declarations at a time. A `struct` or a helper spans
        // several lines, so this deletes from one `}` to the next rather than by line.
        let decls = split_prelude(&prelude);
        for d in 0..decls.len() {
            let kept: String = decls
                .iter()
                .enumerate()
                .filter(|(n, _)| *n != d)
                .map(|(_, t)| t.clone())
                .collect();
            if interesting(&kept, &body) {
                prelude = kept;
                changed = true;
                break;
            }
        }
    }
    (prelude, body)
}

/// The prelude as whole declarations — a `struct`/`union` body or a function body is one
/// unit, so a brace-depth counter is enough and no parsing is needed.
fn split_prelude(prelude: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    for line in prelude.lines() {
        cur.push_str(line);
        cur.push('\n');
        depth += line.matches('{').count() as i32;
        depth -= line.matches('}').count() as i32;
        if depth <= 0 && !cur.trim().is_empty() {
            out.push(std::mem::take(&mut cur));
            depth = 0;
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// Whether two verdicts are the same *kind* of failure.
fn same_class(a: &Verdict, b: &Verdict) -> bool {
    matches!(
        (a, b),
        (Verdict::Mismatch { .. }, Verdict::Mismatch { .. })
            | (Verdict::SilentNoState { .. }, Verdict::SilentNoState { .. })
            | (Verdict::Panic(_), Verdict::Panic(_))
            | (Verdict::InvalidCir { .. }, Verdict::InvalidCir { .. })
    )
}

// ---------------------------------------------------------------------------------------
// The batch
// ---------------------------------------------------------------------------------------

/// **The gaps this suite knows about**, each with the reason it is tolerated.
///
/// A refusal or a declared limit is not a defect — it is chiero saying so out loud, which
/// is what 015 §7 and 023 §7 ask for. But a ledger nobody has to look at becomes a
/// suppression file within a month: the first unexplained entry is noticed, the tenth is
/// scrolled past. So the list is *closed*. A refusal whose text matches nothing here fails
/// the run, and closing it means either fixing the gap or writing down why it stays.
///
/// Matched by substring against the diagnostic, because the messages carry spans and
/// operand types that vary per program while the *reason* does not.
const KNOWN_GAPS: &[(&str, &str)] = &[
    (
        "compares floating values or converts one to `_Bool`",
        "**this entry replaced `uses floating point` in wave 168**, which is what the old \
         one predicted would happen: floats now lower and run, and what is left is the two \
         operations the engine has no arms for. A float *comparison* would produce no \
         value, and `(_Bool)f` is worse than missing — C11 6.3.1.2 makes it \
         \"compares unequal to 0\", so truncating with `FpToSi` answers 0 for 0.5, which is \
         a wrong answer rather than an absent one. Refusing is 015 §7's rule; the fix is \
         float arms in the engine's `cmp`, and this entry is what will fail when they land.",
    ),
    (
        "Unknown",
        "the engine reached a modelling limit and degraded, which 023 §7 requires it to \
         announce. A degraded run is chiero saying it could not model something, which is \
         the honest outcome and not a defect.",
    ),
    (
        "Bounded",
        "a budget was hit. 042 §3 makes `Bounded` the realistic default, not a failure.",
    ),
    (
        "Approximated",
        "an operation chiero models approximately, declared as such.",
    ),
];

/// Whether a ledger entry is one the suite has been told about.
fn is_known_gap(text: &str) -> bool {
    KNOWN_GAPS.iter().any(|(pat, _)| text.contains(pat))
}

/// **Fixed seeds, so this is a test and not a slot machine.**
///
/// An unseeded random test that fails one run in ten gets muted within a month, and a muted
/// channel is worse than none. The open-ended search belongs in a soak run; what lives in
/// CI is a fixed batch, and anything it finds graduates to a named fixture in
/// `differential.rs` where it is deterministic forever.
#[test]
fn generated_programs_agree_with_gcc() {
    let mut refused: Vec<(&str, String)> = Vec::new();
    let mut discarded = 0usize;
    let mut compared = 0usize;
    let mut defects: Vec<(u64, String, Verdict)> = Vec::new();

    for seed in 0..200u64 {
        let (prelude, body) = program(seed);
        match judge(&prelude, &body) {
            Verdict::Agree => compared += 1,
            Verdict::Discarded => discarded += 1,
            Verdict::Refused { stage, message } => refused.push((stage, message)),
            Verdict::Gap { fidelity } => refused.push(("engine", fidelity)),
            v => {
                // Shrunk before it is recorded, so the report is something a human can read
                // and paste into `differential.rs` rather than a 40-line program to bisect
                // by hand. Only the *reported* failure is shrunk — each step is a compile.
                let want = &v;
                let (p2, b2) = if defects.is_empty() {
                    shrink(&prelude, &body, &|p: &str, b: &str| {
                        same_class(&judge(p, b), want)
                    })
                } else {
                    (prelude.clone(), body.clone())
                };
                // **Re-judged after shrinking, not carried over.** A reduction keeps the
                // failure's *class*, not its numbers — the reduced program computes
                // something simpler — so reporting the original verdict beside the reduced
                // source prints a `gcc:` value that source cannot produce. The first
                // version did exactly that, and the numbers were the part a reader would
                // have trusted most.
                let shown = judge(&p2, &b2);
                let shown = if same_class(&shown, &v) { shown } else { v };
                defects.push((seed, format!("{p2}\nint probe(void) {{\n{b2}}}"), shown));
            }
        }
    }

    // The ledger, printed whether or not anything failed — a refusal is a gap and gaps are
    // meant to be looked at, not accumulated quietly.
    if !refused.is_empty() {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for (stage, m) in &refused {
            let key = format!("{stage}: {m}");
            match counts.iter_mut().find(|(k, _)| *k == key) {
                Some((_, n)) => *n += 1,
                None => counts.push((key, 1)),
            }
        }
        counts.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        eprintln!("refusal ledger ({} programs):", refused.len());
        for (k, n) in counts.iter().take(10) {
            eprintln!("  {n:>4}  {k}");
        }
    }
    eprintln!(
        "compared {compared}, discarded {discarded}, refused {}",
        refused.len()
    );

    // **A floor, not `> 0`.** `> 0` is green on a single lucky program, and mutation said
    // so: wave 177 gave `Gen` an out-of-bounds knob, and a mutant that switched it on for
    // *this* channel survived the whole suite. Those programs are undefined, so `judge`
    // discards them — the channel would have gone on passing while comparing a handful.
    //
    // 100 against the 130 observed. The margin is for the grammar drifting, not for a
    // change that guts the corpus: anything that halves what this compares should be a
    // decision somebody made rather than a number nobody looked at.
    assert!(
        compared >= 100,
        "the generator compared only {compared} of 200 programs — a channel that discards \
         almost everything is green while testing almost nothing"
    );

    // **The ratchet.** Every ledger entry must be one the suite was told about; an entry
    // matching nothing is a gap that appeared without a decision being made, which is
    // exactly the moment to make one.
    let unknown: Vec<&(&str, String)> = refused.iter().filter(|(_, m)| !is_known_gap(m)).collect();
    assert!(
        unknown.is_empty(),
        "{} refusal(s) the suite has no entry for. Each is a gap that appeared without a \
         decision: either fix it, or add it to KNOWN_GAPS with the reason it stays.\n{}",
        unknown.len(),
        unknown
            .iter()
            .map(|(s, m)| format!("  {s}: {m}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        defects.is_empty(),
        "{} generated program(s) disagree with gcc. First:\nseed {}\n{}\n{:#?}",
        defects.len(),
        defects[0].0,
        defects[0].1,
        defects[0].2
    );
}

/// **The shrinker keeps the failure and loses the rest.**
///
/// Tested against a *synthetic* predicate rather than a real defect, deliberately: a test
/// that needs chiero to be broken cannot live in a green suite, and the property being
/// checked — "reduce while the predicate holds" — is independent of what the predicate is.
/// The predicate here is "the body still mentions `MARK`", which is exactly the shape a
/// verdict-class check has: cheap, deterministic, and true of the original.
#[test]
fn the_shrinker_reduces_while_the_failure_survives() {
    let prelude = "struct Keep { int a; };\nstruct Drop { int b; };\n\
                   static int unused(int x) { return x; }\n";
    let body = "  int a = 1;\n  int b = 2;\n  int MARK = 3;\n  int c = 4;\n                   int d = 5;\n  return MARK;\n";
    let interesting = |_p: &str, b: &str| b.contains("MARK") && b.contains("return");
    let (p2, b2) = shrink(prelude, body, &interesting);

    assert!(
        interesting(&p2, &b2),
        "the reduction must still fail the way the original did: {p2}\n{b2}"
    );
    assert!(
        b2.lines().count() < body.lines().count(),
        "and it must actually be smaller: {} lines from {}",
        b2.lines().count(),
        body.lines().count()
    );
    // Every line the predicate does not need is gone — this is the property that makes a
    // report readable rather than merely shorter.
    for gone in ["int a = 1", "int b = 2", "int c = 4", "int d = 5"] {
        assert!(!b2.contains(gone), "`{gone}` survived: {b2}");
    }
    // And the prelude reduces by whole declarations, not by line — a `struct` cut in half
    // would not compile and could never be interesting.
    assert!(
        !p2.contains("struct Drop") && !p2.contains("unused"),
        "unneeded declarations survived: {p2}"
    );
    assert_eq!(
        p2.matches('{').count(),
        p2.matches('}').count(),
        "and the prelude stayed brace-balanced — half a struct would not compile and so \
         could never be interesting: {p2}"
    );
}

/// **A `return` is never shrunk away.**
///
/// Found by using the reducer on a live defect: it deleted the trailing `return`, leaving a
/// value-returning function that falls off its end. That is undefined behaviour, the
/// sanitizer filter does not catch it — gcc's return check is not part of
/// `-fsanitize=undefined` for C — and the reduced program still "failed", so it was kept.
/// The report was then a program whose failure meant nothing.
#[test]
fn the_shrinker_keeps_the_return() {
    let body = "  int a = 1;\n  int b = 2;\n  return 0;\n";
    // A predicate that would happily accept the empty program, so only the guard can
    // preserve the `return`.
    let (_, b2) = shrink("", body, &|_, _| true);
    assert!(
        b2.contains("return"),
        "the reduction dropped the return and is undefined C: {b2:?}"
    );
}

/// **Shrinking a program whose failure cannot be reproduced changes nothing.**
///
/// The guard that stops the reducer "reducing" an unrelated program to noise: if the
/// predicate is false at the start, there is nothing to preserve and the input comes back
/// untouched.
#[test]
fn the_shrinker_refuses_to_reduce_what_does_not_fail() {
    let prelude = "struct S { int a; };\n";
    let body = "  int a = 1;\n  int b = 2;\n  return a;\n";

    // A predicate that is *false for the original and true for a reduction* — which is the
    // only shape that can tell the guard apart from its absence. An always-false predicate
    // cannot: without the guard the reducer would keep nothing anyway, so both readings
    // return the input and the mutation is equivalent. That was the first version of this
    // test, and it survived deleting the very guard it was written for.
    let interesting = |_p: &str, b: &str| b.lines().count() == 2;
    let (p2, b2) = shrink(prelude, body, &interesting);
    assert_eq!(
        b2, body,
        "a program whose failure cannot be reproduced must come back untouched, not be \
         'reduced' into an unrelated one that happens to satisfy the predicate"
    );
    assert_eq!(p2, prelude);
}

/// **Open-ended search**, which CI's fixed batch deliberately is not.
///
/// `generated_programs_agree_with_gcc` runs seeds 0..200 every time, because a test that
/// picks new seeds each run fails one time in ten and gets muted within a month. That makes
/// it a regression test: it re-checks what has already been looked at.
///
/// Finding *new* defects needs new seeds, and that is this — `#[ignore]`d so it never runs
/// in CI, with the range under `SOAK_LO`/`SOAK_HI` so a session can push the frontier and
/// record where it got to:
///
/// ```text
/// SOAK_LO=200 SOAK_HI=800 cargo test -p chiero-lower --test generated zz_soak -- --ignored --nocapture
/// ```
///
/// **It prints a census, not just a verdict.** Seeds 200..800 came back with no defects at
/// all — and the interesting number was not the zero. Of 600 programs, 81 were compared,
/// 226 were discarded as undefined, and **293 were refused for floating point**: the
/// channel spends half its budget generating programs chiero declines to lower, and
/// two-thirds of the rest on programs that teach nothing because gcc's answer for UB is not
/// an oracle. A soak that only reported "0 defects" would have hidden the one fact worth
/// acting on.
#[test]
#[ignore = "open-ended search; CI runs the fixed batch instead"]
fn zz_soak() {
    let lo: u64 = std::env::var("SOAK_LO")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    let hi: u64 = std::env::var("SOAK_HI")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(800);
    let (mut compared, mut discarded) = (0usize, 0usize);
    let mut refused: std::collections::BTreeMap<String, usize> = Default::default();
    let mut defects: Vec<(u64, String, String)> = Vec::new();
    for seed in lo..hi {
        let (prelude, body) = program(seed);
        match judge(&prelude, &body) {
            Verdict::Agree => compared += 1,
            Verdict::Discarded => discarded += 1,
            Verdict::Refused { stage, message } => {
                *refused
                    .entry(format!(
                        "{stage}: {}",
                        message.chars().take(60).collect::<String>()
                    ))
                    .or_insert(0) += 1;
            }
            Verdict::Gap { fidelity } => {
                *refused.entry(format!("gap: {fidelity}")).or_insert(0) += 1;
            }
            v => {
                defects.push((
                    seed,
                    format!("{prelude}\nint probe(void) {{\n{body}}}"),
                    format!("{v:?}"),
                ));
            }
        }
    }
    eprintln!(
        "SOAK {lo}..{hi}: compared={compared} discarded={discarded} defects={}",
        defects.len()
    );
    for (k, n) in &refused {
        eprintln!("  refused {n:4}  {k}");
    }
    for (s, p, v) in defects.iter().take(3) {
        eprintln!("  DEFECT seed={s} {v}\n{p}");
    }
}

/// **A float-to-integer overflow is undefined, and the filter must discard it.**
///
/// C11 6.3.1.4: converting a floating value to an integer type is undefined "if the value of
/// the integral part cannot be represented". `(unsigned short)(-4294905087.0)` is exactly
/// that, and gcc answers 0 while chiero answers 62209 — two implementations of undefined
/// behaviour disagreeing, which teaches nothing.
///
/// The filter compiles every fixture under `-fsanitize=undefined,address` and discards what
/// trips. **gcc's `-fsanitize=undefined` does not include `float-cast-overflow`** — it is a
/// separate sub-sanitiser that has to be named. Measured rather than assumed:
///
/// ```text
///   -fsanitize=undefined,address                      -> prints 0, exit 0
///   -fsanitize=undefined,address,float-cast-overflow  -> runtime error, exit 1
/// ```
///
/// So the hole was always there and was unreachable while lowering refused every float.
/// Waves 167–170 made it reachable, and seed 1832 became a **false defect** — the most
/// corrosive thing this channel can produce, because it spends the attention the channel
/// exists to focus and is how a generative test gets muted.
///
/// The second case is the constraint: a fixture whose conversion is *in* range must still
/// be compared. A filter that discarded every float-to-integer conversion would satisfy the
/// first assertion and quietly delete the coverage waves 167–170 added.
#[test]
fn the_ub_filter_discards_a_float_cast_overflow() {
    let out_of_range = "double d = -4294905087.0; return (int)(unsigned short)d;";
    assert!(
        matches!(judge("", out_of_range), Verdict::Discarded),
        "converting {} to `unsigned short` is undefined, so the program teaches nothing and \
         must not be compared: {:?}",
        -4294905087.0f64,
        judge("", out_of_range)
    );

    let in_range = "double d = 300.0; return (int)(unsigned short)d;";
    assert!(
        matches!(judge("", in_range), Verdict::Agree),
        "300.0 is representable as `unsigned short`, so this is an ordinary comparison and \
         discarding it would delete real coverage: {:?}",
        judge("", in_range)
    );
}

#[test]
#[ignore]
fn zz_census() {
    use std::collections::BTreeMap;
    let dir =
        std::path::PathBuf::from(std::env::var("TMPDIR").unwrap_or("/tmp".into())).join("census");
    std::fs::create_dir_all(&dir).unwrap();
    let mut tab: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    let (mut n, mut skipped) = (0u32, 0u32);
    let mut false_examples = 0u32;
    for seed in 0..300u64 {
        let (prelude, body) = program(seed);
        let src = format!("{prelude}\nint probe(void) {{\n{body}}}\n");
        let csrc = format!(
            "{src}\n#include <stdio.h>\nint main(void){{ printf(\"%d\\n\", probe()); return 0; }}\n"
        );
        let c = dir.join(format!("s{seed}.c"));
        let x = dir.join(format!("s{seed}"));
        std::fs::write(&c, &csrc).unwrap();
        let comp = std::process::Command::new("gcc")
            .args([
                "-O0",
                "-fsanitize=undefined,address,float-cast-overflow",
                "-o",
            ])
            .arg(&x)
            .arg(&c)
            .output()
            .unwrap();
        if !comp.status.success() {
            skipped += 1;
            continue;
        }
        let out = std::process::Command::new(&x).output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr).to_string();
        let m = harness::lower_maybe(&src);
        let Some(m) = m else {
            skipped += 1;
            continue;
        };
        let mut arena = chiero_solver::TermArena::new();
        let r = chiero_exec::Engine::new(&m)
            .with_entry("probe")
            .run(&mut arena);
        let kinds: Vec<String> = r
            .states()
            .iter()
            .flat_map(|s| s.ub_events())
            .map(|u| format!("{:?}", u.kind))
            .collect();
        n += 1;
        // **The reverse direction, which the table below cannot see.** It counts only rows
        // where gcc said something, so a chiero report on a program gcc runs clean is
        // invisible to it — and by wave 171's rule that is the expensive kind of wrong.
        if err.is_empty() && !kinds.is_empty() {
            tab.entry("ZZ FALSE POSITIVE (gcc silent)".into())
                .or_default()
                .0 += 1;
            if false_examples < 3 {
                false_examples += 1;
                println!("\n--- chiero reports {kinds:?}, gcc silent, seed {seed} ---\n{src}");
            }
        }
        for pat in [
            "signed integer overflow",
            "left shift of negative",
            // **`places cannot be represented`, not `cannot be represented`.** The shorter
            // substring also matches gcc's *signed overflow* message — "signed integer
            // overflow: X * 31 cannot be represented in type 'long int'" — so this row was
            // counting row 1's programs a second time and grading them against `Shift`,
            // which is not the kind they produce. It read 7/22 for three waves and the
            // gap was the measurement.
            "places cannot be represented",
            "shift exponent",
            "outside the range of representable values",
        ] {
            if err.contains(pat) {
                let want = match pat {
                    "signed integer overflow" => "SignedOverflow",
                    "outside the range of representable values" => "FloatCastOverflow",
                    _ => "Shift",
                };
                let e = tab.entry(pat.into()).or_default();
                e.0 += 1;
                if kinds.iter().any(|k| k == want) {
                    e.1 += 1;
                }
            }
        }
    }
    println!("\n=== census over 300 seeds ({n} compared, {skipped} skipped) ===");
    println!("seen / chiero");
    for (k, (seen, got)) in &tab {
        println!("{seen:4} / {got:<4}  {k}");
    }
}

// ---------------------------------------------------------------------------------------
// The memory-UB oracle
// ---------------------------------------------------------------------------------------
//
// **Nothing grades chiero's memory-UB detection, and memory UB is what it is for.**
//
// `-fsanitize=address` has been in this file's compile line since wave 139 and has never
// once fired. Not because chiero is bad at out-of-bounds accesses — it detects them, with a
// finding naming the object, the offset and the size — but because the grammar cannot
// produce one. `Gen::arrays` carries each array's length precisely so every index is in
// range by construction, and that field's own comment says why: an out-of-bounds read is
// UB, so `judge` would discard the program rather than compare it, and a discarded program
// teaches the value-differential nothing.
//
// That reasoning is right for the *differential* channel and wrong as a global policy.
// Wave 175 built a second channel — `zz_census` — whose whole subject is programs gcc calls
// undefined, and there a discarded program is the only interesting kind. Every UB class the
// census can see today is arithmetic, because arithmetic is all this grammar knows how to
// get wrong.
//
// The gap that leaves is not small. Out-of-bounds access and use-after-free are the defects
// chiero exists to find in VPP; 023 §9's witness requirement, 021's pointer machinery and
// the whole of `chiero-mem` are built for them, and none of it is graded against an oracle.

/// What AddressSanitizer says about a program, and what chiero says.
#[derive(Debug, Default)]
struct Tally {
    /// Programs ASan flagged.
    flagged: usize,
    /// Of those, ones chiero also reported a memory finding for.
    caught: usize,
    /// Seeds ASan flagged and chiero did not, kept for the failure message.
    missed: Vec<u64>,
    /// Seeds chiero reported a memory finding for and ASan did not flag.
    ///
    /// The direction the census had to learn twice (waves 175 and 176): a channel that
    /// counts only what the oracle flags cannot see a false report, and by wave 171's rule
    /// that is the expensive kind of wrong.
    invented: Vec<u64>,
    /// Programs that reached the comparison at all.
    compared: usize,
}

/// Does this finding say the program touched memory it does not own?
///
/// Matched on the finding's rendered text rather than on a kind, because out-of-bounds is
/// reported through the checker framework's `Finding` and not as a `UbKind` — the two
/// mechanisms are 023 §6's report and 020 §4.1's event, and only arithmetic uses the
/// latter.
fn is_memory_finding(s: &str) -> bool {
    s.contains("out-of-bounds") || s.contains("use-after") || s.contains("null-dereference")
}

fn tally(seeds: std::ops::Range<u64>) -> Tally {
    let dir = std::path::PathBuf::from(std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into()))
        .join("chiero-memub");
    let _ = std::fs::create_dir_all(&dir);
    let mut t = Tally::default();
    for seed in seeds {
        let (prelude, body) = program_memory_ub(seed);
        let src = format!("{prelude}\nint probe(void) {{\n{body}}}\n");
        let csrc = format!(
            "{src}\n#include <stdio.h>\nint main(void){{ printf(\"%d\\n\", probe()); return 0; }}\n"
        );
        let c = dir.join(format!("s{seed}.c"));
        let x = dir.join(format!("s{seed}"));
        if std::fs::write(&c, &csrc).is_err() {
            continue;
        }
        let compiled = std::process::Command::new("gcc")
            .args(["-O0", "-fsanitize=address,undefined", "-o"])
            .arg(&x)
            .arg(&c)
            .output();
        match compiled {
            Ok(o) if o.status.success() => {}
            // No gcc, or a program it will not build: not this test's subject.
            _ => continue,
        }
        let Ok(run) = std::process::Command::new(&x).output() else {
            continue;
        };
        let err = String::from_utf8_lossy(&run.stderr).to_string();
        let Some(m) = harness::lower_maybe(&src) else {
            continue;
        };
        t.compared += 1;
        // **`AddressSanitizer` and not the UBSan text.** The two sanitizers are both on and
        // report differently; only ASan's classes are this file's subject, and matching on
        // "runtime error" would count arithmetic the census already grades.
        let mut arena = chiero_solver::TermArena::new();
        let r = chiero_exec::Engine::new(&m)
            .with_entry("probe")
            .run(&mut arena);
        let found = r
            .findings()
            .iter()
            .any(|f| is_memory_finding(&format!("{f:?}")));
        if !err.contains("AddressSanitizer") {
            // **ASan reports the first fault and stops.** A program it did not flag is one
            // with no *reached* memory fault, so a chiero finding on it is either a false
            // report or a fault on a path the single concrete run did not take — and these
            // programs are closed, with one path. Either way it is worth seeing.
            if found {
                t.invented.push(seed);
            }
            continue;
        }
        t.flagged += 1;
        if found {
            t.caught += 1;
        } else {
            t.missed.push(seed);
        }
    }
    t
}

/// The corpus must contain memory UB, and chiero must find all of it.
///
/// Both halves in one test on purpose. Split, the first would read as a statement about the
/// generator and get "fixed" by lowering its threshold; together they say the only thing
/// worth asserting, which is that a real oracle graded a real corpus and chiero passed.
#[test]
fn the_corpus_commits_memory_ub_and_chiero_reports_all_of_it() {
    let t = tally(0..60);
    // Printed, not just asserted: a channel that reports only pass/fail hides whether it
    // graded 40 programs or one, and wave 166's rule is that a soak reporting only its
    // verdict hides its census.
    println!(
        "memory-UB oracle: {} compared, {} flagged by ASan, {} caught, {} invented",
        t.compared,
        t.flagged,
        t.caught,
        t.invented.len()
    );
    assert!(
        t.compared > 0,
        "no generated program reached the comparison; gcc missing or the corpus is empty"
    );
    assert!(
        t.flagged > 0,
        "AddressSanitizer flagged none of {} programs, so nothing here grades chiero's \
         memory-UB detection — the grammar indexes every array in range by construction",
        t.compared
    );
    // **A floor, not just "more than zero".** The corpus is generated, so a change that
    // quietly stopped emitting out-of-bounds indices would leave this green on one lucky
    // program. Five is well under the fifteen observed and well clear of noise.
    assert!(
        t.flagged >= 5,
        "only {} of {} programs commit memory UB; too thin to grade anything",
        t.flagged,
        t.compared
    );
    assert!(
        t.invented.is_empty(),
        "chiero reported a memory fault on {} programs ASan runs clean: seeds {:?}",
        t.invented.len(),
        &t.invented[..t.invented.len().min(8)]
    );
    assert!(
        t.missed.is_empty(),
        "ASan flagged {} of {} programs and chiero missed {} of them: seeds {:?}",
        t.flagged,
        t.compared,
        t.missed.len(),
        &t.missed[..t.missed.len().min(8)]
    );
}
