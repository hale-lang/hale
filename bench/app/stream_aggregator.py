"""Python equivalent of stream_aggregator.ap.
Pub/sub aggregator over a subject-keyed dict.
"""

import time


class Sample:
    __slots__ = ("value",)
    def __init__(self, value): self.value = value


class Aggregator:
    __slots__ = ("count", "sum", "min_v", "max_v")
    def __init__(self):
        self.count = 0
        self.sum = 0
        self.min_v = 999_999_999
        self.max_v = 0
    def on_sample(self, s):
        self.count = self.count + 1
        self.sum = self.sum + s.value
        if s.value < self.min_v: self.min_v = s.value
        if s.value > self.max_v: self.max_v = s.value


agg = Aggregator()
router = {"bench.sample": agg.on_sample}

iters = 200_000
t0 = time.monotonic_ns()
handler = router["bench.sample"]
for i in range(iters):
    v = (i * 31 + 7) % 1000
    handler(Sample(v))
elapsed = time.monotonic_ns() - t0
print(f"iters={iters}")
print(f"count={agg.count}")
print(f"sum={agg.sum}")
print(f"elapsed_ns={elapsed}")
