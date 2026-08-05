//! `GCOV_PREFIX` and `GCOV_PREFIX_STRIP` — where a test's counters land (030 §6).
//!
//! libgcov writes each `.gcda` to the absolute path baked into the object at compile time. To get
//! one coverage set per test, each run is given its own `GCOV_PREFIX`, and `GCOV_PREFIX_STRIP`
//! says how many leading directory components of that compile-time path to drop before re-rooting
//! the rest under it.
//!
//! **The number is computed from the build directory, never configured** (030 §6), because both
//! ways of getting it wrong are silent and one of them corrupts the index:
//!
//! - too small, and every test's tree mirrors the absolute build path — deep, and different on
//!   every machine;
//! - too large, and the components that told two objects apart are gone. `vlib/.../main.c.gcda`
//!   and `vnet/.../main.c.gcda` become one file, and libgcov **accumulates** into an existing
//!   `.gcda` rather than replacing it, so one test's counts are added to another's and nothing
//!   reports a problem. The resulting index says tests covered lines they never ran, which is the
//!   one direction 032 must never be wrong in.
//!
//! # Measured
//!
//! A program compiled at `<dir>/build/vpp/src/sub/p.c` and run under `GCOV_PREFIX=<out>`, on
//! gcc 13.3.0:
//!
//! ```text
//! GCOV_PREFIX_STRIP=0    <out>/<dir>/build/vpp/src/sub/p.gcda
//! GCOV_PREFIX_STRIP=5    <out>/pfx/build/vpp/src/sub/p.gcda
//! GCOV_PREFIX_STRIP=9    <out>/sub/p.gcda            `<dir>/build/vpp/src` is nine components
//! GCOV_PREFIX_STRIP=10   <out>/p.gcda                one too many, and the structure is gone
//! GCOV_PREFIX_STRIP=100  <out>/p.gcda                it saturates; it does not fail
//! ```
//!
//! The leading `/` is not a component, and over-stripping saturates at the basename — libgcov has
//! no way to tell a caller the number was wrong, so nothing downstream will either.

use std::path::{Component, Path, PathBuf};

/// The `GCOV_PREFIX_STRIP` for a build whose objects live under `build_dir`.
///
/// The count of `build_dir`'s own components, so what remains of a compile-time path is exactly
/// its position *within* the build tree: `<build>/vlib/CMakeFiles/vlib.dir/main.c.gcda` becomes
/// `vlib/CMakeFiles/vlib.dir/main.c.gcda`. That is the shallowest re-rooting that still tells
/// every object apart, and telling them apart is the whole requirement — see the module docs for
/// what a collision does.
///
/// A relative `build_dir` counts the same way. It is the wrong thing to pass — the path libgcov
/// strips is absolute — but answering with a plausible number for it would hide that, so the
/// count is of what it was given.
pub fn strip_for(build_dir: &Path) -> u32 {
    build_dir
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count() as u32
}

/// Where libgcov writes `compile_time` given `prefix` and `strip`.
///
/// A model of the measured behaviour, so that a caller can check its own layout — which
/// `tests/prefix_strip.rs` does across 100 tests — without running a hundred instrumented
/// binaries to find out.
///
/// **Saturates rather than failing**, because that is what libgcov does. A `strip` past the end
/// leaves the basename, and every object in the tree becomes the same file.
pub fn reroot(prefix: &Path, strip: u32, compile_time: &Path) -> PathBuf {
    let parts: Vec<Component<'_>> = compile_time
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .collect();
    // The basename always survives: libgcov steps over separators, and the last component is not
    // preceded by one it has not already passed.
    let keep = (strip as usize).min(parts.len().saturating_sub(1));
    let mut out = prefix.to_path_buf();
    for c in &parts[keep..] {
        out.push(c.as_os_str());
    }
    out
}
