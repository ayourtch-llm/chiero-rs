/* A `__attribute__((packed))` struct read through a pointer, and a bit-field beside it.
 *
 * Every VPP wire header is packed: `ip4_header_t` puts a 16-bit total length at byte 2 with
 * no padding, and the version/IHL nibbles share byte 0. A layout that inserted the natural
 * padding would put every field after the first at the wrong offset — and the program would
 * still run, quietly reading the wrong bytes. */
#include "chiero.h"

struct __attribute__((packed)) hdr {
  unsigned char kind;   /* 0 */
  unsigned int len;     /* 1 — unaligned, which is the point */
  unsigned char flags;  /* 5 */
};

struct bits {
  unsigned int lo : 4;
  unsigned int hi : 4;
};

static struct hdr wire = {7, 1000, 3};

int main(void) {
  /* Packed: no padding, so the whole thing is 1 + 4 + 1. `sizeof` is a compile-time fact
   * chiero must agree with gcc about, and 014's differential already checks that — here it
   * is the *access* offsets that follow from it. */
  chiero_assert(sizeof(struct hdr) == 6);

  struct hdr *h = &wire;
  chiero_assert(h->kind == 7);
  chiero_assert(h->len == 1000);
  chiero_assert(h->flags == 3);

  /* A bit-field pair sharing one byte. Writing `lo` must leave `hi` alone — 020 contract 25
   * in the small. */
  struct bits b;
  b.lo = 0;
  b.hi = 0;
  b.lo = 5;
  chiero_assert(b.lo == 5);
  chiero_assert(b.hi == 0);
  return 0;
}
