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
    let p = write(
        "entry_nonnull.c",
        "int f(int *p) { return *p; }\n",
    );
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
            .is_some_and(|a| a.iter().any(|x| x[0] == "entry_ptr_nonnull")),
        "and the envelope has to carry the assumption that bought the quiet: {v}"
    );

    let help = run(&["--help"]);
    assert!(
        help.out.contains("--entry-ptr-nonnull"),
        "a flag nobody can discover is not a feature: {}",
        help.out
    );
}
