

static int helper(int x){ return x * 100; }
int from_b(int x){ return helper(helper(x)); }
