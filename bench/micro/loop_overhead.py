"""Python equivalent of loop_overhead.ap.
XOR accumulation — no bignum blowup since XOR of two small ints
stays small.
"""

import os
import time

iters = 100_000_000 + os.getpid()
t0 = time.monotonic_ns()
acc = os.getpid()
for i in range(iters):
    acc = acc ^ i
elapsed = time.monotonic_ns() - t0
print(f"iters={iters}")
print(f"acc={acc}")
print(f"elapsed_ns={elapsed}")
