// Package tallyhub builds the requests the shipyard sends to the Tallyhub
// ledger service.
package tallyhub

import "fmt"

// Entry is one line posted to a Tallyhub ledger.
type Entry struct {
	Account string
	Cents   int64
}

// Body renders the entry as the form body Tallyhub accepts. It is the only
// place an entry is built.
func Body(e Entry) string {
	return fmt.Sprintf("account=%s&cents=%d", e.Account, e.Cents)
}
