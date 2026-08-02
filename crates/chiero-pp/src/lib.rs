//! C preprocessing (translation phase 4) with macro provenance.

use chiero_lex::{EncPrefix, LexConfig, LexSession, PpToken, PpTokenKind, Punct, Symbol};
use chiero_span::{ExpnCtx, ExpnKind, FileId, MacroId, SourceMap, Span};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub id: ConfigId,
    pub pedantic: bool,
    pub date: String,
    pub time: String,
    pub iquote_paths: Vec<PathBuf>,
    pub include_paths: Vec<PathBuf>,
    pub system_paths: Vec<PathBuf>,
    pub max_include_depth: usize,
    pub max_macro_expansion_depth: usize,
    /// Command-line-style object macros, applied after target predefines.
    pub defines: Vec<(String, String)>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            id: ConfigId(1),
            pedantic: false,
            date: "Jan 01 1970".into(),
            time: "00:00:00".into(),
            iquote_paths: Vec::new(),
            include_paths: Vec::new(),
            system_paths: Vec::new(),
            max_include_depth: 200,
            max_macro_expansion_depth: 256,
            defines: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: Span,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PragmaRecord {
    pub span: Span,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Variadic {
    No,
    Std,
    Named(Symbol),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MacroKind {
    ObjectLike,
    FunctionLike {
        params: Vec<Symbol>,
        variadic: Variadic,
    },
}

#[derive(Clone, Debug)]
pub struct MacroDef {
    pub id: MacroId,
    pub name: Symbol,
    pub kind: MacroKind,
    pub body: Vec<PpToken>,
    pub def_span: Span,
    pub undef_span: Option<Span>,
}

#[derive(Debug)]
pub struct PreprocessedTu {
    pub tokens: Vec<PpToken>,
    pub source_map: SourceMap,
    pub diagnostics: Vec<Diagnostic>,
    pub config: ConfigId,
    pub deps: Vec<FileId>,
    pub pragmas: Vec<PragmaRecord>,
    pub macro_defs: Vec<MacroDef>,
    spellings: Vec<String>,
    symbols: BTreeMap<Symbol, Arc<str>>,
}

impl PreprocessedTu {
    pub fn token_texts(&self) -> impl Iterator<Item = &str> {
        self.spellings.iter().map(String::as_str)
    }

    /// **`text_at` is what a consumer walking the stream wants.** This one has to find
    /// the token's index by identity, so it is linear in the stream — fine for a test
    /// holding one token, quadratic for a parser holding all of them. Kept because it
    /// reads naturally at a call site that already has a `&PpToken`.
    pub fn text(&self, token: &PpToken) -> Option<&str> {
        self.tokens
            .iter()
            .position(|candidate| std::ptr::eq(candidate, token))
            .and_then(|index| self.spellings.get(index))
            .map(String::as_str)
    }

    /// The spelling of the token at `index` in [`Self::tokens`], in constant time.
    pub fn text_at(&self, index: usize) -> Option<&str> {
        self.spellings.get(index).map(String::as_str)
    }

    pub fn symbol_text(&self, symbol: Symbol) -> Option<&str> {
        self.symbols.get(&symbol).map(AsRef::as_ref)
    }
}

#[derive(Clone, Debug)]
struct Tok {
    token: PpToken,
    text: String,
    hide: HideSet,
}

#[derive(Clone, Debug, Default)]
struct HideSet(Vec<u64>);

impl HideSet {
    fn contains(&self, id: &MacroId) -> bool {
        let bit = id.0 as usize;
        self.0
            .get(bit / 64)
            .is_some_and(|word| word & (1_u64 << (bit % 64)) != 0)
    }

    fn insert(&mut self, id: MacroId) {
        let bit = id.0 as usize;
        self.0.resize(self.0.len().max(bit / 64 + 1), 0);
        self.0[bit / 64] |= 1_u64 << (bit % 64);
    }

    fn extend(&mut self, other: &Self) {
        self.0.resize(self.0.len().max(other.0.len()), 0);
        for (target, source) in self.0.iter_mut().zip(&other.0) {
            *target |= source;
        }
    }
}

#[derive(Clone, Debug)]
struct StoredMacro {
    def: MacroDef,
    name: String,
    params: Vec<String>,
    variadic_name: Option<String>,
    std_variadic: bool,
    body: Vec<Tok>,
}

pub fn preprocess_str(path: impl AsRef<Path>, src: &str, config: Config) -> PreprocessedTu {
    Engine::new(path.as_ref(), src, config).run()
}

#[derive(Debug, Default)]
pub struct PreprocessorSession {
    lex_session: LexSession,
}

impl PreprocessorSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn preprocess_str(
        &self,
        path: impl AsRef<Path>,
        src: &str,
        config: Config,
    ) -> PreprocessedTu {
        Engine::new_with_lexer(path.as_ref(), src, config, self.lex_session.clone()).run()
    }

    pub fn preprocess_with_loader<L: FileLoader>(
        &self,
        path: impl AsRef<Path>,
        src: &str,
        config: Config,
        loader: &mut L,
    ) -> PreprocessedTu {
        Engine::new_with_lexer(path.as_ref(), src, config, self.lex_session.clone())
            .run_with_loader(loader)
    }

    pub fn lex_cache_stats(&self) -> (u64, u64) {
        self.lex_session.cache_stats()
    }
}

pub trait FileLoader {
    fn load(&mut self, path: &Path) -> io::Result<String>;
}

pub fn preprocess_with_loader<L: FileLoader>(
    path: impl AsRef<Path>,
    src: &str,
    config: Config,
    loader: &mut L,
) -> PreprocessedTu {
    Engine::new(path.as_ref(), src, config).run_with_loader(loader)
}

#[derive(Copy, Clone)]
struct Conditional {
    parent_active: bool,
    active: bool,
    taken: bool,
    /// Where the `#if` that opened this frame is, so an unterminated one can point at it.
    ///
    /// **The opening span, not the end of the file.** "unterminated `#if`" reported at EOF names
    /// the one place in the file that is certainly not the mistake; 023 §9's rule about a report
    /// a person can act on applies to *where* as much as to what.
    opened: Span,
    /// Whether an `#else` has been seen, so a second one can be reported. Distinct from `taken`,
    /// which says a *branch* was taken and is already true for `#if 1` before any `#else`.
    saw_else: bool,
}

#[derive(Clone)]
struct LineOverride {
    physical_start: u32,
    reported_start: u32,
    file: Option<String>,
}

struct MissingLoader;

impl FileLoader for MissingLoader {
    fn load(&mut self, path: &Path) -> io::Result<String> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no loader configured for {}", path.display()),
        ))
    }
}

fn parse_header_name(tokens: &[Tok]) -> Option<(String, bool)> {
    if tokens.len() == 1 && matches!(tokens[0].token.kind, PpTokenKind::StringLit { .. }) {
        return tokens[0]
            .text
            .strip_prefix('"')
            .and_then(|text| text.strip_suffix('"'))
            .map(|text| (text.to_owned(), true));
    }
    if tokens.first().is_some_and(|token| token.text == "<")
        && tokens.last().is_some_and(|token| token.text == ">")
    {
        return Some((
            tokens[1..tokens.len() - 1]
                .iter()
                .map(|token| token.text.as_str())
                .collect(),
            false,
        ));
    }
    None
}

fn detect_guard_tokens(tokens: &[Tok]) -> Option<String> {
    let mut lines = Vec::new();
    let mut i = 0;
    while i < tokens.len() && lines.len() < 2 {
        let end = (i + 1..tokens.len())
            .find(|&index| tokens[index].token.bol)
            .unwrap_or(tokens.len());
        lines.push(&tokens[i..end]);
        i = end;
    }
    let first = lines.first()?;
    let second = lines.get(1)?;
    if first.first()?.text != "#"
        || first.get(1)?.text != "ifndef"
        || second.first()?.text != "#"
        || second.get(1)?.text != "define"
    {
        return None;
    }
    let guard = &first.get(2)?.text;
    (second.get(2)?.text == *guard).then(|| guard.clone())
}

struct Engine {
    config: Config,
    source_map: SourceMap,
    lex_session: LexSession,
    root_path: PathBuf,
    input: Vec<Tok>,
    deps: Vec<FileId>,
    lex_diagnostics: BTreeMap<(FileId, u32), Vec<Diagnostic>>,
    once: BTreeSet<PathBuf>,
    guards: BTreeMap<PathBuf, String>,
    line_overrides: BTreeMap<FileId, Vec<LineOverride>>,
    probed_headers: BTreeMap<PathBuf, String>,
    macros: Vec<StoredMacro>,
    by_name: BTreeMap<String, usize>,
    diagnostics: Vec<Diagnostic>,
    pragmas: Vec<PragmaRecord>,
    counter: u64,
    expansion_depth: usize,
}

impl Engine {
    fn new(path: &Path, src: &str, config: Config) -> Self {
        Self::new_with_lexer(path, src, config, LexSession::new())
    }

    fn new_with_lexer(path: &Path, src: &str, config: Config, lex_session: LexSession) -> Self {
        let mut source_map = SourceMap::new();
        let file = source_map.add_file(path, src);
        let lexed = lex_session.lex_cached(&source_map, file, LexConfig::default());
        // 012 contract 10: inactive branches are lexed but not analyzed. Lexer
        // diagnostics cannot be promoted until conditional activity is known.
        let diagnostics = Vec::new();
        let input = lexed
            .tokens()
            .iter()
            .enumerate()
            .filter(|(_, token)| !matches!(token.kind, PpTokenKind::Eof))
            .map(|(index, token)| Tok {
                token: token.clone(),
                text: lexed.text_at(index).unwrap_or("").to_owned(),
                hide: HideSet::default(),
            })
            .collect();
        let mut lex_diagnostics = BTreeMap::new();
        for diagnostic in lexed.diagnostics() {
            let line = source_map
                .lookup_loc(diagnostic.span.lo)
                .map_or(0, |loc| loc.line);
            lex_diagnostics
                .entry((file, line))
                .or_insert_with(Vec::new)
                .push(Diagnostic {
                    span: diagnostic.span,
                    message: diagnostic.message.clone(),
                });
        }
        let mut engine = Self {
            config,
            source_map,
            lex_session,
            root_path: path.to_path_buf(),
            input,
            deps: vec![file],
            lex_diagnostics,
            once: BTreeSet::new(),
            guards: BTreeMap::new(),
            line_overrides: BTreeMap::new(),
            probed_headers: BTreeMap::new(),
            macros: Vec::new(),
            by_name: BTreeMap::new(),
            diagnostics,
            pragmas: Vec::new(),
            counter: 0,
            expansion_depth: 0,
        };
        for builtin in [
            "__LINE__",
            "__FILE__",
            "__COUNTER__",
            "__DATE__",
            "__TIME__",
        ] {
            engine.add_builtin(builtin);
        }
        for (name, value) in [
            ("__STDC__", "1"),
            ("__STDC_HOSTED__", "1"),
            ("__STDC_VERSION__", "201112L"),
            ("__GNUC__", "13"),
            ("__x86_64__", "1"),
        ] {
            engine.add_predefined_object(name, value);
        }
        for name in ["__has_include", "__has_attribute", "__has_builtin"] {
            engine.add_predefined_query(name);
        }
        for (name, value) in engine.config.defines.clone() {
            engine.add_config_object(&name, &value);
        }
        engine
    }

    fn run(mut self) -> PreprocessedTu {
        let mut loader = MissingLoader;
        self.finish(&mut loader)
    }

    fn run_with_loader(mut self, loader: &mut dyn FileLoader) -> PreprocessedTu {
        self.finish(loader)
    }

    fn finish(&mut self, loader: &mut dyn FileLoader) -> PreprocessedTu {
        let input = std::mem::take(&mut self.input);
        let root_path = self.root_path.clone();
        let output = self.process_tokens(input, &root_path, 0, loader);
        // **A stray character is one that reaches the program** (C 6.4p3). 010 classifies a
        // character C has no use for as `Other` and says nothing, because at that point it does
        // not know: gcc takes `S(a\b)` where `#define S(x) #x` stringizes the backslash, and
        // takes `#define M @` until `M` is used. Only here, on the token stream that goes to
        // 013, is the question answerable — and answering it is what stops 013 producing three
        // to six "expected a declaration" messages that never name the character.
        for t in &output {
            if let PpTokenKind::Other(c) = t.token.kind {
                self.diagnostics.push(Diagnostic {
                    span: t.token.span,
                    message: format!("stray `{c}` in program"),
                });
            }
        }
        let tokens = output.iter().map(|t| t.token.clone()).collect();
        let spellings = output.into_iter().map(|t| t.text).collect();
        let macro_defs: Vec<_> = self
            .macros
            .iter()
            .map(|stored| stored.def.clone())
            .collect();
        let mut symbols = BTreeMap::new();
        for definition in &macro_defs {
            let symbol_iter = std::iter::once(definition.name).chain(match &definition.kind {
                MacroKind::FunctionLike { params, variadic } => {
                    let mut symbols = params.clone();
                    if let Variadic::Named(symbol) = variadic {
                        symbols.push(*symbol);
                    }
                    symbols
                }
                MacroKind::ObjectLike => Vec::new(),
            });
            for symbol in symbol_iter {
                if let Some(text) = self.lex_session.symbol_text(symbol) {
                    symbols.insert(symbol, text);
                }
            }
        }
        PreprocessedTu {
            tokens,
            source_map: std::mem::take(&mut self.source_map),
            diagnostics: std::mem::take(&mut self.diagnostics),
            config: self.config.id,
            deps: std::mem::take(&mut self.deps),
            pragmas: std::mem::take(&mut self.pragmas),
            macro_defs,
            spellings,
            symbols,
        }
    }

    fn process_tokens(
        &mut self,
        input: Vec<Tok>,
        path: &Path,
        depth: usize,
        loader: &mut dyn FileLoader,
    ) -> Vec<Tok> {
        let mut output = Vec::new();
        let mut ordinary = Vec::new();
        let mut conditionals: Vec<Conditional> = Vec::new();
        let mut i = 0;
        while i < input.len() {
            let end = (i + 1..input.len())
                .find(|&j| input[j].token.bol)
                .unwrap_or(input.len());
            let line = input[i..end].to_vec();
            let active = conditionals.last().is_none_or(|frame| frame.active);
            self.promote_lex_diagnostics(&line, active);
            if line.first().is_some_and(|t| {
                t.token.bol && matches!(t.token.kind, PpTokenKind::Punct(Punct::Hash))
            }) {
                // C11 §6.10.3 ¶10 operates on the preprocessing-token stream, not a
                // physical line. A directive is the only boundary at which an active
                // ordinary-token chunk must be complete.
                output.extend(self.expand(std::mem::take(&mut ordinary)));
                if active
                    && line.get(1).is_some_and(|token| {
                        matches!(token.text.as_str(), "include" | "include_next")
                    })
                {
                    let include_next = line[1].text == "include_next";
                    output.extend(self.include(&line, path, depth, loader, include_next));
                } else if active
                    && line.get(1).is_some_and(|token| token.text == "pragma")
                    && line.get(2).is_some_and(|token| token.text == "once")
                {
                    self.record_pragma_tokens(&line[2..]);
                    self.once.insert(path.to_path_buf());
                } else {
                    self.directive(&line, &mut conditionals, path, loader);
                }
            } else if active {
                ordinary.extend(line);
            }
            i = end;
        }
        output.extend(self.expand(ordinary));
        // **Every `#if` opened in this file is closed in it** (C 6.10.1). Checked per group
        // rather than globally, because a conditional may not span an `#include`: the stack is
        // local to this call, so a header that opens one and does not close it is reported
        // against the header rather than against whatever came after.
        for frame in &conditionals {
            self.diagnostics.push(Diagnostic {
                span: frame.opened,
                message: "unterminated `#if`".into(),
            });
        }
        output
    }

    fn promote_lex_diagnostics(&mut self, line: &[Tok], active: bool) {
        let Some(first) = line.first() else { return };
        let Some(loc) = self.source_map.lookup_loc(first.token.span.lo) else {
            return;
        };
        if let Some(found) = self.lex_diagnostics.remove(&(loc.file, loc.line))
            && active
        {
            self.diagnostics.extend(found);
        }
    }

    fn include(
        &mut self,
        line: &[Tok],
        current: &Path,
        depth: usize,
        loader: &mut dyn FileLoader,
        include_next: bool,
    ) -> Vec<Tok> {
        if depth >= self.config.max_include_depth {
            self.diagnostics.push(Diagnostic {
                span: line.first().map_or(Span::DUMMY, |token| token.token.span),
                message: format!(
                    "maximum include depth {} exceeded",
                    self.config.max_include_depth
                ),
            });
            return Vec::new();
        }
        let operand = line.get(2..).unwrap_or_default();
        let expanded;
        let header_tokens = if parse_header_name(operand).is_some() {
            operand
        } else {
            expanded = self.expand(operand.to_vec());
            &expanded
        };
        let Some((name, quoted)) = parse_header_name(header_tokens) else {
            // **Two faults reached one sentence.** A *computed* include really can be invalid —
            // `#define H` then `#include H` expands to nothing — but `#include <stdio.h> extra`
            // is a perfectly well-formed header name with tokens after it, and saying "invalid
            // computed include" sends a reader to inspect the header name. gcc says "extra
            // tokens at end of #include directive".
            //
            // Told apart by asking whether a **prefix** of the operand is a header name, which
            // is exactly the difference: the name parsed and the line did not stop there.
            let extra =
                (1..header_tokens.len()).any(|n| parse_header_name(&header_tokens[..n]).is_some());
            let message = if extra {
                "extra tokens after the `#include` header name"
            } else {
                "invalid computed include"
            };
            self.diagnostics.push(Diagnostic {
                span: line.get(1).map_or(Span::DUMMY, |token| token.token.span),
                message: message.into(),
            });
            return Vec::new();
        };
        let mut directories = Vec::new();
        if quoted {
            directories.push(
                current
                    .parent()
                    .unwrap_or_else(|| Path::new(""))
                    .to_path_buf(),
            );
            directories.extend(self.config.iquote_paths.iter().cloned());
        }
        directories.extend(self.config.include_paths.iter().cloned());
        directories.extend(self.config.system_paths.iter().cloned());
        if include_next {
            let provider = current.parent().unwrap_or_else(|| Path::new(""));
            if let Some(index) = directories
                .iter()
                .position(|directory| directory == provider)
            {
                directories.drain(..=index);
            }
        }
        let mut candidates: Vec<_> = directories
            .into_iter()
            .map(|directory| directory.join(&name))
            .collect();
        if candidates.is_empty() {
            candidates.push(PathBuf::from(&name));
        }
        let mut last_error = None;
        for resolved in candidates {
            if self.once.contains(&resolved)
                || self
                    .guards
                    .get(&resolved)
                    .is_some_and(|guard| self.by_name.contains_key(guard))
            {
                return Vec::new();
            }
            let loaded = self
                .probed_headers
                .remove(&resolved)
                .map_or_else(|| loader.load(&resolved), Ok);
            match loaded {
                Ok(source) => {
                    let input = self.lex_source(&resolved, &source);
                    if let Some(guard) = detect_guard_tokens(&input) {
                        self.guards.insert(resolved.clone(), guard);
                    }
                    return self.process_tokens(input, &resolved, depth + 1, loader);
                }
                Err(error) => last_error = Some(error),
            }
        }
        self.diagnostics.push(Diagnostic {
            span: line.get(1).map_or(Span::DUMMY, |token| token.token.span),
            message: format!(
                "cannot include {name}: {}",
                last_error.map_or_else(|| "not found".into(), |error| error.to_string())
            ),
        });
        Vec::new()
    }

    fn lex_source(&mut self, path: &Path, source: &str) -> Vec<Tok> {
        let file = self.source_map.add_file(path, source);
        self.deps.push(file);
        let lexed = self
            .lex_session
            .lex_cached(&self.source_map, file, LexConfig::default());
        for diagnostic in lexed.diagnostics() {
            let line = self
                .source_map
                .lookup_loc(diagnostic.span.lo)
                .map_or(0, |loc| loc.line);
            self.lex_diagnostics
                .entry((file, line))
                .or_default()
                .push(Diagnostic {
                    span: diagnostic.span,
                    message: diagnostic.message.clone(),
                });
        }
        lexed
            .tokens()
            .iter()
            .enumerate()
            .filter(|(_, token)| !matches!(token.kind, PpTokenKind::Eof))
            .map(|(index, token)| Tok {
                token: token.clone(),
                text: lexed.text_at(index).unwrap_or("").to_owned(),
                hide: HideSet::default(),
            })
            .collect()
    }

    fn probe_include(
        &mut self,
        tokens: &[Tok],
        current: &Path,
        loader: &mut dyn FileLoader,
    ) -> bool {
        let Some((name, quoted)) = parse_header_name(tokens) else {
            return false;
        };
        let mut directories = Vec::new();
        if quoted {
            directories.push(
                current
                    .parent()
                    .unwrap_or_else(|| Path::new(""))
                    .to_path_buf(),
            );
            directories.extend(self.config.iquote_paths.iter().cloned());
        }
        directories.extend(self.config.include_paths.iter().cloned());
        directories.extend(self.config.system_paths.iter().cloned());
        if directories.is_empty() {
            directories.push(PathBuf::new());
        }
        for directory in directories {
            let resolved = directory.join(&name);
            if self.probed_headers.contains_key(&resolved) {
                return true;
            }
            if let Ok(source) = loader.load(&resolved) {
                self.probed_headers.insert(resolved, source);
                return true;
            }
        }
        false
    }

    fn add_builtin(&mut self, name: &str) {
        let name_symbol = self.lex_session.intern_symbol(name);
        let id = self
            .source_map
            .add_macro_at(name, Span::DUMMY, Span::DUMMY, None, 0);
        let index = self.macros.len();
        self.macros.push(StoredMacro {
            def: MacroDef {
                id,
                name: name_symbol,
                kind: MacroKind::ObjectLike,
                body: Vec::new(),
                def_span: Span::DUMMY,
                undef_span: None,
            },
            name: name.into(),
            params: Vec::new(),
            variadic_name: None,
            std_variadic: false,
            body: Vec::new(),
        });
        self.by_name.insert(name.into(), index);
    }

    fn add_predefined_object(&mut self, name: &str, value: &str) {
        let name_symbol = self.lex_session.intern_symbol(name);
        let id = self
            .source_map
            .add_macro_at(name, Span::DUMMY, Span::DUMMY, None, 0);
        let index = self.macros.len();
        let body = vec![synthetic_number(value, Span::DUMMY)];
        self.macros.push(StoredMacro {
            def: MacroDef {
                id,
                name: name_symbol,
                kind: MacroKind::ObjectLike,
                body: body.iter().map(|token| token.token.clone()).collect(),
                def_span: Span::DUMMY,
                undef_span: None,
            },
            name: name.into(),
            params: Vec::new(),
            variadic_name: None,
            std_variadic: false,
            body,
        });
        self.by_name.insert(name.into(), index);
    }

    fn add_predefined_query(&mut self, name: &str) {
        let name_symbol = self.lex_session.intern_symbol(name);
        let parameter_symbol = self.lex_session.intern_symbol("query");
        let id = self
            .source_map
            .add_macro_at(name, Span::DUMMY, Span::DUMMY, None, 0);
        let index = self.macros.len();
        let body = vec![synthetic_number("0", Span::DUMMY)];
        self.macros.push(StoredMacro {
            def: MacroDef {
                id,
                name: name_symbol,
                kind: MacroKind::FunctionLike {
                    params: vec![parameter_symbol],
                    variadic: Variadic::No,
                },
                body: body.iter().map(|token| token.token.clone()).collect(),
                def_span: Span::DUMMY,
                undef_span: None,
            },
            name: name.into(),
            params: vec!["query".into()],
            variadic_name: None,
            std_variadic: false,
            body,
        });
        self.by_name.insert(name.into(), index);
    }

    fn add_config_object(&mut self, name: &str, value: &str) {
        let (name, params) = parse_configured_name(name);
        let name_symbol = self.lex_session.intern_symbol(&name);
        let mut temporary = SourceMap::new();
        let file = temporary.add_file("<command-line>", value);
        let lexed = self.lex_session.lex(&temporary, file, LexConfig::default());
        let body: Vec<_> = lexed
            .tokens()
            .iter()
            .enumerate()
            .filter(|(_, token)| !matches!(token.kind, PpTokenKind::Eof))
            .map(|(index, token)| Tok {
                token: PpToken {
                    kind: token.kind.clone(),
                    span: Span::DUMMY,
                    leading_space: token.leading_space,
                    bol: token.bol,
                },
                text: lexed.text_at(index).unwrap_or("").to_owned(),
                hide: HideSet::default(),
            })
            .collect();
        let id = self
            .source_map
            .add_macro_at(&name, Span::DUMMY, Span::DUMMY, None, 0);
        let index = self.macros.len();
        let kind = if params.is_empty() {
            MacroKind::ObjectLike
        } else {
            MacroKind::FunctionLike {
                params: params
                    .iter()
                    .map(|parameter| self.lex_session.intern_symbol(parameter))
                    .collect(),
                variadic: Variadic::No,
            }
        };
        self.macros.push(StoredMacro {
            def: MacroDef {
                id,
                name: name_symbol,
                kind,
                body: body.iter().map(|token| token.token.clone()).collect(),
                def_span: Span::DUMMY,
                undef_span: None,
            },
            name: name.clone(),
            params,
            variadic_name: None,
            std_variadic: false,
            body,
        });
        if let Some(previous) = self.by_name.get(&name).copied() {
            self.macros[previous].def.undef_span = Some(Span::DUMMY);
        }
        self.by_name.insert(name, index);
    }

    fn directive(
        &mut self,
        line: &[Tok],
        conditionals: &mut Vec<Conditional>,
        current: &Path,
        loader: &mut dyn FileLoader,
    ) {
        let directive = line.get(1).map(|t| t.text.as_str());
        let active = conditionals.last().is_none_or(|frame| frame.active);
        match directive {
            Some("if") => {
                let parent_active = active;
                // **C 6.10.1p1: `#if` has an expression.** Only when the branch is live: an
                // inactive `#if` inside a skipped region is not evaluated at all, and 012's rule
                // is that skipped text is lexed but not diagnosed.
                if parent_active && line.len() <= 2 {
                    self.diagnostics.push(Diagnostic {
                        span: line[0].token.span,
                        message: "`#if` with no expression".into(),
                    });
                }
                let value =
                    parent_active && self.eval_if(&line[2..], line[0].token.span, current, loader);
                conditionals.push(Conditional {
                    parent_active,
                    active: value,
                    taken: value,
                    opened: line[0].token.span,
                    saw_else: false,
                });
            }
            Some("ifdef" | "ifndef") => {
                let parent_active = active;
                if parent_active && self.check_macro_name_present(line) {
                    self.check_extra_tokens(line, 3);
                }
                let defined = line
                    .get(2)
                    .is_some_and(|name| self.by_name.contains_key(&name.text));
                let value = parent_active
                    && if directive == Some("ifdef") {
                        defined
                    } else {
                        !defined
                    };
                conditionals.push(Conditional {
                    parent_active,
                    active: value,
                    taken: value,
                    opened: line[0].token.span,
                    saw_else: false,
                });
            }
            Some("elif") => {
                let should_eval = conditionals
                    .last()
                    .is_some_and(|frame| frame.parent_active && !frame.taken);
                let value =
                    should_eval && self.eval_if(&line[2..], line[0].token.span, current, loader);
                if let Some(frame) = conditionals.last_mut() {
                    frame.active = value;
                    frame.taken |= value;
                }
            }
            Some("elifdef" | "elifndef") => {
                let should_eval = conditionals
                    .last()
                    .is_some_and(|frame| frame.parent_active && !frame.taken);
                let defined = line
                    .get(2)
                    .is_some_and(|name| self.by_name.contains_key(&name.text));
                let value = should_eval
                    && if directive == Some("elifdef") {
                        defined
                    } else {
                        !defined
                    };
                if let Some(frame) = conditionals.last_mut() {
                    frame.active = value;
                    frame.taken |= value;
                }
                self.diagnostics.push(Diagnostic {
                    span: line.get(1).map_or(Span::DUMMY, |token| token.token.span),
                    message: format!(
                        "#{} is a C23 extension accepted in C11 mode",
                        directive.unwrap_or_default()
                    ),
                });
            }
            Some("else") => {
                if conditionals.last().is_none_or(|f| f.parent_active) {
                    self.check_extra_tokens(line, 2);
                }
                match conditionals.last_mut() {
                    Some(frame) => {
                        // **C 6.10.1p4: one `#else` per group.** `saw_else` and not `taken`:
                        // `#if 1` sets `taken` before any `#else` is written, so keying on it
                        // would report the first one.
                        let again = frame.saw_else;
                        frame.saw_else = true;
                        frame.active = frame.parent_active && !frame.taken;
                        frame.taken = true;
                        if again {
                            self.diagnostics.push(Diagnostic {
                                span: line[0].token.span,
                                message: "`#else` after `#else`".into(),
                            });
                        }
                    }
                    None => self.diagnostics.push(Diagnostic {
                        span: line[0].token.span,
                        message: "`#else` without `#if`".into(),
                    }),
                }
            }
            Some("endif") => {
                // **The frame being closed, not the one enclosing it.** Read before the pop, so
                // that `#if 0 / #endif junk` is reported — the group is live even though its
                // branch is not — while an `#endif` inside a skipped region stays silent.
                if conditionals.last().is_none_or(|f| f.parent_active) {
                    self.check_extra_tokens(line, 2);
                }
                if conditionals.pop().is_none() {
                    self.diagnostics.push(Diagnostic {
                        span: line[0].token.span,
                        message: "`#endif` without `#if`".into(),
                    });
                }
            }
            _ if !active => {}
            Some("define") => self.define(line),
            Some("undef") => {
                if self.check_macro_name_present(line) {
                    self.check_extra_tokens(line, 3);
                }
                if line.get(2).is_some_and(|t| t.text == "defined") {
                    self.diagnostics.push(Diagnostic {
                        span: line[2].token.span,
                        message: "`defined` cannot be used as a macro name".into(),
                    });
                }
                if let Some(name) = line.get(2)
                    && let Some(index) = self.by_name.remove(&name.text)
                {
                    self.macros[index].def.undef_span = Some(name.token.span);
                }
            }
            Some("line") => {
                // A number and optionally a file string, so four tokens with the `#` and the name.
                self.check_extra_tokens(line, 4);
                if let Some(number) = line.get(2)
                    && let Ok(reported_start) = number.text.parse::<u32>()
                    && let Some(loc) = self.source_map.lookup_loc(number.token.span.lo)
                {
                    let file = line.get(3).and_then(|token| {
                        token
                            .text
                            .strip_prefix('"')
                            .and_then(|text| text.strip_suffix('"'))
                            .map(str::to_owned)
                    });
                    self.line_overrides
                        .entry(loc.file)
                        .or_default()
                        .push(LineOverride {
                            physical_start: loc.line + 1,
                            reported_start,
                            file,
                        });
                }
            }
            Some("error" | "warning") => {
                let message = line
                    .get(2..)
                    .unwrap_or_default()
                    .iter()
                    .map(|token| token.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                self.diagnostics.push(Diagnostic {
                    span: line.get(1).map_or(Span::DUMMY, |token| token.token.span),
                    message: format!("#{}: {message}", directive.unwrap_or_default()),
                });
            }
            Some("pragma") => self.record_pragma_tokens(line.get(2..).unwrap_or_default()),
            Some(other) => self.diagnostics.push(Diagnostic {
                span: line[1].token.span,
                message: format!("unsupported preprocessing directive #{other}"),
            }),
            None => {}
        }
    }

    fn record_pragma_tokens(&mut self, tokens: &[Tok]) {
        let Some(first) = tokens.first() else { return };
        self.pragmas.push(PragmaRecord {
            span: extent(tokens).unwrap_or(first.token.span),
            text: tokens
                .iter()
                .map(|token| token.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        });
    }

    fn eval_if(
        &mut self,
        tokens: &[Tok],
        directive: Span,
        current: &Path,
        loader: &mut dyn FileLoader,
    ) -> bool {
        let mut prepared = Vec::new();
        let mut i = 0;
        while i < tokens.len() {
            if tokens[i].text == "__has_include"
                && tokens.get(i + 1).is_some_and(|token| token.text == "(")
                && let Some(close) = tokens[i + 2..]
                    .iter()
                    .position(|token| token.text == ")")
                    .map(|offset| i + 2 + offset)
            {
                let value = self.probe_include(&tokens[i + 2..close], current, loader);
                prepared.push(synthetic_number(
                    if value { "1" } else { "0" },
                    tokens[i].token.span,
                ));
                i = close + 1;
            } else if tokens[i].text == "defined" {
                let parenthesized = tokens.get(i + 1).is_some_and(|t| t.text == "(");
                let name_index = i + if parenthesized { 2 } else { 1 };
                // **`defined` is rewritten before the expression is parsed**, so a malformed one
                // never reaches `primary` and could not be caught by the "ends early" arm there.
                // The operand must exist and be an identifier, and a parenthesized one must
                // close. `#if defined`, `#if defined(` and `#if defined(A` all produced a
                // synthetic `0` and no complaint.
                let named = tokens
                    .get(name_index)
                    .is_some_and(|t| matches!(t.token.kind, PpTokenKind::Ident(_)));
                let closed =
                    !parenthesized || tokens.get(name_index + 1).is_some_and(|t| t.text == ")");
                if !named || !closed {
                    self.diagnostics.push(Diagnostic {
                        span: tokens[i].token.span,
                        message: "`#if` expression ends early".into(),
                    });
                    return false;
                }
                let value = tokens
                    .get(name_index)
                    .is_some_and(|name| self.by_name.contains_key(&name.text));
                prepared.push(synthetic_number(
                    if value { "1" } else { "0" },
                    tokens[i].token.span,
                ));
                i = name_index + if parenthesized { 2 } else { 1 };
            } else {
                prepared.push(tokens[i].clone());
                i += 1;
            }
        }
        let expanded = self.expand(prepared);
        let (value, parsed, ended_early) = {
            let mut parser = ExprParser {
                tokens: &expanded,
                pos: 0,
                diagnostics: &mut self.diagnostics,
                pedantic: self.config.pedantic,
                nesting: 0,
                nesting_diagnosed: false,
                ended_early: false,
                directive,
            };
            let value = parser.expression(true);
            (value, parser.pos, parser.ended_early)
        };
        // **An expression that ran out has already been reported**, and the tokens it did not
        // reach are a consequence rather than a second fault (contract 20). `#if (1` would
        // otherwise say both "ends early" and "unsupported token `)`" — of a `)` that is missing,
        // not present.
        if parsed != expanded.len() && !ended_early {
            self.diagnostics.push(Diagnostic {
                span: expanded[parsed].token.span,
                message: format!(
                    "unsupported token `{}` in #if expression",
                    expanded[parsed].text
                ),
            });
        }
        value.truth()
    }

    /// **A directive that takes a fixed number of tokens takes no more** (C 6.10p1).
    ///
    /// `want` is how long the whole line may be, counting the `#` and the directive name — so 2
    /// for `#endif` and `#else`, 3 for the ones naming a macro.
    ///
    /// The caller decides *whether* to ask, and that is the whole subtlety: none of this is
    /// diagnosed in skipped text, and the activity that governs is the **enclosing** region's,
    /// not the current branch's. `#if 0 / #else junk` is an error — the group itself is live even
    /// though its first branch is not — while the same line nested inside another `#if 0` is not.
    fn check_extra_tokens(&mut self, line: &[Tok], want: usize) {
        if line.len() > want {
            self.diagnostics.push(Diagnostic {
                span: line[want].token.span,
                message: format!(
                    "extra tokens at the end of the `#{}` directive",
                    line.get(1).map_or("", |t| t.text.as_str())
                ),
            });
        }
    }

    /// **`#ifdef`, `#ifndef` and `#undef` name a macro** (C 6.10.1p1, 6.10.3.5p1).
    ///
    /// Its own check rather than a lower bound inside `check_extra_tokens`, because it fails the
    /// other way and a reader needs to be told which: "extra tokens" and "no macro name" are
    /// different mistakes, and one sentence for both is the kind of report 023 §9 rules out.
    fn check_macro_name_present(&mut self, line: &[Tok]) -> bool {
        if line.len() < 3 {
            self.diagnostics.push(Diagnostic {
                span: line[0].token.span,
                message: format!(
                    "no macro name given in the `#{}` directive",
                    line.get(1).map_or("", |t| t.text.as_str())
                ),
            });
            return false;
        }
        true
    }

    /// C 6.10.3's constraints on a macro definition, checked where the parts are still separate.
    ///
    /// Four rules, and the reason they are one function is that each needs a different piece of
    /// the definition — the parameter list, the replacement list, and whether the macro is
    /// function-like at all — which are assembled into a `MacroDef` immediately after this and are
    /// harder to ask about there.
    fn check_macro_constraints(
        &mut self,
        name_tok: &Tok,
        function_like: bool,
        params: &[String],
        variadic_name: &Option<String>,
        std_variadic: bool,
        body: &[Tok],
    ) {
        let at = |t: &Tok| t.token.span;
        // **`defined` is not a macro name** (C 6.10.8p4). Reserved in both directions — `#undef`
        // is handled at its own arm — because a macro called `defined` would change what
        // `#if defined(X)` means, and the operator has no way to escape it.
        if name_tok.text == "defined" {
            self.diagnostics.push(Diagnostic {
                span: at(name_tok),
                message: "`defined` cannot be used as a macro name".into(),
            });
        }
        // **6.10.3p6: the parameters are distinct.** Quadratic on purpose — a parameter list long
        // enough for that to matter does not exist, and a set would need the names interned.
        for (i, p) in params.iter().enumerate() {
            if params[..i].contains(p) {
                self.diagnostics.push(Diagnostic {
                    span: at(name_tok),
                    message: format!("duplicate macro parameter `{p}`"),
                });
                break;
            }
        }
        // **6.10.3p5: `__VA_ARGS__` belongs to a variadic macro's replacement list and nowhere
        // else** — not in an object-like macro, not in a non-variadic function-like one, and not
        // as the name being defined.
        let variadic = std_variadic || variadic_name.is_some();
        if name_tok.text == "__VA_ARGS__" {
            self.diagnostics.push(Diagnostic {
                span: at(name_tok),
                message: "`__VA_ARGS__` cannot be the name of a macro".into(),
            });
        } else if !variadic && let Some(t) = body.iter().find(|t| t.text == "__VA_ARGS__") {
            self.diagnostics.push(Diagnostic {
                span: at(t),
                message: "`__VA_ARGS__` can only appear in a variadic macro's replacement list"
                    .into(),
            });
        }
        // **6.10.3.3p1: `##` appears at neither end.** Object-like macros too — unlike `#` below,
        // this rule says nothing about the kind of macro, because a paste has no left or right
        // operand there either.
        let is_paste = |t: &Tok| matches!(t.token.kind, PpTokenKind::Punct(Punct::HashHash));
        if body.first().is_some_and(is_paste) || body.last().is_some_and(is_paste) {
            self.diagnostics.push(Diagnostic {
                span: at(name_tok),
                message: "`##` cannot appear at either end of a macro's replacement list".into(),
            });
        }
        // **6.10.3.2p1: `#` is followed by a parameter — in a *function-like* macro.** In an
        // object-like one `#` is not an operator at all, so `#define S # a` is ordinary tokens
        // and must stay silent.
        if function_like {
            let names = |t: &Tok| {
                params.contains(&t.text)
                    || variadic_name.as_deref() == Some(t.text.as_str())
                    || (std_variadic && t.text == "__VA_ARGS__")
            };
            for (i, t) in body.iter().enumerate() {
                if matches!(t.token.kind, PpTokenKind::Punct(Punct::Hash))
                    && !body.get(i + 1).is_some_and(names)
                {
                    self.diagnostics.push(Diagnostic {
                        span: at(t),
                        message: "`#` is not followed by a macro parameter".into(),
                    });
                    break;
                }
            }
        }
    }

    fn define(&mut self, line: &[Tok]) {
        let Some(name_tok) = line.get(2) else { return };
        let name = name_tok.text.clone();
        let mut body_start = 3;
        let mut params = Vec::new();
        let mut variadic_name = None;
        let mut std_variadic = false;
        let function_like = line.get(3).is_some_and(|token| {
            !token.token.leading_space
                && matches!(token.token.kind, PpTokenKind::Punct(Punct::LParen))
        });
        if function_like {
            let mut i = 4;
            while i < line.len() {
                if matches!(line[i].token.kind, PpTokenKind::Punct(Punct::RParen)) {
                    body_start = i + 1;
                    break;
                }
                if matches!(line[i].token.kind, PpTokenKind::Punct(Punct::Ellipsis)) {
                    std_variadic = true;
                    i += 1;
                    continue;
                }
                if matches!(line[i].token.kind, PpTokenKind::Ident(_)) {
                    let param = line[i].text.clone();
                    if line.get(i + 1).is_some_and(|t| {
                        matches!(t.token.kind, PpTokenKind::Punct(Punct::Ellipsis))
                    }) {
                        variadic_name = Some(param);
                        i += 2;
                        continue;
                    }
                    params.push(param);
                }
                i += 1;
            }
        }
        let mut body = line.get(body_start..).unwrap_or_default().to_vec();
        self.check_macro_constraints(
            name_tok,
            function_like,
            &params,
            &variadic_name,
            std_variadic,
            &body,
        );
        strip_va_opt(&mut body, &mut self.diagnostics);
        let body_extent = extent(&body).unwrap_or(Span::new(
            name_tok.token.span.hi,
            name_tok.token.span.hi,
            ExpnCtx::ROOT,
        ));
        let id = self
            .source_map
            .add_macro(&name, name_tok.token.span, body_extent);
        let index = self.macros.len();
        let name_symbol = self.lex_session.intern_symbol(&name);
        let kind = if function_like {
            MacroKind::FunctionLike {
                params: params
                    .iter()
                    .map(|parameter| self.lex_session.intern_symbol(parameter))
                    .collect(),
                variadic: if variadic_name.is_some() {
                    Variadic::Named(
                        self.lex_session
                            .intern_symbol(variadic_name.as_deref().unwrap_or_default()),
                    )
                } else if std_variadic {
                    Variadic::Std
                } else {
                    Variadic::No
                },
            }
        } else {
            MacroKind::ObjectLike
        };
        self.macros.push(StoredMacro {
            def: MacroDef {
                id,
                name: name_symbol,
                kind,
                body: body.iter().map(|t| t.token.clone()).collect(),
                def_span: name_tok.token.span,
                undef_span: None,
            },
            name: name.clone(),
            params,
            variadic_name,
            std_variadic,
            body,
        });
        if let Some(previous) = self.by_name.get(&name).copied() {
            let equivalent = {
                let old = &self.macros[previous];
                let new = &self.macros[index];
                old.params == new.params
                    && old.variadic_name == new.variadic_name
                    && old.std_variadic == new.std_variadic
                    // **C 6.10.3p2 compares spelling *and* white-space separation.** Two lists
                    // of the same tokens are still different definitions if one writes `1 + 2`
                    // and the other `1+2`; the standard would rather say so than pick one.
                    //
                    // The **first** token's `leading_space` is skipped, because space before the
                    // list is not separation *within* it — `#define A   1 + 2` and
                    // `#define A 1 + 2` are the same definition, and comparing it would make
                    // every indented header a redefinition of itself.
                    && old
                        .body
                        .iter()
                        .enumerate()
                        .map(|(i, t)| (&t.text, i != 0 && t.token.leading_space))
                        .eq(new
                            .body
                            .iter()
                            .enumerate()
                            .map(|(i, t)| (&t.text, i != 0 && t.token.leading_space)))
            };
            self.macros[previous].def.undef_span = Some(name_tok.token.span);
            if !equivalent {
                self.diagnostics.push(Diagnostic {
                    span: name_tok.token.span,
                    message: format!("redefinition of macro `{name}`"),
                });
            }
        }
        self.by_name.insert(name, index);
    }

    fn expand(&mut self, input: Vec<Tok>) -> Vec<Tok> {
        if self.expansion_depth >= self.config.max_macro_expansion_depth {
            self.diagnostics.push(Diagnostic {
                span: input.first().map_or(Span::DUMMY, |token| token.token.span),
                message: format!(
                    "maximum macro expansion depth {} exceeded",
                    self.config.max_macro_expansion_depth
                ),
            });
            return input;
        }
        self.expansion_depth += 1;
        let output = self.expand_inner(input);
        self.expansion_depth -= 1;
        output
    }

    fn expand_inner(&mut self, input: Vec<Tok>) -> Vec<Tok> {
        let mut input: VecDeque<_> = input.into();
        let mut output = Vec::new();
        while let Some(token) = input.pop_front() {
            // **`_Pragma` decides, rather than declining to match** (C 6.10.9p1). The chain
            // below used to be conditional all the way down, so an operand that was not one
            // string literal fell through untouched — to 013, which has never heard of
            // `_Pragma` and answered "expected a declaration" three to five times. Recognising
            // the *operator* and then judging the *operand* separates "this is not a `_Pragma`"
            // from "this is a bad one".
            if token.text == "_Pragma" {
                let parsed = input
                    .front()
                    .is_some_and(|t| t.text == "(")
                    .then(|| parse_args(&input, 0))
                    .flatten();
                let one_string = match &parsed {
                    Some((args, _)) if args.len() == 1 => {
                        let expanded = self.expand(args[0].clone());
                        (expanded.len() == 1
                            && matches!(expanded[0].token.kind, PpTokenKind::StringLit { .. }))
                        .then_some(expanded)
                    }
                    _ => None,
                };
                let close = parsed.as_ref().map_or(0, |(_, c)| *c);
                let Some(expanded) = one_string else {
                    self.diagnostics.push(Diagnostic {
                        span: token.token.span,
                        message: "`_Pragma` takes one string literal".into(),
                    });
                    // **Consumed, not left behind.** The tokens are what 013 would have flailed
                    // over; dropping them is what turns five sentences into one.
                    if parsed.is_some() {
                        input.drain(..=close);
                    }
                    continue;
                };
                {
                    self.pragmas.push(PragmaRecord {
                        span: token.token.span,
                        text: destringize_pragma(&expanded[0].text),
                    });
                    self.source_map.add_expansion(
                        token.token.span.ctx,
                        None,
                        token.token.span,
                        span_from_ends(token.token.span, input[close].token.span),
                        Vec::new(),
                        ExpnKind::Pragma,
                    );
                    input.drain(..=close);
                    continue;
                }
            }
            let Some(&macro_index) = self.by_name.get(&token.text) else {
                output.push(token);
                continue;
            };
            let def = self.macros[macro_index].clone();
            if token.hide.contains(&def.def.id) {
                output.push(token);
                continue;
            }
            if is_builtin(&def.name) {
                output.push(self.expand_builtin(&token, &def));
                continue;
            }
            match def.def.kind {
                MacroKind::ObjectLike => {
                    let expn = self.source_map.add_expansion(
                        token.token.span.ctx,
                        Some(def.def.id),
                        token.token.span,
                        token.token.span,
                        Vec::new(),
                        ExpnKind::ObjectLike,
                    );
                    let replacement: Vec<_> = def
                        .body
                        .iter()
                        .cloned()
                        .map(|mut body| {
                            body.token.span.ctx = expn;
                            body.hide.extend(&token.hide);
                            body.hide.insert(def.def.id);
                            body
                        })
                        .collect();
                    let replacement = self.paste(replacement, expn);
                    // C11 §6.10.3.4 ¶1: rescan the replacement list *together with all
                    // subsequent source tokens*. This is what lets `#define A B` turn
                    // `A(1)` into an invocation of function-like `B`.
                    for token in replacement.into_iter().rev() {
                        input.push_front(token);
                    }
                }
                MacroKind::FunctionLike { .. } => {
                    if !input
                        .front()
                        .is_some_and(|t| matches!(t.token.kind, PpTokenKind::Punct(Punct::LParen)))
                    {
                        output.push(token);
                        continue;
                    }
                    let Some((args, close)) = parse_args(&input, 0) else {
                        output.push(token);
                        continue;
                    };
                    let actual = if args.len() == 1 && args[0].is_empty() && def.params.is_empty() {
                        0
                    } else {
                        args.len()
                    };
                    let valid_arity = if def.std_variadic || def.variadic_name.is_some() {
                        actual >= def.params.len()
                    } else {
                        actual == def.params.len()
                    };
                    if !valid_arity {
                        self.diagnostics.push(Diagnostic {
                            span: token.token.span,
                            message: format!(
                                "macro `{}` expects {} argument(s), but {} provided",
                                def.name,
                                def.params.len(),
                                actual
                            ),
                        });
                        output.push(token);
                        continue;
                    }
                    let close_token = input[close].clone();
                    input.drain(..=close);
                    let replacement = self.expand_function(&token, &close_token, &def, args);
                    for token in replacement.into_iter().rev() {
                        input.push_front(token);
                    }
                }
            }
        }
        output
    }

    fn expand_builtin(&mut self, call: &Tok, def: &StoredMacro) -> Tok {
        let (reported_line, reported_file) = self.reported_line_file(call.token.span);
        let text = match def.name.as_str() {
            "__COUNTER__" => {
                let value = self.counter.to_string();
                self.counter += 1;
                value
            }
            "__LINE__" => reported_line.to_string(),
            "__FILE__" => format!("\"{reported_file}\""),
            "__DATE__" => format!("\"{}\"", self.config.date),
            "__TIME__" => format!("\"{}\"", self.config.time),
            _ => String::new(),
        };
        let expn = self.source_map.add_expansion(
            call.token.span.ctx,
            Some(def.def.id),
            call.token.span,
            call.token.span,
            Vec::new(),
            ExpnKind::Builtin,
        );
        Tok {
            token: PpToken {
                kind: if matches!(def.name.as_str(), "__DATE__" | "__TIME__" | "__FILE__") {
                    PpTokenKind::StringLit {
                        prefix: EncPrefix::None,
                    }
                } else {
                    PpTokenKind::Number
                },
                span: Span::new(call.token.span.lo, call.token.span.hi, expn),
                leading_space: call.token.leading_space,
                bol: call.token.bol,
            },
            text,
            hide: call.hide.clone(),
        }
    }

    fn reported_line_file(&self, span: Span) -> (u32, String) {
        let Some(loc) = self.source_map.expansion_loc(span) else {
            return (0, String::new());
        };
        let mut line = loc.line;
        let mut file = self.source_map.file(loc.file).path().display().to_string();
        if let Some(overrides) = self.line_overrides.get(&loc.file)
            && let Some(override_) = overrides
                .iter()
                .rev()
                .find(|override_| override_.physical_start <= loc.line)
        {
            line = override_.reported_start + (loc.line - override_.physical_start);
            if let Some(overridden) = &override_.file {
                file.clone_from(overridden);
            }
        }
        (line, file)
    }

    fn expand_function(
        &mut self,
        call: &Tok,
        close: &Tok,
        def: &StoredMacro,
        mut args: Vec<Vec<Tok>>,
    ) -> Vec<Tok> {
        if args.len() == 1 && args[0].is_empty() && def.params.is_empty() {
            args.clear();
        }
        let fixed = def.params.len();
        let mut raw_by_name: BTreeMap<String, Vec<Tok>> = BTreeMap::new();
        for (index, name) in def.params.iter().enumerate() {
            raw_by_name.insert(name.clone(), args.get(index).cloned().unwrap_or_default());
        }
        if def.std_variadic || def.variadic_name.is_some() {
            let name = def
                .variadic_name
                .clone()
                .unwrap_or_else(|| "__VA_ARGS__".into());
            let mut rest = Vec::new();
            for (n, arg) in args.iter().skip(fixed).enumerate() {
                if n != 0 {
                    rest.push(synthetic_punct(",", Punct::Comma, call.token.span));
                }
                rest.extend(arg.clone());
            }
            raw_by_name.insert(name, rest);
        }
        let arg_spans = args
            .iter()
            .map(|arg| {
                extent(arg).unwrap_or(Span::new(
                    close.token.span.lo,
                    close.token.span.lo,
                    close.token.span.ctx,
                ))
            })
            .collect();
        let expn = self.source_map.add_expansion(
            call.token.span.ctx,
            Some(def.def.id),
            call.token.span,
            span_from_ends(call.token.span, close.token.span),
            arg_spans,
            ExpnKind::FunctionLike,
        );

        let mut expansion_order = def.params.clone();
        if def.std_variadic || def.variadic_name.is_some() {
            expansion_order.push(
                def.variadic_name
                    .clone()
                    .unwrap_or_else(|| "__VA_ARGS__".into()),
            );
        }
        let mut expanded_by_name = BTreeMap::new();
        for name in expansion_order {
            if needs_preexpansion(&def.body, &name) {
                let raw = raw_by_name.get(&name).cloned().unwrap_or_default();
                expanded_by_name.insert(name, self.expand(raw));
            }
        }
        let mut replacement = Vec::new();
        let mut i = 0;
        while i < def.body.len() {
            if matches!(def.body[i].token.kind, PpTokenKind::Punct(Punct::Hash))
                && let Some(param) = def.body.get(i + 1)
                && let Some(raw) = raw_by_name.get(&param.text)
            {
                replacement.push(self.stringize(&def.body[i], raw, expn));
                i += 2;
                continue;
            }
            let body = &def.body[i];
            if let Some(raw) = raw_by_name.get(&body.text) {
                let adjacent_paste =
                    def.body.get(i.wrapping_sub(1)).is_some_and(|t| {
                        matches!(t.token.kind, PpTokenKind::Punct(Punct::HashHash))
                    }) || def.body.get(i + 1).is_some_and(|t| {
                        matches!(t.token.kind, PpTokenKind::Punct(Punct::HashHash))
                    });
                let selected = if adjacent_paste {
                    raw.clone()
                } else {
                    expanded_by_name
                        .get(&body.text)
                        .cloned()
                        .unwrap_or_default()
                };
                if adjacent_paste && selected.is_empty() {
                    replacement.push(Tok {
                        token: PpToken {
                            kind: PpTokenKind::Other('\0'),
                            span: Span::new(body.token.span.lo, body.token.span.lo, expn),
                            leading_space: false,
                            bol: false,
                        },
                        text: String::new(),
                        hide: HideSet::default(),
                    });
                }
                for (index, mut arg) in selected.into_iter().enumerate() {
                    if arg.token.span.ctx.is_root() {
                        arg.token.span.ctx = expn;
                    }
                    if index == 0 {
                        arg.token.leading_space = body.token.leading_space;
                    }
                    arg.hide.extend(&call.hide);
                    arg.hide.insert(def.def.id);
                    replacement.push(arg);
                }
            } else {
                let mut copied = body.clone();
                copied.token.span.ctx = expn;
                copied.hide.extend(&call.hide);
                copied.hide.insert(def.def.id);
                replacement.push(copied);
            }
            i += 1;
        }
        self.paste(replacement, expn)
    }

    fn stringize(&mut self, operator: &Tok, raw: &[Tok], parent: ExpnCtx) -> Tok {
        let mut inside = String::new();
        for (index, token) in raw.iter().enumerate() {
            if index != 0 && token.token.leading_space {
                inside.push(' ');
            }
            let escape = matches!(
                token.token.kind,
                PpTokenKind::CharLit { .. } | PpTokenKind::StringLit { .. }
            );
            for ch in token.text.chars() {
                if escape && matches!(ch, '\\' | '"') {
                    inside.push('\\');
                }
                inside.push(ch);
            }
        }
        let expn = self.source_map.add_expansion(
            parent,
            None,
            Span::new(operator.token.span.lo, operator.token.span.hi, parent),
            Span::new(operator.token.span.lo, operator.token.span.hi, parent),
            Vec::new(),
            ExpnKind::Stringize,
        );
        Tok {
            token: PpToken {
                kind: PpTokenKind::StringLit {
                    prefix: EncPrefix::None,
                },
                span: Span::new(operator.token.span.lo, operator.token.span.lo, expn),
                leading_space: operator.token.leading_space,
                bol: operator.token.bol,
            },
            text: format!("\"{inside}\""),
            hide: HideSet::default(),
        }
    }

    fn paste(&mut self, input: Vec<Tok>, parent: ExpnCtx) -> Vec<Tok> {
        let mut output: Vec<Tok> = Vec::new();
        let mut i = 0;
        while i < input.len() {
            if matches!(input[i].token.kind, PpTokenKind::Punct(Punct::HashHash)) {
                let right = input.get(i + 1).cloned();
                if right.is_none() {
                    if output
                        .last()
                        .is_some_and(|t| matches!(t.token.kind, PpTokenKind::Punct(Punct::Comma)))
                    {
                        output.pop();
                    }
                    i += 1;
                    continue;
                }
                let Some(left) = output.pop() else {
                    i += 2;
                    continue;
                };
                let right = right.unwrap();
                if right.text.is_empty() {
                    if !matches!(left.token.kind, PpTokenKind::Punct(Punct::Comma)) {
                        output.push(left);
                    }
                    i += 2;
                    continue;
                }
                // GNU `, ## args`: `##` suppresses the comma only for an empty
                // variadic argument. With tokens present it is not ordinary token
                // pasting; retain comma and argument as two pp-tokens.
                if matches!(left.token.kind, PpTokenKind::Punct(Punct::Comma)) {
                    output.push(left);
                    output.push(right);
                    i += 2;
                    continue;
                }
                if left.text.is_empty() {
                    output.push(right);
                    i += 2;
                    continue;
                }
                let text = format!("{}{}", left.text, right.text);
                let Some(kind) = self.classify_paste(&text) else {
                    self.diagnostics.push(Diagnostic {
                        span: input[i].token.span,
                        message: format!("token paste `{text}` is not one preprocessing token"),
                    });
                    output.push(left);
                    output.push(right);
                    i += 2;
                    continue;
                };
                let expn = self.source_map.add_expansion(
                    parent,
                    None,
                    input[i].token.span,
                    input[i].token.span,
                    Vec::new(),
                    ExpnKind::Paste,
                );
                output.push(Tok {
                    token: PpToken {
                        kind,
                        span: Span::new(input[i].token.span.lo, input[i].token.span.lo, expn),
                        leading_space: left.token.leading_space,
                        bol: left.token.bol,
                    },
                    text,
                    hide: {
                        let mut hide = left.hide.clone();
                        hide.extend(&right.hide);
                        hide
                    },
                });
                i += 2;
            } else {
                output.push(input[i].clone());
                i += 1;
            }
        }
        output.retain(|token| !token.text.is_empty());
        output
    }

    fn classify_paste(&self, text: &str) -> Option<PpTokenKind> {
        let mut map = SourceMap::new();
        let file = map.add_file("<paste>", text);
        let lexed = self.lex_session.lex(&map, file, LexConfig::default());
        let tokens: Vec<_> = lexed
            .tokens()
            .iter()
            .enumerate()
            .filter(|(_, token)| !matches!(token.kind, PpTokenKind::Eof))
            .collect();
        if tokens.len() == 1 && lexed.text_at(tokens[0].0) == Some(text) {
            Some(tokens[0].1.kind.clone())
        } else {
            None
        }
    }
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "__LINE__" | "__FILE__" | "__COUNTER__" | "__DATE__" | "__TIME__"
    )
}

fn extent(tokens: &[Tok]) -> Option<Span> {
    Some(span_from_ends(
        tokens.first()?.token.span,
        tokens.last()?.token.span,
    ))
}

fn span_from_ends(first: Span, last: Span) -> Span {
    if first.lo <= last.hi {
        Span::new(first.lo, last.hi, first.ctx)
    } else {
        // A macro-generated invocation can draw its first and last token from
        // definitions laid out in the opposite global-source order. There is no honest
        // contiguous envelope; retain the first spelling span instead of fabricating an
        // inverted/cross-file range.
        Span::new(first.lo, first.hi, first.ctx)
    }
}

fn needs_preexpansion(body: &[Tok], parameter: &str) -> bool {
    body.iter().enumerate().any(|(index, token)| {
        if token.text != parameter {
            return false;
        }
        let stringized =
            index > 0 && matches!(body[index - 1].token.kind, PpTokenKind::Punct(Punct::Hash));
        let pasted = (index > 0
            && matches!(
                body[index - 1].token.kind,
                PpTokenKind::Punct(Punct::HashHash)
            ))
            || body
                .get(index + 1)
                .is_some_and(|next| matches!(next.token.kind, PpTokenKind::Punct(Punct::HashHash)));
        !stringized && !pasted
    })
}

fn strip_va_opt(body: &mut Vec<Tok>, diagnostics: &mut Vec<Diagnostic>) {
    let mut index = 0;
    while index < body.len() {
        if body[index].text != "__VA_OPT__" {
            index += 1;
            continue;
        }
        diagnostics.push(Diagnostic {
            span: body[index].token.span,
            message: "__VA_OPT__ is outside chiero's v1 preprocessing scope".into(),
        });
        let mut end = index + 1;
        if body.get(end).is_some_and(|token| token.text == "(") {
            let mut depth = 0_u32;
            while end < body.len() {
                match body[end].token.kind {
                    PpTokenKind::Punct(Punct::LParen) => depth += 1,
                    PpTokenKind::Punct(Punct::RParen) => {
                        depth -= 1;
                        end += 1;
                        if depth == 0 {
                            break;
                        }
                        continue;
                    }
                    _ => {}
                }
                end += 1;
            }
        }
        body.drain(index..end);
    }
}

fn parse_args(input: &VecDeque<Tok>, open: usize) -> Option<(Vec<Vec<Tok>>, usize)> {
    let mut args = vec![Vec::new()];
    let mut depth = 0_u32;
    let mut i = open + 1;
    while i < input.len() {
        match input[i].token.kind {
            PpTokenKind::Punct(Punct::LParen) => {
                depth += 1;
                args.last_mut()?.push(input[i].clone());
            }
            PpTokenKind::Punct(Punct::RParen) if depth == 0 => return Some((args, i)),
            PpTokenKind::Punct(Punct::RParen) => {
                depth -= 1;
                args.last_mut()?.push(input[i].clone());
            }
            PpTokenKind::Punct(Punct::Comma) if depth == 0 => args.push(Vec::new()),
            _ => args.last_mut()?.push(input[i].clone()),
        }
        i += 1;
    }
    None
}

fn parse_configured_name(declaration: &str) -> (String, Vec<String>) {
    let Some((name, parameters)) = declaration.split_once('(') else {
        return (declaration.to_owned(), Vec::new());
    };
    let Some(parameters) = parameters.strip_suffix(')') else {
        return (declaration.to_owned(), Vec::new());
    };
    (
        name.to_owned(),
        parameters
            .split(',')
            .map(str::trim)
            .filter(|parameter| !parameter.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

fn synthetic_punct(text: &str, punct: Punct, at: Span) -> Tok {
    Tok {
        token: PpToken {
            kind: PpTokenKind::Punct(punct),
            span: Span::new(at.lo, at.lo, at.ctx),
            leading_space: false,
            bol: false,
        },
        text: text.into(),
        hide: HideSet::default(),
    }
}

fn synthetic_number(text: &str, at: Span) -> Tok {
    Tok {
        token: PpToken {
            kind: PpTokenKind::Number,
            span: at,
            leading_space: false,
            bol: false,
        },
        text: text.into(),
        hide: HideSet::default(),
    }
}

fn destringize_pragma(text: &str) -> String {
    let inside = text
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .unwrap_or(text);
    let mut output = String::new();
    let mut chars = inside.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some(next @ ('\\' | '"')) => output.push(next),
                Some(next) => {
                    output.push('\\');
                    output.push(next);
                }
                None => output.push('\\'),
            }
        } else {
            output.push(ch);
        }
    }
    output
}

struct ExprParser<'a> {
    tokens: &'a [Tok],
    pos: usize,
    diagnostics: &'a mut Vec<Diagnostic>,
    pedantic: bool,
    nesting: usize,
    nesting_diagnosed: bool,
    /// Whether the expression ran out of tokens, so the trailing-token complaint stands down.
    ended_early: bool,
    /// The `#if` or `#elif` token, for the case where there is **no** token to point at.
    ///
    /// `#if` with an empty expression has an empty token list, so "the last token" is nothing and
    /// the span covered no text — a diagnostic an editor highlights as a caret between two
    /// characters. Wave 373 built the gate that found it.
    directive: Span,
}

#[derive(Copy, Clone)]
struct IfValue {
    bits: u64,
    unsigned: bool,
}

impl IfValue {
    const ZERO: Self = Self {
        bits: 0,
        unsigned: false,
    };

    fn signed(value: i64) -> Self {
        Self {
            bits: value as u64,
            unsigned: false,
        }
    }

    fn boolean(value: bool) -> Self {
        Self::signed(i64::from(value))
    }

    fn truth(self) -> bool {
        self.bits != 0
    }

    fn usual(self, other: Self) -> (Self, Self, bool) {
        let unsigned = self.unsigned || other.unsigned;
        (
            Self { unsigned, ..self },
            Self { unsigned, ..other },
            unsigned,
        )
    }
}

impl ExprParser<'_> {
    fn expression(&mut self, live: bool) -> IfValue {
        let mut value = self.conditional(live);
        while self.take(",") {
            value = self.conditional(live);
        }
        value
    }

    fn conditional(&mut self, live: bool) -> IfValue {
        let condition = self.logical_or(live);
        if !self.take("?") {
            return condition;
        }
        let yes = self.expression(live && condition.truth());
        if !self.take(":") {
            return yes;
        }
        let no = self.conditional(live && !condition.truth());
        // C 6.5.15: the result type is the usual arithmetic conversions of *both* arms, so the
        // arm that was not selected still decides whether the result is unsigned. `0 ? 1u : -1`
        // has value -1 and type `uintmax_t`, and so is not less than zero. `usual` only moves the
        // signedness — the bit pattern of a negative value is already its unsigned conversion.
        let (yes, no, _) = yes.usual(no);
        if condition.truth() { yes } else { no }
    }

    fn logical_or(&mut self, live: bool) -> IfValue {
        let mut left = self.logical_and(live);
        while self.take("||") {
            let right = self.logical_and(live && !left.truth());
            left = IfValue::boolean(left.truth() || right.truth());
        }
        left
    }

    fn logical_and(&mut self, live: bool) -> IfValue {
        let mut left = self.bitwise_or(live);
        while self.take("&&") {
            let right = self.bitwise_or(live && left.truth());
            left = IfValue::boolean(left.truth() && right.truth());
        }
        left
    }

    fn bitwise_or(&mut self, live: bool) -> IfValue {
        let mut left = self.bitwise_xor(live);
        while self.take("|") {
            let right = self.bitwise_xor(live);
            let unsigned = left.unsigned || right.unsigned;
            left = IfValue {
                bits: left.bits | right.bits,
                unsigned,
            };
        }
        left
    }

    fn bitwise_xor(&mut self, live: bool) -> IfValue {
        let mut left = self.bitwise_and(live);
        while self.take("^") {
            let right = self.bitwise_and(live);
            let unsigned = left.unsigned || right.unsigned;
            left = IfValue {
                bits: left.bits ^ right.bits,
                unsigned,
            };
        }
        left
    }

    fn bitwise_and(&mut self, live: bool) -> IfValue {
        let mut left = self.equality(live);
        while self.take("&") {
            let right = self.equality(live);
            let unsigned = left.unsigned || right.unsigned;
            left = IfValue {
                bits: left.bits & right.bits,
                unsigned,
            };
        }
        left
    }

    fn equality(&mut self, live: bool) -> IfValue {
        let mut left = self.relational(live);
        loop {
            if self.take("==") {
                let right = self.relational(live);
                left = IfValue::boolean(left.bits == right.bits);
            } else if self.take("!=") {
                let right = self.relational(live);
                left = IfValue::boolean(left.bits != right.bits);
            } else {
                return left;
            }
        }
    }

    fn relational(&mut self, live: bool) -> IfValue {
        let mut left = self.shift(live);
        loop {
            if self.take("<") {
                let right = self.shift(live);
                left = IfValue::boolean(compare(left, right, |a, b| a < b, |a, b| a < b));
            } else if self.take(">") {
                let right = self.shift(live);
                left = IfValue::boolean(compare(left, right, |a, b| a > b, |a, b| a > b));
            } else if self.take("<=") {
                let right = self.shift(live);
                left = IfValue::boolean(compare(left, right, |a, b| a <= b, |a, b| a <= b));
            } else if self.take(">=") {
                let right = self.shift(live);
                left = IfValue::boolean(compare(left, right, |a, b| a >= b, |a, b| a >= b));
            } else {
                return left;
            }
        }
    }

    fn shift(&mut self, live: bool) -> IfValue {
        let mut left = self.additive(live);
        loop {
            if self.take("<<") {
                let right = self.additive(live);
                left.bits = left.bits.wrapping_shl((right.bits & 63) as u32);
            } else if self.take(">>") {
                let right = self.additive(live);
                let count = (right.bits & 63) as u32;
                left.bits = if left.unsigned {
                    left.bits >> count
                } else {
                    ((left.bits as i64) >> count) as u64
                };
            } else {
                return left;
            }
        }
    }

    fn additive(&mut self, live: bool) -> IfValue {
        let mut left = self.multiplicative(live);
        loop {
            if self.take("+") {
                let right = self.multiplicative(live);
                let (a, b, unsigned) = left.usual(right);
                left = IfValue {
                    bits: a.bits.wrapping_add(b.bits),
                    unsigned,
                };
            } else if self.take("-") {
                let right = self.multiplicative(live);
                let (a, b, unsigned) = left.usual(right);
                left = IfValue {
                    bits: a.bits.wrapping_sub(b.bits),
                    unsigned,
                };
            } else {
                return left;
            }
        }
    }

    fn multiplicative(&mut self, live: bool) -> IfValue {
        let mut left = self.unary(live);
        loop {
            if self.take("*") {
                let right = self.unary(live);
                let (a, b, unsigned) = left.usual(right);
                left = IfValue {
                    bits: a.bits.wrapping_mul(b.bits),
                    unsigned,
                };
            } else if self.take("/") {
                let operator = self.tokens.get(self.pos.saturating_sub(1));
                let right = self.unary(live);
                if right.bits == 0 {
                    if live {
                        self.diagnostics.push(Diagnostic {
                            span: operator.map_or(Span::DUMMY, |t| t.token.span),
                            message: "division by zero in #if".into(),
                        });
                    }
                    left = IfValue::ZERO;
                } else {
                    left = divide(left, right, false);
                }
            } else if self.take("%") {
                let operator = self.tokens.get(self.pos.saturating_sub(1));
                let right = self.unary(live);
                if right.bits == 0 {
                    if live {
                        self.diagnostics.push(Diagnostic {
                            span: operator.map_or(Span::DUMMY, |t| t.token.span),
                            message: "modulo by zero in #if".into(),
                        });
                    }
                    left = IfValue::ZERO;
                } else {
                    left = divide(left, right, true);
                }
            } else {
                return left;
            }
        }
    }

    fn unary(&mut self, live: bool) -> IfValue {
        if self.take("!") {
            return IfValue::boolean(!self.unary(live).truth());
        }
        if self.take("~") {
            let value = self.unary(live);
            return IfValue {
                bits: !value.bits,
                unsigned: value.unsigned,
            };
        }
        if self.take("-") {
            let value = self.unary(live);
            return IfValue {
                bits: value.bits.wrapping_neg(),
                unsigned: value.unsigned,
            };
        }
        if self.take("+") {
            return self.unary(live);
        }
        self.primary(live)
    }

    fn primary(&mut self, live: bool) -> IfValue {
        if self.take("(") {
            if self.nesting >= 256 {
                if !self.nesting_diagnosed {
                    self.diagnostics.push(Diagnostic {
                        span: self
                            .tokens
                            .get(self.pos.saturating_sub(1))
                            .map_or(Span::DUMMY, |token| token.token.span),
                        message: "maximum #if nesting depth 256 exceeded".into(),
                    });
                    self.nesting_diagnosed = true;
                }
                let mut depth = 1_usize;
                while self.pos < self.tokens.len() && depth != 0 {
                    match self.tokens[self.pos].text.as_str() {
                        "(" => depth += 1,
                        ")" => depth -= 1,
                        _ => {}
                    }
                    self.pos += 1;
                }
                return IfValue::ZERO;
            }
            self.nesting += 1;
            let value = self.expression(live);
            if !self.take(")") {
                self.ends_early();
            }
            self.nesting -= 1;
            return value;
        }
        let Some(token) = self.tokens.get(self.pos) else {
            // **The token it needed and did not get**, which is the opposite failure from the
            // "unsupported token" the caller reports and looked the same from in here: both
            // arrive as `primary` having nothing to return, and only one had an arm.
            self.ends_early();
            return IfValue::ZERO;
        };
        self.pos += 1;
        // **C 6.10.1p1: the expression is an *integer* constant expression.** A character
        // constant is one and stays legal below; a floating constant and a string are not.
        if matches!(token.token.kind, PpTokenKind::StringLit { .. }) {
            self.report(token.token.span, "a string literal is not allowed in `#if`");
            return IfValue::ZERO;
        }
        if matches!(token.token.kind, PpTokenKind::Number) && is_floating_ppnumber(&token.text) {
            self.report(
                token.token.span,
                "a floating constant is not allowed in `#if`",
            );
            return IfValue::ZERO;
        }
        if matches!(token.token.kind, PpTokenKind::Ident(_)) {
            if live && self.pedantic {
                self.diagnostics.push(Diagnostic {
                    span: token.token.span,
                    message: format!("undefined identifier `{}` in #if", token.text),
                });
            }
            return IfValue::ZERO;
        }
        parse_if_literal(token)
    }

    /// **One complaint per `#if`.** A truncated expression makes every enclosing parser run out
    /// too, so `#if (1 +` would say it three times without this (contract 20).
    fn ends_early(&mut self) {
        if self.ended_early {
            return;
        }
        self.ended_early = true;
        let span = self
            .tokens
            .last()
            .map_or(self.directive, |token| token.token.span);
        self.diagnostics.push(Diagnostic {
            span,
            message: "`#if` expression ends early".into(),
        });
    }

    fn report(&mut self, span: Span, message: &str) {
        self.diagnostics.push(Diagnostic {
            span,
            message: message.into(),
        });
    }

    fn take(&mut self, text: &str) -> bool {
        if self.tokens.get(self.pos).is_some_and(|t| t.text == text) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
}

fn compare(
    left: IfValue,
    right: IfValue,
    signed: impl FnOnce(i64, i64) -> bool,
    unsigned: impl FnOnce(u64, u64) -> bool,
) -> bool {
    let (left, right, use_unsigned) = left.usual(right);
    if use_unsigned {
        unsigned(left.bits, right.bits)
    } else {
        signed(left.bits as i64, right.bits as i64)
    }
}

fn divide(left: IfValue, right: IfValue, remainder: bool) -> IfValue {
    let (left, right, unsigned) = left.usual(right);
    let bits = if unsigned {
        if remainder {
            left.bits % right.bits
        } else {
            left.bits / right.bits
        }
    } else {
        let left = left.bits as i64;
        let right = right.bits as i64;
        if remainder {
            left.checked_rem(right).unwrap_or(0) as u64
        } else {
            left.checked_div(right).unwrap_or(i64::MIN) as u64
        }
    };
    IfValue { bits, unsigned }
}

/// Whether a pp-number spells a **floating** constant (C 6.4.4.2).
///
/// Read from the spelling because the lexer's `Number` covers both — a pp-number is deliberately
/// one token class. A `.` anywhere settles it; otherwise it is the exponent that does, and which
/// letter introduces one depends on the radix: `e` for decimal, `p` for hexadecimal. Asking for
/// `e` in a hex number would call `0xe` floating, and asking for a trailing `f` would call `0xf`
/// floating — both of which are integers every header writes.
fn is_floating_ppnumber(text: &str) -> bool {
    if text.contains('.') {
        return true;
    }
    let hex = text.starts_with("0x") || text.starts_with("0X");
    let exponent = if hex { ['p', 'P'] } else { ['e', 'E'] };
    text.char_indices().any(|(i, c)| {
        i > 0
            && exponent.contains(&c)
            && text[i + c.len_utf8()..]
                .chars()
                .next()
                .is_some_and(|n| n.is_ascii_digit() || n == '+' || n == '-')
    })
}

fn parse_if_literal(token: &Tok) -> IfValue {
    if let PpTokenKind::CharLit { prefix } = token.token.kind {
        // The prefix is the type. `u'x'` is `char16_t` and `U'x'` is `char32_t`, both unsigned
        // integer types; `'x'` is `int`, `u8'x'` is `unsigned char` and promotes to `int`, and
        // `L'x'` is `wchar_t` — signed under the System V ABI this engine models, though unsigned
        // on some targets. All of them spell the same value, so only the signedness distinguishes
        // them, and only in the arithmetic that follows.
        let unsigned = matches!(prefix, EncPrefix::Utf16 | EncPrefix::Utf32);
        return IfValue {
            bits: parse_char_constant(&token.text) as u64,
            unsigned,
        };
    }
    let explicit_unsigned = token.text.bytes().any(|byte| matches!(byte, b'u' | b'U'));
    let digits = token.text.trim_end_matches(['u', 'U', 'l', 'L']);
    let (radix, digits) = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        (16, hex)
    } else if let Some(binary) = digits
        .strip_prefix("0b")
        .or_else(|| digits.strip_prefix("0B"))
    {
        (2, binary)
    } else if digits.len() > 1 && digits.starts_with('0') {
        (8, &digits[1..])
    } else {
        (10, digits)
    };
    let bits = u64::from_str_radix(digits, radix).unwrap_or(0);
    IfValue {
        bits,
        unsigned: explicit_unsigned || bits > i64::MAX as u64,
    }
}

fn parse_char_constant(text: &str) -> i64 {
    // A terminated constant has its body between the first quote and the last. An unterminated
    // one has only the opening quote, so those are the *same* index and the range would run
    // backwards; its body is everything after the quote instead. The preprocessor sees malformed
    // files as a matter of course and must not fault on them (023 §7).
    let inside = match text.find('\'') {
        Some(start) => match text.rfind('\'') {
            Some(end) if end > start => &text[start + 1..end],
            _ => &text[start + 1..],
        },
        None => "",
    };
    let bytes = inside.as_bytes();
    let mut index = 0;
    let mut value = 0_u64;
    while index < bytes.len() {
        let unit = if bytes[index] != b'\\' {
            let unit = u64::from(bytes[index]);
            index += 1;
            unit
        } else {
            index += 1;
            if index >= bytes.len() {
                break;
            }
            match bytes[index] {
                b'x' => {
                    index += 1;
                    let start = index;
                    let mut unit = 0_u64;
                    while index < bytes.len() && bytes[index].is_ascii_hexdigit() {
                        unit = unit
                            .wrapping_mul(16)
                            .wrapping_add(u64::from(hex_value(bytes[index])));
                        index += 1;
                    }
                    if index == start {
                        u64::from(b'x')
                    } else {
                        unit
                    }
                }
                digit @ b'0'..=b'7' => {
                    let mut unit = u64::from(digit - b'0');
                    index += 1;
                    for _ in 1..3 {
                        if index >= bytes.len() || !(b'0'..=b'7').contains(&bytes[index]) {
                            break;
                        }
                        unit = unit * 8 + u64::from(bytes[index] - b'0');
                        index += 1;
                    }
                    unit
                }
                escape => {
                    index += 1;
                    u64::from(match escape {
                        b'a' => 0x07,
                        b'b' => 0x08,
                        b'f' => 0x0c,
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        b'v' => 0x0b,
                        other => other,
                    })
                }
            }
        };
        value = value.wrapping_shl(8) | (unit & 0xff);
    }
    value as i64
}

fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => 0,
    }
}
