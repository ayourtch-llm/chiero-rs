//! Real-header smoke/regression metric. This is evidence toward owed 012 contract 17,
//! but does not claim `Covers:` credit without the configured compilation database.

use chiero_pp::{Config, FileLoader, PreprocessorSession};
use std::io;
use std::path::{Path, PathBuf};

struct DiskLoader;

impl FileLoader for DiskLoader {
    fn load(&mut self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
}

#[test]
fn required_vppinfra_headers_preprocess_without_panicking() {
    let root = Path::new("/home/ubuntu/vpp/src");
    if !root.exists() {
        eprintln!("VPP checkout unavailable; external smoke metric skipped");
        return;
    }
    let session = PreprocessorSession::new();
    let mut loader = DiskLoader;
    let mut diagnostic_counts = Vec::new();
    for name in ["clib.h", "vec.h", "pool.h", "bitmap.h"] {
        let path = root.join("vppinfra").join(name);
        let source = std::fs::read_to_string(&path).unwrap();
        let config = Config {
            include_paths: vec![root.to_path_buf()],
            system_paths: vec![
                PathBuf::from("/usr/lib/gcc/x86_64-linux-gnu/13/include"),
                PathBuf::from("/usr/local/include"),
                PathBuf::from("/usr/include/x86_64-linux-gnu"),
                PathBuf::from("/usr/include"),
            ],
            ..Config::default()
        };
        let tu = session.preprocess_with_loader(&path, &source, config, &mut loader);
        assert!(
            tu.deps.len() > 1,
            "{name} did not resolve any dependency headers"
        );
        assert!(
            tu.diagnostics.is_empty(),
            "{name} diagnostics: {:?}",
            tu.diagnostics
        );
        diagnostic_counts.push((name, tu.diagnostics.len()));
    }
    eprintln!("VPP header diagnostic metric: {diagnostic_counts:?}");
}
