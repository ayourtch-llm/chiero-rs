# Corpus fixtures chiero cannot yet lower

Real C, written to the same standard as `tests/corpus/c/`, that chiero refuses. They live
here rather than in the corpus so the suite stays green, and rather than being deleted so
the gap stays visible and the fixture is ready the day it closes.

Each records the **diagnosis**, not just the symptom.

**A fixture parked here covers nothing.** The suite does not run it, so any fix it motivates
still needs a test where the suite can see it — wave 120 shipped two lowering fixes whose
only exercise was a file in this directory, and mutation caught that they were untested.

## (empty)

`indirect_call.c` graduated to `tests/corpus/c/` in wave 121. It took four defects to get
there — three in lowering (waves 119–120: a call through a declared variable reported as
"undeclared"; a function name lowering to `Undef` rather than `AddrOfFunc`; the `?:` result
slot hardcoded to `CTy::Int`) and one in the engine (wave 121: `direct_into` never bound the
callee's parameters). Wave 119's diagnosis blamed sema and was wrong; the tests written to
prove it are kept in `chiero-sema/tests/function_pointers.rs`.
