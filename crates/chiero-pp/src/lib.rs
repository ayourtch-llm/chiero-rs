//! C preprocessing (translation phase 4) with macro provenance.

pub mod features;
pub mod persona;

pub use persona::Persona;

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
    /// Command-line-style object macros, applied after the persona.
    pub defines: Vec<(String, String)>,
    /// **Who chiero is impersonating.** See [`Persona`] — the default is the set chiero has
    /// always baked, now named and replaceable.
    pub persona: Persona,
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
            persona: Persona::baked(),
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

/// One conditional directive, as written (031 contract 16).
///
/// **Recorded because the token stream cannot carry it.** A `#if` line is consumed by the
/// preprocessor, so a condition that changed while its *outcome* did not leaves no trace at all —
/// and that is precisely the change that behaves differently under another configuration. 031
/// §3.5 needs to see it; nothing else here can show it.
///
/// The condition is kept as **token spellings**, not source text, so that respacing it is not a
/// change — the same normalisation every other comparison in the system uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConditionalRecord {
    /// `if`, `ifdef`, `ifndef` or `elif`.
    pub directive: String,
    /// Where the directive begins.
    pub span: Span,
    /// The condition's tokens, spelled.
    pub condition: Vec<String>,
}

#[derive(Debug)]
pub struct PreprocessedTu {
    pub tokens: Vec<PpToken>,
    pub source_map: SourceMap,
    /// Diagnostics a person can act on — those whose site is the translation unit's own source
    /// or one of its project headers.
    pub diagnostics: Vec<Diagnostic>,
    /// **Diagnostics whose site is inside a system header — real, and not the programmer's.**
    ///
    /// Every compiler does this, and it is measured rather than recalled: the *same header text*
    /// warns from a user path and is silent from a system one.
    ///
    /// ```text
    /// $ cp /usr/include/linux/memfd.h uh/memfd_user.h
    /// $ gcc -E -I/usr/include c.c   # includes "uh/memfd_user.h"
    /// uh/memfd_user.h:8: warning: "MFD_CLOEXEC" redefined      ← and three more
    /// $ gcc -E d.c                  # includes <linux/memfd.h>, byte-identical content
    ///                               ← nothing
    /// ```
    ///
    /// 012 contract 17's corpus run found the cost: five of 25 diagnosed VPP units reported
    /// `redefinition of macro MFD_CLOEXEC` and siblings. chiero was *right* — C11 6.10.3p2 makes
    /// a non-identical redefinition a constraint violation, and `<sys/mman.h>` really does define
    /// it as `1U` before `<linux/memfd.h>` says `0x0001U`. Nobody can act on it: both files
    /// belong to glibc and the kernel headers, and every C program on the machine has the clash.
    ///
    /// **Kept rather than dropped**, so "did not report" and "found nothing" stay distinct —
    /// a preprocessor that deleted these would be claiming a clean tree it never checked.
    pub system_diagnostics: Vec<Diagnostic>,
    pub config: ConfigId,
    pub deps: Vec<FileId>,
    pub pragmas: Vec<PragmaRecord>,
    pub macro_defs: Vec<MacroDef>,
    /// Every conditional directive, in source order — see [`ConditionalRecord`].
    pub conditionals: Vec<ConditionalRecord>,
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
    /// **This `##` is the paste operator**, as opposed to an ordinary `##` punctuator.
    ///
    /// C11 6.10.3.3 identifies the operator in a macro's *replacement list*, at definition
    /// time. A `##` that reaches a substituted sequence any other way — spelled at a call site
    /// and passed as an argument, or produced by an earlier paste, as `# ## #` does — is an
    /// ordinary token, and 6.10.3.3p4's worked example exists to say so.
    ///
    /// It is a flag on the token rather than a position computed at the paste site because
    /// **that is the level the rule lives at**: `substitute` interleaves replacement-list
    /// tokens with argument tokens, and by the time `paste` walks the result, where each one
    /// came from is exactly the thing it can no longer see. Marking at definition and letting
    /// the flag ride through substitution is what makes the three routes one fix.
    paste_op: bool,
    /// This token was substituted for the **variadic** parameter (`__VA_ARGS__`, or a GNU
    /// `args...` name), including the empty placemarker that stands in for an absent one.
    ///
    /// GNU comma-swallowing is a rule about `, ## <variadic>` and about nothing else. `, ## Y`
    /// for an ordinary parameter is an ordinary paste against an empty argument and the comma
    /// survives — so `paste` cannot decide by looking at the comma, or at emptiness, or at the
    /// `##`. It has to know **what the right operand is**, and that is knowable only here, where
    /// the parameter still has a name.
    from_variadic: bool,
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

    /// The set of macros hidden by **both**, which is what a function-like invocation is
    /// entitled to carry (012 §2.4, C99 6.10.3.4p2).
    ///
    /// `extend` and this are opposites, and the difference is the whole of contract 21. Words
    /// beyond `other`'s length are dropped rather than kept, because an absent word means every
    /// bit in it is zero and the intersection with zero is zero — the mistake `extend`'s
    /// `resize` would invite if this were written by analogy to it.
    fn intersect(&self, other: &Self) -> Self {
        Self(self.0.iter().zip(&other.0).map(|(a, b)| a & b).collect())
    }
}

#[derive(Clone, Debug)]
struct StoredMacro {
    def: MacroDef,
    /// **Defined, and never expanded.** `__has_attribute` and friends must answer `defined()`
    /// and `#ifdef` like gcc, but they are not macros: gcc evaluates them as operators inside a
    /// `#if` expression and leaves them as ordinary identifiers in program text. Expanding them
    /// would consume the query before `eval_if` could answer it — which is exactly what happened
    /// to `__glibc_has_attribute(attr)`, whose expansion *produces* the query.
    query_only: bool,
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
    conditionals: Vec<ConditionalRecord>,
    counter: u64,
    expansion_depth: usize,
    /// `#pragma push_macro("X")` saves the **binding**, not the definition.
    ///
    /// A `MacroId` names one definition and is never reused (012 §1), so `macros` is
    /// append-only and what push/pop moves is the entry in `by_name`. `None` is a saved
    /// *absence* — pushing an undefined name and popping it must leave it undefined, which a
    /// stack of bare indices could not express.
    macro_stack: BTreeMap<String, Vec<Option<usize>>>,
    /// `#pragma GCC dependency "f"` requests, resolved in `finish`.
    ///
    /// Deferred because `_Pragma` is handled inside `expand_inner`, which has **no
    /// `FileLoader`** — and `DO_PRAGMA ("GCC dependency …")` reaches it only that way, which is
    /// exactly what the corpus fixture tests. `finish` has the loader, so the request is
    /// recorded where it is seen and answered where it can be.
    pending_dependencies: Vec<(String, Span)>,
    /// Feature-query names `features::TABLE` had no answer for, so each is reported once.
    ///
    /// `sys/cdefs.h` alone queries many times per TU, so a diagnostic per *query* would drown
    /// the channel it is reported in. One per distinct name is what makes an unknown name
    /// readable as a to-do rather than as noise.
    unknown_queries: BTreeSet<String>,
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
                paste_op: false,
                from_variadic: false,
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
            unknown_queries: BTreeSet::new(),
            macro_stack: BTreeMap::new(),
            pending_dependencies: Vec::new(),
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
            conditionals: Vec::new(),
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
        // **The persona, not a literal.** This was an array here; it is a named, replaceable
        // value now — see `Persona`. Installed before `Config::defines`, because a `-D` on the
        // command line beats what the compiler predefines.
        for (name, value) in engine
            .config
            .persona
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<Vec<_>>()
        {
            // **Lexed, not wrapped in a synthetic number.** `add_predefined_object` makes the
            // value one numeric token, which is fine for the baked set (all numerals) and wrong
            // for a real `cc -dM` dump: `#define __PTRDIFF_TYPE__ long int` is two tokens, and
            // as a bogus number it made `stddef.h` fail with "expected a type specifier".
            engine.add_config_object(&name, &value);
        }
        for name in [
            "__has_include",
            "__has_attribute",
            "__has_builtin",
            "__has_c_attribute",
            "__has_cpp_attribute",
        ] {
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
        // **The feature queries are answered in program text too, because both compilers answer
        // them there.** Verified rather than assumed: `int y = __has_attribute(packed);` comes
        // out of gcc *and* clang as `int y = 1;`. A first version of this left them alone
        // outside `#if`, on the plausible-sounding rule that they are preprocessor operators —
        // and the pp-gate caught it immediately, with `__has_attribute` sitting in the output
        // token stream where both compilers had put a number.
        let output = self.answer_feature_queries(output);
        self.resolve_dependencies(&root_path, loader);
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
        let source_map = std::mem::take(&mut self.source_map);
        // **Diagnostics from system headers are separated, not deleted** — see
        // [`PreprocessedTu::system_diagnostics`]. `Path::starts_with` compares whole components,
        // so `/usr/include` contains `/usr/include/linux/memfd.h` and does not accidentally
        // contain `/usr/include-mine/x.h`.
        let (system_diagnostics, diagnostics) = std::mem::take(&mut self.diagnostics)
            .into_iter()
            .partition(|diagnostic: &Diagnostic| {
                source_map
                    .lookup_file(diagnostic.span.lo)
                    .map(|id| source_map.file(id).path())
                    .is_some_and(|path| {
                        self.config
                            .system_paths
                            .iter()
                            .any(|root| path.starts_with(root))
                    })
            });
        PreprocessedTu {
            tokens,
            source_map,
            diagnostics,
            system_diagnostics,
            config: self.config.id,
            deps: std::mem::take(&mut self.deps),
            pragmas: std::mem::take(&mut self.pragmas),
            conditionals: std::mem::take(&mut self.conditionals),
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
                // physical line, so an ordinary-token chunk is normally completed here.
                //
                // **Unless a macro call is still open across it.** gcc keeps collecting a
                // function-like macro's arguments over a directive — undefined by 6.10.3p11 and
                // relied on throughout VPP, where `CLIB_PACKED (struct { #define … })` appears in
                // 49 headers and transitively blocked most of `vnet` and `plugins`. Flushing here
                // left the call unterminated: the macro went out unexpanded, which wave 406 made
                // *say so* and this makes work.
                //
                // The test is an unbalanced `(` in the pending chunk, which is what "still inside
                // an argument list" means at this level — the expander itself decides what is a
                // call. Directives are still processed in order, so `#define K 5` inside the
                // arguments takes effect exactly as it does for gcc; only the *flush* is deferred.
                //
                // A chunk that never closes is not deferred forever: `finish` expands whatever
                // remains, and the unterminated-argument-list diagnostic still fires there.
                if !self.in_open_macro_args(&ordinary) {
                    output.extend(self.expand(std::mem::take(&mut ordinary)));
                }
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
                paste_op: false,
                from_variadic: false,
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
        // No body to mark: a builtin's replacement is computed at each use, not stored.
        self.macros.push(StoredMacro {
            query_only: false,
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

    fn add_predefined_query(&mut self, name: &str) {
        let name_symbol = self.lex_session.intern_symbol(name);
        let parameter_symbol = self.lex_session.intern_symbol("query");
        let id = self
            .source_map
            .add_macro_at(name, Span::DUMMY, Span::DUMMY, None, 0);
        let index = self.macros.len();
        let mut body = vec![synthetic_number("0", Span::DUMMY)];
        mark_paste_operators(&mut body);
        self.macros.push(StoredMacro {
            // Defined so `#ifdef __has_attribute` is true as it is under gcc, and never
            // expanded so `eval_if` still sees the query — including the one that arrives by
            // expanding `__glibc_has_attribute(attr)`.
            query_only: true,
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
        let mut body: Vec<_> = lexed
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
                paste_op: false,
                from_variadic: false,
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
        mark_paste_operators(&mut body);
        self.macros.push(StoredMacro {
            query_only: false,
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

    /// Keep a conditional directive's condition, which the token stream cannot (031 contract 16).
    ///
    /// **Recorded whether or not the branch is live.** An inactive `#if` inside a skipped region
    /// is not *evaluated* — 012's rule — but its text is still what would be evaluated under a
    /// different configuration, which is exactly the question 031 §3.5 asks.
    fn record_conditional(&mut self, line: &[Tok], from: usize) {
        let Some(head) = line.first() else { return };
        self.conditionals.push(ConditionalRecord {
            directive: line.get(1).map(|t| t.text.clone()).unwrap_or_default(),
            span: head.token.span,
            condition: line.iter().skip(from).map(|t| t.text.clone()).collect(),
        });
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
                self.record_conditional(line, 2);
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
                self.record_conditional(line, 2);
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
                self.record_conditional(line, 2);
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
            Some("pragma") => {
                let rest = line.get(2..).unwrap_or_default();
                self.apply_macro_stack_pragma(rest);
                if let Some(first) = rest.first() {
                    let text = rest
                        .iter()
                        .map(|token| token.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    self.report_diagnostic_pragma(&text, first.token.span);
                }
                self.record_pragma_tokens(rest);
            }
            Some(other) => self.diagnostics.push(Diagnostic {
                span: line[1].token.span,
                message: format!("unsupported preprocessing directive #{other}"),
            }),
            None => {}
        }
    }

    /// `push_macro("X")` / `pop_macro("X")`, which gcc and clang both implement.
    ///
    /// The pragma is still recorded afterwards: a consumer that wants to see every pragma should,
    /// and acting on one is not a reason to hide it.
    ///
    /// An **imbalanced pop is a no-op rather than an error** — the corpus fixture ends with a
    /// stray push precisely to check that neither direction crashes, and gcc accepts both.
    fn apply_macro_stack_pragma(&mut self, tokens: &[Tok]) {
        let Some(op) = tokens.first() else { return };
        let push = match op.text.as_str() {
            "push_macro" => true,
            "pop_macro" => false,
            _ => return,
        };
        // `push_macro ( "X" )` — the operand is a string literal, so the quotes come off.
        let Some(name) = tokens
            .get(1)
            .filter(|t| t.text == "(")
            .and_then(|_| tokens.get(2))
            .filter(|_| tokens.get(3).is_some_and(|t| t.text == ")"))
            .and_then(|t| t.text.strip_prefix('"'))
            .and_then(|t| t.strip_suffix('"'))
            .map(str::to_owned)
        else {
            return;
        };
        if push {
            let saved = self.by_name.get(&name).copied();
            self.macro_stack.entry(name).or_default().push(saved);
        } else if let Some(saved) = self.macro_stack.get_mut(&name).and_then(Vec::pop) {
            match saved {
                Some(index) => {
                    self.by_name.insert(name, index);
                }
                None => {
                    self.by_name.remove(&name);
                }
            }
        }
    }

    /// `GCC error "…"` / `GCC warning "…"` — the pragmas that *are* diagnostics.
    ///
    /// Text-based rather than token-based so the **one** implementation serves both routes: the
    /// `#pragma` directive joins its tokens with spaces, and `_Pragma` destringizes to the same
    /// shape. `diagnostic-pragma-1.c` uses the operator inside a macro, so the message has to
    /// fire where the macro is *used*, and a second implementation would have drifted from the
    /// first the way §11.3's duplicated predicate did.
    ///
    /// ⚠️ **chiero's `Diagnostic` carries no severity**, so `warning` and `error` are equally
    /// loud here. That is a real limit; reporting neither would be worse, and grading them needs
    /// a severity channel this type does not have.
    /// Answer the `#pragma GCC dependency` requests recorded during expansion.
    ///
    /// gcc searches the include path and **errors when the file is not found**; that is the whole
    /// observable behaviour the corpus fixture asks for. The freshness comparison gcc also does
    /// is not modelled — a stale dependency is a warning about build order, not about the
    /// program, and chiero has no build clock.
    fn resolve_dependencies(&mut self, current: &Path, loader: &mut dyn FileLoader) {
        for (name, span) in std::mem::take(&mut self.pending_dependencies) {
            let mut roots: Vec<PathBuf> = Vec::new();
            if let Some(dir) = current.parent() {
                roots.push(dir.to_path_buf());
            }
            roots.extend(self.config.iquote_paths.iter().cloned());
            roots.extend(self.config.include_paths.iter().cloned());
            roots.extend(self.config.system_paths.iter().cloned());
            let found = roots
                .iter()
                .any(|root| loader.load(&root.join(&name)).is_ok());
            if !found {
                self.diagnostics.push(Diagnostic {
                    span,
                    message: format!("`{name}` file not found"),
                });
            }
        }
    }

    fn report_diagnostic_pragma(&mut self, text: &str, span: Span) {
        let Some(rest) = text.strip_prefix("GCC ") else {
            return;
        };
        let rest = rest.trim_start();
        if let Some(operand) = rest.strip_prefix("dependency") {
            if let Some(name) = operand
                .trim()
                .strip_prefix('"')
                .and_then(|n| n.strip_suffix('"'))
            {
                self.pending_dependencies.push((name.to_owned(), span));
            }
            return;
        }
        let severity = ["error", "warning"]
            .into_iter()
            .find(|kind| rest.starts_with(kind));
        let Some(severity) = severity else { return };
        let message = rest[severity.len()..].trim();
        // The operand is one string literal; anything else is not this pragma.
        let Some(message) = message.strip_prefix('"').and_then(|m| m.strip_suffix('"')) else {
            return;
        };
        self.diagnostics.push(Diagnostic {
            span,
            message: format!("#pragma GCC {severity}: {message}"),
        });
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
        // **The feature queries are answered after expansion, because that is where they
        // arrive.** `__glibc_has_attribute(attr)` *expands to* `__has_attribute (attr)`, so a
        // pre-expansion rewrite sees `__glibc_has_attribute` and nothing else — which is how the
        // first version of this answered `NOT` to the very idiom `sys/cdefs.h` is built around.
        // gcc evaluates them at the same point for the same reason. `defined` is the opposite
        // case and must stay before expansion, as C requires.
        let expanded = self.expand(prepared);
        let expanded = self.answer_feature_queries(expanded);
        // **`__has_include` arrives the same way and needs the same pass.** A wrapper like
        // `#define HI(x) __has_include(x)` puts the query into the stream only *after*
        // expansion, so the pre-expansion arm above never sees it — and once these names stopped
        // being expandable, what used to expand to nothing and read as `0` became an identifier
        // the expression parser choked on. Answered here, with the loader the pre-expansion arm
        // already uses.
        let expanded = self.answer_has_include(expanded, current, loader);
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
        mark_paste_operators(&mut body);
        self.macros.push(StoredMacro {
            query_only: false,
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

    /// Replace a post-expansion `__has_include(...)` with `1` or `0`.
    ///
    /// Separate from [`Self::answer_feature_queries`] because it needs the file loader, and
    /// because its operand is a header name — a token *sequence* (`<a/b.h>` lexes as several
    /// tokens) rather than one identifier.
    fn answer_has_include(
        &mut self,
        tokens: Vec<Tok>,
        current: &Path,
        loader: &mut dyn FileLoader,
    ) -> Vec<Tok> {
        if !tokens.iter().any(|token| token.text == "__has_include") {
            return tokens;
        }
        let mut out = Vec::with_capacity(tokens.len());
        let mut i = 0;
        while i < tokens.len() {
            if tokens[i].text == "__has_include"
                && tokens.get(i + 1).is_some_and(|token| token.text == "(")
                && let Some(close) = tokens[i + 2..]
                    .iter()
                    .position(|token| token.text == ")")
                    .map(|offset| i + 2 + offset)
            {
                let operand: Vec<Tok> = tokens[i + 2..close].to_vec();
                let value = self.probe_include(&operand, current, loader);
                out.push(synthetic_number(
                    if value { "1" } else { "0" },
                    tokens[i].token.span,
                ));
                i = close + 1;
            } else {
                out.push(tokens[i].clone());
                i += 1;
            }
        }
        out
    }

    /// Replace `__has_attribute(NAME)` / `__has_builtin(NAME)` with `1` or `0`.
    ///
    /// Answers come from [`features::TABLE`] — gcc 13's, because chiero's predefine set is an
    /// impersonation of the build compiler rather than a self-report. A name the table does not
    /// cover answers `0`, since `#if` must yield a number, and says so once per distinct name.
    fn answer_feature_queries(&mut self, tokens: Vec<Tok>) -> Vec<Tok> {
        if !tokens.iter().any(|token| is_feature_query(&token.text)) {
            return tokens;
        }
        let mut out = Vec::with_capacity(tokens.len());
        let mut i = 0;
        while i < tokens.len() {
            // **The operand is `NAME` or `SCOPE :: NAME`, and C has no `::` punctuator.** A
            // scoped operand therefore arrives as four tokens — `gnu`, `:`, `:`, `noreturn` — so
            // a matcher written for one identifier between the parens saw nothing at all for it.
            let operand = tokens
                .get(i + 1)
                .filter(|token| token.text == "(")
                .and_then(|_| match (tokens.get(i + 2), tokens.get(i + 3)) {
                    (Some(name), Some(close)) if close.text == ")" => {
                        Some((None, name.text.clone(), 4))
                    }
                    (Some(scope), Some(colon))
                        if colon.text == ":"
                            && tokens.get(i + 4).is_some_and(|t| t.text == ":")
                            && tokens.get(i + 6).is_some_and(|t| t.text == ")") =>
                    {
                        tokens
                            .get(i + 5)
                            .map(|name| (Some(scope.text.clone()), name.text.clone(), 7))
                    }
                    _ => None,
                });
            if is_feature_query(&tokens[i].text)
                && let Some((scope, operand_name, width)) = operand
            {
                let query = tokens[i].text.clone();
                // A scoped operand answers by rule rather than by row (`features::answer_scoped`).
                let looked_up = match &scope {
                    Some(scope) => features::answer_scoped(scope, &operand_name),
                    None => features::answer(&query, &operand_name),
                };
                let spelled = match &scope {
                    Some(scope) => format!("{scope}::{operand_name}"),
                    None => operand_name.clone(),
                };
                let value = match looked_up {
                    Some(value) => value,
                    None => {
                        if self.unknown_queries.insert(spelled.clone()) {
                            self.diagnostics.push(Diagnostic {
                                span: tokens[i].token.span,
                                message: format!(
                                    "`{query}({spelled})` is not in chiero's compiler-persona \
                                     table; answered 0, which may not be what the build compiler \
                                     says"
                                ),
                            });
                        }
                        0
                    }
                };
                out.push(synthetic_number(&value.to_string(), tokens[i].token.span));
                i += width;
            } else {
                out.push(tokens[i].clone());
                i += 1;
            }
        }
        out
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
                    let text = destringize_pragma(&expanded[0].text);
                    self.report_diagnostic_pragma(&text, token.token.span);
                    self.pragmas.push(PragmaRecord {
                        span: token.token.span,
                        text,
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
            if self.macros[macro_index].query_only {
                output.push(token);
                continue;
            }
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
                        // **An argument list that never closes is reported, not abandoned.** The
                        // `(` was seen and no matching `)` follows, so this is gcc's
                        // "unterminated argument list invoking macro" — and the macro name was
                        // being pushed through unexpanded with nothing said, which is a wrong
                        // token stream rather than a missing diagnostic.
                        //
                        // **A directive inside the arguments arrives here too**, and is why this
                        // matters: `parse_args` scans the current line group and a directive ends
                        // it, so the `)` is out of reach. Multi-line calls are unaffected — their
                        // tokens are in one group — which is the line the six legal rows hold.
                        //
                        // gcc under `-std=gnu11` would expand it, processing the directive as an
                        // extension; under `-pedantic-errors`, where this project calibrates, it
                        // refuses. Supporting the extension is a scope decision for an owner;
                        // reporting rather than silently mis-expanding is not.
                        let name = token.text.clone();
                        self.diagnostics.push(Diagnostic {
                            span: token.token.span,
                            message: format!("unterminated argument list invoking macro `{name}`"),
                        });
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
            paste_op: false,
            from_variadic: false,
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
        // **C99 6.10.3.4p2, Prosser's rule: the invocation's hide set intersects at the closing
        // paren** (012 §2.4). An object-like macro has no paren and carries `HS(name)` alone;
        // this one is `HS(name) ∩ HS(close)`, and the `∪ {M}` half is the `insert(def.def.id)`
        // beside each use below.
        //
        // The intersection is not a refinement of the union it replaced, it is the opposite. A
        // name that came out of an earlier expansion but takes its argument list from the source
        // that followed is only *partly* inside that expansion — so the outer macro's paint drops
        // off, and tokens the union left inert go on expanding. Taking the union stalled
        // `f(2)(9)` at `2*f(9)` where both compilers reach `2*9*g`.
        let invocation_hide = call.hide.intersect(&close.hide);
        if args.len() == 1 && args[0].is_empty() && def.params.is_empty() {
            args.clear();
        }
        let fixed = def.params.len();
        // **Absent is not empty.** `debug(V)` supplies no variadic argument and the GNU comma is
        // swallowed; `debug(Y, )` supplies one that happens to be empty and the comma **stays**.
        // gcc and clang agree on both, and the emptiness of the *tokens* is not what decides it —
        // the presence of the *argument* is, which is knowable only here, from the arity.
        let variadic_supplied = args.len() > fixed;
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
        // **`__VA_OPT__(c)` is resolved here, against this call's arguments** (C23 6.10.3.1).
        // It yields `c` when the variadic argument is present and **non-empty**, and nothing
        // otherwise — `P(1,)` supplies an empty one and still yields nothing, so the test is the
        // argument's *tokens*, not its presence, which is the opposite of the GNU comma rule's
        // test two functions above. Resolving it into an effective body before substitution
        // means the group's contents go through the ordinary parameter walk with no second path.
        let variadic_has_tokens = raw_by_name
            .get(def.variadic_name.as_deref().unwrap_or("__VA_ARGS__"))
            .is_some_and(|tokens| !tokens.is_empty());
        let body = expand_va_opt(&def.body, variadic_has_tokens, &mut self.diagnostics);
        let def = &StoredMacro {
            body,
            ..def.clone()
        };
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
                // **The placemarker remembers which parameter it stands for.** An empty
                // ordinary argument and an empty variadic one look identical by the time
                // `paste` sees them, and the GNU comma rule distinguishes exactly those two.
                let is_variadic_param = def.variadic_name.as_deref() == Some(body.text.as_str())
                    || (def.std_variadic && body.text == "__VA_ARGS__");
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
                        paste_op: false,
                        // ⚠️ **Only the placemarker carries the absent-vs-supplied distinction.**
                        // The substituted *tokens* stay marked (below) so the non-empty GNU form
                        // — `debug(W, 1, 2)` — still takes the comma branch and is not reported
                        // as an invalid paste. Putting the condition on `is_variadic_param`
                        // instead broke exactly those rows.
                        from_variadic: is_variadic_param && !variadic_supplied,
                    });
                }
                for (index, mut arg) in selected.into_iter().enumerate() {
                    arg.from_variadic = is_variadic_param;
                    if arg.token.span.ctx.is_root() {
                        arg.token.span.ctx = expn;
                    }
                    if index == 0 {
                        arg.token.leading_space = body.token.leading_space;
                    }
                    arg.hide.extend(&invocation_hide);
                    arg.hide.insert(def.def.id);
                    replacement.push(arg);
                }
            } else {
                let mut copied = body.clone();
                copied.token.span.ctx = expn;
                copied.hide.extend(&invocation_hide);
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
        // C 6.10.3.2p2 leaves it undefined if the result is not a valid string literal, and a
        // final backslash makes it one: it escapes the closing quote instead of standing for
        // itself. Drop it, as gcc does. The count must be odd — the loop above already doubled
        // any backslash inside a literal, and `S(\\)` is two tokens whose second is escaped by
        // the first, so an even run is well formed and stays.
        if (inside.len() - inside.trim_end_matches('\\').len()) % 2 == 1 {
            inside.pop();
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
            paste_op: false,
            from_variadic: false,
        }
    }

    fn paste(&mut self, input: Vec<Tok>, parent: ExpnCtx) -> Vec<Tok> {
        let mut output: Vec<Tok> = Vec::new();
        let mut i = 0;
        while i < input.len() {
            // **`paste_op`, not the token kind.** By the time this walks the substituted
            // sequence, a `##` here may have come from the replacement list (the operator), from
            // an argument the caller spelled, or from a previous paste — and the kind is
            // identical in all three. C11 6.10.3.3 makes only the first one an operator.
            if input[i].paste_op {
                // **A comma the GNU group has already claimed is not available to paste into.**
                // In `x ## , ## __VA_ARGS__` with an empty tail, the `, ## …` group takes the
                // comma, so there is no `x ## ,` to attempt and no invalid paste to report —
                // both compilers are silent. Left to the ordinary sweep, the earlier `##`
                // arrives first and reports a paste nobody asked for.
                //
                // Only when the tail is **empty**: with `X5(1,2)` gcc really does call
                // `1 ## ,` an error, so the claim is exactly as narrow as the swallow it
                // protects.
                let claimed_by_gnu_group = input
                    .get(i + 1)
                    .is_some_and(|t| matches!(t.token.kind, PpTokenKind::Punct(Punct::Comma)))
                    && input.get(i + 2).is_some_and(|t| t.paste_op)
                    && input
                        .get(i + 3)
                        .is_some_and(|t| t.from_variadic && t.text.is_empty());
                if claimed_by_gnu_group {
                    i += 1;
                    continue;
                }
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
                    // **`right.from_variadic` is the whole GNU rule.** The extension is
                    // `, ## __VA_ARGS__`; `, ## Y` for an ordinary parameter is an ordinary
                    // paste against an empty argument, and the comma survives it — gcc and
                    // clang both keep it. Deciding by "the left operand is a comma and the
                    // right is empty" ate the comma out of `#define X2(Y) fo2{A,##Y}`.
                    let swallow = right.from_variadic
                        && matches!(left.token.kind, PpTokenKind::Punct(Punct::Comma));
                    if !swallow {
                        output.push(left);
                    }
                    i += 2;
                    continue;
                }
                // GNU `, ## args`: `##` suppresses the comma only for an empty
                // variadic argument. With tokens present it is not ordinary token
                // pasting; retain comma and argument as two pp-tokens.
                //
                // **`right.from_variadic` says what the extension applies to**, and it
                // subsumes the earlier `!right.paste_op` guard: an operator `##` and a paste's
                // own result are both `from_variadic: false`, so neither reaches here.
                //
                // With a non-empty variadic tail the GNU form keeps the comma and the argument
                // as two tokens and is legal — `#define X3(b,...) {b, ## __VA_ARGS__}` /
                // `X3(foo, bar)` is `{foo, bar}` under both compilers. With an **ordinary**
                // parameter it is an ordinary paste: `, ## z` forms `,z`, which is not a
                // preprocessing token, and gcc rejects the program. Falling through is what
                // reports that; this branch used to keep both tokens in silence for every
                // right operand, so it hid a real invalid paste as well as eating a comma.
                if right.from_variadic
                    && matches!(left.token.kind, PpTokenKind::Punct(Punct::Comma))
                {
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
                    // **`# ## #` produces a `##` that is not an operator.** This is the token
                    // 6.10.3.3p4's example is built around, and the whole point of the flag: a
                    // paste's *result* is available for further replacement as an ordinary
                    // preprocessing token, never as the operator.
                    paste_op: false,
                    // A pasted token is a new token; it stands for no parameter.
                    from_variadic: false,
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
        // **Nothing leaves this pass still armed.** `output` is a substituted sequence, not a
        // replacement list, so by C11 6.10.3.3 no `##` in it is an operator — including the ones
        // the two recovery branches above push back unconsumed, which is UB input (adjacent `##`)
        // that gcc processes silently and clang rejects, and which neither aborts on.
        //
        // Disarming at the exit rather than in each branch is the point. An adversarial review
        // found the first version of this fix correct at the flag's *source* and unbounded at its
        // exit: a leaked operator rode the rescan into a later macro's sequence and fired there,
        // resurrecting the cross-context panic the whole change was written to remove. A third
        // recovery branch added later would leak the same way; this cannot.
        for token in &mut output {
            token.paste_op = false;
        }
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

/// Mark every `##` in a **replacement list** as the paste operator, and nothing else ever.
///
/// C11 6.10.3.3 identifies the operator here, at definition time. Every `StoredMacro` body goes
/// through this — including the ones built from `-D` on the command line, because
/// `-D'CAT(a,b)=a##b'` defines a macro that pastes exactly like a `#define` does.
///
/// The single call point is what makes the rule checkable: a body reaching `self.macros` without
/// passing through here is a macro whose `##` silently stops working, and there is no second
/// place that can make a token an operator.
/// The queries [`Engine::answer_feature_queries`] answers, in **one** place.
///
/// It was two: a fast-path guard that skipped the whole pass when no query was present, and the
/// matcher inside it. Adding `__has_c_attribute` to the matcher and not the guard made the pass
/// return before it could ever run — a predicate written twice is a predicate that will disagree
/// with itself, and this one did so within a minute of being duplicated.
fn is_feature_query(text: &str) -> bool {
    matches!(
        text,
        "__has_attribute" | "__has_builtin" | "__has_c_attribute" | "__has_cpp_attribute"
    )
}

fn mark_paste_operators(body: &mut [Tok]) {
    for token in body {
        if matches!(token.token.kind, PpTokenKind::Punct(Punct::HashHash)) {
            token.paste_op = true;
        }
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

/// Replace every `__VA_OPT__ ( … )` in a replacement list with its contents or with nothing.
///
/// C23 6.10.3.1: the contents appear when the variadic argument has tokens. Nesting is handled
/// by counting parentheses, so `__VA_OPT__(f(a))` keeps its inner pair.
///
/// An unterminated group is left alone and diagnosed rather than swallowing the rest of the
/// body — the same choice 011 §4 makes for every other malformed construct.
fn expand_va_opt(body: &[Tok], keep: bool, diagnostics: &mut Vec<Diagnostic>) -> Vec<Tok> {
    if !body.iter().any(|token| token.text == "__VA_OPT__") {
        return body.to_vec();
    }
    let mut out = Vec::with_capacity(body.len());
    let mut i = 0;
    while i < body.len() {
        if body[i].text != "__VA_OPT__" {
            out.push(body[i].clone());
            i += 1;
            continue;
        }
        // The `(` itself is only a gate: what follows it is scanned from `i + 1` below, so the
        // token is matched and not otherwise used.
        let Some(_open) = body.get(i + 1).filter(|t| t.text == "(") else {
            diagnostics.push(Diagnostic {
                span: body[i].token.span,
                message: "`__VA_OPT__` must be followed by `(`".into(),
            });
            out.push(body[i].clone());
            i += 1;
            continue;
        };
        let mut depth = 0_usize;
        let mut end = None;
        for (offset, token) in body[i + 1..].iter().enumerate() {
            match token.text.as_str() {
                "(" => depth += 1,
                ")" => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i + 1 + offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            diagnostics.push(Diagnostic {
                span: body[i].token.span,
                message: "unterminated `__VA_OPT__(`".into(),
            });
            out.push(body[i].clone());
            i += 1;
            continue;
        };
        if keep {
            out.extend(body[i + 2..end].iter().cloned());
        }
        i = end + 1;
    }
    out
}

#[allow(dead_code)]
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

impl Engine {
    /// Whether a pending chunk ends inside an **open macro argument list** — the only reason
    /// to hold it across a directive.
    ///
    /// **Not "an unbalanced `(`".** That was wave 408's rule and it is wrong: VPP's X-macro
    /// accumulator opens an ordinary C paren, defines `_`, uses it, and undefines it before
    /// the closing paren. Deferring on that paren carries the uses past the `#undef`, and `_`
    /// no longer exists when they finally expand. A paren counts only when a function-like
    /// macro name opened it.
    ///
    /// A trailing function-like macro name with **no `(` yet does not defer**: measured, both
    /// `gcc -E` and `clang -E` leave `int v = P` / `#define K 5` / `(1);` unexpanded. Only a
    /// paren that is already open holds the chunk.
    fn in_open_macro_args(&self, toks: &[Tok]) -> bool {
        // One entry per unclosed `(`: whether a function-like macro name opened it.
        let mut opened_by_macro: Vec<bool> = Vec::new();
        let mut prev_was_macro = false;
        for t in toks {
            match t.token.kind {
                PpTokenKind::Punct(Punct::LParen) => {
                    opened_by_macro.push(prev_was_macro);
                    prev_was_macro = false;
                }
                PpTokenKind::Punct(Punct::RParen) => {
                    opened_by_macro.pop();
                    prev_was_macro = false;
                }
                PpTokenKind::Ident(_) => prev_was_macro = self.is_function_like_macro(&t.text),
                _ => prev_was_macro = false,
            }
        }
        opened_by_macro.iter().any(|&m| m)
    }

    fn is_function_like_macro(&self, name: &str) -> bool {
        self.by_name
            .get(name)
            .is_some_and(|&i| matches!(self.macros[i].def.kind, MacroKind::FunctionLike { .. }))
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
        paste_op: false,
        from_variadic: false,
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
        paste_op: false,
        from_variadic: false,
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
