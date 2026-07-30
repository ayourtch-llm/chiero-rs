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
    /// Length in **bits**, so a mask knows where it ends independently of `yes`'s
    /// word-rounded length.
    len: u64,
    /// One bit per tracked bit: set means `Yes`. A `Vec<InitBit>` cost eight bytes per
    /// *bit* — 64x the object — which aborted the process at the cap rather than
    /// producing a fault.
    yes: Vec<u64>,
    /// `Cond` bits, sparse. Conditional writes are rare and clustered; paying a word per
    /// bit for them would undo the point of the bitset.
    cond: BTreeMap<u64, Term>,
}

impl InitMask {
    /// `size` is in **bytes**; the mask holds eight entries per byte.
    ///
    /// Saturating rather than wrapping: `(size * 8) as usize` overflowed above 2^61 and
    /// panicked. The `MAX_MATERIALIZED_BYTES` guard lives in `Memory::alloc`, which is
    /// not the only way to reach this constructor.
    pub fn new(size: u64) -> InitMask {
        let len = size.saturating_mul(8);
        let words = usize::try_from(len.div_ceil(64)).unwrap_or(usize::MAX);
        InitMask {
            len,
            yes: vec![0u64; words],
            cond: BTreeMap::new(),
        }
    }

    fn yes_at(&self, bit: u64) -> bool {
        match self.yes.get((bit / 64) as usize) {
            Some(w) => w >> (bit % 64) & 1 == 1,
            None => false,
        }
    }

    fn set_yes(&mut self, bit: u64, to: bool) {
        if let Some(w) = self.yes.get_mut((bit / 64) as usize) {
            if to {
                *w |= 1u64 << (bit % 64);
            } else {
                *w &= !(1u64 << (bit % 64));
            }
        }
    }

    pub fn get(&self, bit: u64) -> InitBit {
        if bit >= self.len {
            return InitBit::No;
        }
        if self.yes_at(bit) {
            InitBit::Yes
        } else {
            match self.cond.get(&bit) {
                Some(t) => InitBit::Cond(*t),
                None => InitBit::No,
            }
        }
    }

    pub fn set_range(&mut self, lo_bit: u64, n_bits: u64, to: InitBit) {
        let hi = (lo_bit.saturating_add(n_bits)).min(self.len);
        if lo_bit >= hi {
            return;
        }
        match to {
            // `join(old, No) == old` for all three, so this is a no-op rather than a
            // clear. Writing `No` over `Yes` would erase an initialization.
            InitBit::No => {}
            InitBit::Yes => {
                // Whole words at a time: a `memset` of the largest allowed object is
                // 2^33 bits, and one iteration per bit made it minutes rather than
                // milliseconds.
                let mut b = lo_bit;
                while b < hi {
                    let w = (b / 64) as usize;
                    let off = b % 64;
                    let take = (64 - off).min(hi - b);
                    let m = if take == 64 {
                        u64::MAX
                    } else {
                        ((1u64 << take) - 1) << off
                    };
                    self.yes[w] |= m;
                    b += take;
                }
                // A bit that is now definitely initialized has no guard left to record.
                let stale: Vec<u64> = self.cond.range(lo_bit..hi).map(|(b, _)| *b).collect();
                for b in stale {
                    self.cond.remove(&b);
                }
            }
            // Through `join`, so the lattice has one definition rather than one per
            // representation. The two fast paths above are `join(old, Yes) == Yes` and
            // `join(old, No) == old`, which hold for every `old`.
            InitBit::Cond(_) => {
                for b in lo_bit..hi {
                    let j = join(self.get(b), to);
                    self.set_exact(b, j);
                }
            }
        }
    }

    /// Force a run of bits back to `No`, bypassing the join — the one operation the
    /// lattice deliberately does not offer, because `join(old, No) == old`. Only for
    /// *invalidation*, where forgetting is the point.
    pub fn set_exact_range_uninit(&mut self, lo_bit: u64, n_bits: u64) {
        for b in lo_bit..lo_bit + n_bits {
            self.set_exact(b, InitBit::No);
        }
    }

    /// Set one bit's status verbatim, bypassing the join. Only for copying an existing
    /// mask (`realloc`), where the destination has no prior state to join with.
    pub fn set_exact(&mut self, bit: u64, to: InitBit) {
        if bit >= self.len {
            return;
        }
        match to {
            InitBit::No => {
                self.set_yes(bit, false);
                self.cond.remove(&bit);
            }
            InitBit::Yes => {
                self.set_yes(bit, true);
                self.cond.remove(&bit);
            }
            InitBit::Cond(t) => {
                self.set_yes(bit, false);
                self.cond.insert(bit, t);
            }
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
    /// The range holds a symbolic byte, which a concrete access cannot answer for.
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
    /// Bytes without initialization. See `Memory::write_uninit_bytes_for_test`.
    pub fn write_raw_uninit(&mut self, off: i64, bytes: &[u8]) {
        if off < 0 {
            return;
        }
        for (i, b) in bytes.iter().enumerate() {
            let at = off as usize + i;
            if at < self.data.len() {
                self.data[at] = *b;
            }
        }
    }

    /// Forget everything about the contents: no bytes, no overlay, nothing initialized.
    /// The object stays the same size and identity — this is invalidation, not a free.
    pub fn clear_contents(&mut self, size: u64) {
        self.data = vec![0; size as usize];
        self.init = InitMask::new(size);
        self.sym.clear();
    }

    /// Place a symbolic byte at a concrete offset, without the checks
    /// `Memory::write_sym_byte` makes — the caller has already bounds-checked. Used by
    /// `copy`, which has to reinstate the overlay `write_bytewise` cleared.
    pub fn set_sym_at(&mut self, b: u64, t: Term) {
        self.sym.insert(b, t);
    }

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
    /// Everything `write_bits` will refuse, decided without mutating. Kept beside it so
    /// the caller can avoid a copy-on-write clone for a write that will not happen, and
    /// factored out rather than duplicated so the two cannot drift.
    pub fn check_bit_write(&self, lo_bit: u64, n_bits: u64) -> Result<(), AccessError> {
        self.check_bits(lo_bit, n_bits)?;
        self.check_writable((lo_bit / 8) as i64)?;
        if let Some(b) = self.first_symbolic_bit_byte(lo_bit, n_bits) {
            return Err(AccessError::SymbolicByte { off: b as i64 });
        }
        Ok(())
    }

    pub fn write_bits(&mut self, lo_bit: u64, n_bits: u64, v: u128) -> Result<(), AccessError> {
        self.check_bit_write(lo_bit, n_bits)?;
        for i in 0..n_bits {
            let bit = lo_bit + i;
            let (byte, sh) = ((bit / 8) as usize, bit % 8);
            let one = (v >> i) & 1;
            self.data[byte] = (self.data[byte] & !(1 << sh)) | ((one as u8) << sh);
        }
        self.init.set_range(lo_bit, n_bits, InitBit::Yes);
        Ok(())
    }

    /// The first byte in a bit range that holds a symbolic value. Shared by the bit read
    /// and write so the two cannot disagree about which bytes are knowable — which is how
    /// the write came to vanish while the read answered confidently.
    fn first_symbolic_bit_byte(&self, lo_bit: u64, n_bits: u64) -> Option<u64> {
        if n_bits == 0 {
            return None;
        }
        let last = lo_bit.checked_add(n_bits - 1)? / 8;
        (lo_bit / 8..=last).find(|b| self.sym.contains_key(b))
    }

    pub fn read_bits(&self, lo_bit: u64, n_bits: u64) -> Result<u128, AccessError> {
        // **`check_bits` first**, or the byte range below overflows on a wrapping request
        // — which an existing test covers, and which is why bounds precede everything
        // else throughout this file.
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
        // **After the initialization checks**, matching the byte path: whether a byte
        // was written is a different question from what is in it, and 021 §3.1's
        // conditional case has to keep reporting `MaybeUninitialized` rather than being
        // pre-empted. Only then does a symbol make the value unanswerable.
        if let Some(b) = self.first_symbolic_bit_byte(lo_bit, n_bits) {
            return Err(AccessError::SymbolicByte { off: b as i64 });
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

/// A symbol `Memory` invented, and what it stands for.
///
/// `array` marks the whole-object havoc arrays: an assignment for one of those is not a
/// number, so a witness cannot bind it the way it binds a scalar — and saying so is the
/// point. Claiming a path has no inputs when it has an unbindable one is the failure this
/// record exists to prevent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MintedSymbol {
    pub term: chiero_solver::Term,
    pub obj: ObjectId,
    pub at: Span,
    pub why: &'static str,
    pub array: bool,
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
        // **Zero is `NULL`.** C spells the null pointer `((void *)0)`, so an `IntToPtr` of
        // a literal zero is the commonest pointer constant there is — falling through to
        // `UNBOUND` reported "matching no known object" for a plain null dereference, and
        // `WildPointer` carries `Unknown` where `NullDeref` is a definite finding.
        if a == 0 {
            return Pointer {
                base: ObjectId::NULL,
                off: 0,
            };
        }
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

/// How a havoc fills what it invalidates (024 §2.1). **No safe default**: `Symbolic`
/// marks bytes initialized-with-unknown-value, which can mask a genuine
/// uninitialized-read bug; `Uninitialized` produces a false-positive storm on any buffer
/// the callee legitimately filled. The caller states which, so the choice is visible
/// rather than folkloric.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HavocFill {
    Symbolic,
    Uninitialized,
}

/// What a havoc actually did. A bare `Vec<ObjectId>` could say what it reached and not
/// what it *failed* to reach, so a scan cut short by `HAVOC_SCAN_BYTES` — or one over an
/// object whose bytes are gone — read as "followed everything".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Havocked {
    /// Objects genuinely invalidated. Skipped ones — read-only, freed, `NULL`,
    /// `UNBOUND`, unmaterialized — are **not** here: a count that includes skips reads
    /// as coverage.
    pub objects: Vec<ObjectId>,
    /// Some object's pointer scan did not finish, so reachable objects may have been
    /// missed and still hold stale contents.
    pub truncated: bool,
}

/// How far a havoc's pointer scan reads into an object before giving up. A havoc'd
/// object can be 64 MiB and the range search is linear in the object count, so the
/// product is not something to run unbounded — but a cap that nobody hears about reads as
/// "followed everything", which is why `havoc` returns whether it was hit.
pub const HAVOC_SCAN_BYTES: u64 = 1 << 16;

/// The shared `MAX_ACCESS_BITS` check for the term API. A free function rather than a
/// method so `read_term` and `write_term` cannot drift — which is how the term path came
/// to be the only one without it.
fn too_wide(size: u64, at: Span) -> Option<MemFault> {
    let want = size.saturating_mul(8);
    (want > MAX_ACCESS_BITS).then_some(MemFault::BadRange {
        want_bits: want,
        max_bits: MAX_ACCESS_BITS,
        at,
    })
}

/// What a byte-wise read hands back: the bytes, their per-bit initialization, and the
/// symbolic overlay. All three travel together — carrying the bytes without the overlay
/// is what turned a `memcpy` of a symbolic field into a fabricated constant.
type RawBytes = (Vec<u8>, Vec<InitBit>, Vec<Option<Term>>);

/// Above this, an object is not materialized and every access to it faults.
///
/// An unconstrained `clib_mem_alloc(n)` used to abort the process, and an abort is not
/// something `catch_unwind` can contain. 023 §10 concretizes symbolic sizes from a solver
/// model, which can hand back anything the constraints allow.
///
/// The number has to be a size chiero can actually *hold*, not just one it is willing to
/// accept. `MemObject` costs `size` bytes of contents plus `size / 8` for the init
/// bitset, and `Memory::set` builds a `size`-byte fill, so the cap is roughly a 2.2x
/// host-memory multiplier. At the previous 1 GiB — with a mask that then cost eight bytes
/// per *bit* — an object exactly at the cap asked for 64 GiB and died. 64 MiB is above
/// any single object in the VPP paths §2 targets and cheap enough that the boundary case
/// is testable, which is what let the old cap go wrong unnoticed.
pub const MAX_MATERIALIZED_BYTES: u64 = 1 << 26;

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
        /// **The guard the engine must discharge**, which the comment above says is its job
        /// and which this variant did not carry until wave 204. Without it every conditional
        /// write ended as a `maybe`, decidable or not — and a `maybe` on memory the path
        /// proves untouched understates a real bug.
        ///
        /// `None` where the fault came from the arena-free `Bytes` path: those APIs have no
        /// `TermArena` to build a term with, so there is no guard to hand over. The engine
        /// then leaves the verdict alone, which is the same answer it gave before — an
        /// undischargeable `maybe` is still honest, and inventing a `true` here would claim
        /// a proof nobody has.
        guard: Option<Term>,
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
    /// A concrete access touched a byte that holds a *symbolic* value.
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
    /// **A pointer computed outside the object it derives from**, before any access.
    ///
    /// C11 6.5.6p8 makes the *computation* undefined once it goes more than one past the
    /// end, so this is a fault in its own right and not a weaker form of
    /// [`MemFault::OutOfBoundsMaybe`]. It carries no access size because there is no
    /// access — which is the concrete reason it needs its own variant rather than a flag:
    /// the caller of the access fault has to invent a width, and did.
    ///
    /// Ranked apart from an out-of-bounds access on purpose (023 §6): forming a pointer
    /// past the end is deliberate in a few real idioms, and touching bytes there is not.
    PointerOutsideObject {
        obj: ObjectId,
        obj_size: u64,
        /// An offset the path allows, which is past the object.
        witness: i64,
        at: Span,
    },
    /// **A read at a symbolic offset whose initialization is a question, not a fact.**
    ///
    /// 021 §3.1 puts the line here: the memory model knows *under what condition* every bit
    /// the read touches was written, and only the engine can decide whether that condition
    /// holds on this path. So this variant carries the question rather than a verdict, and
    /// the engine rewrites it into [`MemFault::Uninitialized`],
    /// [`MemFault::MaybeUninitialized`], or nothing at all.
    ///
    /// It reaches a report only when the solver cannot decide, which is why its message
    /// names no offset: there is no offset to name, and inventing one would be the "witness
    /// nobody can act on" 023 §9 rules out.
    UninitializedSymbolic {
        obj: ObjectId,
        /// The offset term the read used, so the engine can name a concrete offset from a
        /// model rather than describing the fault as "some value of `i`".
        off: Term,
        /// Holds exactly when every bit this read touches had been written.
        guard: Term,
        at: Span,
    },
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

impl MemFault {
    /// A stable slug for the *kind* of fault. 023 §6.1 deduplicates findings on
    /// `(checker, span, object, kind)`, and a `{:?}` dump has no kind in it that anything
    /// downstream can key on — the whole struct is the string.
    pub fn kind(&self) -> &'static str {
        match self {
            MemFault::OutOfBounds { .. } => "out-of-bounds",
            MemFault::Uninitialized { .. } => "uninitialized-read",
            MemFault::MaybeUninitialized { .. } => "maybe-uninitialized-read",
            MemFault::Misaligned { .. } => "misaligned",
            MemFault::UseAfterFree { .. } => "use-after-free",
            MemFault::DoubleFree { .. } => "double-free",
            MemFault::UseAfterScope { .. } => "use-after-scope",
            MemFault::ReadOnly { .. } => "write-to-readonly",
            MemFault::BadRange { .. } => "unsupported-access-width",
            MemFault::AllocationTooLarge { .. } => "allocation-too-large",
            MemFault::NullDeref { .. } => "null-dereference",
            MemFault::WildPointer { .. } => "wild-pointer",
            MemFault::SymbolicByte { .. } => "symbolic-byte",
            MemFault::OutOfBoundsMaybe { .. } => "may-be-out-of-bounds",
            MemFault::PointerOutsideObject { .. } => "pointer-outside-object",
            // Only ever seen when the solver could not decide, so `maybe` is the whole
            // truth about it; the engine renames the two cases it can settle.
            MemFault::UninitializedSymbolic { .. } => "maybe-uninitialized-read",
            MemFault::OverlappingCopy { .. } => "overlapping-copy",
            MemFault::BadFree { .. } => "bad-free",
        }
    }

    /// Whether the program **cannot continue** past this access.
    ///
    /// Not the same question as `yields_unknown_value`, and the two are almost complements:
    /// that one asks whether the *value* is trustworthy, this one whether the *path* still
    /// exists. A null dereference traps; a use-after-free or a definite out-of-bounds write
    /// is undefined behaviour with no defined continuation, so anything chiero reports
    /// afterwards is about a program that does not exist.
    ///
    /// Deliberately excludes the "chiero cannot answer" faults — `BadRange`,
    /// `AllocationTooLarge`, `SymbolicByte`, `MaybeUninitialized`, `OutOfBoundsMaybe`. Those
    /// are chiero's limits or *possibilities*, not facts about the program, and ending the
    /// path on one would silently drop the analysis of code that runs fine.
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            MemFault::NullDeref { .. }
                | MemFault::UseAfterFree { .. }
                | MemFault::DoubleFree { .. }
                | MemFault::UseAfterScope { .. }
                | MemFault::OutOfBounds { .. }
                | MemFault::BadFree { .. }
                | MemFault::WildPointer { .. }
        )
    }

    /// Whether the value this access produced is **not the program's**.
    ///
    /// The distinction matters for fidelity: a null dereference or a bad free is a
    /// definite fact about the program, and chiero modeled it exactly — the finding is
    /// the product, and degrading would say chiero was unsure when it was not. A read of
    /// uninitialized memory is the opposite: the report is right *and* whatever came back
    /// was invented, so anything computed from it is unsound.
    pub fn yields_unknown_value(&self) -> bool {
        matches!(
            self,
            MemFault::Uninitialized { .. }
                | MemFault::MaybeUninitialized { .. }
                | MemFault::UninitializedSymbolic { .. }
                | MemFault::SymbolicByte { .. }
                | MemFault::OutOfBoundsMaybe { .. }
                | MemFault::PointerOutsideObject { .. }
                | MemFault::BadRange { .. }
                | MemFault::WildPointer { .. }
                | MemFault::AllocationTooLarge { .. }
        )
    }

    /// Where the access was. The second component of 023 §6.1's dedup key.
    pub fn at(&self) -> Span {
        match self {
            MemFault::OutOfBounds { at, .. }
            | MemFault::Uninitialized { at, .. }
            | MemFault::MaybeUninitialized { at, .. }
            | MemFault::UninitializedSymbolic { at, .. }
            | MemFault::Misaligned { at, .. }
            | MemFault::UseAfterFree { at, .. }
            | MemFault::DoubleFree { at, .. }
            | MemFault::UseAfterScope { at, .. }
            | MemFault::ReadOnly { at, .. }
            | MemFault::BadRange { at, .. }
            | MemFault::AllocationTooLarge { at, .. }
            | MemFault::NullDeref { at, .. }
            | MemFault::WildPointer { at, .. }
            | MemFault::SymbolicByte { at, .. }
            | MemFault::OutOfBoundsMaybe { at, .. }
            | MemFault::PointerOutsideObject { at, .. }
            | MemFault::OverlappingCopy { at, .. }
            | MemFault::BadFree { at, .. } => *at,
        }
    }

    /// Which object, where there is one. `NullDeref`, `WildPointer` and `BadRange` have
    /// none by construction — that absence is the finding.
    pub fn object(&self) -> Option<ObjectId> {
        match self {
            MemFault::OutOfBounds { obj, .. }
            | MemFault::Uninitialized { obj, .. }
            | MemFault::MaybeUninitialized { obj, .. }
            | MemFault::UninitializedSymbolic { obj, .. }
            | MemFault::Misaligned { obj, .. }
            | MemFault::UseAfterFree { obj, .. }
            | MemFault::DoubleFree { obj, .. }
            | MemFault::UseAfterScope { obj, .. }
            | MemFault::ReadOnly { obj, .. }
            | MemFault::AllocationTooLarge { obj, .. }
            | MemFault::SymbolicByte { obj, .. }
            | MemFault::OutOfBoundsMaybe { obj, .. }
            | MemFault::PointerOutsideObject { obj, .. }
            | MemFault::OverlappingCopy { obj, .. }
            | MemFault::BadFree { obj, .. } => Some(*obj),
            MemFault::BadRange { .. }
            | MemFault::NullDeref { .. }
            | MemFault::WildPointer { .. } => None,
        }
    }
}

/// A sentence, not a struct dump. The findings a run produces are the product — 001 §1
/// puts an LLM at the other end of them — and `Uninitialized { obj: ObjectId(2), off: 0,
/// bit: 0, at: Span { lo: BytePos(0), … } }` makes a reader decode chiero's internals to
/// learn that byte 0 was never written.
impl std::fmt::Display for MemFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: ", self.kind())?;
        match self {
            MemFault::OutOfBounds {
                obj,
                off,
                size,
                obj_size,
                ..
            } => write!(
                f,
                "{size}-byte access at offset {off} of {obj:?}, which is {obj_size} bytes"
            ),
            MemFault::Uninitialized { obj, off, bit, .. } => write!(
                f,
                "read at offset {off} of {obj:?} touches bit {bit}, which was never written"
            ),
            MemFault::MaybeUninitialized { obj, off, bit, .. } => write!(
                f,
                "read at offset {off} of {obj:?} touches bit {bit}, written only under a                  guard the engine has not discharged"
            ),
            MemFault::Misaligned { obj, off, want, .. } => write!(
                f,
                "access at offset {off} of {obj:?} wants {want}-byte alignment"
            ),
            // **Both spans, not just the access.** 024 contracts 8 and 10 ask these to
            // name where the object died as well as where it was touched: "freed
            // earlier" sends a reader looking, and the engine has known the answer all
            // along — the fault has carried it since it was defined.
            MemFault::UseAfterFree { obj, freed_at, .. } => write!(
                f,
                "{obj:?} was freed at bytes {}..{} before this access",
                freed_at.lo.0, freed_at.hi.0
            ),
            MemFault::DoubleFree { obj, freed_at, .. } => write!(
                f,
                "{obj:?} was already freed at bytes {}..{}",
                freed_at.lo.0, freed_at.hi.0
            ),
            MemFault::UseAfterScope {
                obj,
                scope_ended_at,
                ..
            } => write!(
                f,
                "{obj:?} left scope at bytes {}..{}, before this access",
                scope_ended_at.lo.0, scope_ended_at.hi.0
            ),
            MemFault::ReadOnly { obj, off, .. } => {
                write!(f, "write at offset {off} of read-only {obj:?}")
            }
            MemFault::BadRange {
                want_bits,
                max_bits,
                ..
            } => write!(
                f,
                "{want_bits}-bit access exceeds the {max_bits}-bit limit chiero can carry"
            ),
            MemFault::AllocationTooLarge { obj, size, .. } => write!(
                f,
                "{obj:?} at {size} bytes is past the {MAX_MATERIALIZED_BYTES}-byte limit,                  so it is not materialized and every access to it faults"
            ),
            MemFault::NullDeref { off, .. } => write!(f, "access at offset {off} of NULL"),
            MemFault::WildPointer { off, .. } => write!(
                f,
                "access through a pointer at address {off} matching no known object"
            ),
            MemFault::SymbolicByte { obj, off, .. } => write!(
                f,
                "byte {off} of {obj:?} holds a symbolic value, which a concrete access                  cannot answer for"
            ),
            // No offset, on purpose: this variant only reaches a report when the solver
            // could not decide the guard, and every offset it could name would be a guess.
            MemFault::UninitializedSymbolic { obj, .. } => write!(
                f,
                "a read at a symbolic offset of {obj:?} may touch bytes that were never \
                 written, and the solver could not settle which"
            ),
            MemFault::PointerOutsideObject {
                obj,
                obj_size,
                witness,
                ..
            } => write!(
                f,
                "a pointer into {obj:?} ({obj_size} bytes) can be computed at offset \
                 {witness}, which is outside it"
            ),
            MemFault::OutOfBoundsMaybe {
                obj,
                size,
                obj_size,
                witness,
                ..
            } => write!(
                f,
                "{size}-byte access of {obj:?} ({obj_size} bytes) may be out of bounds —                  offset {witness} is"
            ),
            MemFault::OverlappingCopy {
                obj,
                dst,
                src,
                size,
                ..
            } => write!(
                f,
                "{size}-byte copy within {obj:?} from {src} to {dst} overlaps, which                  `memcpy` forbids"
            ),
            MemFault::BadFree { obj, kind, .. } => {
                write!(f, "free of {obj:?}, which is {kind:?} memory, not heap")
            }
        }
    }
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
    /// **Shared until written** (021 contract 20). Forking a state clones `Memory`, and a
    /// `MemObject` held by value copied every byte of every object — quadratic in the
    /// program's memory rather than in its branching, at the engine's most frequent
    /// operation. `Arc::make_mut` clones exactly the one object a write touches.
    obj: Option<std::sync::Arc<MemObject>>,
    repr: Repr,
    /// Present exactly when `repr == Repr::Array`.
    arr: Option<ArrayContents>,
    kind: ObjKind,
    size: u64,
    align: u64,
    state: ObjState,
    readonly: bool,
    /// **Lazily materialized** (021 §6): the bytes were written by something outside the
    /// analysis, so they are *unknown* rather than *unwritten*, and they are materialized
    /// **on first dereference** rather than at entry.
    ///
    /// Filling eagerly instead cost one term per byte per *state* — `State` owns `Memory`
    /// and a fork clones it — which measured 1.3 GB at 8192 states with one 4 KiB pointee
    /// and aborted the process with four. §6 says "on first dereference" for this reason,
    /// and a comment here once called that "the optimisation, not the correctness", which
    /// inverted the spec's own sentence. Found by review.
    lazy: bool,
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
    /// **Every symbol this memory has minted**, in creation order (023 §9).
    ///
    /// Memory mints symbols the engine never sees — havoc'd extern pointees, clobbered
    /// bytes, lazily-materialized contents — and 023 §9 lists "lazily-materialized object
    /// contents" among the things a witness must bind. Without this record the engine
    /// reported "no symbolic inputs on this path" for a path whose whole condition was
    /// built from them, which is not a gap in the witness but a false statement in it.
    minted: Vec<MintedSymbol>,
    entries: Vec<(ObjectId, Entry)>,
    /// Names the arrays a havoc installs. Per-`Memory` and monotone, so two havocs are
    /// two unrelated unknowns and a re-run produces the same names (001 §5).
    havoc_seq: u64,
}

/// Holds exactly when all eight bits of the byte at index `i` had been written.
///
/// `arr.data` is byte-indexed and `arr.init` bit-indexed, so the byte index scales by eight
/// and the guard is a conjunction over the eight bits — the same arithmetic
/// `write_at_symbolic_offset` uses to mark them, and the reason the two must be read together.
///
/// `None` when the chain is too long to eliminate. That is a real answer and not a failure:
/// the alternative is an expansion whose size grows with the number of stores ever made to
/// the object, and a caller that gets `None` reports nothing rather than guessing.
fn init_guard(a: &mut TermArena, arr: ArrayContents, i: Term) -> Option<Term> {
    // Generous, because the chain is per-object and the common shapes are a constant array
    // (length 0) or a handful of concrete stores. A `memset` is what makes it long, and a
    // `memset` is also what makes the answer uninteresting.
    const EXPAND_LIMIT: usize = 256;
    let eight = a.bv(arr.idx_bits, 8);
    let base = a.mul(i, eight);
    let one = a.bv(1, 1);
    let mut acc: Option<Term> = None;
    for k in 0..8u128 {
        let off_k = a.bv(arr.idx_bits, k);
        let bi = a.add(base, off_k);
        let bit = a.select_expand(arr.init, bi, EXPAND_LIMIT)?;
        let is_set = a.eq(bit, one);
        acc = Some(match acc {
            None => is_set,
            Some(prev) => a.and(prev, is_set),
        });
    }
    acc
}

impl Memory {
    pub fn new() -> Memory {
        Memory {
            space: AddressSpace::new(),
            entries: Vec::new(),
            minted: Vec::new(),
            havoc_seq: 0,
        }
    }

    pub fn alloc(&mut self, kind: ObjKind, size: u64, align: u64, at: Span) -> ObjectId {
        // The **true** size goes to the address space. Truncating it there while the
        // entry recorded the real one made `int_to_ptr`'s range search and `in_bounds`
        // disagree with the object about how big it is.
        let id = self.space.alloc(kind, size, align, at);
        // Oversized objects are recorded but not materialized: every access faults, which
        // is a finding rather than a dead process.
        let obj = (size <= MAX_MATERIALIZED_BYTES)
            .then(|| std::sync::Arc::new(MemObject::new(id, kind, size, align, at)));
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
                lazy: false,
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
            if let Some(o) = e.obj.as_mut().map(std::sync::Arc::make_mut) {
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
        let lazy = e.lazy;
        let obj = e.obj.as_ref().expect("materialized");
        // **A lazy object's unwritten byte is unknown, and this API cannot say so with a
        // value.** It has no arena, so it cannot mint the symbol `read_term` would — and
        // minting into a scratch arena would hand back a term from an arena nobody else
        // has, which is the identity hazard 022 §6.2 warns about. `SymbolicByte` is the
        // honest report: a concrete access cannot answer for a value the caller chose.
        // It is *not* `Uninitialized`, because nobody failed to write these (021 §6).
        if lazy && let Some(b) = obj.init.first_no(p.off.max(0) as u64 * 8, size * 8) {
            faults.push(MemFault::SymbolicByte {
                obj: p.base,
                off: (b / 8) as i64,
                at,
            });
            return AccessResult {
                value: obj.raw_bytes(p.off, size),
                faults,
            };
        }
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
                    // No arena on this path, so no guard to hand over.
                    guard: None,
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

    /// A byte-wise write: no alignment requirement, because `memcpy` and friends move
    /// bytes and C imposes none on them. The scalar rule — an N-byte access wants N-byte
    /// alignment — is about scalar loads and stores, and applying it here makes every
    /// `strcpy` into a `char` buffer a false positive.
    pub fn write_bytewise(&mut self, p: Pointer, bytes: &[u8], at: Span) -> AccessResult<()> {
        let mut r = self.write(p, bytes, at);
        r.faults
            .retain(|f| !matches!(f, MemFault::Misaligned { .. }));
        r
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
        let obj = e
            .obj
            .as_mut()
            .map(std::sync::Arc::make_mut)
            .expect("materialized");
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
                    // No arena on this path, so no guard to hand over.
                    guard: None,
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
        let Some(e) = self.entry(p.base) else {
            return AccessResult::fault(MemFault::WildPointer { off: p.off, at });
        };
        // **Ask before cloning.** `MemObject::write_bits` refuses an out-of-range or
        // symbolic write, and calling `make_mut` first paid for a private copy of the
        // object on every refusal — the common case for a bit write into a symbolic byte.
        // The check is the same code the write runs, so the two cannot disagree.
        let obj = e.obj.as_ref().expect("materialized");
        if let Err(err) = obj.check_bit_write(b, n_bits) {
            faults.push(lift(err, p.base, at));
            return AccessResult {
                value: None,
                faults,
            };
        }
        let obj = self
            .entry_mut(p.base)
            .and_then(|e| e.obj.as_mut())
            .map(std::sync::Arc::make_mut)
            .expect("materialized");
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

    /// 024 §2.1. Invalidate `objects` and, to `depth`, whatever they point at. Returns
    /// every object actually invalidated, so the caller can say what it gave up on.
    ///
    /// Breadth-first with a visited set: a linked structure that points back at itself is
    /// the normal case for the VPP pools §2 targets, and a depth counter alone would walk
    /// it `depth` times.
    pub fn havoc(
        &mut self,
        a: &mut TermArena,
        objects: &[ObjectId],
        depth: u32,
        fill: HavocFill,
        at: Span,
    ) -> Havocked {
        let mut out = Havocked::default();
        // Visited is separate from `out.objects`: an object that was *skipped* must not
        // be reported as invalidated, but it still must not be walked twice.
        let mut visited: Vec<ObjectId> = Vec::new();
        let mut frontier: Vec<ObjectId> = objects.to_vec();
        for _ in 0..=depth {
            let mut next = Vec::new();
            for id in std::mem::take(&mut frontier) {
                if visited.contains(&id) {
                    continue;
                }
                visited.push(id);
                // Read the pointees **before** the fill, since the fill is what destroys
                // the addresses they were found through.
                let (found, complete) = self.pointees(id);
                next.extend(found);
                if self.havoc_object(a, id, fill, at) {
                    out.objects.push(id);
                    // Only an object that was actually invalidated can have lost
                    // reachability; a skip loses nothing.
                    out.truncated |= !complete;
                }
            }
            frontier = next;
        }
        out
    }

    /// Every live object's id and address range, for 021 §5.1's resolution search.
    ///
    /// §5.1 calls for an interval tree keyed on base address, because §8 concedes a VPP
    /// entry point may exceed 10⁴ objects and a per-dereference O(objects) *solver* sweep
    /// is not viable. This returns the ranges so the caller can do the cheap arithmetic
    /// filter first and ask the solver only about what survives it — the tree is the
    /// optimisation, the filter is the semantics.
    pub fn live_ranges(&self) -> Vec<(ObjectId, u64, u64)> {
        let at = self.placements();
        self.entries
            .iter()
            .filter(|(_, e)| e.state == ObjState::Live)
            .filter_map(|(id, e)| at.get(id).map(|a| (*id, *a, e.size)))
            .collect()
    }

    /// Every placed object's address, by id, in one pass.
    ///
    /// `AddressSpace::addr_of` is a linear scan, so calling it per entry made building
    /// the ranges O(objects²) — on the very path §5.1 says must not be linear.
    fn placements(&self) -> indexmap::IndexMap<ObjectId, u64> {
        self.space.objs.iter().map(|(i, a, _)| (*i, *a)).collect()
    }

    /// Mark an object lazily materialized — 021 §6. See `Entry::lazy`.
    pub fn mark_lazy(&mut self, id: ObjectId) {
        if let Some(e) = self.entry_mut(id) {
            e.lazy = true;
        }
    }

    /// An object's declared alignment — 021 §7.2 needs it to tell a fact about the object
    /// from an answer the bump allocator invented.
    pub fn align_of(&self, id: ObjectId) -> Option<u64> {
        self.entry(id).and_then(|e| e.obj.as_ref()).map(|o| o.align)
    }

    /// Every symbol this memory invented, in creation order — 023 §9's
    /// "lazily-materialized object contents", plus havoc.
    pub fn minted_symbols(&self) -> &[MintedSymbol] {
        &self.minted
    }

    /// Every object a *pointer* may resolve to, **including freed and out-of-scope ones**.
    ///
    /// 021 §4 keeps those entries precisely so a use-after-free can name its site, and the
    /// §5.1 search is the one consumer that must see them: an address the path pins to a
    /// freed block resolves to *nothing* under `live_ranges`, so the run reports a wild
    /// pointer at address 0 rather than the use-after-free. The access itself still faults
    /// — `state_fault` is what turns the resolution into the finding.
    pub fn resolvable_ranges(&self) -> Vec<(ObjectId, u64, u64)> {
        let at = self.placements();
        // **`NULL` is one of them.** 021 §1 gives it address 0 and size 0, and
        // `AddressSpace::int_to_ptr` has special-cased a *concrete* zero since the day
        // "matching no known object" turned up as the report for a plain null
        // dereference. A symbolic address the path pins to 0 has the same right to the
        // same answer: without an entry here the search finds nothing at 0, and
        // `WildPointer` carries `Unknown` where `NullDeref` is a definite finding.
        // Found by review.
        std::iter::once((ObjectId::NULL, 0, 0))
            .chain(
                self.entries
                    .iter()
                    .filter_map(|(id, e)| at.get(id).map(|a| (*id, *a, e.size))),
            )
            .collect()
    }

    /// The largest address interval containing `a` in which **no resolvable object**
    /// lies — the wild region around `a`.
    ///
    /// This is what lets 021 §5.1's search rule out a whole region of the address space
    /// per solver query instead of one object per query: a model that lands in a guard
    /// gap proves the wild case once, and the *region* — not just that one address — is
    /// what the next query excludes.
    ///
    /// Computed over exactly the objects [`Memory::resolvable_ranges`] reports, not over
    /// every address the space has ever handed out. An object the space placed but this
    /// memory has no entry for can never be a candidate, so a region spanning it is still
    /// a region the address may not be resolved in — but a region computed from the
    /// *space* would end at its edge, and excluding "the gap" would then quietly exclude
    /// its living neighbours too. Bounds are inclusive on both ends.
    pub fn wild_region_around(&self, a: u64) -> (u64, u64) {
        let (mut lo, mut hi) = (0u64, u64::MAX);
        for (_, base, size) in self.resolvable_ranges() {
            // **`wrapping_add`, matching the containment test the caller used.** With
            // `saturating_add` the two disagreed exactly when `base + size` overflows,
            // which is the one case that reaches the inside-an-object branch below — an
            // address the caller called wild and this called owned. Found by review.
            let top = base.wrapping_add(size);
            if top < a {
                lo = lo.max(top.saturating_add(1));
            } else if base > a {
                hi = hi.min(base.saturating_sub(1));
            } else {
                // `a` is inside a resolvable object after all: it is not wild, and the
                // only interval this can honestly claim is the point itself.
                return (a, a);
            }
        }
        (lo, hi)
    }

    /// Whether `id`'s storage is **the same allocation** in both memories — the only way
    /// to tell structural sharing from an identical copy, which is what 021 contract 20
    /// asks for.
    pub fn shares_storage_with(&self, other: &Memory, id: ObjectId) -> bool {
        match (
            self.entry(id).and_then(|e| e.obj.as_ref()),
            other.entry(id).and_then(|e| e.obj.as_ref()),
        ) {
            (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
            // **Nothing to share is shared.** An object past `MAX_MATERIALIZED_BYTES`
            // has no storage in either memory, so a fork cannot have copied it —
            // answering `false` conflated "not shared" with "nothing to share", and
            // would report a phantom copy in any accounting built on this.
            (None, None) => self.entry(id).is_some() && other.entry(id).is_some(),
            _ => false,
        }
    }

    /// How many objects this memory holds. Only for tests: a caller that needs to know
    /// how many objects exist is usually asking the wrong question, but "did this loop
    /// allocate once or three times" has no other observable.
    pub fn object_count_for_test(&self) -> usize {
        self.entries.len()
    }

    /// The address `id` was placed at. `Memory` owns the address space, so a caller that
    /// needs to *store* a pointer — which is how a linked structure is built — cannot get
    /// at it otherwise.
    pub fn addr_of(&self, id: ObjectId) -> Option<u64> {
        self.space.addr_of(id)
    }

    /// Write a term of `size` bytes at a concrete pointer, in target byte order.
    ///
    /// A ground term goes down as concrete bytes; anything else goes into the overlay one
    /// `Extract` per byte, which is what keeps `*p = x; y = *p;` giving back the *same*
    /// unknown rather than a fresh one. Writing the model's bytes behind a symbol would
    /// lose every constraint derived from it.
    pub fn write_term(
        &mut self,
        a: &mut TermArena,
        p: Pointer,
        t: Term,
        size: u64,
        e: Endian,
        at: Span,
    ) -> AccessResult<()> {
        // State first, as in `read_term`, so a chiero limit cannot mask a memory-safety
        // bug. The width check then guards the ground path, which shifts a `u128` by
        // `8 * size` and panics at sixteen bytes and over.
        if let Some(f) = self.state_fault(p.base, p.off, at) {
            return AccessResult::fault(f);
        }
        if let Some(f) = too_wide(size, at) {
            return AccessResult::fault(f);
        }
        // **A promoted object is written through its array, and this check comes before the
        // ground-constant fast path below.**
        //
        // The fast path delegates to `write`, which is arena-free and *refuses* a promoted
        // object — so once any symbolic write promoted an object, every ordinary store of a
        // constant into it declined and the value was lost. Wave 197 shipped that refusal
        // knowingly and pinned it; wave 198 added an array branch further down this function
        // and it never ran, because a constant store returns here first. Order is the whole
        // fix.
        //
        // Symmetric with `read_term`, which has read through the array all along: the
        // `Bytes` view is frozen at promotion, so a write that went there while the read came
        // from the array would let the two representations drift.
        if self.entry(p.base).is_some_and(|x| x.repr == Repr::Array) {
            let obj_size = self.entry(p.base).map_or(0, |x| x.size);
            let mut faults = Vec::new();
            for i in 0..size {
                let idx = if e == Endian::Big { size - 1 - i } else { i };
                let lo = (idx * 8) as u32;
                let byte = a.extract(t, lo + 7, lo);
                let off = p.off + i as i64;
                // Promotion changes the representation, not the object's extent.
                if off < 0 || off as u64 >= obj_size {
                    faults.push(MemFault::OutOfBounds {
                        obj: p.base,
                        off,
                        size: 1,
                        obj_size,
                        at,
                    });
                    continue;
                }
                let Some(arr) = self.entry(p.base).and_then(|x| x.arr) else {
                    break;
                };
                let ix = a.bv(arr.idx_bits, off as u128);
                let data = a.store(arr.data, ix, byte);
                // **`init` is indexed per *bit*, `data` per byte** — and getting that wrong
                // is invisible except as a `maybe-uninitialized-read` on a byte the program
                // just stored. `init_bit_via` selects `arr.init` at `bit`, and the candidate
                // write above loops `b * 8 .. b * 8 + 8`; a byte-indexed store here writes
                // one bit of the wrong byte and leaves the other eight unset.
                let one = a.bv(1, 1);
                let mut init = arr.init;
                for bit in off as u64 * 8..off as u64 * 8 + 8 {
                    let bi = a.bv(arr.idx_bits, bit as u128);
                    init = a.store(init, bi, one);
                }
                if let Some(entry) = self.entry_mut(p.base)
                    && let Some(arr) = entry.arr.as_mut()
                {
                    arr.data = data;
                    arr.init = init;
                }
            }
            return AccessResult {
                value: Some(()),
                faults,
            };
        }
        if let Ok(c) = a.eval_ground(t) {
            let v = c.bits();
            let mut bytes: Vec<u8> = (0..size).map(|i| (v >> (8 * i)) as u8).collect();
            if e == Endian::Big {
                bytes.reverse();
            }
            return self.write(p, &bytes, at);
        }
        // Bounds and state are checked once, by a zero-fill write of the whole range —
        // going byte by byte would report the same out-of-bounds `size` times.
        let probe = self.write(p, &vec![0u8; size as usize], at);
        if probe.value.is_none() {
            return probe;
        }
        let mut faults = probe.faults;
        // **Widen a term narrower than the store**, rather than extracting past its end.
        //
        // A `_Bool` is one *byte* of storage holding a one-*bit* value: `Cmp` yields a
        // 1-bit term and the object is 8 bits wide, so splitting the value into bytes ran
        // `extract(t, 7, 0)` over a one-bit term and panicked. 023 says the engine does not
        // crash on input it was handed, and zero-extension is the only widening that can be
        // right here — the value's own width says what it is, and the bits above it are not
        // part of it.
        //
        // Found by wave 169: the float path reached this first because `(_Bool)f` is a
        // comparison, but nothing about it is float-specific and the integer `_Bool` store
        // had the same shape waiting.
        let want_bits = (size * 8) as u32;
        let t = if a.width(t) < want_bits {
            a.zext(t, want_bits)
        } else {
            t
        };
        for i in 0..size {
            let idx = if e == Endian::Big { size - 1 - i } else { i };
            let lo = (idx * 8) as u32;
            let byte = a.extract(t, lo + 7, lo);
            let w = self.write_sym_byte(
                Pointer {
                    base: p.base,
                    off: p.off + i as i64,
                },
                byte,
                at,
            );
            faults.extend(w.faults);
        }
        AccessResult {
            value: Some(()),
            faults,
        }
    }

    /// Put bytes down **without** marking them initialized — the state fresh heap memory
    /// is in. Only for tests: no program operation writes bytes it does not also
    /// initialize, and the distinction is exactly what `pointees` turns on.
    pub fn write_uninit_bytes_for_test(&mut self, p: Pointer, bytes: &[u8]) {
        if let Some(e) = self.entry_mut(p.base)
            && let Some(o) = e.obj.as_mut().map(std::sync::Arc::make_mut)
        {
            o.write_raw_uninit(p.off, bytes);
        }
    }

    /// 021 §7.1's **fallback**: the object containing `addr`, by range search. Wrong in
    /// both directions by construction — an integer that happens to land inside an object
    /// is followed to it, and a pointer whose object was freed resolves to whatever now
    /// occupies the address — so a caller must record that it guessed. Provenance, when
    /// there is any, is the caller's to keep; `Memory` cannot see where a term came from.
    pub fn object_containing(&self, addr: u64) -> Pointer {
        self.space.int_to_ptr(IntVal::Const(addr))
    }

    /// The NUL-terminated string at `p`, if every byte of it is concrete and
    /// initialized. `None` for anything else — a *partly* readable string is not a
    /// string, and guessing at the readable prefix would put a truncated reason in a
    /// report that reads as the whole one.
    ///
    /// Bounded by the object, so an unterminated buffer gives `None` rather than a walk.
    pub fn c_string_at(&mut self, p: Pointer) -> Option<String> {
        let size = self.size_of_pub(p.base)?;
        let from = u64::try_from(p.off).ok()?;
        let mut out = Vec::new();
        for i in from..size {
            let r = self.read(
                Pointer {
                    base: p.base,
                    off: i as i64,
                },
                1,
                Span::DUMMY,
            );
            if !r.faults.is_empty() {
                return None;
            }
            match r.value.as_deref() {
                Some([0]) => return String::from_utf8(out).ok(),
                Some([b]) => out.push(*b),
                _ => return None,
            }
        }
        None
    }

    /// Objects reachable from `id`'s bytes. Provenance is not stored in bytes, so this is
    /// the same range search `int_to_ptr` falls back to: an aligned pointer-sized word
    /// whose value lands inside a live object. Wrong in both directions in principle — an
    /// integer that happens to look like an address is followed, and a pointer split
    /// across a union is not — which is why 021 §7.1 keeps it a *fallback*. For a havoc
    /// over-approximating is the safe direction.
    ///
    /// The second half of the pair is whether the scan was **complete**. "Nothing there"
    /// and "could not look" are different answers, and only one of them means the
    /// reachable set is closed.
    pub fn pointees(&self, id: ObjectId) -> (Vec<ObjectId>, bool) {
        let Some(e) = self.entry(id) else {
            return (Vec::new(), true);
        };
        // A promoted object has no byte view to scan. Its contents are symbolic, so no
        // concrete address can be recovered from them at all — which is *incomplete*,
        // not empty: a second havoc of the same object would otherwise silently stop
        // following the pointers the first one followed.
        let Some(o) = e.obj.as_ref().filter(|_| e.repr == Repr::Bytes) else {
            return (Vec::new(), false);
        };
        let limit = e.size.min(HAVOC_SCAN_BYTES);
        let mut out = Vec::new();
        let mut b = 0u64;
        while b + 8 <= limit {
            // Only *initialized, concrete* words: an uninitialized word is whatever the
            // allocator left there, and following it would invent a reference.
            let concrete = (b..b + 8).all(|k| o.sym_at(k).is_none())
                && o.init.first_no(b * 8, 64).is_none()
                && o.init.first_cond(b * 8, 64).is_none();
            if concrete {
                let mut w = [0u8; 8];
                for (k, slot) in w.iter_mut().enumerate() {
                    *slot = o.raw_byte(b + k as u64);
                }
                let p = self.space.int_to_ptr(IntVal::Const(u64::from_le_bytes(w)));
                if p.base != ObjectId::UNBOUND && p.base != ObjectId::NULL && p.base != id {
                    out.push(p.base);
                }
            }
            b += 8;
        }
        (out, limit == e.size)
    }

    /// Invalidate **exactly** `[p, p + size)`, leaving the rest of the object alone.
    ///
    /// Distinct from `havoc_object` because 020 §4.3's `Opaque` declares a *range*: an
    /// inline-asm block that says it clobbers eight bytes has said something precise, and
    /// widening that to the whole object would throw away the declaration's whole value.
    /// Returns whether the range was placed at all.
    ///
    /// `Symbolic` fills byte by byte through the overlay rather than promoting, so the
    /// object keeps its byte view and the *untouched* bytes stay concrete — promotion
    /// would take them with it.
    pub fn havoc_range(
        &mut self,
        a: &mut TermArena,
        p: Pointer,
        size: u64,
        fill: HavocFill,
        at: Span,
    ) -> bool {
        // **A refusal is not a success, whatever the size.** `refuse` reports `Some(0)`,
        // which equalled `Some(size)` at `size == 0` — so every refusal (freed, read-only,
        // promoted, wild) reported success for a zero-byte range.
        let r = self.havoc_range_reporting(a, p, size, fill, at);
        r.value == Some(size) && r.faults.is_empty()
    }

    /// As `havoc_range`, but says **how many bytes it managed** and carries the faults it
    /// hit. A bare `bool` could not distinguish "nothing happened" from "half happened",
    /// and it discarded the `OutOfBounds` from a declared clobber running past the end —
    /// so inline asm claiming to write sixteen bytes of an eight-byte buffer was an
    /// overflow chiero detected and did not report.
    pub fn havoc_range_reporting(
        &mut self,
        a: &mut TermArena,
        p: Pointer,
        size: u64,
        fill: HavocFill,
        at: Span,
    ) -> AccessResult<u64> {
        let refuse = |f: MemFault| AccessResult {
            value: Some(0),
            faults: vec![f],
        };
        if self.entry(p.base).is_none() {
            return refuse(MemFault::WildPointer { off: p.off, at });
        }
        // **A negative offset is left to the per-byte check**, which already reports
        // `OutOfBounds` *and names the object*. The original `WildPointer` here said
        // "matching no known object" about an object it had just looked up, lost the
        // object component of 023 §6.1's dedup key, and — being fatal — killed the path so
        // nothing after the asm block was analysed. A special case that duplicates the
        // loop's answer is worse than none: it is a second place for the two write paths
        // to disagree.
        // **The same refusals `havoc_object` makes.** They were on the `Symbolic` path
        // only, and by accident — `write_sym_byte` happens to check — so `Uninitialized`
        // mutated read-only and freed objects, and on a promoted one reported success
        // while changing nothing, which 020 §4.3 forbids.
        if let Some(f) = self.state_fault(p.base, p.off, at) {
            return refuse(f);
        }
        let e = self.entry(p.base).expect("checked");
        if e.readonly {
            return refuse(MemFault::ReadOnly {
                obj: p.base,
                off: p.off,
                at,
            });
        }
        if e.repr != Repr::Bytes || e.obj.is_none() {
            return refuse(MemFault::SymbolicByte {
                obj: p.base,
                off: p.off,
                at,
            });
        }
        let mut faults = Vec::new();
        let mut done = 0u64;
        for i in 0..size {
            let q = Pointer {
                base: p.base,
                off: p.off + i as i64,
            };
            match fill {
                HavocFill::Symbolic => {
                    self.havoc_seq += 1;
                    let name = format!("clobber{}", self.havoc_seq);
                    let t = a.var(chiero_solver::Sort::BitVec(8), &name);
                    self.minted.push(MintedSymbol {
                        term: t,
                        obj: q.base,
                        at,
                        why: "a byte clobbered by opaque code",
                        array: false,
                    });
                    let r = self.write_sym_byte(q, t, at);
                    faults.extend(r.faults);
                    if r.value.is_none() {
                        break;
                    }
                }
                HavocFill::Uninitialized => {
                    let ok = self
                        .entry(p.base)
                        .and_then(|e| e.obj.as_ref())
                        .is_some_and(|o| o.check_only(q.off, 1).is_ok());
                    if !ok {
                        faults.push(MemFault::OutOfBounds {
                            obj: p.base,
                            off: q.off,
                            size: 1,
                            obj_size: self.size_of_pub(p.base).unwrap_or(0),
                            at,
                        });
                        break;
                    }
                    if let Some(e) = self.entry_mut(p.base)
                        && let Some(o) = e.obj.as_mut().map(std::sync::Arc::make_mut)
                    {
                        o.init.set_exact_range_uninit(q.off as u64 * 8, 8);
                        o.sym.remove(&(q.off as u64));
                    }
                }
            }
            done += 1;
        }
        AccessResult {
            value: Some(done),
            faults,
        }
    }

    /// Invalidate one object's contents. `Symbolic` replaces them with an unconstrained
    /// array — the array representation is what makes this O(1) rather than one fresh
    /// variable per byte — and `Uninitialized` clears the init mask instead, leaving the
    /// bytes unreadable rather than unknown.
    ///
    /// Returns whether anything was invalidated: freed, read-only, unmaterialized and
    /// unknown objects are all skipped, and the caller must not count them.
    pub fn havoc_object(
        &mut self,
        a: &mut TermArena,
        id: ObjectId,
        fill: HavocFill,
        at: Span,
    ) -> bool {
        if let Some(f) = self.state_fault(id, 0, at) {
            // Havocking freed memory is not an access the program made, so there is
            // nothing to report — and nothing to invalidate either.
            let _ = f;
            return false;
        }
        let Some(e) = self.entry(id) else {
            return false;
        };
        let size = e.size;
        if e.obj.is_none() {
            return false;
        }
        // **Read-only objects are not written, by anyone.** Every other write path
        // refuses; a callee writing through a `const char *` is UB, so invalidating here
        // would discard what the standard guarantees — and since `Symbolic` promotes, the
        // object would come back unreadable rather than merely changed.
        if e.readonly {
            return false;
        }
        match fill {
            HavocFill::Symbolic => {
                self.havoc_seq += 1;
                let data = a.array_var(64, 8, &format!("havoc{}", self.havoc_seq));
                self.minted.push(MintedSymbol {
                    term: data,
                    obj: id,
                    at,
                    why: "the contents of an object written by code with no model",
                    array: true,
                });
                let init = a.array_const(64, 1, 1);
                if let Some(e) = self.entry_mut(id) {
                    e.repr = Repr::Array;
                    e.arr = Some(ArrayContents {
                        data,
                        init,
                        idx_bits: 64,
                    });
                }
            }
            HavocFill::Uninitialized => {
                if let Some(e) = self.entry_mut(id) {
                    if let Some(o) = e.obj.as_mut().map(std::sync::Arc::make_mut) {
                        o.clear_contents(size);
                    }
                    // 021 §3: promotion is **one-way within a state**. Clearing `arr`
                    // here de-promoted a promoted object and discarded its array
                    // contents, so a read after it answered from stale bytes.
                    if e.repr == Repr::Array {
                        let init = a.array_const(64, 1, 0);
                        if let Some(arr) = e.arr.as_mut() {
                            arr.init = init;
                        }
                    }
                }
            }
        }
        true
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
        let Some((bytes, init, sym)) = read.value else {
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
        let w = self.write_bytewise(dst, &bytes, at);
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
            && let Some(o) = e.obj.as_mut().map(std::sync::Arc::make_mut)
        {
            o.restore_init(dst.off as u64 * 8, &init);
            // The **triple** carries across. `write_bytewise` cleared the destination's
            // overlay for every byte it touched, which is right for the concrete ones and
            // exactly wrong for these.
            for (i, t) in sym.iter().enumerate() {
                if let Some(t) = t {
                    o.set_sym_at(dst.off as u64 + i as u64, *t);
                }
            }
        }
        AccessResult {
            value: Some(()),
            faults,
        }
    }

    /// 021 contract 28: the range becomes initialized and reads back as the set byte.
    pub fn set(&mut self, dst: Pointer, byte: u8, size: u64, at: Span) -> AccessResult<()> {
        // **Guard before materializing.** `vec![byte; size]` ran before any check, so a
        // `calloc(1, 1 << 45)` killed the process — reintroducing exactly the abort
        // `MAX_MATERIALIZED_BYTES` exists to prevent, one level up from where the guard
        // sits. An abort is not something `catch_unwind` can contain.
        if size > MAX_MATERIALIZED_BYTES {
            return AccessResult::fault(MemFault::AllocationTooLarge {
                obj: dst.base,
                size,
                at,
            });
        }
        let bytes = vec![byte; size as usize];
        self.write_bytewise(dst, &bytes, at)
    }

    /// The source side of a `copy`: every check `read` makes **except** alignment (a copy
    /// is byte-wise) and **except** the uninitialized-read fault and memoization.
    ///
    /// A copy moves bytes without using them. `memcpy` of a partially-filled struct is
    /// ubiquitous and correct, so reporting there is a false-positive storm — and
    /// memoizing would mark the source initialized, defeating the propagation this
    /// function exists for. The finding belongs at the eventual *use* of the destination,
    /// which is why the status is carried rather than consumed.
    fn read_raw(&mut self, p: Pointer, size: u64, at: Span) -> AccessResult<RawBytes> {
        // Byte-wise, so no alignment fault here — but every *other* check `read` makes
        // still applies. Omitting them let a copy launder what a read refuses: a promoted
        // object served its frozen `Bytes` view, and a symbolic byte came back concrete
        // with no fault, which turns a `memcpy` of a struct with a symbolic field into a
        // silent constant.
        if let Some(f) = self.state_fault(p.base, p.off, at) {
            return AccessResult::fault(f);
        }
        if let Some(f) = self.too_large(p.base, at) {
            return AccessResult::fault(f);
        }
        if let Some(f) = self.promoted_fault(p, at) {
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
        // The overlay comes **with** the bytes. Reporting a `SymbolicByte` fault and then
        // handing back the stale concrete byte behind it stopped the copy being *silent*
        // without stopping it being a *constant* — the destination held a fabricated
        // value and read clean forever after. A byte-wise copy is exactly the operation
        // that can answer for a symbolic byte, which is what a scalar read cannot do.
        let sym: Vec<Option<Term>> = (0..size).map(|b| obj.sym_at(p.off as u64 + b)).collect();
        let faults = Vec::new();
        AccessResult {
            value: Some((bytes, init, sym)),
            faults,
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
    pub fn size_of_pub(&self, id: ObjectId) -> Option<u64> {
        self.entry(id).map(|e| e.size)
    }

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
        // **Every refusal decided before `make_mut`.** Cloning first meant an operation
        // that changes nothing still paid for a private copy of the object — which undoes
        // contract 20 on the refusal path, and refusals are the common case for a bit
        // write into a symbolic byte.
        let Some(e) = self.entry(p.base) else {
            return AccessResult::fault(MemFault::WildPointer { off: p.off, at });
        };
        let obj_size = e.size;
        let Some(o) = e.obj.as_ref() else {
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
                obj_size,
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
        let o = self
            .entry_mut(p.base)
            .and_then(|e| e.obj.as_mut())
            .map(std::sync::Arc::make_mut)
            .expect("checked above");
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
        // **State before contents** (021 §5), which is why this is not the first check.
        // `BadRange` is a *chiero* limit; a use-after-free is a fact about the program,
        // and reporting the limit instead hides the bug. Same lesson as "bounds must
        // precede alignment", on a new surface.
        if let Some(f) = self.state_fault(p.base, p.off, at) {
            return AccessResult::fault(f);
        }
        // The limit the byte and bit APIs enforce. Omitting it was a process kill rather
        // than a fault: a concrete read folds its `Concat` chain into a `BvConst` wider
        // than the arena allows and asserts.
        if let Some(f) = too_wide(size, at) {
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
        // **A lazy object's unwritten bytes are unknown, not unwritten** (021 §6). They
        // are given symbols here — on first dereference, which is what §6 asks — and
        // marked initialized, so the scan below finds nothing to report and the value is
        // one nobody has claimed rather than the backing store's zero. Suppressing only
        // the *finding* would leave that zero, which 021 §3.1 calls the single most
        // common way a symbolic executor is confidently wrong.
        if self.entry(p.base).is_some_and(|e| e.lazy) && p.off >= 0 {
            self.materialize_fresh(a, p.base, p.off, size);
            self.memoize_via(a, p.base, p.off as u64 * 8, size * 8);
        }
        let range = p.off as u64 * 8..p.off as u64 * 8 + size * 8;
        let mut first_no = None;
        let mut first_cond: Option<(u64, Term)> = None;
        for bit in range {
            match self.init_bit_via(a, p.base, bit) {
                InitBit::No if first_no.is_none() => first_no = Some(bit),
                // The guard travels with the bit. Reporting the *first* conditional bit is
                // unchanged; what is new is that its condition comes too.
                InitBit::Cond(t) if first_cond.is_none() => first_cond = Some((bit, t)),
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
            // **021 §3: a fresh symbol, never zero.** Handing back the `0` behind an
            // uninitialized byte is what the spec calls the single most common way a
            // symbolic executor produces confidently wrong results — a checker then
            // reasons about a value nobody wrote.
            //
            // Minting and memoizing are one act: contract 26 wants a repeated read to
            // give the same term, §3 wants that term to be a symbol, and doing either
            // alone satisfies neither.
            if p.off >= 0 {
                self.materialize_fresh(a, p.base, p.off, size);
                self.memoize_via(a, p.base, p.off as u64 * 8, size * 8);
            }
        } else if let Some((bit, guard)) = first_cond {
            faults.push(MemFault::MaybeUninitialized {
                obj: p.base,
                off: p.off,
                bit,
                guard: Some(guard),
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

    /// Read one byte at a **symbolic** offset (021 §3).
    ///
    /// Within `ITE_THRESHOLD` the answer is an if-then-else chain over the candidates and
    /// the object stays on the fast path; past it the object promotes and the answer is a
    /// single `select`. The threshold governs both directions — it had governed only the
    /// write side, which is half of what §3 says.
    ///
    /// An empty candidate list means "no pinning available", which is a promoted object's
    /// normal case: the select is unpinned and no enumeration happens.
    pub fn read_term_at(
        &mut self,
        a: &mut TermArena,
        id: ObjectId,
        off: Term,
        candidates: &[u64],
        at: Span,
    ) -> AccessResult<Term> {
        if let Some(f) = self.state_fault(id, 0, at) {
            return AccessResult::fault(f);
        }
        if candidates.len() > ITE_THRESHOLD || candidates.is_empty() {
            let r = self.promote_to_array(a, id);
            if !r.faults.is_empty() {
                return AccessResult {
                    value: None,
                    faults: r.faults,
                };
            }
        }
        if let Some(arr) = self.entry(id).and_then(|e| e.arr) {
            let i = fit(a, off, arr.idx_bits);
            // **The initialization check, which wave 202 built and rejected.** Its objection was
            // that proving the byte written needs `select` to fold past seven non-matching
            // stores, so a byte written at the *same* symbolic offset came back
            // `maybe-uninitialized-read` — a false report on memory the program definitely
            // wrote, worse than the silence it replaced.
            //
            // Two things removed that objection. `select_expand` turns the array into `ite`
            // comparisons, so the question reaches a solver as bitvector arithmetic instead of
            // needing the walk to fold; and wave 204's discharge gives an unresolved guard
            // somewhere to go. Neither existed when wave 202 wrote the check.
            let guard = init_guard(a, arr, i);
            let value = Some(a.select(arr.data, i));
            // Ground-true is the overwhelmingly common answer — every static object is
            // all-`Yes` by C11 6.7.9p10, and promotion seeds that as a constant array — so it
            // costs nothing and asks no solver anything.
            //
            // Everything else is handed over as the term it is, including a guard that folds
            // ground-*false*. Reporting that one here would be easy and would still be wrong:
            // the engine can name a concrete offset from a model and this cannot, and one code
            // path for the report beats two that have to agree about the wording.
            let faults = match guard {
                Some(g) if a.eval_ground_bool(g) != Ok(true) => {
                    vec![MemFault::UninitializedSymbolic {
                        obj: id,
                        off: i,
                        guard: g,
                        at,
                    }]
                }
                _ => vec![],
            };
            return AccessResult { value, faults };
        }
        // The `Bytes` path: an ite chain, innermost first, over the candidates.
        let w = a.width(off);
        // Innermost-first, so the last candidate is the unguarded fallback. Building it
        // the other way round is *equivalent*: the fallback is only reached when the
        // offset equals no candidate, and the candidate set is the feasible set — so
        // that case cannot arise, and a test for it would pin an arbitrary choice.
        let mut acc: Option<Term> = None;
        for &k in candidates.iter().rev() {
            let byte = match self
                .read_term(
                    a,
                    Pointer {
                        base: id,
                        off: k as i64,
                    },
                    1,
                    Endian::Little,
                    at,
                )
                .value
            {
                Some(t) => t,
                None => continue,
            };
            let kc = a.bv(w, k as u128);
            let cond = a.eq(off, kc);
            acc = Some(match acc {
                None => byte,
                Some(rest) => a.ite(cond, byte, rest),
            });
        }
        AccessResult {
            value: acc,
            faults: vec![],
        }
    }

    /// An **unpinned** symbolic store: one `store` at a symbolic index, no enumeration.
    ///
    /// This is the write promotion exists for. The object is promoted first if it is not
    /// already, because the `Bytes` representation has nowhere to put a value whose
    /// address is unknown.
    pub fn store_at(
        &mut self,
        a: &mut TermArena,
        id: ObjectId,
        off: Term,
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
        let r = self.promote_to_array(a, id);
        if !r.faults.is_empty() {
            return AccessResult {
                value: None,
                faults: r.faults,
            };
        }
        let Some(mut arr) = self.entry(id).and_then(|e| e.arr) else {
            return AccessResult::fault(MemFault::WildPointer { off: 0, at });
        };
        let i = fit(a, off, arr.idx_bits);
        arr.data = a.store(arr.data, i, val);
        // Every byte the store *may* have touched is conditionally initialized, and with
        // no pinning that is the whole object — so the honest mark is `Cond` everywhere,
        // guarded by "the offset was this one".
        let one = a.bv(1, 1);
        let size = self.entry(id).map_or(0, |e| e.size);
        for b in 0..size {
            let kc = a.bv(a.width(off), b as u128);
            let hit = a.eq(off, kc);
            for bit in b * 8..b * 8 + 8 {
                let bi = a.bv(arr.idx_bits, bit as u128);
                let prev = a.select(arr.init, bi);
                let now = a.ite(hit, one, prev);
                arr.init = a.store(arr.init, bi, now);
            }
        }
        if let Some(e) = self.entry_mut(id) {
            e.arr = Some(arr);
        }
        AccessResult {
            value: Some(()),
            faults: vec![],
        }
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
        // **An empty candidate list is an *unpinned* write, and it needs the array.**
        //
        // `read_term_at` already promotes on an empty list — its doc calls that "no pinning
        // available", a promoted object's normal case. The write did not, and then wrote
        // nothing at all: the loop below iterates `candidates`, so with none there was no
        // offset to write at and the value was silently dropped. The engine that called this
        // for a genuinely unconstrained index got a successful-looking result and stale
        // bytes.
        //
        // Promoted, the answer is one `store` at the symbolic index — the exact counterpart
        // of the read's unpinned `select`, and the only form that says "somewhere in here,
        // and I do not know where".
        if candidates.is_empty() {
            let r = self.promote_to_array(a, id);
            if !r.faults.is_empty() {
                return AccessResult {
                    value: Some(()),
                    faults: r.faults,
                };
            }
            let Some(e) = self.entry_mut(id) else {
                return AccessResult::fault(MemFault::WildPointer { off: 0, at });
            };
            let Some(arr) = e.arr.as_mut() else {
                return AccessResult::fault(MemFault::SymbolicByte {
                    obj: id,
                    off: 0,
                    at,
                });
            };
            let idx = if a.width(off) == arr.idx_bits {
                off
            } else if a.width(off) > arr.idx_bits {
                a.extract(off, arr.idx_bits - 1, 0)
            } else {
                a.zext(off, arr.idx_bits)
            };
            arr.data = a.store(arr.data, idx, val);
            // **And the byte becomes initialized**, which 021 §3.1 requires of any write —
            // a write that does not record having happened is indistinguishable from no
            // write, and the candidate path's own comment records that failure.
            //
            // **Unobserved today, and recorded as such rather than claimed.** Deleting these
            // two lines passes the whole suite: the promoted *read* path does not consult
            // the `init` array, so nothing downstream can tell. That is a gap on the read
            // side, not a reason to leave the write wrong — but it means this pair is
            // correctness by construction and not by test. §9 carries it.
            // **Bit-indexed, like every other writer of `arr.init`.** Wave 200 fixed exactly
            // this in `write_term` and left it here — the sibling function, same array, same
            // mistake. `init_bit_via` selects at a *bit* index, so a byte-indexed store sets
            // one bit of the wrong byte and leaves the other eight unset.
            //
            // Eight `store`s at `idx * 8 + k`, which for a symbolic `idx` is a symbolic bit
            // index — the multiply is in the term, not in Rust.
            //
            // **Unobserved by the suite, and that is a symptom rather than an excuse.**
            // Mutation says so plainly: deleting these lines, indexing by byte again, or
            // writing only the first bit all pass every test. The reason is §9's separate
            // finding — the promoted *read* does not consult `arr.init` — so no init write on
            // a promoted object can be seen from outside yet. Fixing the read is what makes
            // this testable, and until then this is correctness by argument.
            let one = a.bv(1, 1);
            let eight = a.bv(arr.idx_bits, 8);
            let base_bit = a.mul(idx, eight);
            for k in 0..8u128 {
                let off_k = a.bv(arr.idx_bits, k);
                let bi = a.add(base_bit, off_k);
                arr.init = a.store(arr.init, bi, one);
            }
            return AccessResult {
                value: Some(()),
                faults: Vec::new(),
            };
        }
        let w = a.width(off);
        let obj_size = self.entry(id).map_or(0, |e| e.size);
        let Some(e) = self.entry_mut(id) else {
            return AccessResult::fault(MemFault::WildPointer { off: 0, at });
        };
        let mut arr = e.arr;
        let Some(o) = e.obj.as_mut().map(std::sync::Arc::make_mut) else {
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
        // **A uniform mask becomes a constant array rather than `size * 8` stores.** Both
        // encodings mean the same thing, and the difference is whether anything can ask a
        // question about it: `select` folds through an `ArrayConst` at any index, concrete or
        // symbolic, and stops dead at the first store it cannot compare. A 64-byte object
        // seeded store-by-store puts 512 nodes between a symbolic read and its answer; seeded
        // as a constant it puts none. The two cases this catches are the two that matter —
        // C11 6.7.9p10 makes every static object all-`Yes`, and a fresh local all-`No`.
        // The *dominant* bit becomes the constant and only the exceptions get a store, which
        // is a strict generalization: a uniform mask has no exceptions and needs no stores at
        // all. Both directions occur — a fresh local is all-`No` and a static object all-`Yes`
        // (C11 6.7.9p10) — and the shape that made this necessary is neither: a mostly-unwritten
        // buffer with one field set produced 512 stores where 8 will do, which is the difference
        // between a question `select_expand` can eliminate and one it gives up on.
        let yes = bits.iter().filter(|b| **b == InitBit::Yes).count();
        let no = bits.iter().filter(|b| **b == InitBit::No).count();
        let base = u128::from(yes >= no);
        let mut init = a.array_const(idx_bits, 1, base);
        for bit in 0..size * 8 {
            // `No → 0`, `Yes → 1`, `Cond(t) → ite(t, 1, 0)` — 021 §3.1's mapping, which
            // is what makes the two paths agree rather than merely coexist.
            let one = a.bv(1, 1);
            let zero = a.bv(1, 0);
            let v = match bits[bit as usize] {
                InitBit::No if base == 0 => continue,
                InitBit::Yes if base == 1 => continue,
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
            && let Some(o) = e.obj.as_mut().map(std::sync::Arc::make_mut)
        {
            o.memoize_fresh(lo_bit, n_bits);
        }
    }

    /// Give every *definitely uninitialized* byte in the range a fresh symbol, so the
    /// value the caller receives is one nobody has claimed rather than a stale zero.
    ///
    /// Only `No` bytes: a `Cond` byte already has a term whose guard is live, and
    /// overwriting it would discharge that guard in chiero's favour.
    fn materialize_fresh(&mut self, a: &mut TermArena, id: ObjectId, off: i64, size: u64) {
        for k in off as u64..off as u64 + size {
            let fresh_needed = self
                .entry(id)
                .and_then(|e| e.obj.as_ref())
                .is_some_and(|o| o.init.first_no(k * 8, 8).is_some() && o.sym_at(k).is_none());
            if !fresh_needed {
                continue;
            }
            let t = a.var(
                chiero_solver::Sort::BitVec(8),
                &format!("uninit_{}_{k}", id.0),
            );
            self.minted.push(MintedSymbol {
                term: t,
                obj: id,
                at: self.entry(id).map_or(Span::DUMMY, |e| e.origin),
                why: "a lazily-materialized byte",
                array: false,
            });
            match self.entry(id).and_then(|e| e.arr) {
                Some(mut arr) => {
                    let i = a.bv(arr.idx_bits, k as u128);
                    arr.data = a.store(arr.data, i, t);
                    if let Some(e) = self.entry_mut(id) {
                        e.arr = Some(arr);
                    }
                }
                None => {
                    if let Some(e) = self.entry_mut(id)
                        && let Some(o) = e.obj.as_mut().map(std::sync::Arc::make_mut)
                    {
                        o.sym.insert(k, t);
                    }
                }
            }
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
                        && let Some(o) = e.obj.as_mut().map(std::sync::Arc::make_mut)
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
/// Widen or narrow `t` to `w` bits. An array index whose width differs from the array's
/// is a sort error the backend rejects, and the arena cannot catch it because arrays
/// carry no scalar width — so it would surface as an unexplained backend failure.
fn fit(a: &mut TermArena, t: Term, w: u32) -> Term {
    let tw = a.width(t);
    match tw.cmp(&w) {
        std::cmp::Ordering::Equal => t,
        std::cmp::Ordering::Less => a.zext(t, w),
        std::cmp::Ordering::Greater => a.extract(t, w - 1, 0),
    }
}

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
        AccessError::MaybeUninitialized { off, bit } => MemFault::MaybeUninitialized {
            obj,
            off,
            bit,
            guard: None,
            at,
        },
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
