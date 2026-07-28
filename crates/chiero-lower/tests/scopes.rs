//! Covers: 015 contracts 9, 9b, 9c, 10, 11, 18.
//!
//! 015 §4 calls scope markers "the most error-prone part of lowering", and the reason is
//! that a missed marker in *either* direction is a false finding later rather than a
//! crash: 021 §4 creates stack objects on `Scope(Enter)` and retires them on `Scope(Exit)`,
//! so a missing enter makes every access on that path a wild access, and a missing exit
//! keeps dead objects alive across the rest of the function.
//!
//! Contract 9b is the one to read first. Contracts 9–11 test scope **exits** only, so an
//! implementation that never *enters* a scope on a `switch` case path passes every one of
//! them — and that is not an exotic construct, it is any `switch` with a local.

use chiero_cir::{BlockId, Function, MarkerKind, ScopeEvent, ScopeKind, Terminator};

mod harness;
use harness::lower;

/// Every `Scope(Enter)`/`Scope(Exit)` pair on every path, checked by walking the CFG.
///
/// Paths through a loop are infinite, so the walk is bounded by visiting each
/// (block, scope stack) pair once — a loop whose body is balanced reaches its header with
/// the same stack and stops, and one that is not balanced reaches it with a different
/// stack and is reported.
fn check_scope_balance(f: &Function) -> Vec<String> {
    let mut errors = Vec::new();
    let mut seen: Vec<(BlockId, Vec<u32>)> = Vec::new();
    let mut work = vec![(f.entry, Vec::<u32>::new())];
    let mut steps = 0usize;

    while let Some((id, stack)) = work.pop() {
        steps += 1;
        if steps > 20_000 {
            errors.push("the walk did not converge".into());
            break;
        }
        if seen.contains(&(id, stack.clone())) {
            continue;
        }
        seen.push((id, stack.clone()));
        let Some(b) = f.block(id) else { continue };

        let mut stack = stack;
        for i in &b.insts {
            let chiero_cir::InstKind::Marker(MarkerKind::Scope(ScopeEvent { scope, kind })) =
                &i.kind
            else {
                continue;
            };
            match kind {
                ScopeKind::Enter => stack.push(scope.0),
                ScopeKind::Exit => match stack.pop() {
                    Some(top) if top == scope.0 => {}
                    Some(top) => errors.push(format!(
                        "block {id:?}: exit of scope {} while scope {top} was innermost",
                        scope.0
                    )),
                    None => errors.push(format!(
                        "block {id:?}: exit of scope {} with no scope open",
                        scope.0
                    )),
                },
            }
        }

        match &b.term {
            Terminator::Return(_) => {
                if !stack.is_empty() {
                    errors.push(format!(
                        "block {id:?} returns with {stack:?} still open — 015 §3 requires \
                         a `Scope(Exit)` for every open scope before a `Return`"
                    ));
                }
            }
            Terminator::Goto(g) => work.push((*g, stack)),
            Terminator::Br { t, f: fl, .. } => {
                work.push((*t, stack.clone()));
                work.push((*fl, stack));
            }
            Terminator::Switch { cases, default, .. } => {
                for (_, b) in cases {
                    work.push((*b, stack.clone()));
                }
                work.push((*default, stack));
            }
            Terminator::IndirectGoto { targets, .. } => {
                for t in targets {
                    work.push((*t, stack.clone()));
                }
            }
            Terminator::Unreachable(_) => {}
        }
    }
    errors
}

fn scope_events(f: &Function) -> Vec<(u32, ScopeKind)> {
    f.blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| match &i.kind {
            chiero_cir::InstKind::Marker(MarkerKind::Scope(ScopeEvent { scope, kind })) => {
                Some((scope.0, *kind))
            }
            _ => None,
        })
        .collect()
}

fn probe(body: &str) -> chiero_cir::Module {
    lower(&format!("int probe(int n) {{ {body} }}"))
}

/// **Contract 9.** Every `Scope(Enter)` has a matching `Scope(Exit)` on every path that
/// leaves the scope — including exits by `break`, `continue`, `goto` out, and `return`.
#[test]
fn scopes_balance_on_every_path_including_the_abrupt_ones() {
    for body in [
        "{ int a = 1; return a; }",
        "if (n) { int a = 1; return a; } return 0;",
        "while (n) { int a = 1; if (a) break; } return 0;",
        "while (n) { int a = 1; if (a) continue; n--; } return 0;",
        "for (int i = 0; i < n; i++) { int a = i; if (a > 2) break; } return 0;",
        "{ int a = 1; { int b = 2; if (b) goto out; } } out: return 0;",
        "{ int a = 1; { int b = 2; { int c = 3; return c; } } }",
        "switch (n) { case 1: { int a = 1; return a; } default: return 0; }",
    ] {
        let m = probe(body);
        let f = m.funcs.iter().find(|f| &*f.name == "probe").expect("probe");
        let errors = check_scope_balance(f);
        assert!(
            errors.is_empty(),
            "`{body}` leaves scopes unbalanced:\n{}",
            errors.join("\n")
        );
        assert!(
            !scope_events(f).is_empty(),
            "`{body}` emitted no scope markers at all, so the balance check is vacuous"
        );
    }
}

/// **Contract 9b.** `switch (x) { int y; case 1: y = 1; … }` materializes the scope's
/// objects on the case path: the jump to `case 1` carries a `Scope(Enter)`.
///
/// This is the contract that catches what 9–11 structurally cannot. The `Switch`
/// terminator jumps **past** the lexical top of the compound statement, so a lowering that
/// emits the enter marker at the top only never runs it — 021 §4 then never creates the
/// scope's objects, the eventual exit retires objects that never existed, and every access
/// on the case path is a wild access or a false use-after-scope. Any `switch` with a local
/// has this shape.
#[test]
fn a_switch_case_path_enters_the_scope_it_jumps_into() {
    let m = probe("switch (n) { int y; case 1: y = 1; return y; default: return 0; }");
    let f = m.funcs.iter().find(|f| &*f.name == "probe").expect("probe");

    // The balance check is the sharp end: reaching the `case 1` block without an enter
    // means the `return y` path exits a scope it never entered.
    let errors = check_scope_balance(f);
    assert!(
        errors.is_empty(),
        "the case path must enter the switch body's scope:\n{}",
        errors.join("\n")
    );

    // And specifically: the block the `Switch` jumps to for case 1 is preceded by an
    // enter, reached from the switch itself rather than by falling in from above.
    let entry_blocks: Vec<BlockId> = f
        .blocks
        .iter()
        .filter_map(|b| match &b.term {
            Terminator::Switch { cases, .. } => cases.first().map(|(_, t)| *t),
            _ => None,
        })
        .collect();
    assert_eq!(entry_blocks.len(), 1, "one switch");
    let target = f.block(entry_blocks[0]).expect("the case block");
    assert!(
        target.insts.iter().any(|i| matches!(
            &i.kind,
            chiero_cir::InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                kind: ScopeKind::Enter,
                ..
            }))
        )),
        "the case block itself enters the scope: {:#?}",
        target.insts.iter().map(|i| &i.kind).collect::<Vec<_>>()
    );
}

/// **Contract 10.** `goto` out of two nested scopes emits two `Scope(Exit)` markers,
/// **innermost first**.
///
/// The order is the contract, not just the count: 021 retires objects in the order the
/// markers arrive, and retiring an outer scope before an inner one frees storage the
/// inner scope's objects are still inside.
#[test]
fn goto_out_of_two_scopes_exits_innermost_first() {
    let m = probe("{ int a = 1; { int b = 2; goto out; } } out: return 0;");
    let f = m.funcs.iter().find(|f| &*f.name == "probe").expect("probe");

    // The block containing the `goto` carries both exits, in order.
    let exits: Vec<u32> = f
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| match &i.kind {
            chiero_cir::InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                scope,
                kind: ScopeKind::Exit,
            })) => Some(scope.0),
            _ => None,
        })
        .collect();
    assert!(
        exits.len() >= 2,
        "two scopes are left, so two exits: {exits:?}"
    );
    let enters: Vec<u32> = f
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| match &i.kind {
            chiero_cir::InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                scope,
                kind: ScopeKind::Enter,
            })) => Some(scope.0),
            _ => None,
        })
        .collect();
    // The inner scope is entered second, so it must be exited first.
    let inner = *enters.last().expect("an inner scope");
    assert_eq!(
        exits[0], inner,
        "innermost first: entered {enters:?}, exited {exits:?}"
    );
    assert!(check_scope_balance(f).is_empty());
}

/// **Contract 11.** `return` from inside nested scopes emits a `Scope(Exit)` for **every**
/// open scope, before the `Return`.
#[test]
fn return_from_three_scopes_exits_all_three_first() {
    // **Four scopes, not three.** The function body's own compound, two nested inside it,
    // and — enclosing all of them — the scope the *parameters* live in (C11 6.2.1p4).
    //
    // That fourth one arrived in wave 109 and is the whole point of it: parameters used to
    // share `ScopeId(0)` with the body's compound, so entering the body replaced every
    // parameter slot *after* the prologue had stored into it, and every function that read
    // its own parameter reported an uninitialized read. A `return` has to leave it too.
    let m = probe("int a = 1; { int b = 2; { int c = 3; return c; } }");
    let f = m.funcs.iter().find(|f| &*f.name == "probe").expect("probe");

    let returning = f
        .blocks
        .iter()
        .find(|b| matches!(b.term, Terminator::Return(_)) && !b.insts.is_empty())
        .expect("a returning block");
    let exits = returning
        .insts
        .iter()
        .filter(|i| {
            matches!(
                &i.kind,
                chiero_cir::InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                    kind: ScopeKind::Exit,
                    ..
                }))
            )
        })
        .count();
    assert_eq!(
        exits,
        4,
        "four open scopes — the parameter scope, the body's, and two nested — so four \
         exits before the `Return`: {:#?}",
        returning.insts.iter().map(|i| &i.kind).collect::<Vec<_>>()
    );
    assert!(check_scope_balance(f).is_empty());
}

/// **Contract 9c, first half.** A `goto` *into* a nested scope enters it — exactly once,
/// and outermost first when it enters more than one.
///
/// This is the mirror of contract 10's exit rule and it exists for the same reason: 021 §4
/// creates a scope's objects on `Scope(Enter)`, so a jump that lands inside a scope
/// without entering it leaves every object there unmaterialized. C permits the jump
/// (C11 6.8.6.1), so "nobody writes that" is not an answer.
#[test]
fn a_goto_into_a_scope_enters_it_exactly_once() {
    let m = probe("if (n) goto inner; { int a = 1; inner: return a; } return 0;");
    let f = m.funcs.iter().find(|f| &*f.name == "probe").expect("probe");
    assert!(
        check_scope_balance(f).is_empty(),
        "the jump enters the scope it lands in:\n{}",
        check_scope_balance(f).join("\n")
    );

    // The block holding the `goto` carries the enter, and carries it once.
    let goto_blocks: Vec<&chiero_cir::Block> = f
        .blocks
        .iter()
        .filter(|b| {
            b.insts.iter().any(|i| {
                matches!(
                    &i.kind,
                    chiero_cir::InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                        kind: ScopeKind::Enter,
                        ..
                    }))
                )
            })
        })
        .collect();
    for b in &goto_blocks {
        let mut seen: Vec<u32> = Vec::new();
        for i in &b.insts {
            if let chiero_cir::InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                scope,
                kind: ScopeKind::Enter,
            })) = &i.kind
            {
                assert!(
                    !seen.contains(&scope.0),
                    "block {:?} enters scope {} twice",
                    b.id,
                    scope.0
                );
                seen.push(scope.0);
            }
        }
    }

    // Two levels: the jump enters both, **outermost first** — the mirror of contract
    // 10's exit order, and for the same reason: an inner scope's objects live inside the
    // outer one's storage, so creating them in the other order has nowhere to put them.
    let m = probe("if (n) goto deep; { int a = 1; { int b = 2; deep: return a + b; } } return 0;");
    let f = m.funcs.iter().find(|f| &*f.name == "probe").expect("probe");
    assert!(
        check_scope_balance(f).is_empty(),
        "{}",
        check_scope_balance(f).join("\n")
    );
    let jump = f
        .blocks
        .iter()
        .find(|b| {
            b.insts
                .iter()
                .filter(|i| {
                    matches!(
                        &i.kind,
                        chiero_cir::InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                            kind: ScopeKind::Enter,
                            ..
                        }))
                    )
                })
                .count()
                == 2
        })
        .expect("one block enters two scopes: the jump");
    let entered: Vec<u32> = jump
        .insts
        .iter()
        .filter_map(|i| match &i.kind {
            chiero_cir::InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                scope,
                kind: ScopeKind::Enter,
            })) => Some(scope.0),
            _ => None,
        })
        .collect();
    assert!(
        entered[0] < entered[1],
        "outermost first: scopes are numbered in the order they are opened, so the outer \
         one has the lower id — got {entered:?}"
    );
}

/// **Contract 9c, second half.** A **backward** `goto` that re-enters an already-entered
/// scope creates a *new generation* of its objects.
///
/// 015 §4 says this matches the loop-body rule in §3, and the CIR shape is the same: the
/// re-entering edge carries a `Scope(Enter)`, so 021 retires the old objects and creates
/// new ones rather than reusing storage the program has left and re-entered.
#[test]
fn a_backward_goto_re_enters_and_starts_a_new_generation() {
    // `inner: ;` — a label must precede a *statement*, and a declaration is not one in
    // C11 (C23 relaxed it). The null statement is what makes the fixture legal C, which
    // matters because the differential oracle compiles these with gcc.
    let m = probe("{ inner: ; int a = n; n = n - 1; } if (n > 0) goto inner; return n;");
    let f = m.funcs.iter().find(|f| &*f.name == "probe").expect("probe");
    assert!(
        check_scope_balance(f).is_empty(),
        "re-entry is balanced:\n{}",
        check_scope_balance(f).join("\n")
    );

    // The scope is entered from **two** places: falling in at the top, and the backward
    // jump. One enter would mean the second pass reuses objects the first pass retired.
    let inner_scope = f
        .allocas
        .iter()
        .find(|a| a.name.as_deref() == Some("a"))
        .expect("`a`'s slot")
        .scope;
    let enters = f
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter(|i| {
            matches!(
                &i.kind,
                chiero_cir::InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                    scope,
                    kind: ScopeKind::Enter,
                })) if *scope == inner_scope
            )
        })
        .count();
    assert_eq!(
        enters, 2,
        "the scope holding `a` is entered on both edges that reach it — the fallthrough \
         and the backward `goto`"
    );
}

/// **Contract 18.** `switch` with fallthrough lowers to blocks chained by `Goto`, and a
/// `case` range of 4 expands to 4 sorted cases.
#[test]
fn switch_falls_through_by_goto_and_expands_case_ranges() {
    let m = probe(
        "int t = 0; switch (n) { case 1: t = 1; case 2: t += 2; break; default: t = 9; } return t;",
    );
    let f = m.funcs.iter().find(|f| &*f.name == "probe").expect("probe");
    let (cases, _default) = f
        .blocks
        .iter()
        .find_map(|b| match &b.term {
            Terminator::Switch { cases, default, .. } => Some((cases.clone(), *default)),
            _ => None,
        })
        .expect("a switch terminator");
    assert_eq!(
        cases.iter().map(|(v, _)| *v).collect::<Vec<_>>(),
        vec![1, 2],
        "two cases, sorted"
    );
    // Fallthrough: `case 1`'s block ends in a `Goto` to `case 2`'s, not in a break.
    let one = cases[0].1;
    let two = cases[1].1;
    assert_eq!(
        f.block(one).expect("case 1").term,
        Terminator::Goto(two),
        "`case 1` falls through to `case 2` by `Goto`"
    );

    // A range of four expands to four sorted cases.
    let m = probe("switch (n) { case 3 ... 6: return 1; default: return 0; }");
    let f = m.funcs.iter().find(|f| &*f.name == "probe").expect("probe");
    let cases = f
        .blocks
        .iter()
        .find_map(|b| match &b.term {
            Terminator::Switch { cases, .. } => Some(cases.clone()),
            _ => None,
        })
        .expect("a switch terminator");
    assert_eq!(
        cases.iter().map(|(v, _)| *v).collect::<Vec<_>>(),
        vec![3, 4, 5, 6],
        "`case 3 ... 6` is four cases, sorted — a range kept as one entry would make the \
         engine take the default for 4 and 5"
    );
}

/// **A function that falls off the end still exits its parameter scope.**
///
/// Every other fixture in this file returns, and a `return` already emits an exit for each
/// open scope on its way out — so the explicit exit after the body is only reachable when
/// control reaches the closing brace. A `void` function with no `return` is that case, and
/// it is extremely common.
///
/// An unexited scope is not cosmetic: 021 retires stack objects on `Scope(Exit)`, so a
/// scope that never closes is a set of objects that never die, and use-after-scope stops
/// being detectable for everything the function owned.
#[test]
fn a_function_falling_off_the_end_exits_its_parameter_scope() {
    let m = lower("void f(int n) { int t = n; }");
    let f = m.funcs.iter().find(|f| &*f.name == "f").expect("f");
    assert!(
        check_scope_balance(f).is_empty(),
        "{:#?}",
        check_scope_balance(f)
    );

    // And the parameter scope's exit is really there — balance alone is satisfied by a
    // function that never *entered* it either.
    let (enters, exits) = f
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| match &i.kind {
            chiero_cir::InstKind::Marker(MarkerKind::Scope(ScopeEvent { scope, kind })) => {
                Some((*scope, *kind))
            }
            _ => None,
        })
        .fold((0, 0), |(e, x), (_, k)| match k {
            ScopeKind::Enter => (e + 1, x),
            ScopeKind::Exit => (e, x + 1),
        });
    assert!(enters >= 2, "the parameter scope and the body's: {enters}");
    assert_eq!(enters, exits, "every scope entered is left");
}

/// **A function ending in `return` does not exit its scopes twice.**
///
/// `return` unwinds every open scope on its way out (015 §3), and wave 109 added a trailing
/// `exit_scope` after the body so a function *falling off the end* still closes its
/// parameter scope. A body ending in `return` gets both, so the callee of
/// `tests/corpus/owed/header_inline.c` ends with `.scope exit 1`, `.scope exit 0`,
/// `.scope exit 1`, `.scope exit 0`.
///
/// It is not cosmetic: 021 retires stack objects on `Scope(Exit)`, so a scope exited twice
/// retires its objects twice — and wave 128 narrowed a wild pointer in an aggregate return
/// to exactly this.
#[test]
fn a_returning_function_exits_each_scope_once() {
    let m = lower("int f(int n) { int t = n; return t; }");
    let f = m.funcs.iter().find(|x| &*x.name == "f").expect("f");

    let events: Vec<(u32, ScopeKind)> = f
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| match &i.kind {
            chiero_cir::InstKind::Marker(MarkerKind::Scope(ScopeEvent { scope, kind })) => {
                Some((scope.0, *kind))
            }
            _ => None,
        })
        .collect();

    for id in events
        .iter()
        .map(|(s, _)| *s)
        .collect::<std::collections::BTreeSet<_>>()
    {
        let enters = events
            .iter()
            .filter(|(s, k)| *s == id && *k == ScopeKind::Enter)
            .count();
        let exits = events
            .iter()
            .filter(|(s, k)| *s == id && *k == ScopeKind::Exit)
            .count();
        assert_eq!(
            enters, exits,
            "scope {id} is entered {enters} time(s) and exited {exits}: {events:?}"
        );
    }
    assert!(
        check_scope_balance(f).is_empty(),
        "{:#?}",
        check_scope_balance(f)
    );
}

/// **And an aggregate-returning function too**, which is the shape wave 128 traced. Its
/// `return` carries a `CopyMem` before the unwind, so the two paths differ.
#[test]
fn an_aggregate_returning_function_exits_each_scope_once() {
    let m = lower(
        "struct pair { int lo; int hi; };\n\
         static struct pair mk(int a) { struct pair p; p.lo = a; p.hi = a; return p; }\n",
    );
    let f = m.funcs.iter().find(|x| &*x.name == "mk").expect("mk");
    let events: Vec<(u32, ScopeKind)> = f
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter_map(|i| match &i.kind {
            chiero_cir::InstKind::Marker(MarkerKind::Scope(ScopeEvent { scope, kind })) => {
                Some((scope.0, *kind))
            }
            _ => None,
        })
        .collect();
    let exits = events.iter().filter(|(_, k)| *k == ScopeKind::Exit).count();
    let enters = events
        .iter()
        .filter(|(_, k)| *k == ScopeKind::Enter)
        .count();
    assert_eq!(enters, exits, "one exit per enter, not two: {events:?}");
}
