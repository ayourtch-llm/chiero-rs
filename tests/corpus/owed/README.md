# Corpus fixtures chiero cannot yet lower

Real C, written to the same standard as `tests/corpus/c/`, that chiero refuses. They live
here rather than in the corpus so the suite stays green, and rather than being deleted so
the gap stays visible and the fixture is ready the day it closes.

Each records the **diagnosis**, not just the symptom.

**A fixture parked here covers nothing.** The suite does not run it, so any fix it motivates
still needs a test where the suite can see it — wave 120 shipped two lowering fixes whose
only exercise was a file in this directory, and mutation caught that they were untested.

## (empty)

`varargs.c` graduated to `tests/corpus/c/` in wave 124. It took three defects — sema
reporting `__builtin_va_*` undeclared, lowering treating them as calls rather than 020
§4.4.1's instructions, and `__builtin_va_list` having no size (a sentinel `Array` of length
zero, under a comment stating the 24 bytes it did not produce).
