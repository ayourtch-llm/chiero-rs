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

What remains is **in sema, not lowering**: `int (*fn)(int)` is typed as an integer, so the
slot is declared `Int(32)` and storing the (now correct) `Ptr` into it fails verification
with `store value operand is Ptr, declared Int(32)`. `cty` maps `Ty::Ptr(_)` to `CTy::Ptr`
correctly, so the wrong answer is upstream of it — sema does not build a pointer type for a
function-pointer declarator.

Move this file back into `tests/corpus/c/`, bless its golden, and read it.
