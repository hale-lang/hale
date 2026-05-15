"""Python equivalent of tree_fanout.ap."""

import time


class Worker:
    __slots__ = ("id", "batch_size")
    def __init__(self, id, batch_size):
        self.id = id
        self.batch_size = batch_size
    def compute(self):
        total = 0
        for i in range(self.batch_size):
            total = total + i
        return total


class Coordinator:
    __slots__ = ("num_workers", "items_each", "total")
    def __init__(self, num_workers, items_each):
        self.num_workers = num_workers
        self.items_each = items_each
        self.total = 0
    def accept(self, w):
        self.total = self.total + w.compute()
    def run(self):
        for i in range(self.num_workers):
            w = Worker(i, self.items_each)
            self.accept(w)


k = 20
m = 2000
t0 = time.monotonic_ns()
c = Coordinator(k, m)
c.run()
elapsed = time.monotonic_ns() - t0
print(f"k={k}")
print(f"m={m}")
print(f"total_ops={k * m}")
print(f"total={c.total}")
print(f"elapsed_ns={elapsed}")
