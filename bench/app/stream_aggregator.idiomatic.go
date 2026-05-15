// Idiomatic Go equivalent of stream_aggregator.ap.
//
// Long-running aggregator as a goroutine reading from a buffered
// channel. Producer sends N samples; goroutine accumulates
// count/sum/min/max. Standard Go pattern for "consume a stream of
// events into accumulated state."
package main

import (
	"fmt"
	"sync"
	"time"
)

type Sample struct{ value int }

type Aggregator struct {
	count, sum, minV, maxV int
}

func main() {
	iters := 200000
	agg := &Aggregator{minV: 999999999}
	ch := make(chan Sample, 64)

	var wg sync.WaitGroup
	wg.Add(1)
	go func() {
		defer wg.Done()
		for s := range ch {
			agg.count++
			agg.sum += s.value
			if s.value < agg.minV {
				agg.minV = s.value
			}
			if s.value > agg.maxV {
				agg.maxV = s.value
			}
		}
	}()

	t0 := time.Now()
	for i := 0; i < iters; i++ {
		v := (i*31 + 7) % 1000
		ch <- Sample{value: v}
	}
	close(ch)
	wg.Wait()
	elapsed := time.Since(t0).Nanoseconds()

	fmt.Printf("iters=%d\n", iters)
	fmt.Printf("count=%d\n", agg.count)
	fmt.Printf("sum=%d\n", agg.sum)
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
