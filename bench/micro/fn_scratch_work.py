"""Python equivalent of fn_scratch_work.ap."""

import time


def do_work(n):
    v = []
    for i in range(n):
        v.append(i)
    total = 0
    for j in range(n):
        total = total + v[j]
    return total


calls = 100
per_call = 1000
t0 = time.monotonic_ns()
total = 0
for _ in range(calls):
    total = total + do_work(per_call)
elapsed = time.monotonic_ns() - t0
print(f"calls={calls}")
print(f"per_call={per_call}")
print(f"total={total}")
print(f"elapsed_ns={elapsed}")
