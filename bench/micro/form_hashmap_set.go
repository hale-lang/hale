// Go equivalent of form_hashmap_set.ap.
// N inserts into a map[int]Entry. Go's map is hash-based with
// chained-bucket collision resolution — same big-O as Aperio's
// open-addressing implementation, different layout.
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
	n := 1000000
	m := make(map[int]Entry)
	t0 := time.Now()
	for i := 0; i < n; i++ {
		m[i] = Entry{id: i, v: i + 1}
	}
	elapsed := time.Since(t0).Nanoseconds()
	fmt.Printf("n=%d\n", n)
	fmt.Printf("len=%d\n", len(m))
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
