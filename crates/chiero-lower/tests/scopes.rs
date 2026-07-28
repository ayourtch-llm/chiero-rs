//! Covers: 015 contracts 9, 9b, 10, 11, 18.
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

/// **Contract 11.** `return` from inside three nested scopes emits three `Scope(Exit)`
/// markers, **before** the `Return`.
#[test]
fn return_from_three_scopes_exits_all_three_first() {
    let m = probe("{ int a = 1; { int b = 2; { int c = 3; return c; } } }");
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
        3,
        "three open scopes, three exits before the `Return`: {:#?}",
        returning.insts.iter().map(|i| &i.kind).collect::<Vec<_>>()
    );
    assert!(check_scope_balance(f).is_empty());
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
