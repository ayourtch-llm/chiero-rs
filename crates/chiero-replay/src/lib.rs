//! **`chiero-replay` — [040 §3](../../../docs/specs/040-defect-checkers.md)'s harness.**
//!
//! > For each finding, `chiero-replay` emits a self-contained C program.
//!
//! and, for [041 §1.3](../../../docs/specs/041-optimization-analysis.md), the same mechanism
//! pointed at a pair of versions:
//!
//! > "the output is a **distinguishing input plus a replay harness** that compiles both
//! > versions and demonstrates the divergence."
//!
//! # Why this is the only part of a `Differs` that is not chiero marking its own homework
//!
//! Everything upstream rests on chiero's C semantics being right. The witness comes from a
//! solver reasoning about chiero's model of the program; the verdict comes from comparing two
//! such models. **A harness is the one thing in the system that asks a real compiler**, which
//! is why 041 contract 11 makes a harness that fails to demonstrate a *downgrade*:
//!
//! > "the harness is compiled and run, and a divergence the harness fails to demonstrate is
//! > downgraded and flagged, never silently trusted."
//!
//! So [`Outcome`] has four values and three of them are ways of having demonstrated nothing.
//! Collapsing "it built and disagreed with chiero" into "it built" is the failure this crate
//! exists to prevent.
//!
//! # `#include`, not `extern` — 040 §3.1
//!
//! > "An `extern` declaration plus separate compilation only works for externally-linked
//! > functions, and chiero's analysis targets in VPP are overwhelmingly **not** that."
//!
//! The harness includes the source and is compiled as one translation unit, so a `static`
//! helper is reachable and a `static inline` header function is instantiable, with layout and
//! macro configuration identical by construction. `-Dstatic=` is explicitly rejected by §3.1:
//! it changes linkage across the whole TU and would make the harness a different program from
//! the one analysed.

use chiero_exec::Witness;
use std::path::{Path, PathBuf};

/// An emitted harness: the program, and what it is trying to show.
///
/// **No verdict field.** 050 contract 11 gates *execution* behind `--allow-replay-exec`, so a
/// response may legitimately carry the program and no verdict at all — and an `Option` here
/// would conflate "nobody ran it" with "it ran and said nothing", which is the one distinction
/// this crate is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replay {
    /// The complete C program.
    pub source: String,
    /// What the harness claims, for the comment at its head and for a reader who is handed the
    /// text without running it.
    pub claim: String,
}

/// What running a harness established.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The two versions returned different values at the witness. **The only outcome that
    /// confirms anything.**
    Demonstrated { before: i64, after: i64 },
    /// It built, it ran, and the two versions **agreed**. 041 contract 11's case: chiero says
    /// they differ and a real compiler says otherwise, so the finding is downgraded and this
    /// is the reason.
    NotDemonstrated { before: i64, after: i64 },
    /// The harness would not build — a fact about the harness rather than about chiero. Most
    /// often two versions sharing a definition, since including both puts it in one
    /// translation unit twice.
    DidNotBuild { detail: String },
    /// Something else went wrong: the compiler could not be run, the binary faulted, the
    /// output was unreadable. Distinct because "the program crashed" and "the versions agreed"
    /// are not the same news.
    DidNotRun { detail: String },
}

impl Outcome {
    /// Whether this outcome confirms the divergence. Nothing else may be read as confirmation.
    pub fn confirms(&self) -> bool {
        matches!(self, Outcome::Demonstrated { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Outcome::Demonstrated { .. } => "demonstrated",
            Outcome::NotDemonstrated { .. } => "not_demonstrated",
            Outcome::DidNotBuild { .. } => "did_not_build",
            Outcome::DidNotRun { .. } => "did_not_run",
        }
    }
}

/// A C compiler, if one is on `PATH`.
///
/// Discovery at *run* time, like the solver's (022 §4): 010 §1's build rule keeps chiero from
/// depending on a toolchain, and a harness is worth emitting whether or not anything here can
/// build it. `$CC` first, so a specific version can be pointed at.
pub fn compiler() -> Option<PathBuf> {
    let candidates: Vec<String> = match std::env::var("CC") {
        Ok(v) if !v.is_empty() => vec![v],
        _ => ["cc", "gcc", "clang"]
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
    };
    for c in candidates {
        let p = PathBuf::from(&c);
        if p.is_absolute() {
            if p.exists() {
                return Some(p);
            }
            continue;
        }
        if let Some(paths) = std::env::var_os("PATH")
            && let Some(found) = std::env::split_paths(&paths)
                .map(|d| d.join(&c))
                .find(|f| f.is_file())
        {
            return Some(found);
        }
    }
    None
}

/// A witness value as a C literal at its own width.
///
/// **Signed, at the declared width, and `INT_MIN` spelled the way `<limits.h>` spells it.** A
/// 32-bit input printed as an unsigned 64-bit number replays as a different value the moment
/// the parameter is `int`; and `-2147483648` written literally is C's negation of a value that
/// does not fit in `int`, which is why the standard header says `(-2147483647 - 1)`. A harness
/// that got this wrong would fail to reproduce exactly the divergence 041 §1.3 uses as its
/// example.
fn literal(width: u32, value: u128) -> String {
    let shift = 128 - width.max(1);
    let signed = ((value << shift) as i128) >> shift;
    match width {
        32 if signed == i128::from(i32::MIN) => "(-2147483647 - 1)".to_string(),
        64 if signed == i128::from(i64::MIN) => "(-9223372036854775807LL - 1)".to_string(),
        64 => format!("{signed}LL"),
        _ => format!("{signed}"),
    }
}

/// The path an `#include` can use from anywhere.
///
/// **The harness is built in a scratch directory** — 050 contract 12 keeps it out of the
/// analysed tree — and `#include "before.c"` resolves relative to the *harness*, not to
/// wherever the caller was standing. A relative path therefore produced
/// `fatal error: before.c: No such file or directory`: a `did_not_build` that says nothing
/// about the code and everything about the emitter. Found by running the CLI end to end.
///
/// Falls back to the path as given when it cannot be resolved, because a harness naming the
/// wrong file is easier to diagnose than one naming nothing.
fn absolute(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|d| d.join(p))
                .unwrap_or_else(|_| p.to_path_buf())
        }
    })
}

/// Emit a harness that runs both versions of `entry` at `witness` — 041 §1.3.
///
/// The two sources are included into one translation unit with the entry renamed, which is
/// 040 §3.1's mechanism and the only one that reaches a `static` target. The rename is a
/// `#define` around each include rather than a compiler flag, so it applies to exactly one
/// name in exactly one file.
pub fn emit_equivalence(before: &Path, after: &Path, entry: &str, witness: &Witness) -> Replay {
    let before = &absolute(before);
    let after = &absolute(after);
    let args: Vec<String> = witness
        .bindings
        .iter()
        .map(|b| literal(b.width, b.value))
        .collect();
    let call = args.join(", ");
    let described: Vec<String> = witness
        .bindings
        .iter()
        .map(|b| format!("{} = {}", b.origin.label(), literal(b.width, b.value)))
        .collect();
    let claim = format!(
        "`{entry}` differs between {} and {} at {}",
        before.display(),
        after.display(),
        if described.is_empty() {
            "no input".to_string()
        } else {
            described.join(", ")
        }
    );

    // **`main` is renamed out of the way around each include** — 040 §3.1's first hazard. A
    // translation unit with its own `main` would collide with the harness's, and 51 VPP files
    // have one.
    let mut source = String::new();
    source.push_str(&format!(
        "/* chiero replay: {claim}\n   \
         Exits 0 when the two versions disagree, which is what chiero claimed.\n   \
         Exits 1 when they AGREE: chiero and this compiler do not, and the finding is\n   \
         downgraded rather than trusted (041 contract 11). */\n#include <stdio.h>\n\n"
    ));
    for (tag, path) in [("before", before), ("after", after)] {
        source.push_str(&format!(
            "#define {entry} chiero_{tag}_{entry}\n\
             #define main chiero_{tag}_main\n\
             #include \"{}\"\n\
             #undef main\n\
             #undef {entry}\n\n",
            path.display()
        ));
    }
    source.push_str(&format!(
        "int main (void)\n{{\n  \
         long long b = (long long) chiero_before_{entry} ({call});\n  \
         long long a = (long long) chiero_after_{entry} ({call});\n  \
         printf (\"before=%lld after=%lld\\n\", b, a);\n  \
         return b == a;\n}}\n"
    ));
    Replay { source, claim }
}

/// Compile and run a harness, and report which of the four things happened.
///
/// **Never `Demonstrated` by default.** Every path that does not produce two different numbers
/// from a program that built and ran returns one of the other three, because the value of this
/// crate is precisely that it can say chiero was wrong.
pub fn run(r: &Replay, cc: &Path, dir: &Path) -> Outcome {
    if std::fs::create_dir_all(dir).is_err() {
        return Outcome::DidNotRun {
            detail: format!("cannot create {}", dir.display()),
        };
    }
    // **Named after the harness, not after the crate.** A fixed filename meant two harnesses
    // built in one directory — two threads of one test binary, or two findings of one run —
    // overwrote each other's source and binary, and the loser reported `DidNotBuild` about a
    // program that compiles. Found by four tests failing in parallel and passing by hand.
    //
    // FNV-1a over the source: deterministic (001 §5), so re-running a harness reuses its own
    // files rather than accumulating them, and distinct for distinct programs.
    let tag = fnv1a(&r.source);
    let src = dir.join(format!("chiero_replay_{tag:032x}.c"));
    let bin = dir.join(format!("chiero_replay_{tag:032x}.bin"));
    if let Err(e) = std::fs::write(&src, &r.source) {
        return Outcome::DidNotRun {
            detail: format!("cannot write the harness: {e}"),
        };
    }

    match std::process::Command::new(cc)
        .args(["-std=gnu11", "-w", "-O0", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
    {
        Err(e) => {
            return Outcome::DidNotRun {
                detail: format!("{} could not be run: {e}", cc.display()),
            };
        }
        Ok(o) if !o.status.success() => {
            return Outcome::DidNotBuild {
                detail: String::from_utf8_lossy(&o.stderr).trim().to_string(),
            };
        }
        Ok(_) => {}
    }

    let out = match std::process::Command::new(&bin).output() {
        Ok(o) => o,
        Err(e) => {
            return Outcome::DidNotRun {
                detail: format!("the harness would not start: {e}"),
            };
        }
    };
    // **The two numbers, not the exit code.** The status says agree-or-not; a reader weighing a
    // downgrade against the analysis needs the values that were actually produced — and a
    // harness killed by a signal has a status that means neither.
    let text = String::from_utf8_lossy(&out.stdout);
    let Some((b, a)) = parse_line(&text) else {
        return Outcome::DidNotRun {
            detail: format!(
                "the harness produced no comparable output (status {:?}): {}",
                out.status.code(),
                text.trim()
            ),
        };
    };
    if b == a {
        Outcome::NotDemonstrated {
            before: b,
            after: a,
        }
    } else {
        Outcome::Demonstrated {
            before: b,
            after: a,
        }
    }
}

fn parse_line(text: &str) -> Option<(i64, i64)> {
    let line = text.lines().find(|l| l.starts_with("before="))?;
    let mut it = line.split_whitespace();
    let b = it.next()?.strip_prefix("before=")?.parse().ok()?;
    let a = it.next()?.strip_prefix("after=")?.parse().ok()?;
    Some((b, a))
}

/// FNV-1a over 128 bits, as `chiero-gcov::source_hash` and `Envelope::determinism_key` use it
/// and for the same reason: the job is to notice a difference between two texts, and nothing
/// here faces an adversary.
fn fnv1a(text: &str) -> u128 {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000000001000000000000000000013b;
    let mut h = OFFSET;
    for b in text.bytes() {
        h ^= u128::from(b);
        h = h.wrapping_mul(PRIME);
    }
    h
}
