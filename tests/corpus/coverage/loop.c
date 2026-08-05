int f(int n){ int s=0; for(int i=0;i<n;i++) s+=i; return s; }
int main(void){ return f(4) == 6 ? 0 : 1; }
