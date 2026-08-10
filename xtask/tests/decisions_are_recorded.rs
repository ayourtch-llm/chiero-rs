//! **Every contract nobody can meet is written down as a decision, not left uncited.**
//!
//! `xtask contract-coverage` reports uncited contracts; a reader still has to notice and act.
//! On 2026-08-10 one had been uncited for weeks — 011 c12, a throughput floor with no sound
//! instrument — and it was missing from HANDOFF's decision block because that block had been
//! assembled by grepping for phrasings (`owner's call`) and c12's entry says *"needs the
//! owner"*. **A pattern narrower than its subject**, which is the session's most repeated
//! failure; the fix is never a better pattern but a source that *enumerates*.
//!
//! So: an uncited contract must appear in the decision block by name. Either it gains a test
//! and leaves the report, or it is a question for the owner and is on the list where questions
//! live. Silently uncited is the third state this forbids.

use std::path::Path;

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn every_uncited_contract_is_named_in_the_decision_block() {
    let root = root();
    let cov = xtask::contracts::measure(&root).expect("measure contracts");
    let handoff = std::fs::read_to_string(root.join("HANDOFF.md")).expect("read HANDOFF.md");

    // The block is delimited by its own heading and START HERE; reading only it means a
    // mention buried elsewhere in 3800 lines does not count as "recorded as a decision".
    let block = handoff
        .split_once("DECISIONS WAITING ON THE OWNER")
        .and_then(|(_, r)| r.split_once("START HERE"))
        .map(|(b, _)| b.to_string())
        .expect("HANDOFF.md has a decision block");

    let mut missing = Vec::new();
    for doc in xtask::contracts::M1_DOCS
        .iter()
        .chain(xtask::contracts::M2_DOCS)
    {
        for c in cov.uncovered(doc) {
            // `011` + `12` in either spelling the file uses.
            let a = format!("{doc} c{c}");
            let b = format!("{doc} contract {c}");
            if !block.contains(&a) && !block.contains(&b) {
                missing.push(format!("{doc} contract {c}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "uncited contracts that are not on the decision list: {missing:?}\n\
         Either write the test, or add it to the block with what it costs and a \
         recommendation — an uncited contract nobody has decided about is the state this \
         gate exists to forbid."
    );
}
