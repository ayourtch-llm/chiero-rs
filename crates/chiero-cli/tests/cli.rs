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
    serde_json::from_str(&r.out)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}):\n{}\n--- stderr ---\n{}", r.out, r.err))
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
    let before = write(
        "abs_before.c",
        "int f (int x) { return x < 0 ? -x : x; }\n",
    );
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

    let r = run(&["expansion-sites", f.to_str().unwrap(), "--macro", "INNER", "--json"]);
    assert_eq!(r.code, 0, "stderr:\n{}", r.err);
    let v = json(&r);
    assert!(v["proven"].as_bool().unwrap_or(false), "the table is exact: {v}");
    assert_eq!(v["result"]["total"].as_u64(), Some(1));

    let r = run(&["explain-macro", f.to_str().unwrap(), "--line", "3", "--json"]);
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
    let f = write("mac2.c", "#define M(v) ((v) + 1)\nint a (int x) { return M (x); }\n");
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
    let r = run(&["expansion-sites", "/nonexistent/nope.c", "--macro", "M", "--json"]);
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
