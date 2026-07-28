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

`header_inline.c` (+ `pair.h`) graduated in wave 132, after five waves parked here. It took
**four** defects, and only the first was the one the file was filed under:

1. Wave 126: `struct pair p = make_pair(…)` stored the returned *pointer* into `p`'s slot
   instead of copying the struct, so `p.lo` read the low half of an address as an `int`.
2. Waves 126–131 chased a wild pointer at `0x700000003` through the engine, the call ABI
   and scope balance, all of which turned out to be innocent. The cause was in lowering:
   a local aggregate read as a value emitted `load ptr`, so `return p;` copied *from* the
   struct's own bytes `{3, 7}` used as an address.
3. Three sites asked `raw_width_of` for an lvalue's CIR type, which answers 32 for anything
   that is not an integer — so every pointer not held in a plain local was stored and
   loaded as an `i32`.
4. A `struct` **parameter** got a slot of its lowered `CTy` — eight bytes of `CTy::Ptr` —
   and the prologue stored the caller's address into it, so `span_of(p)` read fields out of
   a pointer. C11 6.9.1p9 makes a parameter a *copy*; it is an alloca of the struct's size
   and a `CopyMem` now.

Each has a gcc-differential test in `crates/chiero-lower/tests/differential.rs`, because a
fixture parked here covers nothing and the rule above is not a formality.
