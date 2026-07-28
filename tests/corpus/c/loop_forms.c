/* `do`/`while`, `continue` in a nested loop, and a recursive call.
 *
 * `continue` in a `for` goes to the *latch*, not the header — a distinction that once made
 * an infinite loop here. Recursion exercises 023's `max_recursion_depth` bookkeeping on a
 * program that terminates well inside it. */
#include "chiero.h"

static int fact(int n) { return n <= 1 ? 1 : n * fact(n - 1); }

int main(void) {
  int n = 3;
  chiero_make_symbolic(&n, sizeof n, "n");
  chiero_assume(n >= 1);
  chiero_assume(n <= 4);

  /* `do`/`while` runs its body before testing, so this is at least one iteration whatever
   * `n` is. */
  int seen = 0;
  int i = 0;
  do {
    seen++;
    i++;
  } while (i < n);
  chiero_assert(seen >= 1);
  chiero_assert(seen <= 4);

  /* `continue` skips the rest of the *inner* body and reaches the inner latch, so the
   * outer counter still advances every pass. */
  int evens = 0;
  for (int a = 0; a < 2; a++) {
    for (int b = 0; b < 4; b++) {
      if (b % 2) {
        continue;
      }
      evens++;
    }
  }
  chiero_assert(evens == 4);

  /* 1, 2, 6 or 24 — every one a positive multiple of nothing in particular, so the bounds
   * are what carry it. */
  int f = fact(n);
  chiero_assert(f >= 1);
  chiero_assert(f <= 24);
  return 0;
}
