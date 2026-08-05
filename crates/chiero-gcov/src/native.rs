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
