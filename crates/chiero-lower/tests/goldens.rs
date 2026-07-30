//! Covers: 015 contract 2.
//!
//! 015 §1 says every construct lowers to a **fixed shape**, and a golden file is the only
//! thing that actually holds it fixed. Two conforming implementations can disagree about
//! block order or where a `SeqPoint` sits and both compute the right answer — the shape
//! tests in `shapes.rs` check the properties someone thought to name, and a golden checks
//! everything else, including the parts nobody thought to name.
//!
//! **Regenerate with `CHIERO_BLESS=1 cargo test -p chiero-lower --test goldens`.** Read
//! the diff before blessing: a golden that changes for a reason nobody can state is a
//! shape that was never fixed.

mod harness;
use harness::{lower, print};

fn golden_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/<name>/ has a workspace root above it")
        .join("tests/corpus/lowered")
}

/// Lower `src` twice, require the two runs to be byte-identical, and compare against the
/// checked-in golden.
fn check_golden(name: &str, src: &str) {
    let once = print(&lower(src));
    let twice = print(&lower(src));
    // Contract 21's determinism, asserted here too: a golden compared against a
    // nondeterministic printer would fail at random and teach everyone to re-bless.
    assert_eq!(
        once, twice,
        "`{name}` lowers differently on two runs, so no golden of it can mean anything"
    );

    let path = golden_dir().join(format!("{name}.cir"));
    if std::env::var("CHIERO_BLESS").is_ok() {
        std::fs::create_dir_all(golden_dir()).expect("golden dir");
        std::fs::write(&path, &once).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "no golden at {}. Generate it with \
             `CHIERO_BLESS=1 cargo test -p chiero-lower --test goldens`, then read the \
             diff before committing it.",
            path.display()
        )
    });
    assert_eq!(
        once, expected,
        "`{name}` no longer matches its golden. If the change is intended, say why in the \
         commit and re-bless; if it is not, the shape moved for a reason nobody chose."
    );
}

/// **Contract 2.** `a || b`, `a ? b : c` and the nested `a && b || c` each lower to a
/// fixed shape.
///
/// The nested case is the one that earns its place: `&&` binds tighter than `||`, so
/// `a && b || c` is `(a && b) || c` and its lowering nests one four-block shape inside
/// another's left operand. Block numbering, slot allocation order and marker placement all
/// interact there, and no property test in `shapes.rs` pins any of it.
#[test]
fn the_short_circuit_shapes_match_their_goldens() {
    check_golden("or", "int f(int a, int b) { return a || b; }");
    check_golden("cond", "int f(int a, int b, int c) { return a ? b : c; }");
    check_golden(
        "and_or",
        "int f(int a, int b, int c) { return a && b || c; }",
    );
}

/// A golden for each of the constructs later waves are most likely to disturb.
///
/// Not a numbered contract. These exist because a golden's value is proportional to how
/// much walks over it: 015 §1's "fixed shape" claim covers every construct, and a change
/// to scope markers or block ordering that breaks `switch` should be visible in a diff
/// rather than in a test three waves later.
#[test]
fn the_statement_shapes_match_their_goldens() {
    check_golden(
        "loops",
        "int f(int n) { int t = 0; for (int i = 0; i < n; i++) { t += i; } return t; }",
    );
    check_golden(
        "switch",
        "int f(int n) { int t = 0; switch (n) { case 1: t = 1; case 2: t += 2; break; \
         default: t = 9; } return t; }",
    );
    check_golden(
        "scopes",
        "int f(int n) { { int a = 1; { int b = 2; if (b) goto out; } } out: return n; }",
    );
    check_golden(
        "aggregate",
        "struct S { int a; char b; }; int f(void) { struct S x; struct S y; x.a = 1; \
         y = x; return y.a; }",
    );
}

/// The goldens are **not empty**, and they are CIR the parser accepts.
///
/// A golden that is an empty string compares equal to itself forever. And a golden that
/// the `.cir` parser cannot read is not "the same language" as M1's hand-written
/// fixtures, which is the whole point of contract 22 — so the round trip is checked here
/// even though the corpus half of it is not reachable yet.
#[test]
fn every_golden_is_non_empty_and_reparses() {
    let dir = golden_dir();
    if std::env::var("CHIERO_BLESS").is_ok() {
        return;
    }
    let entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|_| panic!("no golden directory at {}", dir.display()))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "cir"))
        .collect();
    assert!(
        entries.len() >= 7,
        "the goldens exist: found {} in {}",
        entries.len(),
        dir.display()
    );
    for e in entries {
        let text = std::fs::read_to_string(e.path()).expect("read");
        assert!(
            text.lines().count() > 3,
            "{} is too small to be a lowered function",
            e.path().display()
        );
        let m = chiero_cir::text::parse(&text)
            .unwrap_or_else(|err| panic!("{} does not reparse: {err:?}", e.path().display()));
        let errs = chiero_cir::verify::verify(&m);
        assert!(
            errs.is_empty(),
            "{} parses but does not verify: {errs:#?}",
            e.path().display()
        );
    }
}

/// **Contract 22.** For every corpus C file, `lower(parse(f))` printed as text equals the
/// checked-in `.cir` golden.
///
/// This is the round trip that makes M1's hand-written fixtures and M2's real lowering
/// **the same language**. The corpus files are the ones 024's harness intrinsics were
/// written for — they call `chiero_make_symbolic`, `chiero_assume` and `chiero_assert`,
/// which lowering emits as ordinary calls to `Body::Declared` functions and 024's model
/// registry resolves by name.
#[test]
fn every_corpus_c_file_matches_its_lowered_golden() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root");
    let src_dir = root.join("tests/corpus/c");
    let include = root.join("include");
    if harness::gcc_system_paths().is_empty() {
        eprintln!("skipping: no gcc system include path (015 contract 22)");
        return;
    }

    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&src_dir)
        .expect("the C corpus exists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "c"))
        .collect();
    // Sorted: `read_dir` order is the filesystem's, and a test that walks it in a
    // different order on another machine is a different test.
    files.sort();
    assert!(
        files.len() >= 4,
        "the corpus has files: found {} in {}",
        files.len(),
        src_dir.display()
    );

    for path in files {
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        let (m, _) = harness::lower_file(&path, std::slice::from_ref(&include));

        // Every corpus module must **verify**. A golden of invalid CIR would freeze a
        // shape the rest of the system rejects.
        let errs = chiero_cir::verify::verify(&m);
        assert!(errs.is_empty(), "{stem} does not verify: {errs:#?}");

        let text = print(&m);
        let golden = golden_dir().join(format!("corpus_{stem}.cir"));
        if std::env::var("CHIERO_BLESS").is_ok() {
            std::fs::create_dir_all(golden_dir()).expect("golden dir");
            std::fs::write(&golden, &text).expect("write");
            continue;
        }
        let expected = std::fs::read_to_string(&golden).unwrap_or_else(|_| {
            panic!(
                "no golden at {}. `CHIERO_BLESS=1 cargo test -p chiero-lower --test \
                 goldens` writes it; read the diff first.",
                golden.display()
            )
        });
        assert_eq!(text, expected, "{stem} no longer matches its golden");

        // **The round trip is the contract, not the file compare.** A golden that the
        // `.cir` parser cannot read would still compare equal to itself forever, and
        // would not be the same language as M1's hand-written fixtures.
        let back = chiero_cir::text::parse(&text)
            .unwrap_or_else(|e| panic!("{stem}'s golden does not reparse: {e:?}"));
        assert_eq!(
            print(&back),
            text,
            "{stem} does not survive a print/parse/print round trip"
        );
    }
}

/// **The corpus has to contain the shapes the goldens are supposed to certify.**
///
/// A golden quantifies over the corpus, so a construct no corpus file contains is a construct no
/// golden holds fixed — and the goldens are what `shapes.rs` explicitly does *not* cover ("the parts
/// nobody thought to name"). A review recorded in §9 read all thirteen files and found the whole
/// corpus has **no pointer-typed global, no struct returned by value, no struct passed by value and
/// no local array decaying to a pointer**. Six defects passed 1102 tests that way: every changed site
/// was unreachable from the corpus.
///
/// This asserts the inventory rather than a golden's bytes, because the bytes are the mechanism and
/// the coverage is the claim. It is textual and crude on purpose — the same trade the generator's
/// shape counts make — and it fails loudly when a shape leaves rather than quietly certifying
/// nothing.
///
/// (Wave 226 note, since §9 said to re-diagnose this item: it has nothing to do with `->`, which is
/// already in `struct_walk.c` and `packed_header.c` and agrees with gcc on five hand-written shapes.
/// The gap is *value* semantics for aggregates and pointer-typed storage at file scope.)
#[test]
fn the_c_corpus_exercises_pointer_and_aggregate_shapes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root");
    let dir = root.join("tests/corpus/c");
    let mut all = String::new();
    for e in std::fs::read_dir(&dir).expect("the C corpus exists") {
        let p = e.expect("entry").path();
        if p.extension().is_some_and(|x| x == "c") {
            all.push_str(&std::fs::read_to_string(&p).expect("readable"));
            all.push('\n');
        }
    }
    // Each probe is a construct, with the reason it is not covered by the others.
    let wanted: &[(&str, &str)] = &[
        // A pointer-typed *global*: `AddrOfGlobal` plus a store, which wave 132 broke and
        // wave 189 broke again in a different way (`GlobalInit` losing every address form).
        ("a pointer-typed global", "*gp ="),
        // Reading through it, which is the half that reads as null when the initializer is lost.
        ("a load through a pointer global", "= *gp"),
        // A struct **returned** by value: 020 has no aggregate values, so this is an sret slot,
        // and waves 126-132 spent five waves on it.
        ("a struct returned by value", "struct pair make_"),
        // A struct **passed** by value, which is a copy the caller makes.
        ("a struct passed by value", "(struct pair "),
        // A local array decaying to a pointer, distinct from a global array's decay.
        ("a local array decaying", "= local_arr"),
    ];
    let missing: Vec<&str> = wanted
        .iter()
        .filter(|(_, pat)| !all.contains(*pat))
        .map(|(what, _)| *what)
        .collect();
    assert!(
        missing.is_empty(),
        "the goldens quantify over this corpus, so these shapes are held fixed by nothing: \
         {missing:?}"
    );
}
