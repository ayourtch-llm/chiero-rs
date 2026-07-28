/* A `switch` on a symbolic value, with fallthrough and a default.
 *
 * VPP's node dispatch is a switch on a packet field. Under chiero every reachable case is
 * a path; natively `op` is 2 and one runs. */
#include "chiero.h"

int main(void) {
  int op = 2;
  chiero_make_symbolic(&op, sizeof op, "op");
  chiero_assume(op >= 0);
  chiero_assume(op <= 3);

  int r = 0;
  switch (op) {
  case 0:
    r = 10;
    break;
  case 1:
  case 2:
    /* Two labels on one arm, which a lowering that enumerated cases wrongly gets wrong. */
    r = 20;
    break;
  default:
    r = 30;
    break;
  }

  /* Every arm is a positive multiple of ten, and no arm is 0 — so an engine that fell
   * through to the default on every path, or took no arm at all, fails. */
  chiero_assert(r % 10 == 0);
  chiero_assert(r >= 10);
  chiero_assert(r <= 30);
  return 0;
}
