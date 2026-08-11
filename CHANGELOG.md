# Changelog

`v0.1.0` is the first tag. This file says what it holds, so a consumer pinning it can tell what
changed underneath them afterwards.

The audience is a consumer of the **command line and the library**, not a reader of the commit
log. Internal work is summarised only where it changed an answer somebody could have been
relying on — this project publishes numbers, and a number that moved is a change.

## v0.1.0 — 2026-08-11

### The surface

Ten operations, each a command and a library call, each returning an
[envelope](docs/tutorials/05-envelope.md) that qualifies its own answer:
`prove-equivalent`, `find-bugs`, `check-reachable`, `layout`, `find-optimizations`, `impact`,
`select-tests`, `expansion-sites`, `explain-macro`, and `cir`.

- `chiero <operation> --help` gives one operation's arguments and **only the options it reads**.
  A usage error prints that page rather than the global one.
- The exit status is part of the interface: `0` the operation ran, `1` it could not, `2` the
  request was malformed ([050](docs/specs/050-tool-interface.md) contracts 19–20). Failures
  print nothing on stdout, so `--json | jq` is safe.
- [`docs/LIMITS.md`](docs/LIMITS.md) states the supported platform and what each operation does
  not do.

### Added

- **`chiero serve`** — the ten operations over MCP's tools surface (`initialize`,
  `tools/list`, `tools/call`) on newline-delimited JSON-RPC 2.0, dispatched through the same code
  the command line uses, so the two surfaces cannot disagree. Results carry both halves: a text
  rendering in `content` and the envelope in `structuredContent`. ⚠️ **Only `tools`** — no
  resources, prompts, logging or completions — and the shapes are verified against a vendored
  copy of the protocol schema, **never against a real client**.

- **`select-tests` works from the command line.** `--test NAME=PATH`, once per test run, or
  `--coverage-manifest <file>` with a `NAME<TAB>PATH` line each. The selection carries the
  caller's own test names back, so a consumer does not have to join on `TestId` integers.
- **Every answer names a file and a line.** `find-bugs` findings, `find-optimizations`
  proposals and `layout` records all carry the location of what they are about. Each was
  produced by a layer that knew where it was and handed it to a layer that dropped it.
- **`--march <name>`** targets the compiler persona (it had been accepted since 2026-08-09 and
  documented nowhere).
- **`--solver-rlimit <units>`**, a deterministic budget in solver work units — unlike a wall
  clock, a run cut by it is an ordinary answer rather than a measurement.

### Changed

- `select-tests` with `--coverage`/`--stem` **refuses** instead of answering `0 selected`. That
  pair reads one coverage object with no test attached, so an index built from it can select
  nothing whatever the change is. The refusal names the flags that work.
- `chiero cir <file> --entry <fn>` prints one function rather than a quarter of a million lines.
- **`find-bugs` groups findings by kind and location rather than by message text.** One access
  reached on several paths is one entry with a `paths` count, where before a path-specific
  clause in the message could split it into two. Counts can therefore go *down* for a program
  whose defects are reached more than one way; re-measured on the pinned 40 VPP entry points,
  where the answer is unchanged.

### Fixed

- A wild-pointer dereference was reported as an uninitialized read *of the pointer variable*.
- A shift past the operand width went unreported whenever the shifted value was symbolic.
- A symbolic-offset store could leave the rest of the object stale.
- An enum's declared underlying type was parsed and discarded, so `layout` reported a wrong size
  as `proven`.
- `_Alignof` on a typedef of an over-aligned type, for all three spellings.
- Reading a solver model back was O(V²) in the number of variables, which is what a 120-second
  `find-bugs` run was spending its time on.
- A closed pipe (`chiero cir big.c | head`) is no longer a panic.

### Performance

Measured on one 32 768-statement function, over 2026-08-09:

| | before | after |
|---|---|---|
| frontend | 22 671 ms | **1 693 ms** |
| full `find-bugs`, 96k blocks | 53 279 ms | **3 264 ms** |
| peak resident memory | **35 628 MB** | **494 MB** |

### Known gaps

- No MCP or JSON-RPC server; the operation set is reachable from the CLI and the library.
- x86-64 Linux only for the differential gates. The engine builds and passes on aarch64; the
  twelve failures there are all gates comparing chiero against the local compiler.
- The end-to-end VPP walkthrough that proved test selection on real code has not been re-run
  through the new `--test` flags — it was done through a hand-written driver.
