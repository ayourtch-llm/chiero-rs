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
        let head = if self.proven {
            format!("{} (proven, Exact)", self.result)
        } else {
            format!(
                "{} — within this run's bounds ({:?}); not proven",
                self.result, self.fidelity
            )
        };
        let mut out = head;
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
