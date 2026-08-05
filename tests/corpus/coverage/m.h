#define ADD1(V) do { (V) = (V) + 1; (V) = (V) * 2; } while (0)
static inline int hdr_fn(int x){ return x < 0 ? -x : x; }
