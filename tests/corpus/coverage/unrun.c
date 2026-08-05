int never_called(int x){ if (x > 3) return x * 2; return x + 1; }
int ran(int x){ return x + 1; }
int main(void){ return ran(1) == 2 ? 0 : 1; }
