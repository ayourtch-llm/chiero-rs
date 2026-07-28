/* A `static inline` helper in a header, which is how VPP writes every small accessor.
 *
 * 030 attributes these lines to *this file*, not to the includer — `gcov_lines.rs` tests
 * that attribution in isolation, and nothing until wave 126 lowered and *ran* a program
 * whose helper lives here. */
#ifndef PAIR_H
#define PAIR_H

struct pair {
  int lo;
  int hi;
};

/* Returned **by value**: 015 §2 makes an aggregate return memory rather than a register,
 * and no fixture had one. */
static inline struct pair make_pair(int a, int b) {
  struct pair p;
  p.lo = a < b ? a : b;
  p.hi = a < b ? b : a;
  return p;
}

static inline int span_of(struct pair p) { return p.hi - p.lo; }

#endif
