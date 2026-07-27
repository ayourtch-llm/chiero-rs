//! The textual `.cir` format (020 §6).
//!
//! Normative, not a debugging convenience: every M1 core fixture is a `.cir` file, so
//! `print` must canonicalize and `parse` must reject anything it does not understand.
//! Silent tolerance here produces tests that pass by not testing anything.

use crate::*;
use chiero_span::{BytePos, ExpnCtx};
use indexmap::IndexMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based.
    pub line: u32,
    pub message: String,
}

/// Parse a `.cir` module.
///
/// **Unknown constructs are a hard error** (020 §6). Tolerance in a fixture format
/// produces tests that pass by not testing anything, so every unrecognized directive,
/// instruction and type is rejected with the line that caused it.
pub fn parse(src: &str) -> Result<Module, ParseError> {
    // Pre-pass for `@name` resolution, so a function may refer to one declared later.
    let (globals, funcs) = scan_names(src);
    Parser {
        lines: src.lines().collect(),
        at: 0,
        raw: String::new(),
        globals,
        funcs,
        value_names: IndexMap::new(),
        alloca_names: IndexMap::new(),
        label_names: IndexMap::new(),
        tok_hi: std::cell::Cell::new(0),
        tok_base: std::cell::Cell::new(0),
        whole_line: std::cell::Cell::new(false),
        cur_span: Span::DUMMY,
        name_base: 0,
        label_base: 0,
    }
    .module()
}

/// Collect declaration order of `global @n` and `func @n`, which *is* their id order.
fn scan_names(src: &str) -> (Vec<String>, Vec<String>) {
    let (mut g, mut f) = (Vec::new(), Vec::new());
    for line in src.lines() {
        let l = line.split(';').next().unwrap_or("").trim();
        if let Some(rest) = l.strip_prefix("global ") {
            // `const` is optional and was not stripped here, so a const global was
            // parsed (and given an id) but never entered the name table — desyncing the
            // two id spaces and silently resolving `addrglobal` to the wrong global.
            let rest = rest.strip_prefix("const ").unwrap_or(rest);
            g.push(
                rest.trim_start_matches('@')
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string(),
            );
        } else if let Some(rest) = l.strip_prefix("func @") {
            f.push(rest.split(['(', ' ']).next().unwrap_or("").to_string());
        }
    }
    (g, f)
}

struct Parser<'a> {
    lines: Vec<&'a str>,
    at: usize,
    /// The current line with comments stripped. Tokenizing discards commas, which some
    /// constructs (parameter lists, call arguments, switch cases) need as separators.
    raw: String,
    globals: Vec<String>,
    funcs: Vec<String>,
    /// Named values and block labels, per function, numbered in order of first
    /// appearance. 020 §6's own example writes `%len_p` and `bb_ok`, and a fixture
    /// format humans write should not require them to allocate ids by hand. Printing
    /// canonicalizes back to `%N`/`bbN`, consistent with printing being canonicalization.
    value_names: IndexMap<String, u32>,
    /// Allocas are a **separate** id space from values. Sharing `value_names` with an
    /// `"alloca:"` prefix meant a named alloca could be declared but never referenced —
    /// `addrlocal %buf` parsed the name as a number and failed — while `store … -> %slot`
    /// minted a fresh, never-defined value instead. 020 §6's own example hit both.
    alloca_names: IndexMap<String, u32>,
    label_names: IndexMap<String, u32>,
    /// One past the largest literal `%N` in the current function, so named values
    /// cannot collide with numeric ones.
    /// High-water mark of token indices read while parsing one instruction, so
    /// trailing junk can be rejected (020 §6) without every arm counting by hand.
    tok_hi: std::cell::Cell<usize>,
    /// Offset of the sub-slice `tok` indices are currently relative to.
    tok_base: std::cell::Cell<usize>,
    /// Set by parsers that read the raw line instead of tokens (calls), for which the
    /// high-water mark under-counts.
    whole_line: std::cell::Cell<bool>,
    /// Span carried by the line most recently returned from `next_line`, recovered
    /// from its trailing comment before the comment is stripped.
    cur_span: Span,
    name_base: u32,
    /// One past the largest literal `bbN`, likewise for labels.
    label_base: u32,
}

/// Split on whitespace and commas, keeping quoted strings intact.
fn toks(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    for c in line.chars() {
        match c {
            '"' => {
                in_str = !in_str;
                cur.push(c);
            }
            _ if in_str => cur.push(c),
            ' ' | '\t' | ',' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

impl<'a> Parser<'a> {
    fn err<T>(&self, msg: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError {
            line: self.at as u32,
            message: msg.into(),
        })
    }

    /// Next non-blank line's tokens, consuming it.
    ///
    fn next_line(&mut self) -> Option<Vec<String>> {
        while self.at < self.lines.len() {
            let raw = self.lines[self.at];
            self.at += 1;
            let body = raw.split(';').next().unwrap_or("").trim();
            if !body.is_empty() {
                self.raw = body.to_string();
                self.cur_span = parse_span_note(raw);
                return Some(toks(body));
            }
        }
        None
    }

    fn module(&mut self) -> Result<Module, ParseError> {
        let mut m = Module::default();
        while let Some(t) = self.next_line() {
            if t.is_empty() {
                return self.err("empty line after tokenizing");
            }
            match self.tok(&t, 0)? {
                "target" => {}
                "global" => {
                    let mut g = self.global(&t)?;
                    g.id = GlobalId(m.globals.len() as u32);
                    m.globals.push(g);
                }
                "func" => {
                    let f = self.func(&t, m.funcs.len() as u32)?;
                    m.funcs.push(f);
                }
                other => return self.err(format!("unknown directive `{other}`")),
            }
        }
        Ok(m)
    }

    fn global(&mut self, t: &[String]) -> Result<Global, ParseError> {
        // global [const] @name : size N align N
        let is_const = t.get(1).map(String::as_str) == Some("const");
        let o = usize::from(is_const);
        Ok(Global {
            id: GlobalId(0),
            name: self.tok(t, 1 + o)?.trim_start_matches('@').into(),
            size: self
                .tok(t, 4 + o)?
                .parse()
                .map_err(|_| self.perr("bad size"))?,
            align: self
                .tok(t, 6 + o)?
                .parse()
                .map_err(|_| self.perr("bad align"))?,
            is_const,
            span: self.cur_span,
        })
    }

    /// `%N` is literal; `%name` is assigned an id on first appearance.
    fn value_id(&mut self, s: &str) -> Result<ValueId, ParseError> {
        let n = s.trim_start_matches('%').trim_end_matches(',');
        if let Ok(v) = n.parse::<u32>() {
            return Ok(ValueId(v));
        }
        if n.is_empty() {
            return Err(self.perr("empty value name"));
        }
        // Named ids start above every literal `%N` in the function. A counter over
        // *named* values alone made `%1` and a later `%b` the same id — silently a
        // different program than the text says. 020 §6 permits mixing the two forms.
        let next = self.name_base + self.value_names.len() as u32;
        Ok(ValueId(
            *self.value_names.entry(n.to_string()).or_insert(next),
        ))
    }

    /// `%N` is literal; `%name` is assigned an alloca id on first appearance.
    fn alloca_id(&mut self, s: &str) -> Result<AllocaId, ParseError> {
        let n = s.trim_start_matches('%').trim_end_matches(',');
        if let Ok(v) = n.parse::<u32>() {
            return Ok(AllocaId(v));
        }
        if n.is_empty() {
            return Err(self.perr("empty alloca name"));
        }
        // Named allocas start above every literal `%N` in the function, for the same
        // reason values do: without it `%buf` and a literal `%0` alloca are the same
        // object. `name_base` bounds every literal `%N`, whichever id space it names.
        let next = self.name_base + self.alloca_names.len() as u32;
        Ok(AllocaId(
            *self.alloca_names.entry(n.to_string()).or_insert(next),
        ))
    }

    /// Look ahead over the current function body for the largest literal `%N` and
    /// `bbN`, returning one past each. Named ids are allocated above them.
    fn scan_literal_ids(&self) -> (u32, u32) {
        let (mut v, mut b) = (0u32, 1u32);
        let mut depth = 0i32;
        for line in &self.lines[self.at..] {
            let l = line.split(';').next().unwrap_or("").trim();
            if l == "}" {
                depth -= 1;
                if depth < 0 {
                    break;
                }
            }
            if l.ends_with('{') {
                depth += 1;
            }
            for tok in l.split(|c: char| !(c.is_alphanumeric() || c == '%' || c == '_')) {
                if let Some(n) = tok.strip_prefix('%')
                    && let Ok(k) = n.parse::<u32>()
                {
                    v = v.max(k + 1);
                }
                if let Some(n) = tok.strip_prefix("bb")
                    && let Ok(k) = n.parse::<u32>()
                {
                    b = b.max(k + 1);
                }
            }
        }
        (v, b)
    }

    fn label_of(&mut self, s: &str) -> Result<BlockId, ParseError> {
        let n = s.trim_end_matches([',', ':']);
        if n == "entry" {
            return Ok(BlockId(0));
        }
        if let Some(rest) = n.strip_prefix("bb")
            && let Ok(v) = rest.parse::<u32>()
        {
            return Ok(BlockId(v));
        }
        // A symbolic label. Reserve 0 for `entry`, so named blocks start at 1.
        // Allocated above every literal `bbN` in the function, or a branch to an
        // *undefined* label fabricates an in-range id that aliases a real block —
        // and rule 2 cannot catch it, because the id exists.
        let next = self.label_base + self.label_names.len() as u32;
        Ok(BlockId(
            *self.label_names.entry(n.to_string()).or_insert(next),
        ))
    }

    fn global_id(&self, s: &str) -> Result<GlobalId, ParseError> {
        let n = s.trim_start_matches('@');
        self.globals
            .iter()
            .position(|x| x == n)
            .map(|i| GlobalId(i as u32))
            .ok_or_else(|| self.perr(&format!("unknown global `{s}`")))
    }

    fn func_id(&self, s: &str) -> Result<FuncId, ParseError> {
        let n = s.trim_start_matches('@');
        self.funcs
            .iter()
            .position(|x| x == n)
            .map(|i| FuncId(i as u32))
            .ok_or_else(|| self.perr(&format!("unknown function `{s}`")))
    }

    /// Checked token access. The parser indexed positionally in ~20 places, so a
    /// truncated line panicked with an index-out-of-bounds instead of returning a
    /// `ParseError` — and a fixture format whose parser panics on malformed input is
    /// strictly worse than one that errors, since a panic carries no line number.
    fn tok<'t>(&self, t: &'t [String], i: usize) -> Result<&'t str, ParseError> {
        self.tok_hi
            .set(self.tok_hi.get().max(self.tok_base.get() + i + 1));
        t.get(i)
            .map(String::as_str)
            .ok_or_else(|| self.perr(&format!("line is too short: expected a token at {i}")))
    }

    fn perr(&self, m: &str) -> ParseError {
        ParseError {
            line: self.at as u32,
            message: m.to_string(),
        }
    }

    /// `func @name(%0: ty, ...) -> ty` optionally followed by ` {` and a body.
    fn func(&mut self, t: &[String], id: u32) -> Result<Function, ParseError> {
        let _ = t;
        let joined = self.raw.clone();
        let open = joined.find('(').ok_or_else(|| self.perr("func needs ("))?;
        let close = joined.rfind(')').ok_or_else(|| self.perr("func needs )"))?;
        // `joined[5..open]` panics on a reversed range for input like `func(x)`.
        if open < 5 || close < open {
            return self.err("malformed func header");
        }
        let name: Symbol = joined[5..open].trim().trim_start_matches('@').into();

        let mut params = Vec::new();
        let mut pending_names: IndexMap<String, u32> = IndexMap::new();
        let mut variadic = false;
        let inner = joined[open + 1..close].trim();
        if !inner.is_empty() {
            for p in inner.split(',') {
                if p.trim() == "..." {
                    variadic = true;
                    continue;
                }
                let (v, ty_s) = p
                    .split_once(':')
                    .ok_or_else(|| self.perr("param needs :"))?;
                let vname = v.trim().to_string();
                let ty = self.ty(ty_s.trim())?;
                // Parameters are the first values in the function, so they must be
                // interned before the body, or `%v` in the body would get a fresh id.
                let value = ValueId(match vname.trim_start_matches('%').parse::<u32>() {
                    Ok(n) => n,
                    Err(_) => {
                        let next = pending_names.len() as u32;
                        *pending_names
                            .entry(vname.trim_start_matches('%').to_string())
                            .or_insert(next)
                    }
                });
                params.push(Param { value, ty });
            }
        }

        let tail = joined[close + 1..].trim();
        let has_body = tail.ends_with('{');
        let tail = tail.trim_start_matches("->").trim_end_matches('{').trim();
        // The return type is the first token; the rest are attributes.
        let mut it = tail.split_whitespace();
        let ret = self.ty(it.next().unwrap_or("void"))?;
        let mut attrs = FnAttrs::default();
        let rest: Vec<&str> = it.collect();
        let mut i = 0;
        while i < rest.len() {
            match rest[i] {
                "noreturn" => attrs.noreturn = true,
                "pure" => attrs.no_side_effects = true,
                "order_sensitive" => attrs.order_sensitive = true,
                "march" => {
                    i += 1;
                    let v = rest.get(i).ok_or_else(|| self.perr("march needs a name"))?;
                    attrs.march_variant = Some(v.trim_matches('"').into());
                }
                other => return self.err(format!("unknown function attribute `{other}`")),
            }
            i += 1;
        }

        let mut f = Function {
            id: FuncId(id),
            name,
            params,
            ret,
            variadic,
            allocas: Vec::new(),
            blocks: Vec::new(),
            entry: BlockId(0),
            attrs,
            body: if has_body {
                Body::Defined
            } else {
                Body::Declared
            },
            span: self.cur_span,
        };
        if has_body {
            // Names are function-scoped: `%tmp` in two functions is two values, and an
            // alloca named `%buf` in one is not the alloca in another.
            self.alloca_names.clear();
            self.label_names.clear();
            let (vb, lb) = self.scan_literal_ids();
            // Named parameters were interned at 0..k-1 while parsing the signature,
            // *before* the literal scan could run — so a body's `%0` silently resolved
            // to the first parameter instead of being a distinct value. Rebase them
            // above the literals now that the bound is known.
            let named: Vec<u32> = pending_names.values().copied().collect();
            self.value_names = pending_names
                .into_iter()
                .map(|(n, old)| (n, vb + old))
                .collect();
            // Only the *named* parameters move. A numerically-spelled `%0: i32` is a
            // literal id like any other and must stay exactly where the text put it.
            for p in &mut f.params {
                if named.contains(&p.value.0) {
                    p.value = ValueId(vb + p.value.0);
                }
            }
            self.name_base = vb + self.value_names.len() as u32;
            self.label_base = lb;
            self.body(&mut f)?;
        }
        Ok(f)
    }

    fn ty(&self, s: &str) -> Result<CTy, ParseError> {
        Ok(match s {
            "void" => CTy::Void,
            "ptr" => CTy::Ptr,
            "f32" => CTy::Float(FloatKind::F32),
            "f64" => CTy::Float(FloatKind::F64),
            "f80" => CTy::Float(FloatKind::X87_80),
            _ if s.starts_with('i') => CTy::Int(
                s[1..]
                    .parse()
                    .map_err(|_| self.perr(&format!("unknown type `{s}`")))?,
            ),
            _ if s.starts_with('<') && s.ends_with('>') => {
                let inner = &s[1..s.len() - 1];
                let (n, e) = inner
                    .split_once('x')
                    .ok_or_else(|| self.perr("vector needs `NxTy`"))?;
                CTy::Vector {
                    elem: Box::new(self.ty(e.trim())?),
                    lanes: n.trim().parse().map_err(|_| self.perr("bad lane count"))?,
                }
            }
            _ => return Err(self.perr(&format!("unknown type `{s}`"))),
        })
    }

    fn body(&mut self, f: &mut Function) -> Result<(), ParseError> {
        let mut cur: Option<Block> = None;
        // A block is only complete once a terminator line is seen. Defaulting to
        // `Unreachable` would silently accept a truncated block.
        let mut terminated = false;
        let mut labels: Vec<(String, BlockId)> = Vec::new();
        let mut pending: Vec<(usize, String)> = Vec::new(); // block index -> label refs

        loop {
            let Some(t) = self.next_line() else {
                return self.err("unterminated function");
            };
            if t.is_empty() {
                return self.err("empty line after tokenizing");
            }
            let head = self.tok(&t, 0)?;

            if head == "}" {
                if cur.is_some() && !terminated {
                    return self.err("block has no terminator");
                }
                break;
            }
            if head == ".entry" {
                let l = self.tok(&t, 1)?.to_string();
                f.entry = self.label_of(&l)?;
                continue;
            }
            if head == "alloca" {
                f.allocas.push(self.alloca(&t)?);
                continue;
            }
            if head.ends_with(':') && t.len() == 1 {
                if cur.is_some() && !terminated {
                    return self.err("block has no terminator");
                }
                terminated = false;
                let label = head.trim_end_matches(':').to_string();
                let id = self.label_of(&label)?;
                if labels.iter().any(|(_, seen)| *seen == id) {
                    return self.err(format!("duplicate block label `{label}`"));
                }
                labels.push((label, id));
                cur = Some(Block {
                    id,
                    insts: Vec::new(),
                    term: Terminator::Unreachable(UnreachableReason::LoweringGap),
                    gcov_lines: Default::default(),
                    span: self.cur_span,
                });
                continue;
            }

            let Some(b) = cur.as_mut() else {
                return self.err("instruction outside a block");
            };

            if head == ".line" {
                let l: u32 = self
                    .tok(&t, 1)?
                    .parse()
                    .map_err(|_| self.perr("bad line"))?;
                if !b.gcov_lines.contains(&l) {
                    b.gcov_lines.push(l);
                }
                // Deduplicate but **preserve order**. Sorting made
                // `parse(print(m)) != m` for any lowered module whose lines are not
                // already ascending, which is most of them once blocks are merged.
                continue;
            }
            if let Some(term) = self.terminator(&t, &mut pending, f.blocks.len())? {
                b.term = term;
                let done = cur.take().expect("block");
                f.blocks.push(done);
                terminated = true;
                continue;
            }
            self.tok_hi.set(0);
            self.tok_base.set(0);
            self.whole_line.set(false);
            // Captured before `inst()` runs: `call_parts` re-reads the raw line and
            // several arms consume further lines, either of which moves `cur_span`.
            let inst_span = self.cur_span;
            let inst = self.inst(&t)?;
            // 020 §6: unknown input is a hard parse error. The rule was enforced for
            // mnemonics but not operands — every arm indexes fixed token positions and
            // dropped the rest, which is how `fresh i32 "input"` silently lost its
            // reason string. Anything the arm did not read is junk.
            if !self.whole_line.get() && self.tok_hi.get() < t.len() {
                return self.err(format!(
                    "unexpected trailing token `{}`",
                    t[self.tok_hi.get()]
                ));
            }
            b.insts.push(Inst {
                kind: inst,
                span: inst_span,
            });
        }

        if f.blocks.is_empty() {
            return self.err("function body has no blocks");
        }
        // A symbolic label that was branched to but never defined would otherwise
        // survive as a fabricated id pointing at nothing.
        let defined: Vec<BlockId> = f.blocks.iter().map(|b| b.id).collect();
        for (name, id) in &self.label_names {
            if !defined.contains(&BlockId(*id)) {
                return self.err(format!("branch to undefined label `{name}`"));
            }
        }
        Ok(())
    }

    fn alloca(&mut self, t: &[String]) -> Result<AllocaDecl, ParseError> {
        // alloca %N : ty x COUNT align A [scope S] [lifetime L] ["name"]
        //
        // `scope` and `lifetime` are optional and default to scope 0 / Scope, so a
        // fixture that does not care about stack lifetime need not say so.
        let get = |i: usize| t.get(i).map(String::as_str).unwrap_or("");
        let named = get(1).to_string();
        let id = self.alloca_id(&named)?;
        let find = |kw: &str| {
            t.iter()
                .position(|x| x == kw)
                .and_then(|i| t.get(i + 1))
                .map(String::as_str)
        };
        Ok(AllocaDecl {
            id,
            ty: self.ty(get(3))?,
            count: get(5).parse().map_err(|_| self.perr("bad alloca count"))?,
            align: get(7).parse().map_err(|_| self.perr("bad alloca align"))?,
            scope: ScopeId(
                find("scope")
                    .unwrap_or("0")
                    .parse()
                    .map_err(|_| self.perr("bad scope"))?,
            ),
            lifetime: match find("lifetime").unwrap_or("scope") {
                "scope" => Lifetime::Scope,
                "function" => Lifetime::Function,
                other => return self.err(format!("unknown lifetime `{other}`")),
            },
            name: t
                .iter()
                .find(|x| x.starts_with('"'))
                .map(|n| n.trim_matches('"').into()),
            span: self.cur_span,
        })
    }

    fn label_id(&mut self, s: &str) -> Result<BlockId, ParseError> {
        self.label_of(s)
    }

    fn operand(&mut self, s: &str) -> Result<Operand, ParseError> {
        let s = s.trim_end_matches(',');
        if s.starts_with('%') {
            // An alloca name is **not** an operand. CIR is three-address, so a stack
            // slot's address is produced by `addrlocal` and used as a value; accepting
            // the name directly minted a never-defined ValueId, which is how 020 §6's
            // example parsed into a module that then failed to verify.
            let n = s.trim_start_matches('%');
            if self.alloca_names.contains_key(n) {
                return Err(self.perr(&format!(
                    "`%{n}` names an alloca, not a value; use `addrlocal %{n}` first"
                )));
            }
            return Ok(Operand::Value(self.value_id(s)?));
        }
        if s == "null" {
            return Ok(Operand::Const(Const::Null));
        }
        if let Some(t) = s.strip_prefix("undef:") {
            return Ok(Operand::Const(Const::Undef(self.ty(t)?)));
        }
        if let Some(rest) = s.strip_prefix("globaladdr:") {
            let (n, off) = rest
                .rsplit_once(':')
                .ok_or_else(|| self.perr("globaladdr needs :offset"))?;
            return Ok(Operand::Const(Const::GlobalAddr {
                g: self.global_id(n)?,
                off: off.parse().map_err(|_| self.perr("bad offset"))?,
            }));
        }
        if let Some(n) = s.strip_prefix("funcaddr:") {
            return Ok(Operand::Const(Const::FuncAddr(self.func_id(n)?)));
        }
        if let Some(rest) = s.strip_prefix("wide:") {
            let (w, hex) = rest
                .split_once(':')
                .ok_or_else(|| self.perr("wide needs :hex"))?;
            let bits: u32 = w
                .trim_start_matches('i')
                .parse()
                .map_err(|_| self.perr("bad wide width"))?;
            let mut words = Vec::new();
            let mut r = hex.trim_start_matches("0x");
            while !r.is_empty() {
                let cut = r.len().saturating_sub(16);
                let (head, tail) = r.split_at(cut);
                words.push(u64::from_str_radix(tail, 16).map_err(|_| self.perr("bad limb"))?);
                r = head;
            }
            return Ok(Operand::Const(Const::Wide { bits, words }));
        }
        if let Some(rest) = s.strip_prefix("fconst:") {
            let (k, hex) = rest
                .split_once(':')
                .ok_or_else(|| self.perr("fconst needs :bits"))?;
            let kind = match k {
                "f32" => FloatKind::F32,
                "f64" => FloatKind::F64,
                "f80" => FloatKind::X87_80,
                o => return Err(self.perr(&format!("unknown float kind `{o}`"))),
            };
            let bits = u64::from_str_radix(hex.trim_start_matches("0x"), 16)
                .map_err(|_| self.perr("bad float bits"))?;
            return Ok(Operand::Const(Const::Float(kind, bits)));
        }
        if s.starts_with('@') {
            return Ok(Operand::Const(Const::FuncAddr(self.func_id(s)?)));
        }
        // Integer literals are `<value>i<bits>`.
        if let Some(i) = s.rfind('i')
            && i > 0
        {
            let (v, b) = s.split_at(i);
            if let (Ok(val), Ok(bits)) = (v.parse::<i128>(), b[1..].parse::<u32>()) {
                return Ok(Operand::Const(Const::Int { bits, val }));
            }
        }
        Err(self.perr(&format!("unknown operand `{s}`")))
    }

    fn terminator(
        &mut self,
        t: &[String],
        _pending: &mut Vec<(usize, String)>,
        _idx: usize,
    ) -> Result<Option<Terminator>, ParseError> {
        Ok(Some(match self.tok(t, 0)? {
            "ret" => Terminator::Return(match t.get(1) {
                Some(v) => Some(self.operand(v)?),
                None => None,
            }),
            "goto" => Terminator::Goto(self.label_id(self.tok(t, 1)?)?),
            "br" => Terminator::Br {
                cond: self.operand(self.tok(t, 1)?)?,
                t: self.label_id(self.tok(t, 2)?)?,
                f: self.label_id(self.tok(t, 3)?)?,
            },
            "switch" => {
                let ty = self.ty(self.tok(t, 1)?)?;
                let scrut = self.operand(self.tok(t, 2)?)?;
                // [v -> bb, ...], default bb
                let joined = self.raw.clone();
                let open = joined
                    .find('[')
                    .ok_or_else(|| self.perr("switch needs ["))?;
                let close = joined
                    .find(']')
                    .ok_or_else(|| self.perr("switch needs ]"))?;
                let mut cases = Vec::new();
                let body = joined[open + 1..close].trim();
                if !body.is_empty() {
                    for c in body.split(',') {
                        let (v, b) = c
                            .split_once("->")
                            .ok_or_else(|| self.perr("switch case needs ->"))?;
                        cases.push((
                            v.trim().parse().map_err(|_| self.perr("bad case value"))?,
                            self.label_id(b.trim())?,
                        ));
                    }
                }
                let dflt = joined[close + 1..]
                    .trim()
                    .trim_start_matches(',')
                    .trim()
                    .trim_start_matches("default")
                    .trim();
                Terminator::Switch {
                    scrut,
                    ty,
                    cases,
                    default: self.label_id(dflt)?,
                }
            }
            "unreachable" => Terminator::Unreachable(match t.get(1).map(String::as_str) {
                Some("noreturn") => UnreachableReason::AfterNoreturn,
                Some("exhaustive") => UnreachableReason::ExhaustiveSwitch,
                Some("builtin") => UnreachableReason::BuiltinUnreachable,
                Some("gap") | None => UnreachableReason::LoweringGap,
                Some(o) => return self.err(format!("unknown unreachable reason `{o}`")),
            }),
            "indirectgoto" => {
                let joined = self.raw.clone();
                let open = joined.find('[').ok_or_else(|| self.perr("needs ["))?;
                let close = joined.find(']').ok_or_else(|| self.perr("needs ]"))?;
                let targets: Result<Vec<BlockId>, _> = joined[open + 1..close]
                    .split(',')
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| self.label_id(s.trim()))
                    .collect();
                Terminator::IndirectGoto {
                    addr: self.operand(self.tok(t, 1)?)?,
                    targets: targets?,
                }
            }
            _ => return Ok(None),
        }))
    }

    fn inst(&mut self, t: &[String]) -> Result<InstKind, ParseError> {
        // Markers.
        match self.tok(t, 0)? {
            ".seqpoint" => return Ok(InstKind::Marker(MarkerKind::SeqPoint)),
            ".scope" => {
                let kind = match self.tok(t, 1)? {
                    "enter" => ScopeKind::Enter,
                    "exit" => ScopeKind::Exit,
                    o => return self.err(format!("unknown scope event `{o}`")),
                };
                return Ok(InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                    scope: ScopeId(
                        self.tok(t, 2)?
                            .parse()
                            .map_err(|_| self.perr("bad scope"))?,
                    ),
                    kind,
                })));
            }
            ".label" => {
                return Ok(InstKind::Marker(MarkerKind::Label(
                    self.tok(t, 1)?.trim_matches('"').into(),
                )));
            }
            d if d.starts_with('.') => {
                return self.err(format!("unknown directive `{d}`"));
            }
            _ => {}
        }

        // `%N = <rvalue>` or a bare effectful instruction.
        if t.len() > 2 && t[1] == "=" {
            let dst = {
                let d = self.tok(t, 0)?.to_string();
                self.value_id(&d)?
            };
            let rest = &t[2..];
            self.tok_base.set(2);
            if self.tok(rest, 0)? == "call" {
                let (callee, args) = self.call_parts(rest)?;
                return Ok(InstKind::Call {
                    dst: Some(dst),
                    callee,
                    args,
                });
            }
            if self.tok(rest, 0)? == "allocadyn" {
                return Ok(InstKind::AllocaDyn {
                    dst,
                    alloca: {
                        let n = self.tok(rest, 1)?.to_string();
                        self.alloca_id(&n)?
                    },
                    elem: self.ty(self.tok(rest, 3)?)?,
                    count: self.operand(self.tok(rest, 5)?)?,
                    align: self
                        .tok(rest, 7)?
                        .parse()
                        .map_err(|_| self.perr("bad align"))?,
                });
            }
            if self.tok(rest, 0)? == "vaarg" {
                return Ok(InstKind::VaArg {
                    dst,
                    list: self.operand(self.tok(rest, 1)?)?,
                    ty: self.ty(self.tok(rest, 2)?)?,
                });
            }
            return Ok(InstKind::Assign {
                dst,
                rv: self.rvalue(rest)?,
            });
        }

        match self.tok(t, 0)? {
            "store" | "storevolatile" => Ok(InstKind::Store {
                ty: self.ty(self.tok(t, 1)?)?,
                val: self.operand(self.tok(t, 2)?)?,
                addr: self.operand(self.tok(t, 4)?)?,
                align: self
                    .tok(t, 6)?
                    .parse()
                    .map_err(|_| self.perr("bad align"))?,
                vol: if t[0] == "storevolatile" {
                    Volatility::Volatile
                } else {
                    Volatility::Normal
                },
            }),
            "storebits" => {
                let (off, width) = self.bits(self.tok(t, 6)?)?;
                Ok(InstKind::StoreBits {
                    unit: self.ty(self.tok(t, 1)?)?,
                    val: self.operand(self.tok(t, 2)?)?,
                    addr: self.operand(self.tok(t, 4)?)?,
                    bits: BitRange { off, width },
                    align: self
                        .tok(t, 8)?
                        .parse()
                        .map_err(|_| self.perr("bad align"))?,
                })
            }
            "copymem" => Ok(InstKind::CopyMem {
                dst: self.operand(self.tok(t, 1)?)?,
                src: self.operand(self.tok(t, 3)?)?,
                size: self.operand(self.tok(t, 4)?)?,
                align: self
                    .tok(t, 6)?
                    .parse()
                    .map_err(|_| self.perr("bad align"))?,
            }),
            "setmem" => Ok(InstKind::SetMem {
                dst: self.operand(self.tok(t, 1)?)?,
                byte: self.operand(self.tok(t, 2)?)?,
                size: self.operand(self.tok(t, 3)?)?,
            }),
            "call" => {
                let (callee, args) = self.call_parts(t)?;
                Ok(InstKind::Call {
                    dst: None,
                    callee,
                    args,
                })
            }
            "vastart" => Ok(InstKind::VaStart {
                list: self.operand(self.tok(t, 1)?)?,
            }),
            "vaend" => Ok(InstKind::VaEnd {
                list: self.operand(self.tok(t, 1)?)?,
            }),
            "vacopy" => Ok(InstKind::VaCopy {
                src: self.operand(self.tok(t, 1)?)?,
                dst: self.operand(self.tok(t, 3)?)?,
            }),
            other => self.err(format!("unknown instruction `{other}`")),
        }
    }

    fn bits(&self, s: &str) -> Result<(u32, u32), ParseError> {
        let (a, b) = s
            .split_once("..")
            .ok_or_else(|| self.perr("bits need a..b"))?;
        let off: u32 = a.parse().map_err(|_| self.perr("bad bit offset"))?;
        let hi: u32 = b.parse().map_err(|_| self.perr("bad bit end"))?;
        Ok((off, hi.saturating_sub(off)))
    }

    fn call_parts(&mut self, t: &[String]) -> Result<(Callee, Vec<Operand>), ParseError> {
        let _ = t;
        // Reads the raw line rather than tokens, so the high-water mark cannot see how
        // far it got. A call consumes through its closing paren, which ends the line.
        self.whole_line.set(true);
        // Everything from `call` onward on the raw line, so commas survive.
        let joined = match self.raw.find("call ") {
            Some(i) => self.raw[i..].to_string(),
            None => self.raw.clone(),
        };
        let open = joined.find('(').ok_or_else(|| self.perr("call needs ("))?;
        let close = joined.rfind(')').ok_or_else(|| self.perr("call needs )"))?;
        let target = joined[..open].trim().trim_start_matches("call").trim();
        let callee = if target.starts_with('@') {
            Callee::Direct(self.func_id(target)?)
        } else {
            Callee::Indirect(self.operand(target)?)
        };
        let inner = joined[open + 1..close].trim();
        let args = if inner.is_empty() {
            Vec::new()
        } else {
            {
                let parts: Vec<String> = inner.split(',').map(|a| a.trim().to_string()).collect();
                let mut v = Vec::with_capacity(parts.len());
                for a in parts {
                    v.push(self.operand(&a)?);
                }
                v
            }
        };
        Ok((callee, args))
    }

    fn rvalue(&mut self, t: &[String]) -> Result<RValue, ParseError> {
        // A local high-water mark, merged into the parser's on the way out. It cannot
        // borrow `self.tok_hi` directly because the arms need `&mut self`.
        let hi = std::cell::Cell::new(0usize);
        let g = |i: usize| {
            hi.set(hi.get().max(i + 1));
            t.get(i).map(String::as_str).unwrap_or("")
        };
        let r = match g(0) {
            "load" | "loadvolatile" => RValue::Load {
                ty: self.ty(g(1))?,
                addr: self.operand(g(2))?,
                align: g(4).parse().map_err(|_| self.perr("bad align"))?,
                vol: if g(0) == "loadvolatile" {
                    Volatility::Volatile
                } else {
                    Volatility::Normal
                },
            },
            "loadbits" => {
                let (off, width) = self.bits(g(4))?;
                // Scanned across the whole operand list, so nothing after it is junk.
                let signed = t.iter().any(|x| x == "signed");
                hi.set(t.len());
                RValue::LoadBits {
                    unit: self.ty(g(1))?,
                    addr: self.operand(g(2))?,
                    bits: BitRange { off, width },
                    signed,
                    align: t
                        .last()
                        .and_then(|x| x.parse().ok())
                        .ok_or_else(|| self.perr("bad align"))?,
                }
            }
            "cmp" => RValue::Cmp {
                op: match g(1) {
                    "eq" => CmpOp::Eq,
                    "ne" => CmpOp::Ne,
                    "ult" => CmpOp::ULt,
                    "ule" => CmpOp::ULe,
                    "ugt" => CmpOp::UGt,
                    "uge" => CmpOp::UGe,
                    "slt" => CmpOp::SLt,
                    "sle" => CmpOp::SLe,
                    "sgt" => CmpOp::SGt,
                    "sge" => CmpOp::SGe,
                    "foeq" => CmpOp::FOEq,
                    "fone" => CmpOp::FONe,
                    "folt" => CmpOp::FOLt,
                    "fole" => CmpOp::FOLe,
                    "fueq" => CmpOp::FUEq,
                    "fune" => CmpOp::FUNe,
                    "fult" => CmpOp::FULt,
                    "fule" => CmpOp::FULe,
                    "ford" => CmpOp::FOrd,
                    "funo" => CmpOp::FUno,
                    o => return Err(self.perr(&format!("unknown cmp `{o}`"))),
                },
                ty: self.ty(g(2))?,
                a: self.operand(g(3))?,
                b: self.operand(g(4))?,
            },
            "select" => RValue::Select {
                cond: self.operand(g(1))?,
                t: self.operand(g(2))?,
                f: self.operand(g(3))?,
            },
            "ptradd" => RValue::PtrAdd {
                base: self.operand(g(1))?,
                off: self.operand(g(2))?,
            },
            "addrlocal" => RValue::AddrOfLocal {
                alloca: {
                    let n = g(1).to_string();
                    self.alloca_id(&n)?
                },
            },
            "addrglobal" => RValue::AddrOfGlobal {
                g: self.global_id(g(1))?,
            },
            "addrfunc" => RValue::AddrOfFunc(self.func_id(g(1))?),
            "fresh" => RValue::Fresh { ty: self.ty(g(1))? },
            "shuffle" => {
                // The mask is read off the raw line, through `]` at end of line.
                hi.set(t.len());
                let joined = self.raw.clone();
                let open = joined
                    .find('[')
                    .ok_or_else(|| self.perr("shuffle needs ["))?;
                let close = joined
                    .find(']')
                    .ok_or_else(|| self.perr("shuffle needs ]"))?;
                let mask: Result<Vec<u32>, ParseError> = joined[open + 1..close]
                    .split(',')
                    .filter(|x| !x.trim().is_empty())
                    .map(|x| x.trim().parse().map_err(|_| self.perr("bad shuffle index")))
                    .collect();
                RValue::Shuffle {
                    a: self.operand(g(1))?,
                    b: self.operand(g(2))?,
                    mask: mask?,
                }
            }
            "ptrdiff" => RValue::Bin {
                op: BinOp::PtrDiff {
                    elem_size: g(1).parse().map_err(|_| self.perr("bad elem_size"))?,
                },
                ty: CTy::Ptr,
                a: self.operand(g(2))?,
                b: self.operand(g(3))?,
            },
            "undef" => RValue::Use(Operand::Const(Const::Undef(self.ty(g(1))?))),
            "globaladdr" => RValue::Use(Operand::Const(Const::GlobalAddr {
                g: self.global_id(g(1))?,
                off: g(2).parse().map_err(|_| self.perr("bad offset"))?,
            })),
            "funcaddr" => RValue::Use(Operand::Const(Const::FuncAddr(self.func_id(g(1))?))),
            "wide" => {
                let bits: u32 = g(1)
                    .trim_start_matches('i')
                    .parse()
                    .map_err(|_| self.perr("bad wide width"))?;
                let hex = g(2).trim_start_matches("0x");
                let mut words = Vec::new();
                let mut rest = hex;
                while !rest.is_empty() {
                    let cut = rest.len().saturating_sub(16);
                    let (head, tail) = rest.split_at(cut);
                    words.push(
                        u64::from_str_radix(tail, 16).map_err(|_| self.perr("bad wide limb"))?,
                    );
                    rest = head;
                }
                RValue::Use(Operand::Const(Const::Wide { bits, words }))
            }
            "fconst" => {
                let k = match g(1) {
                    "f32" => FloatKind::F32,
                    "f64" => FloatKind::F64,
                    "f80" => FloatKind::X87_80,
                    o => return Err(self.perr(&format!("unknown float kind `{o}`"))),
                };
                let bits = u64::from_str_radix(g(2).trim_start_matches("0x"), 16)
                    .map_err(|_| self.perr("bad float bits"))?;
                RValue::Use(Operand::Const(Const::Float(k, bits)))
            }
            "splat" => RValue::Splat {
                elem: self.operand(g(1))?,
                lanes: g(2).parse().map_err(|_| self.perr("bad lanes"))?,
            },
            "extractlane" => RValue::ExtractLane {
                v: self.operand(g(1))?,
                lane: g(2).parse().map_err(|_| self.perr("bad lane"))?,
            },
            "insertlane" => RValue::InsertLane {
                v: self.operand(g(1))?,
                lane: g(2).parse().map_err(|_| self.perr("bad lane"))?,
                val: self.operand(g(3))?,
            },
            "neg" | "not" | "fneg" => RValue::Un {
                op: match g(0) {
                    "neg" => UnOp::Neg,
                    "not" => UnOp::Not,
                    _ => UnOp::FNeg,
                },
                ty: self.ty(g(1))?,
                a: self.operand(g(2))?,
            },
            k if CAST_KINDS.iter().any(|(n, _)| *n == k) => {
                let kind = CAST_KINDS.iter().find(|(n, _)| *n == k).unwrap().1;
                RValue::Cast {
                    kind,
                    from: self.ty(g(1))?,
                    a: self.operand(g(2))?,
                    to: self.ty(g(4))?,
                }
            }
            k if BIN_OPS.iter().any(|(n, _)| *n == k) => {
                let op = BIN_OPS.iter().find(|(n, _)| *n == k).unwrap().1;
                RValue::Bin {
                    op,
                    ty: self.ty(g(1))?,
                    a: self.operand(g(2))?,
                    b: self.operand(g(3))?,
                }
            }
            // A bare operand is a `Use`.
            other
                if other.starts_with('%')
                    || other == "null"
                    || other.starts_with("undef:")
                    || other.starts_with("globaladdr:")
                    || other.starts_with("funcaddr:")
                    || other.starts_with("wide:")
                    || other.starts_with("fconst:")
                    || other.contains('i') =>
            {
                RValue::Use(self.operand(other)?)
            }
            other => return Err(self.perr(&format!("unknown instruction `{other}`"))),
        };
        self.tok_hi
            .set(self.tok_hi.get().max(self.tok_base.get() + hi.get()));
        Ok(r)
    }
}

const BIN_OPS: &[(&str, BinOp)] = &[
    ("add", BinOp::Add),
    ("sub", BinOp::Sub),
    ("mul", BinOp::Mul),
    ("udiv", BinOp::UDiv),
    ("sdiv", BinOp::SDiv),
    ("urem", BinOp::URem),
    ("srem", BinOp::SRem),
    ("and", BinOp::And),
    ("or", BinOp::Or),
    ("xor", BinOp::Xor),
    ("shl", BinOp::Shl),
    ("lshr", BinOp::LShr),
    ("ashr", BinOp::AShr),
    ("fadd", BinOp::FAdd),
    ("fsub", BinOp::FSub),
    ("fmul", BinOp::FMul),
    ("fdiv", BinOp::FDiv),
    ("frem", BinOp::FRem),
];

const CAST_KINDS: &[(&str, CastKind)] = &[
    ("trunc", CastKind::Trunc),
    ("zext", CastKind::ZExt),
    ("sext", CastKind::SExt),
    ("fptrunc", CastKind::FpTrunc),
    ("fpext", CastKind::FpExt),
    ("fptoui", CastKind::FpToUi),
    ("fptosi", CastKind::FpToSi),
    ("uitofp", CastKind::UiToFp),
    ("sitofp", CastKind::SiToFp),
    ("ptrtoint", CastKind::PtrToInt),
    ("inttoptr", CastKind::IntToPtr),
    ("bitcast", CastKind::Bitcast),
];

/// Print a module in canonical form.
///
/// Printing **is** canonicalization: two modules that differ only in the whitespace
/// their source used print identically. Without that, "canonical form" is undefined and
/// the byte-exact round trip cannot hold for anything a human typed.
pub fn print(m: &Module) -> String {
    let mut o = String::new();
    o.push_str("target x86_64-unknown-linux-gnu\n");
    for g in &m.globals {
        o.push_str(&format!(
            "\nglobal {}@{} : size {} align {}{}\n",
            if g.is_const { "const " } else { "" },
            g.name,
            g.size,
            g.align,
            span_note(g.span)
        ));
    }
    for f in &m.funcs {
        o.push('\n');
        print_func(m, f, &mut o);
    }
    o
}

/// Names, not numbers. 020 §6's example writes `@counts` and `@vec_add1`, and a fixture
/// format humans write and diff must be readable.
fn gname(m: &Module, g: GlobalId) -> String {
    m.globals
        .get(g.0 as usize)
        .map_or_else(|| format!("@g{}", g.0), |x| format!("@{}", x.name))
}

fn fname(m: &Module, f: FuncId) -> String {
    m.funcs
        .get(f.0 as usize)
        .map_or_else(|| format!("@f{}", f.0), |x| format!("@{}", x.name))
}

fn print_func(m: &Module, f: &Function, o: &mut String) {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| format!("%{}: {}", p.value.0, ty(&p.ty)))
        .collect();
    let mut plist = params.join(", ");
    if f.variadic {
        if plist.is_empty() {
            plist.push_str("...");
        } else {
            plist.push_str(", ...");
        }
    }
    o.push_str(&format!("func @{}({}) -> {}", f.name, plist, ty(&f.ret)));
    let fspan = span_note(f.span);
    // Attributes, in a fixed order so printing stays canonical.
    if f.attrs.noreturn {
        o.push_str(" noreturn");
    }
    if f.attrs.no_side_effects {
        o.push_str(" pure");
    }
    if f.attrs.order_sensitive {
        o.push_str(" order_sensitive");
    }
    if let Some(v) = &f.attrs.march_variant {
        o.push_str(&format!(" march \"{v}\""));
    }
    if f.body == Body::Declared {
        o.push_str(&fspan);
        o.push('\n');
        return;
    }
    o.push_str(&format!(" {{{fspan}\n"));
    // `entry` is only implicit when it is block 0. Hardcoding `BlockId(0)` on parse
    // meant a module whose entry was elsewhere silently started at a different block
    // after a round trip.
    if f.entry.0 != 0 {
        o.push_str(&format!("  .entry {}\n", block_label(f, f.entry)));
    }
    for a in &f.allocas {
        o.push_str(&format!(
            "  alloca %{} : {} x {} align {} scope {} lifetime {}",
            a.id.0,
            ty(&a.ty),
            a.count,
            a.align,
            a.scope.0,
            match a.lifetime {
                Lifetime::Scope => "scope",
                Lifetime::Function => "function",
            }
        ));
        if let Some(n) = &a.name {
            o.push_str(&format!(" \"{n}\""));
        }
        o.push_str(&span_note(a.span));
        o.push('\n');
    }
    for b in &f.blocks {
        o.push_str(&format!("{}:{}\n", block_label(f, b.id), span_note(b.span)));
        for line in &b.gcov_lines {
            o.push_str(&format!("  .line {line}\n"));
        }
        for i in &b.insts {
            o.push_str("  ");
            print_inst(m, &i.kind, o);
            o.push_str(&span_note(i.span));
            o.push('\n');
        }
        o.push_str("  ");
        print_term(f, &b.term, o);
        o.push('\n');
    }
    o.push_str("}\n");
}

/// `entry` is the label for `BlockId(0)` **by id**, `bbN` otherwise.
///
/// Printing `entry` *positionally* — for whichever block happened to be `f.entry` —
/// aliased: a function whose entry is `BlockId(3)` alongside a sibling `BlockId(0)`
/// reparsed into two blocks both numbered 0, because the parser maps `entry` to
/// `BlockId(0)`. Keying the label on the id removes the ambiguity.
/// A span, as the reversible raw triple (020 §6). `Span::DUMMY` prints as nothing, so
/// hand-written fixtures stay clean and the corpus does not grow a comment per line.
fn parse_span_note(raw: &str) -> Span {
    let Some((_, c)) = raw.split_once(';') else {
        return Span::DUMMY;
    };
    let Some(rest) = c.trim().strip_prefix("span ") else {
        return Span::DUMMY;
    };
    let mut it = rest.trim().split(':');
    let (Some(lo), Some(hi), Some(ctx)) = (it.next(), it.next(), it.next()) else {
        return Span::DUMMY;
    };
    match (lo.parse(), hi.parse(), ctx.parse()) {
        (Ok(lo), Ok(hi), Ok(ctx)) => Span {
            lo: BytePos(lo),
            hi: BytePos(hi),
            ctx: ExpnCtx(ctx),
        },
        // A comment that merely starts with `span` is still just a comment.
        _ => Span::DUMMY,
    }
}

fn span_note(sp: Span) -> String {
    if sp == Span::DUMMY {
        String::new()
    } else {
        format!(" ; span {}:{}:{}", sp.lo.0, sp.hi.0, sp.ctx.0)
    }
}

fn block_label(_f: &Function, id: BlockId) -> String {
    if id.0 == 0 {
        "entry".to_string()
    } else {
        format!("bb{}", id.0)
    }
}

fn ty(t: &CTy) -> String {
    match t {
        CTy::Void => "void".into(),
        CTy::Int(b) => format!("i{b}"),
        CTy::Float(FloatKind::F32) => "f32".into(),
        CTy::Float(FloatKind::F64) => "f64".into(),
        CTy::Float(FloatKind::X87_80) => "f80".into(),
        CTy::Ptr => "ptr".into(),
        // No spaces: `toks()` splits on whitespace before `ty()` sees the token, so
        // `<16 x i8>` could never appear in an instruction — which meant no `.cir`
        // fixture could contain a vector at all.
        CTy::Vector { elem, lanes } => format!("<{}x{}>", lanes, ty(elem)),
    }
}

fn op(o: &Operand) -> String {
    match o {
        Operand::Value(v) => format!("%{}", v.0),
        Operand::Const(c) => konst(c),
    }
}

/// Operand printing that can resolve `@names`. `konst` alone printed `@g0`/`@f0` while
/// the parser resolves by name, so a global or function address as an *operand* printed
/// in a form that could not be read back.
fn opm(m: &Module, o: &Operand) -> String {
    match o {
        Operand::Const(Const::GlobalAddr { g, off }) => {
            format!("globaladdr:{}:{off}", gname(m, *g))
        }
        Operand::Const(Const::FuncAddr(f)) => format!("funcaddr:{}", fname(m, *f)),
        _ => op(o),
    }
}

fn konst(c: &Const) -> String {
    match c {
        Const::Int { bits, val } => format!("{val}i{bits}"),
        Const::Wide { bits, words } => {
            let hex: Vec<String> = words.iter().rev().map(|w| format!("{w:016x}")).collect();
            format!("wide:i{bits}:0x{}", hex.join(""))
        }
        Const::Float(k, bits) => format!("fconst:{}:0x{bits:x}", ty(&CTy::Float(*k))),
        Const::Null => "null".into(),
        // Only reachable for a module-less print; `opm` handles the named forms.
        Const::GlobalAddr { g, off } => format!("globaladdr:@g{}:{off}", g.0),
        Const::FuncAddr(f) => format!("funcaddr:@f{}", f.0),
        Const::Undef(t) => format!("undef:{}", ty(t)),
    }
}

fn print_inst(m: &Module, k: &InstKind, o: &mut String) {
    // The module-aware printer, in *every* operand position. `print_rvalue` used it and
    // `print_inst` did not, so a constant as a store value or call argument printed in a
    // form the parser rejects — contract 1 broken for a whole class of operand.
    let op = |x: &Operand| opm(m, x);
    match k {
        InstKind::Assign { dst, rv } => {
            o.push_str(&format!("%{} = ", dst.0));
            print_rvalue(m, rv, o);
        }
        InstKind::Store {
            addr,
            val,
            ty: t,
            align,
            vol,
        } => o.push_str(&format!(
            "{} {} {} -> {} align {align}",
            if *vol == Volatility::Volatile {
                "storevolatile"
            } else {
                "store"
            },
            ty(t),
            op(val),
            op(addr)
        )),
        InstKind::StoreBits {
            addr,
            val,
            unit,
            bits,
            align,
        } => o.push_str(&format!(
            "storebits {} {} -> {} bits {}..{} align {align}",
            ty(unit),
            op(val),
            op(addr),
            bits.off,
            bits.off + bits.width
        )),
        InstKind::CopyMem {
            dst,
            src,
            size,
            align,
        } => o.push_str(&format!(
            "copymem {} -> {}, {} align {align}",
            op(dst),
            op(src),
            op(size)
        )),
        InstKind::SetMem { dst, byte, size } => {
            o.push_str(&format!("setmem {}, {}, {}", op(dst), op(byte), op(size)))
        }
        InstKind::Call { dst, callee, args } => {
            if let Some(d) = dst {
                o.push_str(&format!("%{} = ", d.0));
            }
            let c = match callee {
                Callee::Direct(f) => fname(m, *f),
                Callee::Indirect(x) => op(x),
            };
            let a: Vec<String> = args.iter().map(op).collect();
            o.push_str(&format!("call {c}({})", a.join(", ")));
        }
        InstKind::AllocaDyn {
            dst,
            alloca,
            elem,
            count,
            align,
        } => o.push_str(&format!(
            "%{} = allocadyn %{} : {} x {} align {align}",
            dst.0,
            alloca.0,
            ty(elem),
            op(count)
        )),
        InstKind::VaArg { dst, list, ty: t } => {
            o.push_str(&format!("%{} = vaarg {}, {}", dst.0, op(list), ty(t)))
        }
        InstKind::VaStart { list } => o.push_str(&format!("vastart {}", op(list))),
        InstKind::VaCopy { dst, src } => o.push_str(&format!("vacopy {} -> {}", op(src), op(dst))),
        InstKind::VaEnd { list } => o.push_str(&format!("vaend {}", op(list))),
        InstKind::Marker(m) => match m {
            // `.line` is printed with the block's gcov_lines, not here (015 §5).
            MarkerKind::Line(l) => o.push_str(&format!(".line {l}")),
            MarkerKind::SeqPoint => o.push_str(".seqpoint"),
            MarkerKind::Scope(e) => o.push_str(&format!(
                ".scope {} {}",
                match e.kind {
                    ScopeKind::Enter => "enter",
                    ScopeKind::Exit => "exit",
                },
                e.scope.0
            )),
            MarkerKind::Label(n) => o.push_str(&format!(".label \"{n}\"")),
        },
    }
}

fn print_rvalue(m: &Module, rv: &RValue, o: &mut String) {
    let op = |x: &Operand| opm(m, x);
    match rv {
        RValue::Use(a) => o.push_str(&op(a)),
        RValue::Load {
            addr,
            ty: t,
            align,
            vol,
        } => o.push_str(&format!(
            "{} {}, {} align {align}",
            if *vol == Volatility::Volatile {
                "loadvolatile"
            } else {
                "load"
            },
            ty(t),
            op(addr)
        )),
        RValue::LoadBits {
            addr,
            unit,
            bits,
            signed,
            align,
        } => o.push_str(&format!(
            "loadbits {}, {} bits {}..{}{} align {align}",
            ty(unit),
            op(addr),
            bits.off,
            bits.off + bits.width,
            if *signed { " signed" } else { "" }
        )),
        RValue::Bin {
            op: b,
            a,
            b: rhs,
            ty: t,
        } => {
            // `elem_size` is part of the operator, not decoration: dropping it loses
            // the scale the difference is measured in.
            if let BinOp::PtrDiff { elem_size } = b {
                o.push_str(&format!("ptrdiff {elem_size} {}, {}", op(a), op(rhs)));
            } else {
                o.push_str(&format!("{} {} {}, {}", binop(*b), ty(t), op(a), op(rhs)));
            }
        }
        RValue::Un { op: u, a, ty: t } => o.push_str(&format!(
            "{} {} {}",
            match u {
                UnOp::Neg => "neg",
                UnOp::Not => "not",
                UnOp::FNeg => "fneg",
            },
            ty(t),
            op(a)
        )),
        RValue::Cmp { op: c, a, b, ty: t } => {
            o.push_str(&format!("cmp {} {} {}, {}", cmpop(*c), ty(t), op(a), op(b)))
        }
        RValue::Cast { kind, a, from, to } => o.push_str(&format!(
            "{} {} {} to {}",
            castkind(*kind),
            ty(from),
            op(a),
            ty(to)
        )),
        RValue::Select { cond, t, f } => {
            o.push_str(&format!("select {}, {}, {}", op(cond), op(t), op(f)))
        }
        RValue::PtrAdd { base, off } => o.push_str(&format!("ptradd {}, {}", op(base), op(off))),
        RValue::AddrOfLocal { alloca } => o.push_str(&format!("addrlocal %{}", alloca.0)),
        RValue::AddrOfGlobal { g } => o.push_str(&format!("addrglobal {}", gname(m, *g))),
        RValue::AddrOfFunc(f) => o.push_str(&format!("addrfunc {}", fname(m, *f))),
        RValue::Shuffle { a, b, mask } => {
            let m: Vec<String> = mask.iter().map(|x| x.to_string()).collect();
            o.push_str(&format!("shuffle {}, {}, [{}]", op(a), op(b), m.join(", ")))
        }
        RValue::InsertLane { v, lane, val } => {
            o.push_str(&format!("insertlane {}, {lane}, {}", op(v), op(val)))
        }
        RValue::ExtractLane { v, lane } => o.push_str(&format!("extractlane {}, {lane}", op(v))),
        RValue::Splat { elem, lanes } => o.push_str(&format!("splat {}, {lanes}", op(elem))),
        RValue::Fresh { ty: t } => o.push_str(&format!("fresh {}", ty(t))),
    }
}

fn print_term(f: &Function, t: &Terminator, o: &mut String) {
    match t {
        Terminator::Goto(b) => o.push_str(&format!("goto {}", block_label(f, *b))),
        Terminator::Br { cond, t: tb, f: fb } => o.push_str(&format!(
            "br {}, {}, {}",
            op(cond),
            block_label(f, *tb),
            block_label(f, *fb)
        )),
        Terminator::Switch {
            scrut,
            ty: sty,
            cases,
            default,
        } => {
            let c: Vec<String> = cases
                .iter()
                .map(|(v, b)| format!("{v} -> {}", block_label(f, *b)))
                .collect();
            o.push_str(&format!(
                "switch {} {}, [{}], default {}",
                ty(sty),
                op(scrut),
                c.join(", "),
                block_label(f, *default)
            ));
        }
        Terminator::Return(Some(v)) => o.push_str(&format!("ret {}", op(v))),
        Terminator::Return(None) => o.push_str("ret"),
        Terminator::IndirectGoto { addr, targets } => {
            let t: Vec<String> = targets.iter().map(|b| block_label(f, *b)).collect();
            o.push_str(&format!("indirectgoto {}, [{}]", op(addr), t.join(", ")))
        }
        Terminator::Unreachable(r) => o.push_str(&format!(
            "unreachable {}",
            match r {
                UnreachableReason::AfterNoreturn => "noreturn",
                UnreachableReason::ExhaustiveSwitch => "exhaustive",
                UnreachableReason::BuiltinUnreachable => "builtin",
                UnreachableReason::LoweringGap => "gap",
            }
        )),
    }
}

fn binop(b: BinOp) -> &'static str {
    match b {
        BinOp::Add => "add",
        BinOp::Sub => "sub",
        BinOp::Mul => "mul",
        BinOp::UDiv => "udiv",
        BinOp::SDiv => "sdiv",
        BinOp::URem => "urem",
        BinOp::SRem => "srem",
        BinOp::And => "and",
        BinOp::Or => "or",
        BinOp::Xor => "xor",
        BinOp::Shl => "shl",
        BinOp::LShr => "lshr",
        BinOp::AShr => "ashr",
        BinOp::FAdd => "fadd",
        BinOp::FSub => "fsub",
        BinOp::FMul => "fmul",
        BinOp::FDiv => "fdiv",
        BinOp::FRem => "frem",
        // `elem_size` is part of the operator, not decoration: dropping it loses the
        // scale the difference is measured in.
        BinOp::PtrDiff { .. } => "ptrdiff",
    }
}

fn cmpop(c: CmpOp) -> &'static str {
    match c {
        CmpOp::Eq => "eq",
        CmpOp::Ne => "ne",
        CmpOp::ULt => "ult",
        CmpOp::ULe => "ule",
        CmpOp::UGt => "ugt",
        CmpOp::UGe => "uge",
        CmpOp::SLt => "slt",
        CmpOp::SLe => "sle",
        CmpOp::SGt => "sgt",
        CmpOp::SGe => "sge",
        CmpOp::FOEq => "foeq",
        CmpOp::FONe => "fone",
        CmpOp::FOLt => "folt",
        CmpOp::FOLe => "fole",
        CmpOp::FUEq => "fueq",
        CmpOp::FUNe => "fune",
        CmpOp::FULt => "fult",
        CmpOp::FULe => "fule",
        CmpOp::FOrd => "ford",
        CmpOp::FUno => "funo",
    }
}

fn castkind(k: CastKind) -> &'static str {
    match k {
        CastKind::Trunc => "trunc",
        CastKind::ZExt => "zext",
        CastKind::SExt => "sext",
        CastKind::FpTrunc => "fptrunc",
        CastKind::FpExt => "fpext",
        CastKind::FpToUi => "fptoui",
        CastKind::FpToSi => "fptosi",
        CastKind::UiToFp => "uitofp",
        CastKind::SiToFp => "sitofp",
        CastKind::PtrToInt => "ptrtoint",
        CastKind::IntToPtr => "inttoptr",
        CastKind::Bitcast => "bitcast",
    }
}
