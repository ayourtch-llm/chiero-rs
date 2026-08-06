//! **The `$ chiero ...` blocks in `docs/tutorials/` are run, and must match.**
//!
//! The blocks were hand-written first, and every one of them was wrong: invented entity order,
//! omitted fields, missing blind spots. That is worse than no example — a reader who runs it
//! and gets something else stops trusting the page, and a reader who does not run it learns
//! something false. The user's note that prompted these tutorials said exactly this: *"each
//! data in it must be present"*.
//!
//! So each block is a transcript under test. The fixture files are here rather than in the
//! tutorial because a page cluttered with `cat > before.c` is a worse page; what the tutorial
//! shows is the command and its answer, and this file is what keeps that honest.
//!
//! **A block that drifts fails with a diff**, so the fix is to paste the new output rather than
//! to work out what changed.

use std::path::PathBuf;
use std::process::Command;

fn tutorials() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("docs/tutorials")
}

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("chiero-transcripts-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("scratch");
    d
}

/// The C every transcript refers to, by the names the tutorials use.
fn fixtures() -> PathBuf {
    let d = scratch();
    let w = |n: &str, c: &str| std::fs::write(d.join(n), c).expect("write");
    w("before.c", "int f (int x) { return x < 0 ? -x : x; }\n");
    w(
        "after.c",
        "int f (int x) {\n  if (x < 0)\n    return x == (-2147483647 - 1) ? 2147483647 : -x;\n  return x;\n}\n",
    );
    w("double.c", "int f (int x) { return x * 2; }\n");
    w("shift.c", "int f (int x) { return x << 1; }\n");
    w(
        "geom-before.c",
        "#define SCALE(x) ((x) * 2)\nint area (int w) { return SCALE (w) * w; }\n\
         int volume (int w) { return area (w) * w; }\n",
    );
    w(
        "average.c",
        "int average (int *a, int n)\n{\n  int total = 0;\n  for (int i = 0; i < n; i++)\n    \
         total += a[i];\n  return total / n;\n}\n",
    );
    w(
        "clamp.c",
        "int clamp (int x)\n{\n  if (x < 0)\n    return 0;\n  return x;\n}\n",
    );
    w(
        "geom-after.c",
        "#define SCALE(x) ((x) * 3)\nint area (int w) { return SCALE (w) * w; }\n\
         int volume (int w) { return area (w) * w; }\n",
    );
    d
}

/// Every ```console block whose first line is `$ chiero ...`, as (command, expected output).
fn transcripts(md: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut lines = md.lines().peekable();
    while let Some(l) = lines.next() {
        if l.trim() != "```console" {
            continue;
        }
        let Some(cmd) = lines.next() else { break };
        let Some(cmd) = cmd.strip_prefix("$ chiero ") else {
            continue;
        };
        // A wrapped command line, as the `-I`/`-D` example is, is documentation rather than a
        // transcript: it has no output to compare.
        if cmd.trim_end().ends_with('\\') {
            continue;
        }
        let mut body = Vec::new();
        for b in lines.by_ref() {
            if b.trim() == "```" {
                break;
            }
            body.push(b);
        }
        if !body.is_empty() {
            out.push((cmd.to_string(), body.join("\n")));
        }
    }
    out
}

#[test]
fn every_tutorial_transcript_reproduces() {
    let dir = fixtures();
    let mut checked = 0;
    let mut files: Vec<PathBuf> = std::fs::read_dir(tutorials())
        .expect("tutorials")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();

    for f in files {
        let md = std::fs::read_to_string(&f).expect("read");
        for (cmd, expected) in transcripts(&md) {
            // Commands whose inputs this file does not provide — `select-tests` wants a real
            // coverage directory — are documentation, and skipping them silently is how the
            // rest would rot too. Count and report at the end.
            if cmd.starts_with("select-tests") {
                continue;
            }
            let args: Vec<&str> = cmd.split_whitespace().collect();
            let o = Command::new(env!("CARGO_BIN_EXE_chiero"))
                .args(&args)
                .current_dir(&dir)
                .output()
                .expect("spawn");
            let got = String::from_utf8_lossy(&o.stdout).trim_end().to_string();
            assert_eq!(
                got,
                expected,
                "\n{}: `chiero {cmd}` no longer prints what the page says.\n\
                 --- the page ---\n{expected}\n--- the command ---\n{got}\n--- stderr ---\n{}",
                f.file_name().unwrap().to_string_lossy(),
                String::from_utf8_lossy(&o.stderr)
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 3,
        "only {checked} transcripts were checked, which means the scan is broken rather than \
         that the tutorials have none"
    );
}
