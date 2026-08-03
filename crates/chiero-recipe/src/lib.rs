//! `chiero-recipe` — see `docs/specs/`.
//!
//! This crate currently implements the **load** boundary of 042: reading a `.recipe` file
//! and refusing one that its own fixtures could not adjudicate (042 §5, contracts 1 and 3).
//! Evaluation is not here yet, and is deliberately absent rather than stubbed: an evaluator
//! that answers "no findings" would satisfy the `good`-fixture contract while covering
//! nothing, which is the rot §5 exists to prevent.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Tier 1 (042 §3): syntactic match, cheap enough to sweep all of VPP.
    Structural,
    /// Tier 2: needs types, paths and feasibility.
    Semantic,
}

/// A `bad` fixture: the file, how many findings it must produce, and where.
///
/// `at` is not optional. 042 contract 3 requires a finding at the *wrong* location to fail
/// the recipe, and a fixture declaring no location can never detect that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadFixture {
    pub path: String,
    pub expect: usize,
    pub at: String,
}

/// Which functions a recipe applies to (042 §4.2). Tier 1.
///
/// **One variant per selector, never a string.** Adding a selector must break the evaluator
/// at compile time; a stringly-typed selector would let an unknown one fall through a `_` arm
/// and quietly select nothing, which reads exactly like a rule that found no violations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    /// `registered_via VLIB_CLI_COMMAND`
    RegisteredVia(String),
    /// `in_file "src/vnet/**/*_cli.c"`
    InFile(String),
    /// `name matches "^show_"`
    NameMatches(String),
    /// `has_attribute noreturn`
    HasAttribute(String),
    /// `signature "..."`
    Signature(String),
    /// `calls \`unformat_line_input($_)\``
    Calls(String),
}

/// A function as tier 1 sees it before any AST is available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionRef {
    pub name: String,
    /// Path as the sweep records it, `/`-separated.
    pub file: String,
}

/// **Three-valued on purpose.** A selector this crate cannot yet evaluate must not answer
/// `No`: that would silently empty the scope of every recipe using it and report a clean
/// tree, which is the failure 042 §5 exists to prevent wearing a different hat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Yes,
    No,
    /// Needs the typed AST — `registered_via`, `has_attribute`, `signature`, `calls`.
    NeedsAst,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// The metavariable the rest of the recipe refers to, e.g. `$f`.
    pub var: String,
    pub selector: Selector,
}

impl Scope {
    /// Does this scope select `f`, given only a name and a path?
    pub fn selects(&self, f: &FunctionRef) -> Selection {
        match &self.selector {
            Selector::NameMatches(re) => match regex::Regex::new(re) {
                // Validated at load, so a failure here cannot be reached by a loaded recipe;
                // answering `NeedsAst` rather than `No` keeps that unreachable path honest.
                Err(_) => Selection::NeedsAst,
                Ok(r) => yes_no(r.is_match(&f.name)),
            },
            Selector::InFile(glob) => match regex::Regex::new(&glob_to_regex(glob)) {
                Err(_) => Selection::NeedsAst,
                Ok(r) => yes_no(r.is_match(&f.file)),
            },
            Selector::RegisteredVia(_)
            | Selector::HasAttribute(_)
            | Selector::Signature(_)
            | Selector::Calls(_) => Selection::NeedsAst,
        }
    }
}

fn yes_no(b: bool) -> Selection {
    if b { Selection::Yes } else { Selection::No }
}

/// Translate a path glob to an anchored regex.
///
/// `**` crosses separators, `*` does not, `?` is one non-separator character. Everything else
/// is escaped: a `.` in `*_cli.c` must be a literal dot, or the glob would also match
/// `foo_cliXc`.
fn glob_to_regex(glob: &str) -> String {
    let mut out = String::from("^");
    let b: Vec<char> = glob.chars().collect();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            '*' if i + 1 < b.len() && b[i + 1] == '*' => {
                // `**/` should also match zero directories, so the separator is optional.
                if i + 2 < b.len() && b[i + 2] == '/' {
                    out.push_str("(?:.*/)?");
                    i += 3;
                    continue;
                }
                out.push_str(".*");
                i += 2;
                continue;
            }
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            c => out.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    out.push('$');
    out
}

/// A clause this loader has not learned to read yet, kept whole.
///
/// Retained rather than discarded so the unread part of the language is *visible*: a later
/// wave can count them, and a recipe does not silently lose the clause that gives it meaning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnparsedClause {
    pub keyword: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    pub name: String,
    pub title: String,
    pub severity: Severity,
    pub tier: Tier,
    pub rationale: String,
    /// `None` when the recipe applies to every function; 042 §4.2 makes `scope` a way to
    /// narrow a rule, not a requirement.
    pub scope: Option<Scope>,
    pub good: Vec<String>,
    pub bad: Vec<BadFixture>,
    pub unparsed_clauses: Vec<UnparsedClause>,
}

/// Load one recipe. `Err` carries every diagnostic, each naming the recipe.
///
/// **Every diagnostic names the recipe**, because a catalogue is loaded in bulk and
/// "missing good fixture" does not say which of forty it meant (023 §9).
pub fn load(src: &str) -> Result<Recipe, Vec<String>> {
    let mut p = Parser::new(src);
    let name = p.header()?;
    let mut r = Recipe {
        name: name.clone(),
        title: String::new(),
        severity: Severity::Error,
        tier: Tier::Semantic,
        rationale: String::new(),
        scope: None,
        good: Vec::new(),
        bad: Vec::new(),
        unparsed_clauses: Vec::new(),
    };
    let mut errs: Vec<String> = Vec::new();
    let say = |errs: &mut Vec<String>, m: String| errs.push(format!("recipe `{name}`: {m}"));

    while let Some(keyword) = p.next_keyword() {
        match keyword.as_str() {
            "title" => r.title = p.string().unwrap_or_default(),
            "rationale" => r.rationale = p.string().unwrap_or_default(),
            "severity" => match p.word().as_deref() {
                Some("error") => r.severity = Severity::Error,
                Some("warning") => r.severity = Severity::Warning,
                Some("note") => r.severity = Severity::Note,
                other => say(
                    &mut errs,
                    format!("unknown severity `{}`", other.unwrap_or("")),
                ),
            },
            "tier" => match p.word().as_deref() {
                Some("structural") => r.tier = Tier::Structural,
                Some("semantic") => r.tier = Tier::Semantic,
                other => say(&mut errs, format!("unknown tier `{}`", other.unwrap_or(""))),
            },
            "fixture" => match p.word().as_deref() {
                Some("good") => match p.string() {
                    Some(path) => r.good.push(path),
                    None => say(&mut errs, "`fixture good` needs a path".into()),
                },
                Some("bad") => match p.bad_fixture() {
                    Ok(b) => r.bad.push(b),
                    Err(m) => say(&mut errs, m),
                },
                other => say(
                    &mut errs,
                    format!(
                        "a fixture is `good` or `bad`, not `{}`",
                        other.unwrap_or("")
                    ),
                ),
            },
            "scope" => match p.scope_clause() {
                Ok(sc) => r.scope = Some(sc),
                Err(m) => say(&mut errs, m),
            },
            other => {
                let text = p.clause_body();
                r.unparsed_clauses.push(UnparsedClause {
                    keyword: other.to_owned(),
                    text,
                });
            }
        }
    }

    // **A regex is compiled at load, not at sweep time.** A catalogue running over 1552 files
    // is the wrong place to discover that a pattern does not parse.
    if let Some(Scope {
        selector: Selector::NameMatches(re),
        ..
    }) = &r.scope
        && let Err(e) = regex::Regex::new(re)
    {
        say(
            &mut errs,
            format!("`name matches` is not a valid regex: {e}"),
        );
    }

    // 042 §5 — both kinds are mandatory, and they catch opposite failures: `good` catches
    // over-matching, `bad` catches under-matching, which is the more dangerous of the two.
    if r.good.is_empty() {
        say(&mut errs, "no `good` fixture; 042 §5 requires one".into());
    }
    if r.bad.is_empty() {
        say(&mut errs, "no `bad` fixture; 042 §5 requires one".into());
    }
    if errs.is_empty() { Ok(r) } else { Err(errs) }
}

struct Parser<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Parser { s, i: 0 }
    }

    fn skip_ws(&mut self) {
        while self.i < self.s.len() {
            let rest = &self.s[self.i..];
            if let Some(n) = rest.find(|c: char| !c.is_whitespace()) {
                self.i += n;
            } else {
                self.i = self.s.len();
                return;
            }
            if self.s[self.i..].starts_with("//") {
                match self.s[self.i..].find('\n') {
                    Some(n) => self.i += n,
                    None => self.i = self.s.len(),
                }
            } else {
                return;
            }
        }
    }

    /// `recipe <name> {`
    fn header(&mut self) -> Result<String, Vec<String>> {
        let bad = || vec!["not a recipe: expected `recipe <name> {`".to_string()];
        self.skip_ws();
        if !self.s[self.i..].starts_with("recipe") {
            return Err(bad());
        }
        self.i += "recipe".len();
        let Some(name) = self.word() else {
            return Err(bad());
        };
        self.skip_ws();
        if !self.s[self.i..].starts_with('{') {
            return Err(bad());
        }
        self.i += 1;
        Ok(name)
    }

    fn word(&mut self) -> Option<String> {
        self.skip_ws();
        let rest = &self.s[self.i..];
        let n = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        (n > 0).then(|| {
            self.i += n;
            rest[..n].to_owned()
        })
    }

    /// A double-quoted string. **Interior newlines and their indentation collapse to one
    /// space**: 042 §4 writes a rationale across source lines for readability, and a reader
    /// wants the sentence, not the layout.
    fn string(&mut self) -> Option<String> {
        self.skip_ws();
        let rest = &self.s[self.i..];
        let body = rest.strip_prefix('"')?;
        let end = body.find('"')?;
        self.i += 1 + end + 1;
        let raw = &body[..end];
        let mut out = String::new();
        for (n, line) in raw.split('\n').enumerate() {
            if n > 0 {
                out.push(' ');
                out.push_str(line.trim_start());
            } else {
                out.push_str(line);
            }
        }
        Some(out)
    }

    fn number(&mut self) -> Option<usize> {
        self.skip_ws();
        let rest = &self.s[self.i..];
        let n = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        (n > 0).then(|| {
            self.i += n;
            rest[..n].parse().ok()
        })?
    }

    fn bad_fixture(&mut self) -> Result<BadFixture, String> {
        let path = self.string().ok_or("`fixture bad` needs a path")?;
        self.skip_ws();
        if !self.s[self.i..].starts_with("expect") {
            return Err(format!(
                "`fixture bad \"{path}\"` needs `expect N at \"file:line\"`; \
                 without a location a wrong-location finding cannot be detected (042 c3)"
            ));
        }
        self.i += "expect".len();
        let expect = self.number().ok_or("`expect` needs a count")?;
        self.skip_ws();
        if !self.s[self.i..].starts_with("at") {
            return Err(format!("`expect {expect}` needs `at \"file:line\"`"));
        }
        self.i += "at".len();
        let at = self.string().ok_or("`at` needs a \"file:line\"")?;
        Ok(BadFixture { path, expect, at })
    }

    /// `scope fn $var where <selector>`.
    fn scope_clause(&mut self) -> Result<Scope, String> {
        self.skip_ws();
        if !self.s[self.i..].starts_with("fn") {
            return Err("`scope` expects `fn $var where <selector>`".into());
        }
        self.i += "fn".len();
        self.skip_ws();
        if !self.s[self.i..].starts_with('$') {
            return Err("`scope fn` expects a metavariable like `$f`".into());
        }
        self.i += 1;
        let var = format!("${}", self.word().unwrap_or_default());
        self.skip_ws();
        if !self.s[self.i..].starts_with("where") {
            return Err(format!("`scope fn {var}` expects `where <selector>`"));
        }
        self.i += "where".len();

        let head = self.word().unwrap_or_default();
        // A selector's argument is a bare word or a quoted string depending on the selector;
        // `name matches` is two words, which is why the head is not enough on its own.
        let selector = match head.as_str() {
            "registered_via" => Selector::RegisteredVia(self.word_or_string()?),
            "in_file" => Selector::InFile(self.word_or_string()?),
            "has_attribute" => Selector::HasAttribute(self.word_or_string()?),
            "signature" => Selector::Signature(self.word_or_string()?),
            "calls" => Selector::Calls(self.word_or_string()?),
            "name" => {
                self.skip_ws();
                if !self.s[self.i..].starts_with("matches") {
                    return Err("`name` expects `matches <regex>`".into());
                }
                self.i += "matches".len();
                Selector::NameMatches(self.word_or_string()?)
            }
            // **Not a `_` arm that selects nothing.** An unknown selector that quietly matched
            // no function would report zero violations over the whole tree and read as a clean
            // result; a typo must be a load error instead.
            unknown => {
                return Err(format!(
                    "unknown scope selector `{unknown}`; 042 §4.2 lists registered_via, \
                     in_file, name matches, has_attribute, signature, calls"
                ));
            }
        };
        Ok(Scope { var, selector })
    }

    fn word_or_string(&mut self) -> Result<String, String> {
        if let Some(s) = self.string() {
            return Ok(s);
        }
        self.word()
            .ok_or_else(|| "selector needs an argument".into())
    }

    /// The next clause keyword, or `None` at the recipe's closing brace.
    fn next_keyword(&mut self) -> Option<String> {
        self.skip_ws();
        if self.i >= self.s.len() || self.s[self.i..].starts_with('}') {
            return None;
        }
        self.word()
    }

    /// Consume the rest of a clause this loader does not read: up to the end of the line, or
    /// through a balanced `{ ... }` when the clause opens a block.
    fn clause_body(&mut self) -> String {
        let start = self.i;
        let mut depth = 0usize;
        let mut saw_block = false;
        while self.i < self.s.len() {
            let c = self.s.as_bytes()[self.i];
            match c {
                b'{' => {
                    depth += 1;
                    saw_block = true;
                }
                b'}' => {
                    if depth == 0 {
                        break; // the recipe's own closing brace
                    }
                    depth -= 1;
                    if depth == 0 {
                        self.i += 1;
                        break;
                    }
                }
                b'\n' if depth == 0 => {
                    if saw_block {
                        break;
                    }
                    self.i += 1;
                    break;
                }
                _ => {}
            }
            self.i += 1;
        }
        self.s[start..self.i].trim().to_owned()
    }
}

/// A call graph over function names — the input tier 1 needs to build a candidate set.
///
/// Names, not `DeclId`s: a candidate closure crosses translation units (a registered handler
/// and the helper holding the acquisition are routinely in different files), and an id minted
/// per TU cannot be followed across that boundary.
#[derive(Debug, Default, Clone)]
pub struct CallGraph {
    edges: indexmap::IndexMap<String, Vec<String>>,
}

impl CallGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_call(&mut self, caller: &str, callee: &str) {
        let e = self.edges.entry(caller.to_owned()).or_default();
        if !e.iter().any(|c| c == callee) {
            e.push(callee.to_owned());
        }
    }

    pub fn callees(&self, f: &str) -> &[String] {
        self.edges.get(f).map_or(&[], Vec::as_slice)
    }
}

/// What tier 1 hands to tier 2, and what it had to leave behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidates {
    /// Functions to analyse, in breadth-first order from the scope matches.
    pub escalated: Vec<String>,
    /// The **fringe** the bound declined to follow: functions one edge past the last
    /// escalated level. Deliberately not "everything unexamined" — counting that would mean
    /// walking the whole graph, which is what the bound exists to avoid. The number therefore
    /// understates, and `is_bounded` rather than the count is what must be trusted.
    pub excluded_by_bound: usize,
}

impl Candidates {
    /// **Anything unexamined forces `Bounded`, however it was lost.** 042 §3.1: an earlier
    /// draft counted only unescalated candidates, so a function dropped before escalation was
    /// invisible and the recipe reported "conforms" over a set it never looked at. A function
    /// the bound excluded is exactly as unexamined as one never escalated.
    pub fn is_bounded(&self) -> bool {
        self.excluded_by_bound > 0
    }
}

/// The tier-1 candidate set: the **transitive callee closure** of the scope matches, bounded
/// by `max_depth` (042 §3.1 default 3).
///
/// Not "in scope *and* contains the acquisition" — that conjunction has a demonstrated recall
/// hole where a registered handler delegates to an unregistered helper, and neither end
/// qualifies.
pub fn candidates(graph: &CallGraph, roots: &[&str], max_depth: usize) -> Candidates {
    let mut seen: indexmap::IndexSet<String> = roots.iter().map(|r| (*r).to_owned()).collect();
    let mut escalated: Vec<String> = seen.iter().cloned().collect();
    let mut frontier: Vec<String> = escalated.clone();
    let mut excluded: indexmap::IndexSet<String> = indexmap::IndexSet::new();

    for depth in 0..usize::MAX {
        if frontier.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for f in &frontier {
            for callee in graph.callees(f) {
                if seen.contains(callee) {
                    continue;
                }
                if depth >= max_depth {
                    // Past the bound. Recorded, not dropped — and recorded in a set, so a
                    // function reachable by two paths is one unexamined function, not two.
                    excluded.insert(callee.clone());
                    continue;
                }
                seen.insert(callee.clone());
                escalated.push(callee.clone());
                next.push(callee.clone());
            }
        }
        frontier = next;
    }

    // **No filtering against `seen` here, because nothing could ever be filtered.** `depth` is
    // uniform across a BFS level and only increases, `seen` grows only while `depth <
    // max_depth`, and `excluded` fills only once `depth >= max_depth` — so a name cannot move
    // from excluded to escalated. An earlier version filtered anyway and its commit message
    // described the case it handled; mutation showed the line was unreachable.
    let excluded_by_bound = excluded.len();
    Candidates {
        escalated,
        excluded_by_bound,
    }
}
