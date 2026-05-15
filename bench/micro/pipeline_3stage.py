"""Python equivalent of pipeline_3stage.ap.
Subject-keyed dict router, two hops.
"""

import time


class Event:
    __slots__ = ("value",)
    def __init__(self, value): self.value = value


class Filtered:
    __slots__ = ("value",)
    def __init__(self, value): self.value = value


class Sink:
    __slots__ = ("count", "sum")
    def __init__(self):
        self.count = 0
        self.sum = 0
    def on_filtered(self, f):
        self.count = self.count + 1
        self.sum = self.sum + f.value


class Filter:
    __slots__ = ("passed", "router")
    def __init__(self, router):
        self.passed = 0
        self.router = router
    def on_event(self, e):
        if e.value % 2 == 0:
            self.passed = self.passed + 1
            self.router["filtered"](Filtered(e.value))


class Source:
    __slots__ = ("count", "router")
    def __init__(self, count, router):
        self.count = count
        self.router = router
    def run(self):
        emit = self.router["event"]
        for i in range(self.count):
            emit(Event(i))


n = 50000
sink = Sink()
filter_router = {"filtered": sink.on_filtered}
filt = Filter(filter_router)
source_router = {"event": filt.on_event}
source = Source(n, source_router)

t0 = time.monotonic_ns()
source.run()
elapsed = time.monotonic_ns() - t0
print(f"n={n}")
print(f"filter_passed={filt.passed}")
print(f"sink_count={sink.count}")
print(f"sink_sum={sink.sum}")
print(f"elapsed_ns={elapsed}")
