/* A call through a function pointer, which is how VPP dispatches every node.
 *
 * The callee is chosen by a symbolic value, so chiero must explore both and prove the
 * assertion on each. Natively `pick` is 1 and one path runs. */
#include "chiero.h"

static int twice(int v) { return v * 2; }
static int thrice(int v) { return v * 3; }

int main(void) {
  int pick = 1;
  chiero_make_symbolic(&pick, sizeof pick, "pick");

  int (*fn)(int) = pick ? twice : thrice;
  int r = fn(7);

  /* Both callees are multiples of 7 and at least 14, so the assertion holds whichever the
   * pointer resolved to — but only if it resolved to *one of them*. An engine that
   * invented a return value has no reason to satisfy either bound. */
  chiero_assert(r % 7 == 0);
  chiero_assert(r >= 14);
  return 0;
}
