//! **Generated record layouts, graded by gcc.** Covers: 014 contracts 1–8, mechanically.
//!
//! `layout.rs` is seventeen hand-written records and `vpp_layout_gate.rs` is twenty-two VPP
//! headers. Between them they put ten thousand assertions to gcc, and they have found real
//! defects — but both corpora have the same shape of edge, from opposite directions:
//!
//! - the hand fixtures cover what somebody thought to spell. §8.3's yield table records
//!   three layout defects, and every one was found by *widening a corpus*, not by a fixture:
//!   unnamed bit-fields (a gate green for months because no seed had one), a `proven` wrong
//!   answer found inside the fix for the first, and an enum's declared underlying type that
//!   made `layout` wrong on 22 VPP sites while saying `proven`.
//! - the VPP gate covers what VPP happens to write. It cannot contain a construct VPP does
//!   not use, and §7.9's `:0` bit-field is exactly that: a sweep of 69 VPP headers found
//!   none, so the gate is structurally incapable of seeing it however green it goes.
//!
//! This file enumerates the shapes instead of spelling them: member types × array-ness ×
//! bit-field widths × nesting × `packed` × `aligned(n)` × struct/union, explored by seed.
//! The grammar is the thing to audit against C11 6.7.2.1 and 014; everything else falls out.
//!
//! # It reuses the existing oracle rather than writing a second one
//!
//! [`harness::assert_agrees_with_gcc`] already compiles a probe that checks `sizeof`,
//! `_Alignof`, every non-bit-field `__builtin_offsetof`, **and** bit placement at run time
//! by writing all-ones into each bit-field and dumping the bytes. A second oracle would be
//! a second thing to be wrong; this file is a generator and nothing else.
//!
//! # A refusal is counted, never skipped
//!
//! chiero rejecting a generated record is not a pass. 014 §7 and 015 §7 both make a gap a
//! diagnostic rather than a licence, so the run reports how many records it refused and
//! fails if any did — the same rule `generated.rs` arrived at for the value oracle, where a
//! refusal sharing a bucket with a discard is what let `x && <float>` hide for six waves.

mod harness;

use chiero_sema::TargetConfig;
use harness::{Parsed, gcc_available, parse_allowing_diagnostics};

// ---------------------------------------------------------------------------------------
// A PRNG, written out rather than depended on
// ---------------------------------------------------------------------------------------

/// xorshift64*, the same twenty lines `chiero-lower`'s generator carries. 001 §4's
/// dependency rules are checked by `xtask check-deps`, and needing nothing keeps that quiet.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
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

    /// True one time in `n`.
    fn chance(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }
}

// ---------------------------------------------------------------------------------------
// The grammar
// ---------------------------------------------------------------------------------------

/// A scalar member type, with the bit width C gives a bit-field of it.
///
/// **`_Bool` is here deliberately.** Its layout is the one place `size_of * 8` and "how many
/// bits a bit-field of this type may have" disagree — `sizeof(_Bool)` is 1 and a `_Bool:1` is
/// legal — and 014 has a note saying so. A generator that omitted it would leave the only
/// asymmetric case to the hand fixtures.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Scalar {
    c: &'static str,
    /// Width in bits, which bounds a bit-field declared with this type.
    bits: u32,
    /// Whether a bit-field may be declared with it. C11 6.7.2.1p5 allows `_Bool`, `signed
    /// int` and `unsigned int`; gcc permits any integer type, and 013 makes chiero gnu11.
    bitfieldable: bool,
}

const SCALARS: &[Scalar] = &[
    Scalar {
        c: "char",
        bits: 8,
        bitfieldable: true,
    },
    Scalar {
        c: "signed char",
        bits: 8,
        bitfieldable: true,
    },
    Scalar {
        c: "unsigned char",
        bits: 8,
        bitfieldable: true,
    },
    Scalar {
        c: "_Bool",
        bits: 1,
        bitfieldable: true,
    },
    Scalar {
        c: "short",
        bits: 16,
        bitfieldable: true,
    },
    Scalar {
        c: "unsigned short",
        bits: 16,
        bitfieldable: true,
    },
    Scalar {
        c: "int",
        bits: 32,
        bitfieldable: true,
    },
    Scalar {
        c: "unsigned int",
        bits: 32,
        bitfieldable: true,
    },
    Scalar {
        c: "long",
        bits: 64,
        bitfieldable: true,
    },
    Scalar {
        c: "unsigned long",
        bits: 64,
        bitfieldable: true,
    },
    Scalar {
        c: "long long",
        bits: 64,
        bitfieldable: true,
    },
    Scalar {
        c: "float",
        bits: 32,
        bitfieldable: false,
    },
    Scalar {
        c: "double",
        bits: 64,
        bitfieldable: false,
    },
    // A pointer, which is where an alignment mistake costs the most on a 64-bit target.
    Scalar {
        c: "void *",
        bits: 64,
        bitfieldable: false,
    },
    // **The only members that force 16-byte alignment**, and nothing else in this project
    // reaches one: neither `layout.rs`'s hand fixtures nor the 22-seed VPP gate has a member
    // whose natural alignment exceeds 8. Every padding number the corpora have ever checked
    // was computed at 1, 2, 4 or 8, so a rule that happens to be written `min(align, 8)`
    // somewhere would have gone on passing forever.
    Scalar {
        c: "long double",
        bits: 128,
        bitfieldable: false,
    },
    // ⚠️ **`__int128` is deliberately absent, and the omission is a cost, not a tidy-up.**
    // chiero diagnoses it — *"ISO C does not support `__int128` types"* — which is correct
    // pedantry under 013 and matches `gcc -pedantic`. But this gate treats any diagnostic as
    // a refusal, and `SemaDiagnostic` carries no severity, so there is nothing to filter on:
    // 57 of 120 seeds refused and the gate stopped measuring layout at all. What is lost is
    // the 128-bit *bit-field allocation unit*, which nothing now reaches. `long double` above
    // still supplies the 16-byte alignment this widening was for.
];

/// Where a record's attributes were written.
///
/// The three are **not** interchangeable, and that is the whole point of tracking it: gcc and
/// clang apply `Middle` and `Postfix` to the record and ignore `Prefix` entirely. A generator
/// that emits only one of them tests one third of the rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AttrPos {
    /// No attribute on this record.
    None,
    /// `__attribute__((packed)) struct S {…};` — **ignored** by both compilers.
    Prefix,
    /// `struct __attribute__((packed)) S {…};` — applied.
    Middle,
    /// `struct S {…} __attribute__((packed));` — applied.
    Postfix,
}

/// One generated translation unit: the records to define, in dependency order, and the tags
/// worth putting to the oracle.
struct Unit {
    src: String,
    /// `(tag, is_union)` for every record defined, outermost last.
    tags: Vec<(String, bool)>,
    /// Where each record's attributes were written, parallel to `tags`.
    positions: Vec<AttrPos>,
}

struct Gen {
    rng: Rng,
    out: String,
    tags: Vec<(String, bool)>,
    positions: Vec<AttrPos>,
    next_id: usize,
}

impl Gen {
    fn new(seed: u64) -> Gen {
        Gen {
            rng: Rng::new(seed),
            out: String::new(),
            tags: Vec::new(),
            positions: Vec::new(),
            next_id: 0,
        }
    }

    fn fresh(&mut self) -> String {
        self.next_id += 1;
        format!("R{}", self.next_id)
    }

    /// Emit one record and return its tag.
    ///
    /// `depth` bounds nesting. A nested record is emitted **first**, as its own top-level
    /// definition, so the source is always in dependency order and the oracle can be pointed
    /// at the inner record as well as the outer one.
    fn record(&mut self, depth: usize) -> String {
        let tag = self.fresh();
        let is_union = self.rng.chance(5);
        let kw = if is_union { "union" } else { "struct" };

        // **The two attributes with opposite effects, and they compose.** `packed` removes
        // internal padding and drops member alignment to 1; `aligned(n)` raises the record's
        // own alignment. VPP uses `packed` 112 times on wire formats, where a wrong offset
        // means every parsed field is wrong.
        // ⚠️ **All three positions, because two of them are where the attribute does
        // anything.** The first version of this generator emitted only the prefix — and the
        // defect it found was that chiero *honoured* the prefix, which gcc and clang ignore.
        // The moment that was fixed, every `packed` and `aligned` this generator wrote became
        // inert: chiero ignored it, gcc ignored it, and the two agreed about nothing at all.
        // The gate would have gone on reporting 241 agreements with its whole attribute
        // dimension vacuous, and the reach test would have gone on counting `packed >= 40`
        // while none of them packed anything. **A fix can blind the corpus that found it** —
        // the same shape as a probe that reports zero because it cannot report non-zero.
        //
        // So a position is chosen per record: prefix keeps the *ignored* case under test,
        // which is the regression the fix installed; middle and postfix are where the
        // attribute reaches the record and the arithmetic is exercised.
        let mut attr_list = String::new();
        let packed = self.rng.chance(3);
        if packed {
            attr_list.push_str("__attribute__((packed)) ");
        }
        if self.rng.chance(4) {
            let n = *self.rng.pick(&[2u32, 4, 8, 16, 32]);
            attr_list.push_str(&format!("__attribute__((aligned({n}))) "));
        }
        let position = if attr_list.is_empty() {
            AttrPos::None
        } else {
            *self
                .rng
                .pick(&[AttrPos::Prefix, AttrPos::Middle, AttrPos::Postfix])
        };
        let (prefix, middle, postfix) = match position {
            AttrPos::None => (String::new(), String::new(), String::new()),
            AttrPos::Prefix => (attr_list.clone(), String::new(), String::new()),
            AttrPos::Middle => (String::new(), attr_list.clone(), String::new()),
            // A postfix attribute goes after the closing brace, before the `;`.
            AttrPos::Postfix => (
                String::new(),
                String::new(),
                format!(" {}", attr_list.trim_end()),
            ),
        };
        self.positions.push(position);

        let n_members = 1 + self.rng.below(5);
        let mut members = String::new();
        // **At least one named member, by construction.** C11 6.7.2.1p8 makes a record whose
        // members are all unnamed *undefined*, gcc warns under `-Wpedantic` and chiero
        // diagnoses it — correctly. Seven of the first run's records were that shape, and
        // they were chiero being right, not a gap. The generated value corpus avoids the UB
        // it can avoid by construction and discards the rest; this is one it can avoid.
        let mut named = 0usize;
        // A run of bit-fields is where the interesting arithmetic is, so track whether the
        // previous member was one — a `:0` is only meaningful after a nonzero-width run.
        let mut prev_was_bitfield = false;

        for i in 0..n_members {
            let name = format!("m{i}");
            match self.rng.below(10) {
                // A nested record, by value. Bounded by `depth`, and never inside a union
                // member count that would make the oracle's probe enormous.
                0..=1 if depth > 0 => {
                    let inner = self.record(depth - 1);
                    let inner_kw = self
                        .tags
                        .iter()
                        .find(|(t, _)| *t == inner)
                        .map(|(_, u)| if *u { "union" } else { "struct" })
                        .unwrap_or("struct");
                    members.push_str(&format!("  {inner_kw} {inner} {name};\n"));
                    named += 1;
                    prev_was_bitfield = false;
                }
                // An array. The element count stays small so `sizeof` stays printable.
                2 => {
                    let s = *self.rng.pick(SCALARS);
                    let n = 1 + self.rng.below(4);
                    members.push_str(&format!("  {} {name}[{n}];\n", s.c));
                    named += 1;
                    prev_was_bitfield = false;
                }
                // A bit-field.
                3..=6 => {
                    let candidates: Vec<&Scalar> =
                        SCALARS.iter().filter(|s| s.bitfieldable).collect();
                    let s = **self.rng.pick(&candidates);
                    // **Zero width, unnamed, or ordinary — the three cases 014 keeps apart.**
                    // A `:0` after a run forces the next member to a fresh allocation unit
                    // (contract 4) and an unnamed nonzero one occupies bits while declaring
                    // no member (the case a gate stayed green over for months).
                    if prev_was_bitfield && self.rng.chance(4) {
                        // `_Bool:0` is legal but says little; use an integer type so the
                        // allocation unit being flushed is a wide one.
                        members.push_str("  int :0;\n");
                        prev_was_bitfield = false;
                    } else {
                        let width = 1 + self.rng.below(s.bits as usize);
                        if self.rng.chance(4) {
                            members.push_str(&format!("  {} :{width};\n", s.c));
                        } else {
                            members.push_str(&format!("  {} {name}:{width};\n", s.c));
                            named += 1;
                        }
                        prev_was_bitfield = true;
                    }
                }
                // A plain scalar, sometimes with its own `aligned`. **`_Alignas` on a member
                // is the pairing `generated.rs` records as reached at too low a rate to
                // discriminate**; here it is cheap, because a record is the whole program.
                _ => {
                    let s = *self.rng.pick(SCALARS);
                    // ⚠️ `aligned` on a member of a `packed` record is where the two
                    // attributes fight, and gcc's answer is the one that decides it. Emitted
                    // deliberately rather than avoided.
                    if self.rng.chance(6) {
                        let n = *self.rng.pick(&[2u32, 4, 8, 16]);
                        members.push_str(&format!(
                            "  {} __attribute__((aligned({n}))) {name};\n",
                            s.c
                        ));
                    } else {
                        members.push_str(&format!("  {} {name};\n", s.c));
                    }
                    named += 1;
                    prev_was_bitfield = false;
                }
            }
        }

        if named == 0 {
            members.push_str("  int named;\n");
        }
        self.out.push_str(&format!(
            "{prefix}{kw} {middle}{tag} {{\n{members}}}{postfix};\n"
        ));
        self.tags.push((tag.clone(), is_union));
        tag
    }
}

fn unit(seed: u64) -> Unit {
    let mut g = Gen::new(seed);
    g.record(2);
    Unit {
        src: g.out,
        tags: g.tags,
        positions: g.positions,
    }
}

// ---------------------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------------------

/// What one generated record established.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// chiero laid it out and gcc agreed. The only row that is evidence.
    Agrees,
    /// A diagnostic. Not a wrong answer, and never a pass either — 014 §7's rule.
    Refused(String),
    /// chiero laid out no record for a tag it parsed. Its own class: silence is the outcome
    /// a layout gate is least able to notice, because there is no number to compare.
    NoLayout,
    /// **The finding.** chiero laid it out and *both* compilers say the numbers are wrong.
    Disagrees(String),
    /// gcc contradicts chiero and clang does not. **Not a chiero defect** — the two
    /// compilers disagree with each other and chiero took one side.
    ///
    /// **One cause, and the first statement of it here was too narrow.** It was written as
    /// *"`aligned(N)` on a member whose natural alignment exceeds N"* from three rows that
    /// all had that shape; the next run had five, and two of them did not. Read the last one.
    ///
    /// The rule, measured rather than inferred: **gcc lets an alignment-*lowering* context
    /// override a member's explicit `aligned`, and clang never does.** chiero is on clang's
    /// side throughout.
    ///
    /// | | gcc | clang & chiero |
    /// |---|---|---|
    /// | `{ void * __attribute__((aligned(8))) m; } __attribute__((packed));` | 8/**1** | 8/**8** |
    /// | `{ void * __attribute__((aligned(4))) m; };` — no `packed` | 8/**4** | 8/**8** |
    /// | `{ void * __attribute__((aligned(16))) m; } __attribute__((packed));` | 8/**1** | **16/16** |
    ///
    /// ⚠️ **gcc contradicts its own manual in both directions here** — it documents that
    /// "the aligned attribute can only increase the alignment; to decrease it you need packed
    /// as well", yet row two *decreases* without `packed` and row three refuses an *increase*
    /// because of it. chiero implements what is written down. Worth knowing before anyone
    /// "corrects" chiero against a quick `gcc` experiment. Reported as its own row,
    /// never merged into `Agrees`, because a gate that quietly counted it as agreement would
    /// be lowering its own standard; and never into `Disagrees`, because that would be the
    /// gate being wrong about chiero.
    MatchedOne { gcc_says: String },
}

fn judge(u: &Unit) -> Vec<(String, Outcome)> {
    let p: Parsed = parse_allowing_diagnostics(&u.src, TargetConfig::x86_64_linux());
    if let Some(d) = p.analysis.diagnostics.first() {
        return vec![(
            u.tags.last().map(|(t, _)| t.clone()).unwrap_or_default(),
            Outcome::Refused(format!("{d:?}")),
        )];
    }
    let mut out = Vec::new();
    for (tag, _) in &u.tags {
        let Some(sym) = p.symbol(tag) else {
            out.push((tag.clone(), Outcome::NoLayout));
            continue;
        };
        let Some(rid) = p.analysis.record_by_tag(sym) else {
            out.push((tag.clone(), Outcome::NoLayout));
            continue;
        };
        // **The oracle panics on disagreement, and the panic is caught** — not to soften it
        // but because §8.3 step 3 makes the *first run's whole failure list* the measurement,
        // and a gate that stops at the first record cannot produce one. The message carries
        // chiero's numbers and gcc's rejection verbatim, so nothing is lost by re-raising it
        // as a row.
        let l = p.analysis.layout(rid).clone();
        let src = u.src.clone();
        let tag_owned = tag.clone();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            harness::assert_agrees_with_gcc(&src, &tag_owned, &l, &p)
        }));
        match caught {
            Ok(()) => out.push((tag.clone(), Outcome::Agrees)),
            Err(e) => {
                let msg = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_string()))
                    .unwrap_or_else(|| "non-string panic".into());
                // **Ask the second compiler before calling it a defect.** gcc and clang do
                // not always agree about layout: `__attribute__((aligned(4)))` on a `void *`
                // member *lowers* the alignment for gcc (12/4) and does not for clang (16/8).
                // chiero matches clang there, and a gcc-only gate would have filed it as a
                // wrong answer.
                let src2 = u.src.clone();
                let tag2 = tag.clone();
                let l2 = l.clone();
                let clang = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    harness::assert_agrees_with_cc("clang", &src2, &tag2, &l2, &p)
                }));
                if clang.is_ok() {
                    // Print the record, not just the tag. Three rows sharing a verdict is
                    // not three rows sharing a cause (§7.6), and the only way to tell is to
                    // read them.
                    eprintln!("  --- matched-one source for {tag} ---\n{}", u.src);
                    out.push((tag.clone(), Outcome::MatchedOne { gcc_says: msg }));
                } else {
                    out.push((tag.clone(), Outcome::Disagrees(msg)));
                }
            }
        }
    }
    out
}

/// **Fixed seeds, so this is a test and not a slot machine**, and `#[ignore]`d because every
/// record costs a gcc compile *and a run*.
///
/// The count is what to watch. A grammar change that made most records refuse would leave
/// every assertion here passing while the file compared almost nothing, which is the failure
/// `generated.rs` guards with a floor and this guards the same way.
#[test]
#[ignore = "external oracle — one gcc compile and run per generated record"]
fn generated_record_layouts_agree_with_gcc() {
    if !gcc_available() {
        panic!("014 §7 needs the compiler; an oracle that can silently not run is not one");
    }
    let mut agreed = 0usize;
    let mut no_layout: Vec<String> = Vec::new();
    let mut refused: Vec<(u64, String)> = Vec::new();
    let mut disagree: Vec<(u64, String, String)> = Vec::new();
    let mut matched_one: Vec<(u64, String)> = Vec::new();
    let mut records = 0usize;

    for seed in 0..120u64 {
        let u = unit(seed);
        for (tag, outcome) in judge(&u) {
            records += 1;
            match outcome {
                Outcome::Agrees => agreed += 1,
                Outcome::NoLayout => no_layout.push(format!("seed {seed}: {tag}\n{}", u.src)),
                Outcome::Refused(d) => refused.push((seed, format!("{d}\n{}", u.src))),
                Outcome::Disagrees(why) => disagree.push((seed, tag, why)),
                Outcome::MatchedOne { .. } => matched_one.push((seed, tag)),
            }
        }
    }

    eprintln!(
        "generated layouts: {records} records over 120 seeds, {agreed} agree with gcc, \
         {} DISAGREE, {} matched clang where gcc differs, {} refused, {} without a layout",
        disagree.len(),
        matched_one.len(),
        refused.len(),
        no_layout.len()
    );
    for (seed, tag, _) in &disagree {
        eprintln!("  DISAGREES   seed {seed} {tag}");
    }
    for (seed, tag) in &matched_one {
        eprintln!("  MATCHED ONE seed {seed} {tag} — gcc differs, clang agrees with chiero");
    }

    assert!(
        no_layout.is_empty(),
        "{} record(s) parsed clean and got no layout. Silence is the outcome a layout gate \
         is least able to notice — there is no number to disagree with:\n{}",
        no_layout.len(),
        no_layout.join("\n---\n")
    );
    assert!(
        refused.is_empty(),
        "{} generated record(s) were refused. A refusal is 014 §7's honest outcome and never \
         a pass; either the construct is legal C and chiero should lay it out, or the \
         generator emits something it should not:\n{:#?}",
        refused.len(),
        refused
    );
    assert!(
        disagree.is_empty(),
        "{} generated record(s) got a layout gcc contradicts. First:\n{}",
        disagree.len(),
        disagree[0].2
    );
    // **A floor, not `> 0`.** One lucky record is green under `> 0`, and the whole point is
    // volume across shapes.
    assert!(
        agreed >= 150,
        "only {agreed} records agreed; a channel that grades almost nothing is green while \
         testing almost nothing"
    );
}

/// **The generator reaches the shapes the two existing corpora cannot.**
///
/// Presence is not discrimination — the value generator's own header is emphatic about that
/// — so this is not the justification for the file. It is the guard against the justification
/// quietly ceasing to hold: `:0` is the construct a sweep of 69 VPP headers could not find,
/// and an unnamed bit-field is the one a gate stayed green over for months. If a grammar
/// tweak stops emitting them, the gate above goes on passing and means less, exactly the way
/// a stale ledger entry does.
#[test]
fn the_generator_reaches_what_the_vpp_and_hand_corpora_cannot() {
    let (mut zero_width, mut unnamed, mut packed, mut aligned_member, mut nested, mut unions) =
        (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let mut bitfield_after_zero = 0usize;
    let (mut prefix, mut middle, mut postfix) = (0usize, 0usize, 0usize);
    let mut wide_align = 0usize;

    for seed in 0..400u64 {
        let u = unit(seed);
        let mut seen_zero = false;
        for line in u.src.lines() {
            let t = line.trim();
            if t == "int :0;" {
                zero_width += 1;
                seen_zero = true;
                continue;
            }
            // An unnamed *nonzero* bit-field: `type :N;` with N > 0.
            if t.contains(" :") && t.ends_with(';') && !t.contains(":0;") {
                unnamed += 1;
            }
            if t.contains(':') && !t.contains(" :") && seen_zero {
                bitfield_after_zero += 1;
            }
        }
        if u.src.contains("long double") || u.src.contains("__int128") {
            wide_align += 1;
        }
        if u.src.contains("__attribute__((packed))") {
            packed += 1;
        }
        if u.src.contains("__attribute__((aligned(") {
            aligned_member += 1;
        }
        for pos in &u.positions {
            match pos {
                AttrPos::Prefix => prefix += 1,
                AttrPos::Middle => middle += 1,
                AttrPos::Postfix => postfix += 1,
                AttrPos::None => {}
            }
        }
        if u.tags.len() > 1 {
            nested += 1;
        }
        if u.tags.iter().any(|(_, is_union)| *is_union) {
            unions += 1;
        }
    }

    eprintln!(
        "reach over 400 seeds: {zero_width} `:0`, {unnamed} unnamed bit-fields, \
         {bitfield_after_zero} members after a `:0`, {packed} packed, {aligned_member} aligned, \
         {nested} nested, {unions} with a union"
    );
    assert!(zero_width >= 10, "`:0` bit-fields: {zero_width}");
    assert!(unnamed >= 10, "unnamed bit-fields: {unnamed}");
    assert!(
        bitfield_after_zero >= 5,
        "a `:0` only means something if a member follows it: {bitfield_after_zero}"
    );
    assert!(packed >= 40, "packed records: {packed}");
    // The only members whose natural alignment is 16. No other corpus in the project has one,
    // so every padding number ever checked here was computed at 8 or below.
    assert!(
        wide_align >= 40,
        "records with a 16-byte-aligned member: {wide_align}"
    );
    // ⚠️ **The counts that matter are the two positions where an attribute does something.**
    // `packed >= 40` above went on passing after the prefix fix while every one of those
    // attributes had become inert — chiero ignored it, gcc ignored it, and the gate scored an
    // agreement about nothing. Counting a construct is not counting a *test* of it.
    eprintln!(
        "attribute positions: {prefix} prefix (ignored), {middle} middle, {postfix} postfix; \
         {wide_align} records with a 16-byte-aligned member"
    );
    assert!(
        middle >= 30,
        "`struct __attribute__((packed)) S {{…}}` — the position that applies: {middle}"
    );
    assert!(
        postfix >= 30,
        "`struct S {{…}} __attribute__((packed));` — the other position that applies: {postfix}"
    );
    assert!(
        prefix >= 30,
        "the ignored position must stay under test too — it is the regression the fix \
         installed: {prefix}"
    );
    assert!(aligned_member >= 40, "aligned attributes: {aligned_member}");
    assert!(nested >= 40, "nested records: {nested}");
    assert!(unions >= 20, "unions: {unions}");
}
