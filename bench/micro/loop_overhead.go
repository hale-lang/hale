// Go equivalent of loop_overhead.ap.
// XOR accumulation defeats DCE without overflow concerns.
package main

import (
	"fmt"
	"os"
	"time"
)

func main() {
	iters := 100000000 + os.Getpid()
	t0 := time.Now()
	acc := os.Getpid()
	for i := 0; i < iters; i++ {
		acc = acc ^ i
	}
	elapsed := time.Since(t0).Nanoseconds()
	fmt.Printf("iters=%d\n", iters)
	fmt.Printf("acc=%d\n", acc)
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
