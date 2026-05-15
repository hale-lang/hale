// Go equivalent of locus_instantiation.ap.
// Helper fn allocates Empty on the heap, calls a method to
// return a field, XORs the result into a sink. Matches the
// Aperio bench's shape so neither side gets DCE'd.
//
// Previous version used an empty struct, which Go's runtime
// optimizes to a shared singleton pointer — not a fair test.
// Empty now carries a field so each instance is real.
package main

import (
	"fmt"
	"os"
	"time"
)

type Empty struct{ v int }

//go:noinline
func (e *Empty) read() int { return e.v }

//go:noinline
func instantiateOne(seed int) int {
	e := &Empty{v: seed}
	return e.read()
}

func main() {
	iters := 100000
	pid := os.Getpid()
	t0 := time.Now()
	sink := 0
	for i := 0; i < iters; i++ {
		sink ^= instantiateOne(i + pid)
	}
	elapsed := time.Since(t0).Nanoseconds()
	fmt.Printf("iters=%d\n", iters)
	fmt.Printf("sink=%d\n", sink)
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
