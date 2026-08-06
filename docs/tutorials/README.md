# chiero tutorials

Eight worked examples. Every command and every code block on these pages is a test in
`crates/chiero-tool/tests/tutorials.rs`, and every console transcript was pasted from a real
run — documentation that does not run is worse than none, because a reader who pastes it and
gets an error learns not to trust the rest of it.

## If you have five minutes

Read **[5. Reading the envelope](05-envelope.md)**. Everything else here produces an answer;
that page is how to tell what an answer is worth, and it is the only one that is really
required. The single sentence version: an empty result is not a clean result, and the envelope
is the machinery that makes the difference impossible to miss.

## If you are working through it

| | Page | Answers |
|---|---|---|
| 1 | [Reading coverage](01-coverage.md) | What did each test actually execute? And why `None` and `Some(0)` must stay different answers. |
| 2 | [What a change reaches](02-change-impact.md) | Which functions does this edit affect — *including* through a macro no coverage tool attributes a line to? |
| 3 | [Choosing tests](03-test-selection.md) | Which tests are worth running, with a reason for each and a proof behind every one dropped? |
| 4 | [Adjudicating a rewrite](04-prove-equivalent.md) | Are these two versions the same function? If not, at which input do they differ? |
| 5 | [Reading the envelope](05-envelope.md) | How much is this answer worth, and what did it not look at? |
| 6 | [Finding defects](06-find-bugs.md) | Where are the bugs, what input reaches each — and what was not searched? |
| 7 | [Reachability](07-reachability.md) | Can execution get to this line? "Nothing gets there" and "I did not get there" are different verdicts. |
| 8 | [Struct layout](08-layout.md) | What does this struct's padding cost, and is reordering it even allowed? |

1–3 are the coverage-and-selection story and build on each other. 4, 6, 7 and 8 each stand
alone and use the symbolic engine. 5 applies to all of them.

## Two things worth knowing before you start

**Start anywhere, not at `main`.** Every symbolic operation takes `--entry <fn>`, and a
function's parameters begin unconstrained — which is what makes analysing a 1M-line packet
processor tractable, and also why `find-bugs` has flags for telling it what a caller
guarantees ([tutorial 6](06-find-bugs.md#three-flags-and-why-a-real-codebase-needs-them)).

**No answer is a proof unless it says so.** `proven: true` appears only at
`fidelity: "Exact"`, it is derived rather than set, and there is no way to construct an
envelope where the two disagree.

## Beyond the tutorials

- [`../specs/`](../specs/) — the numbered specifications each crate is written against. Every
  one ends with testable contracts the suite cites by number, and most record what the
  alternative was and why it lost.
- `chiero --help` — the nine commands, and the flags each takes.
- `tests/corpus/vpp-findings/` — the same operations pointed at real VPP, with the numbers and
  the script that produced them.
