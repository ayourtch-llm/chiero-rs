#include "samelin.h"
int main (void) { int a = 0; g (&a); g (&a); return a == 2 ? 0 : 1; }
