//! **030 contract 17: `GCOV_PREFIX_STRIP`, computed rather than configured.**
//!
//! > `GCOV_PREFIX_STRIP` is computed such that a build in `/build/vpp/src` writes per-test trees
//! > with no collisions across 100 tests (verified on a fixture tree).
//!
//! Per-test coverage needs each test's run to write into its own tree (030 §6), which libgcov does
//! by taking the `.gcda` path baked in at compile time, removing `GCOV_PREFIX_STRIP` leading
//! directory components, and re-rooting the rest under `GCOV_PREFIX`. Both ways of getting the
//! number wrong are silent:
//!
//! - **too small** and every tree mirrors the whole absolute build path, so `/cov/7/` contains
//!   `home/ubuntu/vpp/build/...` — deep, and different on every machine;
//! - **too large** and the components that told two objects apart are gone, so
//!   `vlib/CMakeFiles/main.c.gcda` and `vnet/CMakeFiles/main.c.gcda` become one file. Counters
//!   *accumulate* into an existing `.gcda` (030 §6), so a collision does not fail — it silently
//!   adds one test's counts to another's, and the index that comes out is wrong in the direction
//!   that makes tests look covered.
//!
//! # Measured against libgcov, not transcribed
//!
//! A program compiled at `<dir>/build/vpp/src/sub/p.c` and run with `GCOV_PREFIX=<out>`:
//!
//! ```text
//! GCOV_PREFIX_STRIP=0    <out>/<dir>/build/vpp/src/sub/p.gcda     the whole path, mirrored
//! GCOV_PREFIX_STRIP=5    <out>/pfx/build/vpp/src/sub/p.gcda
//! GCOV_PREFIX_STRIP=9    <out>/sub/p.gcda                         `<dir>/build/vpp/src` is 9
//! GCOV_PREFIX_STRIP=10   <out>/p.gcda                             one component too many
//! GCOV_PREFIX_STRIP=100  <out>/p.gcda                             it saturates, it does not fail
//! ```
//!
//! So the count is of *components stripped from the front*, the leading `/` is not one of them,
//! and over-stripping saturates at the basename rather than erroring — which is exactly why a
//! hand-configured number goes wrong quietly.

use chiero_gcov::prefix::{reroot, strip_for};
use std::path::{Path, PathBuf};

/// The build directory 030 contract 17 names.
const BUILD: &str = "/build/vpp/src";

/// A fixture tree: object paths as cmake lays them out, including two pairs that differ **only**
/// in a directory component — which is what a too-large strip destroys.
fn objects() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for dir in [
        "vlib/CMakeFiles/vlib.dir",
        "vnet/CMakeFiles/vnet.dir",
        "vppinfra/CMakeFiles/vppinfra.dir",
        "plugins/dpdk/CMakeFiles/dpdk_plugin.dir",
    ] {
        for name in ["main.c", "node.c", "format.c", "init.c", "cli.c"] {
            v.push(PathBuf::from(format!("{BUILD}/{dir}/{name}.gcda")));
        }
    }
    // Two builds of one source under different march variants, which VPP really does.
    for march in ["", "_x86_64_v3", "_x86_64_v4"] {
        v.push(PathBuf::from(format!(
            "{BUILD}/vppinfra/CMakeFiles/vppinfra{march}.dir/vector.c.gcda"
        )));
    }
    v
}

fn tree_for(test: u32, strip: u32) -> Vec<PathBuf> {
    let prefix = PathBuf::from(format!("/cov/{test}"));
    objects().iter().map(|o| reroot(&prefix, strip, o)).collect()
}

/// **Contract 17.** 100 tests, every object, no two destinations equal.
#[test]
fn a_hundred_tests_write_without_a_single_collision() {
    let strip = strip_for(Path::new(BUILD));
    let mut all: Vec<PathBuf> = Vec::new();
    for test in 0..100 {
        all.extend(tree_for(test, strip));
    }
    let total = all.len();
    all.sort();
    all.dedup();
    assert_eq!(
        all.len(),
        total,
        "two runs would have written to one `.gcda`, and libgcov *accumulates* into an existing \
         file — so a collision adds one test's counts to another's without failing"
    );
}

/// **The check can see a collision.** A gate that cannot fail proves nothing, and over-stripping
/// is the mistake it exists to catch: one component too many merges the four `main.c.gcda`s.
#[test]
fn one_component_too_many_collides() {
    let strip = strip_for(Path::new(BUILD)) + 1;
    let mut paths = tree_for(0, strip);
    let total = paths.len();
    paths.sort();
    paths.dedup();
    assert!(
        paths.len() < total,
        "stripping past the build directory removes what told two objects apart, and this test \
         exists so that the no-collision assertion above is not vacuous"
    );
}

/// The other failure: too small a strip mirrors the absolute build path into every test's tree.
#[test]
fn the_computed_strip_does_not_mirror_the_build_path() {
    let strip = strip_for(Path::new(BUILD));
    for p in tree_for(7, strip) {
        let s = p.to_string_lossy().into_owned();
        assert!(
            s.starts_with("/cov/7/"),
            "every object lands under its own test's prefix: {s}"
        );
        assert!(
            !s.contains("/build/vpp/src"),
            "the build directory is what the strip removes; leaving it makes every tree deep and \
             machine-specific: {s}"
        );
    }
    // And it does not strip so far that the structure below the build directory is lost.
    let one = reroot(
        &PathBuf::from("/cov/7"),
        strip,
        Path::new("/build/vpp/src/vlib/CMakeFiles/vlib.dir/main.c.gcda"),
    );
    assert_eq!(
        one,
        PathBuf::from("/cov/7/vlib/CMakeFiles/vlib.dir/main.c.gcda")
    );
}

/// The count is of components, and the leading separator is not one of them — measured against
/// libgcov, where a build directory of nine components needs `GCOV_PREFIX_STRIP=9`.
#[test]
fn the_strip_is_the_build_directorys_component_count() {
    assert_eq!(strip_for(Path::new("/build/vpp/src")), 3);
    assert_eq!(strip_for(Path::new("/build/vpp/src/")), 3);
    assert_eq!(strip_for(Path::new("/")), 0);
    assert_eq!(strip_for(Path::new("/one")), 1);
}

/// Over-stripping **saturates** rather than failing, which is measured libgcov behaviour and the
/// reason a wrong number is silent. Pinned so that a rewrite of `reroot` cannot start erroring
/// and hide the collision this crate is meant to prevent.
#[test]
fn stripping_past_the_root_keeps_the_basename() {
    let p = Path::new("/build/vpp/src/vlib/main.c.gcda");
    for strip in [5, 6, 100] {
        assert_eq!(
            reroot(Path::new("/cov/0"), strip, p),
            PathBuf::from("/cov/0/main.c.gcda"),
            "libgcov saturates at the basename; it does not report the mistake"
        );
    }
}
