#include <stdio.h>
#include "m.h"
int main(void){ int v=1; ADD1(v); ADD1(v); printf("%d %d\n", v, hdr_fn(-3)); return 0; }
