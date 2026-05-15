// Go equivalent of fn_scratch_work.ap.
// 100 fn calls × 1000-element local slice each.
package main

import (
	"fmt"
	"time"
)

//go:noinline
func doWork(n int) int {
	v := make([]int, 0)
	for i := 0; i < n; i++ {
		v = append(v, i)
	}
	sum := 0
	for j := 0; j < n; j++ {
		sum += v[j]
	}
	return sum
}

func main() {
	calls := 100
	perCall := 1000
	t0 := time.Now()
	total := 0
	for c := 0; c < calls; c++ {
		total += doWork(perCall)
	}
	elapsed := time.Since(t0).Nanoseconds()
	fmt.Printf("calls=%d\n", calls)
	fmt.Printf("per_call=%d\n", perCall)
	fmt.Printf("total=%d\n", total)
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
