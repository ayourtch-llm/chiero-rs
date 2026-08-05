/* Built with clang, whose `.gcno` is gcov format version 4.08 — a different
   layout from gcc 13.3's, not merely a different number. Record lengths are
   in words, strings are word-counted and NUL-padded, the header has no
   working directory, and a FUNCTION record carries neither the `artificial`
   flag nor any column or end-line. */
int f (int n)
{
  int s = 0;
  for (int i = 0; i < n; i++)
    s += i;
  return s;
}

int
main (void)
{
  return f (4) == 6 ? 0 : 1;
}
