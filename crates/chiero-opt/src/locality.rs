//! **Cache-line and locality analysis** — [041 §3](../../../docs/specs/041-optimization-analysis.md).
//!
//! > Caches have no semantic effect ([021 §7](021-memory-model.md)) — but VPP tunes for them
//! > deliberately: `CLIB_CACHE_LINE_BYTES` appears in **257** files and
//! > `CLIB_CACHE_LINE_ALIGN_MARK` in **124**. Layout is knowable statically, and access
//! > frequency is knowable from symbolic execution and coverage, so these are real findings
//! > rather than guesses.
//!
//! # The layout arrives; it is not recomputed
//!
//! [014 §3](../../../docs/specs/014-semantics-and-types.md) computes record layout and is
//! measured against gcc in its own corpus gate. Re-deriving straddling and packing rules here
//! would be a second answer to a question that already has one — the mistake `chiero-diff` was
//! corrected for — and 001 §4 rule 7 keeps this crate free of a frontend dependency anyway. So
//! [`Record`] is a plain description a caller fills in from whatever it has.
//!
//! # Two constraints, which are most of what this module is
//!
//! §3 states them, and they are the difference between an analysis and a hazard:
//!
//! > - **A reordering proposal must state whether the struct's layout is observable outside
//! >   the program.** Reordering an `ip4_header_t` is a protocol violation, not an
//! >   optimization. When chiero cannot prove the layout is internal, the proposal is advisory
//! >   and says so prominently.
//! > - **Benefit is labelled honestly.** `Measured` requires access counts from a real run;
//! >   otherwise it is `Estimated` or `Unquantified`. chiero has no cycle model and will not
//! >   pretend to one.
//!
//! The second is why [`Benefit::Estimated`] exists in the enum and is never produced here:
//! estimating requires a model of what a cache miss costs, and inventing one would put a
//! number in front of a reader that nothing measured. `Unquantified` is the honest label for
//! "this is a real observation and chiero cannot tell you what it is worth".

/// One field of a record, as far as this analysis needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    /// Byte offset from the start of the record — 014's answer, not this crate's.
    pub offset: u64,
    pub size: u64,
}

/// A record's layout, plus the two facts that decide whether reordering it may be proposed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub tag: String,
    pub size: u64,
    pub align: u64,
    /// `__attribute__((packed))`. §3 names it as one of the things that makes a layout
    /// externally observable.
    pub packed: bool,
    /// Whether the layout escapes: a wire format, an ABI boundary, anything reaching a
    /// serialization path.
    ///
    /// **The caller's answer, and the caller must default it to `true` when unsure.** §3 says
    /// "when chiero cannot prove the layout is internal, the proposal is advisory" — the
    /// unprovable case and the observable case get the same treatment, so a caller with no
    /// information is not forced to guess in the dangerous direction.
    pub externally_visible: bool,
    pub fields: Vec<Field>,
}

/// What the analysis was given about the machine and the run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalityCfg {
    pub cache_line_bytes: u64,
    /// Per-field access counts from a real run — 041 §3's `FieldAccessProfile`, as far as this
    /// analysis consumes it.
    ///
    /// **Empty is the default and means "no run", not "zero accesses".** Profiling is off by
    /// default (§3), and with it off a benefit is `Unquantified` rather than estimated from
    /// nothing.
    pub counts: Vec<(String, u64)>,
}

impl Default for LocalityCfg {
    fn default() -> Self {
        LocalityCfg {
            // VPP's own default, and the only value that makes 041 contract 18's numbers mean
            // what they say.
            cache_line_bytes: 64,
            counts: Vec::new(),
        }
    }
}

/// How much a proposal is worth — 041 §2's `Benefit`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Benefit {
    /// Backed by access counts from a real run.
    Measured,
    /// From a cost model. **Never produced**: chiero has no cycle model, and §3 says it will
    /// not pretend to one. Present so a future one has somewhere to go and so a reader of the
    /// enum can see the gap.
    Estimated,
    /// A real observation whose value chiero cannot quantify.
    Unquantified,
}

/// An obligation a proposal rests on — 041 §2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Obligation {
    Discharged {
        what: String,
    },
    /// **A proposal with any open obligation is advisory and labelled as such** (§2).
    Open {
        why: String,
    },
}

/// What kind of opportunity this is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptKind {
    /// A field spanning a cache-line boundary — two lines touched for one access.
    LineStraddle {
        field: String,
        offset: u64,
        size: u64,
    },
    /// Alignment padding a reorder would recover, with the size delta.
    PaddingWaste { recoverable: u64 },
    /// Frequently accessed fields beyond the first line while cold fields occupy it.
    HotFieldPlacement { field: String, offset: u64 },
}

impl OptKind {
    /// The field this is about, where it is about one. Used to order proposals.
    pub fn field(&self) -> Option<&str> {
        match self {
            OptKind::LineStraddle { field, .. } | OptKind::HotFieldPlacement { field, .. } => {
                Some(field)
            }
            OptKind::PaddingWaste { .. } => None,
        }
    }

    fn rank(&self) -> u8 {
        match self {
            OptKind::LineStraddle { .. } => 0,
            OptKind::HotFieldPlacement { .. } => 1,
            OptKind::PaddingWaste { .. } => 2,
        }
    }
}

/// One proposal — 041 §2's shape, minus the fields only a CIR-level detector fills in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub kind: OptKind,
    pub rationale: String,
    pub obligations: Vec<Obligation>,
    /// The numbers behind it, in words a reader can check.
    pub evidence: Vec<String>,
    pub benefit: Benefit,
    /// Derived from the obligations, never assigned — the same rule 050 §2 applies to
    /// `proven`. A boolean that could be set independently would eventually be set wrongly,
    /// once, in the flattering direction.
    pub advisory: bool,
}

impl Proposal {
    fn new(
        kind: OptKind,
        rationale: String,
        obligations: Vec<Obligation>,
        evidence: Vec<String>,
        benefit: Benefit,
    ) -> Proposal {
        let advisory = obligations
            .iter()
            .any(|o| matches!(o, Obligation::Open { .. }));
        Proposal {
            kind,
            rationale,
            obligations,
            evidence,
            benefit,
            advisory,
        }
    }
}

/// Analyse one record's layout — 041 §3.
///
/// **Never rewrites anything** (041 §1, contract 17): this returns proposals and the crate has
/// no API that applies one.
pub fn analyse(r: &Record, cfg: &LocalityCfg) -> Vec<Proposal> {
    let mut out = Vec::new();
    // §3's first constraint, computed once: a layout that may be observed from outside is one
    // no reordering may be proposed for without saying so.
    let escapes = layout_escapes(r);

    for f in &r.fields {
        if let Some(p) = straddle(f, r, cfg, escapes.as_deref()) {
            out.push(p);
        }
    }
    if let Some(p) = padding(r, cfg, escapes.as_deref()) {
        out.push(p);
    }
    out.extend(hot_placement(r, cfg, escapes.as_deref()));

    // **Contract 24: byte-identical order across runs.** Sorted by kind then by field name, so
    // two fields with equal access counts cannot swap between runs — which is the way this
    // would actually become non-deterministic, since the counts arrive from a map.
    out.sort_by(|a, b| {
        a.kind
            .rank()
            .cmp(&b.kind.rank())
            .then_with(|| a.kind.field().cmp(&b.kind.field()))
    });
    out
}

/// Why this record's layout may be observable from outside, if it may be.
fn layout_escapes(r: &Record) -> Option<String> {
    if r.packed {
        return Some(format!(
            "`{}` is `packed`, so its layout is observable outside the program — a wire \
             format or an ABI boundary. Reordering it is a protocol change, not an \
             optimization (041 §3)",
            r.tag
        ));
    }
    if r.externally_visible {
        return Some(format!(
            "`{}`'s layout was not shown to be internal to the program, so it may be \
             observable outside it — reordering it may be a protocol change rather than an \
             optimization (041 §3)",
            r.tag
        ));
    }
    None
}

/// The obligations and benefit every proposal about `r` shares.
fn shared(escapes: Option<&str>) -> Vec<Obligation> {
    match escapes {
        Some(why) => vec![Obligation::Open {
            why: why.to_string(),
        }],
        None => vec![Obligation::Discharged {
            what: "the layout is internal to the program".to_string(),
        }],
    }
}

/// Contract 18. A field straddles when the line it starts in is not the line its last byte is
/// in.
///
/// **Computed from first and last byte, not from `offset % line + size > line`.** The two agree
/// for a field smaller than a line and disagree for one larger, where the second says "always"
/// — and a 128-byte field on a 64-byte line does not straddle in the sense that matters, it
/// simply is more than a line. Reporting every such field would bury the ones a reorder can fix.
fn straddle(f: &Field, r: &Record, cfg: &LocalityCfg, escapes: Option<&str>) -> Option<Proposal> {
    if cfg.cache_line_bytes == 0 || f.size == 0 || f.size > cfg.cache_line_bytes {
        return None;
    }
    let first = f.offset / cfg.cache_line_bytes;
    let last = (f.offset + f.size - 1) / cfg.cache_line_bytes;
    if first == last {
        return None;
    }
    Some(Proposal::new(
        OptKind::LineStraddle {
            field: f.name.clone(),
            offset: f.offset,
            size: f.size,
        },
        format!(
            "`{}.{}` spans a {}-byte cache-line boundary, so one access touches two lines{}",
            r.tag,
            f.name,
            cfg.cache_line_bytes,
            match escapes {
                Some(_) => " — but see the obligation: the layout may be externally observable",
                None => "",
            }
        ),
        shared(escapes),
        {
            let mut e = vec![format!(
                "offset {} size {} crosses the boundary at {}",
                f.offset,
                f.size,
                (first + 1) * cfg.cache_line_bytes
            )];
            if let Some(n) = accesses(f, cfg) {
                e.push(format!("{n} accesses, each touching two lines"));
            }
            e
        },
        // **`Measured` when a run counted this field's accesses**, because the cost of a
        // straddle *is* "this many accesses, each touching two lines" — a number backed by a
        // run rather than by a cost model. Without counts there is no cycle model to fall
        // back on and §3 forbids inventing one, so the honest label is `Unquantified` and
        // never `Estimated`.
        match accesses(f, cfg) {
            Some(_) => Benefit::Measured,
            None => Benefit::Unquantified,
        },
    ))
}

/// This field's access count, when a run measured one.
///
/// `None` and `Some(0)` are different: no run at all, against a run in which nobody touched
/// this field. Only the first forbids a `Measured` label — a field a run never touched is a
/// measured zero, and a straddle nobody pays for is worth knowing.
fn accesses(f: &Field, cfg: &LocalityCfg) -> Option<u64> {
    cfg.counts
        .iter()
        .find(|(n, _)| *n == f.name)
        .map(|(_, c)| *c)
}

/// Padding a reorder would recover, and how many bytes.
///
/// The comparison is against the same fields laid out largest-alignment-first, which is the
/// reorder being proposed — so the delta is what *this* suggestion is worth rather than a
/// theoretical minimum nothing achieves.
fn padding(r: &Record, cfg: &LocalityCfg, escapes: Option<&str>) -> Option<Proposal> {
    if r.fields.is_empty() || r.packed {
        return None;
    }
    let mut sizes: Vec<u64> = r.fields.iter().map(|f| f.size).collect();
    // Descending by size, which for scalar members is descending by alignment.
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    let mut off = 0u64;
    for s in &sizes {
        let a = alignment_for(*s, r.align);
        if a > 0 && !off.is_multiple_of(a) {
            off += a - (off % a);
        }
        off += s;
    }
    if r.align > 0 && !off.is_multiple_of(r.align) {
        off += r.align - (off % r.align);
    }
    let recoverable = r.size.saturating_sub(off);
    if recoverable == 0 {
        return None;
    }
    Some(Proposal::new(
        OptKind::PaddingWaste { recoverable },
        format!(
            "`{}` is {} bytes and would be {} with its fields ordered by size{}",
            r.tag,
            r.size,
            off,
            match escapes {
                Some(_) => " — but see the obligation: the layout may be externally observable",
                None => "",
            }
        ),
        shared(escapes),
        vec![format!(
            "{} bytes of alignment padding, {} of {} lines saved per instance",
            recoverable,
            r.size.div_ceil(cfg.cache_line_bytes.max(1))
                - off.div_ceil(cfg.cache_line_bytes.max(1)),
            r.size.div_ceil(cfg.cache_line_bytes.max(1))
        )],
        Benefit::Unquantified,
    ))
}

/// A scalar member's alignment, bounded by the record's.
///
/// **A guess, and bounded so it cannot exceed what 014 computed.** The exact alignment of each
/// member is 014's to know; this analysis only has sizes, and using a size as an alignment is
/// right for every scalar and wrong for a nested aggregate. The bound keeps the error on the
/// side of proposing less.
fn alignment_for(size: u64, record_align: u64) -> u64 {
    let a = size.next_power_of_two().min(record_align.max(1));
    a.max(1)
}

/// §3's hot/cold placement: a frequently accessed field sitting beyond the first line.
///
/// **Only with counts.** Without a run there is no "frequently", and §3 says the hot/cold
/// finding is "not produced at all rather than being produced from nothing".
fn hot_placement(r: &Record, cfg: &LocalityCfg, escapes: Option<&str>) -> Vec<Proposal> {
    if cfg.counts.is_empty() {
        return Vec::new();
    }
    let count = |name: &str| -> u64 {
        cfg.counts
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| *c)
            .unwrap_or(0)
    };
    let hottest = r.fields.iter().map(|f| count(&f.name)).max().unwrap_or(0);
    if hottest == 0 {
        return Vec::new();
    }
    r.fields
        .iter()
        .filter(|f| count(&f.name) == hottest && f.offset >= cfg.cache_line_bytes)
        .map(|f| {
            Proposal::new(
                OptKind::HotFieldPlacement {
                    field: f.name.clone(),
                    offset: f.offset,
                },
                format!(
                    "`{}.{}` is the most-accessed field and sits past the first cache line{}",
                    r.tag,
                    f.name,
                    match escapes {
                        Some(_) =>
                            " — but see the obligation: the layout may be externally observable",
                        None => "",
                    }
                ),
                shared(escapes),
                vec![format!(
                    "{} accesses at offset {}, against {} for the field in the first line",
                    hottest,
                    f.offset,
                    r.fields
                        .iter()
                        .filter(|g| g.offset < cfg.cache_line_bytes)
                        .map(|g| count(&g.name))
                        .max()
                        .unwrap_or(0)
                )],
                // **The one place `Measured` is honest**: this proposal exists *because* of
                // counts from a real run, and the counts are in its evidence.
                Benefit::Measured,
            )
        })
        .collect()
}
