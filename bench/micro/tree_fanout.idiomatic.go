// Idiomatic Go equivalent of tree_fanout.ap.
//
// K workers compute partial sums concurrently using goroutines;
// results collected via a buffered channel; parent aggregates the
// total. This is the standard Go "fanout-fanin" pattern for
// parallel reductions.
package main

import (
	"fmt"
	"sync"
	"time"
)

type Worker struct {
	id, batchSize int
}

//go:noinline
func (w *Worker) compute() int {
	sum := 0
	for i := 0; i < w.batchSize; i++ {
		sum += i
	}
	return sum
}

func main() {
	k := 20
	m := 2000

	t0 := time.Now()
	results := make(chan int, k)
	var wg sync.WaitGroup
	wg.Add(k)
	for i := 0; i < k; i++ {
		go func(id int) {
			defer wg.Done()
			w := &Worker{id: id, batchSize: m}
			results <- w.compute()
		}(i)
	}
	wg.Wait()
	close(results)

	total := 0
	for r := range results {
		total += r
	}
	elapsed := time.Since(t0).Nanoseconds()

	fmt.Printf("k=%d\n", k)
	fmt.Printf("m=%d\n", m)
	fmt.Printf("total_ops=%d\n", k*m)
	fmt.Printf("total=%d\n", total)
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
