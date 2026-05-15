// Go equivalent of stream_aggregator.ap.
// Pub/sub aggregator: publisher fires N typed samples, an
// aggregator subscribes via subject-keyed dispatch (same shape
// as bus_dispatch.go) and maintains running sum/min/max.
package main

import (
	"fmt"
	"time"
)

type Sample struct {
	Value int
}

type Aggregator struct {
	count int
	sum   int
	minV  int
	maxV  int
}

func (a *Aggregator) onSample(s Sample) {
	a.count++
	a.sum += s.Value
	if s.Value < a.minV {
		a.minV = s.Value
	}
	if s.Value > a.maxV {
		a.maxV = s.Value
	}
}

func main() {
	iters := 200000
	agg := &Aggregator{minV: 999999999}
	router := map[string]func(Sample){
		"bench.sample": agg.onSample,
	}

	t0 := time.Now()
	handler := router["bench.sample"]
	for i := 0; i < iters; i++ {
		v := (i*31 + 7) % 1000
		handler(Sample{Value: v})
	}
	elapsed := time.Since(t0).Nanoseconds()

	fmt.Printf("iters=%d\n", iters)
	fmt.Printf("count=%d\n", agg.count)
	fmt.Printf("sum=%d\n", agg.sum)
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
