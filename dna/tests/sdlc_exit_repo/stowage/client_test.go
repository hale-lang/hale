package stowage

import "testing"

func TestRequest(t *testing.T) {
	body, err := Request(NewParcel{Label: "crate", Zone: "A"})
	if err != nil {
		t.Fatal(err)
	}
	if len(body) == 0 || body[0] != '{' {
		t.Fatalf("unexpected body %q", body)
	}
}
