"""Python equivalent of bus_dispatch.ap.
Subject-keyed handler dispatch via a dict — mirrors Aperio's
bus router lookup.
"""

import time


class Tick:
    __slots__ = ("n",)
    def __init__(self, n): self.n = n


class Aggregator:
    __slots__ = ("count",)
    def __init__(self): self.count = 0
    def on_tick(self, t): self.count = self.count + 1


agg = Aggregator()
router = {"bench.tick": agg.on_tick}

iters = 10_000
t0 = time.monotonic_ns()
for i in range(iters):
    handler = router["bench.tick"]
    handler(Tick(i))
elapsed = time.monotonic_ns() - t0
print(f"iters={iters}")
print(f"count={agg.count}")
print(f"elapsed_ns={elapsed}")
