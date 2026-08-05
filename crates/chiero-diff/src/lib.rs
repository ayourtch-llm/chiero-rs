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
    Added,
    /// Also a build break for every user, which is why it is not merely "the entity is gone".
    Removed,
}

/// Why an entity is in the set (031 §3).
///
/// Auditability is a requirement: a maintainer told to run 400 tests must be able to ask why.
/// This wave records the class and the distance; the edge chain arrives with §3's closure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Justification {
    pub class: ChangeClass,
    /// 0 for an entity the change touched directly.
    pub distance: u32,
}

/// What could behave differently (031 §3).
#[derive(Clone, Debug, Default)]
pub struct ImpactSet {
    pub entities: IndexMap<Entity, Justification>,
}

/// One side of a comparison: a translation unit, preprocessed and parsed.
#[derive(Debug)]
pub struct Program {
    entities: IndexMap<Entity, Fingerprint>,
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
    /// `None` when it does not parse. 031 contract 15 puts an unparseable file's entities in the
    /// set and marks the result `Partial`, which is §4's job and not this wave's — and refusing
    /// here is the honest placeholder, because returning an empty program would report *no
    /// impact* for a file nobody could read.
    pub fn parse(file: &str, src: &str) -> Option<Program> {
        let tu = chiero_pp::preprocess_str(file, src, chiero_pp::Config::default());
        if !tu.diagnostics.is_empty() {
            return None;
        }
        let mut oracle = chiero_parse::ScopedTypedefs::new();
        let parsed = chiero_parse::parse_tu(&tu, &mut oracle);
        if !parsed.diagnostics.is_empty() {
            return None;
        }
        Some(Program {
            entities: extract(file, &tu, &parsed),
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
    let texts: Vec<&str> = tu.token_texts().collect();
    tu.tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| t.span.lo.0 >= lo && t.span.lo.0 < hi)
        .filter_map(|(i, _)| texts.get(i).map(|s| (*s).to_string()))
        .collect()
}

/// Every entity of one translation unit, with what it would take to change it.
fn extract(
    file: &str,
    tu: &chiero_pp::PreprocessedTu,
    parsed: &chiero_parse::ParsedTu,
) -> IndexMap<Entity, Fingerprint> {
    let ast = &parsed.ast;
    let name_of = |s| parsed.text(s).unwrap_or("?").to_string();
    let mut out: IndexMap<Entity, Fingerprint> = IndexMap::new();

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
        out.insert(
            entity,
            Fingerprint {
                head: tokens_between(tu, lo, split),
                tail: tokens_between(tu, split, hi),
            },
        );
    }
    out
}

/// What could behave differently between two programs (031 §3.1).
///
/// This wave answers the **direct** half: the entities §2 classifies as changed. Closure over
/// callers, expansion sites and types is §3, and every entity here will be a `root` of it.
pub fn impact(old: &Program, new: &Program) -> ImpactSet {
    let mut entities: IndexMap<Entity, Justification> = IndexMap::new();
    let mut record = |e: &Entity, class| {
        entities.insert(e.clone(), Justification { class, distance: 0 });
    };

    for (e, before) in &old.entities {
        match new.entities.get(e) {
            Some(after) => {
                if let Some(class) = classify(before, after) {
                    record(e, class);
                }
            }
            None => record(e, ChangeClass::Removed),
        }
    }
    for e in new.entities.keys() {
        if !old.entities.contains_key(e) {
            record(e, ChangeClass::Added);
        }
    }

    // **031 §5: by kind, then file, then name** — never by discovery order, so two runs give the
    // same report and two reports can be diffed.
    entities.sort_by(|a, _, b, _| {
        (a.kind_rank(), a.file(), a.name()).cmp(&(b.kind_rank(), b.file(), b.name()))
    });
    ImpactSet { entities }
}

/// Which class of change, or `None` for none at all.
///
/// `None` covers 031 §2's `Cosmetic` *and* its `Unchanged` together: both produce no impact, and
/// a caller that has to tell them apart in order to ignore both is one that will eventually
/// forget to.
fn classify(before: &Fingerprint, after: &Fingerprint) -> Option<ChangeClass> {
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
