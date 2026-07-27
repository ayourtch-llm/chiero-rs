//! 023 contract 13a: **`ExactWitness` cannot be constructed outside `chiero-exec`, and a
//! `RunResult` cannot be forged.**
//!
//! §7.1's whole claim is that the type system prevents a *downstream crate* from
//! presenting a degraded run as a proof. That is a statement about what does **not**
//! compile, so no runtime test can check it — the review demonstrated the forgery from a
//! separate crate precisely because nothing was watching this boundary.
//!
//! `trybuild` is the usual tool; this gate does the same job with `rustc` directly, which
//! keeps the check offline and puts it beside the two architecture gates that already
//! exist rather than in a test harness of its own.

use std::path::Path;
use std::process::Command;

/// Each probe must FAIL to compile. A probe that starts compiling is the failure.
const PROBES: &[(&str, &str)] = &[
    (
        "construct a witness directly",
        r#"fn main() { let _ = chiero_exec::ExactWitness { run: 0 }; }"#,
    ),
    (
        "build a RunResult literal",
        r#"fn main() {
            let _ = chiero_exec::RunResult {
                id: 0, states: vec![], solver_calls: 0,
                backend_spawns: 0, solver_inits: 0,
            };
        }"#,
    ),
    (
        "overwrite a state's fidelity through the accessor",
        r#"fn main() {
            let m = chiero_cir::Module::default();
            let mut a = chiero_solver::TermArena::new();
            let mut r = chiero_exec::Engine::new(&m).run(&mut a);
            // The accessor hands out `&[State]`, so there is no path to a `&mut State`
            // even if the field itself were public.
            r.states()[0].fidelity = chiero_exec::Fidelity::Exact;
            let _ = &mut r;
        }"#,
    ),
    (
        "build a Proven literal",
        r#"fn main() {
            let m = chiero_cir::Module::default();
            let mut a = chiero_solver::TermArena::new();
            let r = chiero_exec::Engine::new(&m).run(&mut a);
            // `seal` must be the only route to a `Proven`; a struct literal was a second.
            let _ = chiero_exec::Proven { result: &r };
        }"#,
    ),
    (
        "clone a witness to reuse it",
        r#"fn main() {
            let m = chiero_cir::Module::default();
            let mut a = chiero_solver::TermArena::new();
            let r = chiero_exec::Engine::new(&m).run(&mut a);
            let w = r.witness();
            let _w2 = w.clone();
            let _ = chiero_exec::seal(&r, w);
        }"#,
    ),
];

/// The most recently built rlib for a crate. Passing a bare `--extern name` lets rustc
/// see several candidates and fail before it ever reaches the code being probed.
fn newest_rlib(deps: &Path, krate: &str) -> Option<std::path::PathBuf> {
    let prefix = format!("lib{krate}-");
    let mut best: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for e in std::fs::read_dir(deps).ok()?.flatten() {
        let p = e.path();
        let name = p.file_name()?.to_string_lossy().to_string();
        if name.starts_with(&prefix) && name.ends_with(".rlib") {
            let t = e.metadata().ok()?.modified().ok()?;
            if best.as_ref().is_none_or(|(bt, _)| t > *bt) {
                best = Some((t, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

pub fn check_proof_surface() -> i32 {
    // The **working directory**, not `CARGO_MANIFEST_DIR`. The latter is baked in when
    // the xtask binary is compiled, so a copy of the tree elsewhere would silently probe
    // the original — which is exactly how I first "verified" this gate and got a pass it
    // had not earned.
    let root = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let deps = root.join("target/debug/deps");
    if !deps.exists() {
        eprintln!("check-proof-surface: build the workspace first (`cargo build`)");
        return 2;
    }
    let tmp = std::env::temp_dir().join("chiero-proof-surface");
    let _ = std::fs::create_dir_all(&tmp);

    let mut bad = Vec::new();
    for (what, src) in PROBES {
        let file = tmp.join("probe.rs");
        if std::fs::write(&file, src).is_err() {
            eprintln!("check-proof-surface: cannot write the probe");
            return 2;
        }
        let mut cmd = Command::new("rustc");
        cmd.args(["--edition", "2024", "--crate-type", "bin", "-o"])
            .arg(tmp.join("probe"))
            .arg(&file)
            .arg("-L")
            .arg(&deps);
        for c in [
            "chiero_exec",
            "chiero_cir",
            "chiero_solver",
            "chiero_span",
            "chiero_mem",
        ] {
            match newest_rlib(&deps, c) {
                Some(p) => {
                    cmd.arg("--extern").arg(format!("{c}={}", p.display()));
                }
                None => {
                    eprintln!("check-proof-surface: no rlib for {c}; build the workspace");
                    return 2;
                }
            }
        }
        match cmd.output() {
            Ok(o) if o.status.success() => bad.push(*what),
            Ok(o) => {
                // **A probe must fail for the right reason.** Ambiguous rlib candidates
                // made every probe fail regardless of the seal, so the gate passed
                // without checking anything — which is how it first "verified" itself.
                let err = String::from_utf8_lossy(&o.stderr);
                if !err.contains("private")
                    && !err.contains("E0616")
                    && !err.contains("E0451")
                    && !err.contains("E0599")
                    && !err.contains("cannot be formatted")
                    && !err.contains("no method named")
                {
                    eprintln!(
                        "check-proof-surface: `{what}` failed for an unrelated reason, so \
                         this gate is not checking anything:\n{}",
                        err.lines().take(6).collect::<Vec<_>>().join("\n")
                    );
                    return 2;
                }
            }
            Err(e) => {
                eprintln!("check-proof-surface: could not run rustc: {e}");
                return 2;
            }
        }
    }

    if bad.is_empty() {
        println!(
            "check-proof-surface: {} forgery attempt(s) rejected by the type system",
            PROBES.len()
        );
        0
    } else {
        eprintln!("check-proof-surface: these compiled and must not:");
        for b in &bad {
            eprintln!("  - {b}");
        }
        1
    }
}
