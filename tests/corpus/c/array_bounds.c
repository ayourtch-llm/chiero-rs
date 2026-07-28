/* A symbolic index constrained into range. The interesting property is the *absence* of a
 * finding: chiero must prove the access in bounds rather than report it, which it can
 * only do if the assume reached the solver. */
#include "chiero.h"

int main(void) {
  int buf[8] = {0, 1, 2, 3, 4, 5, 6, 7};
  int i = 5;
  chiero_make_symbolic(&i, sizeof i, "i");
  chiero_assume(i >= 0);
  chiero_assume(i < 8);
  chiero_assert(buf[i] == i);
  return 0;
}
