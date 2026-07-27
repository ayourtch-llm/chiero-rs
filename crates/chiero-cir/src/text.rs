//! The textual `.cir` format (020 §6).
//!
//! Normative, not a debugging convenience: every M1 core fixture is a `.cir` file, so
//! `print` must canonicalize and `parse` must reject anything it does not understand.
//! Silent tolerance here produces tests that pass by not testing anything.

use crate::*;

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
    }
    .module()
}

/// Collect declaration order of `global @n` and `func @n`, which *is* their id order.
fn scan_names(src: &str) -> (Vec<String>, Vec<String>) {
    let (mut g, mut f) = (Vec::new(), Vec::new());
    for line in src.lines() {
        let l = line.split(';').next().unwrap_or("").trim();
        if let Some(rest) = l.strip_prefix("global @") {
            g.push(rest.split_whitespace().next().unwrap_or("").to_string());
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
    fn next_line(&mut self) -> Option<Vec<String>> {
        while self.at < self.lines.len() {
            let raw = self.lines[self.at];
            self.at += 1;
            let body = raw.split(';').next().unwrap_or("").trim();
            if !body.is_empty() {
                self.raw = body.to_string();
                return Some(toks(body));
            }
        }
        None
    }

    fn module(&mut self) -> Result<Module, ParseError> {
        let mut m = Module::default();
        while let Some(t) = self.next_line() {
            match t[0].as_str() {
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
        // global @name : size N align N
        if t.len() < 7 {
            return self.err("malformed global");
        }
        Ok(Global {
            id: GlobalId(0),
            name: t[1].trim_start_matches('@').into(),
            size: t[4].parse().map_err(|_| self.perr("bad size"))?,
            align: t[6].parse().map_err(|_| self.perr("bad align"))?,
            is_const: false,
            span: Span::DUMMY,
        })
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
        let name: Symbol = joined[5..open].trim().trim_start_matches('@').into();

        let mut params = Vec::new();
        let inner = joined[open + 1..close].trim();
        if !inner.is_empty() {
            for p in inner.split(',') {
                let (v, ty_s) = p
                    .split_once(':')
                    .ok_or_else(|| self.perr("param needs :"))?;
                params.push(Param {
                    value: ValueId(
                        v.trim()
                            .trim_start_matches('%')
                            .parse()
                            .map_err(|_| self.perr("bad param id"))?,
                    ),
                    ty: self.ty(ty_s.trim())?,
                });
            }
        }

        let tail = joined[close + 1..].trim();
        let has_body = tail.ends_with('{');
        let ret_s = tail.trim_start_matches("->").trim_end_matches('{').trim();
        let ret = self.ty(ret_s)?;

        let mut f = Function {
            id: FuncId(id),
            name,
            params,
            ret,
            variadic: false,
            allocas: Vec::new(),
            blocks: Vec::new(),
            entry: BlockId(0),
            attrs: Default::default(),
            body: if has_body {
                Body::Defined
            } else {
                Body::Declared
            },
            span: Span::DUMMY,
        };
        if has_body {
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
                    .split_once(" x ")
                    .ok_or_else(|| self.perr("vector needs `N x ty`"))?;
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
            let head = t[0].as_str();

            if head == "}" {
                if cur.is_some() && !terminated {
                    return self.err("block has no terminator");
                }
                break;
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
                let id = BlockId(if label == "entry" {
                    0
                } else {
                    label
                        .trim_start_matches("bb")
                        .parse()
                        .map_err(|_| self.perr("bad block label"))?
                });
                labels.push((label, id));
                cur = Some(Block {
                    id,
                    insts: Vec::new(),
                    term: Terminator::Unreachable(UnreachableReason::LoweringGap),
                    gcov_lines: Default::default(),
                    span: Span::DUMMY,
                });
                continue;
            }

            let Some(b) = cur.as_mut() else {
                return self.err("instruction outside a block");
            };

            if head == ".line" {
                let l: u32 = t[1].parse().map_err(|_| self.perr("bad line"))?;
                if !b.gcov_lines.contains(&l) {
                    b.gcov_lines.push(l);
                }
                b.gcov_lines.sort_unstable();
                continue;
            }
            if let Some(term) = self.terminator(&t, &mut pending, f.blocks.len())? {
                b.term = term;
                let done = cur.take().expect("block");
                f.blocks.push(done);
                terminated = true;
                continue;
            }
            let inst = self.inst(&t)?;
            b.insts.push(Inst {
                kind: inst,
                span: Span::DUMMY,
            });
        }

        if f.blocks.is_empty() {
            return self.err("function body has no blocks");
        }
        Ok(())
    }

    fn alloca(&mut self, t: &[String]) -> Result<AllocaDecl, ParseError> {
        // alloca %N : ty x COUNT align A scope S lifetime L ["name"]
        let get = |i: usize| t.get(i).map(String::as_str).unwrap_or("");
        Ok(AllocaDecl {
            id: AllocaId(
                get(1)
                    .trim_start_matches('%')
                    .parse()
                    .map_err(|_| self.perr("bad alloca id"))?,
            ),
            ty: self.ty(get(3))?,
            count: get(5).parse().map_err(|_| self.perr("bad alloca count"))?,
            align: get(7).parse().map_err(|_| self.perr("bad alloca align"))?,
            scope: ScopeId(get(9).parse().map_err(|_| self.perr("bad scope"))?),
            lifetime: match get(11) {
                "scope" => Lifetime::Scope,
                "function" => Lifetime::Function,
                other => return self.err(format!("unknown lifetime `{other}`")),
            },
            name: t.get(12).map(|n| n.trim_matches('"').into()),
            span: Span::DUMMY,
        })
    }

    fn label_id(&self, s: &str) -> Result<BlockId, ParseError> {
        Ok(BlockId(if s == "entry" {
            0
        } else {
            s.trim_start_matches("bb")
                .trim_end_matches(',')
                .parse()
                .map_err(|_| self.perr(&format!("bad block label `{s}`")))?
        }))
    }

    fn operand(&self, s: &str) -> Result<Operand, ParseError> {
        let s = s.trim_end_matches(',');
        if let Some(v) = s.strip_prefix('%') {
            return Ok(Operand::Value(ValueId(
                v.parse()
                    .map_err(|_| self.perr(&format!("bad value `{s}`")))?,
            )));
        }
        if s == "null" {
            return Ok(Operand::Const(Const::Null));
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
        Ok(Some(match t[0].as_str() {
            "ret" => Terminator::Return(match t.get(1) {
                Some(v) => Some(self.operand(v)?),
                None => None,
            }),
            "goto" => Terminator::Goto(self.label_id(&t[1])?),
            "br" => Terminator::Br {
                cond: self.operand(&t[1])?,
                t: self.label_id(&t[2])?,
                f: self.label_id(&t[3])?,
            },
            "switch" => {
                let ty = self.ty(&t[1])?;
                let scrut = self.operand(&t[2])?;
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
                    addr: self.operand(&t[1])?,
                    targets: targets?,
                }
            }
            _ => return Ok(None),
        }))
    }

    fn inst(&mut self, t: &[String]) -> Result<InstKind, ParseError> {
        // Markers.
        match t[0].as_str() {
            ".seqpoint" => return Ok(InstKind::Marker(MarkerKind::SeqPoint)),
            ".scope" => {
                let kind = match t[1].as_str() {
                    "enter" => ScopeKind::Enter,
                    "exit" => ScopeKind::Exit,
                    o => return self.err(format!("unknown scope event `{o}`")),
                };
                return Ok(InstKind::Marker(MarkerKind::Scope(ScopeEvent {
                    scope: ScopeId(t[2].parse().map_err(|_| self.perr("bad scope"))?),
                    kind,
                })));
            }
            ".label" => {
                return Ok(InstKind::Marker(MarkerKind::Label(
                    t[1].trim_matches('"').into(),
                )));
            }
            d if d.starts_with('.') => {
                return self.err(format!("unknown directive `{d}`"));
            }
            _ => {}
        }

        // `%N = <rvalue>` or a bare effectful instruction.
        if t.len() > 2 && t[1] == "=" {
            let dst = ValueId(
                t[0].trim_start_matches('%')
                    .parse()
                    .map_err(|_| self.perr("bad dst"))?,
            );
            let rest = &t[2..];
            if rest[0] == "call" {
                let (callee, args) = self.call_parts(rest)?;
                return Ok(InstKind::Call {
                    dst: Some(dst),
                    callee,
                    args,
                });
            }
            if rest[0] == "allocadyn" {
                return Ok(InstKind::AllocaDyn {
                    dst,
                    alloca: AllocaId(
                        rest[1]
                            .trim_start_matches('%')
                            .parse()
                            .map_err(|_| self.perr("bad alloca"))?,
                    ),
                    elem: self.ty(&rest[3])?,
                    count: self.operand(&rest[5])?,
                    align: rest[7].parse().map_err(|_| self.perr("bad align"))?,
                });
            }
            if rest[0] == "vaarg" {
                return Ok(InstKind::VaArg {
                    dst,
                    list: self.operand(&rest[1])?,
                    ty: self.ty(&rest[2])?,
                });
            }
            return Ok(InstKind::Assign {
                dst,
                rv: self.rvalue(rest)?,
            });
        }

        match t[0].as_str() {
            "store" | "storevolatile" => Ok(InstKind::Store {
                ty: self.ty(&t[1])?,
                val: self.operand(&t[2])?,
                addr: self.operand(&t[4])?,
                align: t[6].parse().map_err(|_| self.perr("bad align"))?,
                vol: if t[0] == "storevolatile" {
                    Volatility::Volatile
                } else {
                    Volatility::Normal
                },
            }),
            "storebits" => {
                let (off, width) = self.bits(&t[6])?;
                Ok(InstKind::StoreBits {
                    unit: self.ty(&t[1])?,
                    val: self.operand(&t[2])?,
                    addr: self.operand(&t[4])?,
                    bits: BitRange { off, width },
                    align: t[8].parse().map_err(|_| self.perr("bad align"))?,
                })
            }
            "copymem" => Ok(InstKind::CopyMem {
                dst: self.operand(&t[1])?,
                src: self.operand(&t[3])?,
                size: self.operand(&t[4])?,
                align: t[6].parse().map_err(|_| self.perr("bad align"))?,
            }),
            "setmem" => Ok(InstKind::SetMem {
                dst: self.operand(&t[1])?,
                byte: self.operand(&t[2])?,
                size: self.operand(&t[3])?,
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
                list: self.operand(&t[1])?,
            }),
            "vaend" => Ok(InstKind::VaEnd {
                list: self.operand(&t[1])?,
            }),
            "vacopy" => Ok(InstKind::VaCopy {
                src: self.operand(&t[1])?,
                dst: self.operand(&t[3])?,
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

    fn call_parts(&self, t: &[String]) -> Result<(Callee, Vec<Operand>), ParseError> {
        let _ = t;
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
            inner
                .split(',')
                .map(|a| self.operand(a.trim()))
                .collect::<Result<_, _>>()?
        };
        Ok((callee, args))
    }

    fn rvalue(&self, t: &[String]) -> Result<RValue, ParseError> {
        let g = |i: usize| t.get(i).map(String::as_str).unwrap_or("");
        Ok(match g(0) {
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
                let signed = t.iter().any(|x| x == "signed");
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
                alloca: AllocaId(
                    g(1).trim_start_matches('%')
                        .parse()
                        .map_err(|_| self.perr("bad alloca"))?,
                ),
            },
            "addrglobal" => RValue::AddrOfGlobal {
                g: self.global_id(g(1))?,
            },
            "addrfunc" => RValue::AddrOfFunc(self.func_id(g(1))?),
            "fresh" => RValue::Fresh { ty: self.ty(g(1))? },
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
            other if other.starts_with('%') || other == "null" || other.contains('i') => {
                RValue::Use(self.operand(other)?)
            }
            other => return Err(self.perr(&format!("unknown instruction `{other}`"))),
        })
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
            "\nglobal @{} : size {} align {}\n",
            g.name, g.size, g.align
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
    o.push_str(&format!(
        "func @{}({}) -> {}",
        f.name,
        params.join(", "),
        ty(&f.ret)
    ));
    if f.body == Body::Declared {
        o.push('\n');
        return;
    }
    o.push_str(" {\n");
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
        o.push('\n');
    }
    for b in &f.blocks {
        o.push_str(&format!("{}:\n", block_label(f, b.id)));
        for line in &b.gcov_lines {
            o.push_str(&format!("  .line {line}\n"));
        }
        for i in &b.insts {
            o.push_str("  ");
            print_inst(m, &i.kind, o);
            o.push('\n');
        }
        o.push_str("  ");
        print_term(f, &b.term, o);
        o.push('\n');
    }
    o.push_str("}\n");
}

/// `entry` for the entry block, `bbN` otherwise — matching 020 §6's example.
fn block_label(f: &Function, id: BlockId) -> String {
    if id == f.entry {
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
        CTy::Vector { elem, lanes } => format!("<{} x {}>", lanes, ty(elem)),
    }
}

fn op(o: &Operand) -> String {
    match o {
        Operand::Value(v) => format!("%{}", v.0),
        Operand::Const(c) => konst(c),
    }
}

fn konst(c: &Const) -> String {
    match c {
        Const::Int { bits, val } => format!("{val}i{bits}"),
        Const::Wide { bits, words } => {
            let hex: Vec<String> = words.iter().rev().map(|w| format!("{w:016x}")).collect();
            format!("0x{}i{bits}", hex.join(""))
        }
        Const::Float(k, bits) => format!(
            "{}:0x{bits:x}",
            match k {
                FloatKind::F32 => "f32",
                FloatKind::F64 => "f64",
                FloatKind::X87_80 => "f80",
            }
        ),
        Const::Null => "null".into(),
        Const::GlobalAddr { g, off } => format!("@g{}+{off}", g.0),
        Const::FuncAddr(f) => format!("@f{}", f.0),
        Const::Undef(t) => format!("undef {}", ty(t)),
    }
}

fn print_inst(m: &Module, k: &InstKind, o: &mut String) {
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
        } => o.push_str(&format!("{} {} {}, {}", binop(*b), ty(t), op(a), op(rhs))),
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
