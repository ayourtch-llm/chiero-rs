/* gcc outlines the parallel region into a compiler-generated function, which
   the .gcno marks `artificial`. gcov erases artificial functions before it
   accounts a single line (process_all_functions), so their lines never exist;
   counting them attributes the outlined body's counts to the source lines it
   was written on, on top of the real function's. */
#include <omp.h>

int
main (void)
{
  int s = 0;
#pragma omp parallel for reduction(+ : s) num_threads (2)
  for (int i = 0; i < 8; i++)
    s += i;
  return s == 28 ? 0 : 1;
}
