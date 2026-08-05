/* Two functions of one object both inline `bump`, so both contribute counts
   to the same lines of multi.h. gcov accumulates a source's line counts
   across every function in the object; taking the maximum instead reports
   one caller's count and silently loses the other's. */
#include "multi.h"

static int
one (int x)
{
  return bump (x, 1);
}

static int
two (int x)
{
  return bump (x, 2);
}

int
main (void)
{
  return one (1) + two (2) == 6 ? 0 : 1;
}
