// Idiomatic Go equivalent of bus_dispatch.ap.
//
// Where bus_dispatch.go uses a map[string]func direct-call (the
// theoretical-best, not how real Go programs do cross-component
// messaging), this version uses the canonical Go primitive:
// a buffered channel + goroutine consumer. This is the standard
// shape any production Go pub/sub uses (sometimes wrapped in an
// event-bus library that adds more overhead).
package main

import (
	"fmt"
	"sync"
	"time"
)

type Tick struct{ n int }

type Aggregator struct {
	count int
}

func main() {
	iters := 10000
	agg := &Aggregator{}
	ch := make(chan Tick, 64)

	var wg sync.WaitGroup
	wg.Add(1)
	go func() {
		defer wg.Done()
		for t := range ch {
			agg.count++
			_ = t
		}
	}()

	t0 := time.Now()
	for i := 0; i < iters; i++ {
		ch <- Tick{n: i}
	}
	close(ch)
	wg.Wait()
	elapsed := time.Since(t0).Nanoseconds()

	fmt.Printf("iters=%d\n", iters)
	fmt.Printf("count=%d\n", agg.count)
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
