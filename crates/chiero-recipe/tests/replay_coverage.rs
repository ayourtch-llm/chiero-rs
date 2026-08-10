//! Covers: 023 contract 21 — per-witness replay at line granularity.
//!
//! "For a single witness replayed with all inputs concretized, the `gcov_lines` of the CIR
//! blocks chiero executed equal the lines gcov reports for the gcc-compiled program on the
//! same inputs."
//!
//! The contract is careful about *which* claim is being made, and 023 says why in the same
//! breath: "stated at block granularity across a whole symbolic run it would be false twice
//! over — a symbolic run covers the union over many paths, and chiero's CFG does not
//! correspond block-to-block with gcc's post-gimplification CFG." So the comparison is one
//! path, one input, and *lines* rather than blocks. That is the strongest form that is
//! actually true, and it is the one that matters: it says chiero walked the program gcc
//! would have walked.
//!
//! **This test lives in `chiero-recipe`** because it is the only place the pieces meet:
//! 001 §4 rule 7 forbids a vertical a frontend dependency, and `chiero-recipe` is one of
//! the two crates on the allowlist. Lowering is a dev-dependency here; gcc and gcov are
//! shelled out to, as `chiero-lower/tests/gcov_lines.rs` does.

use chiero_cir::Module;
use chiero_parse::{ParsedTu, ScopedTypedefs, parse_tu};
use chiero_pp::{Config, preprocess_str};
use chiero_sema::{SymbolText, TargetConfig, analyze};
use chiero_span::Symbol;

struct Names<'a>(&'a ParsedTu);

impl SymbolText for Names<'_> {
    fn text(&self, sym: Symbol) -> Option<&str> {
        self.0.text(sym)
    }
}

fn lower(src: &str) -> (Module, chiero_span::SourceMap) {
    let tu = preprocess_str("t.c", src, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu(&tu, &mut oracle);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let names = Names(&parsed);
    let analysis = analyze(&parsed.ast, &TargetConfig::x86_64_linux(), &names);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let lowered = chiero_lower::lower_tu_with_map(&parsed.ast, &analysis, &names, &tu.source_map);
    assert!(lowered.diagnostics.is_empty(), "{:?}", lowered.diagnostics);
    (lowered.module, tu.source_map)
}

fn gcov_available() -> bool {
    std::process::Command::new("gcov")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    // Per-invocation, not per-pid: two tests running concurrently in one process
    // otherwise compile into each other's directory, and each passes when run alone.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("chiero-replay-{}-{tag}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("tmpdir");
    d
}

/// The lines gcov reports as executed for `src` compiled and run with `arg`.
fn gcov_executed(src: &str, arg: i32) -> Vec<u32> {
    let dir = tmpdir("gcov");
    let main = format!("{src}\nint main(void) {{ f({arg}); return 0; }}\n");
    std::fs::write(dir.join("t.c"), &main).expect("write");

    // Compile and link separately: `gcc -o cov t.c --coverage` names the notes file after
    // the *output*, and gcov then cannot find it from the source name.
    let ok = |c: &mut std::process::Command| {
        let o = c.current_dir(&dir).output().expect("spawn");
        assert!(
            o.status.success(),
            "{:?} failed:\n{}",
            c,
            String::from_utf8_lossy(&o.stderr)
        );
    };
    ok(std::process::Command::new("gcc").args([
        "-std=gnu11",
        "-w",
        "-O0",
        "--coverage",
        "-c",
        "-o",
        "t.o",
        "t.c",
    ]));
    ok(std::process::Command::new("gcc").args(["--coverage", "-o", "cov", "t.o"]));
    ok(&mut std::process::Command::new(dir.join("cov")));

    let g = std::process::Command::new("gcov")
        .args(["--json-format", "--stdout", "t.c"])
        .current_dir(&dir)
        .output()
        .expect("gcov");
    // **A gcov failure panics rather than returning nothing.** An oracle that can quietly
    // not run is not an oracle — this project has had to fix that vacuity six times.
    assert!(
        g.status.success(),
        "gcov failed:\n{}",
        String::from_utf8_lossy(&g.stderr)
    );
    let json = String::from_utf8_lossy(&g.stdout).to_string();
    let mut out = parse_executed_lines(&json);
    out.sort_unstable();
    out.dedup();
    out
}

/// Pull `"line_number": N` entries whose `"count"` is non-zero out of gcov's JSON.
///
/// Hand-parsed rather than pulled in: the shape is two fields on one object and a JSON
/// dependency for it would be a workspace dependency `xtask check-deps` gates.
fn parse_executed_lines(json: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for chunk in json.split("\"line_number\":").skip(1) {
        let num: String = chunk
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let Ok(line) = num.parse::<u32>() else {
            continue;
        };
        // The count for this line is the next `"count":` before the following
        // `"line_number":`, which is the same object.
        let Some(cpos) = chunk.find("\"count\":") else {
            continue;
        };
        let count: String = chunk[cpos + 8..]
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if count.parse::<u64>().unwrap_or(0) > 0 {
            out.push(line);
        }
    }
    out
}

/// The lines chiero executed on one replayed path.
fn chiero_executed(m: &Module, sm: &chiero_span::SourceMap, arg: i64) -> Vec<u32> {
    let _ = sm;
    let mut a = chiero_solver::TermArena::new();
    let r = chiero_exec::Engine::new(m)
        .replaying(chiero_exec::Witness::concrete(vec![(32, arg as u128)]))
        .run(&mut a);
    assert_eq!(
        r.states().len(),
        1,
        "a replay with every input concretized takes one path, or it is not a replay: {:#?}",
        r.states().len()
    );
    let s = &r.states()[0];
    let mut lines: Vec<u32> = s
        .trace()
        .iter()
        .filter_map(|(fid, bid)| {
            let f = m.funcs.iter().find(|f| f.id == *fid)?;
            let b = f.blocks.iter().find(|b| b.id == *bid)?;
            Some(b.gcov_lines.iter().copied())
        })
        .flatten()
        .collect();
    lines.sort_unstable();
    lines.dedup();
    lines
}

/// `f` takes one branch or the other depending on its argument — so the two replays must
/// disagree, and each must agree with gcc.
const SRC: &str = "\
int f(int n) {
  int t = 0;
  if (n > 10) {
    t = n * 2;
  } else {
    t = n + 1;
  }
  return t;
}
";

/// **Contract 21.** For one witness with all inputs concretized, the lines chiero executed
/// are the lines gcov reports.
///
/// Run for *both* branches. One alone is passed by an implementation that reports every
/// line of the function — the two runs have to differ from each other in the same way
/// gcc's do, or the agreement is with a constant.
/// ✅ **Un-`#[ignore]`d 2026-08-10, and the reason it carried was doubly stale.** It read
/// *"blocked: a scalar parameter loaded back from its own slot reads as uninitialized, so both
/// replay paths degrade to Unknown and the branch forks … see the wave-108 note in HANDOFF §9"*
/// — the blocker is gone (3/3 green, ~0.12 s), and **there is no wave-108 note in HANDOFF or
/// the archive**, so the citation pointed at nothing.
///
/// 📌 **It mattered more than one test.** This file's header cites **023 contract 21**, and
/// `xtask contract-coverage` counts a citation whether or not the citing test runs — so M1's
/// "166/166 contracts cited by a test" included one that had not executed since the ignore was
/// added. 070 §4's own words: *a gate nobody runs is a gate that is already failing*.
#[test]
fn a_replayed_witness_covers_the_lines_gcov_reports() {
    if !gcov_available() {
        eprintln!("skipping: no gcov");
        return;
    }
    let (m, sm) = lower(SRC);

    for arg in [42, 3] {
        let ours = chiero_executed(&m, &sm, arg as i64);
        let theirs = gcov_executed(SRC, arg);
        assert!(
            !ours.is_empty(),
            "chiero executed no lines for n={arg}, which would make any comparison vacuous"
        );
        // gcov's set includes `main`, which chiero did not run, so the claim is that
        // chiero's lines are exactly gcov's lines *within `f`* — the function replayed.
        let f_lines: Vec<u32> = theirs.iter().copied().filter(|l| *l <= 9).collect();
        assert_eq!(
            ours, f_lines,
            "n={arg}: chiero walked {ours:?}, gcc walked {f_lines:?}"
        );
    }
}

/// **The two branches differ**, which is what makes the agreement above mean something.
#[test]
fn the_two_replays_walk_different_lines() {
    let (m, sm) = lower(SRC);
    let hot = chiero_executed(&m, &sm, 42);
    let cold = chiero_executed(&m, &sm, 3);
    assert_ne!(
        hot, cold,
        "n=42 takes the `then` arm and n=3 the `else`; a replay reporting every line of \
         the function gives the same list twice: {hot:?}"
    );
    assert!(
        hot.contains(&4) && !hot.contains(&6),
        "n=42 runs line 4 and not line 6: {hot:?}"
    );
    assert!(
        cold.contains(&6) && !cold.contains(&4),
        "n=3 runs line 6 and not line 4: {cold:?}"
    );
}
