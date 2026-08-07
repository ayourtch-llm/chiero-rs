# 014 — Semantics and types

`chiero-sema` turns a syntactic AST into a typed, resolved program: name resolution,
type checking, layout computation, and constant evaluation. It is the last frontend
stage before CIR.

Layout correctness is the load-bearing part. Every symbolic memory offset
([021](021-memory-model.md)) derives from a struct layout computed here; a one-byte
error in a struct offset produces confident, wrong answers throughout the entire system.
Layout is therefore validated differentially against the real compiler (§7).

## 1. Target model

All target-dependent behavior is data, not code:

```rust
pub struct TargetConfig {
    pub pointer_width: u32,           // 64
    pub char_signed: bool,            // true on x86-64 Linux, false on aarch64
    pub sizes:  IntSizes,             // short 2, int 4, long 8, long long 8
    pub aligns: IntAligns,
    pub endian: Endian,               // Little
    pub long_double: LongDoubleKind,  // X87_80 on x86-64
    pub enum_underlying: EnumRule,    // gcc: int unless values require wider
    pub bitfield_abi: BitfieldAbi,    // gcc x86-64 rules
    pub cache_line_bytes: u32,        // 64 on x86-64, 128 on some aarch64 (VPP: CLIB_CACHE_LINE_BYTES)
}
```

`cache_line_bytes` has **no semantic effect** — caches are coherent, so no load's value
depends on them, and the memory model ignores the field entirely
([021 §7](021-memory-model.md)). It is here because struct layout is the only place the
number is knowable, and the locality analysis in
[041](041-optimization-analysis.md) consumes it: cache-line straddling, hot/cold field
placement, and false sharing are layout properties, and VPP tunes them deliberately.

Default target is `x86_64-unknown-linux-gnu`. VPP also builds for `aarch64`, where
`char_signed` flips — a difference that silently changes the sign of comparisons in
analysis results, so the target must be recorded on every result alongside the
`ConfigId` ([012 §3.3](012-preprocessor.md)).

## 2. Types

```rust
pub enum Ty {
    Void,
    Int  { signed: bool, bits: u32 },      // incl. _Bool(1), __int128(128)
    Float(FloatKind),                      // F32, F64, X87_80
    Ptr  { pointee: TyId, quals: Quals },
    Array{ elem: TyId, len: ArrayLen },    // Fixed(u64) | Flexible | Zero | Vla(ExprId)
    Func { ret: TyId, params: Vec<TyId>, variadic: bool, kr_style: bool },
    Record(RecordId),                      // struct or union
    Enum(EnumId),
    Vector { elem: TyId, lanes: u32 },     // __attribute__((vector_size))
    Atomic(TyId),
    Error,                                 // poison; suppresses cascading diagnostics
}
```

Types are interned; `TyId` equality is type identity after canonicalization (typedefs
resolved, qualifiers normalized to a side channel). `Ty::Error` propagates so a single
bad declaration does not produce a thousand diagnostics.

## 3. Layout

Computed per record, cached:

```rust
pub struct RecordLayout {
    pub size: u64, pub align: u64,
    pub fields: Vec<FieldLayout>,   // byte offset, or bit offset+width for bitfields
    pub is_union: bool,
    pub flexible_member: Option<usize>,
    pub packed: bool,
}
```

Rules follow the gcc x86-64 ABI. The cases that must be right:

- **`packed`** removes internal padding and sets member alignment to 1. VPP uses it 112
  times, predominantly on wire-format structs where a wrong offset means every parsed
  packet field is wrong.
- **`aligned(n)`** raises alignment; combined with `packed` it does *not* re-introduce
  padding before the member.
- **Bitfields**: gcc's allocation order (little-endian: from the least significant bit),
  straddling rules, zero-width bitfields forcing the next allocation unit, and the
  interaction with `packed` — historically the buggiest area of any layout
  implementation, and pinned by contracts 3–6.
- **Zero-length and flexible arrays** contribute 0 to size but do affect alignment.
  1165 VPP files use `[0]`.
- **Unions** size to the max member, aligned to max alignment.

## 4. Name resolution

C's five namespaces are modeled separately: ordinary identifiers, struct/union/enum
tags, member names (per record), statement labels (per function), and macro names
(owned by `chiero-pp`).

Scopes: file, block, function-prototype, function (labels only). The parser's
`TypedefOracle` ([013 §3](013-parser.md)) is implemented by the same scope stack so
parsing and resolution never disagree.

**Linkage and tentative definitions** matter for cross-TU work: `int x;` at file scope
is a tentative definition, `extern int x;` is a declaration, and multiple TUs must
resolve to one object. A cross-TU symbol table maps `(name, linkage)` to a single
`GlobalId`, which is what lets the call graph in [031](031-change-impact.md) span TUs.

`static` functions with the same name in different TUs are distinct entities and must
not be merged — a real hazard in VPP, where short static helper names repeat across
nodes.

## 5. Type checking

Full C11 conversion rules: integer promotions, usual arithmetic conversions, array/
function decay, null pointer constants, qualifier compatibility.

The output is a **fully explicit** typed AST: every implicit conversion becomes an
explicit `Cast` node with its own `Span`. Lowering must never have to infer a
conversion, because an implicit conversion that lowering gets wrong is an invisible
semantic bug. Making conversions explicit here means CIR is unambiguous about
bit-widths, which is what the solver needs.

Type errors are diagnostics; the node's type becomes `Ty::Error` and analysis continues.

## 6. Constant evaluation

Needed for array bounds, enum values, bitfield widths, `_Static_assert`, case labels,
and static initializers.

```rust
pub enum ConstVal { Int(i128, TyId), Float(f64, TyId), Addr { global: GlobalId, off: i64 },
                    Aggregate(Vec<ConstVal>), Str(StringId) }
```

Address constants matter: `&arr[3]` and `(char*)&s + offsetof(S, f)` are valid static
initializers and appear throughout VPP's node registration tables.

`__builtin_constant_p` is evaluated here and returns true only when the argument folds
to a `ConstVal` — matching gcc's behavior closely enough is required because VPP uses it
in macros that select between implementations.

Integer overflow in a constant expression is a diagnostic (UB), and evaluation continues
with wrapped values.

## 7. Differential validation against the real compiler

Layout and constant evaluation are validated against gcc, and — now that it is being
installed — clang, rather than against hand-written expectations. The harness emits, for
each record type in a corpus:

```c
_Static_assert(sizeof(S)   == <chiero's answer>, "size");
_Static_assert(_Alignof(S) == <chiero's answer>, "align");
_Static_assert(offsetof(S, f) == <chiero's answer>, "off f");
```

and compiles it. A mismatch is a compile error naming the exact field. This turns the
entire layout engine into a property test against ground truth, and it scales: the same
generator can be pointed at **every record type in VPP**, giving thousands of assertions
for free. Contract 12 makes that a gate.

The same technique validates `#if` arithmetic ([012](012-preprocessor.md)) by comparing
against `gcc -E` / `clang -E` output.

## 8. Testable contracts

1. `struct { char a; int b; }` → size 8, align 4, offsets 0 and 4.
2. `struct __attribute__((packed)) { char a; int b; }` → size 5, offsets 0 and 1.
3. `struct { int a:3; int b:5; }` → size 4, with `b` at bit offset 3.
4. `struct { int a:3; int :0; int b:5; }` → `b` starts at the next allocation unit.
4a. **An unnamed bit-field contributes no alignment.** `struct { char c; unsigned :0;
    char d; }` → size 5, align 1, `d` at offset 4; `struct { char c; unsigned :4; char
    d; }` → size 3, align 1. The declared type still sets the allocation unit either
    way — give the same field a name and `struct { char c; unsigned n:4; char d; }` is
    4/4, which is the discriminator. Applying the unit's alignment to both inflated
    every record with an unnamed bit-field.
4b. **A record that declares a `:0` says so** (`has_zero_width_bitfield`), because
    `fields` cannot hold one: it declares no member, and C 6.7.9 has initializers skip
    unnamed bit-fields while the initializer check indexes `fields` positionally. Its
    effect survives only as a gap in its neighbours' offsets, which a consumer cannot
    tell from alignment padding — and [041 §3.1](041-optimization-analysis.md) needs
    exactly that distinction, since this gap is not recoverable by any reorder.
5. A bitfield that would straddle an allocation unit boundary is placed per gcc rules,
   verified by `_Static_assert` against gcc.
6. `struct { char a; int b:24; }` unpacked vs packed differ, both matching gcc.
7. `struct { int n; int a[]; }` → size 4, flexible member recorded, align 4.
8. `union { char a[7]; int b; }` → size 8, align 4.
9. `char` is signed under the x86-64 target config and unsigned under aarch64, and
   `(char)0xFF == -1` only in the former.
10. `enum { A = 0x100000000 }` widens the underlying type beyond `int`.
11. Every implicit conversion in the corpus appears as an explicit `Cast` node in the
    typed AST.
12. **Gate**: for every record type reachable in VPP's `compile_commands.json` that
    chiero can parse, generated `_Static_assert`s for size, alignment and every field
    offset compile cleanly under gcc. Failures are counted and must be zero.
13. `_Static_assert(sizeof(int) == 4, "")` in a corpus file passes; a false one produces
    exactly one diagnostic.
14. `int x; int x;` at file scope is accepted (tentative definitions); `int x = 1; int x = 2;`
    is one diagnostic.
15. Two TUs each defining `static void helper(void)` produce two distinct `GlobalId`s.
16. Two TUs referencing `extern int foo;` and one defining `int foo;` resolve to one
    `GlobalId`.
17. `&arr[3]` as a static initializer evaluates to `ConstVal::Addr` with offset
    `3 * sizeof(elem)`.
18. `__builtin_constant_p(1+1)` is 1; `__builtin_constant_p(argc)` is 0.
19. Signed overflow in a constant expression produces one diagnostic and a wrapped value.
20. A type error produces `Ty::Error` and does not cascade: one bad declaration yields
    exactly one diagnostic regardless of how many times the name is later used.
