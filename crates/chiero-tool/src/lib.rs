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
        let Some(e) = map.expansion(ctx) else { continue };
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
                def_file: info.def_file.map(|f| map.file(f).path().display().to_string()),
                def_line: info.def_line,
                body: map
                    .span_text(info.body_extent)
                    .unwrap_or_default()
                    .trim()
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
