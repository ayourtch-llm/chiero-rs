//! `__has_attribute` / `__has_builtin` — 012 §4, and the persona they answer for.
//!
//! chiero's predefine set is an **impersonation of the build compiler**, not a self-report:
//! `__GNUC__` is baked at 13 and `chiero-cli`'s `frontend` captures the whole `cc -dM` at run
//! time so that headers configure for the code that actually ships. `__has_attribute(x)` is a
//! question in that same register — *does the compiler being impersonated recognise `x`* — so
//! the only correct answer is gcc's, and every assertion here asks gcc rather than asserting a
//! remembered one.
//!
//! Before this, both queries were registered as function-like macros with an **empty body**. So
//! `#ifdef __has_attribute` succeeded — chiero claimed the capability — and then every query
//! expanded to nothing, which `#if` reads as `0`. chiero answered "this compiler has no
//! attributes and no builtins at all", silently, while calling itself gcc 13.
//!
//! # Why not simply drop the two macros
//!
//! Because gcc does not survive it either. With `__has_attribute` undefined,
//! `#if defined __has_attribute && __has_attribute (packed)` is a hard error —
//! *missing binary operator before token "("* — since `#if` parses the whole expression whatever
//! short-circuiting would do at run time. That is the exact idiom `sys/cdefs.h`'s own comment
//! warns about, so dropping them trades silent wrong answers for loud wrong errors on a pattern
//! that is everywhere.
//!
//! # The direction of error, which is what settled the design
//!
//! Answering 0 where gcc answers 1 silently swaps the analysed program for one that never ships.
//! Answering 1 for something chiero cannot model is loud by construction — the parser takes any
//! `__attribute__((...))` and an unmodeled builtin hits the havoc-loudly path with
//! `Approximated` and a named assumption. **A loud approximation of the shipped program beats an
//! exact analysis of an unshipped one**, so the table errs towards gcc, never towards chiero's
//! own capabilities.

use chiero_pp::{Config, preprocess_str};
use std::process::Command;

/// gcc's answer as a **number**, asked in program text.
///
/// ⚠️ **This replaced a `bool` oracle, and the `bool` was hiding something.** A feature query
/// yields a value: `__has_c_attribute(deprecated)` is `201904` under gcc 13. Comparing
/// truthiness would have let the table claim `1` there and called it agreement. gcc evaluates
/// all three queries in program text, so one line of C reads the value out exactly.
fn gcc_value(query: &str, name: &str) -> u32 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("chiero-fqv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("v{}.c", NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&path, format!("{query}({name})\n")).unwrap();
    let output = Command::new("gcc")
        .args(["-E", "-P"])
        .arg(&path)
        .output()
        .expect("gcc is required for the feature-query oracle");
    let _ = std::fs::remove_file(&path);
    let text = String::from_utf8(output.stdout).unwrap();
    text.split_whitespace()
        .collect::<String>()
        .parse()
        .unwrap_or_else(|_| panic!("gcc gave no number for {query}({name})"))
}

/// chiero's answer as a number, through the same program-text path.
fn chiero_value(query: &str, name: &str) -> (u32, Vec<String>) {
    let tu = preprocess_str("q.c", &format!("{query}({name})\n"), Config::default());
    let texts: Vec<_> = tu.token_texts().collect();
    assert_eq!(texts.len(), 1, "expected one token, got {texts:?}");
    (
        texts[0]
            .parse()
            .unwrap_or_else(|_| panic!("not a number: {texts:?}")),
        tu.diagnostics.iter().map(|d| d.message.clone()).collect(),
    )
}

/// **`__has_c_attribute` answers a version, not a truth.**
///
/// gcc 13 defines it at every `-std` level and returns the C standard's version for each
/// attribute. The `0` rows are the discriminating ones: `reproducible` and `unsequenced` are C23
/// attributes gcc 13 does not have, so a table that answered "1 for anything plausible" fails
/// here.
#[test]
fn has_c_attribute_answers_the_version_gcc_does() {
    for name in [
        "deprecated",
        "nodiscard",
        "maybe_unused",
        "fallthrough",
        "noreturn",
        "__deprecated__",
        "reproducible",
        "unsequenced",
    ] {
        let expected = gcc_value("__has_c_attribute", name);
        let (ours, diagnostics) = chiero_value("__has_c_attribute", name);
        assert_eq!(ours, expected, "__has_c_attribute({name})");
        assert!(
            diagnostics.is_empty(),
            "the table covers {name}, so nothing was guessed: {diagnostics:?}"
        );
    }
    // A version is not 1 — the assertion the old `bool` oracle could not make.
    assert!(gcc_value("__has_c_attribute", "deprecated") > 1);
}

/// gcc's answer, asked directly. The oracle for every case in this file.
///
/// ⚠️ **The scratch file is unique per *call*, and it took two goes to get there.** Keying it on
/// the pid alone is not unique — every test in one binary shares it — and keying it on
/// `(query, name)` is not either, because two *different* tests ask about `packed`
/// concurrently, and one deletes the file while the other's gcc is still reading it. Both
/// versions produced a failure that reproduced only under load: first a table mismatch that did
/// not exist, then "gcc gave neither 1 nor 0", the second surviving a clean `-p chiero-pp` run
/// and only falling over in the full workspace.
///
/// A counter is the fix because it is the only key that cannot collide by construction.
/// **Parallel tests must not share mutable filesystem state at all** — every narrower key is a
/// guess about which callers exist, and this file was wrong about that twice.
fn gcc_says(query: &str, name: &str) -> bool {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let source = format!("#if {query}({name})\n1\n#else\n0\n#endif\n");
    let dir = std::env::temp_dir().join(format!("chiero-fq-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("q{}.c", NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&path, source).unwrap();
    let output = Command::new("gcc")
        .args(["-E", "-P"])
        .arg(&path)
        .output()
        .expect("gcc is required for the feature-query oracle");
    let text = String::from_utf8(output.stdout).unwrap();
    let answer = text.split_whitespace().collect::<String>();
    let _ = std::fs::remove_file(&path);
    match answer.as_str() {
        "1" => true,
        "0" => false,
        other => panic!("gcc gave neither 1 nor 0 for {query}({name}): {other:?}"),
    }
}

fn chiero_answer(query: &str, name: &str) -> (bool, Vec<String>) {
    let source = format!("#if {query}({name})\n1\n#else\n0\n#endif\n");
    let tu = preprocess_str("q.c", &source, Config::default());
    let texts: Vec<_> = tu.token_texts().collect();
    let diagnostics = tu
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect();
    assert_eq!(texts.len(), 1, "expected one token, got {texts:?}");
    (texts[0] == "1", diagnostics)
}

/// The two names that made the defect visible, plus the two forms of one attribute — gcc treats
/// `packed` and `__packed__` alike and a table that knows one and not the other is half a table.
#[test]
fn a_supported_attribute_and_builtin_answer_the_way_gcc_does() {
    for (query, name) in [
        ("__has_attribute", "packed"),
        ("__has_attribute", "__packed__"),
        ("__has_attribute", "aligned"),
        ("__has_attribute", "always_inline"),
        ("__has_builtin", "__builtin_expect"),
        ("__has_builtin", "__builtin_unreachable"),
        ("__has_builtin", "__builtin_clz"),
    ] {
        let expected = gcc_says(query, name);
        assert!(expected, "fixture assumes gcc supports {query}({name})");
        let (ours, _) = chiero_answer(query, name);
        assert_eq!(ours, expected, "{query}({name})");
    }
}

/// **A known `0` is knowledge and must not diagnose.** These are real names gcc answers NO to —
/// clang attributes and clang builtins — and getting them right is what distinguishes a table
/// from a rubber stamp. `__init_priority__` in particular is queried by a real header.
#[test]
fn a_known_absent_name_answers_zero_without_complaint() {
    for (query, name) in [
        ("__has_attribute", "__init_priority__"),
        ("__has_attribute", "enable_if"),
        ("__has_attribute", "minsize"),
        ("__has_builtin", "__builtin_debugtrap"),
        ("__has_builtin", "__builtin_fclose"),
    ] {
        let expected = gcc_says(query, name);
        assert!(!expected, "fixture assumes gcc lacks {query}({name})");
        let (ours, diagnostics) = chiero_answer(query, name);
        assert_eq!(ours, expected, "{query}({name})");
        assert!(
            diagnostics.is_empty(),
            "a name the table knows the answer for is knowledge, not ignorance: \
             {query}({name}) said {diagnostics:?}"
        );
    }
}

/// **An unknown name is ignorance and must say so.** `#if` has to yield a number, so "I do not
/// know" cannot ride in-band — it rides in `diagnostics` instead. This is §11.3's "did not look
/// must stay distinct from found nothing", in the one place where the in-band answer is forced
/// to be a lie in one direction or the other.
#[test]
fn an_unknown_name_answers_zero_and_records_that_it_guessed() {
    let (ours, diagnostics) = chiero_answer("__has_attribute", "no_such_attribute_xyzzy");
    assert!(!ours);
    assert!(
        diagnostics
            .iter()
            .any(|d| d.contains("no_such_attribute_xyzzy")),
        "the diagnostic must name the name, so a reader can extend the table: {diagnostics:?}"
    );
}

/// The dedup, because `sys/cdefs.h` alone queries many times per TU and a per-query diagnostic
/// would drown the channel it is reported in.
#[test]
fn one_diagnostic_per_distinct_unknown_name() {
    let source = "#if __has_attribute(unknown_aaa)\n#endif\n\
                  #if __has_attribute(unknown_aaa)\n#endif\n\
                  #if __has_attribute(unknown_bbb)\n#endif\n";
    let tu = preprocess_str("q.c", source, Config::default());
    let named: Vec<_> = tu
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("unknown_"))
        .collect();
    assert_eq!(named.len(), 2, "expected one per distinct name: {named:?}");
}

/// The idiom `sys/cdefs.h` is built around, and the reason dropping the macros is not an option.
#[test]
fn the_guarded_idiom_from_sys_cdefs_h_works() {
    let source = "#if (defined __has_attribute \\\n  && (!defined __clang_minor__ \\\n  || 3 < __clang_major__ + (5 <= __clang_minor__)))\n\
                  # define HAS(a) __has_attribute (a)\n\
                  #else\n\
                  # define HAS(a) 0\n\
                  #endif\n\
                  #if HAS(packed)\nPACKED\n#else\nNOT\n#endif\n";
    let tu = preprocess_str("q.c", source, Config::default());
    let texts: Vec<_> = tu.token_texts().collect();
    assert_eq!(texts, vec!["PACKED"], "diagnostics: {:?}", tu.diagnostics);
}

/// `__has_include` reached **through a wrapper macro**, which is the same rule one query over.
///
/// A regression this wave introduced and then caught: once the query names stopped being
/// expandable, `#define HI(x) __has_include(x)` left `__has_include` in the stream as an
/// identifier the expression parser choked on. It had never *worked* — before, it expanded to
/// nothing and read as `0`, silently answering NO to a header that exists — so the wrapper form
/// is asserted here for the first time, in both directions, against gcc.
#[test]
fn has_include_is_answered_after_expansion_too() {
    struct DiskLoader;
    impl chiero_pp::FileLoader for DiskLoader {
        fn load(&mut self, path: &std::path::Path) -> std::io::Result<String> {
            std::fs::read_to_string(path)
        }
    }
    let config = || Config {
        system_paths: vec![
            "/usr/include".into(),
            "/usr/lib/gcc/x86_64-linux-gnu/13/include".into(),
            "/usr/include/x86_64-linux-gnu".into(),
        ],
        ..Config::default()
    };
    for (source, expected) in [
        (
            "#if __has_include(<stdio.h>)\nYES\n#else\nNO\n#endif\n",
            "YES",
        ),
        (
            "#if __has_include(<no_such_header_xyzzy.h>)\nYES\n#else\nNO\n#endif\n",
            "NO",
        ),
        (
            "#define HI(x) __has_include(x)\n#if HI(<stdio.h>)\nYES\n#else\nNO\n#endif\n",
            "YES",
        ),
        (
            "#define HI(x) __has_include(x)\n#if HI(<no_such_header_xyzzy.h>)\nYES\n#else\nNO\n#endif\n",
            "NO",
        ),
    ] {
        let tu = chiero_pp::preprocess_with_loader("q.c", source, config(), &mut DiskLoader);
        let texts: Vec<_> = tu.token_texts().collect();
        assert_eq!(texts, vec![expected], "{source} — {:?}", tu.diagnostics);
        assert!(tu.diagnostics.is_empty(), "{source} — {:?}", tu.diagnostics);
    }
}

/// The queries evaluate in **program text**, not only in `#if`.
///
/// Both compilers do: `int y = __has_attribute(packed);` comes out of gcc *and* clang as
/// `int y = 1;`. This is asserted because a plausible-sounding rule — "they are preprocessor
/// operators, so they belong to `#if`" — was believed for one commit, and the pp-gate found
/// `__has_attribute` sitting in an output stream where both compilers had put a number.
#[test]
fn the_queries_evaluate_in_program_text() {
    let source = "int y = __has_attribute(packed);\n";
    let tu = preprocess_str("q.c", source, Config::default());
    let texts: Vec<_> = tu.token_texts().collect();
    assert_eq!(texts, vec!["int", "y", "=", "1", ";"]);
}

/// `#ifdef __has_attribute` must stay true — gcc defines both names, and a header that checks
/// before querying is the common case rather than the exception.
#[test]
fn the_query_names_are_still_defined() {
    for name in ["__has_attribute", "__has_builtin", "__has_include"] {
        let source = format!("#ifdef {name}\nYES\n#else\nNO\n#endif\n");
        let tu = preprocess_str("q.c", &source, Config::default());
        let texts: Vec<_> = tu.token_texts().collect();
        assert_eq!(texts, vec!["YES"], "{name} must be defined, as gcc has it");
    }
}

/// **The whole table, re-asked of gcc.** This is the instrument, not a spot check: every entry
/// is a claim about gcc 13 on this machine, and a claim nobody re-checks is a claim that drifts.
/// A gcc upgrade that changes one answer fails here rather than silently changing which branch
/// every system header takes.
#[test]
fn every_table_entry_still_matches_gcc() {
    let mut checked = 0;
    for &(query, name, expected) in chiero_pp::features::TABLE {
        let actual = gcc_value(query, name);
        assert_eq!(
            actual, expected,
            "the table says {query}({name}) = {expected}, gcc 13 on this machine says {actual}"
        );
        checked += 1;
    }
    assert!(
        checked > 50,
        "the table is meant to cover what real headers query; only {checked} entries"
    );
}

/// `__GNUC_PREREQ` must not be constant zero.
///
/// The sibling defect, found in the same review: `__GNUC__` was baked and `__GNUC_MINOR__` was
/// not, and `features.h` defines `__GNUC_PREREQ(maj,min)` as `0` unless **both** exist. Every
/// version shield in every glibc header therefore collapsed for any consumer that does not
/// populate `Config::defines` from a real compiler — which is every test in this workspace, and
/// which is what made the query defect maximal exactly where it was least visible.
#[test]
fn the_baked_persona_supports_gnuc_prereq() {
    let source = "#if defined __GNUC__ && defined __GNUC_MINOR__\n\
                  # define PREREQ(a,b) ((__GNUC__ << 16) + __GNUC_MINOR__ >= ((a) << 16) + (b))\n\
                  #else\n\
                  # define PREREQ(a,b) 0\n\
                  #endif\n\
                  #if PREREQ(4,9)\nNEW\n#else\nOLD\n#endif\n";
    let tu = preprocess_str("q.c", source, Config::default());
    let texts: Vec<_> = tu.token_texts().collect();
    assert_eq!(
        texts,
        vec!["NEW"],
        "a persona claiming __GNUC__ 13 must satisfy __GNUC_PREREQ(4,9)"
    );
}

/// **A scoped operand answers 1, never a version** — and any scope but gcc's answers 0.
///
/// `gnu::noreturn` is 1 under all three queries while bare `noreturn` is 202202, because a
/// vendor-scoped attribute has no *standard* version to report. `clang::packed` is 0 even though
/// `packed` alone is 1: the scope is part of the question.
///
/// ⚠️ C has no `::` punctuator, so `gnu::noreturn` reaches the rewriter as **four** tokens
/// (`gnu`, `:`, `:`, `noreturn`). A matcher written for one identifier between the parens sees
/// nothing here, which is why this is a rule about the operand's *shape* and not a table row.
#[test]
fn a_scoped_operand_answers_one_and_only_for_gcc_s_own_scope() {
    for query in [
        "__has_attribute",
        "__has_c_attribute",
        "__has_cpp_attribute",
    ] {
        for name in [
            "gnu::noreturn",
            "gnu::packed",
            "__gnu__::packed",
            "gnu::deprecated",
        ] {
            let expected = gcc_value(query, name);
            assert_eq!(
                expected, 1,
                "gcc answers 1 for a known gcc-scoped attribute"
            );
            assert_eq!(chiero_value(query, name).0, expected, "{query}({name})");
        }
        for name in ["clang::packed", "foo::packed", "gnu::nonesuch"] {
            let expected = gcc_value(query, name);
            assert_eq!(expected, 0, "gcc answers 0 for an unknown scope or name");
            assert_eq!(chiero_value(query, name).0, expected, "{query}({name})");
        }
    }
}

/// `__has_cpp_attribute` **is defined in C by gcc**, and returns versions.
///
/// ⚠️ A note in HANDOFF said the opposite, written from memory one commit before this was
/// measured. The bare standard attributes carry their version; `packed` and `always_inline`
/// answer 1 because gcc accepts them in the `[[...]]` syntax without a standard version; the C23
/// attributes gcc 13 lacks answer 0. All three shapes are here, because a table that only held
/// the first would pass a rubber stamp.
#[test]
fn has_cpp_attribute_is_available_in_c_and_answers_versions() {
    for name in [
        "noreturn",
        "deprecated",
        "packed",
        "always_inline",
        "unsequenced",
        "nonesuch",
    ] {
        let expected = gcc_value("__has_cpp_attribute", name);
        let (ours, _) = chiero_value("__has_cpp_attribute", name);
        assert_eq!(ours, expected, "__has_cpp_attribute({name})");
    }
    assert!(
        gcc_value("__has_cpp_attribute", "noreturn") > 1,
        "a version, not a truth"
    );
    assert_eq!(
        gcc_value("__has_cpp_attribute", "packed"),
        1,
        "GNU-only: 1, no version"
    );
    assert_eq!(
        gcc_value("__has_cpp_attribute", "unsequenced"),
        0,
        "gcc 13 lacks it"
    );
}

/// **The persona claims gcc 13.3 on x86-64 and then denies it runs on an operating system.**
///
/// Found by 012 contract 17's corpus run over all 1967 configured VPP translation units — the
/// first time the preprocessor had ever been pointed at VPP under VPP's own flags. One of the 25
/// diagnosed units is `vppinfra/pmalloc.c`, whose `#if defined(__linux__) / #elif
/// defined(__FreeBSD__) / #else #error "Unsupported OS"` chain falls straight through, because
/// `__linux__` was not among the eight macros the engine bakes. gcc 13.3 on this machine
/// predefines all five below; a persona that impersonates it and omits them is not incomplete in
/// some abstract way — it compiles a *different program*, one where every Linux-only branch in
/// VPP and in glibc is dead.
///
/// **The `#else` half is the load-bearing one.** Asserting only that `__linux__` is defined would
/// pass on a preprocessor that defined every identifier it had never heard of.
///
/// ⚠️ Deliberately *not* the parked `-march` work (§9.1): nothing here is per-TU, nothing asks a
/// compiler for anything, and no flag is propagated. These five are fixed facts about the persona
/// already baked, and `__linux__` is the one the corpus measured.
#[test]
fn the_baked_persona_admits_it_runs_on_linux() {
    let probe = |macro_name: &str| {
        let source = format!("#if defined({macro_name})\nyes\n#else\nno\n#endif\n");
        let tu = preprocess_str("os.c", &source, Config::default());
        tu.token_texts().map(str::to_owned).collect::<Vec<_>>()
    };
    // **All three spellings of each, because gcc defines all three.** The first version of this
    // test checked `__linux__` alone, the fix defined `__linux__` alone, and VPP's `pmalloc.c`
    // went on reaching `#error "Unsupported OS"` — its guard is `#ifdef __linux`, no trailing
    // underscores. The corpus caught it on the very next run.
    //
    // `linux` and `unix` unprefixed are gnu-mode only (gcc drops them under `-std=c11`), and this
    // persona is gnu mode: `__GNUC__` is baked and `__STRICT_ANSI__` is not. VPP builds `gnu11`.
    for name in [
        "__linux__",
        "__linux",
        "linux",
        "__unix__",
        "__unix",
        "unix",
        "__gnu_linux__",
        "__ELF__",
        "__LP64__",
        "__x86_64__",
        "__x86_64",
    ] {
        assert_eq!(
            probe(name),
            ["yes".to_string()],
            "gcc 13.3 predefines {name}"
        );
    }
    assert_eq!(
        probe("__FreeBSD__"),
        ["no".to_string()],
        "and the persona is one platform, not all of them — without this the test would pass \
         on a preprocessor that treated every unknown identifier as defined"
    );

    // The shape that sent VPP's pmalloc.c into `#error`, reduced to its bones — and spelled
    // `__linux` the way pmalloc.c actually spells it, not the way I first assumed it did.
    //
    // ⚠️ The marker is `IS_LINUX`, not `linux`. This fixture originally said `linux` and started
    // failing with `["1"]` the moment the fix landed — because gnu-mode `linux` **is** a macro.
    // That is the hazard of the unprefixed spellings in one line, and it took under a minute to
    // hit in a file written by someone who had just read gcc's list.
    let chain = "#ifdef __linux\nIS_LINUX\n\
                 #elif defined(__FreeBSD__)\nfreebsd\n\
                 #else\n#error \"Unsupported OS\"\n#endif\n";
    let tu = preprocess_str("pmalloc.c", chain, Config::default());
    assert_eq!(tu.token_texts().collect::<Vec<_>>(), ["IS_LINUX"]);
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
}

/// **The diagnostic attributes glibc's `sys/cdefs.h` queries** — and the one case where chiero
/// answered 0 while gcc answers 1.
///
/// Found by 012 contract 17's corpus run: 20 of VPP's 1967 translation units asked
/// `__has_attribute(error)` and `__has_attribute(diagnose_if)`, and chiero said *"not in the
/// compiler-persona table; answered 0, which may not be what the build compiler says"*. It was
/// telling the truth — that honesty is the design — but for `error` the guess was wrong: gcc 13
/// has `__attribute__((error))` and answers 1.
///
/// This is the module's own stated failure mode, arriving in real code: **answering 0 where gcc
/// answers 1 silently swaps the analysed program for one that never ships.** `_FORTIFY_SOURCE=2`
/// is on for all 20, and `__attribute_error__` collapses to nothing when the query says 0.
///
/// `diagnose_if` is clang's, and 0 *is* gcc's answer — kept because a table that only holds the
/// entries that turned out to be 1 cannot be checked for the difference between "0 because gcc
/// says so" and "0 because nobody looked".
#[test]
fn the_table_covers_the_diagnostic_attributes_glibc_queries() {
    for name in ["error", "__error__", "warning", "diagnose_if"] {
        for query in [
            "__has_attribute",
            "__has_c_attribute",
            "__has_cpp_attribute",
        ] {
            let expected = gcc_value(query, name);
            let (ours, diagnostics) = chiero_value(query, name);
            assert_eq!(ours, expected, "{query}({name})");
            assert!(
                diagnostics.is_empty(),
                "{query}({name}) is in the table now, so nothing is guessed: {diagnostics:?}"
            );
        }
    }
    // The load-bearing row, stated on its own so a future edit cannot quietly flip it to 0.
    assert_eq!(gcc_value("__has_attribute", "error"), 1);
}
