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
}

/// The expansion chain at a point, **innermost first** (050 contract 6).
///
/// `file` matches either the full recorded path or its final component, because a caller
/// asking about `vec.h` should not have to know which include path found it.
///
/// **Depth picks the chain when the column is omitted.** Every expansion on the line
/// resolves to the same written position through `expansion_loc`, so `vec_add1` and the
/// `_vec_resize` nested inside it both match line 3 — they are one chain seen at two
/// depths, and the deepest is the only one that contains the others. A line holding two
/// *independent* calls is the case this cannot separate, which is what `column` is for.
pub fn explain_macro_expansion(
    map: &SourceMap,
    file: &str,
    line: u32,
    column: Option<u32>,
) -> Vec<MacroFrame> {
    let mut best: Option<(usize, ExpnCtx)> = None;
    for i in 1..=map.expansion_count() {
        let ctx = ExpnCtx(i as u32);
        let Some(e) = map.expansion(ctx) else {
            continue;
        };
        // The *written* position: an expansion nested in a macro body has a call site
        // inside that body, and only resolving through the chain reaches the line the user
        // is actually reading.
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
        let depth = depth_of(map, ctx);
        if best.is_none_or(|(d, _)| depth > d) {
            best = Some((depth, ctx));
        }
    }

    let Some((_, leaf)) = best else {
        return Vec::new();
    };

    let mut frames = Vec::new();
    let mut ctx = leaf;
    // Bounded: a malformed parent cycle must terminate with a short answer rather than
    // hang, exactly as `expansion_backtrace` does.
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
                // No trim: `body_extent` runs from the first body token to the last, so it
                // never carries surrounding whitespace. `#define A   1 + 2   ` yields
                // `1 + 2` already. A `.trim()` here was unobservable under mutation, which
                // is the signal that it was guarding against nothing.
                body: map
                    .span_text(info.body_extent)
                    .unwrap_or_default()
                    .to_owned(),
                call_line: call.map_or(0, |l| l.line),
                call_col: call.map_or(0, |l| l.col),
            });
        }
        ctx = e.parent;
    }
    frames
}

fn depth_of(map: &SourceMap, mut ctx: ExpnCtx) -> usize {
    let mut n = 0;
    for _ in 0..=map.expansion_count() {
        if ctx.is_root() {
            break;
        }
        let Some(e) = map.expansion(ctx) else { break };
        n += 1;
        ctx = e.parent;
    }
    n
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
/// **Sorted and deduplicated by written position.** The same position can be reached by
/// more than one recorded expansion — a header read under two configurations occupies two
/// `FileId`s — and a caller paging a list that shifts under it would see a site twice and
/// miss another.
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
        sites.push(Site {
            file: f.path().display().to_string(),
            line: loc.line,
            col: loc.col,
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
