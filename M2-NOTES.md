# M2 frontend notes

## Status

- 011 contracts 1–10 and 14: implemented and tested.
- 011 contracts 11–13: pending corpus, throughput, and warm-cache work.
- 012 contracts 1–19: pending.

## Findings

- The existing `cargo xtask contract-coverage` gate only measures specs 020–024, so it
  cannot report 011/012 coverage. The tests still use the required `Covers:` header.
- `chiero-span::SourceFile` does not expose the splice-position list described by 011
  §2.2. `chiero-lex` therefore keeps a private physical-to-logical mapping while lexing
  and a spelling side table in `LexedFile`; no shared API addition is required yet.

## Mutation checks

- Disabling pp-number exponent-sign absorption made the contract-1 fixture tokenize
  `0x1e+2` into three non-EOF tokens; `pp_numbers_are_single_tokens` failed.
