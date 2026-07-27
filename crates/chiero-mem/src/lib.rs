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

use chiero_solver::{CheckResult, Solver, SolverLite, Term, TermArena};
use chiero_span::Span;
use std::collections::BTreeMap;

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
    /// Initialized **iff the carried guard holds** (021 §3.1 writes this as
    /// `Cond(Term)`).
    ///
    /// The term is not decoration. Without it `MaybeUninitialized` is a report the
    /// engine can only accept or reject wholesale — the two outcomes §3.1 rejects — and
    /// promotion cannot build the init array, whose whole mapping is `No → 0`,
    /// `Yes → 1`, `Cond(t) → ite(t, 1, 0)`. `Term` is `Copy`, so this costs nothing.
    Cond(Term),
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
    /// `size` is in **bytes**; the mask holds eight entries per byte.
    ///
    /// Saturating rather than wrapping: `(size * 8) as usize` overflowed above 2^61 and
    /// panicked. The `MAX_MATERIALIZED_BYTES` guard lives in `Memory::alloc`, which is
    /// not the only way to reach this constructor.
    pub fn new(size: u64) -> InitMask {
        let n = usize::try_from(size.saturating_mul(8)).unwrap_or(usize::MAX);
        InitMask {
            bits: vec![InitBit::No; n],
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

    /// Set one bit's status verbatim, bypassing the join. Only for copying an existing
    /// mask (`realloc`), where the destination has no prior state to join with.
    pub fn set_exact(&mut self, bit: u64, to: InitBit) {
        if let Some(slot) = self.bits.get_mut(bit as usize) {
            *slot = to;
        }
    }

    /// The first **definitely uninitialized** bit in the range.
    pub fn first_no(&self, lo_bit: u64, n_bits: u64) -> Option<u64> {
        (lo_bit..lo_bit + n_bits).find(|&b| self.get(b) == InitBit::No)
    }

    /// The first *conditionally* initialized bit in the range.
    ///
    /// Kept separate from `first_no` because the two produce different findings: a `No`
    /// bit is a definite uninitialized read, a `Cond` bit is one the engine must
    /// discharge against the path condition. 021 contract 6b turns on exactly this —
    /// a read at a conditionally-written offset must not report a *definite* finding.
    pub fn first_cond(&self, lo_bit: u64, n_bits: u64) -> Option<u64> {
        (lo_bit..lo_bit + n_bits).find(|&b| matches!(self.get(b), InitBit::Cond(_)))
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
        // The incoming guard wins over an older one: a later conditional write is the
        // more recent word on whether the byte was written.
        (_, InitBit::Cond(t)) | (InitBit::Cond(t), _) => InitBit::Cond(t),
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
    /// The range touched a `Cond` bit but no `No` bit — conditionally initialized.
    MaybeUninitialized {
        off: i64,
        bit: u64,
    },
    /// The range holds a symbolic byte, which a concrete read cannot answer for.
    SymbolicByte {
        off: i64,
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
    /// Symbolic bytes at concrete offsets (021 §3's `sym` overlay). Sparse, because the
    /// overwhelming majority of bytes are concrete and must not pay for the few that
    /// are not.
    sym: BTreeMap<u64, Term>,
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
            sym: BTreeMap::new(),
        }
    }

    pub fn new_stack(id: ObjectId, size: u64, align: u64, span: Span) -> MemObject {
        MemObject::new(id, ObjKind::Stack, size, align, span)
    }

    pub fn init_bit(&self, bit: u64) -> InitBit {
        self.init.get(bit)
    }

    /// The bounds check on its own, so a caller can order it ahead of the alignment
    /// check the way 021 §5 requires.
    pub fn check_only(&self, off: i64, size: u64) -> Result<(), AccessError> {
        self.check(off, size).map(|_| ())
    }

    /// Bytes without the initialization check, so an uninitialized read can still hand
    /// back a value alongside its fault (021 §5). Bounds still apply.
    pub fn raw_bytes(&self, off: i64, size: u64) -> Option<Vec<u8>> {
        let at = self.check(off, size).ok()?;
        Some(self.data[at..at + size as usize].to_vec())
    }

    /// The first byte in the range holding a symbolic value, if any.
    pub fn first_symbolic(&self, off: i64, size: u64) -> Option<u64> {
        if off < 0 {
            return None;
        }
        (off as u64..off as u64 + size).find(|b| self.sym.contains_key(b))
    }

    /// Bits without the initialization check, so an uninitialized bit read can hand back
    /// a value alongside its fault.
    /// One concrete byte, ignoring initialization. For promotion, which copies the
    /// object's whole state verbatim.
    pub fn raw_byte(&self, b: u64) -> u8 {
        self.data.get(b as usize).copied().unwrap_or(0)
    }

    /// The symbolic byte at `b`, if the overlay holds one.
    pub fn sym_at(&self, b: u64) -> Option<Term> {
        self.sym.get(&b).copied()
    }

    pub fn raw_bits(&self, lo_bit: u64, n_bits: u64) -> Option<u128> {
        self.check_bits(lo_bit, n_bits).ok()?;
        let mut v = 0u128;
        for i in 0..n_bits {
            let bit = lo_bit + i;
            let one = (self.data[(bit / 8) as usize] >> (bit % 8)) & 1;
            v |= (one as u128) << i;
        }
        Some(v)
    }

    /// Record that a fresh symbol has been invented for this range, so a repeated read
    /// returns the same value and does not report the same defect twice (021 §5).
    pub fn memoize_fresh(&mut self, lo_bit: u64, n_bits: u64) {
        if self.check_bits(lo_bit, n_bits).is_err() {
            return;
        }
        // **Only `No` bits are memoized.** Upgrading a `Cond` bit discharges its guard in
        // chiero's favour, and it used to happen as a *side effect* of reading a
        // neighbouring byte — so the read that correctly reported a definite
        // uninitialized read silently laundered the conditional byte beside it.
        for b in lo_bit..lo_bit + n_bits {
            if self.init.get(b) == InitBit::No {
                self.init.set_exact(b, InitBit::Yes);
            }
        }
    }

    /// Copy an initialization range verbatim, so `realloc` preserves the *pair* of value
    /// and initialization status rather than silently marking the copy initialized.
    pub fn restore_init(&mut self, lo_bit: u64, bits: &[InitBit]) {
        for (i, b) in bits.iter().enumerate() {
            self.init.set_exact(lo_bit + i as u64, *b);
        }
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
        self.write_bytes_cond(off, bytes, Cond::Always, None)
    }

    /// A conditional write marks the touched bits `Cond` rather than `Yes` — see
    /// [`InitBit`] for why the distinction cannot be collapsed.
    pub fn write_bytes_cond(
        &mut self,
        off: i64,
        bytes: &[u8],
        cond: Cond,
        guard: Option<Term>,
    ) -> Result<(), AccessError> {
        let at = self.check(off, bytes.len() as u64)?;
        self.check_writable(off)?;
        self.data[at..at + bytes.len()].copy_from_slice(bytes);
        // A concrete write **wins**: leaving the overlay in place made `read_term` return
        // a stale symbol for a byte whose concrete value had just been replaced — a wrong
        // value, not a missing finding.
        for b in off as u64..off as u64 + bytes.len() as u64 {
            self.sym.remove(&b);
        }
        self.init.set_range(
            off as u64 * 8,
            bytes.len() as u64 * 8,
            match (cond, guard) {
                (Cond::Always, _) => InitBit::Yes,
                (Cond::Symbolic, Some(g)) => InitBit::Cond(g),
                // A conditional write with no guard cannot be represented honestly, and
                // the safe direction is "nobody definitely wrote this".
                (Cond::Symbolic, None) => InitBit::No,
            },
        );
        Ok(())
    }

    pub fn read_bytes(&self, off: i64, size: u64) -> Result<Vec<u8>, AccessError> {
        let at = self.check(off, size)?;
        if let Some(bit) = self.init.first_no(off as u64 * 8, size * 8) {
            return Err(AccessError::Uninitialized { off, bit });
        }
        if let Some(bit) = self.init.first_cond(off as u64 * 8, size * 8) {
            return Err(AccessError::MaybeUninitialized { off, bit });
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
        if let Some(bit) = self.init.first_no(lo_bit, n_bits) {
            return Err(AccessError::Uninitialized {
                off: (lo_bit / 8) as i64,
                bit,
            });
        }
        if let Some(bit) = self.init.first_cond(lo_bit, n_bits) {
            return Err(AccessError::MaybeUninitialized {
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
        // Saturating: an exabyte-sized object is refused by `Memory`, but the address
        // space must not wrap on the way to finding that out.
        *bump = addr.saturating_add(size).saturating_add(GUARD_GAP);
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

// ---------------------------------------------------------------------------
// The access API and object lifetime (021 §4, §5).
// ---------------------------------------------------------------------------

/// Above this, an object is not materialized and every access to it faults.
///
/// `MemObject` allocates `size` bytes plus eight times that for the init mask, so an
/// unconstrained `clib_mem_alloc(n)` used to abort the process — and an abort is not
/// something `catch_unwind` can contain. 023 §10 concretizes symbolic sizes from a solver
/// model, which can hand back anything the constraints allow.
pub const MAX_MATERIALIZED_BYTES: u64 = 1 << 30;

/// 020 §4: `memcpy` forbids overlap, `memmove` permits it. Collapsing the two loses a
/// real bug in one direction and invents one in the other.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Overlap {
    Forbidden,
    Allowed,
}

/// 021 §4. Objects are **never deleted**; only this changes.
///
/// Keeping a freed object is what lets a dangling access name which object ended and
/// where. Deleting it would leave only an address matching nothing, indistinguishable
/// from a wild pointer.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ObjState {
    Live,
    Freed(Span),
    OutOfScope(Span),
}

/// What went wrong, or merely what was noticed. Reported by the memory model; the engine
/// decides what a fault *means* (021 §5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemFault {
    OutOfBounds {
        obj: ObjectId,
        off: i64,
        size: u64,
        obj_size: u64,
        at: Span,
    },
    Uninitialized {
        obj: ObjectId,
        off: i64,
        bit: u64,
        at: Span,
    },
    /// The read touched a byte written only under a guard (`InitBit::Cond`).
    ///
    /// The third state needs a third *outcome*, or it collapses back into one of the two
    /// 021 §3.1 rejects: reporting definitely loses to the false-positive storm on
    /// `v[i] = x; … use v[i]`, and staying silent loses real uninitialized reads. The
    /// guard is the engine's to discharge against the path condition, not the memory
    /// model's to guess.
    MaybeUninitialized {
        obj: ObjectId,
        off: i64,
        bit: u64,
        at: Span,
    },
    /// Recorded on every misaligned access; a *finding* only in `ub-strict` mode, since
    /// x86-64 tolerates it and VPP relies on that in places.
    Misaligned {
        obj: ObjectId,
        off: i64,
        want: u64,
        at: Span,
    },
    UseAfterFree {
        obj: ObjectId,
        freed_at: Span,
        at: Span,
    },
    DoubleFree {
        obj: ObjectId,
        freed_at: Span,
        at: Span,
    },
    UseAfterScope {
        obj: ObjectId,
        scope_ended_at: Span,
        at: Span,
    },
    ReadOnly {
        obj: ObjectId,
        off: i64,
        at: Span,
    },
    /// A chiero payload limit rather than a program error.
    BadRange {
        want_bits: u64,
        max_bits: u64,
        at: Span,
    },
    AllocationTooLarge {
        obj: ObjectId,
        size: u64,
        at: Span,
    },
    NullDeref {
        off: i64,
        at: Span,
    },
    /// 021 §1: an access through `ObjectId::UNBOUND` — a pointer produced by `IntToPtr`
    /// from a value matching no known object. Distinct from a null dereference: different
    /// bug, different cause, `Fidelity::Unknown` rather than a definite finding.
    WildPointer {
        off: i64,
        at: Span,
    },
    /// A concrete read touched a byte that holds a *symbolic* value.
    ///
    /// The byte API cannot answer, and inventing a concrete zero is what 021 §3 names as
    /// the single most common way a symbolic executor produces confidently wrong results.
    /// The caller wants `read_term`.
    SymbolicByte {
        obj: ObjectId,
        off: i64,
        at: Span,
    },
    /// 021 §5 step 2: the access **may** be out of bounds. Distinct from `OutOfBounds`,
    /// which is definite — this one continues on the in-bounds branch, and a reader
    /// needs to know the difference between "this is wrong" and "this can be wrong".
    OutOfBoundsMaybe {
        obj: ObjectId,
        size: u64,
        obj_size: u64,
        /// A concrete offset that is actually out of bounds. "Some value of `i` is out
        /// of bounds" is not a bug report anyone can act on.
        witness: i64,
        at: Span,
    },
    /// 021 contract 22: a `memcpy` whose ranges overlap.
    OverlappingCopy {
        obj: ObjectId,
        dst: i64,
        src: i64,
        size: u64,
        at: Span,
    },
    /// `free()` of something that did not come from the heap.
    BadFree {
        obj: ObjectId,
        kind: ObjKind,
        at: Span,
    },
}

/// **Faults alongside a value, not instead of one** (021 §5).
///
/// `Result<_, MemFault>` cannot express the normal case: an uninitialized read yields a
/// value *and* a finding, a misaligned access is recorded *and* succeeds, a may-OOB
/// access reports *and* continues on the in-bounds branch. Several faults per access are
/// possible — misaligned and partially uninitialized is ordinary — so this is a vector.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccessResult<T> {
    pub value: Option<T>,
    pub faults: Vec<MemFault>,
}

impl<T> AccessResult<T> {
    fn fault(f: MemFault) -> AccessResult<T> {
        AccessResult {
            value: None,
            faults: vec![f],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leak {
    pub obj: ObjectId,
    pub allocated_at: Span,
}

/// 021 §3. Objects start as `Bytes`; promotion is one-way within a state.
/// A promoted object's contents: SMT arrays for the bytes and for initialization
/// (021 §3, §3.1).
///
/// The init array is **bit-indexed**, matching `InitMask`, so `LoadBits` keeps the same
/// resolution after promotion that it had before. Promotion maps `No → 0`, `Yes → 1`,
/// `Cond(t) → ite(t, 1, 0)` — which is exactly why `InitBit` has to carry its guard.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ArrayContents {
    pub data: Term,
    pub init: Term,
    pub idx_bits: u32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Repr {
    /// Concrete bytes, a bit-indexed init mask, and a sparse overlay of symbolic bytes
    /// at concrete offsets. The fast path, and the one nearly every VPP access takes.
    Bytes,
    /// SMT array with a parallel init array. Paid for only when a write at a symbolic
    /// offset cannot be pinned to a small set.
    Array,
}

/// 021 §3: reads at a symbolic offset are answered by an if-then-else chain when the
/// feasible set is at most this large, and force promotion otherwise.
///
/// A **documented constant**, not a heuristic. An object that promoted on one run and not
/// the next would answer the same program differently for no reason a reader could see,
/// and 001 §5 makes determinism a hard requirement.
pub const ITE_THRESHOLD: usize = 16;

#[derive(Clone, Debug)]
struct Entry {
    obj: Option<MemObject>,
    repr: Repr,
    /// Present exactly when `repr == Repr::Array`.
    arr: Option<ArrayContents>,
    kind: ObjKind,
    size: u64,
    align: u64,
    state: ObjState,
    readonly: bool,
    origin: Span,
    /// Pointers this object holds, **keyed by slot offset** (021 §4's leak rule).
    ///
    /// Keyed, not appended: `p->next = q` when `p->next` already pointed somewhere must
    /// *drop* the old edge, or the old target stays reachable forever and leaks go
    /// systematically under-reported after any pointer store.
    points_to: Vec<(i64, ObjectId)>,
    /// An explicitly declared root — the return value, say. Globals and live stack
    /// objects are roots without being declared (021 §4).
    root: bool,
}

/// The object store: lifetime, access, and leak reachability.
#[derive(Clone, Debug, Default)]
pub struct Memory {
    space: AddressSpace,
    entries: Vec<(ObjectId, Entry)>,
}

impl Memory {
    pub fn new() -> Memory {
        Memory {
            space: AddressSpace::new(),
            entries: Vec::new(),
        }
    }

    pub fn alloc(&mut self, kind: ObjKind, size: u64, align: u64, at: Span) -> ObjectId {
        // The **true** size goes to the address space. Truncating it there while the
        // entry recorded the real one made `int_to_ptr`'s range search and `in_bounds`
        // disagree with the object about how big it is.
        let id = self.space.alloc(kind, size, align, at);
        // Oversized objects are recorded but not materialized: every access faults, which
        // is a finding rather than a dead process.
        let obj =
            (size <= MAX_MATERIALIZED_BYTES).then(|| MemObject::new(id, kind, size, align, at));
        self.entries.push((
            id,
            Entry {
                obj,
                repr: Repr::Bytes,
                arr: None,
                kind,
                size,
                align,
                state: ObjState::Live,
                readonly: false,
                origin: at,
                points_to: Vec::new(),
                root: false,
            },
        ));
        id
    }

    fn entry_mut(&mut self, id: ObjectId) -> Option<&mut Entry> {
        self.entries
            .iter_mut()
            .find(|(i, _)| *i == id)
            .map(|(_, e)| e)
    }

    fn entry(&self, id: ObjectId) -> Option<&Entry> {
        self.entries.iter().find(|(i, _)| *i == id).map(|(_, e)| e)
    }

    pub fn set_readonly(&mut self, id: ObjectId) {
        if let Some(e) = self.entry_mut(id) {
            e.readonly = true;
            // Mirrored onto the object so *every* write path sees it. Two independent
            // flags is how contract 21 came to hold for one write path out of three.
            if let Some(o) = e.obj.as_mut() {
                o.readonly = true;
            }
        }
    }

    pub fn set_root(&mut self, id: ObjectId) {
        if let Some(e) = self.entry_mut(id) {
            e.root = true;
        }
    }

    /// Record that `from`'s pointer slot at `slot` holds a pointer to `to`, replacing
    /// whatever that slot held before.
    pub fn set_pointer(&mut self, from: ObjectId, slot: i64, to: ObjectId) {
        if let Some(e) = self.entry_mut(from) {
            match e.points_to.iter_mut().find(|(s, _)| *s == slot) {
                Some(x) => x.1 = to,
                None => e.points_to.push((slot, to)),
            }
        }
    }

    /// **021 §5 step 1.** Runs before anything touches contents, so a dangling access
    /// never reads stale bytes and never *also* reports "uninitialized" about memory it
    /// had no business touching.
    fn state_fault(&self, id: ObjectId, off: i64, at: Span) -> Option<MemFault> {
        if id == ObjectId::NULL {
            return Some(MemFault::NullDeref { off, at });
        }
        if id == ObjectId::UNBOUND {
            return Some(MemFault::WildPointer { off, at });
        }
        let e = self.entry(id)?;
        match e.state {
            ObjState::Live => None,
            ObjState::Freed(freed_at) => Some(MemFault::UseAfterFree {
                obj: id,
                freed_at,
                at,
            }),
            ObjState::OutOfScope(scope_ended_at) => Some(MemFault::UseAfterScope {
                obj: id,
                scope_ended_at,
                at,
            }),
        }
    }

    /// **021 §5 step 3.** Always recorded; whether it is a *finding* is `ub-strict`'s
    /// call, not the memory model's.
    fn align_fault(&self, id: ObjectId, off: i64, size: u64, at: Span) -> Option<MemFault> {
        let e = self.entry(id)?;
        // **The requirement comes from the access, not the object.** An N-byte scalar
        // wants N-byte alignment when N is a power of two; a 3-byte access has no
        // requirement at all, and reporting `want: 3` was reporting a number that is not
        // an alignment. The old `min(object_align, size)` was wrong in both directions —
        // it invented that false positive *and* it made misalignment unrecordable inside
        // an align-1 object, which is every VPP packet buffer.
        let want = if size.is_power_of_two() && size <= 16 {
            size
        } else {
            return None;
        };
        // The object's own base bounds what any offset can guarantee.
        let effective = want.min(e.align.max(1));
        (!off.unsigned_abs().is_multiple_of(want) || effective < want).then_some(
            MemFault::Misaligned {
                obj: id,
                off,
                want,
                at,
            },
        )
    }

    fn too_large(&self, id: ObjectId, at: Span) -> Option<MemFault> {
        let e = self.entry(id)?;
        e.obj.is_none().then_some(MemFault::AllocationTooLarge {
            obj: id,
            size: e.size,
            at,
        })
    }

    pub fn read(&mut self, p: Pointer, size: u64, at: Span) -> AccessResult<Vec<u8>> {
        if let Some(f) = self.state_fault(p.base, p.off, at) {
            return AccessResult::fault(f);
        }
        if let Some(f) = self.too_large(p.base, at) {
            return AccessResult::fault(f);
        }
        let Some(e) = self.entry(p.base) else {
            return AccessResult::fault(MemFault::NullDeref { off: p.off, at });
        };
        // A promoted object's contents live in its arrays, and the `Bytes` view beneath
        // is frozen at the moment of promotion. Answering from it would be the drift this
        // representation exists to avoid, and the byte API has no arena with which to
        // consult the arrays — so it says it cannot serve this object. The caller wants
        // `read_term` and `init_bit_via`.
        if e.repr == Repr::Array {
            return AccessResult::fault(MemFault::SymbolicByte {
                obj: p.base,
                off: p.off,
                at,
            });
        }
        let obj = e.obj.as_ref().expect("materialized");
        // 021 §5's order is state, **bounds**, alignment, init. Bounds first is not
        // cosmetic: a concrete must-OOB access does not happen, so reporting its
        // alignment alongside would describe an access that never occurs.
        if let Err(err @ AccessError::OutOfBounds { .. }) = obj.read_bytes(p.off, size) {
            return AccessResult::fault(lift(err, p.base, at));
        }
        let mut faults: Vec<MemFault> = self
            .align_fault(p.base, p.off, size, at)
            .into_iter()
            .collect();
        let Some(e) = self.entry(p.base) else {
            return AccessResult::fault(MemFault::NullDeref { off: p.off, at });
        };
        let obj = e.obj.as_ref().expect("materialized");
        // A concrete read cannot *answer* for a symbolic byte — the `data` zero behind it
        // is stale — but the initialization story is still worth telling, so this is an
        // extra fault rather than a replacement. The caller wants `read_term`.
        if let Some(b) = obj.first_symbolic(p.off, size) {
            faults.push(MemFault::SymbolicByte {
                obj: p.base,
                off: b as i64,
                at,
            });
        }
        match obj.read_bytes(p.off, size) {
            Ok(v) => AccessResult {
                value: Some(v),
                faults,
            },
            // A conditionally-initialized read still produces a value; only the *kind*
            // of finding differs. Falling into the generic error branch dropped the
            // value, which is the one thing this API exists not to do.
            Err(AccessError::MaybeUninitialized { off, bit }) => {
                faults.push(MemFault::MaybeUninitialized {
                    obj: p.base,
                    off,
                    bit,
                    at,
                });
                // No memoization here: the guard is still live, and marking the byte
                // definitely initialized would silently discharge it in chiero's favour.
                AccessResult {
                    value: obj.raw_bytes(p.off, size),
                    faults,
                }
            }
            Err(AccessError::Uninitialized { off, bit }) => {
                faults.push(MemFault::Uninitialized {
                    obj: p.base,
                    off,
                    bit,
                    at,
                });
                // **A value as well as a fault.** The engine gets a fresh symbol here;
                // the concrete core hands back the bytes so it has something to carry.
                let v = obj.raw_bytes(p.off, size);
                if p.off >= 0 {
                    self.memoize(p.base, p.off as u64 * 8, size * 8);
                }
                AccessResult { value: v, faults }
            }
            Err(e) => {
                faults.push(lift(e, p.base, at));
                // Out of bounds under every model: there is no in-bounds branch left to
                // continue on (contract 2).
                AccessResult {
                    value: None,
                    faults,
                }
            }
        }
    }

    pub fn write(&mut self, p: Pointer, bytes: &[u8], at: Span) -> AccessResult<()> {
        if let Some(f) = self.state_fault(p.base, p.off, at) {
            return AccessResult::fault(f);
        }
        if let Some(f) = self.too_large(p.base, at) {
            return AccessResult::fault(f);
        }
        if let Some(e) = self.entry(p.base)
            && let Some(o) = e.obj.as_ref()
            && let Err(err @ AccessError::OutOfBounds { .. }) =
                o.check_only(p.off, bytes.len() as u64)
        {
            return AccessResult::fault(lift(err, p.base, at));
        }
        // A promoted object's contents live in its arrays; writing the frozen `Bytes`
        // view would be invisible, which is the same drift `read` refuses on its side.
        if let Some(f) = self.promoted_fault(p, at) {
            return AccessResult::fault(f);
        }
        let mut faults: Vec<MemFault> = self
            .align_fault(p.base, p.off, bytes.len() as u64, at)
            .into_iter()
            .collect();
        let ro = self.entry(p.base).is_some_and(|e| e.readonly);
        let Some(e) = self.entry_mut(p.base) else {
            return AccessResult::fault(MemFault::NullDeref { off: p.off, at });
        };
        if ro {
            faults.push(MemFault::ReadOnly {
                obj: p.base,
                off: p.off,
                at,
            });
            return AccessResult {
                value: None,
                faults,
            };
        }
        let obj = e.obj.as_mut().expect("materialized");
        match obj.write_bytes(p.off, bytes) {
            Ok(()) => AccessResult {
                value: Some(()),
                faults,
            },
            Err(err) => {
                faults.push(lift(err, p.base, at));
                AccessResult {
                    value: None,
                    faults,
                }
            }
        }
    }

    /// Bit-granular access at a **signed** byte offset (021 §3.1 with §1's premise).
    ///
    /// The byte offset is signed and the bit offset within it is not, which is what makes
    /// `((vec_header_t *)v)[-1].flags` expressible — the founding case the old
    /// unsigned-only bit API could not spell.
    pub fn read_bits(
        &mut self,
        p: Pointer,
        lo_bit: u64,
        n_bits: u64,
        at: Span,
    ) -> AccessResult<u128> {
        if let Some(f) = self.state_fault(p.base, p.off, at) {
            return AccessResult::fault(f);
        }
        if let Some(f) = self.too_large(p.base, at) {
            return AccessResult::fault(f);
        }
        let obj_size = self.entry(p.base).map_or(0, |e| e.size);
        let Some(b) = abs_bit(p.off, lo_bit) else {
            return AccessResult::fault(MemFault::OutOfBounds {
                obj: p.base,
                off: p.off,
                size: n_bits.div_ceil(8),
                obj_size,
                at,
            });
        };
        // The bit API runs the same five steps as the byte API. Skipping the alignment
        // check made a bitfield access silently exempt from 021 §5 step 3.
        let mut faults: Vec<MemFault> = self
            .align_fault(p.base, p.off, n_bits.div_ceil(8), at)
            .into_iter()
            .collect();
        let Some(e) = self.entry(p.base) else {
            return AccessResult::fault(MemFault::WildPointer { off: p.off, at });
        };
        let obj = e.obj.as_ref().expect("materialized");
        match obj.read_bits(b, n_bits) {
            Ok(v) => AccessResult {
                value: Some(v),
                faults,
            },
            Err(AccessError::MaybeUninitialized { off, bit }) => {
                faults.push(MemFault::MaybeUninitialized {
                    obj: p.base,
                    off,
                    bit,
                    at,
                });
                AccessResult {
                    value: obj.raw_bits(b, n_bits),
                    faults,
                }
            }
            Err(AccessError::Uninitialized { off, bit }) => {
                faults.push(MemFault::Uninitialized {
                    obj: p.base,
                    off,
                    bit,
                    at,
                });
                // **A value as well as a fault**, exactly as the byte API does. Returning
                // one instead of the other is the thing this API exists not to do.
                let v = obj.raw_bits(b, n_bits);
                self.memoize(p.base, b, n_bits);
                AccessResult { value: v, faults }
            }
            Err(err) => {
                faults.push(lift(err, p.base, at));
                AccessResult {
                    value: None,
                    faults,
                }
            }
        }
    }

    pub fn write_bits(
        &mut self,
        p: Pointer,
        lo_bit: u64,
        n_bits: u64,
        v: u128,
        at: Span,
    ) -> AccessResult<()> {
        if let Some(f) = self.state_fault(p.base, p.off, at) {
            return AccessResult::fault(f);
        }
        if let Some(f) = self.too_large(p.base, at) {
            return AccessResult::fault(f);
        }
        if let Some(f) = self.promoted_fault(p, at) {
            return AccessResult::fault(f);
        }
        let obj_size = self.entry(p.base).map_or(0, |e| e.size);
        let Some(b) = abs_bit(p.off, lo_bit) else {
            return AccessResult::fault(MemFault::OutOfBounds {
                obj: p.base,
                off: p.off,
                size: n_bits.div_ceil(8),
                obj_size,
                at,
            });
        };
        // **One source of truth for `readonly`.** There were two independent fields — one
        // on `MemObject`, one on the entry — and this path consulted the one nothing ever
        // set, so contract 21 failed in both halves: no finding, and the bytes changed.
        if self.entry(p.base).is_some_and(|e| e.readonly) {
            return AccessResult::fault(MemFault::ReadOnly {
                obj: p.base,
                off: p.off,
                at,
            });
        }
        let mut faults: Vec<MemFault> = self
            .align_fault(p.base, p.off, n_bits.div_ceil(8), at)
            .into_iter()
            .collect();
        let Some(e) = self.entry_mut(p.base) else {
            return AccessResult::fault(MemFault::WildPointer { off: p.off, at });
        };
        let obj = e.obj.as_mut().expect("materialized");
        match obj.write_bits(b, n_bits, v) {
            Ok(()) => AccessResult {
                value: Some(()),
                faults,
            },
            Err(err) => {
                faults.push(lift(err, p.base, at));
                AccessResult {
                    value: None,
                    faults,
                }
            }
        }
    }

    /// 021 contract 22. `Overlap::Forbidden` is `memcpy`; `Overlap::Allowed` is
    /// `memmove` and copies as if through a temporary.
    pub fn copy(
        &mut self,
        dst: Pointer,
        src: Pointer,
        size: u64,
        overlap: Overlap,
        at: Span,
    ) -> AccessResult<()> {
        // Both ends run the same five steps; the source is read and the destination
        // written, so a copy out of freed memory is a use-after-free like any other.
        let read = self.read_raw(src, size, at);
        let mut faults = read.faults;
        let Some((bytes, init)) = read.value else {
            return AccessResult {
                value: None,
                faults,
            };
        };
        if overlap == Overlap::Forbidden
            && dst.base == src.base
            && ranges_overlap(dst.off, src.off, size)
        {
            faults.push(MemFault::OverlappingCopy {
                obj: dst.base,
                dst: dst.off,
                src: src.off,
                size,
                at,
            });
        }
        // `bytes` was snapshotted before any write, which is exactly the temporary
        // `memmove` is defined in terms of.
        let w = self.write(dst, &bytes, at);
        faults.extend(w.faults);
        if w.value.is_none() {
            return AccessResult {
                value: None,
                faults,
            };
        }
        // The **pair** carries across, not just the bytes: marking the destination
        // initialized would launder an uninitialized source.
        if dst.off >= 0
            && let Some(e) = self.entry_mut(dst.base)
            && let Some(o) = e.obj.as_mut()
        {
            o.restore_init(dst.off as u64 * 8, &init);
        }
        AccessResult {
            value: Some(()),
            faults,
        }
    }

    /// 021 contract 28: the range becomes initialized and reads back as the set byte.
    pub fn set(&mut self, dst: Pointer, byte: u8, size: u64, at: Span) -> AccessResult<()> {
        let bytes = vec![byte; size as usize];
        self.write(dst, &bytes, at)
    }

    /// The source side of a `copy`: state, bounds and alignment as usual, but **no
    /// uninitialized-read fault and no memoization**.
    ///
    /// A copy moves bytes without using them. `memcpy` of a partially-filled struct is
    /// ubiquitous and correct, so reporting there is a false-positive storm — and
    /// memoizing would mark the source initialized, defeating the propagation this
    /// function exists for. The finding belongs at the eventual *use* of the destination,
    /// which is why the status is carried rather than consumed.
    fn read_raw(
        &mut self,
        p: Pointer,
        size: u64,
        at: Span,
    ) -> AccessResult<(Vec<u8>, Vec<InitBit>)> {
        if let Some(f) = self.state_fault(p.base, p.off, at) {
            return AccessResult::fault(f);
        }
        if let Some(f) = self.too_large(p.base, at) {
            return AccessResult::fault(f);
        }
        let Some(e) = self.entry(p.base) else {
            return AccessResult::fault(MemFault::WildPointer { off: p.off, at });
        };
        let obj = e.obj.as_ref().expect("materialized");
        let Some(bytes) = obj.raw_bytes(p.off, size) else {
            return AccessResult::fault(MemFault::OutOfBounds {
                obj: p.base,
                off: p.off,
                size,
                obj_size: e.size,
                at,
            });
        };
        let init = (0..size * 8)
            .map(|b| obj.init_bit(p.off as u64 * 8 + b))
            .collect::<Vec<_>>();
        AccessResult {
            value: Some((bytes, init)),
            faults: vec![],
        }
    }

    /// A promoted object cannot be served by an arena-free byte or bit API.
    fn promoted_fault(&self, p: Pointer, at: Span) -> Option<MemFault> {
        self.entry(p.base)
            .filter(|e| e.repr == Repr::Array)
            .map(|_| MemFault::SymbolicByte {
                obj: p.base,
                off: p.off,
                at,
            })
    }

    /// Whether this object is still on the `Bytes` fast path (021 §3).
    pub fn is_bytes(&self, id: ObjectId) -> bool {
        self.entry(id).is_some_and(|e| e.repr == Repr::Bytes)
    }

    pub fn init_bit_of(&self, id: ObjectId, bit: u64) -> InitBit {
        self.entry(id)
            .and_then(|e| e.obj.as_ref())
            .map_or(InitBit::No, |o| o.init_bit(bit))
    }

    /// The same question against a promoted object's init array, which requires an arena
    /// to fold the select. A `Bytes` object answers from the mask.
    pub fn init_bit_via(&self, a: &mut TermArena, id: ObjectId, bit: u64) -> InitBit {
        let Some(e) = self.entry(id) else {
            return InitBit::No;
        };
        let Some(arr) = e.arr else {
            return self.init_bit_of(id, bit);
        };
        let i = a.bv(arr.idx_bits, bit as u128);
        let got = a.select(arr.init, i);
        // Folded to a literal? Then the guard collapsed, which 021 §3.1 says it does
        // "whenever its guard folds to a constant".
        match a.eval_ground(got) {
            Ok(v) if v.bits() == 1 => InitBit::Yes,
            Ok(_) => InitBit::No,
            Err(_) => {
                let one = a.bv(1, 1);
                InitBit::Cond(a.eq(got, one))
            }
        }
    }

    /// Place a symbolic byte at a concrete offset (021 §3's `sym` overlay).
    pub fn write_sym_byte(&mut self, p: Pointer, t: Term, at: Span) -> AccessResult<()> {
        if let Some(f) = self.state_fault(p.base, p.off, at) {
            return AccessResult::fault(f);
        }
        if let Some(f) = self.promoted_fault(p, at) {
            return AccessResult::fault(f);
        }
        let Some(e) = self.entry_mut(p.base) else {
            return AccessResult::fault(MemFault::WildPointer { off: p.off, at });
        };
        let Some(o) = e.obj.as_mut() else {
            return AccessResult::fault(MemFault::AllocationTooLarge {
                obj: p.base,
                size: 0,
                at,
            });
        };
        if o.check_only(p.off, 1).is_err() || p.off < 0 {
            return AccessResult::fault(MemFault::OutOfBounds {
                obj: p.base,
                off: p.off,
                size: 1,
                obj_size: e.size,
                at,
            });
        }
        if o.readonly {
            return AccessResult::fault(MemFault::ReadOnly {
                obj: p.base,
                off: p.off,
                at,
            });
        }
        o.sym.insert(p.off as u64, t);
        o.init.set_range(p.off as u64 * 8, 8, InitBit::Yes);
        AccessResult {
            value: Some(()),
            faults: vec![],
        }
    }

    /// Assemble `size` bytes into a term in target byte order (021 contract 5).
    ///
    /// Concrete bytes become constants and symbolic ones come from the overlay, so a
    /// wholly concrete read folds to a constant and a mixed read is a `Concat` whose
    /// concrete halves stay concrete. Promotion is *not* triggered by this: reads at
    /// concrete offsets are the fast path the whole representation exists for.
    pub fn read_term(
        &mut self,
        a: &mut TermArena,
        p: Pointer,
        size: u64,
        e: Endian,
        at: Span,
    ) -> AccessResult<Term> {
        if let Some(f) = self.state_fault(p.base, p.off, at) {
            return AccessResult::fault(f);
        }
        let obj_size = self.entry(p.base).map_or(0, |x| x.size);
        let Some(entry) = self.entry(p.base) else {
            return AccessResult::fault(MemFault::WildPointer { off: p.off, at });
        };
        let Some(o) = entry.obj.as_ref() else {
            return AccessResult::fault(MemFault::AllocationTooLarge {
                obj: p.base,
                size: obj_size,
                at,
            });
        };
        if p.off < 0 || o.check_only(p.off, size).is_err() {
            return AccessResult::fault(MemFault::OutOfBounds {
                obj: p.base,
                off: p.off,
                size,
                obj_size,
                at,
            });
        }
        // 021 §5 step 3 applies to every read path, not just the byte one.
        let mut faults: Vec<MemFault> = self
            .align_fault(p.base, p.off, size, at)
            .into_iter()
            .collect();
        // Initialization comes from whichever representation is live. A promoted object's
        // mask is frozen, so consulting it would report the state as of promotion rather
        // than the state now.
        let range = p.off as u64 * 8..p.off as u64 * 8 + size * 8;
        let mut first_no = None;
        let mut first_cond = None;
        for bit in range {
            match self.init_bit_via(a, p.base, bit) {
                InitBit::No if first_no.is_none() => first_no = Some(bit),
                InitBit::Cond(_) if first_cond.is_none() => first_cond = Some(bit),
                _ => {}
            }
        }
        if let Some(bit) = first_no {
            faults.push(MemFault::Uninitialized {
                obj: p.base,
                off: p.off,
                bit,
                at,
            });
            // 021 §5 / contract 26: the fresh symbol is memoized, so a repeated read is
            // the same value and not a second finding. `read` did this and `read_term`
            // did not, which left the two APIs disagreeing about the same byte.
            if p.off >= 0 {
                self.memoize_via(a, p.base, p.off as u64 * 8, size * 8);
            }
        } else if let Some(bit) = first_cond {
            faults.push(MemFault::MaybeUninitialized {
                obj: p.base,
                off: p.off,
                bit,
                at,
            });
        }
        // SMT-LIB `concat` puts its first argument in the high bits, so the most
        // significant byte is folded in first.
        let idx: Vec<u64> = match e {
            Endian::Little => (0..size).rev().map(|i| p.off as u64 + i).collect(),
            Endian::Big => (0..size).map(|i| p.off as u64 + i).collect(),
        };
        let arr = self.entry(p.base).and_then(|e| e.arr);
        let Some(entry) = self.entry(p.base) else {
            return AccessResult::fault(MemFault::WildPointer { off: p.off, at });
        };
        let o = entry.obj.as_ref().expect("materialized");
        let mut t: Option<Term> = None;
        for b in idx {
            // A promoted object answers from its array; the `Bytes` view underneath is
            // frozen at the moment of promotion and must not be consulted again, or the
            // two representations drift.
            let byte = match arr {
                Some(arr) => {
                    let i = a.bv(arr.idx_bits, b as u128);
                    a.select(arr.data, i)
                }
                None => match o.sym.get(&b) {
                    Some(x) => *x,
                    None => a.bv(8, o.data[b as usize] as u128),
                },
            };
            t = Some(match t {
                None => byte,
                Some(acc) => a.concat(acc, byte),
            });
        }
        AccessResult { value: t, faults }
    }

    /// A write whose offset is symbolic but pinned to a small set of feasible values
    /// (021 §3.1).
    ///
    /// Each candidate byte becomes `ite(off == k, val, old)` and its initialization
    /// becomes `Cond` — neither definitely written nor definitely not. Past
    /// `ITE_THRESHOLD` candidates the object promotes instead of growing a chain of
    /// nested selects that no solver wants to see.
    pub fn write_at_symbolic_offset(
        &mut self,
        a: &mut TermArena,
        id: ObjectId,
        off: Term,
        candidates: &[u64],
        val: Term,
        at: Span,
    ) -> AccessResult<()> {
        if let Some(f) = self.state_fault(id, 0, at) {
            return AccessResult::fault(f);
        }
        if self.entry(id).is_some_and(|e| e.readonly) {
            return AccessResult::fault(MemFault::ReadOnly {
                obj: id,
                off: 0,
                at,
            });
        }
        // Past the threshold the object promotes — but the write still happens. Promoting
        // and returning was losing the value entirely and then reporting the bytes it
        // claimed to have written as definitely uninitialized.
        if candidates.len() > ITE_THRESHOLD {
            self.promote_to_array(a, id);
        }
        let w = a.width(off);
        let obj_size = self.entry(id).map_or(0, |e| e.size);
        let Some(e) = self.entry_mut(id) else {
            return AccessResult::fault(MemFault::WildPointer { off: 0, at });
        };
        let mut arr = e.arr;
        let Some(o) = e.obj.as_mut() else {
            return AccessResult::fault(MemFault::AllocationTooLarge {
                obj: id,
                size: obj_size,
                at,
            });
        };
        let mut faults = Vec::new();
        for &k in candidates {
            // A candidate past the end **is** the buffer overflow, not a candidate to
            // skip. Dropping them silently meant a feasible set spilling past the object
            // produced no finding at all.
            if o.check_only(k as i64, 1).is_err() {
                faults.push(MemFault::OutOfBounds {
                    obj: id,
                    off: k as i64,
                    size: 1,
                    obj_size,
                    at,
                });
                continue;
            }
            // The guard has to be able to *hold* the candidate. `BvConst` masks, so an
            // 8-bit offset turned candidate 300 into `off == 44` — writing byte 300
            // whenever the index was 44, with no complaint.
            if w < 128 && k >= (1u64 << w.min(63)) {
                faults.push(MemFault::BadRange {
                    want_bits: 64 - k.leading_zeros() as u64,
                    max_bits: w as u64,
                    at,
                });
                continue;
            }
            let old = match arr {
                Some(arr) => {
                    let i = a.bv(arr.idx_bits, k as u128);
                    a.select(arr.data, i)
                }
                None => match o.sym.get(&k) {
                    Some(x) => *x,
                    None => a.bv(8, o.data[k as usize] as u128),
                },
            };
            let kc = a.bv(w, k as u128);
            let cond = a.eq(off, kc);
            let new_v = a.ite(cond, val, old);
            // 021 §3.1: `Cond` collapses whenever its guard folds to a constant. A write
            // at a *concrete* offset produces `k == k` or `k == j`, both of which fold —
            // leaving the tag `Cond` made a definitely-written byte report
            // `MaybeUninitialized`, and disagreed with the array path, which does collapse.
            let folded = a.eval_ground(cond).ok().map(|v| v.bits() != 0);
            match arr.as_mut() {
                Some(arr) => {
                    let i = a.bv(arr.idx_bits, k as u128);
                    arr.data = a.store(arr.data, i, new_v);
                    let one = a.bv(1, 1);
                    // The init array gets the same `Cond(t) → ite(t, 1, 0)` mapping
                    // promotion used, joined against what was already there.
                    for bit in k * 8..k * 8 + 8 {
                        let bi = a.bv(arr.idx_bits, bit as u128);
                        let prev = a.select(arr.init, bi);
                        let now = a.ite(cond, one, prev);
                        arr.init = a.store(arr.init, bi, now);
                    }
                }
                None => {
                    o.sym.insert(k, new_v);
                }
            }
            // The join is what keeps a conditional write from *downgrading* memory that
            // was already definitely initialized: both branches of the `ite` are then
            // initialized, so the result is `Yes`. The guard travels with the bit, so the
            // engine has something to discharge and promotion has something to map.
            if arr.is_none() {
                // **Per bit, not per byte.** Deciding once from bit 0 and applying it to
                // all eight let a bitfield-initialized half decide the untouched half —
                // and 021 §3.1 argues the whole tri-state from bitfields, so a byte whose
                // bits differ in state is the case, not a corner of it.
                for b in k * 8..k * 8 + 8 {
                    let next = match (o.init.get(b), folded) {
                        // Already definite: a guarded write cannot unwrite it, and both
                        // branches of the `ite` are then initialized.
                        (InitBit::Yes, _) => InitBit::Yes,
                        (_, Some(true)) => InitBit::Yes,
                        (prev, Some(false)) => prev,
                        // **The join of two guarded writes is the disjunction of their
                        // guards.** Keeping only the newer loses initialization: after
                        // `v[i] = x` then `v[j] = y` at one candidate, the model
                        // `i = k, j ≠ k` holds `x` while the guard would have said nobody
                        // wrote it.
                        (InitBit::Cond(prev), None) => InitBit::Cond(a.or(prev, cond)),
                        (InitBit::No, None) => InitBit::Cond(cond),
                    };
                    // `set_exact`, because `set_range`'s lattice join cannot see the
                    // disjunction that was just built.
                    o.init.set_exact(b, next);
                }
            }
        }
        if let Some(arr) = arr
            && let Some(e) = self.entry_mut(id)
        {
            e.arr = Some(arr);
        }
        AccessResult {
            value: Some(()),
            faults,
        }
    }

    /// 021 §3: promotion to array theory, **one-way within a state**.
    ///
    /// A representation that oscillated would make cost unpredictable and results
    /// order-dependent. The bytes and the init mask are kept as they are — the array
    /// form is a promise about how *future* symbolic-offset accesses are answered, and
    /// contract 6 requires the `(value, initialization-status)` pair to survive the
    /// change unaltered.
    pub fn promote_to_array(&mut self, a: &mut TermArena, id: ObjectId) -> AccessResult<()> {
        let ok = AccessResult {
            value: Some(()),
            faults: vec![],
        };
        // Promotion is a state change and obeys the state check like any other.
        if let Some(f) = self.state_fault(id, 0, Span::DUMMY) {
            return AccessResult::fault(f);
        }
        // One-way: an object already promoted keeps the contents it has, or a second
        // promotion would rebuild the arrays from the stale `Bytes` view and silently
        // discard everything written since.
        if self.entry(id).is_some_and(|e| e.repr == Repr::Array) {
            return ok;
        }
        let Some(e) = self.entry(id) else {
            return AccessResult::fault(MemFault::WildPointer {
                off: 0,
                at: Span::DUMMY,
            });
        };
        let size = e.size;
        // An unmaterialized object has nothing to promote *from*. Returning silently left
        // the caller believing a promotion had happened.
        let Some(o) = e.obj.as_ref() else {
            return AccessResult::fault(MemFault::AllocationTooLarge {
                obj: id,
                size,
                at: Span::DUMMY,
            });
        };
        let idx_bits = 64u32;
        let mut data = a.array_const(idx_bits, 8, 0);
        let mut init = a.array_const(idx_bits, 1, 0);
        let (bytes, syms, bits): (Vec<_>, Vec<_>, Vec<_>) = (
            (0..size).map(|b| o.raw_byte(b)).collect(),
            (0..size).map(|b| o.sym_at(b)).collect(),
            (0..size * 8).map(|b| o.init_bit(b)).collect(),
        );
        for b in 0..size {
            let v = match syms[b as usize] {
                Some(t) => t,
                None => a.bv(8, bytes[b as usize] as u128),
            };
            let i = a.bv(idx_bits, b as u128);
            data = a.store(data, i, v);
        }
        for bit in 0..size * 8 {
            // `No → 0`, `Yes → 1`, `Cond(t) → ite(t, 1, 0)` — 021 §3.1's mapping, which
            // is what makes the two paths agree rather than merely coexist.
            let one = a.bv(1, 1);
            let zero = a.bv(1, 0);
            let v = match bits[bit as usize] {
                InitBit::No => zero,
                InitBit::Yes => one,
                InitBit::Cond(t) => a.ite(t, one, zero),
            };
            let i = a.bv(idx_bits, bit as u128);
            init = a.store(init, i, v);
        }
        if let Some(e) = self.entry_mut(id) {
            e.repr = Repr::Array;
            e.arr = Some(ArrayContents {
                data,
                init,
                idx_bits,
            });
        }
        ok
    }

    /// **021 §5's justification for `&mut self` on `read`.**
    ///
    /// A read of never-written memory invents a fresh symbol; memoizing it is what makes
    /// 020 contract 10 true — a non-volatile load repeated yields the *same* value. Two
    /// reads returning two different fresh symbols make `x == x` satisfiably false over
    /// uninitialized memory, and two findings for one defect is noise on top of that.
    fn memoize(&mut self, id: ObjectId, lo_bit: u64, n_bits: u64) {
        if let Some(e) = self.entry_mut(id)
            && let Some(o) = e.obj.as_mut()
        {
            o.memoize_fresh(lo_bit, n_bits);
        }
    }

    /// The same, for a promoted object — whose initialization lives in an array, so
    /// writing the mask was a no-op there and contract 26 held on one representation
    /// only.
    fn memoize_via(&mut self, a: &mut TermArena, id: ObjectId, lo_bit: u64, n_bits: u64) {
        let Some(mut arr) = self.entry(id).and_then(|e| e.arr) else {
            self.memoize(id, lo_bit, n_bits);
            return;
        };
        let one = a.bv(1, 1);
        for bit in lo_bit..lo_bit + n_bits {
            let i = a.bv(arr.idx_bits, bit as u128);
            arr.init = a.store(arr.init, i, one);
        }
        if let Some(e) = self.entry_mut(id) {
            e.arr = Some(arr);
        }
    }

    pub fn free(&mut self, id: ObjectId, at: Span) -> AccessResult<()> {
        // `free(NULL)` is legal C and a no-op. Models call it constantly, so reporting it
        // is a false positive on correct code.
        if id == ObjectId::NULL {
            return AccessResult {
                value: Some(()),
                faults: vec![],
            };
        }
        match self.entry(id).map(|e| (e.state, e.kind)) {
            Some((ObjState::Freed(freed_at), _)) => AccessResult::fault(MemFault::DoubleFree {
                obj: id,
                freed_at,
                at,
            }),
            // `free()` of something that did not come from the heap is a real bug, and a
            // different one from a double free.
            Some((_, k)) if k != ObjKind::Heap => AccessResult::fault(MemFault::BadFree {
                obj: id,
                kind: k,
                at,
            }),
            Some(_) => {
                self.entry_mut(id).unwrap().state = ObjState::Freed(at);
                AccessResult {
                    value: Some(()),
                    faults: vec![],
                }
            }
            None => AccessResult::fault(MemFault::WildPointer { off: 0, at }),
        }
    }

    pub fn exit_scope(&mut self, id: ObjectId, at: Span) -> AccessResult<()> {
        if let Some(e) = self.entry_mut(id) {
            // Only a **live, non-global** object leaves scope. Overwriting `Freed` wiped
            // the free record — and a heap pointer normally lives in a stack local, so an
            // engine calling this at frame teardown erased every free in the state along
            // with all double-free detection. 021 §4: globals are `Live` forever.
            if e.state == ObjState::Live && e.kind != ObjKind::Global {
                e.state = ObjState::OutOfScope(at);
            }
        }
        AccessResult {
            value: Some(()),
            faults: vec![],
        }
    }

    /// 021 §4: allocate-new + copy the retained prefix + free-old.
    ///
    /// Modeling it this way is what makes `vec_resize` analysable — the old pointer
    /// becomes dangling and any surviving copy of it is reported, which is a real and
    /// frequent VPP bug class. The new tail is **not** zeroed, because `realloc` does not
    /// zero and a model that did would hide every read-of-uninitialized-tail bug.
    pub fn realloc(&mut self, old: ObjectId, new_size: u64, at: Span) -> AccessResult<ObjectId> {
        // A `realloc` of dead memory is a use-after-free like any other read of it. The
        // old signature returned a bare id with no fault channel, so it copied the dead
        // bytes in silence.
        if let Some(f) = self.state_fault(old, 0, at) {
            return AccessResult::fault(f);
        }
        let (kind, align, keep, points_to, root) = match self.entry(old) {
            Some(e) => (
                e.kind,
                e.align,
                e.size.min(new_size),
                e.points_to.clone(),
                e.root,
            ),
            None => return AccessResult::fault(MemFault::WildPointer { off: 0, at }),
        };
        let new = self.alloc(kind, new_size, align, at);
        // **The new object inherits the old one's position in the graph** — outgoing
        // edges, rootedness, and every incoming edge. Without this, reallocating a live
        // rooted vector reported it leaked, which is 021 §4's own motivating example.
        if let Some(e) = self.entry_mut(new) {
            e.points_to = points_to;
            e.root = root;
        }
        for (_, e) in self.entries.iter_mut() {
            for (_, t) in e.points_to.iter_mut() {
                if *t == old {
                    *t = new;
                }
            }
        }
        let mut faults = Vec::new();
        if keep > 0 {
            let src = self
                .entry(old)
                .and_then(|e| e.obj.as_ref())
                .and_then(|o| o.raw_bytes(0, keep));
            let init = self
                .entry(old)
                .and_then(|e| e.obj.as_ref())
                .map(|o| (0..keep * 8).map(|b| o.init_bit(b)).collect::<Vec<_>>());
            match (src, init) {
                (Some(src), Some(init)) => {
                    if let Some(e) = self.entry_mut(new)
                        && let Some(o) = e.obj.as_mut()
                    {
                        let _ = o.write_bytes(0, &src);
                        o.restore_init(0, &init);
                    }
                }
                // An unmaterialized source has no bytes to carry over, and dropping them
                // silently would be a wrong answer rather than a limit.
                _ => faults.push(MemFault::AllocationTooLarge {
                    obj: old,
                    size: keep,
                    at,
                }),
            }
        }
        faults.extend(self.free(old, at).faults);
        AccessResult {
            value: Some(new),
            faults,
        }
    }

    /// 021 §4: `Live` heap objects unreachable from a root are leaks.
    ///
    /// Reachability is transitive, or every linked list reports every node but its head.
    /// A freed object is not a leak — reporting both would double-count every correct
    /// malloc/free pair — and neither is a stack object, or every return reports every
    /// local.
    pub fn leaks(&self) -> Vec<Leak> {
        // **Roots are derived, not declared** (021 §4): globals, and any *live* stack
        // object. Requiring the caller to mark them by hand made every heap object held
        // only by a live local read as a leak. `root` remains for what cannot be derived
        // — the return value.
        let mut reachable: Vec<ObjectId> = self
            .entries
            .iter()
            // Liveness is *not* rechecked here: the walk below propagates only through
            // live objects, so an out-of-scope frame is already a dead end. Testing it in
            // both places was redundant, and mutation showed the redundant half was dead.
            .filter(|(_, e)| e.root || matches!(e.kind, ObjKind::Global | ObjKind::Stack))
            .map(|(i, _)| *i)
            .collect();
        let mut i = 0;
        while i < reachable.len() {
            let cur = reachable[i];
            i += 1;
            // Only a **live** object propagates reachability. 021 §4 scopes leak roots
            // to live memory, and walking through a freed container hid the commonest
            // leak shape there is: free the head, forget the children.
            if let Some(e) = self.entry(cur).filter(|e| e.state == ObjState::Live) {
                for &(_, t) in &e.points_to {
                    if !reachable.contains(&t) {
                        reachable.push(t);
                    }
                }
            }
        }
        self.entries
            .iter()
            .filter(|(id, e)| {
                e.kind == ObjKind::Heap && e.state == ObjState::Live && !reachable.contains(id)
            })
            .map(|(id, e)| Leak {
                obj: *id,
                allocated_at: e.origin,
            })
            .collect()
    }
}

/// A signed byte offset plus an unsigned bit offset within it. `None` when the byte
/// offset is negative, which the object-relative bit index cannot represent — the caller
/// turns that into an out-of-bounds fault rather than wrapping.
fn ranges_overlap(a: i64, b: i64, size: u64) -> bool {
    let n = size as i64;
    a < b + n && b < a + n
}

fn abs_bit(off: i64, lo_bit: u64) -> Option<u64> {
    // Checked, in i128. The byte API went to i128 two commits ago and the bit API did
    // not follow, so `off * 8 + lo_bit` wrapped and a wildly out-of-bounds pointer became
    // a fault-free read of byte 0.
    if off < 0 {
        return None;
    }
    let b = (off as i128) * 8 + lo_bit as i128;
    u64::try_from(b).ok()
}

fn lift(e: AccessError, obj: ObjectId, at: Span) -> MemFault {
    match e {
        AccessError::OutOfBounds {
            off,
            size,
            obj_size,
        } => MemFault::OutOfBounds {
            obj,
            off,
            size,
            obj_size,
            at,
        },
        AccessError::Uninitialized { off, bit } => MemFault::Uninitialized { obj, off, bit, at },
        AccessError::MaybeUninitialized { off, bit } => {
            MemFault::MaybeUninitialized { obj, off, bit, at }
        }
        AccessError::BadRange {
            want_bits,
            max_bits,
        } => MemFault::BadRange {
            want_bits,
            max_bits,
            at,
        },
        AccessError::ReadOnly { off } => MemFault::ReadOnly { obj, off, at },
        AccessError::SymbolicByte { off } => MemFault::SymbolicByte { obj, off, at },
    }
}

// ---------------------------------------------------------------------------
// Symbolic bounds checking (021 §5 step 2).
// ---------------------------------------------------------------------------

/// The engine's side of an access: a solver and the path condition.
///
/// 021 §5 is explicit that this cannot be folded into `Memory`. Bounds checking *adds a
/// constraint to the path condition*, and symbolic-base resolution forks states — both
/// are the engine's business, not the heap's. Keeping them apart is also what lets the
/// concrete-offset API stay solver-free, which is the path nearly every VPP access takes.
#[derive(Debug, Default)]
pub struct AccessCtx {
    solver: SolverLite,
    path: Vec<Term>,
}

impl AccessCtx {
    pub fn new() -> AccessCtx {
        AccessCtx::default()
    }

    /// Add to the path condition. **Conjunction, never substitution** — an access that
    /// learns something must not discard what the path already knew, or every state it
    /// touches is silently widened.
    pub fn assume(&mut self, t: Term) {
        self.solver.assert(t);
        self.path.push(t);
    }

    pub fn path(&self) -> &[Term] {
        &self.path
    }

    /// A concrete value for `t` consistent with the path condition, if tier 1 can find
    /// one. `None` when it cannot — the caller must not invent one.
    pub fn model_of(&mut self, a: &mut TermArena, t: Term) -> Option<i64> {
        let vars = {
            let mut v = Vec::new();
            a.vars_of(t, &mut v);
            v
        };
        match self.solver.check(a, &[]) {
            CheckResult::Sat(m) => {
                let mut sub = chiero_solver::Model::new();
                for v in vars {
                    sub.set(v, m.get(v)?);
                }
                a.eval(&sub, t).ok().map(|c| c.bits() as i64)
            }
            _ => None,
        }
    }

    /// Whether `t` can hold under the current path condition.
    fn feasible(&mut self, a: &mut TermArena, t: Term) -> Feasibility {
        // A concrete offset folds the condition to a literal at construction (022 §2), and
        // a literal is not something to ask a solver about — `solver-lite` answers
        // `Unknown` for one, which would turn every *concrete* access routed through this
        // path into a non-answer. Deciding it here is both exact and free, and it is what
        // makes the symbolic path agree with the concrete one on the same access.
        if let Ok(v) = a.eval_ground(t) {
            return if v.bits() != 0 {
                Feasibility::Yes(None)
            } else {
                Feasibility::No
            };
        }
        match self.solver.check(a, &[t]) {
            CheckResult::Sat(m) => Feasibility::Yes(m.any_value_i64()),
            CheckResult::Unsat => Feasibility::No,
            // Tier 1 is deliberately incomplete (022 §3), and `Unknown` must never be
            // read as either answer. Treating it as "no" would prune a real path;
            // treating it as "yes" would invent a finding. It is its own outcome.
            CheckResult::Unknown(_) => Feasibility::Unknown,
        }
    }
}

enum Feasibility {
    Yes(Option<i64>),
    No,
    Unknown,
}

impl Memory {
    /// Read `size` bytes at a **symbolic** offset within `id` (021 §5 step 2).
    pub fn read_sym(
        &mut self,
        cx: &mut AccessCtx,
        a: &mut TermArena,
        id: ObjectId,
        off: Term,
        size: u64,
        at: Span,
    ) -> AccessResult<Vec<u8>> {
        match self.bounds_decision(cx, a, id, off, size, at) {
            Err(r) => AccessResult {
                value: None,
                faults: r,
            },
            Ok((faults, witness)) => {
                let mut r = self.read(
                    Pointer {
                        base: id,
                        off: witness,
                    },
                    size,
                    at,
                );
                r.faults.splice(0..0, faults);
                r
            }
        }
    }

    /// Write one symbolic byte at a symbolic offset, bounds-checked the same way.
    ///
    /// The write half is the more dangerous one to leave unchecked: a wild write corrupts
    /// state the analysis then reasons about, so the finding arrives too late to explain
    /// anything.
    pub fn write_sym(
        &mut self,
        cx: &mut AccessCtx,
        a: &mut TermArena,
        id: ObjectId,
        off: Term,
        val: Term,
        at: Span,
    ) -> AccessResult<()> {
        match self.bounds_decision(cx, a, id, off, 1, at) {
            Err(r) => AccessResult {
                value: None,
                faults: r,
            },
            Ok((mut faults, witness)) => {
                let w = self.write_sym_byte(
                    Pointer {
                        base: id,
                        off: witness,
                    },
                    val,
                    at,
                );
                faults.extend(w.faults);
                AccessResult {
                    value: w.value,
                    faults,
                }
            }
        }
    }

    /// A concrete offset consistent with the path condition.
    ///
    /// Every branch of the decision below used to proceed at a hardcoded `0`, so a read
    /// whose path condition pinned `i == 4` returned byte 0 — bounds-checked and then
    /// thrown away, which is worse than not checking, because the answer looks
    /// authoritative. Concretizing here is what 023 §7 calls `Approximated`; the term is
    /// still the truth, and this is the byte the witness names.
    fn witness(&self, a: &mut TermArena, cx: &mut AccessCtx, off: Term, hint: Option<i64>) -> i64 {
        if let Ok(v) = a.eval_ground(off) {
            return v.bits() as i64;
        }
        match cx.model_of(a, off) {
            Some(v) => v,
            None => hint.unwrap_or(0),
        }
    }

    /// The three-way decision of 021 §5 step 2.
    ///
    /// `Err` means the access does not happen: the state check failed, or the access is
    /// out of bounds under *every* model so there is no in-bounds branch to continue on.
    /// `Ok` carries the faults to report and a concrete in-bounds offset to proceed at.
    #[allow(clippy::type_complexity)]
    fn bounds_decision(
        &mut self,
        cx: &mut AccessCtx,
        a: &mut TermArena,
        id: ObjectId,
        off: Term,
        size: u64,
        at: Span,
    ) -> Result<(Vec<MemFault>, i64), Vec<MemFault>> {
        // Step 1 first, and without consulting the solver: a dead object's contents are
        // not the issue, so there is no bounds question to ask.
        if let Some(f) = self.state_fault(id, 0, at) {
            return Err(vec![f]);
        }
        let obj_size = self.entry(id).map_or(0, |e| e.size);
        let w = a.width(off);
        let limit = obj_size.saturating_sub(size.saturating_sub(1));
        if limit == 0 {
            // Nothing fits: every offset is out of bounds under every model.
            return Err(vec![MemFault::OutOfBounds {
                obj: id,
                off: 0,
                size,
                obj_size,
                at,
            }]);
        }
        let lim = a.bv(w, limit as u128);
        // `off <u limit` is in bounds; `limit - 1 <u off` is its complement. Both are
        // stated **positively** rather than as `not(...)`, because `solver-lite`'s
        // fragment (022 §3.2) is comparisons and conjunctions — a negated comparison
        // falls outside it and comes back `Unknown`, which would turn every bounds
        // question into an escalation.
        //
        // Unsigned comparison covers the negative case too: a negative offset is a huge
        // unsigned value. That is why the model's *offsets* are signed and its *checks*
        // are not.
        let in_bounds = a.ult(off, lim);
        let lim_minus_1 = a.bv(w, (limit - 1) as u128);
        let oob = a.ult(lim_minus_1, off);

        let can_be_oob = cx.feasible(a, oob);
        let can_be_ok = cx.feasible(a, in_bounds);

        match (can_be_ok, can_be_oob) {
            // Definitely in bounds: nothing to report, nothing to add — but the access
            // still has to happen *somewhere*, and that somewhere is a model of the path
            // condition, not zero.
            (Feasibility::Yes(w), Feasibility::No) => Ok((vec![], self.witness(a, cx, off, w))),
            (_, Feasibility::No) => Ok((vec![], self.witness(a, cx, off, None))),
            // Definitely out of bounds: report and terminate. Continuing here would carry
            // an unsatisfiable path condition, which 023 §3 calls a chiero bug.
            (Feasibility::No, _) => Err(vec![MemFault::OutOfBounds {
                obj: id,
                off: 0,
                size,
                obj_size,
                at,
            }]),
            // May be out of bounds: report with a witness, then **continue on the
            // in-bounds branch** with the constraint added. Killing the state instead
            // would let one early OOB hide everything downstream of it.
            (_, Feasibility::Yes(oob_witness)) => {
                cx.assume(in_bounds);
                // The *reported* witness is an out-of-bounds one — that is the bug being
                // shown. The offset execution then proceeds at is an **in-bounds** one,
                // re-derived under the constraint just added.
                let go = self.witness(a, cx, off, None);
                Ok((
                    vec![MemFault::OutOfBoundsMaybe {
                        obj: id,
                        size,
                        obj_size,
                        witness: oob_witness.unwrap_or(limit as i64),
                        at,
                    }],
                    go,
                ))
            }
            // Tier 1 could not decide. **No constraint is added**: assuming the access
            // in bounds on the strength of an answer the solver did not give would prune
            // the very path the escalation exists to explore. Escalation is the engine's
            // to perform (022 §4); the honest interim answer is to proceed and claim
            // nothing.
            (_, Feasibility::Unknown) => Ok((vec![], self.witness(a, cx, off, None))),
        }
    }
}
