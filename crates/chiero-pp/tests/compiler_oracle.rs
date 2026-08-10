//! Covers: 012 contracts 1, 2, 3, 4, 5, 9, 11, 12, 15, 19.

use chiero_lex::{LexConfig, LexSession, PpTokenKind};
use chiero_pp::{Config, preprocess_str};
use chiero_span::SourceMap;
use std::process::{Command, Stdio};

fn compiler_tokens(compiler: &str, src: &str) -> Vec<String> {
    // **An error naming what was looked for**, which is this project's house style and was not
    // followed here: a bare `unwrap` on a missing `clang` printed a `NotFound` with no name, so
    // a reader learned that *something* could not be spawned. Reported 2026-08-10 by the first
    // end-to-end user, who hit it on a box without clang.
    let mut child = Command::new(compiler)
        .args(["-E", "-P", "-std=gnu11", "-x", "c", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| {
            panic!(
                "cannot run `{compiler}` as the preprocessor oracle: {e}. It is looked up on \
                 PATH; install it or run with the other compiler."
            )
        });
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

#[test]
fn review_torture_matrix_matches_both_compilers() {
    let cases = [
        ("multiline", "#define f(a,b) <a,b>\nf(1,\n2)\n"),
        ("suffix-rescan", "#define B(x) [x]\n#define A B\nA(1)\n"),
        ("object-paste", "#define A x##y\nA\n"),
        (
            "gnu-varargs",
            "#define D(f,a...) g(f,##a)\nD(\"x\",1)\nD(\"y\")\n",
        ),
        ("placemarkers", "#define h(a,b) a##b\nh(,)\n"),
        ("hex-if", "#if 0xFF\nyes\n#else\nno\n#endif\n"),
        ("octal-if", "#if 010 == 8\nyes\n#else\nno\n#endif\n"),
        ("bitwise-if", "#if (5 & 1) && ((2 | 1) == 3)\nyes\n#endif\n"),
        (
            "shift-if",
            "#if (1 << 4) == 16 && (8 >> 2) == 2\nyes\n#endif\n",
        ),
        ("char-if", "#if 'A' == 65\nyes\n#endif\n"),
        (
            "escaped-char-if",
            "#if '\\x41' == 65 && '\\101' == 65\nyes\n#endif\n",
        ),
        ("multichar-if", "#if 'AB' == 0x4142\nyes\n#endif\n"),
        (
            "elifdef",
            "#define X 1\n#if 0\nno\n#elifdef X\nyes\n#endif\n",
        ),
        (
            "raw-counter",
            "#define str(x) #x\nstr(__COUNTER__) __COUNTER__\n",
        ),
        (
            "argument-order",
            "#define pair(z,a) z a\npair(__COUNTER__,__COUNTER__)\n",
        ),
        (
            "nested-argument",
            "#define pair(a,b) [a][b]\npair((1,2),3)\n",
        ),
        ("argument-blue-paint", "#define f(x) x\nf(f)(1)\n"),
        ("stringize-space", "#define str(x) #x\nstr(a   b/**/c)\n"),
        ("undef", "#define X 1\nX\n#undef X\nX\n"),
        (
            "nested-users",
            "#define B(x) [x]\n#define A(x) B(x)\nA(1) A(2)\n",
        ),
        ("line", "#line 40 \"virtual.c\"\n__LINE__ __FILE__\n"),
        ("pragma", "_Pragma(\"once\") after\n"),
    ];
    for (name, src) in cases {
        let ours: Vec<_> = preprocess_str("oracle.c", src, Config::default())
            .token_texts()
            .map(str::to_owned)
            .collect();
        let gcc = compiler_tokens("gcc", src);
        let clang = compiler_tokens("clang", src);
        assert_eq!(gcc, clang, "{name}: independent compilers disagree");
        assert_eq!(ours, gcc, "{name}: chiero diverges");
    }
}
