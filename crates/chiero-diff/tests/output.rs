//! **031 §5: the two renderings, and what each is for.**
//!
//! > Deterministic ordering (by entity kind, then file, then name), stable across runs. Both a
//! > machine format (JSON, for 050) and a human rendering that **leads with the closure reason**.
//!
//! The human form is not decoration. §3 makes auditability a requirement — *"a maintainer who is
//! told to run 400 tests must be able to ask why"* — and a list of 400 entity names does not
//! answer that. What answers it is the *reason*, which is why §5's sketch puts the class and the
//! root on the first line of each group and the sites underneath.
//!
//! And `PARTIAL` gets its own line rather than a field somebody has to think to check. §4: *"a
//! tool that quietly narrows here is worse than no tool: it converts an unknown into a false
//! assurance."* A report that buries the gap does the same thing more slowly.

use chiero_diff::{Program, impact};

fn prog(src: &str) -> Program {
    Program::parse("f.c", src).expect("the fixture parses")
}

const CHAIN: &str = "static int leaf (int x) { return x + 1; }\n\
                     int middle (int x) { return leaf (x) * 2; }\n\
                     int top (int x) { return middle (x) + 1; }\n";

/// The human rendering leads with what changed and why, not with a list of names.
#[test]
fn the_human_rendering_leads_with_the_reason() {
    let edited = CHAIN.replace("return x + 1;", "return x + 2;");
    let text = impact(&prog(CHAIN), &prog(&edited)).render();

    let first = text.lines().next().expect("a report has a first line");
    assert!(
        first.contains("leaf") && first.contains("BodyChanged"),
        "the root and its class come first: {first:?}"
    );
    assert!(
        text.contains("middle") && text.contains("top"),
        "and the entities it reached are under it:\n{text}"
    );
    // Every reached entity says how it was reached.
    assert!(
        text.contains("Calls"),
        "an entity in the set without its edge is a name a maintainer cannot act on:\n{text}"
    );
}

/// **`PARTIAL` is a line, not a field.** §4 wants the gap reported prominently.
#[test]
fn partial_is_reported_on_its_own_line() {
    let broken = "int a (int x) { return x + ; }\n";
    let text = impact(&prog(CHAIN), &prog(broken)).render();
    assert!(
        text.lines().any(|l| l.starts_with("PARTIAL")),
        "a reader must not have to go looking for it:\n{text}"
    );
    assert!(
        text.contains("f.c"),
        "and it names the file that could not be read:\n{text}"
    );
}

/// A complete, empty answer says so. "Nothing to run" is the most consequential thing this tool
/// can say, and a blank report is indistinguishable from a crash.
#[test]
fn an_empty_complete_answer_says_so() {
    let p = prog(CHAIN);
    let text = impact(&p, &p).render();
    assert!(
        text.contains("no impact") || text.contains("No impact"),
        "an empty report and a blank one are different things:\n{text}"
    );
    assert!(!text.contains("PARTIAL"));
}

/// **The machine format is deterministic**, which is what makes two runs comparable and a report
/// diffable — 001 §5's rule, arriving at the surface a tool actually consumes.
#[test]
fn the_json_is_stable_across_runs() {
    let edited = CHAIN.replace("return x + 1;", "return x + 2;");
    let a = impact(&prog(CHAIN), &prog(&edited)).to_json();
    let b = impact(&prog(CHAIN), &prog(&edited)).to_json();
    assert_eq!(a, b);
    assert!(a.starts_with('{'), "an object, so fields can be added: {a:.60}");
}

/// The machine format carries what 050 needs: the entity, the class, the root, the edges, the
/// distance, and the completeness.
#[test]
fn the_json_carries_the_justification() {
    let edited = CHAIN.replace("return x + 1;", "return x + 2;");
    let json = impact(&prog(CHAIN), &prog(&edited)).to_json();
    for needle in [
        "\"entities\"",
        "\"completeness\"",
        "\"class\"",
        "\"root\"",
        "\"edges\"",
        "\"distance\"",
        "BodyChanged",
        "middle",
    ] {
        assert!(json.contains(needle), "missing {needle} in:\n{json}");
    }
}

/// And it is valid JSON, not a string that looks like it.
#[test]
fn the_json_parses() {
    let edited = CHAIN.replace("return x + 1;", "return x + 2;");
    let json = impact(&prog(CHAIN), &prog(&edited)).to_json();
    let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert!(v["entities"].is_array());
    assert_eq!(v["completeness"]["kind"], "Complete");
}

/// A name with a quote or a backslash in it does not break the document. Entity names come from
/// source text, and a file path on some machine will eventually contain one.
#[test]
fn the_json_escapes_what_it_must() {
    let src = "int a (int x) { return x; }\n";
    let edited = "int a (int x) { return x + 1; }\n";
    let mut set = impact(
        &Program::parse("we\"ird\\path.c", src).expect("parses"),
        &Program::parse("we\"ird\\path.c", edited).expect("parses"),
    );
    assert!(!set.entities.is_empty());
    let json = set.to_json();
    let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON despite the name");
    assert_eq!(v["entities"][0]["file"], "we\"ird\\path.c");
    set.entities.clear();
}
