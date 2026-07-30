/* Pointer-typed storage and aggregate *values* — the shapes the corpus had none of.
 *
 * A golden quantifies over this directory, so a construct no file here contains is a
 * construct no golden holds fixed. A review of the other thirteen files found the whole
 * corpus had no pointer-typed global, no struct returned by value, no struct passed by
 * value and no local array decaying to a pointer — and six defects had passed 1102 tests,
 * every one of them at a site the corpus could not reach.
 *
 * Each shape here broke at least once, in its own way:
 *
 *   - a pointer-typed global (`AddrOfGlobal` plus a store) read as `Undef` in wave 112,
 *     then read as null in wave 189 when `GlobalInit` lost every address form
 *   - a struct returned by value has nowhere to live (020 §1.4 has no aggregate values),
 *     so it is an sret slot, and waves 126–132 spent five waves on it
 *   - a struct passed by value is a copy the caller makes, which is the same `CopyMem`
 *     from the other side
 *   - a local array decaying is a distinct path from a global array's decay
 *
 * Under chiero the index is symbolic and constrained; natively the program exits 0. */
#include "chiero.h"

struct pair {
  int *p;
  int n;
};

/* An `int` at file scope and a pointer at file scope aimed at it. The pointer is what
 * wave 189 read as null: the initializer is an address, and `GlobalInit` had no way to
 * hold one. */
static int cell = 7;
static int *gp = &cell;

/* A second table so the pointer global has somewhere to move to. */
static int pool[4] = {10, 20, 30, 40};

/* Returned by value: the sret shape. `make_pair` cannot hand back an aggregate in a
 * register the way an `int` goes back, so lowering gives it a slot the caller owns. */
static struct pair make_pair(int i) {
  struct pair q;
  q.p = &pool[i];
  q.n = i;
  return q;
}

/* Taken by value: the same copy from the caller's side, and a read through the member
 * rather than through a local pointer. */
static int sum_through(struct pair q) { return *q.p + q.n; }

int main(void) {
  int i = 2;
  chiero_make_symbolic(&i, sizeof i, "i");
  chiero_assume(i >= 0);
  chiero_assume(i < 4);

  /* A load through a pointer global, then a store through it. An engine that lost the
   * initializer dereferences null here rather than reading 7. */
  int seen = *gp;
  *gp = seen + 1;
  chiero_assert(cell == 8);

  /* The pointer global moved to point into the table: a store *to* the pointer itself,
   * which is a different instruction from the store through it above. */
  gp = &pool[i];
  chiero_assert(*gp % 10 == 0);

  /* An aggregate returned, copy-initialized from the return value, then passed by value.
   * Three separate copies of the same struct, which is what makes `CopyMem` observable. */
  struct pair a = make_pair(i);
  struct pair b = a;
  chiero_assert(sum_through(b) == *a.p + i);

  /* A local array decaying to a pointer, distinct from `pool`'s decay because the object
   * is on the stack and its address is not a link-time constant. */
  int local_arr[3] = {1, 2, 3};
  int *lp = local_arr;
  chiero_assert(lp[1] == 2);
  chiero_assert(*(lp + 2) == 3);

  return 0;
}
