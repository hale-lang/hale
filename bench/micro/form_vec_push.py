"""Python equivalent of form_vec_push.ap.
list.append from empty.
"""

import time

iters = 500_000
v = []
t0 = time.monotonic_ns()
for i in range(iters):
    v.append(i)
elapsed = time.monotonic_ns() - t0
print(f"iters={iters}")
print(f"len={len(v)}")
print(f"elapsed_ns={elapsed}")
