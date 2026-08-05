/* Two spellings of one path. gcov canonicalizes a file name before it becomes
   a source (find_source -> canonicalize_name), so `gen.c` and `./gen.c` are
   one file and `p` and `q` — both at line 5 of it — are a group. Keyed by the
   raw string they are two files, and neither the group nor the shared line
   is seen. */
#line 5 "gen.c"
int p (void) { return 1; }
#line 5 "./gen.c"
int q (void) { return 2; }
#line 20 "xline.c"
int
main (void)
{
  return p () + q () == 3 ? 0 : 1;
}
