"""Python equivalent of fn_call.ap.
Free function called in a tight loop. Python's function-call
machinery is famously expensive — frame allocation per call.
"""

import time


def noop(x):
    return x


iters = 10_000_000
t0 = time.monotonic_ns()
acc = 0
for i in range(iters):
    acc = noop(i)
elapsed = time.monotonic_ns() - t0
print(f"iters={iters}")
print(f"acc={acc}")
print(f"elapsed_ns={elapsed}")
