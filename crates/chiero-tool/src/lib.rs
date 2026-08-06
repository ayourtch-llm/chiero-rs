//! `chiero-tool` — see `docs/specs/`.

use chiero_span::{ExpnCtx, SourceMap};

/// One macro invocation in an expansion chain, carrying everything needed to *read* it —
/// the name, where it was defined, and what it expands to — so an answer needs no second
/// lookup (050 §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroFrame {
    pub name: String,
    /// `None` for a `-D` macro or a builtin, which have no defining file. 010 §4 forbids
    /// inventing one: `Span::DUMMY` resolves to whichever file occupies offset 0.
    pub def_file: Option<String>,
    pub def_line: u32,
    /// The replacement list as written.
    pub body: String,
    /// Where *this* invocation is written. For the outermost frame that is the line the
    /// caller asked about; for an inner frame it is the position inside the enclosing
    /// macro's body, which is where a reader has to look to see why it was invoked.
    pub call_line: u32,
    pub call_col: u32,
    /// The arguments this invocation was given, as written. Empty for an object-like macro,
    /// which is a fact and not a gap: 060 contract 10 needs the *item* a list macro
    /// generated, and the item is exactly the per-item macro's argument list.
    pub args: Vec<String>,
}

/// One expansion chain, innermost first.
pub type Chain = Vec<MacroFrame>;

/// The expansion chain at a point, **innermost first** (050 contract 6).
///
/// `file` matches either the full recorded path or its final component, because a caller
/// asking about `vec.h` should not have to know which include path found it.
///
/// **One chain per leaf expansion, not one chain for the line.** Every expansion in a chain
/// resolves to the same written position, so `vec_add1` and the `_vec_resize` in its body
/// both match line 3 — those are one chain seen at two depths. But a list macro's items also
/// share a position, and they are *different* chains: 47 invocations of a per-item macro from
/// one `foreach_` token. Returning only the deepest answered with one arbitrary item and no
/// way to reach the rest, which is the "expanded soup" 060 §3 exists to replace.
///
/// Chains are ordered by where the leaf invocation is written, so a list macro's items come
/// back in source order.
pub fn explain_macro_expansion(
    map: &SourceMap,
    file: &str,
    line: u32,
    column: Option<u32>,
) -> Vec<Chain> {
    let mut matching: Vec<ExpnCtx> = Vec::new();
    for i in 1..=map.expansion_count() {
        let ctx = ExpnCtx(i as u32);
        let Some(e) = map.expansion(ctx) else {
            continue;
        };
        // The *written* position: an expansion nested in a macro body has a call site inside
        // that body, and only resolving through the chain reaches the line the user reads.
        let Some(loc) = map.expansion_loc(e.call_site) else {
            continue;
        };
        if loc.line != line || !path_matches(map, loc.file, file) {
            continue;
        }
        if let Some(col) = column
            && loc.col != col
        {
            continue;
        }
        matching.push(ctx);
    }

    // A match that some other match descends from is an inner frame of that chain, not a
    // chain of its own. What is left are the leaves — one per generated item.
    let interior: std::collections::BTreeSet<ExpnCtx> =
        matching.iter().flat_map(|&c| ancestors(map, c)).collect();
    let mut leaves: Vec<ExpnCtx> = matching
        .into_iter()
        .filter(|c| !interior.contains(c))
        .collect();
    // **An ordering promise, not an observed need.** `chiero-pp` records expansions as it
    // performs them, so the table is already in source order and deleting this sort changes
    // no test — it is an equivalent mutant *under today's table*. It stays because the order
    // is part of what this function promises a caller, and the alternative is for that
    // promise to rest on an undocumented invariant of another crate.
    leaves.sort_by_key(|&c| {
        map.expansion(c)
            .and_then(|e| map.lookup_loc(e.call_site.lo))
            .map_or((0, 0), |l| (l.line, l.col))
    });

    leaves.into_iter().map(|c| chain_from(map, c)).collect()
}

/// Every context strictly above `ctx`.
fn ancestors(map: &SourceMap, ctx: ExpnCtx) -> Vec<ExpnCtx> {
    let mut out = Vec::new();
    let mut cur = ctx;
    for _ in 0..=map.expansion_count() {
        let Some(e) = map.expansion(cur) else { break };
        if e.parent.is_root() {
            break;
        }
        out.push(e.parent);
        cur = e.parent;
    }
    out
}

fn chain_from(map: &SourceMap, leaf: ExpnCtx) -> Chain {
    let mut frames = Vec::new();
    let mut ctx = leaf;
    // Bounded: a malformed parent cycle must terminate with a short answer rather than hang,
    // exactly as `expansion_backtrace` does.
    for _ in 0..=map.expansion_count() {
        if ctx.is_root() {
            break;
        }
        let Some(e) = map.expansion(ctx) else { break };
        if let Some(id) = e.macro_id
            && let Some(info) = map.macro_info(id)
        {
            let call = map.lookup_loc(e.call_site.lo);
            frames.push(MacroFrame {
                name: info.name.to_string(),
                def_file: info
                    .def_file
                    .map(|f| map.file(f).path().display().to_string()),
                def_line: info.def_line,
                body: map
                    .span_text(info.body_extent)
                    .unwrap_or_default()
                    .to_owned(),
                call_line: call.map_or(0, |l| l.line),
                call_col: call.map_or(0, |l| l.col),
                // No trim: an argument span covers the argument's tokens and nothing else,
                // so `_(NONE, "none", 0x0)` yields `NONE`, `"none"`, `0x0` with no
                // surrounding space. Measured — the previous commit claimed the opposite.
                args: e
                    .arg_spans
                    .iter()
                    .map(|&a| map.span_text(a).unwrap_or_default().to_owned())
                    .collect(),
            });
        }
        ctx = e.parent;
    }
    frames
}

/// A caller says `vec.h`; the map holds whatever path the include search produced.
fn path_matches(map: &SourceMap, id: chiero_span::FileId, want: &str) -> bool {
    let Some(f) = map.try_file(id) else {
        return false;
    };
    let p = f.path();
    p.as_os_str() == want || p.file_name().is_some_and(|n| n == want)
}

/// One place a macro expands, as the user wrote it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Site {
    pub file: String,
    pub line: u32,
    pub col: u32,
    /// Where the invocation itself is written, which for a list macro's item is its line in
    /// the list body. Two items of a `foreach_` list share `line`/`col` — they come from one
    /// token — and this is what tells them apart.
    pub item_line: u32,
    pub item_col: u32,
}

/// A page of expansion sites (050 contract 7).
#[derive(Debug, Clone)]
pub struct SiteSummary {
    /// The whole population, not this page — a caller told only `shown` cannot tell a
    /// complete answer from a truncated one, which 050 §1 forbids.
    pub total: usize,
    pub shown: usize,
    pub sites: Vec<Site>,
    /// Index to resume from, or `None` when this page ends the list. Never `Some` pointing
    /// at an empty page: that reads as "more to come" and costs a caller a wasted round.
    pub cursor: Option<usize>,
}

/// Every site where `name` expands, paged (050 contract 7).
///
/// **Sites come from the expansion table, not from a scan for the name.** A textual scan
/// finds hand-written calls only, and in VPP that is nearly none of them: `vec_len` is
/// almost always reached through `vec_end` or `vec_foreach`, and every one of those is a
/// site of `vec_len` resolved to the line the user wrote.
///
/// **Sorted and deduplicated by written position *and* invocation position.** Deduplicating
/// on the user-facing position alone collapses a list macro's items into one site, because
/// all 47 come from a single `foreach_` token — and 060 §3 requires that editing one line of
/// the list impacts exactly what that line generated. The dedup still does its original job:
/// a header read under two configurations occupies two `FileId`s and yields one site.
pub fn expansion_sites(
    map: &SourceMap,
    name: &str,
    cursor: Option<usize>,
    limit: usize,
) -> SiteSummary {
    let mut sites: Vec<Site> = Vec::new();
    for i in 1..=map.expansion_count() {
        let Some(e) = map.expansion(ExpnCtx(i as u32)) else {
            continue;
        };
        let Some(info) = e.macro_id.and_then(|m| map.macro_info(m)) else {
            continue;
        };
        if &*info.name != name {
            continue;
        }
        let Some(loc) = map.expansion_loc(e.call_site) else {
            continue;
        };
        let Some(f) = map.try_file(loc.file) else {
            continue;
        };
        let item = map.lookup_loc(e.call_site.lo);
        sites.push(Site {
            file: f.path().display().to_string(),
            line: loc.line,
            col: loc.col,
            item_line: item.map_or(0, |l| l.line),
            item_col: item.map_or(0, |l| l.col),
        });
    }
    sites.sort();
    sites.dedup();

    let total = sites.len();
    let start = cursor.unwrap_or(0).min(total);
    let end = start.saturating_add(limit).min(total);
    let page: Vec<Site> = sites[start..end].to_vec();
    SiteSummary {
        total,
        shown: page.len(),
        sites: page,
        cursor: (end < total).then_some(end),
    }
}

/// How much an answer is worth (050 §2, 023 §7).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Fidelity {
    /// Proven for all inputs. **The only fidelity that may set `proven`.**
    Exact,
    /// Proven within a bound — a loop unrolled to a depth, an array modelled to a size.
    Bounded,
    /// Modelled with an approximation somewhere in the chain.
    Approximated,
    /// The engine reached something it does not model.
    Unknown,
}

/// The result envelope every operation returns (050 §2).
///
/// 050 calls this "the single most important design decision in the crate", and names the failure
/// it exists to prevent:
///
/// > an LLM reading `"findings": []` will report "the code is safe." It must instead read
/// > `"findings": [], "proven": false, "blind_spots": [...]` and be **structurally unable** to
/// > miss the qualification.
///
/// So `proven` is not a field a caller sets. It is derived from the fidelity at construction, and
/// there is no way to build an envelope whose `proven` disagrees with it. A boolean that *could*
/// be set independently would eventually be set wrongly, once, in a hurry.
///
/// This is the same rule the rest of the system runs on, at the surface a consumer reads: 030
/// keeps "no record" apart from "recorded zero", 031 §4 widens every gap, 032 §3 drops a test
/// only on an `Exact` proof. **An empty answer must carry what made it empty** — otherwise it is
/// indistinguishable from a confident one, which is the failure this project has met four times
/// in its own harnesses.
#[derive(Debug, Clone)]
pub struct Envelope {
    pub result: serde_json::Value,
    pub fidelity: Fidelity,
    /// Derived, never assigned: `fidelity == Exact`.
    pub proven: bool,
    pub assumptions: Vec<(String, String)>,
    pub blind_spots: Vec<String>,
    truncation: Option<(usize, usize)>,
}

impl Envelope {
    /// Wrap a result. `proven` follows from `fidelity` and cannot be set another way.
    pub fn new(result: serde_json::Value, fidelity: Fidelity) -> Envelope {
        Envelope {
            result,
            fidelity,
            proven: fidelity == Fidelity::Exact,
            assumptions: Vec::new(),
            blind_spots: Vec::new(),
            truncation: None,
        }
    }

    /// Record something the run could not model. 050 contract 4 requires **every** assumption
    /// kind that actually occurred, not a representative sample.
    pub fn with_assumption(mut self, kind: &str, detail: &str) -> Envelope {
        self.assumptions.push((kind.into(), detail.into()));
        self
    }

    /// Record a class of thing this answer cannot see.
    pub fn with_blind_spot(mut self, what: &str) -> Envelope {
        self.blind_spots.push(what.into());
        self
    }

    /// Record that the result is a page of a larger population.
    ///
    /// **Visible, not silent.** An LLM shown 50 of 1043 expansion sites and told nothing will
    /// reason about 50.
    pub fn truncated(mut self, shown: usize, total: usize) -> Envelope {
        self.truncation = Some((shown, total));
        self
    }

    /// A key over the result and its qualifications, so two runs can be compared.
    pub fn determinism_key(&self) -> String {
        // FNV-1a over the rendered document, for the reason `chiero-gcov::source_hash` gives:
        // this notices an accidental difference, and nothing here faces an adversary.
        const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
        const PRIME: u128 = 0x0000000001000000000000000000013b;
        let mut h = OFFSET;
        for b in self.body().to_string().bytes() {
            h ^= b as u128;
            h = h.wrapping_mul(PRIME);
        }
        format!("fnv128:{h:032x}")
    }

    fn body(&self) -> serde_json::Value {
        serde_json::json!({
            "result": self.result,
            "fidelity": format!("{:?}", self.fidelity),
            "proven": self.proven,
            "assumptions": self
                .assumptions
                .iter()
                .map(|(k, d)| serde_json::json!({ "kind": k, "detail": d }))
                .collect::<Vec<_>>(),
            "blind_spots": self.blind_spots,
            "truncation": match self.truncation {
                Some((shown, total)) => serde_json::json!({
                    "truncated": true, "shown": shown, "total": total,
                }),
                None => serde_json::json!({ "truncated": false }),
            },
        })
    }

    pub fn to_json(&self) -> String {
        let mut v = self.body();
        v["determinism_key"] = serde_json::Value::String(self.determinism_key());
        v.to_string()
    }

    /// The text rendering, which follows the same rule as the JSON (050 §2).
    ///
    /// > "no defects found **within** <bound>", never "no defects found", unless `proven` is true.
    pub fn render(&self) -> String {
        let mut out = render_value(&self.result, 0);
        out.push('\n');
        out.push_str(&if self.proven {
            "proven — this holds for all inputs (Exact)".to_string()
        } else {
            format!(
                "not proven — within this run's bounds ({:?})",
                self.fidelity
            )
        });
        for b in &self.blind_spots {
            out.push_str(&format!("\n  blind spot: {b}"));
        }
        for (k, d) in &self.assumptions {
            out.push_str(&format!("\n  assumed: {k} ({d})"));
        }
        if let Some((shown, total)) = self.truncation {
            out.push_str(&format!("\n  showing {shown} of {total}"));
        }
        out
    }
}

/// Which tests must run for a change (050 §3, 032).
///
/// # Never `Exact`, and the reason is not a limitation of the implementation
///
/// `proven` means proven for all inputs. A selection cannot be, because **coverage is
/// historical**: it records what the tests did on the code as it was, and the method rests on
/// that being a good guide to what they will do on the code as it is.
///
/// 032 §4's safety set covers the cases with no measurement at all — a new test, a crashed run, a
/// stale index — and 031 §3's closure covers a test reaching new code through a caller it already
/// covered. Neither turns the answer into a proof. So the fidelity is [`Fidelity::Bounded`] at
/// best, the bound is named as a blind spot, and a caller reading the envelope cannot miss it.
///
/// Returning `Exact` here would be exactly the failure 050 §2 exists to prevent, committed by the
/// crate that enforces it.
pub fn select_tests(
    impact: &chiero_diff::ImpactSet,
    program: &chiero_diff::Program,
    coverage: &chiero_gcov::CoverageIndex,
    suite: &chiero_select::Suite,
) -> Envelope {
    let selection = chiero_select::select_with(impact, program, coverage, suite);
    let ranked = selection.ranked();

    let tests: Vec<serde_json::Value> = ranked
        .iter()
        .enumerate()
        .map(|(i, t)| {
            serde_json::json!({
                "test": t.0,
                "rank": i + 1,
                "reasons": selection.tests[t]
                    .iter()
                    .map(describe_reason)
                    .collect::<Vec<_>>(),
            })
        })
        .collect();

    let always_run = selection
        .tests
        .values()
        .filter(|rs| {
            rs.iter()
                .all(|r| matches!(r, chiero_select::SelectionReason::AlwaysRun { .. }))
        })
        .count();

    // **Reduction beside safety** (032 contract 20), through the boundary rather than stopping at
    // it: a caller reading `tests` alone sees a number and nothing to judge it by.
    let result = serde_json::json!({
        "tests": tests,
        "selected": ranked.len(),
        "always_run": always_run,
        "excluded": selection.excluded.len(),
    });

    let (fidelity, caveats) = match &selection.confidence {
        chiero_select::Confidence::Full => (Fidelity::Bounded, Vec::new()),
        // Something upstream could not be computed, so the answer rests on more than the
        // coverage bound.
        chiero_select::Confidence::Reduced { reasons } => (Fidelity::Unknown, reasons.clone()),
    };

    let mut env = Envelope::new(result, fidelity)
        .with_blind_spot(
            "coverage is historical: it records what these tests did on the previous build",
        )
        .with_blind_spot("symbolic refinement did not run, so nothing was proven unnecessary");
    for c in caveats {
        env = env.with_assumption("incomplete_analysis", &c);
    }
    env
}

fn describe_reason(r: &chiero_select::SelectionReason) -> serde_json::Value {
    match r {
        chiero_select::SelectionReason::CoversEntity {
            entity,
            file,
            line,
            distance,
        } => serde_json::json!({
            "kind": "covers_entity",
            "entity": entity,
            "file": file,
            "line": line,
            "distance": distance,
        }),
        chiero_select::SelectionReason::AlwaysRun { why } => {
            serde_json::json!({ "kind": "always_run", "why": why })
        }
    }
}

/// [`expansion_sites`] in 050 §2's envelope (contract 7).
///
/// # Why this is a wrapper rather than a change to `expansion_sites`
///
/// [`SiteSummary`] reports its own truncation, which is the right shape for a Rust caller holding
/// the value. The JSON surface needs it in the envelope, because 050 §2's argument is that a
/// consumer reads *one* place for the qualification of *any* answer — an operation that reported
/// it in its own shape would be one more thing to learn, and the one a reader forgets.
///
/// # The fidelity, which is the only judgement here
///
/// **`Exact`, and this is the first 050 operation for which that is honest.** The sites are the
/// preprocessor's own expansion table rather than a scan or an estimate, so the answer is
/// complete for the translation unit it was given — and a macro that expands nowhere is *proven*
/// empty, which is the one place in this system an empty answer may be read plainly.
///
/// **A truncated page is not proven.** The operation is exact; the response is a page of it, and
/// a caller holding 50 of 1043 sites does not hold the answer. Nothing about the page is
/// approximate, which is exactly why this distinction is easy to lose.
pub fn expansion_sites_envelope(
    map: &SourceMap,
    name: &str,
    cursor: Option<usize>,
    limit: usize,
) -> Envelope {
    let summary = expansion_sites(map, name, cursor, limit);
    let complete = summary.cursor.is_none() && summary.shown == summary.total;

    let result = serde_json::json!({
        "total": summary.total,
        "shown": summary.shown,
        "cursor": summary.cursor,
        "sites": summary
            .sites
            .iter()
            .map(|s| serde_json::json!({
                "file": s.file,
                "line": s.line,
                "col": s.col,
                "item_line": s.item_line,
                "item_col": s.item_col,
            }))
            .collect::<Vec<_>>(),
    });

    let env = Envelope::new(
        result,
        if complete {
            Fidelity::Exact
        } else {
            // Nothing is approximated — the caller simply does not have all of it.
            Fidelity::Bounded
        },
    );
    if complete {
        env
    } else {
        env.truncated(summary.shown, summary.total)
    }
}

/// [`explain_macro_expansion`] in 050 §2's envelope (contract 6).
///
/// # The empty answer is the whole point
///
/// A chain comes from the preprocessor's own expansion records, so it is `Exact`. What needs the
/// judgement is `[]`:
///
/// - **a line with no macro on it** is a *complete* answer that nothing expanded there, and is
///   proven. `[]` from a scan would mean "nothing found within what the scan could see"; `[]`
///   from the expansion table means "nothing is there", and only the second may be read plainly.
/// - **a file the map has never heard of** is not that answer at all. It is a question about
///   something outside this translation unit, and returning a confident empty list for it is the
///   failure 050 §2 exists to prevent, in the smallest possible case.
pub fn explain_macro_expansion_envelope(
    map: &SourceMap,
    file: &str,
    line: u32,
    column: Option<u32>,
) -> Envelope {
    let known = map.files().any(|f| path_matches(map, f.id(), file));
    let chains = explain_macro_expansion(map, file, line, column);

    let result = serde_json::json!({
        "chains": chains
            .iter()
            .map(|c| c.iter().map(frame_json).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
    });

    if known {
        Envelope::new(result, Fidelity::Exact)
    } else {
        Envelope::new(result, Fidelity::Unknown)
            .with_assumption("unknown_file", file)
            .with_blind_spot(&format!(
                "`{file}` is not in this translation unit, so an empty chain says nothing about it"
            ))
    }
}

fn frame_json(f: &MacroFrame) -> serde_json::Value {
    serde_json::json!({
        "name": f.name,
        "def_file": f.def_file,
        "def_line": f.def_line,
        "body": f.body,
        "call_line": f.call_line,
        "call_col": f.call_col,
        "args": f.args,
    })
}

/// `prove_equivalent` in 050 §2's envelope — [050 contract 8](../../../docs/specs/050-tool-interface.md).
///
/// 050 §1: *"the LLM proposes; chiero adjudicates."* This is the operation that sentence is
/// about. [041 §1](../../../docs/specs/041-optimization-analysis.md) does the deciding; the
/// judgement here is what fidelity the answer deserves, which is the only thing this layer
/// is for.
///
/// # Why `Differs` is `Exact`
///
/// It reads like an overclaim and is not. A `Differs` carries a concrete input at which the
/// two versions demonstrably disagree, and "these two functions are not equivalent" is a
/// claim about *all* inputs — it happens to be witnessed by one. 041 skips any pair of paths
/// a budget cut, so a divergence never comes from a truncated path.
///
/// What is *not* proven is that a real compiler agrees, and that is a blind spot rather than
/// a fidelity: 041 §1.3 wants "a replay harness that compiles and demonstrates the
/// divergence", contract 11 wants a harness that fails to demonstrate it to downgrade the
/// result — and **no harness is built yet**. An un-run harness is not a failed one, so the
/// verdict stands and the envelope says, in the place a consumer is structurally unable to
/// miss, that nothing has checked it against a compiler.
///
/// # Why `Equivalent` is whatever 041 said and never better
///
/// `Fidelity::Bounded` is 041 §1.2's own word for "a statement about inputs within the
/// bound, not a proof", and 032 §3.1 refuses to drop a test on one. Promoting it here would
/// let the surface bless a rewrite the pruner would not trust — the two layers disagreeing
/// about the same value, which is how a caveat gets lost.
pub fn prove_equivalent(
    before: &chiero_cir::Module,
    after: &chiero_cir::Module,
    cfg: &chiero_opt::EquivCfg,
) -> Envelope {
    match chiero_opt::prove_equivalent(before, after, cfg) {
        chiero_opt::Equivalence::Equivalent {
            fidelity,
            footprint,
            assumptions,
        } => {
            let result = serde_json::json!({
                "verdict": "equivalent",
                "compared": footprint.compared.iter().map(|c| c.label()).collect::<Vec<_>>(),
            });
            let mut env = Envelope::new(result, exec_fidelity(fidelity));
            for a in &assumptions {
                env = env.with_assumption(&format!("{:?}", a.kind), &a.detail);
            }
            // **What `compared` leaves out is the load-bearing half.** 041 §1.1 makes
            // equivalence three claims and this decides two; a consumer reading
            // `"verdict": "equivalent"` beside a list of two would otherwise have to know
            // the list was meant to be three.
            for missing in [chiero_opt::Claim::Memory, chiero_opt::Claim::SideEffects] {
                if !footprint.compared.contains(&missing) {
                    env = env.with_blind_spot(&format!(
                        "{} was not compared (041 §1.1); the verdict is silent about it",
                        missing.label()
                    ));
                }
            }
            env
        }
        chiero_opt::Equivalence::Differs {
            input,
            observation,
            replay,
        } => {
            let result = serde_json::json!({
                "verdict": "differs",
                "input": input.bindings.iter().map(binding_json).collect::<Vec<_>>(),
                "observation": divergence_json(&observation),
                "replay": serde_json::Value::Null,
            });
            let env = Envelope::new(result, Fidelity::Exact);
            match replay {
                // Unreachable today — `Replay` has no constructor — and written as a match
                // rather than an unconditional blind spot so that building one flips this
                // instead of leaving a stale caveat behind.
                Some(_) => env,
                None => env.with_blind_spot(
                    "no replay harness was compiled (041 §1.3), so the divergence is \
                     chiero's semantics and has not been demonstrated against a compiler",
                ),
            }
        }
        chiero_opt::Equivalence::Unknown { reason } => Envelope::new(
            serde_json::json!({ "verdict": "unknown", "reason": reason }),
            Fidelity::Unknown,
        )
        .with_assumption("undecided", &reason)
        .with_blind_spot("the two versions were not shown to agree and were not shown to differ"),
    }
}

/// 023 §7's fidelity in 050 §2's terms. A plain mapping, named so the one place it happens
/// is greppable — a second, divergent copy of this match is how the two layers would start
/// disagreeing about what `Bounded` means.
fn exec_fidelity(f: chiero_exec::Fidelity) -> Fidelity {
    match f {
        chiero_exec::Fidelity::Exact => Fidelity::Exact,
        chiero_exec::Fidelity::Bounded => Fidelity::Bounded,
        chiero_exec::Fidelity::Approximated => Fidelity::Approximated,
        chiero_exec::Fidelity::Unknown => Fidelity::Unknown,
    }
}

fn binding_json(b: &chiero_exec::Binding) -> serde_json::Value {
    // Both readings, because neither alone is the number a reader wants: 4294967295 and -1
    // are the same input, and which one makes the divergence obvious depends on the code.
    let signed = match b.width {
        0..=64 => {
            let shift = 128 - b.width.max(1);
            Some(((b.value << shift) as i128) >> shift)
        }
        _ => None,
    };
    serde_json::json!({
        "origin": b.origin.label(),
        "width": b.width,
        "value": b.value.to_string(),
        "signed": signed.map(|s| s.to_string()),
        "pinned": b.pinned,
    })
}

fn divergence_json(d: &chiero_opt::Divergence) -> serde_json::Value {
    match d {
        chiero_opt::Divergence::ReturnValue { before, after } => serde_json::json!({
            "kind": "return_value",
            "before": before.bits().to_string(),
            "before_signed": before.signed().to_string(),
            "after": after.bits().to_string(),
            "after_signed": after.signed().to_string(),
            "width": before.width(),
        }),
        chiero_opt::Divergence::Memory {
            object,
            offset,
            before,
            after,
        } => serde_json::json!({
            "kind": "memory",
            "object": object,
            "offset": offset,
            "before": before,
            "after": after,
        }),
        chiero_opt::Divergence::SideEffect {
            index,
            before,
            after,
        } => serde_json::json!({
            "kind": "side_effect",
            "index": index,
            "before": before,
            "after": after,
        }),
        chiero_opt::Divergence::Termination { before, after } => serde_json::json!({
            "kind": "termination",
            "before": format!("{before:?}"),
            "after": format!("{after:?}"),
        }),
    }
}

/// [031](../../../docs/specs/031-change-impact.md)'s change impact in 050 §2's envelope.
///
/// 050 §3's "Change analysis" row. The operation `select_tests` consumes internally, exposed on
/// its own because the question *"what does this change reach"* is one a caller asks without
/// wanting a test list — and because the answer is the one that makes the macro case legible.
///
/// # The fidelity, which is the whole judgement here
///
/// **`Exact` when the analysis is complete, and never otherwise.** The impact set is computed
/// from the two token streams and 014's computed layouts, not sampled or estimated, so a
/// complete one is a complete answer about this translation unit. That is a narrower claim than
/// it sounds and the blind spot says so: the closure is over *this* unit, and a caller of the
/// changed function from a unit chiero was not given is not in it.
///
/// `Completeness::Partial` means a file did not parse cleanly. Its entities are still in the
/// set — 031 §4 widens every gap rather than narrowing it — and that is exactly why the
/// fidelity must drop: an over-reported set is safe to act on and is not a proof of anything.
pub fn impact_envelope(before: &chiero_diff::Program, after: &chiero_diff::Program) -> Envelope {
    let set = chiero_diff::impact(before, after);
    let entities: Vec<serde_json::Value> = set
        .entities
        .iter()
        .map(|(e, j)| {
            serde_json::json!({
                "name": e.name(),
                "file": e.file(),
                "kind": entity_kind(e),
                "class": format!("{:?}", j.class),
                "root": j.root.name(),
                "distance": j.distance,
                "changed_lines": j.changed_lines,
                "edges": j.edges.iter().map(|x| format!("{x:?}")).collect::<Vec<_>>(),
            })
        })
        .collect();

    let result = serde_json::json!({
        "entities": entities,
        "count": set.entities.len(),
        "completeness": format!("{:?}", set.completeness),
    });

    let env = match &set.completeness {
        chiero_diff::Completeness::Complete => Envelope::new(result, Fidelity::Exact),
        other => Envelope::new(result, Fidelity::Bounded)
            .with_assumption("incomplete_analysis", &format!("{other:?}")),
    };
    env.with_blind_spot(
        "the closure is over the translation units given; a caller in a unit chiero was not \
         shown is not in this set",
    )
}

fn entity_kind(e: &chiero_diff::Entity) -> &'static str {
    match e {
        chiero_diff::Entity::Function { .. } => "function",
        chiero_diff::Entity::Global { .. } => "global",
        chiero_diff::Entity::Typedef { .. } => "typedef",
        chiero_diff::Entity::Macro { .. } => "macro",
        chiero_diff::Entity::Record { .. } => "record",
        chiero_diff::Entity::EnumConst { .. } => "enum_const",
    }
}

/// A result as lines a person reads, rather than as the JSON a program does.
///
/// **Deliberately generic.** It walks the value and knows nothing about any operation — a
/// renderer with a case per operation would be a second description of every result, and the
/// second one is the one that goes stale. Keys keep their names, so a reader who moves between
/// this and `--json` is reading the same words.
///
/// A JSON `null` prints as `(none)`: the word `null` is a programmer's habit, and the thing it
/// means here is always "there is nothing here", which is worth saying in a language.
fn render_value(v: &serde_json::Value, indent: usize) -> String {
    let pad = "  ".repeat(indent);
    match v {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, val)| match val {
                serde_json::Value::Object(_) | serde_json::Value::Array(_) if !is_empty(val) => {
                    format!("{pad}{k}:\n{}", render_value(val, indent + 1))
                }
                _ => format!("{pad}{k}: {}", scalar(val)),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        serde_json::Value::Array(items) => items
            .iter()
            .map(|item| match item {
                serde_json::Value::Object(_) => {
                    // The first line of an element carries the bullet, so a list of records
                    // reads as a list rather than as a wall.
                    //
                    // **Only the first line is re-padded.** Trimming and re-padding *every*
                    // line worked for a flat record and flattened anything inside one — a
                    // finding's witness came out with its fields beside the bullet that
                    // introduced them instead of under it. The element is rendered at the
                    // right indent to begin with; the bullet replaces two of its leading
                    // spaces.
                    let body = render_value(item, indent + 1);
                    let mut lines = body.lines();
                    let first = lines.next().unwrap_or("");
                    let mut s = format!("{pad}- {}", first.trim_start());
                    for l in lines {
                        s.push('\n');
                        s.push_str(l);
                    }
                    s
                }
                _ => format!("{pad}- {}", scalar(item)),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        other => format!("{pad}{}", scalar(other)),
    }
}

fn is_empty(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Object(m) => m.is_empty(),
        serde_json::Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

fn scalar(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "(none)".to_string(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) if a.is_empty() => "(empty)".to_string(),
        serde_json::Value::Object(m) if m.is_empty() => "(empty)".to_string(),
        other => other.to_string(),
    }
}

/// How to run [`find_bugs`].
#[derive(Clone, Debug)]
pub struct BugCfg {
    pub entry: String,
    pub budget: chiero_exec::Budget,
    pub backend: Option<chiero_solver::SmtLib>,
}

impl BugCfg {
    /// Discovers a solver, as the engine's own default does (022 §4).
    pub fn new(entry: impl Into<String>) -> BugCfg {
        BugCfg {
            entry: entry.into(),
            budget: chiero_exec::Budget::default(),
            backend: chiero_solver::SmtLib::discover(),
        }
    }
}

/// [040](../../../docs/specs/040-defect-checkers.md)'s checkers in 050 §2's envelope —
/// **050 contract 3**, and the operation the envelope exists for.
///
/// > "an LLM reading `"findings": []` will report 'the code is safe'."
///
/// Every other operation's empty answer is merely uninformative. This one's empty answer is a
/// claim about the code, and it is wrong in exactly the case that is hardest to notice: when
/// the search stopped early. So three things an empty list can mean are kept apart, by the
/// envelope rather than by prose —
///
/// - the run finished and found nothing (`Exact`, `proven`),
/// - the run hit a budget (`Bounded`, and `budgets.hit` names which),
/// - the run met something it could not model (`Approximated`/`Unknown`, with the assumption).
///
/// # `budgets.hit` names the budget, not the fact
///
/// Contract 3 asks for "a non-empty `budgets.hit`". A list containing `"a budget was hit"`
/// would satisfy that string-wise and tell a reader nothing: the actionable difference between
/// `max_loop_iters` and `max_states` is which knob to turn, and a reader who cannot tell them
/// apart cannot decide whether re-running would help.
pub fn find_bugs(module: &chiero_cir::Module, cfg: &BugCfg) -> Envelope {
    if !module.funcs.iter().any(|f| *f.name == cfg.entry) {
        // **An error, not an empty finding list.** A typo in an entry name would otherwise
        // produce the most confident possible all-clear.
        return Envelope::new(
            serde_json::json!({
                "findings": [],
                "budgets": { "hit": [] },
                "error": format!("no function named `{}` in this module", cfg.entry),
            }),
            Fidelity::Unknown,
        )
        .with_assumption("nothing_analysed", &cfg.entry)
        .with_blind_spot(&format!(
            "`{}` was not found, so nothing was analysed and this list is not about your code",
            cfg.entry
        ));
    }

    let mut arena = chiero_solver::TermArena::new();
    let mut engine = chiero_exec::Engine::new(module)
        .with_entry(&cfg.entry)
        .with_budget(cfg.budget);
    engine = match cfg.backend.clone() {
        Some(b) => engine.with_backend(b),
        None => engine.with_solver(chiero_exec::SolverTier::LiteOnly),
    };
    for c in chiero_check::default_checkers() {
        engine = engine.with_checker(c);
    }
    let run = engine.run(&mut arena);

    // **One bug is one entry, and the number of paths that reached it is kept.**
    //
    // 023 §6.1 deduplicates a *fork's* copies and deliberately does not deduplicate a *loop's*
    // — those are separate reports of one bug, which is the right answer for the engine and
    // the wrong shape for a reader: an unrolled loop turns one division by zero into nine
    // near-identical lines and the eighth is where somebody stops reading. Grouped here, at
    // the layer whose job is what a consumer sees.
    //
    // **Never silently.** "1 finding" and "1 finding on 9 paths" are different facts, and the
    // second is what tells a reader the loop is involved — so `paths` is on every entry.
    let mut grouped: Vec<(chiero_exec::Finding, usize)> = Vec::new();
    for f in run.reports() {
        match grouped
            .iter_mut()
            .find(|(g, _)| g.message == f.message && g.span == f.span)
        {
            Some((_, n)) => *n += 1,
            None => grouped.push((f, 1)),
        }
    }

    let findings: Vec<serde_json::Value> = grouped
        .iter()
        .map(|(f, paths)| {
            serde_json::json!({
                "message": f.message,
                "paths": paths,
                "fidelity": format!("{:?}", f.fidelity),
                "solver": f.solver,
                // 023 contract 15: a witness, or the reason there is none. The absence is
                // allowed; the silence is not.
                "witness": f.witness.as_ref().map(|w| {
                    w.bindings.iter().map(binding_json).collect::<Vec<_>>()
                }),
                "unwitnessed": f.unwitnessed,
            })
        })
        .collect();

    // **Which budgets, from the assumptions the run actually recorded** — not from comparing
    // the configured limits against counters, which would be a second implementation of what
    // the engine already knows and would drift from it.
    let mut hit: Vec<String> = Vec::new();
    let mut assumptions: Vec<chiero_exec::Assumption> = Vec::new();
    for s in run.states() {
        for a in s.assumptions() {
            if !assumptions.iter().any(|x| x == a) {
                assumptions.push(a.clone());
            }
            if a.kind == chiero_exec::AssumptionKind::BudgetHit && !hit.contains(&a.detail) {
                hit.push(a.detail.clone());
            }
        }
    }

    let result = serde_json::json!({
        "findings": findings,
        "budgets": { "hit": hit },
    });

    let fidelity = exec_fidelity(run.fidelity());
    let mut env = Envelope::new(result, fidelity);
    for a in &assumptions {
        env = env.with_assumption(&format!("{:?}", a.kind), &a.detail);
    }
    // **The blind spot is about the empty case even when the list is not empty**, because a
    // reader deciding what to do next needs to know what was *not* searched either way.
    if fidelity != Fidelity::Exact {
        env = env.with_blind_spot(
            "the search did not cover the whole program, so an absent finding is not an \
             absent defect",
        );
    }
    env.with_blind_spot(&format!(
        "only the {} checkers of 040 ran; a defect no checker looks for is not reported",
        chiero_check::default_checkers().len()
    ))
}
