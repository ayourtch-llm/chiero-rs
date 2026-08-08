//! **The compiler persona** — 012 §4.1.
//!
//! chiero does not report its own capabilities to a header; it **impersonates the compiler the
//! code is built with**. `#if __GNUC__ > 4` is not asking what chiero can do, and answering it
//! honestly-about-chiero configures a program nobody compiles.
//!
//! Until 2026-08-08 that impersonation was an array literal inside `Engine::new`: real, unnamed,
//! and unreplaceable. Every gap in it was found by a corpus falling into a `#else`, and every fix
//! was another line in the array. In one session it cost five defects — including `__BYTE_ORDER__`
//! undefined, so `#if __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__` read `0 == 0` and reversed bit-field
//! member order across a whole VPP plugin — and **61 million tokens of VPP, 8% of the program,
//! that were never compiled at all**.
//!
//! Worse, there were *two* mechanisms for the one fact: `chiero-cli` captured all 401 of gcc's
//! predefines while the library baked 23. A fix to one proved nothing about the other.
//!
//! # The format is `cc -dM -E` output, deliberately
//!
//! ```text
//! $ gcc -dM -E -x c /dev/null > personas/gcc-13-x86_64-linux.h
//! ```
//!
//! A persona file is what the compiler already prints. **No new parser, no new dependency, no new
//! syntax** — and one can be captured from any real compiler on any target, checked in, diffed,
//! and hand-edited. [`Persona::from_defines`] reads that text; anything that runs a compiler to
//! produce it belongs in a caller, which keeps subprocess logic out of the frontend.

use std::collections::BTreeMap;

/// A named set of object-like predefined macros.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Persona {
    name: String,
    defines: BTreeMap<String, String>,
}

/// Macros a persona never supplies, because the engine owns them.
///
/// The first five have values that depend on *where they appear*, so a fixed string would be
/// wrong everywhere but one line. `__COUNTER__` is stateful. A `cc -dM` dump contains them, so
/// they are filtered here rather than trusted not to be there.
const ENGINE_OWNED: [&str; 5] = [
    "__FILE__",
    "__LINE__",
    "__DATE__",
    "__TIME__",
    "__COUNTER__",
];

impl Persona {
    /// **gcc 13.3 on x86-64 Linux** — the set chiero has always impersonated, now under its name.
    ///
    /// Every entry earned its place by a measured failure; see the git history of this file and
    /// HANDOFF §9.1. The platform and endianness groups in particular are not decoration:
    /// without them VPP's `pmalloc.c` reaches `#error "Unsupported OS"` and every little-endian
    /// layout in the tree silently becomes a big-endian one.
    pub fn baked() -> Self {
        let mut p = Self {
            name: "gcc-13-x86_64-linux".into(),
            defines: BTreeMap::new(),
        };
        for (k, v) in [
            ("__STDC__", "1"),
            ("__STDC_HOSTED__", "1"),
            // ⚠️ C11, because 013 makes the parser C11 + GNU extensions. gcc's own default here
            // is gnu17 (`201710L`), and the difference is deliberate — announcing C17 over a C11
            // parser would be a worse lie than the one this type exists to fix. HANDOFF §9.1.
            ("__STDC_VERSION__", "201112L"),
            ("__GNUC__", "13"),
            ("__GNUC_MINOR__", "3"),
            ("__GNUC_PATCHLEVEL__", "0"),
            ("__x86_64__", "1"),
            ("__x86_64", "1"),
            ("__linux__", "1"),
            ("__linux", "1"),
            ("linux", "1"),
            ("__unix__", "1"),
            ("__unix", "1"),
            ("unix", "1"),
            ("__gnu_linux__", "1"),
            ("__ELF__", "1"),
            ("__LP64__", "1"),
            ("_LP64", "1"),
            ("__amd64__", "1"),
            ("__amd64", "1"),
            ("__SIZEOF_POINTER__", "8"),
            ("__SSE__", "1"),
            ("__SSE2__", "1"),
            ("__ORDER_LITTLE_ENDIAN__", "1234"),
            ("__ORDER_BIG_ENDIAN__", "4321"),
            ("__ORDER_PDP_ENDIAN__", "3412"),
            ("__BYTE_ORDER__", "1234"),
            ("__FLOAT_WORD_ORDER__", "1234"),
        ] {
            p.defines.insert(k.into(), v.into());
        }
        p
    }

    /// Parse `cc -dM -E` output — one `#define NAME [replacement]` per line.
    ///
    /// **Function-like macros are skipped**, not mangled: `#define __has_builtin(x) …` is the
    /// engine's own, and taking it as an object macro would shadow a working feature query with a
    /// broken constant. So are [`ENGINE_OWNED`]. Lines that are not `#define` are ignored, so a
    /// hand-edited persona may carry comments and blanks.
    pub fn from_defines(name: impl Into<String>, text: &str) -> Self {
        let mut defines = BTreeMap::new();
        for line in text.lines() {
            let Some(rest) = line.trim_start().strip_prefix("#define ") else {
                continue;
            };
            let rest = rest.trim_start();
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            let (macro_name, tail) = rest.split_at(end);
            if macro_name.is_empty() || tail.starts_with('(') || ENGINE_OWNED.contains(&macro_name)
            {
                continue;
            }
            defines.insert(macro_name.to_string(), tail.trim().to_string());
        }
        Self {
            name: name.into(),
            defines,
        }
    }

    /// The persona's name, so a result can say which compiler it was answering as.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn get(&self, macro_name: &str) -> Option<&str> {
        self.defines.get(macro_name).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.defines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defines.is_empty()
    }

    /// Every macro, in a stable order — the engine installs these before `Config::defines`.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.defines.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

impl Default for Persona {
    fn default() -> Self {
        Self::baked()
    }
}
