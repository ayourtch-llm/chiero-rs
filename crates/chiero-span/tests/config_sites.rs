//! Covers: 010 contract 19 — per-`ConfigId` expansion sites.
//!
//! "A macro expanded under two different `ConfigId`s produces sites carrying each config,
//! and querying by config returns only the matching subset."
//!
//! This is what makes the cooked index usable on a project that ships more than one build.
//! VPP compiles the same headers under several configurations, and the *same source line*
//! is a different expansion in each: `CLIB_DEBUG` changes what `ASSERT` expands to,
//! `CLIB_ARCH_IS_BIG_ENDIAN` changes which bitfield layout a header declares. An index
//! that merged them answers "where is this macro used?" with a list that is true of no
//! single build.
//!
//! The dedup key is the part that has to change. 010 §6.3 deduplicates on
//! `(entity, file, line)` so that `M + M + M` on one line is one *site* rather than three
//! events; under two configurations that same key collapses two genuinely different sites
//! into one, and the second configuration's is the one that vanishes.

use chiero_span::{
    BytePos, CookedExpansionIndex, ExpnCtx, ExpnKind, GlobalInterner, SourceMap, Span,
};

/// Cook one TU under `config`, expanding `ADD1` at `c_line` of `c_path`.
fn cook(
    interner: &mut GlobalInterner,
    index: &mut CookedExpansionIndex,
    c_path: &str,
    c_line: u32,
    config: u64,
) {
    let header = "#define ADD1(V) ((V) = (V) + 1)\n";
    let mut c_src = String::new();
    for _ in 1..c_line {
        c_src.push('\n');
    }
    c_src.push_str("  ADD1 (x);\n");

    let mut sm = SourceMap::new();
    let hf = sm.add_file("vppinfra/vec.h", header);
    let cf = sm.add_file(c_path, c_src.clone());
    let hbase = sm.file(hf).start_pos.0;
    let cbase = sm.file(cf).start_pos.0;

    let def = Span::new(BytePos(hbase + 8), BytePos(hbase + 12), ExpnCtx::ROOT);
    let body = Span::new(BytePos(hbase + 16), BytePos(hbase + 31), ExpnCtx::ROOT);
    let m = sm.add_macro("ADD1", def, body);

    let off = c_src.find("ADD1").unwrap() as u32;
    let call = Span::new(
        BytePos(cbase + off),
        BytePos(cbase + off + 4),
        ExpnCtx::ROOT,
    );
    sm.add_expansion(
        ExpnCtx::ROOT,
        Some(m),
        call,
        call,
        vec![],
        ExpnKind::FunctionLike,
    );

    index.cook_tu_with_config(interner, &sm, config);
}

fn add1(interner: &GlobalInterner) -> chiero_span::MacroEntity {
    interner
        .lookup_macro("vppinfra/vec.h", "ADD1")
        .expect("ADD1 was interned")
}

/// **Contract 19.** The same macro at the same line under two configurations is two
/// sites, each carrying its config, and a query by config returns only its own.
#[test]
fn two_configs_produce_two_sites_and_the_query_separates_them() {
    let mut interner = GlobalInterner::new();
    let mut index = CookedExpansionIndex::new();
    // **The same file and the same line**, which is the case that matters: a different
    // line would be two sites under any dedup key, so it would prove nothing.
    cook(&mut interner, &mut index, "ip4_forward.c", 40, 1);
    cook(&mut interner, &mut index, "ip4_forward.c", 40, 2);

    let e = add1(&interner);
    let all: Vec<&chiero_span::CookedSite> = index.sites(e).collect();
    assert_eq!(
        all.len(),
        2,
        "one site per configuration — merging them answers `where is this used?` with a \
         list that is true of no single build: {all:#?}"
    );

    let one: Vec<u32> = index.sites_for_config(e, 1).map(|s| s.line).collect();
    let two: Vec<u32> = index.sites_for_config(e, 2).map(|s| s.line).collect();
    assert_eq!(one, vec![40], "config 1's site");
    assert_eq!(two, vec![40], "config 2's site");
    assert!(
        index.sites_for_config(e, 3).next().is_none(),
        "and a configuration nothing was cooked under has no sites"
    );

    // Each site says which configuration it came from, or the query above is the only
    // thing that knows and nothing can report it.
    let mut configs: Vec<u64> = all.iter().map(|s| s.config).collect();
    configs.sort_unstable();
    assert_eq!(configs, vec![1, 2]);
}

/// **Within one configuration, deduplication still holds.**
///
/// 010 §6.3's whole justification for the cooked index is that it is bounded by expansion
/// *sites* rather than *events*. Widening the dedup key to include the config must not
/// turn one site into many — cooking the same TU twice under the same config is one site,
/// exactly as before.
#[test]
fn deduplication_within_a_config_is_unchanged() {
    let mut interner = GlobalInterner::new();
    let mut index = CookedExpansionIndex::new();
    cook(&mut interner, &mut index, "a.c", 10, 5);
    cook(&mut interner, &mut index, "a.c", 10, 5);

    let e = add1(&interner);
    assert_eq!(
        index.sites(e).count(),
        1,
        "same entity, file, line and config: one site, or the index grows with events \
         again and the tens-of-gigabytes problem comes back"
    );
}

/// The unqualified `sites()` still returns everything, so existing callers keep working
/// and a caller that has no configuration in hand is not forced to invent one.
#[test]
fn the_unqualified_query_returns_every_config() {
    let mut interner = GlobalInterner::new();
    let mut index = CookedExpansionIndex::new();
    cook(&mut interner, &mut index, "a.c", 10, 1);
    cook(&mut interner, &mut index, "b.c", 20, 2);

    let e = add1(&interner);
    let mut lines: Vec<u32> = index.sites(e).map(|s| s.line).collect();
    lines.sort_unstable();
    assert_eq!(lines, vec![10, 20]);
    assert_eq!(index.dropped(), 0, "nothing was lost");
}
