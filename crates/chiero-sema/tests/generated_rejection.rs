//! **The mirror of `generated_silence.rs`: what sema *fails* to reject** (014 §7).
//!
//! That channel asserts a program gcc accepts is silent, which catches false positives. This one
//! asks the opposite question — of the programs gcc *rejects*, how many does sema reject too —
//! and it cannot be a pass/fail gate, because plenty of C's constraints are genuinely unchecked
//! and pretending otherwise would mean either a failing suite or a dishonest assertion.
//!
//! So it is a **ratchet**. The number caught is measured and compared against a floor recorded
//! here; the test fails if coverage *drops*. Raising the floor is a deliberate edit that a wave
//! makes when it adds a rule, which turns "we check more of C than we used to" from a feeling
//! into a number in a diff.
//!
//! **Every violation is one gcc confirms.** A generator that believes a program illegal is worth
//! nothing on its own — four censuses in this project turned on cases where my reading of C and
//! gcc's disagreed — so a program gcc *accepts* is dropped from the denominator rather than
//! counted as a miss. `-pedantic-errors`, because wave 314 established that half of C's
//! constraint violations are warnings by default.

mod harness;

use chiero_sema::TargetConfig;

/// One violation: a name for the report, and a program that commits it.
///
/// **Named, because the count alone is not actionable.** A ratchet that says "37 of 52" tells the
/// next wave nothing about what to fix; the failing list is the work queue, and 023 §9's rule —
/// a report a person cannot act on is not a report — applies to a test's output too.
const VIOLATIONS: &[(&str, &str)] = &[
    // Constraint census rows 1-3 (wave 308).
    (
        "member absent",
        "struct S { int m; }; int f(void){ struct S s; return s.nope; }",
    ),
    (
        "member of non-struct",
        "int f(void){ int x = 5; return x.m; }",
    ),
    (
        "subscript non-array",
        "int f(void){ int x = 5; return x[0]; }",
    ),
    ("call non-function", "int f(void){ int q = 5; return q(); }"),
    // Rows 4-7 (wave 311).
    (
        "write const object",
        "int f(void){ const int k = 1; k = 2; return k; }",
    ),
    (
        "increment const object",
        "int f(void){ const int k = 1; k++; return k; }",
    ),
    ("void object", "int f(void){ void w; return 0; }"),
    (
        "void value used",
        "void v(void); int f(void){ return v(); }",
    ),
    (
        "incomplete parameter",
        "struct T; int s(struct T t){ return 0; }",
    ),
    // Rows 8-12 (wave 312).
    (
        "duplicate case",
        "int f(int n){ switch(n){ case 1: return 1; case 1: return 2; } return 0; }",
    ),
    (
        "duplicate default",
        "int f(int n){ switch(n){ default: return 1; default: return 2; } return 0; }",
    ),
    ("break outside loop", "int f(void){ break; return 0; }"),
    (
        "continue outside loop",
        "int f(void){ continue; return 0; }",
    ),
    ("undefined label", "int f(void){ goto nowhere; return 0; }"),
    // Rows 13-16 (wave 313).
    (
        "function redefined",
        "int f(void){ return 0; } int f(void){ return 1; }",
    ),
    ("conflicting types", "int h(int); int h(long){ return 0; }"),
    ("static after non-static", "extern int n; static int n;"),
    ("non-static after static", "static int n; int n;"),
    ("function returns array", "int f(void)[3];"),
    // Initializers (wave 314).
    ("excess array elements", "int a[3] = {1,2,3,4};"),
    (
        "excess struct elements",
        "struct S { int x, y; }; struct S s = {1,2,3};",
    ),
    ("excess scalar elements", "int x = {1,2};"),
    ("string too long", "char s[3] = \"abcd\";"),
    ("designator out of range", "int a[3] = {[5] = 1};"),
    (
        "unknown field designator",
        "struct S { int x, y; }; struct S s = {.nope = 1};",
    ),
    ("non-constant initializer", "int f(void); int g = f();"),
    (
        "excess vector elements",
        "typedef int v4 __attribute__((vector_size(16))); v4 v = {1,2,3,4,5};",
    ),
    ("elided excess", "int a[2][2] = {1,2,3,4,5};"),
    ("read object at file scope", "int x; int g = x;"),
    // Conversions (wave 315).
    ("int to pointer", "int f(void){ int *p = 1; return *p; }"),
    ("pointer to int", "int f(int *p){ int x = p; return x; }"),
    (
        "unrelated pointers assigned",
        "int f(int *p){ char *q = p; return *q; }",
    ),
    (
        "unrelated pointer argument",
        "void g(int *); int f(char *q){ g(q); return 0; }",
    ),
    ("pointer returned as int", "int f(int *p){ return p; }"),
    (
        "unrelated pointers compared",
        "int f(int *p, char *q){ return p == q; }",
    ),
    (
        "struct pointer to int pointer",
        "struct S { int a; }; int f(struct S *s){ int *p = s; return *p; }",
    ),
    (
        "function pointer mismatch",
        "int g(int); int f(void){ int (*fp)(long) = g; return (int)fp(1); }",
    ),
    (
        "discard const",
        "int f(const int *cp){ int *p = cp; return *p; }",
    ),
    // Qualified types (wave 328).
    (
        "discard volatile",
        "int f(volatile int *vp){ int *p = vp; return *p; }",
    ),
    (
        "discard const through void *",
        "int f(const void *cp){ void *p = cp; return p != 0; }",
    ),
    (
        "discard const through &",
        "int f(void){ const int x = 0; int *p = &x; return *p; }",
    ),
    (
        "discard const through an argument",
        "void g(int *); int f(const int *cp){ g(cp); return 0; }",
    ),
    (
        "qualified array typedef, element const",
        "typedef int arr[3]; int f(void){ const arr a = {1,2,3}; int *p = a; return *p; }",
    ),
    (
        "discard const through array decay",
        "int f(void){ const int a[3] = {1,2,3}; int *p = a; return *p; }",
    ),
    (
        "qualifier mismatch below the outermost pointee",
        "int f(int **pp){ const int **cpp = pp; return **cpp; }",
    ),
    (
        "qualifier mismatch below, even adding const",
        "int f(int **pp){ const int *const *cpp = pp; return **cpp; }",
    ),
    (
        "address of a member of a const struct",
        "struct S { int m; }; int f(const struct S *s){ int *p = &s->m; return *p; }",
    ),
    (
        "write a const member",
        "struct S { const int m; }; int f(struct S *s){ s->m = 1; return 0; }",
    ),
    // `switch` (wave 319).
    (
        "switch on double",
        "int f(double d){ switch(d){ case 1: return 1; } return 0; }",
    ),
    (
        "switch on pointer",
        "int f(int *p){ switch(p){ case 0: return 1; } return 0; }",
    ),
    (
        "case not constant",
        "int f(int n, int m){ switch(n){ case m: return 1; } return 0; }",
    ),
    (
        "case not integer",
        "int f(int n){ switch(n){ case 1.5: return 1; } return 0; }",
    ),
    ("case outside switch", "int f(int n){ case 1: return n; }"),
    (
        "default outside switch",
        "int f(int n){ default: return n; }",
    ),
    // Bit-fields (wave 302) and completeness (waves 303, 320).
    (
        "bit-field width non-constant",
        "int nc; struct S { int a; int f : nc; };",
    ),
    (
        "bit-field width negative",
        "struct S { int a; int f : -1; };",
    ),
    (
        "bit-field width too wide",
        "struct S { int a; int f : 33; };",
    ),
    (
        "named zero-width bit-field",
        "struct S { int a; int f : 0; };",
    ),
    ("object of incomplete type", "struct I; struct I x;"),
    ("array of incomplete element", "struct I; struct I arr[10];"),
    (
        // **In a function, not a file-scope initializer.** Written the second way this row
        // exercised *two* constraints — the arithmetic and a non-constant initializer — and both
        // sentences were true, so it reported twice and the contract-20 channel below flagged it.
        // A row that tests one rule has to contain one mistake.
        "arithmetic on incomplete pointee",
        "struct I; int f(struct I *p){ return (int)(long)(p + 1); }",
    ),
    (
        "sizeof incomplete",
        "struct I; int f(void){ return (int)sizeof(struct I); }",
    ),
    (
        "deref incomplete pointee",
        "struct I; int f(struct I *p){ (*p); return 0; }",
    ),
    (
        "member through incomplete pointee",
        "struct I; int f(struct I *p){ return p->m; }",
    ),
    // Wave 327: the shapes around the VLA rule that must stay rejected, including the one with
    // no block at all — the case that shows it is about crossing a declaration, not entering a
    // block.
    (
        "goto into a VLA scope, no block",
        "int f(int n){ goto skip; int a[n]; skip: return 0; }",
    ),
    // Rules nothing checks yet — the ones this channel exists to keep visible.
    (
        "assignment to array",
        "int f(void){ int a[2], b[2]; a = b; return a[0]; }",
    ),
    (
        "too few arguments",
        "static int g(int a, int b){ return a+b; } int f(void){ return g(1); }",
    ),
    (
        "too many arguments",
        "static int g(int a){ return a; } int f(void){ return g(1,2); }",
    ),
    (
        "return value from void function",
        "static void v(void){ return 1; }",
    ),
    ("duplicate struct member", "struct S { int m; int m; };"),
    (
        "duplicate parameter name",
        "static int g(int a, int a){ return a; }",
    ),
    (
        "goto into a VLA scope",
        "int f(int n){ goto skip; { int a[n]; skip: return 0; } }",
    ),
    (
        "address of a register",
        "int f(void){ register int x = 0; return *&x; }",
    ),
    ("negative array length", "int a[-1];"),
    // Wave 329's census: the C 6.5 operator constraints, closed in the same wave.
    (
        "increment a non-lvalue",
        "int f(void){ int x = 1; return x++++; }",
    ),
    (
        "increment an enumeration constant",
        "int f(void){ enum E { A }; return A++; }",
    ),
    (
        "sizeof a function",
        "void g(void); int f(void){ return sizeof(g); }",
    ),
    (
        "unary minus on a pointer",
        "int f(void){ int x = 1; return -&x != 0; }",
    ),
    (
        "bit-complement on a double",
        "int f(double d){ return (int)~d; }",
    ),
    (
        "void value as an operand",
        "int f(void){ void *p = 0; return *p != 0; }",
    ),
    // ...and its C 6.7 declaration rows, which wave 329 did **not** close. Left here on purpose:
    // this channel exists to keep a known gap visible, and a violation nobody has written a rule
    // for is exactly what it is for. They are the next wave's queue.
    (
        "multiple storage classes",
        "int f(void){ static extern int x; return x; }",
    ),
    (
        "variably modified at file scope",
        "const int k = 1; int a[k];",
    ),
    (
        "duplicate enumerator",
        "enum E { A = 1, A = 2 }; int f(void){ return A; }",
    ),
    (
        "multiple storage classes on a function",
        "static extern int g(void);",
    ),
    (
        "variably modified with static storage",
        "const int k = 1; int f(void){ static int a[k]; return a[0]; }",
    ),
    // The `extern` half of "static storage duration", which wave 358 found untested when a
    // mutant dropped `extern` from the predicate and nothing in the suite noticed.
    (
        "variably modified with external linkage",
        "int f(int n){ extern int a[n]; return a[0]; }",
    ),
    (
        "variably modified member",
        "const int k = 1; struct S { int a[k]; };",
    ),
    (
        "enumerator shared by two enums",
        "enum E { A = 1 }; enum F { A = 2 }; int f(void){ return A; }",
    ),
    // Wave 330 found these while probing the three above: nothing checked them.
    (
        "struct redefined",
        "struct S { int m; }; struct S { int m; };",
    ),
    ("union redefined", "union U { int m; }; union U { int m; };"),
    // Wave 331's census.
    ("empty declaration", "int;"),
    ("anonymous struct declaring nothing", "struct { int m; };"),
    ("anonymous union declaring nothing", "union { int m; };"),
    (
        "empty declaration in a block",
        "int f(void){ int; return 0; }",
    ),
    (
        "scalar initializer for a struct",
        "struct S { int a; }; struct S s = 1;",
    ),
    (
        "scalar initializer for a union",
        "union U { int a; }; union U u = 1;",
    ),
    // Wave 332: what a prototype promises.
    (
        "argument to a (void) function",
        "int g(void); int f(void){ return g(1); }",
    ),
    (
        "argument to a defined (void) function",
        "static int g(void){ return 1; } int f(void){ return g(1); }",
    ),
    ("(void) then (int)", "int f(void); int f(int);"),
    ("(int) then (void)", "int f(int); int f(void);"),
    (
        "void as the first of several parameters",
        "int g(void, int);",
    ),
    (
        "void as the last of several parameters",
        "int g(int, void);",
    ),
    // Wave 333.
    ("typedef with static", "typedef static int T;"),
    ("typedef with extern", "typedef extern int T;"),
    ("typedef with _Thread_local", "typedef _Thread_local int T;"),
    // Wave 335's lexer census — C 6.4.4's constant constraints.
    ("invalid integer suffix", "int f(void){ return 1z; }"),
    ("mixed-case ll suffix", "int f(void){ return (int)1Ll; }"),
    (
        "invalid floating suffix",
        "int f(void){ return (int)1.0z; }",
    ),
    (
        "integer suffix on a float",
        "int f(void){ return (int)1.0u; }",
    ),
    ("exponent with no digits", "int f(void){ return (int)1e; }"),
    (
        "hex float with no exponent",
        "int f(void){ return (int)0x1.8; }",
    ),
    (
        "hex constant with no digits",
        "int f(void){ return (int)0x; }",
    ),
    ("invalid octal digit", "int f(void){ return 018; }"),
    (
        "integer constant too large",
        "int f(void){ return (int)99999999999999999999999; }",
    ),
    // Wave 337 — C 6.4.4.4's escape-sequence constraints.
    ("empty character constant", "int f(void){ return \'\'; }"),
    ("unknown escape sequence", "const char *s = \"\\q\";"),
    ("hex escape with no digits", "const char *s = \"\\x\";"),
    ("octal escape out of range", "const char *s = \"\\777\";"),
    ("hex escape out of range", "const char *s = \"\\x100\";"),
    (
        "incomplete universal character name",
        "const char *s = \"\\u41\";",
    ),
    // Wave 338's parser census — C 6.7.2's specifier sets and 6.7.2.1's members.
    ("two data types", "int int x;"),
    ("both signednesses", "signed unsigned x;"),
    ("long on a float", "long float x;"),
    ("three longs", "long long long x;"),
    ("long and short", "short long x;"),
    ("modifier on _Bool", "unsigned _Bool x;"),
    (
        "flexible array member not last",
        "struct S { int a[]; int b; };",
    ),
    ("member declaring nothing", "struct S { ; };"),
    // Wave 339's census — C 6.5.3.2, 6.8.6.4 and 6.9.1.
    ("address of an rvalue", "int f(void){ return *&(1+2); }"),
    (
        "address of a bit-field",
        "struct S { int b : 3; }; int f(struct S *s){ int *p = &s->b; return *p; }",
    ),
    (
        "dereference of a non-pointer",
        "int f(void){ int x = 1; return *x; }",
    ),
    ("return with no value", "int f(void){ return; }"),
    ("initialized function", "int x(void) = 1;"),
    // Wave 342's audit found a rule, not just a wording.
    (
        "_Bool bit-field wider than one bit",
        "struct S { _Bool b : 2; };",
    ),
    // Wave 343: the spellings of pointer arithmetic the message claimed but did not cover.
    (
        "increment a pointer to an incomplete type",
        "struct I; int f(struct I *p){ p++; return p != 0; }",
    ),
    (
        "compound-add a pointer to an incomplete type",
        "struct I; int f(struct I *p){ p += 1; return p != 0; }",
    ),
    (
        "subscript a pointer to an incomplete type",
        "struct I; int f(struct I *p){ return p[0] != 0; }",
    ),
    // Wave 346: an address constant reads no object.
    (
        "initializer through a pointer object",
        "struct S { int m; } s; struct S *p = &s; int *g = &p->m;",
    ),
    ("initializer dereferencing", "int x; int g = *&x;"),
    (
        "initializer with a variable subscript",
        "int a[3]; int i; int *g = &a[i];",
    ),
    // Wave 347: the contexts where `void` escaped the size question.
    ("array of void", "void a[3];"),
    ("void member", "struct S { void m; };"),
    (
        "definition returning an incomplete type",
        "struct I; struct I f(void){ }",
    ),
    // Wave 348: a pointer compared with an integer.
    (
        "pointer compared with a _Bool",
        "int f(int *p, _Bool b){ return p == b; }",
    ),
    (
        "pointer compared with an int",
        "int f(int *p, int i){ return p == i; }",
    ),
    (
        "pointer compared with a non-zero constant",
        "int f(int *p){ return p == 1; }",
    ),
    (
        "pointer ordered against zero",
        "int f(int *p){ return p > 0; }",
    ),
    // Wave 351: a malformed constant expression where one is required.
    ("division by zero in an initializer", "int g = 1/0;"),
    ("division by zero in an array length", "int a[1/0];"),
    // Wave 352's storage-class grid.
    ("auto at file scope", "auto int x;"),
    ("register at file scope", "register int x;"),
    (
        "static in a for initializer",
        "int f(void){ for (static int i = 0; i < 2; i++) ; return 0; }",
    ),
    ("static parameter", "int f(static int a){ return a; }"),
    ("register function", "register int f(void){ return 1; }"),
    ("auto function", "auto int f(void){ return 1; }"),
    // Wave 353: the other two contexts whose cascades wave 351 closed, so the contract-20
    // channel below guards them by name rather than only through the array one.
    (
        "division by zero in a case label",
        "int f(int n){ switch(n){ case 1/0: return 1; } return 0; }",
    ),
    (
        "division by zero in a bit-field width",
        "struct S { int b : 1/0; };",
    ),
    // Wave 355: a compound literal is initialized like an object.
    (
        "excess elements in a compound literal",
        "int f(void){ return (int){1,2}; }",
    ),
    (
        "excess elements in a struct compound literal",
        "struct S { int a; }; int f(void){ return (struct S){1,2}.a; }",
    ),
    (
        "compound literal of incomplete type",
        "int f(void){ return (struct Undefined){1}.a; }",
    ),
    (
        "variably modified compound literal",
        "int f(int n){ return (int[n]){1}[0]; }",
    ),
    // Wave 356: a designator list descends.
    ("nested index out of range", "int a[2][2] = {[0][5] = 1};"),
    ("outer index out of range", "int a[2][2] = {[5][0] = 1};"),
    (
        "nested field designator names nothing",
        "struct P { int x, y; }; struct Q { struct P p; }; struct Q q = {.p.nope = 1};",
    ),
    (
        "member designator on an array element",
        "struct P { int x, y; }; struct P a[2] = {[1].nope = 3};",
    ),
    (
        "remainder on a floating operand",
        "int f(double d){ d %= 2; return (int)d; }",
    ),
    // **Wave 350's case-range rules are deliberately absent.** gcc rejects `case 1 ... 3` under
    // `-pedantic-errors` because ISO C has no case ranges at all, so a row here would be counted
    // as caught for a reason that is not the one under test — the overlap. This list's contract is
    // that every entry is a violation gcc confirms, and it cannot confirm this one. The rules are
    // held by `a_case_range_occupies_every_value_in_it`, which is calibrated to GNU mode.
    // C 6.7.2.2, wave 357. The range rule is calibrated to `-pedantic-errors` — gcc takes
    // these under `-std=gnu11` and widens — and the implicit-successor row is the one a
    // rule that only inspected written initializers would miss.
    (
        "enumerator past int",
        "enum E { A = 2147483648 }; int f(void){ return (int)A; }",
    ),
    (
        "enumerator below int",
        "enum E { A = -2147483649 }; int f(void){ return (int)A; }",
    ),
    (
        "implicit successor past int",
        "enum E { A = 2147483647, B }; int f(void){ return (int)B; }",
    ),
    // Refused by gcc in *both* modes, unlike the three above.
    (
        "enumeration with no enumerators",
        "enum E { }; int f(void){ return 0; }",
    ),
    // C 6.7.2.1 and 6.7.9, wave 358. All refused by gcc in both modes.
    ("bit-field of floating type", "struct S { float a:3; };"),
    ("bit-field of pointer type", "struct S { int *a:3; };"),
    (
        "bit-field of struct type",
        "struct S { struct T { int x; } a:3; };",
    ),
    // The incomplete spelling, which used to draw a second sentence about incompleteness.
    (
        "bit-field of incomplete struct type",
        "struct S { struct I a:3; };",
    ),
    (
        "sizeof a bit-field",
        "struct S { int a:3; }; int f(void){ struct S s; return (int)sizeof(s.a); }",
    ),
    (
        "block-scope static initialized from a global",
        "static int g = 1; int f(void){ static int x = g; return x; }",
    ),
    (
        "block-scope static initialized from a parameter",
        "int f(int n){ static int x = n; return x; }",
    ),
    (
        "block-scope static holding the address of an automatic",
        "int f(void){ int x; static int *p = &x; return *p; }",
    ),
    // C 6.7.6.3 and 6.7.3, wave 359. The ellipsis row is `-pedantic-errors`-calibrated; the
    // rest are refused by gcc in both modes.
    (
        "duplicate parameter in a declaration",
        "int f(int x, int x);",
    ),
    ("ellipsis with no named parameter", "int f(...);"),
    ("restrict on a non-pointer", "int restrict x;"),
    (
        "restrict on a non-pointer through a typedef",
        "typedef int T; T restrict x;",
    ),
    ("restrict on an array of non-pointers", "int restrict a[2];"),
    ("enum tag defined twice", "enum E { A }; enum E { B };"),
    (
        "assignment to an array",
        "int f(void){ int a[2]; a = 0; return a[0]; }",
    ),
    // C 6.2.1 and the six paragraphs that ask for a scalar, wave 360. All refused by gcc in
    // both modes.
    ("duplicate label", "int f(int x){ a: a: return x; }"),
    (
        "duplicate label across a block",
        "int f(int x){ a: { a: return x; } }",
    ),
    (
        "struct as an `if` condition",
        "struct S{int a;}; int f(void){ struct S s; if(s) return 1; return 0; }",
    ),
    (
        "struct as a `while` condition",
        "struct S{int a;}; int f(void){ struct S s; while(s) ; return 0; }",
    ),
    (
        "struct as a `?:` condition",
        "struct S{int a;}; int f(void){ struct S s; return s ? 1 : 0; }",
    ),
    (
        "struct as the operand of `!`",
        "struct S{int a;}; int f(void){ struct S s; return !s; }",
    ),
    (
        "struct as an operand of `&&`",
        "struct S{int a;}; int f(void){ struct S s; return s && 1; }",
    ),
    (
        "`void` as an `if` condition",
        "void g(void); int f(void){ if(g()) return 1; return 0; }",
    ),
    // C 6.5.2.3 and 6.7.4, wave 361. The member rows are refused by gcc in both modes; the
    // specifier rows on objects are `-pedantic-errors` calibration, the one on a member is not.
    (
        "`->` on a structure",
        "struct S{int a;}; int f(void){ struct S s; return s->a; }",
    ),
    (
        "`.` on a pointer",
        "struct S{int a;}; int f(void){ struct S *p=0; return p.a; }",
    ),
    (
        "`.` on a pointer behind a typedef",
        "typedef struct S{int a;} *SP; int f(void){ SP p=0; return p.a; }",
    ),
    ("`inline` on a file-scope object", "inline int x;"),
    ("`_Noreturn` on a file-scope object", "_Noreturn int x;"),
    (
        "`inline` on a block-scope object",
        "int f(void){ inline int y=1; return y; }",
    ),
    ("`inline` on a parameter", "int f(inline int x);"),
    ("`inline` on a typedef", "typedef inline int T;"),
    ("`inline` on a member", "struct S{ inline int a; };"),
    // C 6.5.4 and 6.5.5, wave 362. All refused by gcc in both modes.
    (
        "cast of a structure to a scalar",
        "struct S{int a;}; int f(void){ struct S s; return (int)s; }",
    ),
    (
        "cast to a structure type",
        "struct S{int a;}; int f(void){ struct S s; return (int)(struct S)s.a; }",
    ),
    (
        "cast of a floating value to a pointer",
        "int f(void){ double d=1; return (int)(int*)d; }",
    ),
    (
        "cast of a pointer to a floating type",
        "int f(void){ int *p=0; return (double)p != 0; }",
    ),
    (
        "pointer operand of `*`",
        "int f(void){ int *p=0; return (int)(p * 2); }",
    ),
    (
        "pointer operand of `/`",
        "int f(void){ int *p=0; return (int)(p / 2); }",
    ),
    (
        "floating operand of `%`",
        "int f(void){ double d=2; return (int)(d % 2); }",
    ),
    (
        "pointer operands of `%`",
        "int f(void){ int *p=0; int *q=0; return (int)(p % q); }",
    ),
    // C 6.7p3, wave 363. All refused by gcc in both modes.
    ("typedef then object", "typedef int T; int T;"),
    ("object then typedef", "int T; typedef int T;"),
    (
        "typedef redefined with another type",
        "typedef int T; typedef long T;",
    ),
    (
        "typedef and object in one block",
        "int f(void){ typedef int T; int T; return 0; }",
    ),
    (
        "object declared twice in one block",
        "int f(void){ int x; int x; return x; }",
    ),
    ("function then typedef", "int f(void); typedef int f;"),
    ("enumerator then object", "enum E { A }; int A;"),
    ("object then enumerator", "int A; enum E { A };"),
    (
        "parameter then typedef",
        "int f(int T){ typedef int T; return 0; }",
    ),
    // C 6.5.6 and 6.5.9, wave 364. All refused by gcc in both modes.
    (
        "two pointers added",
        "int f(void){ int *p=0; int *q=0; return (int)(p+q); }",
    ),
    (
        "an integer minus a pointer",
        "int f(void){ int *p=0; return (int)(1-p); }",
    ),
    (
        "a pointer offset by a floating value",
        "int f(void){ double d=1; int *p=0; return (int)(p+d); }",
    ),
    (
        "pointers to incompatible types subtracted",
        "int f(void){ int *p=0; char *q=0; return (int)(p-q); }",
    ),
    (
        "a structure added to an integer",
        "struct S{int a;}; int f(void){ struct S s; return (int)(s+1); }",
    ),
    (
        "structures compared",
        "struct S{int a;}; int f(void){ struct S s; struct S t; return s == t; }",
    ),
    (
        "unions compared",
        "union U{int a;}; int f(void){ union U u; union U v; return u == v; }",
    ),
    // C 6.5.2.1, 6.5.3.1 and 6.7.5, wave 365. All refused by gcc in both modes.
    (
        "a floating subscript",
        "int f(void){ int a[2]; double d=0; return a[d]; }",
    ),
    (
        "a pointer subscript",
        "int f(void){ int a[2]; int *p=0; return a[p]; }",
    ),
    (
        "a structure incremented",
        "struct S{int a;}; int f(void){ struct S s; s++; return s.a; }",
    ),
    (
        "an array incremented",
        "int f(void){ int a[2]; a++; return a[0]; }",
    ),
    (
        "a function incremented",
        "void g(void); int f(void){ g++; return 0; }",
    ),
    ("`_Alignas` on a parameter", "int f(_Alignas(8) int x);"),
    ("`_Alignas` on a typedef", "typedef _Alignas(8) int T;"),
    (
        "an alignment that is not a power of two",
        "_Alignas(3) int x;",
    ),
    ("an alignment weaker than the type", "_Alignas(1) int x;"),
    // C 6.5.1.1 and 6.7.2.1, wave 366. All refused by gcc in both modes.
    (
        "`_Generic` association of `void` type",
        "int f(void){ return _Generic(1, void: 1, default: 0); }",
    ),
    (
        "`_Generic` association of incomplete type",
        "struct I; int f(void){ return _Generic(1, struct I: 1, default: 0); }",
    ),
    (
        "`_Generic` association of function type",
        "int f(void){ return _Generic(1, int(void): 1, default: 0); }",
    ),
    (
        "`_Generic` association variably modified",
        "int f(int n){ return _Generic(1, int[n]: 1, default: 0); }",
    ),
    ("a lone flexible array member", "struct S { int a[]; };"),
    (
        "a flexible array member in a union",
        "union U { int a; int b[]; };",
    ),
    // C 6.7.2.1 and 6.4.5, wave 367. All refused by gcc in both modes.
    (
        "a member of function type",
        "struct S { int f(void); int a; };",
    ),
    (
        "a member of function type through a typedef",
        "typedef int F(void); struct S { F a; };",
    ),
    (
        "an anonymous member colliding with a sibling",
        "struct S { struct { int a; }; int a; };",
    ),
    (
        "two anonymous members sharing a name",
        "struct S { struct { int a; }; struct { int a; }; };",
    ),
    (
        "string literals with different prefixes",
        "int f(void){ return (int)sizeof(u\"a\" U\"b\"); }",
    ),
    // C 6.7.9p6, wave 368. All refused by gcc in both modes.
    (
        "designator naming nothing the record shows",
        "struct S{ union { int a; int b; }; }; int f(void){ struct S s = { .c = 1 }; return s.a; }",
    ),
    (
        "designator naming a member of a named nested record",
        "struct S{ struct { int a; } n; }; int f(void){ struct S s = { .a = 1 }; return s.n.a; }",
    ),
    (
        "negative initializer index",
        "int f(void){ int a[2] = { [-1] = 1 }; return a[0]; }",
    ),
    (
        "negative initializer index under a member",
        "struct S{int a[2];}; int f(void){ struct S s = { .a[-1] = 1 }; return s.a[0]; }",
    ),
    // C 6.5.7 and 6.5.10-6.5.12, wave 371. All refused by gcc in both modes.
    (
        "floating left operand of `<<`",
        "int f(void){ double d=1; return (int)(d << 1); }",
    ),
    (
        "floating shift count",
        "int f(void){ int x=1; double d=1; return (int)(x << d); }",
    ),
    (
        "pointer operand of `<<`",
        "int f(void){ int *p=0; return (int)(p << 1); }",
    ),
    (
        "record operand of `<<`",
        "struct S{int a;}; int f(void){ struct S s; return (int)(s << 1); }",
    ),
    (
        "floating operand of `&`",
        "int f(void){ double d=1; return (int)(d & 1); }",
    ),
    (
        "pointer operand of `&`",
        "int f(void){ int *p=0; return (int)(p & 1); }",
    ),
    (
        "pointer operands of `|`",
        "int f(void){ int *p=0; int *q=0; return (int)(p | q); }",
    ),
    (
        "floating operand of `^`",
        "int f(void){ double d=1; return (int)(d ^ 1); }",
    ),
    // C 6.5.2.2p1 and 6.7.6.3p10, wave 372.
    (
        "call producing an incomplete type",
        "struct I; struct I g(void); int f(void){ g(); return 0; }",
    ),
    (
        "qualified `void` as the only parameter",
        "int f(const void);",
    ),
    // C 6.8.6.1 and 6.5.3.3, wave 377. All refused by gcc in both modes.
    (
        "a `case` label past a variably-modified declaration",
        "int f(int n){ switch(n){ case 1: ; int a[n]; case 2: return 0; } return 0; }",
    ),
    (
        "a `default` label past a variably-modified declaration",
        "int f(int n){ switch(n){ case 1: ; int a[n]; default: return 0; } return 0; }",
    ),
    (
        "a jump past a variably-modified typedef",
        "int f(int n){ goto skip; typedef int T[n]; skip: return 0; }",
    ),
    // C 6.7.2.3p1, wave 379. Refused by gcc in both modes.
    (
        "a tag reused as a different kind",
        "union U { int a; }; struct U { int b; };",
    ),
    (
        "a struct tag reused as an enum",
        "struct S { int a; }; enum S { A };",
    ),
    // C 6.7.9p14 and 6.5.16.2, wave 380. Refused by gcc in both modes.
    ("string literal into an `int` array", "int a[4] = \"abc\";"),
    ("wide literal into a `char` array", "char a[4] = L\"abc\";"),
    (
        "a pointer offset by a floating value in place",
        "int f(void){ int *p=0; double d=1; p += d; return p!=0; }",
    ),
    // Bit-fields and record members, wave 387. The 6.7.2.1 census found the section otherwise
    // complete, so these are the two gaps rather than a new area.
    (
        "_Alignas on a bit-field",
        "struct S { _Alignas(8) int a : 2; };",
    ),
    (
        "_Alignas on an unnamed bit-field",
        "struct S { _Alignas(8) int : 2; int b; };",
    ),
    ("struct with no members", "struct S { };"),
    ("union with no members", "union U { };"),
    ("struct with no named members", "struct S { int : 3; };"),
    ("union with no named members", "union U { int : 3; };"),
    ("tagless struct with no members", "struct { } x;"),
    (
        "tagless struct with no named members",
        "struct { int : 3; } x;",
    ),
];

fn gcc_rejects(src: &str) -> Option<bool> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("gcc")
        .args([
            "-std=c11",
            "-pedantic-errors",
            "-c",
            "-o",
            "/dev/null",
            "-x",
            "c",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.as_mut()?.write_all(src.as_bytes()).ok()?;
    Some(!child.wait().ok()?.success())
}

/// **How much of C's constraint surface sema rejects, as a number that may not fall.**
///
/// Raising `FLOOR` is the deliberate act a wave performs when it adds a rule. Lowering it is
/// how a regression announces itself, and the failure prints the names of everything missed so
/// the next wave has a queue rather than a percentage.
#[test]
fn the_share_of_violations_sema_rejects_does_not_fall() {
    /// The measured count at wave 356. **Raise this when a rule is added; never lower it.**
    ///
    /// Wave 325 measured 54 and closed three; wave 326 closed four more. **The two still below the
    /// line are the two that need machinery sema does not have**, which is why the queue emptied
    /// down to them rather than stopping anywhere arbitrary:
    ///
    ///   - **discarding `const`** needs *qualified types* — 436 `Ty::` match sites across four
    ///     crates, budgeted in §9 as its own effort;
    ///   - **a `goto` into a VLA's scope** — closed in wave 327, by recording which
    ///     variably-modified scopes are open at each label and at each `goto` and requiring the
    ///     first set to be contained in the second.
    ///
    /// Wave 329's census refilled the queue with eight rows and left three open; **wave 330
    /// closed all three**, plus a fourth it found while probing their boundaries — a `struct` tag
    /// could be defined twice with nothing said.
    ///
    /// One violation is knowingly absent rather than missed: `typedef static int T;` is also a
    /// multiple-storage-class error, and `DeclKind::Typedef` carries no `Storage` in this AST, so
    /// the `static` is gone before sema looks. Listing it here would fail against a parser gap
    /// rather than a sema one.
    const FLOOR: usize = 276;

    if gcc_rejects("int main(void){return 0;}") != Some(false) {
        eprintln!("skipping: gcc not usable here");
        return;
    }

    let mut caught = Vec::new();
    let mut missed = Vec::new();
    let mut not_a_violation = Vec::new();
    let mut gcc_unavailable = Vec::new();
    for (name, src) in VIOLATIONS {
        match gcc_rejects(src) {
            // **gcc disagreeing is a bug in this list, not a finding.** Dropped from the
            // denominator rather than counted as a miss.
            Some(false) => {
                not_a_violation.push(*name);
                continue;
            }
            // **"gcc could not be run" is not "gcc accepted it."** These were the same branch,
            // and the difference showed up the first time this test ran under load: a spawn
            // failure landed the row in `not_a_violation` and the failure said "gcc accepts
            // these, so they are bugs in this list" about a program gcc had never seen. 023 §9
            // applies to a test's own output — a report a person cannot act on is not a report,
            // and that one sends them to edit a correct row.
            None => {
                gcc_unavailable.push(*name);
                continue;
            }
            Some(true) => {}
        }
        let p = std::panic::catch_unwind(|| {
            harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux())
                .analysis
                .diagnostics
                .is_empty()
        });
        // A parse rejection is a rejection: the program did not get through.
        match p {
            Ok(true) => missed.push(*name),
            Ok(false) | Err(_) => caught.push(*name),
        }
    }

    assert!(
        not_a_violation.is_empty(),
        "gcc accepts these, so they are bugs in this list rather than missing checks: {not_a_violation:?}"
    );
    assert!(
        gcc_unavailable.is_empty(),
        "gcc could not be run for these, so they were graded against nothing — \
         usually this machine being too busy to spawn it: {gcc_unavailable:?}"
    );
    eprintln!(
        "sema rejects {} of {} constraint violations; missing: {:?}",
        caught.len(),
        VIOLATIONS.len(),
        missed
    );
    assert!(
        caught.len() >= FLOOR,
        "coverage fell to {} from {FLOOR}; newly missed: {missed:?}",
        caught.len()
    );
}

/// **One mistake, one diagnostic** (contract 20) — the channel §9 asked for, and the one question
/// no other channel here asks.
///
/// The ratchet above counts *rejections* and `generated_silence.rs` counts *false positives*;
/// neither can see a program rejected **twice** for the same fault. Wave 351 found three such
/// cascades by hand, all the same shape — a specific sentence ("division by zero") followed by a
/// generic one about its consequence ("not an integer constant expression", "variably modified at
/// file scope", "width is not constant") — and nothing was watching for a fourth.
///
/// **`VIOLATIONS` is the right corpus for it**, because its rows are one-mistake programs by
/// construction: each was written to exercise exactly one constraint. Where that is *not* true the
/// row is the defect, which is what this test found on its first run — and the fix is to the row,
/// not to the engine.
#[test]
fn one_mistake_produces_one_diagnostic() {
    let mut noisy = Vec::new();
    let mut examined = 0usize;
    for (name, src) in VIOLATIONS {
        // A parse rejection is a rejection; the parser has its own diagnostics and its own
        // contract, and counting them here would measure two engines at once.
        let Ok(d) = std::panic::catch_unwind(|| {
            harness::parse_allowing_diagnostics(src, TargetConfig::x86_64_linux())
                .analysis
                .diagnostics
                .iter()
                .map(|x| x.message.clone())
                .collect::<Vec<_>>()
        }) else {
            continue;
        };
        examined += 1;
        if d.len() > 1 {
            noisy.push(format!("{name}: {d:?}"));
        }
    }
    // **A gate that quantifies over an empty set passes vacuously**, and this one skips every row
    // the parser rejects — so if a change made the parser refuse most of the list, the assertion
    // below would go on passing while measuring almost nothing. The seventh such floor here.
    assert!(
        examined * 2 > VIOLATIONS.len(),
        "only {examined} of {} rows reached sema; the rest were rejected by the parser and this \
         test is measuring almost nothing",
        VIOLATIONS.len()
    );
    assert!(
        noisy.is_empty(),
        "{} row(s) report a single mistake more than once:\n  {}",
        noisy.len(),
        noisy.join("\n  ")
    );
}

/// **Every diagnostic points at something a reader can see** (023 §9).
///
/// The message audit was wave 372's; this is the other half of the same claim. A report whose
/// span covers no text is one a reader cannot follow to the fault — an editor puts the caret
/// between two tokens and highlights nothing — and until this test there was **no way to observe
/// a span at all**, which is why four of them had been wrong since wave 365 with every message
/// assertion passing.
///
/// `SourceMap::span_text` is the instrument. It has existed all along; nothing in the suite
/// called it, so "the diagnostic is correct" had only ever meant "the sentence is correct".
///
/// Run over `VIOLATIONS`, which is 260 programs already known to be rejected — the audit needs a
/// corpus of *faults*, and building a second one would have been building a worse one.
///
/// The failure prints the row and the message, so this reads as a queue rather than a count.
#[test]
fn every_diagnostic_points_at_visible_text() {
    let mut invisible: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for (name, src) in VIOLATIONS {
        let tu = chiero_pp::preprocess_str("t.c", src, chiero_pp::Config::default());
        let mut oracle = chiero_parse::ScopedTypedefs::new();
        let parsed = chiero_parse::parse_tu(&tu, &mut oracle);
        let names = harness::names_of(&parsed);
        let analysis = chiero_sema::analyze(&parsed.ast, &TargetConfig::x86_64_linux(), &names);
        for d in &analysis.diagnostics {
            checked += 1;
            match tu.source_map.span_text(d.span) {
                Some(t) if !t.is_empty() => {}
                _ => invisible.push(format!("{name}: {}", d.message)),
            }
        }
    }
    // **A gate that quantifies over an empty set passes vacuously**, and this one would if the
    // rows stopped reaching sema. The seventh such floor in this file, for the same reason.
    assert!(
        checked > VIOLATIONS.len() / 2,
        "only {checked} diagnostics were examined across {} rows",
        VIOLATIONS.len()
    );
    assert!(
        invisible.is_empty(),
        "{} diagnostic(s) point at no visible text:\n  {}",
        invisible.len(),
        invisible.join("\n  ")
    );
}

/// **A span covers the text the message is about, and a message is one line of prose.**
///
/// Wave 373's gate asks whether a span covers *any* text; this asks whether it covers the right
/// text, and it was found by doing what that wave could not — rendering all 252 diagnostics the
/// `VIOLATIONS` corpus produces and reading them. Two families came out, and both are invisible
/// to every message assertion in the suite.
///
/// **A `Func` type node's span is its parameter list.** So the two rules that point at a function
/// *type* point at `(void)`: "a function may not return an array or a function" on `int
/// f(void)[3]` sends a reader to the parameters, and a `_Generic` association of function type
/// names `(void)` where the association is `int(void)`. One cause, two rules, and the parameter
/// list is the one part of the declarator that is *not* at fault in either.
///
/// **One message carries its own source indentation.** A Rust string literal broken across lines
/// keeps the leading whitespace of the continuation, so the `goto`-into-a-VLA diagnostic reads
/// "…the scope of a<38 spaces>variably-modified declaration". Twenty-one messages in this file
/// are written across lines; one of them was written without the `\` that joins them.
#[test]
fn a_span_covers_what_the_message_is_about() {
    for (src, want) in [
        // **The return type is at fault**, so that is what a reader must see. Not the parameter
        // list, which is the one part of the declarator that is fine — and not the whole
        // declaration either, which would be true and useless.
        ("int f(void)[3];", "[3]"),
        (
            "int f(void){ return _Generic(1, int(void): 1, default: 0); }",
            "int(void)",
        ),
    ] {
        let tu = chiero_pp::preprocess_str("t.c", src, chiero_pp::Config::default());
        let mut oracle = chiero_parse::ScopedTypedefs::new();
        let parsed = chiero_parse::parse_tu(&tu, &mut oracle);
        let names = harness::names_of(&parsed);
        let analysis = chiero_sema::analyze(&parsed.ast, &TargetConfig::x86_64_linux(), &names);
        let covered: Vec<&str> = analysis
            .diagnostics
            .iter()
            .filter_map(|d| tu.source_map.span_text(d.span))
            .collect();
        assert_eq!(covered, vec![want], "the span for `{src}`");
    }

    // **Every message is one line with no run of spaces**, checked over the whole corpus because
    // the fault is a typo in a string literal and could be anywhere.
    let mut ragged: Vec<String> = Vec::new();
    for (name, src) in VIOLATIONS {
        let tu = chiero_pp::preprocess_str("t.c", src, chiero_pp::Config::default());
        let mut oracle = chiero_parse::ScopedTypedefs::new();
        let parsed = chiero_parse::parse_tu(&tu, &mut oracle);
        let names = harness::names_of(&parsed);
        let analysis = chiero_sema::analyze(&parsed.ast, &TargetConfig::x86_64_linux(), &names);
        for d in &analysis.diagnostics {
            if d.message.contains("  ") || d.message.contains('\n') {
                ragged.push(format!("{name}: {:?}", d.message));
            }
        }
    }
    assert!(
        ragged.is_empty(),
        "{} message(s) carry their own source formatting:\n  {}",
        ragged.len(),
        ragged.join("\n  ")
    );
}

/// **A span narrows to the enumerator that is wrong**, and **two faults produce two reports**.
///
/// Two claims, both from §9's list after the span audit, and they are in one test because both
/// are about what a *corpus* of diagnostics looks like rather than about one rule.
///
/// **The span half is wave 374's "true and useless" failure mode, found by that audit and left
/// for a wave with gcc open.** `enum E { A = 2147483648 }` names the whole enumeration: accurate,
/// and no help in `enum E { A, B, …, Z = 2147483648 }`. gcc points at the enumerator — at the
/// *value* when one is written and at the *name* when the value is implicit, which is the case
/// wave 357 built the range check to catch and therefore the case that has no value to point at.
///
/// **The two-fault half is a regression gate for a property that already holds.** Nothing
/// measured it: `one_mistake_produces_one_diagnostic` runs over `VIOLATIONS`, where every row is
/// one fault by construction, so "two faults give two reports" and "neither is a consequence of
/// the other" were untested in either direction. Thirty programs were tried while writing this —
/// two faults in one expression, in one declarator, across functions, across *stages* — and every
/// one was already right. The rows below are the ones worth keeping.
#[test]
fn a_span_narrows_and_two_faults_give_two_reports() {
    let analyse = |src: &str| {
        let tu = chiero_pp::preprocess_str("t.c", src, chiero_pp::Config::default());
        let mut oracle = chiero_parse::ScopedTypedefs::new();
        let parsed = chiero_parse::parse_tu(&tu, &mut oracle);
        let names = harness::names_of(&parsed);
        let analysis = chiero_sema::analyze(&parsed.ast, &TargetConfig::x86_64_linux(), &names);
        let out: Vec<(String, String)> = analysis
            .diagnostics
            .iter()
            .map(|d| {
                (
                    d.message.clone(),
                    tu.source_map.span_text(d.span).unwrap_or("").to_owned(),
                )
            })
            .collect();
        out
    };

    for (src, want) in [
        ("enum E { A = 2147483648 };", "2147483648"),
        ("enum E { A = -2147483649 };", "-2147483649"),
        // **The implicit successor**, which has no value written — so the enumerator's own name
        // is the only thing to point at, and it is what gcc points at.
        ("enum E { A = 2147483647, B };", "B"),
        // The offender is not the first enumerator, which is the case the whole-enumeration span
        // made useless.
        ("enum E { A, B, C = 2147483648 };", "2147483648"),
    ] {
        let got = analyse(src);
        assert_eq!(got.len(), 1, "one diagnostic for `{src}`: {got:?}");
        assert_eq!(got[0].1, want, "the span for `{src}`");
    }

    // **Two independent faults, two reports, and nothing else.** The pairs are chosen so neither
    // fault could produce the other: different functions, different declarations, different
    // paragraphs of C.
    for (src, wants) in [
        (
            "int f(void){ int *p=0; return p == 1; }\nint g(void){ double d=1; return (int)(d % 2); }",
            [
                "comparison between a pointer and an integer",
                "`%` needs integer operands",
            ],
        ),
        (
            "int a[-1]; int b[-2];",
            [
                "array length of `a` is negative",
                "array length of `b` is negative",
            ],
        ),
        (
            "struct S { float a:3; int f(void); };",
            [
                "bit-field `a` has a non-integer type",
                "a member may not have a function type",
            ],
        ),
        (
            "struct I; struct I a[-1];",
            [
                "array has an incomplete element type",
                "array length of `a` is negative",
            ],
        ),
    ] {
        let got = analyse(src);
        let messages: Vec<&str> = got.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(messages, wants, "both faults, and only those, for `{src}`");
    }
}
