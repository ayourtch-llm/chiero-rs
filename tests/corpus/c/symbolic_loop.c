/* A loop whose trip count is symbolic, and a `goto` out of a nested one.
 *
 * VPP's vector loops are all this shape: `for (i = 0; i < vec_len(v); i++)`, where the
 * bound comes from data. Under chiero `n` is bounded but unknown, so the loop is explored
 * up to the budget; natively `n` is 3 and it runs three times. */
#include "chiero.h"

int main(void) {
  int n = 3;
  chiero_make_symbolic(&n, sizeof n, "n");
  chiero_assume(n >= 0);
  chiero_assume(n <= 4);

  int total = 0;
  for (int i = 0; i < n; i++) {
    total += i;
  }
  /* The sum of 0..n-1 is never negative and never exceeds 0+1+2+3. Both bounds hold for
   * every feasible `n`, so an engine that ran the loop the wrong number of times fails
   * one of them. */
  chiero_assert(total >= 0);
  chiero_assert(total <= 6);

  /* `goto` out of a nested loop: the scope exits must all be emitted on the jump, or 021
   * never retires the inner objects. */
  int found = 0;
  for (int a = 0; a < 2; a++) {
    for (int b = 0; b < 2; b++) {
      if (a + b == n) {
        found = 1;
        goto done;
      }
    }
  }
done:
  chiero_assert(found == 0 || found == 1);
  return 0;
}
