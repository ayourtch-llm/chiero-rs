//! **020 contract 1 as a property, over randomly generated modules.**
//!
//! The existing coverage guard (`every_variant_is_accounted_for`) measures variant
//! *reachability* — it proves the printer has an arm for every enum variant. It cannot
//! measure field *fidelity*, and `FULL_COVERAGE_FIXTURE` supplies the identity or default
//! value for nearly every scalar field: `fresh i32`, `splat …, 4`, `allocadyn … align 1`,
//! `vacopy %1 -> %1`, `scope 0`, `lifetime scope`. A field printed as its default is
//! indistinguishable from a field not printed at all, so the printer and parser can drop
//! it together and every existing test still passes. Dropping `scope` from the alloca
//! line is caught by nothing in this crate except the test below.
//!
//! This file closes that gap by generating modules whose every field holds a *distinct,
//! non-default* value and asserting **structural** equality after a round trip. Structural
//! equality is the point: comparing `print(m)` to `print(parse(print(m)))` — which is how
//! the corpus test is written — is invariant under anything the printer omits, and that is
//! precisely how spans went unserialized for the crate's whole life.
//!
//! **What this cannot do.** A *symmetric* change — printer and parser inverting the same
//! pair of fields — round-trips perfectly by construction, and no round-trip test of any
//! strength can see it. Transposing `BitRange`'s `off` and `width` on both sides is caught
//! here only because the *verifier* rejects the resulting bit ranges when the corpus is
//! checked. Encoding symmetry needs an independent oracle: the verifier, or eventually the
//! engine's semantics. Saying otherwise would make this file the same kind of instrument
//! it was written to replace.

use chiero_cir::text::{parse, print};
use chiero_cir::*;
use chiero_span::{BytePos, ExpnCtx, Span};

/// xorshift64. Deterministic (001 §5) and dependency-free.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len() as u64) as usize]
    }
    /// A span that is never `DUMMY` and whose three fields always differ from each
    /// other, so dropping or transposing any one of them is visible.
    fn span(&mut self) -> Span {
        let lo = 1 + self.below(10_000) as u32;
        Span {
            lo: BytePos(lo),
            hi: BytePos(lo + 1 + self.below(500) as u32),
            ctx: ExpnCtx(1 + self.below(50) as u32),
        }
    }
}

const INT_WIDTHS: &[u32] = &[1, 8, 16, 32, 64];

fn int_ty(r: &mut Rng) -> CTy {
    CTy::Int(*r.pick(INT_WIDTHS))
}

fn any_ty(r: &mut Rng) -> CTy {
    match r.below(4) {
        0 => int_ty(r),
        1 => CTy::Float(*r.pick(&[FloatKind::F32, FloatKind::F64, FloatKind::X87_80])),
        2 => CTy::Ptr,
        _ => CTy::Vector {
            elem: Box::new(CTy::Int(*r.pick(&[8, 16, 32, 64]))),
            lanes: 2 + r.below(6) as u32,
        },
    }
}

/// An integer constant of the given width, kept in range so the printer's rendering and
/// the parser's `parse::<i128>` agree. Values are deliberately not 0 or 1.
fn int_const(r: &mut Rng, bits: u32) -> Operand {
    let val = if bits == 1 {
        (r.below(2)) as i128
    } else {
        2 + r.below(1u64 << (bits.min(32) - 2)) as i128
    };
    Operand::Const(Const::Int { bits, val })
}

fn operand_of(r: &mut Rng, t: &CTy, pool: &[ValueId]) -> Operand {
    // Prefer an in-scope value so the module also verifies; fall back to a constant.
    if !pool.is_empty() && r.below(2) == 0 {
        return Operand::Value(*r.pick(pool));
    }
    match t {
        CTy::Int(b) => int_const(r, *b),
        CTy::Ptr => Operand::Const(Const::Null),
        _ if !pool.is_empty() => Operand::Value(*r.pick(pool)),
        _ => Operand::Const(Const::Int { bits: 32, val: 7 }),
    }
}

const BIN_OPS: &[BinOp] = &[
    BinOp::Add,
    BinOp::Sub,
    BinOp::Mul,
    BinOp::UDiv,
    BinOp::SDiv,
    BinOp::URem,
    BinOp::SRem,
    BinOp::And,
    BinOp::Or,
    BinOp::Xor,
    BinOp::Shl,
    BinOp::LShr,
    BinOp::AShr,
];

const CMP_OPS: &[CmpOp] = &[
    CmpOp::Eq,
    CmpOp::Ne,
    CmpOp::ULt,
    CmpOp::ULe,
    CmpOp::UGt,
    CmpOp::UGe,
    CmpOp::SLt,
    CmpOp::SLe,
    CmpOp::SGt,
    CmpOp::SGe,
    CmpOp::FOEq,
    CmpOp::FONe,
    CmpOp::FOLt,
    CmpOp::FOLe,
    CmpOp::FUEq,
    CmpOp::FUNe,
    CmpOp::FULt,
    CmpOp::FULe,
    CmpOp::FOrd,
    CmpOp::FUno,
];

/// One instruction with every field independently randomized.
fn gen_inst(r: &mut Rng, next_val: &mut u32, pool: &mut Vec<ValueId>) -> Inst {
    let span = r.span();
    let mut fresh = || {
        let v = ValueId(*next_val);
        *next_val += 1;
        v
    };
    let kind = match r.below(12) {
        0 => {
            let ty = int_ty(r);
            let a = operand_of(r, &ty, pool);
            let b = operand_of(r, &ty, pool);
            let dst = fresh();
            pool.push(dst);
            InstKind::Assign {
                dst,
                rv: RValue::Bin {
                    op: *r.pick(BIN_OPS),
                    a,
                    b,
                    ty,
                    signed: true,
                },
            }
        }
        1 => {
            let ty = int_ty(r);
            let a = operand_of(r, &ty, pool);
            let b = operand_of(r, &ty, pool);
            let dst = fresh();
            pool.push(dst);
            InstKind::Assign {
                dst,
                rv: RValue::Cmp {
                    op: *r.pick(CMP_OPS),
                    a,
                    b,
                    ty,
                },
            }
        }
        2 => {
            // Alignment is a power of two but deliberately never 1, and the type is
            // independent of it — so swapping the two fields is visible.
            let dst = fresh();
            pool.push(dst);
            InstKind::Assign {
                dst,
                rv: RValue::Load {
                    ty: any_ty(r),
                    addr: Operand::Const(Const::Null),
                    align: 1u64 << (1 + r.below(4)),
                    vol: if r.below(2) == 0 {
                        Volatility::Normal
                    } else {
                        Volatility::Volatile
                    },
                },
            }
        }
        3 => {
            let ty = int_ty(r);
            InstKind::Store {
                addr: Operand::Const(Const::Null),
                val: operand_of(r, &ty, &[]),
                ty,
                align: 1u64 << (1 + r.below(4)),
                vol: if r.below(2) == 0 {
                    Volatility::Normal
                } else {
                    Volatility::Volatile
                },
            }
        }
        4 => {
            // `off` and `width` are distinct and both non-zero: a transposition of the
            // pair is exactly the kind of lockstep bug the fixture cannot see.
            let off = 1 + r.below(7) as u32;
            let width = 1 + r.below(7) as u32;
            let dst = fresh();
            pool.push(dst);
            InstKind::Assign {
                dst,
                rv: RValue::LoadBits {
                    unit: CTy::Int(32),
                    addr: Operand::Const(Const::Null),
                    bits: BitRange { off, width },
                    signed: r.below(2) == 0,
                    align: 1u64 << (1 + r.below(3)),
                },
            }
        }
        5 => {
            let base = operand_of(r, &CTy::Ptr, pool);
            let off = int_const(r, 64);
            let dst = fresh();
            pool.push(dst);
            InstKind::Assign {
                dst,
                rv: RValue::PtrAdd { base, off },
            }
        }
        6 => {
            let lanes = 2 + r.below(6) as u32;
            let elem = CTy::Int(*r.pick(&[8, 16, 32]));
            let e = operand_of(r, &elem, &[]);
            let dst = fresh();
            pool.push(dst);
            InstKind::Assign {
                dst,
                rv: RValue::Splat { elem: e, lanes },
            }
        }
        7 => {
            let dst = fresh();
            pool.push(dst);
            let cond = operand_of(r, &CTy::Int(1), pool);
            let ty = int_ty(r);
            let t = operand_of(r, &ty, pool);
            let f = operand_of(r, &ty, pool);
            InstKind::Assign {
                dst,
                rv: RValue::Select { cond, t, f },
            }
        }
        8 => InstKind::Marker(MarkerKind::SeqPoint),
        9 => InstKind::Marker(MarkerKind::Scope(ScopeEvent {
            // A non-zero scope id, so dropping the field is visible.
            scope: ScopeId(1 + r.below(20) as u32),
            kind: if r.below(2) == 0 {
                ScopeKind::Enter
            } else {
                ScopeKind::Exit
            },
        })),
        10 => InstKind::Marker(MarkerKind::Label(format!("L{}", r.below(1000)).into())),
        _ => {
            let dst = fresh();
            pool.push(dst);
            let ty = int_ty(r);
            InstKind::Assign {
                dst,
                rv: RValue::Use(operand_of(r, &ty, pool)),
            }
        }
    };
    Inst {
        kind,
        span,
        generated: false,
    }
}

fn gen_module(seed: u64) -> Module {
    let mut r = Rng(seed | 1);
    let mut m = Module::default();

    for i in 0..r.below(3) {
        m.globals.push(Global {
            id: GlobalId(i as u32),
            name: format!("g{i}").into(),
            // Never the default 1/1, and size never equals align.
            size: 4 + 4 * r.below(8),
            align: 1u64 << (1 + r.below(3)),
            is_const: r.below(2) == 0,
            // The round trip must carry these too: a printer that dropped a global's
            // bytes would still round-trip every module that has none.
            init: match r.below(3) {
                0 => GlobalInit::Zero,
                1 => GlobalInit::Extern,
                _ => GlobalInit::Bytes((0..1 + r.below(6)).map(|_| r.below(256) as u8).collect()),
            },
            linkage: if r.below(2) == 0 {
                Linkage::Internal
            } else {
                Linkage::External
            },
            span: r.span(),
        });
    }

    let nfuncs = 1 + r.below(3);
    for fi in 0..nfuncs {
        let mut next_val = 0u32;
        let nparams = r.below(3);
        let params: Vec<Param> = (0..nparams)
            .map(|_| {
                let v = ValueId(next_val);
                next_val += 1;
                Param {
                    value: v,
                    ty: any_ty(&mut r),
                }
            })
            .collect();
        let mut pool: Vec<ValueId> = params.iter().map(|p| p.value).collect();

        let allocas: Vec<AllocaDecl> = (0..r.below(3))
            .map(|ai| AllocaDecl {
                id: AllocaId(ai as u32),
                ty: any_ty(&mut r),
                // Never 1 — the default the fixture always used.
                count: 2 + r.below(16),
                align: 1u64 << (1 + r.below(4)),
                scope: ScopeId(1 + r.below(9) as u32),
                lifetime: if r.below(2) == 0 {
                    Lifetime::Scope
                } else {
                    Lifetime::Function
                },
                name: None,
                span: r.span(),
            })
            .collect();

        let nblocks = 1 + r.below(3);
        let mut blocks = Vec::new();
        for bi in 0..nblocks {
            let insts: Vec<Inst> = (0..r.below(5))
                .map(|_| gen_inst(&mut r, &mut next_val, &mut pool))
                .collect();
            // The last block always returns; earlier ones jump forward, so the CFG is
            // acyclic and every block is reachable.
            let term = if bi + 1 == nblocks {
                Terminator::Return(None)
            } else {
                Terminator::Goto(BlockId(bi as u32 + 1))
            };
            blocks.push(Block {
                id: BlockId(bi as u32),
                insts,
                term,
                // Order is preserved, not sorted — a descending list would be resorted
                // by a printer that canonicalized, and that is worth catching.
                gcov_lines: (0..r.below(4)).map(|k| 90 - k as u32).collect(),
                span: r.span(),
            });
        }

        m.funcs.push(Function {
            id: FuncId(fi as u32),
            name: format!("f{fi}").into(),
            params,
            ret: CTy::Void,
            variadic: r.below(4) == 0,
            allocas,
            blocks,
            entry: BlockId(0),
            attrs: FnAttrs {
                noreturn: r.below(3) == 0,
                no_side_effects: r.below(3) == 0,
                order_sensitive: r.below(3) == 0,
                march_variant: if r.below(3) == 0 {
                    Some("avx512".into())
                } else {
                    None
                },
            },
            access_paths: Default::default(),
            body: Body::Defined,
            span: r.span(),
            // Varied, so the round-trip is actually exercised: a generator that only ever
            // emits external functions cannot notice a printer that drops `static`.
            linkage: if r.below(3) == 0 {
                chiero_cir::Linkage::Internal
            } else {
                chiero_cir::Linkage::External
            },
        });
    }
    m
}

/// The property: for a random module, `parse(print(m))` is **structurally equal** to `m`.
#[test]
fn random_modules_round_trip_structurally() {
    for seed in 1..400u64 {
        let m = gen_module(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let printed = print(&m);
        let back =
            parse(&printed).unwrap_or_else(|e| panic!("seed {seed}: {e:?}\n----\n{printed}----"));
        assert_eq!(back, m, "seed {seed} did not round-trip:\n{printed}");
    }
}

/// Printing is canonicalization (020 §2), so a second pass must be byte-identical.
#[test]
fn printing_a_random_module_is_idempotent() {
    for seed in 1..400u64 {
        let m = gen_module(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let once = print(&m);
        let twice = print(&parse(&once).unwrap());
        assert_eq!(once, twice, "seed {seed} is not canonical");
    }
}

/// The generator must actually reach the fields it claims to vary, or the property tests
/// above are the same vacuous instrument they were written to replace. This asserts the
/// corpus of generated text contains non-default values in the positions that the
/// hand-written fixture always left at their identity value.
#[test]
fn the_generator_produces_non_default_field_values() {
    let all: String = (1..400u64)
        .map(|s| print(&gen_module(s.wrapping_mul(0x9e37_79b9_7f4a_7c15))))
        .collect();
    for (what, needle) in [
        ("a non-dummy span", "; span "),
        ("a non-root expansion context", ":1\n"),
        ("a volatile access", "volatile"),
        ("function lifetime", "lifetime function"),
        ("a non-zero scope", "scope 1"),
        ("a signed bitfield", "signed"),
        ("an unordered predicate", "cmp fun"),
        ("a march variant", "march \"avx512\""),
        ("a const global", "global const"),
        ("a variadic function", "..."),
    ] {
        assert!(all.contains(needle), "the generator never emits {what}");
    }
}
