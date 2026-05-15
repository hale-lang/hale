// Go equivalent of tree_fanout.ap.
// Parent struct loops K times, constructs a Worker struct,
// invokes compute() on it (analogue of Aperio's accept(w) →
// w.compute()), aggregates result. K=20 matches Aperio's
// accept() cliff so the ratio stays apples-to-apples.
package main

import (
	"fmt"
	"time"
)

type Worker struct {
	id        int
	batchSize int
}

//go:noinline
func (w *Worker) compute() int {
	sum := 0
	for i := 0; i < w.batchSize; i++ {
		sum += i
	}
	return sum
}

type Coordinator struct {
	numWorkers int
	itemsEach  int
	total      int
}

//go:noinline
func (c *Coordinator) accept(w *Worker) {
	c.total += w.compute()
}

func (c *Coordinator) run() {
	for i := 0; i < c.numWorkers; i++ {
		w := &Worker{id: i, batchSize: c.itemsEach}
		c.accept(w)
	}
}

func main() {
	k := 20
	m := 2000
	t0 := time.Now()
	c := &Coordinator{numWorkers: k, itemsEach: m}
	c.run()
	elapsed := time.Since(t0).Nanoseconds()
	fmt.Printf("k=%d\n", k)
	fmt.Printf("m=%d\n", m)
	fmt.Printf("total_ops=%d\n", k*m)
	fmt.Printf("total=%d\n", c.total)
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
