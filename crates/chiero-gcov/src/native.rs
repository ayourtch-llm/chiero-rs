//! The native `.gcno` / `.gcda` artifacts (030 §4).
//!
//! This module owns the *header*: magic, version tag and stamp. The record stream, the flow
//! solve and the arc bookkeeping come after, and deliberately so — every one of their failures
//! would look like a decode bug if the file turned out to be from another build, and contract 8
//! says that is the most common way coverage data goes wrong.
//!
//! Measured on this machine against gcc 13.3.0, and committed as fixtures rather than
//! transcribed:
//!
//! ```text
//! t.gcno:  6f 6e 63 67 | 2a 33 33 42 | 1f 83 0c d1     "oncg"  "*33B"  stamp
//! t.gcda:  61 64 63 67 | 2a 33 33 42 | 1f 83 0c d1     "adcg"  "*33B"  stamp — the same
//! ```

use std::path::{Path, PathBuf};

use crate::IngestError;

/// Which of the two artifacts a file is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `.gcno` — the CFG, written at compile time.
    Notes,
    /// `.gcda` — the counters, written at exit.
    Data,
}

/// `"gcno"` and `"gcda"`, as the words appear in the file.
///
/// gcc writes the magic little-endian, so the bytes read `oncg`/`adcg`; comparing the *word*
/// rather than the byte string is what makes that a fact about the format instead of a fact
/// about this machine's endianness.
const MAGIC_NOTES: u32 = 0x67636e6f;
const MAGIC_DATA: u32 = 0x67636461;

/// The versions this decoder has been tested against.
///
/// **A list, not a range.** 030 §4 is explicit: chiero decodes the versions it has fixtures for
/// and an unknown tag falls back to JSON, because a layout nobody has run against is a layout
/// whose field order is a guess. Adding a version here means adding a fixture that proves it.
const KNOWN: &[(u8, u8)] = &[(13, 3)];

/// A parsed artifact header.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    /// The version word exactly as stored.
    pub version: u32,
    /// The per-compilation stamp, identical in a `.gcno` and the `.gcda` it belongs to.
    pub stamp: u32,
}

impl Header {
    /// The version tag as gcc spells it — `"*33B"` for 13.3.
    pub fn version_tag(&self) -> String {
        self.version
            .to_le_bytes()
            .iter()
            .map(|&b| b as char)
            .collect()
    }

    /// `(major, minor)`, when the tag is one of the shapes gcc writes.
    ///
    /// **Measured, not transcribed.** `t.gcno` holds the bytes `2a 33 33 42`, which read back to
    /// front is `B33*`, and the file was written by gcc 13.3.0 — so the encoding is
    /// `(major / 10 + 'A') (major % 10 + '0') (minor + '0') (release)`: `B` is 10, `3` makes 13,
    /// the second `3` is the minor, and `*` marks a released compiler. A first reading of "a
    /// letter counting from `A` = 10" gives 11.3 and matches nothing.
    ///
    /// `None` for any other shape, and that is the point: [`is_known`] asks this, and a tag
    /// nobody has seen decodes to nothing rather than to a plausible number.
    pub fn gcc_version(&self) -> Option<(u8, u8)> {
        let b = self.version.to_le_bytes();
        // Stored little-endian, so the tag reads back to front.
        let (tens, ones, minor) = (b[3], b[2], b[1]);
        if !tens.is_ascii_uppercase() || !ones.is_ascii_digit() || !minor.is_ascii_digit() {
            return None;
        }
        let major = (tens - b'A').checked_mul(10)?.checked_add(ones - b'0')?;
        Some((major, minor - b'0'))
    }

    /// Whether this decoder has a fixture for this version.
    pub fn is_known(&self) -> bool {
        self.gcc_version().is_some_and(|v| KNOWN.contains(&v))
    }
}

/// Read one artifact's header.
///
/// Fails when the file is not a coverage artifact at all, and separately when it is one of a
/// version this decoder has no fixture for — two different things a reader does two different
/// things about.
pub fn header(path: &Path) -> Result<Header, IngestError> {
    let bytes = std::fs::read(path).map_err(|e| IngestError::Unreadable {
        path: path.to_path_buf(),
        why: e.to_string(),
    })?;
    let word = |i: usize| -> Option<u32> {
        bytes
            .get(i * 4..i * 4 + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let (Some(magic), Some(version), Some(stamp)) = (word(0), word(1), word(2)) else {
        return Err(IngestError::Malformed {
            path: path.to_path_buf(),
            why: "too short to hold a magic, a version and a stamp".into(),
        });
    };
    let kind = match magic {
        MAGIC_NOTES => Kind::Notes,
        MAGIC_DATA => Kind::Data,
        _ => {
            return Err(IngestError::Malformed {
                path: path.to_path_buf(),
                why: format!(
                    "magic {magic:#010x} is neither `gcno` ({MAGIC_NOTES:#010x}) nor `gcda` \
                     ({MAGIC_DATA:#010x})"
                ),
            });
        }
    };
    let h = Header {
        kind,
        version,
        stamp,
    };
    if !h.is_known() {
        return Err(IngestError::UnknownVersion {
            path: path.to_path_buf(),
            tag: h.version_tag(),
        });
    }
    Ok(h)
}

/// A `.gcno` and the `.gcda` that belongs to it.
#[derive(Clone, Debug)]
pub struct Pair {
    pub notes: PathBuf,
    pub data: PathBuf,
    pub header: Header,
}

/// Check that two artifacts are from the same compilation (contract 8).
///
/// **The stamp, not the timestamps.** gcc derives it per compilation and writes the same value
/// into both files, so it answers "were these produced together" exactly, where a modification
/// time answers "were they written near each other" and is wrong every time a build is restored
/// from a cache.
pub fn pair(notes: &Path, data: &Path) -> Result<Pair, IngestError> {
    let n = header(notes)?;
    let d = header(data)?;
    if n.kind != Kind::Notes {
        return Err(IngestError::Malformed {
            path: notes.to_path_buf(),
            why: "expected a `.gcno`, found the counters".into(),
        });
    }
    if d.kind != Kind::Data {
        return Err(IngestError::Malformed {
            path: data.to_path_buf(),
            why: "expected a `.gcda`, found the notes".into(),
        });
    }
    if n.stamp != d.stamp {
        return Err(IngestError::StaleData {
            notes: notes.to_path_buf(),
            data: data.to_path_buf(),
            notes_stamp: n.stamp,
            data_stamp: d.stamp,
        });
    }
    Ok(Pair {
        notes: notes.to_path_buf(),
        data: data.to_path_buf(),
        header: n,
    })
}

/// The record tags this decoder recognises (030 §4).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Tag {
    Function,
    Blocks,
    Arcs,
    Lines,
    CounterArcs,
    ObjectSummary,
    /// A tag gcc writes that this decoder has no use for.
    ///
    /// **Skipped, not refused.** The stream is self-describing — every record carries its own
    /// length — so an unrecognised tag costs exactly its own bytes, and refusing one would make
    /// every future gcc addition a hard failure on data that is otherwise perfectly readable.
    Other(u32),
}

impl Tag {
    fn from_word(w: u32) -> Tag {
        match w {
            0x0100_0000 => Tag::Function,
            0x0141_0000 => Tag::Blocks,
            0x0143_0000 => Tag::Arcs,
            0x0145_0000 => Tag::Lines,
            0x01a1_0000 => Tag::CounterArcs,
            0xa100_0000 => Tag::ObjectSummary,
            other => Tag::Other(other),
        }
    }
}

/// One record of the stream: a tag and its payload bytes.
#[derive(Clone, Debug)]
pub struct Record {
    pub tag: Tag,
    /// The offset the record's *tag* sits at, for a diagnostic that can be checked with `xxd`.
    pub at: usize,
    pub payload: Vec<u8>,
    /// Bytes of counters gcc elided because every one of them is zero.
    ///
    /// Non-zero only for a record whose stored length was negative. The counters are **not**
    /// missing data — their value is known exactly, and it is zero.
    pub elided_zeros: usize,
}

/// Where the records begin, which is not the same in the two artifacts.
///
/// A `.gcno` carries the working directory as a length-prefixed string plus a flag word after the
/// four header words; a `.gcda` goes straight to its records. **Measured** — this is exactly the
/// kind of offset 030 §4 refuses to take from documentation.
fn records_offset(bytes: &[u8], kind: Kind, path: &Path) -> Result<usize, IngestError> {
    match kind {
        Kind::Data => Ok(16),
        Kind::Notes => {
            let len = bytes
                .get(16..20)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
                .ok_or_else(|| IngestError::Malformed {
                    path: path.to_path_buf(),
                    why: "truncated before the working-directory length".into(),
                })?;
            // The string, then a flag word. Not padded to a word boundary: the fixture's records
            // start at 121, which is `20 + 97 + 4`.
            Ok(20 + len + 4)
        }
    }
}

/// Read the whole record stream of a `.gcno` or `.gcda`.
///
/// **All of it or none of it.** A stream that ends mid-record is a diagnostic rather than a short
/// list: returning what was read hands downstream a coverage index quietly missing functions,
/// which is the failure 030 contract 6 forbids for corrupt `.gcda` data and is no better here.
pub fn records(path: &Path) -> Result<Vec<Record>, IngestError> {
    let h = header(path)?;
    let bytes = std::fs::read(path).map_err(|e| IngestError::Unreadable {
        path: path.to_path_buf(),
        why: e.to_string(),
    })?;
    let mut i = records_offset(&bytes, h.kind, path)?;
    let mut out = Vec::new();
    while i < bytes.len() {
        // **A trailing partial word is not a record.** `t.gcda` ends with four zero bytes after
        // its last record; gcc pads, and a decoder that treats the padding as a tag reports a
        // corrupt file for a perfectly good one.
        if bytes.len() - i < 8 {
            if bytes[i..].iter().all(|&b| b == 0) {
                break;
            }
            return Err(IngestError::Malformed {
                path: path.to_path_buf(),
                why: format!(
                    "truncated: {} bytes after the last record at {i}",
                    bytes.len() - i
                ),
            });
        }
        let tag = u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
        if tag == 0 {
            break;
        }
        // **Bytes, not words.** Measured: `FUNCTION` at 121 with length 49 is followed by
        // `BLOCKS` at 178.
        //
        // **And signed.** A *negative* length is gcc's compression for a counter set that is
        // entirely zero: the magnitude is the bytes the counters would have taken and **none of
        // them is stored**, so the next record begins immediately after the length word. Read as
        // a `u32` it is about 4294967280 and every such file looks truncated — which is how 83 of
        // 98 objects in a real `--coverage` build of `vppinfra` failed to ingest, and why none of
        // the fixtures written before that build could find it: in all of them, every function
        // ran.
        let raw = i32::from_le_bytes([bytes[i + 4], bytes[i + 5], bytes[i + 6], bytes[i + 7]]);
        let len = if raw < 0 { 0 } else { raw as usize };
        let elided = if raw < 0 {
            raw.unsigned_abs() as usize
        } else {
            0
        };
        let start = i + 8;
        let Some(payload) = bytes.get(start..start + len) else {
            return Err(IngestError::Malformed {
                path: path.to_path_buf(),
                why: format!(
                    "truncated: the record at {i} claims {len} bytes and only {} remain",
                    bytes.len() - start
                ),
            });
        };
        out.push(Record {
            tag: Tag::from_word(tag),
            at: i,
            payload: payload.to_vec(),
            elided_zeros: elided,
        });
        i = start + len;
    }
    Ok(out)
}

/// Arc flags (030 §4).
///
/// A newtype rather than a bare `u32`: `ON_TREE` decides whether a counter exists for the arc and
/// `FAKE` decides whether it is real control flow, and confusing the two silently changes which
/// tests an arc-level selection returns.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ArcFlags(pub u32);

impl ArcFlags {
    /// gcc's spanning tree: no counter is stored, the count is recovered by conservation.
    pub const ON_TREE: ArcFlags = ArcFlags(1);
    /// To the exit block from a call that may not return. Real for conservation, not for
    /// selection (030 §4.1, contract 7).
    pub const FAKE: ArcFlags = ArcFlags(2);
    pub const FALLTHROUGH: ArcFlags = ArcFlags(4);

    pub fn contains(self, other: ArcFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for ArcFlags {
    type Output = ArcFlags;
    fn bitor(self, rhs: ArcFlags) -> ArcFlags {
        ArcFlags(self.0 | rhs.0)
    }
}

/// One arc of a function's CFG.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Arc {
    pub from: u32,
    pub to: u32,
    pub flags: ArcFlags,
}

/// The lines one block came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockLines {
    pub block: u32,
    pub file: String,
    pub lines: Vec<u32>,
}

/// One function's notes.
#[derive(Clone, Debug)]
pub struct NoteFunction {
    pub ident: u32,
    pub lineno_checksum: u32,
    pub cfg_checksum: u32,
    pub name: String,
    /// Compiler-generated. The flag exists between the name and the source file, and a decoder
    /// that skips it reads the source name out of the next field.
    pub artificial: bool,
    pub source: String,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub blocks: u32,
    pub arcs: Vec<Arc>,
    pub lines: Vec<BlockLines>,
}

/// A decoded `.gcno`.
#[derive(Clone, Debug)]
pub struct Note {
    pub header: Header,
    pub functions: Vec<NoteFunction>,
}

/// A cursor over a record payload.
///
/// **Every read is checked.** A record whose length disagrees with its content is corrupt data,
/// and 030 contract 6's rule — report it, do not guess — applies as much to a short field as to a
/// short file.
struct Cursor<'a> {
    p: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(p: &'a [u8]) -> Cursor<'a> {
        Cursor { p, at: 0 }
    }

    fn u32(&mut self) -> Option<u32> {
        let b = self.p.get(self.at..self.at + 4)?;
        self.at += 4;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A length in bytes, then that many bytes, **unpadded**. The trailing NUL is dropped.
    fn string(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        let b = self.p.get(self.at..self.at + n)?;
        self.at += n;
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        Some(String::from_utf8_lossy(&b[..end]).into_owned())
    }

    fn done(&self) -> bool {
        self.at >= self.p.len()
    }
}

/// Decode a `.gcno` into its functions, their CFGs and their line sets.
pub fn read_notes(path: &Path) -> Result<Note, IngestError> {
    let header = header(path)?;
    if header.kind != Kind::Notes {
        return Err(IngestError::Malformed {
            path: path.to_path_buf(),
            why: "expected a `.gcno`, found the counters".into(),
        });
    }
    let short = |why: &str| IngestError::Malformed {
        path: path.to_path_buf(),
        why: format!("truncated record: {why}"),
    };
    let mut functions: Vec<NoteFunction> = Vec::new();
    for rec in records(path)? {
        match rec.tag {
            Tag::Function => {
                let mut c = Cursor::new(&rec.payload);
                let f = NoteFunction {
                    ident: c.u32().ok_or_else(|| short("function ident"))?,
                    lineno_checksum: c.u32().ok_or_else(|| short("lineno checksum"))?,
                    cfg_checksum: c.u32().ok_or_else(|| short("cfg checksum"))?,
                    name: c.string().ok_or_else(|| short("function name"))?,
                    artificial: c.u32().ok_or_else(|| short("artificial flag"))? != 0,
                    source: c.string().ok_or_else(|| short("source name"))?,
                    start_line: c.u32().ok_or_else(|| short("start line"))?,
                    start_column: c.u32().ok_or_else(|| short("start column"))?,
                    end_line: c.u32().ok_or_else(|| short("end line"))?,
                    end_column: c.u32().ok_or_else(|| short("end column"))?,
                    blocks: 0,
                    arcs: Vec::new(),
                    lines: Vec::new(),
                };
                functions.push(f);
            }
            // **Everything after a `FUNCTION` belongs to it.** The stream is a sequence, not a
            // tree: `BLOCKS`, `ARCS` and `LINES` attach to the most recent function, and a record
            // arriving before any function is corrupt rather than global.
            Tag::Blocks | Tag::Arcs | Tag::Lines => {
                let Some(f) = functions.last_mut() else {
                    return Err(IngestError::Malformed {
                        path: path.to_path_buf(),
                        why: format!("a {:?} record at {} before any function", rec.tag, rec.at),
                    });
                };
                let mut c = Cursor::new(&rec.payload);
                match rec.tag {
                    Tag::Blocks => f.blocks = c.u32().ok_or_else(|| short("block count"))?,
                    Tag::Arcs => {
                        let from = c.u32().ok_or_else(|| short("arc source block"))?;
                        while !c.done() {
                            let to = c.u32().ok_or_else(|| short("arc destination"))?;
                            let flags = c.u32().ok_or_else(|| short("arc flags"))?;
                            f.arcs.push(Arc {
                                from,
                                to,
                                flags: ArcFlags(flags),
                            });
                        }
                    }
                    Tag::Lines => {
                        // **A grammar, not fields**: a 0 introduces a file name, anything else is
                        // a line, and a 0 with an empty name ends the record.
                        //
                        // **One record, possibly several files.** An `always_inline` call puts the
                        // callee's lines in the caller's block, so a block reads
                        // `FILE inl.c 2  FILE inl.h 3 4  END`. Keeping a single `file` per record
                        // attributed `inl.h:3` to `inl.c` — a line that need not exist, and an
                        // answer about the wrong code that reads exactly like a right one. Each
                        // group becomes its own `BlockLines`, all of them for the same block.
                        let block = c.u32().ok_or_else(|| short("line block"))?;
                        let mut file = String::new();
                        let mut lines: Vec<u32> = Vec::new();
                        let flush =
                            |file: &str, lines: &mut Vec<u32>, out: &mut Vec<BlockLines>| {
                                if !lines.is_empty() {
                                    // **Sorted, as gcov sorts it** (`gcc/gcov.cc` ~1413, the pass
                                    // that sizes each source's line vector) — because the block is
                                    // then attributed to the group's *last* entry, which makes it
                                    // the greatest line rather than the last one written. The two
                                    // differ wherever a call is inlined: the block holding the
                                    // call carries the callee's lower line numbers after it, so
                                    // the unsorted last entry is a line inside a function that
                                    // block is not in. Duplicates stay — gcov sorts, it does not
                                    // deduplicate, and a line listed twice is accumulated twice.
                                    lines.sort_unstable();
                                    out.push(BlockLines {
                                        block,
                                        file: file.to_string(),
                                        lines: std::mem::take(lines),
                                    });
                                }
                            };
                        while let Some(v) = c.u32() {
                            if v != 0 {
                                lines.push(v);
                                continue;
                            }
                            let name = c.string().ok_or_else(|| short("line file name"))?;
                            flush(&file, &mut lines, &mut f.lines);
                            if name.is_empty() {
                                break;
                            }
                            file = name;
                        }
                        flush(&file, &mut lines, &mut f.lines);
                    }
                    _ => unreachable!("the arm matched these three tags"),
                }
            }
            // A tag this decoder has no use for costs its own bytes and nothing else.
            _ => {}
        }
    }
    Ok(Note { header, functions })
}

/// One function's counters, as the `.gcda` stores them.
#[derive(Clone, Debug)]
pub struct DataFunction {
    pub ident: u32,
    pub lineno_checksum: u32,
    pub cfg_checksum: u32,
    /// One `u64` per **non-tree** arc, in the order the notes list them.
    pub counters: Vec<u64>,
}

/// A decoded `.gcda`.
#[derive(Clone, Debug)]
pub struct Data {
    pub header: Header,
    pub functions: Vec<DataFunction>,
}

/// Decode a `.gcda` into its per-function counters.
pub fn read_data(path: &Path) -> Result<Data, IngestError> {
    let header = header(path)?;
    if header.kind != Kind::Data {
        return Err(IngestError::Malformed {
            path: path.to_path_buf(),
            why: "expected a `.gcda`, found the notes".into(),
        });
    }
    let short = |why: &str| IngestError::Malformed {
        path: path.to_path_buf(),
        why: format!("truncated record: {why}"),
    };
    let mut functions: Vec<DataFunction> = Vec::new();
    for rec in records(path)? {
        match rec.tag {
            Tag::Function => {
                let mut c = Cursor::new(&rec.payload);
                functions.push(DataFunction {
                    ident: c.u32().ok_or_else(|| short("function ident"))?,
                    lineno_checksum: c.u32().ok_or_else(|| short("lineno checksum"))?,
                    cfg_checksum: c.u32().ok_or_else(|| short("cfg checksum"))?,
                    counters: Vec::new(),
                });
            }
            Tag::CounterArcs => {
                let Some(f) = functions.last_mut() else {
                    return Err(IngestError::Malformed {
                        path: path.to_path_buf(),
                        why: format!("counters at {} before any function", rec.at),
                    });
                };
                // **`u64` each, and a payload that is not a multiple of 8 is corrupt.** Rounding
                // down would drop a counter and leave the flow solve short by exactly one arc,
                // which is the shape of failure that looks like a decoder bug for days.
                if rec.payload.len() % 8 != 0 || rec.elided_zeros % 8 != 0 {
                    return Err(IngestError::Malformed {
                        path: path.to_path_buf(),
                        why: format!(
                            "a counter record of {} bytes ({} elided) is not a whole number of \
                             counters",
                            rec.payload.len(),
                            rec.elided_zeros
                        ),
                    });
                }
                for k in (0..rec.payload.len()).step_by(8) {
                    let b = &rec.payload[k..k + 8];
                    f.counters.push(u64::from_le_bytes([
                        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                    ]));
                }
                // **Materialised rather than left short.** The flow solve wants one counter per
                // non-tree arc and would otherwise report a graph that disagrees with its data —
                // a true sentence about the wrong thing, since the counters are known and zero.
                for _ in 0..rec.elided_zeros / 8 {
                    f.counters.push(0);
                }
            }
            _ => {}
        }
    }
    Ok(Data { header, functions })
}

/// Recover every arc's count from the non-tree counters (030 §4.1).
///
/// `.gcda` stores counters only for arcs **off** gcc's spanning tree — that omission is the space
/// optimisation the format exists for. The rest follow from conservation: at every block other
/// than entry and exit, in-flow equals out-flow, and the spanning-tree property guarantees that
/// iterating to a fixpoint determines all of them.
///
/// **Iterated over every block until nothing changes, rather than in one pass.** A single sweep in
/// block order gets most arcs right and silently leaves one wrong when a block's determining
/// neighbour comes later — the Python that first took this measurement produced a negative count
/// that way while every *block* total still looked correct.
///
/// Anything still unknown at the fixpoint is corrupt data and is reported, never guessed
/// (contract 6).
fn solve_arcs(f: &NoteFunction, counters: &[u64], path: &Path) -> Result<Vec<u64>, IngestError> {
    let n = f.arcs.len();
    let mut known: Vec<Option<u64>> = vec![None; n];
    let mut next = 0usize;
    for (i, a) in f.arcs.iter().enumerate() {
        if !a.flags.contains(ArcFlags::ON_TREE) {
            let Some(&c) = counters.get(next) else {
                return Err(IngestError::Malformed {
                    path: path.to_path_buf(),
                    why: format!(
                        "`{}` has more non-tree arcs than the {} counters recorded for it",
                        f.name,
                        counters.len()
                    ),
                });
            };
            known[i] = Some(c);
            next += 1;
        }
    }
    if next != counters.len() {
        return Err(IngestError::Malformed {
            path: path.to_path_buf(),
            why: format!(
                "`{}` has {next} non-tree arcs and {} counters; the notes and the data disagree \
                 about its control-flow graph",
                f.name,
                counters.len()
            ),
        });
    }

    // Conservation, to a fixpoint.
    loop {
        let mut changed = false;
        for b in 0..f.blocks {
            for incoming in [true, false] {
                let side: Vec<usize> = (0..n)
                    .filter(|&i| {
                        if incoming {
                            f.arcs[i].to == b
                        } else {
                            f.arcs[i].from == b
                        }
                    })
                    .collect();
                let other: Vec<usize> = (0..n)
                    .filter(|&i| {
                        if incoming {
                            f.arcs[i].from == b
                        } else {
                            f.arcs[i].to == b
                        }
                    })
                    .collect();
                // **The entry and exit blocks conserve nothing**: flow enters at one and leaves at
                // the other, so a rule derived from their empty side would be arithmetic about
                // nothing.
                if side.is_empty() || other.is_empty() {
                    continue;
                }
                let missing: Vec<usize> = side
                    .iter()
                    .copied()
                    .filter(|&i| known[i].is_none())
                    .collect();
                if missing.len() != 1 || other.iter().any(|&i| known[i].is_none()) {
                    continue;
                }
                let total: u64 = other.iter().map(|&i| known[i].unwrap()).sum();
                let accounted: u64 = side
                    .iter()
                    .filter(|&&i| i != missing[0])
                    .map(|&i| known[i].unwrap())
                    .sum();
                let Some(rest) = total.checked_sub(accounted) else {
                    return Err(IngestError::Malformed {
                        path: path.to_path_buf(),
                        why: format!(
                            "`{}` block {b}: the arcs into it already exceed the flow out of it, \
                             so the counters do not belong to this graph",
                            f.name
                        ),
                    });
                };
                known[missing[0]] = Some(rest);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    known
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            v.ok_or_else(|| IngestError::Malformed {
                path: path.to_path_buf(),
                why: format!(
                    "`{}` arc {}->{} could not be determined; the spanning tree guarantees it \
                     unless the data is corrupt",
                    f.name, f.arcs[i].from, f.arcs[i].to
                ),
            })
        })
        .collect()
}

/// A block's execution count is the flow **into** it — except the entry block, which nothing
/// flows into and whose count is the flow out.
fn block_counts(f: &NoteFunction, arcs: &[u64]) -> Vec<u64> {
    let mut counts = vec![0u64; f.blocks as usize];
    for b in 0..f.blocks {
        let (mut into, mut out) = (0u64, 0u64);
        for (i, a) in f.arcs.iter().enumerate() {
            if a.to == b {
                into = into.saturating_add(arcs[i]);
            }
            if a.from == b {
                out = out.saturating_add(arcs[i]);
            }
        }
        counts[b as usize] = if b == 0 { out } else { into };
    }
    counts
}

/// The line counts one function contributes, as `(file, line, count)`.
///
/// **Not an aggregation of the blocks' counts.** gcc's own word for summing them is "artificially
/// high" (`gcc/gcov.cc`, `accumulate_line_info`): a line compiled into several blocks that flow
/// into one another has been executed once, not once per block. The rule is a computation on the
/// subgraph induced by the line's blocks —
///
/// 1. every arc entering that subgraph from outside contributes its count, and
/// 2. every elementary cycle *within* it contributes its bottleneck arc,
///
/// — so a `for` loop written entirely on one line is counted by finding the loop. The answer
/// therefore depends on the arcs, which is why no formula over block counts reproduces it; see
/// `crates/chiero-gcov/tests/line_rule.rs` for what that cost to learn.
///
/// A line no block was *attributed* to keeps the accumulated sum instead. That is not a fallback
/// invented here: gcov attributes each block to the last line of each of its location groups, and
/// the entry and exit blocks to none, so the lines those pass over never enter the graph
/// computation at all.
///
/// **The two halves are returned apart because they do not combine by addition.** gcov holds one
/// record per `(source, line)` for the whole object, and the graph count *overwrites* whatever
/// accumulated there — from every function, not just this one. So a line some function attributed
/// a block to takes the sum of the graph counts and discards every accumulation, which a caller
/// cannot reconstruct from one merged number. See [`ObjectLines`].
fn line_counts(f: &NoteFunction, arc_counts: &[u64], blocks: &[u64]) -> FunctionLines {
    let last_block = f.blocks.saturating_sub(1);
    let mut acc: IndexMap<(String, u32), u64> = IndexMap::new();
    // The blocks gcov attributes to each line, in its order, duplicates kept: a block reached
    // twice contributes its entry arcs twice, and dropping the repeat would change the answer.
    let mut on_line: IndexMap<(String, u32), Vec<u32>> = IndexMap::new();

    // `add_line_counts`. `carried` mirrors gcov's `line` pointer, which persists across a block's
    // location groups and is reset per block — a group with no lines pushes the block onto
    // whatever line the previous group ended on.
    let mut carried: Option<(String, u32)> = None;
    let mut current: Option<u32> = None;
    for bl in &f.lines {
        if current != Some(bl.block) {
            current = Some(bl.block);
            carried = None;
        }
        let c = blocks.get(bl.block as usize).copied().unwrap_or(0);
        for &line in &bl.lines {
            let slot = acc.entry((bl.file.clone(), line)).or_insert(0);
            *slot = slot.saturating_add(c);
            carried = Some((bl.file.clone(), line));
        }
        if let (true, Some(key)) = (bl.block != 0 && bl.block != last_block, &carried) {
            on_line.entry(key.clone()).or_default().push(bl.block);
        }
    }

    // `accumulate_line_info`, for the lines this function attributed a block to.
    let succ = succ_lists(f);
    let mut graphed: IndexMap<(String, u32), u64> = IndexMap::new();
    for (key, bs) in &on_line {
        let mut count: u64 = 0;
        for &b in bs {
            for (i, a) in f.arcs.iter().enumerate() {
                if a.to == b && !bs.contains(&a.from) {
                    count = count.saturating_add(arc_counts[i]);
                }
            }
        }
        count = count.saturating_add(cycles_count(f, &succ, bs, arc_counts));
        acc.shift_remove(key);
        graphed.insert(key.clone(), count);
    }

    FunctionLines {
        accumulated: acc,
        graphed,
    }
}

/// One function's contribution to its object's lines, in the two kinds that merge differently.
struct FunctionLines {
    /// Lines this function's blocks passed over without any of them being attributed to them:
    /// the sum of those blocks' counts.
    accumulated: IndexMap<(String, u32), u64>,
    /// Lines this function attributed at least one block to: the count of the subgraph those
    /// blocks induce.
    graphed: IndexMap<(String, u32), u64>,
}

impl FunctionLines {
    /// This function's own answer for one line: the graph count where it has one, the
    /// accumulation otherwise. This is the resolution gcov performs per *table*, which for a
    /// group member is its private one.
    fn resolved(&self) -> impl Iterator<Item = (&(String, u32), u64)> {
        self.graphed.iter().map(|(k, &c)| (k, c)).chain(
            self.accumulated
                .iter()
                .filter(|(k, _)| !self.graphed.contains_key(*k))
                .map(|(k, &c)| (k, c)),
        )
    }
}

/// The lines a function accounts into its own private table rather than the source's.
///
/// gcov gives a function one when it shares a `(source, start_line)` with another non-artificial
/// function — a *group* — and the table covers `start_line ..= end_line` of that function's own
/// source. Lines outside it, and lines in any other file, still go to the shared table.
#[derive(Clone)]
struct GroupRange {
    source: String,
    start_line: u32,
    end_line: u32,
}

impl GroupRange {
    fn holds(&self, file: &str, line: u32) -> bool {
        file == self.source && self.start_line <= line && line <= self.end_line
    }
}

/// Which functions of an object are in a group, by index into `notes.functions`.
///
/// **Both members are marked, and the first is only known to be one when the second turns up** —
/// which is why this is a pass over the whole object before any of it is counted, exactly as
/// `process_all_functions` does it.
fn group_members(functions: &[NoteFunction]) -> Vec<bool> {
    let mut out = vec![false; functions.len()];
    let mut first: IndexMap<(&str, u32), usize> = IndexMap::new();
    for (i, f) in functions.iter().enumerate() {
        // Artificial functions are removed before gcov gets this far, so they neither form a
        // group nor join one.
        if f.artificial {
            continue;
        }
        match first.get(&(f.source.as_str(), f.start_line)) {
            Some(&j) => {
                out[i] = true;
                out[j] = true;
            }
            None => {
                first.insert((f.source.as_str(), f.start_line), i);
            }
        }
    }
    out
}

/// The lines of one object, merged across its functions the way gcov merges them.
///
/// **Attribution by any function wins for every function.** gcov's `accumulate_line_info`
/// overwrites the shared `(source, line)` record with the graph count whenever its block list is
/// non-empty, and that list is the union over the object. So a line one function graphed and
/// another merely accumulated over reports the graph count alone — the accumulation is not added,
/// it is discarded. Summing the two would inflate every line that a header is inlined into.
///
/// Graph counts *do* sum across functions, because no arc crosses a function: the union's induced
/// subgraph is the disjoint union of the per-function ones, and its entry arcs and cycles are
/// theirs.
#[derive(Default)]
struct ObjectLines {
    accumulated: IndexMap<(String, u32), u64>,
    graphed: IndexMap<(String, u32), u64>,
    /// Group members' private tables, already resolved per function and summed. These are added
    /// to whatever the shared table says rather than merged into it, because `--json-format`
    /// emits them as separate line entries and their meaning is the sum.
    private: IndexMap<(String, u32), u64>,
}

impl ObjectLines {
    fn add(&mut self, f: FunctionLines, group: Option<&GroupRange>) {
        // A group member's own-range lines are resolved here, alone, and only their total joins
        // the object — the shared table's overwrite must not reach them.
        if let Some(g) = group {
            for (key, c) in f.resolved() {
                if g.holds(&key.0, key.1) {
                    let slot = self.private.entry(key.clone()).or_insert(0);
                    *slot = slot.saturating_add(c);
                }
            }
        }
        let shared = |g: Option<&GroupRange>, k: &(String, u32)| match g {
            Some(g) => !g.holds(&k.0, k.1),
            None => true,
        };
        for (key, c) in f.accumulated.iter().filter(|(k, _)| shared(group, k)) {
            let slot = self.accumulated.entry(key.clone()).or_insert(0);
            *slot = slot.saturating_add(*c);
        }
        for (key, c) in f.graphed.iter().filter(|(k, _)| shared(group, k)) {
            let slot = self.graphed.entry(key.clone()).or_insert(0);
            *slot = slot.saturating_add(*c);
        }
    }

    /// `(file, line, count)` for every line of the object.
    fn finish(mut self) -> Vec<(String, u32, u64)> {
        for (key, c) in self.graphed {
            self.accumulated.insert(key, c);
        }
        for (key, c) in self.private {
            let slot = self.accumulated.entry(key).or_insert(0);
            *slot = slot.saturating_add(c);
        }
        self.accumulated
            .into_iter()
            .map(|((f, l), c)| (f, l, c))
            .collect()
    }
}

/// Each block's outgoing arcs in gcov's order: **declaration order**, stably sorted by
/// destination.
///
/// The order is not cosmetic — [`handle_cycle`](cycles_count) subtracts each cycle's bottleneck
/// from its arcs, so which cycle is enumerated first can change what the later ones are worth. It
/// can only matter between *parallel* arcs, two with the same source and destination, since the
/// sort separates every other pair.
///
/// `read_graph_file` builds each list by prepending, which reverses it, and `solve_flow_graph`
/// reverses it back before anything reads it — "The arcs were built in reverse order. Fix that
/// now." (`gcc/gcov.cc` ~2131). The sort by destination happens after that and is stable, so what
/// survives is declaration order within each destination.
fn succ_lists(f: &NoteFunction) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); f.blocks as usize];
    for (i, a) in f.arcs.iter().enumerate() {
        if let Some(slot) = out.get_mut(a.from as usize) {
            slot.push(i);
        }
    }
    for slot in &mut out {
        slot.sort_by_key(|&i| f.arcs[i].to);
    }
    out
}

/// The counts of the elementary cycles lying entirely within `bs`, by Hawick and James'
/// enumeration — the algorithm `gcc/gcov.cc` cites and implements in `circuit`/`unblock`.
///
/// Each cycle is worth its **minimum** remaining arc count, which is then subtracted from every
/// arc along it, so two cycles sharing an arc cannot both claim it. A blocked node is released
/// only when it turns out to lie on a cycle, which is what keeps the search over simple paths
/// from being exponential in the common case.
fn cycles_count(f: &NoteFunction, succ: &[Vec<usize>], bs: &[u32], arc_counts: &[u64]) -> u64 {
    let mut cs: Vec<i64> = vec![0; f.arcs.len()];
    for &b in bs {
        for &i in succ.get(b as usize).into_iter().flatten() {
            cs[i] = arc_counts[i].min(i64::MAX as u64) as i64;
        }
    }
    let mut count: i64 = 0;
    for &start in bs {
        let mut path: Vec<usize> = Vec::new();
        let mut blocked: Vec<u32> = Vec::new();
        let mut block_lists: Vec<Vec<u32>> = Vec::new();
        circuit(
            f,
            succ,
            start,
            &mut path,
            start,
            &mut blocked,
            &mut block_lists,
            bs,
            &mut cs,
            &mut count,
        );
    }
    count.max(0) as u64
}

#[allow(clippy::too_many_arguments)]
fn circuit(
    f: &NoteFunction,
    succ: &[Vec<usize>],
    v: u32,
    path: &mut Vec<usize>,
    start: u32,
    blocked: &mut Vec<u32>,
    block_lists: &mut Vec<Vec<u32>>,
    bs: &[u32],
    cs: &mut [i64],
    count: &mut i64,
) -> bool {
    let mut loop_found = false;
    blocked.push(v);
    block_lists.push(Vec::new());

    for &i in succ.get(v as usize).into_iter().flatten() {
        let w = f.arcs[i].to;
        // `w < start` keeps each cycle to the one rotation whose lowest block starts it, which is
        // what makes the enumeration terminate rather than merely deduplicate afterwards.
        if w < start || cs[i] <= 0 || !bs.contains(&w) {
            continue;
        }
        path.push(i);
        if w == start {
            let cycle = path.iter().map(|&e| cs[e]).min().unwrap_or(0);
            *count += cycle;
            for &e in path.iter() {
                cs[e] -= cycle;
            }
            loop_found = true;
        } else if !path.iter().any(|&e| cs[e] <= 0) && !blocked.contains(&w) {
            loop_found |= circuit(f, succ, w, path, start, blocked, block_lists, bs, cs, count);
        }
        path.pop();
    }

    if loop_found {
        unblock(v, blocked, block_lists);
    } else {
        for &i in succ.get(v as usize).into_iter().flatten() {
            let w = f.arcs[i].to;
            if w < start || cs[i] <= 0 || !bs.contains(&w) {
                continue;
            }
            let Some(index) = blocked.iter().position(|&b| b == w) else {
                continue;
            };
            if !block_lists[index].contains(&v) {
                block_lists[index].push(v);
            }
        }
    }
    loop_found
}

/// Release `u`, and transitively everything whose search was blocked waiting on it.
fn unblock(u: u32, blocked: &mut Vec<u32>, block_lists: &mut Vec<Vec<u32>>) {
    let Some(index) = blocked.iter().position(|&b| b == u) else {
        return;
    };
    blocked.remove(index);
    let to_unblock = block_lists.remove(index);
    for w in to_unblock {
        unblock(w, blocked, block_lists);
    }
}

/// Ingest a `.gcno`/`.gcda` pair for one object stem (030 §4).
///
/// The counters are matched to the notes by `ident`; a function in one and not the other is
/// corrupt rather than skippable, because a missing function is a set of lines that will read as
/// "no test covered this".
pub fn ingest_into(
    idx: &mut crate::CoverageIndex,
    test: Option<(crate::TestId, crate::Variant)>,
    dir: &Path,
    stem: &str,
) -> Result<(), IngestError> {
    let notes_path = dir.join(format!("{stem}.gcno"));
    let data_path = dir.join(format!("{stem}.gcda"));
    for p in [&notes_path, &data_path] {
        if !p.exists() {
            return Err(IngestError::Missing { path: p.clone() });
        }
    }
    // The stamp check first: every later failure would look like a decoder bug.
    pair(&notes_path, &data_path)?;
    let notes = read_notes(&notes_path)?;
    let data = read_data(&data_path)?;

    if let Some((t, v)) = &test {
        idx.note_test(*t);
        idx.note_variant(v);
    }
    let mut object = ObjectLines::default();
    let in_group = group_members(&notes.functions);
    for (fi, f) in notes.functions.iter().enumerate() {
        let Some(d) = data.functions.iter().find(|d| d.ident == f.ident) else {
            return Err(IngestError::Malformed {
                path: data_path.clone(),
                why: format!(
                    "no counters for `{}` (ident {:#x}); a function missing from the data reads \
                     downstream as lines no test covered",
                    f.name, f.ident
                ),
            });
        };
        // **The checksums pair a *function*, as the stamp pairs a file.** Same name, same object,
        // recompiled after an edit: the stamps match and this does not.
        if d.cfg_checksum != f.cfg_checksum || d.lineno_checksum != f.lineno_checksum {
            return Err(IngestError::Malformed {
                path: data_path.clone(),
                why: format!(
                    "`{}` has checksums {:#x}/{:#x} in the notes and {:#x}/{:#x} in the data",
                    f.name, f.lineno_checksum, f.cfg_checksum, d.lineno_checksum, d.cfg_checksum
                ),
            });
        }
        let arcs = solve_arcs(f, &d.counters, &data_path)?;
        let blocks = block_counts(f, &arcs);
        let group = in_group[fi].then(|| GroupRange {
            source: f.source.clone(),
            start_line: f.start_line,
            end_line: f.end_line,
        });
        object.add(line_counts(f, &arcs, &blocks), group.as_ref());
    }
    // **Merged across the object before the index sees any of it.** A header inlined into three
    // functions contributes to its lines three times, and the index's merge cannot tell those
    // apart from three separate objects reporting the same line.
    for (file, line, count) in object.finish() {
        match &test {
            Some((t, v)) => idx.add_line_for_variant(*t, v, file, line, count),
            None => idx.add_line(file, line, count),
        }
    }
    idx.set_detail(CoverageDetail::LinesAndArcs);
    idx.push_provenance(crate::IngestRecord {
        artifact: notes_path,
        gcc_version: {
            let (maj, min) = notes.header.gcc_version().unwrap_or((0, 0));
            format!("{maj}.{min}")
        },
        format_version: notes.header.version_tag(),
    });
    Ok(())
}

use crate::CoverageDetail;
use indexmap::IndexMap;

/// What identifies a function for coverage purposes (030 §5).
///
/// **Not the name.** `a.c` and `b.c` may each hold a `static int helper(int)`, and merging them
/// attributes one file's tests to the other's code — silently, since nothing about the merged
/// entry looks wrong. The file and the start line separate those; `march` separates the copies
/// VPP compiles of one source under different `CLIB_MARCH_VARIANT`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FuncKey {
    pub file: String,
    pub name: String,
    pub start_line: u32,
    /// The multiarch variant, when a [`MarchResolver`] identified one.
    pub march: Option<String>,
}

impl FuncKey {
    /// A key with no multiarch variant, which is every function outside VPP.
    pub fn new(file: &str, name: &str, start_line: u32) -> FuncKey {
        FuncKey {
            file: file.to_string(),
            name: name.to_string(),
            start_line,
            march: None,
        }
    }
}

/// Splits a symbol into its function name and multiarch variant (030 §5).
///
/// **An extension point, because the variant set is VPP's knowledge and 001 §4 rule 4 keeps that
/// out of this crate.** The default splits nothing. A resolver must never guess from a bare
/// suffix pattern: collapsing `foo_avx2` into `foo` would attribute the vector variant's coverage
/// to the scalar path, which is the misattribution [`FuncKey`] exists to prevent.
pub trait MarchResolver {
    fn split(&self, symbol: &str) -> (String, Option<String>);
}

/// The resolver for a tree with no multiarch variants.
#[derive(Copy, Clone, Debug, Default)]
pub struct NoMarch;

impl MarchResolver for NoMarch {
    fn split(&self, symbol: &str) -> (String, Option<String>) {
        (symbol.to_string(), None)
    }
}

/// Line coverage **and** the arc-level data only the native path can produce (030 §5).
///
/// **Contract 4 is met by this type existing separately.** There is no `tests_for_arc` on
/// [`crate::CoverageIndex`], so asking a JSON-derived index for arcs does not compile — rather
/// than answering `None`, which a caller can read as "no tests took this arc" and act on.
///
/// Functions are keyed by name here. 030 §5's `FuncKey` adds the file, the start line and the
/// multiarch variant, which matter the moment two objects are merged; this is the single-object
/// shape and the key is where that grows.
#[derive(Clone, Debug, Default)]
pub struct ArcCoverage {
    index: crate::CoverageIndex,
    /// `(function, from, to) -> count`, real arcs only.
    counts: IndexMap<(FuncKey, u32, u32), u64>,
    /// The same keys, and who took them.
    tests: IndexMap<(FuncKey, u32, u32), Vec<crate::TestId>>,
    /// Arc order per function, so a query can list them as the CFG does.
    order: IndexMap<FuncKey, Vec<(u32, u32)>>,
}

impl ArcCoverage {
    /// The line-level index built from the same solve.
    pub fn index(&self) -> &crate::CoverageIndex {
        &self.index
    }

    /// The real arcs of a function, in the order the notes list them.
    ///
    /// `FAKE` arcs are not here: they run to the exit block from a call that may not return, so
    /// they are not control flow a test can be selected by (contract 7).
    pub fn arcs_of(&self, func: &FuncKey) -> Option<Vec<(u32, u32)>> {
        self.order.get(func).cloned()
    }

    /// How often an arc was taken, or `None` when the graph has no such real arc.
    pub fn arc_count(&self, func: &FuncKey, arc: (u32, u32)) -> Option<u64> {
        self.counts.get(&(func.clone(), arc.0, arc.1)).copied()
    }

    /// The tests that took an arc, or `None` when the graph has no such real arc.
    pub fn tests_for_arc(&self, func: &FuncKey, arc: (u32, u32)) -> Option<Vec<crate::TestId>> {
        self.tests.get(&(func.clone(), arc.0, arc.1)).cloned()
    }

    /// Every function this coverage knows about.
    pub fn functions(&self) -> Vec<&FuncKey> {
        self.order.keys().collect()
    }
}

/// Decode one object's arcs and lines together.
pub fn arc_coverage(dir: &Path, stem: &str) -> Result<ArcCoverage, IngestError> {
    let mut cov = ArcCoverage::default();
    arc_coverage_read(&mut cov, None, dir, stem)?;
    Ok(cov)
}

/// The same, attributing every arc it reads to `test`.
pub fn arc_coverage_into(
    cov: &mut ArcCoverage,
    test: crate::TestId,
    dir: &Path,
    stem: &str,
) -> Result<(), IngestError> {
    arc_coverage_read(cov, Some(test), dir, stem)
}

fn arc_coverage_read(
    cov: &mut ArcCoverage,
    test: Option<crate::TestId>,
    dir: &Path,
    stem: &str,
) -> Result<(), IngestError> {
    let notes_path = dir.join(format!("{stem}.gcno"));
    let data_path = dir.join(format!("{stem}.gcda"));
    for p in [&notes_path, &data_path] {
        if !p.exists() {
            return Err(IngestError::Missing { path: p.clone() });
        }
    }
    pair(&notes_path, &data_path)?;
    let notes = read_notes(&notes_path)?;
    let data = read_data(&data_path)?;

    // The line half goes through the ordinary ingest, so there is exactly one solve and one line
    // rule in this crate rather than a second copy that can drift from contract 5's gate.
    ingest_into(
        &mut cov.index,
        test.map(|t| (t, crate::Variant::None)),
        dir,
        stem,
    )?;

    for f in &notes.functions {
        let Some(d) = data.functions.iter().find(|d| d.ident == f.ident) else {
            continue; // `ingest_into` has already refused this file.
        };
        let arcs = solve_arcs(f, &d.counters, &data_path)?;
        // **The identity is built here, once.** `march` comes from the resolver rather than from
        // the artifacts, which carry only the pasted symbol.
        let (name, march) = NoMarch.split(&f.name);
        let key = FuncKey {
            file: f.source.clone(),
            name,
            start_line: f.start_line,
            march,
        };
        let order = cov.order.entry(key.clone()).or_default();
        for (i, a) in f.arcs.iter().enumerate() {
            // **Included in the solve above, excluded here** (contract 7). `solve_arcs` saw every
            // arc — conservation at the block a fake arc leaves does not balance without it — and
            // the query surface holds only the ones a program can take.
            if a.flags.contains(ArcFlags::FAKE) {
                continue;
            }
            let akey = (key.clone(), a.from, a.to);
            if !order.contains(&(a.from, a.to)) {
                order.push((a.from, a.to));
            }
            let slot = cov.counts.entry(akey.clone()).or_insert(0);
            *slot = slot.saturating_add(arcs[i]);
            // **Recorded for the test even when the arc was not taken.** The graph knows the arc
            // exists; only the traversal is absent, and that is the crate's absence-versus-zero
            // rule one level down from the line index.
            if let Some(t) = test {
                let set = cov.tests.entry(akey).or_default();
                if !set.contains(&t) {
                    set.push(t);
                }
            } else {
                cov.tests.entry(akey).or_default();
            }
        }
    }
    Ok(())
}

/// Block counts for one function, for the measurement harness in `examples/`.
pub fn debug_block_counts(f: &NoteFunction, counters: &[u64]) -> Option<Vec<u64>> {
    let arcs = solve_arcs(f, counters, Path::new("<probe>")).ok()?;
    Some(block_counts(f, &arcs))
}
