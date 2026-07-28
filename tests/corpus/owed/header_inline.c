/* A `static inline` from a real header, and a struct returned by value.
 *
 * Under chiero one input is symbolic and bounded; natively the program exits 0. */
#include "chiero.h"
#include "pair.h"

int main(void) {
  int a = 3;
  chiero_make_symbolic(&a, sizeof a, "a");
  chiero_assume(a >= 0);
  chiero_assume(a <= 10);

  /* `make_pair` orders its arguments, so `lo <= hi` whichever way round they arrive — an
   * aggregate return that lost a field, or returned the caller's uninitialised slot,
   * fails this. */
  struct pair p = make_pair(a, 5);
  chiero_assert(p.lo <= p.hi);

  /* And the span is the distance between them, which pins both fields rather than one. */
  int s = span_of(p);
  chiero_assert(s >= 0);
  chiero_assert(s <= 10);

  /* Passing the struct *by value* into a second helper is the other half: a copy that
   * aliased the original would still pass the assertions above. */
  struct pair q = make_pair(p.hi, p.lo);
  chiero_assert(q.lo == p.lo);
  chiero_assert(q.hi == p.hi);
  return 0;
}
