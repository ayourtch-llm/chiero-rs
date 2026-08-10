# 8. Struct layout

**What you get:** padding a field reorder would recover, and fields that straddle a cache line
— with a statement, on every proposal, of whether reordering that struct is allowed at all.

**Why the second half is most of it:** VPP tunes for cache lines deliberately —
`CLIB_CACHE_LINE_BYTES` appears in 257 files — and it also defines wire formats. Reordering an
`ip4_header_t` is a protocol violation, not an optimization, and a tool that cannot tell the two
apart is worse than none.

## Padding a reorder would recover

```c
struct session {
  char active;
  long bytes;
  char flags;
};
```

```console
$ chiero layout sess.c
records:
  - tag: session
    file: sess.c
    line: 1
    size: 24
    align: 8
    packed: false
    proposals:
      - kind: padding_waste
        recoverable: 8
        rationale: `session` is 24 bytes and would be 16 with its fields ordered by size
        benefit: Unquantified
        advisory: false
        evidence:
          - 8 bytes of alignment padding, 0 of 1 lines saved per instance
          - 14 bytes of padding in the record as declared; reordering recovers 8 of them, because the record's 8-byte alignment rounds the end up whatever order the members are in
          - 7 bytes of padding after `active` (offset 0, 1 byte) and before `bytes` at offset 8
          - 7 bytes of padding at the end, after `flags` (offset 16, 1 byte) — tail padding the record's alignment requires
        obligations:
          - state: discharged
            what: the layout is internal to the program
proposals: 1
cache_line_bytes: 64
proven — this holds for all inputs (Exact)
  blind spot: this is an analysis of layout; §3's hot/cold, false-sharing and prefetch findings need a profile and 025's sharing classification, and are not produced at all rather than produced from nothing
  blind spot: no run supplied access counts, so every benefit is Unquantified — chiero has no cycle model and will not estimate one
```

24 bytes that would be 16. The delta is what *this* reorder is worth rather than a theoretical
minimum, because a proposal saying "you could save space" without a number is not actionable.

**And it says where the bytes are.** A total is not advice on a struct with thirty members, so
each hole names the fields on either side of it and its offset — `active` is one byte followed
by seven of nothing, and `flags` leaves seven more at the end.

**The two numbers are different on purpose.** There are 14 bytes of padding in the struct as
declared and reordering gets 8 of them back: the best order still ends `long, char, char`, and
the record's own 8-byte alignment rounds 10 up to 16 whatever the order. A proposal that listed
14 beside `recoverable: 8` and said nothing would read as an arithmetic error.

**The obligation is discharged**, so `advisory` is false: nothing suggests this layout is
visible outside the program, so reordering it is an ordinary change.

## And the struct where it is not

```c
struct pkt_hdr {
  char pad[60];
  long seq;
} __attribute__((packed));
```

```console
$ chiero layout wire.c
records:
  - tag: pkt_hdr
    file: wire.c
    line: 1
    size: 68
    align: 1
    packed: true
    proposals:
      - kind: line_straddle
        field: seq
        offset: 60
        size: 8
        rationale: `pkt_hdr.seq` spans a 64-byte cache-line boundary, so one access touches two lines — but see the obligation: the layout may be externally observable
        benefit: Unquantified
        advisory: true
        evidence:
          - offset 60 size 8 crosses the boundary at 64
        obligations:
          - state: open
            why: `pkt_hdr` is `packed`, so its layout is observable outside the program — a wire format or an ABI boundary. Reordering it is a protocol change, not an optimization (041 §3)
proposals: 1
cache_line_bytes: 64
proven — this holds for all inputs (Exact)
  blind spot: this is an analysis of layout; §3's hot/cold, false-sharing and prefetch findings need a profile and 025's sharing classification, and are not produced at all rather than produced from nothing
  blind spot: no run supplied access counts, so every benefit is Unquantified — chiero has no cycle model and will not estimate one
```

The finding is true — `seq` really does span the boundary at 64 — and acting on it would change
a wire format. So the obligation is **open**, `advisory` is **true**, and the rationale says
"observable" in words rather than leaving a reader to notice the flag.

`externally_visible` is the caller's answer and **defaults to observable when unsure**: 041 §3
gives the unprovable case and the observable case the same treatment, so nobody has to guess in
the dangerous direction.

## When a straddle can happen at all

A naturally-aligned scalar **cannot** straddle a cache line whose size is a multiple of its
alignment — an 8-byte `long` sits at a multiple of 8, and 8 divides 64. So a straddle needs
`packed`, a misaligned outer struct, or an array member.

That is not a limitation. It is precisely VPP's wire formats and its
`CLIB_CACHE_LINE_ALIGN_MARK` structs, which is what §3 is about.

`--cache-line` sets the size; 64 is a fact about a machine rather than about C.

## Benefit is labelled honestly

Every proposal above says `Unquantified`, and the envelope says why:

> no run supplied access counts, so every benefit is Unquantified — chiero has no cycle model
> and will not estimate one

`Measured` requires access counts from a real run. There is no `Estimated` in between here:
estimating needs a model of what a cache miss costs, and inventing one would put a number in
front of a reader that nothing measured.

## What it does not do

The envelope's other blind spot is the honest list:

> this is an analysis of layout; §3's hot/cold, false-sharing and prefetch findings need a
> profile and 025's sharing classification, and are not produced at all rather than produced
> from nothing

Unions are skipped — all members start at zero, so neither straddling nor padding means what
this page means by them — and so are bit-fields, whose extent is bits within a byte.

## Next

Back to [the envelope](05-envelope.md) if you have not read it.

*Reference: [041 §3](../specs/041-optimization-analysis.md). Worked example under test:
`crates/chiero-tool/tests/tutorials.rs::tutorial_08_layout`.*
