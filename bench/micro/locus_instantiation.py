"""Python equivalent of locus_instantiation.ap.
Matches the helper-fn + field-read shape.
"""

import os
import time


class Empty:
    __slots__ = ("v",)
    def __init__(self, v): self.v = v
    def read(self): return self.v


def instantiate_one(seed):
    e = Empty(seed)
    return e.read()


iters = 100_000
pid = os.getpid()
t0 = time.monotonic_ns()
sink = 0
for i in range(iters):
    sink = sink ^ instantiate_one(i + pid)
elapsed = time.monotonic_ns() - t0
print(f"iters={iters}")
print(f"sink={sink}")
print(f"elapsed_ns={elapsed}")
