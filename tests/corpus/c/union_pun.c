/* 020 §4.5's own example: `ip4_address_t` is `union { u32 as_u32; u8 as_u8[4]; }`, and
 * reading a member other than the last written is legal, defined by gcc, and something
 * VPP depends on. Written through one view, read through the other. */
#include "chiero.h"

union ip4 {
  unsigned int as_u32;
  unsigned char as_u8[4];
};

int main(void) {
  union ip4 a;
  a.as_u32 = 0;
  chiero_make_symbolic(&a.as_u32, sizeof a.as_u32, "addr");
  /* Little-endian: byte 0 is the low byte. The assume pins it so the native run and
   * chiero agree on what the read must produce. */
  chiero_assume((a.as_u32 & 0xFF) == 0x7F);
  chiero_assert(a.as_u8[0] == 0x7F);
  return 0;
}
