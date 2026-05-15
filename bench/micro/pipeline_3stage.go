// Go equivalent of pipeline_3stage.ap.
// Three components connected by a subject-keyed router map —
// the same shape as bus_dispatch.go, just with two hops. Each
// publish is a map lookup + indirect call (no queue, no memcpy
// like Aperio's bus pays).
package main

import (
	"fmt"
	"time"
)

type Event struct{ value int }
type Filtered struct{ value int }

type Sink struct {
	count int
	sum   int
}

//go:noinline
func (s *Sink) onFiltered(f Filtered) {
	s.count++
	s.sum += f.value
}

type Filter struct {
	passed int
	router map[string]func(Filtered)
}

//go:noinline
func (f *Filter) onEvent(e Event) {
	if e.value%2 == 0 {
		f.passed++
		f.router["filtered"](Filtered{value: e.value})
	}
}

type Source struct {
	count  int
	router map[string]func(Event)
}

func (s *Source) run() {
	emit := s.router["event"]
	for i := 0; i < s.count; i++ {
		emit(Event{value: i})
	}
}

func main() {
	n := 50000
	sink := &Sink{}
	filter := &Filter{
		router: map[string]func(Filtered){"filtered": sink.onFiltered},
	}
	source := &Source{
		count:  n,
		router: map[string]func(Event){"event": filter.onEvent},
	}
	t0 := time.Now()
	source.run()
	elapsed := time.Since(t0).Nanoseconds()
	fmt.Printf("n=%d\n", n)
	fmt.Printf("filter_passed=%d\n", filter.passed)
	fmt.Printf("sink_count=%d\n", sink.count)
	fmt.Printf("sink_sum=%d\n", sink.sum)
	fmt.Printf("elapsed_ns=%d\n", elapsed)
}
