//! C11 translation phases 1–3: bytes to preprocessing tokens.
//!
//! The lexer deliberately never fails (011 §4). Invalid input is represented in the
//! token stream and diagnostics are reserved for constructs whose boundary is known.

use chiero_span::{BytePos, FileId, SourceMap, Span};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Symbol(pub u32);

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct LexConfig {
    /// 011 §1: trigraphs are available, but off by default because real strings
    /// contain `??!` and VPP does not require them.
    pub trigraphs: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EncPrefix {
    None,
    Wide,
    Utf8,
    Utf16,
    Utf32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Punct {
    LBracket,
    RBracket,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Dot,
    Arrow,
    PlusPlus,
    MinusMinus,
    Amp,
    Star,
    Plus,
    Minus,
    Tilde,
    Bang,
    Slash,
    Percent,
    Shl,
    Shr,
    Lt,
    Gt,
    Le,
    Ge,
    EqEq,
    Ne,
    Caret,
    Pipe,
    AndAnd,
    OrOr,
    Question,
    Colon,
    Semi,
    Ellipsis,
    Eq,
    StarEq,
    SlashEq,
    PercentEq,
    PlusEq,
    MinusEq,
    ShlEq,
    ShrEq,
    AmpEq,
    CaretEq,
    PipeEq,
    Comma,
    Hash,
    HashHash,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PpTokenKind {
    Ident(Symbol),
    Number,
    CharLit { prefix: EncPrefix },
    StringLit { prefix: EncPrefix },
    Punct(Punct),
    Other(char),
    Eof,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PpToken {
    pub kind: PpTokenKind,
    pub span: Span,
    pub leading_space: bool,
    pub bol: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LexDiagnostic {
    pub span: Span,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct LexedFile {
    tokens: Vec<PpToken>,
    diagnostics: Vec<LexDiagnostic>,
    source: Arc<str>,
    start: BytePos,
    spellings: BTreeMap<usize, String>,
    symbols: Vec<Arc<str>>,
}

impl LexedFile {
    pub fn tokens(&self) -> &[PpToken] {
        &self.tokens
    }

    pub fn diagnostics(&self) -> &[LexDiagnostic] {
        &self.diagnostics
    }

    pub fn text<'a>(&'a self, token: &PpToken) -> &'a str {
        let index = self
            .tokens
            .iter()
            .position(|candidate| std::ptr::eq(candidate, token));
        if let Some(text) = index.and_then(|i| self.spellings.get(&i)) {
            return text;
        }
        let lo = token.span.lo.0.saturating_sub(self.start.0) as usize;
        let hi = token.span.hi.0.saturating_sub(self.start.0) as usize;
        self.source.get(lo..hi).unwrap_or("")
    }

    /// O(1) indexed spelling lookup for pipeline consumers already walking `tokens()`.
    /// `text(&token)` accepts detached callers and therefore has to locate the token;
    /// using it once per token would make preprocessing quadratic (REVIEW-1 finding 13).
    pub fn text_at(&self, index: usize) -> Option<&str> {
        let token = self.tokens.get(index)?;
        if let Some(text) = self.spellings.get(&index) {
            return Some(text);
        }
        let lo = token.span.lo.0.saturating_sub(self.start.0) as usize;
        let hi = token.span.hi.0.saturating_sub(self.start.0) as usize;
        self.source.get(lo..hi)
    }

    pub fn symbol_text(&self, symbol: Symbol) -> Option<&str> {
        self.symbols.get(symbol.0 as usize).map(AsRef::as_ref)
    }
}

#[derive(Debug, Default)]
struct Interner {
    by_text: BTreeMap<Arc<str>, Symbol>,
    strings: Vec<Arc<str>>,
}

impl Interner {
    fn intern(&mut self, text: &str) -> Symbol {
        if let Some(&symbol) = self.by_text.get(text) {
            return symbol;
        }
        let symbol = Symbol(self.strings.len() as u32);
        let text: Arc<str> = Arc::from(text);
        self.strings.push(text.clone());
        self.by_text.insert(text, symbol);
        symbol
    }
}

#[derive(Debug, Default)]
pub struct LexSession {
    interner: RefCell<Interner>,
    cache: RefCell<BTreeMap<CacheKey, Arc<LexedFile>>>,
    cache_hits: Cell<u64>,
    cache_misses: Cell<u64>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CacheKey {
    file: FileId,
    content_hash: u64,
    start: BytePos,
    config: LexConfig,
}

impl LexSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lex(&self, map: &SourceMap, file: FileId, config: LexConfig) -> LexedFile {
        let source_file = map.file(file);
        let source: Arc<str> = Arc::from(source_file.src());
        let cooked = Cooked::new(source.as_bytes(), config);
        let (tokens, diagnostics, spellings) = {
            let mut lexer = Lexer {
                cooked: &cooked,
                source: &source,
                start: source_file.start_pos,
                pos: 0,
                tokens: Vec::with_capacity(source.len() / 3 + 1),
                diagnostics: Vec::new(),
                spellings: BTreeMap::new(),
                interner: &self.interner,
                pending_space: false,
                at_bol: true,
            };
            lexer.run();
            (lexer.tokens, lexer.diagnostics, lexer.spellings)
        };
        LexedFile {
            tokens,
            diagnostics,
            source,
            start: source_file.start_pos,
            spellings,
            symbols: self.interner.borrow().strings.clone(),
        }
    }

    /// Reuse a lexed header. The key includes the content hash required by 011 §5:
    /// `FileId` alone is per-map and a file can change between incremental sessions.
    pub fn lex_cached(&self, map: &SourceMap, file: FileId, config: LexConfig) -> Arc<LexedFile> {
        let source_file = map.file(file);
        let key = CacheKey {
            file,
            content_hash: stable_hash(source_file.src().as_bytes()),
            start: source_file.start_pos,
            config,
        };
        if let Some(hit) = self.cache.borrow().get(&key).cloned() {
            self.cache_hits.set(self.cache_hits.get() + 1);
            return hit;
        }
        let lexed = Arc::new(self.lex(map, file, config));
        self.cache.borrow_mut().insert(key, lexed.clone());
        self.cache_misses.set(self.cache_misses.get() + 1);
        lexed
    }

    /// `(hits, misses)`, exposed so performance tests verify that a quick second call
    /// is a cache hit rather than a conveniently fast fixture.
    pub fn cache_stats(&self) -> (u64, u64) {
        (self.cache_hits.get(), self.cache_misses.get())
    }
}

fn stable_hash(bytes: &[u8]) -> u64 {
    // FNV-1a is deterministic, small, and sufficient for a cache key. A collision only
    // reuses lexing work; provenance still keys on FileId and global start position.
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

struct Cooked<'a> {
    bytes: Cow<'a, [u8]>,
    starts: Option<Vec<usize>>,
    ends: Option<Vec<usize>>,
}

impl<'a> Cooked<'a> {
    fn new(raw: &'a [u8], config: LexConfig) -> Self {
        let has_splice = raw.windows(2).any(|pair| pair == b"\\\n")
            || raw.windows(3).any(|triple| triple == b"\\\r\n");
        let has_trigraph = config.trigraphs
            && raw.windows(3).any(|triple| {
                triple[0] == b'?' && triple[1] == b'?' && trigraph(triple[2]).is_some()
            });
        if !has_splice && !has_trigraph {
            return Self {
                bytes: Cow::Borrowed(raw),
                starts: None,
                ends: None,
            };
        }
        let mut phase1 = Vec::with_capacity(raw.len());
        let mut starts = Vec::with_capacity(raw.len());
        let mut ends = Vec::with_capacity(raw.len());
        let mut i = 0;
        while i < raw.len() {
            if config.trigraphs
                && i + 2 < raw.len()
                && raw[i] == b'?'
                && raw[i + 1] == b'?'
                && let Some(replacement) = trigraph(raw[i + 2])
            {
                phase1.push(replacement);
                starts.push(i);
                ends.push(i + 3);
                i += 3;
                continue;
            }
            phase1.push(raw[i]);
            starts.push(i);
            ends.push(i + 1);
            i += 1;
        }

        let mut bytes = Vec::with_capacity(phase1.len());
        let mut out_starts = Vec::with_capacity(phase1.len());
        let mut out_ends = Vec::with_capacity(phase1.len());
        i = 0;
        while i < phase1.len() {
            // 011 §2.2: remove the splice for recognition, while the mapping retains
            // the full physical extent for any token crossing it.
            if phase1[i] == b'\\' && phase1.get(i + 1) == Some(&b'\n') {
                i += 2;
                continue;
            }
            if phase1[i] == b'\\'
                && phase1.get(i + 1) == Some(&b'\r')
                && phase1.get(i + 2) == Some(&b'\n')
            {
                i += 3;
                continue;
            }
            bytes.push(phase1[i]);
            out_starts.push(starts[i]);
            out_ends.push(ends[i]);
            i += 1;
        }
        Self {
            bytes: Cow::Owned(bytes),
            starts: Some(out_starts),
            ends: Some(out_ends),
        }
    }
}

fn trigraph(byte: u8) -> Option<u8> {
    Some(match byte {
        b'=' => b'#',
        b'/' => b'\\',
        b'\'' => b'^',
        b'(' => b'[',
        b')' => b']',
        b'!' => b'|',
        b'<' => b'{',
        b'>' => b'}',
        b'-' => b'~',
        _ => return None,
    })
}

struct Lexer<'a> {
    cooked: &'a Cooked<'a>,
    source: &'a str,
    start: BytePos,
    pos: usize,
    tokens: Vec<PpToken>,
    diagnostics: Vec<LexDiagnostic>,
    spellings: BTreeMap<usize, String>,
    interner: &'a RefCell<Interner>,
    pending_space: bool,
    at_bol: bool,
}

impl Lexer<'_> {
    fn run(&mut self) {
        while self.pos < self.cooked.bytes.len() {
            match self.cooked.bytes[self.pos] {
                b' ' | b'\t' | 0x0b | 0x0c | b'\r' => {
                    self.pending_space = true;
                    self.pos += 1;
                }
                b'\n' => {
                    self.pending_space = true;
                    self.at_bol = true;
                    self.pos += 1;
                }
                b'/' if self.peek(1) == Some(b'/') => self.line_comment(),
                b'/' if self.peek(1) == Some(b'*') => self.block_comment(),
                b if is_ident_start(b) => self.ident_or_prefixed_literal(),
                b'0'..=b'9' => self.number(),
                b'.' if self.peek(1).is_some_and(|b| b.is_ascii_digit()) => self.number(),
                b'\'' => self.literal(EncPrefix::None, b'\'', 0),
                b'"' => self.literal(EncPrefix::None, b'"', 0),
                _ => self.punct_or_other(),
            }
        }
        let pos = self
            .cooked
            .ends
            .as_ref()
            .and_then(|positions| positions.last())
            .copied()
            .unwrap_or(self.cooked.bytes.len());
        self.tokens.push(PpToken {
            kind: PpTokenKind::Eof,
            span: Span::new(
                BytePos(self.start.0 + pos as u32),
                BytePos(self.start.0 + pos as u32),
                Default::default(),
            ),
            leading_space: self.pending_space,
            bol: self.at_bol,
        });
    }

    fn peek(&self, offset: usize) -> Option<u8> {
        self.cooked.bytes.get(self.pos + offset).copied()
    }

    fn ident_or_prefixed_literal(&mut self) {
        let begin = self.pos;
        let (prefix, prefix_len) = match (self.peek(0), self.peek(1), self.peek(2)) {
            (Some(b'u'), Some(b'8'), Some(b'\'' | b'"')) => (EncPrefix::Utf8, 2),
            (Some(b'L'), Some(b'\'' | b'"'), _) => (EncPrefix::Wide, 1),
            (Some(b'u'), Some(b'\'' | b'"'), _) => (EncPrefix::Utf16, 1),
            (Some(b'U'), Some(b'\'' | b'"'), _) => (EncPrefix::Utf32, 1),
            _ => (EncPrefix::None, 0),
        };
        if prefix_len != 0 {
            let quote = self.cooked.bytes[begin + prefix_len];
            self.pos += prefix_len;
            self.literal(prefix, quote, prefix_len);
            return;
        }
        self.pos += 1;
        while self.peek(0).is_some_and(is_ident_continue) {
            self.pos += 1;
        }
        let text = String::from_utf8_lossy(&self.cooked.bytes[begin..self.pos]);
        let symbol = self.interner.borrow_mut().intern(&text);
        self.push(begin, self.pos, PpTokenKind::Ident(symbol), None);
    }

    fn number(&mut self) {
        let begin = self.pos;
        self.pos += 1;
        while let Some(byte) = self.peek(0) {
            let sign_after_exp = matches!(byte, b'+' | b'-')
                && self.pos > begin
                && matches!(self.cooked.bytes[self.pos - 1], b'e' | b'E' | b'p' | b'P');
            if byte.is_ascii_alphanumeric()
                || matches!(byte, b'_' | b'.' | b'\'')
                || sign_after_exp
                || byte >= 0x80
            {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.push(begin, self.pos, PpTokenKind::Number, None);
    }

    fn literal(&mut self, prefix: EncPrefix, quote: u8, prefix_len: usize) {
        let begin = self.pos - prefix_len;
        self.pos += 1;
        let mut terminated = false;
        while let Some(byte) = self.peek(0) {
            if byte == quote {
                self.pos += 1;
                terminated = true;
                break;
            }
            if byte == b'\n' {
                break;
            }
            if byte == b'\\' && self.peek(1).is_some() {
                self.pos += 2;
            } else {
                self.pos += 1;
            }
        }
        let kind = if quote == b'"' {
            PpTokenKind::StringLit { prefix }
        } else {
            PpTokenKind::CharLit { prefix }
        };
        self.push(begin, self.pos, kind, None);
        if !terminated {
            self.diagnostics.push(LexDiagnostic {
                span: self.tokens.last().unwrap().span,
                message: "unterminated literal".into(),
            });
        }
    }

    fn line_comment(&mut self) {
        self.pos += 2;
        while self.peek(0).is_some_and(|b| b != b'\n') {
            self.pos += 1;
        }
        self.pending_space = true;
    }

    fn block_comment(&mut self) {
        let begin = self.pos;
        self.pos += 2;
        while self.pos < self.cooked.bytes.len() {
            if self.peek(0) == Some(b'*') && self.peek(1) == Some(b'/') {
                self.pos += 2;
                self.pending_space = true;
                return;
            }
            if self.peek(0) == Some(b'\n') {
                self.at_bol = true;
            }
            self.pos += 1;
        }
        self.pending_space = true;
        self.diagnostics.push(LexDiagnostic {
            span: self.span(begin, self.pos),
            message: "unterminated block comment".into(),
        });
    }

    fn punct_or_other(&mut self) {
        const PUNCTS: &[(&[u8], Punct)] = &[
            (b"%:%:", Punct::HashHash),
            (b">>=", Punct::ShrEq),
            (b"<<=", Punct::ShlEq),
            (b"...", Punct::Ellipsis),
            (b"##", Punct::HashHash),
            (b"->", Punct::Arrow),
            (b"++", Punct::PlusPlus),
            (b"--", Punct::MinusMinus),
            (b"<<", Punct::Shl),
            (b">>", Punct::Shr),
            (b"<=", Punct::Le),
            (b">=", Punct::Ge),
            (b"==", Punct::EqEq),
            (b"!=", Punct::Ne),
            (b"&&", Punct::AndAnd),
            (b"||", Punct::OrOr),
            (b"*=", Punct::StarEq),
            (b"/=", Punct::SlashEq),
            (b"%=", Punct::PercentEq),
            (b"+=", Punct::PlusEq),
            (b"-=", Punct::MinusEq),
            (b"&=", Punct::AmpEq),
            (b"^=", Punct::CaretEq),
            (b"|=", Punct::PipeEq),
            (b"<:", Punct::LBracket),
            (b":>", Punct::RBracket),
            (b"<%", Punct::LBrace),
            (b"%>", Punct::RBrace),
            (b"%:", Punct::Hash),
        ];
        let rest = &self.cooked.bytes[self.pos..];
        if let Some(&(text, punct)) = PUNCTS.iter().find(|(text, _)| rest.starts_with(text)) {
            let begin = self.pos;
            self.pos += text.len();
            self.push(begin, self.pos, PpTokenKind::Punct(punct), None);
            return;
        }
        let begin = self.pos;
        let byte = self.cooked.bytes[self.pos];
        self.pos += 1;
        let punct = match byte {
            b'[' => Some(Punct::LBracket),
            b']' => Some(Punct::RBracket),
            b'(' => Some(Punct::LParen),
            b')' => Some(Punct::RParen),
            b'{' => Some(Punct::LBrace),
            b'}' => Some(Punct::RBrace),
            b'.' => Some(Punct::Dot),
            b'&' => Some(Punct::Amp),
            b'*' => Some(Punct::Star),
            b'+' => Some(Punct::Plus),
            b'-' => Some(Punct::Minus),
            b'~' => Some(Punct::Tilde),
            b'!' => Some(Punct::Bang),
            b'/' => Some(Punct::Slash),
            b'%' => Some(Punct::Percent),
            b'<' => Some(Punct::Lt),
            b'>' => Some(Punct::Gt),
            b'^' => Some(Punct::Caret),
            b'|' => Some(Punct::Pipe),
            b'?' => Some(Punct::Question),
            b':' => Some(Punct::Colon),
            b';' => Some(Punct::Semi),
            b'=' => Some(Punct::Eq),
            b',' => Some(Punct::Comma),
            b'#' => Some(Punct::Hash),
            _ => None,
        };
        let kind = punct.map_or_else(|| PpTokenKind::Other(char::from(byte)), PpTokenKind::Punct);
        self.push(begin, self.pos, kind, None);
    }

    fn span(&self, begin: usize, end: usize) -> Span {
        let lo = self
            .cooked
            .starts
            .as_ref()
            .map_or(begin, |positions| positions[begin]);
        let hi = self.cooked.ends.as_ref().map_or(end, |positions| {
            end.checked_sub(1)
                .and_then(|i| positions.get(i))
                .copied()
                .unwrap_or(lo)
        });
        Span::new(
            BytePos(self.start.0 + lo as u32),
            BytePos(self.start.0 + hi as u32),
            Default::default(),
        )
    }

    fn push(&mut self, begin: usize, end: usize, kind: PpTokenKind, cooked_text: Option<&str>) {
        let span = self.span(begin, end);
        let index = self.tokens.len();
        let raw_lo = span.lo.0.saturating_sub(self.start.0) as usize;
        let raw_hi = span.hi.0.saturating_sub(self.start.0) as usize;
        let logical = &self.cooked.bytes[begin..end];
        let raw = self.source.as_bytes().get(raw_lo..raw_hi);
        if cooked_text.is_some() || raw != Some(logical) {
            let cooked = cooked_text
                .map(str::to_owned)
                .unwrap_or_else(|| String::from_utf8_lossy(logical).into_owned());
            self.spellings.insert(index, cooked);
        }
        self.tokens.push(PpToken {
            kind,
            span,
            leading_space: std::mem::take(&mut self.pending_space),
            bol: self.at_bol,
        });
        self.at_bol = false;
    }
}

fn is_ident_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic() || byte >= 0x80
}

fn is_ident_continue(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}
