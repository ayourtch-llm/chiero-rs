//! **A witness a reader cannot read is not a witness** — 023 §9, which calls it *a concrete input
//! someone can re-run*.
//!
//! Measured on VPP: `find-bugs` on `plugins/nsh/nsh_node.c --entry nsh_md2_encap` emits **950 KB
//! of JSON for one finding**, whose witness is **10 658 bindings, 10 657 of them the same
//! anonymous "a lazily-materialized byte"**. Under UCSE an entry that walks a packet buffer
//! materialises a byte at a time, so the *execution* is fine and the *reporting* is not: ten
//! thousand unnamed bytes cannot be read, cannot be typed into a harness, and are most of both
//! the runtime and the output.
//!
//! **The fix is not "print fewer".** A quietly shortened witness reads as the whole input and is
//! worse than a long one, and — measured below — the bindings that matter are at the *end*: the
//! path pins the divisor, and the thousands before it are the bytes the walk happened to touch.
//! So the rule has three parts: bound the list, keep what the path pinned, and say what was left
//! out and of what kind.
//!
//! ⚠️ **The fixture here is the one four earlier attempts missed.** They all reached for
//! `copymem`, which forks on the aliasing check between an alloca and a lazy object — and the
//! finding then lands on the *mint-free* fork, so the witness came out empty. Straight-line loads
//! at distinct offsets through the entry pointer mint one byte each, do not fork, and put the
//! fault after the mints: n loads, n + 3 bindings, ~96 JSON bytes each, linear all the way up.

use chiero_cir::Module;
use chiero_tool::find_bugs;

fn m(body: &str) -> Module {
    chiero_cir::text::parse(&format!("target x86_64-unknown-linux-gnu\n\n{body}\n"))
        .unwrap_or_else(|e| panic!("fixture does not parse: {e:?}\n{body}"))
}

/// `n` byte-loads through the entry pointer, then a division by the last one.
///
/// The division is what makes a finding, and it is what pins a binding: the last four bindings
/// are the divisor's bytes, constrained to zero. Everything before them is unpinned noise.
fn many_loads(n: usize) -> String {
    let mut b = String::from("func @f(%0: ptr) -> i32 {\nentry:\n  .line 1\n");
    let (mut v, mut last) = (1, 0);
    for i in 0..n {
        b.push_str(&format!("  %{v} = ptradd %0, {i}i64\n"));
        let p = v;
        v += 1;
        b.push_str(&format!("  %{v} = load i32, %{p} align 1\n"));
        last = v;
        v += 1;
    }
    b.push_str(&format!(
        "  %{v} = sdiv i32 100i32, %{last}\n  ret %{v}\n}}"
    ));
    b
}

fn finding(n: usize) -> serde_json::Value {
    let mut cfg = chiero_tool::BugCfg::new("f");
    cfg.entry_ptr_nonnull = true;
    let env = find_bugs(&m(&many_loads(n)), &cfg);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    let mut f = v["result"]["findings"]
        .as_array()
        .and_then(|a| a.first().cloned())
        .unwrap_or_else(|| panic!("no finding to inspect: {v}"));
    // The blind spots live on the envelope, and what the report says about the omission is as
    // much the contract as the omission itself.
    f["__blind_spots"] = serde_json::json!(env.blind_spots);
    f
}

/// **Bounded, and it says so.**
#[test]
fn a_witness_too_long_to_read_is_rendered_in_part_and_names_the_rest() {
    let f = finding(200);
    let shown = f["witness"].as_array().expect("a witness").len();
    assert!(
        shown <= 64,
        "203 bindings were rendered as {shown}; at VPP's 10 658 that is a megabyte of JSON for \
         one finding"
    );
    let omitted = f["witness_omitted"]["count"].as_u64().unwrap_or(0);
    assert_eq!(
        shown as u64 + omitted,
        203,
        "every binding is either shown or counted as omitted: {}",
        f["witness_omitted"]
    );
    let kinds = f["witness_omitted"]["kinds"]
        .as_array()
        .expect("what was left out, by kind");
    assert_eq!(
        kinds[0][0].as_str(),
        Some("a lazily-materialized byte"),
        "a reader must be told what the omitted inputs *were*: {kinds:?}"
    );
    assert!(
        f["__blind_spots"].as_array().is_some_and(|b| b
            .iter()
            .any(|s| s.as_str().is_some_and(|s| s.contains("witness")))),
        "a partial witness is something the report is silent about, so the envelope says it: {}",
        f["__blind_spots"]
    );
}

/// **And it keeps the bindings the path pinned** — which are the *last* four here.
///
/// Measured before the rule was chosen: at n = 8, 40 and 200 the pinned bindings are always the
/// final four, the divisor's bytes. "Show the first 64" would therefore drop every value the
/// finding depends on and keep 64 that it does not — a witness nobody can re-run, rendered
/// confidently.
#[test]
fn the_bindings_the_path_pinned_survive_the_bound() {
    let f = finding(200);
    let w = f["witness"].as_array().expect("a witness");
    let pinned = w.iter().filter(|b| b["pinned"] == true).count();
    assert_eq!(
        pinned, 4,
        "the four pinned bytes of the divisor are the whole reason this finding reproduces: {w:?}"
    );
}

/// A witness that fits is unchanged — no truncation field, no blind spot, no reordering.
#[test]
fn a_short_witness_is_rendered_whole_and_says_nothing_about_omission() {
    let f = finding(8);
    assert_eq!(f["witness"].as_array().map(Vec::len), Some(11));
    assert!(
        f["witness_omitted"].is_null(),
        "nothing was left out, so there is nothing to report: {}",
        f["witness_omitted"]
    );
}

/// **The same rule at the other site that renders a witness.**
///
/// `check_reachable` solves the path condition itself — the engine attaches a witness to a state
/// carrying a *finding*, and a state that merely arrived somewhere has none — so it builds its
/// bindings by a different route and had its own unbounded rendering. Under UCSE it reaches that
/// line through the same lazily-materialized bytes.
///
/// §7.2's lesson, which this project has paid for twice: when a review finds a defect, the
/// question is what rule it violates, not what line to change. Fixing only the site that was
/// measured is how two earlier defects came back through a different door.
#[test]
fn check_reachable_bounds_its_witness_the_same_way() {
    // ⚠️ **Not the load fixture.** Measured: `check_reachable` builds its bindings from
    // `State::inputs()`, and the lazily-materialized bytes of the loads above are not among
    // them — that witness comes out *empty*. Parameters are, so this is what exercises the
    // bound at this site. Writing the test against the other fixture would have asserted a
    // bound over an empty list and passed while proving nothing.
    let params: Vec<String> = (0..100).map(|i| format!("%{i}: i32")).collect();
    let body = format!(
        "func @f({}) -> i32 {{\nentry:\n  .line 1\n  %100 = add i32 %0, %1\n  .line 2\n  ret %100\n}}",
        params.join(", ")
    );
    let env = chiero_tool::check_reachable(&m(&body), &chiero_tool::BugCfg::new("f"), 2);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    let shown = v["result"]["witness"]
        .as_array()
        .unwrap_or_else(|| panic!("no witness to inspect: {v}"))
        .len();
    assert_eq!(
        shown, 64,
        "100 inputs, and the bound is not a property of one operation: {}",
        v["result"]
    );
    assert_eq!(
        v["result"]["witness_omitted"]["count"].as_u64(),
        Some(36),
        "and it says what it left out: {}",
        v["result"]
    );
}

/// **A bounded rendering must not become a bounded *check*.**
///
/// `check_reachable` licenses `proven` partly on "a solver pinned every input". That test now
/// runs over the whole witness while the report shows 64 of it — because computing it from the
/// rendering would let an unpinned binding past the bound turn an unproven arrival into a proof.
/// 100 pinned parameters is the case where the two readings agree; the assertion is that the
/// verdict is still the strong one after truncation, which is what says the check did not shrink
/// with the report.
#[test]
fn truncating_the_report_does_not_truncate_the_proof_condition() {
    let params: Vec<String> = (0..100).map(|i| format!("%{i}: i32")).collect();
    let body = format!(
        "func @f({}) -> i32 {{\nentry:\n  .line 1\n  %100 = add i32 %0, %1\n  .line 2\n  ret %100\n}}",
        params.join(", ")
    );
    let env = chiero_tool::check_reachable(&m(&body), &chiero_tool::BugCfg::new("f"), 2);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(v["result"]["verdict"].as_str(), Some("reachable"));
    assert!(
        env.proven,
        "every one of the 100 inputs is pinned, and only 64 are printed: {v}"
    );
}
