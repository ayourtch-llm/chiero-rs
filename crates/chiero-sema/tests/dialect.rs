//! The non-pedantic dialect, added 2026-08 at the owner's direction.
//!
//! chiero calibrates constraint violations to `-pedantic-errors` (wave 314). VPP builds under
//! `-std=gnu11`, and a sweep is more useful when it reports what a project's own compiler
//! would. Only rules **measured** to differ between the two modes may consult the dialect.

mod harness;

use chiero_ast::Dialect;
use chiero_parse::{ScopedTypedefs, parse_tu_with};
use chiero_pp::{Config, preprocess_str};
use chiero_sema::{TargetConfig, analyze_with};

fn sema_messages(src: &str, dialect: Dialect) -> Vec<String> {
    let tu = preprocess_str("t.c", src, Config::default());
    assert!(tu.diagnostics.is_empty(), "pp: {:?}", tu.diagnostics);
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu_with(&tu, &mut oracle, dialect);
    assert!(
        parsed.diagnostics.is_empty(),
        "parse: {:?}",
        parsed.diagnostics
    );
    analyze_with(
        &parsed.ast,
        &TargetConfig::x86_64_linux(),
        &harness::names_of(&parsed),
        dialect,
    )
    .diagnostics
    .iter()
    .map(|d| d.message.clone())
    .collect()
}

/// **Measured against gcc, both ways.** `enum { A = 0xffffffffu }` — `gnu11` accepts, and
/// `-pedantic-errors` says "ISO C restricts enumerator values to range of `int`". This rule
/// alone is 336 of `vnet`'s 348 findings, so it is the one that decides whether a
/// non-pedantic sweep says anything useful.
#[test]
fn an_enumerator_wider_than_int_is_a_pedantic_rule_only() {
    let src = "enum big { A = 0xffffffffu };\nint main(void){ return A ? 0 : 1; }\n";
    assert!(
        sema_messages(src, Dialect::pedantic())
            .iter()
            .any(|m| m.contains("enumerator")),
        "the calibration default still reports it"
    );
    assert_eq!(sema_messages(src, Dialect::gnu()), Vec::<String>::new());
}

/// `struct S { union { struct { } inner; } u; };` — `gnu11` accepts an empty record, and
/// `-pedantic-errors` refuses it. VPP reaches this through `tw_timer_template.h`, where the
/// members live inside `#if`s that a given configuration switches off.
#[test]
fn an_empty_record_is_a_pedantic_rule_only() {
    let src = "struct S { int a; union { struct { } inner; } u; };\nstruct S s;\n";
    assert!(
        sema_messages(src, Dialect::pedantic())
            .iter()
            .any(|m| m.contains("has no members")),
        "the calibration default still reports it"
    );
    assert_eq!(sema_messages(src, Dialect::gnu()), Vec::<String>::new());
}

/// **The dialect is not a way to hide defects.** A constraint gcc refuses in *both* modes
/// stays refused: an undeclared identifier, a negative array bound, a call to something that
/// is not a function. Without this the flag would quietly turn the sweep into a tool that
/// reports nothing and looks like a clean tree — the failure mode this codebase has spent
/// many waves closing elsewhere.
#[test]
fn the_gnu_dialect_still_refuses_what_gcc_refuses_in_both_modes() {
    for src in [
        "int f(void) { return undeclared_thing; }\n",
        "int a[-1];\n",
        "int g(void) { int x = 0; return x(); }\n",
    ] {
        assert!(
            !sema_messages(src, Dialect::gnu()).is_empty(),
            "gcc refuses this under gnu11 too: {src}"
        );
    }
}

/// **`1 << 31` is accepted by gcc in both modes, and chiero refused it.** Found by the first
/// full-tree sweep: 871 of 884 findings across all 1552 VPP files were this one construct,
/// reached through `vppinfra/elf.h`'s `1 << ELF_SECTION_FLAG_BIT_##f` and the many headers
/// like it. Measured — `gcc -std=gnu11` and `gcc -std=gnu11 -pedantic-errors` both compile
/// `(int)(1 << 31)` without complaint — so this is not a dialect question but an
/// over-rejection, and it must be refused in **neither** dialect.
///
/// C 6.5.7p4 makes `1 << 31` undefined for a signed `E1`, which is why the rule existed. But
/// the sweep's whole purpose is to agree with the compiler the project builds with, and every
/// bit-flag enum in C is written this way.
#[test]
fn a_shift_into_the_sign_bit_is_not_an_overflow_diagnostic() {
    for dialect in [Dialect::pedantic(), Dialect::gnu()] {
        assert_eq!(
            sema_messages(
                "enum flags { A = 1 << 31 };\nint main(void){ return A ? 0 : 1; }\n",
                dialect
            ),
            Vec::<String>::new(),
            "gcc accepts this in both modes"
        );
        assert_eq!(
            sema_messages("int x = 1 << 31;\n", dialect),
            Vec::<String>::new()
        );
    }

    // **The shift's width comes from the *left* operand, and only a wider left operand can
    // show it.** In `1 << 31` both sides are 32-bit signed `int`, so truncating to the right
    // operand's type is indistinguishable — mutation proved it. These rows also assert a
    // *value* rather than the absence of a message: a `_Static_assert` fails loudly if the
    // result was truncated to the wrong width, where "no diagnostic" could not tell.
    for dialect in [Dialect::pedantic(), Dialect::gnu()] {
        assert_eq!(
            sema_messages(
                "_Static_assert((1L << 40) == 1099511627776L, \"64-bit shift keeps its width\");\n",
                dialect
            ),
            Vec::<String>::new()
        );
        // And the signedness is the left operand's: `1u << 31` is positive, not the negative
        // value a signed truncation would give.
        assert_eq!(
            sema_messages(
                "_Static_assert((1u << 31) > 0, \"an unsigned shift stays unsigned\");\n",
                dialect
            ),
            Vec::<String>::new()
        );
    }

    // **Arithmetic overflow is still diagnosed**, in both dialects: `0x7fffffff + 1` is a
    // constraint violation gcc refuses under `-pedantic-errors`, and the point of the fix is
    // to stop conflating a shift with an addition — not to stop checking.
    assert!(
        !sema_messages("enum e { B = 0x7fffffff + 1 };\n", Dialect::pedantic()).is_empty(),
        "an overflowing addition is still an overflow"
    );
}

/// **A type is named from the target's widths, not from fixed ones.**
///
/// Raised by the owner: platforms differ on `sizeof(int)`. The arithmetic is target-driven
/// throughout — every width comes from `target.sizes` — but this diagnostic named types from a
/// hardcoded table, so on a 16-bit-`int` target it called `int` a `short`. Semantics were never
/// affected; the sentence was.
///
/// Both shipped targets are LP64, so no existing fixture could see this. The test builds a
/// target with a 16-bit `int` rather than waiting for one to be added.
#[test]
fn a_type_is_named_using_the_targets_widths() {
    let mut narrow = TargetConfig::x86_64_linux();
    narrow.sizes.int_ = 2;
    narrow.sizes.short_ = 2;

    let msgs = harness::parse_allowing_diagnostics("int a[3] = \"xy\";\n", narrow.clone())
        .analysis
        .diagnostics
        .iter()
        .map(|d| d.message.clone())
        .collect::<Vec<_>>();
    assert!(
        msgs.iter().any(|m| m.contains("array of `int`")),
        "a 16-bit `int` is still an `int` on this target: {msgs:?}"
    );

    // **`short` and `int` are the same width here, and must still be named apart.** Width
    // alone cannot do it — that is why the first fix guessed. The AST carries the written
    // spelling (`TypeKind::Builtin`, `TypeKind::Named`), so nothing needs to be guessed at all.
    let msgs = harness::parse_allowing_diagnostics("short b[3] = \"xy\";\n", narrow)
        .analysis
        .diagnostics
        .iter()
        .map(|d| d.message.clone())
        .collect::<Vec<_>>();
    assert!(
        msgs.iter().any(|m| m.contains("array of `short`")),
        "written `short`, same width as `int` on this target: {msgs:?}"
    );

    // A typedef is reported as the name the source used, not as what it resolves to.
    let msgs = harness::parse_allowing_diagnostics(
        "typedef int myint;\nmyint c[3] = \"xy\";\n",
        TargetConfig::x86_64_linux(),
    )
    .analysis
    .diagnostics
    .iter()
    .map(|d| d.message.clone())
    .collect::<Vec<_>>();
    assert!(
        msgs.iter().any(|m| m.contains("array of `myint`")),
        "the reader wrote `myint`: {msgs:?}"
    );

    // And unchanged on the default target, where 32 bits is `int`.
    let msgs =
        harness::parse_allowing_diagnostics("int a[3] = \"xy\";\n", TargetConfig::x86_64_linux())
            .analysis
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>();
    assert!(
        msgs.iter().any(|m| m.contains("array of `int`")),
        "{msgs:?}"
    );
}

/// **A declaration in an inner block does not outlive it** (C 6.2.1p4).
///
/// The dominant finding of the second full-tree sweep — 867 of 879 — all reaching
/// `vlib/node_funcs.h:661`, where `vlib_process_yield` does exactly this:
///
/// ```c
/// uword r;
/// r = clib_setjmp (...);
/// if (r == ...) { vlib_process_restore_t r = { ... }; ... }
/// return r;                 /* the outer `uword r` */
/// ```
///
/// chiero resolved the `return` to the *inner* struct and reported "a structure or union is
/// copied only from its own type". The shape is ordinary C, and shadowing a name in a nested
/// block is common enough that this reached 867 translation units through one header.
#[test]
fn an_inner_block_declaration_does_not_escape_its_block() {
    let src = "struct S { int a; };\n\
               unsigned long f(unsigned long x) {\n\
               \x20 unsigned long r = x;\n\
               \x20 if (r) { struct S r = { .a = 2 }; (void)r.a; }\n\
               \x20 return r;\n\
               }\n";
    for dialect in [Dialect::pedantic(), Dialect::gnu()] {
        assert_eq!(sema_messages(src, dialect), Vec::<String>::new());
    }

    // **The inner declaration is still in force inside its own block.** A fix that simply
    // ignored nested declarations would pass the assertion above and break this one: `r.a` is
    // only valid because the inner `r` is a struct there.
    assert!(
        sema_messages(
            "struct S { int a; };\n\
             unsigned long g(void) {\n\
             \x20 unsigned long r = 0;\n\
             \x20 { struct S r = { .a = 1 }; return r; }\n\
             }\n",
            Dialect::pedantic()
        )
        .iter()
        .any(|m| m.contains("structure or union")),
        "returning the inner struct as `unsigned long` is still wrong"
    );
}

/// **`return f();` where `f` returns `void`, inside a `void` function.** The dominant finding
/// of the third full sweep — 735 of 755 — through `vnet/interface_funcs.h` and the many
/// wrappers like it.
///
/// Measured, and the two halves of this rule differ:
///
/// * `return void_expr;` — `gnu11` accepts **silently**, `-pedantic-errors` refuses ("ISO C
///   forbids `return` with expression, in function returning void").
/// * `return 5;` — `gnu11` **warns**, `-pedantic-errors` refuses.
///
/// So only the first is gated. The second stays diagnosed in both dialects: gcc warns rather
/// than accepting, and chiero has no warning level, so silencing it would turn a diagnostic
/// gcc still issues into a `Miss` against the sweep's `misses: 0`.
#[test]
fn returning_a_void_expression_from_a_void_function_is_a_pedantic_rule_only() {
    let void_expr = "static void a(void){}\nstatic void b(void){ return a(); }\n";
    assert!(
        sema_messages(void_expr, Dialect::pedantic())
            .iter()
            .any(|m| m.contains("return")),
        "the calibration default still reports it"
    );
    assert_eq!(
        sema_messages(void_expr, Dialect::gnu()),
        Vec::<String>::new()
    );

    // A *value* is a different rule and stays diagnosed in both: gcc only warns, and chiero
    // matching that would mean saying nothing where gcc says something.
    for dialect in [Dialect::pedantic(), Dialect::gnu()] {
        assert!(
            !sema_messages("void f(void){ return 5; }\n", dialect).is_empty(),
            "a non-void value is not the gated case"
        );
    }
}

/// **An enumerator folded from a floating constant.** Top of the queue after sweep round 4:
/// 11 findings through `plugins/wireguard/wireguard_messages.h`, where
/// `#define WHZ (u32)(1/WG_TICK)` with `WG_TICK 0.01` feeds `REKEY_TIMEOUT_JITTER = WHZ / 3`.
///
/// Measured: `gnu11` folds it silently; `-pedantic-errors` says "enumerator value for `X` is
/// not an integer constant expression", tagged `[-Wpedantic]` — the same sentence chiero
/// emits. A calibration question, so it is gated rather than fixed.
///
/// C 6.7.2.2p2 requires an integer constant expression, and 6.6p6 excludes a cast from a
/// floating type, so the rule is right about ISO C; gcc's default folds the cast anyway.
#[test]
fn an_enumerator_folded_from_a_float_is_a_pedantic_rule_only() {
    let src = "#define TICK 0.01\n\
               #define HZ (unsigned)(1/TICK)\n\
               enum limits { REKEY = 5, JITTER = HZ / 3 };\n\
               int use(void) { return JITTER; }\n";
    assert!(
        sema_messages(src, Dialect::pedantic())
            .iter()
            .any(|m| m.contains("integer constant expression")),
        "the calibration default still reports it"
    );
    assert_eq!(sema_messages(src, Dialect::gnu()), Vec::<String>::new());

    // **And the value must be right, or silence is worse than the diagnostic.** chiero could
    // not fold the cast and fell back to "one more than the previous enumerator", giving
    // `JITTER == 6` where gcc gives 33. Gating the message alone satisfies the assertion
    // above and hides a wrong constant — the failure wave 435 named: an assertion that a
    // diagnostic is absent does not test the value computed instead.
    for dialect in [Dialect::pedantic(), Dialect::gnu()] {
        let with_assert = format!("{src}_Static_assert(JITTER == 33, \"gcc folds this\");\n");
        assert!(
            !sema_messages(&with_assert, dialect)
                .iter()
                .any(|m| m.contains("static assertion failed")),
            "the folded value must match gcc's"
        );
    }

    // **A cast to an integer truncates inside the fold, and only a value shows it.** Mutation
    // removed the `.trunc()` and nothing failed: every row above asserts a message or its
    // absence, and dropping the truncation changes no message at all — it changes the number.
    // `(unsigned)(15.0/2) * 2` is 14 with truncation and 15 without.
    for dialect in [Dialect::pedantic(), Dialect::gnu()] {
        assert!(
            !sema_messages(
                "#define H (unsigned)(15.0/2)\nenum e { X = H * 2 };\n\
                 _Static_assert(X == 14, \"the cast truncates before the multiply\");\n",
                dialect
            )
            .iter()
            .any(|m| m.contains("static assertion failed")),
            "a cast inside a folded constant truncates toward zero"
        );
    }

    // **An earlier enumerator is foldable inside a later one.** VPP's headers chain them —
    // `MAX_TIMER_HANDSHAKES = 90 / REKEY_TIMEOUT` two lines below the finding that started
    // this — so a folder that stopped at identifiers would fall back to "one more than the
    // previous" for the whole rest of the enumeration.
    for dialect in [Dialect::pedantic(), Dialect::gnu()] {
        assert!(
            !sema_messages(
                "#define H (unsigned)(1/0.01)\nenum e2 { A = 5, B = H / A };\n\
                 _Static_assert(B == 20, \"100 / 5\");\n",
                dialect
            )
            .iter()
            .any(|m| m.contains("static assertion failed")),
            "an enumerator reference folds inside a later enumerator"
        );
    }

    // **An enumerator that is not constant at all is still refused in both dialects**, because
    // gcc refuses it in both too — a variable is not a folding question.
    for dialect in [Dialect::pedantic(), Dialect::gnu()] {
        assert!(
            !sema_messages("int v;\nenum e { A = v };\n", dialect).is_empty(),
            "a variable in an enumerator is an error under gnu11 as well"
        );
    }
}

/// **A parameter declared as a function is adjusted to a pointer to function** (C 6.7.6.3p8),
/// exactly as an array parameter adjusts to a pointer.
///
/// Top of the queue after sweep round 5: 8 findings, from
/// `plugins/ioam/analyse/ip6/node.c`, where the registration function writes its callback
/// parameter as a *function type* rather than a pointer:
///
/// ```c
/// int ip6_ioam_analyse_register_hbh_handler (u8 option,
///                                            int options (u32 flow_id, …, u16 len));
/// ```
///
/// Measured: `gcc -std=gnu11` and `gcc -std=gnu11 -pedantic-errors` both accept it with no
/// diagnostic at all, so this is a defect rather than a dialect question and must be silent in
/// **both** dialects.
#[test]
fn a_function_typed_parameter_adjusts_to_a_pointer() {
    let src = "int reg(int h(int a, int b));\n\
               int mine(int a, int b) { return a + b; }\n\
               void f(void) { reg(mine); }\n";
    for dialect in [Dialect::pedantic(), Dialect::gnu()] {
        assert_eq!(sema_messages(src, dialect), Vec::<String>::new());
    }

    // **The adjustment does not make every function acceptable.** A callback with the wrong
    // signature is still refused: the parameter becomes `int (*)(int, int)`, and that is a
    // type, not a wildcard.
    assert!(
        !sema_messages(
            "int reg(int h(int a, int b));\n\
             int wrong(char *s) { return *s; }\n\
             void f(void) { reg(wrong); }\n",
            Dialect::pedantic()
        )
        .is_empty(),
        "a mismatched callback is still an error"
    );
}

/// **Only the escapes gcc accepts *silently* are a dialect question.**
///
/// 5 findings from `plugins/perfmon/arm/bundle/branch_pred.c`, which writes `"\%"` in a table
/// of format-string fragments. Measured, and the rule splits four ways under one sentence:
///
/// | escape | `gnu11` | `-pedantic-errors` |
/// |---|---|---|
/// | `\%` | **silent** | error |
/// | `\e` | **silent** | error (GNU's ESC) |
/// | `\q` | warns | error |
/// | `\8` | warns | error |
///
/// So `\%` and `\e` are gated and the rest are not. Gating the whole rule would silence `\q`
/// and `\8`, where gcc still speaks — turning two findings into `Miss`es, which is the trade
/// the `return` rule refused two waves ago.
#[test]
fn only_the_silently_accepted_escapes_are_a_pedantic_rule() {
    for esc in ["\\%", "\\e"] {
        let src = format!("const char *s = \"a{esc}b\";\n");
        assert!(
            sema_messages(&src, Dialect::pedantic())
                .iter()
                .any(|m| m.contains("escape")),
            "the calibration default still reports {esc}"
        );
        assert_eq!(
            sema_messages(&src, Dialect::gnu()),
            Vec::<String>::new(),
            "gnu11 accepts {esc} silently"
        );
    }

    for esc in ["\\q", "\\8"] {
        let src = format!("const char *s = \"a{esc}b\";\n");
        for dialect in [Dialect::pedantic(), Dialect::gnu()] {
            assert!(
                !sema_messages(&src, dialect).is_empty(),
                "gcc warns about {esc} under gnu11, so chiero must not go silent"
            );
        }
    }
}

/// **A GNU extension is supported *and* reported under the strict dialect.**
///
/// The first strict-dialect sweep found **104 misses** — files `gcc -pedantic-errors` refuses
/// and chiero accepts in silence. 100 of them are `__int128`, reached through
/// `vppinfra/types.h:28`. No `--gnu` sweep could ever have shown these, because the sweep ran
/// permissive on both sides; this class was structurally invisible for six rounds.
///
/// 013's construct table lists `__int128` as **required** at VPP scale, so support stays. The
/// wave-314 calibration says the default dialect answers `-pedantic-errors`, so it is also
/// reported there — exactly the arrangement `\e` now has.
#[test]
fn an_int128_is_supported_and_reported_under_the_strict_dialect() {
    let src = "__int128 wide;\nunsigned __int128 uwide;\n";
    assert!(
        sema_messages(src, Dialect::pedantic())
            .iter()
            .any(|m| m.contains("__int128")),
        "`-pedantic-errors` refuses it, so the calibration default reports it"
    );
    assert_eq!(
        sema_messages(src, Dialect::gnu()),
        Vec::<String>::new(),
        "gnu11 accepts it silently"
    );

    // **Each spelling needs its own row.** The fixture above declares both, so a gate that
    // fired for `__int128` alone still produced a message and satisfied the assertion —
    // mutation showed exactly that. `unsigned __int128` is a separate `Builtin` variant and
    // `vppinfra/types.h` uses it, so a signed-only gate would leave those files silently
    // missed all over again.
    for one in ["__int128 a;\n", "unsigned __int128 b;\n"] {
        assert!(
            sema_messages(one, Dialect::pedantic())
                .iter()
                .any(|m| m.contains("__int128")),
            "each spelling reports on its own: {one}"
        );
        assert_eq!(sema_messages(one, Dialect::gnu()), Vec::<String>::new());
    }

    // **The type still works in both dialects.** Reporting an extension must not stop
    // supporting it — 013 calls `__int128` required, and a report that broke `sizeof` would
    // trade 100 misses for a defect. The value is asserted because a message-only test cannot
    // see a type quietly becoming something else.
    for dialect in [Dialect::pedantic(), Dialect::gnu()] {
        assert!(
            !sema_messages(
                "_Static_assert(sizeof(__int128) == 16, \"still 128 bits\");\n",
                dialect
            )
            .iter()
            .any(|m| m.contains("static assertion failed")),
            "the extension keeps working while being reported"
        );
    }
}

/// **A translation unit contains at least one external declaration** (C 6.9p1).
///
/// The last 4 of the strict sweep's 104 misses, all under `vppinfra/test/` — files whose whole
/// body sits behind an `#ifdef` that the sweep's configuration leaves off, so what reaches the
/// parser is empty.
///
/// Measured: `gnu11` accepts an empty translation unit, `-pedantic-errors` refuses it. A
/// dialect question, so it is reported under the strict dialect only.
#[test]
fn an_empty_translation_unit_is_a_pedantic_rule_only() {
    assert!(
        sema_messages("", Dialect::pedantic())
            .iter()
            .any(|m| m.contains("empty translation unit")),
        "`-pedantic-errors` refuses it"
    );
    assert_eq!(sema_messages("", Dialect::gnu()), Vec::<String>::new());

    // **A typedef is an external declaration**, so this is not empty — gcc accepts it in both
    // modes. Testing only `""` would let "the TU declares no *object or function*" pass, which
    // is a different and wrong rule.
    for dialect in [Dialect::pedantic(), Dialect::gnu()] {
        assert_eq!(
            sema_messages("typedef int t;\n", dialect),
            Vec::<String>::new()
        );
        assert_eq!(
            sema_messages("struct S { int a; };\n", dialect),
            Vec::<String>::new()
        );
    }
}

/// **A zero-size array is a GNU extension; a flexible array member is standard C.**
///
/// Surfaced when system-header suppression cleared the `__int128` finding that had been
/// masking it: `vcl/vppcom.h:64` bucketed as a *finding* on one message, and only once that
/// message went away did the file's real status — a **miss** — become visible. The lid effect,
/// in the misses direction.
///
/// Measured, and the two spellings differ:
///
/// | form | `gnu11` | `-pedantic-errors` |
/// |---|---|---|
/// | `char data[0]` | accepts | refuses — "ISO C forbids zero-size array" |
/// | `char data[]` | accepts | **accepts** — C99 flexible array member |
///
/// 013's construct table puts `[0]` arrays in 1165 VPP files and calls them required, so
/// support is unconditional and only the sentence follows the dialect.
#[test]
fn a_zero_size_array_is_a_pedantic_rule_but_a_flexible_member_is_not() {
    let zero = "struct S { int n; char data[0]; };\nstruct S s;\n";
    assert!(
        sema_messages(zero, Dialect::pedantic())
            .iter()
            .any(|m| m.contains("zero-size array")),
        "`-pedantic-errors` refuses it"
    );
    assert_eq!(sema_messages(zero, Dialect::gnu()), Vec::<String>::new());

    // **A flexible array member is C99 and accepted by gcc in both modes.** Gating on "an
    // array with no elements" would take this too, and every VPP structure that ends in one.
    for dialect in [Dialect::pedantic(), Dialect::gnu()] {
        assert_eq!(
            sema_messages("struct F { int n; char data[]; };\nstruct F f;\n", dialect),
            Vec::<String>::new(),
            "a flexible array member is standard C"
        );
    }

    // The extension keeps working: the member contributes nothing to the size, as gcc has it.
    for dialect in [Dialect::pedantic(), Dialect::gnu()] {
        assert!(
            !sema_messages(
                "struct S { int n; char data[0]; };\n\
                 _Static_assert(sizeof(struct S) == sizeof(int), \"the array adds nothing\");\n",
                dialect
            )
            .iter()
            .any(|m| m.contains("static assertion failed")),
            "reporting an extension must not stop supporting it"
        );
    }
}

/// **`__attribute__((transparent_union))` on a parameter** (gcc's extension, at the owner's
/// direction).
///
/// 67 translation units of the first full VPP build — the largest single category — and the
/// cause is not VPP's code at all: glibc declares
/// `bind (int, __CONST_SOCKADDR_ARG, socklen_t)` where that argument is a transparent union,
/// so every socket call passes a `struct sockaddr *` where a union is expected.
///
/// A member's type is accepted for the parameter, **and which member was selected is
/// recorded** — the call is passed as the first member but the callee sees the union, so a
/// later stage that has only "it was allowed" cannot lower it.
#[test]
fn a_transparent_union_parameter_takes_any_members_type() {
    let src = "typedef union { int *i; char *c; } __attribute__((__transparent_union__)) arg_t;\n\
               int take(arg_t a);\n\
               int f(int *p, char *q) { return take(p) + take(q); }\n";
    for dialect in [Dialect::pedantic(), Dialect::gnu()] {
        assert_eq!(
            sema_messages(src, dialect),
            Vec::<String>::new(),
            "either member's type is accepted"
        );
    }

    // **A type matching no member is still refused.** The attribute widens the parameter to
    // its members, not to anything at all.
    assert!(
        !sema_messages(
            "typedef union { int *i; char *c; } __attribute__((__transparent_union__)) arg_t;\n\
             int take(arg_t a);\n\
             struct S { int x; };\n\
             int f(struct S s) { return take(s); }\n",
            Dialect::pedantic()
        )
        .is_empty(),
        "a non-member type is not accepted"
    );

    // **Passing the union's own type is not a widening.** It is accepted — it always was —
    // but it must not appear in the table, or every ordinary union argument would be recorded
    // as a conversion that never happened, and a later stage would insert one.
    {
        let src = "typedef union { int *i; char *c; } __attribute__((__transparent_union__)) arg_t;\n\
                   int take(arg_t a);\n\
                   int f(arg_t u) { return take(u); }\n";
        let tu = chiero_pp::preprocess_str("t.c", src, Config::default());
        let mut oracle = ScopedTypedefs::new();
        let parsed = parse_tu_with(&tu, &mut oracle, Dialect::gnu());
        let a = analyze_with(
            &parsed.ast,
            &TargetConfig::x86_64_linux(),
            &harness::names_of(&parsed),
            Dialect::gnu(),
        );
        assert!(a.diagnostics.is_empty(), "{:?}", a.diagnostics);
        assert_eq!(
            a.transparent_union_args().count(),
            0,
            "the union's own type is passed, not widened"
        );
    }

    // **Only a union may be transparent.** gcc rejects the attribute on a struct, and a
    // `struct` carrying it must not widen — every row above uses a union, so a rule that
    // ignored the tag kind passed all of them.
    assert!(
        !sema_messages(
            "typedef struct { int *i; char *c; } __attribute__((__transparent_union__)) s_t;\n\
             int take(s_t a);\n\
             int f(int *p) { return take(p); }\n",
            Dialect::pedantic()
        )
        .is_empty(),
        "a struct does not take the attribute"
    );

    // **Without the attribute the union is an ordinary union** and a member's type is not
    // interchangeable with it, which is what chiero did for every union until now.
    assert!(
        !sema_messages(
            "typedef union { int *i; char *c; } arg_t;\n\
             int take(arg_t a);\n\
             int f(int *p) { return take(p); }\n",
            Dialect::pedantic()
        )
        .is_empty(),
        "an ordinary union is unchanged"
    );
}

/// **The selected member is recorded, not merely permitted.**
///
/// gcc passes the argument as the union's *first* member while the callee sees the union, so a
/// later stage handed only "this call was allowed" cannot lower it: it does not know which
/// member the value is, nor that a conversion happened at all. Accepting silently would trade
/// 67 findings for 67 places where lowering has to re-derive a fact sema already knew.
#[test]
fn the_selected_transparent_union_member_is_recorded() {
    let src = "typedef union { int *i; char *c; } __attribute__((__transparent_union__)) arg_t;\n\
               int take(arg_t a);\n\
               int f(char *q) { return take(q); }\n";
    let tu = chiero_pp::preprocess_str("t.c", src, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu_with(&tu, &mut oracle, Dialect::gnu());
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let a = analyze_with(
        &parsed.ast,
        &TargetConfig::x86_64_linux(),
        &harness::names_of(&parsed),
        Dialect::gnu(),
    );
    assert!(a.diagnostics.is_empty(), "{:?}", a.diagnostics);

    // The argument expression that was widened, and the member it became: `c`, index 1.
    let recorded: Vec<(usize, String)> = a
        .transparent_union_args()
        .map(|(_, idx, name)| (idx, parsed.text(name).unwrap_or("?").to_owned()))
        .collect();
    assert_eq!(
        recorded,
        vec![(1usize, "c".to_owned())],
        "the second member was selected, and sema says so"
    );
}

/// **An alignment attribute does not make a distinct type for compatibility.**
///
/// `1849 of 1871` translation units in the first *cache-cold* full VPP build, and the largest
/// finding this project has produced. It appeared only once `-march` reached the predefines:
/// before that `__SSE4_2__` was undefined, VPP took its scalar branch, and Intel's intrinsic
/// headers were never compiled at all.
///
/// gcc's `emmintrin.h` declares
/// `_mm_storeu_si128 (__m128i_u *, …)` where `__m128i_u` is `__m128i` plus `__may_alias__` and
/// `__aligned__(1)`, and VPP passes `(__m128i *) p`. gcc accepts that silently: the attributes
/// change alignment and aliasing, not type identity for assignment.
#[test]
fn an_alignment_attribute_does_not_break_pointer_compatibility() {
    let src = "typedef long long m128 __attribute__((__vector_size__(16)));\n\
               typedef long long m128u __attribute__((__vector_size__(16), __may_alias__, __aligned__(1)));\n\
               int store(m128u *p);\n\
               int f(void *p) { return store((m128 *) p); }\n";
    for dialect in [Dialect::pedantic(), Dialect::gnu()] {
        assert_eq!(sema_messages(src, dialect), Vec::<String>::new());
    }

    // **A genuinely different element type is still incompatible.** The attributes are
    // ignored for compatibility; the vector's shape is not.
    assert!(
        !sema_messages(
            "typedef long long m128 __attribute__((__vector_size__(16)));\n\
             typedef int m128i __attribute__((__vector_size__(16), __aligned__(1)));\n\
             int store(m128i *p);\n\
             int f(void *p) { return store((m128 *) p); }\n",
            Dialect::pedantic()
        )
        .is_empty(),
        "a different element type is a different vector"
    );
}

/// **An alignment change across an assignment is recorded** (owner's request).
///
/// Compatibility ignores the attribute — gcc does — but the *change* is real information and
/// the direction matters: passing a 16-aligned pointer where 1-aligned is wanted is safe,
/// while the reverse is where a misaligned access comes from. A later stage cannot recover
/// this once compatibility has been decided, so sema records it.
///
/// **Not a pedantic-mode error**, and that was measured rather than assumed: gcc reports
/// nothing for either direction under `gnu11`, `-pedantic-errors`, or `-Wcast-align=strict`.
/// This project's strict dialect means "what `gcc -pedantic-errors` says"; a rule gcc does not
/// have belongs to a checker (040), not to the dialect, or `--gnu` and the default stop
/// meaning what §9 says they mean.
#[test]
fn a_pointee_alignment_change_is_recorded_with_its_direction() {
    let src = "typedef long long m16 __attribute__((__vector_size__(16)));\n\
               typedef long long m1 __attribute__((__vector_size__(16), __aligned__(1)));\n\
               int wants_lax(m1 *p);\n\
               int wants_strict(m16 *p);\n\
               int safe(m16 *p) { return wants_lax(p); }\n\
               int risky(m1 *p) { return wants_strict(p); }\n";
    let tu = chiero_pp::preprocess_str("t.c", src, Config::default());
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu_with(&tu, &mut oracle, Dialect::gnu());
    let a = analyze_with(
        &parsed.ast,
        &TargetConfig::x86_64_linux(),
        &harness::names_of(&parsed),
        Dialect::gnu(),
    );
    assert!(a.diagnostics.is_empty(), "{:?}", a.diagnostics);

    let changes: Vec<(u64, u64)> = a.pointee_alignment_changes().map(|(_, f, t)| (f, t)).collect();
    assert_eq!(
        changes,
        vec![(16, 1), (1, 16)],
        "both directions recorded, in source order"
    );
    // The hazardous one is the *increase*: a 1-aligned object reached through a pointer that
    // promises 16. Recorded so a checker can find it without re-deriving the types.
    assert!(a.pointee_alignment_changes().any(|(_, f, t)| t > f));
}
