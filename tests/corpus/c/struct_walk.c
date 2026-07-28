/* Pointer arithmetic across struct members, and a `static inline` from a header.
 *
 * This is the shape of every VPP graph node: a struct with mixed field widths, accessed
 * through a pointer, with the helper inlined from a header so its lines belong to the
 * header and not the caller. Under chiero the index is symbolic and bounded; natively the
 * program exits 0. */
#include "chiero.h"

struct entry {
  unsigned char kind;
  unsigned int weight;
  int next;
};

static struct entry pool[4] = {
    {1, 10, 1},
    {2, 20, 2},
    {3, 30, 3},
    {4, 40, -1},
};

/* Taken by address, so it cannot be folded away and the pointer arithmetic is real. */
static unsigned int weight_of(const struct entry *e) { return e->weight; }

int main(void) {
  int i = 0;
  chiero_make_symbolic(&i, sizeof i, "i");
  chiero_assume(i >= 0);
  chiero_assume(i < 4);

  const struct entry *e = &pool[i];
  unsigned int w = weight_of(e);

  /* Every weight is a positive multiple of ten. An engine reading the table as zeros
   * fails the first; one that read the wrong *member* — `kind` is at offset 0 and
   * `weight` at 4 — fails the second, because the kinds are 1..4. */
  chiero_assert(w % 10 == 0);
  chiero_assert(w >= 10);
  /* `next` is at offset 8, after padding the layout decides. Reading it proves the walk
   * did not stop at the first member. */
  chiero_assert(e->next >= -1);
  return 0;
}
