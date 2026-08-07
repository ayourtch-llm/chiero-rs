# Is `layout`'s padding floor one gcc can actually reach?

041 §3.1's padding proposal says a struct "would be M bytes with its fields ordered by
size". These two scripts put that number to gcc rather than to arithmetic: they enumerate
declaration orders, compile each, and take the minimum `sizeof`. A **bit-field run plus any
trailing `:0` moves as one unit**, because that is the reorder the proposal describes.

They are checked in because they were the only thing that could see the defect they found,
and because scripts that live only in a scratch directory have been lost here before.

- `fixed_diff.py` — the named cases, including the one an adversarial review used to break
  the first version of §3.1: `struct Q { unsigned a:1; unsigned :0; char c; unsigned b:1;
  unsigned :0; char d; }`, 12 bytes, whose floor is 8 and which chiero said was 4.
- `floor_diff.py <seed> <cases>` — randomized structs. **It found nothing, before or after
  the fix**, which is the useful thing to know about it: the shape needs *two*
  `:0`-terminated runs, since with one you can always hide it last, and the generator does
  not reliably produce that. It is here as a regression net, not as the check.

Both want a release build (`cargo build --release -p chiero-cli`) and gcc on `PATH`.

**The instrument was proven able to see the defect before it was trusted.** Run
`fixed_diff.py` against a pre-fix binary and it prints `<== OVER-CLAIM` on `Q`; against the
fixed one it prints no proposal for that record. An instrument that has never been shown
failing is a claim about the code that nothing checked — this repository has shipped three
of those.

## `vpp_sizes.py` — contract 12's method, pointed outside the gate's corpus

`vpp_layout_gate` (014 contract 12) generates `_Static_assert`s for every record it can
parse and lets gcc reject them. Its corpus is `CORPUS_SEEDS`: twenty `vppinfra/` headers.
**None of them contains an unnamed bit-field**, so the gate passing said nothing about the
alignment defect fixed in `68f7924` — that is what this script was written to find out.

    python3 vpp_sizes.py src/vnet/session/session_types.h src/vnet/session/transport_types.h

Needs the VPP checkout at `/home/ubuntu/vpp` and a release build. Measured 2026-08-07:
**269 named records across the two session headers, 0 disagreeing with gcc** on size or
alignment.

And the fix is a **no-op on those headers** — reverting the `name.is_some()` guard and
re-dumping every tag/size/align gives a byte-identical list. VPP's unnamed bit-fields are
padding inside structs that also declare a *named* bit-field of the same type, so the
alignment was already contributed by the named one. The defect is real — the sema test
checks it against gcc — and this corner of VPP does not exhibit it. `pp2_hw.h`, the third
file with unnamed bit-fields, does not preprocess yet and is unmeasured.
