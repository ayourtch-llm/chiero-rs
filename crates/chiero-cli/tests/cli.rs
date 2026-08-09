//! **The `chiero` command — 050 §1's "thin wrapper over the identical" operation surface.**
//!
//! > `chiero-cli` is a thin wrapper over the identical operations the MCP server exposes.
//!
//! Every operation in `chiero-tool` was reachable only from Rust, so the tutorials taught a
//! library API to a reader who wanted a command. These tests are written against the command,
//! not the library, because that is the surface being claimed.
//!
//! **Thin is the requirement, not an aspiration.** The command must not compute anything the
//! library does not: its job is to turn arguments into inputs, call one operation, and print
//! the envelope. Anything it decides on its own is a second implementation that can disagree
//! with the first, and the envelope is the one thing in this system that must not have two.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_chiero")
}

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("chiero-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

fn write(name: &str, src: &str) -> PathBuf {
    let p = scratch().join(name);
    std::fs::write(&p, src).expect("write");
    p
}

struct Run {
    code: i32,
    out: String,
    err: String,
}

fn run(args: &[&str]) -> Run {
    let o = Command::new(bin()).args(args).output().expect("spawn");
    Run {
        code: o.status.code().unwrap_or(-1),
        out: String::from_utf8_lossy(&o.stdout).into_owned(),
        err: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

fn json(r: &Run) -> serde_json::Value {
    serde_json::from_str(&r.out).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}):\n{}\n--- stderr ---\n{}",
            r.out, r.err
        )
    })
}

/// **No arguments prints usage and fails**, rather than succeeding at nothing.
#[test]
fn bare_invocation_names_the_operations() {
    let r = run(&[]);
    assert_ne!(r.code, 0, "doing nothing is not success");
    let text = format!("{}{}", r.out, r.err);
    for op in [
        "prove-equivalent",
        "impact",
        "select-tests",
        "expansion-sites",
        "explain-macro",
    ] {
        assert!(text.contains(op), "usage does not mention `{op}`:\n{text}");
    }
}

/// **041 §1 from a command line.** The `abs` rewrite that looks right, adjudicated without
/// writing any Rust.
#[test]
fn prove_equivalent_from_two_c_files() {
    let before = write("abs_before.c", "int f (int x) { return x < 0 ? -x : x; }\n");
    let after = write(
        "abs_after.c",
        "int f (int x) {\n  if (x < 0)\n    return x == (-2147483647 - 1) ? 2147483647 : -x;\n  return x;\n}\n",
    );
    let r = run(&[
        "prove-equivalent",
        before.to_str().unwrap(),
        after.to_str().unwrap(),
        "--entry",
        "f",
        "--json",
    ]);
    assert_eq!(r.code, 0, "stderr:\n{}", r.err);
    let v = json(&r);

    // The envelope, whole — a command that printed only the result would be the one thing
    // 050 §2 forbids.
    for key in ["result", "fidelity", "proven", "assumptions", "blind_spots"] {
        assert!(!v[key].is_null(), "the envelope is missing `{key}`: {v}");
    }

    // Without a solver on PATH this is `unknown`, which is a real answer and must still be a
    // well-formed one. With z3 it is the divergence at INT_MIN.
    let verdict = v["result"]["verdict"].as_str().unwrap_or("");
    assert!(
        verdict == "differs" || verdict == "unknown",
        "these two are not equivalent: {v}"
    );
    if verdict == "differs" {
        assert_eq!(
            v["result"]["input"][0]["signed"].as_str(),
            Some("-2147483648"),
            "the input that shows it: {v}"
        );
    }
}

/// The agreeing direction, so the command is not just a `Differs` printer.
#[test]
fn prove_equivalent_can_also_bless() {
    let a = write("dbl_a.c", "int f (int x) { return x * 2; }\n");
    let b = write("dbl_b.c", "int f (int x) { return x << 1; }\n");
    let r = run(&[
        "prove-equivalent",
        a.to_str().unwrap(),
        b.to_str().unwrap(),
        "--entry",
        "f",
        "--json",
    ]);
    assert_eq!(r.code, 0, "stderr:\n{}", r.err);
    let v = json(&r);
    let verdict = v["result"]["verdict"].as_str().unwrap_or("");
    assert!(
        verdict == "equivalent" || verdict == "unknown",
        "x * 2 and x << 1 agree: {v}"
    );
}

/// **031 from a command line**: the header macro edit that coverage cannot see.
#[test]
fn impact_follows_a_macro_edit_into_the_functions_that_expand_it() {
    let before = write(
        "geom_before.c",
        "#define SCALE(x) ((x) * 2)\nint area (int w) { return SCALE (w) * w; }\n\
         int volume (int w) { return area (w) * w; }\n",
    );
    let after = write(
        "geom_after.c",
        "#define SCALE(x) ((x) * 3)\nint area (int w) { return SCALE (w) * w; }\n\
         int volume (int w) { return area (w) * w; }\n",
    );
    let r = run(&[
        "impact",
        before.to_str().unwrap(),
        after.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(r.code, 0, "stderr:\n{}", r.err);
    let v = json(&r);
    let names: Vec<&str> = v["result"]["entities"]
        .as_array()
        .expect("entities")
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    for want in ["SCALE", "area", "volume"] {
        assert!(names.contains(&want), "{want} is not in {names:?}");
    }
}

/// **The macro operations**, which need one file and no solver.
#[test]
fn the_macro_operations_answer_from_one_file() {
    let f = write(
        "mac.c",
        "#define INNER(v) ((v) + 1)\n#define OUTER(v) (INNER (v) * 2)\n\
         int a (int x) { return OUTER (x); }\n",
    );

    let r = run(&[
        "expansion-sites",
        f.to_str().unwrap(),
        "--macro",
        "INNER",
        "--json",
    ]);
    assert_eq!(r.code, 0, "stderr:\n{}", r.err);
    let v = json(&r);
    assert!(
        v["proven"].as_bool().unwrap_or(false),
        "the table is exact: {v}"
    );
    assert_eq!(v["result"]["total"].as_u64(), Some(1));

    let r = run(&[
        "explain-macro",
        f.to_str().unwrap(),
        "--line",
        "3",
        "--json",
    ]);
    assert_eq!(r.code, 0, "stderr:\n{}", r.err);
    let v = json(&r);
    let chains = v["result"]["chains"].as_array().expect("chains");
    assert_eq!(chains.len(), 1, "one macro is written on line 3: {v}");
    let names: Vec<&str> = chains[0]
        .as_array()
        .expect("frames")
        .iter()
        .filter_map(|f| f["name"].as_str())
        .collect();
    assert!(
        names.contains(&"OUTER") && names.contains(&"INNER"),
        "the full chain: {names:?}"
    );
}

/// **The human rendering is the default**, and it carries the qualification too.
#[test]
fn the_default_output_is_for_a_person_and_still_says_what_it_is_worth() {
    let f = write(
        "mac2.c",
        "#define M(v) ((v) + 1)\nint a (int x) { return M (x); }\n",
    );
    let r = run(&["expansion-sites", f.to_str().unwrap(), "--macro", "NOPE"]);
    assert_eq!(r.code, 0, "stderr:\n{}", r.err);
    assert!(
        serde_json::from_str::<serde_json::Value>(&r.out).is_err(),
        "the default is not JSON:\n{}",
        r.out
    );
    assert!(!r.out.trim().is_empty(), "and it is not empty either");
}

/// **A file that does not exist is an error, not an empty answer.**
///
/// This is the shape 050 §2 exists to prevent, arriving through the door a command line opens:
/// a reader who types a wrong path and gets `sites: 0` learns something false.
#[test]
fn a_missing_file_is_an_error_not_a_zero() {
    let r = run(&[
        "expansion-sites",
        "/nonexistent/nope.c",
        "--macro",
        "M",
        "--json",
    ]);
    assert_ne!(r.code, 0, "stdout was:\n{}", r.out);
    assert!(
        r.err.contains("nope.c"),
        "the error must name the file: {}",
        r.err
    );
}

/// **An unknown operation names the ones that exist**, rather than failing silently.
#[test]
fn an_unknown_operation_is_refused_by_name() {
    let r = run(&["frobnicate"]);
    assert_ne!(r.code, 0);
    let text = format!("{}{}", r.out, r.err);
    assert!(text.contains("frobnicate"), "{text}");
    assert!(text.contains("prove-equivalent"), "{text}");
}

/// **A file that includes a system header must be analysable.**
///
/// Every operation reads real C, and real C starts with `#include <stdio.h>`. Without the
/// compiler's own include paths and predefined macros, `chiero find-bugs` on anything from an
/// actual codebase answers `cannot include stdarg.h: No such file or directory` — which is not
/// a fact about the code.
///
/// **The predefines matter as much as the paths.** glibc's `bits/floatn.h` alone branches on a
/// dozen `__HAVE_FLOAT*` macros, and a preprocessor lacking them compiles code the compiler
/// never sees. The sweep learned that the hard way — its first run reported 101 findings that
/// were entirely this — and a command line pointed at a tree has exactly the same problem.
#[test]
fn a_file_that_includes_a_system_header_can_be_analysed() {
    let f = write(
        "sys.c",
        "#include <stdarg.h>\n#include <stdint.h>\nint f (int x) { return x / (x - x); }\n",
    );
    let r = run(&["find-bugs", f.to_str().unwrap(), "--entry", "f", "--json"]);
    assert_eq!(
        r.code, 0,
        "a file with a system header is ordinary C:\n{}",
        r.err
    );
    let v = json(&r);
    assert!(
        v["result"]["findings"]
            .as_array()
            .is_some_and(|a| !a.is_empty()),
        "and the division by zero in it is still found: {v}"
    );
}

/// **And the discovery is a runtime fact, reported rather than assumed.**
///
/// chiero links no compiler (010 §1), so where the system headers are is something it asks at
/// run time — like the solver and like the replay compiler. On a machine with no `gcc` the
/// answer is "none", and a caller passing `-I` themselves must still work.
#[test]
fn system_headers_can_be_turned_off_and_supplied_by_hand() {
    let f = write("nosys.c", "int f (int x) { return x / (x - x); }\n");
    let r = run(&[
        "find-bugs",
        f.to_str().unwrap(),
        "--entry",
        "f",
        "--no-system-headers",
        "--json",
    ]);
    assert_eq!(r.code, 0, "stderr:\n{}", r.err);
    assert!(
        json(&r)["result"]["findings"]
            .as_array()
            .is_some_and(|a| !a.is_empty()),
        "a file needing no headers is unaffected"
    );
}

/// **`--entry-ptr-nonnull` — the flag that makes `find-bugs` usable on real code.**
///
/// Measured over 40 VPP entry points: 178 findings, not one of them `Exact`, every one a null
/// dereference or an out-of-bounds access reached through *an unconstrained pointer parameter*.
/// Those are statements about the caller contract, not about the function, and a reader hunting
/// defects cannot act on any of them.
///
/// The flag says "the callers check", and the envelope has to say the flag was used — the
/// assumption is what separates a narrowed search from a quieter one.
#[test]
fn entry_pointers_can_be_declared_non_null_from_the_command_line() {
    let p = write("entry_nonnull.c", "int f(int *p) { return *p; }\n");
    let path = p.to_str().expect("utf-8 path");

    let loose = run(&["find-bugs", path, "--entry", "f", "--json"]);
    let lv = json(&loose);
    assert!(
        lv["result"]["findings"]
            .as_array()
            .is_some_and(|a| !a.is_empty()),
        "by default an unconstrained pointer parameter may be null: {lv}"
    );

    let tight = run(&[
        "find-bugs",
        path,
        "--entry",
        "f",
        "--entry-ptr-nonnull",
        "--json",
    ]);
    let v = json(&tight);
    assert_eq!(tight.code, 0, "stderr: {}", tight.err);
    assert_eq!(
        v["result"]["findings"].as_array().map(Vec::len),
        Some(0),
        "the null path was assumed away: {v}"
    );
    assert!(
        v["assumptions"]
            .as_array()
            .is_some_and(|a| a.iter().any(|x| x["kind"] == "entry_ptr_nonnull")),
        "and the envelope has to carry the assumption that bought the quiet: {v}"
    );

    // The default rendering is the human one, and it is what a reader actually sees. An empty
    // finding list there without the assumption beside it reads as "clean".
    let human = run(&["find-bugs", path, "--entry", "f", "--entry-ptr-nonnull"]);
    assert!(
        human.out.contains("entry_ptr_nonnull"),
        "the human rendering has to say it too: {}",
        human.out
    );

    let help = run(&["--help"]);
    assert!(
        help.out.contains("--entry-ptr-nonnull"),
        "a flag nobody can discover is not a feature: {}",
        help.out
    );
}

/// **Two findings that are about chiero, in a report about the user's code.**
///
/// Widening the VPP sweep to 220 entry points across `vnet/` turned up twenty-one copies of the
/// first, on `vnet/bier/bier_api.c`:
///
/// ```text
/// symbolic-byte: byte 0 of c holds a symbolic value, which a concrete access cannot answer for
/// strcpy: source scan gave CapReached { scanned: 0 }
/// ```
///
/// Neither is a statement about a program. The first is a fact about `Memory::read`, which
/// returns bytes and therefore cannot return a symbol — `MemFault::SymbolicByte`'s own doc says
/// "the byte API cannot answer … the caller wants `read_term`". The second is a `{:?}` of an
/// internal Rust enum, `StrScan`, leaked into a defect message.
///
/// The fifth instance of one confusion in this wave, after the entry object, the `extern`
/// global and the bitfield: **chiero not knowing a value is not the program failing to write
/// one.** The rule already exists — the format model filters exactly this fault with the
/// comment *"reporting that as a program bug is the confusion 023 §7 exists to prevent. Found
/// by review."* It was applied at one site, and this is what one site buys.
///
/// What replaces them is not silence: the run is `Unknown` and says why. That is the whole
/// difference between a limit and a defect.
#[test]
fn chieros_own_limits_are_not_reported_as_defects_in_your_code() {
    let p = write(
        "self_report.c",
        "char *strcpy (char *, const char *);\n\
         int f (unsigned i, char x, char *dst)\n\
         {\n\
         \x20 static char c[256];\n\
         \x20 c[i & 255] = x;      /* a symbolic index: the object promotes to array theory */\n\
         \x20 strcpy (dst, c);     /* and a concrete byte read cannot serve it */\n\
         \x20 return 0;\n\
         }\n",
    );
    let r = run(&[
        "find-bugs",
        p.to_str().expect("utf-8 path"),
        "--entry",
        "f",
        "--entry-ptr-nonnull",
        "--json",
    ]);
    let v = json(&r);
    let findings = v["result"]["findings"].as_array().expect("array").clone();
    for f in &findings {
        let m = f["message"].as_str().unwrap_or_default();
        assert!(
            !m.contains("symbolic-byte"),
            "a value chiero cannot carry in a `Vec<u8>` is not a defect in this program: {m}"
        );
        assert!(
            !m.contains("CapReached") && !m.contains('{'),
            "and a Rust `{{:?}}` of an internal enum is not a sentence anyone can act on: {m}"
        );
    }
    // **Not silence.** The answer really is weaker, and that is what fidelity is for.
    assert_ne!(
        v["fidelity"], "Exact",
        "the run could not read those bytes and must say so: {v}"
    );
}

/// **A lazy object stays initialized when it promotes to array theory.**
///
/// The seventh place in this wave where chiero's own ignorance was reported as the program's
/// defect, found in the 220-entry `vnet/` sweep:
///
/// ```text
/// maybe-uninitialized-read: read at offset 0 of the 4096-byte object reached through an
///                           unconstrained pointer touches bit 0, which was written only
///                           under a condition that may not hold here
/// ```
///
/// 021 §6 gives the object behind an entry pointer "fully symbolic and fully initialized"
/// bytes, because the caller filled it and chiero does not know what with. A write at a
/// symbolic offset promotes the object to array theory, and the initialization *array* is
/// built from scratch — so everything §6 established is lost at the moment of promotion, and
/// every subsequent read of caller memory is a report.
///
/// `struct s { int a[64]; }; p->a[i & 63] = 1; return p->a[(i >> 8) & 63];` is enough: one
/// symbolic-offset write to promote, one read to report.
#[test]
fn promotion_does_not_lose_what_021_6_established() {
    let p = write(
        "lazy_promote.c",
        "struct s { int a[64]; };\n\
         int f (struct s *p, unsigned i)\n\
         {\n\
         \x20 p->a[i & 63] = 1;          /* a symbolic offset: the object promotes */\n\
         \x20 return p->a[(i >> 8) & 63];\n\
         }\n",
    );
    let r = run(&[
        "find-bugs",
        p.to_str().expect("utf-8 path"),
        "--entry",
        "f",
        "--entry-ptr-nonnull",
        "--json",
    ]);
    let v = json(&r);
    for f in v["result"]["findings"].as_array().expect("array") {
        let m = f["message"].as_str().unwrap_or_default();
        assert!(
            !m.contains("uninitialized"),
            "the caller filled this object; chiero does not know what with, and promoting \
             its representation does not change that: {m}"
        );
    }
}

/// **"chiero could not resolve this pointer" is not a defect in the program.**
///
/// 23 of the 42 findings on the 220-entry `vnet/` sweep were these two sentences:
///
/// ```text
/// unresolvable pointer: the value is unconstrained, so it could refer to any object or to none
/// a symbolic pointer could not be resolved: the solver did not decide which objects its
///                                           value can fall in
/// ```
///
/// The second one's own code comment settles it — *"the path ends at `SolverUnknown` instead —
/// a statement about chiero, which is what it is"* — and then pushes a finding anyway. The
/// first is 021 §5.1 step 4, which is the rule that chiero must **not** concretize a pointer it
/// cannot pin; that is a decision about chiero's honesty, not an accusation about the code.
///
/// **The argument against filtering, and why it does not hold.** A genuinely arbitrary pointer
/// value *is* a hazard, so this could be suppressing a real class. It is not, because the real
/// cases are reported by something else and stay: an uninitialized pointer variable is an
/// `uninitialized-read`, and an address chiero *proves* lands in no object is a `WildPointer`.
/// Those are knowledge. These two are the absence of it — and both already degrade the run
/// with a named assumption, so nothing is lost by not also calling them defects.
#[test]
fn a_pointer_chiero_cannot_resolve_is_not_a_defect() {
    let p = write(
        "unresolvable.c",
        // An integer of unknown value, used as an address: chiero has no constraint on it,
        // which is 021 §5.1 step 4 exactly.
        "int f (unsigned long v) { return *(int *) v; }\n",
    );
    let r = run(&[
        "find-bugs",
        p.to_str().expect("utf-8 path"),
        "--entry",
        "f",
        "--json",
    ]);
    let v = json(&r);
    for f in v["result"]["findings"].as_array().expect("array") {
        let m = f["message"].as_str().unwrap_or_default();
        assert!(
            !m.contains("unresolvable pointer") && !m.contains("could not be resolved"),
            "chiero's own inability is not this program's defect: {m}"
        );
    }
    // **Said, not swallowed.** The path really did end there, and the envelope carries why.
    assert_ne!(v["fidelity"], "Exact", "the path ended unresolved: {v}");
    assert!(
        v["assumptions"].as_array().is_some_and(|a| a
            .iter()
            .any(|x| x["detail"].as_str().is_some_and(|d| d.contains("pointer")))),
        "and the assumption names it: {v}"
    );
}

/// **`PREDICT_FALSE (p == 0)` is a null check, and chiero was not reading it as one.**
///
/// Found on the ACL plugin: four `null-dereference: access at offset -8 of NULL` findings, all
/// funnelling into `vec_validate`, whose NULL case *is* guarded —
///
/// ```c
/// if (PREDICT_FALSE (v == 0))
///   { ... return; }
/// vl = _vec_len (v);            /* v[-8] */
/// ```
///
/// — and `PREDICT_FALSE(x)` is `__builtin_expect((x), 0)`. chiero treated the builtin as an
/// opaque call, so the branch condition stopped being *about* `v`, the null path survived into
/// the body, and the dereference fired. The report even says "the function tests it against
/// null at source offset 41": chiero saw the test and could not use it.
///
/// **This is not a modelling question.** GCC defines `__builtin_expect(exp, c)` as returning
/// the value of `exp` — the hint is for the branch predictor and has no effect on semantics.
/// Treating it as opaque is not conservative, it is wrong in the dangerous direction: it
/// invents paths the program does not have, and every one of them is a false finding.
///
/// The blast radius is why this matters more than four findings: `PREDICT_FALSE` and
/// `PREDICT_TRUE` are *the* idiom for guards throughout VPP, so every null check written that
/// way was defeated.
#[test]
fn builtin_expect_is_its_first_argument() {
    let p = write(
        "predict.c",
        "#define PREDICT_FALSE(x) __builtin_expect ((x), 0)\n\
         #define PREDICT_TRUE(x)  __builtin_expect ((x), 1)\n\
         \n\
         int f (int *p)\n\
         {\n\
         \x20 if (PREDICT_FALSE (p == 0))\n\
         \x20   return 0;\n\
         \x20 return *p;                 /* unreachable when p == 0 */\n\
         }\n\
         \n\
         int g (int *p)\n\
         {\n\
         \x20 if (PREDICT_TRUE (p != 0))\n\
         \x20   return *p;               /* likewise */\n\
         \x20 return 0;\n\
         }\n\
         \n\
         int h (int *p)\n\
         {\n\
         \x20 if (PREDICT_FALSE (p != 0))  /* the guard is backwards */\n\
         \x20   return 0;\n\
         \x20 return *p;                   /* reached exactly when p == 0 */\n\
         }\n",
    );
    let path = p.to_str().expect("utf-8 path");
    let nulls = |entry: &str| -> Vec<String> {
        let r = run(&["find-bugs", path, "--entry", entry, "--json"]);
        json(&r)["result"]["findings"]
            .as_array()
            .expect("array")
            .iter()
            .filter_map(|f| f["message"].as_str().map(str::to_string))
            .filter(|m| m.contains("null-dereference"))
            .collect()
    };

    for entry in ["f", "g"] {
        assert!(
            nulls(entry).is_empty(),
            "`{entry}` guards this dereference, and the guard is a hint to the branch \
             predictor rather than a wall chiero cannot see through: {:?}",
            nulls(entry)
        );
    }

    // **The positive control, and it is the assertion that gives the two above their meaning.**
    //
    // `h` has the guard backwards, so the dereference is reached *exactly* when `p` is null.
    // Without this, "no null-dereference" is satisfied by chiero learning nothing from the
    // branch at all — which is the failure mode being fixed, in the opposite direction. Both
    // together say the condition is genuinely being read.
    assert!(
        !nulls("h").is_empty(),
        "`h`'s guard is inverted and the null path is the one that reaches the dereference"
    );

    // **A narrower argument than `long`, which is the case that broke real code.**
    //
    // `self.expr` emits sema's conversion chain, so a `char` argument arrives already promoted
    // to `int`; taking the source type from `type_of` — which walks *down* that chain to the
    // innermost value — declared `Int(8)` for an `Int(32)` operand and the verifier refused the
    // whole function. `mem_dlmalloc.c` stopped lowering, and **the workspace suite stayed
    // green**: nothing in it passed a narrow value through this builtin. Measured instead, by
    // 18 of 40 VPP entry points going from `ok` to `failed`.
    // A `char` promotes to `int` before it gets here, and a `double` is not an integer at
    // all. Both were assumed away — `Int(8)` declared for the promoted `char`, then `Int(32)`
    // for the `F64` — and each time the verifier refused the whole function while the
    // workspace suite stayed green.
    for (name, decl) in [
        ("predict_narrow.c", "char c"),
        ("predict_double.c", "double c"),
        ("predict_long.c", "long c"),
    ] {
        let src = write(
            name,
            &format!("int f ({decl}) {{ return __builtin_expect (c, 0) ? 1 : 0; }}\n"),
        );
        let r = run(&[
            "find-bugs",
            src.to_str().expect("utf-8 path"),
            "--entry",
            "f",
            "--json",
        ]);
        // **The regression is the function being *skipped*, so that is what is asserted.**
        // Not `fidelity`: the `double` arm degrades for a reason that has nothing to do with
        // this builtin — `FpToSi` on a symbolic operand is not modelled — and pinning
        // `Exact` here would be pinning that gap instead.
        assert_eq!(
            r.code, 0,
            "`{decl}` through the builtin still lowers: {}",
            r.err
        );
        assert!(
            !r.err.contains("skipped"),
            "`{decl}`: the verifier rejected the function: {}",
            r.err
        );
        json(&r); // and the run produced an envelope rather than a diagnostic
    }
    // The one arm with no unrelated gap, as the control that the runs above are real
    // analyses and not early exits.
    let plain = write(
        "predict_long.c",
        "int f (long c) { return __builtin_expect (c, 0) ? 1 : 0; }\n",
    );
    let r = run(&[
        "find-bugs",
        plain.to_str().expect("utf-8 path"),
        "--entry",
        "f",
        "--json",
    ]);
    assert_eq!(
        json(&r)["fidelity"],
        "Exact",
        "nothing here is beyond chiero"
    );
}

/// **A struct passed by value was filled in by the caller too.**
///
/// The eighth instance of one confusion in this wave, and §9 had written down what to look for:
/// *chiero not knowing a value is not the program failing to write one.* Found on the ACL
/// plugin, `prefetch_session_entry (acl_main_t *am, fa_full_session_id_t f_sess_id)`:
///
/// ```text
/// uninitialized-read: read at offset 4 of f_sess_id touches bit 32, which was never
///                     written through f_sess_id.thread_index
/// ```
///
/// 021 §6 gives a *pointer* parameter's pointee "fully symbolic and fully initialized" bytes,
/// for exactly this reason — the caller filled it and chiero does not know what with. An
/// aggregate parameter is the same argument with the copy on this side of the call: the
/// caller evaluated every member, and C has no way to pass an indeterminate struct that was
/// never assigned.
#[test]
fn an_aggregate_parameter_arrives_filled_in() {
    let p = write(
        "byval.c",
        "struct id { unsigned thread_index; unsigned session_index; };\n\
         unsigned f (struct id s) { return s.thread_index + s.session_index; }\n",
    );
    let r = run(&[
        "find-bugs",
        p.to_str().expect("utf-8 path"),
        "--entry",
        "f",
        "--json",
    ]);
    let v = json(&r);
    for f in v["result"]["findings"].as_array().expect("array") {
        let m = f["message"].as_str().unwrap_or_default();
        assert!(
            !m.contains("uninitialized"),
            "the caller built this struct member by member: {m}"
        );
    }
    // **A local aggregate is the opposite case and must still report.** Without this, the rule
    // could be "aggregates are never uninitialized", which would lose a real class.
    let local = write(
        "byval_local.c",
        "struct id { unsigned a; unsigned b; };\n\
         unsigned g (void) { struct id s; return s.a; }\n",
    );
    let r = run(&[
        "find-bugs",
        local.to_str().expect("utf-8 path"),
        "--entry",
        "g",
        "--json",
    ]);
    let v = json(&r);
    assert!(
        v["result"]["findings"]
            .as_array()
            .expect("array")
            .iter()
            .any(|f| f["message"]
                .as_str()
                .is_some_and(|m| m.contains("uninitialized"))),
        "nobody wrote this one: {v}"
    );
}

/// **A run nobody waits for still answers** — 023 §8.1's wall clock, on the surface where the
/// waiting happens.
///
/// Measuring `find-bugs` over 220 VPP entry points, six were killed by the harness's external
/// `timeout` and 11 more on the ACL plugin. A killed process prints nothing: no findings, no
/// fidelity, no envelope. So the measurement recorded `timeout` beside `ok` and had no way to
/// say what those functions were or were not hiding — "nothing there" and "did not look" became
/// the same row, which is the one collapse this project does not allow itself.
///
/// With a wall clock the process ends by its own decision and prints what it had: the findings
/// it already made, `Bounded`, and `budgets.hit` naming the clock. That is a *worse* answer than
/// a complete run and an incomparably better one than silence.
///
/// **And it says the answer is not reproducible** (050 contract 16). Everything else chiero
/// prints is a computation over its input; this one depends on how fast the machine was, so the
/// envelope carries `nondeterministic_abort` and a consumer that caches results knows not to.
#[test]
fn a_run_that_cannot_finish_is_cut_by_the_clock_and_says_so() {
    // 64 branch points on one path, each on a fresh symbol: the state cap is thousands of
    // states away and no machine reaches it in 50 ms.
    let p = write(
        "endless.c",
        "int f (unsigned a, unsigned b)\n\
         {\n\
         \x20 int t = 0;\n\
         \x20 for (unsigned i = 0; i < 8; i++)\n\
         \x20   for (unsigned j = 0; j < 8; j++)\n\
         \x20     if (((a >> i) & (b >> j)) & 1u) t += 1; else t -= 1;\n\
         \x20 return t;\n\
         }\n",
    );
    let started = std::time::Instant::now();
    let r = run(&[
        "find-bugs",
        p.to_str().expect("utf-8 path"),
        "--entry",
        "f",
        "--time-budget",
        "0.05",
        "--json",
    ]);
    let took = started.elapsed();
    assert_eq!(r.code, 0, "it ends by its own decision:\n{}", r.err);
    let v = json(&r);

    // The envelope is whole, not a fragment of one — the point is that a cut run is still an
    // answer with every qualification a complete one carries.
    for key in ["result", "fidelity", "proven", "assumptions", "blind_spots"] {
        assert!(!v[key].is_null(), "the envelope is missing `{key}`: {v}");
    }
    assert_eq!(v["proven"], false, "a cut search proves nothing: {v}");
    assert!(
        v["result"]["budgets"]["hit"]
            .as_array()
            .expect("budgets.hit")
            .iter()
            .any(|h| h.as_str().is_some_and(|s| s.contains("wall_clock"))),
        "and names the bound that cut it: {v}"
    );
    assert_eq!(
        v["nondeterministic_abort"], true,
        "the one answer here that is a measurement rather than a computation: {v}"
    );
    // A generous multiple of the budget: the assertion is that the clock is what ended it, not
    // that the process is fast. Without it the test passes on a build where `--time-budget` is
    // parsed and ignored and the run merely happens to finish.
    assert!(
        took < std::time::Duration::from_secs(20),
        "it stopped at the clock rather than at the state cap: {took:?}"
    );
}

/// **No clock is the default at the library, and the CLI's own default does not change an
/// answer that fits inside it.** 001 §5 wants byte-identical output for identical input; a
/// wall clock is the one thing that can break that, so a run that finishes well within it must
/// be indistinguishable from a run with none, `nondeterministic_abort` included.
#[test]
fn a_run_that_finishes_inside_its_clock_is_an_ordinary_answer() {
    let p = write("quick.c", "int f (int x) { return x / 0; }\n");
    let with = run(&[
        "find-bugs",
        p.to_str().expect("utf-8 path"),
        "--entry",
        "f",
        "--time-budget",
        "600",
        "--json",
    ]);
    let without = run(&[
        "find-bugs",
        p.to_str().expect("utf-8 path"),
        "--entry",
        "f",
        "--time-budget",
        "0", // 0 is no limit, as `timeout(1)` has it
        "--json",
    ]);
    assert_eq!(with.code, 0, "{}", with.err);
    assert_eq!(without.code, 0, "{}", without.err);
    assert_eq!(
        with.out, without.out,
        "a clock nothing ran into leaves no trace in the answer"
    );
    assert_eq!(
        json(&with)["nondeterministic_abort"],
        false,
        "it was never hit, so nothing here is a measurement"
    );
}

/// **A function pointer must not be called as a function with a different signature** — and a
/// process that aborts is the worst way to say so.
///
/// Found by sweeping 477 entry points across 92 VPP plugins: `plugins/perfmon/perfmon.c` and
/// `plugins/vmxnet3/vmxnet3_api.c` did not fail, they *panicked* —
///
/// ```text
/// assertion `left == right` failed: operand widths must match for Eq
/// ```
///
/// — inside the solver, from `chiero_exec::cmp`. The measurement recorded both as `failed`,
/// which is the row a file that will not preprocess gets, so two crashes on real code looked
/// like two files chiero could not read.
///
/// **The cause is one sentence in `Engine::indirect` that was not true of the code beneath it.**
/// It says "candidates are every defined function *whose signature could be called here*", and
/// the implementation takes every defined function in the module, capped at `max_indirect`. So
/// `(s->init_fn) (vm, s)` — a pointer to a function returning `clib_error_t *` — forked into a
/// candidate returning `unsigned char`, the caller compared that one-byte result against a null
/// pointer, and the term arena refused a 8-bit-to-64-bit `Eq`.
///
/// It is not only a crash. A path through a callee the program could never have called is a
/// path that does not exist, and every finding on it is about a program nobody wrote.
#[test]
fn an_indirect_call_does_not_enter_a_candidate_of_another_shape() {
    let p = write(
        "indirect_shape.c",
        "typedef struct err err_t;\n\
         typedef struct src { err_t *(*init_fn) (void *, struct src *); struct src *next; } src_t;\n\
         \n\
         /* A candidate with the same arity and a one-byte result. */\n\
         static unsigned char other (void *a, src_t *b) { return b ? 1 : 0; }\n\
         \n\
         int f (src_t *s, void *vm)\n\
         {\n\
         \x20 err_t *err;\n\
         \x20 if (s->init_fn && ((err = (s->init_fn) (vm, s))))\n\
         \x20   return 1;\n\
         \x20 return (int) other (vm, s);\n\
         }\n",
    );
    let r = run(&[
        "find-bugs",
        p.to_str().expect("utf-8 path"),
        "--entry",
        "f",
        "--json",
    ]);
    // **The process survives.** Everything else here is worth nothing if it does not.
    assert_eq!(
        r.code, 0,
        "chiero aborted on this program:\n{}\n{}",
        r.err, r.out
    );
    let v = json(&r);
    assert!(!v["result"].is_null(), "and produced an envelope: {v}");
}

/// **Storing a `_Bool` aborted the process, once the object had been promoted.**
///
/// The second panic from the 92-plugin sweep, `plugins/vmxnet3/vmxnet3_api.c`:
///
/// ```text
/// thread 'main' panicked at chiero-solver/src/lib.rs:710: extract out of range
/// ```
///
/// `mp->admin_up_down = (swif->flags & VNET_SW_INTERFACE_FLAG_ADMIN_UP) ? 1 : 0;` — an API
/// struct's `bool` field, three lines after a `strncpy` into the same struct. CIR types a
/// `_Bool` as `Int(1)` and `size_of_cty` rounds that to one byte, so the store asked memory to
/// write 8 bits of a term that has 1, and the array-backed write path extracted bits 7..0 of a
/// one-bit value.
///
/// **The byte-backed path never noticed**, because it does not decompose the term — which is
/// why this needed both halves to show up: a `_Bool` store *and* something that promoted the
/// object first. Every `bool` field in every VPP API handler is behind that pair.
///
/// C11 6.3.1.2 says what the byte holds — a conversion to `_Bool` yields 0 or 1 — so widening
/// the value to the store's size is the language's own rule, not a guess chiero makes.
#[test]
fn storing_a_bool_into_a_promoted_object_is_a_write_and_not_a_crash() {
    let p = write(
        "bool_store.c",
        "#include <string.h>\n\
         struct s { char name[8]; _Bool up; };\n\
         int f (struct s *p, char *src, int v)\n\
         {\n\
         \x20 strncpy (p->name, src, 7);   /* promotes the object */\n\
         \x20 p->up = (v & 4) ? 1 : 0;\n\
         \x20 return p->up ? 3 : 4;\n\
         }\n",
    );
    let r = run(&[
        "find-bugs",
        p.to_str().expect("utf-8 path"),
        "--entry",
        "f",
        "--json",
    ]);
    assert_eq!(
        r.code, 0,
        "chiero aborted on a `bool` field store:\n{}\n{}",
        r.err, r.out
    );
    let v = json(&r);
    assert!(!v["result"].is_null(), "and produced an envelope: {v}");
    // **The store happened.** A refusal that dropped the write would leave `p->up`
    // uninitialized and manufacture the read-of-uninitialized finding on the line after it —
    // 021 §3.1's confidently-wrong answer, arriving as a false positive.
    for f in v["result"]["findings"].as_array().expect("array") {
        let m = f["message"].as_str().unwrap_or_default();
        assert!(
            !m.contains("uninitialized"),
            "the line above wrote this byte: {m}"
        );
    }
}

/// **The eighth instance of the one confusion — and this time I introduced it.**
///
/// `elt->fp (data)` is VPP's callback-list idiom; `dhcp_api.c` has one. With an unresolvable
/// function pointer the engine forks over candidates, and a one-parameter candidate with an
/// `int` parameter — `__bsfd (int __X)` out of gcc's `ia32intrin.h`, which every VPP
/// translation unit includes — took a *pointer* argument. The 64-bit value did not fit the
/// 4-byte slot, so the store was refused, and the read after it was reported as
/// `uninitialized-read`.
///
/// **108 of the 133 findings on a 477-entry plugin sweep were that**, all naming `__X`. Two
/// separate mistakes, each worth its own sentence:
///
/// - a candidate whose parameter cannot hold the argument is not a call the program can make,
///   so it is not a candidate — arity was never the whole signature; and
/// - a store chiero cannot represent still *happened*. Refusing the value is right; leaving
///   the destination readable as never-written is 021 §6's false-positive storm, and it is the
///   same confusion §7.6 records seven times over: **chiero not knowing a value is not the
///   program failing to write one.** The bytes are havoc'd — symbolic and initialized — and
///   the envelope names the refusal.
#[test]
fn a_callback_list_does_not_report_an_intrinsics_parameter_as_uninitialized() {
    let p = write(
        "callback_list.c",
        // Verbatim from `ia32intrin.h`, because the point is that this is in every VPP TU.
        "extern __inline int\n\
         __attribute__((__gnu_inline__, __always_inline__, __artificial__))\n\
         __bsfd (int __X)\n\
         {\n\
         \x20 return __builtin_ctz (__X);\n\
         }\n\
         \n\
         typedef struct e { void *(*fp) (void *); struct e *next; } e_t;\n\
         void *f (void *data, e_t *elt)\n\
         {\n\
         \x20 void *r = 0;\n\
         \x20 while (elt) { r = elt->fp (data); if (r) return r; elt = elt->next; }\n\
         \x20 return r;\n\
         }\n",
    );
    let r = run(&[
        "find-bugs",
        p.to_str().expect("utf-8 path"),
        "--entry",
        "f",
        "--entry-ptr-nonnull",
        "--json",
    ]);
    let v = json(&r);
    for f in v["result"]["findings"].as_array().expect("array") {
        let m = f["message"].as_str().unwrap_or_default();
        assert!(
            !m.contains("uninitialized"),
            "nobody in this program passed a pointer to `__bsfd`: {m}"
        );
    }
}

/// **A frontend error names the line, and the file the error is really in.**
///
/// Sweeping 92 VPP plugins, eleven entries came back `failed` with sentences like
/// `expected a type specifier` and `` `clib_crc32c_with_init` was not declared `` — attributed
/// to the `.c` file on the command line, with no position at all. Every one of them needed a
/// separate reduction run to find out *where*, and for a construct chiero cannot parse the
/// answer is usually in a header the file included, not in the file itself.
///
/// The span was there all along: `Diagnostic` carries one and the `SourceMap` maps it to a
/// file and a line. The command threw it away.
#[test]
fn a_frontend_error_says_where_it_is() {
    let p = write(
        "syntax_error.c",
        "int ok (void) { return 1; }\n\
         \n\
         int bad (void) { return @@@; }\n",
    );
    let r = run(&[
        "find-bugs",
        p.to_str().expect("utf-8 path"),
        "--entry",
        "ok",
        "--json",
    ]);
    assert_ne!(r.code, 0, "this does not parse:\n{}", r.out);
    assert!(
        r.err.contains(":3:"),
        "the error names the line it is on: {}",
        r.err
    );
}

/// **023 §8's deterministic solver budget, reachable from a command line — and biting.**
///
/// The engine gained `Budget::max_solver_rlimit` and nothing could set it, which is the state
/// §8 called "specified and not built" wearing a different hat.
///
/// ⚠️ **The first version of this test asserted only that the flag was accepted and that the
/// answer stayed deterministic, and a mutant deleting the two lines that carry the value into
/// the run survived it.** The neighbouring `--time-budget` test warns about exactly that in its
/// own comment — "without it the test passes on a build where `--time-budget` is parsed and
/// ignored" — and this one was written anyway. So the assertion is now about an answer that
/// *changes*: a budget too small to decide the path leaves the finding unwitnessed and
/// `Unknown`, a generous one solves it, and the difference is visible from a terminal.
///
/// The contrast with `--time-budget` is why this is a second flag and not a smaller value of
/// the first. A clock is precisely what 023 §8.1 forbids from gating output — a run cut by one
/// is `nondeterministic_abort`. Work units do not move with load, so a run cut by them stays an
/// ordinary answer, asserted here directly.
#[test]
fn the_solver_budget_is_reachable_and_stays_deterministic() {
    // Nested rather than `&&`, and dividing by zero at the bottom: the finding's witness is
    // what needs a solver, so the budget has something to bite on. A flatter fixture was
    // decided by tier 1 alone and the flag could not have changed anything.
    let p = write(
        "hard.c",
        "int f (unsigned a, unsigned b)\n\
         {\n\
         \x20 unsigned p = a * b;\n\
         \x20 if (p == 7u) { if (a > 1u) { if (b > 1u) { int z = 0; return 5 / z; } } }\n\
         \x20 return 0;\n\
         }\n",
    );
    let at = |budget: &str| {
        let path = p.to_str().expect("utf-8 path").to_string();
        let owned = [
            "find-bugs".to_string(),
            path,
            "--entry".to_string(),
            "f".to_string(),
            "--solver-rlimit".to_string(),
            budget.to_string(),
            "--json".to_string(),
        ];
        let borrowed: Vec<&str> = owned.iter().map(String::as_str).collect();
        run(&borrowed)
    };

    let generous = at("50000");
    assert_eq!(generous.code, 0, "{}", generous.err);
    let g = json(&generous);
    let gf = &g["result"]["findings"][0];
    if gf["fidelity"] != "Exact" {
        // No backend on PATH: tier 1 answers everything and no budget can change that, so
        // there is nothing here to measure. Saying so beats a green tick over an unasked
        // question.
        eprintln!("SKIP: no SMT backend, so the solver budget has nothing to bite on");
        return;
    }
    assert!(
        gf["witness"].is_array(),
        "a generous budget solves the witness: {g}"
    );

    // **2000, not 1.** Measured: at `:rlimit 1` z3 is too starved to run `(push 1)` and
    // answers with an `(error ...)` line instead, which chiero reports as a backend that gave
    // no usable answer -- honest, and a different sentence from the one this test is about.
    let tight = at("2000");
    assert_eq!(
        tight.code, 0,
        "a spent budget is an answer, not a failure:\n{}",
        tight.err
    );
    let t = json(&tight);
    for key in ["result", "fidelity", "proven", "assumptions", "blind_spots"] {
        assert!(!t[key].is_null(), "the envelope is missing `{key}`: {t}");
    }
    let tf = &t["result"]["findings"][0];
    assert_eq!(
        tf["fidelity"], "Unknown",
        "the budget stopped the solver, so the finding cannot claim exactness: {t}"
    );
    assert!(
        tf["unwitnessed"]
            .as_str()
            .is_some_and(|w| w.contains("ResourceLimit")),
        "and it names the budget rather than blaming the backend: {t}"
    );
    // **The half `--time-budget` cannot satisfy.** Work units do not move with load, so a run
    // cut by them is reproducible and must not be branded a measurement.
    assert_eq!(
        t["nondeterministic_abort"], false,
        "a deterministic budget is not a clock: {t}"
    );
    assert_eq!(
        tight.out,
        at("2000").out,
        "the same budget cuts the same query in the same place"
    );
}

/// **A budget nobody set stays unset**, and the flag is rejected rather than ignored when it is
/// nonsense.
///
/// The first half matters more than it looks: arming `:rlimit` *displaces* the backend's
/// `:timeout`, so a default leaking in would quietly disarm half the watchdog for every run.
#[test]
fn the_solver_budget_is_off_unless_asked_for_and_refuses_nonsense() {
    let p = write("triv.c", "int f (int x) { return x / 0; }\n");
    let bare = run(&[
        "find-bugs",
        p.to_str().expect("utf-8 path"),
        "--entry",
        "f",
        "--json",
    ]);
    let zero = run(&[
        "find-bugs",
        p.to_str().expect("utf-8 path"),
        "--entry",
        "f",
        "--solver-rlimit",
        "0", // 0 is no limit, the same reading `--time-budget` gives it
        "--json",
    ]);
    assert_eq!(bare.code, 0, "{}", bare.err);
    assert_eq!(zero.code, 0, "{}", zero.err);
    assert_eq!(
        bare.out, zero.out,
        "an explicit `no limit` and saying nothing are the same run"
    );

    let bad = run(&[
        "find-bugs",
        p.to_str().expect("utf-8 path"),
        "--entry",
        "f",
        "--solver-rlimit",
        "lots",
    ]);
    assert_ne!(
        bad.code, 0,
        "a budget that is not a number is a usage error"
    );
    assert!(
        bad.err.contains("--solver-rlimit"),
        "and the message names the flag: {}",
        bad.err
    );
}

/// **The budget reaches `prove-equivalent` too, which it did not.**
///
/// A solver is built in three places — `Engine::new_solver`, `chiero-tool`'s witness solver,
/// and `chiero-opt`'s equivalence solver — and the first attempt at `max_solver_rlimit` reached
/// only the first. `prove-equivalent` accepted `--solver-rlimit` and ignored it, which is the
/// same accepted-and-ignored defect the flag exists to end, one command over.
///
/// ⚠️ The fixture is nonlinear on purpose. `x * 2` against `x << 1` — 041's own headline
/// example — is settled by tier 1 without a backend, so a budget has nothing to bite on and a
/// test built on it passes whatever the plumbing does. Confirmed by counting dumped queries
/// before writing this: seven for the pair below, and the difference in verdict is real.
#[test]
fn the_solver_budget_reaches_prove_equivalent() {
    let before = write(
        "eq_before.c",
        "unsigned f (unsigned x) { if (x * x == 49u) return 1u; return 0u; }\n",
    );
    let after = write(
        "eq_after.c",
        "unsigned f (unsigned x) { if (x == 7u || x == 4294967289u) return 1u; return 0u; }\n",
    );
    let at = |budget: &str| {
        let owned = [
            "prove-equivalent".to_string(),
            before.to_str().expect("utf-8 path").to_string(),
            after.to_str().expect("utf-8 path").to_string(),
            "--entry".to_string(),
            "f".to_string(),
            "--solver-rlimit".to_string(),
            budget.to_string(),
            "--json".to_string(),
        ];
        let borrowed: Vec<&str> = owned.iter().map(String::as_str).collect();
        run(&borrowed)
    };

    let generous = at("50000000");
    assert_eq!(generous.code, 0, "{}", generous.err);
    let g = json(&generous);
    if g["proven"] != true {
        eprintln!("SKIP: no SMT backend, so the solver budget has nothing to bite on");
        return;
    }
    assert_eq!(
        g["result"]["verdict"], "differs",
        "a generous budget decides it: {g}"
    );

    let tight = at("2000");
    assert_eq!(
        tight.code, 0,
        "a spent budget is an answer, not a failure:\n{}",
        tight.err
    );
    let t = json(&tight);
    assert_eq!(
        t["result"]["verdict"], "unknown",
        "and a budget too small to decide must say so rather than guess: {t}"
    );
    assert_eq!(t["proven"], false, "nothing was proven under it: {t}");
    assert_eq!(
        t["nondeterministic_abort"], false,
        "work units are not a clock: {t}"
    );
}

/// **And it reaches `check-reachable`'s own witness solver, the third of the three.**
///
/// `chiero-tool::witness_for_path` builds a solver outside `Engine`, because a state that
/// merely *arrived* somewhere carries no finding and therefore no witness — so the path
/// condition is solved there instead. `Engine::new_solver` cannot reach it, and the first two
/// tests in this group cannot see it: on the `find-bugs` path the engine has already produced
/// the witness, so zeroing this site changed nothing and the mutant survived them both.
///
/// What it is worth is visible in the tight run: the witness comes back **unpinned** rather
/// than zeros presented as the solver's answer, which is the distinction 023 §9's `Witness`
/// exists to make.
#[test]
fn the_solver_budget_reaches_check_reachables_witness() {
    let p = write(
        "reach.c",
        "int f (unsigned a, unsigned b)\n\
         {\n\
         \x20 unsigned p = a * b;\n\
         \x20 if (p == 7u) { if (a > 1u) { if (b > 1u) { int z = 0; return 5 / z; } } }\n\
         \x20 return 0;\n\
         }\n",
    );
    let at = |budget: &str| {
        let owned = [
            "check-reachable".to_string(),
            p.to_str().expect("utf-8 path").to_string(),
            "--entry".to_string(),
            "f".to_string(),
            "--line".to_string(),
            "4".to_string(),
            "--solver-rlimit".to_string(),
            budget.to_string(),
            "--json".to_string(),
        ];
        let borrowed: Vec<&str> = owned.iter().map(String::as_str).collect();
        run(&borrowed)
    };

    let generous = at("50000000");
    assert_eq!(generous.code, 0, "{}", generous.err);
    let g = json(&generous);
    if g["result"]["verdict"] != "reachable" {
        eprintln!("SKIP: no SMT backend, so the solver budget has nothing to bite on");
        return;
    }
    assert!(
        g["result"]["witness"]
            .as_array()
            .expect("witness")
            .iter()
            .all(|b| b["pinned"] == true),
        "a generous budget solves every input: {g}"
    );

    let tight = at("2000");
    assert_eq!(tight.code, 0, "{}", tight.err);
    let t = json(&tight);
    assert_eq!(
        t["result"]["verdict"], "not_shown_reachable",
        "the budget stopped the solver, so arrival is not shown: {t}"
    );
    assert!(
        t["result"]["witness"]
            .as_array()
            .expect("witness")
            .iter()
            .all(|b| b["pinned"] == false),
        "and the inputs are unpinned rather than zeros wearing the solver's authority: {t}"
    );
}

/// **`chiero cir` — a window into what lowering actually produced.**
///
/// 020 makes the textual format normative and round-tripping it a contract, and the printer has
/// been round-trip tested since. Until now **nothing outside Rust could see it**: no operation
/// prints a module, so every question about what a function lowered to was answered by reading
/// `chiero-lower` and guessing.
///
/// That cost a real investigation on 2026-08-08. A `pointer-outside-object` finding on
/// `vnet/dev/counters.c` put four independently-verified facts in contradiction — the offset
/// check probes the path condition, the `PtrAdd` is downstream of the guard, reads are stable in
/// every memory representation, and the report only fires on `Sat` — and settling *which* is
/// false needs to know which term the guard constrains and which the `PtrAdd` uses. That is one
/// `grep` over the lowered CIR and unanswerable without it. Four probes went into hypotheses
/// first; this is the instrument that would have replaced them.
///
/// Not `get_cfg` (050 lists that operation and it remains unbuilt): this prints the *whole*
/// module in 020's format rather than a block/edge summary, which is what a reader debugging
/// lowering needs and a superset of what `get_cfg` would show.
#[test]
fn cir_prints_the_lowered_module_in_the_normative_format() {
    let p = write(
        "dump.c",
        "int f (int x)\n{\n  if (x > 3) return 1;\n  return 2;\n}\n",
    );
    let r = run(&["cir", p.to_str().expect("utf-8 path")]);
    assert_eq!(r.code, 0, "{}", r.err);

    // The shape 020 specifies: a target line, a `func` header, named blocks, a terminator.
    for want in ["target ", "func @f(", "entry:", "ret "] {
        assert!(
            r.out.contains(want),
            "the dump is missing `{want}`, so it is not 020's format:\n{}",
            r.out
        );
    }
    // **The branch has to be there.** Without it this passes on a dump that prints headers and
    // drops the body, which is the shape of every "it printed something" test.
    assert!(
        r.out.contains("br "),
        "the `if` lowered to a conditional branch and the dump must show it:\n{}",
        r.out
    );
    // **And it must round-trip**, which is what makes it the normative format rather than a
    // rendering: 020 contract 5 says a well-formed module re-parses.
    //
    // ⚠️ Compared against the output **minus the newline `main`'s `println!` adds**. The printer
    // already ends the module with one; the extra is the CLI's, not the format's. Trimming it
    // here rather than changing the printer is deliberate — the round trip is 020 contract 5's
    // property of the *format*, and a printer altered to satisfy a test about stdout would break
    // every golden that depends on it.
    let dump = format!("{}\n", r.out.trim_end_matches('\n'));
    let m = chiero_cir::text::parse(&dump).expect("the dump must re-parse (020's round trip)");
    assert_eq!(
        chiero_cir::text::print(&m),
        dump,
        "printing the re-parsed module must reproduce the dump byte for byte"
    );
}

/// **`--march` changes which code exists** — HANDOFF §9.1's parked item, and the reason it
/// mattered.
///
/// `frontend::persona` probed the compiler with **no flags** while VPP builds `-march=x86-64-v2`,
/// so `__SSE4_2__` and `__AVX2__` were undefined and every guarded path was invisible. Measured
/// consequence: every 32-byte type in VPP lives in `vppinfra/vector_avx2.h` under
/// `#if defined(__AVX2__)`, so **every AVX2 and AVX512 path in vppinfra had never once been
/// compiled by any chiero measurement** — silently, because absent code reports nothing.
///
/// The three levels are the assertion: a flag that only ever *added* macros could not produce the
/// middle row, and one that ignored its argument could not produce the third.
///
/// ⚠️ The `always` function is in every row on purpose. The first version of this flag forgot to
/// skip its own argument, so `--march x86-64-v2 file.c` parsed as **two** input files and the
/// command failed outright — printing nothing at all, which a test asserting only the presence of
/// `has_sse42` would have reported as a plain absence.
#[test]
fn march_selects_the_persona_and_therefore_which_functions_exist() {
    // ⚠️ A *private* subdirectory. `scratch()` is shared per-process, and the first version of
    // this test removed it at the end — deleting other tests' fixtures mid-run.
    let dir = scratch().join("march");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("march.c");
    std::fs::write(
        &src,
        "#if defined(__SSE4_2__)\nint has_sse42(void) { return 1; }\n#endif\n\
         #if defined(__AVX2__)\nint has_avx2(void) { return 1; }\n#endif\n\
         int always(void) { return 0; }\n",
    )
    .unwrap();

    let names = |args: &[&str]| -> Vec<String> {
        let out = Command::new(bin())
            .arg("cir")
            .args(args)
            .arg(&src)
            .output()
            .expect("chiero cir");
        assert!(
            out.status.success(),
            "chiero failed with {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.strip_prefix("func @"))
            .filter_map(|l| l.split('(').next())
            .map(str::to_owned)
            .collect()
    };

    assert_eq!(
        names(&[]),
        ["always"],
        "no -march: neither guard is satisfied"
    );
    assert_eq!(
        names(&["--march", "x86-64-v2"]),
        ["has_sse42", "always"],
        "v2 has SSE4.2 and not AVX2"
    );
    assert_eq!(
        names(&["--march", "x86-64-v3"]),
        ["has_sse42", "has_avx2", "always"],
        "v3 adds AVX2 — the paths no chiero measurement had ever compiled"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **A 32-byte vector store past the end of a 16-byte object is caught, from C, `Exact`.**
///
/// This path was unreachable until 2026-08-08 and untested after it. VPP's baseline is
/// `-march=x86-64-v2`, `__AVX2__` is defined only at v3 and above, and none of the pinned 40
/// entries is compiled at either — so no measurement this project published had ever lowered a
/// 256-bit vector access.
///
/// ⚠️ **And it does not go through the wide-load path anything else tests.** `chiero-exec`'s
/// `a_width_limit_does_not_mask_a_use_after_free` calls `Memory::read_term(.., 32, ..)` directly;
/// a `vector_size(32)` access from C lowers to **`copymem`** instead — 020 §4.13b's "no aggregate
/// values in CIR" applied to vector types, and the reason `unsupported-access-width` never fires
/// on VPP's vector code. The AVX-512 lowering of one VPP TU holds 7779 `copymem` of 32 bytes or
/// wider, so this is the shape that half of vppinfra actually produces.
///
/// The bug shape is a real one: "adjacent packet overwrite with very big packets" is a VPP fix
/// in this repo's own replay-candidate list.
#[test]
fn a_wide_vector_store_past_the_end_of_an_object_is_caught() {
    let src = "\
typedef unsigned char u8x32 __attribute__ ((vector_size (32)));
typedef unsigned char u8x16 __attribute__ ((vector_size (16)));

void vec_oob (u8x32 *src)
{
  unsigned char buf[16];
  u8x32 *p = (u8x32 *) buf;
  *p = *src;
}

void vec_ok (u8x16 *src)
{
  unsigned char buf[16];
  u8x16 *p = (u8x16 *) buf;
  *p = *src;
}
";
    let f = write("vec_oob.c", src);
    let p = f.to_str().unwrap();

    let r = run(&[
        "find-bugs",
        p,
        "--entry",
        "vec_oob",
        "--entry-ptr-nonnull",
        "--json",
    ]);
    let v = json(&r);
    let findings = v["result"]["findings"].as_array().expect("findings");
    let msg = findings
        .iter()
        .map(|f| f["message"].as_str().unwrap_or(""))
        .find(|m| m.starts_with("out-of-bounds"))
        .unwrap_or_else(|| panic!("no out-of-bounds finding: {v}"));
    assert!(
        msg.contains("32-byte") && msg.contains("16 bytes"),
        "the report names the width that overran and the object it overran: {msg}"
    );
    assert_eq!(
        findings[0]["fidelity"].as_str(),
        Some("Exact"),
        "a concrete 32-into-16 store needs no approximation: {v}"
    );

    // **The negative half.** A checker that flagged every vector store has to fail something,
    // and this is the assertion written for it — though in practice the message check above
    // fires first: with a mutant flagging every access of 16 bytes or more, the first
    // out-of-bounds finding is no longer the 32-into-16 one, so naming the width catches it
    // before the clean case does. Both were run; the mutant dies twice over.
    let ok = json(&run(&[
        "find-bugs",
        p,
        "--entry",
        "vec_ok",
        "--entry-ptr-nonnull",
        "--json",
    ]));
    assert_eq!(
        ok["result"]["findings"].as_array().map(Vec::len),
        Some(0),
        "a 16-byte vector store into a 16-byte object is in bounds: {ok}"
    );
}

/// **An advisory diagnostic must not throw away a translation unit chiero understood.**
///
/// `chiero cir` stopped at the *first* sema diagnostic at every stage, and `SemaDiagnostic`
/// had no severity, so "I could not model this" and "I did, and here is a concern" were the
/// same event. The discriminator is a signed-overflowing constant expression: chiero folds it
/// to `-2147418114` — byte for byte what gcc and clang fold it to — and then refused the
/// whole file, which gcc compiles with exit 0 and a `-Woverflow` warning.
///
/// Found by auditing the class rather than the site: the identical conflation had already
/// cost three waves inside test harnesses (a `Gap` filed as a `Discarded`, a pedantic
/// `__int128` sentence filed as a refusal, and a diagnostic read before a value). This is the
/// same shape in product code, where it means a file the project's own compiler builds cannot
/// be analysed at all.
#[test]
fn an_advisory_diagnostic_does_not_abort_a_translation_unit() {
    let dir = std::env::temp_dir().join(format!("chiero-advisory-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let c = dir.join("advisory.c");
    std::fs::write(
        &c,
        "int probe(void) { int a[(0x7fffffff + 65535) ? 4 : 4]; a[0] = 1; return a[0]; }\n",
    )
    .expect("write");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_chiero"))
        .arg("cir")
        .arg(&c)
        .output()
        .expect("run chiero");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        out.status.success(),
        "gcc compiles this file with exit 0 and a warning; chiero must not refuse it.\n\
         stderr: {stderr}"
    );
    assert!(
        stdout.contains("func @probe"),
        "and it must actually lower the function, not merely exit 0:\n{stdout}"
    );
    // **The concern is still reported.** Silencing it would be the opposite defect, and a
    // worse one: an advisory that nobody sees is the same as no advisory.
    assert!(
        stderr.contains("signed overflow"),
        "the advisory must still be printed — downgrading a diagnostic is not deleting it:\n\
         {stderr}"
    );
}

/// **An advisory must not demote the next diagnostic, and must not hide a real error.**
///
/// Both halves were introduced by the severity work itself and found by an adversarial review
/// of it, which is what that step of the protocol is for.
///
/// **The leak.** `Cx::error` returns early when `quiet > 0` — it is silent while re-resolving
/// something already resolved — and the early return happened *before* the one-shot
/// `next_severity` reset. A parameter list is re-resolved quietly, and an array bound in one
/// reaches `wrap`, so an advisory raised there armed the flag and never disarmed it. The next
/// diagnostic emitted **anywhere in the translation unit** was demoted to `Advisory`, and with
/// the CLI no longer stopping on advisories, that demoted *Error* reached lowering. Ambient
/// one-shot state was the wrong mechanism; severity is a parameter now.
///
/// **The mask.** The anti-cascade guard around an array length counts *any* new diagnostic to
/// decide the length is unusable. It was written when the overflow was an Error, so counting
/// advisories was moot. It is not moot now: the advisory suppressed the negative-length Error
/// and chiero accepted a translation unit gcc refuses outright.
#[test]
fn an_advisory_neither_demotes_the_next_diagnostic_nor_hides_an_error() {
    let dir = std::env::temp_dir().join(format!("chiero-adv2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let run = |name: &str, src: &str| {
        let c = dir.join(name);
        std::fs::write(&c, src).expect("write");
        std::process::Command::new(env!("CARGO_BIN_EXE_chiero"))
            .arg("cir")
            .arg(&c)
            .output()
            .expect("run chiero")
    };

    // **The leak.** The overflow in the parameter's bound is an advisory; the undeclared name
    // is an Error and must still refuse the file.
    let leak = run(
        "leak.c",
        "int g(int a[0x7fffffff + 0x7fffffff]) { return undeclared_name; }\n",
    );
    assert!(
        !leak.status.success(),
        "an undeclared name is an Error whatever came before it; exit was 0 and lowering \
         received `ret undef`.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&leak.stdout),
        String::from_utf8_lossy(&leak.stderr)
    );

    // **The mask.** gcc: "size of array 'arr' is negative", exit 1.
    let neg = run("neg.c", "int arr[0x7fffffff + 0x7fffffff];\n");
    assert!(
        !neg.status.success(),
        "gcc refuses this outright; chiero emitted `global @arr : size 0` and exited 0.\n\
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&neg.stdout),
        String::from_utf8_lossy(&neg.stderr)
    );

    // **The discriminator, and the behaviour the severity work exists for.** The original
    // case must still succeed, or this test would pass against a revert.
    let ok = run(
        "ok.c",
        "int probe(void) { int a[(0x7fffffff + 65535) ? 4 : 4]; a[0] = 1; return a[0]; }\n",
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        ok.status.success(),
        "gcc compiles this with exit 0 and a warning: {}",
        String::from_utf8_lossy(&ok.stderr)
    );
}
