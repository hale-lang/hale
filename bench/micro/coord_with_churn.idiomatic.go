// Idiomatic Go equivalent of coord_with_churn.ap.
//
// Where coord_with_churn.go runs K=20 workers sequentially in a
// loop, idiomatic Go for "K independent units of work" is K
// goroutines + sync.WaitGroup. This is the canonical fanout
// pattern in real Go programs (web request handler pools, batch
// processors, etc.).
//
// Note: 20 goroutines * ~1μs goroutine startup = ~20μs of pure
// scheduling overhead, which dominates the trivial per-worker
// work. This is the realistic cost of goroutine fanout for small
// K, comparable to Aperio's cooperative scheduler.
package main

import (
	"fmt"
	"sync"
	"time"
)

type Worker struct {
	id int
}

func main() {
	k := 20

	t0 := time.Now()
	var wg sync.WaitGroup
	wg.Add(k)
	for i := 0; i < k; i++ {
		go func(id int) {
			defer wg.Done()
			w := &Worker{id: id}
			_ = w
		}(i)
	}
	wg.Wait()
	elapsed := time.Since(t0).Nanoseconds()

	fmt.Printf("k=%d\n", k)
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
