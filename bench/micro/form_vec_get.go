// Go equivalent of form_vec_get.ap.
// Populate (outside timing) then time bounds-checked indexed
// reads. Go's `v[i]` is bounds-checked by default — closest
// analog to Aperio's `get(i) or raise`.
package main

import (
	"fmt"
	"time"
)

func main() {
	iters := 200000
	v := make([]int, 0, iters)
	for i := 0; i < iters; i++ {
		v = append(v, i)
	}

	t0 := time.Now()
	acc := 0
	for j := 0; j < iters; j++ {
		acc = v[j]
	}
	elapsed := time.Since(t0).Nanoseconds()

	fmt.Printf("iters=%d\n", iters)
	fmt.Printf("acc=%d\n", acc)
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
