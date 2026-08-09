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
    /// **Allow the program to commit memory UB.** Off for every channel that compares *values*,
    /// because an out-of-bounds access is undefined and `judge` discards the program rather
    /// than grading it. On for the memory-UB oracle, whose subject is exactly the programs
    /// the others throw away.
    ///
    /// A knob rather than a second grammar: the shapes worth indexing out of are the ones
    /// already here — local arrays, file-scope arrays, a global pointer aimed at an array,
    /// read and written through all of `access`'s spellings — and forking the grammar to
    /// get them would mean maintaining two of everything and grading the copy.
    memory_ub: bool,
    /// Emit the constructs the wave-218 and wave-220 censuses found unemitted: `switch`,
    /// `do`-`while`, `&&`, `||`, `goto`, `continue`, and the wider expression leaves. Off by
    /// default so every channel that measured a census against the old grammar still sees it byte
    /// for byte.
    ///
    /// One knob rather than one per census: the point of the gate is that *nothing* new perturbs
    /// the old streams, and a second flag would double the combinations without covering a shape
    /// the union does not.
    extended: bool,
    /// **A second stream, for the vector arm only.**
    ///
    /// Every other extended arm draws from `rng`, so adding one more re-rolls every statement
    /// after it — and it did: wave 270's `!`-of-a-negative-zero shape dropped from three
    /// programs to *zero*, caught by that channel's own adequacy guard. Wave 217 gated the
    /// control-flow arms before any `rng` call to keep the other channels byte-for-byte
    /// identical; within one channel the same protection needs a separate stream.
    ///
    /// Seeded from the same seed through a different constant, so it is deterministic, tied to
    /// the program, and cannot disturb a draw anything else depends on.
    vrng: Rng,
    /// A third stream, for wave 284's arms.
    ///
    /// **One stream per independent feature, so adding one cannot perturb another.** Wave 277
    /// learned the first half of this the hard way: a new arm drawing from `rng` re-rolled every
    /// statement after it and silently dropped an existing channel's coverage to zero. Sharing
    /// `vrng` would repeat it one level down — the vector arm's distribution would shift every
    /// time an unrelated arm was added or changed. Streams are cheap; the guarantee is not.
    grng: Rng,
    /// How many of each wave-284 arm this program has emitted, so neither can crowd out the
    /// statement budget the other channels are graded on.
    typeofs: u32,
    typeof_arrays: u32,
    /// Names the wave-284 arms contribute to the checksum, kept **out of `vars`**.
    ///
    /// **A separate stream is not enough; state that gates an `rng` call also shifts it.**
    /// `leaf` reads `if usable.is_empty() || self.rng.chance(3)`, so pushing one more variable
    /// makes the left operand false and the `chance(3)` draw *happen* where `||` had
    /// short-circuited it. Every downstream draw moves, and the channel's adequacy guards caught
    /// it twice: `compared` fell to 103 and wave 270's `!`-of-a-negative-zero to two.
    ///
    /// So these names never enter the grammar's view. They are folded into the checksum at
    /// `finish`, after every draw has been made.
    extra_sink: Vec<String>,
    /// The one over-aligned local this program declared, and what it asked for.
    ///
    /// **Both halves or neither.** `_Alignas(32)` changes no value on its own; `_Alignof` of the
    /// object is the only observable. Remembering the pair is what lets the expression wrapper
    /// ask about *this* object rather than about a type.
    aligned_local: Option<(String, u64)>,
    /// How deep inside a loop or `switch` body the generator currently is.
    ///
    /// The `if` arm has recursed through `nested` since it was written, and bounding *loop* bodies
    /// matters more: three nested loops of four iterations each is sixty-four passes, and a
    /// comparison that takes a second is a comparison nobody runs.
    nest: u32,
    /// **Allow a zero divisor.** Off for every channel that compares *values*, for the same
    /// reason `memory_ub` is: `x / 0` is undefined, so `judge` discards the program and the
    /// value differential learns nothing from it.
    ///
    /// Separate from `memory_ub` rather than folded into it, because the two serve different
    /// oracles. The memory corpus wants faults ASan can see and pays for them with a program
    /// that dies at the first one; the arithmetic corpus wants every site gcc's UBSan can
    /// see and needs the program to *keep running* to reach the later ones.
    div_zero: bool,
    /// Pointers holding the address of a local that has left its scope, as (name, type).
    ///
    /// Populated only under [`Gen::memory_ub`]. Kept separate from `vars` because the
    /// pointer is in scope and perfectly usable — it is the *object* that is gone, which is
    /// the whole distinction `use-after-scope` is about and the one a
    /// pointer-typed variable list would lose.
    escaped: Vec<(String, Ty)>,
    /// Heap blocks the program has allocated, as (name, element type, length, freed).
    ///
    /// The `freed` flag is what makes a *use*-after-free reachable rather than accidental:
    /// the grammar has to know a block is gone in order to deliberately touch it again.
    heaps: Vec<(String, Ty, usize, bool)>,
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
            memory_ub: false,
            extended: false,
            vrng: Rng::new(seed ^ 0x5657_4543_544F_5200),
            grng: Rng::new(seed ^ 0x4752_4944_3238_3400),
            typeofs: 0,
            typeof_arrays: 0,
            extra_sink: Vec::new(),
            aligned_local: None,
            nest: 0,
            div_zero: false,
            heaps: Vec::new(),
            escaped: Vec::new(),
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
        if self.memory_ub && self.rng.chance(3) {
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
            // **A bit-field on an odd-indexed `int` member becomes `unsigned`** (wave 252).
            //
            // A *signed* bit-field cannot expose an extension defect — sign-extending it is what C
            // asks for — so a third of bit-field coverage was going to a case every other integer
            // member already covers. Which type a bit-field lands on falls out of the field list,
            // and the field list is drawn long before the widths are, so this flips the type after
            // the fact rather than biasing the draw.
            //
            // **Odd indices only**, so signed bit-fields keep happening: they are the control for
            // the fix wave 249 made, and a generator that stopped emitting them would let a
            // "never sign-extend" regression through. No `rng` call is added, so every other
            // channel's stream is untouched — wave 250 learned what happens otherwise.
            let mut fields = fields;
            for (i, w) in widths.iter().enumerate() {
                if w.is_some() && i % 2 == 1 && fields[i] == Ty::Int {
                    fields[i] = Ty::UInt;
                }
            }
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
        // **Declared, not `#include <stdlib.h>`.** The fixtures preprocess with a default
        // `Config`, which has no system include path, so an include would fail before the
        // program ever reached lowering — measured, as one diagnostic per program.
        if self.memory_ub {
            out.push_str("void *malloc(unsigned long);\nvoid free(void *);\n");
        }
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
                        // **A bit-field's constant spans its own width; every other member keeps
                        // `1 + i`** (wave 250). An extension defect is invisible unless the stored
                        // value has the field's top bit set, and an ascending small constant sets
                        // it in five of six hundred programs.
                        //
                        // Indexed into the pool rather than drawn from `rng`, and that is the
                        // load-bearing part: consuming randomness here shifts the stream and every
                        // downstream decision with it, so the other eight channels would generate
                        // different programs and their corpora would silently move. The first
                        // attempt did exactly that — the metric got *worse*, because the programs
                        // it was measuring were no longer the same programs. §9's rule about gating
                        // before any `rng` call is the same rule seen from the other side.
                        let mut e = match self.records[r].widths[i] {
                            Some(w) => {
                                let k = POOL[(i * 7 + w as usize) % POOL.len()] as u128;
                                let mask = (1u128 << w) - 1;
                                // **The top bit is set on two fields in three, deliberately.**
                                // Masking a pool value alone was not enough: the pool's low entries
                                // dominate for small indices, and the count barely moved. This is
                                // the half of every field's range where sign- and zero-extension
                                // disagree, so a channel that does not reach it cannot see an
                                // extension defect at all. The remaining third keeps the top bit
                                // clear and is the control — a generator that only ever set it
                                // would stop testing the easy case it has always passed.
                                // Split on the pool entry's own parity, which varies across the
                                // pool where `i` and `w` barely do — ten of twenty-one entries are
                                // odd. `(i + w) % 3` was the first try and put *every* unsigned
                                // bit-field in the top half, because the handful of `(i, w)` pairs
                                // that actually occur all landed on one side.
                                let top = if k.is_multiple_of(2) {
                                    0
                                } else {
                                    1u128 << (w - 1)
                                };
                                format!("{}", (k & mask) | top)
                            }
                            None => format!("{}", 1 + i as i64),
                        };
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
            // **`below(6)`, and the sixth is a negative zero.** One draw either way, so the
            // stream position is unchanged and every earlier census stays comparable.
            //
            // A negative zero is not a *precision* value and so does not fall under the rule
            // above: it is exact in every format, every compiler agrees on it, and no program
            // gets discarded for it. What it does is separate "compares equal to zero" from
            // "has zero bits", which is the only float value that can. Wave 270 found `!`
            // testing the bits and answering 0 for `!(-0.0)` where C says 1, and no corpus
            // built from `0.0 .. 4.0` could ever have seen it.
            let v = self.rng.below(6);
            if v == 5 {
                return if ty == Ty::F32 { "-0.0f" } else { "-0.0" }.to_string();
            }
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
        // **The wave-220 census's expression forms**, gated before any `rng` call so the old
        // grammar's stream is untouched. `~` and `sizeof` are here rather than in `leaf` because
        // both are *conversion* shapes: `~` promotes its operand and `sizeof` yields `size_t`,
        // whose unsignedness wins the usual arithmetic conversions and turns `- 5` into a very
        // large number. That class is where wave 217's defect lived.
        // **Wave 285's forms, on their own stream and before the wave-220 gate.**
        //
        // Each of these is a *value*, not an object: it replaces a subexpression and adds no
        // statement, no storage and no operand to the checksum. Wave 284 measured what an
        // object costs — a multi-dimensional array took this channel from 131 comparisons to
        // 80 — and this is the shape that avoids it.
        //
        let wave_220 = self.extended && self.rng.chance(8);
        if wave_220 {
            self.depth += 1;
            let inner = self.expr(want);
            self.depth -= 1;
            return match self.rng.below(7) {
                // Complement, which promotes a narrow operand to `int` before inverting.
                0 => format!("({})(~({}){inner})", want.c(), want.c()),
                // `sizeof` in arithmetic: unsigned, and wider than `int` on this target.
                1 => format!("({})(sizeof(long) + ({}){inner})", want.c(), want.c()),
                // A character constant, whose type is `int` in C however narrow the value.
                2 => format!("({})('A' + (({}){inner} & 3))", want.c(), want.c()),
                // A string literal indexed at a constant: an array of static storage duration,
                // read through a subscript, with no pointer value reaching the checksum.
                3 => format!("({})(\"abcd\"[(({}){inner} & 3)])", want.c(), want.c()),
                // **`!`, whose absence was the wave-270 census's finding.** A truth test is not
                // the operand's bit pattern, and `!` had never once been through the oracle: it
                // compared bits, so `!(-0.0)` answered 0 where C says 1.
                //
                // **Unconditional, which the first version of this arm was not.** It emitted
                // `x && !x`, and `&&` short-circuits: when `x` is zero the `!x` is never
                // evaluated — and zero is the *only* value that discriminates here. The shape
                // was built so that the one case it existed to reach could not be reached. A
                // surviving mutant is what said so; nothing about re-reading it did.
                //
                // **A float operand is a bare constant, not the nested subexpression.** The
                // site alone is worth nothing: 2000 seeds put `!` on a float 41 times and the
                // mutant survived every one, because `inner` is a sum of small constants and
                // such a sum lands on a *negative* zero essentially never. Generating the
                // shape is not generating the value that makes the shape discriminate — the
                // wave-250 lesson, arriving this time one level down. Straight from `konst`,
                // where `-0.0` is one draw in six, it lands often enough to kill the mutant.
                4 if want.is_float() => {
                    let k = self.konst(want);
                    format!("({})(!({k}))", want.c())
                }
                4 => format!("({})(!(({}){inner}))", want.c(), want.c()),
                // The operand on the **right** of a short circuit, which took a different path
                // through lowering than the left: a float or a pointer there produced CIR the
                // verifier rejects and the function was dropped whole.
                5 => format!("({})(1 && (({}){inner}))", want.c(), want.c()),
                // `_Alignof` in arithmetic, which is `sizeof`'s class: `size_t`, unsigned, and
                // wider than `int`, so its unsignedness wins the usual arithmetic conversions.
                _ => format!("({})(_Alignof(double) + ({}){inner})", want.c(), want.c()),
            };
        }
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
                // **One in five, and integers only.** A float division by zero is not
                // undefined — C99 Annex F gives it an infinity — so emitting one would add
                // a site gcc never reports and chiero correctly stays quiet about.
                let d = if self.div_zero && !want.is_float() && self.rng.chance(5) {
                    0
                } else {
                    1 + self.rng.below(15)
                };
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
        // **Wave 285's forms wrap the expression that was just built.**
        //
        // Every earlier shape drew from `rng` — either by gating on it, or by calling `expr`
        // for a fresh operand — and *any* extra or skipped draw shifts every draw after it in
        // that program. Wave 270's `!`-of-a-negative-zero shape sat at three programs in six
        // hundred and went to zero, then one, whatever the rate; it is not a rate problem, it
        // is that a displaced stream scrambles which programs contain a rare shape at all.
        //
        // Wrapping `e` after the fact costs **no `rng` draw**: the gate and the choice come from
        // `grng`, and the operand is the one the ordinary path already produced. The stream is
        // byte-identical to before this wave, which the channel's adequacy guards check.
        // **A higher rate when this program has an over-aligned local**, and the exact number is
        // a measured trade rather than a preference. The alignment form is the only wrapper whose
        // discrimination depends on a *declaration* elsewhere in the program, so it has to fire
        // often enough to land in a program that is also compared.
        //
        // At **4** it kills wave 282's mutants and the channel falls to 98 comparisons; at **7**
        // the channel holds its floor of 100 and the mutants survive. The floor is a wave-270
        // guarantee, so 7 it is — the pairing is emitted and graded against gcc, and the
        // discrimination is one rate-step away and out of reach. That step is what §9 records:
        // this channel's comparison budget is spent, and more coverage needs a channel with its
        // own budget rather than a larger share of this one.
        let rate = if self.aligned_local.is_some() { 7 } else { 14 };
        let e = if self.extended && self.grng.chance(rate) {
            self.recent_form(want, e)
        } else {
            e
        };
        format!("({e})")
    }

    /// `_Generic`, `__builtin_offsetof` and a floating classification builtin, as expressions.
    ///
    /// **All three yield a value of a type the surrounding expression already wanted**, so each
    /// wraps `inner` and hands back something the grammar can keep using. That is why they are
    /// affordable where a multi-dimensional array was not.
    ///
    ///   - `_Generic` (wave 275) selects on the *controlling expression's type*, so the arm
    ///     names the type it is generating for and a wrong selection changes the value. The
    ///     `default` arm is written **first**, since a `default` that shadows a later exact
    ///     match is precisely what wave 275's mutation found.
    ///   - `__builtin_offsetof` (wave 280) needs a record, so it only fires when one is in
    ///     scope, and it asks for a member other than the first where it can — the first is at
    ///     offset 0 whatever the layout does.
    ///
    /// **They *select*, they do not combine.** The first version added each builtin's value to
    /// `inner`, and that addition is itself undefined once a narrow signed type is near its
    /// range: discards went from 69 programs to 100 and the channel fell to exactly its floor.
    /// A conditional keeps both operands in play and adds no arithmetic the program did not
    /// already have.
    ///   - A classification builtin (wave 271) needs a floating operand, so the value is cast to
    ///     `double` first; `isnan` of an ordinary number is 0, and `islessequal` against itself
    ///     is 1, so both answers appear.
    fn recent_form(&mut self, want: Ty, inner: String) -> String {
        // **The alignment selector, when this program has an over-aligned local.** It reads back
        // what the declaration asked for: if the specifier was dropped, `_Alignof` answers the
        // type's natural alignment, the branch flips and the value changes. A comparison rather
        // than arithmetic, per wave 285 — nothing here can overflow.
        if let Some((name, n)) = self.aligned_local.clone()
            && self.grng.chance(2)
        {
            return format!(
                "({})(_Alignof({name}) == {n} ? ({}){inner} : ({})0)",
                want.c(),
                want.c(),
                want.c()
            );
        }
        let pick = self.grng.below(3);
        // A record with at least two members, so `offsetof` can name one that is not at 0.
        let rec = (0..self.records.len()).find(|&r| self.records[r].readable() >= 2);
        match (pick, rec) {
            (1, Some(r)) => {
                let kw = self.records[r].kw();
                let tag = self.records[r].tag.clone();
                let f = self.records[r].field(1);
                format!(
                    "({})(__builtin_offsetof({kw} {tag}, {f}) > 0 ? ({}){inner} : ({})0)",
                    want.c(),
                    want.c(),
                    want.c()
                )
            }
            (2, _) => {
                // **Only the ones wave 271 implemented.** `isinf` and `isfinite` are declared
                // limits that refuse loudly, and a corpus emitting them would grade the refusal
                // rather than the feature.
                let b = *self.grng.pick(&[
                    "__builtin_isnan",
                    "__builtin_isunordered",
                    "__builtin_islessequal",
                ]);
                let d = format!("(double)({inner})");
                let call = if b == "__builtin_isnan" {
                    format!("{b}({d})")
                } else {
                    format!("{b}({d}, {d})")
                };
                format!(
                    "({})({call} ? ({}){inner} : ({})0)",
                    want.c(),
                    want.c(),
                    want.c()
                )
            }
            _ => format!(
                "({})(_Generic(({}){inner}, default: 0, {}: {inner}))",
                want.c(),
                want.c(),
                want.c()
            ),
        }
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
    /// A local array with a braced initializer, which is also how wave 140's
    /// element-conversion path gets exercised for every element type.
    fn push_local_array(&mut self) {
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

    /// **A `typeof` declaration**: a second object whose type is copied from one in scope.
    ///
    /// Landed in wave 283 and in 37 VPP files. The copy is initialised from the original, so a
    /// `typeof` that resolved to the wrong type produces a conversion the checksum can see.
    fn typeof_stmt(&mut self) -> bool {
        // The three spellings, since they are three tokens in the lexer and one production.
        let kw = *self.grng.pick(&["__typeof__", "typeof", "__typeof"]);
        // **`sizeof` of a `typeof` of an *array*, with its own budget and tried first.**
        //
        // A copied scalar cannot see whether `typeof` decayed its operand — the whole question
        // is arrays and function designators, and `__typeof__(a)` for `int a[4]` must be
        // `int[4]` and not `int *`. This reads the size the type reports: 16 undecayed, 8
        // decayed.
        //
        // Sharing one counter with the scalar form meant it never fired at all. Arrays are
        // pushed by the ordinary dispatch *during* statement generation, so the first two
        // `typeof`s are spent before any array exists: measured 0 in 300 programs while 165 of
        // them had an array. The cap was the reason, not the shape.
        if !self.arrays.is_empty() && self.typeof_arrays == 0 && self.grng.chance(2) {
            self.typeof_arrays += 1;
            let (arr, _, _) = self.arrays[self.grng.below(self.arrays.len())].clone();
            let name = self.fresh();
            let _ = writeln!(
                self.body,
                "  unsigned {name} = (unsigned)sizeof({kw}({arr}));"
            );
            self.extra_sink.push(name);
            return true;
        }
        if self.vars.is_empty() || self.typeofs >= 2 || !self.grng.chance(4) {
            return false;
        }
        self.typeofs += 1;
        let src = self.vars[self.grng.below(self.vars.len())].clone();
        let name = self.fresh();
        let _ = writeln!(self.body, "  {kw}({}) {name} = {};", src.name, src.name);
        self.extra_sink.push(name);
        true
    }

    /// **A vector: declared, initialized, operated on elementwise, and read into the checksum.**
    ///
    /// Waves 272–274 built `vector_size` out and left the corpus emitting none, so all of it was
    /// graded by hand-written fixtures. Every shape those fixtures found load-bearing is here:
    ///
    ///   - A **braced initializer**, which wave 272 found was dropped entirely — and note the
    ///     lanes come from `konst`, so `-0.0` and the adversarial integer pool reach them.
    ///   - A **lane read**, which wave 272 found was typed `Ty::Error` and read as `Int(32)`.
    ///     The read is what reaches the checksum, so every operator here is graded through it.
    ///   - **Narrow and 64-bit lanes.** An `int` lane cannot see either defect: `Int(32)` is
    ///     accidentally right for it. `unsigned char` and `long` lanes are the discriminating
    ///     ones, so they are two of the four element types.
    ///   - **Elementwise arithmetic and a comparison**, waves 273 and 274.
    ///
    /// **Comparisons only on integer lanes.** A float vector's comparison yields a *signed
    /// integer* vector of the lane's width, which is not the operand type — declaring the result
    /// with the operand's typedef would be wrong C rather than a test of anything. That shape is
    /// covered by hand fixtures, which can name the mask type directly.
    ///
    /// **No `/` or `%`.** A zero lane is UB for integers, and the initializer pool contains zero;
    /// the arithmetic-UB channel is where division belongs and it has its own zero knob.
    /// `konst` drawn from the vector stream.
    ///
    /// **The pool is the point, so borrow the stream rather than duplicate it.** `konst` carries
    /// the adversarial integers and the `-0.0` wave 270 added, which is exactly what a lane
    /// wants — and it draws from `rng`, which would put the disturbance straight back. Swapping
    /// the two streams around the call keeps one pool and one guarantee.
    /// `konst` drawn from the wave-284 stream, for the same reason `vkonst` exists: the pool
    /// carries the adversarial integers and wave 270's `-0.0`, and borrowing the stream keeps one
    /// pool rather than duplicating it.
    fn gkonst(&mut self, ty: Ty) -> String {
        std::mem::swap(&mut self.rng, &mut self.grng);
        let k = self.konst(ty);
        std::mem::swap(&mut self.rng, &mut self.grng);
        k
    }

    /// The whole body of a focused program: one construct, and what it needs.
    ///
    /// **Every draw is `grng`.** This channel shares no stream with the others, so its content
    /// can change without moving a single draw in `program` or `program_control_flow` — which is
    /// the failure mode waves 284, 285 and 286 each hit in turn.
    fn focused_body(&mut self) {
        match self.grng.below(3) {
            0 => self.focused_md_array(),
            1 => self.focused_alignment(),
            _ => self.focused_designator(),
        }
    }

    /// **A record with an anonymous member, a designator chain into it, and `typeof` of a type.**
    ///
    /// Three shapes that mutation showed the corpus could not reach, in one program because they
    /// share a record:
    ///
    ///   - `__builtin_offsetof(struct S, n.q)` walks into a *named* member and then through an
    ///     *anonymous* one, which is `offsetof_step`'s chained arm resolving through
    ///     `find_field` — the arm that survived wave 280's sweep and wave 285's.
    ///   - `s.q` names a member of an anonymous struct, which is wave 279's field lookup.
    ///   - `__typeof__(int)` is `TypeKind::TypeofType`, a different arm of `ty_of` from the
    ///     expression form the shared channel emits, and wave 284's surviving mutant.
    ///
    /// The record is written out rather than drawn from `self.records`, because what is being
    /// tested is a *shape* — a named member wrapping an anonymous one — and the existing record
    /// generator has no notion of either.
    fn focused_designator(&mut self) {
        let ty = *self.grng.pick(&[Ty::Int, Ty::Long, Ty::UShort, Ty::UInt]);
        let tag = format!("S{}", {
            self.next_id += 1;
            self.next_id
        });
        // `lead` puts the anonymous member at a nonzero offset: rebasing its fields onto the
        // container adds nothing when it is first, which is how wave 279's two mutants survived
        // fifteen fixtures.
        let lead = self.grng.chance(2);
        let mut def = format!("struct {tag} {{ ");
        if lead {
            let _ = write!(def, "{} head; ", ty.c());
        }
        let _ = write!(
            def,
            "struct {{ {} p; struct {{ {} q; }}; }} n; {} tail; }};",
            ty.c(),
            ty.c(),
            ty.c()
        );
        // **Defined in the body, not the prelude.** A struct definition is a block item like
        // any other, and the prelude has already been rendered by the time this runs — plumbing
        // an extra string through would buy nothing a local definition does not.
        let _ = writeln!(self.body, "  {def}");
        let obj = self.fresh();
        let _ = writeln!(self.body, "  struct {tag} {obj};");
        let vq = self.gkonst(ty);
        let vp = self.gkonst(ty);
        let vt = self.gkonst(ty);
        let _ = writeln!(self.body, "  {obj}.n.q = {vq};");
        let _ = writeln!(self.body, "  {obj}.n.p = {vp};");
        let _ = writeln!(self.body, "  {obj}.tail = {vt};");
        if lead {
            let vh = self.gkonst(ty);
            let _ = writeln!(self.body, "  {obj}.head = {vh};");
            self.extra_sink.push(format!("{obj}.head"));
        }
        // The offsets, which are what a chained designator has to get right rather than merely
        // find. `n.q` is the chain through the anonymous member; `tail` is past all of it.
        let o1 = self.fresh();
        let _ = writeln!(
            self.body,
            "  unsigned long {o1} = (unsigned long)__builtin_offsetof(struct {tag}, n.q);"
        );
        let o2 = self.fresh();
        let _ = writeln!(
            self.body,
            "  unsigned long {o2} = (unsigned long)__builtin_offsetof(struct {tag}, tail);"
        );
        // `typeof` of a *type name*, which is a different arm of `ty_of` from `typeof(expr)`.
        let tv = self.fresh();
        let _ = writeln!(
            self.body,
            "  __typeof__({}) {tv} = ({}){vt};",
            ty.c(),
            ty.c()
        );
        self.extra_sink.push(o1);
        self.extra_sink.push(o2);
        self.extra_sink.push(tv);
        self.extra_sink.push(format!("{obj}.n.q"));
        self.extra_sink.push(format!("{obj}.n.p"));
        self.extra_sink.push(format!("{obj}.tail"));
    }

    /// A non-square multi-dimensional array, initialised, written, and read element by element.
    ///
    /// **Non-square, always.** `int a[2][3]` typed as `int a[3][2]` has the same total size, and
    /// `a[0][0]` and `a[1][0]` read the same element under both layouts — a square array is its
    /// own reverse and proves nothing. Wave 278 found the reversal only because a hand-written
    /// fixture asked for `sizeof(a[0])`, which is why that is here too.
    ///
    /// Braced row-by-row and flat by turns, so the corpus carries both walks: C11 6.7.9p20's
    /// brace elision was the second defect wave 278 found, underneath the first.
    fn focused_md_array(&mut self) {
        let ty = *self
            .grng
            .pick(&[Ty::Int, Ty::UInt, Ty::Long, Ty::SChar, Ty::UShort]);
        let rows = 2 + self.grng.below(2);
        let cols = if rows == 2 { 3 } else { 2 };
        let name = self.fresh();
        let flat = self.grng.chance(2);
        let mut vals: Vec<String> = Vec::new();
        for _ in 0..rows {
            let row: Vec<String> = (0..cols).map(|_| self.gkonst(ty)).collect();
            vals.push(if flat {
                row.join(", ")
            } else {
                format!("{{{}}}", row.join(", "))
            });
        }
        let _ = writeln!(
            self.body,
            "  {} {name}[{rows}][{cols}] = {{{}}};",
            ty.c(),
            vals.join(", ")
        );
        // A write at a row and column that are not both zero: `a[0][0]` is at offset 0 whichever
        // way the extents go.
        let wr = self.gkonst(ty);
        let _ = writeln!(self.body, "  {name}[{}][{}] = {wr};", rows - 1, cols - 1);
        // **The row's size, which is what says the extents are the right way round.**
        // `sizeof(a)` is the same under a reversal; `sizeof(a[0])` is not.
        let sz = self.fresh();
        let _ = writeln!(
            self.body,
            "  unsigned long {sz} = (unsigned long)sizeof({name}[0]);"
        );
        self.extra_sink.push(sz);
        for r in 0..rows {
            for c in 0..cols {
                self.extra_sink.push(format!("{name}[{r}][{c}]"));
            }
        }
    }

    /// An over-aligned local, and an `_Alignof` that names it.
    ///
    /// **Both halves or neither**: the specifier changes no value, and `_Alignof` of a *type*
    /// asks nothing about a declaration. Here the rate can be whatever discrimination needs,
    /// because nothing else is competing for the program's budget.
    fn focused_alignment(&mut self) {
        let ty = *self.grng.pick(&Ty::ALL);
        let ty = if ty == Ty::Bool { Ty::Int } else { ty };
        let n = *self.grng.pick(&[16u64, 32, 64]);
        let name = self.fresh();
        // **Sometimes an array, because that is a different code path.** An alignment specifier
        // sits in the declaration *specifiers*, so for `_Alignas(32) int a[4]` the attribute is
        // on the `int` node while the declaration's type is the array wrapper the declarator
        // built around it — wave 282 had to walk down to find it, and a scalar local cannot tell
        // whether that walk happens.
        let arr = self.grng.chance(2);
        if arr {
            let len = 2 + self.grng.below(3);
            let vals: Vec<String> = (0..len).map(|_| self.gkonst(ty)).collect();
            let _ = writeln!(
                self.body,
                "  _Alignas({n}) {} {name}[{len}] = {{{}}};",
                ty.c(),
                vals.join(", ")
            );
            for i in 0..len {
                self.extra_sink.push(format!("{name}[{i}]"));
            }
        } else {
            let init = self.gkonst(ty);
            let _ = writeln!(self.body, "  _Alignas({n}) {} {name} = {init};", ty.c());
        }
        // The alignment the declaration asked for, and the object's address modulo it — the
        // second is what wave 282 found was wrong even when the first was right.
        let a = self.fresh();
        let _ = writeln!(
            self.body,
            "  unsigned long {a} = (unsigned long)_Alignof({name});"
        );
        let m = self.fresh();
        let _ = writeln!(
            self.body,
            "  unsigned long {m} = (unsigned long)((unsigned long)&{name} & {}ul);",
            n - 1
        );
        self.extra_sink.push(a);
        self.extra_sink.push(m);
        if !arr {
            self.extra_sink.push(name);
        }
    }

    fn vkonst(&mut self, ty: Ty) -> String {
        std::mem::swap(&mut self.rng, &mut self.vrng);
        let k = self.konst(ty);
        std::mem::swap(&mut self.rng, &mut self.vrng);
        k
    }

    fn vector_stmt(&mut self) -> bool {
        if !self.vrng.chance(3) {
            return false;
        }
        // Element type and lane count, sized so the vector is 8 or 16 bytes.
        let (ety, lanes) = *self.vrng.pick(&[
            (Ty::Int, 4usize),
            (Ty::UChar, 8),
            (Ty::Long, 2),
            (Ty::F32, 4),
        ]);
        let bytes = (ety.bits() as usize / 8) * lanes;
        let vty = format!("{} __attribute__((vector_size({bytes})))", ety.c());
        let a = self.fresh();
        let b = self.fresh();
        let r = self.fresh();
        let out = self.fresh();
        let init = |g: &mut Self| {
            (0..lanes)
                .map(|_| g.vkonst(ety))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let ia = init(self);
        let _ = writeln!(self.body, "  {vty} {a} = {{{ia}}};");
        let ib = init(self);
        let _ = writeln!(self.body, "  {vty} {b} = {{{ib}}};");
        let float = ety == Ty::F32;
        // A comparison, a scalar broadcast, or vector-on-vector arithmetic.
        let rhs = match self.vrng.below(4) {
            0 if !float => format!("({a} == {b})"),
            1 if !float => format!("({a} < {b})"),
            2 => {
                let op = if float {
                    *self.vrng.pick(&["+", "-", "*"])
                } else {
                    *self.vrng.pick(&["+", "-", "*", "&", "|", "^"])
                };
                let k = self.vkonst(ety);
                format!("({a} {op} {k})")
            }
            _ => {
                let op = if float {
                    *self.vrng.pick(&["+", "-", "*"])
                } else {
                    *self.vrng.pick(&["+", "-", "*", "&", "|", "^"])
                };
                format!("({a} {op} {b})")
            }
        };
        let _ = writeln!(self.body, "  {vty} {r} = {rhs};");
        let lane = self.vrng.below(lanes);
        let _ = writeln!(self.body, "  {} {out} = {r}[{lane}];", ety.c());
        // **Into `vars`, so the lane reaches the checksum.** A vector the program computes and
        // never reads is a statement the oracle cannot grade — wave 253's rule about the
        // short-circuit witness, in a new place.
        self.vars.push(Var { name: out, ty: ety });
        true
    }

    /// Make sure the program has a **local** array to walk off the end of.
    ///
    /// Every program gets file-scope arrays from `prelude`, and a local one only from the
    /// ordinary dispatch — `below(10) == 0` and then a coin, so about one statement in
    /// twenty. Most programs therefore had no local array at all, which is why
    /// `stack-buffer-overflow` sat at 1 of 41 while `global-buffer-overflow` had seven,
    /// even though the same index knob feeds both. The two reach chiero through different
    /// `chiero-mem` object kinds, so the starved row was the one testing the *less* covered
    /// path.
    ///
    /// **Only when there is none yet.** Emitting these freely would repeat wave 179's
    /// mistake in the other direction: ASan halts at the first fault, so a shape that
    /// becomes common starves every other class. One array per program makes the local case
    /// reachable without making it dominant.
    fn local_array_stmt(&mut self) -> bool {
        if self.arrays.is_empty() && self.rng.chance(2) {
            self.push_local_array();
            return true;
        }
        false
    }

    /// Let a block-scoped local's address escape the block, and read it afterwards.
    ///
    /// **The read is a separate statement, not part of the block.** Emitting both together
    /// would put the read inside the braces, where the object is still alive and the
    /// program is defined — the fault is entirely in the *ordering*, so the two halves have
    /// to be separate `stmt` calls with the closing brace between them.
    ///
    /// The pointer is declared before the block rather than inside it, or it would leave
    /// scope alongside the object it points at and the read would not compile.
    fn scope_stmt(&mut self) -> bool {
        // Read through a pointer whose object is gone. Guarded on the pointer existing, so
        // this can only run after a previous statement opened and closed the block.
        if !self.escaped.is_empty() && self.rng.chance(3) {
            let k = self.rng.below(self.escaped.len());
            let (name, ty) = self.escaped[k].clone();
            let v = self.fresh();
            let _ = writeln!(self.body, "  {} {v} = *{name};", ty.c());
            self.vars.push(Var { name: v, ty });
            // Removed once used: ASan halts at the first fault, so a second read of the
            // same dead object is a statement the program never reaches, and leaving it
            // available would crowd out the other shapes.
            self.escaped.remove(k);
            return true;
        }
        // **One in twelve, and the rate is load-bearing.** ASan halts at the first fault,
        // so whichever class fires earliest is the only one a program ever reports. At one
        // in four this shape took 26 of 39 programs and drove `stack-buffer-overflow` out
        // of the corpus entirely — the table showed it, the total did not.
        if self.escaped.len() < 2 && self.rng.chance(12) {
            let ty = *self.rng.pick(&Ty::ALL);
            let ty = if ty == Ty::Bool { Ty::Int } else { ty };
            let p = self.fresh();
            let a = self.fresh();
            let init = self.konst(ty);
            let _ = writeln!(self.body, "  {} *{p};", ty.c());
            let _ = writeln!(self.body, "  {{ {} {a} = {init}; {p} = &{a}; }}", ty.c());
            self.escaped.push((p, ty));
            return true;
        }
        false
    }

    /// Allocate, use, free, and — deliberately — use or free again.
    ///
    /// Returns whether it emitted anything, so `stmt` can fall through to the ordinary
    /// grammar the rest of the time.
    ///
    /// **Every block is freed exactly once on the ordinary path.** A block that is never
    /// freed is a *leak*, which LeakSanitizer reports at exit as an ASan failure — and a
    /// leak is not undefined behaviour, so chiero says nothing about it and the oracle
    /// would score every allocating program as a miss. The oracle also runs with
    /// `detect_leaks=0` for the same reason; both are needed, because the flag stops the
    /// report and freeing is what makes the *use*-after-free reachable.
    fn heap_stmt(&mut self) -> bool {
        // Allocate. `sizeof` is spelled as a literal size so the grammar needs no new
        // expression form.
        if self.heaps.len() < 2 && self.rng.chance(3) {
            let ty = *self.rng.pick(&Ty::ALL);
            let ty = if ty == Ty::Bool { Ty::Int } else { ty };
            let len = 1 + self.rng.below(3);
            let name = self.fresh();
            let bytes = len * (ty.bits() as usize).div_ceil(8);
            let _ = writeln!(
                self.body,
                "  {} *{name} = ({} *)malloc({bytes});",
                ty.c(),
                ty.c()
            );
            // **Written before it is read.** malloc leaves the block uninitialized, and
            // reading it is undefined in a way ASan does not report and chiero models as
            // `Undef` — a third verdict that would make every allocating program a
            // disagreement about something neither tool is being asked about here.
            for i in 0..len {
                let v = self.konst(ty);
                let _ = writeln!(self.body, "  {name}[{i}] = {v};");
            }
            self.heaps.push((name, ty, len, false));
            return true;
        }
        if self.heaps.is_empty() {
            return false;
        }
        let k = self.rng.below(self.heaps.len());
        let (name, ty, len, freed) = self.heaps[k].clone();
        if !freed {
            // Read it, or free it.
            if self.rng.chance(2) {
                // `index` and not `below`: the same knob that walks off the end of an
                // array walks off the end of a block, which is the difference between
                // ASan's `global-buffer-overflow` and its `heap-buffer-overflow` — two
                // classes chiero must handle through different `chiero-mem` object kinds.
                let i = self.index(len);
                let a = self.access(&name, i);
                let v = self.fresh();
                let _ = writeln!(self.body, "  {} {v} = {a};", ty.c());
                self.vars.push(Var { name: v, ty });
                return true;
            }
            let _ = writeln!(self.body, "  free({name});");
            self.heaps[k].3 = true;
            return true;
        }
        // Freed. One in three programs that get here commits the fault; the rest leave the
        // block alone, so a corpus is not all faults and the shapes after the first one
        // still get generated — ASan halts at the first report.
        match self.rng.below(3) {
            0 => {
                let i = self.rng.below(len);
                let a = self.access(&name, i);
                let v = self.fresh();
                let _ = writeln!(self.body, "  {} {v} = {a};", ty.c());
                self.vars.push(Var { name: v, ty });
                true
            }
            1 => {
                let _ = writeln!(self.body, "  free({name});");
                true
            }
            _ => false,
        }
    }

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
        // **The heap arms come first and only under the knob**, so the value-comparing
        // channels see a grammar byte-for-byte identical to the one they have always seen.
        if self.memory_ub && self.local_array_stmt() {
            return;
        }
        if self.memory_ub && self.scope_stmt() {
            return;
        }
        if self.memory_ub && self.heap_stmt() {
            return;
        }
        // **Two statement forms lowering has always supported and this generator never
        // emitted**, found by asking what `StmtKind` can hold rather than what the grammar
        // happens to say: `Switch` and `DoWhile`.
        //
        // **Behind a knob, checked before any `rng` call**, exactly as the heap arms are. That is
        // not tidiness: `chance` consumes randomness, so an ungated call reshuffles every seed's
        // program, and the first version of this shifted the memory-UB corpus until
        // `stack-buffer-overflow` appeared in two programs instead of enough to grade on. The
        // adequacy guard caught it, which is what that guard is for.
        if self.extended && self.vector_stmt() {
            return;
        }
        // **These two emit and fall through; they do not consume the statement slot.**
        //
        // Returning here made them ordinary arms and they diluted everything else: the channel's
        // statement budget is fixed, so a slot spent on a declaration is a slot not spent on a
        // `switch` or a `continue`. Wave 270's `!`-of-a-negative-zero fell to one program and
        // then `for`-`continue` to nine, each caught by its own adequacy guard.
        //
        // A separate *stream* stops a new arm perturbing another's draws (wave 277); it does not
        // stop it taking another's slot. Emitting a declaration and then letting the ordinary
        // dispatch run keeps every existing count exactly where it was, and the program is a
        // little longer instead.
        if self.extended {
            self.typeof_stmt();
        }
        if self.extended && self.rng.chance(3) && self.switch_stmt() {
            return;
        }
        if self.extended && self.rng.chance(3) && self.do_while_stmt() {
            return;
        }
        if self.extended && self.rng.chance(3) && self.short_circuit_stmt() {
            return;
        }
        if self.extended && self.rng.chance(3) && self.goto_stmt() {
            return;
        }
        match self.rng.below(10) {
            0 if self.arrays.len() < 3 && self.rng.chance(2) => self.push_local_array(),
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
                // **One over-aligned local per program, chosen on `grng`.** Attaching the
                // specifier costs no statement and no storage the checksum reads; what makes it
                // worth anything is the wrapper in `recent_form` that asks `_Alignof` about
                // *this* name.
                //
                // 16 and 32 only: C11 6.7.5p3 makes a specifier *weaker* than the type's own
                // alignment a constraint violation, and gcc rejects it. Every type in `Ty::ALL`
                // aligns to at most 8, so both values are always legal.
                let aligned = self.aligned_local.is_none() && self.grng.chance(2);
                if aligned {
                    let n = *self.grng.pick(&[16u64, 32]);
                    let _ = writeln!(self.body, "  _Alignas({n}) {} {name} = {init};", ty.c());
                    self.aligned_local = Some((name.clone(), n));
                } else {
                    let _ = writeln!(self.body, "  {} {name} = {init};", ty.c());
                }
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
                // **Half of struct locals are steered to a record that has a bit-field** (wave
                // 252). A bit-field is only observable through a local in *this* body — that is
                // what reaches the checksum — and picking uniformly gave one discriminating
                // program per two hundred seeds.
                //
                // The draw is reused rather than repeated: `r` is already a random index, so its
                // parity chooses whether to steer and its value chooses which record to steer to.
                // Adding a second `rng` call here would shift the stream for every channel.
                //
                // The other half stays uniform, so records without bit-fields keep appearing as
                // locals — they are most of the aggregate coverage and none of it should move.
                let bf: Vec<usize> = (0..self.records.len())
                    .filter(|i| self.records[*i].widths.iter().any(Option::is_some))
                    .collect();
                let r = if !r.is_multiple_of(4) && !bf.is_empty() {
                    bf[r % bf.len()]
                } else {
                    r
                };
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
                        let widths = self.records[r].widths.clone();
                        // **A bit-field's initializer spans its own width here too** (wave 252).
                        // Wave 250 did this for the prelude's `out.fN = …` path and this one was
                        // missed: `konst` masks to the *declared* type's thirty-two bits and the
                        // store then truncates to three, so the interesting half of the range was
                        // being chosen and thrown away. Two of six compound literals landed in it.
                        //
                        // `konst` is still called for every field, bit-field or not, and its result
                        // discarded where it is overridden. That is deliberate: it consumes the
                        // same randomness either way, so the stream — and every other channel's
                        // corpus — is byte-for-byte what it was.
                        let vals: Vec<String> = fields
                            .iter()
                            .enumerate()
                            .map(|(i, t)| {
                                let k = self.konst(*t);
                                match widths.get(i).copied().flatten() {
                                    Some(w) => {
                                        let pool = POOL[(i * 7 + w as usize) % POOL.len()] as u128;
                                        let mask = (1u128 << w) - 1;
                                        // **Always set, unlike the prelude path which splits.**
                                        // Six compound literals reach a bit-field per two hundred
                                        // seeds, and a deterministic split over six samples is a
                                        // coin flip — the parity rule landed the wrong way and took
                                        // the count from two to one. The prelude path has a large
                                        // enough sample to split and keeps doing so, so the
                                        // top-bit-clear case is still covered; here the sample is
                                        // too small to spend half of on a case another path tests.
                                        format!("{}", (pool & mask) | (1u128 << (w - 1)))
                                    }
                                    None => k,
                                }
                            })
                            .collect();
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
                    // **`continue` under the control-flow knob**, which the census found the
                    // grammar had never emitted. It is the one place a loop's *increment* still
                    // runs while the rest of the body does not — distinct from `break`, and
                    // distinct again in a `do`-`while`, where `continue` jumps to the condition.
                    // **An arbitrary body, which is what makes the arms compose.** Waves 218-220
                    // added `switch`, `do`-`while`, `goto`, `continue` and the wider expression
                    // leaves, and every one of them agreed with gcc on its own; the single defect
                    // those three waves found came from an *interaction* — a `_Bool` accumulator
                    // inside a `do`-`while`. A loop whose body was one compound assignment could
                    // not produce another one, so "a `goto` out of a `switch` inside a loop" was
                    // unreachable by construction rather than by chance.
                    //
                    // The accumulate stays alongside the nested statements, so the loop still
                    // contributes to the checksum whatever the body does.
                    if self.extended && self.nest < 2 && self.rng.chance(2) {
                        let _ = writeln!(self.body, "  for (int {i} = 0; {i} < {n}; {i}++) {{");
                        self.nest += 1;
                        self.nested();
                        self.nest -= 1;
                        let _ = writeln!(self.body, "  {} += {e};", v.name);
                        let _ = writeln!(self.body, "  }}");
                    } else if self.extended && self.rng.chance(2) {
                        let k = self.rng.below(n.max(1));
                        let _ = writeln!(
                            self.body,
                            "  for (int {i} = 0; {i} < {n}; {i}++) {{ if ({i} == {k}) continue; \
                             {} += {e}; }}",
                            v.name
                        );
                    } else {
                        let _ = writeln!(
                            self.body,
                            "  for (int {i} = 0; {i} < {n}; {i}++) {{ {} += {e}; }}",
                            v.name
                        );
                    }
                }
            }
        }
    }

    /// **A `switch` over a live variable**, with fallthrough, a `default`, and empty cases.
    ///
    /// C's `switch` is a multi-way branch whose arms *fall through* unless broken out of, which is
    /// the part no other construct in this grammar has. An empty case that falls into the next one
    /// and a `default` in a position other than last are both ordinary C and both easy to lower
    /// wrongly.
    ///
    /// The controlling expression is `% 4` of a variable so every arm is reachable, and the
    /// generator emits `break` on some arms and not others so fallthrough is exercised rather than
    /// merely available.
    fn switch_stmt(&mut self) -> bool {
        let Some(v) = self.pick_var() else {
            return false;
        };
        if !matches!(
            v.ty,
            Ty::Int | Ty::UInt | Ty::Long | Ty::ULong | Ty::Short | Ty::UShort
        ) {
            return false;
        }
        let target = match self.pick_var() {
            Some(t) => t,
            None => return false,
        };
        let sel = self.fresh();
        let _ = writeln!(self.body, "  int {sel} = (int)({} & 3);", v.name);
        let _ = writeln!(self.body, "  switch ({sel}) {{");
        // `default` first on purpose: its position is free in C and a lowering that assumes it
        // comes last would still pass every fixture that puts it there.
        let d = self.expr(target.ty);
        let _ = writeln!(self.body, "  default: {} += {d}; break;", target.name);
        for c in 0..3u32 {
            let e = self.expr(target.ty);
            // Every third case is empty and falls into the next, which is the shape that makes
            // `switch` different from a chain of `if`s.
            if c == 1 {
                let _ = writeln!(self.body, "  case {c}:");
            } else if self.extended && self.nest < 2 && self.rng.chance(3) {
                // **A `switch` arm with a block in it.** `break` inside a nested `switch` binds to
                // the inner one and a `continue` inside a loop-inside-a-case binds to the loop —
                // the bindings C11 6.8.6.2 and 6.8.6.3 specify, and nothing generated could reach
                // them while every arm was a single assignment.
                let _ = writeln!(self.body, "  case {c}: {{");
                self.nest += 1;
                self.nested();
                self.nest -= 1;
                let _ = writeln!(self.body, "  {} += {e}; }} break;", target.name);
            } else if self.rng.chance(2) {
                let _ = writeln!(self.body, "  case {c}: {} += {e};", target.name);
            } else {
                let _ = writeln!(self.body, "  case {c}: {} += {e}; break;", target.name);
            }
        }
        let _ = writeln!(self.body, "  }}");
        true
    }

    /// **A forward `goto` over a statement that would otherwise run.**
    ///
    /// The last of the six constructs the census found unemitted, and the one with a termination
    /// constraint attached: a *backward* goto can loop forever and no generated program may hang,
    /// so the label is always emitted after the jump. That is not a limitation of the construct
    /// being tested — chiero agrees with gcc on backward jumps, checked by hand — it is what keeps
    /// this arm safe to put in a corpus.
    ///
    /// The skipped statement mutates a live variable, so whether the jump was taken reaches the
    /// checksum. A `goto` nobody can observe is a `goto` that tests the parser and nothing else.
    ///
    /// The label goes in its own namespace in C (6.2.3p1), so `L7` cannot collide with a variable;
    /// the prefix is for a reader rather than for the compiler.
    fn goto_stmt(&mut self) -> bool {
        let Some(v) = self.pick_var() else {
            return false;
        };
        if matches!(v.ty, Ty::F32 | Ty::F64) {
            return false;
        }
        self.next_id += 1;
        let label = format!("L{}", self.next_id);
        // **Sometimes a GNU local label** (wave 286). `__label__` declares a label local to its
        // block, which is what lets two expansions of one macro coexist in a function —
        // `vppinfra/hash.h` is built on it. It has to be the *first* thing in a compound
        // statement, before any other declaration, so the jump and its label are wrapped in a
        // block rather than emitted into the enclosing one.
        //
        // **A pure naming construct**: no object, no value, nothing added to the checksum or to
        // the program's UB surface. That is why it costs what wave 285's expression wrappers
        // cost rather than what wave 284's array cost, although it is a statement form.
        //
        // The choice comes from `grng`, so the draws every other arm depends on are untouched.
        let local = self.grng.chance(2);
        // **A local label reuses one fixed name, and that is the entire point.** Every other
        // label this generator emits is unique (`L{next_id}`), so a `__label__` that was never
        // renamed, or a scope frame that was never popped, could not collide with anything —
        // both mutants survived until this line. Two blocks in one function each declaring
        // `Ldone` is legal exactly because the declaration makes it block-local, and it is the
        // shape `vppinfra/hash.h` relies on when two `hash_foreach_pair` loops sit in one
        // function.
        let label = if local { "Ldone".to_string() } else { label };
        if local {
            let _ = writeln!(self.body, "  {{ __label__ {label};");
        }
        let cond = self.expr(v.ty);
        let _ = writeln!(self.body, "  if (({cond}) != 0) goto {label};");
        let e = self.expr(v.ty);
        let _ = writeln!(self.body, "  {} += {e};", v.name);
        // **A label needs a statement after it** (6.8.1), and an empty one is the only choice that
        // does not change what the program computes.
        let _ = writeln!(self.body, "  {label}: ;");
        if local {
            let _ = writeln!(self.body, "  }}");
        }
        true
    }

    /// **`&&` and `||`, with a side effect that records whether the right operand ran.**
    ///
    /// Neither operator appeared anywhere in either grammar — the census that found `switch` and
    /// `do`-`while` also found this, and it is the more interesting gap: short-circuiting is
    /// control flow wearing an expression's clothes. C11 6.5.13p4 puts a sequence point between
    /// the operands and evaluates the right one **only if** the left does not already decide the
    /// answer, so a lowering that evaluates both is wrong in a way no value-only comparison of
    /// pure operands could ever see.
    ///
    /// The witness is `sN`, initialized to zero and assigned only inside the right operand. It is
    /// registered as a live variable, so the checksum reads it: if the right operand runs when it
    /// should not, `sN` is 1 where gcc leaves 0.
    ///
    /// One modification, inside an operand with a sequence point before it, and the same variable
    /// is not read elsewhere in the expression — so the fixture is well defined and the UB filter
    /// has nothing to discard.
    fn short_circuit_stmt(&mut self) -> bool {
        let Some(v) = self.pick_var() else {
            return false;
        };
        if matches!(v.ty, Ty::F32 | Ty::F64) {
            // A float operand is fine in C but makes the left operand's truthiness depend on
            // float comparison, which this arm is not about.
            return false;
        }
        let sink = self.fresh();
        let _ = writeln!(self.body, "  int {sink} = 0;");
        let cond = self.expr(v.ty);
        // Both operators, because they short-circuit on *opposite* truth values: `&&` skips the
        // right operand when the left is false and `||` when it is true, so a lowering that got
        // one branch's polarity backwards would pass a fixture set with only one of them.
        let op = if self.rng.chance(2) { "&&" } else { "||" };
        let t = self.fresh();
        let _ = writeln!(
            self.body,
            "  int {t} = (({cond}) != 0) {op} (({sink} = 1) != 0);"
        );
        self.vars.push(Var {
            name: sink,
            ty: Ty::Int,
        });
        self.vars.push(Var {
            name: t,
            ty: Ty::Int,
        });
        true
    }

    /// **A `do`-`while` with a structurally bounded trip count.**
    ///
    /// The one loop form whose body runs before the condition is ever tested, so a lowering that
    /// emits the test first is wrong by exactly one iteration — and nothing in this grammar could
    /// see that, because every loop it generated was a `for`.
    ///
    /// Bounded the same way the `for` arm is: a fresh counter and a literal limit, so no generated
    /// program can fail to terminate and no comparison can hang.
    fn do_while_stmt(&mut self) -> bool {
        let Some(v) = self.pick_var() else {
            return false;
        };
        let n = 1 + self.rng.below(4);
        let i = self.fresh();
        let e = self.expr(v.ty);
        let _ = writeln!(self.body, "  int {i} = 0;");
        // `continue` in a `do`-`while` jumps to the *condition*, so the counter has to be
        // incremented before it or the loop cannot terminate. That ordering is the whole
        // difference between this and the `for` form, and getting it wrong here would hang the
        // comparison rather than fail it.
        if self.extended && self.rng.chance(2) {
            let k = 1 + self.rng.below(n.max(1));
            let _ = writeln!(
                self.body,
                "  do {{ {i}++; if ({i} == {k}) continue; {} += {e}; }} while ({i} < {n});",
                v.name
            );
        } else {
            let _ = writeln!(
                self.body,
                "  do {{ {} += {e}; {i}++; }} while ({i} < {n});",
                v.name
            );
        }
        true
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
        // The wave-284 arms' results — see `extra_sink` for why they do not go through `vars`.
        //
        // **Through an `unsigned long` and into `acc` once.** Folding them the way the loops
        // below fold a global array — `acc = acc * 31 + …` per element — cut the channel from
        // 131 comparisons to **40**: `acc` is a `long`, `* 31` overflows it once the value is
        // large, and four to six extra terms per program makes that common. Signed overflow is
        // undefined, and an undefined program is *discarded* rather than compared. The existing
        // folds carry the same hazard and get away with it by being fewer.
        //
        // `unsigned long` arithmetic wraps and is defined, so the fold cannot be what makes the
        // program undefined; `% 1000003` keeps the one value that reaches `acc` small enough
        // that it does not push `acc` over either. `(long)` before `(unsigned long)` because a
        // *negative* float converted straight to an unsigned type is undefined, and `Ty::ALL`
        // has two float types in it.
        let extra = std::mem::take(&mut self.extra_sink);
        if !extra.is_empty() {
            let _ = writeln!(self.body, "  unsigned long xacc = 0ul;");
            for n in &extra {
                let _ = writeln!(
                    self.body,
                    "  xacc = xacc * 31ul + (unsigned long)(long)({n});"
                );
            }
            let _ = writeln!(self.body, "  acc = acc * 31 + (long)(xacc % 1000003ul);");
        }
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
                // **An integer field is read *bare*; only a float keeps the cast** (wave 253).
                //
                // `(long)(x.f)` makes the member an operand of an explicit cast, and wave 253 showed
                // that is precisely the context where a bit-field's extension bug cannot be seen:
                // `top(e)` becomes the field's own type, so a wrong signedness answer is masked.
                // Every field of every struct went through that shape, which is why raising the
                // frequency of bit-fields twice caught nothing.
                //
                // Dropping the cast changes no value. `acc` is a `long` and `+` promotes its right
                // operand anyway, so the arithmetic is identical — what changes is which conversion
                // the *typed AST* records against the member, and that is the whole question.
                //
                // A `float` or `double` member keeps the cast, because there `(long)` is a real
                // conversion and removing it would change the checksum rather than the AST.
                let ft = self.records[*r].fields[fi];
                if ft.is_float() {
                    let _ = writeln!(self.body, "  acc = acc * 31 + (long)({name}.f{fi});");
                } else {
                    let _ = writeln!(self.body, "  acc = acc * 31 + ({name}.f{fi});");
                }
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

/// The same grammar with a zero divisor allowed, for the arithmetic per-site oracle.
///
/// `memory_ub` stays **off**: a memory fault ends the process at the first one, and this
/// corpus is graded on every arithmetic site in the run.
fn program_arith_ub(seed: u64) -> (String, String) {
    let mut g = Gen::new(seed);
    g.div_zero = true;
    let prelude = g.prelude();
    let n = 3 + g.rng.below(7);
    for _ in 0..n {
        g.stmt();
    }
    (prelude, g.finish())
}

/// The same grammar with [`Gen::oob`] on, for the memory-UB oracle.
/// The same grammar plus `switch` and `do`-`while`.
///
/// A separate entry point rather than a wider default, so every census measured against the old
/// grammar stays comparable and this one can be graded on its own terms.
fn program_control_flow(seed: u64) -> (String, String) {
    let mut g = Gen::new(seed);
    g.extended = true;
    let prelude = g.prelude();
    let n = 3 + g.rng.below(7);
    for _ in 0..n {
        g.stmt();
    }
    (prelude, g.finish())
}

/// **One construct per program, with its own budget.**
///
/// Waves 284 and 287 hit the same wall from opposite directions: coverage in
/// `program_control_flow` is paid for in *comparisons*, because a longer program carrying more
/// adversarial values through more operations is more often undefined somewhere, and undefined
/// is discarded rather than compared. A multi-dimensional array cost 131 comparisons → 80 and
/// bought nothing; the alignment specifier needed a firing rate that cost two below the floor.
/// Neither a better fold nor a lower rate helped — both were tried and measured.
///
/// So this channel does not share that budget. Each program declares what one construct needs
/// and nothing else: no random statements, no adversarial expression tree, no second chance to
/// be undefined. The programs are dull, which is the point — the construct is the only thing in
/// them that can be wrong.
fn program_focused(seed: u64) -> (String, String) {
    let mut g = Gen::new(seed);
    g.extended = true;
    let prelude = g.prelude();
    g.focused_body();
    (prelude, g.finish())
}

fn program_memory_ub(seed: u64) -> (String, String) {
    program_with(seed, true)
}

fn program_with(seed: u64, oob: bool) -> (String, String) {
    let mut g = Gen::new(seed);
    g.memory_ub = oob;
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

    // **And the ratchet runs the other way too, which is the half that was missing.**
    //
    // The check above fails on a refusal with no entry. Nothing failed on an *entry with no
    // refusal* — so an entry outlives the gap it describes, silently, and the ledger becomes
    // a description of a past state that reads exactly like a description of this one. That
    // is the suppression file its own header warns about, arrived at from the other side.
    //
    // The first entry predicted its own removal in so many words — *"this entry is what will
    // fail when they land"* — and float comparisons landed, and it did not fail: an entry
    // that matches nothing is never consulted. Measured before this assertion was written:
    // **57 of the 200 programs contain a float comparison and 0 refuse.**
    //
    // So a gap entry has to still be happening. If it is not, the gap closed and the entry
    // is a claim about chiero that is no longer true.
    let dead: Vec<&str> = KNOWN_GAPS
        .iter()
        .map(|(p, _)| *p)
        .filter(|p| !refused.iter().any(|(_, m)| m.contains(p)))
        .collect();
    assert!(
        dead.is_empty(),
        "{} KNOWN_GAPS entr(ies) matched no refusal in this batch. An entry that fires \
         nowhere is a claim about chiero that nothing checks — either the gap closed and the \
         entry should go, or the corpus stopped reaching it and that is the finding.\n{}",
        dead.len(),
        dead.iter()
            .map(|p| format!("  {p}"))
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

/// **The corpus emits no `typeof`.**
///
/// Wave 277's rule was "ship a construct, then check the corpus can reach it — in the same
/// wave", and I have broken it in every wave since. Of what has landed, only vectors are in the
/// generator: `typeof` (283), `_Generic` (275), `__label__` (276), `__builtin_offsetof` (280),
/// the classification builtins (271), alignment specifiers (282) and multi-dimensional arrays
/// (278) are all graded by hand fixtures alone.
///
/// `typeof` is one wave old, in **37 VPP files**, and cheap to emit: a declaration whose type is
/// copied from something already in scope, plus a `sizeof` of a `typeof` of an *array* — the
/// only shape that can see whether the operand was wrongly decayed.
///
/// # A multi-dimensional array was the other half, and it is not here
///
/// It is the shape that hid wave 278's worst defect, and adding it costs the control-flow
/// channel **a third of its comparisons** — 131 down to 80, and no better at a tenth the rate.
/// The cause is not the fold, which was rewritten twice to rule that out: it is that a longer
/// program carrying more adversarial values through more operations is more likely to be
/// undefined *somewhere*, and an undefined program is discarded rather than compared. Recorded
/// in §9 with the numbers rather than merged at the cost of a guarantee wave 270 put there on
/// purpose.
///
/// Presence is not discrimination; the justification is the mutation sweep in the commit that
/// satisfies this.
/// **The focused channel reaches no designator chain and no `typeof` of a type name.**
///
/// Two mutants are on record as surviving because the corpus cannot reach their shape:
///
///   - **`offsetof_step`'s chained `.field` arm** (wave 280). The corpus emits a single
///     identifier, so `__builtin_offsetof(struct S, n.q)` — which walks into a named member and
///     *then* through an anonymous one — is out of reach and the arm is hand-graded only.
///   - **`TypeKind::TypeofType`** (wave 284). The corpus emits `typeof(<expression>)`; the
///     type-name form is a different arm of `ty_of`.
///
/// Both need a *shape* rather than more seeds, and both are cheap in a channel whose programs
/// contain one construct: an anonymous member costs a record definition, and a `typeof` of a type
/// name costs one declaration. Neither could be afforded in `program_control_flow`, whose
/// comparison budget wave 287 measured as spent.
#[test]
fn the_focused_channel_reaches_the_shapes_the_others_cannot() {
    let (mut chain, mut anon, mut typeof_type) = (0usize, 0usize, 0usize);
    for seed in 0..400u64 {
        let (prelude, body) = program_focused(seed);
        let all = format!("{prelude}{body}");
        // A designator with a `.` in it: `offsetof(struct S, n.q)`.
        if all.contains("__builtin_offsetof") && all.contains(", ") {
            for (i, _) in all.match_indices("__builtin_offsetof(") {
                let rest = &all[i..];
                if let Some(args) = rest.split_once(", ").map(|(_, a)| a)
                    && let Some(desig) = args.split(')').next()
                    && desig.contains('.')
                {
                    chain += 1;
                    break;
                }
            }
        }
        // **An anonymous member is a closing brace with no declarator before the `;`.** The
        // definition is emitted on one line, so `struct { int q; };` shows up as the literal
        // `; };` — a named member would read `; } n;` instead.
        if all.contains("; };") {
            anon += 1;
        }
        // `typeof(int)` rather than `typeof(v)`: the operand is a type keyword.
        if all.contains("__typeof__(int")
            || all.contains("__typeof__(long")
            || all.contains("__typeof__(unsigned")
            || all.contains("__typeof__(char")
            || all.contains("__typeof__(double")
        {
            typeof_type += 1;
        }
    }
    assert!(
        chain >= 30,
        "an `offsetof` designator with a `.` chain: {chain}"
    );
    assert!(anon >= 30, "an anonymous struct or union member: {anon}");
    assert!(
        typeof_type >= 30,
        "`typeof` of a *type name*, not an expression: {typeof_type}"
    );
}

/// **The focused channel computes what gcc computes.**
///
/// The channel waves 284 and 287 concluded was needed: one construct per program, its own
/// statement budget, its own floor. Its whole justification is that it can afford coverage the
/// control-flow channel cannot, so the number to watch is `compared` — if these programs start
/// getting discarded, the design has failed and the constructs have simply moved.
#[test]
fn focused_programs_agree_with_gcc() {
    let mut compared = 0usize;
    let mut discarded = 0usize;
    let mut refused: Vec<(u64, String)> = Vec::new();
    let mut defects: Vec<(u64, String, Verdict)> = Vec::new();
    for seed in 0..200u64 {
        let (prelude, body) = program_focused(seed);
        match judge(&prelude, &body) {
            Verdict::Agree => compared += 1,
            Verdict::Discarded | Verdict::Gap { .. } => discarded += 1,
            Verdict::Refused { stage, message } => {
                refused.push((seed, format!("{stage}: {message}")));
            }
            v => defects.push((seed, format!("{prelude}\nint probe(void) {{\n{body}}}"), v)),
        }
    }
    eprintln!(
        "focused channel: {compared} compared, {discarded} discarded, {} refused",
        refused.len()
    );
    assert!(
        defects.is_empty(),
        "chiero disagreed with gcc on {} program(s): {:#?}",
        defects.len(),
        defects
    );
    assert!(
        refused.is_empty(),
        "an undeclared refusal is never honest (023 §7): {refused:#?}"
    );
    // **The point of the channel.** `program_control_flow` compares 100 of 200 and cannot afford
    // another construct; these programs contain one construct and nothing else, so nearly all of
    // them should be comparable. A floor near the top is what makes the design falsifiable.
    assert!(
        compared >= 180,
        "a channel of single-construct programs has no excuse for discards: {compared} of 200"
    );
}

/// **Nothing emits a non-square multi-dimensional array, or an alignment pairing that is
/// graded.**
///
/// The two constructs waves 284 and 287 could not afford in `program_control_flow`. The
/// multi-dimensional array is the shape that hid wave 278's worst defect — `int a[2][3]` typed
/// as `int a[3][2]` for the life of the project — and a *square* one is its own reverse, so the
/// extents must differ. The alignment specifier is emitted there but at a rate too low to
/// discriminate: rate 4 kills wave 282's mutants and costs two comparisons below the floor.
///
/// `program_focused` is the channel with its own budget. This asserts it carries both, and that
/// its programs are **short** — the property the whole design rests on, and the one that will
/// erode first if someone adds statements to it.
#[test]
fn the_focused_channel_carries_what_the_others_cannot() {
    let (mut md, mut nonsquare, mut aligned_pair) = (0usize, 0usize, 0usize);
    let mut longest = 0usize;
    for seed in 0..400u64 {
        let (prelude, body) = program_focused(seed);
        let all = format!("{prelude}{body}");
        longest = longest.max(body.lines().count());
        for l in all.lines() {
            let t = l.trim();
            if t.contains(" = {") && t.matches('[').count() >= 2 && !t.contains('(') {
                md += 1;
                let dims: Vec<&str> = t[t.find('[').expect("checked")..]
                    .split(']')
                    .filter_map(|p| p.strip_prefix('['))
                    .collect();
                if dims.len() >= 2 && dims[0] != dims[1] {
                    nonsquare += 1;
                }
            }
            if t.starts_with("_Alignas(")
                && let Some((decl, _)) = t.split_once(" = ")
                && let Some(name) = decl.split_whitespace().last()
                && all.contains(&format!("_Alignof({name})"))
            {
                aligned_pair += 1;
            }
        }
    }
    assert!(md >= 40, "a multi-dimensional array is emitted: {md}");
    assert!(
        nonsquare >= 40,
        "a **non-square** one, since a square array is its own reverse: {nonsquare}"
    );
    assert!(
        aligned_pair >= 40,
        "an alignment specifier with an `_Alignof` that names it: {aligned_pair}"
    );
    // **Short is the design.** These programs exist to be comparable, not interesting; the
    // moment they grow they start paying the same price the control-flow channel pays.
    assert!(
        longest <= 30,
        "a focused program stays short, or it is just another crowded channel: {longest} lines"
    );
}

/// **The corpus emits no alignment specifier.**
///
/// The last of wave 277's six lagging constructs, and the only one left untried — the
/// multi-dimensional array is measured out (wave 284).
///
/// # It needs two halves, and neither alone is worth anything
///
/// `_Alignas(32)` on a declaration is invisible: it changes no value the checksum reads. The
/// only observable is `_Alignof` **of that object**, which is why wave 282 needed both a
/// per-declaration channel and a way to read it back. So the corpus has to emit the specifier
/// *and* an expression that asks about it, and the expression has to name the object that
/// carries it.
///
/// # Which makes it an expression wrapper after all
///
/// `_Alignof(v) == 32 ? e : 0` is a **selector**, the shape wave 285 established: no arithmetic
/// that can overflow, no storage, no statement. If the specifier is dropped, `_Alignof` answers
/// the type's natural alignment, the branch flips and the value changes — which is exactly the
/// wave-282 defect, and exactly what a corpus is for.
#[test]
fn the_corpus_reaches_alignment_specifiers() {
    let (mut spec, mut observed) = (0usize, 0usize);
    for seed in 0..600u64 {
        let (prelude, body) = program_control_flow(seed);
        let all = format!("{prelude}{body}");
        if !all.contains("_Alignas(") {
            continue;
        }
        spec += 1;
        // The specifier is worth nothing unless something asks about the object that carries it.
        for l in all.lines() {
            let t = l.trim();
            // `_Alignas(32) unsigned long v7 = …;` — the name is the token before the `=`,
            // since the type can be two or three words.
            if t.starts_with("_Alignas(")
                && let Some((decl, _)) = t.split_once(" = ")
                && let Some(name) = decl.split_whitespace().last()
                && all.contains(&format!("_Alignof({name})"))
            {
                observed += 1;
                break;
            }
        }
    }
    assert!(spec >= 20, "an alignment specifier is emitted: {spec}");
    assert!(
        observed >= 20,
        "and `_Alignof` asks about the object that carries it: {observed}"
    );
}

/// **The corpus emits no `__label__`.**
///
/// The last of wave 277's six lagging constructs that is worth adding — the multi-dimensional
/// array is measured out (wave 284) and the alignment specifiers have no cheap observable.
///
/// # Why this one should be nearly free
///
/// Wave 285's lever was that an *expression wrapper* costs far less than a statement or an
/// object, because it adds no storage, no statement and no `rng` draw. `__label__` is a
/// *statement* form and so does not fit that shape — but it is a **pure naming construct**: it
/// declares no object, computes no value, and adds nothing to the checksum or to the program's
/// UB surface. It should cost close to nothing for the same underlying reason.
///
/// # It has to open a block
///
/// C puts label declarations at the start of a compound statement, before any other
/// declaration — gcc rejects `{ int x; __label__ d; }`. So the natural host is the `goto` arm
/// this channel already has: wrapping its jump and its label in a block whose first line is the
/// declaration is exactly what the construct is for, and needs no new statement of its own.
#[test]
fn the_corpus_reaches_local_labels() {
    let (mut decl, mut used) = (0usize, 0usize);
    for seed in 0..600u64 {
        let (prelude, body) = program_control_flow(seed);
        let all = format!("{prelude}{body}");
        if !all.contains("__label__") {
            continue;
        }
        decl += 1;
        // The declared name has to be the one jumped to, or the construct is decoration.
        for l in all.lines() {
            let t = l.trim();
            // The declaration opens the block, so the line reads `{ __label__ L7;`.
            let t = t.trim_start_matches(['{', ' ']);
            if let Some(rest) = t.strip_prefix("__label__ ")
                && let Some(name) = rest.strip_suffix(';')
                && all.contains(&format!("goto {name};"))
            {
                used += 1;
                break;
            }
        }
    }
    assert!(decl >= 20, "`__label__` is declared: {decl}");
    assert!(used >= 20, "and the label it declares is jumped to: {used}");
}

/// **The corpus emits none of `_Generic`, `__builtin_offsetof` or the classification builtins.**
///
/// §9's three cheapest remaining candidates, and cheap for one reason: each adds a **value**
/// rather than an object. Wave 284 measured what an object costs — a multi-dimensional array
/// took the control-flow channel from 131 comparisons to 80, because a longer program carrying
/// more adversarial values through more operations is more often undefined somewhere, and
/// undefined is discarded. A value that replaces a subexpression adds no statement, no storage
/// and no new operand to the checksum.
///
/// Each is one wave old or less and each is graded only by hand fixtures: `_Generic` (275),
/// `__builtin_offsetof` (280), the classification builtins (271).
///
/// Presence is not discrimination; the justification is the mutation sweep in the commit that
/// satisfies this.
#[test]
fn the_corpus_reaches_recent_expression_forms() {
    let (mut generic, mut offsetof, mut classify) = (0usize, 0usize, 0usize);
    for seed in 0..600u64 {
        let (prelude, body) = program_control_flow(seed);
        let all = format!("{prelude}{body}");
        if all.contains("_Generic") {
            generic += 1;
        }
        if all.contains("__builtin_offsetof") {
            offsetof += 1;
        }
        if all.contains("__builtin_isnan")
            || all.contains("__builtin_isless")
            || all.contains("__builtin_isunordered")
        {
            classify += 1;
        }
    }
    assert!(generic >= 20, "`_Generic` is used: {generic}");
    assert!(offsetof >= 20, "`__builtin_offsetof` is used: {offsetof}");
    assert!(
        classify >= 20,
        "a floating classification builtin is used: {classify}"
    );
}

#[test]
fn the_corpus_reaches_recent_constructs() {
    let (mut tyof, mut tyof_arr) = (0usize, 0usize);
    for seed in 0..600u64 {
        let (prelude, body) = program_control_flow(seed);
        let all = format!("{prelude}{body}");
        if all.contains("__typeof__") || all.contains("typeof(") {
            tyof += 1;
        }
        if all.contains("sizeof(__typeof__(")
            || all.contains("sizeof(typeof(")
            || all.contains("sizeof(__typeof(")
        {
            tyof_arr += 1;
        }
    }
    assert!(tyof >= 20, "`typeof` is used: {tyof}");
    assert!(
        tyof_arr >= 10,
        "`sizeof` of a `typeof` of an array, the only shape that sees a wrong decay: {tyof_arr}"
    );
}

/// **The corpus contains no vectors at all, and three waves of them went in on hand fixtures.**
///
/// Waves 272, 273 and 274 built `vector_size` out — initializers, subscripts, elementwise
/// arithmetic, the scalar broadcast, compound assignment, and comparisons with their mask type —
/// and every one of those is graded by fixtures written by hand. The generator emits zero
/// vectors, so nothing in the corpus can find the next vector defect the way it found the
/// `_Bool b += -1` one.
///
/// # This is the wave-270 rule turned on my own work
///
/// "Adding a construct to the corpus buys nothing until you know the context can discriminate"
/// cuts both ways: shipping a construct the corpus cannot reach leaves it graded by whatever a
/// person thought to spell, which is the bottleneck the generator exists to remove. Three waves
/// is a lot of surface to leave there.
///
/// # What this asserts, and what it deliberately does not
///
/// Presence, at the shapes the hand fixtures showed were load-bearing: a lane whose value the
/// operator changed, a *narrow* lane where the operator's width is not `int`'s, and the result
/// reaching the checksum. Presence is not discrimination — wave 270 is emphatic about that — so
/// the justification for this addition is the mutation sweep in the commit that satisfies it,
/// not this test.
#[test]
fn the_corpus_contains_vectors() {
    let (mut decl, mut init, mut subscript, mut arith, mut cmp, mut narrow) =
        (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    for seed in 0..600u64 {
        let (prelude, body) = program_control_flow(seed);
        let all = format!("{prelude}{body}");
        // The vector locals this program declared, by name, so the later checks key on the
        // generator's actual output rather than on a naming convention it does not have.
        let mut names: Vec<String> = Vec::new();
        let (mut d, mut i, mut s, mut a, mut c, mut n) = (0, 0, 0, 0, 0, 0);
        for l in all.lines() {
            let t = l.trim();
            if !t.contains("__attribute__((vector_size(") {
                continue;
            }
            d += 1;
            if t.contains("vector_size(8)") {
                n += 1;
            }
            let Some(eq) = t.find(" = ") else { continue };
            let name = t[..eq].split_whitespace().last().unwrap_or("").to_string();
            let rhs = &t[eq + 3..];
            // **Every vector local, not only the initialized ones.** The lane read is of the
            // *result* vector, which is assigned an expression — collecting only the braced
            // ones made this count zero and read exactly like a generator that emits no reads.
            names.push(name);
            if rhs.starts_with('{') {
                i += 1;
            } else if rhs.contains("==") || rhs.contains('<') {
                c += 1;
            } else {
                a += 1;
            }
        }
        // A lane read of a vector this program declared.
        for l in all.lines() {
            let t = l.trim();
            if t.contains("__attribute__") {
                continue;
            }
            if names.iter().any(|v| t.contains(&format!("{v}["))) {
                s += 1;
            }
        }
        decl += usize::from(d > 0);
        init += usize::from(i > 0);
        subscript += usize::from(s > 0);
        arith += usize::from(a > 0);
        cmp += usize::from(c > 0);
        narrow += usize::from(n > 0);
    }
    assert!(decl >= 20, "a vector type is declared: {decl}");
    assert!(init >= 20, "a vector is braced-initialized: {init}");
    assert!(arith >= 10, "elementwise arithmetic happens: {arith}");
    assert!(cmp >= 10, "a vector comparison happens: {cmp}");
    assert!(
        narrow >= 10,
        "a narrow lane, where the operator's width is not `int`'s: {narrow}"
    );
    assert!(subscript >= 20, "a vector lane is read: {subscript}");
}

/// **`switch` and `do`-`while` compute what gcc computes.**
///
/// Two statement forms `StmtKind` has always had, the parser has always parsed and lowering has
/// always claimed to handle, which this generator never emitted — found by asking what the AST can
/// hold rather than what the grammar happened to say.
///
/// `&&` and `||` came from the same census and are the more interesting gap: short-circuiting is
/// control flow wearing an expression's clothes, and C11 §6.5.13p4 evaluates the right operand only
/// if the left does not already decide the answer. The generated fixture records that in a live
/// variable, so a lowering that evaluates both shows up in the checksum rather than in nothing.
///
/// `switch` is the one construct here whose arms *fall through*, and the generator emits empty
/// cases that fall into the next, a `default` that is not last, and arms with and without `break`.
/// `do`-`while` is the one loop whose body runs before the condition is tested, so a lowering that
/// emits the test first is wrong by exactly one iteration and every `for` in the old grammar would
/// have agreed with gcc anyway.
///
/// Fixed seeds, for the reason `generated_programs_agree_with_gcc` gives: an unseeded random test
/// that fails one run in ten gets muted, and a muted channel is worse than none.
#[test]
fn control_flow_programs_agree_with_gcc() {
    let mut compared = 0usize;
    let mut discarded = 0usize;
    let mut gaps = 0usize;
    let mut refused: Vec<(u64, String)> = Vec::new();
    let mut defects: Vec<(u64, String, Verdict)> = Vec::new();
    for seed in 0..200u64 {
        let (prelude, body) = program_control_flow(seed);
        match judge(&prelude, &body) {
            Verdict::Agree => compared += 1,
            Verdict::Discarded => discarded += 1,
            // A **declared** limit. 023 §7 calls this the honest outcome, so it is tolerated —
            // but counted, because a grammar that drifted into hitting a bound on every seed
            // would be grading nothing while every assertion here still passed.
            Verdict::Gap { .. } => gaps += 1,
            Verdict::Refused { stage, message } => {
                refused.push((seed, format!("{stage}: {message}")))
            }
            v => defects.push((seed, format!("{prelude}\nint probe(void) {{\n{body}}}"), v)),
        }
    }
    eprintln!(
        "control-flow channel: {compared} compared, {discarded} discarded, {gaps} declared gaps, \
         {} refused",
        refused.len()
    );
    assert!(
        defects.is_empty(),
        "chiero disagreed with gcc on {} program(s): {:#?}",
        defects.len(),
        defects
    );
    // **A refusal is graded, not swept in with the discards.** This assertion is the wave-270
    // finding, and it is about the channel rather than about any program in it: `Refused` used to
    // land in the same bucket as `Discarded`, so lowering that produced *nothing at all* read
    // exactly like a program the oracle chose not to compare.
    //
    // That is how `x && <float>` hid. Every one of them emitted CIR the verifier rejects, the
    // function was dropped whole, and the channel counted it as ordinary. A wrong answer would
    // have been caught on the first seed; producing no answer was free. Under 023 §7 an
    // *undeclared* refusal is the one outcome that is never honest — a declared limit is a `Gap`,
    // which is counted just above — so the bound here is zero.
    assert!(
        refused.is_empty(),
        "lowering refused {} of 200 programs, and a refusal nothing declared is a defect that \
         costs nothing to hide behind: {:#?}",
        refused.len(),
        refused
    );
    assert!(
        gaps < 20,
        "declared gaps are honest but they are not comparisons; {gaps} of 200 means the channel \
         is grading far less than it appears to"
    );
    assert!(
        compared >= 100,
        "the channel has to actually compare programs to be worth running: {compared} of 200"
    );
    // **The channel has to actually run these programs**, or a grammar that quietly stopped
    // emitting `switch` would leave this test passing on nothing. Wave 216's rule: before
    // asserting an absence, produce the thing whose absence you are claiming.
    let with_switch = (0..SHAPE_SEEDS)
        .filter(|s| program_control_flow(*s).1.contains("switch ("))
        .count();
    let with_do = (0..SHAPE_SEEDS)
        .filter(|s| program_control_flow(*s).1.contains("do {"))
        .count();
    assert!(
        with_switch >= 20 && with_do >= 20,
        "the grammar must emit both forms often enough to grade: {with_switch} switch, {with_do} do-while"
    );
    // **And the shapes *within* a `switch`**, because "emits a switch" is not the claim. A
    // `switch` whose every arm breaks is a chain of `if`s with different syntax; the fallthrough
    // is the only thing here that no other construct in this grammar can express. Mutation found
    // the empty-case arm removable with nothing noticing, which is a channel quietly exploring
    // less than it says it does.
    let with_fallthrough = (0..SHAPE_SEEDS)
        .filter(|s| {
            program_control_flow(*s)
                .1
                .lines()
                .any(|l| l.trim_end().ends_with("case 1:"))
        })
        .count();
    assert!(
        with_fallthrough >= 20,
        "an empty case falling into the next one is the shape a `switch` is here for: \
         {with_fallthrough}"
    );
    // A `default` that is **not last**, which C allows and a lowering that assumes otherwise
    // would still pass every fixture that puts it at the end. Also mutation-motivated: swapping
    // `default` for another `case` label changed nothing any test could see.
    let with_early_default = (0..SHAPE_SEEDS)
        .filter(|s| {
            let body = program_control_flow(*s).1;
            let Some(d) = body.find("  default:") else {
                return false;
            };
            body[d..].contains("  case ")
        })
        .count();
    assert!(
        with_early_default >= 20,
        "a `default` before the last case is ordinary C and easy to lower wrongly: \
         {with_early_default}"
    );
    // **The wave-270 census's three forms, each with the *value* that makes it discriminate.**
    //
    // Presence of the shape is not coverage — that is this channel's most repeated lesson and it
    // repeated again here twice in one wave. The first `!` arm emitted `x && !x`, which
    // short-circuits away the `!` for exactly the zero operand that discriminates; the second
    // emitted `!` of a nested subexpression, and across *2000* seeds not one of the 41 sites had
    // an operand that evaluated to a negative zero. Both looked like coverage in a census that
    // counted occurrences.
    //
    // So what is required here is `!` **applied to a float constant**, where the pool's `-0.0`
    // can actually reach the operator. A negative zero is the only float value that separates
    // "compares equal to zero" from "has zero bits", which is the whole of what `!` gets wrong.
    let (mut with_not, mut with_not_float, mut with_sc_right, mut with_alignof) = (0, 0, 0, 0);
    for seed in 0..SHAPE_SEEDS {
        let (prelude, body) = program_control_flow(seed);
        let all = format!("{prelude}{body}");
        if all.contains("(!(") {
            with_not += 1;
        }
        if all.contains("(!(-0.0") {
            with_not_float += 1;
        }
        if all.contains("(1 && (") {
            with_sc_right += 1;
        }
        if all.contains("_Alignof") {
            with_alignof += 1;
        }
    }
    assert!(
        with_not >= 10,
        "`!` appears in no ExprKind census of this grammar before wave 270: {with_not}"
    );
    assert!(
        with_not_float >= 3,
        "`!` of a negative zero is the only shape that catches a truth test done on bits:          {with_not_float}"
    );
    assert!(
        with_sc_right >= 10,
        "a scalar on the *right* of a short circuit takes a different path through lowering          than the left, and the right one was the broken one: {with_sc_right}"
    );
    assert!(
        with_alignof >= 10,
        "`_Alignof` yields `size_t`, whose unsignedness wins the usual arithmetic conversions:          {with_alignof}"
    );
    // **Both short-circuit operators, and the witness that says whether the right operand ran.**
    // `&&` and `||` skip on *opposite* truth values, so a lowering with one branch's polarity
    // backwards would pass a corpus containing only one of them.
    let (mut with_and, mut with_or) = (0usize, 0usize);
    for seed in 0..SHAPE_SEEDS {
        let body = program_control_flow(seed).1;
        if body.contains("&&") {
            with_and += 1;
        }
        if body.contains("||") {
            with_or += 1;
        }
    }
    assert!(
        with_and >= 10 && with_or >= 10,
        "short-circuiting is control flow wearing an expression's clothes, and both directions \
         have to appear: {with_and} `&&`, {with_or} `||`"
    );
    // **And the witness has to reach the checksum.** Mutation found the arm's `vars.push` for the
    // sink removable with nothing noticing: the program still contains `&&`, still agrees with
    // gcc, and no longer observes whether the right operand ran — which is the only thing the arm
    // is for.
    //
    // A count rather than a universal, and the first version was the universal: it failed on seed
    // 89, where the short-circuit sits inside a nested block whose variables `nested` correctly
    // truncates at the closing brace. Out of scope is out of the checksum, so what can be required
    // is that the shape is *often* observable, not always.
    let graded = (0..SHAPE_SEEDS)
        .filter(|seed| {
            let body = program_control_flow(*seed).1;
            body.match_indices("= 1) != 0)").any(|(i, _)| {
                let open = body[..i].rfind("((").expect("the arm emits the pair");
                let name = body[open + 2..i].trim();
                body.contains(&format!("(long)({name})"))
            })
        })
        .count();
    assert!(
        graded >= 10,
        "a short-circuit whose witness nothing reads is emitted and not graded: {graded} \
         program(s) observe theirs"
    );
    // **`goto` and `continue`**, the last two constructs the census found unemitted. Counted the
    // same way and for the same reason: emitting a construct is not the claim, exercising it is.
    // **The two `continue` forms are counted apart**, because they are different constructs
    // wearing one keyword: in a `for` the increment still runs, in a `do`-`while` control jumps to
    // the condition. Counting the keyword found them together, and turning the `for` form off
    // alone changed nothing any assertion could see.
    // **The shape counts sample more seeds than the comparison does**, and deliberately: they ask
    // whether the *grammar* emits a shape often enough to exercise it, which is a different
    // question from how many programs this batch compared. Adding the wave-220 expression arms
    // reshuffled the stream and dropped `for`-continue to nine lines in two hundred programs —
    // the guard fired, which is what it is for, and the answer is a bigger sample rather than a
    // lower bar.
    const SHAPE_SEEDS: u64 = 600;
    let (mut with_goto, mut cont_for, mut cont_do) = (0usize, 0usize, 0usize);
    for seed in 0..SHAPE_SEEDS {
        let body = program_control_flow(seed).1;
        if body.contains("goto L") {
            with_goto += 1;
        }
        for l in body.lines() {
            if !l.contains("continue;") {
                continue;
            }
            if l.contains("for (") {
                cont_for += 1;
            }
            if l.contains("do {") {
                cont_do += 1;
            }
        }
    }
    assert!(
        with_goto >= 10 && cont_for >= 10 && cont_do >= 10,
        "the last two census constructs have to appear, and `continue` in both loop forms: \
         {with_goto} goto, {cont_for} for-continue, {cont_do} do-continue"
    );
    // **A `goto` has to skip something observable.** Deleting the statement between the jump and
    // its label left every count above satisfied and the construct testing the parser and nothing
    // else — the same failure the short-circuit witness had.
    let goto_skips = (0..SHAPE_SEEDS)
        .filter(|seed| {
            let body = program_control_flow(*seed).1;
            body.match_indices("goto L").any(|(i, _)| {
                let label = body[i + 5..]
                    .split(';')
                    .next()
                    .expect("terminated")
                    .trim()
                    .to_string();
                let target = body.find(&format!("{label}: ;")).unwrap_or(0);
                // An *executable* statement, not merely the text of one. The first version
                // matched `" += "` anywhere in the span and a mutant that commented the line
                // out passed it — the characters survive a `//` and the effect does not.
                target > i
                    && body[i..target]
                        .lines()
                        .any(|l| l.contains(" += ") && !l.trim_start().starts_with("//"))
            })
        })
        .count();
    assert!(
        goto_skips >= 10,
        "a jump over nothing is a jump nothing can observe: {goto_skips}"
    );
    // **The wave-220 expression forms**, each counted separately because each is a different
    // conversion question: `~` promotes a narrow operand, `sizeof` contributes an unsigned type
    // wider than `int` and drags the usual arithmetic conversions with it, a character constant is
    // an `int` however narrow its value, and a string literal is an array of static storage
    // duration read through a subscript.
    let mut leaves = [0usize; 4];
    for seed in 0..SHAPE_SEEDS {
        let body = program_control_flow(seed).1;
        for (i, pat) in ["(~(", "sizeof(long)", "'A'", "\"abcd\"["]
            .iter()
            .enumerate()
        {
            if body.contains(pat) {
                leaves[i] += 1;
            }
        }
    }
    // **The compositions themselves.** Wave 221's change is not a new construct but the removal of
    // a structural bound: a loop body and a `switch` arm can now hold arbitrary statements, so a
    // `goto` out of a `switch` inside a loop is reachable. Counting the *constructs* cannot see
    // that — every count above was already satisfied before the arms could nest — so the shapes
    // are counted directly.
    let (mut loop_holds_stmts, mut case_holds_block) = (0usize, 0usize);
    for seed in 0..SHAPE_SEEDS {
        let body = program_control_flow(seed).1;
        // A composing loop is emitted as `for (...) {` with the body on later lines; the old form
        // puts the whole body on one line.
        if body
            .lines()
            .any(|l| l.contains("for (") && l.trim_end().ends_with('{'))
        {
            loop_holds_stmts += 1;
        }
        if body.contains(": {") {
            case_holds_block += 1;
        }
    }
    assert!(
        loop_holds_stmts >= 10 && case_holds_block >= 10,
        "the arms have to actually nest, or this is the old grammar with more words: \
         {loop_holds_stmts} composing loops, {case_holds_block} case blocks"
    );
    // **And the nesting stays bounded**, which is a property of the corpus rather than of the
    // language: three nested loops of four iterations each is sixty-four passes, and a comparison
    // that takes a second is a comparison nobody runs. Mutation raised the `nest` cap from two to
    // nine and nothing noticed, because the programs still terminate and still agree — the cost is
    // in time, which no assertion was watching.
    let deepest = (0..SHAPE_SEEDS)
        .map(|seed| {
            let body = program_control_flow(seed).1;
            let mut depth = 0i32;
            let mut max = 0i32;
            for c in body.chars() {
                match c {
                    '{' => {
                        depth += 1;
                        max = max.max(depth);
                    }
                    '}' => depth -= 1,
                    _ => {}
                }
            }
            max
        })
        .max()
        .unwrap_or(0);
    assert!(
        deepest <= 6,
        "a generated program's blocks must stay shallow enough to compare quickly: depth \
         {deepest}"
    );
    assert!(
        leaves.iter().all(|c| *c >= 10),
        "every expression form the census found unemitted has to be emitted now: \
         ~={} sizeof={} char={} string={}",
        leaves[0],
        leaves[1],
        leaves[2],
        leaves[3]
    );
    // Every `goto` this grammar emits jumps **forward**, and that is a safety property rather than
    // a coverage one: a backward jump can loop forever and a hanging comparison is worse than a
    // failing one. chiero agrees with gcc on backward jumps — checked by hand — so this bounds the
    // corpus, not the construct.
    for seed in 0..SHAPE_SEEDS {
        let body = program_control_flow(seed).1;
        if let Err(why) = backward_goto(&body) {
            panic!("seed {seed}: {why}");
        }
    }
    assert!(
        compared >= 50,
        "too few comparisons to mean anything: {compared}"
    );
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
    // **`SOAK_CF=1` searches the `switch`/`do`-`while` grammar** instead of the default one. A
    // separate switch rather than a wider default, so a census taken here stays comparable with
    // every earlier one — the frontier numbers in §9 were all measured on the old shapes.
    let cf = std::env::var("SOAK_CF").is_ok_and(|v| v != "0");
    for seed in lo..hi {
        let (prelude, body) = if cf {
            program_control_flow(seed)
        } else {
            program(seed)
        };
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
fn arithmetic_ub_agrees_with_gcc_site_for_site() {
    let dir = std::path::PathBuf::from(std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into()))
        .join("chiero-sites");
    let _ = std::fs::create_dir_all(&dir);
    let (mut agree, mut miss, mut extra, mut progs) = (0u32, 0u32, 0u32, 0u32);
    // **Per kind, not just a total.** Waves 176 and 178 both found a row that read clean
    // because it was matching nothing, and only a table split by the thing the corpus is
    // supposed to vary can show that. A total of "68 of 68" says nothing about whether all
    // four kinds are in the corpus at all.
    let mut by_kind: std::collections::BTreeMap<&str, (u32, u32)> = Default::default();
    let mut shown = 0u32;
    for seed in 0..200u64 {
        let (prelude, body) = program_arith_ub(seed);
        let src = format!("{prelude}\nint probe(void) {{\n{body}}}\n");
        let csrc = format!(
            "{src}\n#include <stdio.h>\nint main(void){{ printf(\"%d\\n\", probe()); return 0; }}\n"
        );
        let c = dir.join(format!("s{seed}.c"));
        let x = dir.join(format!("s{seed}"));
        if std::fs::write(&c, &csrc).is_err() {
            continue;
        }
        let ok = std::process::Command::new("gcc")
            .args([
                "-O0",
                "-g",
                "-fsanitize=undefined,float-cast-overflow",
                "-o",
            ])
            .arg(&x)
            .arg(&c)
            .output();
        match ok {
            Ok(o) if o.status.success() => {}
            _ => continue,
        }
        let Ok(run) = std::process::Command::new(&x).output() else {
            continue;
        };
        let err = String::from_utf8_lossy(&run.stderr).to_string();
        let mut gcc_sites: Vec<(u32, &str)> = Vec::new();
        for l in err.lines().filter(|l| l.contains("runtime error:")) {
            let line: Option<u32> = l.split(':').nth(1).and_then(|n| n.parse().ok());
            let kind = if l.contains("signed integer overflow") {
                "SignedOverflow"
            } else if l.contains("shift") {
                "Shift"
            } else if l.contains("outside the range of representable") {
                "FloatCastOverflow"
            } else if l.contains("division by zero") {
                "DivByZero"
            } else if l.contains("cannot be represented in type") {
                // **`INT_MIN / -1`, which UBSan gives a message of its own** (wave 263). It says
                // neither "signed integer overflow" nor "division by zero", so before this arm it
                // classified as `"?"` — a kind chiero never emits, scoring a *miss* and failing
                // loudly. That is the right default and it is why this had to be added alongside
                // the check rather than after it: the corpus does not produce the shape today, and
                // the day it does the oracle would have blamed chiero for gcc's wording.
                "SignedOverflow"
            } else {
                "?"
            };
            if let Some(n) = line {
                gcc_sites.push((n, kind));
            }
        }
        if gcc_sites.is_empty() {
            continue;
        }
        let Some((m, map)) = harness::lower_maybe_with_map(&src) else {
            continue;
        };
        let mut arena = chiero_solver::TermArena::new();
        let r = chiero_exec::Engine::new(&m)
            .with_entry("probe")
            .run(&mut arena);
        let mut ours: Vec<(u32, String)> = r
            .states()
            .iter()
            .flat_map(|s| s.ub_events())
            .filter_map(|u| {
                map.lookup_loc(u.span.lo)
                    .map(|l| (l.line, format!("{:?}", u.kind)))
            })
            .collect();
        ours.sort();
        ours.dedup();
        progs += 1;
        for (gl, gk) in &gcc_sites {
            let e = by_kind.entry(gk).or_default();
            e.0 += 1;
            if ours.iter().any(|(l, k)| l == gl && k == gk) {
                e.1 += 1;
                agree += 1;
            } else {
                miss += 1;
                if shown < 6 {
                    shown += 1;
                    println!("seed {seed}: gcc {gk} at line {gl}; chiero {ours:?}");
                }
            }
        }
        // **A trapping fault ends gcc's run and not chiero's.** Integer division by zero
        // raises SIGFPE on x86-64, so gcc stops there while chiero records the event and
        // carries on (020 §4.1: an arithmetic UB event does not end the path). Every site
        // after that point is one gcc never executed, so counting them as disagreements
        // would compare chiero against a run that did not happen.
        //
        // Only the *extras* are suppressed. `miss` still applies: whatever gcc did manage
        // to report before dying, chiero must have found.
        let truncated = {
            use std::os::unix::process::ExitStatusExt as _;
            run.status.signal().is_some()
        };
        for (l, k) in &ours {
            if truncated {
                break;
            }
            if !gcc_sites.iter().any(|(gl, gk)| gl == l && gk == k) {
                extra += 1;
                println!("EXTRA seed {seed}: chiero {k} at line {l}; gcc {gcc_sites:?}");
                std::fs::write("/tmp/extra.c", &csrc).unwrap();
                for u in r.states().iter().flat_map(|s| s.ub_events()) {
                    println!("    ub {:?}: {}", u.kind, u.detail);
                }
                let src_lines: Vec<&str> = src.lines().collect();
                if let Some(t) = src_lines.get(*l as usize - 1) {
                    println!("    line {l}: {t}");
                }
            }
        }
    }
    println!("arithmetic sites: programs={progs} agree={agree} miss={miss} extra={extra}");
    for (kind, (seen, caught)) in &by_kind {
        println!("  {seen:3} / {caught:<3}  {kind}");
    }
    // **Every kind the engine can report must appear in the corpus.** Three, not one, for
    // the reason wave 180 established: a row of one is a row whose next regression is a coin
    // flip away from being invisible.
    for kind in ["SignedOverflow", "Shift", "DivByZero", "FloatCastOverflow"] {
        let seen = by_kind.get(kind).map(|c| c.0).unwrap_or(0);
        assert!(
            seen >= 3,
            "only {seen} site(s) of kind `{kind}` in the corpus, too few to grade chiero \
             on it. The corpus produced: {:?}",
            by_kind.keys().collect::<Vec<_>>()
        );
    }
    assert!(
        progs >= 10,
        "too few programs commit arithmetic UB to grade anything: {progs}"
    );
    // **Every site gcc reports, chiero must report.** This is the direction that can only
    // be a chiero defect: gcc executed the operation and the standard calls it undefined.
    assert_eq!(
        miss, 0,
        "gcc reported UB at a site chiero did not — see the lines printed above"
    );
    // **`extra` is reported, not asserted, and the reason is measured rather than assumed.**
    //
    // UBSan only checks operations that survive to code generation, and gcc's front end
    // folds an arithmetic result used solely as a condition. Reduced from seed 161 and
    // confirmed both ways in one program:
    //
    // ```text
    //   if (x * (-65536)) { … }        // no report
    //   int y = x * (-65536);          // runtime error: 131329 * -65536 …
    // ```
    //
    // Identical multiplication, identical operands; the first is optimised into `if (x)`
    // because a non-zero constant factor cannot change whether the product is zero, and
    // the check disappears with it. chiero evaluates the program as written and is right.
    //
    // So a site chiero reports and gcc does not is **not** evidence of a false positive
    // here — unlike the memory oracle's `invented`, where ASan's silence does mean the
    // access was in bounds. The asymmetry is real and belongs in the code rather than in a
    // reader's memory: one tool is silent because it checked and found nothing, the other
    // because it never checked.
    if extra > 0 {
        println!(
            "note: {extra} site(s) chiero reports and gcc does not. gcc's front end elides \
             arithmetic whose result is used only as a condition, taking UBSan's check with \
             it, so this is not by itself a false positive."
        );
    }
    // **A number that cannot be asserted to zero can still carry a ceiling** (wave 260).
    //
    // The paragraph above is why `extra == 0` would be wrong: gcc's silence here does not mean it
    // checked and found nothing. But "not assertable" was taken to mean "not assertable at all",
    // and §9 recorded `truncation-not-detected` as a survivor on exactly that basis — the
    // trapping-fault suppression only moves `extra`, and nothing looked at `extra`.
    //
    // It moves it a long way. Deleting the suppression takes the count from **1 to 12**, because a
    // program that dies on SIGFPE has every later site counted against a run gcc never performed.
    // A ceiling catches that and still leaves room for the corpus to drift: one program in
    // fifty-four hits the documented eliding case today, and four would be a real change worth
    // looking at rather than noise.
    // Two mutants on this ceiling survive and both are recognised classes rather than gaps.
    // Forcing `truncated` *always* true suppresses every extra and passes — but `extra` is not a
    // verdict, and the only assertion that would catch it is a *floor*, which would amount to
    // asserting that chiero must keep reporting sites gcc does not. That is backwards. And
    // disabling the assertion itself passes, as no assertion can observe its own removal.
    assert!(
        extra <= 4,
        "chiero reports {extra} sites gcc does not, over {progs} programs. A few are expected — \
         gcc elides arithmetic used only as a condition — but this many means either a real \
         false-positive class or a run being compared against one that did not happen"
    );
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
    /// ASan's fault classes, as (seen, caught by chiero).
    ///
    /// A total is not enough once the corpus can commit more than one kind of fault: 15 of
    /// 15 caught reads as parity while an entire class is missing from the corpus, which is
    /// exactly the state wave 177 left behind.
    classes: std::collections::BTreeMap<&'static str, (usize, usize)>,
    /// Seeds with more execution paths than the program's `malloc` calls explain.
    ///
    /// **This is what makes `invented` mean anything.** ASan executes one path; chiero
    /// explores all of them. "chiero found a fault ASan did not" is evidence of a false
    /// report only while there is no *other path* for the fault to legitimately live on.
    ///
    /// The memory grammar branches nowhere on an unknown value today — every condition it
    /// emits is concrete, so the engine follows one path — and the only forks are
    /// `malloc` succeeding or failing, which the engine models because C says the call can
    /// fail. Measured before it was asserted: worst case 3 states, and every one accounted
    /// for by a `malloc`.
    ///
    /// Nothing *enforces* that, which is why this exists. The ordinary grammar beside it is
    /// full of `if` and `for`, and the moment a `memory_ub` program gains a genuinely
    /// unknown value, a correct finding on an untaken path becomes an accusation against
    /// the engine — and it would read as a chiero regression rather than a corpus one.
    multipath: Vec<(u64, usize, usize)>,
    /// Seeds where ASan flagged a fault and no line could be parsed from its report.
    ///
    /// **Tracked, because "no line" and "the lines agree" are the same silence.** Mutation
    /// proved it: reverting the frame search to `#0` — which cannot find the program's
    /// frame for an interceptor fault like double-free — left the suite green, since a
    /// program with no parsed line simply skipped the comparison.
    unparsed: Vec<u64>,
    /// Seeds where chiero found the right *class* at the wrong *line*.
    ///
    /// 023 §9's whole point: a report that names the fault but not the place is not a
    /// report a person can act on. Nothing checked this until wave 181, so "use-after-free
    /// somewhere in the program" and "use-after-free on line 33" scored the same.
    mislocated: Vec<(u64, u32, Vec<u32>)>,
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
/// Is every finding here the null dereference that follows a failed `malloc`?
///
/// Deliberately narrow: it clears a run only when **all** its memory findings are
/// null-dereferences. A program that genuinely faults *and* has a failure path reports both,
/// and this must not let the genuine one through unexamined.
/// AddressSanitizer's fault classes, and the wording chiero's finding must carry for that
/// class to count as caught.
///
/// The pairing is the whole point: `heap-use-after-free` is answered by a use-after-free
/// finding and nothing else. Without it the oracle grades "did chiero say anything about
/// memory", which every allocating program satisfies before the real fault is examined.
const ASAN_CLASSES: &[(&str, &str)] = &[
    ("heap-use-after-free", "use-after-free"),
    ("attempting double-free", "double-free"),
    ("stack-buffer-overflow", "out-of-bounds"),
    ("global-buffer-overflow", "out-of-bounds"),
    ("heap-buffer-overflow", "out-of-bounds"),
    ("stack-use-after-scope", "use-after-scope"),
];

fn is_malloc_failure_path(r: &chiero_exec::RunResult) -> bool {
    let mem: Vec<String> = r
        .findings()
        .iter()
        .map(|f| format!("{f:?}"))
        .filter(|f| is_memory_finding(f))
        .collect();
    !mem.is_empty() && mem.iter().all(|f| f.contains("null-dereference"))
}

fn is_memory_finding(s: &str) -> bool {
    s.contains("out-of-bounds")
        || s.contains("use-after")
        || s.contains("double-free")
        || s.contains("bad-free")
        || s.contains("null-dereference")
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
            .args([
                "-O0",
                "-g",
                "-fsanitize=address,undefined",
                "-fsanitize-recover=address",
                "-o",
            ])
            .arg(&x)
            .arg(&c)
            .output();
        match compiled {
            Ok(o) if o.status.success() => {}
            // No gcc, or a program it will not build: not this test's subject.
            _ => continue,
        }
        // **`redzone=64`, and this one is not tuning.** ASan detects an overflow by putting
        // a poisoned band after each allocation; an access that jumps *past* that band into
        // another live object is indistinguishable from a valid access to that object, and
        // it stays silent. Measured directly — two `malloc(24)` blocks land 48 bytes apart,
        // so `a[6]` reads `b[0]` and ASan says nothing:
        //
        // ```text
        //   default:      a=0x503000000040 b=0x503000000070 delta=48   a[6]=2
        //   redzone=64:   ERROR: AddressSanitizer: heap-buffer-overflow
        // ```
        //
        // The generator indexes up to three elements past the end, so at eight bytes an
        // element it can reach 32 bytes beyond — past the default band. Without this the
        // oracle scores chiero's *correct* finding as an invention, which is how wave 180
        // found it: the `invented` column exists to catch chiero being wrong and caught the
        // oracle being blind instead.
        //
        // **`detect_leaks=0`.** LeakSanitizer ships inside ASan and reports an un-freed
        // block at exit. A leak is not undefined behaviour — the program's every operation
        // is defined and its result is the one C promises — so chiero says nothing about
        // it, and left on it would score every allocating program as a miss for a fault
        // that is not this oracle's subject.
        let Ok(run) = std::process::Command::new(&x)
            .env("ASAN_OPTIONS", "detect_leaks=0:redzone=64:halt_on_error=0")
            .output()
        else {
            continue;
        };
        let err = String::from_utf8_lossy(&run.stderr).to_string();
        let Some((m, map)) = harness::lower_maybe_with_map(&src) else {
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
        // **How many paths, and are they all explained?** Counted from the *source* rather
        // than from the engine, so the two sides of the comparison are independent; the
        // `- 1` drops the prototype the prelude declares.
        let mallocs = src.matches("malloc(").count().saturating_sub(1);
        if r.states().len() > 1 + mallocs {
            t.multipath.push((seed, r.states().len(), mallocs));
        }
        let found = r
            .findings()
            .iter()
            .any(|f| is_memory_finding(&format!("{f:?}")));
        if !err.contains("AddressSanitizer") {
            // **ASan reports the first fault and stops.** A program it did not flag is one
            // with no *reached* memory fault, so a chiero finding on it is either a false
            // report or a fault on a path the single concrete run did not take — and these
            // programs are closed, with one path. Either way it is worth seeing.
            // **The malloc-failure path is not a false report.** Every allocating program
            // runs with two states: the engine forks on malloc succeeding and failing, and
            // the generated code does not check the result, so the failure path really does
            // dereference NULL. ASan's malloc succeeds, so its single run never goes there.
            //
            // Wave 177 wrote "these programs are closed, with one path" and it was true of
            // that corpus; malloc is what makes it false. So a `null-dereference` here is
            // chiero seeing further than the oracle, and counting it as invented would
            // punish the engine for being right.
            if found && !is_malloc_failure_path(&r) {
                t.invented.push(seed);
            }
            continue;
        }
        t.flagged += 1;
        // **ASan names the class it found; record it.** The classes are matched on ASan's
        // own wording rather than inferred from the source, so a corpus that stops emitting
        // one of them shows up as an empty row instead of as nothing at all.
        // **Matched by class, not by "chiero said something".** Nearly every allocating
        // program carries a `null-dereference` from the malloc-failure path, so a predicate
        // that accepts any memory finding is satisfied before the real fault is looked for
        // — mutation found this by leaving `detect_leaks` on and still passing: a
        // leak-only program was being scored as caught on the strength of that null.
        // **ASan's *first* fault is the one graded, and only it.**
        //
        // `-fsanitize-recover=address` makes ASan report every fault in the run, and the
        // temptation is to require chiero to match them all. That would be wrong. chiero
        // reports the first fault and stops — `report_faults`: "the path ends at a definite
        // crash; everything after it would be about a program that does not exist" — and
        // that is the position C takes. An execution has no defined continuation past
        // undefined behaviour, so ASan's recover mode is *simulating* a program the
        // language does not describe, and its second and later reports belong to that
        // simulation rather than to the program.
        //
        // Recover mode still earns its place: it is what reveals the *order*. Without it
        // ASan names one fault and a chiero that reported some unrelated later fault would
        // be scored a plain miss, indistinguishable from finding nothing. With it, the
        // requirement can be the sharp one — chiero's finding must be ASan's **first**,
        // not merely one of the faults present somewhere in the program.
        let first_class: Option<(&str, &str)> = err
            .lines()
            .filter(|l| l.contains("ERROR: AddressSanitizer:"))
            .find_map(|l| {
                ASAN_CLASSES
                    .iter()
                    .find(|(c, _)| l.contains(*c))
                    .map(|(c, e)| (*c, *e))
            });
        let mut got_all = true;
        match first_class {
            Some((class, expect)) => {
                let hit = r
                    .findings()
                    .iter()
                    .any(|f| format!("{f:?}").contains(expect));
                let e = t.classes.entry(class).or_default();
                e.0 += 1;
                if hit {
                    e.1 += 1;
                } else {
                    got_all = false;
                }
            }
            // ASan flagged something none of the classes name — a class this table does not
            // know yet. Counting it as caught would hide it, so it is a miss.
            None => got_all = false,
        }
        if got_all {
            t.caught += 1;
        } else {
            t.missed.push(seed);
        }
        // **And at the right line.** ASan prints the faulting frame; chiero's finding
        // carries a span, and the `SourceMap` turns it into a line. Only checked when the
        // class matched, since a wrong class at any line is already a miss.
        //
        // **The first frame *inside `probe`*, not frame `#0`.** For a double-free the
        // topmost frame is ASan's own `free` interceptor and the program's frame is `#1`,
        // so keying on `#0` silently failed to parse exactly the classes that go through an
        // interceptor — which is how this was nearly written to grade nothing.
        if got_all {
            let asan_line: Option<u32> = err
                .lines()
                .find(|l| l.trim_start().starts_with('#') && l.contains(" in probe "))
                .and_then(|l| l.rsplit(':').next().and_then(|n| n.trim().parse().ok()));
            if asan_line.is_none() {
                t.unparsed.push(seed);
            }
            if let Some(al) = asan_line {
                let ours: Vec<u32> = r
                    .reports()
                    .iter()
                    .filter(|f| is_memory_finding(&f.message))
                    .filter_map(|f| map.lookup_loc(f.span.lo).map(|l| l.line))
                    .collect();
                if !ours.contains(&al) {
                    t.mislocated.push((seed, al, ours));
                }
            }
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
    for (class, (seen, caught)) in &t.classes {
        println!("  {seen:3} / {caught:<3}  {class}");
    }
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
    // **Every class the grammar claims to emit must actually appear.** Wave 177's corpus
    // committed exactly one kind of fault and reported `15 / 15`, which reads as parity and
    // was really one row of a table with the rest missing.
    for class in [
        "heap-use-after-free",
        "attempting double-free",
        "heap-buffer-overflow",
        "global-buffer-overflow",
        // Asserted too, after wave 179 watched it vanish silently: adding the scope shape
        // at one in four starved every other class, and this one dropped to zero while the
        // test stayed green because nothing named it.
        "stack-buffer-overflow",
        // **The compile line was checked before the grammar**, which is the order section 9
        // recorded — a grammar extension alone would produce programs neither tool flags,
        // and that reads as agreement. It needed no change: gcc 13 catches
        // `stack-use-after-scope` under a plain `-fsanitize=address`, with
        // `-fsanitize-address-use-after-scope` on by default. Measured, not assumed.
        "stack-use-after-scope",
    ] {
        let seen = t.classes.get(class).map(|c| c.0).unwrap_or(0);
        // **Three, not one.** `> 0` says a class exists; it does not say the class is
        // *graded*. `stack-buffer-overflow` sat at 1 of 41 for a wave — one program, one
        // element type, one access spelling — while `global-buffer-overflow` had seven, and
        // the two go through different `chiero-mem` object kinds. A row of one is a class
        // whose next regression is a coin flip away from being invisible.
        assert!(
            seen >= 3,
            "only {seen} generated program(s) commit `{class}`, too few to grade chiero \
             on it. The corpus produced: {:?}",
            t.classes.keys().collect::<Vec<_>>()
        );
    }
    // **Checked before `invented`, because `invented` is only meaningful if this holds.**
    assert!(
        t.multipath.is_empty(),
        "{} program(s) have more paths than their `malloc` calls explain \
         (seed, states, mallocs): {:?}.\n\
         The `invented` column below assumes ASan's single concrete run visits every path \
         chiero does. It no longer does, so a *correct* finding on an untaken path will be \
         reported as a chiero false positive. Decide which before continuing: constrain the \
         grammar back to one path, or downgrade `invented` to a report for multi-path \
         programs. Do not just raise this bound.",
        t.multipath.len(),
        &t.multipath[..t.multipath.len().min(5)]
    );
    assert!(
        t.unparsed.is_empty(),
        "no source line could be parsed from ASan's report on {} program(s): seeds {:?}. \
         The location check skips those, so it would pass while grading nothing",
        t.unparsed.len(),
        &t.unparsed[..t.unparsed.len().min(5)]
    );
    assert!(
        t.mislocated.is_empty(),
        "chiero found the right fault at the wrong line on {} program(s) \
         (seed, ASan line, chiero lines): {:?}",
        t.mislocated.len(),
        &t.mislocated[..t.mislocated.len().min(5)]
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

/// **The generator emits bit-fields and never fills their top bit.**
///
/// Wave 249 found an `unsigned` bit-field being sign-extended on read — a wrong answer that
/// survived a hundred waves of differential testing. This channel is why: bit-fields *are*
/// generated, in about a quarter of programs, and *are* read. What they are never given is a value
/// the defect can be seen through.
///
/// **Two conditions have to coincide and they were counted separately.** An extension defect is
/// visible only when the field is `unsigned` *and* the stored value has the field's top bit set —
/// a signed field is supposed to sign-extend, and a value below the halfway point extends the same
/// way either way. Measured over the fixed seed range:
///
/// ```text
///   bit-field initializers            44
///   with the top bit set              15   <- looks adequate
///   unsigned AND with the top bit set  5   <- the number that matters
/// ```
///
/// Five chances in six hundred seeds, and each of those still has to survive being read, summed and
/// cast to the function's return type. That is why 400 soak seeds run against the wave-249 defect
/// found nothing.
///
/// The first version of this test counted only the top bit and **passed**, which is worth recording:
/// an adequacy guard measuring one of two necessary conditions reports the coverage you hoped for.
/// It is an adequacy guard in the shape §9 records for the memory-UB corpus — a channel that runs
/// and cannot observe its subject is worse than one that does not run, because it reports coverage
/// it does not have.
///
/// Only the parameterless shape is counted, because only there is the initializer a literal this
/// test can read. That undercounts, which is the safe direction for a lower bound.
#[test]
fn the_generator_fills_an_unsigned_bitfield_s_top_bit() {
    let mut top_set = 0usize;
    let mut assigned = 0usize;
    let mut unsigned_n = 0usize;
    // **Three thousand seeds, not six hundred.** Generating a program is string work — the whole
    // scan runs in well under a second — and the six-hundred-seed sample yielded seven unambiguous
    // unsigned bit-field initializers, which is too few for a ratio to mean anything. Mutation said
    // so: removing the top-bit rule entirely still passed, because seven samples can land either
    // way by accident.
    for seed in 1..=3000u64 {
        let (src, _) = program(seed);
        // `  <type> fN:W;` — the declaration, giving each bit-field's width. A `Vec` rather than a
        // map because the workspace disallows `HashMap` for determinism, and a struct has a
        // handful of fields.
        let mut width: Vec<(String, u32)> = Vec::new();
        for l in src.lines() {
            let t = l.trim();
            if let Some(c) = t.find(':')
                && let Some(semi) = t.find(';')
                && semi > c
                && !t.contains('(')
                && let Ok(w) = t[c + 1..semi].parse::<u32>()
                && let Some(name) = t[..c].rsplit(' ').next()
            {
                let uns = t.starts_with("unsigned");
                width.push((name.to_string(), if uns { w } else { w | 0x1000 }));
            } else if let Some(semi) = t.find(';')
                && !t.contains('(')
                && !t.contains(':')
                && let Some(name) = t[..semi].rsplit(' ').next()
                && name.starts_with('f')
            {
                // **A plain member with the same name, which must poison the lookup.** Field names
                // repeat across records — every struct has an `f0` — so a map keyed on the name
                // alone attributes one record's `f0` to another's. Forcing the top bit on and
                // watching this count *not* reach the total is what exposed it: five of fifteen
                // "unsigned bit-fields" were some other struct's ordinary member. An adequacy guard
                // that misattributes is the failure this whole wave is about, one level up.
                width.push((name.to_string(), 0));
            }
        }
        if width.is_empty() {
            continue;
        }
        // `  out.fN = (type)(K);` — the parameterless initializer, whose value is a literal.
        for l in src.lines() {
            let t = l.trim();
            let Some(rest) = t.strip_prefix("out.") else {
                continue;
            };
            let Some((name, val)) = rest.split_once(" = ") else {
                continue;
            };
            // **Mutation records this check as a survivor, and it stays.** Dropping it leaves every
            // assertion passing — it makes the count wrong (ten "unsigned bit-fields" where seven
            // exist) without moving the ratio far enough to fail. A number wrong for a knowable
            // reason is worse than one merely coarse, and this misattribution is what hid an
            // earlier measurement error for most of wave 250.
            //
            // Ambiguous or non-bit-field names are skipped rather than guessed at: more than one
            // entry means two records share the name, and a zero width means it is not a
            // bit-field at all.
            let mut hits = width.iter().filter(|(n, _)| n == name);
            let Some(&(_, w)) = hits.next() else {
                continue;
            };
            if hits.next().is_some() || w & 0xfff == 0 {
                continue;
            }
            let Some(inner) = val.rsplit_once(")(").map(|x| x.1) else {
                continue;
            };
            let Some(k) = inner.trim_end_matches(");").parse::<i64>().ok() else {
                continue;
            };
            let signed_field = w & 0x1000 != 0;
            let w = w & 0xfff;
            assigned += 1;
            // A *signed* field is supposed to sign-extend, so no value of one can expose an
            // extension defect. It counts toward `assigned` and toward nothing else.
            if signed_field {
                continue;
            }
            unsigned_n += 1;
            if (k as u64) & (1u64 << (w - 1)) != 0 {
                top_set += 1;
            }
        }
    }
    assert!(
        assigned > 0 && unsigned_n > 0,
        "the scan must find `unsigned` bit-field initializers at all, or it is measuring nothing: \
         {unsigned_n} of {assigned}"
    );
    // **The denominator is the `unsigned` fields, not all of them.** The RED asserted a quarter of
    // *all* bit-field initializers, which conflates two independent knobs: how often the generator
    // picks an unsigned member for a bit-field (15 of 44, and not this wave's business) and whether
    // the value spans the half of the range where the two extension rules differ (this wave's).
    // Only a signed-vs-unsigned change could move the first, and it would shift the type
    // distribution every other channel depends on. Recorded rather than quietly rescaled.
    // **And the other half must still be reached.** Mutation: forcing the top bit on *every*
    // bit-field passed everything above, which would quietly stop testing the case that has always
    // worked — the same trap as a fix that never extends at all. The comment beside the generator
    // called the remaining third "the control", and until now nothing checked that it existed.
    assert!(
        top_set < unsigned_n,
        "some `unsigned` bit-field initializers must leave the top bit clear, or the half of the \
         range that has always passed stops being tested: {top_set} of {unsigned_n} unsigned"
    );
    assert!(
        top_set * 2 >= unsigned_n,
        "at least half of `unsigned` bit-field initializers must set the field's top bit, or an \
         extension defect is invisible to this channel: {top_set} of {unsigned_n} unsigned \
         ({assigned} bit-fields in all)"
    );
}

/// **A bit-field's value reaches the checksum, which is where a difference can be seen.**
///
/// §9 recorded a hypothesis after wave 250: that a struct with a bit-field only ever appears as a
/// *parameter* of a prelude function, never as a local in the `probe()` body, so its contents never
/// reach the comparison. **Running the count disproves it** — of 3000 seeds, 180 declare such a
/// struct in the body and 156 checksum its fields.
///
/// The guard stays because the routing is what makes the construct observable at all, and nothing
/// else asserts it. A later change to the body grammar could quietly stop declaring struct locals,
/// and every bit-field test in this file would still pass while testing a value nothing reads.
///
/// # The controls are not optional here
///
/// The first version of this scan read `program(seed).0` — the prelude — and reported **zero** for
/// every count, which reads exactly like the hypothesis being confirmed. What exposed it was asking
/// the scan to also count things it obviously should find: any struct local at all, any field in
/// any checksum line. Both were zero too, and a scan that cannot see the ordinary case is not
/// evidence about the rare one. A generated program is two strings and the body is the second.
#[test]
fn a_bitfield_struct_reaches_the_checksum() {
    let (mut declared, mut checksummed, mut any_local, mut any_field) = (0usize, 0usize, 0, 0);
    for seed in 1..=3000u64 {
        let (prelude, body) = program(seed);
        let src = format!("{prelude}\n{body}");
        let mut tag = String::new();
        let mut bf_tags: Vec<String> = Vec::new();
        for l in src.lines() {
            let t = l.trim();
            if let Some(rest) = t.strip_prefix("struct ")
                && t.ends_with('{')
                && let Some(name) = rest.split_whitespace().next()
            {
                tag = name.to_string();
            }
            if let Some(c) = t.find(':')
                && t.ends_with(';')
                && !t.contains('(')
                && !tag.is_empty()
                && t[..c].contains(' ')
                && !bf_tags.contains(&tag)
            {
                bf_tags.push(tag.clone());
            }
        }
        // The controls: the ordinary shapes this scan must be able to see.
        if src.lines().any(|l| {
            let t = l.trim();
            t.starts_with("struct ") && t.contains('=') && t.ends_with(';')
        }) {
            any_local += 1;
        }
        if src
            .lines()
            .any(|l| l.contains("acc = acc * 31") && l.contains('.'))
        {
            any_field += 1;
        }
        if bf_tags.is_empty() {
            continue;
        }
        let locals: Vec<String> = src
            .lines()
            .filter_map(|l| {
                let t = l.trim();
                bf_tags.iter().find_map(|bt| {
                    t.strip_prefix(&format!("struct {bt} "))
                        .filter(|_| t.contains('='))
                        .and_then(|rest| rest.split_whitespace().next())
                        .map(|n| n.trim_end_matches(';').to_string())
                })
            })
            .collect();
        if locals.is_empty() {
            continue;
        }
        declared += 1;
        if src.lines().any(|l| {
            l.contains("acc = acc * 31") && locals.iter().any(|n| l.contains(&format!("({n}.")))
        }) {
            checksummed += 1;
        }
    }
    assert!(
        any_local > 100 && any_field > 100,
        "the scan must see the ordinary shapes before its rare counts mean anything: \
         {any_local} struct locals, {any_field} checksummed fields"
    );
    assert!(
        checksummed >= 50,
        "a struct with a bit-field must reach the checksum, or every bit-field test above is \
         testing a value nothing reads: {checksummed} of {declared} declared"
    );
}

/// **How many of the fixed batch can see an extension defect at all.**
///
/// Wave 251 estimated the answer by multiplying four measured rates and got about one program in
/// two hundred. This counts it directly: **one**, over the exact seed range
/// `generated_programs_agree_with_gcc` uses. Twenty-one over three thousand.
///
/// A program is counted only when all of it lines up — an `unsigned` bit-field, a local of that
/// record initialized by a compound literal, a value whose bit `w-1` is set, and a checksum line
/// reading that field. Anything less cannot distinguish sign- from zero-extension, so it is not
/// coverage of one however much it exercises bit-fields.
///
/// **This is the number to move, and not by adding seeds.** More seeds buys the rate linearly; the
/// four factors multiply, so fixing the weakest buys it in one change. §9 names the two knobs.
///
/// # This number is a floor, not coverage, and wave 252 proved the difference
///
/// Raising it from one to five did **not** make the fixed batch catch wave 249's defect. The
/// controlled experiment — revert `field_signed` in `chiero-lower`, run this batch — still passes,
/// and seed 49 is the counterexample that says why the predicate below is not a predictor:
///
/// ```text
///   struct S0 { float f0; unsigned short f1; unsigned f2:3; };
///   struct S0 v8 = (struct S0){1.0f, 1u, 4};      <- 4 is 0b100: the top bit of a 3-bit field
///   acc = acc * 31 + (long)(v8.f2);               <- and it is read into the checksum
/// ```
///
/// Every condition this test counts is satisfied, and chiero *agrees with gcc anyway*. So the
/// defect is **context-dependent**: `return s.a` triggers it and `(long)(v8.f2)` does not, and the
/// generator reads a bit-field exactly one way — through that cast. A fifth condition belongs in
/// the model, and until someone characterises it this count should be read as "the shape is
/// present", never as "the defect is reachable".
///
/// The assertion is therefore five, which is measured and holds, rather than the ten this test was
/// committed asking for. Lowering a threshold to meet the code is usually the wrong move; here the
/// number was never the goal and the experiment above says so directly.
#[test]
fn the_fixed_batch_can_discriminate_an_extension_defect() {
    let mut discriminating = 0usize;
    for seed in 0..200u64 {
        let (prelude, body) = program(seed);
        let src = format!("{prelude}\n{body}");
        if discriminates(&src) {
            discriminating += 1;
        }
    }
    assert!(
        discriminating >= 5,
        "only {discriminating} of the 200 fixed-batch programs have the shape an extension defect \
         needs; it was one before wave 252, and a channel at that rate is how wave 249's bit-field \
         bug survived a hundred waves"
    );
}

/// Whether one generated program could observe a bit-field extension defect.
///
/// Four things have to coincide, and the scan reports `true` only when they all do in the *same*
/// record and field: an `unsigned` bit-field, a local of that record with a compound-literal
/// initializer, a value whose top bit for that width is set, and a checksum line reading it.
///
/// Compound literals only, because that is the initializer whose per-field values are readable
/// here — a local initialized from a struct-returning helper is real coverage this undercounts. A
/// lower bound is the safe direction for a floor.
fn discriminates(src: &str) -> bool {
    let mut tag = String::new();
    let mut fields: Vec<(String, usize, u32)> = Vec::new();
    let mut nth = 0usize;
    for l in src.lines() {
        let t = l.trim();
        if let Some(rest) = t.strip_prefix("struct ")
            && t.ends_with('{')
            && let Some(n) = rest.split_whitespace().next()
        {
            tag = n.to_string();
            nth = 0;
        } else if t.ends_with(';') && !t.contains('(') && !tag.is_empty() && t.contains(' ') {
            // A member of the record being declared, bit-field or not — the *position* is what a
            // compound literal is indexed by, so plain members have to be counted too.
            if let Some(c) = t.find(':')
                && t.starts_with("unsigned")
                && let Ok(w) = t[c + 1..t.len() - 1].parse::<u32>()
            {
                fields.push((tag.clone(), nth, w));
            }
            nth += 1;
        } else if t == "};" {
            tag.clear();
        }
    }
    fields.iter().any(|(bt, idx, w)| {
        src.lines().any(|l| {
            let t = l.trim();
            let Some(rest) = t.strip_prefix(&format!("struct {bt} ")) else {
                return false;
            };
            let Some((lname, init)) = rest.split_once(" = ") else {
                return false;
            };
            let Some((_, after)) = init.split_once("){") else {
                return false;
            };
            let Some((inner, _)) = after.split_once('}') else {
                return false;
            };
            let Some(v) = inner.split(", ").nth(*idx) else {
                return false;
            };
            let Ok(k) = v.trim_end_matches(['u', 'U', 'l', 'L']).parse::<i64>() else {
                return false;
            };
            (k as u64) & (1u64 << (w - 1)) != 0
                && src.lines().any(|c| {
                    c.contains("acc = acc * 31") && c.contains(&format!("({lname}.f{idx})"))
                })
        })
    })
}

/// **Every read of a struct field is wrapped in a cast, and that is the shape the defect hides in.**
///
/// Wave 253 found the fifth condition wave 251's model was missing: a bit-field read that is an
/// operand of an explicit cast has `top(e)` equal to the field's own type, so a signedness bug in
/// the extension never fires. Read directly, `top(e)` is the promoted `int` and it does.
///
/// The checksum writes `acc = acc * 31 + (long)(x.f);` for every field of every struct in scope, so
/// **every** bit-field the generator can observe is observed through the hiding shape. Waves 250 and
/// 252 raised the frequency of that shape five-fold and caught nothing, which is what this test
/// exists to stop happening again.
///
/// The cast is load-bearing for a `float` or `double` member — `(long)` there is a real conversion,
/// not a formality — so this asks only that *some* integer-typed field is read without one, not that
/// none has a cast.
#[test]
fn the_generator_reads_a_field_without_a_cast() {
    let mut bare = 0usize;
    let mut cast = 0usize;
    for seed in 0..200u64 {
        let (_, body) = program(seed);
        for l in body.lines() {
            let t = l.trim();
            let Some(rest) = t.strip_prefix("acc = acc * 31 + ") else {
                continue;
            };
            if !rest.contains('.') {
                continue;
            }
            if is_cast_read(rest) {
                cast += 1;
            } else {
                bare += 1;
            }
        }
    }
    // **The discriminator is tested on literals, not only through the counts.** Mutation: swapping
    // the two arms survived, because after the fix both counts are large and each assertion is
    // satisfied by either. A count can only tell you a classifier ran, never that it was right.
    assert!(
        is_cast_read("(long)(v8.f2);"),
        "a cast read is the cast form"
    );
    assert!(!is_cast_read("(v8.f2);"), "a bare read is not");
    assert!(
        cast > 0,
        "the scan must see the cast form it is contrasting against, or it is measuring nothing"
    );
    // **Bare reads must outnumber cast ones**, which is the assertion that tells the two counts
    // apart. Mutation: swapping which arm increments which survived every check above, because
    // after the fix both counts are large and each threshold is met by either. Only `float` and
    // `double` members keep a cast and they are a minority of field types — 96 bare against 22 cast
    // — so the direction of the inequality is what carries the information.
    assert!(
        bare > cast,
        "integer fields are read bare and only floats keep a cast, so bare reads must dominate: \
         {bare} bare, {cast} cast"
    );
    assert!(
        bare >= 20,
        "no struct field is read without a cast in the fixed batch ({bare} bare, {cast} cast); \
         a cast operand is exactly the context wave 253 showed a bit-field extension defect hides \
         in, so the channel cannot observe one however often it emits the construct"
    );
}

/// `(long)(x.f)` versus `(x.f)`: the tell is the `)(` between a cast and its operand.
///
/// A free function so the classification can be asserted on literals. The first version of the
/// caller asked whether the text began with `(` followed by a letter, which is true of *both*
/// forms — caught by a control that reported zero casts in a batch made entirely of them.
fn is_cast_read(rest: &str) -> bool {
    rest.contains(")(")
}

#[test]
fn probe_arr() {
    let (mut with_arr, mut with_szof) = (0usize, 0usize);
    for seed in 0..300u64 {
        let (p, b) = program_control_flow(seed);
        let all = format!("{p}{b}");
        if all.contains("sizeof(__typeof__")
            || all.contains("sizeof(typeof")
            || all.contains("sizeof(__typeof(")
        {
            with_szof += 1;
        }
        for l in all.lines() {
            let t = l.trim();
            if t.contains("] = {")
                && !t.contains("][")
                && t.starts_with(|c: char| c.is_alphabetic())
            {
                with_arr += 1;
                break;
            }
        }
    }
    eprintln!("ARR: local 1-D arrays in {with_arr}/300, sizeof(typeof(arr)) in {with_szof}/300");
}

/// The first backward `goto` in `body`, or `Ok(())`.
///
/// **Extracted so it can be tested directly** (wave 286). Inline in the channel it sat behind
/// half a dozen other assertions, every one of which fires first on a corpus that jumps
/// backward — so nothing could show that this check itself still worked.
///
/// **Scoped, because a GNU local label deliberately reuses one name.** Two blocks in one
/// function each declaring `Ldone` is the shape the construct exists for, and a name-keyed
/// search of the whole body is wrong twice over: it finds an earlier block's label and calls a
/// forward jump backward, and searching forward instead would find a *later* block's label and
/// call a backward jump **forward**. The second is the dangerous direction and this is a safety
/// property, so the search is bounded to the jump's own block.
///
/// A local-label block opens with `{ __label__ NAME;`; without one the label is unique and the
/// whole body is the right scope.
fn backward_goto(body: &str) -> Result<(), String> {
    for (i, _) in body.match_indices("goto L") {
        let label = body[i + 5..]
            .split(';')
            .next()
            .expect("the arm emits a terminated statement")
            .trim()
            .to_string();
        let scope_end = body[..i]
            .rfind(&format!("{{ __label__ {label};"))
            .map(|open| {
                let mut depth = 0i32;
                for (k, c) in body[open..].char_indices() {
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                return open + k;
                            }
                        }
                        _ => {}
                    }
                }
                body.len()
            })
            .unwrap_or(body.len());
        match body[i..scope_end].find(&format!("{label}: ;")) {
            Some(0) => return Err(format!("`goto {label}` targets itself")),
            Some(_) => {}
            None => {
                return Err(format!(
                    "`goto {label}` has no label after it in its own block"
                ));
            }
        }
    }
    Ok(())
}

/// **The backward-`goto` guard catches what it names.**
///
/// The channel applies it to every generated program, where it is unreachable behind earlier
/// assertions — every one of them fires first on a corpus that jumps backward. These are the
/// three shapes that matter, written out.
#[test]
fn the_backward_goto_guard_is_effective() {
    // Forward, unique label.
    assert!(backward_goto("  if (x) goto L1;\n  y += 1;\n  L1: ;\n").is_ok());
    // Forward, with the *same* name declared in two blocks — the local-label shape. A search
    // keyed on the name alone finds the first block's label and calls this backward.
    let two = "  { __label__ Ldone;\n  if (x) goto Ldone;\n  Ldone: ;\n  }\n\
               { __label__ Ldone;\n  if (y) goto Ldone;\n  Ldone: ;\n  }\n";
    assert!(backward_goto(two).is_ok(), "two blocks, both forward");
    // **Backward, with a later block defining the same name.** A search that merely went forward
    // from the jump would find the *second* block's label and call this forward — the direction
    // that matters, because a backward jump can loop forever.
    let back = "  { __label__ Ldone;\n  Ldone: ;\n  if (x) goto Ldone;\n  }\n\
                { __label__ Ldone;\n  if (y) goto Ldone;\n  Ldone: ;\n  }\n";
    assert!(
        backward_goto(back).is_err(),
        "a backward jump inside a local-label block must be caught even though a later block \
         defines the same name"
    );
    // Backward with a unique label, which has no later definition at all.
    assert!(backward_goto("  L1: ;\n  if (x) goto L1;\n").is_err());
}
