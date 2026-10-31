// Package stowage builds the requests the shipyard sends to the Stowage
// parcel service.
package stowage

import "encoding/json"

// NewParcel is the body of POST /parcels.
type NewParcel struct {
	Label string `json:"label"`
	Zone  string `json:"zone"`
}

// Request returns the JSON body of a parcel registration. It is the only
// place a parcel is built.
func Request(p NewParcel) ([]byte, error) {
	return json.Marshal(p)
}
