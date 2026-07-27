//! The global source space and position→location lookup.
//!
//! Covers **010 contract 12** (`BytePos` → `FileId` lookup agrees with a linear scan
//! over every file boundary) and the `SourceFile`/`SourceMap` shape in 010 §3.
//!
//! Files are laid out consecutively in one global `BytePos` space, which is what lets
//! `Span` be 12 bytes with no `FileId` field (010 contract 1). Every boundary case in
//! that layout is a place where an off-by-one silently misattributes a token to the
//! wrong file — so the boundaries are tested exhaustively rather than sampled.

use chiero_span::{BytePos, SourceMap};

#[test]
fn empty_map_has_no_files() {
    let sm = SourceMap::new();
    assert_eq!(sm.files().count(), 0);
    assert!(sm.lookup_file(BytePos(0)).is_none());
}

#[test]
fn files_occupy_disjoint_consecutive_ranges() {
    let mut sm = SourceMap::new();
    let a = sm.add_file("a.c", "abc");
    let b = sm.add_file("b.c", "de");
    let c = sm.add_file("c.c", "fghi");

    // Ranges are disjoint, consecutive, and sized to the source.
    assert_eq!(sm.file(a).byte_len(), 3);
    assert_eq!(sm.file(b).byte_len(), 2);
    assert_eq!(sm.file(c).byte_len(), 4);
    assert!(sm.file(a).start_pos < sm.file(b).start_pos);
    assert!(sm.file(b).start_pos < sm.file(c).start_pos);
    assert!(sm.file(a).end_pos() <= sm.file(b).start_pos);
    assert!(sm.file(b).end_pos() <= sm.file(c).start_pos);
}

/// 010 contract 12: binary-search lookup must agree with a linear scan at **every**
/// byte position, including the exact boundaries between files.
#[test]
fn lookup_agrees_with_linear_scan_at_every_position() {
    let mut sm = SourceMap::new();
    let contents: Vec<String> = (0..100)
        .map(|i| format!("file {i} contents\nsecond line\n"))
        .collect();
    let ids: Vec<_> = contents
        .iter()
        .enumerate()
        .map(|(i, s)| sm.add_file(format!("f{i}.c"), s.clone()))
        .collect();

    let last_end = sm.file(*ids.last().unwrap()).end_pos();
    for pos in 0..last_end.0 {
        let p = BytePos(pos);
        let via_search = sm.lookup_file(p);
        let via_scan = ids.iter().copied().find(|&id| {
            let f = sm.file(id);
            f.start_pos <= p && p < f.end_pos()
        });
        assert_eq!(via_search, via_scan, "disagreement at {p:?}");
    }
    // One past the last file belongs to nothing.
    assert!(sm.lookup_file(last_end).is_none());
}

/// An empty source file must not swallow the position that belongs to its neighbour.
/// Zero-length files are real: an empty header, or one whose content is entirely
/// conditioned out.
#[test]
fn empty_files_claim_no_positions() {
    let mut sm = SourceMap::new();
    let a = sm.add_file("a.c", "xy");
    let empty = sm.add_file("empty.h", "");
    let b = sm.add_file("b.c", "z");

    assert_eq!(sm.file(empty).byte_len(), 0);
    assert!(sm.file(empty).is_empty());
    // Every position resolves to a non-empty file.
    assert_eq!(sm.lookup_file(sm.file(a).start_pos), Some(a));
    assert_eq!(sm.lookup_file(sm.file(b).start_pos), Some(b));
    for pos in 0..sm.file(b).end_pos().0 {
        assert_ne!(sm.lookup_file(BytePos(pos)), Some(empty));
    }
}

/// Line and column are 1-based, which is what every compiler and editor expects and
/// what diagnostics render. Column counts bytes, not characters — matching gcov and
/// gcc, which is what the coverage join needs (030 §1).
#[test]
fn line_and_column_are_one_based() {
    let mut sm = SourceMap::new();
    let f = sm.add_file("t.c", "ab\ncd\n\nefg");
    let base = sm.file(f).start_pos.0;
    let loc = |off: u32| sm.lookup_loc(BytePos(base + off)).expect("in range");

    assert_eq!((loc(0).line, loc(0).col), (1, 1), "'a'");
    assert_eq!((loc(1).line, loc(1).col), (1, 2), "'b'");
    assert_eq!((loc(2).line, loc(2).col), (1, 3), "the newline itself");
    assert_eq!((loc(3).line, loc(3).col), (2, 1), "'c'");
    assert_eq!((loc(6).line, loc(6).col), (3, 1), "the empty line");
    assert_eq!((loc(7).line, loc(7).col), (4, 1), "'e'");
    assert_eq!((loc(9).line, loc(9).col), (4, 3), "'g'");
    assert_eq!(loc(0).file, f);
}

/// A file that does not end in a newline still has its last line addressable — a
/// surprisingly common source of off-by-one panics in line tables.
#[test]
fn file_without_trailing_newline() {
    let mut sm = SourceMap::new();
    let f = sm.add_file("t.c", "one\ntwo");
    let base = sm.file(f).start_pos.0;
    let last = sm.lookup_loc(BytePos(base + 6)).unwrap();
    assert_eq!((last.line, last.col), (2, 3));
    assert_eq!(sm.file(f).line_count(), 2);
}

/// `\r\n` must not produce a phantom line. VPP is Unix-only but headers arrive from
/// elsewhere, and a phantom line would shift every subsequent coverage attribution.
#[test]
fn crlf_does_not_create_phantom_lines() {
    let mut sm = SourceMap::new();
    let f = sm.add_file("t.c", "a\r\nb\r\n");
    assert_eq!(sm.file(f).line_count(), 2);
    let base = sm.file(f).start_pos.0;
    assert_eq!(sm.lookup_loc(BytePos(base + 3)).unwrap().line, 2);
}

/// Retrieving the text of a span is what diagnostics and the lexer round-trip
/// (010 contract 11) both need.
#[test]
fn span_text_is_recoverable() {
    let mut sm = SourceMap::new();
    let f = sm.add_file("t.c", "int x = 42;");
    let base = sm.file(f).start_pos.0;
    let sp = chiero_span::Span::new(
        BytePos(base + 8),
        BytePos(base + 10),
        chiero_span::ExpnCtx::ROOT,
    );
    assert_eq!(sm.span_text(sp), Some("42"));
}

/// A span crossing a file boundary is malformed and must be rejected rather than
/// returning bytes spliced from two files.
#[test]
fn span_crossing_a_file_boundary_is_rejected() {
    let mut sm = SourceMap::new();
    let a = sm.add_file("a.c", "abc");
    let _b = sm.add_file("b.c", "def");
    let sp = chiero_span::Span::new(
        sm.file(a).start_pos,
        BytePos(sm.file(a).end_pos().0 + 2),
        chiero_span::ExpnCtx::ROOT,
    );
    assert_eq!(sm.span_text(sp), None);
}

/// Adding the same path twice yields two distinct files. The preprocessor may read a
/// header more than once under different configurations, and merging them would make
/// one configuration's spans point into another's text.
#[test]
fn same_path_added_twice_is_two_files() {
    let mut sm = SourceMap::new();
    let a = sm.add_file("h.h", "one");
    let b = sm.add_file("h.h", "two");
    assert_ne!(a, b);
    assert_eq!(sm.file(a).src(), "one");
    assert_eq!(sm.file(b).src(), "two");
}
