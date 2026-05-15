"""Python equivalent of form_hashmap_get.ap."""

import time


class Entry:
    __slots__ = ("id", "v")
    def __init__(self, id, v):
        self.id = id
        self.v = v


n = 150_000
m = {}
for i in range(n):
    m[i] = Entry(i, i + 1)

t0 = time.monotonic_ns()
acc = 0
for j in range(n):
    e = m[j]
    acc = acc + e.v
elapsed = time.monotonic_ns() - t0
print(f"n={n}")
print(f"acc={acc}")
print(f"elapsed_ns={elapsed}")
