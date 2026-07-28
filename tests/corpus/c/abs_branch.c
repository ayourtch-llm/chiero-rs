/* The smallest thing that is genuinely two programs: a symbolic branch.
 *
 * Under chiero `x` is unconstrained, so both sides of the comparison are explored and
 * `chiero_assert` must hold on each. Natively `x` is 3, one path runs, and the program
 * exits 0. */
#include "chiero.h"

static int my_abs(int v) { return v < 0 ? -v : v; }

int main(void) {
  int x = 3;
  chiero_make_symbolic(&x, sizeof x, "x");
  /* INT_MIN has no positive counterpart, and `-INT_MIN` is undefined. Excluding it is the
   * assumption the function itself makes, stated rather than left implicit. */
  chiero_assume(x > -2147483647 - 1);
  chiero_assert(my_abs(x) >= 0);
  return 0;
}
