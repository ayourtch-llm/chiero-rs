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
            other => {
                let text = p.clause_body();
                r.unparsed_clauses.push(UnparsedClause {
                    keyword: other.to_owned(),
                    text,
                });
            }
        }
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
