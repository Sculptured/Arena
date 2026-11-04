# isaac-sim-standin

The `isaac-sim` integration on its `standin` backend, as a package a world can name: the factory plant with a test
double in place of Isaac Sim, so no GPU is needed. Everything else about it (config, tools, keys, restore) is in
[`integrations/isaac-sim/README.md`](../isaac-sim/README.md), and the same tests cover it. It reads the factory
example, which is not public yet, so without a checkout of that example it can't start.
