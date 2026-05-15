// Go equivalent of fn_call.ap.
// Free-fn call overhead.
package main

import (
	"fmt"
	"time"
)

//go:noinline
func noop(x int) int {
	return x
}

func main() {
	iters := 10000000
	t0 := time.Now()
	acc := 0
	for i := 0; i < iters; i++ {
		acc = noop(i)
	}
	elapsed := time.Since(t0).Nanoseconds()
	fmt.Printf("iters=%d\n", iters)
	fmt.Printf("acc=%d\n", acc)
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
