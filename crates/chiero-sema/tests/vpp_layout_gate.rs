//! Covers: 014 contract 12.
//!
//! **The gate.** For every record type chiero can parse out of the VPP corpus, generated
//! `_Static_assert`s for size, alignment and every field offset must compile cleanly under
//! gcc. Failures are counted and must be zero.
//!
//! This is 014 §7's argument taken to its conclusion. Every other layout test states a
//! number I believed; this one states nothing at all — it asks chiero for the layout of a
//! few hundred real records and lets the compiler that defines the ABI reject the answers.
//! There is no expected output in this file, which is the point.

mod harness;

use chiero_parse::{ScopedTypedefs, parse_tu};
use chiero_pp::{Config, FileLoader, PreprocessedTu, PreprocessorSession};
use chiero_sema::{RecordLayout, TargetConfig, analyze};
use std::io;
use std::path::{Path, PathBuf};

struct Disk;
impl FileLoader for Disk {
    fn load(&mut self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name>/ has a workspace root above it")
        .join("tests/corpus/vpp")
}

const SEEDS: &[&str] = &[
    "vppinfra/vec.h",
    "vppinfra/pool.h",
    "vppinfra/bitmap.h",
    "vppinfra/format.h",
    "vppinfra/hash.h",
    "vppinfra/error.h",
];

fn system_include_paths() -> Option<Vec<PathBuf>> {
    let out = std::process::Command::new("gcc")
        .args(["-E", "-v", "-std=gnu11", "-x", "c", "/dev/null"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let mut paths = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with("#include <...>") {
            inside = true;
            continue;
        }
        if line.starts_with("End of search list") {
            break;
        }
        if inside {
            let p = PathBuf::from(line.trim());
            if p.is_dir() {
                paths.push(p);
            }
        }
    }
    (!paths.is_empty()).then_some(paths)
}

fn gcc_predefines() -> Vec<(String, String)> {
    let Ok(out) = std::process::Command::new("gcc")
        .args(["-dM", "-E", "-std=gnu11", "-x", "c", "/dev/null"])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut it = line.splitn(3, ' ');
            if it.next() != Some("#define") {
                return None;
            }
            let name = it.next()?;
            if name.contains('(')
                || matches!(
                    name,
                    "__FILE__" | "__LINE__" | "__DATE__" | "__TIME__" | "__COUNTER__"
                )
            {
                return None;
            }
            Some((name.to_string(), it.next().unwrap_or("1").to_string()))
        })
        .collect()
}

fn preprocess(seed: &str, sys: &[PathBuf]) -> PreprocessedTu {
    let cfg = Config {
        include_paths: vec![corpus_dir()],
        system_paths: sys.to_vec(),
        defines: gcc_predefines(),
        ..Config::default()
    };
    let session = PreprocessorSession::new();
    session.preprocess_with_loader(
        corpus_dir().join("tu.c"),
        &format!("#include <{seed}>\n"),
        cfg,
        &mut Disk,
    )
}

/// A record chiero laid out, named the way C can refer to it.
struct Named<'a> {
    /// `"struct"`, `"union"`, or empty when the record is reached through a typedef.
    keyword: &'static str,
    tag: String,
    layout: &'a RecordLayout,
}

#[test]
fn every_corpus_record_layout_is_accepted_by_gcc() {
    let Some(sys) = system_include_paths() else {
        eprintln!("skipping: gcc not found (014 contract 12)");
        return;
    };

    let mut total_records = 0usize;
    let mut total_asserts = 0usize;
    let mut failures = Vec::new();

    for seed in SEEDS {
        let tu = preprocess(seed, &sys);
        assert!(
            tu.diagnostics.is_empty(),
            "{seed}: the preprocessor must be clean or the layouts are of nothing: {:?}",
            tu.diagnostics.iter().take(3).collect::<Vec<_>>()
        );
        let mut oracle = ScopedTypedefs::new();
        let parsed = parse_tu(&tu, &mut oracle);
        assert!(
            parsed.diagnostics.is_empty(),
            "{seed}: and the parse must be clean: {:?}",
            parsed.diagnostics.iter().take(3).collect::<Vec<_>>()
        );

        let names = harness::names_of(&parsed);
        let analysis = analyze(&parsed.ast, &TargetConfig::x86_64_linux(), &names);

        // A record can be named in a generated assertion two ways, and **both are
        // needed**: by its tag, or — when it has none — by a `typedef` that names it.
        // `typedef struct { ... } foo_t;` is the dominant idiom in both VPP and glibc, so
        // a generator that only knew tags would silently skip four fifths of the records
        // and still report zero failures. That was the first version of this gate.
        let mut named: Vec<Named> = Vec::new();
        let mut anonymous = 0usize;
        let mut by_typedef: indexmap::IndexMap<u32, String> = Default::default();
        for &item in parsed.ast.items() {
            if let chiero_ast::DeclKind::Typedef { name, .. } = &parsed.ast.decl(item).kind {
                let Some(ty) = analysis.ty_of_decl(item) else {
                    continue;
                };
                if let (chiero_sema::Ty::Record(r), Some(text)) =
                    (analysis.ty(ty), parsed.text(*name))
                {
                    by_typedef.entry(r.0).or_insert_with(|| text.to_owned());
                }
            }
        }
        for (i, layout) in analysis.records().iter().enumerate() {
            total_records += 1;
            let rid = chiero_sema::RecordId(i as u32);
            let tagged = analysis.tag_of(rid).and_then(|s| parsed.text(s)).map(|t| {
                (
                    if layout.is_union { "union" } else { "struct" },
                    t.to_owned(),
                )
            });
            match tagged.or_else(|| by_typedef.get(&rid.0).map(|t| ("", t.clone()))) {
                Some((keyword, tag)) => named.push(Named {
                    keyword,
                    tag,
                    layout,
                }),
                None => anonymous += 1,
            }
        }

        let mut prog = format!("#include <{seed}>\n");
        let mut asserts = 0usize;
        for n in &named {
            prog.push_str(&format!(
                "_Static_assert(sizeof({} {}) == {}, \"size {}\");\n",
                n.keyword, n.tag, n.layout.size, n.tag
            ));
            prog.push_str(&format!(
                "_Static_assert(_Alignof({} {}) == {}, \"align {}\");\n",
                n.keyword, n.tag, n.layout.align, n.tag
            ));
            asserts += 2;
            for f in &n.layout.fields {
                // `__builtin_offsetof` is ill-formed on a bit-field, and an unnamed
                // member cannot be referred to at all.
                if f.bits.is_some() {
                    continue;
                }
                let Some(fname) = f.name.and_then(|s| parsed.text(s)) else {
                    continue;
                };
                prog.push_str(&format!(
                    "_Static_assert(__builtin_offsetof({} {}, {}) == {}, \"off {}.{}\");\n",
                    n.keyword, n.tag, fname, f.offset, n.tag, fname
                ));
                asserts += 1;
            }
        }
        total_asserts += asserts;

        eprintln!(
            "{seed}: {} records ({} named, {} anonymous), {asserts} assertions",
            named.len() + anonymous,
            named.len(),
            anonymous
        );

        let dir = std::env::temp_dir().join(format!(
            "chiero-vpp-gate-{}-{}",
            std::process::id(),
            seed.replace('/', "_")
        ));
        let _ = std::fs::create_dir_all(&dir);
        let c = dir.join("gate.c");
        std::fs::write(&c, &prog).expect("write");
        let mut cmd = std::process::Command::new("gcc");
        cmd.args(["-std=gnu11", "-w", "-fsyntax-only"]);
        cmd.arg(format!("-I{}", corpus_dir().display()));
        cmd.arg(&c);
        let out = cmd.output().expect("run gcc");
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            // Each failed `_Static_assert` names the record and field in its message.
            for line in err
                .lines()
                .filter(|l| l.contains("static assertion failed"))
            {
                failures.push(format!("{seed}: {}", line.trim()));
            }
            if failures.is_empty() {
                failures.push(format!(
                    "{seed}: gcc failed without a static-assertion message:\n{err}"
                ));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    eprintln!("gate: {total_records} records, {total_asserts} assertions put to gcc");

    // A gate that quantifies over an empty set passes vacuously — the sixth vacuity this
    // project has had to fix. These floors are what make "zero failures" a claim.
    assert!(
        total_records > 100,
        "only {total_records} records were laid out; the corpus has hundreds, so \
         something resolved to the wrong file"
    );
    assert!(
        total_asserts > 500,
        "only {total_asserts} assertions were generated from {total_records} records"
    );
    assert!(
        failures.is_empty(),
        "{} layout(s) rejected by gcc:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
