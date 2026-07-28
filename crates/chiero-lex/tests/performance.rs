//! Covers: 011 contracts 11, 13.

use chiero_lex::{LexConfig, LexSession};
use chiero_span::SourceMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn source_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            source_files(&path, out);
        } else if matches!(path.extension().and_then(|x| x.to_str()), Some("c" | "h")) {
            out.push(path);
        }
    }
}

#[test]
fn every_vpp_source_file_lexes_without_panicking() {
    let root = Path::new("/home/ubuntu/vpp/src");
    if !root.exists() {
        eprintln!("VPP corpus unavailable; skipping external-corpus assertion");
        return;
    }
    let mut paths = Vec::new();
    source_files(root, &mut paths);
    paths.sort();
    assert!(!paths.is_empty(), "the VPP corpus must contain C sources");
    let session = LexSession::new();
    for path in paths {
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut map = SourceMap::new();
        let file = map.add_file(&path, src);
        let _ = session.lex(&map, file, LexConfig::default());
    }
}

#[test]
fn warm_header_cache_avoids_lexing_work() {
    let mut map = SourceMap::new();
    let file = map.add_file("large.h", "#define X(a) ((a) + 1)\n".repeat(20_000));
    let session = LexSession::new();
    let cold_start = Instant::now();
    let cold = session.lex_cached(&map, file, LexConfig::default());
    let cold_elapsed = cold_start.elapsed();
    let warm_start = Instant::now();
    let warm = session.lex_cached(&map, file, LexConfig::default());
    let warm_elapsed = warm_start.elapsed();
    assert!(std::sync::Arc::ptr_eq(&cold, &warm));
    assert_eq!(session.cache_stats(), (1, 1));
    assert!(
        warm_elapsed.saturating_mul(20) <= cold_elapsed,
        "warm {warm_elapsed:?}, cold {cold_elapsed:?}"
    );
}

/// Run explicitly on the reference machine: debug instrumentation is not a meaningful
/// throughput measurement.
///
/// Manual throughput evidence only. Ignored tests do not carry contract coverage credit
/// under 070 §4, so this comment intentionally does not cite a contract number.
#[test]
#[ignore = "performance gate: run with cargo test --release -- --ignored"]
fn fifty_megabytes_lex_above_100_mb_per_second() {
    let line = "unsigned long value = 0x1e+2 + other_value; /* comment */\n";
    let src = line.repeat(50_000_000 / line.len() + 1);
    let mut map = SourceMap::new();
    let file = map.add_file("blob.c", src);
    let started = Instant::now();
    let _ = LexSession::new().lex(&map, file, LexConfig::default());
    let elapsed = started.elapsed();
    let rate = map.file(file).src().len() as f64 / elapsed.as_secs_f64();
    assert!(
        rate >= 100_000_000.0,
        "{:.1} MB/s is below the contract",
        rate / 1_000_000.0
    );
}
