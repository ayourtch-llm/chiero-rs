/* Two force-inlined calls on one line, so two blocks both list line 16 —
   and for each of them line 16 is the *greatest* line in the group, not the
   last one written. gcov sorts each block's line list (gcc/gcov.cc ~1413)
   before attributing the block to its last line, so both belong to line 16
   and its count is a graph computation. Attributing by the unsorted last
   line leaves line 16 with no blocks at all, and it falls back to the sum. */
__attribute__ ((always_inline)) static inline int
bump (int *p, int n)
{
  int q = *p;
  q += n;
  *p = q;
  return q;
}

int
main (void)
{
  int s = 0;
  for (int i = 0; i < 3; i++)
    { s = bump (&s, i); s = bump (&s, 1); }
  return s == 6 ? 0 : 1;
}
