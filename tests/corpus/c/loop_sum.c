/* A bounded loop, which is what makes `max_depth` and the unroll bound observable. The
 * sum is concrete despite `n` being symbolic, because the assume pins it. */
#include "chiero.h"

int main(void) {
  int n = 4;
  chiero_make_symbolic(&n, sizeof n, "n");
  chiero_assume(n == 4);
  int sum = 0;
  for (int k = 0; k < n; k++) {
    sum += k;
  }
  chiero_assert(sum == 6);
  return 0;
}
