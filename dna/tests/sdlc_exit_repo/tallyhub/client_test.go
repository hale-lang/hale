package tallyhub

import "testing"

func TestBody(t *testing.T) {
	got := Body(Entry{Account: "dock-7", Cents: 1250})
	if got != "account=dock-7&cents=1250" {
		t.Fatalf("unexpected body %q", got)
	}
}
