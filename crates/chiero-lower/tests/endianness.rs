//! Covers: 020 contract 30 — endianness-conditional layouts and the `ConfigId`.
//!
//! 020 §4.4: "Endianness-conditional layouts are resolved **before** CIR: the two bitfield
//! orderings live behind `#if`, so they are two different `ConfigId`s producing two
//! different `RecordLayout`s. CIR sees whichever one the configuration selected, and the
//! `ConfigId` is recorded on every result."
//!
//! That is the whole design in one sentence, and it is why this test lives here rather
//! than in the memory model: there is no endianness *switch* inside CIR to test. The two
//! layouts are two preprocessor configurations, and the only thing CIR must do is carry
//! which one it was — because a `BitRange` of `0..3` means two different bits depending on
//! an answer that is no longer visible by the time anyone reads the module.
//!
//! Several vppinfra headers really are written this way; 020 §4.5 names them.

use chiero_cir::{BitRange, InstKind, Module, RValue};
use chiero_pp::ConfigId;

mod harness;
use harness::lower_with_config;

/// The shape vppinfra uses: two bitfield orderings behind `#if`.
const SRC: &str = "\
struct hdr {
#if CLIB_ARCH_IS_BIG_ENDIAN
  unsigned version:4;
  unsigned ihl:4;
#else
  unsigned ihl:4;
  unsigned version:4;
#endif
};
unsigned get_version(struct hdr *h) { return h->version; }
";

fn version_bits(m: &Module) -> BitRange {
    m.funcs
        .iter()
        .find(|f| &*f.name == "get_version")
        .expect("get_version")
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .find_map(|i| match &i.kind {
            InstKind::Assign {
                rv: RValue::LoadBits { bits, .. },
                ..
            } => Some(*bits),
            _ => None,
        })
        .expect("a bit-field read is a `LoadBits`")
}

/// **Contract 30.** The same source under two configurations produces two modules with
/// different `BitRange`s, and each records its `ConfigId`.
#[test]
fn two_configurations_give_two_layouts_and_each_records_its_config() {
    let le = lower_with_config(SRC, ConfigId(1), &[("CLIB_ARCH_IS_BIG_ENDIAN", "0")]);
    let be = lower_with_config(SRC, ConfigId(2), &[("CLIB_ARCH_IS_BIG_ENDIAN", "1")]);

    let (lb, bb) = (version_bits(&le), version_bits(&be));
    assert_ne!(
        lb, bb,
        "`version` is declared second little-endian and first big-endian, so it is not at \
         the same bits — a run that produced one layout for both would be silently wrong \
         about every IP header"
    );
    assert_eq!(
        lb,
        BitRange { off: 4, width: 4 },
        "little-endian: after `ihl`"
    );
    assert_eq!(bb, BitRange { off: 0, width: 4 }, "big-endian: first");

    // **And each module says which configuration it was.** Without this the two modules
    // are indistinguishable artifacts that disagree, and nothing downstream can say why.
    assert_eq!(
        le.config,
        Some(1),
        "the little-endian module records its id"
    );
    assert_eq!(be.config, Some(2));
    assert_ne!(
        le.config, be.config,
        "two configurations, two ids — a constant would make the field decoration"
    );
}

/// A source with **no** conditional layout still records its `ConfigId`.
///
/// The field is not a marker for "this had an `#if` in it"; it is which build this was.
/// A lowering that set it only when the preprocessor took a branch would leave it absent
/// on almost every real file.
#[test]
fn a_module_with_no_conditional_still_records_its_config() {
    let m = lower_with_config("int f(int n) { return n; }", ConfigId(7), &[]);
    assert_eq!(m.config, Some(7));
}
