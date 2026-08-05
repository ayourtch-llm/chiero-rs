/* One line holding a whole loop: the line's blocks form a cycle.
   gcov's rule (gcc/gcov.cc, accumulate_line_info) is "entry counts into the
   line's block subgraph, plus the counts of the elementary cycles within it",
   which on this line differs from both the max and the sum of block counts. */
int f (int n) { int s = 0; for (int i = 0; i < n; i++) s += i; return s; }

int
main (void)
{
  return f (4) == 6 ? 0 : 1;
}
