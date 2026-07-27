//! Covers: 011 contract 13.

use chiero_pp::{Config, PreprocessorSession};

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
