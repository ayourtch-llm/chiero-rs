//! `chiero-mem` — the object/offset memory model (021).
//!
//! A pointer is an object identity plus a **signed** offset, never a bare integer. The
//! signedness is not generalization for its own sake: vppinfra puts the vector header
//! *below* the user pointer, so `vec_len(v)` reads `((vec_header_t *)v)[-1].len`, and a
//! model with unsigned offsets could not express that access at all.
//!
//! This module is the concrete-offset core: objects, byte contents, and the
//! initialization mask. Symbolic offsets, `Contents::Array` promotion, lifetime and
//! provenance build on it.

use chiero_span::Span;

/// An object's identity. Two reserved values are present in every state.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(pub u32);

impl ObjectId {
    /// Size 0 at address 0; any access is a null-dereference finding.
    pub const NULL: ObjectId = ObjectId(0);
    /// The target of an `IntToPtr` matching no known object; any access is a
    /// wild-pointer finding with `Fidelity::Unknown`.
    pub const UNBOUND: ObjectId = ObjectId(u32::MAX);
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ObjKind {
    Global,
    Stack,
    Heap,
    Extern,
    Lazy,
    /// Every `FuncId` gets a zero-size object so `AddrOfFunc` has somewhere to point.
    /// Without it there is no `Term` → `FuncId` mapping and the indirect-call resolution
    /// 023 §5 depends on cannot be implemented — VPP needs this constantly.
    Function,
    VarArgs,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Endian {
    Little,
    Big,
}

/// One bit's initialization status (021 §3.1).
///
/// The third state is required, not a refinement. A write at a symbolic offset that
/// stays in `Bytes` writes each candidate byte conditionally, and such a byte is neither
/// definitely initialized nor definitely not. Forcing it to `Yes` silently loses real
/// uninitialized reads; forcing it to `No` produces a false-positive storm on
/// `v[i] = x; … use v[i]`, which is ubiquitous.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum InitBit {
    No,
    Yes,
    /// Initialized iff a guard holds. The guard lives with the state's terms; this
    /// crate's concrete core only needs to know the status is not decided.
    Cond,
}

/// Whether a write is unconditional or guarded.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Cond {
    Always,
    Symbolic,
}

/// Bit-indexed initialization, length `8 * size` (021 §3.1).
///
/// Bit granularity is what makes `LoadBits`/`StoreBits` meaningful: a per-byte mask can
/// only answer "yes" for a whole bitfield word (missing every real uninitialized-bitfield
/// read) or "no" (firing on every correct one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitMask {
    bits: Vec<InitBit>,
}

impl InitMask {
    pub fn new(size: u64) -> InitMask {
        InitMask {
            bits: vec![InitBit::No; (size * 8) as usize],
        }
    }

    pub fn get(&self, bit: u64) -> InitBit {
        self.bits.get(bit as usize).copied().unwrap_or(InitBit::No)
    }

    pub fn set_range(&mut self, lo_bit: u64, n_bits: u64, to: InitBit) {
        for b in lo_bit..lo_bit + n_bits {
            if let Some(slot) = self.bits.get_mut(b as usize) {
                *slot = to;
            }
        }
    }

    /// The first bit in the range that is not *definitely* initialized. `Cond` counts as
    /// not-definitely: the point of the third state is that it decides neither way.
    pub fn first_not_yes(&self, lo_bit: u64, n_bits: u64) -> Option<u64> {
        (lo_bit..lo_bit + n_bits).find(|&b| self.get(b) != InitBit::Yes)
    }
}

/// Why an access could not be performed. Each carries enough to name the access in a
/// finding — a finding that cannot say *where* is not actionable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessError {
    OutOfBounds {
        off: i64,
        size: u64,
        obj_size: u64,
    },
    /// 021 §3.1: this yields a fresh symbol *plus* a finding, never zero. Silently
    /// reading zero is the single most common way a symbolic executor produces
    /// confidently wrong results.
    Uninitialized {
        off: i64,
        bit: u64,
    },
}

/// A memory object: a contiguous extent with byte contents and an init mask.
#[derive(Clone, Debug, PartialEq)]
pub struct MemObject {
    pub id: ObjectId,
    pub kind: ObjKind,
    pub size: u64,
    pub align: u64,
    pub readonly: bool,
    pub span: Span,
    data: Vec<u8>,
    init: InitMask,
}

impl MemObject {
    pub fn new(id: ObjectId, kind: ObjKind, size: u64, align: u64, span: Span) -> MemObject {
        MemObject {
            id,
            kind,
            size,
            align,
            readonly: false,
            span,
            data: vec![0; size as usize],
            init: InitMask::new(size),
        }
    }

    pub fn new_stack(id: ObjectId, size: u64, align: u64, span: Span) -> MemObject {
        MemObject::new(id, ObjKind::Stack, size, align, span)
    }

    pub fn init_bit(&self, bit: u64) -> InitBit {
        self.init.get(bit)
    }

    /// Bounds check for `[off, off + size)`.
    ///
    /// A **zero-size access one past the end is in bounds**: `memcpy(p, q, 0)` is legal
    /// C and one-past-the-end is exactly where a loop's final `p + n` lands. Rejecting it
    /// would report a finding on correct code at every loop exit.
    fn check(&self, off: i64, size: u64) -> Result<usize, AccessError> {
        let end = off.checked_add(size as i64);
        let oob = off < 0 || end.is_none_or(|e| e > self.size as i64);
        if oob {
            return Err(AccessError::OutOfBounds {
                off,
                size,
                obj_size: self.size,
            });
        }
        Ok(off as usize)
    }

    pub fn write_bytes(&mut self, off: i64, bytes: &[u8]) -> Result<(), AccessError> {
        self.write_bytes_cond(off, bytes, Cond::Always)
    }

    /// A conditional write marks the touched bits `Cond` rather than `Yes` — see
    /// [`InitBit`] for why the distinction cannot be collapsed.
    pub fn write_bytes_cond(
        &mut self,
        off: i64,
        bytes: &[u8],
        cond: Cond,
    ) -> Result<(), AccessError> {
        let at = self.check(off, bytes.len() as u64)?;
        self.data[at..at + bytes.len()].copy_from_slice(bytes);
        self.init.set_range(
            off as u64 * 8,
            bytes.len() as u64 * 8,
            match cond {
                Cond::Always => InitBit::Yes,
                Cond::Symbolic => InitBit::Cond,
            },
        );
        Ok(())
    }

    pub fn read_bytes(&self, off: i64, size: u64) -> Result<Vec<u8>, AccessError> {
        let at = self.check(off, size)?;
        if let Some(bit) = self.init.first_not_yes(off as u64 * 8, size * 8) {
            return Err(AccessError::Uninitialized { off, bit });
        }
        Ok(self.data[at..at + size as usize].to_vec())
    }

    /// Assemble `size` bytes into an integer in target byte order.
    pub fn read_int(&self, off: i64, size: u64, e: Endian) -> Result<u128, AccessError> {
        let b = self.read_bytes(off, size)?;
        Ok(match e {
            Endian::Little => b.iter().rev().fold(0u128, |a, &x| (a << 8) | x as u128),
            Endian::Big => b.iter().fold(0u128, |a, &x| (a << 8) | x as u128),
        })
    }

    pub fn write_int(
        &mut self,
        off: i64,
        size: u64,
        v: u128,
        e: Endian,
    ) -> Result<(), AccessError> {
        let mut b: Vec<u8> = (0..size).map(|i| (v >> (8 * i)) as u8).collect();
        if e == Endian::Big {
            b.reverse();
        }
        self.write_bytes(off, &b)
    }

    /// A bitfield write: `n_bits` starting at absolute bit index `lo_bit` (020 §4.5.1).
    ///
    /// Bit-addressed rather than byte-addressed, because that is the whole reason
    /// `StoreBits` is a distinct instruction — two fields in the same byte must be
    /// independently tracked.
    pub fn write_bits(&mut self, lo_bit: u64, n_bits: u64, v: u128) -> Result<(), AccessError> {
        self.check_bits(lo_bit, n_bits)?;
        for i in 0..n_bits {
            let bit = lo_bit + i;
            let (byte, sh) = ((bit / 8) as usize, bit % 8);
            let one = (v >> i) & 1;
            self.data[byte] = (self.data[byte] & !(1 << sh)) | ((one as u8) << sh);
        }
        self.init.set_range(lo_bit, n_bits, InitBit::Yes);
        Ok(())
    }

    pub fn read_bits(&self, lo_bit: u64, n_bits: u64) -> Result<u128, AccessError> {
        self.check_bits(lo_bit, n_bits)?;
        if let Some(bit) = self.init.first_not_yes(lo_bit, n_bits) {
            return Err(AccessError::Uninitialized {
                off: (lo_bit / 8) as i64,
                bit,
            });
        }
        let mut v = 0u128;
        for i in 0..n_bits {
            let bit = lo_bit + i;
            let one = (self.data[(bit / 8) as usize] >> (bit % 8)) & 1;
            v |= (one as u128) << i;
        }
        Ok(v)
    }

    fn check_bits(&self, lo_bit: u64, n_bits: u64) -> Result<(), AccessError> {
        if lo_bit + n_bits > self.size * 8 {
            return Err(AccessError::OutOfBounds {
                off: (lo_bit / 8) as i64,
                size: n_bits.div_ceil(8),
                obj_size: self.size,
            });
        }
        Ok(())
    }
}
