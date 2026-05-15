// Idiomatic Go equivalent of pipeline_3stage.ap.
//
// Canonical Go pipeline pattern: each stage is a goroutine,
// stages connected by buffered channels. The Source closes its
// output channel when done; each stage closes its output channel
// after its input drains, propagating shutdown.
package main

import (
	"fmt"
	"sync"
	"time"
)

type Event struct{ value int }
type Filtered struct{ value int }

type Sink struct {
	count, sum int
}

type Filter struct {
	passed int
}

func main() {
	n := 50000
	eventCh := make(chan Event, 64)
	filteredCh := make(chan Filtered, 64)

	sink := &Sink{}
	filter := &Filter{}

	var wg sync.WaitGroup

	// Sink goroutine
	wg.Add(1)
	go func() {
		defer wg.Done()
		for f := range filteredCh {
			sink.count++
			sink.sum += f.value
		}
	}()

	// Filter goroutine
	wg.Add(1)
	go func() {
		defer wg.Done()
		defer close(filteredCh)
		for e := range eventCh {
			if e.value%2 == 0 {
				filter.passed++
				filteredCh <- Filtered{value: e.value}
			}
		}
	}()

	t0 := time.Now()
	for i := 0; i < n; i++ {
		eventCh <- Event{value: i}
	}
	close(eventCh)
	wg.Wait()
	elapsed := time.Since(t0).Nanoseconds()

	fmt.Printf("n=%d\n", n)
	fmt.Printf("filter_passed=%d\n", filter.passed)
	fmt.Printf("sink_count=%d\n", sink.count)
	fmt.Printf("sink_sum=%d\n", sink.sum)
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
