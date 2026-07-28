# Corpus fixtures chiero cannot yet lower

Real C, written to the same standard as `tests/corpus/c/`, that chiero refuses. They live
here rather than in the corpus so the suite stays green, and rather than being deleted so
the gap stays visible and the fixture is ready the day it closes.

Each records the **diagnosis**, not just the symptom.

**A fixture parked here covers nothing.** The suite does not run it, so any fix it motivates
still needs a test where the suite can see it — wave 120 shipped two lowering fixes whose
only exercise was a file in this directory, and mutation caught that they were untested.

## `header_inline.c` (+ `pair.h`) — a struct returned by value

A `static inline` helper in a real header returning a `struct` by value. Every VPP accessor
is shaped this way, and 030 attributes the helper's lines to the *header* — which
`gcov_lines.rs` tests in isolation and nothing runs end to end.

Wave 126 fixed one defect it exposed: `struct pair p = make_pair(…)` **stored the returned
pointer** into `p`'s slot instead of copying the struct, so `p.lo` read the low half of an
address as an `int`. The program ran and every field was wrong. It is a `CopyMem` now
(015 c6: one copy of the *layout's* size).

What remains is that **an aggregate return has nowhere to live**. `return p;` in the callee
yields `addrlocal` of the callee's own stack slot, whose scope exits on return — so the
caller copies from bytes that are already dead:

    uninitialized-read: read at offset 0 of p touches bit 0, which was never written through p.lo

015 §2 says an aggregate return is memory; it needs to be memory the *caller* owns. The
usual shape is an sret slot: the caller allocates, passes its address as a hidden first
argument, and the callee writes through it instead of returning a pointer. That touches
lowering's call and return paths and the engine's frame setup together.

Note the finding names `p.lo` — wave 110's `AccessPath`s and wave 111's naming, doing
exactly what they were built for.

Move both files back into `tests/corpus/c/`, bless, and read the golden.

## (empty)

`varargs.c` graduated to `tests/corpus/c/` in wave 124. It took three defects — sema
reporting `__builtin_va_*` undeclared, lowering treating them as calls rather than 020
§4.4.1's instructions, and `__builtin_va_list` having no size (a sentinel `Array` of length
zero, under a comment stating the 24 bytes it did not produce).
