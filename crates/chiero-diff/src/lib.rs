//! `chiero-diff` — given this change, what could behave differently? (031)
//!
//! The output is an [`ImpactSet`] of entities, each with a justification. 032 intersects that with
//! coverage to pick tests.
//!
//! # Why this is not a diff reader
//!
//! Impact is computed by **comparing two parsed programs**, never by reading diff hunks (031 §1).
//! Hunks say which bytes moved; they cannot say that moving a `}` changed which function a
//! statement belongs to, and they cannot say that a hundred blank lines changed nothing at all.
//!
//! # Why this vertical exists
//!
//! Coverage cannot see macro bodies — 030 §1 measures it: a macro expanded twice at `t.c:3` puts
//! both expansions on that line and leaves the macro's own line with *no record at all*. So a
//! coverage-only tool is blind to any edit inside `vec.h`, `pool.h`, or any of VPP's 754
//! `foreach_*` X-macros. Owning the preprocessor is what makes those edits answerable, and 031
//! §3.2 is where that is spent.

use indexmap::IndexMap;

/// One thing a change can be *about* (031 §1).
///
/// **Keyed by name, not by `FileId` and `Symbol`.** 031 §1 writes those, and both are indices
/// into one `SourceMap` and one interner — while impact compares two separately parsed programs
/// whose indices are unrelated, so `FileId(3)` is a different file on each side. Text is the only
/// identity that survives the crossing.
///
/// A `static` function is file-scoped and never merged across translation units (014 §4), which
/// the file component gives for free: two `helper`s in two files are two entities, and changing
/// one must not select the other's tests.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Entity {
    Function { file: String, name: String },
    Global { file: String, name: String },
    Typedef { file: String, name: String },
    Record { file: String, tag: String },
    EnumConst { file: String, name: String },
    Macro { file: String, name: String },
}

impl Entity {
    pub fn function(file: &str, name: &str) -> Entity {
        Entity::Function {
            file: file.into(),
            name: name.into(),
        }
    }

    pub fn global(file: &str, name: &str) -> Entity {
        Entity::Global {
            file: file.into(),
            name: name.into(),
        }
    }

    pub fn typedef(file: &str, name: &str) -> Entity {
        Entity::Typedef {
            file: file.into(),
            name: name.into(),
        }
    }

    /// Named with a trailing underscore because `macro` is a Rust keyword.
    pub fn macro_(file: &str, name: &str) -> Entity {
        Entity::Macro {
            file: file.into(),
            name: name.into(),
        }
    }

    pub fn record(file: &str, tag: &str) -> Entity {
        Entity::Record {
            file: file.into(),
            tag: tag.into(),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Entity::Function { name, .. }
            | Entity::Global { name, .. }
            | Entity::Typedef { name, .. }
            | Entity::EnumConst { name, .. }
            | Entity::Macro { name, .. } => name,
            Entity::Record { tag, .. } => tag,
        }
    }

    pub fn file(&self) -> &str {
        match self {
            Entity::Function { file, .. }
            | Entity::Global { file, .. }
            | Entity::Typedef { file, .. }
            | Entity::EnumConst { file, .. }
            | Entity::Macro { file, .. }
            | Entity::Record { file, .. } => file,
        }
    }

    /// The `kind` half of 031 §5's ordering, so a report is stable across runs.
    fn kind_rank(&self) -> u8 {
        match self {
            Entity::Function { .. } => 0,
            Entity::Global { .. } => 1,
            Entity::Typedef { .. } => 2,
            Entity::Record { .. } => 3,
            Entity::EnumConst { .. } => 4,
            Entity::Macro { .. } => 5,
        }
    }
}

/// How an entity differs between the two sides (031 §2).
///
/// **`Cosmetic` is absent from this enum on purpose.** 031 §2 lists it as a class with no impact;
/// an entity that differs only in whitespace, comments or line position produces *no entry at
/// all*, because an `ImpactSet` a caller has to filter is one a caller will forget to filter.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ChangeClass {
    /// Statements or expressions differ; callers may behave differently.
    BodyChanged,
    /// Parameters, return type, variadicity or linkage differ — so **every** caller is affected,
    /// even though no body did.
    SignatureChanged,
    /// A global's initial value; its readers are affected.
    InitializerChanged,
    /// A macro's replacement list differs — **every expansion site** (031 §3.2).
    MacroBodyChanged,
    /// A macro's parameter names or count, its variadicity, or object-versus-function-like.
    ///
    /// Renaming a parameter lands here even when nothing can behave differently: chiero does not
    /// attempt to prove two macro bodies equivalent (contract 7), and the direction a wrong guess
    /// would fail in is the one that skips tests.
    MacroInterfaceChanged,
    Added,
    /// Also a build break for every user, which is why it is not merely "the entity is gone".
    Removed,
    /// A record's **computed layout** differs: its size, alignment, or any field offset.
    ///
    /// **Computed, never syntactic** (031 §2). Reordering two same-size fields moves every
    /// offset after the first while changing two tokens; renaming a field changes the same two
    /// tokens and moves nothing. `size_delta` is `new - old` in bytes, so a report can say
    /// "8 → 5" — which is what tells a maintainer whether a wire format moved.
    LayoutChanged {
        size_delta: i64,
    },
    /// **chiero could not read this entity**, so it must be assumed changed (031 §4).
    ///
    /// Not a guess about what happened to it. A file that failed to parse tells you nothing about
    /// its contents, and the only safe reading of nothing is everything.
    Unknown,
}

/// How an entity was reached (031 §3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImpactEdge {
    /// This is the entity the change is *in*.
    DirectlyChanged,
    /// It calls something that changed.
    Calls { callee: String },
    /// It reads or writes a global that changed.
    UsesGlobal { name: String },
    /// It mentions a type or typedef that changed.
    UsesType { name: String },
    /// **It expands a macro that changed** — 031 §3.2, and the edge coverage cannot produce.
    ExpandsMacro { name: String },
    /// Its file could not be parsed, so nothing about it is known (031 §4).
    FileUnparsed { file: String },
}

/// What the analysis could not see (031 §4).
///
/// **Every gap widens the set rather than narrowing it.** Missing an impacted entity means
/// silently skipping the test that would have caught the regression; an extra entity costs a test
/// run. §4 is explicit that a tool which quietly narrows here "is worse than no tool: it converts
/// an unknown into a false assurance".
///
/// `Partial` is what makes 032's always-run set non-empty, so a caller must match on it rather
/// than read the entity list alone.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Completeness {
    #[default]
    Complete,
    Partial {
        /// Files neither side could parse. Every entity of one is in the set, classed
        /// [`ChangeClass::Unknown`].
        unparsed_files: Vec<String>,
        /// Indirect calls whose targets could not be resolved. §4 widens each to every
        /// signature-compatible target; **not yet implemented**, so this is always 0 and a
        /// caller must not read 0 as "there were none".
        unresolved_calls: u32,
        /// Build configurations the analysis did not enumerate. §4 widens to all of them;
        /// **not yet implemented**, so this is always empty.
        unknown_configs: Vec<String>,
        /// Functions whose address was taken and whose indirect callers were approximated.
        /// **Not yet implemented**, so this is always 0.
        address_taken_fallbacks: u32,
    },
}

/// Why an entity is in the set (031 §3).
///
/// **Auditability is a requirement**, not a nicety: a maintainer told to run 400 tests must be
/// able to ask why and get "because `foo()` calls `bar`, whose body you changed". A tool that
/// cannot answer that is one whose answers get overridden, and an overridden test-selection tool
/// is a slower way to run the whole suite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Justification {
    /// The class of the change at the **root**, which is what a report leads with.
    pub class: ChangeClass,
    /// The entity that actually changed, at the far end of the chain.
    pub root: Entity,
    /// How this entity reaches the root. `[DirectlyChanged]` when it *is* the root.
    pub edges: Vec<ImpactEdge>,
    /// How far the closure walked: 0 for the root, 1 for its callers, and so on.
    pub distance: u32,
}

/// What could behave differently (031 §3).
#[derive(Clone, Debug, Default)]
pub struct ImpactSet {
    pub entities: IndexMap<Entity, Justification>,
    /// What the analysis could not see. **Match on this**; the entity list alone is not the
    /// answer (031 §4).
    pub completeness: Completeness,
}

/// `ParsedTu`'s symbol table, as 014 wants it.
struct Names<'a>(&'a chiero_parse::ParsedTu);

impl chiero_sema::SymbolText for Names<'_> {
    fn text(&self, sym: chiero_span::Symbol) -> Option<&str> {
        self.0.text(sym)
    }
}

/// Where one named field sits: its byte offset, and its bit offset and width if it is a
/// bit-field.
///
/// A bit-field's offset is absolute rather than relative to the byte, because gcc's straddling
/// rules move the byte offset around (014 §3).
type FieldPlace = (String, u64, Option<(u64, u64)>);

/// What a change to a record would have to alter for its users to care.
///
/// Field *names* are deliberately absent: renaming one is a change, and the token fingerprint
/// already sees it. What this holds is what only 014 can compute.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordShape {
    size: u64,
    align: u64,
    is_union: bool,
    packed: bool,
    /// Each named field's byte offset, and its bit offset and width where it is a bit-field.
    ///
    /// **Keyed by name, and that is the whole subtlety.** Reordering two *same-size* fields moves
    /// no offset in the list — `int a; short b; short c;` and `int a; short c; short b;` both lay
    /// out at 0, 4, 6 — but the offset *of the field named `b`* goes from 4 to 6, and `p->b` now
    /// reads different bytes. Comparing a bare offset list would call that unchanged.
    fields: Vec<FieldPlace>,
}

/// A loader for a translation unit that includes nothing.
///
/// Reaching a `#include` through this is an error rather than an empty file, so a fixture that
/// forgot its include paths fails loudly instead of comparing two programs that are both missing
/// the same declarations.
struct NoIncludes;

impl chiero_pp::FileLoader for NoIncludes {
    fn load(&mut self, path: &std::path::Path) -> std::io::Result<String> {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "no include paths were configured, so `{}` cannot be read",
                path.display()
            ),
        ))
    }
}

/// One side of a comparison: a translation unit, preprocessed and parsed.
#[derive(Debug)]
pub struct Program {
    entities: IndexMap<Entity, Fingerprint>,
    /// Which macro each entity expands, by name, from the preprocessor's reverse index.
    ///
    /// **This is the join coverage cannot make.** gcov records the `.c` line a macro was *used*
    /// on and nothing about the macro itself (030 §1, measured), so a coverage index has no way
    /// back from a header edit to the functions it reaches.
    expands: IndexMap<Entity, Vec<String>>,
    /// The file this translation unit is, for reporting it as unparsed.
    file: String,
    /// Whether the preprocessor and the parser were both silent.
    ///
    /// **Recorded rather than refused.** An unparseable file's entities must reach the impact set
    /// (031 §4), so the program is still built from whatever the parser recovered — and the flag
    /// is what stops that partial recovery being read as a complete answer.
    parsed_cleanly: bool,
    /// Each record's computed layout, by tag — 014 §3's answer, not this crate's.
    ///
    /// **The one thing tokens cannot supply.** `__attribute__((packed))` on an already-tight
    /// struct changes no byte, and a field rename changes no offset; both differ in the token
    /// stream and neither is a layout change.
    layouts: IndexMap<String, RecordShape>,
    /// Which names each entity mentions, for §3's closure.
    ///
    /// **Names, not resolved bindings.** A local variable shadowing a global's name puts its
    /// holder in the set for a change it cannot see — an extra test run. Resolving would need
    /// 014's scopes on both sides, and the error it removes is in the safe direction while every
    /// error it could introduce is in the other.
    refs: IndexMap<Entity, Vec<String>>,
}

/// What an entity *is*, reduced to what a change would have to alter for it to matter.
///
/// **Two token strings, not one.** A signature change and a body change close over different
/// things — every caller, against the callers that may now differ — so they cannot be told apart
/// once concatenated. Splitting them here is what lets [`classify`] answer with the class 031 §2
/// names rather than with "something differs".
#[derive(Clone, Debug, PartialEq, Eq)]
struct Fingerprint {
    /// Everything up to the body or initializer: the specifiers and the declarator.
    head: Vec<String>,
    /// The body, the initializer, or nothing.
    tail: Vec<String>,
}

impl Program {
    /// Preprocess and parse one translation unit.
    ///
    /// **`None` only when the source cannot be turned into a program at all.** A file that fails
    /// to *parse* still yields one, flagged: 031 §4 requires its entities to reach the impact set
    /// and the result to be `Partial`, because a file nobody could read tells you nothing about
    /// its contents and the only safe reading of nothing is everything.
    pub fn parse(file: &str, src: &str) -> Option<Program> {
        Program::parse_with(file, src, chiero_pp::Config::default(), &mut NoIncludes)
    }

    /// The same, with include paths and a loader — which a diff that touches only a header
    /// needs, because the contract is about a change no `.c` file contains.
    ///
    /// The loader is the caller's because `chiero-pp` has no disk implementation on purpose: a
    /// preprocessor that reaches for the filesystem cannot be tested against a header that does
    /// not exist yet, which is precisely the shape of a *before* and *after* comparison.
    pub fn parse_with<L: chiero_pp::FileLoader>(
        file: &str,
        src: &str,
        config: chiero_pp::Config,
        loader: &mut L,
    ) -> Option<Program> {
        let tu = chiero_pp::preprocess_with_loader(file, src, config, loader);
        let mut oracle = chiero_parse::ScopedTypedefs::new();
        let parsed = chiero_parse::parse_tu(&tu, &mut oracle);
        let parsed_cleanly = tu.diagnostics.is_empty() && parsed.diagnostics.is_empty();
        // **014 computes the layouts; this crate must not.** Duplicating the rules here is
        // exactly the syntactic comparison 031 §2 rules out, and gcc's straddling and packing
        // behaviour is measured in 014's own corpus gate against gcc itself.
        let analysis = chiero_sema::analyze(
            &parsed.ast,
            &chiero_sema::TargetConfig::x86_64_linux(),
            &Names(&parsed),
        );
        let mut layouts: IndexMap<String, RecordShape> = IndexMap::new();
        for (i, l) in analysis.records().iter().enumerate() {
            if !l.complete {
                continue;
            }
            let Some(tag) = analysis
                .tag_of(chiero_sema::RecordId(i as u32))
                .and_then(|t| parsed.text(t))
            else {
                // An anonymous record has no name for an entity to be keyed by, and nothing
                // outside its declaration can refer to it.
                continue;
            };
            layouts.insert(
                tag.to_string(),
                RecordShape {
                    size: l.size,
                    align: l.align,
                    is_union: l.is_union,
                    packed: l.packed,
                    fields: l
                        .fields
                        .iter()
                        .filter_map(|f| {
                            let name = parsed.text(f.name?)?.to_string();
                            Some((name, f.offset, f.bits.map(|b| (b.bit_offset, b.width))))
                        })
                        .collect(),
                },
            );
        }

        let (entities, refs, spans) = extract(file, &tu, &parsed);
        let (macros, expands) = extract_macros(&tu, &spans);
        let mut entities = entities;
        entities.extend(macros);
        Some(Program {
            file: file.to_string(),
            parsed_cleanly,
            layouts,
            entities,
            expands,
            refs,
        })
    }

    /// Every entity this translation unit declares, in written order.
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.keys()
    }
}

/// The tokens whose span lies in `[lo, hi)`, as text.
///
/// **This is 031 §2's "normalized token stream".** Whitespace, comments and line positions are
/// gone by the time the preprocessor is done, so two spellings that differ only in those produce
/// the same vector — which is contracts 1, 2 and 3 together, and for free.
fn tokens_between(tu: &chiero_pp::PreprocessedTu, lo: u32, hi: u32) -> Vec<String> {
    // `text_at` rather than `token_texts`, which would collect a vector of every token's text
    // *per entity* — quadratic in a translation unit, and VPP's headers expand to a million
    // tokens before the first declaration of the file itself.
    tu.tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| t.span.lo.0 >= lo && t.span.lo.0 < hi)
        .filter_map(|(i, _)| tu.text_at(i).map(str::to_string))
        .collect()
}

/// Every entity of one translation unit, with what it would take to change it.
#[allow(clippy::type_complexity)]
fn extract(
    file: &str,
    tu: &chiero_pp::PreprocessedTu,
    parsed: &chiero_parse::ParsedTu,
) -> (
    IndexMap<Entity, Fingerprint>,
    IndexMap<Entity, Vec<String>>,
    IndexMap<Entity, (u32, u32)>,
) {
    let ast = &parsed.ast;
    let name_of = |s| parsed.text(s).unwrap_or("?").to_string();
    let mut out: IndexMap<Entity, Fingerprint> = IndexMap::new();
    let mut refs: IndexMap<Entity, Vec<String>> = IndexMap::new();
    let mut spans: IndexMap<Entity, (u32, u32)> = IndexMap::new();

    for &id in ast.items() {
        let decl = ast.decl(id);
        let (lo, hi) = (decl.span.lo.0, decl.span.hi.0);
        let (entity, split) = match &decl.kind {
            chiero_ast::DeclKind::Func { name, body, .. } => {
                // The head is everything before the body's `{`. A prototype has no body, so its
                // tail is empty and a declaration never looks like an edit of its definition.
                let at = body.map_or(hi, |b| ast.stmt(b).span.lo.0);
                (Entity::function(file, &name_of(*name)), at)
            }
            chiero_ast::DeclKind::Var {
                name: Some(name),
                init,
                ..
            } => {
                let at = init.map_or(hi, |e| ast.expr(e).span.lo.0);
                (Entity::global(file, &name_of(*name)), at)
            }
            chiero_ast::DeclKind::Typedef { name, .. } => {
                (Entity::typedef(file, &name_of(*name)), hi)
            }
            chiero_ast::DeclKind::TagDef { ty } => {
                let chiero_ast::TypeKind::Tag {
                    name: Some(tag), ..
                } = &ast.ty(*ty).kind
                else {
                    // An anonymous tag has no name to key an entity by, and nothing outside the
                    // declaration it sits in can refer to it.
                    continue;
                };
                (Entity::record(file, &name_of(*tag)), hi)
            }
            _ => continue,
        };
        // **Every identifier the declaration mentions**, taken from its own token span. A body
        // that calls `leaf` mentions `leaf`; one that reads `limit` mentions `limit`. Keywords
        // and punctuation are not identifiers and cannot name an entity, so they are dropped.
        let mut mentioned: Vec<String> = tokens_between(tu, lo, hi)
            .into_iter()
            .filter(|t| {
                let mut cs = t.chars();
                cs.next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
            .filter(|t| *t != entity.name())
            .collect();
        mentioned.sort();
        mentioned.dedup();
        refs.insert(entity.clone(), mentioned);
        spans.insert(entity.clone(), (lo, hi));
        out.insert(
            entity,
            Fingerprint {
                head: tokens_between(tu, lo, split),
                tail: tokens_between(tu, split, hi),
            },
        );
    }
    (out, refs, spans)
}

/// Every macro the translation unit defined, and which entity expands which.
///
/// **031 §3.2 steps 1 and 2, and the whole differentiator.** `SourceMap::expansion_sites` is the
/// preprocessor's reverse index — where was this macro expanded — and it is already transitive in
/// the sense that matters: an expansion of `m` nested inside another macro's body is still
/// recorded against `m`. Each site's `expansion_loc` gives a position in a `.c` file, and the
/// entity whose declaration spans that position is the function that expands it.
#[allow(clippy::type_complexity)]
fn extract_macros(
    tu: &chiero_pp::PreprocessedTu,
    spans: &IndexMap<Entity, (u32, u32)>,
) -> (IndexMap<Entity, Fingerprint>, IndexMap<Entity, Vec<String>>) {
    let sm = &tu.source_map;
    let mut out: IndexMap<Entity, Fingerprint> = IndexMap::new();
    let mut expands: IndexMap<Entity, Vec<String>> = IndexMap::new();

    for def in &tu.macro_defs {
        let Some(name) = tu.symbol_text(def.name) else {
            continue;
        };
        let file = sm
            .lookup_file(def.def_span.lo)
            .map(|f| file_name(sm.file(f).path()))
            .unwrap_or_else(|| "<command line>".to_string());
        let entity = Entity::macro_(&file, name);

        // The interface and the replacement list are separate for the same reason a function's
        // signature and body are: 031 §2 gives them different classes because they close over
        // differently — though for a macro both reach every expansion site, so the distinction is
        // in what the report says rather than in what it selects.
        let head = match &def.kind {
            chiero_pp::MacroKind::ObjectLike => vec!["object-like".to_string()],
            chiero_pp::MacroKind::FunctionLike { params, variadic } => {
                let mut v = vec![format!("function-like({variadic:?})")];
                v.extend(
                    params
                        .iter()
                        .map(|p| tu.symbol_text(*p).unwrap_or("?").to_string()),
                );
                v
            }
        };
        // **The body's own text, span by span.** `text_at` indexes the *translation unit's*
        // token stream, so indexing it by a body position hands back the first N tokens of the
        // file — identical for every macro and identical on both sides, which makes a body edit
        // invisible. The parameter-rename test passed anyway, because its head differed.
        let tail: Vec<String> = def
            .body
            .iter()
            .filter_map(|t| sm.span_text(t.span))
            .map(str::to_string)
            .collect();
        out.insert(entity, Fingerprint { head, tail });

        for ctx in sm.expansion_sites(def.id).collect::<Vec<_>>() {
            let Some(e) = sm.expansion(ctx) else { continue };
            let Some(loc) = sm.expansion_loc(e.call_site) else {
                continue;
            };
            let at = loc.pos.0;
            for (holder, (lo, hi)) in spans {
                if at >= *lo && at < *hi {
                    let v = expands.entry(holder.clone()).or_default();
                    if !v.iter().any(|n| n == name) {
                        v.push(name.to_string());
                    }
                }
            }
        }
    }
    (out, expands)
}

/// A path's final component, which is how [`Entity`] names files.
fn file_name(p: &std::path::Path) -> String {
    p.file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string_lossy().into_owned())
}

/// What could behave differently between two programs (031 §3.1).
///
/// This wave answers the **direct** half: the entities §2 classifies as changed. Closure over
/// callers, expansion sites and types is §3, and every entity here will be a `root` of it.
pub fn impact(old: &Program, new: &Program) -> ImpactSet {
    let mut entities: IndexMap<Entity, Justification> = IndexMap::new();
    let mut direct = |e: &Entity, class| {
        entities.insert(
            e.clone(),
            Justification {
                class,
                root: e.clone(),
                edges: vec![ImpactEdge::DirectlyChanged],
                distance: 0,
            },
        );
    };

    // §3.1 — the entities §2 classifies as changed.
    for (e, before) in &old.entities {
        match new.entities.get(e) {
            Some(after) => {
                // **A record is compared by its computed layout first** (031 §2). Two tokens
                // differ whether a field was reordered or renamed, and only 014 knows which of
                // those moved a byte.
                let class = match e {
                    Entity::Record { tag, .. } => {
                        layout_class(old.layouts.get(tag), new.layouts.get(tag), before, after)
                    }
                    _ => classify(before, after),
                };
                if let Some(class) = class {
                    direct(e, class);
                }
            }
            None => direct(e, ChangeClass::Removed),
        }
    }
    for e in new.entities.keys() {
        if !old.entities.contains_key(e) {
            direct(e, ChangeClass::Added);
        }
    }

    // **§4: a file nobody could read puts every one of its entities in the set.** Done before the
    // closure, so that whatever those entities reach comes with them — a gap that widened the set
    // and then stopped would be a narrower gap than the one that exists.
    let mut unparsed_files: Vec<String> = Vec::new();
    for p in [old, new] {
        if p.parsed_cleanly || unparsed_files.contains(&p.file) {
            continue;
        }
        unparsed_files.push(p.file.clone());
        for e in p.entities.keys().filter(|e| e.file() == p.file) {
            entities.entry(e.clone()).or_insert_with(|| Justification {
                class: ChangeClass::Unknown,
                root: e.clone(),
                edges: vec![ImpactEdge::FileUnparsed {
                    file: p.file.clone(),
                }],
                distance: 0,
            });
        }
    }

    close_over_references(old, new, &mut entities);

    // **031 §5: by kind, then file, then name** — never by discovery order, so two runs give the
    // same report and two reports can be diffed.
    entities.sort_by(|a, _, b, _| {
        (a.kind_rank(), a.file(), a.name()).cmp(&(b.kind_rank(), b.file(), b.name()))
    });
    ImpactSet {
        entities,
        completeness: if unparsed_files.is_empty() {
            Completeness::Complete
        } else {
            Completeness::Partial {
                unparsed_files,
                unresolved_calls: 0,
                unknown_configs: Vec::new(),
                address_taken_fallbacks: 0,
            }
        },
    }
}

/// Grow the set to a fixpoint over the entities that mention what is already in it (031 §3).
///
/// **Both sides' reference graphs, unioned.** An entity that *stopped* calling a deleted function
/// is reached only through the old program, and one that started calling a changed function only
/// through the new — and contract 18 is exactly the first case. Taking the union over-approximates
/// where they disagree, which is the safe direction.
///
/// Breadth-first, so `distance` is the length of the shortest path to a root and an entity placed
/// once is never re-placed by a longer chain. That is also what terminates it: mutual recursion is
/// ordinary C, and a fixpoint that revisited a placed entity would not stop.
fn close_over_references(
    old: &Program,
    new: &Program,
    entities: &mut IndexMap<Entity, Justification>,
) {
    let mut frontier: Vec<Entity> = entities.keys().cloned().collect();
    let mut distance = 0u32;

    while !frontier.is_empty() {
        distance += 1;
        let mut next: Vec<Entity> = Vec::new();
        let by_reference = old.refs.iter().chain(new.refs.iter());
        // **The macro edge is separate from the name edge**, because an expansion site is not a
        // mention: `a()` never writes `BUMP`'s name after preprocessing, and the token stream it
        // is fingerprinted on holds the *expansion*. Only the preprocessor's reverse index knows.
        let by_expansion = old.expands.iter().chain(new.expands.iter());
        for (holder, mentioned) in by_reference.chain(by_expansion) {
            if entities.contains_key(holder) {
                continue;
            }
            let Some(target) = frontier
                .iter()
                .find(|t| mentioned.iter().any(|m| m == t.name()))
            else {
                continue;
            };
            let via = &entities[target];
            let mut edges = vec![edge_to(target)];
            edges.extend(
                via.edges
                    .iter()
                    .filter(|e| **e != ImpactEdge::DirectlyChanged)
                    .cloned(),
            );
            entities.insert(
                holder.clone(),
                Justification {
                    // The class stays the *root's*: what a report leads with is the change
                    // somebody made, not the fact that this entity mentions it.
                    class: via.class,
                    root: via.root.clone(),
                    edges,
                    distance,
                },
            );
            next.push(holder.clone());
        }
        frontier = next;
    }
}

/// The edge that reaching `target` represents.
fn edge_to(target: &Entity) -> ImpactEdge {
    match target {
        Entity::Function { name, .. } => ImpactEdge::Calls {
            callee: name.clone(),
        },
        Entity::Global { name, .. } => ImpactEdge::UsesGlobal { name: name.clone() },
        Entity::Typedef { name, .. } | Entity::Record { tag: name, .. } => {
            ImpactEdge::UsesType { name: name.clone() }
        }
        Entity::Macro { name, .. } => ImpactEdge::ExpandsMacro { name: name.clone() },
        Entity::EnumConst { name, .. } => ImpactEdge::UsesGlobal { name: name.clone() },
    }
}

/// How a record changed: its layout if any byte moved, otherwise whatever its tokens say.
///
/// **The layout question is asked first and answered by 014.** `__attribute__((packed))` on an
/// already-tight struct changes no offset and no size — the tokens differ loudly and nothing
/// downstream can observe it — while a `#pragma pack` can move every offset without touching the
/// struct's own tokens. Neither is answerable syntactically.
///
/// A record with no computed layout on one side — incomplete, or anonymous — falls back to the
/// token comparison rather than being silently called unchanged.
fn layout_class(
    before: Option<&RecordShape>,
    after: Option<&RecordShape>,
    before_tokens: &Fingerprint,
    after_tokens: &Fingerprint,
) -> Option<ChangeClass> {
    let (Some(b), Some(a)) = (before, after) else {
        // Incomplete or anonymous on one side: fall back to the tokens rather than silently
        // calling it unchanged.
        return classify(before_tokens, after_tokens);
    };

    // Size, alignment, and packing are observable whether or not a field moved. `packed` on an
    // already-tight struct removes no padding and still drops the alignment from 4 to 1 —
    // measured against gcc — so a struct embedding it moves from offset 4 to offset 1.
    let shape_moved = b.size != a.size || b.align != a.align || b.is_union != a.is_union;

    // **A field that exists on both sides and sits somewhere else.** Restricted to the common
    // names, because a name that appears on one side only is a field added, removed or renamed —
    // a source-compatibility change for its users, and 031 §2 is explicit that it is not a layout
    // one.
    let moved_field = a.fields.iter().any(|(name, off, bits)| {
        b.fields
            .iter()
            .find(|(n, _, _)| n == name)
            .is_some_and(|(_, o, bt)| o != off || bt != bits)
    });

    if shape_moved || moved_field {
        return Some(ChangeClass::LayoutChanged {
            size_delta: a.size as i64 - b.size as i64,
        });
    }
    // No byte moved. The definition may still differ — a renamed field — and its users name it.
    (before_tokens != after_tokens).then_some(ChangeClass::BodyChanged)
}

/// Which class of change, or `None` for none at all.
///
/// `None` covers 031 §2's `Cosmetic` *and* its `Unchanged` together: both produce no impact, and
/// a caller that has to tell them apart in order to ignore both is one that will eventually
/// forget to.
fn classify(before: &Fingerprint, after: &Fingerprint) -> Option<ChangeClass> {
    // A macro's head is its interface — object- versus function-like, the parameter names, the
    // variadicity — and is marked so the class can say which half moved.
    let is_macro = before
        .head
        .first()
        .is_some_and(|h| h.starts_with("object-like") || h.starts_with("function-like"));
    if is_macro {
        if before.head != after.head {
            return Some(ChangeClass::MacroInterfaceChanged);
        }
        return (before.tail != after.tail).then_some(ChangeClass::MacroBodyChanged);
    }
    if before.head != after.head {
        // The specifiers or the declarator differ: return type, parameters, linkage. Every caller
        // is affected whether or not the body moved.
        return Some(ChangeClass::SignatureChanged);
    }
    if before.tail == after.tail {
        return None;
    }
    Some(if before.tail.first().map(String::as_str) == Some("{") {
        ChangeClass::BodyChanged
    } else {
        ChangeClass::InitializerChanged
    })
}
