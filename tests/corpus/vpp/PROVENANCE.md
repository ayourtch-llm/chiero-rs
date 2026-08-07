# VPP corpus — verbatim upstream source

These files are copied **unmodified** from VPP and are not chiero's work.

- Upstream: <https://gerrit.fd.io/r/vpp>
- Commit: `7fe9c2669396d2dba708ea406508e73cf83b31bc`, 2026-07-17
- Licence: Apache-2.0, as stated in each file's own header. Copyright remains with
  the original authors.
- Local checkout the copy was taken from: `/home/ubuntu/vpp/src`

## Why these 113 files and not others

They are the **transitive VPP-local include closure** of six vppinfra headers —
`vec.h`, `pool.h`, `bitmap.h`, `format.h`, `hash.h`, `error.h` — computed by following
every `#include` and keeping the ones that resolve inside `vpp/src`. Everything else the
closure reaches is a system header (`stddef.h`, `stdarg.h`, `string.h`, …) or is behind a
false conditional (`vppinfra/config.h`, which is generated at build time and never
reached in this configuration).

**Widened 2026-08-07 by one seed** — `vnet/session/session_types.h` — which adds its own
closure of 86 files and brings `vlib`, `svm` and `vnet` in for the first time. §8.3 of the
handoff records why: twenty headers from one directory is a gate that can only see what that
directory does, and this seed found a defect on its first run (an attribute after a typedef
declarator, read as the record definition's — glibc's `__pthread_unwind_buf_t`, and so every
TU that reaches `<pthread.h>`).

**Two files here are generated, not upstream source**: `vlib/config.h` and `vpp/vnet/config.h`,
copied from `build-root/install-vpp-native/vpp/include/` of the same checkout. They are cmake
output rather than checked-in source, so the "resolves inside `vpp/src`" rule above does not
describe them — but `vlib/buffer.h` includes `vlib/config.h` unconditionally and the closure
does not preprocess without it. Their contents are four `#define`s of build configuration
(`VLIB_BUFFER_PRE_DATA_SIZE` and friends), which is what VPP really compiles with.
`vppinfra/config.h` is still absent and still unreached, as described above.

The closure is the point. Contract 19 is about **preprocessed** TUs, and a hand-written
fixture cannot stand in for one: these six headers expand to 250,000–290,000 tokens each,
and they are where VPP's `foreach_*` X-macros, `CLIB_CACHE_LINE_ALIGN_MARK`, vector
intrinsics, statement expressions and packed structs actually live. 013 §4's whole
extension table was measured against this code.

## Rules

- **Do not edit these files.** They are an oracle: the value of parsing them comes
  entirely from their being what upstream really wrote. A fixture edited until it parses
  proves that the parser handles the edit.
- They are exempt from the 001 §4 rule-4 VPP-leak gate by construction — `xtask
  check-vpp-leak` scans `crates/*/src/**/*.rs` only, and its own doc comment records that
  the rule is about knowledge baked into logic, not about test fixtures.
- To refresh, re-run the closure computation against a newer VPP checkout and update the
  commit hash above. Expect the pinned diagnostic counts in `vpp_corpus.rs` to move; that
  is what makes them a regression metric rather than a constant.
