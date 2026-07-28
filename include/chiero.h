/* chiero harness intrinsics — 024 §7.
 *
 * A corpus file that includes this header is **two programs**: an input to chiero, and a
 * program gcc compiles and runs. 070's differential oracle compares the two, and that
 * only works without maintaining separate copies of every test if one source serves both.
 *
 * ## The conditional is load-bearing
 *
 * chiero defines `__CHIERO__`. Under it these are *declarations* — externs with no body,
 * which 024 §1 resolves through the model registry. Everywhere else they are
 * `static inline` no-ops.
 *
 * Defining them unconditionally would be the natural way to write this header and would
 * quietly destroy the corpus. 023 §5 is explicit that "the module's own definition always
 * wins" over a registered model — a rule that exists so a project reimplementing a libc
 * function is analysed as written. Give chiero a no-op body for `chiero_make_symbolic`
 * and it will faithfully analyse the no-op: nothing becomes symbolic, every corpus
 * program is explored along exactly one concrete path, every assertion holds, and the
 * suite reports success over a symbolic execution that never happened.
 *
 * ## `chiero_make_symbolic` does not initialize
 *
 * Under gcc it does nothing at all, so the bytes it names keep whatever they had. Write
 *
 *     int x = 3;
 *     chiero_make_symbolic(&x, sizeof x, "x");
 *
 * and not a bare `int x;` — otherwise the native run reads an uninitialized variable and
 * the oracle compares chiero against undefined behaviour. The concrete value should be
 * one that makes the program take an interesting path, since it is the only path the
 * native run takes.
 */

#ifndef CHIERO_H
#define CHIERO_H

#include <stddef.h>

#ifdef __CHIERO__

/* Declarations only: 024 §1 step 1 finds these in the model registry. A body here would
 * shadow the model — see the note above. */
void chiero_make_symbolic(void *addr, size_t n, const char *name);
void chiero_assume(int cond);
void chiero_assert(int cond);
int chiero_is_symbolic(long v);
void chiero_mark_fidelity(const char *why);

#else

/* No-ops, so the same file is a runnable C program.
 *
 * `chiero_assert` is a no-op and **not** `assert()`. The native run's job is to produce
 * the program's real behaviour for the oracle to compare against; aborting on a violated
 * assertion would replace that behaviour with a signal. Under chiero the same call
 * produces a finding with a witness (024 contract 15), which is where a violated
 * assertion is supposed to show up.
 *
 * `chiero_assume` is likewise a no-op rather than an early exit. Under chiero it kills the
 * state silently; natively the caller is responsible for having picked a concrete value
 * that satisfies it, which is the same discipline the initialization note above asks for.
 */

static inline void chiero_make_symbolic(void *addr, size_t n, const char *name) {
  (void)addr;
  (void)n;
  (void)name;
}

static inline void chiero_assume(int cond) { (void)cond; }

static inline void chiero_assert(int cond) { (void)cond; }

/* Nothing is symbolic in a concrete run, so this is `0` — not `1`, and not left
 * undefined. A corpus file may branch on it to check chiero's own introspection, and the
 * native run must take the concrete side. */
static inline int chiero_is_symbolic(long v) {
  (void)v;
  return 0;
}

static inline void chiero_mark_fidelity(const char *why) { (void)why; }

#endif /* __CHIERO__ */

#endif /* CHIERO_H */
