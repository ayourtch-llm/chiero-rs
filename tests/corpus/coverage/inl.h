static inline __attribute__((always_inline)) int hot(int x)
{
  int y = x + 1;
  return y * 2;
}
