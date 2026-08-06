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

use chiero_exec::{InputOrigin, Witness};
use std::path::{Path, PathBuf};

/// An emitted harness: the program, and what it is trying to show.
///
/// **No verdict field.** 050 contract 11 gates *execution* behind `--allow-replay-exec`, so a
/// response may legitimately carry the program and no verdict at all — and an `Option` here
/// would conflate "nobody ran it" with "it ran and said nothing", which is the one distinction
/// this crate is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replay {
    /// The harness's own translation unit — what a reader is shown, and what calls the two
    /// versions through the wrappers in [`Replay::units`].
    pub source: String,
    /// The two versions, each as its own translation unit: `(filename, text)`.
    ///
    /// **Separate units, and that is the whole design.** Including both versions into one TU
    /// put every `static` helper they share in it twice, so any pair of real files failed to
    /// build. Separate compilation keeps a `static` file-local — and the wrapper appended to
    /// each unit is what keeps a `static` *entry* reachable, which is 040 §3.1's requirement
    /// and the reason one TU was chosen first.
    pub units: Vec<(String, String)>,
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
    /// The caller allowed execution and no C compiler was found.
    ///
    /// **Not the same as "the caller did not allow it."** Both used to be an absent outcome, so
    /// a consumer could not tell a deliberate `--replay` from a machine with no toolchain — and
    /// the accompanying note cited 050 contract 11's gate, which is the wrong reason for the
    /// second. Found by review.
    NoCompiler,
    /// The caller asked for the program and not for a verdict — 050 contract 11's default.
    NotRun,
    /// **The program does not give the same answer twice**, so comparing two of its runs
    /// establishes nothing.
    ///
    /// Found by running the first version twice before comparing anything. A function that
    /// reads the clock, or `rand()` without a fixed seed, or anything else the process does not
    /// determine, produces two different numbers from one program — and every earlier version
    /// of this reported that as `Demonstrated`. Isolation cannot fix it, because the
    /// nondeterminism is the program's rather than the harness's; the only honest move is to
    /// notice and refuse.
    Nondeterministic { first: i64, second: i64 },
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
            Outcome::NoCompiler => "no_compiler",
            Outcome::Nondeterministic { .. } => "nondeterministic",
            Outcome::NotRun => "not_run",
        }
    }
}

/// **What running a harness is actually bounded by, on this machine** — 050 §6.
///
/// > "a sandbox with no network, a scratch working directory, a wall-clock limit, and a memory
/// > cap"
///
/// Reported rather than assumed, because not all of it is enforceable everywhere and **a limit
/// claimed and not enforced is worse than one honestly absent** — the first gets acted on. A
/// test asserts this report against what a fixture harness actually manages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sandbox {
    /// The harness runs in a network namespace of its own, so it has no route anywhere.
    pub network: bool,
    /// An address-space cap, applied via the shell's `ulimit`.
    pub memory_bytes: Option<u64>,
    /// **Not enforced today, and said so rather than hoped.** Confining writes without root
    /// needs more than an unprivileged user namespace: remounting the filesystem read-only
    /// inside one fails on the underlying device, and building a pivoted root is more
    /// machinery than this warrants. The working directory *is* the scratch directory, which
    /// bounds a well-behaved program and nothing else.
    pub writes_confined: bool,
    /// The wall-clock limit, which is enforced by the runner rather than by the shell.
    pub timeout: std::time::Duration,
}

impl Sandbox {
    /// How to launch, given what is available — **the single place that decides, so the report
    /// and the run cannot disagree.**
    ///
    /// `sandbox()` used to derive `network` from `unshare` alone while the runner applied
    /// isolation only when a shell was present too. On a machine with no `sh` the report said
    /// "network is isolated" and a harness opened a TCP connection. That report is attached to
    /// every confirmation as an assumption, so it was a false statement in the one place a
    /// reader looks to decide what a verdict rests on.
    fn plan(&self) -> Option<(PathBuf, Vec<String>)> {
        let sh = shell()?;
        let sh_s = sh.to_string_lossy().into_owned();
        match (self.network, which("unshare")) {
            (true, Some(u)) => Some((u, vec!["-rn".into(), "--".into(), sh_s, "-c".into()])),
            _ => Some((sh, vec!["-c".into()])),
        }
    }

    /// One line per limit, in the words a reader needs to decide what the verdict rests on.
    pub fn describe(&self) -> String {
        format!(
            "replay sandbox: network is {}; memory is {}; writes are {}; wall clock {}s",
            if self.network {
                "isolated (a namespace of its own)"
            } else {
                "NOT isolated — this machine cannot create a network namespace unprivileged"
            },
            match self.memory_bytes {
                Some(b) => format!("capped at {} MiB", b / (1024 * 1024)),
                None => "NOT capped".to_string(),
            },
            // **Read from the field rather than hardcoded.** The one thing this method exists
            // to report was the one it did not consult, so if the field ever became true the
            // description would have lied.
            if self.writes_confined {
                "confined to the scratch directory"
            } else {
                "NOT confined to the scratch directory (050 §6 wants them to be)"
            },
            self.timeout.as_secs()
        )
    }
}

/// What this machine can enforce, discovered once.
///
/// Discovery at run time, like the compiler's and the solver's: whether an unprivileged user
/// namespace may be created is a property of the kernel's configuration, not of the build.
pub fn sandbox() -> Sandbox {
    // **Probed once.** The doc said "discovered once" and it spawned `unshare -rn -- true` on
    // every call — several times per replay, and more once the compile got its own clock. The
    // answer is a property of the kernel's configuration, which does not change under us.
    static PROBED: std::sync::OnceLock<Sandbox> = std::sync::OnceLock::new();
    PROBED
        .get_or_init(|| {
            // **Both limits need a shell**, because both are applied through one. Reporting
            // the network isolated without one was the defect this rules out by construction.
            let have_shell = shell().is_some();
            Sandbox {
                network: have_shell && unshare_works(),
                memory_bytes: have_shell.then_some(2 * 1024 * 1024 * 1024),
                writes_confined: false,
                timeout: std::time::Duration::from_secs(10),
            }
        })
        .clone()
}

/// Whether `unshare -rn` can be used here — an unprivileged user namespace plus a network one.
///
/// Probed by running it, because the answer depends on `kernel.unprivileged_userns_clone`, on
/// AppArmor, and on whether the binary exists. Asking is cheaper than being wrong.
fn unshare_works() -> bool {
    which("unshare").is_some_and(|u| {
        std::process::Command::new(u)
            .args(["-rn", "--", "true"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

fn shell() -> Option<PathBuf> {
    which("sh")
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join(name))
            .find(|f| f.is_file())
    })
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

/// Why no harness was emitted.
///
/// **A refusal is about the *witness*, not about the harness.** Before this existed, a witness
/// the emitter could not render produced a program that would not compile, reported as
/// `DidNotBuild` — which reads as "the harness is broken" when the truth is "this is not
/// something a harness of this shape can check". The two need different words because they
/// send a reader to different places.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub why: String,
}

/// What this harness shape can actually measure, checked before anything is emitted.
///
/// **The narrowness is the honest part.** A harness that compares two return values at one
/// input can adjudicate exactly that; asked about anything else it will report the two versions
/// agreeing, and 041 contract 11 would turn that into a downgrade of a true finding. So the
/// conditions are checked here, once, rather than discovered as a compiler error later.
fn renderable(witness: &Witness) -> Result<Vec<String>, Refusal> {
    let mut args: Vec<(usize, String)> = Vec::new();
    for b in &witness.bindings {
        match &b.origin {
            InputOrigin::Param { index, .. } => {
                // 040 §3's construction rules want memory objects materialized and extern
                // stubs generated; until they are, a witness with anything but parameters is
                // not an argument list.
                if b.width > 64 {
                    return Err(Refusal {
                        why: format!(
                            "a {}-bit input has no C literal this emitter can write — above 64 bits gcc \
                             truncates a decimal constant silently",
                            b.width
                        ),
                    });
                }
                args.push((*index, literal(b.width, b.value)));
            }
            other => {
                return Err(Refusal {
                    why: format!(
                        "the witness binds {}, which is not a parameter — 040 §3 wants unmodeled extern calls stubbed to return the engine's values and memory objects materialized, and neither is built",
                        other.label()
                    ),
                });
            }
        }
    }
    // The indices must be 0..n and each present once, or "positional" means nothing.
    args.sort_by_key(|(i, _)| *i);
    if args.iter().enumerate().any(|(n, (i, _))| n != *i) {
        return Err(Refusal {
            why: "the witness's parameter indices are not 0..n, so they cannot be passed positionally"
                .to_string(),
        });
    }
    Ok(args.into_iter().map(|(_, a)| a).collect())
}

/// Emit a harness that runs both versions of `entry` at `witness` — 041 §1.3.
///
/// The two sources are included into one translation unit with the entry renamed, which is
/// 040 §3.1's mechanism and the only one that reaches a `static` target. The rename is a
/// `#define` around each include rather than a compiler flag, so it applies to exactly one
/// name in exactly one file.
///
/// Refuses rather than emitting something that cannot answer — see [`renderable`].
pub fn emit_equivalence(
    before: &Path,
    after: &Path,
    entry: &str,
    witness: &Witness,
) -> Result<Replay, Refusal> {
    let args = renderable(witness)?;
    let before = &absolute(before);
    let after = &absolute(after);
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
    //
    // Each version becomes its own unit with a non-static wrapper appended. The wrapper is in
    // the same TU as the entry, so a `static` entry is reachable; the unit is separate, so a
    // `static` helper the two versions share does not collide.
    let mut units = Vec::new();
    for (tag, path) in [("before", before), ("after", after)] {
        units.push((
            format!("chiero_{tag}.c"),
            format!(
                "/* chiero replay unit: the {tag} version of `{entry}`, as its own program */\n\
                 #include <stdio.h>\n\
                 #include <unistd.h>\n\
                 #define {entry} chiero_{tag}_{entry}\n\
                 #define main chiero_{tag}_main\n\
                 #include \"{}\"\n\
                 #undef main\n\
                 #undef {entry}\n\n\
                 int main (void)\n{{\n  \
                 long long v = (long long) chiero_{tag}_{entry} ({});\n  \
                 FILE *o = fopen (CHIERO_RESULT, \"w\");\n  \
                 if (!o) return 2;\n  \
                 fprintf (o, \"value=%lld\\n\", v);\n  \
                 fclose (o);\n  \
                 /* `_exit`, not `return`: an atexit handler the analysed code registered\n     \
                    must not get a chance to rewrite what we just wrote. */\n  \
                 _exit (0);\n}}\n",
                path.display(),
                call
            ),
        ));
    }

    // **One program per version, run in its own process.**
    //
    // Calling both from one `main` shared everything outside a translation unit — libc's PRNG,
    // the clock, both constructors — so two identical programs reported different numbers and
    // `Demonstrated` could be fabricated four ways. Four earlier rounds closed such doors one
    // at a time; separate processes closes the class. Each program computes one number and
    // says nothing about the other.
    //
    // **`_exit` after writing**, so an `atexit` handler registered by the analysed code cannot
    // rewrite the result: `_exit` does not run them. That is the other half of "the verdict
    // travels on a channel the included code cannot write" — the first half, moving it off
    // stdout, was necessary and not sufficient.
    let source = format!(
        "/* chiero replay: {claim}\n   \
         Each version is built and run as its OWN program, writing one number to the path it\n   \
         is compiled with. Sharing a process let libc state make two identical programs look\n   \
         different, which is a fabricated confirmation and the worst thing this can do. */\n"
    );
    Ok(Replay {
        source,
        units,
        claim,
    })
}

/// Compile and run a harness, and report which of the four things happened.
///
/// **Never `Demonstrated` by default.** Every path that does not produce two different numbers
/// from a program that built and ran returns one of the other three, because the value of this
/// crate is precisely that it can say chiero was wrong.
pub fn run(r: &Replay, cc: &Path, dir: &Path) -> Outcome {
    run_with(r, cc, dir, &[])
}

/// [`run`] with the translation unit's own flags — 040 §3's last construction rule.
///
/// > "The harness compiles using the `compile_commands.json` flags for that TU so layout, `-D`
/// > flags and `march` variant match. A harness compiled with different flags can reproduce a
/// > different program."
///
/// Without them any source needing an `-I` or a `-D` is a `DidNotBuild`, which says nothing
/// about the code. The flags are the caller's, because the caller is the one that knows how the
/// file is really built.
pub fn run_with(r: &Replay, cc: &Path, dir: &Path, flags: &[String]) -> Outcome {
    if let Some(objection) = launchable(dir, flags) {
        return Outcome::DidNotRun { detail: objection };
    }
    if std::fs::create_dir_all(dir).is_err() {
        return Outcome::DidNotRun {
            detail: format!("cannot create {}", dir.display()),
        };
    }
    // **A name unique to this call, not to the harness text.**
    //
    // A fixed filename meant two harnesses built in one directory overwrote each other and the
    // loser reported `DidNotBuild` about a program that compiles. Naming them after an FNV of
    // the source fixed that and left a subtler one: two callers running the *same* harness
    // concurrently — which is exactly what a test suite does — still collide, and `cc` writing
    // one binary from two processes yields a corrupt file and a `DidNotRun` about nothing.
    //
    // Nothing downstream sees these names, so uniqueness costs nothing. The FNV stays in the
    // name so a leftover file can be traced back to its harness; the counter is what makes it
    // safe.
    let tag = unique_tag(&r.source);
    // **One build and one run per version.** `Replay::source` is documentation — the comment a
    // reader is shown — and the units are the programs. Nothing links them together, which is
    // what keeps one version's process state out of the other's.
    if r.units.len() != 2 {
        return Outcome::DidNotRun {
            detail: format!(
                "a harness needs exactly two programs, one per version; this one has {}",
                r.units.len()
            ),
        };
    }
    let mut values = Vec::new();
    for (n, (name, text)) in r.units.iter().enumerate() {
        let src = dir.join(format!("chiero_unit_{tag}_{n}_{name}"));
        let bin = dir.join(format!("chiero_unit_{tag}_{n}.bin"));
        let result = dir.join(format!("chiero_result_{tag}_{n}.txt"));
        if let Err(e) = std::fs::write(&src, text) {
            return Outcome::DidNotRun {
                detail: format!("cannot write {}: {e}", src.display()),
            };
        }
        let _ = std::fs::remove_file(&result);
        // **The compile is bounded too.** 050 §6 covers "compilation and replay execution",
        // and the limit wrapped only the produced binary — so a source whose `#include` names
        // a FIFO hung the tool forever, which is the consequence the execution limit was added
        // for, through the neighbouring door.
        if let Err(e) = compile(cc, flags, &result, &bin, &src, dir, sandbox().timeout) {
            return e;
        }
        if let Err(e) = bounded(&bin, dir, sandbox().timeout) {
            return Outcome::DidNotRun { detail: e };
        }
        let text = match std::fs::read_to_string(&result) {
            Ok(t) => t,
            Err(e) => {
                return Outcome::DidNotRun {
                    detail: format!("the {} program wrote no result: {e}", label(n)),
                };
            }
        };
        match parse_value(&text) {
            Some(v) => values.push(v),
            None => {
                return Outcome::DidNotRun {
                    detail: format!(
                        "the {} program's result is unreadable: {}",
                        label(n),
                        text.trim()
                    ),
                };
            }
        }
    }
    // **Is the program deterministic at all?**
    //
    // Running the first version a second time costs one execution and rules out the whole class
    // the previous rounds kept meeting: a function reading the clock, or `rand()` without a
    // fixed seed, gives two different numbers from ONE program, and comparing two such numbers
    // establishes nothing. Isolation cannot fix that — the nondeterminism is the program's —
    // so the only honest move is to notice.
    //
    // Checked after both runs rather than before, so a pair that does not build is reported as
    // not building rather than as nondeterministic.
    match rerun_first(r, cc, dir, flags, &tag) {
        Ok(again) if again != values[0] => {
            return Outcome::Nondeterministic {
                first: values[0],
                second: again,
            };
        }
        Ok(_) => {}
        Err(detail) => return Outcome::DidNotRun { detail },
    }

    let (b, a) = (values[0], values[1]);
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

/// Why this call cannot be launched, if it cannot — one answer for every harness shape.
///
/// **The scratch directory must be absolute and quote-free.** Its paths are interpolated into
/// `sh -c "... exec '{}'"`, so a relative one becomes a bare word searched on a PATH that does
/// not contain it and a quote ends the string early — both of which surfaced as "the harness
/// wrote no result", pointing a reader at the harness rather than at the argument they passed.
///
/// **And no flag may merge the two programs.** 040 §3 requires the translation unit's own
/// flags, and `-fcommon` makes tentative definitions merge across units, so two identical
/// sources sharing `int g;` report as differing. Passing the TU's flags is right; passing one
/// that defeats the separation is not.
fn launchable(dir: &Path, flags: &[String]) -> Option<String> {
    if !dir.is_absolute() {
        return Some(format!(
            "the scratch directory must be absolute; `{}` is relative and the launcher cannot \
             use it",
            dir.display()
        ));
    }
    if dir.to_string_lossy().contains('\'') {
        return Some(format!(
            "the scratch directory must not contain a quote; `{}` would end the launcher's \
             command early",
            dir.display()
        ));
    }
    for bad in ["-fcommon", "--fcommon"] {
        if flags.iter().any(|f| f == bad) {
            return Some(format!(
                "`{bad}` merges tentative definitions across the two programs, which would let \
                 one version's globals reach the other — the separation this harness depends on"
            ));
        }
    }
    None
}

/// A filename component unique to this call.
///
/// A fixed name meant two harnesses in one directory overwrote each other and the loser
/// reported `DidNotBuild` about a program that compiles; naming them after the source's hash
/// left the subtler case of two callers running the *same* harness at once. Nothing downstream
/// sees these names, so uniqueness costs nothing — the hash stays so a leftover file can be
/// traced back, and the counter is what makes it safe.
fn unique_tag(text: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{:032x}_{}_{}",
        fnv1a(text),
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

/// Kill anything still running with `path` on its command line.
///
/// **By command line rather than by process group.** Putting a run in its own session needs the
/// group id, which `setsid` does not report back; a path unique to this call needs nothing this
/// crate does not already have. `fork` does not change `argv` and a compiler driver passes the
/// source name to `cc1`, so this finds exactly this call's descendants.
fn kill_descendants(path: &Path) {
    let _ = std::process::Command::new("pkill")
        .args(["-KILL", "-f"])
        .arg(path.display().to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Compile one unit, under the same wall clock the run gets.
fn compile(
    cc: &Path,
    flags: &[String],
    result: &Path,
    bin: &Path,
    src: &Path,
    dir: &Path,
    limit: std::time::Duration,
) -> Result<(), Outcome> {
    let mut child = std::process::Command::new(cc)
        .args(["-std=gnu11", "-w", "-O0"])
        .args(flags)
        .arg(format!("-DCHIERO_RESULT=\"{}\"", result.display()))
        .arg("-o")
        .arg(bin)
        .arg(src)
        .current_dir(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| Outcome::DidNotRun {
            detail: format!("{} could not be run: {e}", cc.display()),
        })?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = child
                    .wait_with_output()
                    .unwrap_or_else(|_| std::process::Output {
                        status,
                        stdout: Vec::new(),
                        stderr: Vec::new(),
                    });
                if status.success() {
                    return Ok(());
                }
                return Err(Outcome::DidNotBuild {
                    detail: String::from_utf8_lossy(&out.stderr).trim().to_string(),
                });
            }
            Ok(None) => {}
            Err(e) => {
                return Err(Outcome::DidNotRun {
                    detail: format!("waiting for the compiler: {e}"),
                });
            }
        }
        if start.elapsed() >= limit {
            let _ = child.kill();
            let _ = child.wait();
            // **And `cc1`.** A compiler driver spawns the real compiler as a child, so killing
            // `cc` leaves `cc1` running on the blocked source — the same leak as the harness's
            // forked child, in the path the fix for that one did not touch. The unit's own
            // filename is on `cc1`'s command line.
            kill_descendants(src);
            return Err(Outcome::DidNotRun {
                detail: format!(
                    "the compiler did not finish within {}s and was killed — a source that \
                     blocks it (an `#include` naming a FIFO, say) is not a verdict",
                    limit.as_secs()
                ),
            });
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Run a binary under 050 §6's limits, killing it if it overruns.
///
/// **Kills the whole process group.** The leader is `unshare`, which execs in place, so a kill
/// reaches the harness — but a child it forked outlives it and the runner returns while a
/// grandchild is still running. `setsid` puts the run in a group of its own and a negative pid
/// signals all of it.
fn bounded(bin: &Path, dir: &Path, limit: std::time::Duration) -> Result<(), String> {
    bounded_status(bin, dir, limit).map(|_| ())
}

/// [`bounded`], returning how the process ended.
///
/// A *divergence* harness cares only that its program finished; a *finding* harness is about
/// whether it **faulted**, and the signal is the whole answer. Same launcher, same limits — the
/// sandbox, the wall clock and the descendant kill were each earned by a review finding a hole,
/// and a second copy would start again from the first one.
fn bounded_status(
    bin: &Path,
    dir: &Path,
    limit: std::time::Duration,
) -> Result<std::process::ExitStatus, String> {
    let sb = sandbox();
    // **The limits applied are the limits reported.** `sandbox()` used to say the network was
    // isolated whenever `unshare` existed, while this applied isolation only when a shell was
    // there too — so on a machine with no `sh` the report said "isolated" and a harness reached
    // the network. One decision, made once, in `Sandbox::plan`.
    let mut cmd = match sb.plan() {
        Some((launcher, args)) => {
            let mut c = std::process::Command::new(launcher);
            c.args(args).arg(format!(
                "ulimit -v {}; exec '{}'",
                sb.memory_bytes.unwrap_or(2 << 30) / 1024,
                bin.display()
            ));
            c
        }
        None => std::process::Command::new(bin),
    };
    let mut child = cmd
        .current_dir(dir)
        .env_clear()
        // `sh` needs to find `unshare`'s target and its own builtins; an empty PATH broke the
        // launcher rather than the harness.
        .env("PATH", "/usr/bin:/bin")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("the harness would not start: {e}"))?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return Ok(st),
            Ok(None) => {}
            Err(e) => return Err(format!("waiting for the harness: {e}")),
        }
        if start.elapsed() >= limit {
            let _ = child.kill();
            let _ = child.wait();
            // **And anything it forked.** Killing our child reaches the harness, because
            // `unshare -rn` execs in place — but a process the harness forked survives, is
            // reparented to init, and keeps running after this returns. `fork` does not change
            // `argv`, and the binary's path is unique to this call, so matching on it finds
            // exactly this run's descendants and nothing else.
            //
            // By command line rather than by process group: putting the run in its own session
            // needs the group id, which `setsid` does not report back, and a known-unique path
            // needs nothing this crate does not already have.
            kill_descendants(bin);
            return Err(format!(
                "the harness did not finish within {}s and was killed — the witness may be an \
                 input on which the program does not terminate",
                limit.as_secs()
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Run the `before` program once more, to see whether it answers the same way twice.
fn rerun_first(
    r: &Replay,
    cc: &Path,
    dir: &Path,
    flags: &[String],
    tag: &str,
) -> Result<i64, String> {
    let (name, text) = &r.units[0];
    let src = dir.join(format!("chiero_recheck_{tag}_{name}"));
    let bin = dir.join(format!("chiero_recheck_{tag}.bin"));
    let result = dir.join(format!("chiero_recheck_{tag}.txt"));
    std::fs::write(&src, text).map_err(|e| format!("cannot write {}: {e}", src.display()))?;
    let _ = std::fs::remove_file(&result);
    let o = std::process::Command::new(cc)
        .args(["-std=gnu11", "-w", "-O0"])
        .args(flags)
        .arg(format!("-DCHIERO_RESULT=\"{}\"", result.display()))
        .arg("-o")
        .arg(&bin)
        .arg(&src)
        .output()
        .map_err(|e| format!("{} could not be run: {e}", cc.display()))?;
    if !o.status.success() {
        return Err(format!(
            "the determinism re-check would not build: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ));
    }
    bounded(&bin, dir, sandbox().timeout)?;
    let text = std::fs::read_to_string(&result)
        .map_err(|e| format!("the determinism re-check wrote no result: {e}"))?;
    parse_value(&text).ok_or_else(|| "the determinism re-check's result is unreadable".to_string())
}

fn label(n: usize) -> &'static str {
    if n == 0 { "before" } else { "after" }
}

fn parse_value(text: &str) -> Option<i64> {
    text.lines()
        .find_map(|l| l.strip_prefix("value="))
        .and_then(|v| v.trim().parse().ok())
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

/// What running a finding's harness established — 040 contract 4, and contract 11's rule in the
/// shape one program needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindingOutcome {
    /// The program died on a signal at the witness. **The only outcome that reproduces a
    /// fault**, and the signal says which one — `SIGFPE` for a division by zero, `SIGSEGV` for
    /// a null dereference.
    Faulted {
        signal: i32,
    },
    /// It ran to completion and returned this. **Not a confirmation**: the witness did not
    /// reach the fault chiero reported, or the fault is one that does not trap.
    Completed {
        value: i64,
    },
    DidNotBuild {
        detail: String,
    },
    DidNotRun {
        detail: String,
    },
}

impl FindingOutcome {
    /// Whether this reproduces the finding. Nothing else may be read as confirmation.
    pub fn confirms(&self) -> bool {
        matches!(self, FindingOutcome::Faulted { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            FindingOutcome::Faulted { .. } => "faulted",
            FindingOutcome::Completed { .. } => "completed",
            FindingOutcome::DidNotBuild { .. } => "did_not_build",
            FindingOutcome::DidNotRun { .. } => "did_not_run",
        }
    }
}

/// Emit a harness for one finding — 040 §3.
///
/// One program: include the source, call the entry at the witness, report what came back. The
/// refusals are [`renderable`]'s, unchanged, because they are the same rule — a witness that is
/// not an argument list cannot be passed positionally whatever it is a witness *for*.
pub fn emit_finding(
    source: &Path,
    entry: &str,
    witness: &Witness,
    what: &str,
) -> Result<Replay, Refusal> {
    let args = renderable(witness)?;
    let source_path = absolute(source);
    let claim = format!("`{entry}` in {} reaches {what}", source_path.display());
    let unit = format!(
        "/* chiero replay: {claim}\n   \
         Runs the function once at the witness. A fault is the finding reproduced; running to\n   \
         completion is not, and is reported as such rather than as a confirmation. */\n\
         #include <stdio.h>\n\
         #include <unistd.h>\n\
         #define {entry} chiero_target_{entry}\n\
         #define main chiero_target_main\n\
         #include \"{}\"\n\
         #undef main\n\
         #undef {entry}\n\n\
         int main (void)\n{{\n  \
         long long v = (long long) chiero_target_{entry} ({});\n  \
         FILE *o = fopen (CHIERO_RESULT, \"w\");\n  \
         if (!o) return 2;\n  \
         fprintf (o, \"value=%lld\\n\", v);\n  \
         fclose (o);\n  \
         /* `_exit`: an atexit handler the analysed code registered must not rewrite this. */\n  \
         _exit (0);\n}}\n",
        source_path.display(),
        args.join(", ")
    );
    Ok(Replay {
        source: format!("/* chiero replay: {claim} */\n"),
        units: vec![("chiero_target.c".to_string(), unit)],
        claim,
    })
}

/// Build and run a finding's harness under 050 §6's limits.
pub fn run_finding(r: &Replay, cc: &Path, dir: &Path, flags: &[String]) -> FindingOutcome {
    if r.units.len() != 1 {
        return FindingOutcome::DidNotRun {
            detail: format!(
                "a finding's harness is one program; this one has {}",
                r.units.len()
            ),
        };
    }
    if let Some(objection) = launchable(dir, flags) {
        return FindingOutcome::DidNotRun { detail: objection };
    }
    if std::fs::create_dir_all(dir).is_err() {
        return FindingOutcome::DidNotRun {
            detail: format!("cannot create {}", dir.display()),
        };
    }
    let tag = unique_tag(&r.units[0].1);
    let src = dir.join(format!("chiero_finding_{tag}.c"));
    let bin = dir.join(format!("chiero_finding_{tag}.bin"));
    let result = dir.join(format!("chiero_finding_{tag}.txt"));
    if let Err(e) = std::fs::write(&src, &r.units[0].1) {
        return FindingOutcome::DidNotRun {
            detail: format!("cannot write {}: {e}", src.display()),
        };
    }
    let _ = std::fs::remove_file(&result);
    if let Err(o) = compile(cc, flags, &result, &bin, &src, dir, sandbox().timeout) {
        return match o {
            Outcome::DidNotBuild { detail } => FindingOutcome::DidNotBuild { detail },
            other => FindingOutcome::DidNotRun {
                detail: format!("{other:?}"),
            },
        };
    }
    let status = match bounded_status(&bin, dir, sandbox().timeout) {
        Ok(s) => s,
        Err(detail) => return FindingOutcome::DidNotRun { detail },
    };
    // **A signal is the answer, and the exit code is not.** A program killed by `SIGFPE` has no
    // exit code at all, and one that returned 136 did not fault. `ExitStatus`'s Unix extension
    // is the only place that distinction lives.
    use std::os::unix::process::ExitStatusExt;
    if let Some(signal) = status.signal() {
        return FindingOutcome::Faulted { signal };
    }
    match std::fs::read_to_string(&result)
        .ok()
        .and_then(|t| parse_value(&t))
    {
        Some(value) => FindingOutcome::Completed { value },
        None => FindingOutcome::DidNotRun {
            detail: "the harness finished without faulting and wrote no value".to_string(),
        },
    }
}
