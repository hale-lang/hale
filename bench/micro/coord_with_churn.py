"""Python equivalent of coord_with_churn.ap.
K=20 to match Aperio's accept() ceiling.
"""

import time


class Worker:
    __slots__ = ("n",)
    def __init__(self, n): self.n = n


class Coord:
    __slots__ = ("batch",)
    def __init__(self, batch): self.batch = batch
    def on_accept(self, w): pass
    def run(self):
        for i in range(self.batch):
            w = Worker(i)
            self.on_accept(w)


k = 20
t0 = time.monotonic_ns()
c = Coord(k)
c.run()
elapsed = time.monotonic_ns() - t0
print(f"k={k}")
print(f"elapsed_ns={elapsed}")
