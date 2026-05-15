"""Python equivalent of form_hashmap_set.ap."""

import time


class Entry:
    __slots__ = ("id", "v")
    def __init__(self, id, v):
        self.id = id
        self.v = v


n = 1_000_000
m = {}
t0 = time.monotonic_ns()
for i in range(n):
    m[i] = Entry(i, i + 1)
elapsed = time.monotonic_ns() - t0
print(f"n={n}")
print(f"len={len(m)}")
print(f"elapsed_ns={elapsed}")
