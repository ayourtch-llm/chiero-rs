//! Covers: 020 §4.4's `AccessPath`.
//!
//! §4.4: "`AccessPath` is **reporting-only** — it makes a finding read `p->adj[3].counter`,
//! or `b->opaque as ip4_rewrite_t.adj_index`, instead of `*(i64*)(%7 + 24)`. No analysis
//! may branch on it."
//!
//! Reporting-only is a real constraint, not a disclaimer, and it is why the paths live in
//! a side table on the function rather than on the instructions: an `Inst` that carried
//! one would put it in front of every pass and every checker, and "no analysis may branch
//! on it" would be a rule nobody could see they were breaking.

use chiero_cir::*;
use chiero_span::{BytePos, ExpnCtx, Span};

fn at(lo: u32) -> Span {
    Span::new(BytePos(lo), BytePos(lo + 1), ExpnCtx(0))
}

fn sym(s: &str) -> Symbol {
    s.into()
}

/// `b->opaque as ip4_rewrite_t.adj_index` — §4.4's own example, which is why it is the
/// one asserted here.
#[test]
fn a_union_view_renders_the_way_the_spec_writes_it() {
    let p = AccessPath {
        root: PathRoot::Local {
            alloca: AllocaId(0),
            name: Some(sym("b")),
        },
        steps: [
            PathStep::Deref,
            PathStep::UnionMember {
                name: sym("adj_index"),
                off: 0,
                view: sym("ip4_rewrite_t"),
            },
        ]
        .into_iter()
        .collect(),
    };
    assert_eq!(p.render(), "(*b) as ip4_rewrite_t.adj_index");
}

/// `p->adj[3].counter`, §4.4's other example: a field, an index, a field.
#[test]
fn fields_and_indices_render_as_c_writes_them() {
    let p = AccessPath {
        root: PathRoot::Local {
            alloca: AllocaId(1),
            name: Some(sym("p")),
        },
        steps: [
            PathStep::Deref,
            PathStep::Field {
                name: sym("adj"),
                off: 8,
            },
            PathStep::Index(Operand::Const(Const::Int { bits: 64, val: 3 })),
            PathStep::Field {
                name: sym("counter"),
                off: 16,
            },
        ]
        .into_iter()
        .collect(),
    };
    assert_eq!(p.render(), "p->adj[3].counter", "020 §4.4's own spelling");
}

/// A bit-field step names its range, because "the wrong bits" and "the wrong bytes" are
/// different bugs and a reader needs to know which one they are looking at.
#[test]
fn a_bitfield_step_names_its_range() {
    let p = AccessPath {
        root: PathRoot::Global {
            g: GlobalId(0),
            name: sym("cfg"),
        },
        steps: [PathStep::Bits {
            name: sym("flags"),
            bits: BitRange { off: 3, width: 5 },
        }]
        .into_iter()
        .collect(),
    };
    assert_eq!(p.render(), "cfg.flags:3..8");
}

/// An **unnamed** root still renders, because a path that panicked or rendered nothing on
/// a compiler-generated temporary would take the whole finding with it.
#[test]
fn an_unnamed_root_renders_as_the_slot_it_is() {
    let p = AccessPath {
        root: PathRoot::Local {
            alloca: AllocaId(7),
            name: None,
        },
        steps: Default::default(),
    };
    assert_eq!(p.render(), "%alloca7");

    // And a symbolic index, which has no value to print.
    let p = AccessPath {
        root: PathRoot::Local {
            alloca: AllocaId(0),
            name: Some(sym("v")),
        },
        steps: [PathStep::Index(Operand::Value(ValueId(4)))]
            .into_iter()
            .collect(),
    };
    assert_eq!(p.render(), "v[%4]");
}

/// The side table survives print → parse → print (020 contract 2).
///
/// A path that did not round-trip would vanish the first time a module was written to
/// `.cir` and read back — silently, since a missing path degrades a finding rather than
/// failing anything.
#[test]
fn access_paths_survive_the_text_round_trip() {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Void,
        variadic: false,
        allocas: vec![AllocaDecl {
            id: AllocaId(0),
            ty: CTy::Int(8),
            count: 40,
            align: 8,
            scope: ScopeId(0),
            lifetime: Lifetime::Scope,
            name: Some("opaque".into()),
            span: at(1),
        }],
        blocks: vec![Block {
            id: BlockId(0),
            insts: vec![Inst {
                kind: InstKind::Assign {
                    dst: ValueId(0),
                    rv: RValue::AddrOfLocal {
                        alloca: AllocaId(0),
                    },
                },
                span: at(10),
                generated: false,
            }],
            term: Terminator::Return(None),
            gcov_lines: Default::default(),
            span: at(1),
        }],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        access_paths: [(
            ValueId(0),
            AccessPath {
                root: PathRoot::Local {
                    alloca: AllocaId(0),
                    name: Some(sym("opaque")),
                },
                steps: [PathStep::UnionMember {
                    name: sym("adj_index"),
                    off: 0,
                    view: sym("ip4_rewrite_t"),
                }]
                .into_iter()
                .collect(),
            },
        )]
        .into_iter()
        .collect(),
        span: at(1),
    };
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    let text = text::print(&m);
    assert!(text.contains("path"), "the printer emits it: {text}");
    let back = text::parse(&text).unwrap_or_else(|e| panic!("{e:?}\n{text}"));
    assert_eq!(text::print(&back), text, "byte-exact round trip");
    assert_eq!(
        back.funcs[0].access_paths.len(),
        1,
        "and the path itself survived, not merely a line that mentions one"
    );
    assert_eq!(
        back.funcs[0].access_paths[&ValueId(0)].render(),
        "opaque as ip4_rewrite_t.adj_index"
    );
    // **Structurally, not just as it renders.** `render()` never prints an offset, so a
    // round trip that dropped every `off` would produce the same string and this test
    // would agree with itself about a path that had lost half its content.
    assert_eq!(
        back.funcs[0].access_paths, m.funcs[0].access_paths,
        "every field survived, including the ones `render` does not show"
    );
}

/// **The verifier does not require a path**, and does not reject one that names a value
/// the function does not define.
///
/// Reporting-only cuts both ways: a stale path is a bad message, not an invalid module,
/// and rejecting it would make a reporting aid able to fail a run.
#[test]
fn a_path_is_never_a_verification_error() {
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Void,
        variadic: false,
        allocas: vec![],
        blocks: vec![Block {
            id: BlockId(0),
            insts: vec![],
            term: Terminator::Return(None),
            gcov_lines: Default::default(),
            span: at(1),
        }],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        access_paths: [(
            // `%99` is defined nowhere.
            ValueId(99),
            AccessPath {
                root: PathRoot::Local {
                    alloca: AllocaId(3),
                    name: None,
                },
                steps: Default::default(),
            },
        )]
        .into_iter()
        .collect(),
        span: at(1),
    };
    let m = Module {
        funcs: vec![f],
        ..Default::default()
    };
    let errs = verify::verify(&m);
    assert!(
        errs.iter().all(|e| !e.is_error()),
        "a reporting aid cannot fail a run: {errs:#?}"
    );
}

/// Offsets round-trip, and they are what `render` cannot show.
///
/// A path's `off` fields are never printed by `render` — `.adj[3].counter` says nothing
/// about byte 16 — so a serialization that dropped them looks perfect in every message.
/// They exist because 020 §4.4 lets a `StructLayoutId` be attached for reporting, and an
/// offset is how a reader checks a path against a layout.
#[test]
fn path_step_offsets_survive_the_round_trip() {
    let steps: Vec<PathStep> = vec![
        PathStep::Field {
            name: sym("adj"),
            off: 8,
        },
        PathStep::UnionMember {
            name: sym("adj_index"),
            off: 24,
            view: sym("ip4_rewrite_t"),
        },
        PathStep::Bits {
            name: sym("flags"),
            bits: BitRange { off: 3, width: 5 },
        },
        PathStep::Index(Operand::Const(Const::Int { bits: 64, val: 3 })),
        PathStep::Deref,
    ];
    let f = Function {
        id: FuncId(0),
        name: "f".into(),
        params: vec![],
        ret: CTy::Void,
        variadic: false,
        allocas: vec![],
        blocks: vec![Block {
            id: BlockId(0),
            insts: vec![],
            term: Terminator::Return(None),
            gcov_lines: Default::default(),
            span: at(1),
        }],
        entry: BlockId(0),
        attrs: Default::default(),
        body: Body::Defined,
        access_paths: [(
            ValueId(3),
            AccessPath {
                root: PathRoot::Global {
                    g: GlobalId(0),
                    name: sym("cfg"),
                },
                steps: steps.iter().cloned().collect(),
            },
        )]
        .into_iter()
        .collect(),
        span: at(1),
    };
    let m = Module {
        funcs: vec![f],
        globals: vec![Global {
            id: GlobalId(0),
            name: "cfg".into(),
            size: 64,
            align: 8,
            is_const: false,
            init: GlobalInit::Zero,
            linkage: Linkage::External,
            span: at(1),
        }],
        ..Default::default()
    };
    let back = text::parse(&text::print(&m)).expect("reparse");
    let got: Vec<PathStep> = back.funcs[0].access_paths[&ValueId(3)]
        .steps
        .iter()
        .cloned()
        .collect();
    assert_eq!(got, steps, "every step, with its offsets, came back");
}
