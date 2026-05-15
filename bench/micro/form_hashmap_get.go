// Go equivalent of form_hashmap_get.ap.
// Populate (outside timing) then N indexed lookups. Go's
// `v, ok := m[k]` is the analog of Aperio's `m.get(k) or raise` —
// both check for presence; we use the one-value form since
// every key is present in this bench.
package main

import (
	"fmt"
	"time"
)

type Entry struct {
	id int
	v  int
}

func main() {
	n := 150000
	m := make(map[int]Entry, n)
	for i := 0; i < n; i++ {
		m[i] = Entry{id: i, v: i + 1}
	}

	t0 := time.Now()
	acc := 0
	for j := 0; j < n; j++ {
		e := m[j]
		acc += e.v
	}
	elapsed := time.Since(t0).Nanoseconds()
	fmt.Printf("n=%d\n", n)
	fmt.Printf("acc=%d\n", acc)
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
