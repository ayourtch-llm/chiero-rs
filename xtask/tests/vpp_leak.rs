//! 001 §4 rule 4 / contract 5: VPP knowledge lives only in `chiero-vpp`.

use std::path::Path;

fn crates_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("crates")
}

/// The gate itself: the real workspace is clean.
#[test]
fn workspace_has_no_vpp_leaks() {
    let leaks = xtask::vpp_leak::scan(&crates_dir()).expect("scan");
    assert!(
        leaks.is_empty(),
        "VPP identifiers outside chiero-vpp:\n{leaks:#?}"
    );
}

/// The gate detects a leak when there is one. Without this the gate could pass
/// forever by scanning nothing — the same vacuity that made the dependency gate's
/// workspace test meaningless before review.
#[test]
fn a_planted_leak_is_detected() {
    let tmp = std::env::temp_dir().join("chiero-vpp-leak-fixture");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("chiero-mem/src")).unwrap();
    std::fs::create_dir_all(tmp.join("chiero-vpp/src")).unwrap();

    std::fs::write(
        tmp.join("chiero-mem/src/lib.rs"),
        "fn f() { let n = vec_add1(v, x); }\n",
    )
    .unwrap();
    // The same identifier inside chiero-vpp is fine — that is the whole point.
    std::fs::write(
        tmp.join("chiero-vpp/src/lib.rs"),
        "fn g() { let n = vec_add1(v, x); }\n",
    )
    .unwrap();

    let leaks = xtask::vpp_leak::scan(&tmp).expect("scan");
    assert_eq!(
        leaks.len(),
        1,
        "expected exactly the chiero-mem leak: {leaks:#?}"
    );
    assert_eq!(leaks[0].marker, "vec_add1");
    assert_eq!(leaks[0].line, 1);
    assert!(leaks[0].file.to_string_lossy().contains("chiero-mem"));

    std::fs::remove_dir_all(&tmp).unwrap();
}

/// A comment naming VPP is not a leak. Every crate's docs legitimately explain why it
/// must not know about VPP, and flagging those would make the gate unusable.
#[test]
fn comments_are_exempt() {
    let tmp = std::env::temp_dir().join("chiero-vpp-leak-comments");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("chiero-mem/src")).unwrap();
    std::fs::write(
        tmp.join("chiero-mem/src/lib.rs"),
        "//! Must not know about vlib_ or clib_mem_alloc (001 §4 rule 4).\nfn f() {}\n",
    )
    .unwrap();
    assert!(xtask::vpp_leak::scan(&tmp).unwrap().is_empty());
    std::fs::remove_dir_all(&tmp).unwrap();
}
