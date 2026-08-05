/* SYNTHETIC — not compiler output. See README.
   A graph whose counters contradict its arcs: block 3 has no predecessor, so
   nothing can reach it, yet the counters require flow through the block it
   feeds. No assignment of the on-tree arcs satisfies conservation. */
int
f (int n)
{
  return n;
}
