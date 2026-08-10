//! **Do the defect checkers fire at all?** A corpus with known ground truth, both ways.
//!
//! Every VPP sweep this project has published reports two kinds — `null-dereference` and one
//! `division-by-zero` — while the vocabulary has nine (`out-of-bounds`, `use-after-free`,
//! `use-after-scope`, `uninitialized-read`, `pointer-outside-object`, `wild-pointer`, …). A
//! `findings: 0` over VPP therefore has two readings that no measurement here could tell apart:
//! **the code is clean**, or **the checker never fires**.
//!
//! The hand fixtures in `chiero-tool/tests/find_bugs.rs` are 16 cases of hand-written *CIR*.
//! This drives **C through the real CLI**, which is the path every published number took.
//!
//! **Both directions, and the second is the one that makes the first mean something.** Each
//! defect is paired with a control that differs by as little as possible — one initializer, one
//! index — and the control must report nothing. A checker that fires on everything scores 100%
//! recall, which is why recall alone is not evidence.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_chiero")
}

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("chiero-defects-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

/// Run `find-bugs` over `src` and return the messages of whatever it found.
fn findings(name: &str, src: &str) -> (Vec<String>, String) {
    let p = scratch().join(name);
    std::fs::write(&p, src).expect("write");
    let out = Command::new(bin())
        .args([
            "find-bugs",
            p.to_str().unwrap(),
            "--entry",
            "probe",
            "--json",
            "--no-system-headers",
        ])
        .output()
        .expect("spawn");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => panic!(
            "envelope is not JSON ({e}): {text}\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
    };
    let msgs = v["result"]["findings"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|f| f["message"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    (msgs, text)
}

/// `(name, defective source, control source, the kind the defect must be reported as)`
///
/// The controls are deliberately minimal edits: `p = &v` instead of `p = 0`, index `1` instead
/// of `9`, divisor `2` instead of `0`. A pair that differed in more than the defect would let a
/// checker pass by reacting to the difference rather than to the fault.
const CASES: &[(&str, &str, &str, &str)] = &[
    (
        "null_deref",
        "int probe(void) { int *p = 0; return *p; }",
        "int probe(void) { int v = 7; int *p = &v; return *p; }",
        "null-dereference",
    ),
    (
        "oob_write",
        // ⚠️ **`a[1]` is written before it is read on both sides, and that is not cosmetic.**
        // The first version of this pair returned `a[0]` without writing it, so the *control*
        // contained a real uninitialised read — and this gate caught it on its first run,
        // before it caught anything about chiero. A control with a second defect measures
        // nothing.
        "int probe(void) { int a[4]; a[1] = 2; a[9] = 1; return a[1]; }",
        "int probe(void) { int a[4]; a[1] = 2; a[3] = 1; return a[1]; }",
        "out-of-bounds",
    ),
    (
        "div_zero",
        "int probe(void) { int z = 0; return 10 / z; }",
        "int probe(void) { int z = 2; return 10 / z; }",
        "division-by-zero",
    ),
    // **The four below are the point of the corpus.** No VPP sweep this project has published
    // has ever reported one of them; the vocabulary has them and the measurements never do.
    (
        "use_after_free",
        "void *malloc(unsigned long); void free(void *);\n         int probe(void) { int *p = malloc(4); if (!p) return 0; *p = 1; free(p); return *p; }",
        "void *malloc(unsigned long); void free(void *);\n         int probe(void) { int *p = malloc(4); if (!p) return 0; *p = 1; int v = *p; free(p); return v; }",
        "use-after-free",
    ),
    (
        "use_after_scope",
        "int probe(void) { int *p; { int x = 1; p = &x; } return *p; }",
        "int probe(void) { int x = 1; int *p = &x; return *p; }",
        "use-after-scope",
    ),
    (
        "wild_pointer",
        "int probe(void) { int *p = (int *) 0x1234; return *p; }",
        "int probe(void) { int v = 1; int *p = &v; return *p; }",
        "wild-pointer",
    ),
    (
        "pointer_outside_object",
        "int probe(void) { int a[4]; a[0] = 1; int *p = a + 8; return (int) (p != 0); }",
        "int probe(void) { int a[4]; a[0] = 1; int *p = a + 4; return (int) (p != 0); }",
        "pointer-outside-object",
    ),
    (
        "uninit_read",
        "int probe(void) { int x; return x + 1; }",
        "int probe(void) { int x = 0; return x + 1; }",
        "uninitialized",
    ),
];

#[test]
fn an_injected_defect_is_reported_and_its_control_is_not() {
    let mut caught = Vec::new();
    let mut missed = Vec::new();
    let mut false_positive = Vec::new();

    for (name, bad, good, kind) in CASES {
        let (bad_msgs, bad_raw) = findings(&format!("{name}_bad.c"), bad);
        if bad_msgs.iter().any(|m| m.contains(kind)) {
            caught.push(*name);
        } else {
            missed.push((*name, *kind, bad_msgs.clone(), bad_raw));
        }
        let (good_msgs, _) = findings(&format!("{name}_good.c"), good);
        if !good_msgs.is_empty() {
            false_positive.push((*name, good_msgs));
        }
    }

    // **The control half is a hard assertion**: a finding on code with nothing wrong is a defect
    // in chiero whatever the recall number says.
    assert!(
        false_positive.is_empty(),
        "clean controls must report nothing: {false_positive:#?}"
    );

    // **The recall half is recorded rather than demanded, and the reason is on the record.**
    // Which checkers exist and which fire is exactly what this corpus was built to *find out*;
    // asserting a number chosen before the first run would pin today's behaviour as correct.
    // A `missed` entry is a lead, not necessarily a defect — some kinds may need a shape these
    // four-line programs do not reach.
    eprintln!(
        "injected-defect recall: {}/{} — caught {caught:?}",
        caught.len(),
        CASES.len()
    );
    for (name, kind, got, _) in &missed {
        eprintln!("  MISSED {name}: expected `{kind}`, got {got:?}");
    }

    // **What the two standing misses are, measured 2026-08-10 — neither is silence.**
    //
    // `wild_pointer` — `*(int *)0x1234` is reported as
    //   *"uninitialized-read: read at offset 0 of p touches bit 0, which was never written"*.
    //   chiero knows exactly what happened; the envelope carries *"IntToPtr of an integer with
    //   no provenance: the object was found by address"*. `wild-pointer` exists
    //   (`MemFault::WildPointer`) and fires in `chiero-mem`'s own tests, so this is a
    //   **misclassification, not a gap** — and the message names `p`, the pointer *variable*,
    //   for a fault about the invented object it points at, which sends a reader to the wrong
    //   line. A write through the same pointer reports the same thing.
    //
    // `pointer_outside_object` — forming `a + 8` reports nothing; *dereferencing* it reports
    //   `out-of-bounds: 4-byte access at offset 32 of a, which is 16 bytes`. So chiero reports
    //   the access and not the formation, while C 6.5.6p8 makes forming it undefined on its
    //   own. A real difference from the standard, and a small one — the access is the part that
    //   hurts.
    //
    // Both are recorded rather than asserted: pinning today's kind would make a future fix look
    // like a regression.

    // What *is* asserted: the corpus must not be silently inert. If nothing is caught at all,
    // either the checkers are off or this harness is not reaching them, and both are findings.
    assert!(
        !caught.is_empty(),
        "no injected defect was caught at all — the harness or the checkers are not running"
    );
}
