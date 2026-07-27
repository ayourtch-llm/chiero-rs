# M2 frontend notes

## Status

- 011 contracts 1–14: implemented and tested. The 50 MB throughput assertion is an
  explicit ignored release-mode performance gate; it passed on the reference machine.
- 012 contracts 1–19: implemented and cited by tests. Contract 17's external
  `compile_commands.json` smoke test is explicit but ignored because this checkout has
  no VPP build directory or compilation database.

## Findings

- The existing `cargo xtask contract-coverage` gate only measures specs 020–024, so it
  cannot report 011/012 coverage. The tests still use the required `Covers:` header.
- `chiero-span::SourceFile` does not expose the splice-position list described by 011
  §2.2. `chiero-lex` therefore keeps a private physical-to-logical mapping while lexing
  and a spelling side table in `LexedFile`; no shared API addition is required yet.
- `/home/ubuntu/vpp/build-root/compile_commands.json` does not exist, and `find
  /home/ubuntu/vpp -name compile_commands.json` returns no alternatives. Contract 17's
  full configured-TU regression metric therefore cannot run in this environment.
- A representative macro/conditional/builtin fixture is compared token-for-token with
  both `gcc -E -P` and `clang -E -P`; all three agree.

## Mutation checks

- Disabling pp-number exponent-sign absorption made the contract-1 fixture tokenize
  `0x1e+2` into three non-EOF tokens; `pp_numbers_are_single_tokens` failed.
- Disabling cache lookup made the pointer-identity assertion in the contract-13 test
  fail before its timing assertion.
- Substituting raw arguments at ordinary parameter uses changed `xstr(__LINE__)` to
  `"__LINE__"`; the contract-4 test expected `"3"` and failed.
- Evaluating the right side of a false `&&` as live made `#if 0 && 1/0` diagnose;
  the contract-11 short-circuit test failed.
