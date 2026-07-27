//! C preprocessing (translation phase 4) with macro provenance.

use chiero_lex::{EncPrefix, LexConfig, LexSession, PpToken, PpTokenKind, Punct, Symbol};
use chiero_span::{ExpnCtx, ExpnKind, FileId, MacroId, SourceMap, Span};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;
use std::path::PathBuf;

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
    spellings: Vec<String>,
}

impl PreprocessedTu {
    pub fn token_texts(&self) -> impl Iterator<Item = &str> {
        self.spellings.iter().map(String::as_str)
    }

    pub fn text(&self, token: &PpToken) -> Option<&str> {
        self.tokens
            .iter()
            .position(|candidate| std::ptr::eq(candidate, token))
            .and_then(|index| self.spellings.get(index))
            .map(String::as_str)
    }
}

#[derive(Clone, Debug)]
struct Tok {
    token: PpToken,
    text: String,
    hide: BTreeSet<MacroId>,
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
    macros: Vec<StoredMacro>,
    by_name: BTreeMap<String, usize>,
    diagnostics: Vec<Diagnostic>,
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
                hide: BTreeSet::new(),
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
            macros: Vec::new(),
            by_name: BTreeMap::new(),
            diagnostics,
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
        let tokens = output.iter().map(|t| t.token.clone()).collect();
        let spellings = output.into_iter().map(|t| t.text).collect();
        PreprocessedTu {
            tokens,
            source_map: std::mem::take(&mut self.source_map),
            diagnostics: std::mem::take(&mut self.diagnostics),
            config: self.config.id,
            deps: std::mem::take(&mut self.deps),
            spellings,
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
                    self.once.insert(path.to_path_buf());
                } else {
                    self.directive(&line, &mut conditionals);
                }
            } else if active {
                ordinary.extend(line);
            }
            i = end;
        }
        output.extend(self.expand(ordinary));
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
        let expanded = self.expand(line.get(2..).unwrap_or_default().to_vec());
        let Some((name, quoted)) = parse_header_name(&expanded) else {
            self.diagnostics.push(Diagnostic {
                span: line.get(1).map_or(Span::DUMMY, |token| token.token.span),
                message: "invalid computed include".into(),
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
            match loader.load(&resolved) {
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
                hide: BTreeSet::new(),
            })
            .collect()
    }

    fn add_builtin(&mut self, name: &str) {
        let id = self
            .source_map
            .add_macro_at(name, Span::DUMMY, Span::DUMMY, None, 0);
        let index = self.macros.len();
        self.macros.push(StoredMacro {
            def: MacroDef {
                id,
                name: Symbol(index as u32),
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
        let id = self
            .source_map
            .add_macro_at(name, Span::DUMMY, Span::DUMMY, None, 0);
        let index = self.macros.len();
        let body = vec![synthetic_number(value, Span::DUMMY)];
        self.macros.push(StoredMacro {
            def: MacroDef {
                id,
                name: Symbol(index as u32),
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
        let id = self
            .source_map
            .add_macro_at(name, Span::DUMMY, Span::DUMMY, None, 0);
        let index = self.macros.len();
        let body = vec![synthetic_number("0", Span::DUMMY)];
        self.macros.push(StoredMacro {
            def: MacroDef {
                id,
                name: Symbol(index as u32),
                kind: MacroKind::FunctionLike {
                    params: vec![Symbol(0)],
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
                hide: BTreeSet::new(),
            })
            .collect();
        let id = self
            .source_map
            .add_macro_at(name, Span::DUMMY, Span::DUMMY, None, 0);
        let index = self.macros.len();
        self.macros.push(StoredMacro {
            def: MacroDef {
                id,
                name: Symbol(index as u32),
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

    fn directive(&mut self, line: &[Tok], conditionals: &mut Vec<Conditional>) {
        let directive = line.get(1).map(|t| t.text.as_str());
        let active = conditionals.last().is_none_or(|frame| frame.active);
        match directive {
            Some("if") => {
                let parent_active = active;
                let value = parent_active && self.eval_if(&line[2..]);
                conditionals.push(Conditional {
                    parent_active,
                    active: value,
                    taken: value,
                });
            }
            Some("ifdef" | "ifndef") => {
                let parent_active = active;
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
                });
            }
            Some("elif") => {
                let should_eval = conditionals
                    .last()
                    .is_some_and(|frame| frame.parent_active && !frame.taken);
                let value = should_eval && self.eval_if(&line[2..]);
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
                if let Some(frame) = conditionals.last_mut() {
                    frame.active = frame.parent_active && !frame.taken;
                    frame.taken = true;
                }
            }
            Some("endif") => {
                conditionals.pop();
            }
            _ if !active => {}
            Some("define") => self.define(line),
            Some("undef") => {
                if let Some(name) = line.get(2)
                    && let Some(index) = self.by_name.remove(&name.text)
                {
                    self.macros[index].def.undef_span = Some(name.token.span);
                }
            }
            Some("line") => {
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
            Some("pragma") => {}
            Some(other) => self.diagnostics.push(Diagnostic {
                span: line[1].token.span,
                message: format!("unsupported preprocessing directive #{other}"),
            }),
            None => {}
        }
    }

    fn eval_if(&mut self, tokens: &[Tok]) -> bool {
        let mut prepared = Vec::new();
        let mut i = 0;
        while i < tokens.len() {
            if tokens[i].text == "defined" {
                let parenthesized = tokens.get(i + 1).is_some_and(|t| t.text == "(");
                let name_index = i + if parenthesized { 2 } else { 1 };
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
        let (value, parsed) = {
            let mut parser = ExprParser {
                tokens: &expanded,
                pos: 0,
                diagnostics: &mut self.diagnostics,
                pedantic: self.config.pedantic,
            };
            let value = parser.expression(true);
            (value, parser.pos)
        };
        if parsed != expanded.len() {
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
        let kind = if function_like {
            MacroKind::FunctionLike {
                params: params
                    .iter()
                    .enumerate()
                    .map(|(i, _)| Symbol(i as u32))
                    .collect(),
                variadic: if variadic_name.is_some() {
                    Variadic::Named(Symbol(params.len() as u32))
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
                name: Symbol(index as u32),
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
        let mut output = Vec::new();
        let mut i = 0;
        while i < input.len() {
            let token = &input[i];
            if token.text == "_Pragma"
                && input.get(i + 1).is_some_and(|token| token.text == "(")
                && input
                    .get(i + 2)
                    .is_some_and(|token| matches!(token.token.kind, PpTokenKind::StringLit { .. }))
                && input.get(i + 3).is_some_and(|token| token.text == ")")
            {
                self.source_map.add_expansion(
                    token.token.span.ctx,
                    None,
                    token.token.span,
                    span_from_ends(token.token.span, input[i + 3].token.span),
                    Vec::new(),
                    ExpnKind::Pragma,
                );
                output.extend(self.expand(input[i + 4..].to_vec()));
                return output;
            }
            let Some(&macro_index) = self.by_name.get(&token.text) else {
                output.push(token.clone());
                i += 1;
                continue;
            };
            let def = self.macros[macro_index].clone();
            if token.hide.contains(&def.def.id) {
                output.push(token.clone());
                i += 1;
                continue;
            }
            if is_builtin(&def.name) {
                output.push(self.expand_builtin(token, &def));
                i += 1;
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
                            body.hide.extend(token.hide.iter().copied());
                            body.hide.insert(def.def.id);
                            body
                        })
                        .collect();
                    let mut replacement = self.paste(replacement, expn);
                    // C11 §6.10.3.4 ¶1: rescan the replacement list *together with all
                    // subsequent source tokens*. This is what lets `#define A B` turn
                    // `A(1)` into an invocation of function-like `B`.
                    replacement.extend_from_slice(&input[i + 1..]);
                    output.extend(self.expand(replacement));
                    return output;
                }
                MacroKind::FunctionLike { .. } => {
                    if !input
                        .get(i + 1)
                        .is_some_and(|t| matches!(t.token.kind, PpTokenKind::Punct(Punct::LParen)))
                    {
                        output.push(token.clone());
                        i += 1;
                        continue;
                    }
                    let Some((args, close)) = parse_args(&input, i + 1) else {
                        output.push(token.clone());
                        i += 1;
                        continue;
                    };
                    let mut replacement = self.expand_function(token, &input[close], &def, args);
                    replacement.extend_from_slice(&input[close + 1..]);
                    output.extend(self.expand(replacement));
                    return output;
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
                replacement.push(self.stringize(&def.body[i], raw, expn, def.def.id));
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
                        hide: BTreeSet::new(),
                    });
                }
                for mut arg in selected {
                    if arg.token.span.ctx.is_root() {
                        arg.token.span.ctx = expn;
                    }
                    arg.hide.extend(call.hide.iter().copied());
                    arg.hide.insert(def.def.id);
                    replacement.push(arg);
                }
            } else {
                let mut copied = body.clone();
                copied.token.span.ctx = expn;
                copied.hide.extend(call.hide.iter().copied());
                copied.hide.insert(def.def.id);
                replacement.push(copied);
            }
            i += 1;
        }
        self.paste(replacement, expn)
    }

    fn stringize(
        &mut self,
        operator: &Tok,
        raw: &[Tok],
        parent: ExpnCtx,
        macro_id: MacroId,
    ) -> Tok {
        let mut inside = String::new();
        for (index, token) in raw.iter().enumerate() {
            if index != 0 && token.token.leading_space {
                inside.push(' ');
            }
            for ch in token.text.chars() {
                if matches!(ch, '\\' | '"') {
                    inside.push('\\');
                }
                inside.push(ch);
            }
        }
        let expn = self.source_map.add_expansion(
            parent,
            Some(macro_id),
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
            hide: BTreeSet::new(),
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
                        kind: classify_paste(&text),
                        span: Span::new(input[i].token.span.lo, input[i].token.span.lo, expn),
                        leading_space: left.token.leading_space,
                        bol: left.token.bol,
                    },
                    text,
                    hide: left.hide.union(&right.hide).copied().collect(),
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

fn parse_args(input: &[Tok], open: usize) -> Option<(Vec<Vec<Tok>>, usize)> {
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

fn synthetic_punct(text: &str, punct: Punct, at: Span) -> Tok {
    Tok {
        token: PpToken {
            kind: PpTokenKind::Punct(punct),
            span: Span::new(at.lo, at.lo, at.ctx),
            leading_space: false,
            bol: false,
        },
        text: text.into(),
        hide: BTreeSet::new(),
    }
}

fn classify_paste(text: &str) -> PpTokenKind {
    if text.as_bytes().first().is_some_and(|b| b.is_ascii_digit()) {
        PpTokenKind::Number
    } else {
        PpTokenKind::Ident(Symbol(0))
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
        hide: BTreeSet::new(),
    }
}

struct ExprParser<'a> {
    tokens: &'a [Tok],
    pos: usize,
    diagnostics: &'a mut Vec<Diagnostic>,
    pedantic: bool,
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
            let value = self.expression(live);
            self.take(")");
            return value;
        }
        let Some(token) = self.tokens.get(self.pos) else {
            return IfValue::ZERO;
        };
        self.pos += 1;
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

fn parse_if_literal(token: &Tok) -> IfValue {
    if matches!(token.token.kind, PpTokenKind::CharLit { .. }) {
        return IfValue::signed(parse_char_constant(&token.text));
    }
    let unsigned = token.text.bytes().any(|byte| matches!(byte, b'u' | b'U'));
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
    IfValue {
        bits: u64::from_str_radix(digits, radix).unwrap_or(0),
        unsigned,
    }
}

fn parse_char_constant(text: &str) -> i64 {
    let inside = text
        .find('\'')
        .and_then(|start| text.rfind('\'').map(|end| &text[start + 1..end]))
        .unwrap_or("");
    let mut chars = inside.chars();
    match chars.next() {
        Some('\\') => match chars.next() {
            Some('n') => i64::from(b'\n'),
            Some('r') => i64::from(b'\r'),
            Some('t') => i64::from(b'\t'),
            Some('0') => 0,
            Some('\\') => i64::from(b'\\'),
            Some('\'') => i64::from(b'\''),
            Some(other) => other as i64,
            None => 0,
        },
        Some(ch) => ch as i64,
        None => 0,
    }
}
