//! C preprocessing (translation phase 4) with macro provenance.

use chiero_lex::{EncPrefix, LexConfig, LexSession, PpToken, PpTokenKind, Punct, Symbol};
use chiero_span::{ExpnCtx, ExpnKind, FileId, MacroId, SourceMap, Span};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub id: ConfigId,
    pub pedantic: bool,
    pub date: String,
    pub time: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            id: ConfigId(1),
            pedantic: false,
            date: "Jan 01 1970".into(),
            time: "00:00:00".into(),
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

struct Engine {
    config: Config,
    source_map: SourceMap,
    file: FileId,
    input: Vec<Tok>,
    macros: Vec<StoredMacro>,
    by_name: BTreeMap<String, usize>,
    diagnostics: Vec<Diagnostic>,
    counter: u64,
}

impl Engine {
    fn new(path: &Path, src: &str, config: Config) -> Self {
        let mut source_map = SourceMap::new();
        let file = source_map.add_file(path, src);
        let lexed = LexSession::new().lex(&source_map, file, LexConfig::default());
        let mut diagnostics = Vec::new();
        diagnostics.extend(lexed.diagnostics().iter().map(|d| Diagnostic {
            span: d.span,
            message: d.message.clone(),
        }));
        let input = lexed
            .tokens()
            .iter()
            .filter(|token| !matches!(token.kind, PpTokenKind::Eof))
            .map(|token| Tok {
                token: token.clone(),
                text: lexed.text(token).to_owned(),
                hide: BTreeSet::new(),
            })
            .collect();
        let mut engine = Self {
            config,
            source_map,
            file,
            input,
            macros: Vec::new(),
            by_name: BTreeMap::new(),
            diagnostics,
            counter: 0,
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
        engine
    }

    fn run(mut self) -> PreprocessedTu {
        let mut ordinary = Vec::new();
        let mut i = 0;
        while i < self.input.len() {
            let end = (i + 1..self.input.len())
                .find(|&j| self.input[j].token.bol)
                .unwrap_or(self.input.len());
            let line = self.input[i..end].to_vec();
            if line.first().is_some_and(|t| {
                t.token.bol && matches!(t.token.kind, PpTokenKind::Punct(Punct::Hash))
            }) {
                self.directive(&line);
            } else {
                ordinary.extend(line);
            }
            i = end;
        }
        let expanded = self.expand(ordinary);
        let tokens = expanded.iter().map(|t| t.token.clone()).collect();
        let spellings = expanded.into_iter().map(|t| t.text).collect();
        PreprocessedTu {
            tokens,
            source_map: self.source_map,
            diagnostics: self.diagnostics,
            config: self.config.id,
            deps: vec![self.file],
            spellings,
        }
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

    fn directive(&mut self, line: &[Tok]) {
        match line.get(1).map(|t| t.text.as_str()) {
            Some("define") => self.define(line),
            Some("undef") => {
                if let Some(name) = line.get(2)
                    && let Some(index) = self.by_name.remove(&name.text)
                {
                    self.macros[index].def.undef_span = Some(name.token.span);
                }
            }
            Some(other) => self.diagnostics.push(Diagnostic {
                span: line[1].token.span,
                message: format!("unsupported preprocessing directive #{other}"),
            }),
            None => {}
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
        let body = line.get(body_start..).unwrap_or_default().to_vec();
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
        let mut output = Vec::new();
        let mut i = 0;
        while i < input.len() {
            let token = &input[i];
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
                    let replacement = def
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
                    output.extend(self.expand(replacement));
                    i += 1;
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
                    let replacement = self.expand_function(token, &input[close], &def, args);
                    output.extend(self.expand(replacement));
                    i = close + 1;
                }
            }
        }
        output
    }

    fn expand_builtin(&mut self, call: &Tok, def: &StoredMacro) -> Tok {
        let text = match def.name.as_str() {
            "__COUNTER__" => {
                let value = self.counter.to_string();
                self.counter += 1;
                value
            }
            "__LINE__" => self
                .source_map
                .expansion_loc(call.token.span)
                .map_or_else(|| "0".into(), |loc| loc.line.to_string()),
            "__FILE__" => self.source_map.lookup_file(call.token.span.lo).map_or_else(
                || "\"\"".into(),
                |file| format!("\"{}\"", self.source_map.file(file).path().display()),
            ),
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
            Span::new(call.token.span.lo, close.token.span.hi, call.token.span.ctx),
            arg_spans,
            ExpnKind::FunctionLike,
        );

        let mut expanded_by_name = BTreeMap::new();
        for (name, raw) in &raw_by_name {
            expanded_by_name.insert(name.clone(), self.expand(raw.clone()));
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
            Span::DUMMY,
            Span::DUMMY,
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
                if left.text.is_empty() {
                    output.push(right);
                    i += 2;
                    continue;
                }
                let text = format!("{}{}", left.text, right.text);
                let expn = self.source_map.add_expansion(
                    parent,
                    None,
                    Span::DUMMY,
                    Span::DUMMY,
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
    Some(Span::new(
        tokens.first()?.token.span.lo,
        tokens.last()?.token.span.hi,
        tokens.first()?.token.span.ctx,
    ))
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
