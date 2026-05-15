// Go equivalent of form_vec_push.ap.
// `append` to a `[]int` from cap=0; Go's growth policy is
// similar to Aperio's @form(vec) doubling.
package main

import (
	"fmt"
	"time"
)

func main() {
	iters := 500000
	v := make([]int, 0)
	t0 := time.Now()
	for i := 0; i < iters; i++ {
		v = append(v, i)
	}
	elapsed := time.Since(t0).Nanoseconds()
	fmt.Printf("iters=%d\n", iters)
	fmt.Printf("len=%d\n", len(v))
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
