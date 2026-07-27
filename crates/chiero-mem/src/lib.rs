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
                *slot = join(*slot, to);
            }
        }
    }

    /// The first bit in the range that is not *definitely* initialized. `Cond` counts as
    /// not-definitely: the point of the third state is that it decides neither way.
    pub fn first_not_yes(&self, lo_bit: u64, n_bits: u64) -> Option<u64> {
        (lo_bit..lo_bit + n_bits).find(|&b| self.get(b) != InitBit::Yes)
    }
}

/// The initialization lattice: `No < Cond < Yes`, joined on write.
///
/// A conditional write is `ite(off == k, val, old)`. If `old` is already `Yes`, *both*
/// branches are initialized, so the result is `Yes` — assigning `Cond` unconditionally
/// would downgrade definitely-initialized memory and reintroduce the false-positive storm
/// on `v[i] = x; … use v[i]` that the tri-state exists to prevent. The join is
/// one-directional: over uninitialized memory a conditional write is still `Cond`.
fn join(old: InitBit, new: InitBit) -> InitBit {
    match (old, new) {
        (InitBit::Yes, _) | (_, InitBit::Yes) => InitBit::Yes,
        (InitBit::Cond, _) | (_, InitBit::Cond) => InitBit::Cond,
        _ => InitBit::No,
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
    /// An access wider than the payload can represent. Distinct from `OutOfBounds`
    /// because it is a *chiero* limit rather than a program error: the object might be
    /// large enough, and the caller still cannot be answered exactly.
    BadRange {
        want_bits: u64,
        max_bits: u64,
    },
    /// 021 §4: `readonly` globals reject writes with a finding, and contract 21 requires
    /// the bytes to be unchanged.
    ReadOnly {
        off: i64,
    },
}

/// The widest integer the byte and bit APIs can carry. Accesses beyond it are refused
/// rather than silently truncated — 020 permits `Int(512)` for AVX-512, so this is a real
/// boundary that `Const::Wide` exists to cross, not a theoretical one.
pub const MAX_ACCESS_BITS: u64 = 128;

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
        // i128 throughout. `size as i64` is a *wrapping* cast, so any size above 2^63
        // came out negative, the end landed at or below the offset, and the check passed
        // — turning `clib_memcpy(d, s, a - b)` with `a < b` into an in-bounds access.
        let end = off as i128 + size as i128;
        if off < 0 || end > self.size as i128 {
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
        self.check_writable(off)?;
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
        MemObject::check_int_width(size)?;
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
        MemObject::check_int_width(size)?;
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
        self.check_writable((lo_bit / 8) as i64)?;
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
        // The payload bound comes first: `v >> 128` is `v >> 0` when overflow checks are
        // off, so an over-wide field silently wrote bit 0 of the value into bit 128 of
        // the object. Refusing is honest; truncating is not.
        if n_bits > MAX_ACCESS_BITS {
            return Err(AccessError::BadRange {
                want_bits: n_bits,
                max_bits: MAX_ACCESS_BITS,
            });
        }
        // `lo_bit + n_bits` wrapped, the wrapped sum passed, and the indexing panicked.
        let end = lo_bit as u128 + n_bits as u128;
        if end > self.size as u128 * 8 {
            return Err(AccessError::OutOfBounds {
                off: (lo_bit / 8) as i64,
                size: n_bits.div_ceil(8),
                obj_size: self.size,
            });
        }
        Ok(())
    }

    /// Same payload bound for the byte-addressed integer API. Above 16 bytes the write
    /// duplicated the value's low bytes and the read silently narrowed, so the two were
    /// not inverses and neither said so.
    fn check_int_width(size: u64) -> Result<(), AccessError> {
        if size * 8 > MAX_ACCESS_BITS {
            return Err(AccessError::BadRange {
                want_bits: size * 8,
                max_bits: MAX_ACCESS_BITS,
            });
        }
        Ok(())
    }

    fn check_writable(&self, off: i64) -> Result<(), AccessError> {
        if self.readonly {
            return Err(AccessError::ReadOnly { off });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Concrete addresses and pointer provenance (021 §7, §7.1).
// ---------------------------------------------------------------------------

/// 021 §7. Objects are separated by this much so an OOB pointer does not land in another
/// object by accident and `PtrToInt` comparisons behave like a real program.
///
/// Chosen for OOB detection, **not** to mimic any real allocator's placement: 021 §7 is
/// explicit that no analysis may infer locality from these addresses. They are logical,
/// carry no timing meaning, and model nothing about caches, TLBs, NUMA or DMA.
pub const GUARD_GAP: u64 = 4096;

/// A pointer: an object identity plus a **signed** offset. Never a bare integer.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Pointer {
    pub base: ObjectId,
    pub off: i64,
}

/// An integer that may carry pointer provenance (021 §7.1).
///
/// The tag is what makes `uword_to_pointer` round-trips exact, and VPP does them
/// constantly. Without it, `IntToPtr` has only address-range search, which is wrong in
/// both directions — see [`AddressSpace::int_to_ptr`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum IntVal {
    /// A plain integer with no recorded provenance.
    Const(u64),
    /// The result of a `PtrToInt`, possibly with intervening arithmetic. `addr` is the
    /// concrete value the program would see; `from` is where it came from.
    Tagged { addr: u64, from: Pointer },
}

impl IntVal {
    pub fn addr(self) -> u64 {
        match self {
            IntVal::Const(a) => a,
            IntVal::Tagged { addr, .. } => addr,
        }
    }
}

/// Deterministic placement of objects, plus the `PtrToInt`/`IntToPtr` pair.
#[derive(Clone, Debug, Default)]
pub struct AddressSpace {
    /// `(id, addr, size)`, in allocation order.
    objs: Vec<(ObjectId, u64, u64)>,
    next_global: u64,
    next_heap: u64,
    next_stack: u64,
    next_lazy: u64,
    next_id: u32,
}

impl AddressSpace {
    pub fn new() -> AddressSpace {
        AddressSpace {
            objs: Vec::new(),
            next_global: 0x0000_1000_0000,
            next_heap: 0x0000_2000_0000,
            next_stack: 0x7fff_0000_0000,
            next_lazy: 0x0000_4000_0000,
            // 0 is NULL.
            next_id: 1,
        }
    }

    /// Place an object and return its id.
    ///
    /// A simple bump per region, seeded identically every run — **no randomization**,
    /// because determinism is a hard requirement (001 §5, contract 15) and a flaky
    /// address makes every `PtrToInt`-dependent branch look flaky.
    pub fn alloc(&mut self, kind: ObjKind, size: u64, align: u64, _span: Span) -> ObjectId {
        let bump = match kind {
            ObjKind::Global | ObjKind::Function => &mut self.next_global,
            ObjKind::Heap | ObjKind::Extern => &mut self.next_heap,
            ObjKind::Stack | ObjKind::VarArgs => &mut self.next_stack,
            ObjKind::Lazy => &mut self.next_lazy,
        };
        let a = align.max(1);
        let addr = bump.next_multiple_of(a);
        // The gap goes *after* the object, so the next allocation cannot abut it.
        *bump = addr + size + GUARD_GAP;
        let id = ObjectId(self.next_id);
        self.next_id += 1;
        self.objs.push((id, addr, size));
        id
    }

    pub fn addr_of(&self, id: ObjectId) -> Option<u64> {
        self.objs
            .iter()
            .find(|(i, _, _)| *i == id)
            .map(|(_, a, _)| *a)
    }

    fn size_of(&self, id: ObjectId) -> Option<u64> {
        self.objs
            .iter()
            .find(|(i, _, _)| *i == id)
            .map(|(_, _, s)| *s)
    }

    /// 021 §7.1: yields `addr + off` **and records the provenance in the value**.
    pub fn ptr_to_int(&self, p: Pointer) -> IntVal {
        let base = self.addr_of(p.base).unwrap_or(0);
        IntVal::Tagged {
            addr: base.wrapping_add(p.off as u64),
            from: p,
        }
    }

    /// Integer arithmetic that **carries the tag** (021 contract 12c).
    ///
    /// `(T*)((uword)p + 8 - 4)` must resolve to `p`'s object at offset 4. A tag that
    /// survived only a bare round trip would miss all of it, and VPP does this
    /// constantly.
    pub fn int_add(&self, v: IntVal, delta: i64) -> IntVal {
        match v {
            IntVal::Const(a) => IntVal::Const(a.wrapping_add(delta as u64)),
            IntVal::Tagged { addr, from } => IntVal::Tagged {
                addr: addr.wrapping_add(delta as u64),
                from: Pointer {
                    base: from.base,
                    off: from.off.wrapping_add(delta),
                },
            },
        }
    }

    /// 021 §7.1. **Provenance first, range search only on a miss.**
    ///
    /// Address-range search must never be the primary mechanism, because it is wrong in
    /// both directions:
    ///
    /// - It converts a real bug into a legitimate access. An object out of bounds by more
    ///   than a guard gap has an address inside an unrelated object, so the search returns
    ///   a valid in-bounds pointer there and the OOB write becomes a silent, legal-looking
    ///   write to the wrong object.
    /// - It reports a bug on conforming code. A page-aligned object of size exactly one
    ///   gap has its legal one-past-the-end pointer land in the gap, matching nothing.
    ///
    /// Guard gaps only bound OOB distances smaller than the gap, so no choice of gap
    /// fixes either case.
    pub fn int_to_ptr(&self, v: IntVal) -> Pointer {
        if let IntVal::Tagged { from, .. } = v {
            return from;
        }
        let a = v.addr();
        for (id, base, size) in &self.objs {
            // `<=` on the upper bound: one-past-the-end is a legal C pointer.
            if a >= *base && a <= base + size {
                return Pointer {
                    base: *id,
                    off: (a - base) as i64,
                };
            }
        }
        Pointer {
            base: ObjectId::UNBOUND,
            off: a as i64,
        }
    }

    /// Whether a pointer is within its own object — the check that stays meaningful
    /// precisely because provenance was not laundered.
    pub fn in_bounds(&self, p: Pointer, size: u64) -> bool {
        match self.size_of(p.base) {
            Some(s) => p.off >= 0 && p.off as u64 + size <= s,
            None => false,
        }
    }
}
