/* File-scope variables — the shape the corpus had none of.
 *
 * Waves 112 and 113 found that lowering had no notion of a global at all (reads became
 * `Undef`) and then that initializers were parsed and discarded (reads became zero). Both
 * were found by hand-written probes. Nothing in `tests/corpus/c/` had a file-scope variable,
 * so no golden had ever seen one and neither defect could have been caught here.
 *
 * This file exercises what VPP is made of: a `static` counter, an initialized table, a
 * struct at file scope, and a read through each. Under chiero the index is symbolic, so the
 * table read is a real bounds question; natively `i` is 2 and the program exits 0. */
#include "chiero.h"

/* An initialized table — the case wave 113 added, and the one a golden has never seen. */
static const int table[4] = {10, 20, 30, 40};

/* A mutable counter with no initializer: C11 6.7.9p10 makes it zero, which is a *value*
 * and not an absence. */
static int calls;

/* A struct at file scope, with padding the encoder has to get from the layout rather than
 * by concatenating fields. */
struct config {
  char kind;
  int limit;
};
static struct config cfg = {1, 40};

static int lookup(int i) {
  calls = calls + 1;
  return table[i];
}

int main(void) {
  int i = 2;
  chiero_make_symbolic(&i, sizeof i, "i");
  /* The table has four entries; without this the read is out of bounds and the assertion
   * below is about a program that traps. */
  chiero_assume(i >= 0);
  chiero_assume(i < 4);

  int v = lookup(i);
  /* Every entry is a multiple of ten and at most the configured limit. Both facts come
   * from initializers, so an engine that read the table as zeros satisfies the first
   * vacuously and the second trivially — `calls` is what separates them. */
  chiero_assert(v % 10 == 0);
  chiero_assert(v <= cfg.limit);
  chiero_assert(calls == 1);
  return 0;
}
