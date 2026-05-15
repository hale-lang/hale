// Go equivalent of bus_dispatch.ap.
// Subject-keyed handler dispatch — a map[string]func lookup
// per publish, mirroring Aperio's bus router. NOT channels:
// channels add buffering semantics Aperio's in-scheduler bus
// doesn't have. NOT direct fn calls: those skip the subject
// match Aperio pays for.
package main

import (
	"fmt"
	"time"
)

type Tick struct {
	N int
}

type Aggregator struct {
	count int
}

func (a *Aggregator) onTick(t Tick) {
	a.count++
}

func main() {
	iters := 10000
	agg := &Aggregator{}
	router := map[string]func(Tick){
		"bench.tick": agg.onTick,
	}

	t0 := time.Now()
	for i := 0; i < iters; i++ {
		handler := router["bench.tick"]
		handler(Tick{N: i})
	}
	elapsed := time.Since(t0).Nanoseconds()

	fmt.Printf("iters=%d\n", iters)
	fmt.Printf("count=%d\n", agg.count)
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
