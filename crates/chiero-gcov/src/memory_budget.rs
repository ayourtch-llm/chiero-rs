//! **030 contract 11: the index over 1M lines × 5000 tests stays under a documented budget.**
//!
//! > Memory: an index over 1M lines × 5000 tests with realistic sparsity stays under a documented
//! > budget, asserted by a benchmark test.
//!
//! # The budget, and where it comes from
//!
//! **256 MiB.** It is not arbitrary. A correct representation needs, per line, a key of two
//! integers — an interned file and a line number — and a bitmap of the tests that reached it:
//!
//! ```text
//! keys      1M x (u32 file + u32 line)                        =   8 MiB
//! bitmaps   1M x ~32 bytes for a handful of ids out of 5000   =  32 MiB
//! counts    1M x u64                                          =   8 MiB
//! slack     hash tables, growth, the hot lines' dense bitmaps  ~ 100 MiB
//! ```
//!
//! So a representation that meets the contract has room to spare inside 256 MiB, and one that
//! does not misses by a multiple rather than by a margin. That is the property a budget wants:
//! passing it should mean the design is right, not that the constant was chosen kindly.
//!
//! # Measured, not modelled
//!
//! A `heap_bytes()` method summing the structures this crate knows it allocated would measure
//! *the model*, and would silently omit whatever the model forgot — an `IndexMap`'s index table,
//! a `String`'s capacity beyond its length, the growth slack in every `Vec`. So this counts real
//! allocations with a wrapping global allocator, which cannot forget anything and needs no
//! dependency.
//!
//! # Why this is `#[ignore]`
//!
//! It builds a million-line index and holds it. That is minutes and hundreds of megabytes in a
//! debug build, which is not something to put in the path of every `cargo test`. Run it with:
//!
//! ```text
//! cargo test --release -p chiero-gcov --test memory_budget -- --ignored --nocapture
//! ```
//!
//! `--nocapture` because the number it prints is the point: the contract is a budget, and a
//! budget that is only ever asserted is a budget nobody is watching approach its limit.
//!
//! # Why this lives in `src/` rather than `tests/`
//!
//! Every other test in this crate is an integration test, deliberately: they are about what a
//! caller can observe. This one is about the *representation* — it exists to fail when
//! `TestBitmap` is the wrong shape, and passing it must not require widening the public API with
//! a "record a line directly" method that only a benchmark would ever call.

use crate::{TestId, Variant};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Live bytes, incremented on allocation and decremented on free.
static LIVE: AtomicUsize = AtomicUsize::new(0);

struct Counting;

// SAFETY: every method forwards to `System` unchanged and only adds bookkeeping around it, so
// the allocator contract is exactly `System`'s.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_add(new_size, Ordering::Relaxed);
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

const BUDGET: usize = 256 * 1024 * 1024;
const FILES: u32 = 5_000;
const LINES_PER_FILE: u32 = 200;
const TESTS: u32 = 5_000;

/// How many tests reach a line, modelled on what a real tree looks like rather than on a uniform
/// draw.
///
/// Most lines sit in one `.c` file and are reached by the handful of tests that exercise it. A
/// header's lines are reached by everything that includes it, and VPP's hot layer — `vec.h`,
/// `pool.h`, `buffer_funcs.h` — is `static inline` functions included nearly everywhere. A
/// uniform sparsity would understate the memory badly, because it is exactly those dense lines
/// that a per-line `Vec<TestId>` handles worst.
fn tests_on_line(i: u64) -> u32 {
    match i % 1000 {
        0 => 2000,    // 0.1% — a hot header line, reached by most of the suite
        1..=99 => 40, // 9.9% — a shared utility
        _ => 4,       // the rest — one test file's worth
    }
}

/// A deterministic spread of test ids, so the benchmark measures the same thing every run.
///
/// **No `rand` and no `HashMap`.** 001 §5 forbids unordered containers on output paths, and a
/// benchmark whose numbers move between runs cannot be compared with the last one — which is the
/// only thing a budget test is for.
fn spread(seed: u64, n: u32) -> Vec<TestId> {
    let mut out = Vec::with_capacity(n as usize);
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    for _ in 0..n {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        out.push(TestId(((x >> 33) as u32) % TESTS));
    }
    out.sort_unstable_by_key(|t| t.0);
    out.dedup();
    out
}

#[test]
#[ignore = "builds a 1M-line index; run with --release --ignored"]
fn an_index_over_a_million_lines_stays_within_budget() {
    let before = LIVE.load(Ordering::Relaxed);

    let mut idx = crate::CoverageIndex::default();
    let mut lines = 0u64;
    for f in 0..FILES {
        let file = format!("src/vppinfra/generated_{f}.c");
        for l in 1..=LINES_PER_FILE {
            let n = tests_on_line(lines);
            for t in spread(lines, n) {
                idx.add_line_for_variant(t, &Variant::None, file.clone(), l, 1);
            }
            lines += 1;
        }
    }

    let used = LIVE.load(Ordering::Relaxed).saturating_sub(before);
    println!(
        "030 contract 11: {lines} lines x {TESTS} tests -> {} MiB live ({} MiB budget)",
        used / (1024 * 1024),
        BUDGET / (1024 * 1024)
    );

    // Read something back, so the index cannot be optimised away and so a representation that
    // saved memory by losing answers fails here rather than passing quietly.
    assert_eq!(idx.lines_of("src/vppinfra/generated_0.c").len(), 200);
    assert!(
        idx.tests_for_line("src/vppinfra/generated_0.c", 1)
            .is_some()
    );

    assert!(
        used <= BUDGET,
        "the index needs {} MiB against a {} MiB budget — 030 §5's roaring bitmaps and an \
         interned file table are what close that gap, and neither changes an answer",
        used / (1024 * 1024),
        BUDGET / (1024 * 1024)
    );
}
