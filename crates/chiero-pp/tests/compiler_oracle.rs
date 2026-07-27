//! Covers: 012 contracts 1, 2, 3, 4, 5, 9, 11, 12, 15, 19.

use chiero_lex::{LexConfig, LexSession, PpTokenKind};
use chiero_pp::{Config, preprocess_str};
use chiero_span::SourceMap;
use std::process::{Command, Stdio};

fn compiler_tokens(compiler: &str, src: &str) -> Vec<String> {
    let mut child = Command::new(compiler)
        .args(["-E", "-P", "-x", "c", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), src.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{compiler} rejected oracle fixture"
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let mut map = SourceMap::new();
    let file = map.add_file("compiler-output.c", text);
    let lexed = LexSession::new().lex(&map, file, LexConfig::default());
    lexed
        .tokens()
        .iter()
        .filter(|token| !matches!(token.kind, PpTokenKind::Eof))
        .map(|token| lexed.text(token).to_owned())
        .collect()
}

#[test]
fn representative_expansion_matches_gcc_and_clang_token_for_token() {
    let src = "#define A B\n#define B 7\n\
               #define cat(a,b) a##b\n\
               #define D(f, a...) f(1, ##a)\n\
               #if defined(A) && 0 && 1/0\nbad\n#else\n\
               A cat(1,2) D(g)\n#endif\n\
               __COUNTER__ __COUNTER__\n";
    let ours: Vec<_> = preprocess_str("oracle.c", src, Config::default())
        .token_texts()
        .map(str::to_owned)
        .collect();
    let gcc = compiler_tokens("gcc", src);
    let clang = compiler_tokens("clang", src);
    assert_eq!(gcc, clang, "the independent compilers must agree first");
    assert_eq!(ours, gcc);
}
