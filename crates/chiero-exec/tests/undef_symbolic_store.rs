//! **A store the engine cannot translate must not leave the old bytes behind** (item 8g).
//!
//! `chiero-exec/src/lib.rs:3655` states the rule for the *concrete*-offset store path:
//!
//! > A refusal that silently keeps stale bytes is worse than a refusal, because the run then
//! > produces a confident wrong answer (021 §3.1).
//!
//! and the concrete path obeys it — `Some(Value::Undef)` havocs the range. The **symbolic**
//! offset path declares the same gap and then `return`s, writing nothing. Both emit the
//! assumption `a store of an untranslatable value`, so an envelope cannot tell them apart;
//! only memory can, which is why this test lives here and not in the C corpus.
//!
//! Reachable from C: `long double src(void); … a[i & 3] = (int) src();` — `FpToSi 80 -> 32` is
//! unmodelled, so the value is `Undef`.

use chiero_cir::*;
use chiero_exec::*;
use chiero_solver::TermArena;
use chiero_span::Span;

/// `a` is four bytes of `0x11`; then `undef` is stored at a **symbolic** offset that the path
/// allows to be 0. Reading offset 0 afterwards must not still be `0x11`.
const SRC: &str = "\
target x86_64-unknown-linux-gnu

func @probe() -> i32 {
  alloca %0 : i8 x 4 align 1 scope 0 lifetime scope \"a\"
entry:
  .line 1
  %1 = addrlocal %0
  store i8 17i8 -> %1 align 1
  %2 = fresh i64
  %3 = ptradd %1, %2
  store i8 undef:i8 -> %3 align 1
  ret 0i32
}
";

#[test]
fn an_untranslatable_store_at_a_symbolic_offset_does_not_keep_stale_bytes() {
    let m = text::parse(SRC).unwrap_or_else(|e| panic!("fixture does not parse: {e:?}"));
    assert!(
        verify::verify(&m).iter().all(|e| !e.is_error()),
        "{:?}",
        verify::verify(&m)
    );
    let mut a = TermArena::new();
    let r = Engine::new(&m).run(&mut a);
    let st = r.states().first().expect("one state");
    let base = match st.local(ValueId(1)) {
        Some(Value::Ptr(p)) => p.base,
        other => panic!("expected the alloca's address, got {other:?}"),
    };
    let mut mem = st.mem.clone();
    let after = mem.read(chiero_mem::Pointer { base, off: 0 }, 1, Span::DUMMY);

    // **The offset is fresh, so the path allows it to be 0.** A store there may have landed on
    // this byte, and answering `0x11` with certainty is the "confident wrong answer" the rule
    // above forbids. Either the byte is havoc'd (no concrete value) or it is something else —
    // what it must not be is unchanged.
    // **The failure mode is confidence, not the byte.** `HavocFill::Uninitialized` clears the
    // initialization mask rather than the contents, so the stale `0x11` may still sit there —
    // what must not survive is chiero *answering* with it. A read of a byte the store may have
    // reached has to come back as a question: a fault, or no value at all.
    assert!(
        !after.faults.is_empty() || after.value != Some(vec![17u8]),
        "byte 0 answers 0x11 with no fault after an untranslatable store at an offset the \
         path allows to be 0 — the store was skipped and the old value survived as a \
         confident answer. assumptions: {:#?}",
        st.assumptions()
            .iter()
            .map(|x| &x.detail)
            .collect::<Vec<_>>()
    );
}
