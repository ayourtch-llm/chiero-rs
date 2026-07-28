//! Covers: 011 contract 13.

use chiero_pp::{Config, FileLoader, PreprocessorSession};
use std::io;
use std::path::Path;

struct HeaderLoader;

impl FileLoader for HeaderLoader {
    fn load(&mut self, path: &Path) -> io::Result<String> {
        if path == Path::new("common.h") {
            Ok("common_token\n".into())
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "missing"))
        }
    }
}

#[test]
fn repeated_tus_use_the_pipeline_lexer_cache() {
    let session = PreprocessorSession::new();
    let src = "#define F(x) ((x) + 1)\nF(2)\n";
    let first = session.preprocess_str("same.c", src, Config::default());
    let second = session.preprocess_str("same.c", src, Config::default());
    assert_eq!(
        first.token_texts().collect::<Vec<_>>(),
        second.token_texts().collect::<Vec<_>>()
    );
    assert_eq!(
        session.lex_cache_stats(),
        (1, 1),
        "one cold TU and one real pipeline cache hit"
    );
}

#[test]
fn shared_header_cache_relocates_tokens_across_distinct_tus() {
    let session = PreprocessorSession::new();
    let mut loader = HeaderLoader;
    let first = session.preprocess_with_loader(
        Path::new("one.c"),
        "#include \"common.h\"\n",
        Config::default(),
        &mut loader,
    );
    let second = session.preprocess_with_loader(
        Path::new("two.c"),
        "padding_before_header\n#include \"common.h\"\n",
        Config::default(),
        &mut loader,
    );
    assert_eq!(first.token_texts().collect::<Vec<_>>(), ["common_token"]);
    assert_eq!(
        second.token_texts().collect::<Vec<_>>(),
        ["padding_before_header", "common_token"]
    );
    let header_loc = second
        .source_map
        .spelling_loc(second.tokens[1].span)
        .unwrap();
    assert_eq!(
        second.source_map.file(header_loc.file).path(),
        Path::new("common.h")
    );
    assert_eq!(
        session.lex_cache_stats(),
        (1, 3),
        "different roots miss, while the relocated shared header hits"
    );
}
