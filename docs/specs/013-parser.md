# 013 — Parser

Translation phases 5–7: pp-tokens → tokens → AST.

## 1. Strategy

Hand-written recursive descent with operator-precedence expression parsing. Not a parser
generator: C's grammar needs context feedback (§3), error recovery must be good enough
to survive a 1M-line codebase, and every node must carry a provenance-bearing `Span`.

The parser is **error-tolerant by construction**. It never returns `Err` for a malformed
translation unit; it emits diagnostics, inserts `ExprKind::Error`/`StmtKind::Error`
nodes, and continues. A partial AST for a file chiero does not fully understand is worth
far more than a hard failure, because whole-tree analysis must degrade gracefully.

## 2. Phases 5 and 6

Before parsing:
- **Phase 5**: escape sequences in char/string literals → execution charset values.
  Target charset is UTF-8; `\x` and octal escapes are evaluated with target-dependent
  `char` signedness ([014](014-semantics-and-types.md)).
- **Phase 6**: adjacent string literals concatenate. The resulting node's `Span` covers
  all constituents, and each constituent's provenance is retained in a side list — VPP
  builds format strings by concatenating macro-produced fragments, and a diagnostic that
  cannot say *which fragment* came from *which macro* is not actionable.

## 3. The typedef problem

`A * B;` is a declaration if `A` is a typedef name and a multiplication otherwise. The
parser therefore requires a symbol table during parsing. `chiero-sema` owns scoping, so
the parser holds a `&mut dyn TypedefOracle`:

```rust
pub trait TypedefOracle {
    fn is_typedef_name(&self, sym: Symbol) -> bool;
    fn enter_scope(&mut self);
    fn exit_scope(&mut self);
    fn declare(&mut self, sym: Symbol, is_typedef: bool);
}
```

This is the one place where the clean phase separation is deliberately broken; C's
grammar leaves no alternative. The oracle interface is kept minimal so the parser still
knows nothing about types themselves.

Known hard cases that are pinned as contracts: a parameter named the same as a typedef
shadowing it for the rest of the declarator; `typedef int T; void f(int T, T x);` (the
second `T` is *not* a type); and old-style K&R parameter lists.

## 4. GNU extensions — the required set

Measured against `/home/ubuntu/vpp/src` at `7fe9c26`, counted by file:

| Extension | Files | Required |
|---|---|---|
| Designated initializers (`.field =`, `[i] =`) | 1019 | **yes** — C99, but pervasive |
| Zero/flexible array members `[0]` | 1165 | **yes** |
| `__attribute__((...))` | 155 | **yes** |
| `_Static_assert` (via VPP's `STATIC_ASSERT`) | 140 | **yes** |
| Statement expressions `({ ... })` | 217 | **yes** |
| `typeof` / `__typeof__` | 52 | **yes** |
| `asm` / `__asm__`, incl. `volatile` and extended operands | 31 | **parse, do not model** |
| `__builtin_*` | 30 | **yes** — see [024](024-environment-models.md) |
| `__restrict` | 6 | yes (ignorable qualifier) |
| Case ranges `case 1 ... 5:` | 7 | yes |
| `__int128` | 1 | yes |
| `__label__` / nested functions | 1 | **no** — diagnose and skip the function |
| `_Generic` | 0 | defer |
| `__extension__` | 0 | defer (trivial: skip the token) |
| `__auto_type` | 0 | defer |

**Attributes actually used**, in frequency order: `packed`/`__packed__` (112),
`unused` (85), `weak` (27), `aligned`/`__aligned__` (31), `constructor`/`__constructor__`
(33), `destructor`/`__destructor__` (18), `always_inline` (15), `fallthrough` (6),
`visibility` (3), `vector_size` (2), `section` (2), `may_alias` (2), `noinline` (2),
`warn_unused_result`, `used`.

Of these, only three change *analysis semantics* and must be interpreted rather than
recorded: `packed` and `aligned` (they change struct layout, so getting them wrong
corrupts every memory offset — see [014](014-semantics-and-types.md)), and `may_alias`.
`vector_size` introduces GCC vector types, needed by VPP's SIMD paths. The rest are
parsed, attached to the declaration, and otherwise ignored.

Attributes appear in positions the C grammar does not anticipate — after the declaration
specifiers, after the declarator, after `struct`/`union`/`enum` keywords, after the
closing brace, on individual struct members, and on parameters. The declarator parser
accepts an optional attribute list at each of these points rather than at a single one.

**`asm` is parsed but not modeled.** Basic and extended asm statements are parsed into
`StmtKind::Asm` with operands recorded. Lowering ([020](020-cir.md)) turns any asm
statement into an opaque effect that clobbers its outputs with fresh symbolic values and
marks the containing path `Fidelity::Approximated`. Attempting to model x86/ARM assembly
semantics is out of scope; silently treating asm as a no-op would be unsound in a way
that produces confident wrong answers, which is worse.

## 5. AST

Arena-allocated, ID-indexed, no `Rc`:

```rust
pub struct Ast {
    exprs: Vec<Expr>, stmts: Vec<Stmt>, decls: Vec<Decl>, types: Vec<TypeExpr>,
}
pub struct Expr { pub kind: ExprKind, pub span: Span }
```

Every node carries a `Span` with its `ExpnCtx`. A node synthesized by the parser (an
implicit conversion, an error node) uses a zero-width span at the relevant position with
`ExpnKind::Builtin`, never a fabricated range over unrelated source
([010 §4](010-source-and-provenance.md)).

The AST is **syntactic**. It records what was written, not what it means: no implicit
conversions inserted, no types resolved, no constant folding. All of that is
[014](014-semantics-and-types.md). This keeps the AST a faithful, printable record of
source — which is what change-impact analysis needs to diff entities.

## 6. Error recovery

Recovery points, in order of preference: statement boundary (`;`), block boundary (`}`),
top-level declaration boundary. The parser tracks brace depth and resynchronizes to the
nearest enclosing boundary.

A cap of **100 diagnostics per TU** stops cascade floods; beyond that the parser
continues silently and records that it truncated.

## 7. Performance

Target: **≥ 20 MB/s** of preprocessed token stream. Parsing is cheaper than
preprocessing, and preprocessed VPP TUs are large (a typical VPP `.c` file expands to
tens of MB of tokens after headers), so the practical constraint is memory, not CPU: the
AST for a preprocessed TU must stay under **10× the token stream size**.

## 8. Testable contracts

1. `typedef int T; T x;` parses `x` as a declaration; `int A; A * B;` parses as a
   multiplication expression statement.
2. `typedef int T; void f(int T, T x);` diagnoses — the second `T` is a parameter name,
   not a type.
3. A typedef declared in an inner scope does not affect parsing after `exit_scope`.
4. K&R style `int f(a, b) int a; int b; { return a+b; }` parses.
5. `struct S { int a[0]; };` and `struct S { int n; int a[]; };` both parse, with the
   member flagged as a zero-length/flexible array.
6. `struct __attribute__((packed)) S {...}`, `struct S {...} __attribute__((packed))`,
   and `int x __attribute__((aligned(64)));` all parse with the attribute attached to
   the right entity.
7. `int x = ({ int t = 1; t + 1; });` parses as a statement expression whose value is
   the last expression.
8. `typeof(x) y;` and `__typeof__(*p) z;` parse.
9. `switch (c) { case 1 ... 5: break; }` parses as a case range.
10. `asm volatile ("" ::: "memory");` and extended asm with operands parse into
    `StmtKind::Asm` without diagnostics.
11. Designated initializers `{ .a = 1, .b.c = 2, [3] = 4, [1 ... 2] = 5 }` all parse.
12. `_Static_assert(sizeof(int) == 4, "msg");` parses.
13. `__int128 x;` parses.
14. A nested function definition produces exactly one diagnostic and the enclosing
    function still parses.
15. A file with an unclosed brace produces diagnostics but still yields declarations for
    every complete top-level declaration preceding it.
16. Diagnostic count per TU never exceeds 100, and truncation is recorded.
17. Every AST node's span, mapped through `expansion_loc`, lands inside a real file.
18. Concatenated string literals retain per-fragment provenance: a literal formed from
    two macro-produced fragments reports two distinct `ExpnCtx`s.
19. Parsing every preprocessed TU in VPP produces zero panics; the count of TUs with
    diagnostics is tracked as a regression metric and must not increase.
20. AST memory for a preprocessed TU is under 10× the token stream size.
