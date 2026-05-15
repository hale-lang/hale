"""Python equivalent of form_vec_get.ap.
Populate (outside timing) then time indexed reads.
"""

import time

iters = 200_000
v = [i for i in range(iters)]

t0 = time.monotonic_ns()
acc = 0
for j in range(iters):
    acc = v[j]
elapsed = time.monotonic_ns() - t0
print(f"iters={iters}")
print(f"acc={acc}")
print(f"elapsed_ns={elapsed}")
