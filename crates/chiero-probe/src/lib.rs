//! **The one place that asks a real compiler what it predefines** — 012 §4.1's persona, probed.
//!
//! A [`chiero_pp::Persona`] is a named set of predefines and its file format *is* `cc -dM -E`
//! output; `Persona::from_defines` reads that text and deliberately knows nothing about running a
//! compiler. Something has to run one, and until now two things did — `chiero-cli`'s frontend, and
//! nothing else, which is why 012 contract 17's corpus gate preprocessed VPP under a persona the
//! real build never uses. Adding a second probe to `chiero-vpp` would have made **three mechanisms
//! for one fact**, which is the shape of defect this project has now paid for twice (HANDOFF §9.1).
//!
//! So it is a crate: `chiero-pp` stays free of subprocesses, and every caller that needs "what does
//! *this* compiler predefine under *these* flags" gets the same answer from the same place.
//!
//! # Why the answer is keyed on the flags
//!
//! `__SSE4_2__` and `__AVX2__` exist only under the right `-march`, and **only the compiler knows
//! what each flag implies** — which is why `chiero_vpp::builddb::TranslationUnit::target_flags`
//! hands them over uninterpreted. VPP compiles the same source repeatedly under different `-march`
//! (060 §1.1), so within *one run* there are several right answers and the key is what tells them
//! apart. The cache that this replaces was a `OnceLock` holding one: it took the flags, answered
//! the first caller correctly, and gave that caller's persona to every later one.
//!
//! # What it costs
//!
//! One `cc -dM -E` per distinct flag-set, not per translation unit. On VPP that is **5 probes for
//! 1967 units** — the five distinct `-march` values its build uses. [`Probe::persona_probes`]
//! counts the compiler invocations rather than the calls, because a cache that is asserted and not
//! measured is a cache nobody can check.

use chiero_pp::Persona;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

/// A compiler, and what it says about itself — memoized per flag-set.
///
/// **Discovery at run time, like the solver's (022 §4) and the replay compiler's.** chiero links
/// no toolchain (010 §1); a machine without one gets [`Persona::baked`] rather than a build-time
/// dependency, and an empty include path rather than a guess.
#[derive(Debug)]
pub struct Probe {
    cc: String,
    personas: Mutex<BTreeMap<Vec<String>, Persona>>,
    includes: OnceLock<Vec<PathBuf>>,
    persona_probes: AtomicUsize,
}

impl Default for Probe {
    fn default() -> Self {
        Self::new()
    }
}

impl Probe {
    /// `$CC`, or `cc` — the same compiler the rest of chiero asks (040 §3's replay harness, and
    /// the system include paths).
    pub fn new() -> Self {
        Self::with_compiler(std::env::var("CC").unwrap_or_else(|_| "cc".to_string()))
    }

    pub fn with_compiler(cc: impl Into<String>) -> Self {
        Self {
            cc: cc.into(),
            personas: Mutex::new(BTreeMap::new()),
            includes: OnceLock::new(),
            persona_probes: AtomicUsize::new(0),
        }
    }

    /// The process-wide probe.
    ///
    /// A cache is only worth having if callers share it, and the compiler's answers are a property
    /// of the machine. Tests that want to count probes make their own with [`Probe::new`].
    pub fn shared() -> &'static Probe {
        static SHARED: OnceLock<Probe> = OnceLock::new();
        SHARED.get_or_init(Probe::new)
    }

    /// What the compiler predefines under `target_flags` — `-march=x86-64-v3`, `-mavx2`, and
    /// friends, passed **verbatim**.
    ///
    /// Uninterpreted on purpose: `-march=x86-64-v3` implies `__AVX2__` and `-march=x86-64-v2` does
    /// not, and no table in chiero should be the authority on which. The flags are the cache key,
    /// in the order given, because that is what the caller asked and two orders are two questions
    /// as far as this crate is concerned.
    pub fn persona(&self, target_flags: &[String]) -> Persona {
        let key = target_flags.to_vec();
        if let Some(p) = self.personas.lock().expect("persona cache").get(&key) {
            return p.clone();
        }
        let p = self.run_persona_probe(target_flags);
        self.personas
            .lock()
            .expect("persona cache")
            .insert(key, p.clone());
        p
    }

    /// How many times a compiler was actually run for a persona.
    ///
    /// **The counter is on the subprocess, not on the call site.** A cache that is asserted rather
    /// than measured is a cache nobody can check, and a counter attached to the caller would keep
    /// reporting a hit rate after the memoization was removed (HANDOFF §9.1 5a).
    pub fn persona_probes(&self) -> usize {
        self.persona_probes.load(Ordering::Relaxed)
    }

    /// Where the compiler keeps its own headers.
    ///
    /// **Both this and the persona, or neither is much use.** glibc's `bits/floatn.h` branches on
    /// a dozen `__HAVE_FLOAT*` macros, so a preprocessor with the paths and not the predefines
    /// compiles code the compiler never sees — the full-tree sweep's first run reported 101
    /// findings that were entirely that.
    ///
    /// Not keyed on the target flags: `-march` selects instructions, not header directories.
    pub fn include_paths(&self) -> Vec<PathBuf> {
        self.includes
            .get_or_init(|| {
                let Ok(out) = std::process::Command::new(&self.cc)
                    .args(["-E", "-v", "-std=gnu11", "-x", "c", "/dev/null"])
                    .output()
                else {
                    return Vec::new();
                };
                let text = String::from_utf8_lossy(&out.stderr);
                let mut paths = Vec::new();
                let mut inside = false;
                for line in text.lines() {
                    if line.starts_with("#include <...>") {
                        inside = true;
                    } else if line.starts_with("End of search list") {
                        break;
                    } else if inside {
                        paths.push(PathBuf::from(line.trim()));
                    }
                }
                paths
            })
            .clone()
    }

    /// The compiler the answers come from, so a report can say who it impersonated.
    pub fn compiler(&self) -> &str {
        &self.cc
    }

    fn run_persona_probe(&self, target_flags: &[String]) -> Persona {
        // ⚠️ `-std=gnu11` deliberately: 013 makes the parser C11 + GNU extensions, and a persona
        // announcing C17 over a C11 parser would be a worse lie than the one the persona exists to
        // fix. HANDOFF §9.1 records this as the owner's open call about the language level.
        let mut args: Vec<String> = vec!["-dM".into(), "-E".into(), "-std=gnu11".into()];
        args.extend(target_flags.iter().cloned());
        args.extend(["-x".into(), "c".into(), "/dev/null".into()]);
        self.persona_probes.fetch_add(1, Ordering::Relaxed);
        let Ok(out) = std::process::Command::new(&self.cc).args(&args).output() else {
            // No compiler is a fact about the machine, not about the code — fall back to the set
            // chiero has always baked rather than to an empty persona, which would predefine
            // nothing and send every real header down its `#else`.
            return Persona::baked();
        };
        let name = if target_flags.is_empty() {
            self.cc.clone()
        } else {
            format!("{} {}", self.cc, target_flags.join(" "))
        };
        Persona::from_defines(name, &String::from_utf8_lossy(&out.stdout))
    }
}
