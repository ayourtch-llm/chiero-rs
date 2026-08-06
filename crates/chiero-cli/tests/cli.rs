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
