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

    /// Set one bit's status verbatim, bypassing the join. Only for copying an existing
    /// mask (`realloc`), where the destination has no prior state to join with.
    pub fn set_exact(&mut self, bit: u64, to: InitBit) {
        if let Some(slot) = self.bits.get_mut(bit as usize) {
            *slot = to;
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

#[derive(Clone, Debug)]
struct Entry {
    obj: Option<MemObject>,
    kind: ObjKind,
    size: u64,
    align: u64,
    state: ObjState,
    readonly: bool,
    origin: Span,
    /// Objects this one holds pointers to, for reachability (021 §4's leak rule).
    points_to: Vec<ObjectId>,
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
        let id = self
            .space
            .alloc(kind, size.min(MAX_MATERIALIZED_BYTES), align, at);
        // Oversized objects are recorded but not materialized: every access faults, which
        // is a finding rather than a dead process.
        let obj =
            (size <= MAX_MATERIALIZED_BYTES).then(|| MemObject::new(id, kind, size, align, at));
        self.entries.push((
            id,
            Entry {
                obj,
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
        }
    }

    pub fn set_root(&mut self, id: ObjectId) {
        if let Some(e) = self.entry_mut(id) {
            e.root = true;
        }
    }

    pub fn point_at(&mut self, from: ObjectId, to: ObjectId) {
        if let Some(e) = self.entry_mut(from) {
            e.points_to.push(to);
        }
    }

    /// **021 §5 step 1.** Runs before anything touches contents, so a dangling access
    /// never reads stale bytes and never *also* reports "uninitialized" about memory it
    /// had no business touching.
    fn state_fault(&self, id: ObjectId, at: Span) -> Option<MemFault> {
        if id == ObjectId::NULL {
            return Some(MemFault::NullDeref { off: 0, at });
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
        let want = e.align.min(size.max(1)).max(1);
        (!off.unsigned_abs().is_multiple_of(want)).then_some(MemFault::Misaligned {
            obj: id,
            off,
            want,
            at,
        })
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
        if let Some(f) = self.state_fault(p.base, at) {
            return AccessResult::fault(f);
        }
        if let Some(f) = self.too_large(p.base, at) {
            return AccessResult::fault(f);
        }
        let Some(e) = self.entry(p.base) else {
            return AccessResult::fault(MemFault::NullDeref { off: p.off, at });
        };
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
        match obj.read_bytes(p.off, size) {
            Ok(v) => AccessResult {
                value: Some(v),
                faults,
            },
            Err(AccessError::Uninitialized { off, bit }) => {
                faults.push(MemFault::Uninitialized {
                    obj: p.base,
                    off,
                    bit,
                    at,
                });
                // **A value as well as a fault.** The engine gets a fresh symbol here;
                // the concrete core hands back the bytes so it has something to carry.
                AccessResult {
                    value: obj.raw_bytes(p.off, size),
                    faults,
                }
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
        if let Some(f) = self.state_fault(p.base, at) {
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
        if let Some(f) = self.state_fault(p.base, at) {
            return AccessResult::fault(f);
        }
        if let Some(f) = self.too_large(p.base, at) {
            return AccessResult::fault(f);
        }
        let Some(e) = self.entry(p.base) else {
            return AccessResult::fault(MemFault::NullDeref { off: p.off, at });
        };
        let obj = e.obj.as_ref().expect("materialized");
        match abs_bit(p.off, lo_bit) {
            None => AccessResult::fault(MemFault::OutOfBounds {
                obj: p.base,
                off: p.off,
                size: n_bits.div_ceil(8),
                obj_size: e.size,
                at,
            }),
            Some(b) => match obj.read_bits(b, n_bits) {
                Ok(v) => AccessResult {
                    value: Some(v),
                    faults: vec![],
                },
                Err(err) => AccessResult::fault(lift(err, p.base, at)),
            },
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
        if let Some(f) = self.state_fault(p.base, at) {
            return AccessResult::fault(f);
        }
        if let Some(f) = self.too_large(p.base, at) {
            return AccessResult::fault(f);
        }
        let size = self.entry(p.base).map_or(0, |e| e.size);
        let Some(e) = self.entry_mut(p.base) else {
            return AccessResult::fault(MemFault::NullDeref { off: p.off, at });
        };
        let obj = e.obj.as_mut().expect("materialized");
        match abs_bit(p.off, lo_bit) {
            None => AccessResult::fault(MemFault::OutOfBounds {
                obj: p.base,
                off: p.off,
                size: n_bits.div_ceil(8),
                obj_size: size,
                at,
            }),
            Some(b) => match obj.write_bits(b, n_bits, v) {
                Ok(()) => AccessResult {
                    value: Some(()),
                    faults: vec![],
                },
                Err(err) => AccessResult::fault(lift(err, p.base, at)),
            },
        }
    }

    pub fn free(&mut self, id: ObjectId, at: Span) -> AccessResult<()> {
        match self.entry(id).map(|e| e.state) {
            Some(ObjState::Freed(freed_at)) => AccessResult::fault(MemFault::DoubleFree {
                obj: id,
                freed_at,
                at,
            }),
            Some(_) => {
                self.entry_mut(id).unwrap().state = ObjState::Freed(at);
                AccessResult {
                    value: Some(()),
                    faults: vec![],
                }
            }
            None => AccessResult::fault(MemFault::NullDeref { off: 0, at }),
        }
    }

    pub fn exit_scope(&mut self, id: ObjectId, at: Span) -> AccessResult<()> {
        if let Some(e) = self.entry_mut(id) {
            e.state = ObjState::OutOfScope(at);
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
    pub fn realloc(&mut self, old: ObjectId, new_size: u64, at: Span) -> ObjectId {
        let (kind, align, keep) = match self.entry(old) {
            Some(e) => (e.kind, e.align, e.size.min(new_size)),
            None => (ObjKind::Heap, 8, 0),
        };
        let new = self.alloc(kind, new_size, align, at);
        if keep > 0
            && let Some(src) = self
                .entry(old)
                .and_then(|e| e.obj.as_ref())
                .and_then(|o| o.raw_bytes(0, keep))
        {
            let init = self
                .entry(old)
                .and_then(|e| e.obj.as_ref())
                .map(|o| (0..keep * 8).map(|b| o.init_bit(b)).collect::<Vec<_>>());
            if let Some(e) = self.entry_mut(new)
                && let Some(o) = e.obj.as_mut()
            {
                let _ = o.write_bytes(0, &src);
                if let Some(init) = init {
                    o.restore_init(0, &init);
                }
            }
        }
        self.free(old, at);
        new
    }

    /// 021 §4: `Live` heap objects unreachable from a root are leaks.
    ///
    /// Reachability is transitive, or every linked list reports every node but its head.
    /// A freed object is not a leak — reporting both would double-count every correct
    /// malloc/free pair — and neither is a stack object, or every return reports every
    /// local.
    pub fn leaks(&self) -> Vec<Leak> {
        let mut reachable: Vec<ObjectId> = self
            .entries
            .iter()
            .filter(|(_, e)| e.root)
            .map(|(i, _)| *i)
            .collect();
        let mut i = 0;
        while i < reachable.len() {
            let cur = reachable[i];
            i += 1;
            if let Some(e) = self.entry(cur) {
                for &t in &e.points_to {
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
fn abs_bit(off: i64, lo_bit: u64) -> Option<u64> {
    (off >= 0).then(|| off as u64 * 8 + lo_bit)
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
        AccessError::BadRange {
            want_bits,
            max_bits,
        } => MemFault::BadRange {
            want_bits,
            max_bits,
            at,
        },
        AccessError::ReadOnly { off } => MemFault::ReadOnly { obj, off, at },
    }
}
