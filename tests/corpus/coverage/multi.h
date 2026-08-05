/* Force-inlined, so every caller gets its own copy of these lines. */
__attribute__ ((always_inline)) static inline int
bump (int x, int n)
{
  int q = x + n;
  return q;
}
