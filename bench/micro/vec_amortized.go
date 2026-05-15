// Go equivalent of vec_amortized.ap.
// Build + consume + GC, single timed region.
package main

import (
	"fmt"
	"time"
)

func main() {
	n := 200000
	t0 := time.Now()

	v := make([]int, 0)
	for i := 0; i < n; i++ {
		v = append(v, i)
	}
	sum := 0
	for j := 0; j < n; j++ {
		sum += v[j]
	}

	elapsed := time.Since(t0).Nanoseconds()
	fmt.Printf("n=%d\n", n)
	fmt.Printf("sum=%d\n", sum)
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
