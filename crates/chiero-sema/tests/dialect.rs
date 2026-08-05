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

    // **The adjustment has to reach the body, and it did not.** Everything above tests the
    // *caller's* view — the parameter list of an interned `Ty::Func`, which is what an argument
    // is checked against. The body reads a different store, and only the caller's view was
    // adjusted, so inside `f` the parameter was still a function type.
    //
    // Measured on gcc 13.3.0, both modes: every row below compiles, and `sizeof g` is **8**.
    // That is the failure mode this whole wave exists to prevent for arrays — an adjustment
    // applied at one of the places a parameter's type is recorded and not the others — sitting
    // in the tree already, for functions.
    for src in [
        "int f(int g(void)){ _Static_assert(sizeof g == 8, \"a pointer, not a function\"); return 0; }\n",
        "int f(int g(void)){ g = 0; return 0; }\n",
        "int f(int g(void)){ return g(); }\n",
        "int f(int g(void)){ int (**q)(void) = &g; return (*q)(); }\n",
    ] {
        for dialect in [Dialect::pedantic(), Dialect::gnu()] {
            assert_eq!(
                sema_messages(src, dialect),
                Vec::<String>::new(),
                "gcc compiles this in both modes: {src}"
            );
        }
    }
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

    // **The shape is still checked, and each half needs its own row.** `vector_size(16)` of
    // `long long` against `vector_size(16)` of `int` differs in *both* element and lane count,
    // so it cannot say which half is doing the work — mutation dropped each in turn and that
    // single row passed both times. A compound condition needs one fixture per component.

    // Same lanes (2), different element: `long long` against `double`.
    assert!(
        !sema_messages(
            "typedef long long a2 __attribute__((__vector_size__(16)));\n\
             typedef double d2 __attribute__((__vector_size__(16), __aligned__(1)));\n\
             int store(d2 *p);\n\
             int f(void *p) { return store((a2 *) p); }\n",
            Dialect::pedantic()
        )
        .is_empty(),
        "same lanes, different element type"
    );

    // Same element (`int`), different lanes: 4 against 8.
    assert!(
        !sema_messages(
            "typedef int i4 __attribute__((__vector_size__(16)));\n\
             typedef int i8 __attribute__((__vector_size__(32), __aligned__(1)));\n\
             int store(i8 *p);\n\
             int f(void *p) { return store((i4 *) p); }\n",
            Dialect::pedantic()
        )
        .is_empty(),
        "same element type, different lane count"
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

    let changes: Vec<(u64, u64)> = a
        .pointee_alignment_changes()
        .map(|(_, f, t)| (f, t))
        .collect();
    assert_eq!(
        changes,
        vec![(16, 1), (1, 16)],
        "both directions recorded, in source order"
    );
    // The hazardous one is the *increase*: a 1-aligned object reached through a pointer that
    // promises 16. Recorded so a checker can find it without re-deriving the types.
    assert!(a.pointee_alignment_changes().any(|(_, f, t)| t > f));

    // **A conversion that changes nothing is not recorded.** `m16 *` to `m16 *` has equal
    // alignment; logging it would fill the table with non-events and a checker reading it
    // would report conversions where nothing moved. Mutation recorded every conversion and
    // the assertion above could not see it — the source had no equal-alignment pointer
    // conversion to notice.
    let same = "typedef long long m16 __attribute__((__vector_size__(16)));\n\
                int takes(m16 *p);\n\
                int g(m16 *p) { return takes(p); }\n";
    let tu2 = chiero_pp::preprocess_str("t.c", same, Config::default());
    let mut o2 = ScopedTypedefs::new();
    let p2 = parse_tu_with(&tu2, &mut o2, Dialect::gnu());
    let a2 = analyze_with(
        &p2.ast,
        &TargetConfig::x86_64_linux(),
        &harness::names_of(&p2),
        Dialect::gnu(),
    );
    assert_eq!(a2.pointee_alignment_changes().count(), 0);
}

/// **A cast to a union type** — `(ip4_address_t) la` where `ip4_address_t` is a union with a
/// `u32` member. The GNU extension: the result is a union object with that member set. 11 of
/// the 32 remaining VPP findings, and the largest kind left — VPP writes it in `ipsec_input.c`,
/// `ipsec_output.h`, `http3.c` and `iavf/rx_node.c`, three of them inside an aggregate
/// initializer and one as `((iavf_rx_desc_qw1_t) qw1).length`.
///
/// **Measured in both modes**, which is what makes it a dialect rule rather than an
/// over-rejection: `gnu11` compiles all four silently, `-pedantic-errors` says "ISO C forbids
/// casts to union type" at each. So chiero accepts the construct and only the sentence follows
/// the dialect — the pattern `__int128`, `char d[0]` and `\e` already set.
///
/// **The member match is by type compatibility, not by conversion**, and this is the half that
/// keeps the extension from swallowing real defects. gcc refuses `(U)x` for an `int` x against a
/// union whose member is `unsigned int` — "cast to union type from type not present in union" —
/// in *both* modes. A rule written with `assignable`, as `transparent_union`'s member search
/// correctly is, would take that one and go quiet on a genuine mismatch.
///
/// Compatibility, though, is not type *identity*: an `enum e` operand matches an `unsigned int`
/// member, because C makes an enumeration compatible with its underlying type. A strict-identity
/// rule refuses that and diverges from gcc, so the row is here even though no VPP site has one.
///
/// **gcc does not look inside anonymous members.** Two of the three real target unions are
/// `union { struct { …bitfields… }; uN as_uN; }`, and the match is on `as_uN` alone — a union
/// whose *only* candidate is a field of an anonymous struct is refused in both modes. That is a
/// live trap rather than a hypothetical: the member access `.length` on the result **does** see
/// through the anonymous struct, so an implementation that reuses that lookup for the cast is
/// wrong in a way every naive fixture would miss.
#[test]
fn a_cast_to_a_union_type_is_a_pedantic_rule_only() {
    let u = "typedef union { unsigned char b[4]; unsigned int u; } U;\n";
    for src in [
        &format!("{u}unsigned int f(unsigned int x){{ return ((U)x).u; }}\n"),
        &format!("{u}U f(unsigned int x){{ U a = (U)x; return a; }}\n"),
        // **The `iavf_rx_desc_qw1_t` shape**, which is `((iavf_rx_desc_qw1_t) qw1).length` at
        // `drivers/iavf/rx_node.c:166`: an anonymous struct of bitfields beside the scalar
        // member the cast actually matches. `http_req_handle_t` is the same shape.
        "union T { struct { unsigned long ptype : 8; unsigned long length : 26; }; unsigned long as_u64; };\n\
         unsigned long f(unsigned long q){ return ((union T)q).length; }\n",
        // A pointer member. The `const` row below matches because lvalue conversion drops the
        // *operand's* top-level qualifier — not because members are compared unqualified, which
        // they are not: see the two refused qualified-pointer rows further down.
        "union P { int *p; long l; };\nlong f(int *q){ return ((union P)q).l; }\n",
        &format!("{u}unsigned int f(const unsigned int x){{ return ((U)x).u; }}\n"),
        // A struct member matches too — `((union V)s).s.a` compiles under gnu11.
        "struct S { int a; };\nunion V { struct S s; int i; };\n\
         int f(struct S s){ return ((union V)s).s.a; }\n",
        // **An enumeration is compatible with its underlying type**, and gcc takes it.
        "enum e { A = 1 };\nunion U { unsigned int u; char c; };\n\
         unsigned int f(enum e x){ return ((union U)x).u; }\n",
        // **The promoting half of C 6.2.7p3.** `double` is unchanged by the default argument
        // promotions, so an unprototyped member *does* take this prototype — the row that keeps
        // the fix for the refused `char` pair from becoming "unprototyped never matches".
        "union F { int (*g)(); long l; };\nlong f(int (*h)(double)){ return ((union F)h).l; }\n",
    ] {
        assert_eq!(
            sema_messages(src, Dialect::gnu()),
            Vec::<String>::new(),
            "gnu11 compiles this silently: {src}"
        );
        assert_eq!(
            sema_messages(src, Dialect::pedantic()),
            vec!["ISO C forbids casts to union type".to_string()],
            "the calibration default still reports it: {src}"
        );
    }

    // **What the extension does not excuse**, refused in *both* modes. Each row is a
    // measured gcc error, not a guess.
    //
    // **Each row carries the sentence it must draw**, not merely a count. The two rules meet
    // here — a union target that finds no member, and a target that is not a union at all —
    // and counting alone lets either one answer for the other.
    let no_member = "a cast to a union names a type no member has";
    let not_scalar = "a cast names a scalar type or `void`";
    for (src, expect, why) in [
        // No member has the operand's type — and `int` against an `unsigned int` member is
        // exactly the row that separates "compatible type" from "assignable".
        (
            format!("{u}unsigned int f(int x){{ return ((U)x).u; }}\n"),
            no_member,
            "signedness is not a member match",
        ),
        (
            format!("{u}unsigned int f(double x){{ return ((U)x).u; }}\n"),
            no_member,
            "no member is a double",
        ),
        (
            "union P { int *p; long l; };\nlong f(char *q){ return ((union P)q).l; }\n".to_string(),
            no_member,
            "a pointer to another type is not a member match",
        ),
        // **A member's own qualifiers are part of the match**, both ways round. This is the pair
        // the accepted `const`-operand row above would otherwise be read as licensing.
        (
            "union P { const int *p; long l; };\nlong f(int *q){ return ((union P)q).l; }\n"
                .to_string(),
            no_member,
            "a `const int *` member does not take an `int *`",
        ),
        (
            "union P { int *p; long l; };\nlong f(const int *q){ return ((union P)q).l; }\n"
                .to_string(),
            no_member,
            "an `int *` member does not take a `const int *`",
        ),
        // **Anonymous members are not searched.** The member access on the result sees through
        // an anonymous struct; the cast's match does not, and reusing one lookup for both is the
        // mutant this row exists to kill.
        (
            "union A { struct { unsigned int a; unsigned int b; }; unsigned long l; };\n\
             unsigned int f(unsigned int x){ return ((union A)x).a; }\n"
                .to_string(),
            no_member,
            "a field of an anonymous struct is not a member of the union",
        ),
        // An incomplete union has no members to match against. Reached through `(void)` rather
        // than `!= 0` so the row cannot pass by way of cascade suppression on a second
        // diagnostic about the failed cast's type.
        (
            "union V;\nvoid f(int x){ (void)(union V)x; }\n".to_string(),
            no_member,
            "an incomplete union names no member",
        ),
        // **A bit-field is not a member of its declared type for this search.** `layout.fields`
        // records a bit-field's `ty` as what it was declared with, so a match on it is a false
        // acceptance — precisely the "extension swallows a gcc error" failure the rule above is
        // written to prevent, and the gnu sweep is where it would go silent.
        (
            "union B { unsigned int whole : 8; unsigned long l; };\n\
             unsigned long f(unsigned int q){ return ((union B)q).l; }\n"
                .to_string(),
            no_member,
            "a bit-field member is not a member of that type",
        ),
        // **An unprototyped function type is not compatible with an arbitrary prototype**
        // (C 6.2.7p3): only one whose parameters are all unchanged by the default argument
        // promotions. `char` is changed, so both directions are refused — and `types_conflict`,
        // which is calibrated for *redeclarations* where leniency is the safe error, says
        // compatible for either. For this search leniency inverts into a swallowed gcc error.
        (
            "union F { int (*g)(); long l; };\nlong f(int (*h)(char)){ return ((union F)h).l; }\n"
                .to_string(),
            no_member,
            "an unprototyped member does not take a non-promoting prototype",
        ),
        (
            "union F { int (*g)(char); long l; };\nlong f(int (*h)()){ return ((union F)h).l; }\n"
                .to_string(),
            no_member,
            "a non-promoting prototype member does not take an unprototyped argument",
        ),
        // **A struct target is still refused**, in both modes — the extension is unions only,
        // and gcc keeps saying "conversion to non-scalar type requested" for a struct. It keeps
        // chiero's existing sentence, because the union rule never applies to it.
        (
            "struct S { int a; };\nint f(int x){ return ((struct S)x).a; }\n".to_string(),
            not_scalar,
            "the extension does not extend to structs",
        ),
    ] {
        for dialect in [Dialect::gnu(), Dialect::pedantic()] {
            assert_eq!(
                sema_messages(&src, dialect),
                vec![expect.to_string()],
                "gcc refuses this in both modes ({why}): {src}"
            );
        }
    }

    // **The real `ipsec` shape: the cast result initializes a union-typed member of an
    // aggregate.** `ipsec_output.h:23` writes `.ip4_addr = { (ip4_address_t) la,
    // (ip4_address_t) ra }` and `ipsec_input.c:52` writes `.ip4_src_addr = (ip4_address_t) sa` —
    // the cast in *union* position, not the scalar position a `.u = ((U)x).u` reduction puts it
    // in. Three of the four real sites are this, so reducing away from it drops what is being
    // measured.
    //
    // **Two casts, two sentences.** The count is the assertion: a report-once-per-translation-
    // unit dedup would satisfy every single-cast row above and fail here, and gcc emits one per
    // cast.
    let two_casts = format!(
        "{u}struct T {{ U s; U d; }};\n\
         struct T f(unsigned int a, unsigned int b){{ struct T t = {{ .s = (U)a, .d = (U)b }}; return t; }}\n"
    );
    assert_eq!(
        sema_messages(&two_casts, Dialect::gnu()),
        Vec::<String>::new(),
        "gnu11 compiles the initializer shape silently"
    );
    assert_eq!(
        sema_messages(&two_casts, Dialect::pedantic()),
        vec![
            "ISO C forbids casts to union type".to_string(),
            "ISO C forbids casts to union type".to_string(),
        ],
        "one sentence per cast, not one per translation unit"
    );

    // **An already-reported operand gets no second sentence** (contract 20). gcc says
    // "undeclared" once and stops; chiero said that and then added "a cast names a scalar type
    // or `void`", which is not merely extra but *false* — `U` is a union one may cast to. The
    // same holds for an operand of incomplete type, which the scalar path below already excuses
    // and this one did not.
    for (src, why) in [
        (
            format!("{u}unsigned int f(void){{ return ((U)undeclared_x).u; }}\n"),
            "a poisoned operand",
        ),
        (
            format!("union V;\n{u}unsigned int f(union V *p){{ return ((U)*p).u; }}\n"),
            "an operand of incomplete type",
        ),
    ] {
        for dialect in [Dialect::gnu(), Dialect::pedantic()] {
            assert_eq!(
                sema_messages(&src, dialect).len(),
                1,
                "one diagnostic, not two ({why}): {src}"
            );
        }
    }

    // **The result is not an lvalue.** gcc refuses `((U)x).u = 1` under `gnu11` too, so
    // accepting the cast must not also make it assignable — and the sentence is pinned, because
    // a count alone is satisfied by any complaint at all about that line.
    //
    // Under the strict dialect gcc draws *both*: the cast sentence first, then the lvalue one.
    // That order is the assertion's other half — it was reversed, and doubled, until the
    // assignment stopped typing its target twice.
    let lvalue = format!("{u}int f(unsigned int x){{ ((U)x).u = 1; return 0; }}\n");
    assert_eq!(
        sema_messages(&lvalue, Dialect::gnu()),
        vec!["assignment to something that is not an lvalue".to_string()],
        "a cast is not an lvalue, extension or not"
    );
    assert_eq!(
        sema_messages(&lvalue, Dialect::pedantic()),
        vec![
            "ISO C forbids casts to union type".to_string(),
            "assignment to something that is not an lvalue".to_string(),
        ],
        "the cast is reported once, and before the write it does not license"
    );
}

/// **A cast to the operand's own non-scalar type** — `(struct S)s` — is the second half of the
/// same measurement, and a separate gcc extension with its own sentence: `gnu11` accepts it,
/// `-pedantic-errors` says "ISO C forbids casting nonscalar to the same type". It applies to a
/// struct as well as a union, which is what makes it *not* the union rule above.
///
/// **`(union U)u` is why this rule is not optional.** The union rule above refuses it — a union
/// has no member of its own type — while gcc accepts it, through *this* extension and with this
/// sentence. So the two interlock: implementing cast-to-union without this one produces a
/// measured divergence on the very next row.
///
/// The struct half has no such forcing argument, and the adversarial review argued for dropping
/// it: no corpus finding in 1552 files, gcc-13 phrasing pinned for a rule with no user, pure
/// maintenance surface. Kept anyway, and the reason is that the *implementation* cannot be
/// narrowed to unions without becoming wrong: the same-type arm is reached for any record, gcc
/// gives structs the identical sentence, and restricting the arm to unions would trade an
/// untested-but-correct branch for a tested-and-wrong one. An accepted construct with no test is
/// the worse of the two.
#[test]
fn a_cast_to_the_operands_own_record_type_is_a_pedantic_rule_only() {
    for src in [
        "struct S { int a; };\nint f(struct S s){ return ((struct S)s).a; }\n",
        "union U { int a; char c; };\nint f(union U u){ return ((union U)u).a; }\n",
    ] {
        assert_eq!(
            sema_messages(src, Dialect::gnu()),
            Vec::<String>::new(),
            "gnu11 compiles this silently: {src}"
        );
        assert_eq!(
            sema_messages(src, Dialect::pedantic()),
            vec!["ISO C forbids casting nonscalar to the same type".to_string()],
            "the calibration default still reports it: {src}"
        );
    }

    // **Another record of the same shape is not the same type**, and stays refused in both
    // modes — gcc: "conversion to non-scalar type requested".
    let other = "struct S { int a; };\nstruct T { int a; };\n\
                 int f(struct T t){ return ((struct S)t).a; }\n";
    for dialect in [Dialect::gnu(), Dialect::pedantic()] {
        assert_eq!(
            sema_messages(other, dialect),
            vec!["a cast names a scalar type or `void`".to_string()],
            "a different record type is not the same type"
        );
    }
}

/// **A statement expression is an lvalue, and `not_an_lvalue` said it was not.**
///
/// Measured on gcc 13: `({ x; }) = 1` and `({ s; }).m = 1` both compile under `-std=gnu11`, and
/// the only thing `-pedantic-errors` says about either is "ISO C forbids braced-groups within
/// expressions" — a complaint about the construct, never about the assignment. chiero refused
/// both with "not an lvalue".
///
/// The entry was unmeasured and nothing pinned it. It surfaced when `not_an_lvalue` gained its
/// `Member` arm — `({ s; }).m = 1` began recursing into the `StmtExpr` entry and inheriting a
/// refusal that had until then only been reachable directly. The arm did not create the divergence;
/// it widened it, which is what made it visible.
///
/// **The pedantic sentence is deliberately not asserted.** chiero has no braced-group rule, and
/// inventing one here would pin a rule this test did not measure a need for.
#[test]
fn a_statement_expression_is_an_lvalue() {
    for src in [
        "int f(void){ int x = 0; ({ x; }) = 1; return x; }\n",
        "struct S { int m; };\nvoid f(struct S s){ ({ s; }).m = 1; }\n",
        "struct S { int m; };\nvoid f(struct S s){ ({ s; }).m++; }\n",
    ] {
        assert_eq!(
            sema_messages(src, Dialect::gnu()),
            Vec::<String>::new(),
            "gcc compiles this under gnu11: {src}"
        );
    }

    // **The rest of `not_an_lvalue` still refuses what gcc refuses**, so this is not a licence
    // to write to anything. Each is a measured gcc "lvalue required".
    for src in [
        "int f(void){ int x = 0; (x + 1) = 1; return x; }\n",
        "int f(void){ int x = 0; x++ = 1; return x; }\n",
        "enum e { A };\nint f(void){ A = 1; return 0; }\n",
        "struct S { int m; };\nstruct S g(void);\nvoid f(void){ g().m = 1; }\n",
    ] {
        assert!(
            sema_messages(src, Dialect::gnu())
                .iter()
                .any(|m| m.contains("not an lvalue")),
            "gcc refuses this: {src}"
        );
    }
}

/// **A null pointer constant reaches a parameter declared as an array.**
///
/// `clib_socket_sendmsg (cs, &msg, sizeof (msg), 0, 0)` — the fourth parameter is `int fds[]`,
/// and `0` is a null pointer constant, so gcc compiles it silently in **both** modes. chiero
/// said "passing an argument makes a pointer from an integer without a cast". All 6 of the
/// remaining findings of that kind are this one shape, across `vppinfra/socket.h`'s
/// `clib_socket_sendmsg`/`recvmsg`, `vlib_register_errors` (`char *error_strings[]`,
/// `vlib_error_desc_t counters[]`) and `vlib_pci_device_open` (`pci_device_id_t ids[]`).
///
/// **The handoff carried this as a severity question — "gcc warns" — and that was the wrong
/// diagnosis.** gcc does warn about `g(1)` and errors on it under `-pedantic-errors`; it says
/// nothing at all about `g(0)`. The queue entry was written from the *kind* of the message
/// rather than from the sites, and the sites all pass `0`.
///
/// The cause is C 6.7.6.3p7: a parameter declared "array of T" is adjusted to "pointer to T".
/// chiero keeps the array type — deliberately, and `compatible` normalises pointer and array on
/// both sides for exactly that reason — but `assignable`'s null-pointer-constant arm asked only
/// about `Ty::Ptr`, so the one rule that tests the destination's *kind* rather than its pointee
/// fell through. See the backlog note in HANDOFF §9 on adjusting the parameter type properly.
#[test]
fn a_null_pointer_constant_reaches_an_array_typed_parameter() {
    for src in [
        // The three real spellings: unsized, sized, and an array of pointers.
        "void g(int p[]);\nvoid f(void){ g(0); }\n",
        "void g(int p[4]);\nvoid f(void){ g(0); }\n",
        "void g(char *p[]);\nvoid f(void){ g(0); }\n",
        // The `clib_socket_sendmsg` shape, with the array parameter in the middle.
        "typedef struct { int x; } sock_t;\n\
         int send(sock_t *s, void *m, int len, int fds[], int nfds);\n\
         int f(sock_t *s, void *m){ return send(s, m, 4, 0, 0); }\n",
        // A pointer parameter already worked, and must keep working.
        "void g(int *p);\nvoid f(void){ g(0); }\n",
        // An explicit `(void *)0` reaches it through the pointee arm, not this one.
        "void g(int p[]);\nvoid f(void){ g((void *)0); }\n",
    ] {
        for dialect in [Dialect::gnu(), Dialect::pedantic()] {
            assert_eq!(
                sema_messages(src, dialect),
                Vec::<String>::new(),
                "gcc compiles this in both modes: {src}"
            );
        }
    }

    // **`1` is not a null pointer constant** (C 6.3.2.3p3), and this is the half the fix must
    // not take with it. gcc warns under `gnu11` and errors under `-pedantic-errors`; chiero
    // calibrates constraint violations to the latter, so it reports in both.
    for src in [
        "void g(int p[]);\nvoid f(void){ g(1); }\n",
        "void g(int *p);\nvoid f(void){ g(1); }\n",
        "void g(int p[]);\nvoid f(int n){ g(n); }\n",
    ] {
        for dialect in [Dialect::gnu(), Dialect::pedantic()] {
            assert_eq!(
                sema_messages(src, dialect),
                vec![
                    "passing an argument makes a pointer from an integer without a cast"
                        .to_string()
                ],
                "only `0` is the null pointer constant: {src}"
            );
        }
    }
}

/// **`p > 0` is a pedantic-only rule; `p > 1` is an error in both modes.**
///
/// Measured on gcc 13: `v[i] > 0` where `v` is `int **` compiles silently under `-std=gnu11`
/// and draws "ordered comparison of pointer with integer zero [-Wpedantic]" under
/// `-pedantic-errors`. Replace the `0` with a `1` and it is "comparison between pointer and
/// integer" in *both* modes.
///
/// chiero drew its flat both-modes sentence for the zero form. Both of VPP's remaining
/// `comparison between a pointer and an integer` findings are that form, and both are a null
/// check written with the wrong operator — `flowprobe.c:1390` on `u32 **` and `pvti_if.c:125`
/// on `index_t **`, the latter inside an `ALWAYS_ASSERT` guarding a `vec_len (…) == 0` branch.
///
/// **A null pointer constant is what the two operators disagree about** (C 6.5.9p2 vs 6.5.8p2):
/// `p == 0` is legal outright, `p > 0` is a constraint violation the standard states and gcc
/// declines to enforce outside pedantic mode. The comment on this arm said "`p > 0` is not
/// legal" and stopped there, which is true of the standard and wrong about the compiler the
/// corpus is measured against.
#[test]
fn an_ordered_comparison_with_a_null_constant_is_a_pedantic_rule_only() {
    let ordered = "ordered comparison of a pointer with integer zero";
    for src in [
        "int f(int **v, int i){ return v[i] > 0; }\n",
        "int f(int **v, int i){ return v[i] >= 0; }\n",
        "int f(int **v, int i){ return 0 < v[i]; }\n",
    ] {
        assert_eq!(
            sema_messages(src, Dialect::gnu()),
            Vec::<String>::new(),
            "gnu11 compiles this silently: {src}"
        );
        assert_eq!(
            sema_messages(src, Dialect::pedantic()),
            vec![ordered.to_string()],
            "the calibration default still reports it: {src}"
        );
    }

    // **Equality with a null constant stays legal in both**, which is the row the fix must not
    // sweep up: `p == 0` is not a constraint violation at all.
    for src in [
        "int f(int **v, int i){ return v[i] == 0; }\n",
        "int f(int **v, int i){ return v[i] != 0; }\n",
    ] {
        for dialect in [Dialect::gnu(), Dialect::pedantic()] {
            assert_eq!(
                sema_messages(src, dialect),
                Vec::<String>::new(),
                "`p == 0` is legal C: {src}"
            );
        }
    }

    // **A non-zero integer is refused in both modes**, ordered or not — the half that keeps this
    // from becoming "a pointer may be compared with anything".
    for src in [
        "int f(int **v, int i){ return v[i] > 1; }\n",
        "int f(int **v, int i){ return v[i] == 1; }\n",
        "int f(int **v, int i, int n){ return v[i] > n; }\n",
    ] {
        for dialect in [Dialect::gnu(), Dialect::pedantic()] {
            assert_eq!(
                sema_messages(src, dialect),
                vec!["comparison between a pointer and an integer".to_string()],
                "a non-null integer is refused in both modes: {src}"
            );
        }
    }
}

/// **`c ? (x = 1) : v()` — a conditional with one `void` side — is a GNU extension.**
///
/// ```text
/// vnet/ip/vtep.c:19:5: a `void` value is used where a value is required
///   ip46_address_is_ip4 (ip) ? hash_set (t->vtep4, key4.as_u64, 1)
///                            : hash_set_mem_alloc (&t->vtep6, &key6, 1);
/// ```
///
/// One arm is an assignment inside a statement expression and the other a `void` call. Measured
/// against gcc 13.3.0:
///
/// | mode | gcc |
/// |---|---|
/// | `-std=gnu11 -Wall -Wextra` | **silent** |
/// | `-std=c11 -pedantic-errors` | error: "ISO C forbids conditional expr with only one void side" |
///
/// So it is exactly the shape 014's dialect gate exists for: supported unconditionally, reported
/// under the strict dialect, and worded the way gcc words it. The old message — "a `void` value
/// is used where a value is required" — came out of `coerce` and described a constraint
/// violation rather than a divergence, which sends a reader looking for a missing return type.
///
/// **The result is `void`**, which is what makes it usable only as a statement: gcc gives
/// `sizeof(c ? (x=1) : v())` an error, and nothing in the corpus asks for the value.
#[test]
fn a_conditional_with_one_void_side_is_a_gnu_extension() {
    let src = "void v(void); int x;\nvoid f(int c) { c ? (x = 1) : v(); }\n";
    assert_eq!(
        sema_messages(src, Dialect::gnu()),
        Vec::<String>::new(),
        "gnu11 compiles this silently"
    );
    assert_eq!(
        sema_messages(src, Dialect::pedantic()),
        vec!["ISO C forbids a conditional expression with only one void side".to_string()],
        "and the strict dialect reports it once, as a divergence"
    );
    // Both sides `void` is ordinary C and stays silent in both.
    let both = "void v(void); void w(void);\nvoid f(int c) { c ? v() : w(); }\n";
    assert_eq!(sema_messages(both, Dialect::gnu()), Vec::<String>::new());
    assert_eq!(sema_messages(both, Dialect::pedantic()), Vec::<String>::new());
    // A `void` value where a value is genuinely required is still an error in both dialects —
    // the rule is not being dropped, only the one shape gcc accepts.
    let used = "void v(void);\nint f(void) { return v(); }\n";
    assert_eq!(
        sema_messages(used, Dialect::gnu()),
        vec!["a `void` value is used where a value is required".to_string()]
    );
}
