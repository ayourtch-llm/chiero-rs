/* A variadic function, which 020 §4.4.1's `VaArg` exists for and no corpus file exercised.
 *
 * VPP's `format`/`vlib_cli_output` paths are all variadic, and §4.4.1 puts the `va_list` in
 * *memory* precisely so `va_list *` can cross a function boundary. Under chiero one
 * argument is symbolic; natively the program exits 0. */
#include "chiero.h"
#include <stdarg.h>

static int sum_n(int count, ...) {
  va_list ap;
  va_start(ap, count);
  int total = 0;
  for (int i = 0; i < count; i++) {
    total += va_arg(ap, int);
  }
  va_end(ap);
  return total;
}

int main(void) {
  int x = 5;
  chiero_make_symbolic(&x, sizeof x, "x");
  chiero_assume(x >= 0);
  chiero_assume(x <= 10);

  int t = sum_n(3, 1, x, 2);
  /* `1 + x + 2` with `x` in 0..=10 is 3..=13. Both bounds are needed: an engine that read
   * the wrong argument, or stopped the walk early, lands outside one of them. */
  chiero_assert(t >= 3);
  chiero_assert(t <= 13);

  /* Zero variadic arguments is the edge `va_start`/`va_end` still have to survive. */
  chiero_assert(sum_n(0) == 0);
  return 0;
}
