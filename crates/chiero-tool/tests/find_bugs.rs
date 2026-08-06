//! **050 contract 3 — `find_bugs`, and the rendering that may never be read as "all clear".**
//!
//! > 3. A `find_bugs` run that hits a budget returns `proven: false`, a non-empty
//! >    `budgets.hit`, and text containing "within"; the string "no defects found" never
//! >    appears unqualified in any rendering.
//!
//! This is the operation 050 §2's whole argument is about. Every other one answers a question
//! whose empty answer is merely uninformative; this one's empty answer is *"your code is
//! fine"*, and it is wrong exactly when the search did not finish.
//!
//! The three cases below are the three an empty finding list can mean, and the envelope must
//! keep them apart:
//!
//! | the run | what the empty list means |
//! |---|---|
//! | finished, `Exact` | nothing here — the strongest thing chiero can say |
//! | hit a budget | nothing *within the bound*, and nothing at all about beyond it |
//! | could not model something | nothing chiero could see |

use chiero_cir::Module;
use chiero_tool::{Fidelity, find_bugs};

fn m(body: &str) -> Module {
    chiero_cir::text::parse(&format!("target x86_64-unknown-linux-gnu\n\n{body}\n"))
        .unwrap_or_else(|e| panic!("fixture does not parse: {e:?}\n{body}"))
}

fn cfg(entry: &str) -> chiero_tool::BugCfg {
    chiero_tool::BugCfg::new(entry)
}

/// `int f (int x) { return x + 1; }` — nothing wrong, and nothing in the way of saying so.
const CLEAN: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  %1 = add i32 %0, 1i32
  ret %1
}";

/// **A clean exhaustive run is the one place an empty list is a real answer.**
///
/// Contract 4b's requirement, for this operation: an implementation that always answers
/// `proven: false` satisfies every other contract here and can never license a negative claim.
#[test]
fn a_finished_run_with_no_findings_is_proven_empty() {
    let env = find_bugs(&m(CLEAN), &cfg("f"));
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(v["result"]["findings"].as_array().map(Vec::len), Some(0));
    assert_eq!(env.fidelity, Fidelity::Exact);
    assert!(
        env.proven,
        "an exhaustive search that found nothing has found nothing: {v}"
    );
    assert_eq!(
        v["result"]["budgets"]["hit"].as_array().map(Vec::len),
        Some(0),
        "nothing was cut: {v}"
    );
}

/// A loop whose trip count is an input, so chiero's bound decides where the search stops.
const UNBOUNDED: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  goto bb1
bb1:
  .line 2
  %1 = phi i32 [entry 0i32] [bb1 %2]
  %2 = add i32 %1, 1i32
  %3 = cmp slt i32 %2, %0
  br %3, bb1, bb2
bb2:
  .line 3
  ret %1
}";

/// **Contract 3, in full.**
#[test]
fn a_run_that_hit_a_budget_says_which_and_is_not_proven() {
    let env = find_bugs(&m(UNBOUNDED), &cfg("f"));
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");

    assert!(!env.proven, "a truncated search proves nothing: {v}");
    let hit = v["result"]["budgets"]["hit"]
        .as_array()
        .expect("budgets.hit");
    assert!(
        !hit.is_empty(),
        "the budget that stopped the search must be named: {v}"
    );
    assert!(
        hit.iter()
            .any(|b| b.as_str().is_some_and(|s| s.contains("max_loop_iters"))),
        "and named specifically, not as `a budget`: {hit:?}"
    );
    assert!(
        env.render().contains("within"),
        "the rendering must say the answer is within a bound:\n{}",
        env.render()
    );
}

/// **The string a reader must never see unqualified**, over every case.
///
/// 050 §2 names this failure directly: *"an LLM reading `findings: []` will report 'the code
/// is safe'"*. The wording of the empty case is therefore part of the contract, not a
/// presentation detail.
#[test]
fn no_rendering_says_no_defects_found_bare() {
    for (what, src, entry) in [
        ("clean", CLEAN, "f"),
        ("budget-cut", UNBOUNDED, "f"),
        ("missing entry", CLEAN, "nosuch"),
    ] {
        let env = find_bugs(&m(src), &cfg(entry));
        let r = env.render();
        let bare = r.contains("no defects found") && !r.contains("within") && !env.proven;
        assert!(!bare, "{what}: an unqualified all-clear:\n{r}");
        // And an unproven one always carries something to read.
        if !env.proven {
            assert!(
                !env.blind_spots.is_empty() || !env.assumptions.is_empty(),
                "{what}: proven false and nothing said about why:\n{r}"
            );
        }
    }
}

/// **A real defect is reported with the input that reaches it.**
///
/// Without this the operation could satisfy every contract above by finding nothing, ever.
#[test]
fn a_signed_overflow_is_found_and_witnessed() {
    // `int f (int x) { return x + 1; }` overflows at INT_MAX — but only the *maybe* kind,
    // since it depends on the input. `INT_MAX + 1` written as constants is definite.
    let definite = "\
func @f() -> i32 {
entry:
  .line 1
  %0 = add i32 2147483647i32, 1i32 signed
  ret %0
}";
    let env = find_bugs(&m(definite), &cfg("f"));
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    let findings = v["result"]["findings"].as_array().expect("findings");
    assert!(
        !findings.is_empty(),
        "signed overflow of INT_MAX + 1 is a defect: {v}"
    );
    assert!(
        findings[0]["message"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "a finding with no message is not a finding: {v}"
    );
    // 023 contract 15: a witness, or a recorded reason there is none. Never silence.
    assert!(
        !findings[0]["witness"].is_null() || !findings[0]["unwitnessed"].is_null(),
        "the absence is allowed; the silence is not: {v}"
    );
}

/// **An entry that is not there is an error, not a clean bill of health.**
#[test]
fn a_missing_entry_is_not_an_empty_finding_list() {
    let env = find_bugs(&m(CLEAN), &cfg("nosuch"));
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert!(!env.proven, "nothing was analysed: {v}");
    assert!(
        v["result"]["error"]
            .as_str()
            .is_some_and(|e| e.contains("nosuch")),
        "the error must name what was not found: {v}"
    );
}

/// A division by zero after a loop: the engine reports it once per unrolled iteration, because
/// 023 §6.1 is explicit that those are separate reports of one bug and it will not merge them.
const DIV_AFTER_LOOP: &str = "\
func @f(%0: i32) -> i32 {
entry:
  .line 1
  goto bb1
bb1:
  .line 2
  %1 = phi i32 [entry 0i32] [bb1 %2]
  %2 = add i32 %1, 1i32
  %3 = cmp slt i32 %2, %0
  br %3, bb1, bb2
bb2:
  .line 3
  %4 = sub i32 %0, %0
  %5 = sdiv i32 %0, %4
  ret %5
}";

/// **One bug is one entry, and the count of paths that reached it is not thrown away.**
///
/// Nine identical `division-by-zero` lines differing only in which loop iteration produced
/// them is a worse answer than one line saying nine — a reader scrolling past eight
/// near-duplicates is a reader who stops reading. But collapsing them *silently* would be the
/// other failure: "1 finding" and "1 finding seen on 9 paths" are different facts, and the
/// second is what tells a reader the loop matters.
#[test]
fn identical_findings_are_one_entry_that_says_how_many_paths_reached_it() {
    let env = find_bugs(&m(DIV_AFTER_LOOP), &cfg("f"));
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    let findings = v["result"]["findings"].as_array().expect("findings");
    assert_eq!(
        findings.len(),
        1,
        "one bug, one entry — got {}: {v}",
        findings.len()
    );
    assert!(
        findings[0]["paths"].as_u64().is_some_and(|n| n > 1),
        "and the paths that reached it are counted, not discarded: {v}"
    );
}

/// **A caller must be able to say the entry's pointers are not null.**
///
/// Measured over 40 VPP functions: 36 analysed, **178 findings, none of them `Exact`**. Every
/// one rested on an unconstrained entry — "`%N` is a pointer parameter assumed to be possibly
/// null" — which is a statement about the *caller contract*, not about the function. A reader
/// cannot act on any of them, and there is no way to say otherwise.
///
/// The engine has the knob and its own documentation says why:
///
/// > For a caller that is known to check — an internal helper reached only through a guarded
/// > path — the null state is a path the program does not have, and every dereference in it is
/// > a finding nobody can act on.
///
/// **It is an assumption, so the envelope must carry it.** Turning off a real path to get a
/// quieter answer is exactly the trade 050 §2 exists to make visible.
#[test]
fn entry_pointers_can_be_assumed_non_null_and_the_envelope_says_so() {
    let deref = "\
func @f(%0: ptr) -> i32 {
entry:
  .line 1
  %1 = load i32, %0 align 4
  ret %1
}";
    // The default: a null pointer parameter is a path, and the finding is real for a caller
    // that does not check.
    let loose = find_bugs(&m(deref), &cfg("f"));
    let lv: serde_json::Value = serde_json::from_str(&loose.to_json()).expect("valid JSON");
    assert!(
        lv["result"]["findings"]
            .as_array()
            .is_some_and(|a| !a.is_empty()),
        "an unconstrained pointer parameter may be null: {lv}"
    );

    // Told the caller checks, that path is gone.
    let mut tight = cfg("f");
    tight.entry_ptr_nonnull = true;
    let env = find_bugs(&m(deref), &tight);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(
        v["result"]["findings"].as_array().map(Vec::len),
        Some(0),
        "the null path was assumed away: {v}"
    );
    assert!(
        env.assumptions
            .iter()
            .any(|(k, _)| k == "entry_ptr_nonnull"),
        "and an assumption that removes a real path must be in the envelope: {:?}",
        env.assumptions
    );
}

/// **The one `Exact` finding chiero produced on VPP was wrong, and said `proven: true`.**
///
/// Measured over 40 VPP entry points: 231 findings, exactly one of them `Exact` —
/// `_vec_update_len` in `vppinfra/vec.c`:
///
/// ```text
/// out-of-bounds: 4-byte access at offset -8 of the 4096-byte object reached through
/// an unconstrained pointer
/// proven — this holds for all inputs (Exact)
/// ```
///
/// That access is `_vec_find(v)->len = n_elts`, and `_vec_find(v)` is `((vec_header_t *)(v) - 1)`.
/// **Every VPP vector is an interior pointer by design**; the header lives behind the data. The
/// finding is a false positive, and it is the worst kind this project can emit: `Exact` means
/// `proven: true`, which is chiero's strongest claim.
///
/// The cause is two inventions of chiero's, neither of them a fact about the program:
///
/// - the object an entry pointer parameter points at is `ENTRY_PARAM_BYTES` (4096) big, and
///   `ENTRY_PARAM_BYTES`'s own doc comment says "the caller is outside the analysis, so there
///   is no right answer — this is a *bound chiero chose*";
/// - the pointer is `Pointer { base: obj, off: 0 }`, so it is assumed to point at that
///   object's **base**, when an unconstrained pointer is exactly one that may point anywhere
///   inside a larger one.
///
/// The message even contains the contradiction: a pointer cannot be both "unconstrained" and
/// known to sit at offset 0 of a 4096-byte object.
///
/// **The rule, not the site**: a bounds fault decided by a lazily-materialized object's extent
/// is decided by a number chiero picked, so it can never be `Exact`, and the degradation has to
/// name its cause the way 023 §7 rule 3 requires of every other one.
#[test]
fn a_bounds_fault_against_an_invented_object_is_never_proven() {
    // `_vec_find(v)->len = n_elts` exactly: a **store** through an interior pointer.
    //
    // A load here would prove nothing — its result is invented, so the path degrades for an
    // unrelated reason and the test passes without the rule existing. That is what the first
    // version of this fixture did.
    let interior = "\
func @f(%0: ptr, %1: i32) -> void {
entry:
  .line 1
  %2 = ptradd %0, -8i64
  store i32 %1 -> %2 align 4
  ret
}";
    let mut c = cfg("f");
    c.entry_ptr_nonnull = true; // isolate the bounds question from the null one
    // These are not shown by default any more — see the test below. This one is about what
    // the finding may *claim* when it is shown, which is a separate question and stays one:
    // suppressing a report is a decision about a reader's attention, and it must not be the
    // thing that stops a wrong `proven: true` from being possible.
    c.report_invented_bounds = true;
    let env = find_bugs(&m(interior), &c);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");

    let findings = v["result"]["findings"]
        .as_array()
        .expect("a findings array")
        .clone();
    let oob: Vec<_> = findings
        .iter()
        .filter(|f| {
            f["message"]
                .as_str()
                .is_some_and(|m| m.contains("out-of-bounds") || m.contains("outside"))
        })
        .collect();
    assert!(!oob.is_empty(), "the access is still worth reporting: {v}");
    for f in &oob {
        assert_ne!(
            f["fidelity"], "Exact",
            "the object's extent and the pointer's base are chiero's, not the program's, \
             so this cannot be proven: {f}"
        );
    }
    assert_eq!(
        v["proven"], false,
        "and the run that produced it cannot claim proof either: {v}"
    );
    assert!(
        env.assumptions.iter().any(|(_, d)| d.contains("4096")),
        "023 §7 rule 3: the degradation names its cause, including the number chiero chose: {:?}",
        env.assumptions
    );
}

/// **147 of 157 findings on VPP were about a number chiero picked.**
///
/// With the null case assumed away, the 40-entry-point VPP measurement leaves 157 findings.
/// By kind:
///
/// | | |
/// |---|---|
/// | `pointer-outside-object`, against the invented object | 113 |
/// | `out-of-bounds`, against the invented object | 34 |
/// | `uninitialized-read` | 9 |
/// | `division-by-zero` | 1 |
///
/// **94% of the output is one artifact.** The bound crossed is `ENTRY_PARAM_BYTES`, and chiero
/// has no information about the caller's real object at all: not its size, and not where in it
/// the pointer points. So such a fault says nothing whatever about the program — an access at
/// offset -8 is a bug if the caller passed the base of an object and correct if it passed an
/// interior pointer, and *every VPP vector is the second case*.
///
/// The previous wave stopped these claiming proof. That was necessary and is not enough: an
/// unactionable `Unknown` finding still costs a reader the time to dismiss it, and 147 of them
/// bury the 10 that are about the function.
///
/// **So they are not reported by default — and the count is, which is the whole difference.**
/// "Nothing found" and "147 suppressed" must never be the same output; 032 §6's mistake was
/// exactly a number quietly not counted. `--report-invented-bounds` brings them back for
/// someone who knows the entry's objects really are that size.
#[test]
fn bounds_faults_against_an_invented_object_are_suppressed_but_counted() {
    // `_vec_find (v)->len = n_elts` — an interior-pointer store, VPP's universal idiom.
    let interior = "\
func @f(%0: ptr, %1: i32) -> void {
entry:
  .line 1
  %2 = ptradd %0, -8i64
  store i32 %1 -> %2 align 4
  ret
}";
    let mut c = cfg("f");
    c.entry_ptr_nonnull = true;
    let env = find_bugs(&m(interior), &c);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(
        v["result"]["findings"].as_array().map(Vec::len),
        Some(0),
        "chiero knows nothing about the caller's object, so it has nothing to report: {v}"
    );
    assert!(
        env.blind_spots.iter().any(|b| b.contains("1")
            && (b.contains("invented") || b.contains("chiero chose") || b.contains("suppress"))),
        "but silence about a suppression is the failure this project exists to prevent: {:?}",
        env.blind_spots
    );

    // And a caller who knows better can see them.
    let mut loud = cfg("f");
    loud.entry_ptr_nonnull = true;
    loud.report_invented_bounds = true;
    let env = find_bugs(&m(interior), &loud);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert_eq!(
        v["result"]["findings"].as_array().map(Vec::len),
        Some(1),
        "--report-invented-bounds shows them: {v}"
    );

    // **A bounds fault against an object whose size chiero did NOT invent is untouched.**
    // The rule is about the provenance of the bound, not about bounds — suppressing a real
    // out-of-bounds write on a local array would trade a false-positive storm for a silence.
    let real = "\
func @g() -> void {
  alloca %buf : i32 x 16 align 4 scope 0 lifetime scope \"buf\"
entry:
  .line 1
  %0 = addrlocal %buf
  %1 = ptradd %0, 64i64
  store i32 7i32 -> %1 align 4
  ret
}";
    let mut c = cfg("g");
    c.entry_ptr_nonnull = true;
    let env = find_bugs(&m(real), &c);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    assert!(
        v["result"]["findings"]
            .as_array()
            .is_some_and(|a| !a.is_empty()),
        "a 16-element array is the program's own size, and 64 is past it: {v}"
    );
}

/// **Reading an `extern` global is not an uninitialized read.**
///
/// Found by suppressing the 147 invented-bound findings, which is the argument for having done
/// it: 5 of the 10 that were left underneath said this, on four VPP entry points —
///
/// ```text
/// clib_mem_alloc: uninitialized-read: read at offset 0 of clib_mem_thread_main touches
///                 bit 0, which was never written
/// ```
///
/// — where `clib_mem_thread_main` is `extern __thread clib_mem_thread_main_t` (`vppinfra/mem.h`).
/// An object with static storage duration is initialized before the program starts, by C's own
/// rules: zero if nothing else, and whatever the defining translation unit says otherwise
/// (C11 6.7.9p10). Its contents are **unknown to chiero**, which is not the same fact.
///
/// This is 021 §6's own distinction, one object kind over. §6 solved it for the object behind
/// an entry pointer:
///
/// > "fully symbolic and fully initialized" … leaving the bytes uninitialized turned every
/// > function that takes a pointer into an uninitialized-read report — §6 calls that "an
/// > uninitialized-read false-positive storm".
///
/// A `static` global with no initializer already reads as zero, so only the `extern` case is
/// wrong — and it is the one that matters on real code, since a header full of `extern`s is
/// what a VPP source file spends its first hundred lines on.
#[test]
fn an_extern_global_is_initialized_by_someone_even_if_not_by_us() {
    let ext = "\
global @shared : size 4 align 4 extern

func @f() -> i32 {
entry:
  .line 1
  %0 = addrglobal @shared
  %1 = load i32, %0 align 4
  ret %1
}";
    let env = find_bugs(&m(ext), &cfg("f"));
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    let uninit: Vec<_> = v["result"]["findings"]
        .as_array()
        .expect("a findings array")
        .iter()
        .filter(|f| {
            f["message"]
                .as_str()
                .is_some_and(|m| m.contains("uninitialized"))
        })
        .collect();
    assert!(
        uninit.is_empty(),
        "another translation unit initialized it, and C guarantees that happened: {v}"
    );
}

/// **A bitfield read through an entry pointer is not an uninitialized read either.**
///
/// The last of the noise on the VPP sample, and the fourth finding of the same shape:
///
/// ```text
/// clib_mem_alloc: uninitialized-read: read at offset 25 of the 4096-byte object reached
///                 through an unconstrained pointer touches bit 201, which was never
///                 written through h->traced
/// ```
///
/// `h->traced` is a bitfield in `clib_mem_heap_t`, and `h` is a pointer parameter. 021 §6 says
/// that object's bytes are "fully symbolic and fully initialized" — so this report should not
/// be constructible at all. It is, because laziness is discharged on a **byte** read and a
/// bitfield read does not take that path.
///
/// Two lines of C are enough:
///
/// ```c
/// struct s { unsigned int a:1; unsigned int traced:1; int rest; };
/// int f (struct s *p) { return p->traced; }
/// ```
///
/// The same rule as the `extern` global and the entry object before it, for the third time:
/// **chiero not knowing a value is not the program failing to write one.** What is different
/// here is that the rule was already implemented and one access path went around it — so this
/// is not a missing decision, it is a decision with a hole in it.
#[test]
fn a_bitfield_read_discharges_laziness_like_every_other_read() {
    let bitfield = "\
func @f(%0: ptr) -> i32 {
entry:
  .line 1
  %1 = loadbits i32, %0 bits 1..2 align 4
  ret %1
}";
    let mut c = cfg("f");
    c.entry_ptr_nonnull = true;
    let env = find_bugs(&m(bitfield), &c);
    let v: serde_json::Value = serde_json::from_str(&env.to_json()).expect("valid JSON");
    let uninit: Vec<_> = v["result"]["findings"]
        .as_array()
        .expect("a findings array")
        .iter()
        .filter(|f| {
            f["message"]
                .as_str()
                .is_some_and(|m| m.contains("uninitialized"))
        })
        .collect();
    assert!(
        uninit.is_empty(),
        "021 §6: the caller filled this object; chiero does not know what with: {v}"
    );
    // **And the byte being symbolic is not a finding either** — it is the whole point of §6.
    //
    // Materializing the byte turned `uninitialized-read` into `symbolic-byte: … which a
    // concrete access cannot answer for`, which is a true statement about `Memory::read_bits`
    // returning a `u128` and says nothing about the program. Trading one unactionable report
    // for another is not progress; a bitfield in caller memory has to read as a *term*.
    assert_eq!(
        v["result"]["findings"].as_array().map(Vec::len),
        Some(0),
        "a bitfield of unknown contents has an unknown value, not a defect: {v}"
    );
}
