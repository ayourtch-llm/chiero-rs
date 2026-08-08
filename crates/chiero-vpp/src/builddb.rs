//! **The build ingest** — [060 §1](../../../docs/specs/060-vpp-integration.md), contract 1.
//!
//! A compile database says how each translation unit is *actually* compiled. Those flags are not
//! optional detail: they "determine the `ConfigId` ([012 §3.3]), which determines which `#if`
//! branches exist, which determines layout, which determines every offset in the analysis"
//! (060 §1). Analysing VPP without them analyses a different program.
//!
//! **This reads text, not a path.** 060 §1 assumed a `compile_commands.json` file; VPP's build
//! does not write one, but `ninja -C <build> -t compdb` emits the identical format on stdout in
//! 90 ms. Taking `&str` means the caller chooses, and no VPP tree needs re-configuring.
//!
//! Built for what VPP's database actually contains, measured before a line was written: `-D`
//! (5495 occurrences) and `-I` (7857) are the only configuration-bearing flags in it. Anything
//! else that *would* bear on configuration is collected into [`TranslationUnit::unhandled`]
//! rather than ignored — see the note there.

use chiero_pp::ConfigId;
use std::path::{Path, PathBuf};

/// One compilation of one source file.
///
/// **One source may have several of these**, and any index keyed on `src` alone is wrong: VPP
/// recompiles 208 of its 1562 C sources under different `CLIB_MARCH_VARIANT`s, one of them five
/// times over (060 §1.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranslationUnit {
    /// The source, resolved against `dir`. Every VPP compilation names it absolutely; CMake's
    /// own writer does not, and the format permits either.
    pub src: PathBuf,
    /// The database's `directory`: the cwd the compiler ran in, and the root for every
    /// relative path in the entry.
    pub dir: PathBuf,
    /// The database's `output`. It is what distinguishes two TUs of the same source, so a
    /// finding can name the variant it came from.
    pub object: PathBuf,
    /// The full argument vector, after shell tokenization. Kept so a caller can ask something
    /// this module did not think to extract, instead of re-tokenizing the string itself.
    pub args: Vec<String>,
    /// `-D` in command-line order. A bare `-DNAME` is `("NAME", "1")`, per C11 6.10.3p9 —
    /// **not** the empty string, which would make `#if NAME` false and delete live code.
    pub defines: Vec<(String, String)>,
    /// `-I` in command-line order, which is search order, resolved against `dir`.
    pub include_paths: Vec<PathBuf>,
    /// **`-m…` in command-line order — the flags that select a compiler persona.**
    ///
    /// Kept apart from [`Self::defines`] and [`Self::include_paths`] because they are not
    /// preprocessor configuration in the same sense: `__SSE4_2__` and `__AVX2__` exist only under
    /// the right `-march`, and **only the compiler knows what each flag implies**. Hand these to
    /// `chiero_pp::Persona` (via a `cc -dM -E` probe) rather than interpreting them here.
    ///
    /// VPP compiles the same source repeatedly under different `-march` (060 §1.1), so this is
    /// per translation unit, not per project — which is why the whole item was per-TU from the
    /// start. `-mtune` rides along deliberately: whether it moves a predefine is the compiler's
    /// business, not this ingest's.
    pub target_flags: Vec<String>,
    /// **Flags that would change the configuration and that this ingest does not model.**
    ///
    /// None of them occurs in VPP's database — that is measured, not assumed — so building
    /// support for them would be building for an imagined caller. But silently dropping one
    /// would produce a confidently wrong `Config` on some other project, so they are named
    /// instead. `-U` and `-include` in particular have no representation in
    /// [`chiero_pp::Config`] at all, so the gap is in the config type, not just here.
    pub unhandled: Vec<String>,
    /// A function of exactly [`Self::defines`] and [`Self::include_paths`] — see
    /// [`BuildDb::distinct_configs`] for why that scoping is the whole point.
    pub config: ConfigId,
}

/// Flags this ingest does not model but that would change what the preprocessor sees.
const UNHANDLED: [&str; 6] = [
    "-U",
    "-isystem",
    "-iquote",
    "-include",
    "-imacros",
    "-nostdinc",
];

impl TranslationUnit {
    /// The preprocessor configuration this unit compiles under, carrying its own [`ConfigId`]
    /// **and the persona its own `-march` selects**.
    ///
    /// The ingest hands this over ready-made because otherwise every caller re-derives it, and
    /// two callers deriving it differently is exactly the bug the `ConfigId` exists to catch.
    ///
    /// **The probe is a parameter rather than an option** for the same reason. This used to return
    /// a config with the baked persona, so every one of VPP's 1963 target-carrying units was
    /// preprocessed as a compiler with no `-march` — and `__AVX2__`, which guards every 32-byte
    /// vector type in vppinfra, was undefined in all of them. A caller could not have noticed:
    /// the wrong branch of an `#if` emits no diagnostic. Making the join unskippable is what
    /// stops that being one caller's mistake to repeat.
    ///
    /// It costs one `cc -dM -E` per *distinct* flag-set — 8 for VPP's 1967 units, measured — because
    /// [`chiero_probe::Probe`] memoizes on the flags. A machine with no compiler gets the baked
    /// persona, which is what chiero has always impersonated.
    pub fn pp_config(&self, probe: &chiero_probe::Probe) -> chiero_pp::Config {
        chiero_pp::Config {
            id: self.config,
            include_paths: self.include_paths.clone(),
            defines: self.defines.clone(),
            persona: probe.persona(&self.target_flags),
            ..chiero_pp::Config::default()
        }
    }
}

/// Every compilation in one build, kept whole.
#[derive(Clone, Debug, Default)]
pub struct BuildDb {
    units: Vec<TranslationUnit>,
    non_compilations: usize,
}

impl BuildDb {
    /// Parse a compile database — either `ninja -t compdb` output or a `compile_commands.json`.
    ///
    /// **Malformed input is an error, never an empty database.** An empty one is the dangerous
    /// answer: every downstream count would report "0 TUs failed" and read as green.
    pub fn parse(json: &str) -> Result<Self, String> {
        let v: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("compile database is not JSON: {e}"))?;
        let arr = v
            .as_array()
            .ok_or_else(|| "compile database must be a JSON array of entries".to_string())?;
        let mut units = Vec::with_capacity(arr.len());
        let mut non_compilations = 0;
        for (i, e) in arr.iter().enumerate() {
            match unit_from(e).map_err(|m| format!("entry {i}: {m}"))? {
                Some(u) => units.push(u),
                None => non_compilations += 1,
            }
        }
        Ok(Self {
            units,
            non_compilations,
        })
    }

    pub fn units(&self) -> &[TranslationUnit] {
        &self.units
    }

    /// Rows that describe no compilation.
    ///
    /// **`ninja -t compdb` with no rule argument dumps every edge**, and 2902 of VPP's 6235 are
    /// phony order-only ones: empty `command`, an `output` like
    /// `cmake_object_order_depends_target_vlibmemoryclient`, and a `file` naming a *generated*
    /// source. Counting them as units turns 1967 C compilations into 2226, each with no defines
    /// and no include paths — a configuration that would analyse a different program in silence.
    ///
    /// Counted rather than dropped, because a filter that quietly shrinks a corpus is one nobody
    /// can check.
    pub fn non_compilations(&self) -> usize {
        self.non_compilations
    }

    /// The C translation units. The extension test lives here rather than in each caller,
    /// because "which of VPP's 3333 compilations are C" should have one answer, not one per caller.
    pub fn c_units(&self) -> impl Iterator<Item = &TranslationUnit> {
        self.units.iter().filter(|u| {
            u.src
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("c"))
        })
    }

    /// Every unit built from `src`. Returns all of them — see [`TranslationUnit`] on why a
    /// map keyed by path would be wrong.
    pub fn units_for<'a>(&'a self, src: &'a Path) -> impl Iterator<Item = &'a TranslationUnit> {
        self.units.iter().filter(move |u| u.src == src)
    }

    /// How many distinct configurations the whole build uses.
    ///
    /// **This number is the contract's value.** VPP's 1967 C units carry 423 configurations, a
    /// 4.6× collapse, and every per-configuration analysis is that much cheaper. The collapse
    /// exists only because the id ignores `-o`, `-MF`, `-Wall` and the rest; an id hashed over
    /// the whole command line would be unique per unit and buy nothing at all.
    pub fn distinct_configs(&self) -> usize {
        let mut ids: Vec<u64> = self.units.iter().map(|u| u.config.0).collect();
        ids.sort_unstable();
        ids.dedup();
        ids.len()
    }
}

/// `Ok(None)` for a row that describes no compilation — see [`BuildDb::non_compilations`].
fn unit_from(e: &serde_json::Value) -> Result<Option<TranslationUnit>, String> {
    let file = e
        .get("file")
        .and_then(|v| v.as_str())
        .ok_or("no `file`, so it names no translation unit")?;
    let dir = PathBuf::from(e.get("directory").and_then(|v| v.as_str()).unwrap_or(""));

    // Both spellings are the format: CMake writes `arguments`, `ninja -t compdb` writes
    // `command`. An entry with neither, or with a blank one, invokes no compiler.
    let args: Vec<String> = if let Some(a) = e.get("arguments").and_then(|v| v.as_array()) {
        a.iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    } else {
        tokenize(e.get("command").and_then(|v| v.as_str()).unwrap_or(""))
    };
    if args.is_empty() {
        return Ok(None);
    }

    let mut defines = Vec::new();
    let mut include_paths = Vec::new();
    let mut target_flags = Vec::new();
    let mut unhandled = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        // `-I/src` and `-I /src` are the same flag; so are `-DA=1` and `-D A=1`.
        let split = |a: &str, it: &mut std::slice::Iter<'_, String>| -> Option<String> {
            let rest = a[2..].to_string();
            if rest.is_empty() {
                it.next().cloned()
            } else {
                Some(rest)
            }
        };
        if let Some(rest) = a.strip_prefix("-D").and_then(|_| split(a, &mut it)) {
            let (name, val) = match rest.split_once('=') {
                Some((n, v)) => (n.to_string(), v.to_string()),
                // C11 6.10.3p9: an object-like macro defined with no value is `1`.
                None => (rest, "1".to_string()),
            };
            defines.push((name, val));
        } else if let Some(rest) = a.strip_prefix("-I").and_then(|_| split(a, &mut it)) {
            include_paths.push(resolve(&dir, &rest));
        } else if a.starts_with("-m") && a.len() > 2 {
            target_flags.push(a.clone());
        } else if let Some(f) = UNHANDLED.iter().find(|f| a.starts_with(**f)) {
            unhandled.push(a.clone());
            // The separated spelling eats its argument too, or it would be read as a source.
            if a.len() == f.len() && *f != "-nostdinc" {
                it.next();
            }
        }
    }

    let config = config_id(&defines, &include_paths);
    Ok(Some(TranslationUnit {
        src: resolve(&dir, file),
        object: PathBuf::from(e.get("output").and_then(|v| v.as_str()).unwrap_or("")),
        dir,
        args,
        defines,
        include_paths,
        target_flags,
        unhandled,
        config,
    }))
}

fn resolve(dir: &Path, p: &str) -> PathBuf {
    let p = Path::new(p);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        dir.join(p)
    }
}

/// FNV-1a over a canonical rendering of the configuration-bearing flags.
///
/// **Deliberately not `DefaultHasher`**: a `ConfigId` can be recorded in a finding, so two runs
/// — and two chiero versions — must agree on it, which `RandomState` does not promise. Order is
/// preserved rather than sorted: `-I` order *is* search order, and a redefinition's outcome
/// depends on `-D` order too.
fn config_id(defines: &[(String, String)], includes: &[PathBuf]) -> ConfigId {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |b: &[u8]| {
        for &c in b {
            h ^= c as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    };
    for (n, v) in defines {
        eat(b"D");
        eat(n.as_bytes());
        eat(b"=");
        eat(v.as_bytes());
        eat(b"\0");
    }
    for i in includes {
        eat(b"I");
        eat(i.as_os_str().as_encoded_bytes());
        eat(b"\0");
    }
    // `ConfigId::default()` is 0 and means "unset"; a real configuration must never collide
    // with it, or "every TU yields a ConfigId" becomes unfalsifiable.
    ConfigId(if h == 0 { 1 } else { h })
}

/// Shell-style tokenization of a `command` string.
///
/// Quotes are not decoration: `-DFOO="a b"` is one flag with a two-word value, and splitting on
/// whitespace turns it into a define of `"a` plus a stray source file.
fn tokenize(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut in_tok) = (Vec::new(), String::new(), false);
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match c {
            c if c.is_ascii_whitespace() => {
                if in_tok {
                    out.push(std::mem::take(&mut cur));
                    in_tok = false;
                }
            }
            '\'' => {
                in_tok = true;
                for c in it.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    cur.push(c);
                }
            }
            '"' => {
                in_tok = true;
                while let Some(c) = it.next() {
                    match c {
                        '"' => break,
                        '\\' => cur.extend(it.next()),
                        c => cur.push(c),
                    }
                }
            }
            '\\' => {
                in_tok = true;
                cur.extend(it.next());
            }
            c => {
                in_tok = true;
                cur.push(c);
            }
        }
    }
    if in_tok {
        out.push(cur);
    }
    out
}
