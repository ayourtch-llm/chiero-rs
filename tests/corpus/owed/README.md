# Corpus fixtures chiero cannot yet lower

Real C, written to the same standard as `tests/corpus/c/`, that chiero refuses. They live
here rather than in the corpus so the suite stays green, and rather than being deleted so
the gap stays visible and the fixture is ready the day it closes.

Each records the **diagnosis**, not just the symptom.

**A fixture parked here covers nothing.** The suite does not run it, so any fix it motivates
still needs a test where the suite can see it — wave 120 shipped two lowering fixes whose
only exercise was a file in this directory, and mutation caught that they were untested.

## `varargs.c` — a variadic function

020 §4.4.1's `VaArg` is implemented and no corpus file exercised it. VPP's `format` and
`vlib_cli_output` paths are all variadic.

Wave 123 fixed two defects it exposed, both of which hit **every variadic function in C**:

- sema reported `__builtin_va_start` / `_va_arg` / `_va_end` as *undeclared*. `stdarg.h` is
  `#define va_start(v,l) __builtin_va_start(v,l)` and nothing declares the target — gcc
  knows it intrinsically.
- lowering treated them as calls, so 015 §7 refused the whole enclosing function. They are
  **instructions** (§4.4.1: `VaArg` mutates, so it is not an `RValue`).

The file now lowers and its golden reads `vastart`, `%23 = vaarg %22, i32`, `vaend`.

What remains is **sema sizing `__builtin_va_list`**:

    out-of-bounds: 8-byte access at offset 0 of ap, which is 1 bytes

`va_list ap` gets a one-byte object. On x86-64 `__builtin_va_list` is `__va_list_tag[1]` —
24 bytes: two `unsigned int` offsets, then `overflow_arg_area` and `reg_save_area`
pointers. Sema needs a builtin type for it with the target's layout, which is also what
lets `va_list *` cross a function boundary the way §4.4.1 requires.

Move this file back into `tests/corpus/c/`, bless, and read the golden.

`indirect_call.c` graduated to `tests/corpus/c/` in wave 121. It took four defects to get
there — three in lowering (waves 119–120: a call through a declared variable reported as
"undeclared"; a function name lowering to `Undef` rather than `AddrOfFunc`; the `?:` result
slot hardcoded to `CTy::Int`) and one in the engine (wave 121: `direct_into` never bound the
callee's parameters). Wave 119's diagnosis blamed sema and was wrong; the tests written to
prove it are kept in `chiero-sema/tests/function_pointers.rs`.
