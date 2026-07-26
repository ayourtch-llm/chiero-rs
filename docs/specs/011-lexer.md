# 011 — Lexer

Covers C11 translation phases 1–3. Output is a stream of **preprocessing tokens**
(pp-tokens), which are *not* the same as the tokens the parser consumes; conversion
happens in phase 7 ([013](013-parser.md)).

## 1. Translation phases

| Phase | Task | Owner |
|---|---|---|
| 1 | Map physical bytes to source charset; trigraphs | `chiero-lex` |
| 2 | Splice lines ending in `\` | `chiero-lex` |
| 3 | Decompose into pp-tokens and whitespace; comments → one space | `chiero-lex` |
| 4 | Preprocessing directives, macro expansion | `chiero-pp` |
| 5 | Escape sequences → execution charset | `chiero-parse` |
| 6 | Adjacent string literal concatenation | `chiero-parse` |
| 7 | pp-tokens → tokens; parse | `chiero-parse` |

**Trigraphs are off by default.** gcc disables them without `-trigraphs`, VPP does not
use them, and `??!` appears in real string literals. Available behind a config flag for
completeness.

## 2. pp-token model

```rust
pub struct PpToken {
    pub kind: PpTokenKind,
    pub span: Span,
    /// True if whitespace or a comment preceded this token. Required for correct
    /// stringization (#) and for round-tripping.
    pub leading_space: bool,
    /// True if this token is the first on a logical line. Directive detection needs it.
    pub bol: bool,
}

pub enum PpTokenKind {
    Ident(Symbol),
    Number,              // pp-number: a superset of valid numeric literals
    CharLit { prefix: EncPrefix },
    StringLit { prefix: EncPrefix },
    Punct(Punct),
    /// A character that is not part of any valid pp-token (e.g. stray `@`, `$`).
    Other(char),
    Eof,
}
```

### 2.1 pp-number

Phase 3 lexes `pp-number`, which is intentionally sloppier than a C numeric constant:

```
pp-number ::= digit | . digit
            | pp-number ( digit | identifier-nondigit | e+ | e- | E+ | E- | p+ | p- | P+ | P- | . )
```

So `0x1e+2` is **one** pp-token, and `1.0e+5f` is one token. Validation and conversion
to a typed value is phase 7's job, not the lexer's. Getting this wrong produces
mysterious failures in `#if` arithmetic, so it is contract 4.

### 2.2 Line splicing and spans

Phase 2 splicing is the first place where the naive "span = byte range" model is
stressed:

```c
#define FOO ba\
r
```

The identifier is `bar`, but its bytes are not contiguous. Rule: **the `Span` covers the
full physical extent including the splice** (`ba\\\nr`), and the token's text is stored
separately when it differs from the raw slice. `SourceFile` retains a list of splice
positions so `Loc` computation and re-lexing (contract 11 of [010](010-source-and-provenance.md))
stay correct.

Splices are rare, so the "text differs from raw slice" case is a side table, not a field
on every token.

## 3. Interning

Identifiers and string contents are interned into a `Symbol` (`u32`) in a per-session
interner. Interning is mandatory, not an optimization: macro lookup happens on every
identifier in a 1M-line codebase, and pointer/integer comparison is what makes the
preprocessor fast enough to run over the whole VPP tree.

Keywords are **not** distinguished by the lexer. `if`, `while`, `int` are all `Ident`,
because a preprocessing token stream is keyword-agnostic — `#define int long` is legal.
Keyword recognition happens in phase 7.

## 4. Error recovery

The lexer never fails. Malformed input produces:
- An unterminated string/char literal → token ends at end-of-line, diagnostic emitted.
- An unterminated block comment → runs to EOF, diagnostic emitted.
- A stray character → `PpTokenKind::Other(c)`, no diagnostic at lex time (it may be
  discarded by a false `#if` branch and thus be entirely legal).

That last point matters: emitting diagnostics for text inside inactive conditional
blocks is a common tooling bug, and on a codebase with as much `#ifdef` as VPP it would
produce thousands of spurious messages.

## 5. Performance

Target: **≥ 100 MB/s** single-threaded on the VPP tree. The lexer is on the critical
path for whole-tree analysis; at ~1M lines re-lexed across thousands of TUs (headers are
re-lexed per TU absent caching) this is the dominant frontend cost.

Consequences:
- Byte-oriented, no `char` decoding outside string/char literals and comments.
- Table-driven dispatch on the first byte.
- No allocation per token; tokens go into a `Vec<PpToken>` sized from file length.
- A **lexed-header cache** keyed by `(FileId, content hash)` so a header lexed once is
  reused across TUs. This is the single biggest win available and is required, not
  optional — VPP headers are included thousands of times.

## 6. Testable contracts

1. `0x1e+2` lexes as exactly one `Number` pp-token.
2. `1.0e+5f`, `.5`, `0b1010`, `1'000` (C23 separators, accepted and diagnosed under
   C11) each lex as one `Number`.
3. `ba\` + newline + `r` lexes as a single `Ident` with text `bar`, whose span covers
   the full physical extent.
4. `//` comments and `/* */` comments each become exactly one whitespace separator, and
   the following token has `leading_space == true`.
5. `a/**/b` produces two `Ident` tokens, not one — comment replacement happens before
   token pasting is possible.
6. An unterminated `"` produces one `StringLit` ending at the newline plus exactly one
   diagnostic.
7. An unterminated `/*` produces exactly one diagnostic and an `Eof` token.
8. A stray `@` at top level produces `Other('@')` and **zero** diagnostics.
9. `<<=`, `>>=`, `...`, `->`, `++`, `##`, `%:%:` all lex as single `Punct` tokens
   (maximal munch), and `<::>` lexes as `<:` `:>` digraphs.
10. Keywords lex as `Ident`: the token for `int` has kind `Ident`, not a keyword kind.
11. Lexing every `.c` and `.h` file under `/home/ubuntu/vpp/src` produces zero panics.
12. Lexing throughput on a concatenated 50 MB VPP source blob is ≥ 100 MB/s on the
    reference machine.
13. Re-lexing a header with a warm cache is ≥ 20× faster than a cold lex (verifies the
    cache is actually used).
14. Trigraph `??=` lexes as three `Punct` tokens by default, and as `#` with
    `trigraphs = true`.
