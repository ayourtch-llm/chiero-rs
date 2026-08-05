int two (int x) { return x + 2; } int one (int x) { int y = x + 1;
  return y; }
/* `two` and `one` begin on the same line of the same file, which is how gcov
   decides two functions are a *group* (process_all_functions): each then gets
   a private line table for the lines in its own range, and --json-format
   emits one entry per function rather than folding them into the source.

   `one` spans two lines, so the block holding its body belongs to line 2 and
   line 1 is left with no block of `one` attributed to it — an accumulation.
   `two` fits on line 1 and graphs it. gcov keeps those in separate tables and
   the JSON sums them; sharing one table lets the graph count overwrite the
   accumulation, and `one`'s contribution to line 1 disappears. */

int
main (void)
{
  return one (1) + two (2) == 6 ? 0 : 1;
}
