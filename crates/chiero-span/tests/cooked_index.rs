//! The whole-tree expansion index (010 §6.2).
//!
//! Covers **010 contracts 13–17**. Contract 18 (peak-memory bound) needs a large fixture
//! and is owed and listed in HANDOFF rather than silently claimed here; contract 19
//! (per-`ConfigId` sites) landed in wave 106 and is covered in `config_sites.rs`. This is the fix for a design error the adversarial
//! review found: the earlier design dropped per-TU expansion tables while retaining a
//! reverse index of `ExpnCtx` values — which are *indices into the dropped tables*. The
//! headline capability would have broken at exactly the scale where it matters.
//!
//! Contract 13 therefore has to be written as a test that **actually drops** the
//! per-TU maps. A test that kept them alive would pass against the broken design.

use chiero_span::{
    BytePos, CookedExpansionIndex, ExpnCtx, ExpnKind, GlobalInterner, SourceMap, Span,
};

/// Preprocess one TU that includes a shared header, cook it, and drop the `SourceMap`.
/// Returns nothing but the cooked index — which is the point.
fn cook_tu(
    interner: &mut GlobalInterner,
    index: &mut CookedExpansionIndex,
    c_path: &str,
    c_line: u32,
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

    index.cook_tu(interner, &sm);
    // `sm` is dropped here. Everything the index needs must already be resolved.
}

/// 010 contract 13: the retained index survives the tables it was built from.
#[test]
fn cooked_index_outlives_the_source_maps() {
    let mut interner = GlobalInterner::new();
    let mut index = CookedExpansionIndex::new();
    cook_tu(&mut interner, &mut index, "ip4_forward.c", 900);
    cook_tu(&mut interner, &mut index, "ip6_forward.c", 42);

    let entity = interner
        .lookup_macro("vppinfra/vec.h", "ADD1")
        .expect("macro entity");
    let sites: Vec<_> = index.sites(entity).collect();
    assert_eq!(sites.len(), 2, "one site per TU");

    let mut located: Vec<(String, u32)> = sites
        .iter()
        .map(|s| (interner.path(s.file).display().to_string(), s.line))
        .collect();
    located.sort();
    assert_eq!(
        located,
        vec![
            ("ip4_forward.c".to_string(), 900),
            ("ip6_forward.c".to_string(), 42),
        ],
        "sites must be resolved to (.c file, line) before the SourceMaps are dropped"
    );
}

/// 010 contract 14: one global identity per header and per macro, across TUs.
#[test]
fn shared_header_and_macro_have_one_identity() {
    let mut interner = GlobalInterner::new();
    let mut index = CookedExpansionIndex::new();
    cook_tu(&mut interner, &mut index, "a.c", 10);
    cook_tu(&mut interner, &mut index, "b.c", 20);

    let a = interner.lookup_macro("vppinfra/vec.h", "ADD1").unwrap();
    let b = interner.lookup_macro("vppinfra/vec.h", "ADD1").unwrap();
    assert_eq!(a, b, "same macro across TUs is one entity");
    assert_eq!(index.sites(a).count(), 2);
    assert_eq!(
        interner.macro_count(),
        1,
        "the header's macro must not be interned once per TU"
    );
}

/// 010 contract 15: same name, different definition line ⇒ different entities.
#[test]
fn redefinition_at_a_new_line_is_a_distinct_entity() {
    let mut sm = SourceMap::new();
    let src = "#define M 1\n#undef M\n#define M 2\n";
    let f = sm.add_file("h.h", src);
    let base = sm.file(f).start_pos.0;
    let at = |off: u32, len: u32| {
        Span::new(
            BytePos(base + off),
            BytePos(base + off + len),
            ExpnCtx::ROOT,
        )
    };
    let first = sm.add_macro("M", at(8, 1), at(10, 1));
    let second = sm.add_macro("M", at(29, 1), at(31, 1));
    sm.add_expansion(
        ExpnCtx::ROOT,
        Some(first),
        at(8, 1),
        at(8, 1),
        vec![],
        ExpnKind::ObjectLike,
    );
    sm.add_expansion(
        ExpnCtx::ROOT,
        Some(second),
        at(29, 1),
        at(29, 1),
        vec![],
        ExpnKind::ObjectLike,
    );

    let mut interner = GlobalInterner::new();
    let mut index = CookedExpansionIndex::new();
    index.cook_tu(&mut interner, &sm);
    assert_eq!(
        interner.macro_count(),
        2,
        "a redefinition at a different line is a different macro"
    );
}

/// 010 contract 17: cooking is order-independent.
#[test]
fn cooking_is_order_independent() {
    // Distinct macros in distinct headers. With one shared macro from one header the
    // entity numbering is identical in either order *by construction*, so the test
    // could not observe that entity ids were not canonicalized.
    fn cook_distinct(
        interner: &mut GlobalInterner,
        index: &mut CookedExpansionIndex,
        hdr: &str,
        mac: &str,
        c_path: &str,
        line: u32,
    ) {
        let header = format!("#define {mac} 1\n");
        let mut c_src = String::new();
        for _ in 1..line {
            c_src.push('\n');
        }
        c_src.push_str(&format!("{mac}\n"));

        let mut sm = SourceMap::new();
        let hf = sm.add_file(hdr, header.clone());
        let cf = sm.add_file(c_path, c_src.clone());
        let hb = sm.file(hf).start_pos.0;
        let cb = sm.file(cf).start_pos.0;
        let m = sm.add_macro(
            mac,
            Span::new(
                BytePos(hb + 8),
                BytePos(hb + 8 + mac.len() as u32),
                ExpnCtx::ROOT,
            ),
            Span::new(
                BytePos(hb + 9 + mac.len() as u32),
                BytePos(hb + 10 + mac.len() as u32),
                ExpnCtx::ROOT,
            ),
        );
        let off = c_src.find(mac).unwrap() as u32;
        let call = Span::new(
            BytePos(cb + off),
            BytePos(cb + off + mac.len() as u32),
            ExpnCtx::ROOT,
        );
        sm.add_expansion(
            ExpnCtx::ROOT,
            Some(m),
            call,
            call,
            vec![],
            ExpnKind::ObjectLike,
        );
        index.cook_tu(interner, &sm);
    }

    let build = |reverse: bool| {
        let mut interner = GlobalInterner::new();
        let mut index = CookedExpansionIndex::new();
        let tus = [("p.h", "PPP", "x.c", 5u32), ("q.h", "QQQ", "y.c", 9)];
        let order: Vec<_> = if reverse {
            tus.iter().rev().copied().collect()
        } else {
            tus.to_vec()
        };
        for (h, mac, p, l) in order {
            cook_distinct(&mut interner, &mut index, h, mac, p, l);
        }
        index.finalize(&mut interner);
        // A **full dump**: every path and every entity's every site, raw. Comparing a
        // sorted projection of one macro would sort away the property under test.
        let mut paths = Vec::new();
        let mut n = 0;
        while let Some(p) = interner.try_path(chiero_span::GlobalFileId(n)) {
            paths.push(p.display().to_string());
            n += 1;
        }
        let dump: Vec<(String, Vec<chiero_span::CookedSite>)> = ["PPP", "QQQ"]
            .iter()
            .map(|mac| {
                let hdr = if *mac == "PPP" { "p.h" } else { "q.h" };
                let e = interner.lookup_macro(hdr, mac).unwrap();
                (format!("{e:?}"), index.sites(e).cloned().collect())
            })
            .collect();
        (dump, paths)
    };
    assert_eq!(build(false), build(true));
}

/// 010 contract 16: the invariant that keeps §6.2 true as the code evolves.
///
/// No long-lived structure may hold a per-TU id. Checked structurally here; the
/// mechanical grep lives in `xtask`.
#[test]
fn cooked_types_hold_no_per_tu_ids() {
    // NOTE: the "mechanical grep in xtask" this used to promise does not exist. The
    // structural check below is the whole of contract 16's enforcement today.
    // An exhaustive destructuring: adding a field to `CookedSite` fails to compile here
    // until it is considered. A `Send + Sync + 'static` bound cannot serve this purpose
    // — `ExpnCtx`, `MacroId` and `FileId` all satisfy it, so such a check passes even
    // with a dangling per-TU handle bolted onto the struct.
    #[allow(clippy::no_effect_underscore_binding)]
    fn field_audit(s: &chiero_span::CookedSite) {
        let chiero_span::CookedSite {
            file: _,
            line: _,
            depth: _,
            config: _,
        } = s;
    }

    let mut interner = GlobalInterner::new();
    let mut index = CookedExpansionIndex::new();
    cook_tu(&mut interner, &mut index, "a.c", 1);
    let e = interner.lookup_macro("vppinfra/vec.h", "ADD1").unwrap();
    let site = index.sites(e).next().unwrap();
    field_audit(site);
    // A `CookedSite`'s file is a *global* id, resolvable without any SourceMap.
    assert_eq!(interner.path(site.file).display().to_string(), "a.c");
    assert_eq!(site.depth, 0);
    // An upper bound, not an equality: 010 §6.2 still owes `func: Option<FuncKey>`, and
    // pinning the exact size would make adding it read as a regression. `config` arrived
    // in wave 106 (contract 19) and cost 8 bytes without disturbing this. The point is
    // that no *per-TU handle* has crept in.
    assert!(std::mem::size_of::<chiero_span::CookedSite>() <= 32);
}

/// Depth must be *measured*, not hard-coded — a nested expansion has depth 1.
#[test]
fn depth_reflects_nesting() {
    let mut sm = SourceMap::new();
    let src = "#define INNER 1\n#define OUTER INNER\nOUTER\n";
    let f = sm.add_file("d.c", src);
    let base = sm.file(f).start_pos.0;
    let at = |needle: &str, nth: usize| {
        let off = src.match_indices(needle).nth(nth).unwrap().0 as u32;
        Span::new(
            BytePos(base + off),
            BytePos(base + off + needle.len() as u32),
            ExpnCtx::ROOT,
        )
    };
    let inner = sm.add_macro("INNER", at("INNER", 0), at("1", 0));
    let outer_m = sm.add_macro("OUTER", at("OUTER", 0), at("INNER", 1));

    let use_outer = at("OUTER", 1);
    let e1 = sm.add_expansion(
        ExpnCtx::ROOT,
        Some(outer_m),
        use_outer,
        use_outer,
        vec![],
        ExpnKind::ObjectLike,
    );
    let inner_call = Span::new(at("INNER", 1).lo, at("INNER", 1).hi, e1);
    sm.add_expansion(
        e1,
        Some(inner),
        inner_call,
        inner_call,
        vec![],
        ExpnKind::ObjectLike,
    );

    let mut interner = GlobalInterner::new();
    let mut index = CookedExpansionIndex::new();
    index.cook_tu(&mut interner, &sm);

    let oe = interner.lookup_macro("d.c", "OUTER").unwrap();
    let ie = interner.lookup_macro("d.c", "INNER").unwrap();
    assert_eq!(index.sites(oe).next().unwrap().depth, 0, "written directly");
    assert_eq!(
        index.sites(ie).next().unwrap().depth,
        1,
        "reached only through OUTER's body"
    );
}

/// One line, three expansions of one macro: three *events*, one *site*. 010 §6.3's
/// whole justification is that the index is bounded by sites, not events.
#[test]
fn repeated_expansion_on_one_line_is_one_site() {
    let mut sm = SourceMap::new();
    let src = "#define M 1\nint z = M + M + M;\n";
    let f = sm.add_file("r.c", src);
    let base = sm.file(f).start_pos.0;
    let at = |off: u32, len: u32| {
        Span::new(
            BytePos(base + off),
            BytePos(base + off + len),
            ExpnCtx::ROOT,
        )
    };
    let m = sm.add_macro("M", at(8, 1), at(10, 1));
    for nth in 1..=3 {
        let off = src.match_indices('M').nth(nth).unwrap().0 as u32;
        let call = Span::new(BytePos(base + off), BytePos(base + off + 1), ExpnCtx::ROOT);
        sm.add_expansion(
            ExpnCtx::ROOT,
            Some(m),
            call,
            call,
            vec![],
            ExpnKind::ObjectLike,
        );
    }

    let mut interner = GlobalInterner::new();
    let mut index = CookedExpansionIndex::new();
    index.cook_tu(&mut interner, &sm);
    let e = interner.lookup_macro("r.c", "M").unwrap();
    assert_eq!(
        index.sites(e).count(),
        1,
        "three events on one line are one site"
    );
}

/// The dedup key is `(file, line)` — **both parts**. Deduping on line alone would merge
/// `ip4_forward.c:900` with `ip6_forward.c:900`, and a dropped site is a false negative
/// in test selection. Deduping on file alone would collapse every site in a file to one.
#[test]
fn dedup_key_is_file_and_line() {
    let mut sm = SourceMap::new();
    let hdr = "#define M 1\n";
    let h = sm.add_file("m.h", hdr);
    let hb = sm.file(h).start_pos.0;
    let m = sm.add_macro(
        "M",
        Span::new(BytePos(hb + 8), BytePos(hb + 9), ExpnCtx::ROOT),
        Span::new(BytePos(hb + 10), BytePos(hb + 11), ExpnCtx::ROOT),
    );

    // Same line number, different files.
    for name in ["a.c", "b.c"] {
        let src = "\nM\n";
        let f = sm.add_file(name, src);
        let base = sm.file(f).start_pos.0;
        let call = Span::new(BytePos(base + 1), BytePos(base + 2), ExpnCtx::ROOT);
        sm.add_expansion(
            ExpnCtx::ROOT,
            Some(m),
            call,
            call,
            vec![],
            ExpnKind::ObjectLike,
        );
    }
    // Two different lines in one file.
    let f = sm.add_file("c.c", "M\nM\n");
    let base = sm.file(f).start_pos.0;
    for off in [0u32, 2] {
        let call = Span::new(BytePos(base + off), BytePos(base + off + 1), ExpnCtx::ROOT);
        sm.add_expansion(
            ExpnCtx::ROOT,
            Some(m),
            call,
            call,
            vec![],
            ExpnKind::ObjectLike,
        );
    }

    let mut interner = GlobalInterner::new();
    let mut index = CookedExpansionIndex::new();
    index.cook_tu(&mut interner, &sm);
    let e = interner.lookup_macro("m.h", "M").unwrap();
    let mut got: Vec<(String, u32)> = index
        .sites(e)
        .map(|s| (interner.path(s.file).display().to_string(), s.line))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("a.c".to_string(), 2),
            ("b.c".to_string(), 2),
            ("c.c".to_string(), 1),
            ("c.c".to_string(), 2),
        ],
        "same line in two files are two sites; two lines in one file are two sites"
    );
}

/// A site reached at two depths keeps the *shallowest*, which is the path a reader
/// should be shown first. `max` would pass a test that only checks "some depth".
#[test]
fn a_site_reached_at_two_depths_keeps_the_shallowest() {
    let mut sm = SourceMap::new();
    let src = "#define IN 1\n#define OUT IN\nIN\nOUT\n";
    let f = sm.add_file("d.c", src);
    let base = sm.file(f).start_pos.0;
    let at = |needle: &str, nth: usize| {
        let off = src.match_indices(needle).nth(nth).unwrap().0 as u32;
        Span::new(
            BytePos(base + off),
            BytePos(base + off + needle.len() as u32),
            ExpnCtx::ROOT,
        )
    };
    let inn = sm.add_macro("IN", at("IN", 0), at("1", 0));
    let out = sm.add_macro("OUT", at("OUT", 0), at("IN", 1));

    // Direct use of IN at line 3: depth 0.
    let d = at("IN", 2);
    sm.add_expansion(ExpnCtx::ROOT, Some(inn), d, d, vec![], ExpnKind::ObjectLike);
    // Use of OUT at line 4, which expands IN: depth 1 — but at a different line, so a
    // separate site. Re-expand IN at line 3 through OUT to force the same-line case.
    let e1 = sm.add_expansion(ExpnCtx::ROOT, Some(out), d, d, vec![], ExpnKind::ObjectLike);
    let nested = Span::new(at("IN", 1).lo, at("IN", 1).hi, e1);
    sm.add_expansion(e1, Some(inn), nested, nested, vec![], ExpnKind::ObjectLike);

    let mut interner = GlobalInterner::new();
    let mut index = CookedExpansionIndex::new();
    index.cook_tu(&mut interner, &sm);
    let ie = interner.lookup_macro("d.c", "IN").unwrap();
    let sites: Vec<_> = index.sites(ie).collect();
    assert_eq!(sites.len(), 1, "one line, one site");
    assert_eq!(sites[0].depth, 0, "shallowest wins");
}

/// A builtin or `-D` macro has no source location, and must not be attributed to
/// whichever file occupies offset 0.
#[test]
fn builtin_macros_are_not_attributed_to_offset_zero() {
    let mut sm = SourceMap::new();
    let f = sm.add_file("vppinfra/vec.h", "#define REAL 1\nREAL\n");
    let base = sm.file(f).start_pos.0;
    let at = |off: u32, len: u32| {
        Span::new(
            BytePos(base + off),
            BytePos(base + off + len),
            ExpnCtx::ROOT,
        )
    };
    sm.add_macro("REAL", at(8, 4), at(13, 1));
    // A `-D` macro: DUMMY span, no location.
    let builtin = sm.add_macro("CLIB_DEBUG", Span::DUMMY, Span::DUMMY);
    assert_eq!(
        sm.macro_info(builtin).unwrap().def_file,
        None,
        "a DUMMY definition span must not resolve to a real file"
    );

    let mut interner = GlobalInterner::new();
    let mut index = CookedExpansionIndex::new();
    index.cook_tu(&mut interner, &sm);
    assert!(
        interner
            .lookup_macro("vppinfra/vec.h", "CLIB_DEBUG")
            .is_none(),
        "the builtin must not be filed under the file at offset 0"
    );
    let _ = index;
}

/// An expansion with a synthesized call site (`##`, `_Pragma`) is counted, not
/// attributed to offset 0 — and `dropped()` is what makes that visible.
#[test]
fn synthesized_call_sites_are_counted_not_fabricated() {
    let mut sm = SourceMap::new();
    let f = sm.add_file("vppinfra/vec.h", "#define P 1\n");
    let base = sm.file(f).start_pos.0;
    let at = |off: u32, len: u32| {
        Span::new(
            BytePos(base + off),
            BytePos(base + off + len),
            ExpnCtx::ROOT,
        )
    };
    let m = sm.add_macro("P", at(8, 1), at(10, 1));
    sm.add_expansion(
        ExpnCtx::ROOT,
        Some(m),
        Span::DUMMY,
        Span::DUMMY,
        vec![],
        ExpnKind::Paste,
    );

    let mut interner = GlobalInterner::new();
    let mut index = CookedExpansionIndex::new();
    index.cook_tu(&mut interner, &sm);
    let e = interner.lookup_macro("vppinfra/vec.h", "P").unwrap();
    assert_eq!(index.sites(e).count(), 0, "no fabricated site");
    assert_eq!(index.dropped(), 1, "dropped must be counted, not silent");
}

/// Two files sharing a basename, each defining a same-named macro. An interner keyed on
/// the basename, or a lookup ignoring its file argument, would conflate them.
#[test]
fn same_basename_in_different_directories_is_distinct() {
    let mut sm = SourceMap::new();
    for dir in ["a", "b"] {
        let src = "#define DUP 1\nDUP\n";
        let f = sm.add_file(format!("{dir}/h.h"), src);
        let base = sm.file(f).start_pos.0;
        let at = |off: u32, len: u32| {
            Span::new(
                BytePos(base + off),
                BytePos(base + off + len),
                ExpnCtx::ROOT,
            )
        };
        let m = sm.add_macro("DUP", at(8, 3), at(12, 1));
        let call = at(14, 3);
        sm.add_expansion(
            ExpnCtx::ROOT,
            Some(m),
            call,
            call,
            vec![],
            ExpnKind::ObjectLike,
        );
    }

    let mut interner = GlobalInterner::new();
    let mut index = CookedExpansionIndex::new();
    index.cook_tu(&mut interner, &sm);

    let a = interner.lookup_macro("a/h.h", "DUP").expect("a");
    let b = interner.lookup_macro("b/h.h", "DUP").expect("b");
    assert_ne!(a, b, "same name in different files is not one entity");
    assert_eq!(index.sites(a).count(), 1);
    assert_eq!(index.sites(b).count(), 1);
    assert_eq!(interner.macro_count(), 2);
}

/// Contract 14, spelled differently. Real include resolution yields `vec.h`, `./vec.h`
/// and `x/../vec.h` for one file; without normalization those are three ids and the
/// contract holds only for tests that spell the path identically.
#[test]
fn differently_spelled_paths_intern_to_one_id() {
    let mut i = GlobalInterner::new();
    let a = i.intern_file(std::path::Path::new("vppinfra/vec.h"));
    let b = i.intern_file(std::path::Path::new("./vppinfra/vec.h"));
    let c = i.intern_file(std::path::Path::new("vppinfra/../vppinfra/vec.h"));
    assert_eq!(a, b);
    assert_eq!(a, c);
}

/// An entity with no expansion sites answers empty rather than panicking — a macro
/// that is defined and never used is entirely normal.
#[test]
fn unused_macro_has_no_sites() {
    let mut sm = SourceMap::new();
    let f = sm.add_file("h.h", "#define UNUSED 1\n");
    let base = sm.file(f).start_pos.0;
    let sp = Span::new(BytePos(base + 8), BytePos(base + 14), ExpnCtx::ROOT);
    sm.add_macro("UNUSED", sp, sp);

    let mut interner = GlobalInterner::new();
    let mut index = CookedExpansionIndex::new();
    index.cook_tu(&mut interner, &sm);

    let e = interner.lookup_macro("h.h", "UNUSED").expect("interned");
    assert_eq!(index.sites(e).count(), 0);
}
