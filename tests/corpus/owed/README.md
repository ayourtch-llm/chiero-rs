# Corpus fixtures chiero cannot yet lower

These are real C, written to the same standard as `tests/corpus/c/`, that chiero refuses.
They live here rather than in the corpus so the suite stays green, and rather than being
deleted so the gap stays visible and the fixture is ready the day it closes.

Each one records the **diagnosis**, not just the symptom.

## `indirect_call.c` — a call through a function pointer

VPP dispatches every graph node this way.

Wave 119 fixed two lowering defects it exposed:

- a call whose callee is a *declared variable* was reported as "call to undeclared
  function", which 015 §7 turns into refusing the whole enclosing function;
- a bare function name used as a value lowered to `Undef` rather than `AddrOfFunc`
  (C11 6.3.2.1p4: a function designator decays to a pointer).

Wave 120 fixed a third and **corrected wave 119's diagnosis, which was wrong**:

- the `?:` operator's result slot was hardcoded `CTy::Int`, so `pick ? twice : thrice`
  stored a `Ptr` into an `Int(32)` slot and failed verification.

Wave 119 blamed sema for that. **Sema is right** — `chiero-sema/tests/function_pointers.rs`
types `int (*fn)(int)` as a pointer at file scope *and* as a local, and all six of its tests
passed on arrival. The wrong answer was made in lowering, one `?:` away from where anyone
looked. Those tests are kept: they were written to catch a defect that turned out not to
exist, and they now pin behaviour nothing else did.

The file lowers and **verifies** as of wave 120. What remains is in the **engine**: an
indirect call does not bind the callee's parameters, so `twice`/`thrice` read `v` as
uninitialized —

    uninitialized-read: read at offset 0 of v touches bit 0, which was never written

Two of them, one per resolved callee, so the indirect dispatch itself works and the argument
does not arrive. Start at `Callee::Indirect` in the engine's call handling and compare what
it binds against the `Callee::Direct` path.

Move this file back into `tests/corpus/c/`, bless its golden, and read it.
