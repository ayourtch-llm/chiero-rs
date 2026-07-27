//! The model registry and the standard models (024).
//!
//! Covers **024 contracts 1, 2, 3, 4, 5, 6, 18, 19, 21** and §2.1.
//!
//! §2.1 is the load-bearing rule here, and it is easy to read as editorial when it is
//! not: declaring a model `Approximate` has a **mechanical** fidelity effect. Without it
//! there is a hole straight through the project's central guarantee — a run calling
//! `scanf`, or any `<math.h>` function, or `read`, could finish `Exact`, mint a witness
//! and report "no bugs exist" as a proof. The *unmodeled* path was already loud; this is
//! the modeled path, which is worse because it looks deliberate.

use chiero_model::*;

/// **024 contract 18.** Registering a name twice is an error; `replace` is the way to
/// override. Silent last-wins registration would make which model you got depend on link
/// order, and 001 §5 makes determinism a hard requirement.
#[test]
fn registering_a_name_twice_is_an_error_but_replacing_is_not() {
    let mut r = ModelRegistry::with_builtins();
    let before = r.len();
    assert!(r.lookup("malloc").is_some(), "the builtins are present");

    match r.register(ModelEntry::approximate("malloc", "a second malloc")) {
        Err(ModelError::Duplicate(n)) => assert_eq!(&*n, "malloc"),
        other => panic!("expected Duplicate, got {other:?}"),
    }
    assert_eq!(r.len(), before, "a rejected registration changes nothing");

    r.replace(ModelEntry::approximate("malloc", "a deliberate override"))
        .expect("replace succeeds");
    assert_eq!(r.len(), before, "replacing does not add an entry");
    assert_eq!(
        r.lookup("malloc").unwrap().precision,
        Precision::Approximate("a deliberate override".into())
    );
}

/// **024 §2.1 and contract 21.** Every `Approximate` model carries a reason of at least
/// eight non-whitespace characters. A non-empty check is satisfied by `" "` and says
/// nothing, which is why the spec states the length.
#[test]
fn every_approximate_model_gives_a_real_reason() {
    let r = ModelRegistry::with_builtins();
    let mut checked = 0;
    for e in r.entries() {
        if let Precision::Approximate(reason) = &e.precision {
            let n = reason.chars().filter(|c| !c.is_whitespace()).count();
            assert!(
                n >= 8,
                "`{}` has an approximate precision with a {n}-character reason: {reason:?}",
                e.name
            );
            checked += 1;
        }
    }
    assert!(
        checked > 0,
        "the default registry must contain approximate models, or this proves nothing"
    );
}

/// **024 §2.1: `Approximate` is mechanical, not editorial.** Dispatching one sets
/// `Fidelity >= Approximated` and pushes an assumption carrying the reason — so a program
/// calling `scanf` cannot come back `Exact` (contract 21b).
#[test]
fn dispatching_an_approximate_model_degrades_and_says_why() {
    let r = ModelRegistry::with_builtins();
    let e = r.lookup("scanf").expect("scanf is modeled as approximate");
    let Precision::Approximate(reason) = &e.precision else {
        panic!("scanf must be approximate, got {:?}", e.precision)
    };
    assert_eq!(
        e.fidelity_effect(),
        Some(ModelFidelity::Approximated),
        "an approximate model degrades by dispatching, not by anyone remembering to"
    );
    assert!(reason.contains("input"));

    // An exact model does not degrade, or every program would be approximate and the
    // distinction would carry no information.
    let m = r.lookup("memcpy").expect("memcpy is modeled");
    assert_eq!(m.precision, Precision::Exact);
    assert_eq!(m.fidelity_effect(), None);
}

/// **024 contract 21c.** A `Havoc` outcome degrades identically whether it came from the
/// default unmodeled fallback or from a registered model that chose to havoc — otherwise
/// "I don't know" said politely counts for less than "I don't know" said by omission.
#[test]
fn a_havoc_outcome_degrades_wherever_it_comes_from() {
    let from_default = HavocSpec::unmodeled_extern();
    let from_model = HavocSpec {
        objects: vec![],
        reachable_depth: 0,
        init: HavocInit::Uninitialized,
        may_free: false,
    };
    assert_eq!(
        from_default.fidelity_effect(),
        from_model.fidelity_effect(),
        "a deliberate havoc is exactly as imprecise as an accidental one"
    );
    assert_eq!(from_default.fidelity_effect(), ModelFidelity::Approximated);
}

/// **024 §2.1's default, spelled out.** An unmodeled extern havocs with `Symbolic` init
/// and `reachable_depth: 1`. `init` has no safe default and the spec says so: `Symbolic`
/// can mask a genuine uninitialized-read bug, `Uninitialized` produces a false-positive
/// storm on any buffer the callee legitimately filled. The choice is recorded so it is
/// visible rather than folkloric.
#[test]
fn the_unmodeled_default_is_symbolic_at_depth_one_and_says_so() {
    let h = HavocSpec::unmodeled_extern();
    assert_eq!(h.init, HavocInit::Symbolic);
    assert_eq!(h.reachable_depth, 1);
    assert!(!h.may_free, "an unknown function is not assumed to free");
    let note = h.describe();
    assert!(note.contains("symbolic"), "{note}");
    assert!(note.contains("depth 1"), "{note}");
}

/// **024 contract 19**, which is 001 §7's reusable-library requirement made checkable:
/// this crate contains no VPP knowledge. `chiero-vpp` registers vppinfra models *into*
/// it; if the names leak the other way the layering has already failed.
#[test]
fn the_crate_contains_no_vpp_knowledge() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    for e in std::fs::read_dir(&src).expect("src exists").flatten() {
        let text = std::fs::read_to_string(e.path()).unwrap_or_default();
        for (n, line) in text.lines().enumerate() {
            // Whole-token matching: a substring grep lets `clib_` match a comment about
            // *why* the rule exists, which is how this kind of guard becomes decoration.
            for tok in line.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                if ["vec_len", "pool_get", "clib_mem_alloc", "vlib_buffer_t"].contains(&tok) {
                    hits.push(format!("{}:{}: {tok}", e.path().display(), n + 1));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "VPP identifiers in chiero-model: {hits:#?}"
    );
}

/// A model that is not registered is not found, and looking one up must not invent an
/// entry — the unmodeled path is the engine's to handle loudly (023 §5).
#[test]
fn an_unregistered_name_is_simply_absent() {
    let r = ModelRegistry::with_builtins();
    assert!(r.lookup("a_function_nobody_modeled").is_none());
}

/// **024 contract 1's shape.** `malloc` forks into success and `NULL` by default, because
/// allocation failure is a real path and pretending otherwise silently prunes it. With
/// `alloc_may_fail = false` it is one state (contract 2).
#[test]
fn malloc_forks_into_success_and_failure_unless_told_not_to() {
    let d = AllocPolicy::default();
    assert!(
        d.may_fail,
        "allocation failure is a real path; pruning it by default hides a bug class"
    );
    assert_eq!(d.outcomes(), 2);
    let never = AllocPolicy { may_fail: false };
    assert_eq!(never.outcomes(), 1);
}

// ---------------------------------------------------------------------------
// Models that actually execute (024 §3, contracts 1-5, 10).
// ---------------------------------------------------------------------------

use chiero_mem::{Endian, Memory, ObjKind, Pointer};
use chiero_solver::TermArena;
use chiero_span::Span;

fn ctx<'a>(m: &'a mut Memory, a: &'a mut TermArena) -> ModelCtx<'a> {
    ModelCtx::new(m, a, Span::DUMMY, Endian::Little)
}

/// **024 contract 1.** `malloc(16)` produces one `Heap` object of size 16 with all bytes
/// uninitialized, and forks into a success state and a `NULL` state.
#[test]
fn malloc_allocates_uninitialized_and_forks_on_failure() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let mut cx = ctx(&mut m, &mut a);
    let out = models::malloc(&mut cx, 16, AllocPolicy::default());

    let ModelOutcome::Fork(branches) = out else {
        panic!("malloc must fork by default, got {out:?}")
    };
    assert_eq!(branches.len(), 2, "success and NULL");
    let (ok, null) = (&branches[0], &branches[1]);
    let ModelOutcome::Value(Some(Value::Ptr(p))) = &ok.1 else {
        panic!("the success branch returns a pointer, got {:?}", ok.1)
    };
    assert_eq!(p.off, 0);
    assert_eq!(cx.mem().size_of_pub(p.base), Some(16));
    // Uninitialized: reading it is a finding, which is the whole reason `malloc` is not
    // `calloc` and the reason a checker can tell them apart.
    assert!(
        cx.mem()
            .read(*p, 4, Span::DUMMY)
            .faults
            .iter()
            .any(|f| matches!(f, chiero_mem::MemFault::Uninitialized { .. })),
        "malloc'd bytes are uninitialized"
    );
    assert!(
        matches!(&null.1, ModelOutcome::Value(Some(Value::Ptr(q))) if q.base == chiero_mem::ObjectId::NULL),
        "the failure branch returns NULL, got {:?}",
        null.1
    );
}

/// **024 contract 2.** With `alloc_may_fail = false` the same call is one outcome. This
/// is how an allocator that aborts instead of returning `NULL` is modeled.
#[test]
fn malloc_does_not_fork_when_the_allocator_cannot_fail() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let mut cx = ctx(&mut m, &mut a);
    let out = models::malloc(&mut cx, 16, AllocPolicy { may_fail: false });
    assert!(
        matches!(out, ModelOutcome::Value(Some(Value::Ptr(_)))),
        "one outcome, got {out:?}"
    );
}

/// **024 contract 3.** `calloc(4, 8)` yields 32 zeroed, *initialized* bytes — reading
/// them produces no finding. A `calloc` that allocated without initializing would be
/// indistinguishable from `malloc` and would report a false uninitialized read on every
/// correct use.
#[test]
fn calloc_yields_zeroed_initialized_bytes() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let mut cx = ctx(&mut m, &mut a);
    let out = models::calloc(&mut cx, 4, 8, AllocPolicy { may_fail: false });
    let ModelOutcome::Value(Some(Value::Ptr(p))) = out else {
        panic!("expected a pointer, got {out:?}")
    };
    assert_eq!(cx.mem().size_of_pub(p.base), Some(32));
    let r = cx.mem().read(p, 32, Span::DUMMY);
    assert!(r.faults.is_empty(), "{:#?}", r.faults);
    assert_eq!(r.value.unwrap(), vec![0u8; 32]);
}

/// **024 contract 4.** `calloc(SIZE_MAX, 2)` is exactly one overflow finding. Silently
/// wrapping would allocate a *small* object for a request that cannot be satisfied, which
/// is the classic integer-overflow-to-heap-overflow chain.
#[test]
fn calloc_reports_the_multiplication_overflow() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let mut cx = ctx(&mut m, &mut a);
    let out = models::calloc(&mut cx, u64::MAX, 2, AllocPolicy { may_fail: false });
    match out {
        ModelOutcome::Finding(f) => assert!(
            f.contains("overflow"),
            "the finding must name the cause: {f}"
        ),
        other => panic!("expected one overflow finding, got {other:?}"),
    }
    assert_eq!(cx.findings().len(), 1, "exactly one: {:#?}", cx.findings());
}

/// **024 contract 5.** `free(NULL)` is a no-op producing no findings; freeing a non-heap
/// object is exactly one finding. `free(NULL)` is legal C that models call constantly,
/// and reporting it is a false positive on correct code.
#[test]
fn free_of_null_is_silent_and_free_of_a_stack_object_is_not() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let stack = m.alloc(ObjKind::Stack, 16, 8, Span::DUMMY);
    let heap = m.alloc(ObjKind::Heap, 16, 8, Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);

    let n = models::free(
        &mut cx,
        Pointer {
            base: chiero_mem::ObjectId::NULL,
            off: 0,
        },
    );
    assert!(matches!(n, ModelOutcome::Value(None)));
    assert!(cx.findings().is_empty(), "{:#?}", cx.findings());

    models::free(
        &mut cx,
        Pointer {
            base: stack,
            off: 0,
        },
    );
    assert_eq!(cx.findings().len(), 1, "{:#?}", cx.findings());

    // And a real heap free is silent, or the test above is satisfied by a model that
    // reports every free.
    models::free(&mut cx, Pointer { base: heap, off: 0 });
    assert_eq!(cx.findings().len(), 1, "a legitimate free adds nothing");
}

/// **024 contract 10.** `memcpy` with overlapping ranges is one finding; `memmove` with
/// the same ranges is none and produces the correct bytes.
#[test]
fn memcpy_and_memmove_differ_on_overlap() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let o = m.alloc(ObjKind::Heap, 32, 8, Span::DUMMY);
    m.write(
        Pointer { base: o, off: 0 },
        &[1, 2, 3, 4, 5, 6, 7, 8],
        Span::DUMMY,
    );
    let mut cx = ctx(&mut m, &mut a);

    let dst = Pointer { base: o, off: 2 };
    let src = Pointer { base: o, off: 0 };
    models::memcpy(&mut cx, dst, src, 6);
    assert_eq!(cx.findings().len(), 1, "overlap is a memcpy violation");
    // The copy still *happens* — reporting is not refusing, and execution continues on a
    // state whose bytes reflect what the program actually did.
    assert_eq!(
        cx.mem()
            .read(Pointer { base: o, off: 0 }, 8, Span::DUMMY)
            .value
            .unwrap(),
        vec![1, 2, 1, 2, 3, 4, 5, 6]
    );

    // A **fresh** object for memmove: the copy above already mutated this one, so reusing
    // it would compare against bytes the first call produced rather than the second.
    let mut m2 = Memory::new();
    let mut a2 = TermArena::new();
    let o2 = m2.alloc(ObjKind::Heap, 32, 8, Span::DUMMY);
    m2.write(
        Pointer { base: o2, off: 0 },
        &[1, 2, 3, 4, 5, 6, 7, 8],
        Span::DUMMY,
    );
    let mut cx2 = ctx(&mut m2, &mut a2);
    models::memmove(
        &mut cx2,
        Pointer { base: o2, off: 2 },
        Pointer { base: o2, off: 0 },
        6,
    );
    assert!(cx2.findings().is_empty(), "memmove permits overlap");
    assert_eq!(
        cx2.mem()
            .read(Pointer { base: o2, off: 0 }, 8, Span::DUMMY)
            .value
            .unwrap(),
        vec![1, 2, 1, 2, 3, 4, 5, 6],
        "memmove copies as if through a temporary"
    );
}

/// `memset` marks the range initialized and readable as the set byte, and a read past it
/// still reports uninitialized — or the model has quietly initialized the whole object.
#[test]
fn memset_initializes_exactly_its_range() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let o = m.alloc(ObjKind::Heap, 16, 8, Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    models::memset(&mut cx, Pointer { base: o, off: 0 }, 0xAB, 8);
    assert!(cx.findings().is_empty());
    let r = cx.mem().read(Pointer { base: o, off: 0 }, 8, Span::DUMMY);
    assert_eq!(r.value.unwrap(), vec![0xAB; 8]);
    assert!(
        !cx.mem()
            .read(Pointer { base: o, off: 8 }, 8, Span::DUMMY)
            .faults
            .is_empty(),
        "beyond the range nothing changed"
    );
}

/// The endianness the models use comes from the target, not from a constant chosen here —
/// a model crate that hardcoded little-endian would silently produce byte-swapped answers
/// on a big-endian target.
#[test]
fn the_model_context_carries_the_target_byte_order() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let cx = ctx(&mut m, &mut a);
    assert_eq!(cx.endian(), Endian::Little, "the default target is x86-64");
}

/// **A model registered `Exact` must actually exist.** `realloc`, `strlen`, `strcpy` and
/// friends were registered exact while nothing implemented them — so `fidelity_effect()`
/// said "dispatching this degrades nothing" about a function that cannot be dispatched at
/// all. That is the confidently-wrong shape this crate's own doc rails against, pointed
/// the wrong way.
#[test]
fn every_exact_model_is_actually_implemented() {
    let r = ModelRegistry::with_builtins();
    for e in r.entries() {
        if e.precision == Precision::Exact {
            assert!(
                models::is_implemented(&e.name),
                "`{}` claims Exact precision but has no implementation; \
                 a declaration that cannot run must not claim faithfulness",
                e.name
            );
        }
    }
}

/// **The byte order comes from the target.** The field was hardcoded under a comment
/// saying it was not, and the test asserted the same constant — so both candidate
/// implementations gave the same answer and neither the code nor the test could tell
/// them apart.
#[test]
fn the_byte_order_follows_the_target_rather_than_a_constant() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let le = ModelCtx::new(&mut m, &mut a, Span::DUMMY, Endian::Little);
    assert_eq!(le.endian(), Endian::Little);
    drop(le);
    let be = ModelCtx::new(&mut m, &mut a, Span::DUMMY, Endian::Big);
    assert_eq!(
        be.endian(),
        Endian::Big,
        "a hardcoded order would answer Little here"
    );
}

/// **024 contract 19 is a prefix rule, not a list of four names.** The spec's own check is
/// `grep -rE 'vec_|pool_|clib_|vlib_'`, and matching four exact tokens let `clib_warning`,
/// `vec_add1`, `pool_put` and `vlib_main_t` all pass. The walk is recursive too: a
/// subdirectory read as clean.
#[test]
fn no_vpp_prefix_appears_anywhere_in_the_crate_source() {
    fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).expect("readable").flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
                continue;
            }
            let text = std::fs::read_to_string(&p).expect("this crate's sources are utf-8");
            for (n, line) in text.lines().enumerate() {
                for tok in line.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                    if ["vec_", "pool_", "clib_", "vlib_"]
                        .iter()
                        .any(|pre| tok.starts_with(pre))
                    {
                        out.push(format!("{}:{}: {tok}", p.display(), n + 1));
                    }
                }
            }
        }
    }
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    walk(&src, &mut hits);
    assert!(
        hits.is_empty(),
        "VPP identifiers in chiero-model: {hits:#?}"
    );
}

/// `replace` on a name nobody registered is an error, not a silent insert — otherwise a
/// typo in an override registers a *second* model under the wrong name and the intended
/// one keeps its old behaviour.
#[test]
fn replacing_an_unregistered_name_is_an_error() {
    let mut r = ModelRegistry::with_builtins();
    let before = r.len();
    match r.replace(ModelEntry::exact("no_such_function")) {
        Err(ModelError::NotFound(n)) => assert_eq!(&*n, "no_such_function"),
        other => panic!("expected NotFound, got {other:?}"),
    }
    assert_eq!(r.len(), before);
}

/// `malloc` returns **heap** memory. Contract 5 makes freeing a non-heap object a
/// finding, so an allocator that produced a stack object would turn every correct
/// `free(malloc(n))` into a false positive — and the size and initialization assertions
/// are both blind to the kind.
#[test]
fn malloc_returns_heap_memory_so_freeing_it_is_not_a_finding() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let mut cx = ctx(&mut m, &mut a);
    let out = models::malloc(&mut cx, 16, AllocPolicy { may_fail: false });
    let ModelOutcome::Value(Some(Value::Ptr(p))) = out else {
        panic!("expected a pointer")
    };
    models::free(&mut cx, p);
    assert!(
        cx.findings().is_empty(),
        "freeing what malloc returned is correct C: {:#?}",
        cx.findings()
    );
}

// ---------------------------------------------------------------------------
// String models (024 §4, contracts 6-9).
// ---------------------------------------------------------------------------

/// **024 contract 6.** `strlen` over the concrete bytes `"abc\0"` returns 3 with no
/// forking. The concrete fast path carries almost all real traffic, and a model that
/// forked here would make every string in a program a branch point.
#[test]
fn strlen_over_concrete_bytes_is_a_plain_answer() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let o = m.alloc(ObjKind::Heap, 8, 1, Span::DUMMY);
    m.write(Pointer { base: o, off: 0 }, b"abc\0", Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    match models::strlen(
        &mut cx,
        Pointer { base: o, off: 0 },
        StringPolicy::default(),
    ) {
        StrScan::Exact(n) => assert_eq!(n, 3),
        other => panic!("expected a definite length, got {other:?}"),
    }
    assert!(cx.findings().is_empty());
}

/// **024 §4 step 4, and the most valuable thing these models catch.** Running off the end
/// of the object is an **OOB finding**, not a silent stop: an unterminated string is a
/// real bug class, and a model that just stopped at the boundary would report nothing at
/// all for it.
#[test]
fn an_unterminated_string_is_an_out_of_bounds_finding() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let o = m.alloc(ObjKind::Heap, 4, 1, Span::DUMMY);
    m.write(Pointer { base: o, off: 0 }, b"abcd", Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    let r = models::strlen(
        &mut cx,
        Pointer { base: o, off: 0 },
        StringPolicy::default(),
    );
    assert!(
        matches!(r, StrScan::Unterminated { .. }),
        "no NUL in the object, got {r:?}"
    );
    assert_eq!(cx.findings().len(), 1, "{:#?}", cx.findings());
    assert!(cx.findings()[0].contains("unterminated"));
}

/// **024 contract 8, and §4's warning about steps 3 and 4 cancelling each other.**
///
/// The scan is bounded by `min(max_string_scan, object size)`. Reaching the **object's**
/// end is always an OOB finding; reaching the **cap** first adds no constraint and gives
/// `Bounded`. An earlier draft of the spec had the cap "constrain a terminator to exist
/// within the bound", which assumes away exactly the unterminated-string bug step 4
/// exists to find whenever the object is smaller than the cap.
#[test]
fn the_scan_cap_bounds_without_assuming_a_terminator_exists() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let o = m.alloc(ObjKind::Heap, 1000, 1, Span::DUMMY);
    m.set(Pointer { base: o, off: 0 }, b'x', 1000, Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    let r = models::strlen(
        &mut cx,
        Pointer { base: o, off: 0 },
        StringPolicy { max_scan: 256 },
    );
    match r {
        StrScan::CapReached { scanned } => assert_eq!(scanned, 256),
        other => panic!("expected the cap, got {other:?}"),
    }
    // **No unterminated finding**: the scan stopped early, so nothing is known about the
    // rest of the object — claiming a bug there would be inventing one.
    assert!(
        !cx.findings().iter().any(|f| f.contains("unterminated")),
        "the cap says 'I stopped looking', not 'there is no NUL': {:#?}",
        cx.findings()
    );
    assert_eq!(cx.findings().len(), 1);
    assert!(cx.findings()[0].contains("max_string_scan"));
}

/// The other half of that rule: when the object is **smaller** than the cap, its end wins
/// and the unterminated finding still fires. This is the case the earlier draft got
/// wrong, so it needs its own test rather than being implied by the two above.
#[test]
fn an_object_smaller_than_the_cap_still_reports_unterminated() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let o = m.alloc(ObjKind::Heap, 8, 1, Span::DUMMY);
    m.set(Pointer { base: o, off: 0 }, b'x', 8, Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    let r = models::strlen(
        &mut cx,
        Pointer { base: o, off: 0 },
        StringPolicy { max_scan: 256 },
    );
    assert!(
        matches!(r, StrScan::Unterminated { .. }),
        "the object ends before the cap, so its end decides: {r:?}"
    );
    assert!(cx.findings()[0].contains("unterminated"));
}

/// **024 contract 9.** `strcpy` into a 4-byte destination from a 10-byte source is
/// exactly one OOB finding, reported **at the destination**. This is the classic overflow
/// and the reason these models exist at all.
#[test]
fn strcpy_into_a_short_destination_is_one_finding() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let dst = m.alloc(ObjKind::Heap, 4, 1, Span::DUMMY);
    let src = m.alloc(ObjKind::Heap, 16, 1, Span::DUMMY);
    m.write(Pointer { base: src, off: 0 }, b"0123456789\0", Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    models::strcpy(
        &mut cx,
        Pointer { base: dst, off: 0 },
        Pointer { base: src, off: 0 },
        StringPolicy::default(),
    );
    assert_eq!(cx.findings().len(), 1, "{:#?}", cx.findings());
    assert!(
        cx.findings()[0].contains("destination"),
        "the finding is about the destination, not the source: {}",
        cx.findings()[0]
    );
}

/// A `strcpy` that fits copies the bytes **including the terminator** and reports
/// nothing. Without this the test above is satisfied by a model that reports every
/// `strcpy`, and the copy itself could be doing anything.
#[test]
fn a_strcpy_that_fits_copies_the_string_and_its_terminator() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let dst = m.alloc(ObjKind::Heap, 8, 1, Span::DUMMY);
    let src = m.alloc(ObjKind::Heap, 8, 1, Span::DUMMY);
    m.write(Pointer { base: src, off: 0 }, b"abc\0", Span::DUMMY);
    // **Pre-filled with non-zero bytes.** A freshly allocated object's backing reads as
    // zero, so a `strcpy` that forgot the terminator would leave a zero there anyway and
    // look correct — the same-answer trap, with the fixture supplying the right answer.
    m.write(Pointer { base: dst, off: 0 }, &[0xFF; 8], Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    models::strcpy(
        &mut cx,
        Pointer { base: dst, off: 0 },
        Pointer { base: src, off: 0 },
        StringPolicy::default(),
    );
    assert!(cx.findings().is_empty(), "{:#?}", cx.findings());
    assert_eq!(
        cx.mem()
            .read(Pointer { base: dst, off: 0 }, 5, Span::DUMMY)
            .value
            .unwrap(),
        vec![b'a', b'b', b'c', 0, 0xFF],
        "the NUL is copied, and nothing past it is"
    );
}

/// The destination bound is `strlen + 1`, exactly. A buffer of `strlen` bytes is one
/// short — that off-by-one *is* the classic overflow — and one of `strlen + 1` fits.
#[test]
fn the_destination_must_hold_the_terminator_too() {
    for (size, expect_finding) in [(3u64, true), (4, false)] {
        let mut m = Memory::new();
        let mut a = TermArena::new();
        let dst = m.alloc(ObjKind::Heap, size, 1, Span::DUMMY);
        let src = m.alloc(ObjKind::Heap, 8, 1, Span::DUMMY);
        m.write(Pointer { base: src, off: 0 }, b"abc\0", Span::DUMMY);
        let mut cx = ctx(&mut m, &mut a);
        models::strcpy(
            &mut cx,
            Pointer { base: dst, off: 0 },
            Pointer { base: src, off: 0 },
            StringPolicy::default(),
        );
        assert_eq!(
            !cx.findings().is_empty(),
            expect_finding,
            "\"abc\" needs 4 bytes; a {size}-byte destination"
        );
    }
}

// ---------------------------------------------------------------------------
// Compiler builtins and harness intrinsics (024 §6, §7; contracts 13-16).
// ---------------------------------------------------------------------------

/// **024 contract 13.** `__builtin_add_overflow(INT_MAX, 1, &r)` returns 1 and stores
/// `INT_MIN`; `(1, 1, &r)` returns 0 and stores 2. Both halves matter: a model that
/// always reported overflow would pass the first, and one that never did would pass the
/// second.
#[test]
fn add_overflow_reports_and_stores_the_wrapped_result() {
    for (x, y, want_overflow, want_stored) in [
        (i32::MAX as i128, 1i128, true, i32::MIN as i128),
        (1, 1, false, 2),
        (i32::MIN as i128, -1, true, i32::MAX as i128),
    ] {
        let got = builtins::add_overflow(32, x, y);
        assert_eq!(got.overflowed, want_overflow, "add_overflow({x}, {y})");
        assert_eq!(got.wrapped, want_stored, "add_overflow({x}, {y}) stored");
    }
}

/// The same for multiplication, where the wide computation matters most — a 32-bit
/// product that fits in 64 bits is exactly the case a narrow implementation gets wrong.
#[test]
fn mul_overflow_uses_a_wide_computation() {
    let got = builtins::mul_overflow(32, 0x1_0000, 0x1_0000);
    assert!(got.overflowed, "65536 * 65536 does not fit in 32 bits");
    assert_eq!(got.wrapped, 0);
    let ok = builtins::mul_overflow(32, 3, 5);
    assert!(!ok.overflowed);
    assert_eq!(ok.wrapped, 15);
}

/// **024 contract 14.** `__builtin_clz(0)` is exactly one UB finding; `__builtin_clz(1)`
/// returns 31 on a 32-bit input with none. The zero case is undefined behaviour, and a
/// model that returned 32 for it would silently answer a question C does not define.
#[test]
fn count_leading_zeros_treats_zero_as_undefined() {
    match builtins::clz(32, 0) {
        BuiltinResult::Undefined(why) => assert!(why.contains("zero"), "{why}"),
        other => panic!("clz(0) is UB, got {other:?}"),
    }
    assert_eq!(builtins::clz(32, 1), BuiltinResult::Value(31));
    assert_eq!(builtins::clz(32, 0x8000_0000), BuiltinResult::Value(0));
    assert_eq!(builtins::ctz(32, 1), BuiltinResult::Value(0));
    assert_eq!(builtins::ctz(32, 8), BuiltinResult::Value(3));
    assert!(matches!(builtins::ctz(32, 0), BuiltinResult::Undefined(_)));
    assert_eq!(builtins::popcount(32, 0xFF), BuiltinResult::Value(8));
    assert_eq!(builtins::popcount(32, 0), BuiltinResult::Value(0));
}

/// `__builtin_bswap` is a byte permutation, and the width matters — a 32-bit swap of a
/// value that happens to be symmetric in 16 bits would hide a width mix-up.
#[test]
fn byte_swap_permutes_at_the_declared_width() {
    assert_eq!(builtins::bswap(16, 0x1234), 0x3412);
    assert_eq!(builtins::bswap(32, 0x1122_3344), 0x4433_2211);
    assert_eq!(
        builtins::bswap(64, 0x0102_0304_0506_0708),
        0x0807_0605_0403_0201
    );
}

/// **024 contract 16.** `chiero_assume(0)` terminates the state with **no finding** — it
/// says "this path cannot happen", which is a statement about the harness rather than
/// about the program.
#[test]
fn assuming_a_contradiction_kills_the_state_silently() {
    let r = intrinsics::assume(Some(false));
    assert_eq!(r, IntrinsicOutcome::KillState);
}

/// **024 contract 15, and §7's warning.** `chiero_assert` produces a **finding** where
/// `chiero_assume` produces a constraint. They are not interchangeable, and confusing
/// them is the classic way to make a test suite vacuous: an `assert` that silently
/// constrained would turn every failing check into a pruned path, so the suite would
/// pass by not testing anything.
#[test]
fn assert_and_assume_are_not_interchangeable() {
    assert_eq!(intrinsics::assume(Some(true)), IntrinsicOutcome::Continue);
    assert_eq!(intrinsics::assume(Some(false)), IntrinsicOutcome::KillState);
    // An assert that *can* be violated is a finding, not a pruned path.
    assert_eq!(
        intrinsics::assert_(Some(false)),
        IntrinsicOutcome::Finding("chiero_assert failed".into())
    );
    assert_eq!(intrinsics::assert_(Some(true)), IntrinsicOutcome::Continue);
    // A condition nobody can decide is where the two differ most: `assume` constrains,
    // `assert` reports.
    assert_eq!(intrinsics::assume(None), IntrinsicOutcome::Constrain);
    assert!(matches!(
        intrinsics::assert_(None),
        IntrinsicOutcome::Finding(_)
    ));
}

/// `chiero_mark_fidelity` degrades deliberately, which is how a harness says "what
/// follows is approximate" without pretending otherwise.
#[test]
fn marking_fidelity_degrades_with_the_given_reason() {
    match intrinsics::mark_fidelity("the driver stubs out the PHY") {
        IntrinsicOutcome::Degrade(why) => assert!(why.contains("PHY")),
        other => panic!("expected a degradation, got {other:?}"),
    }
}

/// The builtins are registered, and every registered `Exact` name is implemented — the
/// invariant that stopped the earlier declaration-only list from claiming faithfulness
/// it did not have.
#[test]
fn the_builtins_are_registered_and_implemented() {
    let r = ModelRegistry::with_builtins();
    for n in [
        "__builtin_clz",
        "__builtin_ctz",
        "__builtin_popcount",
        "__builtin_bswap32",
        "__builtin_add_overflow",
        "__builtin_mul_overflow",
        "chiero_assume",
        "chiero_assert",
    ] {
        assert!(r.lookup(n).is_some(), "`{n}` is not registered");
        assert!(models::is_implemented(n), "`{n}` is registered but absent");
    }
}

/// **`strlen` must report the memory faults it provoked.** Every other model lifts them;
/// `strlen` read `r.value` and dropped `r.faults`, so:
///
/// - `strlen` over a `malloc`'d buffer answered **0** with no finding, because an
///   uninitialized byte reads back as `Some([0])` *plus* a fault — the model saw the zero
///   and called it a terminator;
/// - and it consumed the memory model's report-once memoization while discarding it, so a
///   later genuine read of those bytes was clean forever.
#[test]
fn strlen_reports_the_faults_its_reads_provoke() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let o = m.alloc(ObjKind::Heap, 16, 1, Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    let r = models::strlen(
        &mut cx,
        Pointer { base: o, off: 0 },
        StringPolicy::default(),
    );
    assert!(
        !cx.findings().is_empty(),
        "an uninitialized buffer has no string in it: {r:?}"
    );
    assert!(
        !matches!(r, StrScan::Exact(0)),
        "a zero that was never written is not a terminator"
    );
}

/// The same for a dead object: `strlen` on freed memory is a use-after-free, not an
/// unimplemented-feature note.
#[test]
fn strlen_on_freed_memory_reports_the_use_after_free() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let o = m.alloc(ObjKind::Heap, 16, 1, Span::DUMMY);
    m.write(Pointer { base: o, off: 0 }, b"abc\0", Span::DUMMY);
    m.free(o, Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    models::strlen(
        &mut cx,
        Pointer { base: o, off: 0 },
        StringPolicy::default(),
    );
    assert!(
        cx.findings().iter().any(|f| f.contains("UseAfterFree")),
        "{:#?}",
        cx.findings()
    );
}

/// **An `Unterminated` finding requires that the walk actually read something.** With
/// zero bytes of room — a NULL pointer, an offset past the end, a size-0 object — the
/// loop body never ran, yet the model asserted an out-of-bounds read. §4's rule cuts both
/// ways: claiming a bug you did not observe is inventing one.
#[test]
fn a_walk_that_reads_nothing_does_not_claim_an_unterminated_string() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let empty = m.alloc(ObjKind::Heap, 0, 1, Span::DUMMY);
    let small = m.alloc(ObjKind::Heap, 4, 1, Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    for (what, p) in [
        (
            "a null pointer",
            Pointer {
                base: chiero_mem::ObjectId::NULL,
                off: 0,
            },
        ),
        (
            "a zero-size object",
            Pointer {
                base: empty,
                off: 0,
            },
        ),
        (
            "an offset past the end",
            Pointer {
                base: small,
                off: 100,
            },
        ),
    ] {
        let before = cx.findings().len();
        let r = models::strlen(&mut cx, p, StringPolicy::default());
        assert!(
            !matches!(r, StrScan::Unterminated { .. }),
            "{what}: nothing was read, so nothing is known: {r:?}"
        );
        assert!(
            !cx.findings()[before..]
                .iter()
                .any(|f| f.contains("unterminated")),
            "{what}: {:#?}",
            &cx.findings()[before..]
        );
    }
}

/// **The scan cap actually bounds the walk.** The uniform-`'x'` fixture could not tell:
/// scanning 256 bytes or 1000 gives the same `CapReached { 256 }` and the same text. A
/// NUL past the cap distinguishes them — a model that ignored the cap would find it.
#[test]
fn the_cap_stops_the_walk_rather_than_only_labelling_it() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let o = m.alloc(ObjKind::Heap, 1000, 1, Span::DUMMY);
    m.set(Pointer { base: o, off: 0 }, b'x', 1000, Span::DUMMY);
    m.write(Pointer { base: o, off: 300 }, b"\0", Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    let r = models::strlen(
        &mut cx,
        Pointer { base: o, off: 0 },
        StringPolicy { max_scan: 256 },
    );
    assert!(
        matches!(r, StrScan::CapReached { .. }),
        "the NUL is at 300 and the cap is 256, so the walk must not reach it: {r:?}"
    );
}

/// `strlen` walks from the **pointer**, not the object base, and measures its room from
/// there. Every earlier fixture used offset 0, so reading from the base instead was
/// indistinguishable.
#[test]
fn strlen_walks_from_the_pointer_not_the_object_base() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let o = m.alloc(ObjKind::Heap, 16, 1, Span::DUMMY);
    m.write(Pointer { base: o, off: 0 }, b"abcdef\0", Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    match models::strlen(
        &mut cx,
        Pointer { base: o, off: 4 },
        StringPolicy::default(),
    ) {
        StrScan::Exact(n) => assert_eq!(n, 2, "\"ef\" is two bytes"),
        other => panic!("expected 2, got {other:?}"),
    }
}

/// **`calloc` of an unrepresentable size must not abort the process.** `Memory::set`
/// materialized the fill buffer before any guard, so `calloc(1, 1 << 45)` killed the run
/// — reintroducing exactly the abort `MAX_MATERIALIZED_BYTES` exists to prevent, one
/// level up from where the guard sits.
#[test]
fn calloc_of_an_enormous_size_is_a_fault_not_an_abort() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let mut cx = ctx(&mut m, &mut a);
    let out = models::calloc(&mut cx, 1, 1 << 45, AllocPolicy { may_fail: false });
    assert!(!cx.findings().is_empty(), "{out:?}");
}

/// A pointer **before** the object has no room to walk. Measuring room from `max(0, off)`
/// licensed a scan that started before the object and ran past its end, and the faults it
/// provoked were then misfiled as an unimplemented-feature note.
#[test]
fn strlen_before_the_object_is_reported_rather_than_walked() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let o = m.alloc(ObjKind::Heap, 8, 1, Span::DUMMY);
    m.write(Pointer { base: o, off: 0 }, b"abc\0", Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    let r = models::strlen(
        &mut cx,
        Pointer { base: o, off: -4 },
        StringPolicy::default(),
    );
    assert!(
        !matches!(r, StrScan::Exact(_)),
        "a pointer before the object has no string at it: {r:?}"
    );
    assert!(
        cx.findings()
            .iter()
            .any(|f| f.contains("before the object")),
        "{:#?}",
        cx.findings()
    );
}

/// **`strcpy` measures the destination's room from the *pointer*, not the object base.**
/// This is the identical `max(0)` mistake that was found and fixed in `strlen` — "measuring
/// from `max(0)` licensed a walk that started before the object" — left in place on
/// `strcpy`'s destination. At a negative offset `have` came out as the whole object, so a
/// ten-byte string into ten bytes of room *starting four before the object* reported
/// success. Worse than a missed finding: the outcome was `Value`, so the engine kept
/// `translated` and the run stayed **`Exact`** — the negative-offset overflow was less
/// degraded than the ordinary one. Found by review.
#[test]
fn strcpy_measures_destination_room_from_the_pointer_not_the_base() {
    let mut m = Memory::new();
    let mut a = TermArena::new();
    let dst = m.alloc(ObjKind::Heap, 10, 1, Span::DUMMY);
    let src = m.alloc(ObjKind::Heap, 16, 1, Span::DUMMY);
    m.write(Pointer { base: src, off: 0 }, b"012345678\0", Span::DUMMY);
    let mut cx = ctx(&mut m, &mut a);
    let out = models::strcpy(
        &mut cx,
        Pointer {
            base: dst,
            off: -4,
        },
        Pointer { base: src, off: 0 },
        StringPolicy::default(),
    );
    assert!(
        matches!(out, ModelOutcome::Finding(_)),
        "a copy that starts before the object is not a success: {out:?}"
    );
    assert!(
        cx.findings().iter().any(|f| f.contains("destination")),
        "reported at the destination: {:#?}",
        cx.findings()
    );
}
