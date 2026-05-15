"""Python equivalent of vec_amortized.ap."""

import time

n = 200_000
t0 = time.monotonic_ns()

v = []
for i in range(n):
    v.append(i)
total = 0
for j in range(n):
    total = total + v[j]

elapsed = time.monotonic_ns() - t0
print(f"n={n}")
print(f"sum={total}")
print(f"elapsed_ns={elapsed}")
