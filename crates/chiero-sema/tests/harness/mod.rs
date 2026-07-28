#![allow(dead_code, unreachable_pub)]

//! Shared test harness: parse + analyse, and the 014 §7 differential against gcc.

use chiero_parse::{ParsedTu, ScopedTypedefs, parse_tu};
use chiero_pp::{Config, preprocess_str};
use chiero_sema::{Analysis, RecordLayout, SymbolText, TargetConfig, TyId, analyze};
use chiero_span::Symbol;

/// A parsed and analysed TU, with the symbol table needed to read names back.
pub struct Parsed {
    pub parsed: ParsedTu,
    pub analysis: Analysis,
}

/// `chiero-sema` cannot depend on `chiero-parse` to read a `Symbol`, so the caller
/// supplies the lookup. A newtype is needed because both the trait and `ParsedTu` are
/// foreign to this crate.
struct Names<'a>(&'a ParsedTu);

impl SymbolText for Names<'_> {
    fn text(&self, sym: Symbol) -> Option<&str> {
        self.0.text(sym)
    }
}

impl Parsed {
    pub fn text(&self, sym: Symbol) -> Option<String> {
        self.parsed.text(sym).map(str::to_owned)
    }

    /// The symbol whose text is `name`, if the TU interned one.
    pub fn symbol(&self, name: &str) -> Option<Symbol> {
        (0..u32::MAX)
            .map(Symbol)
            .take_while(|s| self.parsed.text(*s).is_some())
            .find(|s| self.parsed.text(*s) == Some(name))
    }

    /// The analysed type of the file-scope variable called `name`.
    pub fn decl_ty(&self, name: &str) -> Option<TyId> {
        let sym = self.symbol(name)?;
        self.parsed
            .ast
            .items()
            .iter()
            .find(|&&id| {
                matches!(&self.parsed.ast.decl(id).kind,
                    chiero_ast::DeclKind::Var { name: Some(n), .. } if *n == sym)
            })
            .and_then(|&id| self.analysis.ty_of_decl(id))
    }
}

pub fn parse(src: &str, target: TargetConfig) -> Parsed {
    let tu = preprocess_str("t.c", src, Config::default());
    assert!(
        tu.diagnostics.is_empty(),
        "the fixture must preprocess cleanly: {:?}",
        tu.diagnostics
    );
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu(&tu, &mut oracle);
    assert!(
        parsed.diagnostics.is_empty(),
        "and parse cleanly, or sema is being graded on a broken tree: {:?}",
        parsed.diagnostics
    );
    let analysis = analyze(&parsed.ast, &target, &Names(&parsed));
    Parsed { parsed, analysis }
}

/// Parse and analyse without requiring sema to be clean — for the tests whose subject
/// *is* a diagnostic.
pub fn parse_allowing_diagnostics(src: &str, target: TargetConfig) -> Parsed {
    let tu = preprocess_str("t.c", src, Config::default());
    assert!(tu.diagnostics.is_empty(), "{:?}", tu.diagnostics);
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu(&tu, &mut oracle);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let analysis = analyze(&parsed.ast, &target, &Names(&parsed));
    Parsed { parsed, analysis }
}

/// A `SymbolText` view over a parsed TU, for calling `const_eval` directly.
pub fn names(p: &Parsed) -> impl SymbolText + '_ {
    Names(&p.parsed)
}

/// The same, over a bare `ParsedTu` from [`expression`].
pub fn names_of(p: &ParsedTu) -> impl SymbolText + '_ {
    Names(p)
}

/// Parse a bare expression by wrapping it in an initializer, and hand back the tree plus
/// the expression's id.
///
/// The returned `ParsedTu` **owns the interner those symbols index**, so the caller must
/// take its `SymbolText` from this TU and not from another. Mixing the two is silent:
/// symbol 7 exists in both and means different things, which is exactly the hazard
/// `Symbol`'s own doc comment in `chiero-span` warns about, and it produced `None` from
/// every fold in the first version of these tests.
pub fn expression(src: &str) -> (ParsedTu, chiero_ast::ExprId) {
    let text = format!("int probe = {src};");
    let tu = preprocess_str("e.c", &text, Config::default());
    assert!(tu.diagnostics.is_empty(), "{src}: {:?}", tu.diagnostics);
    let mut oracle = ScopedTypedefs::new();
    let parsed = parse_tu(&tu, &mut oracle);
    assert!(
        parsed.diagnostics.is_empty(),
        "{src}: {:?}",
        parsed.diagnostics
    );
    let init = parsed
        .ast
        .items()
        .iter()
        .find_map(|&id| match &parsed.ast.decl(id).kind {
            chiero_ast::DeclKind::Var { init: Some(i), .. } => Some(*i),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no initializer parsed for `{src}`"));
    (parsed, init)
}

pub fn gcc_available() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// **014 §7.** Put chiero's layout to gcc and let the compiler disagree.
///
/// Two halves, because they can check different things:
///
/// - `_Static_assert` on `sizeof`, `_Alignof` and `__builtin_offsetof` — compile-time, and
///   a mismatch is an error naming the exact field. `__builtin_offsetof` is ill-formed on
///   a bit-field, so those fields are skipped here.
/// - a **run-time bit probe** for bit-fields: write all-ones into one field, read the
///   object back as bytes, and compare against the mask chiero's `bit_offset`/`width`
///   predict. Without it, contract 5's straddling rules would be checked only against the
///   arithmetic that produced them, and a layout whose sizes are right while every bit
///   sits in the wrong place would pass.
pub fn assert_agrees_with_gcc(src: &str, tag: &str, l: &RecordLayout, p: &Parsed) {
    let kw = if l.is_union { "union" } else { "struct" };
    let mut prog = String::from("#include <string.h>\n#include <stdio.h>\n");
    prog.push_str(src);
    prog.push('\n');
    prog.push_str(&format!(
        "_Static_assert(sizeof({kw} {tag}) == {}, \"size\");\n",
        l.size
    ));
    prog.push_str(&format!(
        "_Static_assert(_Alignof({kw} {tag}) == {}, \"align\");\n",
        l.align
    ));
    for f in &l.fields {
        let Some(name) = f.name.and_then(|n| p.text(n)) else {
            continue;
        };
        if f.bits.is_some() {
            continue; // `__builtin_offsetof` is ill-formed on a bit-field.
        }
        prog.push_str(&format!(
            "_Static_assert(__builtin_offsetof({kw} {tag}, {name}) == {}, \"off {name}\");\n",
            f.offset
        ));
    }

    // The bit probe: one `main` per bit-field, each writing all-ones and dumping bytes.
    let bitfields: Vec<(String, u64, u64)> = l
        .fields
        .iter()
        .filter_map(|f| {
            let b = f.bits?;
            let name = f.name.and_then(|n| p.text(n))?;
            (b.width > 0).then_some((name, b.bit_offset, b.width))
        })
        .collect();

    prog.push_str("int main(void) {\n");
    for (name, off, width) in &bitfields {
        prog.push_str(&format!(
            "  {{ {kw} {tag} v; memset(&v, 0, sizeof v); v.{name} = ~0; \
             unsigned char b[sizeof v]; memcpy(b, &v, sizeof v); \
             printf(\"{name}\"); for (unsigned i = 0; i < sizeof v; i++) printf(\" %02x\", b[i]); \
             printf(\"\\n\"); (void){off}; (void){width}; }}\n"
        ));
    }
    prog.push_str("  return 0;\n}\n");

    // **A unique directory per invocation.** Keying it on the process id and the tag
    // collided: cargo runs tests in parallel and nearly every fixture here calls its
    // record `S`, so two probes wrote the same `probe.c` and ran the other's binary. The
    // failures were real gcc rejections of somebody else's layout, and every test passed
    // when run alone — the same shape as the process-global allocation counter that
    // wave-63 only caught because unrelated tests happened to run beside it.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("chiero-sema-{}-{}-{tag}", std::process::id(), seq));
    let _ = std::fs::create_dir_all(&dir);
    let c = dir.join("probe.c");
    let bin = dir.join("probe");
    std::fs::write(&c, &prog).expect("write probe");

    let out = std::process::Command::new("gcc")
        .args(["-std=gnu11", "-w", "-o"])
        .arg(&bin)
        .arg(&c)
        .output()
        .expect("run gcc");
    assert!(
        out.status.success(),
        "gcc rejected chiero's layout for `{kw} {tag}`.\n\
         chiero said size={} align={} fields={:?}\n\
         --- gcc ---\n{}\n--- program ---\n{prog}",
        l.size,
        l.align,
        l.fields
            .iter()
            .map(|f| (
                f.name.and_then(|n| p.text(n)).unwrap_or_default(),
                f.offset,
                f.bits
            ))
            .collect::<Vec<_>>(),
        String::from_utf8_lossy(&out.stderr)
    );

    if bitfields.is_empty() {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }

    let run = std::process::Command::new(&bin)
        .output()
        .expect("run probe");
    let text = String::from_utf8_lossy(&run.stdout);
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else { continue };
        let observed: Vec<u8> = parts
            .map(|h| u8::from_str_radix(h, 16).expect("hex byte"))
            .collect();
        let (_, off, width) = bitfields
            .iter()
            .find(|(n, _, _)| n == name)
            .expect("probe named a field we asked for");

        // What chiero's bit_offset/width predict, little-endian: bits [off, off+width).
        let mut expected = vec![0u8; observed.len()];
        for bit in *off..(*off + *width) {
            let byte = (bit / 8) as usize;
            if byte < expected.len() {
                expected[byte] |= 1 << (bit % 8);
            }
        }
        assert_eq!(
            observed, expected,
            "bit placement of `{kw} {tag}::{name}` disagrees with gcc.\n\
             chiero: bit_offset {off}, width {width} -> {expected:02x?}\n\
             gcc:                                      {observed:02x?}\n\
             --- program ---\n{prog}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
